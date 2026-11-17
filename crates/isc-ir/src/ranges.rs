//! The values a scalar expression can take (D687): an interval of integers,
//! or an interval of `f64` and whether NaN may come. Used to store an array
//! in a narrower type (`narrow_arrays`, D218): the colors of a ray tracer
//! clamped to `0 .. 255` by comparisons of floats, then converted.
//!
//! Every value a variable takes is the value of the `let` that binds it,
//! read anywhere (the IR binds each variable once): a variable is known by
//! its expression, and inside the branch of an `if` by what its condition
//! tells of it (`x < c`, `!(x < c)`, `x == x`); its value is the same there,
//! so what the condition tells holds for it. The bounds of a float
//! operation are computed by the same operation on the bounds: rounding to
//! the nearest is monotone, so the result of any two values between the
//! bounds lies between the results of the bounds. An integer operation
//! whose result may leave its type (it wraps, D329) takes the whole type.
//! What is not followed (`f32`, a parameter, a call, a part of a tuple)
//! takes every value of its type.

use std::collections::HashMap;

use crate::*;

/// The values of a scalar expression.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Vals {
    /// Every integer from the first to the second.
    Int(i128, i128),
    /// Every `f64` from the first to the second (infinities included), and
    /// NaN too when the flag is set.
    Float(f64, f64, bool),
    /// No value: a branch that cannot be taken.
    Never,
}

impl Vals {
    fn union(self, o: Vals) -> Option<Vals> {
        match (self, o) {
            (Vals::Never, x) | (x, Vals::Never) => Some(x),
            (Vals::Int(a, b), Vals::Int(c, d)) => Some(Vals::Int(a.min(c), b.max(d))),
            (Vals::Float(a, b, n), Vals::Float(c, d, m)) => Some(Vals::Float(a.min(c), b.max(d), n || m)),
            _ => None,
        }
    }
}

/// The union of two values, None (unknown) when one is unknown.
fn either(a: Option<Vals>, b: Option<Vals>) -> Option<Vals> {
    a?.union(b?)
}

/// The values of any expression of type `t`; None for a type not followed.
fn whole(t: &Type) -> Option<Vals> {
    match t {
        Type::Int(k) => Some(Vals::Int(k.min_value(), k.max_value())),
        Type::Bool => Some(Vals::Int(0, 1)),
        Type::F64 => Some(Vals::Float(f64::NEG_INFINITY, f64::INFINITY, true)),
        _ => None,
    }
}

/// What is known of the variables where an expression is: the expression
/// of each `let` around it, and what the conditions of the branches it is
/// in tell.
pub struct Known<'a> {
    inits: HashMap<VarId, &'a Expr>,
    told: HashMap<VarId, Vals>,
    /// (the steps left: each variable is followed to its expression where
    /// it is read, a bounded work)
    budget: u32,
}

impl Default for Known<'_> {
    fn default() -> Self {
        Known { inits: HashMap::new(), told: HashMap::new(), budget: 20000 }
    }
}

/// The values `e` can take; None when unknown (not a scalar followed).
pub fn vals<'a>(e: &'a Expr, k: &mut Known<'a>) -> Option<Vals> {
    let w = whole(&e.ty);
    if k.budget == 0 {
        return w;
    }
    k.budget -= 1;
    let r = match &e.kind {
        ExprKind::Lit(Lit::Int(c)) => match e.ty {
            Type::Int(IntKind::U64) => Some(Vals::Int(*c as u64 as i128, *c as u64 as i128)),
            _ => Some(Vals::Int(*c as i128, *c as i128)),
        },
        ExprKind::Lit(Lit::Bool(b)) => Some(Vals::Int(*b as i128, *b as i128)),
        ExprKind::Lit(Lit::F64(x)) => Some(Vals::Float(*x, *x, x.is_nan())),
        ExprKind::Var(x, _) => match (k.told.get(x), k.inits.get(x).copied()) {
            (Some(t), _) => Some(*t),
            (None, Some(init)) => vals(init, k),
            _ => w,
        },
        ExprKind::Let { var, init, body, .. } => {
            k.inits.insert(*var, init);
            vals(body, k)
        }
        ExprKind::If(c, a, b) => {
            let saved = k.told.clone();
            let ra = if tell(c, true, k) { vals(a, k) } else { Some(Vals::Never) };
            k.told = saved.clone();
            let rb = if tell(c, false, k) { vals(b, k) } else { Some(Vals::Never) };
            k.told = saved;
            either(ra, rb)
        }
        ExprKind::Unary(UnOp::Neg, x) => match vals(x, k) {
            Some(Vals::Int(a, b)) => Some(Vals::Int(-b, -a)),
            Some(Vals::Float(a, b, n)) => Some(Vals::Float(-b, -a, n)),
            Some(Vals::Never) => Some(Vals::Never),
            None => w,
        },
        ExprKind::Binary(op, x, y) => match (vals(x, k), vals(y, k)) {
            (Some(Vals::Never), _) | (_, Some(Vals::Never)) => Some(Vals::Never),
            (Some(a), Some(b)) => binary(*op, a, b, &e.ty),
            _ => w,
        },
        ExprKind::Convert(x) => match vals(x, k) {
            Some(a) => convert(a, &e.ty),
            None => w,
        },
        ExprKind::Sqrt(x) => match vals(x, k) {
            Some(Vals::Float(a, b, n)) if a >= 0.0 => Some(Vals::Float(a.sqrt(), b.sqrt(), n)),
            Some(Vals::Float(_, b, _)) => Some(Vals::Float(0.0, b.max(0.0).sqrt(), true)),
            Some(Vals::Never) => Some(Vals::Never),
            _ => w,
        },
        _ => w,
    };
    // (values in the type, or the whole type; f32 operations not followed)
    match (r, &e.ty) {
        (Some(Vals::Int(a, b)), Type::Int(t)) if a < t.min_value() || b > t.max_value() => w,
        (Some(Vals::Int(a, b)), Type::Bool) if a < 0 || b > 1 => w,
        (Some(Vals::Float(..)), t) if *t != Type::F64 => w,
        (Some(Vals::Int(..)), Type::F64) => w,
        _ => r.or(w),
    }
}

/// Integers from `a` to `b` of the type `t`: the whole type when they may
/// leave it (the arithmetic wraps).
fn fit(a: i128, b: i128, t: &Type) -> Option<Vals> {
    match t {
        Type::Int(k) if a >= k.min_value() && b <= k.max_value() => Some(Vals::Int(a, b)),
        _ => whole(t),
    }
}

fn binary(op: BinOp, x: Vals, y: Vals, t: &Type) -> Option<Vals> {
    use BinOp::*;
    match (x, y) {
        (Vals::Int(a, b), Vals::Int(c, d)) => match op {
            Add => fit(a + c, b + d, t),
            Sub => fit(a - d, b - c, t),
            Mul => {
                let ps = [a.checked_mul(c)?, a.checked_mul(d)?, b.checked_mul(c)?, b.checked_mul(d)?];
                fit(*ps.iter().min()?, *ps.iter().max()?, t)
            }
            // (by a positive divisor: the quotient, truncated, is monotone
            // in each operand; its extremes at the corners)
            Div if c > 0 => {
                let ps = [a / c, a / d, b / c, b / d];
                fit(*ps.iter().min()?, *ps.iter().max()?, t)
            }
            // (the sign of the dividend, less than the divisor in size)
            Rem if c > 0 => {
                let m = d - 1;
                if a >= 0 {
                    fit(0, b.min(m), t)
                } else if b <= 0 {
                    fit(a.max(-m), 0, t)
                } else {
                    fit(a.max(-m), b.min(m), t)
                }
            }
            // (with a number that is not negative: not negative, and not
            // above it)
            BitAnd if a >= 0 || c >= 0 => {
                let hi = match (a >= 0, c >= 0) {
                    (true, true) => b.min(d),
                    (true, false) => b,
                    _ => d,
                };
                fit(0, hi, t)
            }
            Shr if a >= 0 && c >= 0 && matches!(t, Type::Int(k) if d < k.bits() as i128) => fit(a >> d, b >> c, t),
            Min => fit(a.min(c), b.min(d), t),
            Max => fit(a.max(c), b.max(d), t),
            Eq | Ne | Lt | Le | Gt | Ge => Some(Vals::Int(0, 1)),
            _ => whole(t),
        },
        (Vals::Float(a, b, n), Vals::Float(c, d, m)) if *t == Type::F64 => {
            let bounds = |ps: [f64; 4], nan: bool| -> Option<Vals> {
                if ps.iter().any(|p| p.is_nan()) {
                    return whole(t);
                }
                Some(Vals::Float(ps.iter().copied().fold(f64::INFINITY, f64::min), ps.iter().copied().fold(f64::NEG_INFINITY, f64::max), nan))
            };
            let inf = f64::INFINITY;
            match op {
                // (infinities of the two signs give NaN)
                Add => bounds([a + c, a + c, b + d, b + d], n || m || (b == inf && c == -inf) || (a == -inf && d == inf)),
                Sub => bounds([a - d, a - d, b - c, b - c], n || m || (b == inf && d == inf) || (a == -inf && c == -inf)),
                Mul => {
                    let zero = |lo: f64, hi: f64| lo <= 0.0 && hi >= 0.0;
                    let infinite = |lo: f64, hi: f64| lo.is_infinite() || hi.is_infinite();
                    bounds([a * c, a * d, b * c, b * d], n || m || (zero(a, b) && infinite(c, d)) || (zero(c, d) && infinite(a, b)))
                }
                _ => whole(t),
            }
        }
        _ => whole(t),
    }
}

/// `x` converted to `t`: from a float, truncated towards zero, saturated at
/// the bounds of `t`, NaN giving 0 (SPEC, conversions).
fn convert(x: Vals, t: &Type) -> Option<Vals> {
    match (x, t) {
        (Vals::Never, _) => Some(Vals::Never),
        (Vals::Float(a, b, n), Type::Int(k)) => {
            let sat = |f: f64| -> i128 {
                let f = f.trunc();
                if f <= k.min_value() as f64 {
                    k.min_value()
                } else if f >= k.max_value() as f64 {
                    k.max_value()
                } else {
                    f as i128
                }
            };
            let (lo, hi) = if a > b { (0, 0) } else { (sat(a), sat(b)) };
            Some(match (n, a > b) {
                (true, true) => Vals::Int(0, 0),
                (true, false) => Vals::Int(lo.min(0), hi.max(0)),
                _ => Vals::Int(lo, hi),
            })
        }
        (Vals::Int(a, b), Type::Int(_)) => fit(a, b, t),
        (Vals::Int(a, b), Type::F64) => Some(Vals::Float(a as f64, b as f64, false)),
        (Vals::Float(..), Type::F64) => Some(x),
        _ => whole(t),
    }
}

/// What `c` being `truth` tells of a variable it compares with another
/// value or with itself, added to `k.told`; false when `c` cannot be
/// `truth`.
fn tell<'a>(c: &'a Expr, truth: bool, k: &mut Known<'a>) -> bool {
    use BinOp::*;
    match &c.kind {
        ExprKind::Unary(UnOp::Not, x) => tell(x, !truth, k),
        ExprKind::Lit(Lit::Bool(b)) => *b == truth,
        // `x == x` true: x is not NaN
        ExprKind::Binary(op @ (Eq | Ne), x, y) if matches!((&x.kind, &y.kind), (ExprKind::Var(a, _), ExprKind::Var(b, _)) if a == b) => {
            let ExprKind::Var(v, _) = x.kind else { unreachable!() };
            if (*op == Eq) == truth {
                if let Some(Vals::Float(a, b, _)) = vals(x, k) {
                    if a > b {
                        return false;
                    }
                    k.told.insert(v, Vals::Float(a, b, false));
                }
            }
            true
        }
        ExprKind::Binary(op @ (Lt | Le | Gt | Ge), x, y) => {
            // `v op c` or `c op v` (the order turned), `c` not a variable
            let (v, side, other, op) = match (&x.kind, &y.kind) {
                (ExprKind::Var(v, _), _) if !matches!(y.kind, ExprKind::Var(..)) => (*v, &**x, &**y, *op),
                (_, ExprKind::Var(v, _)) if !matches!(x.kind, ExprKind::Var(..)) => {
                    let turned = match op {
                        Lt => Gt,
                        Le => Ge,
                        Gt => Lt,
                        _ => Le,
                    };
                    (*v, &**y, &**x, turned)
                }
                _ => return true,
            };
            let (Some(cur), Some(cv)) = (vals(side, k), vals(other, k)) else { return true };
            // (false: the opposite comparison, or a NaN)
            let op = if truth {
                op
            } else {
                match op {
                    Lt => Ge,
                    Le => Gt,
                    Gt => Le,
                    _ => Lt,
                }
            };
            let narrowed = match (cur, cv) {
                (Vals::Never, _) | (_, Vals::Never) => return false,
                (Vals::Int(a, b), Vals::Int(lo, hi)) => {
                    let (a, b) = match op {
                        Lt => (a, b.min(hi - 1)),
                        Le => (a, b.min(hi)),
                        Gt => (a.max(lo + 1), b),
                        _ => (a.max(lo), b),
                    };
                    if a > b {
                        return false;
                    }
                    Vals::Int(a, b)
                }
                (Vals::Float(a, b, n), Vals::Float(lo, hi, cn)) => {
                    // a comparison with a NaN is false: false tells nothing
                    // when either side may be NaN, true tells neither is
                    if !truth && cn {
                        return true;
                    }
                    let nan = n && !truth;
                    let (a, b) = match op {
                        Lt | Le => (a, b.min(hi)),
                        _ => (a.max(lo), b),
                    };
                    if a > b {
                        if !nan {
                            return false;
                        }
                        // (NaN alone: an empty interval)
                        Vals::Float(f64::INFINITY, f64::NEG_INFINITY, true)
                    } else {
                        Vals::Float(a, b, nan)
                    }
                }
                _ => return true,
            };
            k.told.insert(v, narrowed);
            true
        }
        _ => true,
    }
}

/// The values of each part of the tuple that `e` gives (after `let`s and
/// the branches of `if`s); None for a part not followed. None for all when
/// `e` is another form.
pub fn tuple_vals<'a>(e: &'a Expr, k: &mut Known<'a>) -> Option<Vec<Option<Vals>>> {
    let Type::Tuple(ts) = &e.ty else { return None };
    match &e.kind {
        ExprKind::Let { var, init, body, .. } => {
            k.inits.insert(*var, init);
            tuple_vals(body, k)
        }
        ExprKind::Tuple(es) => Some(es.iter().map(|x| vals(x, k)).collect()),
        ExprKind::If(c, a, b) => {
            let never = || Some(vec![Some(Vals::Never); ts.len()]);
            let saved = k.told.clone();
            let ra = if tell(c, true, k) { tuple_vals(a, k) } else { never() };
            k.told = saved.clone();
            let rb = if tell(c, false, k) { tuple_vals(b, k) } else { never() };
            k.told = saved;
            Some(ra?.into_iter().zip(rb?).map(|(p, q)| either(p, q)).collect())
        }
        _ => None,
    }
}
