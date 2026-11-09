//! Verification of the memory decisions of the IR, run after the ownership
//! pass at every compilation (DECISIONS D96): every allocation is freed
//! exactly once, no variable is used after it was moved, and no view (a
//! window, a row, a projection) outlives the memory it borrows. It is
//! independent of the ownership pass: it follows the values in evaluation
//! order (new memory, views of owned variables, moves, frees) instead of
//! recomputing aliases and last uses. A failure is a compilation error: a
//! defect of the compiler, never of the program, found before the program
//! runs.

use crate::passes::is_producer;
use crate::*;
use isc_syntax::{Diagnostic, Span};

type R<T> = std::result::Result<T, Diagnostic>;

/// What evaluating an expression gives, for memory.
#[derive(Clone, Debug, PartialEq)]
enum Val {
    /// No memory to free.
    Scalar,
    /// Newly allocated memory: whoever receives it must free it.
    Fresh,
    /// The memory of these owned variables, borrowed (empty: an input of the
    /// program or an element of an array, which nobody frees here).
    View(Vec<VarId>),
}

#[derive(Clone, Debug)]
enum State {
    Unbound,
    Scalar,
    /// Owns its value; `moved` is where it was moved, `region` where it was
    /// defined (a move must happen in the same region: not inside a loop body
    /// or an `if` branch entered after the definition).
    Owned { moved: Option<Span>, region: u32 },
    View(Vec<VarId>),
}

struct Checker<'p> {
    p: &'p Program,
    st: Vec<State>,
    region: u32,
    next_region: u32,
    /// The components taken out of each tuple variable (`Take`, D138).
    taken: std::collections::HashMap<VarId, Vec<u32>>,
    /// The arrays whose texts are views (D555): the owned variables whose
    /// memory their texts borrow. Each owns its block alone.
    spines: std::collections::HashMap<VarId, Vec<VarId>>,
}

fn fail<T>(span: Span, msg: String) -> R<T> {
    Err(Diagnostic::new(span, format!("memory check failed (a defect of the compiler, not of the program): {}", msg)))
}

/// Checks the memory decisions of a recursive function: its parameters are
/// borrowed (the state of a loop is owned, its copy made on entry, D206),
/// its result is new memory or a scalar (D102).
pub(crate) fn verify_fn(p: &Program, f: &Function) -> R<()> {
    let mut c = Checker { p, st: vec![State::Unbound; p.vars.len()], region: 0, next_region: 0, taken: Default::default(), spines: Default::default() };
    for (k, v) in f.params.iter().enumerate() {
        c.st[*v as usize] = if f.consumed.get(k).copied().unwrap_or(false) {
            State::Owned { moved: None, region: 0 }
        } else if p.vars[*v as usize].ty.has_heap() {
            State::View(Vec::new())
        } else {
            State::Scalar
        };
    }
    let r = c.eval(&f.body)?;
    // an owned parameter is moved or taken apart on every path (D206, D351)
    for (k, v) in f.params.iter().enumerate() {
        if f.consumed.get(k).copied().unwrap_or(false) && matches!(c.st[*v as usize], State::Owned { moved: None, .. }) {
            return fail(f.body.span, format!("`{}` owns `{}` and never frees it", f.name, p.vars[*v as usize].name));
        }
    }
    match r {
        Val::View(_) if f.body.ty.has_heap() => {
            fail(f.body.span, format!("`{}` returns memory it borrows", f.name))
        }
        _ => Ok(()),
    }
}

/// Checks the memory decisions of a program after the ownership pass.
pub fn verify(p: &Program) -> R<()> {
    for f in &p.fns {
        verify_fn(p, f)?;
    }
    let mut c = Checker { p, st: vec![State::Unbound; p.vars.len()], region: 0, next_region: 0, taken: Default::default(), spines: Default::default() };
    // `main` owns `args`: it frees it at the end, or gives it away
    c.st[p.args as usize] = State::Owned { moved: None, region: 0 };
    let r = c.eval(&p.body)?;
    if r == Val::Fresh {
        return fail(p.body.span, "the result of `main` holds memory that is never freed".into());
    }
    match (&c.st[p.args as usize], p.drop_args) {
        (State::Owned { moved: Some(at), .. }, true) => {
            fail(*at, "`args` is moved here and also freed at the end of `main`".into())
        }
        (State::Owned { moved: None, .. }, false) => fail(p.body.span, "`args` is never freed".into()),
        _ => Ok(()),
    }
}

fn union(mut a: Vec<VarId>, b: Vec<VarId>) -> Vec<VarId> {
    a.extend(b);
    a.sort();
    a.dedup();
    a
}

impl<'p> Checker<'p> {
    fn name(&self, v: VarId) -> String {
        self.p.vars[v as usize].name.clone()
    }

    fn state_of(&self, val: &Val, ty: &Type) -> State {
        match val {
            _ if !ty.has_heap() => State::Scalar,
            Val::Scalar => State::Scalar,
            Val::Fresh => State::Owned { moved: None, region: self.region },
            Val::View(o) => State::View(o.clone()),
        }
    }

    /// The value goes to a slot that takes ownership of it.
    fn consume(&self, v: Val, e: &Expr) -> R<()> {
        match v {
            Val::View(o) if !o.is_empty() => fail(
                e.span,
                format!("a value borrowed from `{}` is given away, so that it would be freed twice", self.name(o[0])),
            ),
            Val::View(_) if e.ty.has_heap() => {
                fail(e.span, "a borrowed input or element is given away, so that it would be freed by the program".into())
            }
            _ => Ok(()),
        }
    }

    /// The value goes to a slot that only reads it.
    fn borrow(&self, v: Val, e: &Expr) -> R<Val> {
        if v == Val::Fresh {
            return fail(e.span, "new memory is only read here and never freed".into());
        }
        Ok(v)
    }

    fn use_var(&mut self, v: VarId, u: Use, span: Span) -> R<Val> {
        if self.taken.contains_key(&v) && matches!(self.st[v as usize], State::Owned { moved: None, .. }) {
            return fail(span, format!("`{}` is used after a part of it was taken", self.name(v)));
        }
        match self.st[v as usize].clone() {
            State::Unbound => fail(span, format!("`{}` is used outside its definition", self.name(v))),
            State::Scalar => Ok(Val::Scalar),
            State::View(o) => {
                if u == Use::Move {
                    return fail(span, format!("`{}` borrows its value and is moved", self.name(v)));
                }
                Ok(Val::View(o))
            }
            State::Owned { moved: Some(at), .. } => {
                fail(span, format!("`{}` is used after it was moved at line {}", self.name(v), at.line))
            }
            // an array whose texts are views: never moved, read while the
            // memory of its texts lives (D555)
            State::Owned { moved: None, .. } if self.spines.contains_key(&v) => {
                if u == Use::Move {
                    return fail(span, format!("`{}` holds views of texts and is moved", self.name(v)));
                }
                let owners = self.spines[&v].clone();
                for w in &owners {
                    match &self.st[*w as usize] {
                        State::Owned { moved: Some(at), .. } => {
                            return fail(
                                span,
                                format!("`{}` holds views of the texts of `{}`, moved or freed at line {}", self.name(v), self.name(*w), at.line),
                            );
                        }
                        State::Unbound => {
                            return fail(span, format!("`{}` holds views of the texts of `{}`, out of its scope", self.name(v), self.name(*w)));
                        }
                        _ => {}
                    }
                }
                Ok(Val::View(union(owners, vec![v])))
            }
            State::Owned { moved: None, region } => {
                if u == Use::Move {
                    if region != self.region {
                        return fail(span, format!("`{}` is moved inside a loop or a branch, so possibly more than once", self.name(v)));
                    }
                    self.st[v as usize] = State::Owned { moved: Some(span), region };
                    return Ok(Val::Fresh);
                }
                Ok(Val::View(vec![v]))
            }
        }
    }

    /// Runs `f` in a new region (a loop body or a branch).
    fn in_region<T>(&mut self, f: impl FnOnce(&mut Self) -> R<T>) -> R<T> {
        let saved = self.region;
        self.next_region += 1;
        self.region = self.next_region;
        let r = f(self);
        self.region = saved;
        r
    }

    /// The source of a consumer that reads it element by element: a producer
    /// is fused (never materialized), anything else is borrowed. `in_order`:
    /// the consumer reads the elements in order, so that a `filter` can be
    /// fused too (rule F2, D99); read by index, it would be new memory that
    /// nothing frees.
    fn eval_src(&mut self, e: &Expr, in_order: bool) -> R<Val> {
        if let ExprKind::Filter(x, l) = &e.kind {
            if !in_order {
                return fail(e.span, "a `filter` read by index is never materialized, so never freed".into());
            }
            let xv = self.eval_src(x, false)?;
            let el = self.elem_of(&xv, x);
            self.lambda(l, &[el], |_, _, _| Ok(()))?;
            return Ok(match xv {
                Val::View(o) => Val::View(o),
                _ => Val::View(Vec::new()),
            });
        }
        if is_producer(e) {
            return match &e.kind {
                ExprKind::Slice(x, a, b) => {
                    let v = self.eval_src(x, false)?;
                    self.eval(a)?;
                    self.eval(b)?;
                    Ok(v)
                }
                ExprKind::Map(ins, l) => {
                    let mut owners = Vec::new();
                    let mut elems = Vec::new();
                    let one = ins.len() == 1;
                    for i in ins {
                        let v = self.eval_src(i, in_order && one)?;
                        elems.push(self.elem_of(&v, i));
                        if let Val::View(o) = v {
                            owners = union(owners, o);
                        }
                    }
                    self.lambda(l, &elems, |_, _, _| Ok(()))?;
                    Ok(Val::View(owners))
                }
                _ => {
                    for c in e.children() {
                        let v = self.eval(c)?;
                        self.borrow(v, c)?;
                    }
                    Ok(Val::View(Vec::new()))
                }
            };
        }
        let v = self.eval(e)?;
        self.borrow(v, e)
    }

    /// The value of an element of an array given by `v`: a scalar, or a view
    /// of the same memory for a row of an array of arrays.
    fn elem_of(&self, v: &Val, arr: &Expr) -> Val {
        if !arr.ty.is_array() || !arr.ty.elem().has_heap() {
            return Val::Scalar;
        }
        match v {
            Val::View(o) => Val::View(o.clone()),
            _ => Val::View(Vec::new()),
        }
    }

    /// The body of a lambda in a new region, its parameters bound to `args`;
    /// `result` checks the value of the body.
    fn lambda(&mut self, l: &Lambda, args: &[Val], result: impl FnOnce(&mut Self, Val, &Expr) -> R<()>) -> R<()> {
        self.in_region(|c| {
            for (p, a) in l.params.iter().zip(args) {
                let t = &c.p.vars[*p as usize].ty;
                c.st[*p as usize] = c.state_of(a, t);
            }
            let v = c.eval(&l.body)?;
            result(c, v, &l.body)
        })
    }

    /// The value of an array whose texts are views (D555): a `map` of texts
    /// each borrowed, or the lines or parts of a borrowed text; the owned
    /// variables whose memory they borrow.
    fn view_init(&mut self, e: &Expr) -> R<Vec<VarId>> {
        // the lines or the parts of a text, made as views of it
        if let ExprKind::StrOp(crate::StrOp::Lines | crate::StrOp::Split, es) = &e.kind {
            let owners = match self.eval(&es[0])? {
                Val::View(o) => o,
                _ => return fail(e.span, "the lines of a new text as views of it, which nothing frees".into()),
            };
            for x in &es[1..] {
                let v = self.eval(x)?;
                self.borrow(v, x)?;
            }
            return Ok(owners);
        }
        let ExprKind::Map(ins, l) = &e.kind else {
            return fail(e.span, "an array of views of texts that no `map` makes".into());
        };
        if *e.ty.elem() != Type::Str {
            return fail(e.span, "an array of views of something else than texts".into());
        }
        let mut elems = Vec::new();
        let one = ins.len() == 1;
        for i in ins {
            let v = self.eval_src(i, one)?;
            elems.push(self.elem_of(&v, i));
        }
        let mut owners = Vec::new();
        self.lambda(l, &elems, |_, v, b| match v {
            Val::View(o) => {
                owners = o;
                Ok(())
            }
            _ => fail(b.span, "a new text in an array of views of texts, which nothing frees".into()),
        })?;
        Ok(owners)
    }

    fn eval(&mut self, e: &Expr) -> R<Val> {
        use ExprKind::*;
        let heap = e.ty.has_heap();
        let fresh = |h: bool| if h { Val::Fresh } else { Val::Scalar };
        match &e.kind {
            Lit(_) => Ok(Val::Scalar),
            // data of the program (D173): borrowed from no variable, never
            // given away nor freed
            ConstArray(_) | ConstStrs(_) => Ok(Val::View(Vec::new())),
            // a string literal: borrowed data of the program (D301); a
            // `bigint` literal is made
            StrLit(_) if e.ty == Type::Str => Ok(Val::View(Vec::new())),
            StrLit(_) => Ok(Val::Fresh),
            // a slice or the bytes of a string read where it is: a view of
            // it (D303), which cannot free a new string
            StrOp(op, es) if crate::passes::str_view(*op, es, &e.ty) => {
                let v = self.eval(&es[0])?;
                for x in &es[1..] {
                    self.eval(x)?;
                }
                if v == Val::Fresh {
                    return fail(e.span, "a part of a new string, which nothing can free".into());
                }
                Ok(v)
            }
            // the operands of an operation on strings are only read (D134),
            // except the text of a replacement by a regular expression moved
            // into it, which the replacement owns (D315), and the left text
            // of a concatenation moved into it (D351), the bytes of `stringOf`
            // moved into it (D399)
            StrOp(op, es) => {
                let gives = matches!(op, crate::StrOp::Rx(crate::RxOp::Replace | crate::RxOp::ReplaceAll) | crate::StrOp::Concat | crate::StrOp::FromBytes);
                for (i, x) in es.iter().enumerate() {
                    let v = self.eval(x)?;
                    if i == 0 && gives && v == Val::Fresh && matches!(x.kind, Var(_, Use::Move)) {
                        continue;
                    }
                    self.borrow(v, x)?;
                }
                Ok(if e.ty.has_heap() { Val::Fresh } else { Val::Scalar })
            }
            // an empty array, or a zero tuple: owned like any new value
            Zero => Ok(fresh(heap)),
            Var(v, u) => self.use_var(*v, *u, e.span),
            Let { var, init, body, drop, .. } => {
                if self.p.views.contains(var) {
                    // an array whose texts are views (D555): its block owned
                    let owners = self.view_init(init)?;
                    self.spines.insert(*var, owners);
                    self.st[*var as usize] = State::Owned { moved: None, region: self.region };
                } else {
                    let iv = self.eval(init)?;
                    let t = self.p.vars[*var as usize].ty.clone();
                    self.st[*var as usize] = self.state_of(&iv, &t);
                }
                let bv = self.eval(body)?;
                let name = self.name(*var);
                if self.taken.contains_key(var) && matches!(self.st[*var as usize], State::Owned { moved: None, .. }) {
                    return fail(e.span, format!("only some parts of `{}` are taken: the others are never freed", name));
                }
                match self.st[*var as usize].clone() {
                    State::Owned { moved, .. } => match (moved, *drop) {
                        (Some(at), true) => {
                            return fail(at, format!("`{}` is moved here and also freed at the end of its scope", name));
                        }
                        (None, false) => return fail(e.span, format!("`{}` is never freed", name)),
                        (None, true) => {
                            if let Val::View(o) = &bv {
                                if o.contains(var) {
                                    return fail(
                                        e.span,
                                        format!("this value borrows `{}`, which is freed at the end of its scope", name),
                                    );
                                }
                            }
                        }
                        (Some(_), false) => {}
                    },
                    _ if *drop => return fail(e.span, format!("`{}` is freed but does not own its value", name)),
                    _ => {}
                }
                Ok(bv)
            }
            CheckSize(a, b, body) => {
                self.eval(a)?;
                self.eval(b)?;
                self.eval(body)
            }
            CheckNonNegative(a, body) => {
                self.eval(a)?;
                self.eval(body)
            }
            // each branch from the same state; a variable defined before the
            // `if` is consumed (moved or freed) in both branches or in
            // neither (D105)
            If(c, t, f) => {
                self.eval(c)?;
                let before = self.st.clone();
                // the parts taken too: each branch from those taken before
                // it (D367: a payload taken in one branch, the tuple read
                // and freed in the other), then those of both
                let taken_before = self.taken.clone();
                let tv = self.eval(t)?;
                let after_t = std::mem::replace(&mut self.st, before.clone());
                let taken_t = std::mem::replace(&mut self.taken, taken_before);
                let fv = self.eval(f)?;
                for (k, parts) in taken_t {
                    let t = self.taken.entry(k).or_default();
                    for p in parts {
                        if !t.contains(&p) {
                            t.push(p);
                        }
                    }
                }
                let consumed = |s: &State| matches!(s, State::Owned { moved: Some(_), .. });
                for (i, b) in before.iter().enumerate() {
                    if matches!(b, State::Unbound) {
                        self.st[i] = State::Unbound;
                        continue;
                    }
                    if consumed(&after_t[i]) != consumed(&self.st[i]) {
                        return fail(
                            e.span,
                            format!("`{}` is moved or freed in one branch of this `if` only", self.name(i as VarId)),
                        );
                    }
                }
                match (tv, fv) {
                    (Val::Scalar, Val::Scalar) => Ok(Val::Scalar),
                    (Val::Fresh, Val::Fresh) => Ok(Val::Fresh),
                    (Val::View(a), Val::View(b)) => Ok(Val::View(union(a, b))),
                    _ => fail(e.span, "one branch gives new memory and the other borrowed memory".into()),
                }
            }
            Tuple(es) => {
                for x in es {
                    let v = self.eval(x)?;
                    self.consume(v, x)?;
                }
                Ok(fresh(heap))
            }
            // a component taken out of a tuple that owns its value, in the
            // region of its definition; when every component that owns
            // memory is taken, the tuple counts as moved (D138)
            Take(x, i) => {
                let ExprKind::Var(r, _) = &x.kind else {
                    return fail(e.span, "a part taken out of something that is not a variable".into());
                };
                let r = *r;
                let name = self.name(r);
                let comps = match &x.ty {
                    Type::Tuple(ts) => ts.clone(),
                    _ => return fail(e.span, format!("a part taken out of `{}`, which is not a tuple", name)),
                };
                match self.st[r as usize].clone() {
                    State::Owned { moved: None, region } if region == self.region => {
                        let t = self.taken.entry(r).or_default();
                        if t.contains(i) {
                            return fail(e.span, format!("a part of `{}` is taken twice", name));
                        }
                        t.push(*i);
                        if comps.iter().enumerate().all(|(k, c)| !c.has_heap() || t.contains(&(k as u32))) {
                            self.st[r as usize] = State::Owned { moved: Some(e.span), region };
                        }
                        Ok(if comps[*i as usize].has_heap() { Val::Fresh } else { Val::Scalar })
                    }
                    // a scalar part is only read: allowed once the parts
                    // that own memory are taken
                    State::Owned { .. } if !comps[*i as usize].has_heap() => Ok(Val::Scalar),
                    _ => fail(e.span, format!("a part of `{}` is taken, but it does not own its value here", name)),
                }
            }
            Proj(x, i) => {
                let v = self.eval(x)?;
                Ok(match v {
                    _ if !heap => Val::Scalar,
                    Val::View(o) => Val::View(o),
                    Val::Fresh => {
                        // the other parts of a new tuple would be lost
                        let others = match &x.ty {
                            Type::Tuple(ts) => ts.iter().enumerate().any(|(k, t)| k != *i as usize && t.has_heap()),
                            _ => false,
                        };
                        if others {
                            return fail(e.span, "a part of a new tuple is taken and the other parts are never freed".into());
                        }
                        Val::Fresh
                    }
                    Val::Scalar => Val::Scalar,
                })
            }
            Update { target, index, value, .. } => {
                self.eval(index)?;
                let vv = self.eval(value)?;
                self.consume(vv, value)?;
                let tv = self.eval(target)?;
                self.consume(tv, target)?;
                Ok(Val::Fresh)
            }
            Copy(x) => match self.eval(x)? {
                Val::Fresh => fail(e.span, "new memory is copied and the original is never freed".into()),
                _ => Ok(Val::Fresh),
            },
            ArrayLit(es) => {
                // the rows of an array of arrays are taken by the literal
                for x in es {
                    let v = self.eval(x)?;
                    self.consume(v, x)?;
                }
                Ok(Val::Fresh)
            }
            Zeros(n) | Iota(n) => {
                self.eval(n)?;
                Ok(Val::Fresh)
            }
            Fill(n, x) => {
                self.eval(n)?;
                let v = self.eval(x)?;
                self.borrow(v, x)?;
                Ok(Val::Fresh)
            }
            Map(ins, l) => {
                let mut elems = Vec::new();
                let one = ins.len() == 1;
                for i in ins {
                    // an input moved into the `map`: its memory becomes the
                    // result, itself new (D207)
                    let v = if matches!(i.kind, Var(_, Use::Move)) { self.eval(i)? } else { self.eval_src(i, one)? };
                    elems.push(self.elem_of(&v, i));
                }
                if e.ty.elem().is_array() {
                    // rows of an array of arrays: written into its block, a
                    // new row in place, a borrowed one copied (D126)
                    self.lambda(l, &elems, |c, v, b| if v == Val::Fresh { Ok(()) } else { c.borrow(v, b).map(|_| ()) })?;
                } else {
                    self.lambda(l, &elems, |c, v, b| c.consume(v, b))?;
                }
                Ok(Val::Fresh)
            }
            Reduce(x, l, z) | Scan(x, l, z) => {
                let xv = self.eval_src(x, matches!(e.kind, Reduce(..)))?;
                let el = self.elem_of(&xv, x);
                let zv = self.eval(z)?;
                self.consume(zv, z)?;
                self.lambda(l, &[el.clone(), el], |c, v, b| c.consume(v, b))?;
                Ok(if matches!(e.kind, Scan(..)) { Val::Fresh } else { fresh(heap) })
            }
            Filter(x, l) => {
                let xv = self.eval_src(x, false)?;
                let el = self.elem_of(&xv, x);
                self.lambda(l, &[el], |_, _, _| Ok(()))?;
                Ok(Val::Fresh)
            }
            // each result moved into the new array (D373)
            Tasks(x, l) => {
                let xv = self.eval_src(x, false)?;
                let el = self.elem_of(&xv, x);
                self.lambda(l, &[el], |c, v, b| c.consume(v, b))?;
                Ok(Val::Fresh)
            }
            Fold(x, init, l) => {
                let xv = self.eval_src(x, true)?;
                let el = self.elem_of(&xv, x);
                let iv = self.eval(init)?;
                self.consume(iv, init)?;
                let acc_heap = init.ty.has_heap();
                let acc = l.params[0];
                let drop_acc = l.drop_params[0];
                let acc_val = if acc_heap { Val::Fresh } else { Val::Scalar };
                self.lambda(l, &[acc_val, el], |c, v, b| {
                    c.consume(v, b)?;
                    // the accumulator is moved into the result, or freed
                    if let State::Owned { moved, .. } = &c.st[acc as usize] {
                        match (moved, drop_acc) {
                            (Some(at), true) => {
                                return fail(*at, "the accumulator of a `fold` is moved and also freed".into());
                            }
                            (None, false) => {
                                return fail(b.span, "the accumulator of a `fold` is never freed".into());
                            }
                            _ => {}
                        }
                    }
                    Ok(())
                })?;
                Ok(fresh(heap))
            }
            Index(a, i) => {
                self.eval(i)?;
                let av = self.eval_src(a, false)?;
                Ok(if heap { self.elem_of(&av, a) } else { Val::Scalar })
            }
            Len(x) => {
                self.eval_src(x, false)?;
                Ok(Val::Scalar)
            }
            // a slice of a producer: its elements computed into new memory
            Slice(x, a, b) if is_producer(x) => {
                self.eval_src(x, false)?;
                self.eval(a)?;
                self.eval(b)?;
                Ok(Val::Fresh)
            }
            Slice(x, a, b) => {
                let v = self.eval(x)?;
                self.eval(a)?;
                self.eval(b)?;
                if !e.ty.slice_is_view() {
                    self.borrow(v, x)?;
                    return Ok(Val::Fresh);
                }
                // a view of a part of its source (rule F4): it cannot free a
                // new source, since its pointer is inside the block
                if v == Val::Fresh {
                    return fail(e.span, "a slice of new memory, which nothing can free".into());
                }
                Ok(v)
            }
            // a view of its source: it owns the memory of a new source
            Window(x, k) => {
                let v = self.eval(x)?;
                self.eval(k)?;
                Ok(v)
            }
            Math(_, xs) => {
                for x in xs {
                    let v = self.eval(x)?;
                    self.borrow(v, x)?;
                }
                Ok(Val::Scalar)
            }
            Unary(_, x) | Convert(x) | Sqrt(x) => {
                let v = self.eval(x)?;
                self.borrow(v, x)?;
                Ok(Val::Scalar)
            }
            Binary(_, a, b) | Pow(a, b) => {
                for x in [a, b] {
                    let v = self.eval(x)?;
                    self.borrow(v, x)?;
                }
                Ok(Val::Scalar)
            }
            Action(_, args) => {
                for x in args {
                    let v = self.eval(x)?;
                    self.borrow(v, x)?;
                }
                Ok(fresh(heap))
            }
            // the body, then the variable freed: it must still own its value,
            // and the result must not borrow it
            Free(v, body) => {
                let bv = self.eval(body)?;
                let name = self.name(*v);
                match self.st[*v as usize].clone() {
                    State::Owned { moved: None, region } => {
                        if let Val::View(o) = &bv {
                            if o.contains(v) {
                                return fail(e.span, format!("this value borrows `{}`, which is freed after it", name));
                            }
                        }
                        self.st[*v as usize] = State::Owned { moved: Some(e.span), region };
                        Ok(bv)
                    }
                    State::Owned { moved: Some(at), .. } => {
                        fail(e.span, format!("`{}` is freed after it was moved at line {}", name, at.line))
                    }
                    _ => fail(e.span, format!("`{}` is freed but does not own its value", name)),
                }
            }
            // a node takes its tuple; the tuple of a node is borrowed from it
            Box(x) => {
                let v = self.eval(x)?;
                self.consume(v, x)?;
                Ok(Val::Fresh)
            }
            Unbox(x) => match self.eval(x)? {
                Val::Fresh => fail(e.span, "a new node is read and never freed".into()),
                v => Ok(v),
            },
            // the arguments are borrowed by the function; its result is new
            Call(_, args) => {
                for x in args {
                    let v = self.eval(x)?;
                    self.borrow(v, x)?;
                }
                Ok(fresh(heap))
            }
            Bench(r, body) => {
                self.eval(r)?;
                self.in_region(|c| {
                    let v = c.eval(body)?;
                    c.consume(v, body)
                })?;
                Ok(fresh(heap))
            }
        }
    }
}
