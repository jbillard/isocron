//! What a place of the source is (D269): the type of a local or a
//! parameter where it is declared or read, the signature of a function
//! where it is declared or called, the type of a field read or of another
//! expression that starts there; and where the name is declared. Read from
//! the typed tree of the checker, which keeps a type and the start of each
//! expression and of each local.
//!
//! Spans are the start of a construct only: a call is at its `(`, a field
//! at its `.`, a local and a reference to it at their name. The caller gives
//! the word under the cursor and these three places around it.

use isc_syntax::Span;
use isc_types::check::for_each_child;
use isc_types::tast::{Callee, FnId, TExpr, TExprKind, TFn, TProgram};
use isc_types::types::{Ty, TyDisplay};

/// The answer: what it is (`parameter`, `const`, `size`, `function`,
/// `action`, `field`, `value`), its name, its type or its signature, where
/// it is declared (a local, a function), and the function it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Hover {
    pub kind: &'static str,
    pub name: String,
    pub ty: String,
    pub decl: Option<Span>,
    pub func: Option<FnId>,
}

fn function(p: &TProgram, id: FnId, name: &str) -> Hover {
    let f = &p.fns[id];
    Hover { kind: if f.is_action { "action" } else { "function" }, name: name.into(), ty: signature(p, id), decl: Some(f.span), func: Some(id) }
}

fn local(f: &TFn, params: &[u32], l: u32) -> Hover {
    let x = &f.locals[l as usize];
    let kind = if params.contains(&l) { "parameter" } else { "const" };
    Hover { kind, name: x.name.clone(), ty: show(f, &x.ty), decl: Some(x.span), func: None }
}

fn other(kind: &'static str, name: &str, ty: String) -> Hover {
    Hover { kind, name: name.into(), ty, decl: None, func: None }
}

/// The place of a word of the source.
pub struct Place {
    pub file: u32,
    pub line: u32,
    /// The column of the word's first character.
    pub start: u32,
    /// The column of a `(` right after the word (spaces between), if any.
    pub paren: Option<u32>,
    /// The column of a `.` right before the word, if any.
    pub dot: Option<u32>,
}

/// `t` as the messages print it, with the names of the sizes and the type
/// parameters of `f`.
pub fn show(f: &TFn, t: &Ty) -> String {
    let sym_name = |s| f.sym_name(s);
    let param_name = |p: u32| f.generics.get(p as usize).map(|g| g.0.clone()).unwrap_or_else(|| "?".into());
    let size_var = |_v: u32| None;
    format!("{}", TyDisplay { ty: t, sym_name: &sym_name, param_name: &param_name, size_var: &size_var })
}

/// `function name<T>(a: A, b: B): R` (`action` for an action), its names as
/// the program writes them (a function of a module without its prefix).
pub fn signature(p: &TProgram, id: FnId) -> String {
    let f = &p.fns[id];
    let gens = if f.generics.is_empty() {
        String::new()
    } else {
        format!("<{}>", f.generics.iter().map(|g| g.0.clone()).collect::<Vec<_>>().join(", "))
    };
    let params: Vec<String> = f
        .params
        .iter()
        .zip(&f.param_tys)
        .map(|(l, t)| format!("{}: {}", f.locals[*l as usize].name, show(f, t)))
        .collect();
    format!("{} {}{}({}): {}", if f.is_action { "action" } else { "function" }, short(&f.name), gens, params.join(", "), show(f, &f.ret))
}

fn short(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn at(s: Span, file: u32, line: u32, col: u32) -> bool {
    s.file == file && s.line == line && s.col == col
}

/// What `word` at `place` is, if the program says.
pub fn hover(p: &TProgram, word: &str, place: &Place) -> Option<Hover> {
    let (file, line) = (place.file, place.line);
    let user: Vec<FnId> = (0..p.fns.len()).filter(|&i| p.fns[i].span.file == file && !p.fns[i].from_prelude).collect();
    // a declaration of a function, on its line
    for &i in &user {
        let f = &p.fns[i];
        if f.span.line == line && short(&f.name) == word {
            return Some(function(p, i, word));
        }
    }
    // a local or a parameter (of the function or of a lambda) where it is
    // declared
    for &i in &user {
        let f = &p.fns[i];
        let params = params_of(f);
        for (l, x) in f.locals.iter().enumerate() {
            if at(x.span, file, line, place.start) && x.name == word {
                return Some(local(f, &params, l as u32));
            }
        }
    }
    // in the expressions: a local read, a call, a field, anything else
    // that starts at the word (the innermost)
    let mut best: Option<(u8, Hover)> = None;
    for &i in &user {
        let f = &p.fns[i];
        let params = params_of(f);
        walk(&f.body, &mut |e: &TExpr| {
            let found = match &e.kind {
                TExprKind::Local(l) if at(e.span, file, line, place.start) => Some((0, local(f, &params, *l))),
                TExprKind::SizeVal(_) if at(e.span, file, line, place.start) => Some((0, other("size", word, show(f, &e.ty)))),
                TExprKind::Call { callee: Callee::Fn { id, .. }, .. } if place.paren.is_some_and(|c| at(e.span, file, line, c)) => {
                    Some((1, function(p, *id, word)))
                }
                // a function passed as a value: `map(x, twice)`
                TExprKind::FnRef { id, .. } if at(e.span, file, line, place.start) => Some((1, function(p, *id, word))),
                TExprKind::Field(..) if place.dot.is_some_and(|c| at(e.span, file, line, c)) => Some((2, other("field", word, show(f, &e.ty)))),
                _ if at(e.span, file, line, place.start) => Some((3, other("value", word, show(f, &e.ty)))),
                _ => None,
            };
            if let Some((rank, h)) = found {
                // a better kind wins; of the same kind, the later (inner) one
                if best.as_ref().is_none_or(|(r, _)| rank <= *r) {
                    best = Some((rank, h));
                }
            }
        });
    }
    best.map(|(_, h)| h)
}

/// The parameters of `f` and of its lambdas.
fn params_of(f: &TFn) -> Vec<u32> {
    let mut out = f.params.clone();
    walk(&f.body, &mut |e: &TExpr| {
        if let TExprKind::Lambda(ps, _) = &e.kind {
            out.extend(ps);
        }
    });
    out
}

fn walk(e: &TExpr, f: &mut dyn FnMut(&TExpr)) {
    f(e);
    for_each_child(e, &mut |c| walk(c, &mut *f));
}
