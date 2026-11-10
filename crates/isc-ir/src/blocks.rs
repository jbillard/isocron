//! The arrays the code generator makes by blocks or by windows, never
//! whole (D225, D293), and the helpers of their recognition: shared with
//! the memory report (D383).

use crate::*;
use std::collections::HashSet;

pub fn free_vars(e: &Expr, bound: &mut HashSet<VarId>, out: &mut Vec<VarId>) {
    match &e.kind {
        ExprKind::Var(v, _) => {
            if !bound.contains(v) && !out.contains(v) {
                out.push(*v);
            }
        }
        ExprKind::Let { var, init, body, .. } => {
            free_vars(init, bound, out);
            bound.insert(*var);
            free_vars(body, bound, out);
        }
        _ => {
            match &e.kind {
                ExprKind::Map(_, l) | ExprKind::Reduce(_, l, _) | ExprKind::Fold(_, _, l) | ExprKind::Scan(_, l, _) | ExprKind::Filter(_, l) | ExprKind::Tasks(_, l) => {
                    for p in &l.params {
                        bound.insert(*p);
                    }
                }
                _ => {}
            }
            for c in e.children() {
                free_vars(c, bound, out);
            }
        }
    }
}

pub fn mentions(e: &Expr, vars: &HashSet<VarId>) -> bool {
    if let ExprKind::Var(v, _) = e.kind {
        return vars.contains(&v);
    }
    e.children().into_iter().any(|c| mentions(c, vars))
}

/// `e` contains a loop of its own (`reduce`, `fold`, `scan`, `filter`):
/// an element of a `map` that costs much more than an operation.
pub fn inner_loop(e: &Expr) -> bool {
    // a search of a regular expression or a replacement in a text is a loop
    // over the text too (D174): a `map` of a few of them is shared out
    matches!(e.kind, ExprKind::Reduce(..) | ExprKind::Fold(..) | ExprKind::Scan(..) | ExprKind::Filter(..) | ExprKind::Call(..))
        || matches!(&e.kind, ExprKind::StrOp(StrOp::Rx(_) | StrOp::ReplaceAll, _))
        || e.children().into_iter().any(inner_loop)
}

/// `let x = filter(...) in body` made by windows (D293): `init` is a
/// `filter` of scalars, behind `let`s of scalars or of views (`slice`), and
/// `body` reads `x` only by its length and, in the function of a `map`
/// over an `iota` written by blocks (`print_stream`), by indices whose
/// interval over a range of the index of the `map` follows from it
/// (`interval_ok`). Returns the `let`s before the `filter` and the `filter`.
pub fn windowed_filter<'a>(x: VarId, init: &'a Expr, body: &Expr) -> Option<(Vec<(VarId, &'a Expr)>, &'a Expr)> {
    let mut lets = Vec::new();
    let mut e = init;
    while let ExprKind::Let { var, init: i, body: b, stmt: false, drop: false } = &e.kind {
        if !(i.ty.is_scalar() || (i.ty.is_array() && i.ty.slice_is_view() && matches!(i.kind, ExprKind::Slice(..)))) {
            return None;
        }
        lets.push((*var, &**i));
        e = b;
    }
    let ExprKind::Filter(src, l) = &e.kind else { return None };
    let scalar = |t: &Type| matches!(t, Type::Int(_) | Type::F64 | Type::F32 | Type::Bool);
    if !scalar(src.ty.elem()) || l.params.len() != 1 || !windowed_uses(body, x) {
        return None;
    }
    Some((lets, e))
}

/// `x` is read in `e` only by its length, and by the indices of the maps
/// that `print_stream` writes (`windowed_filter`): what `print_stream_any`
/// does with the `let`s it writes.
pub fn windowed_uses(e: &Expr, x: VarId) -> bool {
    let is_x = |a: &Expr| matches!(a.kind, ExprKind::Var(v, _) if v == x);
    match &e.kind {
        ExprKind::Len(a) if is_x(a) => true,
        ExprKind::Var(v, _) => *v != x,
        ExprKind::Let { var, init, body, .. } if print_stream_ok(*var, init, body) => windowed_stream(init, x) && windowed_uses(body, x),
        _ => e.children().into_iter().all(|c| windowed_uses(c, x)),
    }
}

/// `windowed_uses` in an array written by `print_stream_any`.
pub fn windowed_stream(e: &Expr, x: VarId) -> bool {
    match &e.kind {
        ExprKind::If(c, a, b) => windowed_uses(c, x) && windowed_stream(a, x) && windowed_stream(b, x),
        ExprKind::Let { init, body, .. } => windowed_uses(init, x) && windowed_stream(body, x),
        ExprKind::CheckNonNegative(k, body) => windowed_uses(k, x) && windowed_stream(body, x),
        ExprKind::Map(ins, l) if stream_map_ok(e) => match ins.as_slice() {
            [n] if matches!(n.kind, ExprKind::Iota(_)) => {
                let (mut bound, mut outer) = (HashSet::new(), Vec::new());
                free_vars(e, &mut bound, &mut outer);
                let outer: HashSet<VarId> = outer.into_iter().collect();
                windowed_uses(n, x) && windowed_lambda(&l.body, x, l.params[0], &outer, &mut Vec::new())
            }
            _ => windowed_uses(e, x),
        },
        _ => windowed_uses(e, x),
    }
}

/// `windowed_uses` in the function of a written `map` of index `q`: `x[i]`
/// with `interval_ok(i)`; `outer` the variables bound outside the `map`,
/// `scope` the `let`s around the read.
pub fn windowed_lambda<'a>(e: &'a Expr, x: VarId, q: VarId, outer: &HashSet<VarId>, scope: &mut Vec<(VarId, &'a Expr)>) -> bool {
    let is_x = |a: &Expr| matches!(a.kind, ExprKind::Var(v, _) if v == x);
    match &e.kind {
        ExprKind::Index(a, i) if is_x(a) => !mentions(i, &HashSet::from([x])) && interval_ok(i, q, outer, scope),
        ExprKind::Len(a) if is_x(a) => true,
        ExprKind::Var(v, _) => *v != x,
        ExprKind::Let { var, init, body, .. } => {
            if !windowed_lambda(init, x, q, outer, scope) {
                return false;
            }
            scope.push((*var, &**init));
            let ok = windowed_lambda(body, x, q, outer, scope);
            scope.pop();
            ok
        }
        // inside another function (a `reduce`, a `map` of its own): only
        // the length
        ExprKind::Map(..) | ExprKind::Reduce(..) | ExprKind::Fold(..) | ExprKind::Scan(..) | ExprKind::Filter(..) | ExprKind::Tasks(..) => windowed_uses(e, x),
        _ => e.children().into_iter().all(|c| windowed_lambda(c, x, q, outer, scope)),
    }
}

/// The integer `e`, of the index `q` of a `map`, of the variables `outer`
/// bound outside it (one value each) and of the `let`s of `scope`, has an
/// interval that `index_interval` computes from the one of `q`: sums,
/// differences, products and quotients by literals, remainders by positive
/// literals, negations, choices, conversions; any integer of fewer than 64
/// bits by the range of its type.
pub fn interval_ok(e: &Expr, q: VarId, outer: &HashSet<VarId>, scope: &[(VarId, &Expr)]) -> bool {
    let Type::Int(k) = e.ty else { return false };
    if k.bits() < 64 {
        return true;
    }
    if k != IntKind::I64 {
        return false;
    }
    let lit = |a: &Expr| matches!(a.kind, ExprKind::Lit(Lit::Int(_)));
    let positive = |a: &Expr| matches!(a.kind, ExprKind::Lit(Lit::Int(c)) if c > 0);
    match &e.kind {
        ExprKind::Var(v, _) if *v == q => true,
        ExprKind::Var(v, _) => match scope.iter().rposition(|(w, _)| w == v) {
            Some(at) => interval_ok(scope[at].1, q, outer, &scope[..at]),
            None => outer.contains(v),
        },
        ExprKind::Lit(Lit::Int(_)) => true,
        ExprKind::Binary(BinOp::Add | BinOp::Sub, a, b) => interval_ok(a, q, outer, scope) && interval_ok(b, q, outer, scope),
        ExprKind::Binary(BinOp::Mul, a, b) if lit(b) => interval_ok(a, q, outer, scope),
        ExprKind::Binary(BinOp::Mul, a, b) if lit(a) => interval_ok(b, q, outer, scope),
        ExprKind::Binary(BinOp::Div | BinOp::Rem, a, b) if positive(b) => interval_ok(a, q, outer, scope),
        ExprKind::Unary(UnOp::Neg, a) => interval_ok(a, q, outer, scope),
        ExprKind::If(_, a, b) => interval_ok(a, q, outer, scope) && interval_ok(b, q, outer, scope),
        ExprKind::Convert(a) => interval_ok(a, q, outer, scope),
        _ => false,
    }
}

pub fn print_stream_ok(var: VarId, init: &Expr, body: &Expr) -> bool {
    streamable(init) && matches!(&body.kind, ExprKind::Action(ActionOp::PrintBytes, args) if args.len() == 1 && matches!(args[0].kind, ExprKind::Var(x, _) if x == var))
}

/// The array of bytes `e` can be made by blocks (`print_stream`): a `map` of
/// the shape `materialize` makes, or such a `map` behind the choice of a
/// table (D230: `if ... then (let H = ... in map) else map`, one branch
/// enough: the other made whole) and the `let`s of scalars or tables
/// before them (D234).
pub fn streamable(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::If(_, a, b) => e.ty.is_array() && (streamable(a) || streamable(b)),
        ExprKind::Let { body, stmt: false, .. } | ExprKind::CheckNonNegative(_, body) => streamable(body),
        _ => stream_map_ok(e),
    }
}

pub fn stream_map_ok(init: &Expr) -> bool {
    matches!(&init.kind, ExprKind::Map(..))
        && init.ty.is_array()
        && *init.ty.elem() == Type::Int(IntKind::U8)
        && !crate::passes::is_stream(init)
        && !matches!(&init.kind, ExprKind::Map(_, l) if inner_loop(&l.body))
        // an input moved into it would no longer be freed with the result
        && !matches!(&init.kind, ExprKind::Map(ins, _) if ins.iter().any(|i| matches!(i.kind, ExprKind::Var(_, Use::Move))))
}
