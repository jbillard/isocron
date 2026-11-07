//! Type checker for the POC subset (SPEC sections 4, 6, 12 and 15).
//!
//! Every top-level function is checked once, generically: its type
//! parameters are rigid (`Ty::Param`) and its size variables are rigid
//! symbols. Local types (lets, lambdas, literals) are inferred by
//! unification. Calls instantiate the callee's signature.

use std::collections::{HashMap, HashSet};

use crate::tast::*;
use crate::types::class::Classes;
use crate::types::{class, Handle, IntKind, Poly, Size, SymId, Ty, TyDisplay};
use isc_syntax::ast::{self, BinOp, Class, ExprKind, PatternKind, SizeExpr, TypeKind, UnOp};
use isc_syntax::lexer::Scalar;
use isc_syntax::{Diagnostic, Result, Span};

fn err<T>(span: Span, msg: impl Into<String>) -> Result<T> {
    Err(Diagnostic::new(span, msg))
}

fn poc(span: Span, what: &str) -> Diagnostic {
    Diagnostic::new(span, format!("{} is not supported in the POC (SPEC section 15)", what))
}

fn class_bits(c: Option<Class>) -> Classes {
    // every type parameter can be an array element (`[n]T`)
    class::ELEM
        | match c {
            None => 0,
            Some(Class::Num) => class::NUM | class::EQ | class::SCALAR,
            // ordered: the scalars and the strings (D134, D155)
            Some(Class::Ord) => class::ORD | class::EQ,
            Some(Class::Eq) => class::EQ,
            Some(Class::Key) => class::KEY | class::ORD | class::EQ,
        }
}

/// Can `t` be the element type of an array: a scalar, a string, a type
/// parameter, or a tuple of these.
/// Does a value of type `t` hold a `Map` (D158)?
fn holds_map(g: &Globals, t: &Ty) -> bool {
    match t {
        Ty::Sum { id, args, .. } => is_map(&g.sums, *id) || args.iter().any(|a| holds_map(g, a)),
        Ty::Tuple(ts) => ts.iter().any(|a| holds_map(g, a)),
        Ty::Record(fs) => fs.iter().any(|(_, a)| holds_map(g, a)),
        Ty::Array(_, e) => holds_map(g, e),
        _ => false,
    }
}

/// Is the sum type `id` the `Map` of the prelude (D158)?
fn is_map(sums: &[SumDef], id: u32) -> bool {
    let d = &sums[id as usize];
    d.variants.len() == 1 && d.variants[0].name == crate::MAP_CTOR
}

/// The parameter that binds a size of a signature
fn i_of(o: &SymOrigin) -> usize {
    match o {
        SymOrigin::Param(i) | SymOrigin::ParamPath(i, _) | SymOrigin::SizeParam(i) => *i,
        _ => 0,
    }
}

fn is_elem_ty(t: &Ty) -> bool {
    match t {
        Ty::Int(_) | Ty::F64 | Ty::F32 | Ty::Bool | Ty::Str | Ty::Param(_) => true,
        Ty::Tuple(ts) => ts.iter().all(is_part_ty),
        Ty::Record(fs) => fs.iter().all(|(_, t)| is_part_ty(t)),
        // checked when the program is lowered (a payload may hold an array)
        Ty::Sum { .. } => true,
        // arrays of arrays of scalars (SPEC section 4; D95)
        Ty::Array(_, e) => matches!(**e, Ty::Int(_) | Ty::F64 | Ty::F32 | Ty::Bool | Ty::Param(_)),
        _ => false,
    }
}

/// A part of a tuple or a record that is an element of an array: an array
/// there is a block of its own (D336), of any element an array may hold
/// (its size its own or not, D341); the rest as `is_elem_ty`.
fn is_part_ty(t: &Ty) -> bool {
    match t {
        Ty::Array(_, e) => is_elem_ty(e),
        _ => is_elem_ty(t),
    }
}

fn scalar_ty(s: Scalar, _span: Span) -> Result<Ty> {
    Ok(match s {
        Scalar::I8 => Ty::Int(IntKind::I8),
        Scalar::I16 => Ty::Int(IntKind::I16),
        Scalar::I32 => Ty::Int(IntKind::I32),
        Scalar::I64 => Ty::Int(IntKind::I64),
        Scalar::U8 => Ty::Int(IntKind::U8),
        Scalar::U16 => Ty::Int(IntKind::U16),
        Scalar::U32 => Ty::Int(IntKind::U32),
        Scalar::U64 => Ty::Int(IntKind::U64),
        Scalar::F64 => Ty::F64,
        Scalar::Bool => Ty::Bool,
        Scalar::F32 => Ty::F32,
    })
}

/// The functions of TypeScript's `Math` that Isocron has (D151): `Math.f`
/// is the primitive `f`.
const MATH: &[&str] = &[
    "sqrt", "exp", "log", "sin", "cos", "pow", "min", "max", "tan", "asin", "acos", "atan", "atan2", "floor", "ceil", "trunc", "round", "abs",
];

fn prim_by_name(name: &str) -> Option<Prim> {
    if let Some(op) = SysOp::ALL.iter().find(|o| o.name() == name) {
        return Some(Prim::Sys(*op));
    }
    let name = match name.strip_prefix("Math.") {
        Some(f) if MATH.contains(&f) => f,
        Some(_) => return None,
        None => name,
    };
    Some(match name {
        "map" => Prim::Map,
        "mapParallel" => Prim::MapParallel,
        "isPure" => Prim::IsPure,
        "zip" => Prim::Zip,
        "scan" => Prim::Scan,
        "filter" => Prim::Filter,
        "window" => Prim::Window,
        "min" => Prim::Min,
        "max" => Prim::Max,
        "reduce" => Prim::Reduce,
        "fold" => Prim::Fold,
        "zeros" => Prim::Zeros,
        "iota" => Prim::Iota,
        "fill" => Prim::Fill,
        "sqrt" => Prim::Sqrt,
        "exp" => Prim::Math(MathFn::Exp),
        "log" => Prim::Math(MathFn::Log),
        "sin" => Prim::Math(MathFn::Sin),
        "cos" => Prim::Math(MathFn::Cos),
        "pow" => Prim::Math(MathFn::Pow),
        "fma" => Prim::Math(MathFn::Fma),
        "tan" => Prim::Math(MathFn::Tan),
        "asin" => Prim::Math(MathFn::Asin),
        "acos" => Prim::Math(MathFn::Acos),
        "atan" => Prim::Math(MathFn::Atan),
        "atan2" => Prim::Math(MathFn::Atan2),
        "floor" => Prim::Math(MathFn::Floor),
        "ceil" => Prim::Math(MathFn::Ceil),
        "trunc" => Prim::Math(MathFn::Trunc),
        "round" => Prim::Math(MathFn::Round),
        // of an integer too: lowered to a comparison (D155)
        "abs" => Prim::Math(MathFn::Abs),
        "readF64" => Prim::ReadF64,
        "readF32" => Prim::ReadF32,
        "readBytes" => Prim::ReadBytes,
        "writeF64" => Prim::WriteF64,
        "writeF32" => Prim::WriteF32,
        "writeBytes" => Prim::WriteBytes,
        "print" => Prim::Print,
        "clock" => Prim::Clock,
        "bench" => Prim::Bench,
        "parseI64" => Prim::ParseI64,
        "parseF64" => Prim::ParseF64,
        "concat" | "++" => Prim::Concat,
        "len" | "$length" => Prim::StrLen,
        "bytes" => Prim::Bytes,
        "stringOf" => Prim::StringOf,
        // `$text`: an insertion of a template literal (D150)
        // `String(x)`, `x.toString()`, and `$text`, an insertion of a
        // template literal (D150, D151)
        "toString" | "String" | "$text" => Prim::ToString,
        "readText" => Prim::ReadText,
        "writeText" => Prim::WriteText,
        "parseCsvF64" => Prim::ParseCsvF64,
        "readCsvF64" => Prim::ReadCsvF64,
        "lines" => Prim::Lines,
        "split" => Prim::Split,
        "trim" => Prim::Trim,
        "indexOf" => Prim::IndexOf,
        "lastIndexOf" => Prim::LastIndexOf,
        "includes" => Prim::Includes,
        "startsWith" => Prim::StartsWith,
        "endsWith" => Prim::EndsWith,
        "replaceAll" => Prim::ReplaceAll,
        "toUpperCase" => Prim::ToUpper,
        "toLowerCase" => Prim::ToLower,
        "padStart" => Prim::PadStart,
        "padEnd" => Prim::PadEnd,
        "repeat" => Prim::Repeat,
        "trimStart" => Prim::TrimStart,
        "trimEnd" => Prim::TrimEnd,
        "toFixed" => Prim::ToFixed,
        "readStdin" => Prim::ReadStdin,
        "readStdinText" => Prim::ReadStdinText,
        "readStdinLines" => Prim::ReadStdinLines,
        "argsort" => Prim::ArgSort,
        "Number" => Prim::JsNumber,
        "parseFloat" => Prim::ParseFloat,
        "parseInt" => Prim::ParseInt,
        "BigInt" => Prim::ToBigInt,
        "RegExp" | "$RegExp" => Prim::RegExpNew,
        "$mapOf" => Prim::MapOf,
        "$group" => Prim::Group,
        "$groupCounts" => Prim::GroupCounts,
        "$mapParts" => Prim::MapParts,
        "test" => Prim::RxTest,
        "search" => Prim::RxSearch,
        "countMatches" => Prim::RxCount,
        "matches" => Prim::RxMatches,
        "replace" => Prim::Replace,
        "join" => Prim::Join,
        "printBytes" => Prim::PrintBytes,
        _ if Fallible::from_name(name).is_some() => Prim::OrError(Fallible::from_name(name).unwrap()),
        "i8" => Prim::Convert(ScalarTy::Int(IntKind::I8)),
        "i16" => Prim::Convert(ScalarTy::Int(IntKind::I16)),
        "i32" => Prim::Convert(ScalarTy::Int(IntKind::I32)),
        "i64" => Prim::Convert(ScalarTy::Int(IntKind::I64)),
        "u8" => Prim::Convert(ScalarTy::Int(IntKind::U8)),
        "u16" => Prim::Convert(ScalarTy::Int(IntKind::U16)),
        "u32" => Prim::Convert(ScalarTy::Int(IntKind::U32)),
        "u64" => Prim::Convert(ScalarTy::Int(IntKind::U64)),
        "f64" => Prim::Convert(ScalarTy::F64),
        "f32" => Prim::Convert(ScalarTy::F32),
        _ => return None,
    })
}

/// Names of the specification that exist outside the POC subset.
const NOT_IN_POC: &[&str] = &[];

// ---------------------------------------------------------------------
// Signatures

struct Sig {
    name: String,
    span: Span,
    is_action: bool,
    /// a constant of the top level (D159)
    is_const: bool,
    generics: Vec<(String, Classes)>,
    syms: Vec<SymInfo>,
    param_names: Vec<(String, Span)>,
    param_tys: Vec<Ty>,
    ret: Ty,
}

struct SigCtx<'a> {
    generics: &'a [(String, Classes)],
    /// The sum types declared in the program: (name, number of parameters).
    sums: &'a [(String, usize)],
    syms: &'a mut Vec<SymInfo>,
    /// May new size variables be introduced (parameter types, and the
    /// existential sizes of a result)?
    allow_new: bool,
    /// The type of the result of a signature (`[k <= n]T` is allowed there).
    result: bool,
    /// The module of the declaration (D139): its own types first.
    module: Option<&'a str>,
    /// The names its file imports by name (D150): after its own types.
    imports: Option<&'a HashMap<String, String>>,
}

/// The names each file imports by name (D150): file -> local -> `m.x`.
type Imports = HashMap<u32, HashMap<String, String>>;

/// The path (the indices of fields and elements of tuples, `ELEMENT` for
/// the elements of an array when `through`) from `t` to an array whose
/// size is exactly the symbol `s` (D336); not through sum types, whose
/// arrays are not one per value of `t`.
pub fn array_path(t: &Ty, s: SymId, through: bool, path: &mut Vec<u32>) -> bool {
    match t {
        Ty::Array(Size::Poly(p), e) => {
            if p.as_sym() == Some(s) {
                return true;
            }
            if through {
                path.push(crate::tast::ELEMENT);
                if array_path(e, s, through, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        Ty::Record(fs) => {
            for (i, (_, f)) in fs.iter().enumerate() {
                path.push(i as u32);
                if array_path(f, s, through, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        Ty::Tuple(ts) => {
            for (i, f) in ts.iter().enumerate() {
                path.push(i as u32);
                if array_path(f, s, through, path) {
                    return true;
                }
                path.pop();
            }
            false
        }
        _ => false,
    }
}

/// The payload types of a variant of `def` for the type arguments `args`
/// and the sizes `sizes` of its payload (D335).
fn payload_sized(def: &SumDef, variant: usize, args: &[Ty], sizes: &[Size]) -> Vec<Ty> {
    def.variants[variant].payload.iter().map(|t| subst_value(t, args, sizes)).collect()
}

/// The module of a declaration named `m.x` (D139).
fn module_of(name: &str) -> Option<&str> {
    name.rsplit_once('.').map(|(m, _)| m)
}

fn size_from_ast(se: &SizeExpr, cx: &mut SigCtx) -> Result<Poly> {
    Ok(match se {
        SizeExpr::Own(span) => return err(*span, "`[]` (an array of its own size) stands alone as the size of an array"),
        SizeExpr::Lit(v, _) => Poly::constant(*v as i64),
        SizeExpr::Var(name, span) => {
            if let Some(i) = cx.syms.iter().position(|s| &s.name == name) {
                Poly::sym(i as SymId)
            } else if cx.allow_new {
                cx.syms.push(SymInfo { name: name.clone(), origin: SymOrigin::Existential(*span) });
                Poly::sym((cx.syms.len() - 1) as SymId)
            } else {
                return err(
                    *span,
                    format!(
                        "unknown size variable `{}` (in the POC, the size variables of a result must appear in a parameter type)",
                        name
                    ),
                );
            }
        }
        SizeExpr::Add(a, b) => size_from_ast(a, cx)?.add(&size_from_ast(b, cx)?),
        SizeExpr::Sub(a, b) => size_from_ast(a, cx)?.sub(&size_from_ast(b, cx)?),
        SizeExpr::Mul(a, b) => size_from_ast(a, cx)?.mul(&size_from_ast(b, cx)?),
    })
}

fn type_from_ast(t: &ast::TypeExpr, cx: &mut SigCtx) -> Result<Ty> {
    Ok(match &t.kind {
        TypeKind::Scalar(s) => scalar_ty(*s, t.span)?,
        TypeKind::Str => Ty::Str,
        TypeKind::BigInt => Ty::BigInt,
        TypeKind::Unit => Ty::Unit,
        // the built-in `RegExp` (D157), unless the program declares its own
        TypeKind::Named(name, args) if name == "RegExp" && args.is_empty() && !cx.sums.iter().any(|(n, _)| n == "RegExp") => Ty::RegExp,
        // the resources of the system (D339), named by the module `net`
        TypeKind::Named(name, args) if name == "sysConn" && args.is_empty() => Ty::Handle(crate::types::Handle::Conn),
        TypeKind::Named(name, args) if name == "sysListener" && args.is_empty() => Ty::Handle(crate::types::Handle::Listener),
        TypeKind::Named(name, args) if name == "sysServer" && args.is_empty() => Ty::Handle(crate::types::Handle::Server),
        TypeKind::Named(name, args) => {
            if let Some(i) = cx.generics.iter().position(|g| &g.0 == name) {
                if !args.is_empty() {
                    return err(t.span, format!("type parameter `{}` takes no arguments", name));
                }
                Ty::Param(i as u32)
            } else if let Some(id) = cx
                .module
                .filter(|_| !name.contains('.'))
                .and_then(|m| cx.sums.iter().position(|(n, _)| *n == format!("{}.{}", m, name)))
                .or_else(|| cx.imports.and_then(|i| i.get(name)).and_then(|q| cx.sums.iter().position(|(n, _)| n == q)))
                .or_else(|| cx.sums.iter().position(|(n, _)| n == name))
            {
                let arity = cx.sums[id].1;
                if args.len() != arity {
                    return err(t.span, format!("the type `{}` takes {} type argument(s), but {} were given", name, arity, args.len()));
                }
                let args = args.iter().map(|a| type_from_ast(a, cx)).collect::<Result<_>>()?;
                Ty::Sum { id: id as u32, name: name.clone(), args }
            } else {
                return err(t.span, format!("unknown type `{}`", name));
            }
        }
        TypeKind::Array(size, elem) => {
            let p = match size {
                SizeExpr::Own(_) => Size::Own,
                _ => Size::Poly(size_from_ast(size, cx)?),
            };
            let e = type_from_ast(elem, cx)?;
            if !is_elem_ty(&e) {
                return Err(poc(elem.span, "an array whose elements are not scalars, strings or tuples of these"));
            }
            // the rows of an array of arrays share their size (D341)
            if matches!(e, Ty::Array(Size::Own, _)) {
                return err(elem.span, "the rows of an array of arrays have one size: an array of its own size (`T[]`) goes in a record or a tuple of the elements");
            }
            Ty::Array(p, Box::new(e))
        }
        TypeKind::BoundedArray(k, n, elem) => {
            // `[k <= n]T`: an existential size `k`, at most `n` (SPEC section
            // 4); the bound names sizes of the parameters
            if !cx.result {
                return err(t.span, "a bounded array type `[k <= n]T` can only be the type of a result");
            }
            if cx.syms.iter().any(|s| &s.name == k) {
                return err(t.span, format!("the size `{}` of `[{} <= ...]T` must be a new size variable", k, k));
            }
            let saved = cx.allow_new;
            cx.allow_new = false;
            let bound = size_from_ast(n, cx);
            cx.allow_new = saved;
            if bound.is_err() {
                return err(t.span, format!("the bound of `[{} <= ...]T` must be made of sizes of the parameters", k));
            }
            let p = size_from_ast(&SizeExpr::Var(k.clone(), t.span), cx)?;
            let e = type_from_ast(elem, cx)?;
            if !is_elem_ty(&e) {
                return Err(poc(elem.span, "an array whose elements are not scalars, strings or tuples of these"));
            }
            Ty::Array(Size::Poly(p), Box::new(e))
        }
        TypeKind::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| type_from_ast(t, cx)).collect::<Result<_>>()?),
        TypeKind::Record(fields) => {
            let mut fs = Vec::new();
            for (n, ft) in fields {
                if fs.iter().any(|(m, _): &(String, Ty)| m == n) {
                    return err(t.span, format!("the field `{}` appears twice in this record type", n));
                }
                fs.push((n.clone(), type_from_ast(ft, cx)?));
            }
            fs.sort_by(|a, b| a.0.cmp(&b.0));
            Ty::Record(fs)
        }
        TypeKind::Fn(ps, r, act) => Ty::Fn(
            ps.iter().map(|t| type_from_ast(t, cx)).collect::<Result<_>>()?,
            Box::new(type_from_ast(r, cx)?),
        *act),
    })
}

/// Does the size symbol `s` appear in `t` only alone as the size of arrays?
fn alone_in(t: &Ty, s: SymId) -> bool {
    match t {
        Ty::Array(Size::Poly(p), e) => (p.as_sym() == Some(s) || !p.syms().contains(&s)) && alone_in(e, s),
        Ty::Tuple(ts) => ts.iter().all(|t| alone_in(t, s)),
        Ty::Record(fs) => fs.iter().all(|(_, t)| alone_in(t, s)),
        Ty::Sum { args, .. } => args.iter().all(|t| alone_in(t, s)),
        Ty::Fn(ps, r, _) => ps.iter().all(|t| alone_in(t, s)) && alone_in(r, s),
        _ => true,
    }
}

fn sig_of(d: &ast::FnDecl, heads: &[(String, usize)], imports: &Imports) -> Result<Sig> {
    let mut generics = Vec::new();
    for g in &d.generics {
        if generics.iter().any(|(n, _): &(String, Classes)| n == &g.name) {
            return err(g.span, format!("duplicate type parameter `{}`", g.name));
        }
        // the type of `json.decode` and `json.encode` is any type that JSON
        // carries: never specialized, each call given the functions written
        // for its type (D336)
        let classes = if d.name == DECODE || d.name == ENCODE { 0 } else { class_bits(g.class) };
        generics.push((g.name.clone(), classes));
    }
    for (_, sp) in &d.modifiers {
        if d.is_action {
            return err(*sp, "a decorator goes on a `function`, not on an `action`".to_string());
        }
    }
    let mut syms = Vec::new();
    let mut param_tys = Vec::new();
    let mut param_names: Vec<(String, Span)> = Vec::new();
    for p in &d.params {
        if param_names.iter().any(|(n, _)| n == &p.name) {
            return err(p.span, format!("duplicate parameter `{}`", p.name));
        }
        let ty = match &p.ty {
            ast::ParamType::Size => {
                // a size usable in the types of the signature and as an `i64`
                if syms.iter().any(|s: &SymInfo| s.name == p.name) {
                    return err(p.span, format!("`{}` is already a size variable of this signature", p.name));
                }
                syms.push(SymInfo { name: p.name.clone(), origin: SymOrigin::SizeParam(param_tys.len()) });
                Ty::Int(IntKind::I64)
            }
            ast::ParamType::Type(t) => {
                type_from_ast(t, &mut SigCtx { generics: &generics, sums: heads, syms: &mut syms, allow_new: true, result: false, module: module_of(&d.name), imports: imports.get(&d.span.file) })?
            }
        };
        // an action as a parameter only of an action: a `function` has no
        // effect (D352)
        if !d.is_action && contains_action(&ty) {
            return err(p.span, format!("the parameter `{}` of the `function` `{}` is an action: only an `action` takes one (SPEC section 12)", p.name, d.name));
        }
        // `x: T[]`: a size of its own for the parameter (D341), named after it
        let ty = match ty {
            Ty::Array(Size::Own, e) => {
                syms.push(SymInfo { name: format!("{}.length", p.name), origin: SymOrigin::Param(param_tys.len()) });
                Ty::Array(Size::Poly(Poly::sym((syms.len() - 1) as SymId)), e)
            }
            ty => ty,
        };
        param_names.push((p.name.clone(), p.span));
        param_tys.push(ty);
    }
    // Each size variable is bound by the first parameter whose type is an
    // array of exactly that size.
    let mut bound: Vec<bool> = syms.iter().map(|s| matches!(s.origin, SymOrigin::SizeParam(_))).collect();
    for (i, t) in param_tys.iter().enumerate() {
        if let Ty::Array(Size::Poly(p), _) = t {
            if let Some(s) = p.as_sym() {
                if !bound[s as usize] {
                    bound[s as usize] = true;
                    syms[s as usize].origin = SymOrigin::Param(i);
                }
            }
        }
    }
    // then by an array in a record or a tuple of a parameter, then by the
    // elements of an array of a parameter (D336)
    for through in [false, true] {
        for s in 0..syms.len() {
            if bound[s] {
                continue;
            }
            for (i, t) in param_tys.iter().enumerate() {
                let mut path = Vec::new();
                if array_path(t, s as SymId, through, &mut path) && !path.is_empty() {
                    bound[s] = true;
                    syms[s].origin = SymOrigin::ParamPath(i, path);
                    break;
                }
            }
        }
    }
    for (i, b) in bound.iter().enumerate() {
        if !b {
            return err(
                d.span,
                format!(
                    "size variable `{}` must appear alone as the size of an array of a parameter (`x: T[{}]`, `r: {{ xs: T[{}] }}`)",
                    syms[i].name, syms[i].name, syms[i].name
                ),
            );
        }
    }
    // A size variable that appears only in the result is existential: fixed
    // at run time by the function (SPEC section 4), alone as the size of an
    // array (`[k]T` or `[k <= n]T`).
    let nparams_syms = syms.len();
    let ret = type_from_ast(&d.ret, &mut SigCtx { generics: &generics, sums: heads, syms: &mut syms, allow_new: true, result: true, module: module_of(&d.name), imports: imports.get(&d.span.file) })?;
    // a result `T[]`: an existential size (D341)
    let ret = match ret {
        Ty::Array(Size::Own, e) => {
            syms.push(SymInfo { name: format!("k@{}:{}", d.ret.span.line, d.ret.span.col), origin: SymOrigin::Existential(d.ret.span) });
            Ty::Array(Size::Poly(Poly::sym((syms.len() - 1) as SymId)), e)
        }
        ret => ret,
    };
    for s in nparams_syms..syms.len() {
        if !alone_in(&ret, s as SymId) {
            return err(
                d.ret.span,
                format!("the existential size `{}` must appear alone as the size of an array (`[{}]T`)", syms[s].name, syms[s].name),
            );
        }
    }
    Ok(Sig { name: d.name.clone(), span: d.span, is_action: d.is_action, is_const: d.is_const, generics, syms, param_names, param_tys, ret })
}

// ---------------------------------------------------------------------
// Inference state

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LitKind {
    None,
    Int,
    Float,
}

#[derive(Clone, Debug)]
struct TVar {
    bound: Option<Ty>,
    classes: Classes,
    lit: LitKind,
    span: Span,
}

enum UErr {
    Mismatch,
    /// Two sizes that can never be equal.
    Size,
    Constraint(String),
    Occurs,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Strict,
    /// Unprovable top-level size equality becomes a run-time test (of an
    /// array, or of an array in a record or a tuple, D336).
    Dynamic,
}

struct FnCx<'g> {
    g: &'g Globals,
    /// The call being checked is `x.m(...)` (D158): `m` names a function,
    /// not a local variable that is not one.
    method_call: bool,
    fn_id: FnId,
    /// The module of the function (D139): an unqualified name is first
    /// looked for there, then among the names its file imports (D150).
    module: Option<String>,
    file: u32,
    is_action: bool,
    lambda_depth: u32,
    /// The enclosing lambdas that are actions (D352).
    act_lambdas: u32,
    /// For each enclosing lambda that may act, whether it has an effect so
    /// far: one without any is pure (D386).
    act_effects: Vec<bool>,
    generics: Vec<(String, Classes)>,
    syms: Vec<SymInfo>,
    tvars: Vec<TVar>,
    svars: Vec<(Option<Size>, Span)>,
    locals: Vec<Local>,
    scopes: Vec<HashMap<String, LocalId>>,
    deferred_scalar: Vec<(Ty, Span)>,
    /// The arguments of `len` whose type was unknown when checked: a string
    /// or an array, checked at the end (D135).
    deferred_len: Vec<(Ty, Span)>,
    deferred_printable: Vec<(Ty, Span)>,
    /// Set when the last `unify` in dynamic mode could not prove a size
    /// equality.
    dyn_flag: bool,
    /// The sum types whose constraint is being checked: a recursive type
    /// meets it inside itself when it meets it (D103).
    constraining: Vec<(u32, Vec<Ty>, Classes)>,
}

struct Globals {
    sigs: Vec<Sig>,
    names: HashMap<String, FnId>,
    /// The signatures from this index on are those of the `extern`
    /// functions, in their order (D125).
    first_extern: usize,
    /// Declared sum types, their (name, number of parameters), and every
    /// constructor: name -> (type, variant).
    sums: Vec<SumDef>,
    heads: Vec<(String, usize)>,
    ctors: HashMap<String, (u32, u32)>,
    imports: Imports,
    /// The functions `codecDecode`, `codecEncode` written for each type of
    /// `json.decode<T>` and `json.encode(x)`, by its key (D336).
    codecs: HashMap<String, (FnId, FnId)>,
    /// The placeholders `json.decode` and `json.encode`.
    placeholders: (Option<FnId>, Option<FnId>),
}

impl<'g> FnCx<'g> {
    // ----- variables ----------------------------------------------------

    fn fresh(&mut self, classes: Classes, lit: LitKind, span: Span) -> Ty {
        self.tvars.push(TVar { bound: None, classes, lit, span });
        Ty::Var((self.tvars.len() - 1) as u32)
    }

    fn fresh_size(&mut self, span: Span) -> Size {
        self.svars.push((None, span));
        Size::Var((self.svars.len() - 1) as u32)
    }

    fn shallow(&self, t: &Ty) -> Ty {
        let mut t = t.clone();
        while let Ty::Var(v) = t {
            match &self.tvars[v as usize].bound {
                Some(b) => t = b.clone(),
                None => break,
            }
        }
        t
    }

    fn size_res(&self, s: &Size) -> Size {
        let mut s = s.clone();
        while let Size::Var(v) = s {
            match &self.svars[v as usize].0 {
                Some(b) => s = b.clone(),
                None => break,
            }
        }
        s
    }

    fn show(&self, t: &Ty) -> String {
        let t = self.zonk_soft(t);
        let syms = &self.syms;
        let generics = &self.generics;
        let sym_name = |s: SymId| syms.get(s as usize).map(|i| i.name.clone()).unwrap_or_else(|| "?".into());
        let param_name = |p: u32| generics.get(p as usize).map(|g| g.0.clone()).unwrap_or_else(|| "?".into());
        let size_var = |_v: u32| None;
        format!("{}", TyDisplay { ty: &t, sym_name: &sym_name, param_name: &param_name, size_var: &size_var })
    }

    /// Resolves variables as far as possible, leaving unbound ones.
    fn zonk_soft(&self, t: &Ty) -> Ty {
        match self.shallow(t) {
            Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| self.zonk_soft(t)).collect()),
            Ty::Record(fs) => Ty::Record(fs.iter().map(|(n, t)| (n.clone(), self.zonk_soft(t))).collect()),
            Ty::Sum { id, name, args } => {
                Ty::Sum { id, name, args: args.iter().map(|t| self.zonk_soft(t)).collect() }
            }
            Ty::Array(s, e) => Ty::Array(self.size_res(&s), Box::new(self.zonk_soft(&e))),
            Ty::Fn(ps, r, act) => Ty::Fn(ps.iter().map(|t| self.zonk_soft(t)).collect(), Box::new(self.zonk_soft(&r)), act),
            t => t,
        }
    }

    // ----- unification --------------------------------------------------

    fn occurs(&self, v: u32, t: &Ty) -> bool {
        match self.shallow(t) {
            Ty::Var(w) => w == v,
            Ty::Tuple(ts) => ts.iter().any(|t| self.occurs(v, t)),
            Ty::Record(fs) => fs.iter().any(|(_, t)| self.occurs(v, t)),
            Ty::Sum { args, .. } => args.iter().any(|t| self.occurs(v, t)),
            Ty::Array(_, e) => self.occurs(v, &e),
            Ty::Fn(ps, r, _) => ps.iter().any(|t| self.occurs(v, t)) || self.occurs(v, &r),
            _ => false,
        }
    }

    /// Applies class and literal constraints to a type.
    fn constrain(&mut self, t: &Ty, classes: Classes, lit: LitKind) -> std::result::Result<(), UErr> {
        let t = self.shallow(t);
        let what = || -> String {
            let mut v = Vec::new();
            if lit == LitKind::Int {
                v.push("an integer literal");
            }
            if lit == LitKind::Float {
                v.push("a floating-point literal");
            }
            if classes & class::INTEGRAL != 0 {
                v.push("an integer type");
            }
            if classes & class::FLOAT != 0 {
                v.push("a floating-point type");
            }
            if classes & class::NUM != 0 {
                v.push("a numeric type (class Num)");
            }
            if classes & class::KEY != 0 {
                v.push("a key of a Map (class Key: an integer, a boolean, a string or a bigint)");
            } else if classes & class::ORD != 0 {
                v.push("an ordered type (class Ord)");
            }
            if classes & class::EQ != 0 && v.is_empty() {
                v.push("a type with equality (class Eq)");
            }
            if classes & class::SCALAR != 0 && v.is_empty() {
                v.push("a scalar type");
            }
            if classes & class::ELEM != 0 && v.is_empty() {
                v.push("an array element type (a scalar, a string, a tuple of these or an array of scalars or strings)");
            }
            if classes & class::ROW != 0 && v.is_empty() {
                v.push("an element of a row of an array of arrays (a scalar or a string)");
            }
            v.join(" and ")
        };
        let fail = |t: &Ty, s: &Self| Err(UErr::Constraint(format!("expected {}, found `{}`", what(), s.show(t))));
        match &t {
            Ty::Var(v) => {
                let tv = &mut self.tvars[*v as usize];
                tv.classes |= classes;
                tv.lit = match (tv.lit, lit) {
                    (LitKind::Float, _) | (_, LitKind::Float) => LitKind::Float,
                    (LitKind::Int, _) | (_, LitKind::Int) => LitKind::Int,
                    _ => LitKind::None,
                };
                if tv.lit == LitKind::Float && tv.classes & class::INTEGRAL != 0 {
                    return Err(UErr::Constraint(
                        "a floating-point literal cannot have an integer type".to_string(),
                    ));
                }
                if tv.lit == LitKind::Float && tv.classes & class::KEY != 0 {
                    return Err(UErr::Constraint(
                        "a float cannot be a key of a Map (class Key: an integer, a boolean, a string or a bigint)".to_string(),
                    ));
                }
                Ok(())
            }
            Ty::Int(_) => {
                if lit == LitKind::Float || classes & class::FLOAT != 0 {
                    return fail(&t, self);
                }
                Ok(())
            }
            Ty::F64 | Ty::F32 => {
                if classes & (class::INTEGRAL | class::KEY) != 0 {
                    return fail(&t, self);
                }
                Ok(())
            }
            Ty::Bool => {
                if lit != LitKind::None || classes & (class::NUM | class::INTEGRAL | class::FLOAT) != 0 {
                    return fail(&t, self);
                }
                Ok(())
            }
            Ty::Param(p) => {
                let have = self.generics[*p as usize].1;
                let need = classes | if lit != LitKind::None { class::NUM } else { 0 };
                if lit == LitKind::Float || need & (class::INTEGRAL | class::FLOAT) != 0 || need & !have != 0 {
                    return Err(UErr::Constraint(format!(
                        "type parameter `{}` does not satisfy the constraint: expected {}{}",
                        self.generics[*p as usize].0,
                        what(),
                        if have & class::NUM == 0 && need & class::NUM != 0 { " (declare it `T: Num`)" } else { "" }
                    )));
                }
                Ok(())
            }
            Ty::Unit => {
                if lit != LitKind::None || classes & !class::EQ != 0 {
                    return fail(&t, self);
                }
                Ok(())
            }
            // a regular expression (D157): an element of an array, nothing
            // more (no equality, no order)
            Ty::RegExp => {
                if lit != LitKind::None || classes & !class::ELEM != 0 {
                    return fail(&t, self);
                }
                Ok(())
            }
            // a resource of the system (D339): unique, so neither in an
            // array nor compared, nor a type parameter of a function
            Ty::Handle(h) => {
                if lit != LitKind::None || classes != 0 {
                    return Err(UErr::Constraint(format!(
                        "a `{}` is unique (it is never copied): it cannot be stored in an array, compared, printed or given as a type parameter",
                        h.name()
                    )));
                }
                Ok(())
            }
            // an integer of any size (D156): every class of the integers
            Ty::BigInt => {
                if lit == LitKind::Float || classes & class::FLOAT != 0 {
                    return fail(&t, self);
                }
                Ok(())
            }
            // ordered by bytes, lexicographic (D134)
            Ty::Str => {
                if lit != LitKind::None || classes & !(class::EQ | class::ORD | class::ELEM | class::ROW | class::KEY) != 0 {
                    return fail(&t, self);
                }
                Ok(())
            }
            Ty::Tuple(ts) => {
                if lit != LitKind::None || classes & !(class::EQ | class::ELEM) != 0 {
                    return fail(&t, self);
                }
                for c in ts.clone() {
                    self.constrain_part(&c, classes)?;
                }
                Ok(())
            }
            Ty::Record(fs) => {
                if lit != LitKind::None || classes & !(class::EQ | class::ELEM) != 0 {
                    return fail(&t, self);
                }
                for (_, c) in fs.clone() {
                    self.constrain_part(&c, classes)?;
                }
                Ok(())
            }
            Ty::Sum { id, args, .. } => {
                if lit != LitKind::None || classes & !(class::EQ | class::ELEM) != 0 {
                    return fail(&t, self);
                }
                // a `Map` (D158) is three arrays: not an element of an
                // array, no equality (a type parameter is an element)
                if is_map(&self.g.sums, *id) && classes != 0 {
                    return Err(UErr::Constraint(format!(
                        "a Map cannot be {}: it is neither compared, nor stored in an array or a constructor, nor the value of a type parameter",
                        if classes & class::EQ != 0 { "compared" } else { "stored here" }
                    )));
                }
                let key = (*id, args.clone(), classes);
                if self.constraining.contains(&key) {
                    return Ok(());
                }
                let def = &self.g.sums[*id as usize];
                let payloads: Vec<Ty> = (0..def.variants.len()).flat_map(|k| def.payload(k, args)).collect();
                // a recursive type is stored as nodes (D130): an array of it
                // holds their pointers, its payloads are in the nodes, so
                // being an element of an array asks nothing of them (an
                // array in a payload: D335); its equality still does
                let classes = if def.recursive { classes & !class::ELEM } else { classes };
                self.constraining.push(key);
                let mut r = Ok(());
                for c in payloads {
                    r = self.constrain(&c, classes, LitKind::None);
                    if r.is_err() {
                        break;
                    }
                }
                self.constraining.pop();
                r
            }
            Ty::Array(_, e) => {
                // an array is an element of an array when its own elements are
                // scalars or strings (arrays of arrays, D95, D154)
                if lit != LitKind::None || classes & !(class::EQ | class::ELEM) != 0 {
                    return fail(&t, self);
                }
                let e = (**e).clone();
                let inner = if classes & class::ELEM != 0 { (classes & class::EQ) | class::ROW } else { classes };
                self.constrain(&e, inner, LitKind::None)
            }
            // a function can be an element of an array (section 16, v1:
            // D114), nothing more
            Ty::Fn(..) => {
                if lit != LitKind::None || classes & !class::ELEM != 0 {
                    return fail(&t, self);
                }
                Ok(())
            }
        }
    }

    /// A part of a tuple or a record of class `classes`: an array there is a
    /// block of its own (D336), its elements any that an array holds (not
    /// only those of a row), its size its own or not (D341).
    fn constrain_part(&mut self, c: &Ty, classes: Classes) -> std::result::Result<(), UErr> {
        match self.shallow(c) {
            Ty::Array(_, e) if classes & class::ELEM != 0 => self.constrain(&e, (classes & class::EQ) | class::ELEM, LitKind::None),
            _ => self.constrain(c, classes, LitKind::None),
        }
    }

    fn bind(&mut self, v: u32, t: &Ty) -> std::result::Result<(), UErr> {
        if let Ty::Var(w) = t {
            if *w == v {
                return Ok(());
            }
        }
        if self.occurs(v, t) {
            return Err(UErr::Occurs);
        }
        let (classes, lit) = (self.tvars[v as usize].classes, self.tvars[v as usize].lit);
        self.constrain(t, classes, lit)?;
        self.tvars[v as usize].bound = Some(t.clone());
        Ok(())
    }

    fn unify_size(&mut self, a: &Size, b: &Size, mode: Mode, top: bool) -> std::result::Result<(), UErr> {
        let a = self.size_res(a);
        let b = self.size_res(b);
        match (&a, &b) {
            // a size forgotten where each value has its own (D341); never
            // the other way: a size of its own given to a size of the
            // signature would let two arrays of different sizes pass as one
            (Size::Own, _) => Ok(()),
            (_, Size::Own) => Err(UErr::Size),
            (Size::Var(x), Size::Var(y)) if x == y => Ok(()),
            (Size::Var(x), _) => {
                self.svars[*x as usize].0 = Some(b.clone());
                Ok(())
            }
            (_, Size::Var(y)) => {
                self.svars[*y as usize].0 = Some(a.clone());
                Ok(())
            }
            (Size::Poly(p), Size::Poly(q)) => {
                if p == q {
                    Ok(())
                } else if p.sub(q).as_const().is_some() {
                    Err(UErr::Size)
                } else if mode == Mode::Dynamic && top {
                    self.dyn_flag = true;
                    Ok(())
                } else {
                    Err(UErr::Size)
                }
            }
        }
    }

    fn unify_in(&mut self, a: &Ty, b: &Ty, mode: Mode, top: bool) -> std::result::Result<(), UErr> {
        let a = self.shallow(a);
        let b = self.shallow(b);
        match (&a, &b) {
            (Ty::Var(x), _) => self.bind(*x, &b),
            (_, Ty::Var(y)) => self.bind(*y, &a),
            (Ty::Int(k1), Ty::Int(k2)) if k1 == k2 => Ok(()),
            (Ty::F64, Ty::F64) | (Ty::F32, Ty::F32) | (Ty::Bool, Ty::Bool) | (Ty::Unit, Ty::Unit) | (Ty::Str, Ty::Str) | (Ty::BigInt, Ty::BigInt) | (Ty::RegExp, Ty::RegExp) => Ok(()),
            (Ty::Param(p), Ty::Param(q)) if p == q => Ok(()),
            (Ty::Handle(x), Ty::Handle(y)) if x == y => Ok(()),
            // the arrays of a record or a tuple are tested as the array
            // itself (the lowering tests each one, D336)
            (Ty::Tuple(x), Ty::Tuple(y)) if x.len() == y.len() => {
                for (s, t) in x.iter().zip(y.iter()) {
                    self.unify_in(s, t, mode, top)?;
                }
                Ok(())
            }
            (Ty::Sum { id: i, args: x, .. }, Ty::Sum { id: j, args: y, .. }) if i == j => {
                for (s, t) in x.iter().zip(y.iter()) {
                    self.unify_in(s, t, mode, false)?;
                }
                Ok(())
            }
            (Ty::Record(x), Ty::Record(y)) if x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| a.0 == b.0) => {
                for ((_, s), (_, t)) in x.iter().zip(y.iter()) {
                    self.unify_in(s, t, mode, top)?;
                }
                Ok(())
            }
            (Ty::Array(s1, e1), Ty::Array(s2, e2)) => {
                self.unify_in(e1, e2, mode, false)?;
                self.unify_size(s1, s2, mode, top)
            }
            // the parameters the other way: the function found must take what
            // the type expected passes (an array of its own size, D341)
            // an action found only where an action is expected; a function
            // where an action is: a pure function is an action without
            // effect (D352)
            (Ty::Fn(_, _, false), Ty::Fn(_, _, true)) => Err(UErr::Mismatch),
            (Ty::Fn(p1, r1, _), Ty::Fn(p2, r2, _)) if p1.len() == p2.len() => {
                for (s, t) in p1.iter().zip(p2.iter()) {
                    self.unify_in(t, s, Mode::Strict, false)?;
                }
                self.unify_in(r1, r2, Mode::Strict, false)
            }
            _ => Err(UErr::Mismatch),
        }
    }

    /// Unifies the expected type with the type found.
    fn unify(&mut self, expected: &Ty, found: &Ty, span: Span) -> Result<()> {
        self.unify_mode(expected, found, span, Mode::Strict).map(|_| ())
    }

    /// Returns true when a run-time size test is needed (dynamic mode).
    fn unify_mode(&mut self, expected: &Ty, found: &Ty, span: Span, mode: Mode) -> Result<bool> {
        self.dyn_flag = false;
        match self.unify_in(expected, found, mode, true) {
            Ok(()) => Ok(self.dyn_flag),
            Err(UErr::Mismatch) | Err(UErr::Occurs) => err(
                span,
                format!("type mismatch: expected `{}`, found `{}`", self.show(expected), self.show(found)),
            ),
            Err(UErr::Size) => err(
                span,
                format!("size mismatch: expected `{}`, found `{}`", self.show(expected), self.show(found)),
            ),
            Err(UErr::Constraint(m)) => err(span, m),
        }
    }

    fn require(&mut self, t: &Ty, classes: Classes, span: Span) -> Result<()> {
        match self.constrain(t, classes, LitKind::None) {
            Ok(()) => Ok(()),
            Err(UErr::Constraint(m)) => err(span, m),
            Err(_) => err(span, "type error"),
        }
    }

    fn fresh_array(&mut self, span: Span) -> (Ty, Size, Ty) {
        let s = self.fresh_size(span);
        let e = self.fresh(0, LitKind::None, span);
        (Ty::Array(s.clone(), Box::new(e.clone())), s, e)
    }

    /// Ensures `t` is an array and returns its element type.
    fn expect_array(&mut self, t: &Ty, span: Span, what: &str) -> Result<Ty> {
        match self.shallow(t) {
            Ty::Array(_, e) => Ok(*e),
            Ty::Var(_) => {
                let (a, _, e) = self.fresh_array(span);
                self.unify(&a, t, span)?;
                Ok(e)
            }
            other => err(span, format!("{} must be an array, found `{}`", what, self.show(&other))),
        }
    }

    // ----- scopes -------------------------------------------------------

    fn add_local(&mut self, name: &str, ty: Ty, span: Span) -> LocalId {
        let ty = self.open_own(&ty, span);
        self.locals.push(Local { name: name.to_string(), ty, span });
        let id = (self.locals.len() - 1) as LocalId;
        if name != "_" {
            self.scopes.last_mut().unwrap().insert(name.to_string(), id);
        }
        id
    }

    /// The size that an argument gives to a size of the callee: its own,
    /// or a new existential size for an array of its own size (D341).
    fn size_of_call(&mut self, sz: &Size, span: Span) -> Size {
        match self.size_res(sz) {
            Size::Own => {
                let k = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                Size::Poly(Poly::sym(k))
            }
            other => other,
        }
    }

    /// The type parameters of a callee not yet known, bound to the parts of
    /// the argument type `a` where the parameter type `p` names them; the
    /// sizes left to the binding of the call. Only fresh variables are
    /// bound: nothing can fail here.
    fn bind_type_args(&mut self, p: &Ty, a: &Ty, type_args: &[Ty]) {
        match (p, self.shallow(a)) {
            (Ty::Param(i), at) => {
                if let Some(Ty::Var(v)) = type_args.get(*i as usize).map(|t| self.shallow(t)) {
                    if !matches!(at, Ty::Var(w) if w == v) && !self.occurs(v, &at) {
                        let _ = self.bind(v, &at);
                    }
                }
            }
            (Ty::Array(_, pe), Ty::Array(_, ae)) => self.bind_type_args(pe, &ae, type_args),
            (Ty::Tuple(ps), Ty::Tuple(xs)) if ps.len() == xs.len() => {
                for (q, x) in ps.iter().zip(xs.iter()) {
                    self.bind_type_args(q, x, type_args);
                }
            }
            (Ty::Record(pf), Ty::Record(af)) => {
                for (n, q) in pf {
                    if let Some((_, x)) = af.iter().find(|(m, _)| m == n) {
                        self.bind_type_args(q, x, type_args);
                    }
                }
            }
            (Ty::Sum { id: i, args: ps, .. }, Ty::Sum { id: j, args: xs, .. }) if *i == j && ps.len() == xs.len() => {
                for (q, x) in ps.iter().zip(xs.iter()) {
                    self.bind_type_args(q, x, type_args);
                }
            }
            _ => {}
        }
    }

    /// Does `t` hold an array of its own size (D341)?
    fn holds_own(&self, t: &Ty) -> bool {
        match self.shallow(t) {
            Ty::Array(s, e) => self.size_res(&s) == Size::Own || self.holds_own(&e),
            Ty::Tuple(ts) => ts.iter().any(|t| self.holds_own(t)),
            Ty::Record(fs) => fs.iter().any(|(_, t)| self.holds_own(t)),
            Ty::Sum { args, .. } => args.iter().any(|t| self.holds_own(t)),
            Ty::Fn(ps, r, _) => ps.iter().any(|t| self.holds_own(t)) || self.holds_own(&r),
            _ => false,
        }
    }

    /// `t`, an array of its own size (D341) given a new existential size:
    /// what a value read out of a record, a tuple, a payload or a call of a
    /// function type has. Its parts keep theirs, opened when they are read.
    fn open_own(&mut self, t: &Ty, span: Span) -> Ty {
        match self.shallow(t) {
            Ty::Array(s, e) if self.size_res(&s) == Size::Own => {
                let k = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                Ty::Array(Size::Poly(Poly::sym(k)), e)
            }
            other => other,
        }
    }

    fn lookup(&self, name: &str) -> Option<LocalId> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn sym_by_name(&self, name: &str) -> Option<SymId> {
        self.syms
            .iter()
            .position(|s| {
                s.name == name
                    && match &s.origin {
                        SymOrigin::Param(_) | SymOrigin::SizeParam(_) => true,
                        SymOrigin::ParamPath(_, p) => crate::tast::path_has_value(p),
                        _ => false,
                    }
            })
            .map(|i| i as SymId)
    }

    fn can_act(&self) -> bool {
        self.is_action && self.lambda_depth == self.act_lambdas
    }

    /// The local `h` of `isPure(h)` and its type as a pure function, when
    /// it is a function that may act (D386).
    fn pure_test(&self, c: &ast::Expr) -> Option<(LocalId, Ty)> {
        let ExprKind::Call(f, args) = &c.kind else { return None };
        let ExprKind::Var(n) = &f.kind else { return None };
        if n != "isPure" || args.len() != 1 || self.lookup(n).is_some() {
            return None;
        }
        let ExprKind::Var(h) = &args[0].kind else { return None };
        let id = self.lookup(h)?;
        match self.shallow(&self.locals[id as usize].ty) {
            Ty::Fn(ps, r, true) => Some((id, Ty::Fn(ps, r, false))),
            _ => None,
        }
    }

    /// An effect where one is allowed: each enclosing lambda is an action
    /// (D386), all of them up to the action, since the effect is allowed.
    fn note_effect(&mut self) {
        for e in self.act_effects.iter_mut() {
            *e = true;
        }
    }

    // ----- expressions --------------------------------------------------

    fn mk(&self, kind: TExprKind, ty: Ty, span: Span) -> TExpr {
        TExpr { kind, ty, span }
    }

    fn check(&mut self, e: &ast::Expr, expected: Option<&Ty>) -> Result<TExpr> {
        let span = e.span;
        match &e.kind {
            ExprKind::Int { value, neg, suffix } => {
                let v = if *neg { -(*value as i128) } else { *value as i128 };
                let ty = match suffix {
                    None => self.fresh(class::NUM | class::SCALAR, LitKind::Int, span),
                    Some(s) => scalar_ty(*s, span)?,
                };
                if ty == Ty::Bool {
                    return err(span, "invalid literal suffix");
                }
                Ok(self.mk(TExprKind::Lit(Lit::Int(v)), ty, span))
            }
            ExprKind::Float { value, value32, suffix } => {
                let ty = match suffix {
                    None => self.fresh(class::NUM | class::SCALAR, LitKind::Float, span),
                    Some(Scalar::F64) => Ty::F64,
                    Some(Scalar::F32) => Ty::F32,
                    Some(s) => {
                        return err(span, format!("a floating-point literal cannot have the integer type `{}`", s.name()))
                    }
                };
                Ok(self.mk(TExprKind::Lit(Lit::Float(*value, *value32)), ty, span))
            }
            ExprKind::Bool(b) => Ok(self.mk(TExprKind::Lit(Lit::Bool(*b)), Ty::Bool, span)),
            ExprKind::Str(s) => Ok(self.mk(TExprKind::Lit(Lit::Str(s.clone())), Ty::Str, span)),
            ExprKind::BigInt { digits, radix, neg } => {
                let t = self.mk(TExprKind::Lit(Lit::BigInt(digits.clone(), *radix, *neg)), Ty::BigInt, span);
                if let Some(e) = expected {
                    self.unify(e, &Ty::BigInt, span)?;
                }
                Ok(t)
            }
            ExprKind::Unit => Ok(self.mk(TExprKind::Lit(Lit::Unit), Ty::Unit, span)),
            ExprKind::Var(name) => self.check_var(name, span),
            ExprKind::OpFn(op) => {
                let ty = if op.is_logic() {
                    Ty::Fn(vec![Ty::Bool, Ty::Bool], Box::new(Ty::Bool), false)
                } else if op.is_arith() {
                    let a = self.fresh(class::NUM | class::SCALAR, LitKind::None, span);
                    Ty::Fn(vec![a.clone(), a.clone()], Box::new(a), false)
                } else if op.is_bitwise() {
                    let a = self.fresh(class::INTEGRAL | class::SCALAR, LitKind::None, span);
                    Ty::Fn(vec![a.clone(), a.clone()], Box::new(a), false)
                } else {
                    let c = if op.is_order() { class::ORD | class::SCALAR } else { class::EQ };
                    let a = self.fresh(c, LitKind::None, span);
                    Ty::Fn(vec![a.clone(), a], Box::new(Ty::Bool), false)
                };
                Ok(self.mk(TExprKind::OpRef(*op), ty, span))
            }
            ExprKind::Tuple(es) => {
                let hints: Vec<Option<Ty>> = match expected.map(|t| self.shallow(t)) {
                    Some(Ty::Tuple(ts)) if ts.len() == es.len() => ts.into_iter().map(Some).collect(),
                    _ => vec![None; es.len()],
                };
                let mut out = Vec::new();
                for (e, h) in es.iter().zip(hints.iter()) {
                    out.push(self.check(e, h.as_ref())?);
                }
                let ty = Ty::Tuple(out.iter().map(|e| e.ty.clone()).collect());
                Ok(self.mk(TExprKind::Tuple(out), ty, span))
            }
            // `[a, b]` where a tuple is expected: a tuple (D148)
            ExprKind::Array(es)
                if matches!(expected.map(|t| self.shallow(t)), Some(Ty::Tuple(ts)) if ts.len() == es.len()) =>
            {
                let t = ast::Expr { kind: ExprKind::Tuple(es.clone()), span };
                self.check(&t, expected)
            }
            ExprKind::Array(es) => {
                // elements of arrays of their own size (D341): the element
                // type expected, so that each element keeps its own sizes
                let elem = match expected.map(|t| self.shallow(t)) {
                    Some(Ty::Array(_, e)) if self.holds_own(&e) => *e,
                    _ => self.fresh(class::ELEM, LitKind::None, span),
                };
                let mut out = Vec::new();
                for x in es {
                    let t = self.check(x, Some(&elem))?;
                    self.unify(&elem, &t.ty, x.span)?;
                    out.push(t);
                }
                let ty = Ty::Array(Size::Poly(Poly::constant(es.len() as i64)), Box::new(elem));
                Ok(self.mk(TExprKind::Array(out), ty, span))
            }
            ExprKind::Record(fields) => {
                let hints: Vec<(String, Ty)> = match expected.map(|t| self.shallow(t)) {
                    Some(Ty::Record(fs)) => fs,
                    _ => Vec::new(),
                };
                let mut out: Vec<(String, TExpr)> = Vec::new();
                for (n, x) in fields {
                    if out.iter().any(|(m, _)| m == n) {
                        return err(x.span, format!("the field `{}` is given twice", n));
                    }
                    let h = hints.iter().find(|(m, _)| m == n).map(|(_, t)| t.clone());
                    out.push((n.clone(), self.check(x, h.as_ref())?));
                }
                // the fields are values in the order of their names; they
                // are evaluated in the order of the program, as TypeScript
                // does, so that `{ x: f(v), v: v }` reads `v` before it is
                // moved into its field (D164)
                let in_order = out.windows(2).all(|w| w[0].0 < w[1].0);
                let mut stmts = Vec::new();
                if !in_order {
                    for (n, x) in out.iter_mut() {
                        if matches!(x.kind, TExprKind::Local(_) | TExprKind::Lit(_)) {
                            continue;
                        }
                        let (ty, sp) = (x.ty.clone(), x.span);
                        let l = self.add_local(&format!("${}", n), ty.clone(), sp);
                        let value = std::mem::replace(x, self.mk(TExprKind::Local(l), ty, sp));
                        stmts.push(TStmt::Let(TPat::Bind(l), value));
                    }
                }
                out.sort_by(|a, b| a.0.cmp(&b.0));
                let ty = Ty::Record(out.iter().map(|(n, x)| (n.clone(), x.ty.clone())).collect());
                let tuple = self.mk(TExprKind::Tuple(out.into_iter().map(|(_, x)| x).collect()), ty.clone(), span);
                if stmts.is_empty() {
                    return Ok(tuple);
                }
                Ok(self.mk(TExprKind::Block(stmts, Box::new(tuple)), ty, span))
            }
            ExprKind::Field(x, name) => {
                let x = self.check(x, None)?;
                // `m.size` of a `Map` (D158): the number of its keys
                if name == "size" && self.map_args(&x.ty).is_some() {
                    let call = TExprKind::Call { callee: Callee::Prim(Prim::MapSize), args: vec![x], size_checks: Vec::new() };
                    return Ok(self.mk(call, Ty::Int(IntKind::I64), span));
                }
                let (i, ty) = self.field_of(&x.ty, name, x.span)?;
                let ty = self.open_own(&ty, span);
                Ok(self.mk(TExprKind::Field(Box::new(x), i), ty, span))
            }
            ExprKind::Slice(x0, a0, b0) => {
                // `[b - a]T`: a size known at run time, like the result of
                // `readF64`; the bounds are tested then (a panic outside)
                let x = self.check(x0, None)?;
                // the bytes of a string (D155)
                if self.shallow(&x.ty) == Ty::Str {
                    let i64t = Ty::Int(IntKind::I64);
                    let a = self.check(a0, Some(&i64t))?;
                    self.unify(&i64t, &a.ty, a.span)?;
                    let b = self.check(b0, Some(&i64t))?;
                    self.unify(&i64t, &b.ty, b.span)?;
                    return Ok(self.mk(
                        TExprKind::Call { callee: Callee::Prim(Prim::StrSlice), args: vec![x, a, b], size_checks: Vec::new() },
                        Ty::Str,
                        span,
                    ));
                }
                let (a, b) = (a0, b0);
                let elem = self.expect_array(&x.ty, x.span, "a sliced value")?;
                let a = self.check(a, None)?;
                self.require(&a.ty, class::INTEGRAL | class::SCALAR, a.span)?;
                let b = self.check(b, None)?;
                self.require(&b.ty, class::INTEGRAL | class::SCALAR, b.span)?;
                let s = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                let ty = Ty::Array(Size::Poly(Poly::sym(s)), Box::new(elem));
                Ok(self.mk(TExprKind::Slice(Box::new(x), Box::new(a), Box::new(b)), ty, span))
            }
            ExprKind::Match(scrutinee, arms) => {
                let x = self.check(scrutinee, None)?;
                // each arm given the form expected, not its sizes: arms of
                // arrays of other sizes have a size fixed at run time (D337)
                let mut ty: Option<Ty> = None;
                let mut out = Vec::new();
                for (p, body) in arms.iter() {
                    self.scopes.push(HashMap::new());
                    let tp = self.bind_pattern_in(p, &x.ty, true)?;
                    let hint = match (&ty, expected) {
                        (Some(t), _) => Some(self.loosen(t)),
                        (None, Some(e)) => Some(self.loosen(e)),
                        (None, None) => None,
                    };
                    let b = self.check(body, hint.as_ref())?;
                    ty = Some(match ty {
                        None => b.ty.clone(),
                        Some(t) => self.join(&t, &b.ty, body.span)?,
                    });
                    self.scopes.pop();
                    out.push((tp, b));
                }
                let ty = match ty {
                    Some(t) => t,
                    None => self.fresh(0, LitKind::None, span),
                };
                if let Some(e) = expected {
                    self.unify(e, &ty, span)?;
                }
                self.exhaustive(&x.ty, &out.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(), span)?;
                Ok(self.mk(TExprKind::Match(Box::new(x), out), ty, span))
            }
            ExprKind::Call(f, args) => self.check_call(f, args, span, expected),
            ExprKind::TypeApp(..) => err(span, "type arguments are given to a function that is called: `f<T>(x)`"),
            ExprKind::Index(a, i) => {
                let a = self.check(a, None)?;
                let elem = self.expect_array(&a.ty, a.span, "an indexed value")?;
                let i = self.check(i, None)?;
                self.require(&i.ty, class::INTEGRAL | class::SCALAR, i.span)?;
                Ok(self.mk(TExprKind::Index(Box::new(a), Box::new(i)), elem, span))
            }
            ExprKind::Update { target, access, value, require_unique } => {
                let idx = match access {
                    ast::Access::Index(i) => i,
                    ast::Access::Field(name) => {
                        if *require_unique {
                            return err(span, "`requireUnique` applies to an array update `e.with(i, v)`");
                        }
                        let t = self.check(target, None)?;
                        let (i, fty) = self.field_of(&t.ty, name, t.span)?;
                        let v = self.check(value, Some(&fty))?;
                        self.unify(&fty, &v.ty, v.span)?;
                        let ty = t.ty.clone();
                        return Ok(self.mk(TExprKind::FieldUpdate { target: Box::new(t), index: i, value: Box::new(v) }, ty, span));
                    }
                };
                let t = self.check(target, None)?;
                let elem = self.expect_array(&t.ty, t.span, "the target of `.with(i, v)`")?;
                let i = self.check(idx, None)?;
                self.require(&i.ty, class::INTEGRAL | class::SCALAR, i.span)?;
                let v = self.check(value, Some(&elem))?;
                self.unify(&elem, &v.ty, v.span)?;
                let ty = t.ty.clone();
                Ok(self.mk(
                    TExprKind::Update {
                        target: Box::new(t),
                        index: Box::new(i),
                        value: Box::new(v),
                        require_unique: *require_unique,
                    },
                    ty,
                    span,
                ))
            }
            ExprKind::Unary(op, x) => {
                let x = self.check(x, expected)?;
                match op {
                    UnOp::Not => self.unify(&Ty::Bool, &x.ty, x.span)?,
                    UnOp::Neg => {
                        let t = self.shallow(&x.ty);
                        match t {
                            Ty::Array(_, e) => self.require(&e, class::NUM | class::SCALAR, x.span)?,
                            _ => {
                                self.require(&t, class::NUM | class::SCALAR, x.span)?;
                                self.deferred_scalar.push((x.ty.clone(), x.span));
                            }
                        }
                    }
                }
                let ty = x.ty.clone();
                Ok(self.mk(TExprKind::Unary(*op, Box::new(x)), ty, span))
            }
            ExprKind::Binary(op, l, r) => self.check_binary(*op, l, r, span),
            ExprKind::Pow(b, x) => {
                let float_exp = matches!(x.kind, ExprKind::Float { .. });
                let b = self.check(b, expected)?;
                let x = if float_exp { self.check(x, Some(&b.ty.clone()))? } else { self.check(x, None)? };
                // a floating-point exponent: `pow(x, y)` (SPEC section 10)
                if float_exp || matches!(self.shallow(&x.ty), Ty::F64 | Ty::F32) {
                    self.require(&b.ty, class::FLOAT | class::SCALAR, b.span)?;
                    self.unify(&b.ty, &x.ty, x.span)?;
                    let ty = b.ty.clone();
                    let callee = Callee::Prim(Prim::Math(MathFn::Pow));
                    return Ok(self.mk(TExprKind::Call { callee, args: vec![b, x], size_checks: Vec::new() }, ty, span));
                }
                self.require(&x.ty, class::INTEGRAL | class::SCALAR, x.span)?;
                match self.shallow(&b.ty) {
                    Ty::Array(_, e) => self.require(&e, class::NUM | class::SCALAR, b.span)?,
                    t => {
                        self.require(&t, class::NUM | class::SCALAR, b.span)?;
                        self.deferred_scalar.push((b.ty.clone(), b.span));
                    }
                }
                let ty = b.ty.clone();
                Ok(self.mk(TExprKind::Pow(Box::new(b), Box::new(x)), ty, span))
            }
            ExprKind::If(c, t, f) => {
                // `if (isPure(h))`, `h` a parameter or a local that may act:
                // a pure function in the first branch (D386)
                let narrowed = self.pure_test(c);
                let c = self.check(c, Some(&Ty::Bool))?;
                self.unify(&Ty::Bool, &c.ty, c.span)?;
                // each branch given the form expected, not its sizes: arrays
                // of two sizes have a size fixed at run time (D337)
                let hint = expected.map(|e| self.loosen(e));
                let saved = narrowed.map(|(id, pure)| (id, std::mem::replace(&mut self.locals[id as usize].ty, pure)));
                let t = self.check(t, hint.as_ref());
                if let Some((id, ty)) = saved {
                    self.locals[id as usize].ty = ty;
                }
                let t = t?;
                // the form of the first branch, or the form expected when it
                // holds arrays of their own size (D341), which the first
                // branch may have of a size
                let hint = match hint {
                    Some(h) if self.holds_own(&h) => h,
                    _ => self.loosen(&t.ty),
                };
                let f = self.check(f, Some(&hint))?;
                let ty = self.join(&t.ty, &f.ty, f.span)?;
                Ok(self.mk(TExprKind::If(Box::new(c), Box::new(t), Box::new(f)), ty, span))
            }
            ExprKind::Lambda(params, body) => {
                let hint = expected.map(|t| self.shallow(t));
                let (ptys, rhint, act) = match hint {
                    Some(Ty::Fn(ps, r, act)) if ps.len() == params.len() => (ps, Some(*r), act && self.can_act()),
                    _ => (params.iter().map(|(_, sp)| self.fresh(0, LitKind::None, *sp)).collect(), None, false),
                };
                self.scopes.push(HashMap::new());
                let mut ids = Vec::new();
                for ((name, sp), t) in params.iter().zip(ptys.iter()) {
                    if name != "_" && params.iter().filter(|(n, _)| n == name).count() > 1 {
                        return err(*sp, format!("duplicate lambda parameter `{}`", name));
                    }
                    ids.push(self.add_local(name, t.clone(), *sp));
                }
                self.lambda_depth += 1;
                self.act_lambdas += act as u32;
                if act {
                    self.act_effects.push(false);
                }
                let b = self.check(body, rhint.as_ref());
                // a lambda where an action is expected is an action only
                // when its body has an effect: else it is pure (D386)
                let acted = act && self.act_effects.pop().unwrap_or(true);
                self.act_lambdas -= act as u32;
                self.lambda_depth -= 1;
                self.scopes.pop();
                let b = b?;
                if let Some(r) = &rhint {
                    self.unify(r, &b.ty, b.span)?;
                }
                let ty = Ty::Fn(ptys, Box::new(b.ty.clone()), acted);
                Ok(self.mk(TExprKind::Lambda(ids, Box::new(b)), ty, span))
            }
            ExprKind::Block(stmts, result) => {
                self.scopes.push(HashMap::new());
                let r = self.check_block(stmts, result, expected, span);
                self.scopes.pop();
                r
            }
        }
    }

    fn check_block(
        &mut self,
        stmts: &[ast::Stmt],
        result: &ast::Expr,
        expected: Option<&Ty>,
        span: Span,
    ) -> Result<TExpr> {
        let mut out = Vec::new();
        for s in stmts {
            match s {
                ast::Stmt::Let(pat, ann, value, _) => {
                    let ann = match ann {
                        None => None,
                        Some(t) => Some(self.local_type(t)?),
                    };
                    let v = self.check(value, ann.as_ref())?;
                    if let Some(a) = &ann {
                        self.unify(a, &v.ty, value.span)?;
                    }
                    let p = self.bind_pattern(pat, &v.ty)?;
                    out.push(TStmt::Let(p, v));
                }
                ast::Stmt::Expr(e) => {
                    if !self.can_act() {
                        // the action called, when the statement is a call of one
                        let called = match &e.kind {
                            ExprKind::Call(f, _) => match &f.kind {
                                ExprKind::Var(n) if prim_by_name(n).is_some_and(|p| p.is_action()) || self.g.names.get(n).is_some_and(|&id| self.g.sigs[id].is_action) => {
                                    format!(" (`{}` is one)", n)
                                }
                                _ => String::new(),
                            },
                            _ => String::new(),
                        };
                        let why = if self.is_action {
                            "this lambda is pure, as the functions given to `fold`, `filter` and the other functions of arrays are, but `map` and `mapParallel` (a lambda is an action only where an `action (T) => U` is expected, SPEC section 12)"
                        } else {
                            "a `function` computes a value and has no effect; effects go in an `action` (a handler with effects: an action given to `http.serve`, D352, D386)"
                        };
                        return err(e.span, format!("an expression statement is only allowed in an `action`{}: {}", called, why));
                    }
                    self.note_effect();
                    let v = self.check(e, None)?;
                    out.push(TStmt::Expr(v));
                }
            }
        }
        let r = self.check(result, expected)?;
        let ty = r.ty.clone();
        Ok(self.mk(TExprKind::Block(out, Box::new(r)), ty, span))
    }

    /// A type annotation inside a body: may use the function's type
    /// parameters and size variables, not introduce new ones.
    fn local_type(&mut self, t: &ast::TypeExpr) -> Result<Ty> {
        let mut syms: Vec<SymInfo> = self.syms.clone();
        let generics = self.generics.clone();
        let n = syms.len();
        let ty = type_from_ast(t, &mut SigCtx { generics: &generics, sums: &self.g.heads, syms: &mut syms, allow_new: false, result: false, module: self.module.as_deref(), imports: self.g.imports.get(&self.file) })?;
        debug_assert_eq!(n, syms.len());
        // `const x: T[] = ...`: any size (D341)
        Ok(match ty {
            Ty::Array(Size::Own, e) => Ty::Array(self.fresh_size(t.span), e),
            ty => ty,
        })
    }

    /// The index and type of field `name` of a record type, which must be
    /// known at this point (records are structural, SPEC section 4).
    /// The type `Map<k, v>` of the prelude (D158).
    fn map_ty(&self, k: Ty, v: Ty, span: Span) -> Result<Ty> {
        match self.g.sums.iter().position(|d| d.variants.len() == 1 && d.variants[0].name == crate::MAP_CTOR) {
            Some(id) => Ok(Ty::Sum { id: id as u32, name: "Map".into(), args: vec![k, v] }),
            None => err(span, "the built-in `Map` is replaced by the type `Map` of the program"),
        }
    }

    /// `Some((k, v))` if `t` is `Map<k, v>` of the prelude.
    fn map_args(&self, t: &Ty) -> Option<(Ty, Ty)> {
        match self.shallow(t) {
            Ty::Sum { id, args, .. } if is_map(&self.g.sums, id) => Some((args[0].clone(), args[1].clone())),
            _ => None,
        }
    }

    fn field_of(&mut self, t: &Ty, name: &str, span: Span) -> Result<(u32, Ty)> {
        match self.shallow(t) {
            Ty::Record(fs) => match fs.iter().position(|(n, _)| n == name) {
                Some(i) => Ok((i as u32, fs[i].1.clone())),
                None => err(span, format!("this record has no field `{}`: its type is `{}`", name, self.show(&Ty::Record(fs)))),
            },
            Ty::Var(_) => err(span, format!("the type of this value must be known to use its field `{}`; add a type annotation", name)),
            other => err(span, format!("`.{}` needs a record, found `{}`", name, self.show(&other))),
        }
    }

    fn bind_pattern(&mut self, p: &ast::Pattern, ty: &Ty) -> Result<TPat> {
        let tp = self.bind_pattern_in(p, ty, false)?;
        if !self.irrefutable(&tp) {
            return err(p.span, "this pattern may not match: `const` needs a pattern that always matches; use `match`");
        }
        Ok(tp)
    }

    /// Does a pattern match every value of its type?
    fn irrefutable(&self, p: &TPat) -> bool {
        match p {
            TPat::Wild(_) | TPat::Bind(_) => true,
            TPat::Tuple(ps, _) => ps.iter().all(|q| self.irrefutable(q)),
            TPat::Lit(..) => false,
            TPat::Ctor { sum, subs, .. } => {
                self.g.sums[*sum as usize].variants.len() == 1 && subs.iter().all(|q| self.irrefutable(q))
            }
        }
    }

    /// The arms of a `match` must cover every value: an arm that always
    /// matches, or, for a sum type, an arm per constructor whose payload
    /// patterns always match, or, for `bool`, both `true` and `false`.
    fn exhaustive(&mut self, ty: &Ty, pats: &[TPat], span: Span) -> Result<()> {
        if pats.iter().any(|p| self.irrefutable(p)) {
            return Ok(());
        }
        match self.shallow(ty) {
            Ty::Sum { id, .. } => {
                let def = &self.g.sums[id as usize];
                let missing: Vec<String> = (0..def.variants.len())
                    .filter(|k| {
                        !pats.iter().any(|p| match p {
                            TPat::Ctor { variant, subs, .. } => {
                                *variant as usize == *k && subs.iter().all(|q| self.irrefutable(q))
                            }
                            _ => false,
                        })
                    })
                    .map(|k| def.variants[k].name.clone())
                    .collect();
                if missing.is_empty() {
                    return Ok(());
                }
                err(span, format!("this `match` does not cover the constructor(s) {}; add an arm or `_ => ...`", missing.join(", ")))
            }
            Ty::Bool => {
                let has = |b: bool| pats.iter().any(|p| matches!(p, TPat::Lit(Lit::Bool(x), _) if *x == b));
                if has(true) && has(false) {
                    return Ok(());
                }
                err(span, "this `match` does not cover both `true` and `false`; add an arm or `_ => ...`")
            }
            _ => err(span, "this `match` may not cover every value; add a last arm `_ => ...`"),
        }
    }

    fn bind_pattern_in(&mut self, p: &ast::Pattern, ty: &Ty, refutable: bool) -> Result<TPat> {
        match &p.kind {
            PatternKind::Wild => Ok(TPat::Wild(ty.clone())),
            PatternKind::Var(name) if self.g.ctors.contains_key(&self.q(name)) => {
                // a constructor without payload: `None -> ...`
                self.bind_pattern_in(&ast::Pattern { kind: PatternKind::Ctor(self.q(name), Vec::new()), span: p.span }, ty, refutable)
            }
            PatternKind::Var(name) => Ok(TPat::Bind(self.add_local(name, ty.clone(), p.span))),
            PatternKind::Tuple(ps) => {
                let comps: Vec<Ty> = match self.shallow(ty) {
                    Ty::Tuple(ts) if ts.len() == ps.len() => ts,
                    Ty::Var(_) => {
                        let ts: Vec<Ty> = ps.iter().map(|q| self.fresh(0, LitKind::None, q.span)).collect();
                        self.unify(&Ty::Tuple(ts.clone()), ty, p.span)?;
                        ts
                    }
                    other => {
                        return err(
                            p.span,
                            format!("this tuple pattern has {} elements but the value has type `{}`", ps.len(), self.show(&other)),
                        )
                    }
                };
                let mut out = Vec::new();
                for (q, t) in ps.iter().zip(comps.iter()) {
                    out.push(self.bind_pattern_in(q, t, refutable)?);
                }
                Ok(TPat::Tuple(out, ty.clone()))
            }
            PatternKind::Unit => {
                self.unify(&Ty::Unit, ty, p.span)?;
                Ok(TPat::Wild(Ty::Unit))
            }
            // `{price: p, volume: v}`: the fields not named are ignored
            PatternKind::Record(fields) => {
                let fs = match self.shallow(ty) {
                    Ty::Record(fs) => fs,
                    other => {
                        return err(p.span, format!("a record pattern needs a record whose type is known, found `{}`", self.show(&other)))
                    }
                };
                let mut subs: Vec<TPat> = fs.iter().map(|(_, t)| TPat::Wild(t.clone())).collect();
                for (n, q) in fields {
                    match fs.iter().position(|(m, _)| m == n) {
                        Some(i) => subs[i] = self.bind_pattern_in(q, &fs[i].1, refutable)?,
                        None => return err(q.span, format!("the record has no field `{}`", n)),
                    }
                }
                Ok(TPat::Tuple(subs, ty.clone()))
            }
            PatternKind::Int { value, neg } => {
                let _ = refutable;
                let v = if *neg { -(*value as i128) } else { *value as i128 };
                let lt = self.fresh(class::NUM | class::SCALAR | class::EQ, LitKind::Int, p.span);
                self.unify(&lt, ty, p.span)?;
                Ok(TPat::Lit(Lit::Int(v), ty.clone()))
            }
            PatternKind::Float(f, f32v) => {
                let lt = self.fresh(class::NUM | class::SCALAR | class::EQ, LitKind::Float, p.span);
                self.unify(&lt, ty, p.span)?;
                Ok(TPat::Lit(Lit::Float(*f, *f32v), ty.clone()))
            }
            PatternKind::Bool(b) => {
                self.unify(&Ty::Bool, ty, p.span)?;
                Ok(TPat::Lit(Lit::Bool(*b), ty.clone()))
            }
            PatternKind::Ctor(name, ps) => {
                let g = self.g;
                let name = &self.q(name);
                let (sum, variant) = match g.ctors.get(name) {
                    Some(c) => *c,
                    None => return err(p.span, format!("unknown constructor `{}`", name)),
                };
                let def = &g.sums[sum as usize];
                let targs: Vec<Ty> = def.params.iter().map(|_| self.fresh(0, LitKind::None, p.span)).collect();
                let sty = Ty::Sum { id: sum, name: def.name.clone(), args: targs.clone() };
                self.unify(&sty, ty, p.span)?;
                // the sizes of the payload taken apart: new existential ones
                let nsizes = def.variants[variant as usize].sizes;
                let sizes: Vec<Size> = (0..nsizes)
                    .map(|_| {
                        let s = self.syms.len() as SymId;
                        self.syms.push(SymInfo { name: format!("k@{}:{}", p.span.line, p.span.col), origin: SymOrigin::Existential(p.span) });
                        Size::Poly(Poly::sym(s))
                    })
                    .collect();
                let payload = payload_sized(def, variant as usize, &targs, &sizes);
                if payload.len() != ps.len() {
                    return err(
                        p.span,
                        format!("the constructor `{}` has {} field(s), but the pattern has {}", name, payload.len(), ps.len()),
                    );
                }
                let mut subs = Vec::new();
                for (q, t) in ps.iter().zip(payload.iter()) {
                    subs.push(self.bind_pattern_in(q, t, refutable)?);
                }
                Ok(TPat::Ctor { sum, variant, subs, ty: ty.clone() })
            }
        }
    }

    /// A global name seen from the function's module (D139): `m.x` when the
    /// module declares `x`, else `x`.
    fn q(&self, name: &str) -> String {
        if name.contains('.') {
            return name.to_string();
        }
        if let Some(m) = &self.module {
            let qn = format!("{}.{}", m, name);
            if self.g.names.contains_key(&qn) || self.g.ctors.contains_key(&qn) {
                return qn;
            }
            // in a module of the standard library (or the prelude kept for
            // it), a function of the prelude that the program declares
            // again keeps its meaning (D337)
            if m.starts_with(crate::modules::STD_KEY) {
                let pn = format!("{}{}", PRELUDE_KEPT, name);
                if self.g.names.contains_key(&pn) {
                    return pn;
                }
            }
        }
        if let Some(qn) = self.g.imports.get(&self.file).and_then(|i| i.get(name)) {
            return qn.clone();
        }
        name.to_string()
    }

    fn check_var(&mut self, name: &str, span: Span) -> Result<TExpr> {
        if let Some(l) = self.lookup(name) {
            let ty = self.locals[l as usize].ty.clone();
            return Ok(self.mk(TExprKind::Local(l), ty, span));
        }
        if let Some(s) = self.sym_by_name(name) {
            return Ok(self.mk(TExprKind::SizeVal(Poly::sym(s)), Ty::Int(IntKind::I64), span));
        }
        let qn = self.q(name);
        let name = qn.as_str();
        if self.g.ctors.contains_key(name) {
            return self.check_construct(name, &[], span, None);
        }
        if let Some(&id) = self.g.names.get(name) {
            let sig = &self.g.sigs[id];
            // a constant: its value
            if sig.is_const {
                return self.check_call_fn(id, &[], span);
            }
            if id >= self.g.first_extern {
                return err(span, format!("the extern function `{}` cannot be used as a value", name));
            }
            if sig.is_action && !self.can_act() {
                return err(
                    span,
                    format!("the action `{}` is a value only inside an `action` (a `function` has no effect, SPEC section 12)", name),
                );
            }
            if sig.is_action {
                self.note_effect();
            }
            let (type_args, ptys, ret) = self.instantiate_value(id, span);
            let ty = Ty::Fn(ptys, Box::new(ret), sig.is_action);
            return Ok(self.mk(TExprKind::FnRef { id, type_args }, ty, span));
        }
        if let Some(p) = prim_by_name(name) {
            return match p {
                Prim::Sqrt => {
                    let a = self.fresh(class::FLOAT | class::SCALAR, LitKind::None, span);
                    Ok(self.mk(TExprKind::PrimRef(p), Ty::Fn(vec![a.clone()], Box::new(a), false), span))
                }
                Prim::Math(f) => {
                    let c = if f == MathFn::Abs { class::NUM } else { class::FLOAT };
                    let a = self.fresh(c | class::SCALAR, LitKind::None, span);
                    Ok(self.mk(TExprKind::PrimRef(p), Ty::Fn(vec![a.clone(); f.arity()], Box::new(a), false), span))
                }
                Prim::Convert(t) => {
                    let a = self.fresh(class::NUM | class::SCALAR, LitKind::None, span);
                    Ok(self.mk(TExprKind::PrimRef(p), Ty::Fn(vec![a], Box::new(t.to_ty()), false), span))
                }
                // the functions of strings whose result has no size of its
                // own (D135): `map(xs, trim)`, `reduce(xs, (++), "")`
                Prim::Trim | Prim::ToUpper | Prim::ToLower | Prim::TrimStart | Prim::TrimEnd => {
                    Ok(self.mk(TExprKind::PrimRef(p), Ty::Fn(vec![Ty::Str], Box::new(Ty::Str), false), span))
                }
                Prim::ParseI64 => Ok(self.mk(TExprKind::PrimRef(p), Ty::Fn(vec![Ty::Str], Box::new(Ty::Int(IntKind::I64)), false), span)),
                Prim::ParseF64 | Prim::ParseFloat | Prim::JsNumber => Ok(self.mk(TExprKind::PrimRef(p), Ty::Fn(vec![Ty::Str], Box::new(Ty::F64), false), span)),
                Prim::StrLen => Ok(self.mk(TExprKind::PrimRef(p), Ty::Fn(vec![Ty::Str], Box::new(Ty::Int(IntKind::I64)), false), span)),
                Prim::Concat => Ok(self.mk(TExprKind::PrimRef(p), Ty::Fn(vec![Ty::Str, Ty::Str], Box::new(Ty::Str), false), span)),
                Prim::ToString => {
                    let a = self.fresh(class::SCALAR, LitKind::None, span);
                    Ok(self.mk(TExprKind::PrimRef(p), Ty::Fn(vec![a], Box::new(Ty::Str), false), span))
                }
                Prim::StringOf => {
                    let s = self.fresh_size(span);
                    let b = Ty::Array(s, Box::new(Ty::Int(IntKind::U8)));
                    Ok(self.mk(TExprKind::PrimRef(p), Ty::Fn(vec![b], Box::new(Ty::Str), false), span))
                }
                Prim::Join => {
                    let s = self.fresh_size(span);
                    let xs = Ty::Array(s, Box::new(Ty::Str));
                    Ok(self.mk(TExprKind::PrimRef(p), Ty::Fn(vec![xs, Ty::Str], Box::new(Ty::Str), false), span))
                }
                // as a value, the function of two numbers (`reduce(x, min, e)`)
                Prim::Min | Prim::Max => {
                    let a = self.fresh(class::NUM | class::SCALAR, LitKind::None, span);
                    Ok(self.mk(TExprKind::PrimRef(p), Ty::Fn(vec![a.clone(), a.clone()], Box::new(a), false), span))
                }
                _ => err(span, format!("the primitive `{}` must be called directly", name)),
            };
        }
        if NOT_IN_POC.contains(&name) {
            return Err(poc(span, &format!("`{}`", name)));
        }
        if self.syms.iter().any(|s| s.name == name && matches!(&s.origin, SymOrigin::ParamPath(_, p) if !crate::tast::path_has_value(p))) {
            return err(span, format!("the size `{}` of the elements of an array of a parameter is known to the types only: read it from an element (`x[0].length`)", name));
        }
        err(span, format!("unknown variable `{}`", name))
    }

    /// Instantiates a function used as a value: fresh type variables for its
    /// type parameters and fresh size variables for its size variables.
    fn instantiate_value(&mut self, id: FnId, span: Span) -> (Vec<Ty>, Vec<Ty>, Ty) {
        let sig = &self.g.sigs[id];
        let gens: Vec<Classes> = sig.generics.iter().map(|g| g.1).collect();
        let nsyms = sig.syms.len();
        let type_args: Vec<Ty> = gens.iter().map(|c| self.fresh(*c, LitKind::None, span)).collect();
        let sizes: Vec<Size> = (0..nsyms).map(|_| self.fresh_size(span)).collect();
        let sig = &self.g.sigs[id];
        let ptys: Vec<Ty> = sig.param_tys.iter().map(|t| subst_value(t, &type_args, &sizes)).collect();
        let ret = subst_value(&sig.ret, &type_args, &sizes);
        (type_args, ptys, ret)
    }

    fn check_binary(&mut self, op: BinOp, l: &ast::Expr, r: &ast::Expr, span: Span) -> Result<TExpr> {
        let lhs = self.check(l, None)?;
        let rhs = self.check(r, None)?;
        let lt = self.shallow(&lhs.ty);
        let rt = self.shallow(&rhs.ty);
        // `a + b` of strings: their concatenation (D148)
        if op == BinOp::Add && (lt == Ty::Str || rt == Ty::Str) {
            self.unify(&Ty::Str, &lhs.ty, lhs.span)?;
            self.unify(&Ty::Str, &rhs.ty, rhs.span)?;
            let args = vec![lhs, rhs];
            return Ok(self.mk(TExprKind::Call { callee: Callee::Prim(Prim::Concat), args, size_checks: Vec::new() }, Ty::Str, span));
        }
        let mut size_check = false;
        let ty = if op.is_logic() {
            self.unify(&Ty::Bool, &lhs.ty, lhs.span)?;
            self.unify(&Ty::Bool, &rhs.ty, rhs.span)?;
            Ty::Bool
        } else if op.is_equality() {
            match (&lt, &rt) {
                (Ty::Array(_, e1), Ty::Array(_, e2)) => {
                    // Value equality: arrays of different sizes are unequal.
                    self.unify(e1, e2, rhs.span)?;
                }
                _ => self.unify(&lhs.ty, &rhs.ty, rhs.span)?,
            }
            self.require(&lhs.ty, class::EQ, lhs.span)?;
            Ty::Bool
        } else if op.is_order() {
            if lt.is_array() || rt.is_array() {
                return err(span, format!("`{}` cannot compare arrays; only `==` and `!=` can", op.text()));
            }
            self.unify(&lhs.ty, &rhs.ty, rhs.span)?;
            // strings: ordered by bytes (D134); a type parameter of class
            // `Ord`: a scalar or a string, known in each instance (D155)
            match self.shallow(&lhs.ty) {
                Ty::Str | Ty::BigInt => {}
                Ty::Param(_) => self.require(&lhs.ty, class::ORD, lhs.span)?,
                _ => {
                    self.require(&lhs.ty, class::ORD | class::SCALAR, lhs.span)?;
                    self.deferred_scalar.push((lhs.ty.clone(), lhs.span));
                }
            }
            Ty::Bool
        } else {
            // arithmetic, possibly element-wise; the bitwise operators on
            // integers only (D136)
            let cls = if op.is_bitwise() { class::INTEGRAL | class::SCALAR } else { class::NUM | class::SCALAR };
            match (lt.is_array(), rt.is_array()) {
                (true, true) => {
                    size_check = self.unify_mode(&lhs.ty, &rhs.ty, rhs.span, Mode::Dynamic)?;
                    let e = lt.elem().unwrap().clone();
                    self.require(&e, cls, lhs.span)?;
                    lhs.ty.clone()
                }
                (true, false) => {
                    let e = lt.elem().unwrap().clone();
                    self.unify(&e, &rhs.ty, rhs.span)?;
                    self.require(&e, cls, lhs.span)?;
                    lhs.ty.clone()
                }
                (false, true) => {
                    let e = rt.elem().unwrap().clone();
                    self.unify(&e, &lhs.ty, lhs.span)?;
                    self.require(&e, cls, rhs.span)?;
                    rhs.ty.clone()
                }
                (false, false) => {
                    self.unify(&lhs.ty, &rhs.ty, rhs.span)?;
                    self.require(&lhs.ty, cls, lhs.span)?;
                    self.deferred_scalar.push((lhs.ty.clone(), lhs.span));
                    lhs.ty.clone()
                }
            }
        };
        Ok(self.mk(TExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), size_check }, ty, span))
    }

    fn arity(&self, name: &str, want: usize, got: usize, span: Span) -> Result<()> {
        if want != got {
            return err(
                span,
                format!("`{}` takes {} argument{}, but {} were given", name, want, if want == 1 { "" } else { "s" }, got),
            );
        }
        Ok(())
    }

    fn check_call(
        &mut self,
        f: &ast::Expr,
        args: &[ast::Expr],
        span: Span,
        expected: Option<&Ty>,
    ) -> Result<TExpr> {
        let method_call = std::mem::take(&mut self.method_call);
        // `x.m(a, ...)`: the call `m(x, a, ...)`, unless `x` is a variable
        // holding a record with a field `m` (a function stored there, D148)
        if let ExprKind::Field(recv, m) = &f.kind {
            let field_fn = match &recv.kind {
                ExprKind::Var(v) => match self.lookup(v) {
                    Some(l) => {
                        let t = self.locals[l as usize].ty.clone();
                        matches!(self.shallow(&t), Ty::Record(fs) if fs.iter().any(|(n, _)| n == m))
                    }
                    None => false,
                },
                _ => false,
            };
            if !field_fn {
                let callee = ast::Expr { kind: ExprKind::Var(m.clone()), span: f.span };
                let mut all = vec![(**recv).clone()];
                all.extend(args.iter().cloned());
                self.method_call = true;
                return self.check_call(&callee, &all, span, expected);
            }
        }
        // `f<T>(x)`: the type parameters of `f` given (D336)
        if let ExprKind::TypeApp(inner, ts) = &f.kind {
            let ExprKind::Var(name) = &inner.kind else {
                return err(f.span, "type arguments are given to a function by its name: `f<T>(x)`");
            };
            if self.lookup(name).is_some() || self.sym_by_name(name).is_some() {
                return err(inner.span, format!("`{}` is a variable: type arguments are given to a function of the program or of a module", name));
            }
            let q = self.q(name);
            let Some(&id) = self.g.names.get(q.as_str()) else {
                return err(inner.span, format!("unknown function `{}`", name));
            };
            // the sizes that the type of `json.decode` names are those of
            // the value read: fixed by the call, as those of a result
            let existential = q == DECODE;
            let mut tys = Vec::new();
            for t in ts {
                tys.push(self.type_arg(t, existential)?);
            }
            return self.check_call_fn_with(id, args, span, Some(&tys));
        }
        if let ExprKind::Var(name) = &f.kind {
            // `m.keys()` with a local `keys` that is not a function: the
            // function `keys`, as in TypeScript
            let shadowed = method_call
                && self.lookup(name).is_some_and(|l| !matches!(self.shallow(&self.locals[l as usize].ty), Ty::Fn(..)));
            if (shadowed || self.lookup(name).is_none()) && self.sym_by_name(name).is_none() {
                let name = &self.q(name);
                if self.g.ctors.contains_key(name.as_str()) {
                    return self.check_construct(name, args, span, expected);
                }
                if let Some(&id) = self.g.names.get(name.as_str()) {
                    let call = self.check_call_fn(id, args, span)?;
                    if id >= self.g.first_extern {
                        // a C function (D125)
                        let TExpr { kind, ty, span } = call;
                        let TExprKind::Call { args, size_checks, .. } = kind else { unreachable!() };
                        let callee = Callee::Extern((id - self.g.first_extern) as u32);
                        return Ok(self.mk(TExprKind::Call { callee, args, size_checks }, ty, span));
                    }
                    return Ok(call);
                }
                if let Some(p) = prim_by_name(name) {
                    return self.check_prim(p, name, args, span, expected);
                }
                if NOT_IN_POC.contains(&name.as_str()) {
                    return Err(poc(f.span, &format!("`{}`", name)));
                }
                return err(f.span, format!("unknown function `{}`", name));
            }
        }
        let callee = self.check(f, None)?;
        let ct = self.shallow(&callee.ty);
        let (ptys, ret, act) = match ct {
            Ty::Fn(ps, r, act) => (ps, *r, act),
            Ty::Var(_) => {
                let ps: Vec<Ty> = args.iter().map(|a| self.fresh(0, LitKind::None, a.span)).collect();
                let r = self.fresh(0, LitKind::None, span);
                self.unify(&Ty::Fn(ps.clone(), Box::new(r.clone()), false), &callee.ty, f.span)?;
                (ps, r, false)
            }
            other => return err(f.span, format!("this value of type `{}` is not a function", self.show(&other))),
        };
        if act && !self.can_act() {
            let what = match &f.kind {
                ExprKind::Var(n) => format!("`{}`", n),
                _ => "this value".to_string(),
            };
            return err(
                f.span,
                if self.is_action {
                    format!("the action {} cannot be called inside a lambda that is not an action", what)
                } else {
                    format!("a `function` cannot call the action {} (SPEC section 12)", what)
                },
            );
        }
        if act {
            self.note_effect();
        }
        self.arity("this function", ptys.len(), args.len(), span)?;
        let mut targs = Vec::new();
        for (a, p) in args.iter().zip(ptys.iter()) {
            let t = self.check(a, Some(p))?;
            self.unify(p, &t.ty, a.span)?;
            targs.push(t);
        }
        // a result of its own size: a new existential size (D341)
        let ret = self.open_own(&ret, span);
        Ok(self.mk(
            TExprKind::Call { callee: Callee::Value(Box::new(callee)), args: targs, size_checks: Vec::new() },
            ret,
            span,
        ))
    }

    /// `C` or `C(a, b)`: a value of the sum type of constructor `C`; the type
    /// arguments are inferred.
    fn check_construct(&mut self, name: &str, args: &[ast::Expr], span: Span, expected: Option<&Ty>) -> Result<TExpr> {
        let g = self.g;
        let (sum, variant) = g.ctors[name];
        let def = &g.sums[sum as usize];
        // the type arguments expected when they hold arrays of their own
        // size (D341): `Ok({ body: [] })` where a `Result<Response, E>` is
        let targs: Vec<Ty> = match expected.map(|e| self.shallow(e)) {
            Some(Ty::Sum { id, args: ea, .. }) if id == sum && ea.iter().any(|t| self.holds_own(t)) => ea,
            _ => def.params.iter().map(|_| self.fresh(0, LitKind::None, span)).collect(),
        };
        // the sizes of the payload: any (D335)
        let sizes: Vec<Size> = (0..def.variants[variant as usize].sizes).map(|_| self.fresh_size(span)).collect();
        let payload = payload_sized(def, variant as usize, &targs, &sizes);
        if payload.len() != args.len() {
            return err(
                span,
                format!("the constructor `{}` takes {} argument(s), but {} were given", name, payload.len(), args.len()),
            );
        }
        let mut out = Vec::new();
        for (a, p) in args.iter().zip(payload.iter()) {
            let t = self.check(a, Some(p))?;
            self.unify(p, &t.ty, a.span)?;
            out.push(t);
        }
        let ty = Ty::Sum { id: sum, name: def.name.clone(), args: targs };
        Ok(self.mk(TExprKind::Construct { sum, variant, args: out }, ty, span))
    }

    fn check_call_fn(&mut self, id: FnId, args: &[ast::Expr], span: Span) -> Result<TExpr> {
        self.check_call_fn_with(id, args, span, None)
    }

    /// A type argument of a call (D336): it may name the type parameters
    /// and sizes of the function; a size it names that the function has not
    /// is any size (`decode<Point[k]>`).
    fn type_arg(&mut self, t: &ast::TypeExpr, existential: bool) -> Result<Ty> {
        let mut syms: Vec<SymInfo> = self.syms.clone();
        let generics = self.generics.clone();
        let n = syms.len();
        let ty = type_from_ast(t, &mut SigCtx { generics: &generics, sums: &self.g.heads, syms: &mut syms, allow_new: true, result: false, module: self.module.as_deref(), imports: self.g.imports.get(&self.file) })?;
        for s in n..syms.len() {
            if !alone_in(&ty, s as SymId) {
                return err(t.span, format!("the new size `{}` of a type argument stands alone as the size of an array (`T[{}]`)", syms[s].name, syms[s].name));
            }
        }
        if existential {
            for s in syms.iter_mut().skip(n) {
                s.name = format!("{}@{}:{}", s.name, t.span.line, t.span.col);
            }
            self.syms = syms;
            return Ok(ty);
        }
        let fresh: Vec<Size> = (n..syms.len()).map(|_| self.fresh_size(t.span)).collect();
        fn open(t: &Ty, n: usize, fresh: &[Size]) -> Ty {
            match t {
                Ty::Array(Size::Poly(p), e) => {
                    let e = Box::new(open(e, n, fresh));
                    match p.as_sym() {
                        Some(s) if s as usize >= n => Ty::Array(fresh[s as usize - n].clone(), e),
                        _ => Ty::Array(Size::Poly(p.clone()), e),
                    }
                }
                Ty::Array(s, e) => Ty::Array(s.clone(), Box::new(open(e, n, fresh))),
                Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| open(t, n, fresh)).collect()),
                Ty::Record(fs) => Ty::Record(fs.iter().map(|(k, t)| (k.clone(), open(t, n, fresh))).collect()),
                Ty::Sum { id, name, args } => Ty::Sum { id: *id, name: name.clone(), args: args.iter().map(|t| open(t, n, fresh)).collect() },
                Ty::Fn(ps, r, act) => Ty::Fn(ps.iter().map(|t| open(t, n, fresh)).collect(), Box::new(open(r, n, fresh)), *act),
                other => other.clone(),
            }
        }
        Ok(open(&ty, n, &fresh))
    }

    fn check_call_fn_with(&mut self, id: FnId, args: &[ast::Expr], span: Span, explicit: Option<&[Ty]>) -> Result<TExpr> {
        let g = self.g;
        let sig = &g.sigs[id];
        if sig.is_action && !self.can_act() {
            let what = if id >= g.first_extern { "extern function" } else { "action" };
            return err(
                span,
                if self.is_action {
                    format!("the {} `{}` cannot be called inside a lambda", what, sig.name)
                } else {
                    format!("a `function` cannot call the {} `{}` (SPEC section 12)", what, sig.name)
                },
            );
        }
        if sig.is_action {
            self.note_effect();
        }
        self.arity(&sig.name, sig.param_tys.len(), args.len(), span)?;
        let type_args: Vec<Ty> = sig.generics.iter().map(|gc| self.fresh(gc.1, LitKind::None, span)).collect();
        if let Some(ex) = explicit {
            if ex.len() != type_args.len() {
                return err(span, format!("`{}` takes {} type argument(s), but {} were given", sig.name, type_args.len(), ex.len()));
            }
            for (a, t) in type_args.iter().zip(ex) {
                self.unify(a, t, span)?;
            }
        }
        // Check the arguments, with hints for lambdas.
        let mut targs = Vec::new();
        for (a, p) in args.iter().zip(sig.param_tys.iter()) {
            let hint_sizes: Vec<Size> = (0..sig.syms.len()).map(|_| self.fresh_size(a.span)).collect();
            let hint = subst_value(p, &type_args, &hint_sizes);
            let t = self.check(a, Some(&hint))?;
            // the type parameters that this argument fixes, known to the
            // lambdas after it: `count(rows, (r) => r.id == 1)` reads the
            // fields of the rows' records
            self.bind_type_args(p, &t.ty, &type_args);
            targs.push(t);
        }
        // Bind the callee's size variables from the binding parameters.
        let mut sym_map: Vec<Option<Size>> = vec![None; sig.syms.len()];
        for (s, info) in sig.syms.iter().enumerate() {
            if let SymOrigin::SizeParam(i) = info.origin {
                // a size expression, or a local `i64` variable (whose value
                // then stands for the size in the caller's types)
                let size = match self.try_size_arg(&args[i]) {
                    Some(p) => p,
                    None => match &args[i].kind {
                        ExprKind::Var(name) if self.lookup(name).is_some() => {
                            let l = self.lookup(name).unwrap();
                            let existing = self.syms.iter().position(|x| x.origin == SymOrigin::Local(l));
                            let sym = match existing {
                                Some(k) => k as SymId,
                                None => {
                                    self.syms.push(SymInfo { name: name.clone(), origin: SymOrigin::Local(l) });
                                    (self.syms.len() - 1) as SymId
                                }
                            };
                            Poly::sym(sym)
                        }
                        _ => {
                            return err(
                                args[i].span,
                                format!(
                                    "the `size` argument `{}` must be a size (a literal, a size variable, a sum or product of these) or a variable; bind it with `const` first",
                                    sig.param_names[i].0
                                ),
                            )
                        }
                    },
                };
                sym_map[s] = Some(Size::Poly(size));
            }
            // an array in a record or a tuple of the argument (D336)
            if let SymOrigin::ParamPath(i, path) = &info.origin {
                let mut at = self.shallow(&targs[*i].ty);
                let mut pt = sig.param_tys[*i].clone();
                for &k in path {
                    let (next, pnext) = match (&at, &pt) {
                        (Ty::Array(_, e), Ty::Array(_, pe)) if k == crate::tast::ELEMENT => (Some((**e).clone()), (**pe).clone()),
                        (Ty::Record(fs), Ty::Record(ps)) => {
                            let name = &ps[k as usize].0;
                            (fs.iter().find(|(n, _)| n == name).map(|(_, t)| t.clone()), ps[k as usize].1.clone())
                        }
                        (Ty::Tuple(ts), Ty::Tuple(ps)) if ts.len() == ps.len() => (Some(ts[k as usize].clone()), ps[k as usize].clone()),
                        _ => (None, pt.clone()),
                    };
                    match next {
                        Some(n) => {
                            at = self.shallow(&n);
                            pt = pnext;
                        }
                        None => {
                            return err(
                                targs[*i].span,
                                format!(
                                    "type mismatch: expected `{}`, found `{}`",
                                    self.show_sig_ty(id, &sig.param_tys[*i]),
                                    self.show(&targs[*i].ty)
                                ),
                            )
                        }
                    }
                }
                let at = if let Ty::Var(_) = at {
                    return err(targs[*i].span, "the type of this argument must be known here (an array in it gives a size of the function)");
                } else {
                    at
                };
                match at {
                    // an array of its own size: its size for this call, a new
                    // existential one (D341)
                    Ty::Array(sz, _) => sym_map[s] = Some(self.size_of_call(&sz, args[i_of(&info.origin)].span)),
                    other => {
                        return err(
                            targs[*i].span,
                            format!("type mismatch: expected an array, found `{}`", self.show(&other)),
                        )
                    }
                }
            }
            if let SymOrigin::Param(i) = info.origin {
                let at = self.shallow(&targs[i].ty);
                let at = if let Ty::Var(_) = at {
                    let (arr, _, _) = self.fresh_array(targs[i].span);
                    self.unify(&arr, &targs[i].ty, targs[i].span)?;
                    self.shallow(&arr)
                } else {
                    at
                };
                match at {
                    // an array of its own size: its size for this call, a new
                    // existential one (D341)
                    Ty::Array(sz, _) => sym_map[s] = Some(self.size_of_call(&sz, args[i_of(&info.origin)].span)),
                    other => {
                        return err(
                            targs[i].span,
                            format!(
                                "type mismatch: expected `{}`, found `{}`",
                                self.show_sig_ty(id, &sig.param_tys[i]),
                                self.show(&other)
                            ),
                        )
                    }
                }
            }
        }
        // each existential size of the result: a new existential size of the
        // caller, fixed by this call
        for (s, info) in sig.syms.iter().enumerate() {
            if let SymOrigin::Existential(_) = info.origin {
                if sym_map[s].is_none() {
                    let k = self.syms.len() as SymId;
                    self.syms.push(SymInfo {
                        name: format!("{}@{}:{}", info.name, span.line, span.col),
                        origin: SymOrigin::Existential(span),
                    });
                    sym_map[s] = Some(Size::Poly(Poly::sym(k)));
                }
            }
        }
        let mut size_checks = Vec::new();
        for (i, p) in sig.param_tys.iter().enumerate() {
            let inst = self.subst_call(p, &type_args, &sym_map, targs[i].span)?;
            let needs = self.unify_mode(&inst, &targs[i].ty, targs[i].span, Mode::Dynamic)?;
            if needs {
                size_checks.push(i);
            }
        }
        let ret = self.subst_call(&sig.ret, &type_args, &sym_map, span)?;
        Ok(self.mk(TExprKind::Call { callee: Callee::Fn { id, type_args }, args: targs, size_checks }, ret, span))
    }

    /// `t` with each size of an array replaced by a fresh size variable (the
    /// hint of the second branch of a conditional, D337).
    fn loosen(&mut self, t: &Ty) -> Ty {
        match self.shallow(t) {
            Ty::Array(s, e) => {
                let e = self.loosen(&e);
                // a size of its own stays (D341)
                let s = match self.size_res(&s) {
                    Size::Own => Size::Own,
                    _ => self.fresh_size(Span::default()),
                };
                Ty::Array(s, Box::new(e))
            }
            Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| self.loosen(t)).collect()),
            Ty::Record(fs) => Ty::Record(fs.iter().map(|(n, t)| (n.clone(), self.loosen(t))).collect()),
            Ty::Sum { id, name, args } => Ty::Sum { id, name, args: args.iter().map(|t| self.loosen(t)).collect() },
            other => other,
        }
    }

    /// The type of a value that is `a` or `b` (the branches of a conditional,
    /// the arms of a `match`, D337): their types unified, but where both
    /// are arrays of sizes that differ, a new size fixed at run time (as the
    /// size of an array that a call returns), through tuples and records.
    fn join(&mut self, a: &Ty, b: &Ty, span: Span) -> Result<Ty> {
        let (x, y) = (self.shallow(a), self.shallow(b));
        Ok(match (&x, &y) {
            (Ty::Array(s1, e1), Ty::Array(s2, e2)) => {
                let e = self.join(e1, e2, span)?;
                let (r1, r2) = (self.size_res(s1), self.size_res(s2));
                let s = match (&r1, &r2) {
                    (Size::Own, _) | (_, Size::Own) => Size::Own,
                    (Size::Poly(p), Size::Poly(q)) if p != q => {
                        let k = self.syms.len() as SymId;
                        self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                        Size::Poly(Poly::sym(k))
                    }
                    _ => {
                        self.unify_size(&r1, &r2, Mode::Strict, false).map_err(|_| Diagnostic::new(span, "size mismatch"))?;
                        r1
                    }
                };
                Ty::Array(s, Box::new(e))
            }
            (Ty::Tuple(p), Ty::Tuple(q)) if p.len() == q.len() => {
                let mut ts = Vec::new();
                for (u, v) in p.iter().zip(q.iter()) {
                    ts.push(self.join(u, v, span)?);
                }
                Ty::Tuple(ts)
            }
            (Ty::Record(p), Ty::Record(q)) if p.len() == q.len() && p.iter().zip(q.iter()).all(|(u, v)| u.0 == v.0) => {
                let mut fs = Vec::new();
                for ((n, u), (_, v)) in p.iter().zip(q.iter()) {
                    fs.push((n.clone(), self.join(u, v, span)?));
                }
                Ty::Record(fs)
            }
            // `Result<string[a], E>` and `Result<string[b], E>`: through the
            // arguments of a sum type
            (Ty::Sum { id: i, name, args: p }, Ty::Sum { id: j, args: q, .. }) if i == j && p.len() == q.len() => {
                let mut args = Vec::new();
                for (u, v) in p.iter().zip(q.iter()) {
                    args.push(self.join(u, v, span)?);
                }
                Ty::Sum { id: *i, name: name.clone(), args }
            }
            _ => {
                self.unify(&x, &y, span)?;
                x
            }
        })
    }

    /// `t` with each existential size symbol replaced by a fresh size variable.
    fn open_existentials(&mut self, t: &Ty) -> Ty {
        match t {
            Ty::Array(Size::Poly(p), e) => {
                let e = Box::new(self.open_existentials(e));
                match p.as_sym() {
                    Some(s) if matches!(self.syms[s as usize].origin, SymOrigin::Existential(_)) => {
                        let v = self.fresh_size(Span::default());
                        Ty::Array(v, e)
                    }
                    _ => Ty::Array(Size::Poly(p.clone()), e),
                }
            }
            Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| self.open_existentials(t)).collect()),
            Ty::Record(fs) => Ty::Record(fs.iter().map(|(n, t)| (n.clone(), self.open_existentials(t))).collect()),
            // the arguments of a sum type: `Result<[k]f64, string>` (D138)
            Ty::Sum { id, name, args } => {
                Ty::Sum { id: *id, name: name.clone(), args: args.iter().map(|t| self.open_existentials(t)).collect() }
            }
            other => other.clone(),
        }
    }

    fn show_sig_ty(&self, id: FnId, t: &Ty) -> String {
        let sig = &self.g.sigs[id];
        let sym_name = |s: SymId| sig.syms[s as usize].name.clone();
        let param_name = |p: u32| sig.generics[p as usize].0.clone();
        let size_var = |_v: u32| None;
        format!("{}", TyDisplay { ty: t, sym_name: &sym_name, param_name: &param_name, size_var: &size_var })
    }

    /// Substitutes a callee type: type parameters by `type_args`, size
    /// symbols by the caller's sizes.
    fn subst_call(&mut self, t: &Ty, type_args: &[Ty], sym_map: &[Option<Size>], span: Span) -> Result<Ty> {
        Ok(match t {
            Ty::Param(p) => type_args[*p as usize].clone(),
            Ty::Tuple(ts) => {
                Ty::Tuple(ts.iter().map(|t| self.subst_call(t, type_args, sym_map, span)).collect::<Result<_>>()?)
            }
            Ty::Record(fs) => Ty::Record(
                fs.iter()
                    .map(|(n, t)| Ok((n.clone(), self.subst_call(t, type_args, sym_map, span)?)))
                    .collect::<Result<_>>()?,
            ),
            Ty::Sum { id, name, args } => Ty::Sum {
                id: *id,
                name: name.clone(),
                args: args.iter().map(|t| self.subst_call(t, type_args, sym_map, span)).collect::<Result<_>>()?,
            },
            Ty::Fn(ps, r, act) => Ty::Fn(
                ps.iter().map(|t| self.subst_call(t, type_args, sym_map, span)).collect::<Result<_>>()?,
                Box::new(self.subst_call(r, type_args, sym_map, span)?),
            *act),
            Ty::Array(Size::Poly(p), e) => {
                let e = self.subst_call(e, type_args, sym_map, span)?;
                let size = if let Some(s) = p.as_sym() {
                    sym_map[s as usize].clone().unwrap()
                } else {
                    let mut ok = true;
                    let polys: Vec<Poly> = (0..sym_map.len())
                        .map(|s| match sym_map[s].as_ref().map(|z| self.size_res(z)) {
                            Some(Size::Poly(q)) => q,
                            _ => {
                                ok = !p.syms().contains(&(s as SymId)) && ok;
                                Poly::default()
                            }
                        })
                        .collect();
                    if !ok {
                        return err(span, "cannot infer the size of this argument; add a type annotation");
                    }
                    Size::Poly(p.subst(&|s| polys[s as usize].clone()))
                };
                Ty::Array(size, Box::new(e))
            }
            other => other.clone(),
        })
    }

    /// A primitive of the system (D322): its arguments and its result.
    fn check_sys(&mut self, op: SysOp, name: &str, args: &[ast::Expr], span: Span) -> Result<TExpr> {
        let i64t = Ty::Int(IntKind::I64);
        let exist = |c: &mut Self, elem: Ty| -> Ty {
            let s = c.syms.len() as SymId;
            c.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
            Ty::Array(Size::Poly(Poly::sym(s)), Box::new(elem))
        };
        let (params, result): (Vec<Ty>, Ty) = match op {
            SysOp::Getenv => (vec![Ty::Str], Ty::Tuple(vec![Ty::Str, i64t.clone()])),
            SysOp::Environ => (vec![], exist(self, Ty::Str)),
            SysOp::Stat => (vec![Ty::Str], Ty::Tuple(vec![i64t.clone(), i64t.clone(), i64t.clone(), i64t.clone()])),
            SysOp::ReadDir => (vec![Ty::Str], Ty::Tuple(vec![exist(self, Ty::Str), i64t.clone()])),
            SysOp::Mkdir | SysOp::RemoveFile | SysOp::RemoveDir => (vec![Ty::Str], i64t.clone()),
            SysOp::Rename => (vec![Ty::Str, Ty::Str], i64t.clone()),
            SysOp::Cwd => (vec![], Ty::Tuple(vec![Ty::Str, i64t.clone()])),
            SysOp::Now => (vec![], i64t.clone()),
            SysOp::ReadFile => (vec![Ty::Str], Ty::Tuple(vec![exist(self, Ty::Int(IntKind::U8)), i64t.clone()])),
            SysOp::WriteFile => {
                let s = self.fresh_size(span);
                (vec![Ty::Str, Ty::Array(s, Box::new(Ty::Int(IntKind::U8))), Ty::Bool], i64t.clone())
            }
            // the resources of the system (D339)
            SysOp::Connect => (vec![Ty::Str, i64t.clone()], Ty::Tuple(vec![Ty::Handle(Handle::Conn), i64t.clone()])),
            SysOp::Listen => (vec![Ty::Str, i64t.clone()], Ty::Tuple(vec![Ty::Handle(Handle::Listener), i64t.clone()])),
            SysOp::Accept => (vec![Ty::Handle(Handle::Listener)], Ty::Tuple(vec![Ty::Handle(Handle::Conn), Ty::Str, i64t.clone()])),
            SysOp::Recv => (vec![Ty::Handle(Handle::Conn), i64t.clone()], Ty::Tuple(vec![exist(self, Ty::Int(IntKind::U8)), i64t.clone()])),
            SysOp::Send => {
                let s = self.fresh_size(span);
                (vec![Ty::Handle(Handle::Conn), Ty::Array(s, Box::new(Ty::Int(IntKind::U8)))], i64t.clone())
            }
            SysOp::Close => (vec![Ty::Handle(Handle::Conn)], i64t.clone()),
            SysOp::CloseListener => (vec![Ty::Handle(Handle::Listener)], i64t.clone()),
            SysOp::SetTimeout => (vec![Ty::Handle(Handle::Conn), i64t.clone()], i64t.clone()),
            SysOp::Shutdown => (vec![Ty::Handle(Handle::Conn)], i64t.clone()),
            SysOp::ServerOpen => (vec![Ty::Handle(Handle::Listener)], Ty::Tuple(vec![Ty::Handle(Handle::Server), i64t.clone()])),
            SysOp::ServerWait => {
                let own = |t: Ty| Ty::Array(Size::Own, Box::new(t));
                (vec![Ty::Handle(Handle::Server), i64t.clone(), i64t.clone()], Ty::Tuple(vec![own(i64t.clone()), own(i64t.clone()), own(Ty::Int(IntKind::U8)), i64t.clone()]))
            }
            SysOp::ServerSend => {
                let s = self.fresh_size(span);
                (vec![Ty::Handle(Handle::Server), i64t.clone(), Ty::Array(s, Box::new(Ty::Int(IntKind::U8)))], i64t.clone())
            }
            SysOp::ServerDrop => (vec![Ty::Handle(Handle::Server), i64t.clone()], i64t.clone()),
            SysOp::ServerClose => (vec![Ty::Handle(Handle::Server)], i64t.clone()),
            SysOp::TlsStart => (vec![Ty::Handle(Handle::Conn), Ty::Str, Ty::Str], Ty::Tuple(vec![Ty::Str, i64t.clone()])),
            SysOp::ServerOpenTls => (vec![Ty::Handle(Handle::Listener), Ty::Str, Ty::Str], Ty::Tuple(vec![Ty::Handle(Handle::Server), Ty::Str, i64t.clone()])),
            SysOp::LocalPort => (vec![Ty::Handle(Handle::Listener)], i64t.clone()),
            SysOp::UdpQuery => {
                let s = self.fresh_size(span);
                (vec![Ty::Str, i64t.clone(), Ty::Array(s, Box::new(Ty::Int(IntKind::U8))), i64t.clone()], Ty::Tuple(vec![exist(self, Ty::Int(IntKind::U8)), i64t.clone()]))
            }
            SysOp::Platform | SysOp::Threads => (vec![], i64t.clone()),
            SysOp::ListenShared => (vec![Ty::Str, i64t.clone()], Ty::Tuple(vec![Ty::Handle(Handle::Listener), i64t.clone()])),
            SysOp::Resolve => (vec![Ty::Str], Ty::Tuple(vec![exist(self, Ty::Str), i64t.clone()])),
        };
        if args.len() != params.len() {
            return err(span, format!("`{}` takes {} arguments, not {}", name, params.len(), args.len()));
        }
        let mut out = Vec::new();
        for (a, t) in args.iter().zip(&params) {
            let x = self.check(a, Some(t))?;
            self.unify(t, &x.ty, x.span)?;
            out.push(x);
        }
        Ok(self.mk(TExprKind::Call { callee: Callee::Prim(Prim::Sys(op)), args: out, size_checks: Vec::new() }, result, span))
    }

    fn check_prim(
        &mut self,
        p: Prim,
        name: &str,
        args: &[ast::Expr],
        span: Span,
        expected: Option<&Ty>,
    ) -> Result<TExpr> {
        // the primitives of the system: the standard modules only (D322)
        if let Prim::Sys(op) = p {
            if !self.module.as_deref().is_some_and(|m| m.starts_with(crate::modules::STD_KEY)) {
                return err(span, format!("`{}` is a primitive of the standard library: a program calls the functions of its modules (`import {{ env }} from \"sys\"`)", name));
            }
            if !self.can_act() {
                return err(span, format!("the action `{}` cannot be called here (SPEC section 12)", name));
            }
            self.note_effect();
            return self.check_sys(op, name, args, span);
        }
        if p.is_action() && !self.can_act() {
            return err(
                span,
                if self.is_action {
                    format!("the action `{}` cannot be called inside a lambda", name)
                } else {
                    format!("a `function` cannot call the action `{}` (SPEC section 12)", name)
                },
            );
        }
        if p.is_action() {
            self.note_effect();
        }
        let n = match p {
            // checked by `check_sys`, which returned
            Prim::Sys(_) => unreachable!(),
            Prim::Zeros | Prim::Clock | Prim::ReadStdin | Prim::ReadStdinText | Prim::ReadStdinLines => 0,
            Prim::Iota | Prim::Sqrt | Prim::Convert(_) | Prim::ReadF64 | Prim::ReadF32 | Prim::ReadBytes | Prim::Print | Prim::IsPure => 1,
            Prim::Math(f) => f.arity(),
            Prim::ParseI64 | Prim::ParseF64 | Prim::StrLen | Prim::Bytes | Prim::StringOf | Prim::ToString => 1,
            Prim::Concat => 2,
            Prim::ReadText | Prim::Lines | Prim::Trim | Prim::PrintBytes => 1,
            Prim::OrError(Fallible::ReadCsvF64 | Fallible::WriteBytes | Fallible::WriteText) => 2,
            Prim::OrError(_) => 1,
            Prim::WriteText | Prim::ParseCsvF64 | Prim::ReadCsvF64 | Prim::Split | Prim::Join => 2,
            Prim::Map | Prim::MapParallel | Prim::Zip | Prim::Filter | Prim::Window | Prim::Fill | Prim::WriteF64 | Prim::WriteF32 | Prim::WriteBytes | Prim::Bench => 2,
            Prim::Reduce | Prim::Fold | Prim::Scan => 3,
            Prim::ToUpper | Prim::ToLower | Prim::TrimStart | Prim::TrimEnd | Prim::ArgSort | Prim::JsNumber | Prim::ParseFloat | Prim::ToBigInt | Prim::Group | Prim::GroupCounts => 1,
            Prim::ParseInt => {
                if args.len() == 1 {
                    1
                } else {
                    2
                }
            }
            Prim::IndexOf | Prim::LastIndexOf | Prim::Includes | Prim::StartsWith | Prim::EndsWith | Prim::Repeat | Prim::ToFixed => 2,
            Prim::ReplaceAll | Prim::StrSlice | Prim::Replace => 3,
            Prim::RxTest | Prim::RxSearch | Prim::RxCount | Prim::RxMatches => 2,
            Prim::MapOf => 3,
            Prim::MapParts | Prim::MapSize => 1,
            Prim::RegExpNew => {
                if args.len() == 1 {
                    1
                } else {
                    2
                }
            }
            // `padStart(s, n)` pads with spaces, as TypeScript
            Prim::PadStart | Prim::PadEnd => {
                if args.len() == 2 {
                    2
                } else {
                    3
                }
            }
            // `min(a, b)` of two numbers, `min(x)` of an array
            Prim::Min | Prim::Max => {
                if args.len() == 1 {
                    1
                } else {
                    2
                }
            }
        };
        self.arity(name, n, args.len(), span)?;
        let mk = |kind_args: Vec<TExpr>, ty: Ty, s: &Self| {
            s.mk(TExprKind::Call { callee: Callee::Prim(p), args: kind_args, size_checks: Vec::new() }, ty, span)
        };
        match p {
            // in an action, its function may be an action: applied to each
            // element in order, once each (D387); a pure one in parallel
            Prim::Map => {
                let x = self.check(&args[0], None)?;
                let a = self.expect_array(&x.ty, x.span, "the first argument of `map`")?;
                let b = self.fresh(class::ELEM, LitKind::None, span);
                let fty = Ty::Fn(vec![a.clone()], Box::new(b.clone()), self.can_act());
                let f = self.check(&args[1], Some(&fty))?;
                self.unify(&fty, &f.ty, f.span)?;
                let size = match self.shallow(&x.ty) {
                    Ty::Array(s, _) => s,
                    _ => unreachable!(),
                };
                Ok(mk(vec![x, f], Ty::Array(size, Box::new(b)), self))
            }
            // the action of the lambda on each element, at the same time
            // (D373): its results are not arrays (in the POC)
            Prim::MapParallel => {
                let x = self.check(&args[0], None)?;
                let a = self.expect_array(&x.ty, x.span, "the first argument of `mapParallel`")?;
                let b = self.fresh(class::ELEM, LitKind::None, span);
                let fty = Ty::Fn(vec![a.clone()], Box::new(b.clone()), true);
                let f = self.check(&args[1], Some(&fty))?;
                self.unify(&fty, &f.ty, f.span)?;
                for (t, what) in [(&a, "its elements"), (&b, "its results")] {
                    if matches!(self.shallow(t), Ty::Array(..)) {
                        return err(span, format!("`mapParallel` over arrays of arrays is not supported in the POC ({} are arrays; a record may hold one)", what));
                    }
                }
                let size = match self.shallow(&x.ty) {
                    Ty::Array(s, _) => s,
                    _ => unreachable!(),
                };
                Ok(mk(vec![x, f], Ty::Array(size, Box::new(b)), self))
            }
            Prim::Min | Prim::Max if args.len() == 2 => {
                // two numbers of the same type (IEEE 754-2019 `minimum` and
                // `maximum` for floats, SPEC section 10)
                let a = self.check(&args[0], expected)?;
                let b = self.check(&args[1], Some(&a.ty))?;
                self.unify(&a.ty, &b.ty, b.span)?;
                self.require(&a.ty, class::NUM | class::SCALAR, a.span)?;
                let ty = a.ty.clone();
                Ok(mk(vec![a, b], ty, self))
            }
            Prim::Min | Prim::Max => {
                // the smallest (largest) element of an array of numbers
                let x = self.check(&args[0], None)?;
                let what = format!("the argument of `{}`", name);
                let t = match self.expect_array(&x.ty, x.span, &what) {
                    Ok(t) => t,
                    Err(_) => {
                        return err(
                            x.span,
                            format!("`{0}(x)` takes an array; `{0}(a, b)` takes two numbers", name),
                        )
                    }
                };
                self.require(&t, class::NUM | class::SCALAR, x.span)?;
                Ok(mk(vec![x], t, self))
            }
            Prim::Zip => {
                // `zip(a, b)`: `[n](A, B)`; the sizes of `a` and `b` are the
                // same, or a run-time test when they cannot be proved equal,
                // as for the element-wise operators (a panic otherwise)
                let a = self.check(&args[0], None)?;
                let ta = self.expect_array(&a.ty, a.span, "the first argument of `zip`")?;
                let b = self.check(&args[1], None)?;
                let tb = self.expect_array(&b.ty, b.span, "the second argument of `zip`")?;
                let (sa, sb) = match (self.shallow(&a.ty), self.shallow(&b.ty)) {
                    (Ty::Array(sa, _), Ty::Array(sb, _)) => (sa, sb),
                    _ => unreachable!(),
                };
                // the sizes alone (a placeholder element), reported with the
                // types of the arguments
                let dynamic = match self.unify_mode(
                    &Ty::Array(sa.clone(), Box::new(Ty::Bool)),
                    &Ty::Array(sb, Box::new(Ty::Bool)),
                    b.span,
                    Mode::Dynamic,
                ) {
                    Ok(d) => d,
                    Err(_) => {
                        return err(
                            b.span,
                            format!(
                                "size mismatch: `zip` of `{}` and `{}`",
                                self.show(&a.ty),
                                self.show(&b.ty)
                            ),
                        )
                    }
                };
                let ty = Ty::Array(sa, Box::new(Ty::Tuple(vec![ta, tb])));
                let size_checks = if dynamic { vec![1] } else { Vec::new() };
                Ok(self.mk(TExprKind::Call { callee: Callee::Prim(p), args: vec![a, b], size_checks }, ty, span))
            }
            Prim::Reduce => {
                let x = self.check(&args[0], None)?;
                let a = self.expect_array(&x.ty, x.span, "the first argument of `reduce`")?;
                self.require(&a, class::ELEM, x.span)?;
                let fty = Ty::Fn(vec![a.clone(), a.clone()], Box::new(a.clone()), false);
                let f = self.check(&args[1], Some(&fty))?;
                self.unify(&fty, &f.ty, f.span)?;
                let e = self.check(&args[2], Some(&a))?;
                self.unify(&a, &e.ty, e.span)?;
                Ok(mk(vec![x, f, e], a, self))
            }
            Prim::Window => {
                // `window(x, k)`: `[n - k + 1][k]T`, the sliding windows of
                // `x`, a view that is never copied (SPEC section 6); a panic
                // when `k > n`
                let x = self.check(&args[0], None)?;
                let t = self.expect_array(&x.ty, x.span, "the first argument of `window`")?;
                if !matches!(self.shallow(&t), Ty::Int(_) | Ty::F64 | Ty::F32 | Ty::Bool | Ty::Var(_)) {
                    return Err(poc(x.span, "`window` over an array whose elements are not scalars"));
                }
                let k = self.size_arg(&args[1])?;
                let karg = self.mk(TExprKind::SizeVal(k.clone()), Ty::Int(IntKind::I64), args[1].span);
                let rows = match self.shallow(&x.ty) {
                    Ty::Array(Size::Poly(n), _) => Size::Poly(n.sub(&k).add(&Poly::constant(1))),
                    _ => {
                        let s = self.syms.len() as SymId;
                        self.syms.push(SymInfo { name: format!("w@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                        Size::Poly(Poly::sym(s))
                    }
                };
                let ty = Ty::Array(rows, Box::new(Ty::Array(Size::Poly(k), Box::new(t))));
                Ok(mk(vec![x, karg], ty, self))
            }
            Prim::Filter => {
                // `[k <= n]T`: k is known at run time (an existential size)
                let x = self.check(&args[0], None)?;
                let a = self.expect_array(&x.ty, x.span, "the first argument of `filter`")?;
                let fty = Ty::Fn(vec![a.clone()], Box::new(Ty::Bool), false);
                let f = self.check(&args[1], Some(&fty))?;
                self.unify(&fty, &f.ty, f.span)?;
                let s = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                Ok(mk(vec![x, f], Ty::Array(Size::Poly(Poly::sym(s)), Box::new(a)), self))
            }
            Prim::Scan => {
                // `[n]T`: out[i] = op(out[i - 1], x[i]), out[-1] = e
                let x = self.check(&args[0], None)?;
                let a = self.expect_array(&x.ty, x.span, "the first argument of `scan`")?;
                self.require(&a, class::ELEM, x.span)?;
                let fty = Ty::Fn(vec![a.clone(), a.clone()], Box::new(a.clone()), false);
                let f = self.check(&args[1], Some(&fty))?;
                self.unify(&fty, &f.ty, f.span)?;
                let e = self.check(&args[2], Some(&a))?;
                self.unify(&a, &e.ty, e.span)?;
                let ty = x.ty.clone();
                Ok(mk(vec![x, f, e], ty, self))
            }
            Prim::Fold => {
                let x = self.check(&args[0], None)?;
                let t = self.expect_array(&x.ty, x.span, "the first argument of `fold`")?;
                let init = self.check(&args[1], expected)?;
                let acc = init.ty.clone();
                let fty = Ty::Fn(vec![acc.clone(), t], Box::new(acc.clone()), false);
                let f = self.check(&args[2], Some(&fty))?;
                self.unify(&fty, &f.ty, f.span)?;
                Ok(mk(vec![x, init, f], acc, self))
            }
            Prim::Zeros => {
                let s = self.fresh_size(span);
                let e = self.fresh(class::NUM | class::SCALAR, LitKind::None, span);
                Ok(mk(vec![], Ty::Array(s, Box::new(e)), self))
            }
            Prim::Iota => {
                let (arg, n) = self.length_arg(&args[0], span)?;
                Ok(mk(vec![arg], Ty::Array(n, Box::new(Ty::Int(IntKind::I64))), self))
            }
            Prim::Fill => {
                let (arg, n) = self.length_arg(&args[0], span)?;
                let hint = match expected.map(|t| self.shallow(t)) {
                    Some(Ty::Array(_, e)) => Some(*e),
                    _ => None,
                };
                let v = self.check(&args[1], hint.as_ref())?;
                self.require(&v.ty, class::ELEM, v.span)?;
                let ty = Ty::Array(n, Box::new(v.ty.clone()));
                Ok(mk(vec![arg, v], ty, self))
            }
            Prim::Sqrt => {
                let x = self.check(&args[0], expected)?;
                self.require(&x.ty, class::FLOAT | class::SCALAR, x.span)?;
                let t = x.ty.clone();
                Ok(mk(vec![x], t, self))
            }
            // `abs` of a number (D155)
            Prim::Math(MathFn::Abs) => {
                let x = self.check(&args[0], expected)?;
                self.require(&x.ty, class::NUM | class::SCALAR, x.span)?;
                let t = x.ty.clone();
                Ok(mk(vec![x], t, self))
            }
            // floats of one type (D108)
            Prim::Math(_) => {
                let first = self.check(&args[0], expected)?;
                self.require(&first.ty, class::FLOAT | class::SCALAR, first.span)?;
                let t = first.ty.clone();
                let mut xs = vec![first];
                for a in &args[1..] {
                    let x = self.check(a, Some(&t))?;
                    self.unify(&t, &x.ty, x.span)?;
                    xs.push(x);
                }
                Ok(mk(xs, t, self))
            }
            Prim::Convert(t) => {
                let x = self.check(&args[0], None)?;
                self.require(&x.ty, class::NUM | class::SCALAR, x.span)?;
                self.deferred_scalar.push((x.ty.clone(), x.span));
                Ok(mk(vec![x], t.to_ty(), self))
            }
            Prim::ReadF64 | Prim::ReadF32 | Prim::ReadBytes => {
                let path = self.check(&args[0], None)?;
                self.unify(&Ty::Str, &path.ty, path.span)?;
                let s = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                let elem = match p {
                    Prim::ReadF64 => Ty::F64,
                    Prim::ReadF32 => Ty::F32,
                    _ => Ty::Int(IntKind::U8),
                };
                Ok(mk(vec![path], Ty::Array(Size::Poly(Poly::sym(s)), Box::new(elem)), self))
            }
            Prim::WriteF64 | Prim::WriteF32 | Prim::WriteBytes => {
                let path = self.check(&args[0], None)?;
                self.unify(&Ty::Str, &path.ty, path.span)?;
                let elem = match p {
                    Prim::WriteF64 => Ty::F64,
                    Prim::WriteF32 => Ty::F32,
                    _ => Ty::Int(IntKind::U8),
                };
                let s = self.fresh_size(span);
                let want = Ty::Array(s, Box::new(elem));
                let x = self.check(&args[1], Some(&want))?;
                self.unify(&want, &x.ty, x.span)?;
                Ok(mk(vec![path, x], Ty::Unit, self))
            }
            Prim::Print => {
                let x = self.check(&args[0], None)?;
                self.deferred_printable.push((x.ty.clone(), x.span));
                Ok(mk(vec![x], Ty::Unit, self))
            }
            Prim::Clock => Ok(mk(vec![], Ty::Int(IntKind::U64), self)),
            // whether the function given is pure: a constant of each call
            // compiled (D386)
            Prim::IsPure => {
                let f = self.check(&args[0], None)?;
                if !matches!(self.shallow(&f.ty), Ty::Fn(..)) {
                    return err(f.span, format!("`isPure` takes a function; this is a `{}`", self.show(&f.ty)));
                }
                Ok(mk(vec![f], Ty::Bool, self))
            }
            Prim::ParseI64 => {
                let x = self.check(&args[0], None)?;
                self.unify(&Ty::Str, &x.ty, x.span)?;
                Ok(mk(vec![x], Ty::Int(IntKind::I64), self))
            }
            Prim::ParseF64 => {
                let x = self.check(&args[0], None)?;
                self.unify(&Ty::Str, &x.ty, x.span)?;
                Ok(mk(vec![x], Ty::F64, self))
            }
            Prim::Concat => {
                let a = self.check(&args[0], None)?;
                // two arrays: `[n]T ++ [m]T -> [n + m]T` (D136)
                if let Ty::Array(sa, ea) = self.shallow(&a.ty) {
                    let (want, _, _) = self.fresh_array(span);
                    let b = self.check(&args[1], Some(&want))?;
                    self.unify(&want, &b.ty, b.span)?;
                    let Ty::Array(sb, eb) = self.shallow(&b.ty) else { unreachable!() };
                    self.unify(&ea, &eb, b.span)?;
                    let size = match (sa, sb) {
                        (Size::Poly(x), Size::Poly(y)) => Size::Poly(x.add(&y)),
                        _ => {
                            let s = self.syms.len() as SymId;
                            self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                            Size::Poly(Poly::sym(s))
                        }
                    };
                    return Ok(mk(vec![a, b], Ty::Array(size, ea), self));
                }
                self.unify(&Ty::Str, &a.ty, a.span)?;
                let b = self.check(&args[1], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &b.ty, b.span)?;
                Ok(mk(vec![a, b], Ty::Str, self))
            }
            // the fallible forms (D138): `(value, message)`, a message for
            // a write
            Prim::OrError(f) => {
                let path = self.check(&args[0], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &path.ty, path.span)?;
                let mut checked = vec![path];
                let exist = |c: &mut Self, tag: &str| {
                    let s = c.syms.len() as SymId;
                    c.syms.push(SymInfo { name: format!("{}@{}:{}", tag, span.line, span.col), origin: SymOrigin::Existential(span) });
                    Size::Poly(Poly::sym(s))
                };
                let value = match f {
                    Fallible::ParseI64 => Ty::Int(IntKind::I64),
                    Fallible::ParseF64 => Ty::F64,
                    Fallible::ReadBytes => Ty::Array(exist(self, "k"), Box::new(Ty::Int(IntKind::U8))),
                    Fallible::ReadF64 => Ty::Array(exist(self, "k"), Box::new(Ty::F64)),
                    Fallible::ReadText => Ty::Str,
                    Fallible::ReadCsvF64 => {
                        let k = self.check(&args[1], Some(&Ty::Int(IntKind::I64)))?;
                        self.unify(&Ty::Int(IntKind::I64), &k.ty, k.span)?;
                        checked.push(k);
                        let r = exist(self, "r");
                        let c = exist(self, "c");
                        Ty::Array(r, Box::new(Ty::Array(c, Box::new(Ty::F64))))
                    }
                    Fallible::WriteBytes | Fallible::WriteText => {
                        let want = if f == Fallible::WriteText {
                            Ty::Str
                        } else {
                            let s = self.fresh_size(span);
                            Ty::Array(s, Box::new(Ty::Int(IntKind::U8)))
                        };
                        let x = self.check(&args[1], Some(&want))?;
                        self.unify(&want, &x.ty, x.span)?;
                        checked.push(x);
                        return Ok(mk(checked, Ty::Str, self));
                    }
                };
                Ok(mk(checked, Ty::Tuple(vec![value, Ty::Str]), self))
            }
            Prim::PrintBytes => {
                let s = self.fresh_size(span);
                let want = Ty::Array(s, Box::new(Ty::Int(IntKind::U8)));
                let x = self.check(&args[0], Some(&want))?;
                self.unify(&want, &x.ty, x.span)?;
                Ok(mk(vec![x], Ty::Unit, self))
            }
            // the order of the elements (D155): integers, floats, booleans,
            // strings
            Prim::Sys(_) => unreachable!(),
            Prim::Group | Prim::GroupCounts => {
                let x = self.check(&args[0], None)?;
                let elem = self.expect_array(&x.ty, x.span, "the keys of `$group`")?;
                self.require(&elem, class::KEY, x.span)?;
                let s = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                Ok(mk(vec![x], Ty::Array(Size::Poly(Poly::sym(s)), Box::new(Ty::Int(IntKind::I64))), self))
            }
            Prim::ArgSort => {
                let x = self.check(&args[0], None)?;
                let elem = self.expect_array(&x.ty, x.span, "the argument of `argsort`")?;
                self.require(&elem, class::ORD, x.span)?;
                let size = match self.shallow(&x.ty) {
                    Ty::Array(s, _) => s,
                    _ => self.fresh_size(span),
                };
                Ok(mk(vec![x], Ty::Array(size, Box::new(Ty::Int(IntKind::I64))), self))
            }
            // `BigInt(x)` of an integer, or of a text (a panic if it is not
            // an integer) (D156)
            Prim::ToBigInt => {
                let x = self.check(&args[0], None)?;
                match self.shallow(&x.ty) {
                    Ty::Str | Ty::BigInt => {}
                    _ => self.require(&x.ty, class::INTEGRAL | class::SCALAR, x.span)?,
                }
                Ok(mk(vec![x], Ty::BigInt, self))
            }
            // ECMAScript's conversions (D156): `Number` of a text or of a number
            Prim::JsNumber => {
                let x = self.check(&args[0], None)?;
                if self.shallow(&x.ty) != Ty::Str {
                    self.require(&x.ty, class::NUM | class::SCALAR, x.span)?;
                }
                Ok(mk(vec![x], Ty::F64, self))
            }
            Prim::ParseFloat | Prim::ParseInt => {
                let a = self.check(&args[0], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &a.ty, a.span)?;
                let mut xs = vec![a];
                if p == Prim::ParseInt {
                    let i64t = Ty::Int(IntKind::I64);
                    let r = match args.get(1) {
                        Some(r) => {
                            let r = self.check(r, Some(&i64t))?;
                            self.unify(&i64t, &r.ty, r.span)?;
                            r
                        }
                        None => self.mk(TExprKind::Lit(Lit::Int(0)), i64t, span),
                    };
                    xs.push(r);
                }
                Ok(mk(xs, Ty::F64, self))
            }
            // the standard input (D155)
            Prim::ReadStdin => {
                let s = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                Ok(mk(vec![], Ty::Array(Size::Poly(Poly::sym(s)), Box::new(Ty::Int(IntKind::U8))), self))
            }
            Prim::ReadStdinText => Ok(mk(vec![], Ty::Str, self)),
            Prim::ReadStdinLines => {
                let s = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                Ok(mk(vec![], Ty::Array(Size::Poly(Poly::sym(s)), Box::new(Ty::Str)), self))
            }
            // the functions of strings of TypeScript (D155)
            Prim::ToUpper | Prim::ToLower | Prim::TrimStart | Prim::TrimEnd => {
                let a = self.check(&args[0], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &a.ty, a.span)?;
                Ok(mk(vec![a], Ty::Str, self))
            }
            // regular expressions (D157)
            Prim::RegExpNew => {
                let a = self.check(&args[0], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &a.ty, a.span)?;
                let f = match args.get(1) {
                    Some(f) => {
                        let f = self.check(f, Some(&Ty::Str))?;
                        self.unify(&Ty::Str, &f.ty, f.span)?;
                        f
                    }
                    None => self.mk(TExprKind::Lit(Lit::Str(String::new())), Ty::Str, span),
                };
                Ok(mk(vec![a, f], Ty::RegExp, self))
            }
            // `$mapOf(keys, values, order)` and `$mapParts(m)` (D158)
            Prim::MapOf => {
                let ks = self.check(&args[0], None)?;
                let k = self.expect_array(&ks.ty, ks.span, "the keys of a Map")?;
                self.require(&k, class::KEY, ks.span)?;
                let vs = self.check(&args[1], None)?;
                let v = self.expect_array(&vs.ty, vs.span, "the values of a Map")?;
                let ord = self.check(&args[2], None)?;
                let n = match self.shallow(&ks.ty) {
                    Ty::Array(s, _) => s,
                    _ => unreachable!(),
                };
                self.unify(&Ty::Array(n.clone(), Box::new(v.clone())), &vs.ty, vs.span)?;
                self.unify(&Ty::Array(n, Box::new(Ty::Int(IntKind::I64))), &ord.ty, ord.span)?;
                let t = self.map_ty(k, v, span)?;
                Ok(mk(vec![ks, vs, ord], t, self))
            }
            Prim::MapParts | Prim::MapSize => {
                let m = self.check(&args[0], None)?;
                let (k, v) = match self.map_args(&m.ty) {
                    Some(kv) => kv,
                    None => return err(m.span, format!("expected a Map, found `{}`", self.show(&m.ty))),
                };
                if p == Prim::MapSize {
                    return Ok(mk(vec![m], Ty::Int(IntKind::I64), self));
                }
                let s = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                let n = Size::Poly(Poly::sym(s));
                let t = Ty::Tuple(vec![Ty::Array(n.clone(), Box::new(k)), Ty::Array(n.clone(), Box::new(v)), Ty::Array(n, Box::new(Ty::Int(IntKind::I64)))]);
                Ok(mk(vec![m], t, self))
            }
            Prim::RxTest => {
                let r = self.check(&args[0], Some(&Ty::RegExp))?;
                self.unify(&Ty::RegExp, &r.ty, r.span)?;
                let x = self.check(&args[1], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &x.ty, x.span)?;
                Ok(mk(vec![r, x], Ty::Bool, self))
            }
            Prim::RxSearch | Prim::RxCount | Prim::RxMatches => {
                let x = self.check(&args[0], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &x.ty, x.span)?;
                let r = self.check(&args[1], Some(&Ty::RegExp))?;
                self.unify(&Ty::RegExp, &r.ty, r.span)?;
                let t = if p == Prim::RxMatches {
                    let s = self.syms.len() as SymId;
                    self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                    Ty::Array(Size::Poly(Poly::sym(s)), Box::new(Ty::Str))
                } else {
                    Ty::Int(IntKind::I64)
                };
                Ok(mk(vec![x, r], t, self))
            }
            // `s.replace(p, to)`, `s.replaceAll(p, to)`: `p` a text or a
            // `RegExp`
            Prim::Replace | Prim::ReplaceAll => {
                let x = self.check(&args[0], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &x.ty, x.span)?;
                let pat = self.check(&args[1], None)?;
                if self.shallow(&pat.ty) != Ty::RegExp {
                    self.unify(&Ty::Str, &pat.ty, pat.span)?;
                }
                // `replaceAll` of a RegExp whose flags are known here: `g`
                // checked now (else at run time, D157)
                if p == Prim::ReplaceAll {
                    if let TExprKind::Call { callee: Callee::Prim(Prim::RegExpNew), args: ra, .. } = &pat.kind {
                        if let Some(TExprKind::Lit(Lit::Str(f))) = ra.get(1).map(|a| &a.kind) {
                            if !f.contains('g') {
                                return err(span, "replaceAll: the RegExp must have the flag g");
                            }
                        }
                    }
                }
                let to = self.check(&args[2], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &to.ty, to.span)?;
                Ok(mk(vec![x, pat, to], Ty::Str, self))
            }
            Prim::IndexOf | Prim::LastIndexOf | Prim::Includes | Prim::StartsWith | Prim::EndsWith => {
                let mut xs = Vec::new();
                for a in args {
                    let x = self.check(a, Some(&Ty::Str))?;
                    self.unify(&Ty::Str, &x.ty, x.span)?;
                    xs.push(x);
                }
                let t = match p {
                    Prim::IndexOf | Prim::LastIndexOf => Ty::Int(IntKind::I64),
                    Prim::ReplaceAll => Ty::Str,
                    _ => Ty::Bool,
                };
                Ok(mk(xs, t, self))
            }
            Prim::PadStart | Prim::PadEnd | Prim::Repeat | Prim::StrSlice => {
                let a = self.check(&args[0], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &a.ty, a.span)?;
                let i64t = Ty::Int(IntKind::I64);
                let b = self.check(&args[1], Some(&i64t))?;
                self.unify(&i64t, &b.ty, b.span)?;
                let mut xs = vec![a, b];
                match p {
                    Prim::StrSlice => {
                        let c = self.check(&args[2], Some(&i64t))?;
                        self.unify(&i64t, &c.ty, c.span)?;
                        xs.push(c);
                    }
                    Prim::PadStart | Prim::PadEnd => {
                        let c = match args.get(2) {
                            Some(f) => {
                                let c = self.check(f, Some(&Ty::Str))?;
                                self.unify(&Ty::Str, &c.ty, c.span)?;
                                c
                            }
                            None => self.mk(TExprKind::Lit(Lit::Str(" ".into())), Ty::Str, span),
                        };
                        xs.push(c);
                    }
                    _ => {}
                }
                Ok(mk(xs, Ty::Str, self))
            }
            Prim::ToFixed => {
                let x = self.check(&args[0], None)?;
                self.require(&x.ty, class::FLOAT | class::SCALAR, x.span)?;
                let i64t = Ty::Int(IntKind::I64);
                let d = self.check(&args[1], Some(&i64t))?;
                self.unify(&i64t, &d.ty, d.span)?;
                Ok(mk(vec![x, d], Ty::Str, self))
            }
            // text (D135)
            Prim::ReadText | Prim::Trim => {
                let a = self.check(&args[0], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &a.ty, a.span)?;
                Ok(mk(vec![a], Ty::Str, self))
            }
            Prim::WriteText | Prim::Split => {
                let a = self.check(&args[0], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &a.ty, a.span)?;
                let b = self.check(&args[1], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &b.ty, b.span)?;
                if p == Prim::WriteText {
                    return Ok(mk(vec![a, b], Ty::Unit, self));
                }
                let s = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                Ok(mk(vec![a, b], Ty::Array(Size::Poly(Poly::sym(s)), Box::new(Ty::Str)), self))
            }
            Prim::Lines => {
                let a = self.check(&args[0], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &a.ty, a.span)?;
                let s = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                Ok(mk(vec![a], Ty::Array(Size::Poly(Poly::sym(s)), Box::new(Ty::Str)), self))
            }
            Prim::Join => {
                let s = self.fresh_size(span);
                let want = Ty::Array(s, Box::new(Ty::Str));
                let a = self.check(&args[0], Some(&want))?;
                self.unify(&want, &a.ty, a.span)?;
                let b = self.check(&args[1], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &b.ty, b.span)?;
                Ok(mk(vec![a, b], Ty::Str, self))
            }
            Prim::ParseCsvF64 | Prim::ReadCsvF64 => {
                let a = self.check(&args[0], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &a.ty, a.span)?;
                let k = self.check(&args[1], Some(&Ty::Int(IntKind::I64)))?;
                self.unify(&Ty::Int(IntKind::I64), &k.ty, k.span)?;
                let r = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("r@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                let c = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("c@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                let row = Ty::Array(Size::Poly(Poly::sym(c)), Box::new(Ty::F64));
                Ok(mk(vec![a, k], Ty::Array(Size::Poly(Poly::sym(r)), Box::new(row)), self))
            }
            // the bytes of a string, the elements of an array (D135)
            Prim::StrLen => {
                let a = self.check(&args[0], None)?;
                // `r.length` of a record with a field `length`: the field
                if name == "$length" {
                    if let Ty::Record(fs) = self.shallow(&a.ty) {
                        if fs.iter().any(|(n, _)| n == "length") {
                            let (i, ty) = self.field_of(&a.ty, "length", a.span)?;
                            let ty = self.open_own(&ty, span);
                            return Ok(self.mk(TExprKind::Field(Box::new(a), i), ty, span));
                        }
                    }
                }
                match self.shallow(&a.ty) {
                    Ty::Array(..) | Ty::Str => {}
                    Ty::Var(_) => self.deferred_len.push((a.ty.clone(), a.span)),
                    _ => self.unify(&Ty::Str, &a.ty, a.span)?,
                }
                Ok(mk(vec![a], Ty::Int(IntKind::I64), self))
            }
            Prim::Bytes => {
                let a = self.check(&args[0], Some(&Ty::Str))?;
                self.unify(&Ty::Str, &a.ty, a.span)?;
                let s = self.syms.len() as SymId;
                self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
                Ok(mk(vec![a], Ty::Array(Size::Poly(Poly::sym(s)), Box::new(Ty::Int(IntKind::U8))), self))
            }
            Prim::StringOf => {
                let (arr, _, _) = self.fresh_array(span);
                let b = self.check(&args[0], Some(&arr))?;
                self.unify(&arr, &b.ty, b.span)?;
                let Ty::Array(_, el) = self.shallow(&arr) else { unreachable!() };
                self.unify(&Ty::Int(IntKind::U8), &el, b.span)?;
                Ok(mk(vec![b], Ty::Str, self))
            }
            Prim::ToString => {
                let x = self.check(&args[0], None)?;
                // the text of a string is itself (as `String(s)`)
                if self.shallow(&x.ty) == Ty::Str {
                    return Ok(x);
                }
                let t = self.fresh(class::SCALAR, LitKind::None, x.span);
                self.unify(&t, &x.ty, x.span)?;
                Ok(mk(vec![x], Ty::Str, self))
            }
            Prim::Bench => {
                let r = self.check(&args[0], Some(&Ty::Int(IntKind::I64)))?;
                self.unify(&Ty::Int(IntKind::I64), &r.ty, r.span)?;
                let u = self.fresh(0, LitKind::None, span);
                let fty = Ty::Fn(vec![], Box::new(u.clone()), false);
                let f = self.check(&args[1], Some(&fty))?;
                self.unify(&fty, &f.ty, f.span)?;
                Ok(mk(vec![r, f], Ty::Tuple(vec![u, Ty::Int(IntKind::U64)]), self))
            }
        }
    }

    /// The argument of `iota`/`fill`: a size expression written as an
    /// expression (integer literals and size variables with `+ - *`).
    /// The size variable named `name`, if no local hides it; a `size`
    /// parameter is both a local `i64` and a size variable.
    fn size_var(&self, name: &str) -> Option<SymId> {
        let s = self.sym_by_name(name)?;
        match self.lookup(name) {
            None => Some(s),
            Some(l) => match self.syms[s as usize].origin {
                // the parameters are the first locals of the function
                SymOrigin::SizeParam(i) if l as usize == i => Some(s),
                _ => None,
            },
        }
    }

    /// `size_arg` without an error: None if `e` is not a size expression.
    fn try_size_arg(&mut self, e: &ast::Expr) -> Option<Poly> {
        match &e.kind {
            ExprKind::Int { value, neg: false, suffix: None } if *value <= i64::MAX as u128 => Some(Poly::constant(*value as i64)),
            ExprKind::Var(name) => self.size_var(name).map(Poly::sym),
            ExprKind::Binary(op @ (BinOp::Add | BinOp::Sub | BinOp::Mul), a, b) => {
                let a = self.try_size_arg(a)?;
                let b = self.try_size_arg(b)?;
                Some(match op {
                    BinOp::Add => a.add(&b),
                    BinOp::Sub => a.sub(&b),
                    _ => a.mul(&b),
                })
            }
            _ => None,
        }
    }

    /// The length of `iota` and `fill`: a size (its type knows it), or any
    /// `i64`, a size known at run time (D155: `iota(n)` of a `const n`),
    /// checked positive or zero when the arrays are built.
    fn length_arg(&mut self, e: &ast::Expr, span: Span) -> Result<(TExpr, Size)> {
        let saved = self.syms.len();
        if let Ok(n) = self.size_arg(e) {
            let arg = self.mk(TExprKind::SizeVal(n.clone()), Ty::Int(IntKind::I64), e.span);
            return Ok((arg, Size::Poly(n)));
        }
        self.syms.truncate(saved);
        let i64t = Ty::Int(IntKind::I64);
        let x = self.check(e, Some(&i64t))?;
        self.unify(&i64t, &x.ty, x.span)?;
        let s = self.syms.len() as SymId;
        self.syms.push(SymInfo { name: format!("k@{}:{}", span.line, span.col), origin: SymOrigin::Existential(span) });
        Ok((x, Size::Poly(Poly::sym(s))))
    }

    fn size_arg(&mut self, e: &ast::Expr) -> Result<Poly> {
        match &e.kind {
            ExprKind::Int { value, neg: false, suffix: None } if *value <= i64::MAX as u128 => {
                Ok(Poly::constant(*value as i64))
            }
            ExprKind::Var(name) => match self.size_var(name) {
                Some(s) => Ok(Poly::sym(s)),
                None => err(e.span, format!("`{}` is not a size variable", name)),
            },
            ExprKind::Binary(op @ (BinOp::Add | BinOp::Sub | BinOp::Mul), a, b) => {
                let a = self.size_arg(a)?;
                let b = self.size_arg(b)?;
                Ok(match op {
                    BinOp::Add => a.add(&b),
                    BinOp::Sub => a.sub(&b),
                    _ => a.mul(&b),
                })
            }
            _ => err(
                e.span,
                "expected a size: an integer literal, a size variable, or a sum or product of these",
            ),
        }
    }

    // ----- final resolution ----------------------------------------------

    fn zonk(&mut self, t: &Ty) -> Result<Ty> {
        Ok(match self.shallow(t) {
            Ty::Var(v) => {
                let tv = self.tvars[v as usize].clone();
                let d = if tv.lit == LitKind::Float || tv.classes & class::FLOAT != 0 {
                    Ty::F64
                } else if tv.lit == LitKind::Int || tv.classes & class::INTEGRAL != 0 {
                    Ty::Int(IntKind::I64)
                } else if tv.classes & class::NUM != 0 {
                    return err(tv.span, "cannot infer the numeric type of this expression; add a type annotation");
                } else if tv.classes & (class::SCALAR | class::ELEM) != 0 {
                    return err(tv.span, "cannot infer the element type of this array; add a type annotation");
                } else {
                    Ty::Unit
                };
                self.tvars[v as usize].bound = Some(d.clone());
                d
            }
            Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| self.zonk(t)).collect::<Result<_>>()?),
            Ty::Record(fs) => Ty::Record(fs.iter().map(|(n, t)| Ok((n.clone(), self.zonk(t)?))).collect::<Result<_>>()?),
            Ty::Sum { id, name, args } => {
                Ty::Sum { id, name, args: args.iter().map(|t| self.zonk(t)).collect::<Result<_>>()? }
            }
            Ty::Array(s, e) => {
                let e = self.zonk(&e)?;
                match self.size_res(&s) {
                    Size::Own => Ty::Array(Size::Own, Box::new(e)),
                    Size::Poly(p) => Ty::Array(Size::Poly(p), Box::new(e)),
                    Size::Var(v) => {
                        return err(self.svars[v as usize].1, "cannot infer the size of this array; add a type annotation")
                    }
                }
            }
            Ty::Fn(ps, r, act) => Ty::Fn(ps.iter().map(|t| self.zonk(t)).collect::<Result<_>>()?, Box::new(self.zonk(&r)?), act),
            t => t,
        })
    }

    fn zonk_expr(&mut self, e: &mut TExpr) -> Result<()> {
        e.ty = self.zonk(&e.ty)?;
        match &mut e.kind {
            TExprKind::Lit(Lit::Int(v)) => match &e.ty {
                Ty::Int(k) => {
                    if *v < k.min_value() || *v > k.max_value() {
                        return err(e.span, format!("the literal `{}` is out of range for `{}`", v, k.name()));
                    }
                }
                Ty::F64 | Ty::F32 | Ty::Param(_) => {}
                t => return err(e.span, format!("an integer literal cannot have type `{}`", self.show(t))),
            },
            TExprKind::Lit(_) | TExprKind::Local(_) | TExprKind::SizeVal(_) | TExprKind::PrimRef(_) => {}
            TExprKind::OpRef(_) => {}
            TExprKind::FnRef { type_args, .. } => {
                for t in type_args.iter_mut() {
                    *t = self.zonk(t)?;
                }
            }
            TExprKind::Tuple(es) | TExprKind::Array(es) => {
                for x in es.iter_mut() {
                    self.zonk_expr(x)?;
                }
            }
            TExprKind::Call { callee, args, .. } => {
                match callee {
                    Callee::Fn { id, type_args } => {
                        for t in type_args.iter_mut() {
                            *t = self.zonk(t)?;
                        }
                        // `json.decode<T>`, `json.encode(x)`: the function
                        // written for `T` (D336)
                        let (d, en) = self.g.placeholders;
                        if !self.g.codecs.is_empty() && (Some(*id) == d || Some(*id) == en) {
                            if let Some(k) = type_args.first().and_then(|t| crate::codec::codec_key(t, &self.g.sums)) {
                                if let Some(&(dec, enc)) = self.g.codecs.get(&k) {
                                    *id = if Some(*id) == d { dec } else { enc };
                                    type_args.clear();
                                }
                            }
                        }
                    }
                    Callee::Value(f) => self.zonk_expr(f)?,
                    Callee::Prim(_) | Callee::Extern(_) => {}
                }
                for x in args.iter_mut() {
                    self.zonk_expr(x)?;
                }
            }
            TExprKind::Index(a, b) | TExprKind::Pow(a, b) => {
                self.zonk_expr(a)?;
                self.zonk_expr(b)?;
            }
            TExprKind::Slice(x, a, b) => {
                self.zonk_expr(x)?;
                self.zonk_expr(a)?;
                self.zonk_expr(b)?;
            }
            TExprKind::Binary { lhs, rhs, .. } => {
                self.zonk_expr(lhs)?;
                self.zonk_expr(rhs)?;
            }
            TExprKind::Update { target, index, value, .. } => {
                self.zonk_expr(target)?;
                self.zonk_expr(index)?;
                self.zonk_expr(value)?;
            }
            TExprKind::Field(x, _) => self.zonk_expr(x)?,
            TExprKind::Construct { args, .. } => {
                for a in args.iter_mut() {
                    self.zonk_expr(a)?;
                }
            }
            TExprKind::Match(x, arms) => {
                self.zonk_expr(x)?;
                for (p, b) in arms.iter_mut() {
                    self.zonk_pat(p)?;
                    self.zonk_expr(b)?;
                }
            }
            TExprKind::FieldUpdate { target, value, .. } => {
                self.zonk_expr(target)?;
                self.zonk_expr(value)?;
            }
            TExprKind::Unary(_, x) => self.zonk_expr(x)?,
            TExprKind::If(c, t, f) => {
                self.zonk_expr(c)?;
                self.zonk_expr(t)?;
                self.zonk_expr(f)?;
            }
            TExprKind::Lambda(_, b) => self.zonk_expr(b)?,
            TExprKind::Block(stmts, r) => {
                for s in stmts.iter_mut() {
                    match s {
                        TStmt::Let(p, v) => {
                            self.zonk_expr(v)?;
                            self.zonk_pat(p)?;
                        }
                        TStmt::Expr(v) => self.zonk_expr(v)?,
                    }
                }
                self.zonk_expr(r)?;
            }
        }
        Ok(())
    }

    fn zonk_pat(&mut self, p: &mut TPat) -> Result<()> {
        match p {
            TPat::Wild(t) => *t = self.zonk(t)?,
            TPat::Bind(_) => {}
            TPat::Tuple(ps, t) | TPat::Ctor { subs: ps, ty: t, .. } => {
                *t = self.zonk(t)?;
                for q in ps.iter_mut() {
                    self.zonk_pat(q)?;
                }
            }
            TPat::Lit(l, t) => {
                *t = self.zonk(t)?;
                if let (Lit::Int(v), Ty::Int(k)) = (&*l, &*t) {
                    if *v < k.min_value() || *v > k.max_value() {
                        return err(Span::new(0, 1, 1), format!("the literal `{}` is out of range for `{}`", v, k.name()));
                    }
                }
            }
        }
        Ok(())
    }
}

fn subst_value(t: &Ty, type_args: &[Ty], sizes: &[Size]) -> Ty {
    match t {
        Ty::Param(p) => type_args[*p as usize].clone(),
        Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| subst_value(t, type_args, sizes)).collect()),
        Ty::Record(fs) => Ty::Record(fs.iter().map(|(n, t)| (n.clone(), subst_value(t, type_args, sizes))).collect()),
        Ty::Sum { id, name, args } => {
            Ty::Sum { id: *id, name: name.clone(), args: args.iter().map(|t| subst_value(t, type_args, sizes)).collect() }
        }
        Ty::Fn(ps, r, act) => Ty::Fn(
            ps.iter().map(|t| subst_value(t, type_args, sizes)).collect(),
            Box::new(subst_value(r, type_args, sizes)),
        *act),
        Ty::Array(Size::Poly(p), e) => {
            let size = match p.as_sym() {
                Some(s) => sizes[s as usize].clone(),
                None => match p.as_const() {
                    Some(_) => Size::Poly(p.clone()),
                    // compound sizes are not tracked through function values
                    None => sizes.last().cloned().unwrap_or(Size::Poly(p.clone())),
                },
            };
            Ty::Array(size, Box::new(subst_value(e, type_args, sizes)))
        }
        other => other.clone(),
    }
}

/// Whether a value of type `t` holds a resource of the system (D339).
pub fn holds_handle(t: &Ty) -> bool {
    match t {
        Ty::Handle(_) => true,
        Ty::Tuple(ts) => ts.iter().any(holds_handle),
        Ty::Record(fs) => fs.iter().any(|(_, t)| holds_handle(t)),
        Ty::Sum { args, .. } => args.iter().any(holds_handle),
        Ty::Array(_, e) => holds_handle(e),
        _ => false,
    }
}

/// Whether a value of type `t` holds a function, in the payload of a sum
/// type too (D153).
fn holds_function(g: &Globals, t: &Ty, seen: &mut Vec<u32>) -> bool {
    match t {
        Ty::Fn(..) => true,
        Ty::Tuple(ts) => ts.iter().any(|x| holds_function(g, x, seen)),
        Ty::Record(fs) => fs.iter().any(|(_, x)| holds_function(g, x, seen)),
        Ty::Array(_, e) => holds_function(g, e, seen),
        Ty::Sum { id, args, .. } => {
            if args.iter().any(|x| holds_function(g, x, seen)) {
                return true;
            }
            if seen.contains(id) {
                return false;
            }
            seen.push(*id);
            let def = &g.sums[*id as usize];
            (0..def.variants.len()).any(|k| def.payload(k, args).iter().any(|x| holds_function(g, x, seen)))
        }
        _ => false,
    }
}

/// `t` holds the type of an action (D352).
fn contains_action(t: &Ty) -> bool {
    match t {
        Ty::Fn(ps, r, act) => *act || ps.iter().any(contains_action) || contains_action(r),
        Ty::Tuple(ts) => ts.iter().any(contains_action),
        Ty::Record(fs) => fs.iter().any(|(_, t)| contains_action(t)),
        Ty::Sum { args, .. } => args.iter().any(contains_action),
        Ty::Array(_, e) => contains_action(e),
        _ => false,
    }
}

fn contains_fn(t: &Ty) -> bool {
    match t {
        Ty::Fn(..) => true,
        Ty::Tuple(ts) => ts.iter().any(contains_fn),
        Ty::Record(fs) => fs.iter().any(|(_, t)| contains_fn(t)),
        Ty::Sum { args, .. } => args.iter().any(contains_fn),
        Ty::Array(_, e) => contains_fn(e),
        _ => false,
    }
}

// ---------------------------------------------------------------------
// Program

/// Checks a program. `prelude` is the standard library written in
/// Isocron; user declarations shadow prelude declarations of the same name.
/// The prefix of the functions of the prelude that a program declares again,
/// kept for the modules of the standard library (D337).
const PRELUDE_KEPT: &str = "std:prelude.";

/// The name of the placeholders `decode` and `encode` of the module `json`
/// (D336).
const DECODE: &str = "std:json.decode";
const ENCODE: &str = "std:json.encode";

pub fn check_program(user: &ast::Program, prelude: &ast::Program) -> Result<TProgram> {
    let first = check_program_with(user, prelude, &HashMap::new())?;
    // `json.decode<T>` and `json.encode(x)` (D325, D336): the functions of
    // each `T`, written in the module `json`, then the program checked again
    // with them, each call given those of its type
    let ids: Vec<(usize, bool)> = first.fns.iter().enumerate().filter(|(_, f)| f.name == DECODE || f.name == ENCODE).map(|(i, f)| (i, f.name == DECODE)).collect();
    if ids.is_empty() {
        return Ok(first);
    }
    let mut wanted: Vec<(Ty, Span, bool)> = Vec::new();
    fn calls(e: &TExpr, ids: &[(usize, bool)], out: &mut Vec<(Ty, Span, bool)>) {
        if let TExprKind::Call { callee: Callee::Fn { id, type_args }, .. } = &e.kind {
            if let Some((_, decode)) = ids.iter().find(|(i, _)| i == id) {
                if let Some(t) = type_args.first() {
                    out.push((t.clone(), e.span, *decode));
                }
            }
        }
        for_each_child(e, &mut |c| calls(c, ids, out));
    }
    for f in &first.fns {
        calls(&f.body, &ids, &mut wanted);
    }
    if wanted.is_empty() {
        return Ok(first);
    }
    let (src, keys) = crate::codec::codec_source(&wanted, &first.sums)?;
    let file = user
        .decls
        .iter()
        .find_map(|d| match d {
            ast::Decl::Fn(f) if f.name == DECODE => Some(f.span.file),
            _ => None,
        })
        .unwrap_or(crate::USER_FILE);
    let generated = isc_syntax::parse(&src, file).map_err(|d| Diagnostic::new(d.span, format!("the functions of JSON written by the compiler: {} (a defect of the compiler)", d.message)))?;
    let mut with = user.clone();
    for d in generated.decls {
        if let ast::Decl::Fn(mut f) = d {
            f.name = format!("std:json.{}", f.name);
            with.decls.push(ast::Decl::Fn(f));
        }
    }
    check_program_with(&with, prelude, &keys)
}

fn check_program_with(user: &ast::Program, prelude: &ast::Program, codecs: &HashMap<String, usize>) -> Result<TProgram> {
    // the types of the prelude (`Result`, D138) after those of the program,
    // unless the program declares one of their names: then they, and the
    // functions of the prelude whose signature mentions them, are left out
    let mut taken: HashSet<String> = HashSet::new();
    for d in &user.decls {
        match d {
            ast::Decl::Fn(f) => {
                taken.insert(f.name.clone());
            }
            ast::Decl::Extern(e) => {
                taken.insert(e.name.clone());
            }
            ast::Decl::Type(t) => {
                taken.insert(t.name.clone());
                taken.extend(t.variants.iter().map(|v| v.name.clone()));
            }
            ast::Decl::Import(..) => {}
        }
    }
    let mut imports: Imports = HashMap::new();
    for (file, local, q) in &user.imported {
        imports.entry(*file).or_default().insert(local.clone(), q.clone());
    }
    // the type aliases replaced (D159)
    let mut expanded = user.clone();
    expand_aliases(&mut expanded, &imports)?;
    let user = &expanded;
    let mut merged = user.clone();
    let mut dropped: Vec<String> = Vec::new();
    for d in &prelude.decls {
        if let ast::Decl::Type(t) = d {
            if taken.contains(&t.name) || t.variants.iter().any(|v| taken.contains(&v.name)) {
                dropped.push(t.name.clone());
            } else {
                merged.decls.push(d.clone());
            }
        }
    }
    fn mentions(t: &ast::TypeExpr, names: &[String]) -> bool {
        use ast::TypeKind as K;
        match &t.kind {
            K::Named(n, args) => names.contains(n) || args.iter().any(|a| mentions(a, names)),
            K::Array(_, e) | K::BoundedArray(_, _, e) => mentions(e, names),
            K::Tuple(ts) => ts.iter().any(|a| mentions(a, names)),
            K::Record(fs) => fs.iter().any(|(_, a)| mentions(a, names)),
            K::Fn(ps, r, _) => ps.iter().any(|a| mentions(a, names)) || mentions(r, names),
            K::Scalar(_) | K::Str | K::BigInt | K::Unit => false,
        }
    }
    let (sums, heads, ctors) = check_type_decls(&merged, &imports)?;
    let mut decls: Vec<(&ast::FnDecl, bool)> = Vec::new();
    let mut names: HashMap<String, FnId> = HashMap::new();
    for d in &user.decls {
        match d {
            ast::Decl::Fn(f) => {
                if names.contains_key(&f.name) {
                    return err(f.span, format!("duplicate declaration of `{}`", f.name));
                }
                names.insert(f.name.clone(), decls.len());
                decls.push((f, false));
            }
            ast::Decl::Extern(_) | ast::Decl::Type(_) | ast::Decl::Import(..) => {}
        }
    }
    // a function of the prelude that the program declares again: replaced
    // in the program, kept for the modules of the standard library under
    // `std:prelude.` (D337)
    let mut kept: Vec<ast::FnDecl> = Vec::new();
    for d in &prelude.decls {
        if let ast::Decl::Fn(f) = d {
            let uses_dropped = !dropped.is_empty()
                && (mentions(&f.ret, &dropped)
                    || f.params.iter().any(|p| matches!(&p.ty, ast::ParamType::Type(t) if mentions(t, &dropped))));
            if uses_dropped {
                continue;
            }
            if !names.contains_key(&f.name) {
                names.insert(f.name.clone(), decls.len());
                decls.push((f, true));
            } else {
                kept.push(ast::FnDecl { name: format!("{}{}", PRELUDE_KEPT, f.name), ..f.clone() });
            }
        }
    }
    for f in &kept {
        names.insert(f.name.clone(), decls.len());
        decls.push((f, true));
    }
    for (d, _) in &decls {
        if ctors.contains_key(&d.name) {
            return err(d.span, format!("`{}` is both a function and a constructor", d.name));
        }
    }
    let mut sigs = Vec::new();
    for (d, _) in &decls {
        sigs.push(sig_of(d, &heads, &imports)?);
    }
    // the `extern` functions after the functions (D125)
    let first_extern = sigs.len();
    let mut externs = Vec::new();
    for d in &user.decls {
        if let ast::Decl::Extern(e) = d {
            if names.contains_key(&e.name) && names[&e.name] < first_extern && !decls[names[&e.name]].1 {
                return err(e.span, format!("duplicate declaration of `{}`", e.name));
            }
            if externs.iter().any(|x: &TExtern| x.name == e.name) {
                return err(e.span, format!("duplicate declaration of `{}`", e.name));
            }
            if ctors.contains_key(&e.name) {
                return err(e.span, format!("`{}` is both an extern function and a constructor", e.name));
            }
            if e.name == "main" || e.name.starts_with("isc_") || e.name.starts_with("llvm") {
                return err(e.span, format!("`{}` cannot be the name of an extern function (it is reserved)", e.name));
            }
            let decl = ast::FnDecl {
                is_action: true,
                name: e.name.clone(),
                span: e.span,
                generics: Vec::new(),
                params: e.params.clone(),
                ret: e.ret.clone(),
                modifiers: Vec::new(),
                body: ast::Expr { kind: ast::ExprKind::Unit, span: e.span },
                is_const: false,
            };
            let sig = sig_of(&decl, &heads, &imports)?;
            check_extern_sig(e, &sig)?;
            externs.push(TExtern { name: e.name.clone(), span: e.span, params: sig.param_tys.clone(), ret: sig.ret.clone() });
            names.insert(e.name.clone(), sigs.len());
            sigs.push(sig);
        }
    }
    // the functions written for the types of `decode` and `encode` (D336)
    let mut codec_ids: HashMap<String, (FnId, FnId)> = HashMap::new();
    for (k, i) in codecs {
        if let (Some(&d), Some(&e)) = (names.get(&format!("std:json.codecDecode{}", i)), names.get(&format!("std:json.codecEncode{}", i))) {
            codec_ids.insert(k.clone(), (d, e));
        }
    }
    let placeholders = (names.get(DECODE).copied(), names.get(ENCODE).copied());
    let globals = Globals { sigs, names, first_extern, sums, heads, ctors, imports, codecs: codec_ids, placeholders };
    let mut fns = Vec::new();
    for (id, (d, from_prelude)) in decls.iter().enumerate() {
        fns.push(check_fn(&globals, id, d, *from_prelude)?);
    }
    // main
    let main = match globals.names.get("main") {
        Some(&m) => m,
        None => return err(isc_syntax::Span::new(0, 1, 1), "the program has no `action main(args: string[k]): i32`"),
    };
    let m = &fns[main];
    let ok_params = m.param_tys.len() == 1
        && matches!(&m.param_tys[0], Ty::Array(_, e) if **e == Ty::Str);
    if !m.is_action || !ok_params || m.ret != Ty::Int(IntKind::I32) {
        return err(m.span, "`main` must be declared `action main(args: string[k]): i32`");
    }
    // the panics certain at compile time (D162)
    for f in fns.iter().filter(|f| !f.from_prelude) {
        certain_panics(&f.body)?;
    }
    // the connections and listeners closed exactly once (D339)
    check_linear(&fns, &sums_for_linear(&globals))?;
    let recursive = recursive_fns(&fns);
    let recursive = outline_large(&fns, recursive, main);
    // a constant computed from itself would never end (D159)
    for (id, f) in fns.iter().enumerate() {
        if recursive[id] && globals.sigs[id].is_const {
            return err(f.span, format!("the constant `{}` depends on itself", f.name));
        }
    }
    Ok(TProgram { fns, main, sums: globals.sums, externs, recursive })
}

/// The types an `extern` function may take and return (D125): scalars,
/// `string`, arrays of scalars as parameters; a scalar or `()` as result;
/// at most 6 parameters in integer registers (an array takes two: its
/// pointer and its length) and 8 in vector registers (System V x86-64).
fn check_extern_sig(e: &ast::ExternDecl, sig: &Sig) -> Result<()> {
    let scalar = |t: &Ty| matches!(t, Ty::Int(_) | Ty::F64 | Ty::F32 | Ty::Bool);
    let show = |t: &Ty| {
        let sym_name = |s: SymId| sig.syms[s as usize].name.clone();
        let param_name = |p: u32| format!("T{}", p);
        let size_var = |_v: u32| None;
        format!("{}", TyDisplay { ty: t, sym_name: &sym_name, param_name: &param_name, size_var: &size_var })
    };
    let (mut ints, mut floats) = (0, 0);
    for (t, p) in sig.param_tys.iter().zip(e.params.iter()) {
        match t {
            Ty::F64 | Ty::F32 => floats += 1,
            Ty::Int(_) | Ty::Bool | Ty::Str => ints += 1,
            Ty::Array(_, el) if scalar(el) => ints += 2,
            other => {
                return err(
                    p.span,
                    format!(
                        "the parameter `{}` of an extern function has the type `{}`: a scalar, a `string` or an array of scalars is expected",
                        p.name,
                        show(other)
                    ),
                )
            }
        }
    }
    if !(scalar(&sig.ret) || sig.ret == Ty::Unit) {
        return err(e.ret.span, format!("an extern function returns a scalar or `void`, not `{}`", show(&sig.ret)));
    }
    if ints > 6 || floats > 8 {
        return err(
            e.span,
            format!(
                "the extern function `{}` takes too many parameters: at most 6 in integer registers (an array takes two) and 8 floating-point ones",
                e.name
            ),
        );
    }
    Ok(())
}

/// The type aliases `type X<T> = <type>;` (D159), replaced by their types
/// everywhere, then removed: an alias is not a new type, as in TypeScript.
/// The names in the body of an alias are first resolved in its own module;
/// a size variable in it becomes one of the signature that uses it.
fn expand_aliases(p: &mut ast::Program, imports: &Imports) -> Result<()> {
    use ast::{TypeExpr, TypeKind as K, Visit};
    struct Alias {
        name: String,
        generics: Vec<String>,
        body: TypeExpr,
    }
    let mut aliases: Vec<Alias> = Vec::new();
    let types: HashSet<String> = p.decls.iter().filter_map(|d| if let ast::Decl::Type(t) = d { Some(t.name.clone()) } else { None }).collect();
    // the declared name a name `n` stands for in the decl `owner`
    let resolve = |n: &str, owner: &str, file: u32, known: &dyn Fn(&str) -> bool| -> Option<String> {
        if let Some(m) = module_of(owner).filter(|_| !n.contains('.')) {
            let q = format!("{}.{}", m, n);
            if known(&q) {
                return Some(q);
            }
        }
        if let Some(q) = imports.get(&file).and_then(|i| i.get(n)) {
            if known(q) {
                return Some(q.clone());
            }
        }
        known(n).then(|| n.to_string())
    };
    for d in &p.decls {
        if let ast::Decl::Type(t) = d {
            if let Some(body) = &t.alias {
                if !t.generics.iter().all(|g| g.class.is_none()) {
                    return err(t.span, "the parameters of a `type` take no class");
                }
                let generics: Vec<String> = t.generics.iter().map(|g| g.name.clone()).collect();
                let mut body = body.clone();
                // the names of its body, resolved in its module
                let mut prog = ast::Program { decls: vec![ast::Decl::Type(ast::TypeDecl { alias: None, variants: vec![ast::Variant { name: String::new(), fields: vec![body.clone()], span: t.span }], ..t.clone() })], exports: Vec::new(), imported: Vec::new() };
                let owner = t.name.clone();
                ast::visit_names(&mut prog, &mut |v, sp| {
                    if let Visit::Type(TypeExpr { kind: K::Named(n, _), .. }) = v {
                        if !generics.contains(n) {
                            if let Some(q) = resolve(n, &owner, sp.file, &|x| types.contains(x)) {
                                *n = q;
                            }
                        }
                    }
                    Ok(())
                })?;
                if let ast::Decl::Type(td) = prog.decls.pop().unwrap() {
                    body = td.variants.into_iter().next().unwrap().fields.pop().unwrap();
                }
                aliases.push(Alias { name: t.name.clone(), generics, body });
            }
        }
    }
    if aliases.is_empty() {
        return Ok(());
    }
    // an alias that contains itself, used or not
    fn contains(t: &TypeExpr, target: &str, aliases: &[Alias], depth: usize) -> bool {
        match &t.kind {
            K::Named(n, args) => {
                if n == target {
                    return true;
                }
                if depth < 64 {
                    if let Some(a) = aliases.iter().find(|a| &a.name == n) {
                        if contains(&a.body, target, aliases, depth + 1) {
                            return true;
                        }
                    }
                }
                args.iter().any(|x| contains(x, target, aliases, depth))
            }
            K::Array(_, e) | K::BoundedArray(_, _, e) => contains(e, target, aliases, depth),
            K::Tuple(ts) => ts.iter().any(|x| contains(x, target, aliases, depth)),
            K::Record(fs) => fs.iter().any(|(_, x)| contains(x, target, aliases, depth)),
            K::Fn(ps, r, _) => ps.iter().any(|x| contains(x, target, aliases, depth)) || contains(r, target, aliases, depth),
            _ => false,
        }
    }
    for a in &aliases {
        if contains(&a.body, &a.name, &aliases, 0) {
            return err(a.body.span, format!("the type alias `{}` contains itself: a recursive type is a sum type (`type List = Nil | Cons(i64, List);`)", a.name));
        }
    }
    fn subst(t: &TypeExpr, gs: &[String], args: &[TypeExpr]) -> TypeExpr {
        let kind = match &t.kind {
            K::Named(n, a) if a.is_empty() && gs.contains(n) => return args[gs.iter().position(|g| g == n).unwrap()].clone(),
            K::Named(n, a) => K::Named(n.clone(), a.iter().map(|x| subst(x, gs, args)).collect()),
            K::Array(sz, e) => K::Array(sz.clone(), Box::new(subst(e, gs, args))),
            K::BoundedArray(k, sz, e) => K::BoundedArray(k.clone(), sz.clone(), Box::new(subst(e, gs, args))),
            K::Tuple(ts) => K::Tuple(ts.iter().map(|x| subst(x, gs, args)).collect()),
            K::Record(fs) => K::Record(fs.iter().map(|(n, x)| (n.clone(), subst(x, gs, args))).collect()),
            K::Fn(ps, r, act) => K::Fn(ps.iter().map(|x| subst(x, gs, args)).collect(), Box::new(subst(r, gs, args)), *act),
            k => k.clone(),
        };
        TypeExpr { kind, span: t.span }
    }
    let alias_names: HashSet<String> = aliases.iter().map(|a| a.name.clone()).collect();
    let decls = std::mem::take(&mut p.decls);
    for d in decls {
        if matches!(&d, ast::Decl::Type(t) if t.alias.is_some()) {
            continue;
        }
        let (owner, generics): (String, Vec<String>) = match &d {
            ast::Decl::Fn(f) => (f.name.clone(), f.generics.iter().map(|g| g.name.clone()).collect()),
            ast::Decl::Type(t) => (t.name.clone(), t.generics.iter().map(|g| g.name.clone()).collect()),
            ast::Decl::Extern(e) => (e.name.clone(), Vec::new()),
            ast::Decl::Import(..) => (String::new(), Vec::new()),
        };
        let mut prog = ast::Program { decls: vec![d], exports: Vec::new(), imported: Vec::new() };
        let mut steps = 0usize;
        ast::visit_names(&mut prog, &mut |v, sp| {
            let Visit::Type(t) = v else {
                return Ok(());
            };
            loop {
                let K::Named(n, args) = &t.kind else {
                    return Ok(());
                };
                if generics.contains(n) {
                    return Ok(());
                }
                let Some(q) = resolve(n, &owner, sp.file, &|x| alias_names.contains(x)) else {
                    return Ok(());
                };
                let a = aliases.iter().find(|a| a.name == q).unwrap();
                if args.len() != a.generics.len() {
                    return err(sp, format!("the type `{}` takes {} type argument(s), but {} were given", n, a.generics.len(), args.len()));
                }
                steps += 1;
                if steps > 10_000 {
                    return err(sp, format!("the type alias `{}` contains itself", n));
                }
                let mut x = subst(&a.body, &a.generics, args);
                x.span = sp;
                *t = x;
            }
        })?;
        p.decls.push(prog.decls.pop().unwrap());
    }
    Ok(())
}

/// The `type` declarations: names first (a type may use one declared later),
/// then the payloads. A type that contains itself, directly or through other
/// types, is rejected until reference counting exists (stage 2, phase 3).
#[allow(clippy::type_complexity)]
fn check_type_decls(p: &ast::Program, imports: &Imports) -> Result<(Vec<SumDef>, Vec<(String, usize)>, HashMap<String, (u32, u32)>)> {
    let decls: Vec<&ast::TypeDecl> = p
        .decls
        .iter()
        .filter_map(|d| if let ast::Decl::Type(t) = d { Some(t) } else { None })
        .collect();
    let mut heads: Vec<(String, usize)> = Vec::new();
    for t in &decls {
        if heads.iter().any(|(n, _)| n == &t.name) {
            return err(t.span, format!("duplicate declaration of the type `{}`", t.name));
        }
        if matches!(t.name.as_str(), "string" | "boolean" | "void" | "size") || isc_syntax::lexer::Scalar::from_name(&t.name).is_some() {
            return err(t.span, format!("`{}` is a built-in type", t.name));
        }
        heads.push((t.name.clone(), t.generics.len()));
    }
    let mut sums = Vec::new();
    let mut ctors: HashMap<String, (u32, u32)> = HashMap::new();
    for (id, t) in decls.iter().enumerate() {
        let mut generics: Vec<(String, Classes)> = Vec::new();
        for g in &t.generics {
            if generics.iter().any(|(n, _)| n == &g.name) {
                return err(g.span, format!("duplicate type parameter `{}`", g.name));
            }
            if g.class.is_some() {
                return err(g.span, "the parameters of a `type` take no class");
            }
            // no class: a payload can be any value (`Result<[r][c]f64,
            // string>`, D138); a sum stored in an array constrains its
            // payloads then (`constrain` of `Ty::Sum`)
            generics.push((g.name.clone(), 0));
        }
        let mut variants = Vec::new();
        for v in &t.variants {
            if ctors.contains_key(&v.name) {
                return err(v.span, format!("duplicate constructor `{}`", v.name));
            }
            ctors.insert(v.name.clone(), (id as u32, variants.len() as u32));
            let mut syms = Vec::new();
            let mut payload = Vec::new();
            for f in &v.fields {
                // a size named in a payload: of each value (D335)
                let ty = type_from_ast(f, &mut SigCtx { generics: &generics, sums: &heads, syms: &mut syms, allow_new: true, result: false, module: module_of(&t.name), imports: imports.get(&t.span.file) })?;
                // a function is stored as its code (D153), alone in its
                // field
                if !matches!(ty, Ty::Fn(..)) && contains_fn(&ty) {
                    return err(f.span, "a structure of functions stored in a constructor (SPEC section 11): give each function its own field");
                }
                payload.push(ty);
            }
            variants.push(VariantDef { name: v.name.clone(), payload, sizes: syms.len() });
        }
        if variants.is_empty() {
            return err(t.span, format!("the type `{}` has no constructor", t.name));
        }
        sums.push(SumDef { name: t.name.clone(), params: generics.into_iter().map(|g| g.0).collect(), variants, recursive: false });
    }
    // no type may contain itself
    fn uses(t: &Ty, out: &mut Vec<u32>) {
        match t {
            Ty::Sum { id, args, .. } => {
                out.push(*id);
                args.iter().for_each(|a| uses(a, out));
            }
            Ty::Tuple(ts) => ts.iter().for_each(|a| uses(a, out)),
            Ty::Record(fs) => fs.iter().for_each(|(_, a)| uses(a, out)),
            Ty::Array(_, e) => uses(e, out),
            _ => {}
        }
    }
    let edges: Vec<Vec<u32>> = sums
        .iter()
        .map(|d| {
            let mut v = Vec::new();
            d.variants.iter().for_each(|x| x.payload.iter().for_each(|t| uses(t, &mut v)));
            v
        })
        .collect();
    for start in 0..decls.len() {
        let mut seen = vec![false; sums.len()];
        let mut todo = edges[start].clone();
        while let Some(n) = todo.pop() {
            if n as usize == start {
                sums[start].recursive = true;
                break;
            }
            if !std::mem::replace(&mut seen[n as usize], true) {
                todo.extend(edges[n as usize].iter().copied());
            }
        }
    }
    Ok((sums, heads, ctors))
}

fn check_fn(g: &Globals, id: FnId, d: &ast::FnDecl, from_prelude: bool) -> Result<TFn> {
    let sig = &g.sigs[id];
    let mut cx = FnCx {
        method_call: false,
        g,
        fn_id: id,
        module: module_of(&d.name).map(|m| m.to_string()),
        file: d.span.file,
        is_action: sig.is_action,
        lambda_depth: 0,
        act_lambdas: 0,
        act_effects: Vec::new(),
        generics: sig.generics.clone(),
        syms: sig.syms.clone(),
        tvars: Vec::new(),
        svars: Vec::new(),
        locals: Vec::new(),
        scopes: vec![HashMap::new()],
        deferred_scalar: Vec::new(),
        deferred_len: Vec::new(),
        deferred_printable: Vec::new(),
        dyn_flag: false,
        constraining: Vec::new(),
    };
    let _ = cx.fn_id;
    let mut params = Vec::new();
    for ((name, sp), t) in sig.param_names.iter().zip(sig.param_tys.iter()) {
        params.push(cx.add_local(name, t.clone(), *sp));
    }
    let ret = sig.ret.clone();
    // the existential sizes of the result are fixed by the body: open, they
    // unify with the size of what the body computes
    let opened = cx.open_existentials(&ret);
    let mut body = cx.check(&d.body, Some(&opened))?;
    cx.unify(&opened, &body.ty, d.body.span)?;
    cx.zonk_expr(&mut body)?;
    for i in 0..cx.locals.len() {
        let t = cx.locals[i].ty.clone();
        cx.locals[i].ty = cx.zonk(&t)?;
    }
    for (t, sp) in cx.deferred_len.clone() {
        let t = cx.zonk(&t)?;
        if !matches!(t, Ty::Array(..) | Ty::Str) {
            return err(sp, format!("`len` takes a string or an array, not `{}`", cx.show(&t)));
        }
    }
    for (t, sp) in cx.deferred_scalar.clone() {
        let t = cx.zonk(&t)?;
        if !t.is_scalar() && !matches!(t, Ty::Param(_) | Ty::BigInt) {
            return err(sp, format!("expected a scalar here, found `{}`; add a type annotation", cx.show(&t)));
        }
    }
    for (t, sp) in cx.deferred_printable.clone() {
        let t = cx.zonk(&t)?;
        if holds_function(g, &t, &mut Vec::new()) {
            return err(sp, "a function cannot be printed (here, inside this value)");
        }
        if t == Ty::RegExp {
            return err(sp, "a RegExp cannot be printed");
        }
        if holds_handle(&t) {
            return err(sp, "a resource of the system (a connection, a listener) cannot be printed");
        }
        if holds_map(g, &t) {
            return err(sp, "a Map cannot be printed: print `m.entries()`");
        }
    }
    Ok(TFn {
        name: sig.name.clone(),
        span: sig.span,
        is_action: sig.is_action,
        from_prelude,
        generics: sig.generics.clone(),
        syms: cx.syms.clone(),
        params,
        param_tys: sig.param_tys.clone(),
        ret,
        locals: cx.locals,
        body,
        requires: d.modifiers.clone(),
    })
}

/// An integer literal, negated or not.
fn lit_int(e: &TExpr) -> Option<i128> {
    match &e.kind {
        TExprKind::Lit(Lit::Int(v)) => Some(*v),
        TExprKind::Unary(ast::UnOp::Neg, x) => lit_int(x).map(|v| -v),
        _ => None,
    }
}

/// The operations whose operands are literals that make them panic, as
/// compile errors with the message of the panic (D162): as Rust refuses
/// `1 / 0` or `[1, 2][5]`, the run-time check is then only a second net.
fn certain_panics(e: &TExpr) -> Result<()> {
    match &e.kind {
        TExprKind::Binary { op: ast::BinOp::Div | ast::BinOp::Rem, rhs, .. } if matches!(rhs.ty, Ty::Int(_)) && lit_int(rhs) == Some(0) => {
            return err(e.span, "integer division by zero");
        }
        TExprKind::Index(a, i) => {
            if let (TExprKind::Array(es), Some(v)) = (&a.kind, lit_int(i)) {
                if v < 0 || v >= es.len() as i128 {
                    return err(e.span, format!("index out of bounds: the index is {} but the length is {}", v, es.len()));
                }
            }
        }
        TExprKind::Pow(b, x) => {
            let neg = match &x.kind {
                TExprKind::Lit(Lit::BigInt(d, _, true)) => d.trim_start_matches('0') != "",
                _ => matches!(b.ty, Ty::Int(_)) && lit_int(x).is_some_and(|v| v < 0),
            };
            if neg {
                return err(e.span, "negative integer exponent");
            }
        }
        TExprKind::Call { callee: Callee::Prim(p), args, .. } => {
            let at = |i: usize| args.get(i).and_then(lit_int);
            match p {
                Prim::Iota | Prim::Fill if at(0).is_some_and(|v| v < 0) => {
                    return err(e.span, format!("negative size: {}", at(0).unwrap()));
                }
                Prim::ToFixed if at(1).is_some_and(|v| !(0..=100).contains(&v)) => {
                    return err(e.span, format!("toFixed: the number of digits must be between 0 and 100, got {}", at(1).unwrap()));
                }
                Prim::Repeat if at(1).is_some_and(|v| v < 0) => {
                    return err(e.span, format!("repeat: the count is negative: {}", at(1).unwrap()));
                }
                _ => {}
            }
        }
        _ => {}
    }
    let mut res = Ok(());
    for_each_child(e, &mut |c| {
        if res.is_ok() {
            res = certain_panics(c);
        }
    });
    res
}

/// Each direct sub-expression of `e` (also for the tools that read the
/// typed tree, such as the language server).
pub fn for_each_child(e: &TExpr, f: &mut dyn FnMut(&TExpr)) {
    match &e.kind {
        TExprKind::Lit(_) | TExprKind::Local(_) | TExprKind::SizeVal(_) | TExprKind::FnRef { .. } | TExprKind::PrimRef(_) | TExprKind::OpRef(_) => {}
        TExprKind::Tuple(es) | TExprKind::Array(es) | TExprKind::Construct { args: es, .. } => es.iter().for_each(f),
        TExprKind::Field(x, _) | TExprKind::Unary(_, x) | TExprKind::Lambda(_, x) => f(x),
        TExprKind::FieldUpdate { target, value, .. } => {
            f(target);
            f(value);
        }
        TExprKind::Match(x, arms) => {
            f(x);
            arms.iter().for_each(|(_, b)| f(b));
        }
        TExprKind::Call { callee, args, .. } => {
            if let Callee::Value(c) = callee {
                f(c);
            }
            args.iter().for_each(f);
        }
        TExprKind::Index(a, b) | TExprKind::Pow(a, b) => {
            f(a);
            f(b);
        }
        TExprKind::Binary { lhs, rhs, .. } => {
            f(lhs);
            f(rhs);
        }
        TExprKind::Slice(a, b, c) | TExprKind::If(a, b, c) => {
            f(a);
            f(b);
            f(c);
        }
        TExprKind::Update { target, index, value, .. } => {
            f(target);
            f(index);
            f(value);
        }
        TExprKind::Block(stmts, last) => {
            for st in stmts {
                match st {
                    TStmt::Let(_, x) | TStmt::Expr(x) => f(x),
                }
            }
            f(last);
        }
    }
}

fn collect_calls(e: &TExpr, out: &mut Vec<(FnId, Span)>) {
    let mut go = |x: &TExpr| collect_calls(x, out);
    match &e.kind {
        TExprKind::FnRef { id, .. } => {
            out.push((*id, e.span));
        }
        TExprKind::Call { callee, args, .. } => {
            match callee {
                Callee::Fn { id, .. } => {
                    out.push((*id, e.span));
                }
                Callee::Value(f) => collect_calls(f, out),
                Callee::Prim(_) | Callee::Extern(_) => {}
            }
            for a in args {
                collect_calls(a, out);
            }
        }
        TExprKind::Tuple(es) | TExprKind::Array(es) => es.iter().for_each(&mut go),
        TExprKind::Index(a, b) | TExprKind::Pow(a, b) => {
            go(a);
            go(b);
        }
        TExprKind::Binary { lhs, rhs, .. } => {
            go(lhs);
            go(rhs);
        }
        TExprKind::Update { target, index, value, .. } => {
            go(target);
            go(index);
            go(value);
        }
        TExprKind::FieldUpdate { target, value, .. } => {
            go(target);
            go(value);
        }
        TExprKind::Construct { args, .. } => args.iter().for_each(&mut go),
        TExprKind::Match(x, arms) => {
            go(x);
            arms.iter().for_each(|(_, b)| go(b));
        }
        TExprKind::Unary(_, x) | TExprKind::Lambda(_, x) | TExprKind::Field(x, _) => go(x),
        TExprKind::If(a, b, c) => {
            go(a);
            go(b);
            go(c);
        }
        TExprKind::Block(stmts, r) => {
            for s in stmts {
                match s {
                    TStmt::Let(_, v) | TStmt::Expr(v) => go(v),
                }
            }
            go(r);
        }
        _ => {}
    }
}

/// The functions that can call themselves, directly or through others (SPEC
/// section 11: they are not inlined).
/// The size above which a function is called rather than inlined (D336):
/// its nodes once the functions it calls are inlined in it.
const INLINE_LIMIT: usize = 4000;
/// A function of at least `COPY_MIN` nodes is called rather than inlined
/// when its copies would hold more than `COPY_LIMIT` nodes (D336).
const COPY_MIN: usize = 100;
const COPY_LIMIT: usize = 2000;

/// The functions called rather than inlined because of their size (D336):
/// a function whose nodes, the bodies it inlines counted, exceed
/// `INLINE_LIMIT`, or whose copies in the program (one at each place where
/// it is inlined) would exceed `COPY_LIMIT`; decided again until nothing
/// changes, as each such function is compiled once. Not a function that
/// takes or returns a function (each call specializes it, SPEC section 11),
/// nor `main`.
fn outline_large(fns: &[TFn], mut called: Vec<bool>, main: FnId) -> Vec<bool> {
    fn nodes(e: &TExpr) -> usize {
        let mut n = 1;
        for_each_child(e, &mut |c| n += nodes(c));
        n
    }
    fn holds_fn(t: &Ty) -> bool {
        match t {
            Ty::Fn(..) => true,
            Ty::Tuple(ts) => ts.iter().any(holds_fn),
            Ty::Record(fs) => fs.iter().any(|(_, t)| holds_fn(t)),
            Ty::Array(_, e) => holds_fn(e),
            Ty::Sum { args, .. } => args.iter().any(holds_fn),
            _ => false,
        }
    }
    let n = fns.len();
    let own: Vec<usize> = fns.iter().map(|f| nodes(&f.body)).collect();
    let calls: Vec<Vec<FnId>> = fns
        .iter()
        .map(|f| {
            let mut v = Vec::new();
            collect_calls(&f.body, &mut v);
            v.into_iter().map(|(g, _)| g).collect()
        })
        .collect();
    // the functions that the program reaches
    let mut reached = vec![false; n];
    let mut todo = vec![main];
    while let Some(f) = todo.pop() {
        if !std::mem::replace(&mut reached[f], true) {
            todo.extend(calls[f].iter().copied());
        }
    }
    let eligible: Vec<bool> = (0..n).map(|f| f != main && reached[f] && !fns[f].param_tys.iter().any(holds_fn) && !holds_fn(&fns[f].ret)).collect();
    loop {
        // the size of each function with what it inlines, from the leaves
        fn size(f: FnId, calls: &[Vec<FnId>], own: &[usize], called: &[bool], sizes: &mut Vec<Option<usize>>) -> usize {
            if let Some(s) = sizes[f] {
                return s;
            }
            sizes[f] = Some(1);
            let mut s = own[f];
            for &g in &calls[f] {
                if !called[g] {
                    s += size(g, calls, own, called, sizes);
                }
            }
            sizes[f] = Some(s);
            s
        }
        let mut sizes = vec![None; n];
        let sz: Vec<usize> = (0..n).map(|f| size(f, &calls, &own, &called, &mut sizes)).collect();
        // the copies of each function: one in each copy of a function that
        // inlines it; a called function is one
        fn copies(f: FnId, callers: &[Vec<FnId>], called: &[bool], main: FnId, memo: &mut Vec<Option<usize>>) -> usize {
            if let Some(c) = memo[f] {
                return c;
            }
            memo[f] = Some(0);
            let mut c = 0usize;
            for &g in &callers[f] {
                c = c.saturating_add(if called[g] || g == main { 1 } else { copies(g, callers, called, main, memo) });
            }
            memo[f] = Some(c);
            c
        }
        let mut callers: Vec<Vec<FnId>> = vec![Vec::new(); n];
        for f in (0..n).filter(|&f| reached[f]) {
            for &g in &calls[f] {
                callers[g].push(f);
            }
        }
        let mut memo = vec![None; n];
        let mut changed = false;
        for f in 0..n {
            if called[f] || !eligible[f] {
                continue;
            }
            let c = copies(f, &callers, &called, main, &mut memo);
            if sz[f] > INLINE_LIMIT || (sz[f] >= COPY_MIN && c >= 2 && sz[f].saturating_mul(c) > COPY_LIMIT) {
                called[f] = true;
                changed = true;
            }
        }
        if !changed {
            return called;
        }
    }
}

fn recursive_fns(fns: &[TFn]) -> Vec<bool> {
    let edges: Vec<Vec<FnId>> = fns
        .iter()
        .map(|f| {
            let mut v = Vec::new();
            collect_calls(&f.body, &mut v);
            v.into_iter().map(|(id, _)| id).collect()
        })
        .collect();
    (0..fns.len())
        .map(|start| {
            let mut seen = vec![false; fns.len()];
            let mut todo = edges[start].clone();
            while let Some(n) = todo.pop() {
                if n == start {
                    return true;
                }
                if !std::mem::replace(&mut seen[n], true) {
                    todo.extend(edges[n].iter().copied());
                }
            }
            false
        })
        .collect()
}

/// The primitives of the system that `main` can reach (D324), each with the
/// first path of calls found to it (the names of the functions from `main`)
/// and the place of its call in the last function of the program on that
/// path (outside the standard modules): a target that lacks one refuses
/// the program, naming them. Through every call of a reached function, a
/// function value too (more than run time takes, never less).
pub fn system_paths(p: &TProgram) -> Vec<(SysOp, Vec<String>, Span)> {
    fn prims(e: &TExpr, out: &mut Vec<(SysOp, Span)>) {
        if let TExprKind::Call { callee: Callee::Prim(Prim::Sys(op)), .. } = &e.kind {
            out.push((*op, e.span));
        }
        for_each_child(e, &mut |c| prims(c, out));
    }
    let n = p.fns.len();
    let calls: Vec<Vec<(FnId, Span)>> = p
        .fns
        .iter()
        .map(|f| {
            let mut v = Vec::new();
            collect_calls(&f.body, &mut v);
            v
        })
        .collect();
    // breadth first from `main`: the parent of each function and the place
    // of the call that reached it
    let mut parent: Vec<Option<(FnId, Span)>> = vec![None; n];
    let mut seen = vec![false; n];
    let mut queue = std::collections::VecDeque::from([p.main]);
    seen[p.main] = true;
    let mut out: Vec<(SysOp, Vec<String>, Span)> = Vec::new();
    while let Some(f) = queue.pop_front() {
        let mut ps = Vec::new();
        prims(&p.fns[f].body, &mut ps);
        for (op, sp) in ps {
            if out.iter().any(|(o, _, _)| *o == op) {
                continue;
            }
            let mut path = vec![f];
            let mut at = sp;
            while let Some((g, s)) = parent[*path.last().unwrap()] {
                path.push(g);
                let _ = s;
            }
            path.reverse();
            // the call made by the last function of the program on the path
            let mut k = path.len();
            while k > 1 && p.fns[path[k - 1]].name.starts_with(crate::modules::STD_KEY) {
                k -= 1;
            }
            if k < path.len() {
                at = parent[path[k]].map(|(_, s)| s).unwrap_or(sp);
            }
            let names = path.iter().map(|&g| p.fns[g].name.replace(crate::modules::STD_KEY, "")).collect();
            out.push((op, names, at));
        }
        for &(g, sp) in &calls[f] {
            if !seen[g] {
                seen[g] = true;
                parent[g] = Some((f, sp));
                queue.push_back(g);
            }
        }
    }
    out
}

// ----- the resources of the system are linear (D339) ---------------------

/// A value of type `t` holds a resource of the system: a `Conn`, a
/// `Listener`, or a value that holds one (in a tuple, a record, the payload
/// of a sum).
fn linear_ty(t: &Ty, sums: &[SumDef], seen: &mut Vec<u32>) -> bool {
    match t {
        Ty::Handle(_) => true,
        Ty::Tuple(ts) => ts.iter().any(|x| linear_ty(x, sums, seen)),
        Ty::Record(fs) => fs.iter().any(|(_, x)| linear_ty(x, sums, seen)),
        Ty::Sum { id, args, .. } => {
            if seen.contains(id) {
                return false;
            }
            seen.push(*id);
            let def = &sums[*id as usize];
            let r = (0..def.variants.len()).any(|k| def.payload(k, args).iter().any(|x| linear_ty(x, sums, seen)));
            seen.pop();
            r
        }
        _ => false,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Lin {
    Live,
    /// Closed or given away, at this place.
    Gone(Span),
}

#[derive(Clone, Copy, PartialEq)]
enum Take {
    /// The value is only read.
    Borrow,
    /// The value is taken: closed, stored, returned.
    Move,
}

struct LinCx<'a> {
    f: &'a TFn,
    sums: &'a [SumDef],
    /// For each function, whether it consumes each parameter.
    consumes: &'a [Vec<bool>],
    st: HashMap<LocalId, Lin>,
    in_lambda: Vec<Vec<LocalId>>,
}

impl LinCx<'_> {
    fn linear(&self, t: &Ty) -> bool {
        linear_ty(t, self.sums, &mut Vec::new())
    }

    fn name(&self, l: LocalId) -> String {
        self.f.locals[l as usize].name.clone()
    }

    /// The local `l` used where `e` is, taken or read.
    fn local(&mut self, l: LocalId, take: Take, span: Span) -> Result<()> {
        let ty = self.f.locals[l as usize].ty.clone();
        if !self.linear(&ty) {
            return Ok(());
        }
        if let Some(own) = self.in_lambda.last() {
            if !own.contains(&l) {
                return err(span, format!("`{}` holds a connection or a listener: a lambda cannot capture it (it is closed exactly once)", self.name(l)));
            }
        }
        match self.st.get(&l).copied() {
            Some(Lin::Gone(at)) => err(span, format!("`{}` is used after it was closed or given away at line {}", self.name(l), at.line)),
            _ => {
                if take == Take::Move {
                    self.st.insert(l, Lin::Gone(span));
                }
                Ok(())
            }
        }
    }

    /// The locals that a pattern binds from a value of type `ty`: the
    /// linear ones alive; a part of a linear value left by `_` is lost.
    fn bind(&mut self, p: &TPat, ty: &Ty, linear: bool, span: Span) -> Result<()> {
        match p {
            TPat::Bind(l) => {
                if linear && self.linear(ty) {
                    self.st.insert(*l, Lin::Live);
                }
                Ok(())
            }
            TPat::Wild(_) => {
                if linear && self.linear(ty) {
                    return err(span, "a connection or a listener is left by `_` here: it would never be closed");
                }
                Ok(())
            }
            TPat::Lit(..) => Ok(()),
            TPat::Tuple(ps, _) => {
                let ts: Vec<Ty> = match ty {
                    Ty::Tuple(ts) => ts.clone(),
                    Ty::Record(fs) => fs.iter().map(|(_, t)| t.clone()).collect(),
                    _ => return Ok(()),
                };
                for (q, t) in ps.iter().zip(ts.iter()) {
                    self.bind(q, t, linear, span)?;
                }
                Ok(())
            }
            TPat::Ctor { sum, variant, subs, .. } => {
                let args = match ty {
                    Ty::Sum { args, .. } => args.clone(),
                    _ => Vec::new(),
                };
                let payload = self.sums[*sum as usize].payload(*variant as usize, &args);
                for (q, t) in subs.iter().zip(payload.iter()) {
                    self.bind(q, t, linear, span)?;
                }
                Ok(())
            }
        }
    }

    /// The locals bound by a pattern.
    fn bound(p: &TPat, out: &mut Vec<LocalId>) {
        match p {
            TPat::Bind(l) => out.push(*l),
            TPat::Tuple(ps, _) | TPat::Ctor { subs: ps, .. } => ps.iter().for_each(|q| Self::bound(q, out)),
            TPat::Wild(_) | TPat::Lit(..) => {}
        }
    }

    /// The locals of `ls` that hold a resource and are still alive: a fault,
    /// they would never be closed.
    fn all_gone(&self, ls: &[LocalId], span: Span) -> Result<()> {
        for &l in ls {
            if self.st.get(&l) == Some(&Lin::Live) {
                return err(span, format!("`{}` is never closed: a connection or a listener is closed (or given away) on every path", self.name(l)));
            }
        }
        Ok(())
    }

    /// The states after two paths: the same, or a fault.
    fn agree(&self, a: &HashMap<LocalId, Lin>, b: &HashMap<LocalId, Lin>, span: Span) -> Result<HashMap<LocalId, Lin>> {
        let mut out = a.clone();
        for (l, x) in a {
            let y = b.get(l).copied().unwrap_or(Lin::Live);
            match (*x, y) {
                (Lin::Live, Lin::Gone(at)) | (Lin::Gone(at), Lin::Live) => {
                    return err(span, format!("`{}` is closed or given away on one path (line {}) and not on the other", self.name(*l), at.line));
                }
                _ => {}
            }
        }
        for (l, y) in b {
            if !a.contains_key(l) {
                if let Lin::Gone(at) = y {
                    return err(span, format!("`{}` is closed or given away on one path (line {}) and not on the other", self.name(*l), at.line));
                }
                out.insert(*l, *y);
            }
        }
        Ok(out)
    }

    /// A new value that holds a resource, which its context only reads: it
    /// would be lost.
    fn lost(&self, e: &TExpr, take: Take) -> Result<()> {
        if take == Take::Borrow && self.linear(&e.ty) && !matches!(e.kind, TExprKind::Local(_)) {
            return err(e.span, "the connection or listener made here is never closed: bind it, close it or give it away");
        }
        Ok(())
    }

    fn walk(&mut self, e: &TExpr, take: Take) -> Result<()> {
        let span = e.span;
        match &e.kind {
            TExprKind::Lit(_) | TExprKind::SizeVal(_) | TExprKind::FnRef { .. } | TExprKind::PrimRef(_) | TExprKind::OpRef(_) => Ok(()),
            TExprKind::Local(l) => self.local(*l, take, span),
            TExprKind::Tuple(es) | TExprKind::Array(es) | TExprKind::Construct { args: es, .. } => {
                for x in es {
                    self.walk(x, Take::Move)?;
                }
                self.lost(e, take)
            }
            TExprKind::Field(x, _) => {
                if self.linear(&e.ty) {
                    return err(span, "a connection or a listener in a value is taken by a pattern (`const { c, ... } = v`), not read as a field");
                }
                self.walk(x, Take::Borrow)
            }
            TExprKind::FieldUpdate { target, value, .. } => {
                let t = if self.linear(&target.ty) { Take::Move } else { Take::Borrow };
                self.walk(target, t)?;
                self.walk(value, Take::Move)?;
                self.lost(e, take)
            }
            TExprKind::Match(x, arms) => {
                let linear = self.linear(&x.ty);
                self.walk(x, if linear { Take::Move } else { Take::Borrow })?;
                let before = self.st.clone();
                let mut after: Option<HashMap<LocalId, Lin>> = None;
                for (p, body) in arms {
                    self.st = before.clone();
                    self.bind(p, &x.ty, linear, span)?;
                    let mut own = Vec::new();
                    Self::bound(p, &mut own);
                    self.walk(body, take)?;
                    self.all_gone(&own, body.span)?;
                    for l in &own {
                        self.st.remove(l);
                    }
                    after = Some(match after {
                        None => self.st.clone(),
                        Some(a) => self.agree(&a, &self.st.clone(), body.span)?,
                    });
                }
                self.st = after.unwrap_or(before);
                Ok(())
            }
            TExprKind::Call { callee, args, .. } => {
                match callee {
                    Callee::Prim(Prim::Sys(op)) => {
                        for (i, a) in args.iter().enumerate() {
                            self.walk(a, if i == 0 && op.consumes() { Take::Move } else { Take::Borrow })?;
                        }
                    }
                    Callee::Fn { id, .. } => {
                        for (i, a) in args.iter().enumerate() {
                            let moves = self.consumes.get(*id).and_then(|c| c.get(i)).copied().unwrap_or(false);
                            self.walk(a, if moves { Take::Move } else { Take::Borrow })?;
                        }
                    }
                    Callee::Value(f) => {
                        self.walk(f, Take::Borrow)?;
                        for a in args {
                            self.walk(a, Take::Borrow)?;
                        }
                    }
                    Callee::Prim(_) | Callee::Extern(_) => {
                        for a in args {
                            self.walk(a, Take::Borrow)?;
                        }
                    }
                }
                self.lost(e, take)
            }
            TExprKind::If(c, t, f) => {
                self.walk(c, Take::Borrow)?;
                let before = self.st.clone();
                self.walk(t, take)?;
                let after_t = std::mem::replace(&mut self.st, before);
                self.walk(f, take)?;
                let after_f = self.st.clone();
                self.st = self.agree(&after_t, &after_f, span)?;
                Ok(())
            }
            TExprKind::Lambda(params, body) => {
                self.in_lambda.push(params.clone());
                let r = self.walk(body, Take::Move);
                self.in_lambda.pop();
                r
            }
            TExprKind::Block(stmts, last) => {
                let mut own = Vec::new();
                for s in stmts {
                    match s {
                        TStmt::Let(p, init) => {
                            let linear = self.linear(&init.ty);
                            self.walk(init, if linear { Take::Move } else { Take::Borrow })?;
                            self.bind(p, &init.ty, linear, init.span)?;
                            Self::bound(p, &mut own);
                        }
                        TStmt::Expr(x) => self.walk(x, Take::Borrow)?,
                    }
                }
                self.walk(last, take)?;
                self.all_gone(&own, last.span)?;
                for l in &own {
                    self.st.remove(l);
                }
                Ok(())
            }
            TExprKind::Index(a, b) | TExprKind::Pow(a, b) | TExprKind::Binary { lhs: a, rhs: b, .. } => {
                self.walk(a, Take::Borrow)?;
                self.walk(b, Take::Borrow)
            }
            TExprKind::Slice(a, b, c) => {
                self.walk(a, Take::Borrow)?;
                self.walk(b, Take::Borrow)?;
                self.walk(c, Take::Borrow)
            }
            TExprKind::Update { target, index, value, .. } => {
                self.walk(target, Take::Borrow)?;
                self.walk(index, Take::Borrow)?;
                self.walk(value, Take::Borrow)
            }
            TExprKind::Unary(_, x) => self.walk(x, Take::Borrow),
        }
    }
}

fn sums_for_linear(g: &Globals) -> Vec<SumDef> {
    g.sums.clone()
}

/// The resources of the system are linear (D339): each local that holds a
/// connection or a listener is closed, or given away (to a value, to a
/// function that takes it, as a result), exactly once on every path; a use
/// after it, a value lost, a capture by a lambda, a close on one path only
/// are refused. Whether a function takes each of its parameters is found
/// from its body (again until nothing changes, for the recursive ones).
fn check_linear(fns: &[TFn], sums: &[SumDef]) -> Result<()> {
    let any = fns.iter().any(|f| f.locals.iter().any(|l| linear_ty(&l.ty, sums, &mut Vec::new())));
    if !any {
        return Ok(());
    }
    let mut consumes: Vec<Vec<bool>> = fns.iter().map(|f| vec![false; f.params.len()]).collect();
    let with = |f: &TFn| f.locals.iter().any(|l| linear_ty(&l.ty, sums, &mut Vec::new()));
    // what each function takes, found again until nothing changes (a
    // function whose check fails keeps what it had: the last pass tells)
    for _round in 0..=fns.len() {
        let mut changed = false;
        for (i, f) in fns.iter().enumerate() {
            if !with(f) {
                continue;
            }
            let mut cx = LinCx { f, sums, consumes: &consumes, st: HashMap::new(), in_lambda: Vec::new() };
            for &p in &f.params {
                if cx.linear(&f.locals[p as usize].ty) {
                    cx.st.insert(p, Lin::Live);
                }
            }
            if cx.walk(&f.body, Take::Move).is_err() {
                // a recursive function that takes its parameters on every
                // path (a loop that closes its server at its ends, D342):
                // tried as taking them all, kept when that holds
                let trial: Vec<bool> = f.params.iter().map(|p| cx.linear(&f.locals[*p as usize].ty)).collect();
                if trial != consumes[i] {
                    let mut alt = consumes.clone();
                    alt[i] = trial.clone();
                    let mut cx = LinCx { f, sums, consumes: &alt, st: HashMap::new(), in_lambda: Vec::new() };
                    for &p in &f.params {
                        if cx.linear(&f.locals[p as usize].ty) {
                            cx.st.insert(p, Lin::Live);
                        }
                    }
                    if cx.walk(&f.body, Take::Move).is_ok() {
                        let now: Vec<bool> = f.params.iter().map(|p| matches!(cx.st.get(p), Some(Lin::Gone(_)))).collect();
                        if now == trial {
                            consumes[i] = trial;
                            changed = true;
                        }
                    }
                }
                continue;
            }
            let now: Vec<bool> = f.params.iter().map(|p| matches!(cx.st.get(p), Some(Lin::Gone(_)))).collect();
            if now != consumes[i] {
                consumes[i] = now;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // the check, with what each function takes
    for f in fns.iter().filter(|f| with(f)) {
        let mut cx = LinCx { f, sums, consumes: &consumes, st: HashMap::new(), in_lambda: Vec::new() };
        for &p in &f.params {
            if cx.linear(&f.locals[p as usize].ty) {
                cx.st.insert(p, Lin::Live);
            }
        }
        cx.walk(&f.body, Take::Move)?;
    }
    Ok(())
}
