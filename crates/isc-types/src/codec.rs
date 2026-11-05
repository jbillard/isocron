//! The decoders and encoders of JSON written for the types of a program
//! (D325, D336): for each `T` of a `decode<T>(text)` or an `encode(x)` of
//! the module `json`, functions of Isocron added to that module and checked
//! as it is. A value is read from a segment of the text in two passes: a
//! check, whose fault names the path of the value, then the building, which
//! cannot fail; an array of values holding arrays (rows) checks that their
//! lengths agree before it is built, as the rows of an array of arrays have
//! one length.

use crate::tast::SumDef;
use crate::types::{IntKind, Size, Ty};
use isc_syntax::{Diagnostic, Span};
use std::collections::HashMap;
use std::fmt::Write;

/// The forms of value that JSON carries.
#[derive(Clone, Debug, PartialEq)]
enum Shape {
    Bool,
    Int(IntKind),
    F64,
    F32,
    Str,
    Opt(Box<Shape>),
    /// An array: its length when the type fixes it.
    Arr(Option<i64>, Box<Shape>),
    Tuple(Vec<Shape>),
    /// A record, its fields in the order of their names.
    Record(Vec<(String, Shape)>),
}

/// The form of `t`, or why JSON cannot carry it, naming where in `t`.
fn shape_of(t: &Ty, sums: &[SumDef], at: &str) -> Result<Shape, String> {
    let place = |what: &str| if at.is_empty() { what.to_string() } else { format!("{} at `{}`", what, at) };
    Ok(match t {
        Ty::Bool => Shape::Bool,
        Ty::Int(k) => Shape::Int(*k),
        Ty::F64 => Shape::F64,
        Ty::F32 => Shape::F32,
        Ty::Str => Shape::Str,
        Ty::Sum { id, args, .. } if sums[*id as usize].name == "Option" && args.len() == 1 => {
            let inner = shape_of(&args[0], sums, at)?;
            if matches!(inner, Shape::Opt(_)) {
                return Err(place("an `Option` of an `Option` (both are `null` in JSON)"));
            }
            Shape::Opt(Box::new(inner))
        }
        Ty::Array(size, e) => {
            let fixed = match size {
                Size::Poly(p) => p.as_const(),
                Size::Var(_) => None,
            };
            let inner = shape_of(e, sums, &format!("{}[]", at))?;
            if holds_array(&inner) && holds_option_of_array(&inner) {
                return Err(place("an `Option` holding an array, in an array (its rows must have one length)"));
            }
            Shape::Arr(fixed, Box::new(inner))
        }
        Ty::Tuple(ts) => Shape::Tuple(ts.iter().enumerate().map(|(i, x)| shape_of(x, sums, &format!("{}[{}]", at, i))).collect::<Result<_, _>>()?),
        Ty::Record(fs) => Shape::Record(fs.iter().map(|(n, x)| Ok((n.clone(), shape_of(x, sums, &format!("{}.{}", at, n))?))).collect::<Result<_, String>>()?),
        Ty::Unit => return Err(place("`void`")),
        Ty::BigInt => return Err(place("a `bigint` (JSON numbers are read as `f64` or integers of 64 bits)")),
        Ty::Fn(..) => return Err(place("a function")),
        Ty::RegExp => return Err(place("a `RegExp`")),
        Ty::Sum { id, .. } if sums[*id as usize].variants.len() == 1 && sums[*id as usize].variants[0].name == crate::MAP_CTOR => return Err(place("a `Map` (decode an object into a record, or into `json.Json`)")),
        Ty::Sum { name, .. } => return Err(place(&format!("the sum type `{}` (of the sum types, JSON carries `Option` only)", name))),
        Ty::Param(_) | Ty::Var(_) => return Err(place("a type parameter (the type must be known where it is called)")),
    })
}

/// `s` holds an array whose length the type does not fix.
fn holds_array(s: &Shape) -> bool {
    match s {
        Shape::Arr(None, _) => true,
        Shape::Arr(Some(_), e) | Shape::Opt(e) => holds_array(e),
        Shape::Tuple(ts) => ts.iter().any(holds_array),
        Shape::Record(fs) => fs.iter().any(|(_, x)| holds_array(x)),
        _ => false,
    }
}

fn holds_option_of_array(s: &Shape) -> bool {
    match s {
        Shape::Opt(e) => holds_array(e),
        Shape::Arr(_, e) => holds_option_of_array(e),
        Shape::Tuple(ts) => ts.iter().any(holds_option_of_array),
        Shape::Record(fs) => fs.iter().any(|(_, x)| holds_option_of_array(x)),
        _ => false,
    }
}

/// The key of a form: the same for the same type, whatever its sizes.
fn key(s: &Shape) -> String {
    match s {
        Shape::Bool => "boolean".into(),
        Shape::Int(k) => k.name().into(),
        Shape::F64 => "f64".into(),
        Shape::F32 => "f32".into(),
        Shape::Str => "string".into(),
        Shape::Opt(e) => format!("Option<{}>", key(e)),
        Shape::Arr(n, e) => format!("{}[{}]", key(e), n.map(|n| n.to_string()).unwrap_or_else(|| "_".into())),
        Shape::Tuple(ts) => format!("[{}]", ts.iter().map(key).collect::<Vec<_>>().join(", ")),
        Shape::Record(fs) => format!("{{{}}}", fs.iter().map(|(n, x)| format!("{}: {}", n, key(x))).collect::<Vec<_>>().join(", ")),
    }
}

/// The key of the type `t` of a call of `decode` or `encode`, if JSON
/// carries it.
pub fn codec_key(t: &Ty, sums: &[SumDef]) -> Option<String> {
    shape_of(t, sums, "").ok().map(|s| key(&s))
}

/// The type of `s` in Isocron, each array of its own size `q0`, `q1`, ...
fn render(s: &Shape, next: &mut usize) -> String {
    match s {
        Shape::Bool => "boolean".into(),
        Shape::Int(k) => k.name().into(),
        Shape::F64 => "f64".into(),
        Shape::F32 => "f32".into(),
        Shape::Str => "string".into(),
        Shape::Opt(e) => format!("Option<{}>", render(e, next)),
        Shape::Arr(n, e) => {
            let inner = render(e, next);
            match n {
                Some(n) => format!("{}[{}]", inner, n),
                None => {
                    *next += 1;
                    format!("{}[q{}]", inner, *next - 1)
                }
            }
        }
        Shape::Tuple(ts) => format!("[{}]", ts.iter().map(|t| render(t, next)).collect::<Vec<_>>().join(", ")),
        Shape::Record(fs) => format!("{{ {} }}", fs.iter().map(|(n, x)| format!("{}: {}", n, render(x, next))).collect::<Vec<_>>().join(", ")),
    }
}

const SEG: &str = "b: u8[n], text: string, ix: Index, s: i64, e: i64";

struct Gen<'a> {
    out: String,
    ids: HashMap<String, usize>,
    shapes: Vec<Shape>,
    _sums: &'a [SumDef],
}

impl Gen<'_> {
    /// The number of the functions of `s`, written with those of its parts.
    fn id(&mut self, s: &Shape) -> usize {
        let k = key(s);
        if let Some(&i) = self.ids.get(&k) {
            return i;
        }
        let i = self.shapes.len();
        self.ids.insert(k, i);
        self.shapes.push(s.clone());
        self.write(i, s);
        i
    }

    fn write(&mut self, i: usize, s: &Shape) {
        let ty = render(s, &mut 0);
        let (check, build, encode) = match s {
            Shape::Bool => (
                "return checkBool(b, text, s, e);".to_string(),
                "return boolValue(b, s, e);".to_string(),
                "return x ? \"true\" : \"false\";".to_string(),
            ),
            Shape::Int(k) => {
                let (neg, pos) = int_range(*k);
                let build = match k {
                    IntKind::U64 => "return unsignedValue(b, s, e);".to_string(),
                    IntKind::I64 => "return integerValue(b, s, e);".to_string(),
                    k => format!("return {}(integerValue(b, s, e));", k.name()),
                };
                (format!("return checkInteger(b, s, e, {}u64, {}u64, \"{}\");", neg, pos, k.name()), build, "return toString(x);".to_string())
            }
            Shape::F64 => (
                "return checkNumber(b, s, e);".to_string(),
                "return numberValue(b, text, s, e);".to_string(),
                "return numberText(x);".to_string(),
            ),
            Shape::F32 => (
                "return checkNumber(b, s, e);".to_string(),
                "return f32(numberValue(b, text, s, e));".to_string(),
                "return number32Text(x);".to_string(),
            ),
            Shape::Str => (
                "return checkString(b, text, ix, s, e);".to_string(),
                "return stringValue(b, text, ix, s, e);".to_string(),
                "return quote(x);".to_string(),
            ),
            Shape::Opt(inner) => {
                let j = self.id(inner);
                (
                    format!("return isNull(b, text, s, e) ? fine() : codecCheck{}(b, text, ix, s, e);", j),
                    format!("return isNull(b, text, s, e) ? None : Some(codecBuild{}(b, text, ix, s, e));", j),
                    format!("return match (x) {{\n        Some(v) => codecEncode{}(v),\n        None => \"null\",\n    }};", j),
                )
            }
            Shape::Arr(n, inner) => {
                let j = self.id(inner);
                let mut check = String::from("const [o, c, at, why] = containerOf(b, ix, s, e, 91u8);\n    if (at >= 0) {\n        return [at, why] as const;\n    }\n    const [ss, es] = elements(b, ix, o, c);\n");
                if let Some(n) = n {
                    let _ = write!(check, "    if (ss.length != {}) {{\n        return [o, \": an array of {} elements expected\"] as const;\n    }}\n", n, n);
                }
                let _ = write!(check, "    const [fat, fwhy] = firstFault(ss, es, (x, y) => codecCheck{}(b, text, ix, x, y));\n    if (fat >= 0) {{\n        return [fat, fwhy] as const;\n    }}\n", j);
                if holds_array(inner) {
                    // the rows of an array have one length
                    let _ = write!(
                        check,
                        "    const shapes = map(iota(ss.length), (k) => codecShape{}(b, text, ix, ss[k], es[k]));\n    const odd = filter(iota(ss.length), (k) => count(iota(shapes[0].length), (q) => shapes[k][q] != shapes[0][q]) > 0);\n    if (odd.length > 0) {{\n        return [skip(b, ss[odd[0]]), `[${{odd[0]}}]: an array of another length than in the first element (an array of arrays has rows of one length)`] as const;\n    }}\n",
                        j
                    );
                }
                check.push_str("    return fine();");
                let len = match n {
                    Some(n) => n.to_string(),
                    None => "ss.length".to_string(),
                };
                let build = format!(
                    "const [a, z] = trim(b, s, e);\n    const [ss, es] = elements(b, ix, a, z - 1);\n    return map(iota({}), (k) => codecBuild{}(b, text, ix, ss[k], es[k]));",
                    len, j
                );
                let encode = format!("return \"[\" + join(map(x, (v) => codecEncode{}(v)), \",\") + \"]\";", j);
                (check, build, encode)
            }
            Shape::Tuple(ts) => {
                let js: Vec<usize> = ts.iter().map(|t| self.id(t)).collect();
                let mut check = format!(
                    "const [o, c, at, why] = containerOf(b, ix, s, e, 91u8);\n    if (at >= 0) {{\n        return [at, why] as const;\n    }}\n    const [ss, es] = elements(b, ix, o, c);\n    if (ss.length != {}) {{\n        return [o, \": an array of {} elements expected\"] as const;\n    }}\n",
                    ts.len(),
                    ts.len()
                );
                for (k, j) in js.iter().enumerate() {
                    let _ = write!(check, "    const [at{k}, why{k}] = codecCheck{j}(b, text, ix, ss[{k}], es[{k}]);\n    if (at{k} >= 0) {{\n        return [at{k}, `[{k}]${{why{k}}}`] as const;\n    }}\n");
                }
                check.push_str("    return fine();");
                let parts: Vec<String> = js.iter().enumerate().map(|(k, j)| format!("codecBuild{}(b, text, ix, ss[{}], es[{}])", j, k, k)).collect();
                let build = format!("const [a, z] = trim(b, s, e);\n    const [ss, es] = elements(b, ix, a, z - 1);\n    return [{}] as const;", parts.join(", "));
                let names: Vec<String> = (0..ts.len()).map(|k| format!("v{}", k)).collect();
                let encs: Vec<String> = js.iter().enumerate().map(|(k, j)| format!("codecEncode{}(v{})", j, k)).collect();
                let encode = format!("const [{}] = x;\n    return \"[\" + {} + \"]\";", names.join(", "), encs.join(" + \",\" + "));
                (check, build, encode)
            }
            Shape::Record(fs) => {
                let js: Vec<usize> = fs.iter().map(|(_, t)| self.id(t)).collect();
                // the check of a member by its name
                let mut member = format!("function codecMember{}(b: u8[n], text: string, ix: Index, k: string, s: i64, e: i64): [i64, string] {{\n", i);
                for ((name, _), j) in fs.iter().zip(&js) {
                    let _ = write!(member, "    if (k == \"{}\") {{\n        return codecCheck{}(b, text, ix, s, e);\n    }}\n", name, j);
                }
                member.push_str("    return checkAny(b, text, ix, s, e);\n}\n\n");
                self.out.push_str(&member);
                let mut check = format!(
                    "const [o, c, at, why] = containerOf(b, ix, s, e, 123u8);\n    if (at >= 0) {{\n        return [at, why] as const;\n    }}\n    const [ks, vs, ve, kat, kwhy] = members(b, text, ix, o, c);\n    if (kat >= 0) {{\n        return [kat, kwhy] as const;\n    }}\n    const [mat, mwhy] = firstMemberFault(ks, vs, ve, (k, x, y) => codecMember{i}(b, text, ix, k, x, y));\n    if (mat >= 0) {{\n        return [mat, mwhy] as const;\n    }}\n"
                );
                for (name, t) in fs {
                    if !matches!(t, Shape::Opt(_)) {
                        let _ = write!(check, "    if (lastKey(ks, \"{name}\") < 0) {{\n        return [o, \".{name}: missing\"] as const;\n    }}\n");
                    }
                }
                check.push_str("    return fine();");
                let mut build = String::from("const [a, z] = trim(b, s, e);\n    const [ks, vs, ve, kat, kwhy] = members(b, text, ix, a, z - 1);\n");
                for (k, (name, _)) in fs.iter().enumerate() {
                    let _ = writeln!(build, "    const i{} = lastKey(ks, \"{}\");", k, name);
                }
                let fields: Vec<String> = fs
                    .iter()
                    .zip(&js)
                    .enumerate()
                    .map(|(k, ((name, t), j))| match t {
                        // a field `Option` missing: `None`
                        Shape::Opt(inner) => {
                            let ji = self.ids[&key(inner)];
                            format!("{}: i{k} < 0 || isNull(b, text, vs[i{k}], ve[i{k}]) ? None : Some(codecBuild{ji}(b, text, ix, vs[i{k}], ve[i{k}]))", name)
                        }
                        _ => format!("{}: codecBuild{}(b, text, ix, vs[i{k}], ve[i{k}])", name, j),
                    })
                    .collect();
                let _ = write!(build, "    return {{ {} }};", fields.join(", "));
                let parts: Vec<String> = fs.iter().zip(&js).map(|((name, _), j)| format!("\"\\\"{}\\\":\" + codecEncode{}(x.{})", name, j, name)).collect();
                let encode = if parts.is_empty() { "return \"{}\";".to_string() } else { format!("return \"{{\" + {} + \"}}\";", parts.join(" + \",\" + ")) };
                (check, build, encode)
            }
        };
        let _ = write!(self.out, "function codecCheck{i}({SEG}): [i64, string] {{\n    {check}\n}}\n\n");
        let _ = write!(self.out, "function codecBuild{i}({SEG}): {ty} {{\n    {build}\n}}\n\n");
        let _ = write!(self.out, "function codecEncode{i}(x: {ty}): string {{\n    {encode}\n}}\n\n");
        if holds_array(s) {
            let shape = self.shape_fn(s);
            let _ = write!(self.out, "function codecShape{i}({SEG}): i64[{}] {{\n    {shape}\n}}\n\n", shape_len(s));
        }
    }

    /// The body of the function of the lengths of the arrays of `s`.
    fn shape_fn(&mut self, s: &Shape) -> String {
        match s {
            Shape::Arr(None, _) => "return [countOf(b, ix, s, e)];".to_string(),
            Shape::Arr(Some(_), e) => {
                let j = self.ids[&key(e)];
                format!("const [a, z] = trim(b, s, e);\n    const [ss, es] = elements(b, ix, a, z - 1);\n    return ss.length > 0 ? codecShape{}(b, text, ix, ss[0], es[0]) : fill({}, 0);", j, shape_len(e))
            }
            Shape::Tuple(ts) => {
                let mut body = String::from("const [a, z] = trim(b, s, e);\n    const [ss, es] = elements(b, ix, a, z - 1);\n");
                let mut parts = Vec::new();
                for (k, t) in ts.iter().enumerate() {
                    if holds_array(t) {
                        let j = self.ids[&key(t)];
                        let _ = writeln!(body, "    const p{k} = codecShape{j}(b, text, ix, ss[{k}], es[{k}]);");
                        parts.extend((0..shape_len(t)).map(|q| format!("p{}[{}]", k, q)));
                    }
                }
                let _ = write!(body, "    return [{}];", parts.join(", "));
                body
            }
            Shape::Record(fs) => {
                let mut body = String::from("const [a, z] = trim(b, s, e);\n    const [ks, vs, ve, kat, kwhy] = members(b, text, ix, a, z - 1);\n");
                let mut parts = Vec::new();
                for (k, (name, t)) in fs.iter().enumerate() {
                    if holds_array(t) {
                        let j = self.ids[&key(t)];
                        let _ = writeln!(body, "    const i{k} = lastKey(ks, \"{name}\");\n    const p{k} = codecShape{j}(b, text, ix, vs[i{k}], ve[i{k}]);");
                        parts.extend((0..shape_len(t)).map(|q| format!("p{}[{}]", k, q)));
                    }
                }
                let _ = write!(body, "    return [{}];", parts.join(", "));
                body
            }
            _ => "return [];".to_string(),
        }
    }
}

/// The number of the arrays of `s` whose length the type does not fix.
fn shape_len(s: &Shape) -> usize {
    match s {
        Shape::Arr(None, _) => 1,
        Shape::Arr(Some(_), e) => shape_len(e),
        Shape::Tuple(ts) => ts.iter().map(shape_len).sum(),
        Shape::Record(fs) => fs.iter().map(|(_, x)| shape_len(x)).sum(),
        _ => 0,
    }
}

/// The largest magnitudes of the negative and positive values of `k`.
fn int_range(k: IntKind) -> (u128, u128) {
    let min = k.min_value();
    let max = k.max_value();
    ((-min) as u128, max as u128)
}

/// The text of the functions for the types `ts` (with the places of their
/// calls, for the errors), and the key of each type with the number of its
/// functions `codecDecode{i}` and `codecEncode{i}`.
pub fn codec_source(ts: &[(Ty, Span, bool)], sums: &[SumDef]) -> Result<(String, HashMap<String, usize>), Diagnostic> {
    let mut g = Gen { out: String::new(), ids: HashMap::new(), shapes: Vec::new(), _sums: sums };
    let mut entries = HashMap::new();
    for (t, span, decode) in ts {
        let s = shape_of(t, sums, "").map_err(|why| {
            let what = if *decode { "`json.decode`" } else { "`json.encode`" };
            Diagnostic::new(*span, format!("JSON cannot carry the type of {}: {}", what, why))
        })?;
        let k = key(&s);
        if entries.contains_key(&k) {
            continue;
        }
        let i = g.id(&s);
        entries.insert(k, i);
        let ty = render(&s, &mut 0);
        let _ = write!(
            g.out,
            "function codecDecode{i}(text: string): Result<{ty}, JsonError> {{\n    const b = bytes(text);\n    const ix = indexed(b);\n    const [tat, twhy] = checkText(b, ix);\n    if (tat >= 0) {{\n        return Err(errorAt(b, tat, twhy));\n    }}\n    const [at, why] = codecCheck{i}(b, text, ix, 0, b.length);\n    if (at >= 0) {{\n        return Err(errorAt(b, at, \"$\" + why));\n    }}\n    return Ok(codecBuild{i}(b, text, ix, 0, b.length));\n}}\n\n"
        );
    }
    Ok((g.out, entries))
}
