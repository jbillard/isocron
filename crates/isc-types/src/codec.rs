//! The decoders and encoders of JSON written for the types of a program
//! (D325, D336): for each `T` of a `decode<T>(text)` or an `encode(x)` of
//! the module `json`, functions of Isocron added to that module and checked
//! as it is. A value is read from its place in two passes, as a cursor
//! (D337): a check, which returns the place after the value or its fault
//! with the path of the value, then the building, which cannot fail; the
//! members of a record are read one after the other by a tail recursion,
//! their names compared byte by byte where they are; the elements of an
//! array, found by the index of the text, are read in parallel. An array of
//! values holding arrays (rows) checks that their lengths agree before it
//! is built, as the rows of an array of arrays have one length.

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
    /// An array of its own size (`T[]`, D341): each value its own length,
    /// never compared with the others.
    Own(Box<Shape>),
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
                Size::Var(_) | Size::Own => None,
            };
            let inner = shape_of(e, sums, &format!("{}[]", at))?;
            if holds_array(&inner) && holds_option_of_array(&inner) {
                return Err(place("an `Option` holding an array, in an array (its rows must have one length)"));
            }
            if *size == Size::Own {
                Shape::Own(Box::new(inner))
            } else {
                Shape::Arr(fixed, Box::new(inner))
            }
        }
        Ty::Tuple(ts) => Shape::Tuple(ts.iter().enumerate().map(|(i, x)| shape_of(x, sums, &format!("{}[{}]", at, i))).collect::<Result<_, _>>()?),
        Ty::Record(fs) if fs.len() > MAX_FIELDS => return Err(place(&format!("a record of {} fields (at most {}: their presence is checked by the bits of one integer)", fs.len(), MAX_FIELDS))),
        Ty::Record(fs) => Shape::Record(fs.iter().map(|(n, x)| Ok((n.clone(), shape_of(x, sums, &format!("{}.{}", at, n))?))).collect::<Result<_, String>>()?),
        Ty::Unit => return Err(place("`void`")),
        Ty::BigInt => return Err(place("a `bigint` (JSON numbers are read as `f64` or integers of 64 bits)")),
        Ty::Fn(..) => return Err(place("a function")),
        Ty::RegExp => return Err(place("a `RegExp`")),
        Ty::Handle(h) => return Err(place(&format!("a `{}` (a resource of the system)", h.name()))),
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
        Shape::Arr(_, e) | Shape::Own(e) => holds_option_of_array(e),
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
        Shape::Own(e) => format!("{}[]", key(e)),
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
    render_with(s, next, "q")
}

/// The type of `s`, its sizes named `<prefix>0`, `<prefix>1`, ...
fn render_with(s: &Shape, next: &mut usize, prefix: &str) -> String {
    let render = |s: &Shape, next: &mut usize| render_with(s, next, prefix);
    match s {
        Shape::Bool => "boolean".into(),
        Shape::Int(k) => k.name().into(),
        Shape::F64 => "f64".into(),
        Shape::F32 => "f32".into(),
        Shape::Str => "string".into(),
        Shape::Opt(e) => format!("Option<{}>", render(e, next)),
        Shape::Own(e) => format!("{}[]", render(e, next)),
        Shape::Arr(n, e) => {
            let inner = render(e, next);
            match n {
                Some(n) => format!("{}[{}]", inner, n),
                None => {
                    *next += 1;
                    format!("{}[{}{}]", inner, prefix, *next - 1)
                }
            }
        }
        Shape::Tuple(ts) => format!("[{}]", ts.iter().map(|t| render(t, next)).collect::<Vec<_>>().join(", ")),
        Shape::Record(fs) => format!("{{ {} }}", fs.iter().map(|(n, x)| format!("{}: {}", n, render(x, next))).collect::<Vec<_>>().join(", ")),
    }
}

/// The parameters of a function that reads a value at the place `at`.
const AT: &str = "b: u8[n], text: string, ix: Index, at: i64";

/// The most fields of a record: their presence is the bits of an `i64`.
const MAX_FIELDS: usize = 62;

/// A value of `s` in Isocron (the accumulators of a record before its
/// members are read).
fn default_of(s: &Shape) -> String {
    match s {
        Shape::Bool => "false".into(),
        Shape::Int(k) => format!("{}(0)", k.name()),
        Shape::F64 => "0.0".into(),
        Shape::F32 => "0.0f32".into(),
        Shape::Str => "\"\"".into(),
        Shape::Opt(_) => "None".into(),
        Shape::Arr(n, e) => format!("fill({}, {})", n.unwrap_or(0), default_of(e)),
        Shape::Own(_) => "[]".into(),
        Shape::Tuple(ts) => format!("[{}] as const", ts.iter().map(default_of).collect::<Vec<_>>().join(", ")),
        Shape::Record(fs) => format!("{{ {} }}", fs.iter().map(|(n, x)| format!("{}: {}", n, default_of(x))).collect::<Vec<_>>().join(", ")),
    }
}

struct Gen {
    out: String,
    ids: HashMap<String, usize>,
}

impl Gen {
    /// The number of the functions of `s`, written with those of its parts.
    fn id(&mut self, s: &Shape) -> usize {
        let k = key(s);
        if let Some(&i) = self.ids.get(&k) {
            return i;
        }
        let i = self.ids.len();
        self.ids.insert(k, i);
        self.write(i, s);
        i
    }

    fn write(&mut self, i: usize, s: &Shape) {
        let ty = render(s, &mut 0);
        let (check, build, encode) = match s {
            Shape::Bool => ("return boolAt(b, at);".to_string(), "return boolValueAt(b, at);".to_string(), "return x ? \"true\" : \"false\";".to_string()),
            Shape::Int(k) => {
                let (neg, pos) = int_range(*k);
                let build = match k {
                    IntKind::U64 => "return unsignedValueAt(b, at);".to_string(),
                    IntKind::I64 => "return integerValueAt(b, at);".to_string(),
                    k => format!("const [v, e] = integerValueAt(b, at);\n    return [{}(v), e] as const;", k.name()),
                };
                (format!("return integerAt(b, at, {}u64, {}u64, \"{}\");", neg, pos, k.name()), build, "return toString(x);".to_string())
            }
            Shape::F64 => ("return numberAt(b, at);".to_string(), "return numberValueAt(b, text, at);".to_string(), "return numberText(x);".to_string()),
            Shape::F32 => (
                "return numberAt(b, at);".to_string(),
                "const [v, e] = numberValueAt(b, text, at);\n    return [f32(v), e] as const;".to_string(),
                "return number32Text(x);".to_string(),
            ),
            Shape::Str => ("return stringCheckAt(b, text, ix, at);".to_string(), "return stringValueAt(b, text, ix, at);".to_string(), "return quote(x);".to_string()),
            Shape::Opt(inner) => {
                let j = self.id(inner);
                (
                    format!("const z = nullAt(b, at);\n    return z >= 0 ? [z, -1, \"\"] as const : codecCheck{j}(b, text, ix, at);"),
                    format!("const z = nullAt(b, at);\n    if (z >= 0) {{\n        return [None, z] as const;\n    }}\n    const [v, e] = codecBuild{j}(b, text, ix, at);\n    return [Some(v), e] as const;"),
                    format!("return match (x) {{\n        Some(v) => codecEncode{j}(v),\n        None => \"null\",\n    }};"),
                )
            }
            Shape::Arr(_, inner) | Shape::Own(inner) => {
                let n = match &s {
                    Shape::Arr(n, _) => *n,
                    _ => None,
                };
                let j = self.id(inner);
                let mut check = format!("const [o, c, p, why] = arrayCheckAt(b, ix, at, (x) => codecCheck{j}(b, text, ix, x));\n    if (p >= 0) {{\n        return [c + 1, p, why] as const;\n    }}\n");
                if let Some(n) = n {
                    let _ = write!(check, "    if (elementCount(b, ix, o, c) != {n}) {{\n        return [c + 1, o, \": an array of {n} elements expected\"] as const;\n    }}\n");
                }
                if holds_array(inner) {
                    // the rows of an array have one length
                    let _ = write!(
                        check,
                        "    const [ss, es] = elements(b, ix, o, c);\n    const shapes = map(iota(ss.length), (k) => codecShape{j}(b, text, ix, ss[k]));\n    const odd = filter(iota(ss.length), (k) => count(iota(shapes[0].length), (q) => shapes[k][q] != shapes[0][q]) > 0);\n    if (odd.length > 0) {{\n        return [c + 1, skip(b, ss[odd[0]]), `[${{odd[0]}}]: an array of another length than in the first element (an array of arrays has rows of one length)`] as const;\n    }}\n"
                    );
                }
                check.push_str("    return [c + 1, -1, \"\"] as const;");
                let len = n.map(|n| n.to_string()).unwrap_or_else(|| "ss.length".into());
                let build = format!("const [ss, after] = arrayStarts(b, ix, at);\n    return [map(iota({len}), (k) => {{\n        const [v, e] = codecBuild{j}(b, text, ix, ss[k]);\n        return v;\n    }}), after] as const;");
                let encode = format!("return \"[\" + join(map(x, (v) => codecEncode{j}(v)), \",\") + \"]\";");
                (check, build, encode)
            }
            Shape::Tuple(ts) => {
                let js: Vec<usize> = ts.iter().map(|t| self.id(t)).collect();
                let l = ts.len();
                let mut check = format!("const a = skip(b, at);\n    if (a >= b.length || b[a] != 91u8) {{\n        return [a, a, \": an array expected\"] as const;\n    }}\n    const c = closingOf(b, ix, a);\n    if (c < 0) {{\n        return [b.length, a, \": a bracket without its end\"] as const;\n    }}\n    if (b[c] != 93u8) {{\n        return [c + 1, c, \": a bracket closed by the other kind\"] as const;\n    }}\n    const [ss, es] = elements(b, ix, a, c);\n    if (ss.length != {l}) {{\n        return [c + 1, a, \": an array of {l} elements expected\"] as const;\n    }}\n");
                for (k, j) in js.iter().enumerate() {
                    let _ = write!(check, "    const [e{k}, p{k}, w{k}] = codecCheck{j}(b, text, ix, ss[{k}]);\n    if (p{k} >= 0) {{\n        return [c + 1, p{k}, `[{k}]${{w{k}}}`] as const;\n    }}\n    if (skip(b, e{k}) != es[{k}]) {{\n        return [c + 1, skip(b, e{k}), \"[{k}]: a comma or a closing bracket expected\"] as const;\n    }}\n");
                }
                check.push_str("    return [c + 1, -1, \"\"] as const;");
                let mut build = String::from("const [ss, after] = arrayStarts(b, ix, at);\n");
                for (k, j) in js.iter().enumerate() {
                    let _ = writeln!(build, "    const [v{k}, e{k}] = codecBuild{j}(b, text, ix, ss[{k}]);");
                }
                let vs: Vec<String> = (0..l).map(|k| format!("v{}", k)).collect();
                let _ = write!(build, "    return [[{}] as const, after] as const;", vs.join(", "));
                let encs: Vec<String> = js.iter().enumerate().map(|(k, j)| format!("codecEncode{}(v{})", j, k)).collect();
                let encode = format!("const [{}] = x;\n    return \"[\" + {} + \"]\";", vs.join(", "), encs.join(" + \",\" + "));
                (check, build, encode)
            }
            Shape::Record(fs) => {
                let js: Vec<usize> = fs.iter().map(|(_, t)| self.id(t)).collect();
                self.record(i, fs, &js, &ty);
                let check = "const [m, p, why] = objectOpen(b, at);\n    if (p >= 0) {\n        return [m, p, why] as const;\n    }\n    const z = emptyObject(b, m);\n    if (z >= 0) {\n        return codecMissing{i}(m - 1, z, 0);\n    }\n    return codecMembers{i}(b, text, ix, m, 0, m - 1);".replace("{i}", &i.to_string());
                let defaults: Vec<String> = fs.iter().map(|(_, t)| default_of(t)).collect();
                let build = format!(
                    "const a = skip(b, at);\n    const z = emptyObject(b, a + 1);\n    if (z >= 0) {{\n        return [{}, z] as const;\n    }}\n    return codecBuildMembers{i}(b, text, ix, a + 1, {});",
                    default_of(s),
                    defaults.join(", ")
                );
                let parts: Vec<String> = fs.iter().zip(&js).map(|((name, _), j)| format!("\"\\\"{}\\\":\" + codecEncode{}(x.{})", name, j, name)).collect();
                let encode = format!("return \"{{\" + {} + \"}}\";", parts.join(" + \",\" + "));
                (check, build, encode)
            }
        };
        let _ = write!(self.out, "function codecCheck{i}({AT}): [i64, i64, string] {{\n    {check}\n}}\n\n");
        let _ = write!(self.out, "function codecBuild{i}({AT}): [{ty}, i64] {{\n    {build}\n}}\n\n");
        let _ = write!(self.out, "function codecEncode{i}(x: {ty}): string {{\n    {encode}\n}}\n\n");
        if holds_array(s) {
            let shape = self.shape_fn(i, s);
            let _ = write!(self.out, "function codecShape{i}({AT}): i64[{}] {{\n    {shape}\n}}\n\n", shape_len(s));
        }
    }

    /// The functions of the members of the record `i`: the number of the
    /// field a key names, the loop of the check and of the building, the
    /// fields missing.
    fn record(&mut self, i: usize, fs: &[(String, Shape)], js: &[usize], ty: &str) {
        // a key without escape compared byte by byte where it is; with one,
        // read first
        let mut field = format!("function codecField{i}(b: u8[n], text: string, ix: Index, a: i64, z: i64): i64 {{\n    if (findByte(b, 92u8, a, z) < z) {{\n        const k = keyText(b, text, ix, a, z);\n");
        for (k, (name, _)) in fs.iter().enumerate() {
            let _ = write!(field, "        if (k == \"{name}\") {{\n            return {k};\n        }}\n");
        }
        field.push_str("        return -1;\n    }\n");
        for (k, (name, _)) in fs.iter().enumerate() {
            let _ = write!(field, "    if (sameBytes(b, a, z, \"{name}\")) {{\n        return {k};\n    }}\n");
        }
        field.push_str("    return -1;\n}\n\n");
        self.out.push_str(&field);
        let mut missing = format!("function codecMissing{i}(o: i64, end: i64, seen: i64): [i64, i64, string] {{\n");
        for (k, (name, t)) in fs.iter().enumerate() {
            if !matches!(t, Shape::Opt(_)) {
                let _ = write!(missing, "    if ((seen & (1 << {k})) == 0) {{\n        return [end, o, \".{name}: missing\"] as const;\n    }}\n");
            }
        }
        missing.push_str("    return [end, -1, \"\"] as const;\n}\n\n");
        self.out.push_str(&missing);
        // the check of the members, one after the other
        let mut chain = String::new();
        for (k, j) in js.iter().enumerate() {
            let _ = write!(chain, "f == {k} ? codecCheck{j}(b, text, ix, v) : (");
        }
        chain.push_str("anyAt(b, text, ix, v)");
        chain.push_str(&")".repeat(js.len()));
        let _ = write!(
            self.out,
            "function codecMembers{i}(b: u8[n], text: string, ix: Index, at: i64, seen: i64, o: i64): [i64, i64, string] {{\n    const [ka, kz, v, kp, kwhy] = keyAt(b, ix, at);\n    if (kp >= 0) {{\n        return [kp, kp, kwhy] as const;\n    }}\n    const f = codecField{i}(b, text, ix, ka, kz);\n    const [e, p, why] = {chain};\n    if (p >= 0) {{\n        return [e, p, `.${{keyText(b, text, ix, ka, kz)}}${{why}}`] as const;\n    }}\n    const [next, after] = afterMember(b, e);\n    const now = f >= 0 ? seen | (1 << f) : seen;\n    if (next == 1) {{\n        return codecMembers{i}(b, text, ix, after, now, o);\n    }}\n    if (next == 0) {{\n        return codecMissing{i}(o, after, now);\n    }}\n    return [after, after, \": a comma or a closing brace expected\"] as const;\n}}\n\n"
        );
        // the building of the members: each field carried by the loop
        let mut nr = 0;
        let params: Vec<String> = fs.iter().enumerate().map(|(k, (_, t))| format!("f{}: {}", k, render(t, &mut nr))).collect();
        let mut body = String::from("    const [ka, kz, v, kp, kwhy] = keyAt(b, ix, at);\n    const f = codecField{i}(b, text, ix, ka, kz);\n".replace("{i}", &i.to_string()));
        for (k, j) in js.iter().enumerate() {
            let _ = writeln!(body, "    const [n{k}, e{k}] = f == {k} ? codecBuild{j}(b, text, ix, v) : [f{k}, -1] as const;");
        }
        let mut end = String::new();
        for k in 0..js.len() {
            let _ = write!(end, "f == {k} ? e{k} : (");
        }
        end.push_str("valueAfter(b, ix, v)");
        end.push_str(&")".repeat(js.len()));
        let ns: Vec<String> = (0..js.len()).map(|k| format!("n{}", k)).collect();
        let record: Vec<String> = fs.iter().enumerate().map(|(k, (name, _))| format!("{}: n{}", name, k)).collect();
        let _ = write!(
            body,
            "    const [next, after] = afterMember(b, {end});\n    return next == 1 ? codecBuildMembers{i}(b, text, ix, after, {}) : [{{ {} }}, after] as const;\n",
            ns.join(", "),
            record.join(", ")
        );
        // the sizes of the result its own, not those of the fields carried
        let result = render_with(&Shape::Record(fs.to_vec()), &mut 0, "r");
        let _ = ty;
        let _ = write!(self.out, "function codecBuildMembers{i}(b: u8[n], text: string, ix: Index, at: i64, {}): [{result}, i64] {{\n{body}}}\n\n", params.join(", "));
    }

    /// The body of the function of the lengths of the arrays of `s`.
    fn shape_fn(&mut self, i: usize, s: &Shape) -> String {
        match s {
            Shape::Arr(None, _) => "const a = skip(b, at);\n    return [elementCount(b, ix, a, closingOf(b, ix, a))];".to_string(),
            Shape::Arr(Some(_), e) => {
                let j = self.ids[&key(e)];
                format!("const [ss, after] = arrayStarts(b, ix, at);\n    return ss.length > 0 ? codecShape{}(b, text, ix, ss[0]) : fill({}, 0);", j, shape_len(e))
            }
            Shape::Tuple(ts) => {
                let mut body = String::from("const [ss, after] = arrayStarts(b, ix, at);\n");
                let mut parts = Vec::new();
                for (k, t) in ts.iter().enumerate() {
                    if holds_array(t) {
                        let j = self.ids[&key(t)];
                        let _ = writeln!(body, "    const p{k} = codecShape{j}(b, text, ix, ss[{k}]);");
                        parts.extend((0..shape_len(t)).map(|q| format!("p{}[{}]", k, q)));
                    }
                }
                let _ = write!(body, "    return [{}];", parts.join(", "));
                body
            }
            Shape::Record(fs) => {
                let c = shape_len(s);
                // the lengths of the fields that hold arrays, at their offsets
                let mut cases = String::new();
                let mut lets = String::new();
                let mut off = 0;
                for (k, (_, t)) in fs.iter().enumerate() {
                    let l = shape_len(t);
                    if l > 0 {
                        let j = self.ids[&key(t)];
                        let _ = writeln!(lets, "    const s{k} = f == {k} ? codecShape{j}(b, text, ix, v) : fill({l}, 0);");
                        let _ = write!(cases, "f == {k} && q >= {off} && q < {} ? s{k}[q - {off}] : (", off + l);
                        off += l;
                    }
                }
                let closes = cases.matches('(').count();
                let _ = write!(
                    self.out,
                    "function codecShapeMembers{i}(b: u8[n], text: string, ix: Index, at: i64, acc: i64[{c}]): i64[{c}] {{\n    const [ka, kz, v, kp, kwhy] = keyAt(b, ix, at);\n    const f = codecField{i}(b, text, ix, ka, kz);\n{lets}    const now = map(iota({c}), (q) => {cases}acc[q]{});\n    const [next, after] = afterMember(b, valueAfter(b, ix, v));\n    return next == 1 ? codecShapeMembers{i}(b, text, ix, after, now) : now;\n}}\n\n",
                    ")".repeat(closes)
                );
                format!("const a = skip(b, at);\n    return emptyObject(b, a + 1) >= 0 ? fill({c}, 0) : codecShapeMembers{i}(b, text, ix, a + 1, fill({c}, 0));")
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
    ((-k.min_value()) as u128, k.max_value() as u128)
}

/// The text of the functions for the types `ts` (with the places of their
/// calls, for the errors), and the key of each type with the number of its
/// functions `codecDecode{i}` and `codecEncode{i}`.
pub fn codec_source(ts: &[(Ty, Span, bool)], sums: &[SumDef]) -> Result<(String, HashMap<String, usize>), Diagnostic> {
    let mut g = Gen { out: String::new(), ids: HashMap::new() };
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
            "function codecDecode{i}(text: string): Result<{ty}, JsonError> {{\n    const b = bytes(text);\n    const ix = indexed(b);\n    const [tat, twhy] = checkText(b, ix);\n    if (tat >= 0) {{\n        return Err(errorAt(b, tat, twhy));\n    }}\n    const [end, at, why] = codecCheck{i}(b, text, ix, 0);\n    if (at >= 0) {{\n        return Err(errorAt(b, at, \"$\" + why));\n    }}\n    const after = skip(b, end);\n    if (after != b.length) {{\n        return Err(errorAt(b, after, \"$: text after the value\"));\n    }}\n    const [v, e] = codecBuild{i}(b, text, ix, 0);\n    return Ok(v);\n}}\n\n"
        );
    }
    Ok((g.out, entries))
}
