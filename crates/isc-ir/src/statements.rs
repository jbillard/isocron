//! The statements of an action (SPEC section 12).
//!
//! D701: the value of a `const` computed by pure code is computed in the
//! statement that reads it, when no effect runs between the two: nothing
//! can then see that it was not computed before (no clock read, no output,
//! no action called in between). Its computation is fused there with the
//! rest, as in a function: `const words = matches(text, r)` read by the
//! next statement alone is never made whole.

use std::collections::HashMap;

use crate::passes::lambda_body_index;
use crate::*;
use isc_syntax::Span;

/// The functions whose calls have effects: their bodies run an action, a
/// `bench` or tasks, or call such a function (a recursive action is a call,
/// not an `Action` node).
pub fn effectful_fns(p: &Program) -> Vec<bool> {
    let n = p.fns.len();
    let mut eff = vec![false; n];
    loop {
        let mut changed = false;
        for i in 0..n {
            if !eff[i] && !pure_in(&p.fns[i].body, &eff) {
                eff[i] = true;
                changed = true;
            }
        }
        if !changed {
            return eff;
        }
    }
}

/// `e` has no effect: no action, no `bench`, no tasks, no call of a
/// function `eff` marks.
pub fn pure_in(e: &Expr, eff: &[bool]) -> bool {
    match &e.kind {
        ExprKind::Action(..) | ExprKind::Bench(..) | ExprKind::Tasks(..) | ExprKind::InputPieces(_) => false,
        ExprKind::Call(f, _) if eff.get(*f as usize).copied().unwrap_or(true) => false,
        _ => e.children().into_iter().all(|c| pure_in(c, eff)),
    }
}

/// The effects of `e` run once every value it reads is computed: `e` is
/// pure, or pure `let`s around an action (or a call of one) whose
/// arguments are pure (they are computed before it runs).
pub fn effects_last(e: &Expr, eff: &[bool]) -> bool {
    if pure_in(e, eff) {
        return true;
    }
    match &e.kind {
        ExprKind::Let { init, body, stmt: false, .. } => pure_in(init, eff) && effects_last(body, eff),
        ExprKind::Action(_, args) | ExprKind::Call(_, args) => args.iter().all(|a| pure_in(a, eff)),
        _ => false,
    }
}

/// The number of reads of `v` in `e`.
pub fn uses(e: &Expr, v: VarId) -> usize {
    let here = matches!(e.kind, ExprKind::Var(w, _) if w == v) as usize;
    here + e.children().into_iter().map(|c| uses(c, v)).sum::<usize>()
}

/// `v` is read inside the body of a lambda of `e` (or of a `bench`): a
/// value moved there would be computed at each call.
pub fn read_in_lambda(e: &Expr, v: VarId) -> bool {
    let lam = lambda_body_index(e);
    e.children().into_iter().enumerate().any(|(i, c)| if Some(i) == lam { uses(c, v) > 0 } else { read_in_lambda(c, v) })
}

/// D701: the pure `const`s of the actions of `e` moved into the statement
/// that reads them (see the module).
pub fn statement_values(e: &mut Expr, eff: &[bool]) {
    for c in e.children_mut() {
        statement_values(c, eff);
    }
    let ExprKind::Let { var, init, body, stmt: true, .. } = &e.kind else { return };
    let v = *var;
    if !pure_in(init, eff) || uses(body, v) == 0 {
        // an effect stays where it is written; a value never read is
        // computed there all the same (section 12)
        return;
    }
    let Some(k) = reader(body, v, eff) else { return };
    let ExprKind::Let { init, body, drop, .. } = std::mem::replace(&mut e.kind, ExprKind::Zero) else { unreachable!() };
    let mut rest = *body;
    let target = statement_at(&mut rest, k);
    let inner = std::mem::replace(target, Expr::lit(Lit::Unit, Type::Unit, init.span));
    let (ty, span) = (inner.ty.clone(), inner.span);
    *target = Expr::new(ExprKind::Let { var: v, init, body: Box::new(inner), stmt: false, drop }, ty, span);
    *e = rest;
}

/// The statement of the chain `e` that reads `v`, as the number of
/// statements before it, when those statements are pure and the reader
/// reads `v` before any of its effects, never in a lambda; `None`
/// otherwise.
fn reader(mut e: &Expr, v: VarId, eff: &[bool]) -> Option<usize> {
    let mut k = 0;
    loop {
        match &e.kind {
            ExprKind::Let { init, body, stmt: true, .. } => {
                if uses(init, v) > 0 {
                    return (uses(body, v) == 0 && !read_in_lambda(init, v) && effects_last(init, eff)).then_some(k);
                }
                if !pure_in(init, eff) {
                    return None;
                }
                e = body;
                k += 1;
            }
            // the value of the action
            _ => return (!read_in_lambda(e, v) && effects_last(e, eff)).then_some(k),
        }
    }
}

/// The value of the statement `k` of the chain `e` (as `reader` counts
/// them): the value of its `let`, or the last value of the chain.
fn statement_at(e: &mut Expr, k: usize) -> &mut Expr {
    if k == 0 {
        return match &mut e.kind {
            ExprKind::Let { stmt: true, .. } => {
                let ExprKind::Let { init, .. } = &mut e.kind else { unreachable!() };
                init
            }
            _ => e,
        };
    }
    let ExprKind::Let { body, .. } = &mut e.kind else { unreachable!() };
    statement_at(body, k - 1)
}

// ---------------------------------------------------------------------
// D702: the standard input read by parts

/// The case of the texts of the parts: the maps applied to the text read,
/// in order (`toLowerCase`, `toUpperCase`).
type Cases = Vec<StrOp>;

/// `e` is `readStdinText()` with case maps applied to it (as the lowering
/// writes `readStdinText().toLowerCase()`: the read bound to a `let`, the
/// map of that variable): the span of the read and the maps, innermost
/// first.
fn input_read(e: &Expr) -> Option<(Span, Cases)> {
    match &e.kind {
        ExprKind::Action(ActionOp::ReadStdinText, args) if args.is_empty() => Some((e.span, Vec::new())),
        _ => {
            let (inner, op) = case_of(e)?;
            let (sp, mut cases) = input_read(inner)?;
            cases.push(op);
            Some((sp, cases))
        }
    }
}

/// `e` is a case map of a text: that text and the map (`s.toLowerCase()`,
/// or the text bound by a `let` read once by the map).
fn case_of(e: &Expr) -> Option<(&Expr, StrOp)> {
    match &e.kind {
        ExprKind::StrOp(op @ (StrOp::ToLower | StrOp::ToUpper), xs) if xs.len() == 1 => Some((&xs[0], *op)),
        ExprKind::Let { var, init, body, stmt: false, .. } => match &body.kind {
            ExprKind::StrOp(op @ (StrOp::ToLower | StrOp::ToUpper), xs) if xs.len() == 1 && matches!(xs[0].kind, ExprKind::Var(v, _) if v == *var) => Some((init, *op)),
            _ => None,
        },
        _ => None,
    }
}

/// `e` is the variable `x` with case maps applied to it: the maps,
/// innermost first.
fn cases_of_var(e: &Expr, x: VarId) -> Option<Cases> {
    match &e.kind {
        ExprKind::Var(v, _) if *v == x => Some(Vec::new()),
        _ => {
            let (inner, op) = case_of(e)?;
            let mut cases = cases_of_var(inner, x)?;
            cases.push(op);
            Some(cases)
        }
    }
}

/// An ASCII byte in another case.
fn ascii_case(c: u8, op: StrOp) -> u8 {
    match op {
        StrOp::ToLower => c.to_ascii_lowercase(),
        StrOp::ToUpper => c.to_ascii_uppercase(),
        _ => c,
    }
}

/// The path from `e` to the first node `want` accepts, children by index.
fn path_to(e: &Expr, want: &dyn Fn(&Expr) -> bool) -> Option<Vec<usize>> {
    if want(e) {
        return Some(Vec::new());
    }
    for (i, c) in e.children().into_iter().enumerate() {
        if let Some(mut p) = path_to(c, want) {
            p.insert(0, i);
            return Some(p);
        }
    }
    None
}

fn node_at<'a>(e: &'a Expr, path: &[usize]) -> &'a Expr {
    path.iter().fold(e, |e, &i| e.children()[i])
}

fn node_at_mut<'a>(e: &'a mut Expr, path: &[usize]) -> &'a mut Expr {
    match path.split_first() {
        None => e,
        Some((i, rest)) => node_at_mut(e.children_mut().into_iter().nth(*i).unwrap(), rest),
    }
}

/// The nodes on `path` from `e` are each evaluated once when `e` is: no
/// body of a lambda (or of a `bench`), no branch of an `if`.
fn evaluated_once(e: &Expr, path: &[usize]) -> bool {
    let mut at = e;
    for &i in path {
        if Some(i) == lambda_body_index(at) || (matches!(at.kind, ExprKind::If(..)) && i > 0) {
            return false;
        }
        at = at.children()[i];
    }
    true
}

/// What reads the runs of the input: the `fold` (or the length) whose
/// source they are, through `let`s of variables read once there and
/// `map`s or `filter`s of them alone.
struct Reader {
    /// The path of the consumer in the statement.
    at: Vec<usize>,
    /// The variables bound to the runs (or to a value made of them) on the
    /// way: their `let`s removed, their value moved where they are read.
    aliases: Vec<VarId>,
}

/// From the node at `path` of `t` (the runs, or a value made of them), up
/// to the `fold` or the length that reads it.
fn reader_of(t: &Expr, mut path: Vec<usize>) -> Option<Reader> {
    let mut aliases = Vec::new();
    loop {
        let (&i, up) = path.split_last()?;
        let parent = node_at(t, up);
        match &parent.kind {
            ExprKind::Fold(..) | ExprKind::Len(_) if i == 0 => return Some(Reader { at: up.to_vec(), aliases }),
            ExprKind::Map(ins, _) if ins.len() == 1 && i == 0 => path.pop(),
            ExprKind::Filter(..) if i == 0 => path.pop(),
            ExprKind::Let { var, body, stmt: false, .. } if i == 0 => {
                let w = *var;
                if uses(body, w) != 1 || read_in_lambda(body, w) {
                    return None;
                }
                let mut p = up.to_vec();
                p.push(1);
                let rest = path_to(body, &|e| matches!(e.kind, ExprKind::Var(v, _) if v == w))?;
                p.extend(rest);
                aliases.push(w);
                path = p;
                continue;
            }
            _ => return None,
        };
    }
}

/// The value at `e` with the `let`s of `aliases` removed, their values
/// kept in `taken`.
fn take_aliases(e: &mut Expr, aliases: &[VarId], taken: &mut HashMap<VarId, Expr>) {
    loop {
        match &e.kind {
            ExprKind::Let { var, stmt: false, .. } if aliases.contains(var) => {
                let ExprKind::Let { var, init, body, .. } = std::mem::replace(&mut e.kind, ExprKind::Zero) else { unreachable!() };
                taken.insert(var, *init);
                *e = *body;
            }
            _ => break,
        }
    }
    for c in e.children_mut() {
        take_aliases(c, aliases, taken);
    }
}

/// The source `e` of the consumer, its variables of `taken` replaced by
/// their values (again within those).
fn put_back(e: &mut Expr, taken: &mut HashMap<VarId, Expr>) {
    while let ExprKind::Var(v, _) = e.kind {
        match taken.remove(&v) {
            Some(x) => *e = x,
            None => break,
        }
    }
    for c in e.children_mut() {
        put_back(c, taken);
    }
}

/// D702: in an action, `const x = readStdinText()` (in a case or not) read
/// once, by the runs of a class of bytes of it (`matches(x, /[c]+/g)`,
/// D690, inlined) that a `fold` (or a length) reads, in the next statement
/// that reads anything with no effect in between (D701 brought the pure
/// statements there), with no effect of that statement before the `fold`,
/// which it evaluates once: the input is read by parts. The `fold` runs
/// over the parts of the input (`InputPieces`), each a text that ends after
/// a byte out of the class (an ASCII byte, in the case of the text), the
/// last one the rest; for each part, the `fold` of its runs, in the same
/// case. The runs of the whole are those of the parts in order (no run
/// crosses the end of a part), the case of the whole that of the parts (no
/// part cuts a character). The input is never whole in memory: one part,
/// a buffer of 64 KiB that grows only for a run longer than it.
pub fn input_pieces(e: &mut Expr, runs: &[RunsSite], eff: &[bool], vars: &mut Vec<VarInfo>) {
    for c in e.children_mut() {
        input_pieces(c, runs, eff, vars);
    }
    let ExprKind::Let { var, init, body, stmt: true, .. } = &e.kind else { return };
    let x = *var;
    let Some((read_span, outer)) = input_read(init) else { return };
    if uses(body, x) != 1 {
        return;
    }
    let Some(k) = reader(body, x, eff) else { return };
    // the statement that reads `x`, its runs and their consumer
    let ExprKind::Let { body, .. } = &mut e.kind else { unreachable!() };
    let t = statement_at(body, k);
    let Some(site_path) = path_to(t, &|n| matches!(&n.kind, ExprKind::Let { var, init, .. } if runs.iter().any(|r| r.var == *var) && cases_of_var(init, x).is_some())) else { return };
    let ExprKind::Let { var: s, init: arg, .. } = &node_at(t, &site_path).kind else { unreachable!() };
    let site = runs.iter().find(|r| r.var == *s).expect("a site of runs");
    let inner = cases_of_var(arg, x).expect("the text of the runs");
    let Some(rd) = reader_of(t, site_path.clone()) else { return };
    if !evaluated_once(t, &rd.at) || !evaluated_once(t, &site_path) {
        return;
    }
    // the bytes after which a part may end: ASCII, out of the class in the
    // case of the text
    let cases: Cases = outer.iter().chain(inner.iter()).copied().collect();
    let cut: Vec<Lit> = (0..256u32)
        .map(|c| {
            let m = if c < 128 { cases.iter().fold(c as u8, |b, op| ascii_case(b, *op)) } else { 0 };
            Lit::Int((c < 128 && site.class[m as usize] == 0) as i64)
        })
        .collect();
    // the runs of a part: those of its text in the case of the text
    let piece = vars.len() as VarId;
    vars.push(VarInfo { name: "part".into(), ty: Type::Str, prelude: false });
    let mut text = Expr::var(piece, Type::Str, read_span);
    for op in &cases {
        text = Expr::new(ExprKind::StrOp(*op, vec![text]), Type::Str, read_span);
    }
    let ExprKind::Let { init: arg, .. } = &mut node_at_mut(t, &site_path).kind else { unreachable!() };
    **arg = text;
    // the consumer taken out, a mark in its place
    let mark = vars.len() as VarId;
    vars.push(VarInfo { name: "mark".into(), ty: Type::Unit, prelude: false });
    let place = node_at_mut(t, &rd.at);
    let mut cons = std::mem::replace(place, Expr::var(mark, Type::Unit, read_span));
    // the `let`s of the values made of the runs removed, the source of the
    // consumer made of those values again
    let mut taken = HashMap::new();
    take_aliases(t, &rd.aliases, &mut taken);
    take_aliases(&mut cons, &rd.aliases, &mut taken);
    put_back(cons.children_mut().into_iter().next().expect("a source"), &mut taken);
    let at = path_to(t, &|n| matches!(n.kind, ExprKind::Var(v, _) if v == mark)).expect("the place of the consumer");
    let u8t = Type::Int(IntKind::U8);
    let table = Expr::new(ExprKind::ConstArray(cut), Type::Array(Box::new(u8t), Fixed(Some(256))), read_span);
    let parts = Expr::new(ExprKind::InputPieces(Box::new(table)), Type::Array(Box::new(Type::Str), Fixed::ANY), read_span);
    let sp = cons.span;
    let new = match cons.kind {
        ExprKind::Fold(src, init, g) => {
            // (the type of the accumulator as the function of the `fold`
            // has it: that of `init` may fix sizes that its steps change)
            let at = vars[g.params[0] as usize].ty.clone();
            let a = vars.len() as VarId;
            vars.push(VarInfo { name: "acc".into(), ty: at.clone(), prelude: false });
            let ty = cons.ty.clone();
            let each = Expr::new(ExprKind::Fold(src, Box::new(Expr::var(a, at, sp)), g), ty.clone(), sp);
            Expr::new(ExprKind::Fold(Box::new(parts), init, Lambda { params: vec![a, piece], body: Box::new(each), drop_params: vec![false, false] }), ty, sp)
        }
        ExprKind::Len(src) => {
            let i64t = Type::Int(IntKind::I64);
            let a = vars.len() as VarId;
            vars.push(VarInfo { name: "count".into(), ty: i64t.clone(), prelude: false });
            let each = Expr::new(ExprKind::Binary(BinOp::Add, Box::new(Expr::var(a, i64t.clone(), sp)), Box::new(Expr::new(ExprKind::Len(src), i64t.clone(), sp))), i64t.clone(), sp);
            let zero = Expr::lit(Lit::Int(0), i64t.clone(), sp);
            Expr::new(ExprKind::Fold(Box::new(parts), Box::new(zero), Lambda { params: vec![a, piece], body: Box::new(each), drop_params: vec![false, false] }), i64t, sp)
        }
        _ => unreachable!("a fold or a length"),
    };
    *node_at_mut(t, &at) = new;
    // the statement of the read removed: the parts are read by the fold
    let ExprKind::Let { body, .. } = std::mem::replace(&mut e.kind, ExprKind::Zero) else { unreachable!() };
    *e = *body;
}

// ---------------------------------------------------------------------
// D703: the elements of a `map` that are loops, made before a `reduce`

/// In the statements of `main` (outside the functions passed to arrays),
/// a `reduce` of tuples (reduced in the fixed form of section 10, one
/// element at a time) of a `map` whose elements are loops: the `map` made
/// first, its elements shared out by chunks among the threads as those of
/// any such `map` (D143), then reduced. Fused, the blocks of 4096 elements
/// of the fixed form put fewer than 4096 of them on one thread: the chunks
/// of fannkuch-redux, once D701 computed them in the statement of their
/// `reduce`, ran on one thread (21 % slower on 4). The bits are the same:
/// the fixed form groups the elements, whatever made them. (Inside a
/// function given to an array, the tasks of that array share the work. A
/// `reduce` of scalars keeps its fusion: an exact one shares its elements
/// that are loops (D201), the blocks of the others are parallel once there
/// are many; a few such elements reduced as floats stay on one thread.)
pub fn made_before_reduce(e: &mut Expr, vars: &mut Vec<VarInfo>) {
    let lam = lambda_body_index(e);
    for (i, c) in e.children_mut().into_iter().enumerate() {
        if Some(i) != lam {
            made_before_reduce(c, vars);
        }
    }
    let ExprKind::Reduce(x, _, _) = &mut e.kind else { return };
    let t = x.ty.elem().clone();
    let heavy = matches!(&x.kind, ExprKind::Map(_, ml) if crate::blocks::inner_loop(&ml.body));
    if !heavy || t.is_scalar() || t.has_heap() {
        return;
    }
    let v = vars.len() as VarId;
    vars.push(VarInfo { name: "made".into(), ty: x.ty.clone(), prelude: false });
    let (xt, sp) = (x.ty.clone(), x.span);
    let val = std::mem::replace(&mut **x, Expr::var(v, xt, sp));
    let inner = std::mem::replace(e, Expr::lit(Lit::Unit, Type::Unit, sp));
    let ty = inner.ty.clone();
    *e = Expr::new(ExprKind::Let { var: v, init: Box::new(val), body: Box::new(inner), stmt: false, drop: false }, ty, sp);
}
