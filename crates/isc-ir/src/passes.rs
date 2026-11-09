//! Middle-end passes (SPEC sections 7 and 8).
//!
//! 1. `copy_prop`: `let v = w` is removed and `v` renamed to `w`.
//! 2. `fuse`: prepares rule F1. A `let`-bound element-wise producer (`map`,
//!    element-wise operators, `iota`, `fill`, `zeros`) used exactly once, in a
//!    consumer slot (`map` input, `reduce`/`fold` array, indexed array) and not
//!    inside a lambda (where it would be used once per iteration, i.e. more than
//!    once: rule M1), is moved into that slot. A producer sitting directly in a
//!    consumer slot is never materialized: the code generator evaluates its
//!    elements inside the consumer's loop. `let` and size tests that wrap a
//!    producer are floated out of the consumer.
//! 3. `bind_temporaries`: a freshly allocated array consumed by a borrowing
//!    operation (`print`, `writeF64`, an index into a literal, ...) is bound
//!    to a `let`, so that it is freed after use.
//! 4. `ownership`: uniqueness analysis restricted to what the POC needs.
//!    An owned variable (bound to a freshly allocated value, or the
//!    accumulator of a `fold`) is *moved* at its last use when that use is in
//!    the same region (not inside a lambda body or an `if` branch entered after
//!    its definition) and the use consumes the value (target of `with`,
//!    initial value of `fold`, result of a `let`, `fold` or `bench` body,
//!    tuple component). Any other consuming use gets an explicit `Copy`.
//!    `requireUnique` fails when a copy would be needed. Owned variables that
//!    are not moved are freed at the end of their scope.

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::*;
use isc_syntax::{Diagnostic, Span};

type R<T> = std::result::Result<T, Diagnostic>;

/// Runs all passes.
pub fn optimize(p: &mut Program) -> R<()> {
    optimize_body(&mut p.body, &mut p.vars);
    for f in p.fns.iter_mut() {
        optimize_body(&mut f.body, &mut p.vars);
    }
    ownership(p)?;
    borrow_elements(p);
    check_require_tail(p)?;
    // every memory decision checked, independently of the passes (D96)
    crate::verify::verify(p)
}

// ---------------------------------------------------------------------
// copy propagation

fn copy_prop(e: &mut Expr, ren: &mut HashMap<VarId, VarId>) {
    loop {
        if let ExprKind::Var(v, _) = &mut e.kind {
            while let Some(w) = ren.get(v) {
                *v = *w;
            }
            return;
        }
        if let ExprKind::Let { var, init, .. } = &mut e.kind {
            copy_prop(init, ren);
            if let ExprKind::Var(w, _) = init.kind {
                ren.insert(*var, w);
                let body = match std::mem::replace(&mut e.kind, ExprKind::Lit(Lit::Unit)) {
                    ExprKind::Let { body, .. } => body,
                    _ => unreachable!(),
                };
                *e = *body;
                continue;
            }
            if let ExprKind::Let { body, .. } = &mut e.kind {
                copy_prop(body, ren);
            }
            return;
        }
        for c in e.children_mut() {
            copy_prop(c, ren);
        }
        return;
    }
}

// ---------------------------------------------------------------------
// fusion preparation (rule F1)

pub fn is_producer(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Map(..) | ExprKind::Iota(_) | ExprKind::Fill(..) | ExprKind::Zeros(_) => true,
        // a slice of a producer computes only its elements (F1, views)
        ExprKind::Slice(x, _, _) => is_producer(x),
        _ => false,
    }
}

/// A stream (rule F2, D99): a `filter`, or a `map` of one input that is a
/// stream. Its elements come one at a time, without an index: only a
/// consumer that reads its input in order takes it unmaterialized (see
/// `is_stream_slot`).
pub fn is_stream(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Filter(..) => true,
        ExprKind::Map(ins, _) => ins.len() == 1 && is_stream(&ins[0]) && !owns_elems(e),
        _ => false,
    }
}

/// An array whose elements own memory of their own (strings, nodes, D134),
/// not rows: a producer of them is evaluated, never fused into its
/// consumer, which reads its elements without freeing them.
pub fn owns_elems(e: &Expr) -> bool {
    e.ty.is_array() && {
        let t = e.ty.elem();
        t.has_heap() && !t.is_array()
    }
}

/// Slot `i` of `e` reads a stream in order: the array of a `reduce` or a
/// `fold`, the input of a `map` of one input (itself evaluated, or read in
/// order by its consumer).
pub fn is_stream_slot(e: &Expr, i: usize) -> bool {
    i == 0
        && match &e.kind {
            ExprKind::Reduce(..) | ExprKind::Fold(..) => true,
            ExprKind::Map(ins, _) => ins.len() == 1 && !owns_elems(e),
            _ => false,
        }
}

/// A producer possibly wrapped in `let`s of scalars and size checks.
fn is_wrapped_producer(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Let { body, stmt: false, .. } => is_wrapped_producer(body),
        ExprKind::CheckSize(_, _, b) | ExprKind::CheckNonNegative(_, b) => is_wrapped_producer(b),
        _ => is_producer(e),
    }
}

/// Indices (in `children_mut` order) of the fusable consumer slots of `e`.
pub(crate) fn fusable_slots(e: &Expr) -> Vec<usize> {
    match &e.kind {
        ExprKind::Map(ins, _) => (0..ins.len()).collect(),
        // a row of a producer of rows is not computed alone: its memory
        // would have no owner (the lambda may build it or return a view,
        // D336); the producer is evaluated, the row a view of it
        ExprKind::Index(..) if e.ty.is_array() => vec![],
        ExprKind::Reduce(..) | ExprKind::Fold(..) | ExprKind::Scan(..) | ExprKind::Filter(..) | ExprKind::Index(..) | ExprKind::Len(_) => vec![0],
        // the source of a slice: a producer there is read from `a` to `b`
        ExprKind::Slice(..) => vec![0],
        _ => vec![],
    }
}

pub fn is_pure(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Action(..) | ExprKind::Bench(..) | ExprKind::Tasks(..) => false,
        _ => e.children().into_iter().all(is_pure),
    }
}

fn count_uses(e: &Expr, counts: &mut HashMap<VarId, usize>) {
    if let ExprKind::Var(v, _) = e.kind {
        *counts.entry(v).or_insert(0) += 1;
    }
    for c in e.children() {
        count_uses(c, counts);
    }
}

/// Is the (single) use of `v` in `e` directly in a fusable slot, outside any
/// lambda body?
fn use_in_slot(e: &Expr, v: VarId) -> bool {
    let slots = fusable_slots(e);
    let lambda_child = lambda_body_index(e);
    for (i, c) in e.children().into_iter().enumerate() {
        if let ExprKind::Var(w, _) = c.kind {
            if w == v {
                return slots.contains(&i);
            }
        }
        if Some(i) == lambda_child {
            continue; // uses inside a lambda body are repeated uses
        }
        if use_in_slot(c, v) {
            return true;
        }
    }
    false
}

pub(crate) fn lambda_body_index(e: &Expr) -> Option<usize> {
    match &e.kind {
        ExprKind::Map(ins, _) => Some(ins.len()),
        ExprKind::Reduce(..) | ExprKind::Fold(..) | ExprKind::Scan(..) => Some(2),
        ExprKind::Filter(..) | ExprKind::Tasks(..) => Some(1),
        ExprKind::Bench(..) => Some(1),
        _ => None,
    }
}

fn replace_var(e: &mut Expr, v: VarId, with: &mut Option<Expr>) {
    if let ExprKind::Var(w, _) = e.kind {
        if w == v {
            if let Some(x) = with.take() {
                *e = x;
            }
            return;
        }
    }
    for c in e.children_mut() {
        if with.is_none() {
            return;
        }
        replace_var(c, v, with);
    }
}

// ---------------------------------------------------------------------
// arrays stored in a narrower type (D218)

/// `let v = map(.., (x) => f)` whose values all fit a narrower integer type
/// (`int_range` of `f`: the leaves of an `if`, `x & c`, a byte or a
/// boolean converted), every use of `v` an element `v[i]` or the length:
/// `v` is stored in that type, `f` converted to it, each `v[i]` converted
/// back. The same values for the program, fewer bytes written and read
/// (k-nucleotide: the codes 0 to 3 of 125 million bases, 125 MB of bytes
/// instead of 1 GB of `i64`).
fn narrow_arrays(e: &mut Expr, vars: &mut Vec<VarInfo>) {
    for c in e.children_mut() {
        narrow_arrays(c, vars);
    }
    let ExprKind::Let { var, init, body, .. } = &mut e.kind else { return };
    let v = *var;
    let Type::Array(et, fixed) = init.ty.clone() else { return };
    let Type::Int(k) = *et else { return };
    let ExprKind::Map(_, l) = &init.kind else { return };
    let Some((lo, hi)) = int_range(&l.body) else { return };
    let narrow = if lo >= 0 && hi <= 255 {
        IntKind::U8
    } else if lo >= 0 && hi <= 65535 {
        IntKind::U16
    } else if lo >= -128 && hi <= 127 {
        IntKind::I8
    } else {
        return;
    };
    if narrow.bits() >= k.bits() {
        return;
    }
    let (mut index, mut other) = (0, 0);
    uses_by_element(body, v, &mut index, &mut other);
    if other > 0 || index == 0 {
        return;
    }
    let nt = Type::Array(Box::new(Type::Int(narrow)), fixed);
    let ExprKind::Map(_, l) = &mut init.kind else { return };
    let lb = take(&mut l.body);
    let sp = lb.span;
    *l.body = Expr::new(ExprKind::Convert(Box::new(lb)), Type::Int(narrow), sp);
    init.ty = nt.clone();
    vars[v as usize].ty = nt.clone();
    widen_reads(body, v, &nt, &Type::Int(k));
}

/// The values an integer expression can take, when it is one of the simple
/// forms of `narrow_arrays`.
fn int_range(e: &Expr) -> Option<(i64, i64)> {
    match &e.kind {
        ExprKind::Lit(Lit::Int(c)) => Some((*c, *c)),
        ExprKind::If(_, t, f) => {
            let (a, b) = (int_range(t)?, int_range(f)?);
            Some((a.0.min(b.0), a.1.max(b.1)))
        }
        ExprKind::Let { body, .. } => int_range(body),
        ExprKind::Binary(BinOp::BitAnd, _, m) => match m.kind {
            ExprKind::Lit(Lit::Int(c)) if c >= 0 => Some((0, c)),
            _ => None,
        },
        ExprKind::Convert(x) => match x.ty {
            Type::Int(IntKind::U8) => Some((0, 255)),
            Type::Bool => Some((0, 1)),
            _ => None,
        },
        _ => None,
    }
}

// ---------------------------------------------------------------------
// the largest byte of a value (D450)

/// The largest byte each variable bound by a `let` can hold, when it is
/// known from its value: a string (its bytes), an array of `u8` (its
/// elements) or a `u8`. Only the variables with a bound are kept. A
/// variable bound by a `let` more than once (code copied) takes the largest
/// of its values, none when one is unknown; a parameter has none. The code
/// generator gives `stringOf` of bytes below `0x80` their block without
/// checking them as UTF-8 (the hexadecimal digits of `sha256.hex`).
pub fn byte_bounds(p: &Program) -> HashMap<VarId, u8> {
    fn params(e: &Expr) -> &[VarId] {
        match &e.kind {
            ExprKind::Map(_, l) | ExprKind::Reduce(_, l, _) | ExprKind::Fold(_, _, l) | ExprKind::Filter(_, l) | ExprKind::Tasks(_, l) | ExprKind::Scan(_, l, _) => &l.params,
            _ => &[],
        }
    }
    fn walk(e: &Expr, env: &mut HashMap<VarId, Option<u8>>) {
        for x in params(e) {
            env.insert(*x, None);
        }
        match &e.kind {
            ExprKind::Let { var, init, body, .. } => {
                walk(init, env);
                let b = byte_bound(init, env);
                let b = match env.get(var) {
                    Some(old) => match (*old, b) {
                        (Some(x), Some(y)) => Some(x.max(y)),
                        _ => None,
                    },
                    None => b,
                };
                env.insert(*var, b);
                walk(body, env);
            }
            _ => {
                for c in e.children() {
                    walk(c, env);
                }
            }
        }
    }
    let mut env = HashMap::new();
    for f in &p.fns {
        for x in &f.params {
            env.insert(*x, None);
        }
    }
    env.insert(p.args, None);
    for f in &p.fns {
        walk(&f.body, &mut env);
    }
    walk(&p.body, &mut env);
    env.into_iter().filter_map(|(v, b)| Some((v, b?))).collect()
}

/// The largest byte of the value of `e` (a string, an array of `u8` or a
/// `u8`) for the forms whose bytes are known: a literal, the bytes of a
/// string, an element, a slice or a copy, the branches of an `if`, the
/// values of a map, `&`, `>>` by a constant, `min`; `None` otherwise.
fn byte_bound(e: &Expr, env: &HashMap<VarId, Option<u8>>) -> Option<u8> {
    let u8t = Type::Int(IntKind::U8);
    let both = |a: &Expr, b: &Expr| -> (Option<u8>, Option<u8>) { (byte_bound(a, env), byte_bound(b, env)) };
    match &e.kind {
        ExprKind::StrLit(s) => Some(s.bytes().max().unwrap_or(0)),
        ExprKind::Lit(Lit::Int(c)) if e.ty == u8t => Some(*c as u8),
        ExprKind::Var(x, _) => env.get(x).copied().flatten(),
        ExprKind::StrOp(StrOp::Bytes, a) => byte_bound(&a[0], env),
        ExprKind::Index(a, _) if e.ty == u8t => byte_bound(a, env),
        ExprKind::Slice(a, _, _) | ExprKind::Copy(a) => byte_bound(a, env),
        ExprKind::Let { body, .. } => byte_bound(body, env),
        ExprKind::If(_, a, b) => Some(byte_bound(a, env)?.max(byte_bound(b, env)?)),
        ExprKind::Map(_, l) if matches!(&e.ty, Type::Array(t, _) if **t == u8t) => byte_bound(&l.body, env),
        ExprKind::ArrayLit(es) => es.iter().try_fold(0u8, |m, x| Some(m.max(byte_bound(x, env)?))),
        ExprKind::ConstArray(ls) if matches!(&e.ty, Type::Array(t, _) if **t == u8t) => ls.iter().try_fold(0u8, |m, l| match l {
            Lit::Int(c) => Some(m.max(*c as u8)),
            _ => None,
        }),
        ExprKind::Binary(BinOp::BitAnd | BinOp::Min, a, b) if e.ty == u8t => match both(a, b) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (Some(x), None) | (None, Some(x)) => Some(x),
            _ => None,
        },
        ExprKind::Binary(BinOp::Shr, a, k) if e.ty == u8t => match k.kind {
            ExprKind::Lit(Lit::Int(c)) => Some(byte_bound(a, env).unwrap_or(255) >> (c & 7)),
            _ => None,
        },
        _ => None,
    }
}

/// `narrow_arrays`: each `v[i]` read as the narrow type and converted back
/// to `wide`; `v` of type `nt` where it is named.
fn widen_reads(e: &mut Expr, v: VarId, nt: &Type, wide: &Type) {
    match &mut e.kind {
        ExprKind::Index(a, i) if matches!(a.kind, ExprKind::Var(w, _) if w == v) => {
            widen_reads(i, v, nt, wide);
            a.ty = nt.clone();
            let mut read = take(e);
            read.ty = nt.elem().clone();
            let sp = read.span;
            *e = Expr::new(ExprKind::Convert(Box::new(read)), wide.clone(), sp);
        }
        ExprKind::Len(a) if matches!(a.kind, ExprKind::Var(w, _) if w == v) => {
            a.ty = nt.clone();
        }
        _ => {
            for c in e.children_mut() {
                widen_reads(c, v, nt, wide);
            }
        }
    }
}

/// The uses of `v` in `e`: as `v[i]` or `v.length` (`index`), and the
/// others.
fn uses_by_element(e: &Expr, v: VarId, index: &mut u32, other: &mut u32) {
    match &e.kind {
        ExprKind::Index(a, i) if matches!(a.kind, ExprKind::Var(w, _) if w == v) => {
            *index += 1;
            uses_by_element(i, v, index, other);
        }
        ExprKind::Len(a) if matches!(a.kind, ExprKind::Var(w, _) if w == v) => *index += 1,
        ExprKind::Var(w, _) if *w == v => *other += 1,
        _ => {
            for c in e.children() {
                uses_by_element(c, v, index, other);
            }
        }
    }
}

/// The passes before the ownership pass, on one body (`main` or a recursive
/// function).
pub(crate) fn optimize_body(body: &mut Expr, vars: &mut Vec<VarInfo>) {
    copy_prop(body, &mut HashMap::new());
    producer_lengths(body);
    constant_lengths(body, &mut HashMap::new());
    table_functions(body, vars, &mut HashMap::new());
    inline_zips(body, vars);
    fuse_fixpoint(body);
    // a `map` of a zip of scalars that reads its pairs by their components:
    // one `map` over the inputs of the zip (D495)
    compose_zip_maps(body);
    // (after the fusion: the size checks around an `iota` floated out)
    iota_vars(body, vars);
    bind_temporaries(body, vars);
    fuse_fixpoint(body);
    // (the source of a map a variable, the `let`s floated out of the value
    // by the fusion just before)
    narrow_arrays(body, vars);
    horizontal(body, vars);
    masked_reductions(body, vars);
    // a value read by several `reduce`s now grouped (F3) is read once: it
    // can be fused into the group
    fuse_fixpoint(body);
    // the components of a new tuple read only by projections bound, so
    // that the ownership pass takes it apart (D434)
    bind_projections(body, vars);
    let pending = place_streams(body, vars, Mode::Evaluated);
    debug_assert!(pending.is_empty());
}

// ---------------------------------------------------------------------
// the components of a new tuple read only by projections (D434)

/// Each `proj k r` of `e` replaced by the variable of its component.
fn read_components(e: &mut Expr, r: VarId, comps: &[(u32, VarId)]) {
    if let ExprKind::Proj(x, k) = &e.kind {
        if matches!(x.kind, ExprKind::Var(w, _) if w == r) {
            if let Some((_, v)) = comps.iter().find(|(j, _)| j == k) {
                e.kind = ExprKind::Var(*v, Use::Borrow);
                return;
            }
        }
    }
    for c in e.children_mut() {
        read_components(c, r, comps);
    }
}

/// `let r = <new tuple> in proj k r` (`new_tuple`), the component `k`
/// owning memory: the value of the `let` is a component of a tuple that
/// dies with it (the body of a response read out of `textResponse(...)`),
/// which the ownership pass copied. Each component that owns memory bound
/// (`let c = proj j r`), the value read through its variable: `split_tuples`
/// then takes the tuple apart (D138), the component moved out without a
/// copy, the others freed on their own as values never read. A tuple that
/// lives on, read by projections, is left as it is: its projections borrow
/// (D434).
fn bind_projections(e: &mut Expr, vars: &mut Vec<VarInfo>) {
    for c in e.children_mut() {
        bind_projections(c, vars);
    }
    let ExprKind::Let { var, init, body, .. } = &mut e.kind else { return };
    let r = *var;
    let Type::Tuple(ts) = &init.ty else { return };
    let ExprKind::Proj(x, k) = &body.kind else { return };
    if !matches!(x.kind, ExprKind::Var(w, _) if w == r) || !ts[*k as usize].has_heap() || !new_tuple(init) {
        return;
    }
    let heap: Vec<(u32, Type)> = ts.iter().enumerate().filter(|(_, t)| t.has_heap()).map(|(k, t)| (k as u32, t.clone())).collect();
    let mut comps = Vec::new();
    for (k, t) in &heap {
        let v = vars.len() as VarId;
        vars.push(VarInfo { name: format!("{}.{}", vars[r as usize].name, k), ty: t.clone() });
        comps.push((*k, v, t.clone()));
    }
    let pairs: Vec<(u32, VarId)> = comps.iter().map(|(k, v, _)| (*k, *v)).collect();
    read_components(body, r, &pairs);
    let span = body.span;
    let rty = init.ty.clone();
    for (k, v, t) in comps.into_iter().rev() {
        let inner = std::mem::replace(&mut **body, Expr::new(ExprKind::Lit(Lit::Unit), Type::Unit, span));
        let ty = inner.ty.clone();
        let proj = Expr::new(ExprKind::Proj(Box::new(Expr::var(r, rty.clone(), span)), k), t, span);
        **body = Expr::new(ExprKind::Let { var: v, init: Box::new(proj), body: Box::new(inner), stmt: false, drop: false }, ty, span);
    }
}

// ---------------------------------------------------------------------
// ranges never made (D244)

/// Every use of `v` in `e` reads it in place: an input of a loop (`map`,
/// `reduce`, `fold`, `scan`, `filter`), its length, one of its elements,
/// the source of a slice (`fusable_slots`), inside lambdas too.
fn read_in_place(e: &Expr, v: VarId) -> bool {
    let slots = fusable_slots(e);
    e.children().into_iter().enumerate().all(|(i, c)| match c.kind {
        ExprKind::Var(w, _) if w == v => slots.contains(&i),
        _ => read_in_place(c, v),
    })
}

/// Each use of `v` replaced by `iota(len)`.
fn iota_of_var(e: &mut Expr, v: VarId, len: VarId, ty: &Type) {
    if let ExprKind::Var(w, _) = e.kind {
        if w == v {
            let n = Expr { kind: ExprKind::Var(len, Use::Borrow), ty: Type::Int(IntKind::I64), span: e.span, reassoc: e.reassoc };
            e.kind = ExprKind::Iota(Box::new(n));
            e.ty = ty.clone();
            return;
        }
    }
    for c in e.children_mut() {
        iota_of_var(c, v, len, ty);
    }
}

/// `let v = iota(n) in body`, every use of `v` reading it in place
/// (`read_in_place`): `let len = n in body` with each use `iota(len)`, a
/// producer computed where it is read, never an array of n indices (D244:
/// `countBy(iota(n), f)` reads its input twice, a `map` of it and its
/// elements at the first index of each group; made whole, 8n bytes and the
/// index of a `map` read from memory, a gather in vector code, instead of
/// the index of the loop).
fn iota_vars(e: &mut Expr, vars: &mut Vec<VarInfo>) {
    for c in e.children_mut() {
        iota_vars(c, vars);
    }
    if let ExprKind::Let { var, init, body, stmt: false, .. } = &mut e.kind {
        if let ExprKind::Iota(n) = &mut init.kind {
            if read_in_place(body, *var) {
                let len = vars.len() as VarId;
                vars.push(VarInfo { name: "length".into(), ty: Type::Int(IntKind::I64) });
                let ty = init.ty.clone();
                iota_of_var(body, *var, len, &ty);
                let n = take(n);
                **init = n;
                *var = len;
            }
        }
    }
}

// ---------------------------------------------------------------------
// `requireTail` (SPEC section 9, D106)

/// The calls of recursive functions in `e`, with whether each one is in tail
/// position of the body (through `let` bodies, `if` branches, size tests,
/// `Free`).
fn recursive_calls<'e>(e: &'e Expr, tail: bool, out: &mut Vec<(&'e Expr, bool)>) {
    if let ExprKind::Call(..) = e.kind {
        out.push((e, tail));
    }
    // the initial value of a `let`, the condition of an `if`, the sizes of a
    // test are not in tail position; their bodies and branches are
    let tails: Vec<bool> = match &e.kind {
        ExprKind::Let { .. } => vec![false, tail],
        ExprKind::If(..) | ExprKind::CheckSize(..) => vec![false, tail, tail],
        ExprKind::CheckNonNegative(..) => vec![false, tail],
        ExprKind::Free(..) => vec![tail],
        _ => vec![false; e.children().len()],
    };
    for (c, t) in e.children().into_iter().zip(tails) {
        recursive_calls(c, t, out);
    }
}

/// The functions `fi` reaches through calls, itself included if it is in a
/// cycle.
fn reachable(p: &Program, fi: u32) -> HashSet<u32> {
    let mut seen = HashSet::new();
    let mut todo = vec![fi];
    while let Some(f) = todo.pop() {
        let mut calls = Vec::new();
        recursive_calls(&p.fns[f as usize].body, true, &mut calls);
        for (c, _) in calls {
            if let ExprKind::Call(g, _) = c.kind {
                if seen.insert(g) {
                    todo.push(g);
                }
            }
        }
    }
    seen
}

/// `requireTail`: every recursive call of the function is a direct tail
/// call compiled as a loop.
fn check_require_tail(p: &Program) -> R<()> {
    for (i, f) in p.fns.iter().enumerate() {
        let Some(at) = f.require_tail else { continue };
        let loops = tail_calls(i as u32, f);
        let others = other_tail_calls(i as u32, f);
        let mut calls = Vec::new();
        recursive_calls(&f.body, true, &mut calls);
        let mut notes = Vec::new();
        for (c, tail) in calls {
            let ExprKind::Call(g, args) = &c.kind else { continue };
            let gname = &p.fns[*g as usize].name;
            if *g == i as u32 {
                if loops.contains_key(&(c as *const Expr)) {
                    continue;
                }
                if !tail {
                    notes.push(format!("the call of `{}` at line {} is not in tail position: something is computed after it", gname, c.span.line));
                } else {
                    let arr = args.iter().find(|a| a.ty.has_heap()).map(|a| a.span.line).unwrap_or(c.span.line);
                    notes.push(format!(
                        "the call of `{}` at line {} passes an array (line {}) that is neither the same parameter nor a new array",
                        gname, c.span.line, arr
                    ));
                }
            } else if reachable(p, *g).contains(&(i as u32)) && !others.contains_key(&(c as *const Expr)) {
                // a mutual tail call reuses the frame (D113); another call
                // in the cycle does not
                let why = if tail { "passes an array that is not a parameter of the caller" } else { "is not in tail position" };
                notes.push(format!("the call of `{}` at line {} (a mutual recursion) {}", gname, c.span.line, why));
            }
        }
        if !notes.is_empty() {
            let mut d = Diagnostic::new(at, format!("requireTail not satisfied (line {})", f.span.line));
            d.notes = notes;
            return Err(d);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// direct tail calls (SPEC section 11, D102)

/// A direct tail call: the parameters that receive a temporary (moved into
/// the loop), those that receive a value the loop keeps without owning it
/// (`lent`: a string literal, D301, or a part of a parameter the loop never
/// owns, D561), the temporaries freed before the jump.
pub struct TailCall {
    pub moved: Vec<(usize, VarId)>,
    pub lent: Vec<usize>,
    pub free_first: Vec<VarId>,
}

/// The direct tail calls of function `fi` that can be jumps: each array
/// argument is the same parameter, a temporary freed after the call (then
/// moved into the loop), a variable bound to a string literal (borrowed
/// data of the program, D301: the loop keeps it without owning it), or a
/// variable bound to a part of a borrowed parameter (the tail of a list, a
/// row, a component: D561), when that parameter's place never receives a
/// value the loop owns (moved there by a tail call: freed at a later jump,
/// it would free the part); the other temporaries freed after the call are
/// freed before the jump.
pub fn tail_calls(fi: u32, f: &Function) -> HashMap<*const Expr, TailCall> {
    struct Walk<'a> {
        fi: u32,
        f: &'a Function,
        pending: Vec<VarId>,
        literals: Vec<VarId>,
        /// The variables bound to a part of borrowed parameters: the places
        /// of those parameters.
        parts: Vec<(VarId, Vec<usize>)>,
        out: HashMap<*const Expr, TailCall>,
        /// For each call, the places of the parameters its lent parts come
        /// from.
        roots: Vec<(*const Expr, Vec<usize>)>,
    }
    impl Walk<'_> {
        /// The places of the borrowed parameters `e` is a part of (a
        /// projection, the payload of a node, an element, a view, through
        /// `let`s of such parts), or None.
        fn part_of(&self, e: &Expr, inner: &mut Vec<(VarId, Vec<usize>)>) -> Option<Vec<usize>> {
            match &e.kind {
                ExprKind::Var(v, _) => {
                    if let Some(k) = self.f.params.iter().position(|p| p == v) {
                        let consumed = self.f.consumed.get(k).copied().unwrap_or(false);
                        return (!consumed).then(|| vec![k]);
                    }
                    inner.iter().rev().chain(self.parts.iter().rev()).find(|(w, _)| w == v).map(|(_, r)| r.clone())
                }
                ExprKind::Proj(x, _) | ExprKind::Unbox(x) | ExprKind::Index(x, _) | ExprKind::Window(x, _) => self.part_of(x, inner),
                ExprKind::Slice(x, _, _) if e.ty.slice_is_view() => self.part_of(x, inner),
                ExprKind::CheckSize(_, _, b) | ExprKind::CheckNonNegative(_, b) => self.part_of(b, inner),
                ExprKind::Let { var, init, body, drop: false, .. } => {
                    let r = self.part_of(init, inner)?;
                    inner.push((*var, r));
                    let out = self.part_of(body, inner);
                    inner.pop();
                    out
                }
                _ => None,
            }
        }

        fn walk(&mut self, e: &Expr) {
            match &e.kind {
                ExprKind::Let { var, init, body, drop, .. } => {
                    if *drop {
                        self.pending.push(*var);
                    }
                    let literal = matches!(init.kind, ExprKind::StrLit(_)) && init.ty == Type::Str && !*drop;
                    if literal {
                        self.literals.push(*var);
                    }
                    let part = if !*drop && init.ty.has_heap() { self.part_of(init, &mut Vec::new()) } else { None };
                    let has_part = part.is_some();
                    if let Some(r) = part {
                        self.parts.push((*var, r));
                    }
                    self.walk(body);
                    if has_part {
                        self.parts.pop();
                    }
                    if literal {
                        self.literals.pop();
                    }
                    if *drop {
                        self.pending.pop();
                    }
                }
                ExprKind::If(_, t, x) => {
                    self.walk(t);
                    self.walk(x);
                }
                ExprKind::CheckSize(_, _, b) | ExprKind::CheckNonNegative(_, b) => self.walk(b),
                ExprKind::Free(v, b) => {
                    self.pending.push(*v);
                    self.walk(b);
                    self.pending.pop();
                }
                ExprKind::Call(g, args) if *g == self.fi => {
                    let mut moved = Vec::new();
                    let mut lent = Vec::new();
                    let mut roots = Vec::new();
                    // an owned parameter given again at its place (D351): it
                    // stays in its slot, the next iteration owns it, not freed
                    let mut kept = Vec::new();
                    for (k, a) in args.iter().enumerate() {
                        if !a.ty.has_heap() {
                            continue;
                        }
                        match a.kind {
                            ExprKind::Var(v, _) if v == self.f.params[k] => {
                                if self.f.consumed.get(k).copied().unwrap_or(false) {
                                    kept.push(v);
                                }
                            }
                            ExprKind::Var(v, _) if self.pending.contains(&v) && !moved.iter().any(|(_, w)| *w == v) => moved.push((k, v)),
                            ExprKind::Var(v, _) if self.literals.contains(&v) => lent.push(k),
                            ExprKind::Var(v, _) if self.parts.iter().any(|(w, _)| *w == v) => {
                                let (_, r) = self.parts.iter().rev().find(|(w, _)| *w == v).unwrap();
                                roots.extend(r.iter().copied());
                                lent.push(k);
                            }
                            _ => return,
                        }
                    }
                    let free_first = self.pending.iter().copied().filter(|v| !moved.iter().any(|(_, w)| w == v) && !kept.contains(v)).collect();
                    self.roots.push((e as *const Expr, roots));
                    self.out.insert(e as *const Expr, TailCall { moved, lent, free_first });
                }
                _ => {}
            }
        }
    }
    let mut w = Walk { fi, f, pending: Vec::new(), literals: Vec::new(), parts: Vec::new(), out: HashMap::new(), roots: Vec::new() };
    w.walk(&f.body);
    // the places that receive a value the loop owns: a part of the value
    // there would be freed with it at a later jump; the calls that lend such
    // parts stay calls
    let owned: HashSet<usize> = w.out.values().flat_map(|c| c.moved.iter().map(|(k, _)| *k)).collect();
    for (call, roots) in &w.roots {
        if roots.iter().any(|k| owned.contains(k)) {
            w.out.remove(call);
        }
    }
    w.out
}

/// The tail calls of function `fi` to another function that can reuse its
/// frame (section 16, v1: mutual tail recursion, D113): each array
/// argument is a parameter of `fi` (borrowed from its caller, alive after
/// `fi` is gone); the temporaries freed after the call are freed before
/// it, and none is passed. Value: the temporaries to free first.
pub fn other_tail_calls(fi: u32, f: &Function) -> HashMap<*const Expr, Vec<VarId>> {
    fn walk(e: &Expr, fi: u32, f: &Function, pending: &mut Vec<VarId>, out: &mut HashMap<*const Expr, Vec<VarId>>) {
        match &e.kind {
            ExprKind::Let { var, body, drop, .. } => {
                if *drop {
                    pending.push(*var);
                }
                walk(body, fi, f, pending, out);
                if *drop {
                    pending.pop();
                }
            }
            ExprKind::If(_, t, x) => {
                walk(t, fi, f, pending, out);
                walk(x, fi, f, pending, out);
            }
            ExprKind::CheckSize(_, _, b) | ExprKind::CheckNonNegative(_, b) => walk(b, fi, f, pending, out),
            ExprKind::Free(v, b) => {
                pending.push(*v);
                walk(b, fi, f, pending, out);
                pending.pop();
            }
            ExprKind::Call(g, args) if *g != fi => {
                let ok = args.iter().all(|a| !a.ty.has_heap() || matches!(a.kind, ExprKind::Var(v, _) if f.params.contains(&v)));
                if ok {
                    out.insert(e as *const Expr, pending.clone());
                }
            }
            _ => {}
        }
    }
    let mut out = HashMap::new();
    walk(&f.body, fi, f, &mut Vec::new(), &mut out);
    out
}

// ---------------------------------------------------------------------
// horizontal fusion (rule F3, D100)

/// The array a `reduce` reads through `map`s of one input: its variable and
/// the `map`s, innermost first.
fn reduce_base(x: &Expr) -> Option<(VarId, Vec<&Lambda>)> {
    match &x.kind {
        ExprKind::Var(v, _) => Some((*v, Vec::new())),
        ExprKind::Map(ins, m) if ins.len() == 1 => {
            let (v, mut ms) = reduce_base(&ins[0])?;
            ms.push(m);
            Some((v, ms))
        }
        _ => None,
    }
}

/// Variables used in `e` and not bound inside it.
fn free_in(e: &Expr, bound: &mut Vec<VarId>, out: &mut HashSet<VarId>) {
    match &e.kind {
        ExprKind::Var(v, _) => {
            if !bound.contains(v) {
                out.insert(*v);
            }
            return;
        }
        ExprKind::Let { var, init, body, .. } => {
            free_in(init, bound, out);
            bound.push(*var);
            free_in(body, bound, out);
            bound.pop();
            return;
        }
        _ => {}
    }
    let lam = lambda_body_index(e);
    let params: Vec<VarId> = match &e.kind {
        ExprKind::Map(_, l) | ExprKind::Reduce(_, l, _) | ExprKind::Fold(_, _, l) | ExprKind::Scan(_, l, _) | ExprKind::Filter(_, l) | ExprKind::Tasks(_, l) => {
            l.params.clone()
        }
        _ => Vec::new(),
    };
    for (i, c) in e.children().into_iter().enumerate() {
        if Some(i) == lam {
            let n = bound.len();
            bound.extend(&params);
            free_in(c, bound, out);
            bound.truncate(n);
        } else {
            free_in(c, bound, out);
        }
    }
}

/// Variables bound inside `e` (by `let`s and lambdas).
fn bound_in(e: &Expr, out: &mut HashSet<VarId>) {
    match &e.kind {
        ExprKind::Let { var, .. } => {
            out.insert(*var);
        }
        ExprKind::Map(_, l) | ExprKind::Reduce(_, l, _) | ExprKind::Fold(_, _, l) | ExprKind::Scan(_, l, _) | ExprKind::Filter(_, l) | ExprKind::Tasks(_, l) => {
            out.extend(&l.params);
        }
        _ => {}
    }
    for c in e.children() {
        bound_in(c, out);
    }
}

struct Site {
    elem: usize,
    path: Vec<usize>,
    base: VarId,
    free: HashSet<VarId>,
}

/// The `reduce`s evaluated whenever `e` is: not inside a lambda, a branch,
/// the rest of an action after a statement, or another `reduce`.
fn find_sites(e: &Expr, elem: usize, path: &mut Vec<usize>, out: &mut Vec<Site>) {
    if let ExprKind::Reduce(x, _, _) = &e.kind {
        if e.ty.is_scalar() && !is_stream(x) {
            if let Some((base, _)) = reduce_base(x) {
                let mut free = HashSet::new();
                free_in(e, &mut Vec::new(), &mut free);
                out.push(Site { elem, path: path.clone(), base, free });
                return;
            }
        }
    }
    let lam = lambda_body_index(e);
    for (i, c) in e.children().into_iter().enumerate() {
        let skip = Some(i) == lam
            || (matches!(e.kind, ExprKind::If(..)) && i > 0)
            || (matches!(e.kind, ExprKind::Let { stmt: true, .. }) && i == 1);
        if skip {
            continue;
        }
        path.push(i);
        find_sites(c, elem, path, out);
        path.pop();
    }
}

fn at_path<'a>(e: &'a mut Expr, path: &[usize]) -> &'a mut Expr {
    match path.split_first() {
        None => e,
        Some((i, rest)) => at_path(e.children_mut().into_iter().nth(*i).unwrap(), rest),
    }
}

/// The node at position `s` of the spine of non-statement `let`s that starts
/// at `e`.
fn spine_node(e: &mut Expr, s: usize) -> &mut Expr {
    if s == 0 {
        return e;
    }
    match &mut e.kind {
        ExprKind::Let { body, .. } => spine_node(body, s - 1),
        _ => unreachable!(),
    }
}

/// Rule F3: independent `reduce`s of one array, evaluated together (in one
/// spine of `let`s of a pure computation, or in one expression), become one
/// `reduce` of tuples whose operator works component by component: each
/// component gets its elements in the lanes and blocks of the form of
/// section 10, so each result is the one of its own `reduce`, and the code
/// generator reads the array once (`reduce_columns`). A `reduce` that uses a
/// variable defined at or after the place of the first one is left alone.
fn horizontal(e: &mut Expr, vars: &mut Vec<VarInfo>) {
    group_spine(e, vars);
    for c in e.children_mut() {
        horizontal(c, vars);
    }
}

fn group_spine(e: &mut Expr, vars: &mut Vec<VarInfo>) {
    // the elements of the spine: the initial values, then the last body
    let mut elems: Vec<&Expr> = Vec::new();
    let mut spine_vars: Vec<VarId> = Vec::new();
    let mut cur: &Expr = e;
    while let ExprKind::Let { var, init, body, stmt: false, .. } = &cur.kind {
        elems.push(init);
        spine_vars.push(*var);
        cur = body;
    }
    elems.push(cur);
    let mut sites = Vec::new();
    for (s, x) in elems.iter().enumerate() {
        find_sites(x, s, &mut Vec::new(), &mut sites);
    }
    if sites.len() < 2 {
        return;
    }
    // what is bound from element s on: the spine variables of the `let`s at
    // s and after, and what the elements bind inside
    let n = elems.len();
    let mut bound_from: Vec<HashSet<VarId>> = vec![HashSet::new(); n + 1];
    for s in (0..n).rev() {
        let mut b = bound_from[s + 1].clone();
        bound_in(elems[s], &mut b);
        if s < spine_vars.len() {
            b.insert(spine_vars[s]);
        }
        bound_from[s] = b;
    }
    let mut groups: Vec<(VarId, Vec<usize>)> = Vec::new();
    for (i, st) in sites.iter().enumerate() {
        match groups.iter_mut().find(|(b, _)| *b == st.base) {
            Some((_, g)) => g.push(i),
            None => groups.push((st.base, vec![i])),
        }
    }
    for (base, members) in groups {
        if members.len() < 2 {
            continue;
        }
        let s0 = members.iter().map(|&i| sites[i].elem).min().unwrap();
        let ok: Vec<usize> = members.into_iter().filter(|&i| sites[i].free.is_disjoint(&bound_from[s0])).collect();
        if ok.len() < 2 {
            continue;
        }
        let s0 = ok.iter().map(|&i| sites[i].elem).min().unwrap();
        combine(e, vars, base, &ok.iter().map(|&i| (&sites[i].elem, &sites[i].path)).map(|(a, b)| (*a, b.clone())).collect::<Vec<_>>(), s0, n);
        return; // the paths changed: the rest at the next visit
    }
}

fn combine(e: &mut Expr, vars: &mut Vec<VarInfo>, base: VarId, sites: &[(usize, Vec<usize>)], s0: usize, n: usize) {
    let sp = e.span;
    let base_ty = vars[base as usize].ty.clone();
    let et = base_ty.elem().clone();
    // the tuple variable and its components
    let mut reduces: Vec<Expr> = Vec::new();
    let mut tys: Vec<Type> = Vec::new();
    let rr_info = VarInfo { name: "reduced".into(), ty: Type::Unit };
    vars.push(rr_info);
    let rr = (vars.len() - 1) as VarId;
    for (k, (s, path)) in sites.iter().enumerate() {
        let elem: &mut Expr = if *s + 1 < n {
            match &mut spine_node(e, *s).kind {
                ExprKind::Let { init, .. } => init,
                _ => unreachable!(),
            }
        } else {
            spine_node(e, *s)
        };
        let node = at_path(elem, path);
        let ty = node.ty.clone();
        let proj = Expr::new(ExprKind::Proj(Box::new(Expr::var(rr, Type::Unit, node.span)), k as u32), ty.clone(), node.span);
        reduces.push(std::mem::replace(node, proj));
        tys.push(ty);
    }
    let tt = Type::Tuple(tys.clone());
    vars[rr as usize].ty = tt.clone();
    fix_rr_type(e, rr, &tt);
    // map: (v) -> (f_1(v), ..., f_k(v)); operator component by component
    vars.push(VarInfo { name: "v".into(), ty: et.clone() });
    let v = (vars.len() - 1) as VarId;
    vars.push(VarInfo { name: "a".into(), ty: tt.clone() });
    let a = (vars.len() - 1) as VarId;
    vars.push(VarInfo { name: "b".into(), ty: tt.clone() });
    let b = (vars.len() - 1) as VarId;
    let bind = |var: VarId, init: Expr, body: Expr| {
        let ty = body.ty.clone();
        Expr::new(ExprKind::Let { var, init: Box::new(init), body: Box::new(body), stmt: false, drop: false }, ty, sp)
    };
    let mut cols = Vec::new();
    let mut ops = Vec::new();
    let mut zs = Vec::new();
    for (k, r) in reduces.into_iter().enumerate() {
        let ExprKind::Reduce(x, l, z) = r.kind else { unreachable!() };
        // the maps of x, innermost first, applied to v
        let mut maps = Vec::new();
        let mut cur = *x;
        while let ExprKind::Map(mut ins, m) = cur.kind {
            maps.push(m);
            cur = ins.pop().unwrap();
        }
        let mut col = Expr::var(v, et.clone(), sp);
        for m in maps.into_iter().rev() {
            col = bind(m.params[0], col, *m.body);
        }
        cols.push(col);
        let ty = tys[k].clone();
        let pa = Expr::new(ExprKind::Proj(Box::new(Expr::var(a, tt.clone(), sp)), k as u32), ty.clone(), sp);
        let pb = Expr::new(ExprKind::Proj(Box::new(Expr::var(b, tt.clone(), sp)), k as u32), ty, sp);
        ops.push(bind(l.params[0], pa, bind(l.params[1], pb, *l.body)));
        zs.push(*z);
    }
    let map_body = Expr::new(ExprKind::Tuple(cols), tt.clone(), sp);
    let src = Expr::new(
        ExprKind::Map(vec![Expr::var(base, base_ty, sp)], Lambda { params: vec![v], body: Box::new(map_body), drop_params: vec![false] }),
        Type::Array(Box::new(tt.clone()), crate::Fixed::ANY),
        sp,
    );
    let op = Lambda { params: vec![a, b], body: Box::new(Expr::new(ExprKind::Tuple(ops), tt.clone(), sp)), drop_params: vec![false, false] };
    let z = Expr::new(ExprKind::Tuple(zs), tt.clone(), sp);
    let red = Expr::new(ExprKind::Reduce(Box::new(src), op, Box::new(z)), tt, sp);
    let at = spine_node(e, s0);
    let inner = take(at);
    let ty = inner.ty.clone();
    *at = Expr::new(ExprKind::Let { var: rr, init: Box::new(red), body: Box::new(inner), stmt: false, drop: false }, ty, sp);
}

/// The projections of the combined result were built before its type was
/// known.
fn fix_rr_type(e: &mut Expr, rr: VarId, tt: &Type) {
    if let ExprKind::Var(v, _) = e.kind {
        if v == rr {
            e.ty = tt.clone();
        }
    }
    for c in e.children_mut() {
        fix_rr_type(c, rr, tt);
    }
}

// ---------------------------------------------------------------------
// streams (rule F2, D99)

/// Evaluating `e` cannot panic (SPEC section 5: an index out of bounds, an
/// integer division by zero, a negative integer exponent, sizes that differ,
/// an action).
pub fn cannot_panic(e: &Expr) -> bool {
    use ExprKind::*;
    match &e.kind {
        Index(..) | Update { .. } | Slice(..) | Window(..) | CheckSize(..) | CheckNonNegative(..) | Action(..) | Bench(..) => false,
        Binary(BinOp::Div | BinOp::Rem, a, b) if matches!(a.ty, Type::Int(_)) => {
            matches!(b.kind, Lit(crate::Lit::Int(d)) if d != 0) && cannot_panic(a)
        }
        Pow(b, k) => {
            let k_ok = matches!(k.kind, Lit(crate::Lit::Int(n)) if n >= 0) || !matches!(k.ty, Type::Int(ik) if ik.signed());
            k_ok && cannot_panic(b) && cannot_panic(k)
        }
        _ => e.children().into_iter().all(cannot_panic),
    }
}

/// The operator of a `reduce` lambda `(a, b) -> a op b` when its result
/// does not depend on the order of the operands nor on their grouping,
/// exactly: integer `+`, `*` (modulo 2^bits), `min`, `max` of integers and
/// floats (IEEE 754-2019 `minimum`, `maximum`).
fn exact_op(l: &Lambda) -> Option<BinOp> {
    let ExprKind::Binary(op, a, b) = &l.body.kind else { return None };
    let var = |e: &Expr| match e.kind {
        ExprKind::Var(v, _) => Some(v),
        _ => None,
    };
    let (x, y) = (var(a)?, var(b)?);
    let ps = (l.params[0], l.params[1]);
    if (x, y) != ps && (y, x) != ps {
        return None;
    }
    match (op, &l.body.ty) {
        (BinOp::Add | BinOp::Mul | BinOp::Min | BinOp::Max, Type::Int(_)) => Some(*op),
        (BinOp::Min | BinOp::Max, Type::F64 | Type::F32) => Some(*op),
        _ => None,
    }
}

/// `+` or `*` of floats with the two parameters (in any order).
fn float_op(l: &Lambda) -> Option<BinOp> {
    let ExprKind::Binary(op, a, b) = &l.body.kind else { return None };
    let var = |e: &Expr| match e.kind {
        ExprKind::Var(v, _) => Some(v),
        _ => None,
    };
    let (x, y) = (var(a)?, var(b)?);
    let ps = (l.params[0], l.params[1]);
    if (x, y) != ps && (y, x) != ps {
        return None;
    }
    match (op, &l.body.ty) {
        (BinOp::Add | BinOp::Mul, Type::F64 | Type::F32) => Some(*op),
        _ => None,
    }
}

/// `z` is 0.0 (either sign) for `+`, 1.0 for `*`: the identity of a float
/// operator when the sign of a zero is ignored.
fn is_float_identity(op: BinOp, z: &Expr) -> bool {
    let v = match &z.kind {
        ExprKind::Lit(crate::Lit::F64(v)) => *v,
        ExprKind::Lit(crate::Lit::F32(v)) => *v as f64,
        _ => return false,
    };
    (op == BinOp::Add && v == 0.0) || (op == BinOp::Mul && v == 1.0)
}

/// The value of `reduce(x, l, z)` does not depend on the order of the
/// elements nor on how many times the form of section 10 starts from `z`
/// (D183): an exact operator, and `z` its identity or the operator
/// idempotent (`min`, `max`: `z` repeated changes nothing).
pub fn order_free(l: &Lambda, z: &Expr) -> bool {
    match exact_op(l) {
        Some(BinOp::Min | BinOp::Max) => true,
        Some(op) => is_identity(op, z),
        None => false,
    }
}

/// `z` is exactly the identity of `op`: op(z, v) = v for every v.
fn is_identity(op: BinOp, z: &Expr) -> bool {
    match (&z.kind, &z.ty) {
        (ExprKind::Lit(crate::Lit::Int(v)), Type::Int(k)) => match op {
            BinOp::Add => *v == 0,
            BinOp::Mul => *v == 1,
            BinOp::Min => *v == canon(*k, k.max_value()),
            BinOp::Max => *v == canon(*k, k.min_value()),
            _ => false,
        },
        (ExprKind::Lit(crate::Lit::F64(v)), _) => {
            (op == BinOp::Min && *v == f64::INFINITY) || (op == BinOp::Max && *v == f64::NEG_INFINITY)
        }
        (ExprKind::Lit(crate::Lit::F32(v)), _) => {
            (op == BinOp::Min && *v == f32::INFINITY) || (op == BinOp::Max && *v == f32::NEG_INFINITY)
        }
        _ => false,
    }
}

/// `reduce(filter(x, p) [then maps f], op, z)` with an exact operator (see
/// `exact_op`) and its exact identity `z` becomes `reduce(map(x, (v) -> if
/// p(v) then f(v) else z), op, z)`: the result of the form of section 10 on
/// the kept elements equals the combination of all of them in any order,
/// and the rejected elements add `z`, which changes nothing. The predicate
/// and the maps are then computed for every element, so only when they
/// cannot panic. The `reduce` is then vectorized and parallel, without the
/// lane counter of the stream (D99).
fn masked_reductions(e: &mut Expr, vars: &mut Vec<VarInfo>) {
    for c in e.children_mut() {
        masked_reductions(c, vars);
    }
    // under `allowReassoc` (D109), `+` and `*` of floats too: the form of
    // the reduction is free and the sign of a zero ignored (`nsz`), so a
    // rejected element may count as 0.0 (or 1.0) (D123)
    let reassoc = e.reassoc;
    let ExprKind::Reduce(x, l, z) = &mut e.kind else { return };
    if !is_stream(x) {
        return;
    }
    let Some(op) = exact_op(l).or_else(|| if reassoc { float_op(l) } else { None }) else { return };
    if !is_identity(op, z) && !(reassoc && is_float_identity(op, z)) {
        return;
    }
    // the maps after the filter (innermost first), the filter
    let mut posts = Vec::new();
    let mut cur: &Expr = x;
    while let ExprKind::Map(ins, m) = &cur.kind {
        posts.push(m.clone());
        cur = &ins[0];
    }
    let ExprKind::Filter(src, pred) = &cur.kind else { return };
    if !cannot_panic(&pred.body) || !posts.iter().all(|m| cannot_panic(&m.body)) {
        return;
    }
    posts.reverse();
    let src = (**src).clone();
    let et = src.ty.elem().clone();
    let sp = x.span;
    vars.push(VarInfo { name: vars[pred.params[0] as usize].name.clone(), ty: et.clone() });
    let p = (vars.len() - 1) as VarId;
    let bind = |var: VarId, init: Expr, body: Expr| {
        let ty = body.ty.clone();
        Expr::new(ExprKind::Let { var, init: Box::new(init), body: Box::new(body), stmt: false, drop: false }, ty, sp)
    };
    // then: the maps applied in order to the element
    let mut kept = Expr::var(p, et.clone(), sp);
    for m in &posts {
        kept = bind(m.params[0], kept, (*m.body).clone());
    }
    let cond = bind(pred.params[0], Expr::var(p, et, sp), (*pred.body).clone());
    let rt = l.body.ty.clone();
    let body = Expr::new(ExprKind::If(Box::new(cond), Box::new(kept), Box::new((**z).clone())), rt.clone(), sp);
    let lam = Lambda { params: vec![p], body: Box::new(body), drop_params: vec![false] };
    **x = Expr::new(ExprKind::Map(vec![src], lam), Type::Array(Box::new(rt), crate::Fixed::ANY), sp);
}

/// How the parent of an expression reads it.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// Evaluated into a value (not fused).
    Evaluated,
    /// Fused, its elements read in order.
    InOrder,
    /// Fused, its elements read by index.
    Indexed,
}

/// A `filter` fused into a consumer that reads it by index (an indexing, a
/// `scan`, a `map` of several inputs, a `filter`) cannot give its elements
/// that way: it is bound to a `let` (materialized) before the consumer that
/// starts the fused chain. Returns the bindings still to place, for the
/// parent, when `e` is itself fused.
fn place_streams(e: &mut Expr, vars: &mut Vec<VarInfo>, mode: Mode) -> Vec<(VarId, Expr)> {
    let slots = fusable_slots(e);
    let mut pending = Vec::new();
    let in_order = |e: &Expr, i: usize| is_stream_slot(e, i) && (mode != Mode::Indexed || !matches!(e.kind, ExprKind::Map(..)));
    let modes: Vec<Mode> = e
        .children()
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let fused = slots.contains(&i) && (is_producer(c) || matches!(c.kind, ExprKind::Filter(..)));
            if !fused {
                Mode::Evaluated
            } else if in_order(e, i) {
                Mode::InOrder
            } else {
                Mode::Indexed
            }
        })
        .collect();
    for (c, m) in e.children_mut().into_iter().zip(modes) {
        let mut sub = place_streams(c, vars, m);
        if m == Mode::Indexed && matches!(c.kind, ExprKind::Filter(..)) {
            vars.push(VarInfo { name: "filtered".into(), ty: c.ty.clone() });
            let t = (vars.len() - 1) as VarId;
            let (ty, sp) = (c.ty.clone(), c.span);
            sub.push((t, std::mem::replace(c, Expr::var(t, ty, sp))));
        }
        pending.extend(sub);
    }
    if mode == Mode::Evaluated {
        for (t, init) in pending.drain(..).rev() {
            let inner = take(e);
            let (ty, sp) = (inner.ty.clone(), init.span);
            *e = Expr::new(ExprKind::Let { var: t, init: Box::new(init), body: Box::new(inner), stmt: false, drop: false }, ty, sp);
        }
    }
    pending
}

// ---------------------------------------------------------------------
// lengths of producers (D98)

/// The length of an array given by `e`, computed without evaluating its
/// elements, when it is cheap: the length of the first input of a `map`, the
/// size given to `iota`, `zeros` or `fill` when it is a variable or a
/// literal.
fn length_expr(e: &Expr) -> Option<Expr> {
    let i64t = Type::Int(IntKind::I64);
    match &e.kind {
        ExprKind::Var(..) => Some(Expr::new(ExprKind::Len(Box::new(e.clone())), i64t, e.span)),
        ExprKind::Map(ins, _) => ins.first().and_then(length_expr),
        ExprKind::Slice(_, a, b)
            if matches!(a.kind, ExprKind::Var(..) | ExprKind::Lit(_)) && matches!(b.kind, ExprKind::Var(..) | ExprKind::Lit(_)) =>
        {
            Some(Expr::new(ExprKind::Binary(BinOp::Sub, b.clone(), a.clone()), i64t, e.span))
        }
        ExprKind::Iota(n) | ExprKind::Zeros(n) | ExprKind::Fill(n, _)
            if matches!(n.kind, ExprKind::Var(..) | ExprKind::Lit(_)) && n.ty == i64t =>
        {
            Some((**n).clone())
        }
        _ => None,
    }
}

fn replace_len(e: &mut Expr, v: VarId, with: &Expr) {
    if let ExprKind::Len(x) = &e.kind {
        if matches!(x.kind, ExprKind::Var(w, _) if w == v) {
            *e = with.clone();
            return;
        }
    }
    for c in e.children_mut() {
        replace_len(c, v, with);
    }
}

/// `length(v)`, where `v` is bound to a producer, becomes the length of the
/// producer's input: a test of sizes (the size check of `zip`, of `a + b`)
/// is not a use that materializes the producer (no rule M applies to it).
fn producer_lengths(e: &mut Expr) {
    for c in e.children_mut() {
        producer_lengths(c);
    }
    if let ExprKind::Let { var, init, body, .. } = &mut e.kind {
        if is_producer(init) {
            if let Some(l) = length_expr(init) {
                replace_len(body, *var, &l);
            }
        }
    }
}

/// The length of the array `e` when it is known before running (D228): the
/// bytes of a constant string, a literal or constant array, `iota`, `fill` or `zeros`
/// of a constant length, a `map` of such an input, through `let`s; `strs`
/// the variables bound to constant strings around it.
fn static_len(e: &Expr, strs: &HashMap<VarId, i64>) -> Option<i64> {
    match &e.kind {
        ExprKind::StrOp(crate::StrOp::Bytes, xs) if xs.len() == 1 => match &xs[0].kind {
            ExprKind::StrLit(s) => Some(s.len() as i64),
            ExprKind::Var(v, _) => strs.get(v).copied(),
            _ => None,
        },
        ExprKind::ArrayLit(es) => Some(es.len() as i64),
        ExprKind::ConstArray(ls) => Some(ls.len() as i64),
        ExprKind::ConstStrs(ss) => Some(ss.len() as i64),
        ExprKind::Iota(n) | ExprKind::Zeros(n) | ExprKind::Fill(n, _) => match n.kind {
            ExprKind::Lit(crate::Lit::Int(c)) if c >= 0 => Some(c),
            _ => None,
        },
        ExprKind::Map(ins, _) => ins.first().and_then(|i| static_len(i, strs)),
        ExprKind::Let { var, init, body, .. } => {
            let mut inner = strs.clone();
            if let ExprKind::StrLit(s) = &init.kind {
                inner.insert(*var, s.len() as i64);
            }
            static_len(body, &inner)
        }
        _ => None,
    }
}

/// `length(v)`, where `v` is bound to an array whose length is known before
/// running, becomes that constant (D228): `i % alu.length` over the bytes
/// of a constant string is then a remainder by a constant, carried by the
/// loop (D180), and its loop a vector one.
fn constant_lengths(e: &mut Expr, strs: &mut HashMap<VarId, i64>) {
    if let ExprKind::Let { var, init, body, .. } = &mut e.kind {
        constant_lengths(init, strs);
        if let ExprKind::StrLit(s) = &init.kind {
            strs.insert(*var, s.len() as i64);
        }
        if init.ty.is_array() {
            if let Some(n) = static_len(init, strs) {
                let lit = Expr::new(ExprKind::Lit(crate::Lit::Int(n)), Type::Int(IntKind::I64), init.span);
                replace_len(body, *var, &lit);
            }
        }
        constant_lengths(body, strs);
        return;
    }
    for c in e.children_mut() {
        constant_lengths(c, strs);
    }
}

// ---------------------------------------------------------------------
// a function of an element of a small table (D230)

/// The bounds of the integer `e` when they are known: constants, `min`,
/// `max`, sums and differences, the branches of an `if`, a sum (`reduce`
/// with `+`) of the terms of a `map` over an array of known length.
fn int_bounds(e: &Expr, lens: &HashMap<VarId, i64>) -> Option<(i64, i64)> {
    match &e.kind {
        ExprKind::Lit(Lit::Int(c)) => Some((*c, *c)),
        ExprKind::Convert(x) if x.ty == Type::Bool => Some((0, 1)),
        ExprKind::If(_, t, f) => {
            let (a, b) = (int_bounds(t, lens)?, int_bounds(f, lens)?);
            Some((a.0.min(b.0), a.1.max(b.1)))
        }
        // `let v = map(...) in reduce(v, +, z)`: the sum of the map
        ExprKind::Let { var, init, body, .. } => match &body.kind {
            ExprKind::Reduce(x, l, z) if matches!(x.kind, ExprKind::Var(v, _) if v == *var) && matches!(init.kind, ExprKind::Map(..)) => {
                sum_bounds(init, l, z, lens)
            }
            _ => int_bounds(body, lens),
        },
        ExprKind::Binary(op, a, b) => {
            let (x, y) = (int_bounds(a, lens)?, int_bounds(b, lens)?);
            match op {
                BinOp::Min => Some((x.0.min(y.0), x.1.min(y.1))),
                BinOp::Max => Some((x.0.max(y.0), x.1.max(y.1))),
                BinOp::Add => Some((x.0.checked_add(y.0)?, x.1.checked_add(y.1)?)),
                BinOp::Sub => Some((x.0.checked_sub(y.1)?, x.1.checked_sub(y.0)?)),
                _ => None,
            }
        }
        ExprKind::Reduce(x, l, z) => sum_bounds(x, l, z, lens),
        _ => None,
    }
}

/// The bounds of `reduce(x, +, z)` where `x` is a `map` over an array of
/// known length: z plus that many terms within the bounds of its body.
fn sum_bounds(x: &Expr, l: &Lambda, z: &Expr, lens: &HashMap<VarId, i64>) -> Option<(i64, i64)> {
    if exact_op(l) != Some(BinOp::Add) {
        return None;
    }
    let ExprKind::Map(ins, m) = &x.kind else { return None };
    let n = match &ins.first()?.kind {
        ExprKind::Var(v, _) => *lens.get(v)?,
        _ => static_len(&ins[0], &HashMap::new())?,
    };
    let (a, b) = int_bounds(&m.body, lens)?;
    let (z0, z1) = int_bounds(z, lens)?;
    Some((z0.checked_add(a.min(0).checked_mul(n)?)?, z1.checked_add(b.max(0).checked_mul(n)?)?))
}

/// `e` cannot panic, whatever its free variables: as `cannot_panic`, and
/// besides the elements of arrays of known length at indices whose bounds
/// are inside it, the tests of sizes of two equal constants.
fn total(e: &Expr, lens: &HashMap<VarId, i64>) -> bool {
    use ExprKind::*;
    match &e.kind {
        Index(a, i) => {
            let ok = match a.kind {
                Var(v, _) => match (lens.get(&v), int_bounds(i, lens)) {
                    (Some(&n), Some((lo, hi))) => lo >= 0 && hi < n,
                    _ => false,
                },
                _ => false,
            };
            ok && total(i, lens)
        }
        CheckSize(a, b, body) => matches!((&a.kind, &b.kind), (Lit(crate::Lit::Int(x)), Lit(crate::Lit::Int(y))) if x == y) && total(body, lens),
        Update { .. } | Slice(..) | Window(..) | CheckNonNegative(..) | Action(..) | Bench(..) | Call(..) => false,
        Binary(BinOp::Div | BinOp::Rem, a, b) if matches!(a.ty, Type::Int(_)) => {
            matches!(b.kind, Lit(crate::Lit::Int(d)) if d != 0 && d != -1) && total(a, lens)
        }
        Pow(..) => cannot_panic(e),
        _ => e.children().into_iter().all(|c| total(c, lens)),
    }
}

/// `e` costs more than reading an element: a loop, a division of floats,
/// a root.
fn dear(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Map(..) | ExprKind::Reduce(..) | ExprKind::Fold(..) | ExprKind::Scan(..) | ExprKind::Filter(..) | ExprKind::Tasks(..) | ExprKind::Sqrt(_) => true,
        ExprKind::Binary(BinOp::Div | BinOp::Rem, a, _) if matches!(a.ty, Type::F64 | Type::F32) => true,
        _ => e.children().into_iter().any(dear),
    }
}

/// The spans of `e` all set to `sp` (to compare two expressions).
fn unspan(e: &mut Expr, sp: Span) {
    e.span = sp;
    for c in e.children_mut() {
        unspan(c, sp);
    }
}

fn same(a: &Expr, b: &Expr) -> bool {
    let (mut x, mut y) = (a.clone(), b.clone());
    unspan(&mut x, a.span);
    unspan(&mut y, a.span);
    x == y
}

/// Each `t[i]` in `e` (the same `i` as `idx`) replaced by `with`.
fn replace_loads(e: &mut Expr, t: VarId, idx: &Expr, with: &Expr) -> usize {
    if let ExprKind::Index(a, i) = &e.kind {
        if matches!(a.kind, ExprKind::Var(v, _) if v == t) && same(i, idx) {
            *e = with.clone();
            return 1;
        }
    }
    e.children_mut().into_iter().map(|c| replace_loads(c, t, idx, with)).sum()
}

/// The variables bound inside `e` renamed to new ones (a copy of `e`
/// beside it).
fn rename_bound(e: &mut Expr, vars: &mut Vec<VarInfo>) {
    let mut bound = HashSet::new();
    bound_in(e, &mut bound);
    let map: HashMap<VarId, VarId> = bound
        .into_iter()
        .map(|v| {
            vars.push(vars[v as usize].clone());
            (v, (vars.len() - 1) as VarId)
        })
        .collect();
    fn go(e: &mut Expr, map: &HashMap<VarId, VarId>) {
        match &mut e.kind {
            ExprKind::Var(v, _) => {
                if let Some(w) = map.get(v) {
                    *v = *w;
                }
            }
            ExprKind::Let { var, .. } => {
                if let Some(w) = map.get(var) {
                    *var = *w;
                }
            }
            ExprKind::Map(_, l) | ExprKind::Reduce(_, l, _) | ExprKind::Fold(_, _, l) | ExprKind::Scan(_, l, _) | ExprKind::Filter(_, l) | ExprKind::Tasks(_, l) => {
                for p in l.params.iter_mut() {
                    if let Some(w) = map.get(p) {
                        *p = *w;
                    }
                }
            }
            _ => {}
        }
        for c in e.children_mut() {
            go(c, map);
        }
    }
    go(e, &map);
}

/// The reads `t[i]` in `e` of an array `t` not bound in it (`inner`), of
/// scalars, at an index that depends on what is bound in it: their paths.
fn table_loads(e: &Expr, inner: &HashSet<VarId>, path: &mut Vec<usize>, out: &mut Vec<(Vec<usize>, VarId)>) {
    if let ExprKind::Index(a, i) = &e.kind {
        if let ExprKind::Var(t, _) = a.kind {
            let mut fi = HashSet::new();
            free_in(i, &mut Vec::new(), &mut fi);
            if !inner.contains(&t) && a.ty.elem().is_scalar() && fi.iter().any(|v| inner.contains(v)) {
                out.push((path.clone(), t));
            }
        }
    }
    for (k, c) in e.children().into_iter().enumerate() {
        path.push(k);
        table_loads(c, inner, path, out);
        path.pop();
    }
}

fn at_path_ref<'a>(e: &'a Expr, path: &[usize]) -> &'a Expr {
    match path.split_first() {
        None => e,
        Some((i, rest)) => at_path_ref(e.children().into_iter().nth(*i).unwrap(), rest),
    }
}

/// `map(x, (q) -> ... h(t[i]) ...)`, where `h(t[i])` depends on `q` only
/// through the element `t[i]` of an array `t` bound outside, costs more
/// than a read (`dear`) and cannot panic whatever the element (`total`):
/// when the `map` has at least twice as many elements as `t`, `h` is
/// computed once per element of `t` (`H = map(t, h)`) and each element of
/// the `map` reads `H[i]`; otherwise the `map` as it was. The same
/// operations on the same values: the same bits; `H[i]` panics where `t[i]`
/// did.
fn table_function(m: &Expr, vars: &mut Vec<VarInfo>, lens: &HashMap<VarId, i64>) -> Option<Expr> {
    let ExprKind::Map(ins, _) = &m.kind else { return None };
    if ins.len() != 1 {
        return None;
    }
    // the length of `iota(n)`, `n` computed once: bound to a variable
    let mut m = m.clone();
    let mut bind_len = None;
    if let ExprKind::Map(ins, _) = &mut m.kind {
        if let ExprKind::Iota(n) = &mut ins[0].kind {
            if !matches!(n.kind, ExprKind::Var(..) | ExprKind::Lit(_)) {
                vars.push(VarInfo { name: "length".into(), ty: Type::Int(IntKind::I64) });
                let nv = (vars.len() - 1) as VarId;
                let value = std::mem::replace(&mut **n, Expr::var(nv, Type::Int(IntKind::I64), m.span));
                bind_len = Some((nv, value));
            }
        }
    }
    let m = &m;
    let ExprKind::Map(ins, l) = &m.kind else { unreachable!() };
    // (through the test of a size around the input: it stays in the `map`)
    let input = match &ins[0].kind {
        ExprKind::CheckNonNegative(_, x) => &**x,
        _ => &ins[0],
    };
    let n_map = length_expr(input)?;
    let mut inner = HashSet::new();
    bound_in(&l.body, &mut inner);
    inner.extend(&l.params);
    let mut loads = Vec::new();
    table_loads(&l.body, &inner, &mut Vec::new(), &mut loads);
    let sp = m.span;
    for (path, t) in loads {
        let ExprKind::Index(_, idx) = &at_path_ref(&l.body, &path).kind else { continue };
        let idx = (**idx).clone();
        let mut fi = HashSet::new();
        free_in(&idx, &mut Vec::new(), &mut fi);
        let elem = vars[t as usize].ty.elem().clone();
        vars.push(VarInfo { name: "elem".into(), ty: elem.clone() });
        let ev = (vars.len() - 1) as VarId;
        let hole = Expr::var(ev, elem.clone(), sp);
        // the highest node holding the read that depends on nothing else
        // bound in the `map`, and where the index is in scope
        let mut best = None;
        for k in (0..path.len()).rev() {
            let x = at_path_ref(&l.body, &path[..k]);
            if !x.ty.is_scalar() || matches!(x.kind, ExprKind::Let { stmt: true, .. }) {
                continue;
            }
            let mut inside = HashSet::new();
            bound_in(x, &mut inside);
            if fi.iter().any(|v| inside.contains(v)) {
                continue;
            }
            let mut xc = x.clone();
            replace_loads(&mut xc, t, &idx, &hole);
            let mut free = HashSet::new();
            free_in(&xc, &mut Vec::new(), &mut free);
            free.remove(&ev);
            if free.iter().any(|v| inner.contains(v)) {
                continue;
            }
            best = Some((k, xc));
        }
        let Some((k, h)) = best else { continue };
        if !dear(&h) || !total(&h, lens) {
            continue;
        }
        // H = map(t, (elem) -> h), its own variables
        let mut h = h;
        rename_bound(&mut h, vars);
        let hty = Type::Array(Box::new(h.ty.clone()), crate::Fixed::ANY);
        vars.push(VarInfo { name: "table".into(), ty: hty.clone() });
        let hv = (vars.len() - 1) as VarId;
        let tvar = Expr::var(t, vars[t as usize].ty.clone(), sp);
        let table = Expr::new(ExprKind::Map(vec![tvar.clone()], Lambda { params: vec![ev], body: Box::new(h), drop_params: vec![false] }), hty.clone(), sp);
        // the `map` with H[i] where h(t[i]) was
        let mut fast = m.clone();
        {
            let ExprKind::Map(_, fl) = &mut fast.kind else { unreachable!() };
            let x = at_path(&mut fl.body, &path[..k]);
            let read = Expr::new(ExprKind::Index(Box::new(Expr::var(hv, hty.clone(), sp)), Box::new(idx.clone())), x.ty.clone(), sp);
            *x = read;
        }
        let i64t = Type::Int(IntKind::I64);
        let tlen = Expr::new(ExprKind::Len(Box::new(tvar)), i64t.clone(), sp);
        let twice = Expr::new(ExprKind::Binary(BinOp::Mul, Box::new(Expr::i64(2, sp)), Box::new(tlen)), i64t, sp);
        let worth = Expr::new(ExprKind::Binary(BinOp::Ge, Box::new(n_map), Box::new(twice)), Type::Bool, sp);
        let with_table = Expr::new(ExprKind::Let { var: hv, init: Box::new(table), body: Box::new(fast), stmt: false, drop: false }, m.ty.clone(), sp);
        let mut slow = m.clone();
        rename_bound(&mut slow, vars);
        let choice = Expr::new(ExprKind::If(Box::new(worth), Box::new(with_table), Box::new(slow)), m.ty.clone(), sp);
        return Some(match bind_len {
            Some((nv, value)) => Expr::new(ExprKind::Let { var: nv, init: Box::new(value), body: Box::new(choice), stmt: false, drop: false }, m.ty.clone(), sp),
            None => choice,
        });
    }
    None
}

/// `table_function` at every `map`, the lengths known before running of
/// the arrays bound around it (`static_len`).
fn table_functions(e: &mut Expr, vars: &mut Vec<VarInfo>, lens: &mut HashMap<VarId, i64>) {
    if let ExprKind::Let { var, init, body, .. } = &mut e.kind {
        table_functions(init, vars, lens);
        if init.ty.is_array() {
            if let Some(n) = static_len(init, &HashMap::new()) {
                lens.insert(*var, n);
            }
        }
        table_functions(body, vars, lens);
        return;
    }
    for c in e.children_mut() {
        table_functions(c, vars, lens);
    }
    if matches!(e.kind, ExprKind::Map(..)) {
        if let Some(new) = table_function(e, vars, lens) {
            *e = new;
        }
    }
}

// ---------------------------------------------------------------------
// zip, a view (rule F4, D97)

/// The `map` that `zip` is lowered to: its lambda builds the tuple of its
/// parameters, in order.
/// D495: `map(zip(a, b), (p) => f)`, the components of the pairs scalars
/// and `f` reading `p` only by its components (`const [x, y] = p`, `p[0]`):
/// one `map` over `a` and `b` whose parameters are the components, each
/// `p[k]` replaced by the k-th. The pairs are never made: a reduction of
/// that `map` reads its inputs in vectors (the sum of `map(zip(...))` 3.36
/// times its floor: the `map` made into a buffer, its source of pairs not
/// read in vectors).
fn compose_zip_maps(e: &mut Expr) {
    for c in e.children_mut() {
        compose_zip_maps(c);
    }
    let ExprKind::Map(ins, l2) = &mut e.kind else { return };
    if ins.len() != 1 || l2.params.len() != 1 || !is_zip(&ins[0]) {
        return;
    }
    let ExprKind::Map(_, l1) = &ins[0].kind else { return };
    let ExprKind::Tuple(comps) = &l1.body.kind else { return };
    if !comps.iter().all(|c| c.ty.is_scalar()) {
        return;
    }
    let p = l2.params[0];
    if !only_projected(&l2.body, p) {
        return;
    }
    let tys: Vec<Type> = comps.iter().map(|c| c.ty.clone()).collect();
    let inner = ins.pop().unwrap();
    let ExprKind::Map(srcs, l1) = inner.kind else { unreachable!() };
    let params = l1.params.clone();
    let mut body = take(&mut l2.body);
    replace_projections(&mut body, p, &params, &tys);
    let n = params.len();
    e.kind = ExprKind::Map(srcs, Lambda { params, body: Box::new(body), drop_params: vec![false; n] });
}

/// `p` read only by its components in `e` (`Proj` of the variable itself).
fn only_projected(e: &Expr, p: VarId) -> bool {
    match &e.kind {
        ExprKind::Proj(inner, _) if matches!(inner.kind, ExprKind::Var(w, _) if w == p) => true,
        ExprKind::Var(w, _) => *w != p,
        _ => e.children().into_iter().all(|c| only_projected(c, p)),
    }
}

/// Each `p[k]` of `e` replaced by the variable `params[k]` (of type `tys[k]`).
fn replace_projections(e: &mut Expr, p: VarId, params: &[VarId], tys: &[Type]) {
    if let ExprKind::Proj(inner, k) = &e.kind {
        if matches!(inner.kind, ExprKind::Var(w, _) if w == p) {
            let k = *k as usize;
            *e = Expr::var(params[k], tys[k].clone(), e.span);
            return;
        }
    }
    for c in e.children_mut() {
        replace_projections(c, p, params, tys);
    }
}

fn is_zip(e: &Expr) -> bool {
    let ExprKind::Map(ins, l) = &e.kind else { return false };
    let ExprKind::Tuple(es) = &l.body.kind else { return false };
    ins.len() >= 2
        && es.len() == l.params.len()
        && es.iter().zip(&l.params).all(|(x, p)| matches!(x.kind, ExprKind::Var(v, _) if v == *p))
}

/// An input a copied zip can read again without computing anything: a
/// variable, or a zip of such inputs.
fn is_view_input(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Var(..) => true,
        ExprKind::Map(ins, _) => is_zip(e) && ins.iter().all(is_view_input),
        _ => false,
    }
}

/// A copy of a zip of view inputs, with parameters of its own.
fn fresh_zip(e: &Expr, vars: &mut Vec<VarInfo>) -> Expr {
    let ExprKind::Map(ins, l) = &e.kind else { return e.clone() };
    let ins: Vec<Expr> = ins.iter().map(|i| if is_zip(i) { fresh_zip(i, vars) } else { i.clone() }).collect();
    let mut params = Vec::new();
    let mut es = Vec::new();
    for p in &l.params {
        let info = vars[*p as usize].clone();
        let ty = info.ty.clone();
        vars.push(info);
        let q = (vars.len() - 1) as VarId;
        params.push(q);
        es.push(Expr::var(q, ty, l.body.span));
    }
    let body = Expr::new(ExprKind::Tuple(es), l.body.ty.clone(), l.body.span);
    let drop_params = vec![false; params.len()];
    Expr::new(ExprKind::Map(ins, Lambda { params, body: Box::new(body), drop_params }), e.ty.clone(), e.span)
}

fn replace_all_zip(e: &mut Expr, v: VarId, zip: &Expr, vars: &mut Vec<VarInfo>) {
    if let ExprKind::Var(w, _) = e.kind {
        if w == v {
            *e = fresh_zip(zip, vars);
        }
        return;
    }
    for c in e.children_mut() {
        replace_all_zip(c, v, zip, vars);
    }
}

/// Rule F4: a zip is never materialized, even shared, except by rule M3. A
/// `let`-bound zip used more than once, or inside a lambda (once per
/// iteration), is replaced at each use by a copy of itself, which the
/// consumer fuses (F1); a use that is not a consumer slot (an action, a
/// tuple, ...) materializes it there (M3). An input that is not a variable
/// (a `map`, ...) is bound to a `let` first: that one is a result used more
/// than once, materialized once (M1). The size test of the zip and the
/// `let`s of its inputs stay where they were, once.
fn inline_zips(e: &mut Expr, vars: &mut Vec<VarInfo>) {
    for c in e.children_mut() {
        inline_zips(c, vars);
    }
    let ExprKind::Let { var, init, body, .. } = &mut e.kind else { return };
    let mut core: &Expr = init;
    loop {
        match &core.kind {
            ExprKind::CheckSize(_, _, b) | ExprKind::Let { body: b, .. } => core = b,
            _ => break,
        }
    }
    if !is_zip(core) {
        return;
    }
    let z = *var;
    let mut counts = HashMap::new();
    count_uses(body, &mut counts);
    let n = counts.get(&z).copied().unwrap_or(0);
    if n == 0 || (n == 1 && use_in_slot(body, z)) {
        return; // dead, or fused by F1
    }
    // the size tests and the `let`s of the inputs, kept around the uses
    enum Wrap {
        Check(Box<Expr>, Box<Expr>),
        Let(VarId, Box<Expr>, bool, bool),
    }
    let mut zip = take(init);
    let mut wraps = Vec::new();
    loop {
        match zip.kind {
            ExprKind::CheckSize(a, b, inner) => {
                wraps.push((Wrap::Check(a, b), zip.span));
                zip = *inner;
            }
            ExprKind::Let { var, init, body, stmt, drop } => {
                wraps.push((Wrap::Let(var, init, stmt, drop), zip.span));
                zip = *body;
            }
            kind => {
                zip.kind = kind;
                break;
            }
        }
    }
    // inputs that would be computed again: bound once
    let mut lets = Vec::new();
    if let ExprKind::Map(ins, _) = &mut zip.kind {
        for i in ins.iter_mut() {
            if !is_view_input(i) {
                vars.push(VarInfo { name: "zip_in".into(), ty: i.ty.clone() });
                let t = (vars.len() - 1) as VarId;
                let (ty, sp) = (i.ty.clone(), i.span);
                lets.push((t, std::mem::replace(i, Expr::var(t, ty, sp))));
            }
        }
    }
    replace_all_zip(body, z, &zip, vars);
    let mut out = take(body);
    for (w, sp) in wraps.into_iter().rev() {
        let ty = out.ty.clone();
        let kind = match w {
            Wrap::Check(a, b) => ExprKind::CheckSize(a, b, Box::new(out)),
            Wrap::Let(var, init, stmt, drop) => ExprKind::Let { var, init, body: Box::new(out), stmt, drop },
        };
        out = Expr::new(kind, ty, sp);
    }
    for (t, init) in lets.into_iter().rev() {
        let (ty, sp) = (out.ty.clone(), init.span);
        out = Expr::new(
            ExprKind::Let { var: t, init: Box::new(init), body: Box::new(out), stmt: false, drop: false },
            ty,
            sp,
        );
    }
    *e = out;
}

fn fuse_fixpoint(e: &mut Expr) {
    for _ in 0..1000 {
        let mut counts = HashMap::new();
        count_uses(e, &mut counts);
        if !fuse(e, &counts) {
            break;
        }
    }
}

fn take(e: &mut Expr) -> Expr {
    std::mem::replace(e, Expr::lit(Lit::Unit, Type::Unit, e.span))
}

fn fuse(e: &mut Expr, counts: &HashMap<VarId, usize>) -> bool {
    let mut changed = false;
    for c in e.children_mut() {
        changed |= fuse(c, counts);
    }
    // let v = <producer or filter> in body, single use in a fusable slot (a
    // filter read by index there is bound again by `place_streams`)
    if let ExprKind::Let { var, init, body, stmt: false, .. } = &mut e.kind {
        let movable = (is_producer(init) && !owns_elems(init)) || matches!(init.kind, ExprKind::Filter(..));
        if movable && counts.get(var).copied().unwrap_or(0) == 1 && use_in_slot(body, *var) {
            let v = *var;
            let mut with = Some(take(init));
            replace_var(body, v, &mut with);
            let b = take(body);
            *e = b;
            return true;
        }
    }
    // let v = W(P) in body  =>  W(let v = P in body), W a scalar let or a size check
    if let ExprKind::Let { init, .. } = &mut e.kind {
        if matches!(init.kind, ExprKind::Let { stmt: false, .. } | ExprKind::CheckSize(..) | ExprKind::CheckNonNegative(..)) && is_wrapped_producer(init) {
            let w = split_wrapper(init);
            let inner = take(e);
            *e = fill_wrapper(w, inner);
            return true;
        }
    }
    // consumer(.., wrapped producer, ..) => float the wrappers out
    let slots = fusable_slots(e);
    if !slots.is_empty() && e.children().into_iter().all(is_pure) {
        for (i, c) in e.children_mut().into_iter().enumerate() {
            if slots.contains(&i)
                && matches!(c.kind, ExprKind::Let { stmt: false, .. } | ExprKind::CheckSize(..) | ExprKind::CheckNonNegative(..))
                && is_wrapped_producer(c)
            {
                let _ = c;
                float_slot(e, i);
                return true;
            }
        }
    }
    changed
}

/// Takes the wrapper (`let s = a in []` or `check_size(a, b, [])`) at the
/// top of `slot`, leaving its body in place, and returns it with a hole.
fn split_wrapper(slot: &mut Expr) -> Expr {
    let mut w = take(slot);
    let body = match &mut w.kind {
        ExprKind::Let { body, .. } => take(body),
        ExprKind::CheckSize(_, _, b) | ExprKind::CheckNonNegative(_, b) => take(b),
        _ => unreachable!(),
    };
    *slot = body;
    w
}

fn fill_wrapper(mut w: Expr, inner: Expr) -> Expr {
    w.ty = inner.ty.clone();
    match &mut w.kind {
        ExprKind::Let { body, .. } => **body = inner,
        ExprKind::CheckSize(_, _, b) | ExprKind::CheckNonNegative(_, b) => **b = inner,
        _ => unreachable!(),
    }
    w
}

fn float_slot(e: &mut Expr, i: usize) {
    let w = {
        let mut ch = e.children_mut();
        split_wrapper(&mut ch[i])
    };
    let inner = take(e);
    *e = fill_wrapper(w, inner);
}

// ---------------------------------------------------------------------
// binding of temporaries

fn is_borrowed_expr(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Var(..) => true,
        ExprKind::Proj(x, _) => is_borrowed_expr(x),
        _ => false,
    }
}

/// `s.slice(a, b)` and `bytes(s)` of a string read where it is (a variable
/// or a component of one, as every operand after `bind_temporaries`): a
/// view of its bytes, as a slice of an array (rule F4), neither copied nor
/// freed; a consumer that keeps it gets a copy (D303).
pub fn str_view(op: crate::StrOp, es: &[Expr], ty: &Type) -> bool {
    match op {
        crate::StrOp::Slice => *ty == Type::Str && is_borrowed_expr(&es[0]),
        crate::StrOp::Bytes => es.len() == 1 && is_borrowed_expr(&es[0]),
        _ => false,
    }
}

/// Indices (in `children_mut` order) of the slots that only read their
/// argument.
fn borrowing_slots(e: &Expr) -> Vec<usize> {
    match &e.kind {
        ExprKind::Index(..) | ExprKind::Len(_) | ExprKind::Proj(..) | ExprKind::Slice(..) | ExprKind::Unbox(_) => vec![0],
        ExprKind::Map(ins, _) => (0..ins.len()).collect(),
        ExprKind::Reduce(..) | ExprKind::Fold(..) | ExprKind::Scan(..) | ExprKind::Filter(..) => vec![0],
        ExprKind::Action(_, args) | ExprKind::Call(_, args) => (0..args.len()).collect(),
        ExprKind::StrOp(_, args) => (0..args.len()).collect(),
        _ => vec![],
    }
}

fn bind_temporaries(e: &mut Expr, vars: &mut Vec<VarInfo>) {
    for c in e.children_mut() {
        bind_temporaries(c, vars);
    }
    let slots = borrowing_slots(e);
    let fusable = fusable_slots(e);
    let mut lets: Vec<(VarId, Expr)> = Vec::new();
    for (i, c) in e.children_mut().into_iter().enumerate() {
        if !slots.contains(&i) || !c.ty.has_heap() || is_borrowed_expr(c) {
            continue;
        }
        if fusable.contains(&i) && ((is_producer(c) && !owns_elems(c)) || matches!(c.kind, ExprKind::Filter(..))) {
            continue;
        }
        vars.push(VarInfo { name: "tmp".into(), ty: c.ty.clone() });
        let t = (vars.len() - 1) as VarId;
        let sp = c.span;
        let ty = c.ty.clone();
        let val = std::mem::replace(c, Expr::var(t, ty, sp));
        lets.push((t, val));
    }
    for (t, val) in lets.into_iter().rev() {
        let inner = take(e);
        let ty = inner.ty.clone();
        let sp = inner.span;
        *e = Expr::new(
            ExprKind::Let { var: t, init: Box::new(val), body: Box::new(inner), stmt: false, drop: false },
            ty,
            sp,
        );
    }
}

// ---------------------------------------------------------------------
// ownership

struct Info {
    alias: Vec<Option<Vec<VarId>>>,
    owned: Vec<bool>,
    def_region: Vec<u32>,
    /// uses[v] = (traversal index, region, span) of every use of v or of an
    /// alias of v.
    uses: Vec<Vec<(usize, u32, Span)>>,
    candidates: HashSet<usize>,
    moved: Vec<bool>,
    /// The tree of regions: the parent of each, and for the branch of an
    /// `if`, the `if` and its side (a lambda body has none) (D105).
    region_parent: Vec<u32>,
    region_side: Vec<Option<(u32, u8)>>,
}

impl Info {
    /// The regions from `r` up to `top` (both included), if `top` is above.
    fn chain(&self, mut r: u32, top: u32) -> Option<Vec<u32>> {
        let mut out = vec![r];
        while r != top {
            if r == 0 {
                return None;
            }
            r = self.region_parent[r as usize];
            out.push(r);
        }
        Some(out)
    }

    /// A use in region `a` and a later one in region `b`, both under `top`,
    /// never run together: they are in the two branches of one `if`.
    fn exclusive(&self, a: u32, b: u32, top: u32) -> bool {
        let (Some(ca), Some(cb)) = (self.chain(a, top), self.chain(b, top)) else { return false };
        // from the top down, the first regions where the chains differ
        let (mut i, mut j) = (ca.len(), cb.len());
        while i > 0 && j > 0 && ca[i - 1] == cb[j - 1] {
            i -= 1;
            j -= 1;
        }
        if i == 0 || j == 0 {
            return false; // one is inside the other
        }
        match (self.region_side[ca[i - 1] as usize], self.region_side[cb[j - 1] as usize]) {
            (Some((x, s)), Some((y, t))) => x == y && s != t,
            _ => false,
        }
    }

    /// The use `(index, region)` of `v` is its last on every path: it is in
    /// the region of `v` or in branches below it (not in a lambda), and every
    /// later use is in the other branch of an `if`.
    fn last_on_its_path(&self, v: usize, index: usize, region: u32) -> bool {
        let top = self.def_region[v];
        let Some(ch) = self.chain(region, top) else { return false };
        if ch[..ch.len() - 1].iter().any(|r| self.region_side[*r as usize].is_none()) {
            return false;
        }
        self.uses[v].iter().filter(|(j, _, _)| *j > index).all(|(_, r, _)| self.exclusive(region, *r, top))
    }
    fn base(&self, v: VarId) -> Vec<VarId> {
        match &self.alias[v as usize] {
            Some(b) => b.clone(),
            None => vec![v],
        }
    }

    /// The variables an expression may alias (None: fresh or scalar).
    fn borrows(&self, e: &Expr) -> Option<Vec<VarId>> {
        borrowed_from(e, &|v| self.base(v))
    }
}

/// The variables whose memory `e` borrows, `base` giving those of a
/// variable (None: new memory or a scalar).
fn borrowed_from(e: &Expr, base: &dyn Fn(VarId) -> Vec<VarId>) -> Option<Vec<VarId>> {
    if !e.ty.has_heap() {
        return None;
    }
    match &e.kind {
        ExprKind::Var(v, _) => Some(base(*v)),
        // data of the program (D173): borrowed from no variable; a string
        // literal too (D301)
        ExprKind::ConstArray(_) | ExprKind::ConstStrs(_) => Some(Vec::new()),
        ExprKind::StrLit(_) if e.ty == Type::Str => Some(Vec::new()),
        ExprKind::Proj(x, _) | ExprKind::Unbox(x) => borrowed_from(x, base),
        // views of an array of arrays: its windows, its rows (D95); slices
        // (rule F4, D97)
        ExprKind::Window(x, _) | ExprKind::Index(x, _) => borrowed_from(x, base),
        ExprKind::Slice(x, _, _) if e.ty.slice_is_view() => borrowed_from(x, base),
        // a slice or the bytes of a string (D303)
        ExprKind::StrOp(op, es) if str_view(*op, es, &e.ty) => borrowed_from(&es[0], base),
        ExprKind::CheckSize(_, _, b) | ExprKind::CheckNonNegative(_, b) | ExprKind::Free(_, b) => borrowed_from(b, base),
        ExprKind::Let { var, body, .. } => match borrowed_from(body, base) {
            Some(s) if s.contains(var) => None,
            other => other,
        },
        ExprKind::If(_, t, f) => match (borrowed_from(t, base), borrowed_from(f, base)) {
            (Some(mut a), Some(b)) => {
                a.extend(b);
                a.sort();
                a.dedup();
                Some(a)
            }
            _ => None,
        },
        _ => None,
    }
}

struct Walker<'a> {
    info: &'a mut Info,
    index: usize,
    next_region: u32,
    next_if: u32,
    /// The innermost position in the program's file: a use inside an
    /// inlined function of the prelude is reported where it is called.
    user: Span,
}

fn region_children(e: &Expr) -> Vec<bool> {
    // true for the children that start a new region
    let n = e.children().len();
    let mut v = vec![false; n];
    match &e.kind {
        ExprKind::If(..) => {
            v[1] = true;
            v[2] = true;
        }
        _ => {
            if let Some(i) = lambda_body_index(e) {
                v[i] = true;
            }
        }
    }
    v
}

fn lambda_of(e: &Expr) -> Option<&Lambda> {
    match &e.kind {
        ExprKind::Map(_, l) | ExprKind::Reduce(_, l, _) | ExprKind::Fold(_, _, l) | ExprKind::Scan(_, l, _) | ExprKind::Filter(_, l) | ExprKind::Tasks(_, l) => Some(l),
        _ => None,
    }
}

impl<'a> Walker<'a> {
    /// Analysis: aliases, owned variables, uses in evaluation order.
    fn scan(&mut self, e: &Expr, region: u32) {
        let saved = self.user;
        if e.span.file == isc_types::USER_FILE {
            self.user = e.span;
        }
        self.scan_in(e, region);
        self.user = saved;
    }

    fn scan_in(&mut self, e: &Expr, region: u32) {
        match &e.kind {
            ExprKind::Var(v, _) => {
                let i = self.index;
                self.index += 1;
                let bases = self.info.base(*v);
                for b in bases {
                    self.info.uses[b as usize].push((i, region, self.user));
                }
                return;
            }
            ExprKind::Let { var, init, body, .. } => {
                self.scan(init, region);
                let v = *var as usize;
                self.info.def_region[v] = region;
                if init.ty.has_heap() {
                    match self.info.borrows(init) {
                        Some(b) => self.info.alias[v] = Some(b),
                        None => self.info.owned[v] = true,
                    }
                }
                self.scan(body, region);
                return;
            }
            _ => {}
        }
        let regs = region_children(e);
        let lam = lambda_of(e);
        let is_fold = matches!(e.kind, ExprKind::Fold(..));
        let if_id = if matches!(e.kind, ExprKind::If(..)) {
            self.next_if += 1;
            Some(self.next_if)
        } else {
            None
        };
        for (i, c) in e.children().into_iter().enumerate() {
            let r = if regs[i] {
                self.next_region += 1;
                self.info.region_parent.push(region);
                self.info.region_side.push(if_id.map(|x| (x, i as u8)));
                self.next_region
            } else {
                region
            };
            if regs[i] {
                if let Some(l) = lam {
                    for (k, p) in l.params.iter().enumerate() {
                        self.info.def_region[*p as usize] = r;
                        // the accumulator of a fold owns its value
                        if is_fold && k == 0 && l.body.ty.has_heap() {
                            self.info.owned[*p as usize] = true;
                        }
                    }
                }
            }
            self.scan(c, r);
        }
    }
}

#[derive(Clone, Debug)]
enum Own {
    Scalar,
    Fresh,
    /// Aliases these variables; `tail` is the traversal index of the variable
    /// in tail position, if the expression is a chain ending in a variable.
    Borrowed(Vec<VarId>, Option<usize>),
}

struct Rewriter<'a> {
    info: &'a mut Info,
    vars: &'a [VarInfo],
    index: usize,
    copy_reasons: Vec<(Span, String)>,
    /// The branches of an `if` whose two branches borrow, with what each
    /// borrows: made new one by one if the `if` must be (D138).
    branch_owns: HashMap<*const Expr, Own>,
    /// The `map`s of scalars whose input `k` is a variable at its last use,
    /// owned, of the same scalars: written into it when the `map` is
    /// materialized (D207).
    reusable: HashMap<*const Expr, usize>,
}

impl<'a> Rewriter<'a> {
    fn walk(&mut self, e: &mut Expr) -> R<Own> {
        let heap = e.ty.has_heap();
        // a slice of an array of tuples is a copy, not a view (D97)
        let view = !matches!(&e.kind, ExprKind::Slice(..)) || e.ty.slice_is_view();
        let str_part = matches!(&e.kind, ExprKind::StrOp(op, es) if str_view(*op, es, &e.ty));
        match &mut e.kind {
            ExprKind::Var(v, _) => {
                let i = self.index;
                self.index += 1;
                return Ok(if heap { Own::Borrowed(self.info.base(*v), Some(i)) } else { Own::Scalar });
            }
            // data of the program (D173): a consumer gets a copy; a string
            // literal too, read where it is (D301: made and freed at each
            // evaluation before, a test against a literal in a loop)
            ExprKind::ConstArray(_) | ExprKind::ConstStrs(_) => return Ok(Own::Borrowed(Vec::new(), None)),
            ExprKind::StrLit(_) if e.ty == Type::Str => return Ok(Own::Borrowed(Vec::new(), None)),
            ExprKind::Let { var, init, body, .. } => {
                let v = *var;
                self.walk(init)?;
                self.reuse(init);
                let o = self.walk(body)?;
                if let Own::Borrowed(s, _) = &o {
                    if s.contains(&v) {
                        self.make_fresh(body, o.clone(), false)?;
                        return Ok(Own::Fresh);
                    }
                }
                return Ok(o);
            }
            ExprKind::CheckSize(a, b, body) => {
                self.walk(a)?;
                self.walk(b)?;
                return self.walk(body);
            }
            // views: of the memory of their source (D95, D97); a slice or
            // the bytes of a string (D303)
            ExprKind::Window(..) | ExprKind::Index(..) | ExprKind::Slice(..) if heap && view => {
                let mut first = None;
                for c in e.children_mut() {
                    let o = self.walk(c)?;
                    first.get_or_insert(o);
                }
                return Ok(match first {
                    Some(Own::Borrowed(s, _)) => Own::Borrowed(s, None),
                    Some(Own::Fresh) => Own::Fresh,
                    _ => Own::Scalar,
                });
            }
            // the text of a replacement by a regular expression at its last
            // use, owned and without alias: given to it (D315), which writes
            // its result there when each match is replaced by as many bytes,
            // and frees it otherwise
            ExprKind::StrOp(crate::StrOp::Rx(crate::RxOp::Replace | crate::RxOp::ReplaceAll), es) => {
                let mut first = None;
                for c in es.iter_mut() {
                    let o = self.walk(c)?;
                    first.get_or_insert(o);
                }
                if let (Some(Own::Borrowed(_, Some(i))), ExprKind::Var(v, u)) = (&first, &mut es[0].kind) {
                    let w = *v as usize;
                    if self.info.candidates.contains(i) && self.info.owned[w] && self.info.alias[w].is_none() {
                        *u = Use::Move;
                        self.info.moved[w] = true;
                    }
                }
                return Ok(Own::Fresh);
            }
            // the bytes of `stringOf(b)` at their last use, owned and without
            // alias: given to it, the text taking their block once they are
            // checked as UTF-8, without a copy (D399)
            ExprKind::StrOp(crate::StrOp::FromBytes, es) if heap && matches!(es[0].kind, ExprKind::Var(..)) => {
                let o = self.walk(&mut es[0])?;
                if let (Own::Borrowed(_, Some(i)), ExprKind::Var(v, u)) = (&o, &mut es[0].kind) {
                    let w = *v as usize;
                    if self.info.candidates.contains(i) && self.info.owned[w] && self.info.alias[w].is_none() {
                        *u = Use::Move;
                        self.info.moved[w] = true;
                    }
                }
                return Ok(Own::Fresh);
            }
            // the left text of `a + b` (a chain, a template) at its last use,
            // owned and without alias: given to it (D351), which writes the
            // rest after it in its block when the block has room
            ExprKind::StrOp(crate::StrOp::Concat, es) if heap => {
                let mut first = None;
                for c in es.iter_mut() {
                    let o = self.walk(c)?;
                    first.get_or_insert(o);
                }
                if let (Some(Own::Borrowed(_, Some(i))), ExprKind::Var(v, u)) = (&first, &mut es[0].kind) {
                    let w = *v as usize;
                    if self.info.candidates.contains(i) && self.info.owned[w] && self.info.alias[w].is_none() {
                        *u = Use::Move;
                        self.info.moved[w] = true;
                    }
                }
                return Ok(Own::Fresh);
            }
            ExprKind::StrOp(..) if str_part => {
                let mut first = None;
                for c in e.children_mut() {
                    let o = self.walk(c)?;
                    first.get_or_insert(o);
                }
                return Ok(match first {
                    Some(Own::Borrowed(s, _)) => Own::Borrowed(s, None),
                    Some(Own::Fresh) => Own::Fresh,
                    _ => Own::Scalar,
                });
            }
            // a node takes its tuple (D103)
            ExprKind::Box(x) => {
                let o = self.walk(x)?;
                if matches!(o, Own::Borrowed(..)) {
                    self.make_fresh(x, o, false)?;
                }
                return Ok(Own::Fresh);
            }
            ExprKind::Proj(x, _) | ExprKind::Unbox(x) => {
                let o = self.walk(x)?;
                return Ok(match o {
                    Own::Borrowed(s, _) if heap => Own::Borrowed(s, None),
                    _ if heap => Own::Fresh,
                    _ => Own::Scalar,
                });
            }
            ExprKind::If(c, t, f) => {
                self.walk(c)?;
                let ot = self.walk(t)?;
                let of = self.walk(f)?;
                if !heap {
                    return Ok(Own::Scalar);
                }
                return Ok(match (ot, of) {
                    (Own::Borrowed(mut a, ta), Own::Borrowed(b, tb)) => {
                        self.branch_owns.insert(&**t as *const Expr, Own::Borrowed(a.clone(), ta));
                        self.branch_owns.insert(&**f as *const Expr, Own::Borrowed(b.clone(), tb));
                        a.extend(b);
                        a.sort();
                        a.dedup();
                        Own::Borrowed(a, None)
                    }
                    (ot, of) => {
                        if matches!(ot, Own::Borrowed(..)) {
                            self.make_fresh(t, ot, false)?;
                        }
                        if matches!(of, Own::Borrowed(..)) {
                            self.make_fresh(f, of, false)?;
                        }
                        Own::Fresh
                    }
                });
            }
            ExprKind::Update { target, index, value, require_unique } => {
                let ru = *require_unique;
                self.walk(index)?;
                // the new element is taken by the array
                let ov = self.walk(value)?;
                if matches!(ov, Own::Borrowed(..)) {
                    self.make_fresh(value, ov, false)?;
                }
                let o = self.walk(target)?;
                if matches!(o, Own::Borrowed(..)) {
                    self.make_fresh(target, o, ru)?;
                }
                return Ok(Own::Fresh);
            }
            ExprKind::Fold(x, init, lam) => {
                self.walk(x)?;
                let o = self.walk(init)?;
                if matches!(o, Own::Borrowed(..)) {
                    self.make_fresh(init, o, false)?;
                }
                let ob = self.walk(&mut lam.body)?;
                if matches!(ob, Own::Borrowed(..)) {
                    self.make_fresh(&mut lam.body, ob, false)?;
                }
                return Ok(if heap { Own::Fresh } else { Own::Scalar });
            }
            ExprKind::Bench(r, body) => {
                self.walk(r)?;
                let ob = self.walk(body)?;
                if matches!(ob, Own::Borrowed(..)) {
                    self.make_fresh(body, ob, false)?;
                }
                return Ok(Own::Fresh);
            }
            // the components of a tuple and the rows of a literal are taken
            // by the new value (D96)
            ExprKind::Tuple(es) | ExprKind::ArrayLit(es) => {
                for c in es.iter_mut() {
                    let o = self.walk(c)?;
                    if matches!(o, Own::Borrowed(..)) {
                        self.make_fresh(c, o, false)?;
                    }
                    self.reuse(c);
                }
                return Ok(if heap { Own::Fresh } else { Own::Scalar });
            }
            // `reduce` and `scan` of values that own memory (strings, nodes,
            // D134): each accumulator owns its value, so the neutral element
            // and every result of the operator are new ones (a borrowed one
            // is copied); the operands stay borrowed
            ExprKind::Reduce(x, l, z) | ExprKind::Scan(x, l, z) if l.body.ty.has_heap() && !l.body.ty.is_array() => {
                self.walk(x)?;
                let oz = self.walk(z)?;
                if matches!(oz, Own::Borrowed(..)) {
                    self.make_fresh(z, oz, false)?;
                }
                let o = self.walk(&mut l.body)?;
                if matches!(o, Own::Borrowed(..)) {
                    self.make_fresh(&mut l.body, o, false)?;
                }
                return Ok(Own::Fresh);
            }
            // a `map` of scalars: an input it may write its result into
            // (D207), taken by its parent if the `map` is materialized there
            ExprKind::Map(ins, l) if e.ty.elem().is_scalar() && !may_panic(&l.body) => {
                let et = e.ty.elem().clone();
                let mut found = None;
                for (k, i) in ins.iter_mut().enumerate() {
                    let o = self.walk(i)?;
                    if found.is_some() {
                        continue;
                    }
                    if let (ExprKind::Var(v, _), Own::Borrowed(_, Some(idx))) = (&i.kind, &o) {
                        let v = *v as usize;
                        if i.ty.is_array()
                            && *i.ty.elem() == et
                            && self.info.candidates.contains(idx)
                            && self.info.owned[v]
                            && self.info.alias[v].is_none()
                        {
                            found = Some(k);
                        }
                    }
                }
                self.walk(&mut l.body)?;
                if let Some(k) = found {
                    self.reusable.insert(e as *const Expr, k);
                }
                return Ok(Own::Fresh);
            }
            // the results of `mapParallel` (D373): each new, moved into the
            // array (a borrowed one copied)
            ExprKind::Tasks(x, l) => {
                self.walk(x)?;
                let o = self.walk(&mut l.body)?;
                if matches!(o, Own::Borrowed(..)) {
                    self.make_fresh(&mut l.body, o, false)?;
                }
                return Ok(Own::Fresh);
            }
            // the elements of a `map` that hold nodes (not rows, D126): a
            // borrowed one is copied, a node gets one owner more (D130)
            ExprKind::Map(ins, l) if e.ty.elem().has_heap() && !e.ty.elem().is_array() => {
                for i in ins.iter_mut() {
                    self.walk(i)?;
                }
                let o = self.walk(&mut l.body)?;
                if matches!(o, Own::Borrowed(..)) {
                    self.make_fresh(&mut l.body, o, false)?;
                }
                return Ok(Own::Fresh);
            }
            _ => {}
        }
        for c in e.children_mut() {
            self.walk(c)?;
        }
        Ok(if heap { Own::Fresh } else { Own::Scalar })
    }

    /// A `map` materialized here (a component, the value of a `let`) that may
    /// write into its input (D207): that input moved into it.
    fn reuse(&mut self, e: &mut Expr) {
        let Some(k) = self.reusable.remove(&(e as *const Expr)) else { return };
        if let ExprKind::Map(ins, _) = &mut e.kind {
            if let ExprKind::Var(v, u) = &mut ins[k].kind {
                *u = Use::Move;
                self.info.moved[*v as usize] = true;
            }
        }
    }

    /// Makes a borrowed value owned: moves the variable in tail position when
    /// allowed, otherwise copies.
    fn make_fresh(&mut self, e: &mut Expr, o: Own, require_unique: bool) -> R<()> {
        // an `if` whose branches borrow: each branch made new on its own, a
        // move where its variable is used for the last time (D105, D138)
        {
            let tail = tail_var(e);
            if let ExprKind::If(_, t, f) = &mut tail.kind {
                let ot = self.branch_owns.get(&(&**t as *const Expr)).cloned();
                let of = self.branch_owns.get(&(&**f as *const Expr)).cloned();
                if let (Some(ot), Some(of)) = (ot, of) {
                    self.make_fresh(t, ot, require_unique)?;
                    self.make_fresh(f, of, require_unique)?;
                    return Ok(());
                }
            }
        }
        if let Own::Borrowed(vars, Some(i)) = &o {
            if self.info.candidates.contains(i) {
                let tail = tail_var(e);
                let info = &*self.info;
                let ok = matches!(tail.kind, ExprKind::Var(v, _) if info.owned[v as usize] && info.alias[v as usize].is_none());
                if let (true, ExprKind::Var(v, u)) = (ok, &mut tail.kind) {
                    *u = Use::Move;
                    self.info.moved[*v as usize] = true;
                    let _ = vars;
                    return Ok(());
                }
            }
        }
        if require_unique {
            let span = e.span;
            let why = self.copy_reason(e, &o);
            return Err(Diagnostic::new(span, format!("requireUnique not satisfied (line {}): {}", span.line, why)));
        }
        let why = self.copy_reason(e, &o);
        self.copy_reasons.push((e.span, why));
        let inner = take(e);
        let ty = inner.ty.clone();
        let sp = inner.span;
        *e = Expr::new(ExprKind::Copy(Box::new(inner)), ty, sp);
        Ok(())
    }

    /// Why a borrowed value must be copied (the memory report, the error of
    /// `requireUnique`).
    fn copy_reason(&self, e: &mut Expr, o: &Own) -> String {
        let (vars, idx) = match o {
            Own::Borrowed(v, i) => (v.clone(), *i),
            _ => (vec![], None),
        };
        let name = |v: VarId| self.vars[v as usize].name.clone();
        let view = !matches!(tail_var(e).kind, ExprKind::Var(..));
        for v in &vars {
            let uses = &self.info.uses[*v as usize];
            // borrowed (an input, a parameter): copied whatever comes next
            if !self.info.owned[*v as usize] && !view {
                return format!("`{}` is shared (it is an input or a part of another value)", name(*v));
            }
            if let Some(i) = idx {
                // a later use on the same path (not in the other branch of an
                // `if`, D105)
                let here = uses.iter().find(|(j, _, _)| *j == i).map(|(_, r, _)| *r);
                let top = self.info.def_region[*v as usize];
                let later = uses.iter().find(|(j, r, _)| {
                    *j > i && !here.map(|h| self.info.exclusive(h, *r, top)).unwrap_or(false)
                });
                if let Some((_, _, sp)) = later {
                    return format!("`{}` is used again at line {}", name(*v), sp.line);
                }
            }
            if view {
                return format!(
                    "a part of `{}` (a slice, a window, a row or a component) is kept as a value of its own",
                    name(*v)
                );
            }
            if !self.info.owned[*v as usize] {
                return format!("`{}` is shared (it is an input or a part of another value)", name(*v));
            }
            return format!("`{}` is defined outside this loop or branch, so it is used more than once", name(*v));
        }
        "the value is borrowed".into()
    }
}

/// `e` may panic: a check of an index, of a size, a division, a call (D207:
/// a `map` that writes into its input must not stop half done, its replays
/// reading elements already written).
fn may_panic(e: &Expr) -> bool {
    use ExprKind::*;
    match &e.kind {
        Index(..) | CheckSize(..) | CheckNonNegative(..) | Call(..) | Slice(..) | Window(..) | Update { .. } => true,
        Binary(BinOp::Div | BinOp::Rem, a, _) if !a.ty.is_float() => true,
        _ => e.children().into_iter().any(may_panic),
    }
}

fn tail_var(e: &mut Expr) -> &mut Expr {
    match e.kind {
        ExprKind::Let { .. } | ExprKind::CheckSize(..) | ExprKind::CheckNonNegative(..) => {}
        _ => return e,
    }
    match &mut e.kind {
        ExprKind::Let { body, .. } => tail_var(body),
        ExprKind::CheckSize(_, _, b) | ExprKind::CheckNonNegative(_, b) => tail_var(b),
        _ => unreachable!(),
    }
}

/// `v` is moved or freed on every path through `e`; a branch of an `if`
/// that does not consume `v` while the other does gets a `Free` of `v` at
/// its end (D105). A move inside a lambda body is never of `v` (not a
/// candidate).
fn consumes(e: &mut Expr, v: VarId) -> bool {
    match &mut e.kind {
        ExprKind::Var(w, Use::Move) => *w == v,
        ExprKind::Free(w, body) => *w == v || consumes(body, v),
        ExprKind::If(c, t, f) => {
            if consumes(c, v) {
                return true;
            }
            let ct = consumes(t, v);
            let cf = consumes(f, v);
            if ct != cf {
                let other = if ct { f } else { t };
                let inner = take(other);
                let (ty, sp) = (inner.ty.clone(), inner.span);
                **other = Expr::new(ExprKind::Free(v, Box::new(inner)), ty, sp);
            }
            ct || cf
        }
        _ => {
            let lam = lambda_body_index(e);
            let mut any = false;
            for (i, c) in e.children_mut().into_iter().enumerate() {
                if Some(i) != lam && consumes(c, v) {
                    any = true;
                }
            }
            any
        }
    }
}

fn set_drops(e: &mut Expr, info: &Info) {
    if let ExprKind::Let { var, drop, body, .. } = &mut e.kind {
        let v = *var as usize;
        *drop = info.owned[v] && !(info.moved[v] && consumes(body, *var));
    }
    match &mut e.kind {
        ExprKind::Fold(_, _, l) => {
            let p = l.params[0] as usize;
            l.drop_params[0] = info.owned[p] && !(info.moved[p] && consumes(&mut l.body, l.params[0]));
        }
        _ => {}
    }
    for c in e.children_mut() {
        set_drops(c, info);
    }
}

/// A tuple that is new memory: a call, an action, a tuple literal, the
/// result of a `fold` (D164), an `if` whose two branches are such tuples
/// (D504: `f == 1 ? build(...) : [f1, -1]`, a field of the decoder of a
/// record, was borrowed from the tuple and copied at each member).
fn new_tuple(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Call(..) | ExprKind::Action(..) | ExprKind::Tuple(_) | ExprKind::Fold(..) => true,
        ExprKind::Let { body, .. } | ExprKind::CheckSize(_, _, body) | ExprKind::CheckNonNegative(_, body) => new_tuple(body),
        ExprKind::If(_, t, f) => new_tuple(t) && new_tuple(f),
        _ => false,
    }
}

fn reads_var(e: &Expr, v: VarId) -> bool {
    matches!(e.kind, ExprKind::Var(w, _) if w == v) || e.children().into_iter().any(|c| reads_var(c, v))
}

/// The lets of projections of `r` right at the top of `e` (`let a = proj 0
/// r in let b = proj 1 r in ...`): marks the components bound; the rest,
/// or None if a component is bound twice.
fn proj_chain<'a>(e: &'a Expr, r: VarId, bound: &mut [bool]) -> Option<&'a Expr> {
    if let ExprKind::Let { init, body, .. } = &e.kind {
        if let ExprKind::Proj(x, k) = &init.kind {
            if matches!(x.kind, ExprKind::Var(w, _) if w == r) {
                if bound[*k as usize] {
                    return None;
                }
                bound[*k as usize] = true;
                return proj_chain(body, r, bound);
            }
        }
    }
    Some(e)
}

fn rewrite_takes(e: &mut Expr, r: VarId) {
    if let ExprKind::Let { init, body, .. } = &mut e.kind {
        if let ExprKind::Proj(x, k) = &init.kind {
            if matches!(x.kind, ExprKind::Var(w, _) if w == r) {
                init.kind = ExprKind::Take(x.clone(), *k);
                rewrite_takes(body, r);
            }
        }
    }
}

/// `let r = <new tuple> in let a = proj 0 r in let b = proj 1 r in rest`,
/// with every component that owns memory bound once and `rest` not reading
/// `r` (`let (a, b) = f(x)`): the components are taken out of `r` (`Take`,
/// D138), so that each one is owned on its own, moved without a copy or
/// freed, and `r` is not freed as a whole. Returns the tuples split.
fn split_tuples(e: &mut Expr, out: &mut Vec<VarId>) {
    for c in e.children_mut() {
        split_tuples(c, out);
    }
    // the accumulator of a `fold`, a tuple it owns, taken apart at the top
    // of the body and not read otherwise: its components are owned, so that
    // `const [a, b] = acc; ... a.with(i, v)` updates in place (D164)
    if let ExprKind::Fold(_, _, l) = &mut e.kind {
        let r = l.params[0];
        if let Type::Tuple(ts) = &l.body.ty {
            if l.body.ty.has_heap() {
                let mut bound = vec![false; ts.len()];
                let ok = match proj_chain(&l.body, r, &mut bound) {
                    Some(rest) => ts.iter().enumerate().all(|(k, t)| !t.has_heap() || bound[k]) && !reads_var(rest, r),
                    None => false,
                };
                if ok {
                    rewrite_takes(&mut l.body, r);
                    out.push(r);
                }
            }
        }
        return;
    }
    let ExprKind::Let { var, init, body, .. } = &mut e.kind else { return };
    let r = *var;
    let Type::Tuple(ts) = &init.ty else { return };
    if !init.ty.has_heap() || !new_tuple(init) {
        return;
    }
    let mut bound = vec![false; ts.len()];
    let Some(rest) = proj_chain(body, r, &mut bound) else { return };
    if !ts.iter().enumerate().all(|(k, t)| !t.has_heap() || bound[k]) || reads_var(rest, r) {
        return;
    }
    rewrite_takes(body, r);
    out.push(r);
}

/// The ownership pass on one body. `owned`: a parameter it owns (`args` of
/// `main`); the parameters of a recursive function are borrowed. A result
/// that borrows is made new (copied), since the caller frees it (D102).
/// Returns whether `owned` is still owned at the end (not moved).
pub(crate) fn ownership_body(body: &mut Expr, vars: &[VarInfo], owned: Option<VarId>, copy_reasons: &mut Vec<(Span, String)>) -> R<bool> {
    ownership_body_with(body, vars, owned, &[], copy_reasons)
}

/// `ownership_body` with the parameters `params` owned too (D351).
fn ownership_body_with(body: &mut Expr, vars: &[VarInfo], owned: Option<VarId>, params: &[VarId], copy_reasons: &mut Vec<(Span, String)>) -> R<bool> {
    let n = vars.len();
    let mut info = Info {
        alias: vec![None; n],
        owned: vec![false; n],
        def_region: vec![0; n],
        uses: vec![Vec::new(); n],
        candidates: HashSet::new(),
        moved: vec![false; n],
        region_parent: vec![0],
        region_side: vec![None],
    };
    if let Some(v) = owned {
        info.owned[v as usize] = true;
    }
    for &v in params {
        info.owned[v as usize] = true;
    }
    let mut split = Vec::new();
    split_tuples(body, &mut split);
    {
        let mut w = Walker { info: &mut info, index: 0, next_region: 0, next_if: 0, user: body.span };
        w.scan(body, 0);
    }
    // a tuple split into its components owns nothing more (D138)
    for r in split {
        info.owned[r as usize] = false;
    }
    // candidates: the last use of an owned variable on its path (in its
    // region, or in the branches of `if`s below it: one per branch, D105)
    for v in 0..n {
        if !info.owned[v] || info.alias[v].is_some() {
            continue;
        }
        let cands: Vec<usize> =
            info.uses[v].iter().filter(|(i, r, _)| info.last_on_its_path(v, *i, *r)).map(|(i, _, _)| *i).collect();
        info.candidates.extend(cands);
    }
    {
        let mut rw =
            Rewriter { info: &mut info, vars, index: 0, copy_reasons: Vec::new(), branch_owns: HashMap::new(), reusable: HashMap::new() };
        let o = rw.walk(body)?;
        if matches!(o, Own::Borrowed(..)) && body.ty.has_heap() {
            rw.make_fresh(body, o, false)?;
        }
        copy_reasons.extend(rw.copy_reasons);
    }
    set_drops(body, &info);
    // an owned parameter moved on some paths: freed at the end of the
    // branches that do not move it (D105, D351)
    for &v in params {
        if info.moved[v as usize] {
            consumes(body, v);
        }
    }
    Ok(match owned {
        Some(v) => info.owned[v as usize] && !(info.moved[v as usize] && consumes(body, v)),
        None => false,
    })
}

/// The state of a loop (D206): a recursive function whose calls to itself
/// are all tail calls, and whose other results are all its tuple parameter
/// `p` itself (`if (n == 0) return b; return steps(advance(b), n - 1)`),
/// owns `p`: its body starts with `let p.k = take k p` for each field, reads
/// `p.k` for `p[k]` and gives back the tuple of the `p.k` for `p`. Each
/// field is then owned on its own: moved instead of copied at its last use,
/// freed when not moved. The borrowed form copies `p` once, as the result of
/// the loop; this form copies it once on entry (the generated code, for
/// `consumed`), and an iteration no longer copies its fields.
/// The calls of `e` to the function `fi`.
fn calls(e: &Expr, fi: u32) -> usize {
    let own = matches!(e.kind, ExprKind::Call(g, _) if g == fi) as usize;
    own + e.children().into_iter().map(|c| calls(c, fi)).sum::<usize>()
}

/// Every result of `e` is a tail call to `fi` or `pv` itself: their
/// numbers (D206).
fn results(e: &Expr, fi: u32, pv: VarId, tails: &mut usize, states: &mut usize) -> bool {
    match &e.kind {
        ExprKind::If(c, t, f) => calls(c, fi) == 0 && results(t, fi, pv, tails, states) && results(f, fi, pv, tails, states),
        ExprKind::Let { init, body, .. } => calls(init, fi) == 0 && results(body, fi, pv, tails, states),
        ExprKind::CheckSize(a, b, body) => calls(a, fi) + calls(b, fi) == 0 && results(body, fi, pv, tails, states),
        ExprKind::CheckNonNegative(k, body) => calls(k, fi) == 0 && results(body, fi, pv, tails, states),
        ExprKind::Call(g, args) if *g == fi => {
            *tails += 1;
            args.iter().all(|a| calls(a, fi) == 0)
        }
        ExprKind::Var(v, _) if *v == pv => {
            *states += 1;
            true
        }
        _ => false,
    }
}

/// The accumulators of the function `fi` (D351): its parameters that own
/// memory and are not tuples (those are the states of D206), in a function
/// whose calls to itself are all tail calls and that gives back such a
/// parameter itself as one of its results (`build(i + 1, acc + x)`, `return
/// acc`). Owned, the parameter is moved at its last use (into the `concat`
/// that grows it in place) and given back without a copy.
fn accumulators(p: &Program, fi: usize) -> Vec<usize> {
    let f = &p.fns[fi];
    let fu = fi as u32;
    // the calls of `e` to `fi` in its tail positions
    fn tail_self(e: &Expr, fi: u32) -> usize {
        match &e.kind {
            ExprKind::If(_, t, f) => tail_self(t, fi) + tail_self(f, fi),
            ExprKind::Let { body, .. } | ExprKind::CheckSize(_, _, body) | ExprKind::CheckNonNegative(_, body) => tail_self(body, fi),
            ExprKind::Call(g, args) if *g == fi => args.iter().all(|a| calls(a, fi) == 0) as usize,
            _ => 0,
        }
    }
    // a call to `fi` gives its parameter `k` another value than `pv`
    fn changes(e: &Expr, fi: u32, k: usize, pv: VarId) -> bool {
        if let ExprKind::Call(g, args) = &e.kind {
            if *g == fi && !matches!(args[k].kind, ExprKind::Var(v, _) if v == pv) {
                return true;
            }
        }
        e.children().into_iter().any(|c| changes(c, fi, k, pv))
    }
    let tails = tail_self(&f.body, fu);
    let all_tail = tails > 0 && calls(&f.body, fu) == tails;
    (0..f.params.len())
        .filter(|&k| {
            let pv = f.params[k];
            let ty = &p.vars[pv as usize].ty;
            // (the nodes of recursive types keep their own owners: counted,
            // not atomic inside a task, D175)
            if !all_tail || f.consumed.get(k).copied().unwrap_or(false) || !ty.has_heap() || ty.has_nodes() || matches!(ty, Type::Tuple(_)) {
                return false;
            }
            // an accumulator given back, or the state of a loop that each
            // turn may change (`serveLoop(s, next)`), its exits freeing it
            let (mut t, mut states) = (0, 0);
            (results(&f.body, fu, pv, &mut t, &mut states) && states > 0) || changes(&f.body, fu, k, pv)
        })
        .collect()
}

fn own_states(p: &mut Program) {
    // `pv` read only as `pv[k]`, or given back whole as a result
    fn fields_only(e: &Expr, pv: VarId, tail: bool) -> bool {
        match &e.kind {
            ExprKind::Proj(x, _) if matches!(x.kind, ExprKind::Var(v, _) if v == pv) => true,
            ExprKind::Var(v, _) if *v == pv => tail,
            ExprKind::If(c, t, f) => fields_only(c, pv, false) && fields_only(t, pv, tail) && fields_only(f, pv, tail),
            ExprKind::Let { init, body, .. } => fields_only(init, pv, false) && fields_only(body, pv, tail),
            ExprKind::CheckSize(a, b, body) => fields_only(a, pv, false) && fields_only(b, pv, false) && fields_only(body, pv, tail),
            ExprKind::CheckNonNegative(k, body) => fields_only(k, pv, false) && fields_only(body, pv, tail),
            _ => e.children().into_iter().all(|c| fields_only(c, pv, false)),
        }
    }
    fn rewrite(e: &mut Expr, pv: VarId, parts: &[(VarId, Type)]) {
        match &mut e.kind {
            ExprKind::Proj(x, k) if matches!(x.kind, ExprKind::Var(v, _) if v == pv) => {
                let (c, t) = parts[*k as usize].clone();
                *e = Expr::var(c, t, e.span);
            }
            ExprKind::Var(v, _) if *v == pv => {
                let es = parts.iter().map(|(c, t)| Expr::var(*c, t.clone(), e.span)).collect();
                *e = Expr::new(ExprKind::Tuple(es), e.ty.clone(), e.span);
            }
            _ => {
                for c in e.children_mut() {
                    rewrite(c, pv, parts);
                }
            }
        }
    }
    for fi in 0..p.fns.len() {
        for k in 0..p.fns[fi].params.len() {
            let pv = p.fns[fi].params[k];
            let ty = p.vars[pv as usize].ty.clone();
            let Type::Tuple(ts) = &ty else { continue };
            if !ty.has_heap() {
                continue;
            }
            let f = &p.fns[fi];
            let (mut tails, mut states) = (0, 0);
            let shape = results(&f.body, fi as u32, pv, &mut tails, &mut states) && calls(&f.body, fi as u32) == tails;
            if !shape || tails == 0 || states == 0 || !fields_only(&f.body, pv, true) {
                continue;
            }
            let name = p.vars[pv as usize].name.clone();
            let mut parts = Vec::new();
            for (j, t) in ts.iter().enumerate() {
                let c = p.vars.len() as VarId;
                p.vars.push(VarInfo { name: format!("{}.{}", name, j), ty: t.clone() });
                parts.push((c, t.clone()));
            }
            let f = &mut p.fns[fi];
            let span = f.body.span;
            let mut body = std::mem::replace(&mut f.body, Expr::new(ExprKind::Zero, Type::Unit, span));
            rewrite(&mut body, pv, &parts);
            for (j, (c, t)) in parts.iter().enumerate().rev() {
                let bty = body.ty.clone();
                let take = Expr::new(ExprKind::Take(Box::new(Expr::var(pv, ty.clone(), span)), j as u32), t.clone(), span);
                body = Expr::new(
                    ExprKind::Let { var: *c, init: Box::new(take), body: Box::new(body), stmt: false, drop: false },
                    bty,
                    span,
                );
            }
            f.body = body;
            f.consumed[k] = true;
        }
    }
}

pub fn ownership(p: &mut Program) -> R<()> {
    own_states(p);
    // `main` owns the array of its arguments (allocated by the runtime at
    // start-up), defined in the outermost region
    p.drop_args = ownership_body(&mut p.body, &p.vars, Some(p.args), &mut p.copy_reasons)?;
    for fi in 0..p.fns.len() {
        // its accumulators owned (D351), kept only when the memory check of
        // the function holds (each moved on every path); otherwise borrowed
        let acc = accumulators(p, fi);
        if !acc.is_empty() {
            let saved = p.fns[fi].body.clone();
            let params: Vec<VarId> = acc.iter().map(|&k| p.fns[fi].params[k]).collect();
            let mut reasons = Vec::new();
            let f = &mut p.fns[fi];
            if ownership_body_with(&mut f.body, &p.vars, None, &params, &mut reasons).is_ok() {
                for &k in &acc {
                    p.fns[fi].consumed[k] = true;
                }
                if crate::verify::verify_fn(p, &p.fns[fi]).is_ok() {
                    p.copy_reasons.extend(reasons);
                    continue;
                }
                for &k in &acc {
                    p.fns[fi].consumed[k] = false;
                }
            }
            p.fns[fi].body = saved;
        }
        let f = &mut p.fns[fi];
        ownership_body(&mut f.body, &p.vars, None, &mut p.copy_reasons)?;
    }
    take_payloads(p);
    Ok(())
}

/// Arrays whose texts are views (D555). `let y = map(t, (i) => copy(x[i]))`
/// (the gather of `sort` and `sortBy`, D488, or any `map` whose texts are
/// those of an array or a text that lives longer) made a block and a copy
/// of each text, freed with `y`. The copy is left out when nothing can free
/// or move the texts while `y` lives: `y` is never moved (so never given to
/// an owner, never taken apart) and the variables whose memory the texts
/// borrow are bound outside `y`'s `let` and neither moved, freed nor taken
/// apart inside it (their own `let`s end after it). `y` then owns its block
/// alone (`Program::views`): the generated code frees it without its texts.
/// The gather of a permutation of an array read for the last time keeps its
/// moves instead (D548: the elements taken, its block freed early).
pub fn borrow_elements(p: &mut Program) {
    let mut st = Elements::default();
    st.walk(&mut p.body);
    for f in p.fns.iter_mut() {
        st.walk(&mut f.body);
    }
    // the copies left out are not in the memory report
    for sp in st.removed {
        if let Some(k) = p.copy_reasons.iter().position(|(s, _)| *s == sp) {
            p.copy_reasons.remove(k);
        }
    }
    p.views = st.views;
}

#[derive(Default)]
struct Elements {
    /// The variables whose memory each variable borrows (a variable absent:
    /// its own).
    alias: HashMap<VarId, Vec<VarId>>,
    views: BTreeSet<VarId>,
    /// The places of the copies left out.
    removed: Vec<Span>,
    /// The arrays made by the gather of a permutation that moves (D548).
    keep: HashSet<VarId>,
}

impl Elements {
    fn base(&self, v: VarId) -> Vec<VarId> {
        self.alias.get(&v).cloned().unwrap_or_else(|| vec![v])
    }

    fn walk(&mut self, e: &mut Expr) {
        hoist_text(e);
        if let ExprKind::Let { var, init, body, drop, .. } = &mut e.kind {
            let y = *var;
            if init.ty.has_heap() {
                if let Some(b) = borrowed_from(init, &|v| self.base(v)) {
                    self.alias.insert(y, b);
                }
            }
            // its elements moved by the gather of a permutation (D548)
            if *drop && !self.views.contains(&y) && matches!(&init.ty, Type::Array(t, _) if !t.is_array() && t.has_heap()) {
                if let Some(g) = permutation_gather(body, y) {
                    self.keep.insert(g);
                }
            }
            if *drop && !self.keep.contains(&y) {
                self.try_view(y, init, body);
            }
        }
        // the parameters of a lambda that are elements of a borrowed input
        // borrow what it borrows
        let inputs: Vec<(VarId, Option<Vec<VarId>>)> = match &e.kind {
            ExprKind::Map(ins, l) => ins.iter().zip(&l.params).map(|(i, p)| (*p, borrowed_from(i, &|v| self.base(v)))).collect(),
            ExprKind::Filter(x, l) | ExprKind::Tasks(x, l) => vec![(l.params[0], borrowed_from(x, &|v| self.base(v)))],
            ExprKind::Fold(x, _, l) => vec![(l.params[1], borrowed_from(x, &|v| self.base(v)))],
            _ => Vec::new(),
        };
        for (p, b) in inputs {
            if let Some(b) = b {
                self.alias.insert(p, b);
            }
        }
        for c in e.children_mut() {
            self.walk(c);
        }
    }

    /// `let y = init in body`: `init` a `map` of texts each copied from
    /// memory that lives longer than `y`; the copies left out.
    fn try_view(&mut self, y: VarId, init: &mut Expr, body: &Expr) {
        // the lines or the parts of a text that lives longer (made as views
        // of it by the runtime)
        if let ExprKind::StrOp(StrOp::Lines | StrOp::Split, es) = &init.kind {
            let Some(roots) = borrowed_from(&es[0], &|v| self.base(v)) else { return };
            if init.ty.elem() == &Type::Str && !moves_var(body, y) && !takes_var(body, y) && read_before_consumed(body, y, &roots) {
                self.views.insert(y);
            }
            return;
        }
        let ExprKind::Map(ins, l) = &init.kind else { return };
        if *init.ty.elem() != Type::Str {
            return;
        }
        let ExprKind::Copy(inner) = &l.body.kind else { return };
        // the parameters of the `map` borrow what its inputs borrow
        let mut params = HashMap::new();
        for (i, p) in ins.iter().zip(&l.params) {
            if let Some(b) = borrowed_from(i, &|v| self.base(v)) {
                params.insert(*p, b);
            }
        }
        let Some(roots) = borrowed_from(inner, &|v| params.get(&v).cloned().unwrap_or_else(|| self.base(v))) else { return };
        let mut inside = HashSet::new();
        bound_in(init, &mut inside);
        let in_init = |v: VarId| moves_var(init, v) || frees_var(init, v) || takes_var(init, v);
        if moves_var(body, y) || takes_var(body, y) || roots.iter().any(|r| inside.contains(r) || in_init(*r)) || !read_before_consumed(body, y, &roots) {
            return;
        }
        let ExprKind::Map(_, l) = &mut init.kind else { unreachable!() };
        let ExprKind::Copy(inner) = std::mem::replace(&mut l.body.kind, ExprKind::Zero) else { unreachable!() };
        self.removed.push(l.body.span);
        l.body.kind = inner.kind;
        self.views.insert(y);
    }
}

/// `let y = (let t = i in lines(t)) in b` (`readStdinLines()`, a `lines`
/// or `split` of a temporary text, its separator bound first), `y` never
/// moved nor taken apart in `b`: the `let`s of its value taken out of it,
/// `let t = i in let y = lines(t) in b`, so that `t` lives as long as `y`,
/// whose lines are then views of it (D555). A `let` that frees a value the
/// text does not come from stays (its value would live longer for nothing).
fn hoist_text(e: &mut Expr) {
    let ExprKind::Let { var: y, init, body, drop: true, .. } = &e.kind else { return };
    // the chain of `let`s of the value, down to a `lines` or a `split`
    let mut lets = Vec::new();
    let mut tail = &**init;
    while let ExprKind::Let { var, body: b, stmt: false, drop, .. } = &tail.kind {
        lets.push((*var, *drop));
        tail = b;
    }
    let ExprKind::StrOp(StrOp::Lines | StrOp::Split, es) = &tail.kind else { return };
    if lets.is_empty() || *tail.ty.elem() != Type::Str || moves_var(body, *y) || takes_var(body, *y) {
        return;
    }
    let text_vars: HashSet<VarId> = lets.iter().map(|(v, _)| *v).collect();
    let roots = borrowed_from(&es[0], &|v| vec![v]).unwrap_or_default();
    if crate::blocks::mentions(body, &text_vars) || lets.iter().any(|(v, d)| *d && !roots.contains(v)) {
        return;
    }
    let (ty, span, reassoc) = (e.ty.clone(), e.span, e.reassoc);
    let ExprKind::Let { var: y, init, body, stmt, .. } = std::mem::replace(&mut e.kind, ExprKind::Zero) else { unreachable!() };
    // the `let`s of the value, outermost first, then the `lines` or `split`
    let mut outer = Vec::new();
    let mut cur = *init;
    while let ExprKind::Let { var, init: i, body: b, drop, .. } = cur.kind {
        outer.push((var, i, drop));
        cur = *b;
    }
    let mut out = Expr { kind: ExprKind::Let { var: y, init: Box::new(cur), body, stmt, drop: true }, ty: ty.clone(), span, reassoc };
    for (var, i, drop) in outer.into_iter().rev() {
        out = Expr { kind: ExprKind::Let { var, init: i, body: Box::new(out), stmt: false, drop }, ty: ty.clone(), span, reassoc };
    }
    *e = out;
}

/// In `body`, evaluated after the value of `y`, every read of `y` and of the
/// values that borrow from it (bound to variables, or the elements a lambda
/// reads) comes before any move, free or taking apart of the variables
/// `roots`: the texts of `y` are read only while they live (its block,
/// freed at the end of its scope, never reads them).
fn read_before_consumed(body: &Expr, y: VarId, roots: &[VarId]) -> bool {
    struct Order<'a> {
        deps: HashSet<VarId>,
        roots: &'a [VarId],
        at: usize,
        last_read: usize,
        first_consumed: Option<usize>,
    }
    impl Order<'_> {
        fn borrows_y(&self, e: &Expr) -> bool {
            borrowed_from(e, &|v| vec![v]).is_some_and(|b| b.iter().any(|v| self.deps.contains(v)))
        }
        fn consumed(&mut self) {
            self.first_consumed.get_or_insert(self.at);
        }
        fn walk(&mut self, e: &Expr) {
            match &e.kind {
                ExprKind::Var(v, u) => {
                    self.at += 1;
                    if self.deps.contains(v) {
                        self.last_read = self.at;
                    }
                    if *u == Use::Move && self.roots.contains(v) {
                        self.consumed();
                    }
                    return;
                }
                ExprKind::Take(x, _) if matches!(x.kind, ExprKind::Var(v, _) if self.roots.contains(&v)) => {
                    self.at += 1;
                    self.consumed();
                    return;
                }
                ExprKind::Free(v, b) => {
                    self.walk(b);
                    self.at += 1;
                    if self.roots.contains(v) {
                        self.consumed();
                    }
                    return;
                }
                ExprKind::Let { var, init, body, .. } => {
                    self.walk(init);
                    if self.borrows_y(init) {
                        self.deps.insert(*var);
                    }
                    self.walk(body);
                    return;
                }
                _ => {}
            }
            let params: Vec<VarId> = match &e.kind {
                ExprKind::Map(ins, l) => ins.iter().zip(&l.params).filter(|(i, _)| self.borrows_y(i)).map(|(_, p)| *p).collect(),
                ExprKind::Filter(x, l) | ExprKind::Tasks(x, l) if self.borrows_y(x) => vec![l.params[0]],
                ExprKind::Fold(x, _, l) if self.borrows_y(x) => vec![l.params[1]],
                _ => Vec::new(),
            };
            self.deps.extend(params);
            for c in e.children() {
                self.walk(c);
            }
        }
    }
    let mut o = Order { deps: HashSet::from([y]), roots, at: 0, last_read: 0, first_consumed: None };
    o.walk(body);
    o.first_consumed.is_none_or(|c| c > o.last_read)
}

/// `v` is moved somewhere in `e`, lambdas included.
fn moves_var(e: &Expr, v: VarId) -> bool {
    matches!(e.kind, ExprKind::Var(w, Use::Move) if w == v) || e.children().into_iter().any(|c| moves_var(c, v))
}

/// `e` frees `v` somewhere (`Free`, D105).
fn frees_var(e: &Expr, v: VarId) -> bool {
    matches!(e.kind, ExprKind::Free(w, _) if w == v) || e.children().into_iter().any(|c| frees_var(c, v))
}

/// A part of `v` is taken somewhere in `e` (`Take`, D138).
fn takes_var(e: &Expr, v: VarId) -> bool {
    matches!(&e.kind, ExprKind::Take(x, _) if matches!(x.kind, ExprKind::Var(w, _) if w == v)) || e.children().into_iter().any(|c| takes_var(c, v))
}

/// The body of the binding of the owned array `x` reads it only in the
/// gather of a permutation of its indices (D548): `let t = argsort(x)` (or
/// the `argsort` of a `map` of `x`, `sortBy`), then `let y = map(t, (i) =>
/// copy(x[i]))`, then nothing more of `x`. Each element is taken once (a
/// permutation): moved rather than copied, and `x` gives back its block
/// alone. The variable `y`.
pub fn permutation_gather(e: &Expr, x: VarId) -> Option<VarId> {
    let is_x = |e: &Expr| matches!(e.kind, ExprKind::Var(v, _) if v == x);
    let map_of_x = |e: &Expr| matches!(&e.kind, ExprKind::Map(ins, _) if ins.len() == 1 && is_x(&ins[0]));
    let ExprKind::Let { var: t, init, body, .. } = &e.kind else { return None };
    let perm = match &init.kind {
        ExprKind::StrOp(StrOp::ArgSort, a) => a.len() == 1 && (is_x(&a[0]) || map_of_x(&a[0])),
        ExprKind::Let { var: k, init: keys, body: sorted, .. } => {
            map_of_x(keys) && matches!(&sorted.kind, ExprKind::StrOp(StrOp::ArgSort, a) if a.len() == 1 && matches!(a[0].kind, ExprKind::Var(v, _) if v == *k))
        }
        _ => false,
    };
    if !perm {
        return None;
    }
    let ExprKind::Let { var: y, init: gather, body: rest, .. } = &body.kind else { return None };
    let ExprKind::Map(ins, l) = &gather.kind else { return None };
    let takes = ins.len() == 1
        && matches!(ins[0].kind, ExprKind::Var(v, _) if v == *t)
        && l.params.len() == 1
        && matches!(&l.body.kind, ExprKind::Copy(c) if matches!(&c.kind, ExprKind::Index(a, i) if is_x(a) && matches!(i.kind, ExprKind::Var(v, _) if v == l.params[0])));
    (takes && !crate::blocks::mentions(rest, &HashSet::from([x])) && !frees_var(rest, x)).then_some(*y)
}

/// The payload of a `match` on a temporary taken out of it (D367): `let
/// drop m = f(x) in if (proj 0 m == 1) copy (proj 1 m) else e` (`f(x) ??
/// d`, `match (f(x)) { Ok(v) => v, ... }`) copied the payload, then freed
/// the temporary after the `if`. The branch that gives the payload now
/// takes it (`take 1 m`, the other parts that own memory taken and freed:
/// they are zero there), the others free `m` (`Free(m, e)`), and `m` is no
/// longer freed after the `if`. A function whose memory check fails after
/// the rewrite is kept as it was; `main` too.
fn take_payloads(p: &mut Program) {
    for fi in 0..p.fns.len() {
        let saved = p.fns[fi].body.clone();
        if rewrite_payloads(&mut p.fns[fi].body, &mut p.vars) && crate::verify::verify_fn(p, &p.fns[fi]).is_err() {
            p.fns[fi].body = saved;
        }
    }
    let saved = p.body.clone();
    if rewrite_payloads(&mut p.body, &mut p.vars) && crate::verify::verify(p).is_err() {
        p.body = saved;
    }
}

/// `take_payloads` in `e`: whether something changed.
fn rewrite_payloads(e: &mut Expr, vars: &mut Vec<VarInfo>) -> bool {
    let mut changed = false;
    for c in e.children_mut() {
        changed |= rewrite_payloads(c, vars);
    }
    let span = e.span;
    let ExprKind::Let { var, init, body, stmt, drop } = &mut e.kind else { return changed };
    let Type::Tuple(ts) = &init.ty else { return changed };
    if !*drop || *stmt || !matches!(body.kind, ExprKind::If(..)) {
        return changed;
    }
    let heap: Vec<usize> = ts.iter().enumerate().filter(|(_, t)| t.has_heap()).map(|(k, _)| k).collect();
    if heap.is_empty() {
        return changed;
    }
    let (m, tt, ts) = (*var, init.ty.clone(), ts.clone());
    let b = std::mem::replace(&mut **body, Expr::new(ExprKind::Zero, Type::Unit, span));
    let (nb, took) = payload_branches(b, m, &tt, &ts, &heap, vars);
    if took {
        **body = nb;
        *drop = false;
        changed = true;
    } else {
        // nothing taken: as it was (the `Free`s of the branches undone)
        **body = undo_frees(nb, m);
    }
    changed
}

/// The component of `m` that `e` reads (`proj k m`, or `let x = proj k m
/// in x`), if that is all `e` is.
fn payload_of(e: &Expr, m: VarId) -> Option<u32> {
    match &e.kind {
        ExprKind::Proj(x, k) if matches!(x.kind, ExprKind::Var(w, _) if w == m) => Some(*k),
        ExprKind::Let { var, init, body, .. } if matches!(body.kind, ExprKind::Var(w, _) if w == *var) => payload_of(init, m),
        _ => None,
    }
}

/// `m` is read in `e` only by projections of scalar components.
fn scalar_reads(e: &Expr, m: VarId, ts: &[Type]) -> bool {
    match &e.kind {
        ExprKind::Proj(x, k) if matches!(x.kind, ExprKind::Var(w, _) if w == m) => !ts[*k as usize].has_heap(),
        ExprKind::Var(w, _) => *w != m,
        _ => e.children().into_iter().all(|c| scalar_reads(c, m, ts)),
    }
}

/// The branches of the `if`s at the top of the body of `let m`: the copy of
/// a payload taken, the others ending with `m` freed; whether one takes.
fn payload_branches(e: Expr, m: VarId, tt: &Type, ts: &[Type], heap: &[usize], vars: &mut Vec<VarInfo>) -> (Expr, bool) {
    let (ty, span, reassoc) = (e.ty.clone(), e.span, e.reassoc);
    match e.kind {
        ExprKind::If(c, t, f) if scalar_reads(&c, m, ts) => {
            let (t2, a) = payload_branches(*t, m, tt, ts, heap, vars);
            let (f2, b) = payload_branches(*f, m, tt, ts, heap, vars);
            (Expr { kind: ExprKind::If(c, Box::new(t2), Box::new(f2)), ty, span, reassoc }, a || b)
        }
        ExprKind::Copy(inner) if payload_of(&inner, m).is_some_and(|k| heap.contains(&(k as usize))) => {
            let k = payload_of(&inner, m).unwrap();
            let take = |j: u32| Expr::new(ExprKind::Take(Box::new(Expr::var(m, tt.clone(), span)), j), ts[j as usize].clone(), span);
            let mut out = take(k);
            for &j in heap.iter().filter(|&&j| j != k as usize) {
                let v = vars.len() as VarId;
                vars.push(VarInfo { name: "unused".into(), ty: ts[j].clone() });
                out = Expr::new(ExprKind::Let { var: v, init: Box::new(take(j as u32)), body: Box::new(out), stmt: false, drop: true }, ty.clone(), span);
            }
            (out, true)
        }
        kind => (Expr::new(ExprKind::Free(m, Box::new(Expr { kind, ty: ty.clone(), span, reassoc })), ty, span), false),
    }
}

/// The `Free(m, e)` that `payload_branches` put at the ends of the branches
/// of the `if`s at the top of `e`, removed.
fn undo_frees(e: Expr, m: VarId) -> Expr {
    let (ty, span, reassoc) = (e.ty.clone(), e.span, e.reassoc);
    match e.kind {
        ExprKind::If(c, t, f) => Expr { kind: ExprKind::If(c, Box::new(undo_frees(*t, m)), Box::new(undo_frees(*f, m))), ty, span, reassoc },
        ExprKind::Free(v, body) if v == m => *body,
        kind => Expr { kind, ty, span, reassoc },
    }
}
