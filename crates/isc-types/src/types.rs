//! Types, sizes and their normalized polynomial form (SPEC section 4).

use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IntKind {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}

impl IntKind {
    pub fn bits(self) -> u32 {
        match self {
            IntKind::I8 | IntKind::U8 => 8,
            IntKind::I16 | IntKind::U16 => 16,
            IntKind::I32 | IntKind::U32 => 32,
            IntKind::I64 | IntKind::U64 => 64,
        }
    }
    pub fn signed(self) -> bool {
        matches!(self, IntKind::I8 | IntKind::I16 | IntKind::I32 | IntKind::I64)
    }
    pub fn name(self) -> &'static str {
        match self {
            IntKind::I8 => "i8",
            IntKind::I16 => "i16",
            IntKind::I32 => "i32",
            IntKind::I64 => "i64",
            IntKind::U8 => "u8",
            IntKind::U16 => "u16",
            IntKind::U32 => "u32",
            IntKind::U64 => "u64",
        }
    }
    pub fn min_value(self) -> i128 {
        if self.signed() {
            -(1i128 << (self.bits() - 1))
        } else {
            0
        }
    }
    pub fn max_value(self) -> i128 {
        if self.signed() {
            (1i128 << (self.bits() - 1)) - 1
        } else {
            (1i128 << self.bits()) - 1
        }
    }
}

/// Identifier of a size symbol, local to one function definition.
pub type SymId = u32;

/// A size expression in normalized polynomial form: a map from monomials
/// (sorted lists of symbols, with repetition) to non-zero coefficients.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Poly {
    pub terms: BTreeMap<Vec<SymId>, i64>,
}

impl Poly {
    pub fn constant(c: i64) -> Poly {
        let mut p = Poly::default();
        if c != 0 {
            p.terms.insert(Vec::new(), c);
        }
        p
    }
    pub fn sym(s: SymId) -> Poly {
        let mut p = Poly::default();
        p.terms.insert(vec![s], 1);
        p
    }
    pub fn add(&self, o: &Poly) -> Poly {
        let mut r = self.clone();
        for (m, c) in &o.terms {
            let e = r.terms.entry(m.clone()).or_insert(0);
            *e = e.wrapping_add(*c);
            if *e == 0 {
                r.terms.remove(m);
            }
        }
        r
    }
    pub fn neg(&self) -> Poly {
        Poly { terms: self.terms.iter().map(|(m, c)| (m.clone(), c.wrapping_neg())).collect() }
    }
    pub fn sub(&self, o: &Poly) -> Poly {
        self.add(&o.neg())
    }
    pub fn mul(&self, o: &Poly) -> Poly {
        let mut r = Poly::default();
        for (m1, c1) in &self.terms {
            for (m2, c2) in &o.terms {
                let mut m: Vec<SymId> = m1.iter().chain(m2.iter()).copied().collect();
                m.sort();
                let mut t = Poly::default();
                t.terms.insert(m, c1.wrapping_mul(*c2));
                r = r.add(&t);
            }
        }
        r
    }
    pub fn as_const(&self) -> Option<i64> {
        match self.terms.len() {
            0 => Some(0),
            1 => self.terms.get(&Vec::new()).copied(),
            _ => None,
        }
    }
    pub fn as_sym(&self) -> Option<SymId> {
        if self.terms.len() == 1 {
            let (m, c) = self.terms.iter().next().unwrap();
            if *c == 1 && m.len() == 1 {
                return Some(m[0]);
            }
        }
        None
    }
    pub fn syms(&self) -> Vec<SymId> {
        let mut v: Vec<SymId> = self.terms.keys().flatten().copied().collect();
        v.sort();
        v.dedup();
        v
    }
    /// Replaces every symbol by a polynomial.
    pub fn subst(&self, f: &dyn Fn(SymId) -> Poly) -> Poly {
        let mut r = Poly::default();
        for (m, c) in &self.terms {
            let mut t = Poly::constant(*c);
            for s in m {
                t = t.mul(&f(*s));
            }
            r = r.add(&t);
        }
        r
    }
    pub fn show(&self, name: &dyn Fn(SymId) -> String) -> String {
        if self.terms.is_empty() {
            return "0".to_string();
        }
        let mut out = String::new();
        // print non-constant terms first, constant last: `n - k + 1`
        let mut terms: Vec<(&Vec<SymId>, &i64)> = self.terms.iter().filter(|(m, _)| !m.is_empty()).collect();
        if let Some(c) = self.terms.get(&Vec::new()) {
            terms.push((&EMPTY, c));
        }
        for (i, (m, c)) in terms.iter().enumerate() {
            let c = **c;
            let mag = c.unsigned_abs();
            if i == 0 {
                if c < 0 {
                    out.push('-');
                }
            } else {
                out.push_str(if c < 0 { " - " } else { " + " });
            }
            let names: Vec<String> = m.iter().map(|s| name(*s)).collect();
            if m.is_empty() {
                out.push_str(&mag.to_string());
            } else if mag == 1 {
                out.push_str(&names.join(" * "));
            } else {
                out.push_str(&format!("{} * {}", mag, names.join(" * ")));
            }
        }
        out
    }
}

static EMPTY: Vec<SymId> = Vec::new();

/// A size: a polynomial, or an inference variable during checking.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Size {
    Poly(Poly),
    Var(u32),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Ty {
    Int(IntKind),
    F64,
    F32,
    Bool,
    Unit,
    Str,
    Tuple(Vec<Ty>),
    /// A record: its fields sorted by name (records are structural:
    /// `{a: T, b: U}` and `{b: U, a: T}` are the same type).
    Record(Vec<(String, Ty)>),
    /// A sum type declared with `type` (`TProgram::sums[id]`), with its
    /// type arguments (`Option<f64>`).
    Sum { id: u32, name: String, args: Vec<Ty> },
    Array(Size, Box<Ty>),
    Fn(Vec<Ty>, Box<Ty>),
    /// Type parameter of the enclosing generic function.
    Param(u32),
    /// Inference variable (never present after checking).
    Var(u32),
}

impl Ty {
    pub fn is_array(&self) -> bool {
        matches!(self, Ty::Array(..))
    }
    pub fn is_scalar(&self) -> bool {
        matches!(self, Ty::Int(_) | Ty::F64 | Ty::F32 | Ty::Bool)
    }
    pub fn elem(&self) -> Option<&Ty> {
        match self {
            Ty::Array(_, e) => Some(e),
            _ => None,
        }
    }
}

/// Class constraints, as a bit set.
pub mod class {
    pub const NUM: u8 = 1;
    pub const ORD: u8 = 2;
    pub const EQ: u8 = 4;
    /// Internal: must be an integer type (indices, integer exponents).
    pub const INTEGRAL: u8 = 8;
    /// Internal: must be a scalar (int, float or bool).
    pub const SCALAR: u8 = 16;
    /// Internal: must be a floating-point type (`sqrt`).
    pub const FLOAT: u8 = 32;
    /// Internal: can be an array element: a scalar, a string, or a tuple of
    /// these (arrays of tuples are stored in columns, SPEC section 4).
    pub const ELEM: u8 = 64;
    /// Internal: can be an element of a row of an array of arrays: a scalar
    /// or a string (D154).
    pub const ROW: u8 = 128;
}

pub struct TyDisplay<'a> {
    pub ty: &'a Ty,
    pub sym_name: &'a dyn Fn(SymId) -> String,
    pub param_name: &'a dyn Fn(u32) -> String,
    pub size_var: &'a dyn Fn(u32) -> Option<Poly>,
}

impl fmt::Display for TyDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ty {
            Ty::Int(k) => write!(f, "{}", k.name()),
            Ty::F64 => write!(f, "f64"),
            Ty::F32 => write!(f, "f32"),
            Ty::Bool => write!(f, "boolean"),
            Ty::Unit => write!(f, "void"),
            Ty::Str => write!(f, "string"),
            // the syntax of the programs (D148): `[A, B]`
            Ty::Tuple(ts) => {
                write!(f, "[")?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", TyDisplay { ty: t, ..*self })?;
                }
                write!(f, "]")
            }
            Ty::Sum { name, args, .. } => {
                write!(f, "{}", name)?;
                if !args.is_empty() {
                    write!(f, "<")?;
                    for (i, t) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", TyDisplay { ty: t, ..*self })?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
            }
            Ty::Record(fs) => {
                write!(f, "{{")?;
                for (i, (n, t)) in fs.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", n, TyDisplay { ty: t, ..*self })?;
                }
                write!(f, "}}")
            }
            // `T[r][c]`: the element, then the sizes from the outside in
            Ty::Array(..) => {
                let mut sizes = Vec::new();
                let mut t = self.ty;
                while let Ty::Array(s, e) = t {
                    sizes.push(match s {
                        Size::Poly(p) => p.show(self.sym_name),
                        Size::Var(v) => match (self.size_var)(*v) {
                            Some(p) => p.show(self.sym_name),
                            None => "_".to_string(),
                        },
                    });
                    t = e;
                }
                if matches!(t, Ty::Fn(..)) {
                    write!(f, "({})", TyDisplay { ty: t, ..*self })?;
                } else {
                    write!(f, "{}", TyDisplay { ty: t, ..*self })?;
                }
                for s in sizes {
                    write!(f, "[{}]", s)?;
                }
                Ok(())
            }
            Ty::Fn(ps, r) => {
                write!(f, "(")?;
                for (i, t) in ps.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", TyDisplay { ty: t, ..*self })?;
                }
                write!(f, ") => {}", TyDisplay { ty: r, ..*self })
            }
            Ty::Param(p) => write!(f, "{}", (self.param_name)(*p)),
            Ty::Var(_) => write!(f, "_"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly_normal_form() {
        let n = Poly::sym(0);
        let k = Poly::sym(1);
        // n - k + 1 == 1 + n - k
        let a = n.sub(&k).add(&Poly::constant(1));
        let b = Poly::constant(1).add(&n).sub(&k);
        assert_eq!(a, b);
        // 2 * n == n + n
        assert_eq!(Poly::constant(2).mul(&n), n.add(&n));
        // (n + 1) * (n - 1) == n*n - 1
        let l = n.add(&Poly::constant(1)).mul(&n.sub(&Poly::constant(1)));
        let r = n.mul(&n).sub(&Poly::constant(1));
        assert_eq!(l, r);
        assert_eq!(n.sub(&n).as_const(), Some(0));
        assert_eq!(n.as_sym(), Some(0));
        let names = |s: SymId| ["n", "k"][s as usize].to_string();
        assert_eq!(a.show(&names), "n - k + 1");
        assert_eq!(Poly::constant(2).mul(&n).show(&names), "2 * n");
    }

    #[test]
    fn int_ranges() {
        assert_eq!(IntKind::I8.min_value(), -128);
        assert_eq!(IntKind::I8.max_value(), 127);
        assert_eq!(IntKind::U64.max_value(), u64::MAX as i128);
        assert_eq!(IntKind::I64.min_value(), i64::MIN as i128);
    }
}
