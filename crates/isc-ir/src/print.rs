//! Textual dump of the IR (`iscc ir`), for debugging and tests.

use crate::*;
use std::fmt::Write;

pub fn type_str(t: &Type) -> String {
    match t {
        Type::Int(k) => k.name().to_string(),
        Type::F64 => "f64".into(),
        Type::F32 => "f32".into(),
        Type::Bool => "boolean".into(),
        Type::Unit => "void".into(),
        Type::Str => "string".into(),
        Type::Big => "bigint".into(),
        Type::Tuple(ts) => format!("[{}]", ts.iter().map(type_str).collect::<Vec<_>>().join(", ")),
        // (the length fixed by the program, D204, is not shown: the messages
        // and the memory report speak of the array)
        Type::Array(e, _) => format!("{}[]", type_str(e)),
        Type::Rec(i) => format!("rec#{}", i),
        Type::Handle => "handle".into(),
    }
}

pub fn program(p: &Program) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "main(%{}{}):", p.args, if p.drop_args { " drop" } else { "" });
    expr(&p.body, p, 1, &mut s);
    // (not the functions of the allocator of the runtime, in every program,
    // D605: a program reads its own code here)
    let hidden = p.hidden_fns();
    for (i, f) in p.fns.iter().enumerate() {
        if hidden.contains(&(i as u32)) {
            continue;
        }
        let ps: Vec<String> = f.params.iter().map(|v| name(p, *v)).collect();
        let _ = writeln!(s, "fn {}#{}({}):", f.name, i, ps.join(", "));
        expr(&f.body, p, 1, &mut s);
    }
    s
}

fn name(p: &Program, v: VarId) -> String {
    format!("{}%{}", p.vars[v as usize].name, v)
}

fn lambda(l: &Lambda, p: &Program, ind: usize, s: &mut String) {
    let ps: Vec<String> = l
        .params
        .iter()
        .zip(l.drop_params.iter().chain(std::iter::repeat(&false)))
        .map(|(v, d)| format!("{}{}", name(p, *v), if *d { "!drop" } else { "" }))
        .collect();
    let _ = writeln!(s, "{}\\({}) ->", "  ".repeat(ind), ps.join(", "));
    expr(&l.body, p, ind + 1, s);
}

pub fn expr(e: &Expr, p: &Program, ind: usize, s: &mut String) {
    let pad = "  ".repeat(ind);
    use ExprKind::*;
    let head = match &e.kind {
        StrLit(s) => format!("string {:?}", s),
        Take(_, i) => format!("take {}", i),
        StrOp(op, _) => format!("string {:?}", op),
        Lit(crate::Lit::Int(v)) => format!("{}:{}", v, type_str(&e.ty)),
        Lit(crate::Lit::F64(v)) => format!("{:?}", v),
        Lit(crate::Lit::F32(v)) => format!("{:?}f32", v),
        Lit(crate::Lit::Bool(b)) => format!("{}", b),
        Lit(crate::Lit::Unit) => "()".into(),
        Zero => "zero".into(),
        ConstArray(ls) => format!("const array {} ({} elements)", type_str(&e.ty), ls.len()),
        ConstStrs(ss) => format!("const array {} ({} strings)", type_str(&e.ty), ss.len()),
        Var(v, u) => format!("{}{}", name(p, *v), if *u == Use::Move { " (move)" } else { "" }),
        Let { var, stmt, drop, .. } => format!(
            "let{}{} {}: {} =",
            if *stmt { " stmt" } else { "" },
            if *drop { " drop" } else { "" },
            name(p, *var),
            type_str(&p.vars[*var as usize].ty)
        ),
        Tuple(_) => "tuple".into(),
        Proj(_, i) => format!("proj {}", i),
        Unary(op, _) => format!("{:?}", op),
        Binary(op, _, _) => op.text().to_string(),
        Pow(..) => "pow".into(),
        Convert(_) => format!("convert to {}", type_str(&e.ty)),
        Sqrt(_) => "sqrt".into(),
        Math(f, _) => f.name().into(),
        If(..) => "if".into(),
        Len(_) => "len".into(),
        ArrayLit(_) => format!("array {}", type_str(&e.ty)),
        Index(..) => "index".into(),
        Slice(..) => "slice".into(),
        Window(..) => "window".into(),
        Update { require_unique, .. } => format!("update{}", if *require_unique { " requireUnique" } else { "" }),
        Map(ins, _) => format!("map/{} -> {}", ins.len(), type_str(&e.ty)),
        Reduce(..) => format!("reduce -> {}", type_str(&e.ty)),
        Scan(..) => format!("scan -> {}", type_str(&e.ty)),
        Filter(..) => format!("filter -> {}", type_str(&e.ty)),
        Tasks(..) => format!("mapParallel -> {}", type_str(&e.ty)),
        Fold(..) => format!("fold -> {}", type_str(&e.ty)),
        Zeros(_) => format!("zeros {}", type_str(&e.ty)),
        Iota(_) => "iota".into(),
        Fill(..) => "fill".into(),
        CheckSize(..) => "check_size".into(),
        CheckNonNegative(..) => "check_non_negative".into(),
        Copy(_) => "copy".into(),
        Action(op, _) => format!("action {:?}", op),
        Bench(..) => "bench".into(),
        Call(f, _) => format!("call {}#{}", p.fns[*f as usize].name, f),
        Box(_) => "box".into(),
        Unbox(_) => "unbox".into(),
        Free(v, _) => format!("free {}", name(p, *v)),
    };
    let _ = writeln!(s, "{}{}", pad, head);
    match &e.kind {
        Map(ins, l) => {
            for i in ins {
                expr(i, p, ind + 1, s);
            }
            lambda(l, p, ind + 1, s);
        }
        Reduce(x, l, z) | Scan(x, l, z) => {
            expr(x, p, ind + 1, s);
            lambda(l, p, ind + 1, s);
            expr(z, p, ind + 1, s);
        }
        Fold(x, i, l) => {
            expr(x, p, ind + 1, s);
            expr(i, p, ind + 1, s);
            lambda(l, p, ind + 1, s);
        }
        Update { target, index, value, .. } => {
            expr(target, p, ind + 1, s);
            expr(index, p, ind + 1, s);
            expr(value, p, ind + 1, s);
        }
        _ => {
            for c in e.children() {
                expr(c, p, ind + 1, s);
            }
        }
    }
}
