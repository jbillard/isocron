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

use std::collections::{HashMap, HashSet};

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
        ExprKind::Reduce(..) | ExprKind::Fold(..) | ExprKind::Scan(..) | ExprKind::Filter(..) | ExprKind::Index(..) | ExprKind::Len(_) => vec![0],
        // the source of a slice: a producer there is read from `a` to `b`
        ExprKind::Slice(..) => vec![0],
        _ => vec![],
    }
}

pub fn is_pure(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Action(..) | ExprKind::Bench(..) => false,
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
        ExprKind::Filter(..) => Some(1),
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

/// The passes before the ownership pass, on one body (`main` or a recursive
/// function).
pub(crate) fn optimize_body(body: &mut Expr, vars: &mut Vec<VarInfo>) {
    copy_prop(body, &mut HashMap::new());
    producer_lengths(body);
    inline_zips(body, vars);
    fuse_fixpoint(body);
    bind_temporaries(body, vars);
    fuse_fixpoint(body);
    horizontal(body, vars);
    masked_reductions(body, vars);
    // a value read by several `reduce`s now grouped (F3) is read once: it
    // can be fused into the group
    fuse_fixpoint(body);
    let pending = place_streams(body, vars, Mode::Evaluated);
    debug_assert!(pending.is_empty());
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
/// the loop), the temporaries freed before the jump.
pub struct TailCall {
    pub moved: Vec<(usize, VarId)>,
    pub free_first: Vec<VarId>,
}

/// The direct tail calls of function `fi` that can be jumps: each array
/// argument is the same parameter, or a temporary freed after the call
/// (then moved into the loop); the other temporaries freed after the call
/// are freed before the jump.
pub fn tail_calls(fi: u32, f: &Function) -> HashMap<*const Expr, TailCall> {
    fn walk(e: &Expr, fi: u32, f: &Function, pending: &mut Vec<VarId>, out: &mut HashMap<*const Expr, TailCall>) {
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
            ExprKind::Call(g, args) if *g == fi => {
                let mut moved = Vec::new();
                for (k, a) in args.iter().enumerate() {
                    if !a.ty.has_heap() {
                        continue;
                    }
                    match a.kind {
                        ExprKind::Var(v, _) if v == f.params[k] => {}
                        ExprKind::Var(v, _) if pending.contains(&v) && !moved.iter().any(|(_, w)| *w == v) => moved.push((k, v)),
                        _ => return,
                    }
                }
                let free_first = pending.iter().copied().filter(|v| !moved.iter().any(|(_, w)| w == v)).collect();
                out.insert(e as *const Expr, TailCall { moved, free_first });
            }
            _ => {}
        }
    }
    let mut out = HashMap::new();
    walk(&f.body, fi, f, &mut Vec::new(), &mut out);
    out
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
        ExprKind::Map(_, l) | ExprKind::Reduce(_, l, _) | ExprKind::Fold(_, _, l) | ExprKind::Scan(_, l, _) | ExprKind::Filter(_, l) => {
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
        ExprKind::Map(_, l) | ExprKind::Reduce(_, l, _) | ExprKind::Fold(_, _, l) | ExprKind::Scan(_, l, _) | ExprKind::Filter(_, l) => {
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
        Type::Array(Box::new(tt.clone())),
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
    **x = Expr::new(ExprKind::Map(vec![src], lam), Type::Array(Box::new(rt)), sp);
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

// ---------------------------------------------------------------------
// zip, a view (rule F4, D97)

/// The `map` that `zip` is lowered to: its lambda builds the tuple of its
/// parameters, in order.
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
        if !e.ty.has_heap() {
            return None;
        }
        match &e.kind {
            ExprKind::Var(v, _) => Some(self.base(*v)),
            // data of the program (D173): borrowed from no variable
            ExprKind::ConstArray(_) => Some(Vec::new()),
            ExprKind::Proj(x, _) | ExprKind::Unbox(x) => self.borrows(x),
            // views of an array of arrays: its windows, its rows (D95);
            // slices (rule F4, D97)
            ExprKind::Window(x, _) | ExprKind::Index(x, _) => self.borrows(x),
            ExprKind::Slice(x, _, _) if e.ty.slice_is_view() => self.borrows(x),
            ExprKind::CheckSize(_, _, b) | ExprKind::CheckNonNegative(_, b) => self.borrows(b),
            ExprKind::Let { var, body, .. } => match self.borrows(body) {
                Some(s) if s.contains(var) => None,
                other => other,
            },
            ExprKind::If(_, t, f) => match (self.borrows(t), self.borrows(f)) {
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
        ExprKind::Map(_, l) | ExprKind::Reduce(_, l, _) | ExprKind::Fold(_, _, l) | ExprKind::Scan(_, l, _) | ExprKind::Filter(_, l) => Some(l),
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
}

impl<'a> Rewriter<'a> {
    fn walk(&mut self, e: &mut Expr) -> R<Own> {
        let heap = e.ty.has_heap();
        // a slice of an array of tuples is a copy, not a view (D97)
        let view = !matches!(&e.kind, ExprKind::Slice(..)) || e.ty.slice_is_view();
        match &mut e.kind {
            ExprKind::Var(v, _) => {
                let i = self.index;
                self.index += 1;
                return Ok(if heap { Own::Borrowed(self.info.base(*v), Some(i)) } else { Own::Scalar });
            }
            // data of the program (D173): a consumer gets a copy
            ExprKind::ConstArray(_) => return Ok(Own::Borrowed(Vec::new(), None)),
            ExprKind::Let { var, init, body, .. } => {
                let v = *var;
                self.walk(init)?;
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
            // views: of the memory of their source (D95, D97)
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
/// result of a `fold` (D164).
fn new_tuple(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Call(..) | ExprKind::Action(..) | ExprKind::Tuple(_) | ExprKind::Fold(..) => true,
        ExprKind::Let { body, .. } | ExprKind::CheckSize(_, _, body) | ExprKind::CheckNonNegative(_, body) => new_tuple(body),
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
        let mut rw = Rewriter { info: &mut info, vars, index: 0, copy_reasons: Vec::new(), branch_owns: HashMap::new() };
        let o = rw.walk(body)?;
        if matches!(o, Own::Borrowed(..)) && body.ty.has_heap() {
            rw.make_fresh(body, o, false)?;
        }
        copy_reasons.extend(rw.copy_reasons);
    }
    set_drops(body, &info);
    Ok(match owned {
        Some(v) => info.owned[v as usize] && !(info.moved[v as usize] && consumes(body, v)),
        None => false,
    })
}

pub fn ownership(p: &mut Program) -> R<()> {
    // `main` owns the array of its arguments (allocated by the runtime at
    // start-up), defined in the outermost region
    p.drop_args = ownership_body(&mut p.body, &p.vars, Some(p.args), &mut p.copy_reasons)?;
    for f in p.fns.iter_mut() {
        ownership_body(&mut f.body, &p.vars, None, &mut p.copy_reasons)?;
    }
    Ok(())
}
