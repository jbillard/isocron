//! Lowering from the typed AST to the IR.
//!
//! Every call to a top-level function is inlined (SPEC section 7: inlining
//! without threshold before fusion), every function argument is resolved at
//! compile time and specialized (section 11), element-wise operators become
//! `Map`, and the size tests of section 4 become `CheckSize`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use crate::*;
use isc_syntax::{Diagnostic, Span};
use isc_types::tast::{self, Callee, FnId, LocalId, Prim, TExpr, TExprKind, TPat, TProgram, TStmt};
use isc_types::types::{Poly, Size, SymId, Ty};

type R<T> = std::result::Result<T, Diagnostic>;

fn poc<T>(span: Span, what: &str) -> R<T> {
    Err(Diagnostic::new(span, format!("{} is not supported in the POC (SPEC section 15)", what)))
}

fn internal<T>(span: Span, what: &str) -> R<T> {
    Err(Diagnostic::new(span, format!("internal compiler error: {}", what)))
}

#[derive(Clone)]
enum Binding<'p> {
    Var(VarId),
    Func(FuncVal<'p>),
}

#[derive(Clone)]
enum FuncVal<'p> {
    /// `act`: the lambda has an effect (its type is an action, D386).
    Lambda { params: &'p [LocalId], body: &'p TExpr, env: Rc<Env<'p>>, act: bool },
    Global { id: FnId, type_args: Vec<Ty> },
    Prim(Prim),
    Op(BinOp),
    /// A function chosen at run time (`if c then g else h`): each call is a
    /// branch on the condition (defunctionalization, SPEC section 11, D107).
    Choice { cond: Box<Expr>, then: Box<FuncVal<'p>>, other: Box<FuncVal<'p>> },
    /// A tuple or a record of functions, an array of functions (section 16,
    /// v1, D114): known when the program is compiled.
    Tuple(Vec<FuncVal<'p>>),
    Array(Vec<FuncVal<'p>>),
    /// The function at a run-time index of an array of functions: each call
    /// is a branch on the index.
    Table { fns: Vec<FuncVal<'p>>, index: Box<Expr> },
    /// A function that reads variables bound by the pattern of an arm of a
    /// `match` (D132): their values, computed where it is bound.
    Bound { binds: Vec<(VarId, Expr)>, inner: Box<FuncVal<'p>> },
}

/// A type that holds a function: passed and bound as a function value.
/// `isPure(h)` (D386).
fn is_pure_test(c: &TExpr) -> bool {
    matches!(&c.kind, TExprKind::Call { callee: tast::Callee::Prim(Prim::IsPure), args, .. } if args.len() == 1)
}

fn holds_fn(t: &Ty) -> bool {
    match t {
        Ty::Fn(..) => true,
        Ty::Tuple(ts) => ts.iter().any(holds_fn),
        Ty::Record(fs) => fs.iter().any(|(_, t)| holds_fn(t)),
        Ty::Array(_, e) => holds_fn(e),
        _ => false,
    }
}

/// The conditions of a chosen function moved into variables, computed once
/// where the function is bound: `(variable, condition)` in order.
fn hoist_choices<'p>(l: &mut Lower<'p>, fv: FuncVal<'p>, out: &mut Vec<(VarId, Expr)>) -> FuncVal<'p> {
    match fv {
        FuncVal::Bound { binds, inner } => {
            out.extend(binds);
            hoist_choices(l, *inner, out)
        }
        FuncVal::Choice { cond, then, other } => {
            let then = hoist_choices(l, *then, out);
            let other = hoist_choices(l, *other, out);
            let cond = match cond.kind {
                ExprKind::Var(..) | ExprKind::Lit(_) => cond,
                _ => {
                    let t = l.new_var("chosen", Type::Bool);
                    let sp = cond.span;
                    out.push((t, *cond));
                    Box::new(Expr::var(t, Type::Bool, sp))
                }
            };
            FuncVal::Choice { cond, then: Box::new(then), other: Box::new(other) }
        }
        fv => fv,
    }
}

enum Arg<'p> {
    Val(Expr),
    Func(FuncVal<'p>),
}

#[derive(Clone)]
struct Env<'p> {
    fn_id: FnId,
    /// Concrete type arguments of the instance.
    type_args: Rc<Vec<Ty>>,
    locals: HashMap<LocalId, Binding<'p>>,
    sizes: HashMap<SymId, Expr>,
    /// True inside a lambda body or a `fn`: lets are not action statements.
    pure: bool,
}

pub struct Lower<'p> {
    p: &'p TProgram,
    vars: Vec<VarInfo>,
    shapes: Vec<Shape>,
    copy_reasons: Vec<(Span, String)>,
    /// The recursive functions lowered so far, by function and type
    /// arguments (D102); `None` while its body is lowered.
    specs: HashMap<(FnId, Vec<Ty>, Vec<Option<FnKey>>), u32>,
    fns: Vec<Option<Function>>,
    rets: Vec<Type>,
    /// The recursive types, by sum and type arguments (D103); `None` while
    /// its layout is computed (it refers to itself).
    rec_ids: RefCell<HashMap<(u32, Vec<Ty>), u32>>,
    recs: RefCell<Vec<Option<RecType>>>,
    /// The tuples of the sum types laid out, with the slots of each variant;
    /// the tuples of records, tuples and pairs of `zip`, which a sum of the
    /// same tuple could not be told from (D448).
    sum_types: RefCell<Vec<(Type, Vec<Vec<u32>>)>>,
    plain_tuples: RefCell<HashSet<Type>>,
    /// The functions with `requireFused` or `requireNoAlloc` met, with
    /// their type arguments (D106).
    required: Vec<(FnId, Vec<Ty>)>,
    /// Where a function is chosen at run time (D107).
    choices: Vec<Span>,
    /// The functions with `allowReassoc` met (D109).
    reassoc_fns: Vec<(String, Span)>,
    /// The values a function returned by a function reads (its arguments,
    /// its `const`s), computed once into these variables around the
    /// expression being lowered (D152).
    hoisted: Vec<(VarId, Expr)>,
    /// The functions stored in each slot of a function of a sum type
    /// (D153): a value holds the code of its function, its index here.
    /// `stored` grows as the constructors are lowered; `known` is the
    /// table of the previous pass, which the calls branch on.
    stored: Vec<(SlotKey, Vec<(StoredKey, FuncVal<'p>)>)>,
    known: Vec<(SlotKey, Vec<(StoredKey, FuncVal<'p>)>)>,
    stored_grew: bool,
    /// The parameters (function, index) that a call cannot prove the sizes
    /// of: those of the arrays in their records and tuples are tested on
    /// entry of a function that is not inlined (D336).
    unproved: HashSet<(FnId, usize)>,
}

/// The parameters that a call tests the sizes of (`size_checks`).
fn unproved_params(p: &TProgram) -> HashSet<(FnId, usize)> {
    fn walk(e: &tast::TExpr, out: &mut HashSet<(FnId, usize)>) {
        if let tast::TExprKind::Call { callee: tast::Callee::Fn { id, .. }, size_checks, .. } = &e.kind {
            for &i in size_checks {
                out.insert((*id, i));
            }
        }
        isc_types::check::for_each_child(e, &mut |c| walk(c, out));
    }
    let mut out = HashSet::new();
    for f in &p.fns {
        walk(&f.body, &mut out);
    }
    out
}

/// A slot of a function in a sum type: (type, its arguments, variant,
/// field) (D153).
type SlotKey = (u32, Vec<Ty>, u32, u32);
/// A function stored in a sum type, which reads nothing of its context.
type StoredKey = (FnKey, Vec<Ty>);

fn allows_reassoc(f: &tast::TFn) -> bool {
    f.requires.iter().any(|(m, _)| *m == isc_syntax::ast::Modifier::AllowReassoc)
}

fn has_alloc_requirement(f: &tast::TFn) -> bool {
    f.requires
        .iter()
        .any(|(m, _)| matches!(m, isc_syntax::ast::Modifier::RequireFused | isc_syntax::ast::Modifier::RequireNoAlloc))
}

impl<'p> Lower<'p> {
    fn new(p: &'p TProgram) -> Lower<'p> {
        Lower {
            p,
            vars: Vec::new(),
            shapes: Vec::new(),
            copy_reasons: Vec::new(),
            specs: HashMap::new(),
            fns: Vec::new(),
            rets: Vec::new(),
            rec_ids: RefCell::new(HashMap::new()),
            recs: RefCell::new(Vec::new()),
            sum_types: RefCell::new(Vec::new()),
            plain_tuples: RefCell::new(HashSet::new()),
            required: Vec::new(),
            choices: Vec::new(),
            reassoc_fns: Vec::new(),
            hoisted: Vec::new(),
            stored: Vec::new(),
            known: Vec::new(),
            stored_grew: false,
            unproved: unproved_params(p),
        }
    }

    /// The code of a function stored in the slot `key` of a value (D153):
    /// a literal, or a choice of codes for a function chosen at run time.
    fn stored_code(&mut self, key: &SlotKey, fv: FuncVal<'p>, span: Span) -> R<Expr> {
        let u32t = Type::Int(IntKind::U32);
        match fv {
            FuncVal::Choice { cond, then, other } => {
                let a = self.stored_code(key, *then, span)?;
                let b = self.stored_code(key, *other, span)?;
                Ok(Expr::new(ExprKind::If(cond, Box::new(a), Box::new(b)), u32t, span))
            }
            fv => {
                let sk = match &fv {
                    FuncVal::Lambda { env, .. } if captures(&fv).is_empty() => {
                        fn_key(&fv).map(|k| (k, (*env.type_args).clone()))
                    }
                    FuncVal::Lambda { .. } => None,
                    _ => fn_key(&fv).map(|k| (k, Vec::new())),
                };
                let Some(sk) = sk else {
                    return Err(Diagnostic::new(
                        span,
                        "a function stored in a value of a sum type cannot read a variable or a size of its context (D153): store that value in another field",
                    ));
                };
                let pos = match self.stored.iter().position(|(k, _)| k == key) {
                    Some(p) => p,
                    None => {
                        self.stored.push((key.clone(), Vec::new()));
                        self.stored.len() - 1
                    }
                };
                let table = &mut self.stored[pos].1;
                let code = match table.iter().position(|(k, _)| *k == sk) {
                    Some(c) => c,
                    None => {
                        table.push((sk, fv));
                        self.stored_grew = true;
                        table.len() - 1
                    }
                };
                Ok(Expr::lit(Lit::Int(code as i64), u32t, span))
            }
        }
    }

    /// The function held by a value whose code is `code` (D153): a branch
    /// on the code among the functions stored in that slot.
    fn stored_fn(&mut self, key: &SlotKey, code: Expr, span: Span) -> FuncVal<'p> {
        let fns = self.known.iter().find(|(k, _)| k == key).map(|(_, t)| t.iter().map(|(_, f)| f.clone()).collect()).unwrap_or_default();
        self.choices.push(span);
        FuncVal::Table { fns, index: Box::new(code) }
    }

    /// The slot key and the type of each field of a variant whose type is
    /// a function (D153).
    fn fn_fields(&self, env: &Env, ty: &Ty, variant: u32) -> Vec<Option<SlotKey>> {
        match self.conc(env, ty) {
            Ty::Sum { id, args, .. } => {
                let def = &self.p.sums[id as usize];
                def.payload(variant as usize, &args)
                    .iter()
                    .enumerate()
                    .map(|(k, t)| is_fn_ty(&self.conc(env, t)).then(|| (id, args.clone(), variant, k as u32)))
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    /// The body of `f` with `allowReassoc`: marked, and `f` listed.
    fn reassoc_body(&mut self, f: &tast::TFn, body: &mut Expr) {
        if allows_reassoc(f) {
            body.mark_reassoc();
            if !self.reassoc_fns.iter().any(|(n, s)| *n == f.name && *s == f.span) {
                self.reassoc_fns.push((f.name.clone(), f.span));
            }
        }
    }
}

/// The `extern` functions of a program in the types of the IR (D125).
fn extern_fns(p: &TProgram) -> Vec<ExternFn> {
    fn ty(t: &Ty) -> Type {
        match t {
            Ty::Int(k) => Type::Int(*k),
            Ty::F64 => Type::F64,
            Ty::F32 => Type::F32,
            Ty::Bool => Type::Bool,
            Ty::Str | Ty::RegExp => Type::Str,
            Ty::BigInt => Type::Big,
            Ty::Array(_, e) => Type::Array(Box::new(ty(e)), crate::Fixed::ANY),
            _ => Type::Unit,
        }
    }
    p.externs.iter().map(|e| ExternFn { name: e.name.clone(), params: e.params.iter().map(ty).collect(), ret: ty(&e.ret) }).collect()
}

/// A function lowered alone, for the check of its requirements: its body
/// as the body of the program (its parameters unbound: borrowed inputs),
/// the recursive functions it calls (D106).
pub fn probe(p: &TProgram, id: FnId, type_args: Vec<Ty>) -> R<Program> {
    let mut l = Lower::new(p);
    let span = p.fns[id].span;
    let args = l.new_var("args", Type::Array(Box::new(Type::Str), crate::Fixed::ANY));
    let n = p.fns[id].params.len();
    let idx = l.lower_fn(id, type_args, vec![None; n], vec![None; n], span)?;
    let fns: Vec<Function> = l.fns.into_iter().map(|f| f.expect("a recursive function lowered")).collect();
    let body = fns[idx as usize].body.clone();
    let recs = l.recs.into_inner().into_iter().map(|r| r.expect("a recursive type laid out")).collect();
    let sums = sum_slots(&l.sum_types.borrow(), &l.plain_tuples.borrow());
    Ok(Program { args, drop_args: false, copy_reasons: l.copy_reasons, fns, recs, body, vars: l.vars, shapes: l.shapes, choices: l.choices, reassoc_fns: l.reassoc_fns, externs: extern_fns(p), sys_paths: Vec::new(), sum_slots: sums, views: Default::default() })
}

pub fn lower(p: &TProgram) -> R<Program> {
    // the functions stored in values of sum types (D153): their tables are
    // complete when a pass stores no function the previous one did not
    let mut known = Vec::new();
    for _ in 0..16 {
        let mut l = Lower::new(p);
        l.known = std::mem::take(&mut known);
        let start = l.known.clone();
        l.stored = start;
        let out = lower_pass(p, &mut l)?;
        if !l.stored_grew {
            return Ok(out);
        }
        known = std::mem::take(&mut l.stored);
    }
    Err(Diagnostic::new(p.fns[p.main].span, "the functions stored in values do not reach a fixed set (an internal limit of 16 passes)"))
}

fn lower_pass<'p>(p: &'p TProgram, l: &mut Lower<'p>) -> R<Program> {
    let main = &p.fns[p.main];
    let args = l.new_var("args", Type::Array(Box::new(Type::Str), crate::Fixed::ANY));
    let mut env = Env {
        fn_id: p.main,
        type_args: Rc::new(Vec::new()),
        locals: HashMap::new(),
        sizes: HashMap::new(),
        pure: false,
    };
    env.locals.insert(main.params[0], Binding::Var(args));
    let sp = main.span;
    l.bind_sizes(&mut env, &main.param_tys[0].clone(), Expr::var(args, Type::Array(Box::new(Type::Str), crate::Fixed::ANY), sp));
    let body = l.expr(&mut env, &main.body)?;
    // the requirements of the functions met, and of those that are not
    // generic, met or not
    let mut required = std::mem::take(&mut l.required);
    for (id, f) in p.fns.iter().enumerate() {
        if has_alloc_requirement(f) && f.generics.is_empty() && !required.iter().any(|(i, _)| *i == id) {
            required.push((id, Vec::new()));
        }
    }
    crate::requirements::check(p, &required)?;
    let fns = std::mem::take(&mut l.fns).into_iter().map(|f| f.expect("a recursive function lowered")).collect();
    let recs = std::mem::take(&mut *l.recs.borrow_mut()).into_iter().map(|r| r.expect("a recursive type laid out")).collect();
    Ok(Program {
        args,
        drop_args: false,
        copy_reasons: std::mem::take(&mut l.copy_reasons),
        fns,
        recs,
        body,
        vars: std::mem::take(&mut l.vars),
        shapes: std::mem::take(&mut l.shapes),
        choices: std::mem::take(&mut l.choices),
        reassoc_fns: std::mem::take(&mut l.reassoc_fns),
        externs: extern_fns(p),
        sys_paths: isc_types::check::system_paths(p),
        sum_slots: sum_slots(&l.sum_types.borrow(), &l.plain_tuples.borrow()),
        views: Default::default(),
    })
}

/// The positions in the prelude replaced by `at`.
fn relocate_prelude(e: &mut Expr, at: Span) {
    if e.span.file == isc_types::PRELUDE_FILE {
        e.span = at;
    }
    for c in e.children_mut() {
        relocate_prelude(c, at);
    }
}


fn is_fn_ty(t: &Ty) -> bool {
    matches!(t, Ty::Fn(..))
}

/// The identity of a function given to a recursive function: part of the
/// key of its specialization (D128). A lambda by its expression and the
/// functions of its context it reads (D387).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum FnKey {
    Global(FnId, Vec<Ty>),
    Prim(Prim),
    Op(BinOp),
    Lambda(*const TExpr, Vec<(LocalId, FnKey)>),
}

/// The key of a function given to a recursive function, when it reads
/// nothing of the context where it was written (a function of the program,
/// a primitive, an operator, a lambda without captures): the specialized
/// function then needs no other parameter (D128).
fn fn_key(f: &FuncVal) -> Option<FnKey> {
    match f {
        FuncVal::Global { id, type_args } => Some(FnKey::Global(*id, type_args.clone())),
        FuncVal::Prim(p) => Some(FnKey::Prim(*p)),
        FuncVal::Op(o) => Some(FnKey::Op(*o)),
        FuncVal::Lambda { params, body, env, .. } => {
            // its captured locals and the sizes it reads become parameters
            // of the specialization (D131, D152): values of the caller
            let mut bound: HashSet<LocalId> = params.iter().copied().collect();
            let mut free = Vec::new();
            let mut sizes = Vec::new();
            free_locals(body, &mut bound, &mut free, &mut sizes);
            // a function of its context it reads: known when the program is
            // compiled, part of the key (D387); what it reads of its own
            // context, parameters of the specialization too (`captures`,
            // D563)
            let mut fns = Vec::new();
            for l in &free {
                match env.locals.get(l) {
                    Some(Binding::Var(_)) => {}
                    Some(Binding::Func(g)) => fns.push((*l, fn_key(g)?)),
                    _ => return None,
                }
            }
            let all_sizes = sizes.iter().all(|s| env.sizes.contains_key(s));
            all_sizes.then_some(FnKey::Lambda(*body as *const TExpr, fns))
        }
        _ => None,
    }
}

/// The size variables of a type.
fn ty_syms(t: &Ty, out: &mut Vec<SymId>) {
    match t {
        Ty::Array(sz, e) => {
            if let Size::Poly(p) = sz {
                for s in p.syms() {
                    if !out.contains(&s) {
                        out.push(s);
                    }
                }
            }
            ty_syms(e, out);
        }
        Ty::Tuple(ts) => ts.iter().for_each(|x| ty_syms(x, out)),
        Ty::Record(fs) => fs.iter().for_each(|(_, x)| ty_syms(x, out)),
        Ty::Fn(ps, r, _) => {
            ps.iter().for_each(|x| ty_syms(x, out));
            ty_syms(r, out);
        }
        Ty::Sum { args, .. } => args.iter().for_each(|x| ty_syms(x, out)),
        _ => {}
    }
}

/// The locals of enclosing scopes an expression reads, in their order of
/// first reading; `sizes`: the size variables it reads (in a value or in a
/// type), all of an enclosing scope (a lambda declares none).
fn free_locals(e: &TExpr, bound: &mut HashSet<LocalId>, out: &mut Vec<LocalId>, sizes: &mut Vec<SymId>) {
    use TExprKind as K;
    let go = |x: &TExpr, bound: &mut HashSet<LocalId>, out: &mut Vec<LocalId>, sizes: &mut Vec<SymId>| free_locals(x, bound, out, sizes);
    ty_syms(&e.ty, sizes);
    match &e.kind {
        K::Lit(_) | K::FnRef { .. } | K::PrimRef(_) | K::OpRef(_) => {}
        K::Local(l) => {
            if !bound.contains(l) && !out.contains(l) {
                out.push(*l);
            }
        }
        K::SizeVal(p) => {
            for s in p.syms() {
                if !sizes.contains(&s) {
                    sizes.push(s);
                }
            }
        }
        K::Tuple(es) | K::Array(es) => es.iter().for_each(|x| go(x, bound, out, sizes)),
        K::Construct { args, .. } => args.iter().for_each(|x| go(x, bound, out, sizes)),
        K::Field(x, _) | K::Unary(_, x) => go(x, bound, out, sizes),
        K::FieldUpdate { target, value, .. } => {
            go(target, bound, out, sizes);
            go(value, bound, out, sizes);
        }
        K::Index(a, b) | K::Pow(a, b) => {
            go(a, bound, out, sizes);
            go(b, bound, out, sizes);
        }
        K::Binary { lhs, rhs, .. } => {
            go(lhs, bound, out, sizes);
            go(rhs, bound, out, sizes);
        }
        K::Slice(x, a, b) | K::If(x, a, b) => {
            go(x, bound, out, sizes);
            go(a, bound, out, sizes);
            go(b, bound, out, sizes);
        }
        K::Update { target, index, value, .. } => {
            go(target, bound, out, sizes);
            go(index, bound, out, sizes);
            go(value, bound, out, sizes);
        }
        K::Call { callee, args, .. } => {
            if let Callee::Value(f) = callee {
                go(f, bound, out, sizes);
            }
            args.iter().for_each(|x| go(x, bound, out, sizes));
        }
        K::Lambda(ps, b) => {
            bound.extend(ps.iter().copied());
            go(b, bound, out, sizes);
        }
        K::Match(x, arms) => {
            go(x, bound, out, sizes);
            for (p, b) in arms {
                pat_locals(p, bound);
                go(b, bound, out, sizes);
            }
        }
        K::Block(stmts, r) => {
            for s in stmts {
                match s {
                    TStmt::Let(p, v) => {
                        go(v, bound, out, sizes);
                        pat_locals(p, bound);
                    }
                    TStmt::Expr(v) => go(v, bound, out, sizes),
                }
            }
            go(r, bound, out, sizes);
        }
    }
}

/// What a function given to a recursive function reads of the context where
/// it was written: a value of a variable of the caller, or a size.
enum Capture {
    Var(VarId),
    Size(Expr),
}

/// What `f` reads of its context (D131, D152), then, depth first, what the
/// functions of its context it reads read of theirs (D563): the arguments
/// of its specialization after the others, in this order (`rebind` makes
/// the parameters in the same order).
fn captures(f: &FuncVal) -> Vec<Capture> {
    fn into(f: &FuncVal, out: &mut Vec<Capture>) {
        let FuncVal::Lambda { params, body, env, .. } = f else { return };
        let mut bound: HashSet<LocalId> = params.iter().copied().collect();
        let mut free = Vec::new();
        let mut sizes = Vec::new();
        free_locals(body, &mut bound, &mut free, &mut sizes);
        for l in &free {
            if let Some(Binding::Var(v)) = env.locals.get(l) {
                out.push(Capture::Var(*v));
            }
        }
        for s in &sizes {
            if let Some(x) = env.sizes.get(s) {
                out.push(Capture::Size(x.clone()));
            }
        }
        for l in &free {
            if let Some(Binding::Func(g)) = env.locals.get(l) {
                into(g, out);
            }
        }
    }
    let mut out = Vec::new();
    into(f, &mut out);
    out
}

fn pat_locals(p: &TPat, out: &mut HashSet<LocalId>) {
    match p {
        TPat::Bind(l) => {
            out.insert(*l);
        }
        TPat::Tuple(ps, _) => ps.iter().for_each(|q| pat_locals(q, out)),
        TPat::Ctor { subs, .. } => subs.iter().for_each(|q| pat_locals(q, out)),
        TPat::Wild(_) | TPat::Lit(..) => {}
    }
}


/// The tuples of sum types whose values are told apart by their type alone
/// (D448): the tag (`u32`) first, at least two variants, one layout of slots
/// for the tuple, and no record, tuple or pair of `zip` of the same tuple.
fn sum_slots(sums: &[(Type, Vec<Vec<u32>>)], plain: &HashSet<Type>) -> Vec<(Type, Vec<Vec<u32>>)> {
    let mut out: Vec<(Type, Vec<Vec<u32>>)> = Vec::new();
    let mut bad: HashSet<Type> = HashSet::new();
    for (t, slots) in sums {
        let tagged = matches!(t, Type::Tuple(cs) if matches!(cs.first(), Some(Type::Int(IntKind::U32))));
        if !tagged || slots.len() < 2 || plain.contains(t) {
            bad.insert(t.clone());
            continue;
        }
        match out.iter().find(|(u, _)| u == t) {
            Some((_, s)) if s != slots => {
                bad.insert(t.clone());
            }
            Some(_) => {}
            None => out.push((t.clone(), slots.clone())),
        }
    }
    out.retain(|(t, _)| !bad.contains(t));
    out
}

impl<'p> Lower<'p> {
    /// The IR tuple of a sum type: the tag (`u32`), then the slots of every
    /// variant one after the other; and the slots of each variant.
    fn sum_layout(&self, env: &Env, id: u32, args: &[Ty], span: Span) -> R<(Vec<Type>, Vec<Vec<u32>>)> {
        let def = &self.p.sums[id as usize];
        let mut comps = vec![Type::Int(IntKind::U32)];
        let mut slots = Vec::new();
        for k in 0..def.variants.len() {
            let mut mine = Vec::new();
            for t in def.payload(k, args) {
                mine.push(comps.len() as u32);
                // a function: its code (D153)
                if is_fn_ty(&self.conc(env, &t)) {
                    comps.push(Type::Int(IntKind::U32));
                } else {
                    comps.push(self.ty(env, &t, span)?);
                }
            }
            slots.push(mine);
        }
        self.sum_types.borrow_mut().push((Type::Tuple(comps.clone()), slots.clone()));
        Ok((comps, slots))
    }

    /// The index of the recursive type `sums[id]<args>` in `Program::recs`,
    /// laid out the first time (D103).
    fn rec_index(&self, env: &Env, id: u32, args: &[Ty], span: Span) -> R<u32> {
        let key = (id, args.to_vec());
        if let Some(&i) = self.rec_ids.borrow().get(&key) {
            return Ok(i);
        }
        let idx = self.recs.borrow().len() as u32;
        self.recs.borrow_mut().push(None);
        self.rec_ids.borrow_mut().insert(key, idx);
        let def = &self.p.sums[id as usize];
        let (comps, slots) = self.sum_layout(env, id, args, span)?;
        let mut vs = Vec::new();
        for (k, v) in def.variants.iter().enumerate() {
            let shapes = def.payload(k, args).iter().map(|t| self.shape(env, t, span)).collect::<R<_>>()?;
            vs.push((v.name.clone(), slots[k].clone(), shapes));
        }
        self.recs.borrow_mut()[idx as usize] =
            Some(RecType { name: def.name.clone(), layout: Type::Tuple(comps), shape: Shape::Sum(vs) });
        Ok(idx)
    }

    /// Is the sum type `id` the built-in `Map` (D158)?
    fn is_map(&self, id: u32) -> bool {
        let d = &self.p.sums[id as usize];
        d.variants.len() == 1 && d.variants[0].name == isc_types::MAP_CTOR
    }

    fn is_rec(&self, env: &Env, t: &Ty) -> bool {
        matches!(self.conc(env, t), Ty::Sum { id, .. } if self.p.sums[id as usize].recursive)
    }

    fn sum_of(&self, env: &Env, t: &Ty, span: Span) -> R<(Vec<Type>, Vec<Vec<u32>>)> {
        match self.conc(env, t) {
            Ty::Sum { id, args, .. } => self.sum_layout(env, id, &args, span),
            _ => internal(span, "not a sum type"),
        }
    }

    /// The shape of a printed value of checker type `t` (record field names,
    /// constructors).
    fn shape(&self, env: &Env, t: &Ty, span: Span) -> R<Shape> {
        Ok(match self.conc(env, t) {
            Ty::Record(fs) => Shape::Record(
                fs.iter().map(|(n, _)| n.clone()).collect(),
                fs.iter().map(|(_, t)| self.shape(env, t, span)).collect::<R<_>>()?,
            ),
            Ty::Tuple(ts) => {
                let s: Vec<Shape> = ts.iter().map(|t| self.shape(env, t, span)).collect::<R<_>>()?;
                if s.iter().all(|x| *x == Shape::Plain) {
                    Shape::Plain
                } else {
                    Shape::Tuple(s)
                }
            }
            Ty::Array(_, e) => match self.shape(env, &e, span)? {
                Shape::Plain => Shape::Plain,
                s => Shape::Array(Box::new(s)),
            },
            Ty::Sum { id, args, .. } if self.p.sums[id as usize].recursive => Shape::Rec(self.rec_index(env, id, &args, span)?),
            Ty::Sum { id, .. } if self.is_map(id) => Shape::Plain,
            Ty::Sum { id, args, .. } => {
                let def = &self.p.sums[id as usize];
                let (_, slots) = self.sum_layout(env, id, &args, span)?;
                let mut vs = Vec::new();
                for (k, v) in def.variants.iter().enumerate() {
                    let shapes = def.payload(k, &args).iter().map(|t| self.shape(env, t, span)).collect::<R<_>>()?;
                    vs.push((v.name.clone(), slots[k].clone(), shapes));
                }
                Shape::Sum(vs)
            }
            _ => Shape::Plain,
        })
    }

    /// `None` when the pattern matches every value; else the condition.
    fn pattern_test(&mut self, env: &Env, p: &TPat, val: &Expr, span: Span) -> R<Option<Expr>> {
        let bt = Type::Bool;
        let and = |a: Option<Expr>, b: Option<Expr>| -> Option<Expr> {
            match (a, b) {
                (None, x) | (x, None) => x,
                (Some(a), Some(b)) => Some(Expr::new(
                    ExprKind::If(Box::new(a), Box::new(b), Box::new(Expr::lit(Lit::Bool(false), Type::Bool, span))),
                    Type::Bool,
                    span,
                )),
            }
        };
        let proj = |i: u32, t: &Type| Expr::new(ExprKind::Proj(Box::new(val.clone()), i), t.clone(), span);
        Ok(match p {
            TPat::Wild(_) | TPat::Bind(_) => None,
            TPat::Lit(l, t) => {
                let lit = self.lit(env, l, t, span)?;
                Some(Expr::new(ExprKind::Binary(BinOp::Eq, Box::new(val.clone()), Box::new(lit)), bt, span))
            }
            TPat::Tuple(ps, _) => {
                let ts = match &val.ty {
                    Type::Tuple(ts) => ts.clone(),
                    _ => return internal(span, "tuple pattern on a non-tuple"),
                };
                let mut acc = None;
                for (i, q) in ps.iter().enumerate() {
                    let c = self.pattern_test(env, q, &proj(i as u32, &ts[i]), span)?;
                    acc = and(acc, c);
                }
                acc
            }
            TPat::Ctor { variant, subs, ty, .. } => {
                let (comps, slots) = self.sum_of(env, ty, span)?;
                // a recursive type: the tuple of the node (D103)
                let unboxed;
                let proj = if self.is_rec(env, ty) {
                    unboxed = Expr::new(ExprKind::Unbox(Box::new(val.clone())), Type::Tuple(comps.clone()), span);
                    Box::new(move |i: u32, t: &Type| Expr::new(ExprKind::Proj(Box::new(unboxed.clone()), i), t.clone(), span))
                        as Box<dyn Fn(u32, &Type) -> Expr>
                } else {
                    Box::new(proj)
                };
                let tag = Expr::lit(Lit::Int(*variant as i64), Type::Int(IntKind::U32), span);
                let mut acc = Some(Expr::new(
                    ExprKind::Binary(BinOp::Eq, Box::new(proj(0, &comps[0])), Box::new(tag)),
                    Type::Bool,
                    span,
                ));
                for (q, &slot) in subs.iter().zip(slots[*variant as usize].iter()) {
                    let c = self.pattern_test(env, q, &proj(slot, &comps[slot as usize]), span)?;
                    acc = and(acc, c);
                }
                acc
            }
        })
    }

    fn new_var(&mut self, name: &str, ty: Type) -> VarId {
        self.vars.push(VarInfo { name: name.to_string(), ty });
        (self.vars.len() - 1) as VarId
    }

    fn tfn(&self, env: &Env) -> &'p tast::TFn {
        &self.p.fns[env.fn_id]
    }

    /// Substitutes the type parameters of the instance.
    fn conc(&self, env: &Env, t: &Ty) -> Ty {
        match t {
            Ty::Param(i) => env.type_args[*i as usize].clone(),
            Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| self.conc(env, t)).collect()),
            Ty::Record(fs) => Ty::Record(fs.iter().map(|(n, t)| (n.clone(), self.conc(env, t))).collect()),
            Ty::Sum { id, name, args } => {
                Ty::Sum { id: *id, name: name.clone(), args: args.iter().map(|t| self.conc(env, t)).collect() }
            }
            Ty::Array(s, e) => Ty::Array(s.clone(), Box::new(self.conc(env, e))),
            Ty::Fn(ps, r, act) => Ty::Fn(ps.iter().map(|t| self.conc(env, t)).collect(), Box::new(self.conc(env, r)), *act),
            t => t.clone(),
        }
    }

    fn ty(&self, env: &Env, t: &Ty, span: Span) -> R<Type> {
        Ok(match self.conc(env, t) {
            Ty::Int(k) => Type::Int(k),
            Ty::F64 => Type::F64,
            Ty::F32 => Type::F32,
            Ty::Bool => Type::Bool,
            Ty::Unit => Type::Unit,
            Ty::Str | Ty::RegExp => Type::Str,
            Ty::BigInt => Type::Big,
            Ty::Handle(_) => Type::Handle,
            Ty::Sum { id, args, .. } if self.p.sums[id as usize].recursive => Type::Rec(self.rec_index(env, id, &args, span)?),
            // `Map<K, V>` (D158): the keys, the values, the positions of the
            // keys sorted
            Ty::Sum { id, args, .. } if self.is_map(id) => Type::Tuple(vec![
                Type::Array(Box::new(self.ty(env, &args[0], span)?), crate::Fixed::ANY),
                Type::Array(Box::new(self.ty(env, &args[1], span)?), crate::Fixed::ANY),
                Type::Array(Box::new(Type::Int(IntKind::I64)), crate::Fixed::ANY),
            ]),
            Ty::Sum { id, args, .. } => Type::Tuple(self.sum_layout(env, id, &args, span)?.0),
            // a record is a tuple of its fields, sorted by name
            Ty::Record(fs) => {
                let mut out = Vec::new();
                for (_, t) in &fs {
                    if is_fn_ty(t) {
                        return poc(span, "a function stored in a record (SPEC section 11)");
                    }
                    out.push(self.ty(env, t, span)?);
                }
                self.plain_tuples.borrow_mut().insert(Type::Tuple(out.clone()));
                Type::Tuple(out)
            }
            Ty::Tuple(ts) => {
                let mut out = Vec::new();
                for t in &ts {
                    if is_fn_ty(t) {
                        return poc(span, "a function stored in a tuple (SPEC section 11)");
                    }
                    out.push(self.ty(env, t, span)?);
                }
                self.plain_tuples.borrow_mut().insert(Type::Tuple(out.clone()));
                Type::Tuple(out)
            }
            Ty::Array(sz, e) => {
                let et = self.ty(env, &e, span)?;
                // its length when the type fixes it (`f64[5]`, D204)
                let fixed = match &sz {
                    Size::Poly(p) => p.as_const().filter(|c| (0..=u32::MAX as i64).contains(c)).map(|c| c as u32),
                    // a size of its own (D341): known at run time
                    Size::Var(_) | Size::Own => None,
                };
                // an array of nodes: pointers, one owner each (D130)
                Type::Array(Box::new(et), crate::Fixed(fixed))
            }
            Ty::Fn(..) => return internal(span, "function type in value position"),
            Ty::Param(_) | Ty::Var(_) => return internal(span, "unresolved type"),
        })
    }

    /// The run-time value of a size polynomial of the current function.
    fn size(&mut self, env: &Env, p: &Poly, span: Span) -> R<Expr> {
        if let Some(c) = p.as_const() {
            return Ok(Expr::i64(c, span));
        }
        let i64t = Type::Int(IntKind::I64);
        let mut acc: Option<Expr> = None;
        for (m, c) in &p.terms {
            let mut t: Option<Expr> = if *c == 1 && !m.is_empty() { None } else { Some(Expr::i64(*c, span)) };
            for s in m {
                // a local `i64` given as a `size` argument stands for itself
                let local = match self.tfn(env).syms.get(*s as usize).map(|i| &i.origin) {
                    Some(tast::SymOrigin::Local(l)) => match env.locals.get(l) {
                        Some(Binding::Var(v)) => Some(Expr::var(*v, i64t.clone(), span)),
                        _ => None,
                    },
                    _ => None,
                };
                let v = match env.sizes.get(s).cloned().or(local) {
                    Some(v) => v,
                    None => {
                        let name = self.tfn(env).sym_name(*s);
                        return internal(span, &format!("the value of size `{}` is not available here", name));
                    }
                };
                t = Some(match t {
                    None => v,
                    Some(t) => Expr::new(ExprKind::Binary(BinOp::Mul, Box::new(t), Box::new(v)), i64t.clone(), span),
                });
            }
            let t = t.unwrap();
            acc = Some(match acc {
                None => t,
                Some(a) => Expr::new(ExprKind::Binary(BinOp::Add, Box::new(a), Box::new(t)), i64t.clone(), span),
            });
        }
        Ok(acc.unwrap())
    }

    fn try_size(&mut self, env: &Env, t: &Ty, span: Span) -> Option<Expr> {
        match t {
            Ty::Array(Size::Poly(p), _) => {
                if p.syms().iter().all(|s| env.sizes.contains_key(s)) {
                    self.size(env, p, span).ok()
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Records the run-time value of size symbols that are fixed by binding
    /// `v` (of type `t` in the current function).
    fn bind_sizes(&mut self, env: &mut Env, t: &Ty, v: Expr) {
        match t {
            Ty::Array(Size::Poly(p), _) => {
                if let Some(s) = p.as_sym() {
                    if !env.sizes.contains_key(&s) {
                        let sp = v.span;
                        env.sizes.insert(s, Expr::new(ExprKind::Len(Box::new(v)), Type::Int(IntKind::I64), sp));
                    }
                }
            }
            Ty::Tuple(ts) => {
                for (i, c) in ts.iter().enumerate() {
                    if let Ok(ct) = self.ty(env, c, v.span) {
                        let pr = Expr::new(ExprKind::Proj(Box::new(v.clone()), i as u32), ct, v.span);
                        self.bind_sizes(env, c, pr);
                    }
                }
            }
            _ => {}
        }
    }

    fn lit(&self, env: &Env, l: &tast::Lit, ty: &Ty, span: Span) -> R<Expr> {
        let t = self.ty(env, ty, span)?;
        if let tast::Lit::Str(s) = l {
            return Ok(Expr::new(ExprKind::StrLit(s.clone()), Type::Str, span));
        }
        // a `bigint` (D156): its bytes, computed here
        let big = |b: Vec<u8>| Expr::new(ExprKind::StrLit(String::from_utf8(b).unwrap_or_default()), Type::Big, span);
        match (l, &t) {
            (tast::Lit::BigInt(digits, radix, neg), _) => {
                return Ok(big(isc_runtime_core::big::literal_radix(digits, *radix, *neg)));
            }
            (tast::Lit::Int(v), Type::Big) => {
                let b = if *v < 0 { isc_runtime_core::big::literal(&(-v).to_string(), true) } else { isc_runtime_core::big::literal(&v.to_string(), false) };
                return Ok(big(b));
            }
            _ => {}
        }
        let lit = match (l, &t) {
            (tast::Lit::Int(v), Type::Int(k)) => {
                // in a generic function, the type of a literal is known only here
                if *v < k.min_value() || *v > k.max_value() {
                    return Err(Diagnostic::new(span, format!("the literal `{}` is out of range for `{}`", v, k.name())));
                }
                Lit::Int(canon(*k, *v))
            }
            (tast::Lit::Int(v), Type::F64) => Lit::F64(*v as f64),
            (tast::Lit::Int(v), Type::F32) => Lit::F32(*v as f32),
            (tast::Lit::Float(f, _), Type::F64) => Lit::F64(*f),
            (tast::Lit::Float(_, f), Type::F32) => Lit::F32(*f),
            (tast::Lit::Bool(b), Type::Bool) => Lit::Bool(*b),
            (tast::Lit::Unit, Type::Unit) => Lit::Unit,
            _ => return internal(span, "ill-typed literal"),
        };
        Ok(Expr::lit(lit, t, span))
    }

    fn expr(&mut self, env: &mut Env<'p>, e: &'p TExpr) -> R<Expr> {
        let mark = self.hoisted.len();
        let mut out = self.expr_here(env, e)?;
        // the values read by the functions returned by functions met in `e`
        // (D152): computed once, around `e`
        while self.hoisted.len() > mark {
            let (v, x) = self.hoisted.pop().unwrap();
            let sp = x.span;
            out = self.let_(v, x, out, false, sp);
        }
        Ok(out)
    }

    fn expr_here(&mut self, env: &mut Env<'p>, e: &'p TExpr) -> R<Expr> {
        let span = e.span;
        match &e.kind {
            TExprKind::Lit(l) => self.lit(env, l, &e.ty, span),
            TExprKind::Local(l) => match env.locals.get(l) {
                Some(Binding::Var(v)) => {
                    let t = self.vars[*v as usize].ty.clone();
                    Ok(Expr::var(*v, t, span))
                }
                Some(Binding::Func(_)) => internal(span, "function used as a value"),
                None => internal(span, "unbound local"),
            },
            TExprKind::SizeVal(p) => self.size(env, p, span),
            TExprKind::FnRef { .. } | TExprKind::PrimRef(_) | TExprKind::OpRef(_) | TExprKind::Lambda(..) => {
                poc(span, "a function used as data")
            }
            TExprKind::Tuple(es) => {
                let ty = self.ty(env, &e.ty, span)?;
                let mut out = Vec::new();
                for x in es {
                    out.push(self.expr(env, x)?);
                }
                Ok(Expr::new(ExprKind::Tuple(out), ty, span))
            }
            TExprKind::Array(es) => {
                let ty = self.ty(env, &e.ty, span)?;
                let mut out = Vec::new();
                for x in es {
                    out.push(self.expr(env, x)?);
                }
                // scalar constants: data of the program, not allocated (D173)
                if !out.is_empty() && ty.elem().is_scalar() && out.iter().all(|x| matches!(x.kind, ExprKind::Lit(_))) {
                    let lits = out.into_iter().map(|x| if let ExprKind::Lit(l) = x.kind { l } else { unreachable!() }).collect();
                    return Ok(Expr::new(ExprKind::ConstArray(lits), ty, span));
                }
                // string literals: data of the program too (D401)
                if !out.is_empty() && *ty.elem() == Type::Str && out.iter().all(|x| x.ty == Type::Str && matches!(x.kind, ExprKind::StrLit(_))) {
                    let strs = out.into_iter().map(|x| if let ExprKind::StrLit(s) = x.kind { s } else { unreachable!() }).collect();
                    return Ok(Expr::new(ExprKind::ConstStrs(strs), ty, span));
                }
                Ok(Expr::new(ExprKind::ArrayLit(out), ty, span))
            }
            TExprKind::Call { callee, args, size_checks } => self.call(env, callee, args, size_checks, e),
            TExprKind::Index(a, i) => {
                let ty = self.ty(env, &e.ty, span)?;
                let a = self.expr(env, a)?;
                let i = self.expr(env, i)?;
                // A row of an array of arrays is a view of it (D95). Of a
                // temporary array, it is copied: the view would point into a
                // block freed after the indexing.
                // The temporary is bound outside the copy, so that it is freed
                // after it.
                let temporary = !matches!(a.kind, ExprKind::Var(..) | ExprKind::Proj(..));
                if ty.is_array() && temporary {
                    let at = a.ty.clone();
                    let t = self.new_var("rows", at.clone());
                    let ix = Expr::new(ExprKind::Index(Box::new(Expr::var(t, at, span)), Box::new(i)), ty.clone(), span);
                    let copy = Expr::new(ExprKind::Copy(Box::new(ix)), ty, span);
                    self.copy_reasons.push((span, "a row of a temporary array, freed after the indexing".into()));
                    return Ok(self.let_(t, a, copy, false, span));
                }
                Ok(Expr::new(ExprKind::Index(Box::new(a), Box::new(i)), ty, span))
            }
            TExprKind::Slice(x, a, b) => {
                let ty = self.ty(env, &e.ty, span)?;
                let x = self.expr(env, x)?;
                let i64t = Type::Int(IntKind::I64);
                let to_i64 = |v: Expr| {
                    if v.ty == i64t {
                        v
                    } else {
                        let sp = v.span;
                        Expr::new(ExprKind::Convert(Box::new(v)), i64t.clone(), sp)
                    }
                };
                let a = to_i64(self.expr(env, a)?);
                let b = to_i64(self.expr(env, b)?);
                Ok(Expr::new(ExprKind::Slice(Box::new(x), Box::new(a), Box::new(b)), ty, span))
            }
            TExprKind::Update { target, index, value, require_unique } => {
                let t = self.expr(env, target)?;
                let i = self.expr(env, index)?;
                let v = self.expr(env, value)?;
                let ty = t.ty.clone();
                Ok(Expr::new(
                    ExprKind::Update {
                        target: Box::new(t),
                        index: Box::new(i),
                        value: Box::new(v),
                        require_unique: *require_unique,
                    },
                    ty,
                    span,
                ))
            }
            TExprKind::Construct { variant, args, .. } => {
                let ty = self.ty(env, &e.ty, span)?;
                let (comps, slots) = self.sum_of(env, &e.ty, span)?;
                let mut out: Vec<Expr> = comps.iter().map(|t| Expr::new(ExprKind::Zero, t.clone(), span)).collect();
                out[0] = Expr::lit(Lit::Int(*variant as i64), Type::Int(IntKind::U32), span);
                let fields = self.fn_fields(env, &e.ty, *variant);
                for (k, (a, &slot)) in args.iter().zip(slots[*variant as usize].iter()).enumerate() {
                    out[slot as usize] = match fields.get(k).cloned().flatten() {
                        Some(key) => {
                            let fv = self.fn_value(env, a)?;
                            self.stored_code(&key, fv, a.span)?
                        }
                        None => self.expr(env, a)?,
                    };
                }
                if self.is_rec(env, &e.ty) {
                    // a new node on the heap (D103)
                    let tuple = Expr::new(ExprKind::Tuple(out), Type::Tuple(comps), span);
                    return Ok(Expr::new(ExprKind::Box(Box::new(tuple)), ty, span));
                }
                Ok(Expr::new(ExprKind::Tuple(out), ty, span))
            }
            TExprKind::Match(x, arms) => {
                let ty = self.ty(env, &e.ty, span)?;
                let xv = self.expr(env, x)?;
                let s = self.new_var("match", xv.ty.clone());
                let sv = Expr::var(s, xv.ty.clone(), span);
                // from the last arm (taken when no other matches: the arms
                // are exhaustive) back to the first
                let mut acc: Option<Expr> = None;
                for (p, body) in arms.iter().rev() {
                    let mut q = VecDeque::new();
                    q.push_back((p, sv.clone()));
                    let taken = self.bind_list(env, q, false, &mut |me, env| me.expr(env, body))?;
                    acc = Some(match acc {
                        None => taken,
                        Some(rest) => match self.pattern_test(env, p, &sv, span)? {
                            None => taken,
                            Some(c) => Expr::new(ExprKind::If(Box::new(c), Box::new(taken), Box::new(rest)), ty.clone(), span),
                        },
                    });
                }
                let body = acc.ok_or_else(|| Diagnostic::new(span, "a `match` needs at least one arm"))?;
                Ok(self.let_(s, xv, body, false, span))
            }
            TExprKind::Field(x, i) => {
                let ty = self.ty(env, &e.ty, span)?;
                let x = self.expr(env, x)?;
                Ok(Expr::new(ExprKind::Proj(Box::new(x), *i), ty, span))
            }
            TExprKind::FieldUpdate { target, index, value } => {
                // a new tuple: the other fields of the target, and `value`
                let t = self.expr(env, target)?;
                let v = self.expr(env, value)?;
                let ty = t.ty.clone();
                let comps = match &ty {
                    Type::Tuple(ts) => ts.clone(),
                    _ => return internal(span, "record update of a non-record"),
                };
                let r = self.new_var("r", ty.clone());
                let fields: Vec<Expr> = comps
                    .iter()
                    .enumerate()
                    .map(|(k, ct)| {
                        if k as u32 == *index {
                            v.clone()
                        } else {
                            Expr::new(ExprKind::Proj(Box::new(Expr::var(r, ty.clone(), span)), k as u32), ct.clone(), span)
                        }
                    })
                    .collect();
                let body = Expr::new(ExprKind::Tuple(fields), ty.clone(), span);
                Ok(self.let_(r, t, body, false, span))
            }
            TExprKind::Unary(op, x) => {
                let x = self.expr(env, x)?;
                if x.ty == Type::Big {
                    return Ok(Expr::new(ExprKind::StrOp(StrOp::Big(BigOp::Neg), vec![x]), Type::Big, span));
                }
                if x.ty.is_array() {
                    let et = x.ty.elem().clone();
                    let p = self.new_var("x", et.clone());
                    let body =
                        Expr::new(ExprKind::Unary(*op, Box::new(Expr::var(p, et.clone(), span))), et, span);
                    let ty = x.ty.clone();
                    Ok(self.map1(x, p, body, ty, span))
                } else {
                    let ty = x.ty.clone();
                    Ok(Expr::new(ExprKind::Unary(*op, Box::new(x)), ty, span))
                }
            }
            TExprKind::Binary { op, lhs, rhs, size_check } => {
                let l = self.expr(env, lhs)?;
                let r = self.expr(env, rhs)?;
                if *size_check {
                    let sl = self.try_size(env, &lhs.ty, span);
                    let sr = self.try_size(env, &rhs.ty, span);
                    if let (Some(a), Some(b)) = (sl, sr) {
                        let res = self.binary(*op, l, r, span)?;
                        let ty = res.ty.clone();
                        return Ok(Expr::new(ExprKind::CheckSize(Box::new(a), Box::new(b), Box::new(res)), ty, span));
                    }
                    // sizes not available symbolically: bind the operands
                    let va = self.new_var("lhs", l.ty.clone());
                    let vb = self.new_var("rhs", r.ty.clone());
                    let (lt, rt) = (l.ty.clone(), r.ty.clone());
                    let len = |v: VarId, t: Type| {
                        Expr::new(ExprKind::Len(Box::new(Expr::var(v, t, span))), Type::Int(IntKind::I64), span)
                    };
                    let res = self.binary(*op, Expr::var(va, lt.clone(), span), Expr::var(vb, rt.clone(), span), span)?;
                    let ty = res.ty.clone();
                    let chk = Expr::new(
                        ExprKind::CheckSize(Box::new(len(va, lt)), Box::new(len(vb, rt)), Box::new(res)),
                        ty.clone(),
                        span,
                    );
                    let inner = self.let_(vb, r, chk, false, span);
                    return Ok(self.let_(va, l, inner, false, span));
                }
                self.binary(*op, l, r, span)
            }
            TExprKind::Pow(b, k) => {
                let b = self.expr(env, b)?;
                let k = self.expr(env, k)?;
                // `bigint` (D156): an exponent of any integer type
                if b.ty == Type::Big {
                    let k = self.to_big(k, span);
                    return Ok(Expr::new(ExprKind::StrOp(StrOp::Big(BigOp::Pow), vec![b, k]), Type::Big, span));
                }
                if b.ty.is_array() {
                    let et = b.ty.elem().clone();
                    let p = self.new_var("x", et.clone());
                    let ty = b.ty.clone();
                    let (kexpr, kvar) = self.share_scalar(k);
                    let body = Expr::new(
                        ExprKind::Pow(Box::new(Expr::var(p, et.clone(), span)), Box::new(kexpr)),
                        et,
                        span,
                    );
                    let m = self.map1(b, p, body, ty, span);
                    Ok(self.wrap_shared(kvar, m, span))
                } else {
                    let ty = b.ty.clone();
                    Ok(Expr::new(ExprKind::Pow(Box::new(b), Box::new(k)), ty, span))
                }
            }
            // `if (isPure(h))`: the branch of the function received, the
            // other never compiled (D386)
            TExprKind::If(c, t, f) if is_pure_test(c) => {
                let TExprKind::Call { args, .. } = &c.kind else { unreachable!() };
                let fv = self.fn_value(env, &args[0])?;
                self.expr(env, if self.fn_pure(&fv) { t } else { f })
            }
            TExprKind::If(c, t, f) => {
                let c = self.expr(env, c)?;
                let t = self.expr(env, t)?;
                let f = self.expr(env, f)?;
                // its own type: the branches may be arrays of two lengths,
                // fixed or not (D337)
                let ty = self.ty(env, &e.ty, span)?;
                Ok(Expr::new(ExprKind::If(Box::new(c), Box::new(t), Box::new(f)), ty, span))
            }
            TExprKind::Block(stmts, r) => self.block(env, stmts, r),
        }
    }

    /// `fill(n, v)` of a value that owns memory: `v` bound, copied into each
    /// element, then freed (D154); of an array, an array of arrays, built as
    /// `map(iota(n), (_) => v)` builds its rows (D337).
    fn fill_of(&mut self, n: Expr, v: Expr, ty: Type, span: Span) -> Expr {
        let t = self.new_var("filled", v.ty.clone());
        let (vt, sp) = (v.ty.clone(), v.span);
        let inner = if vt.is_array() {
            let i64t = Type::Int(IntKind::I64);
            let p = self.new_var("i", i64t.clone());
            let ids = Expr::new(ExprKind::Iota(Box::new(n)), Type::Array(Box::new(i64t), Fixed::ANY), span);
            Expr::new(ExprKind::Map(vec![ids], Lambda { params: vec![p], body: Box::new(Expr::var(t, vt, sp)), drop_params: vec![false] }), ty, span)
        } else {
            Expr::new(ExprKind::Fill(Box::new(n), Box::new(Expr::var(t, vt, sp))), ty, span)
        };
        self.let_(t, v, inner, false, span)
    }

    /// An integer as a `bigint` (D156).
    fn to_big(&mut self, x: Expr, span: Span) -> Expr {
        match &x.ty {
            Type::Big => x,
            Type::Int(k) if !k.signed() => {
                let x = if k.bits() < 64 { Expr::new(ExprKind::Convert(Box::new(x)), Type::Int(IntKind::U64), span) } else { x };
                Expr::new(ExprKind::StrOp(StrOp::Big(BigOp::FromU64), vec![x]), Type::Big, span)
            }
            _ => {
                let x = if x.ty != Type::Int(IntKind::I64) { Expr::new(ExprKind::Convert(Box::new(x)), Type::Int(IntKind::I64), span) } else { x };
                Expr::new(ExprKind::StrOp(StrOp::Big(BigOp::FromI64), vec![x]), Type::Big, span)
            }
        }
    }

    fn let_(&mut self, var: VarId, init: Expr, body: Expr, stmt: bool, span: Span) -> Expr {
        let ty = body.ty.clone();
        Expr::new(ExprKind::Let { var, init: Box::new(init), body: Box::new(body), stmt, drop: false }, ty, span)
    }

    /// `a ++ b` of arrays (D136): `map(iota(len(a) + len(b)), (i) -> if i <
    /// len(a) then a[i] else b[i - len(a)])`, both sides bound first.
    fn concat_arrays(&mut self, a: Expr, b: Expr, ty: Type, span: Span) -> Expr {
        let i64t = Type::Int(IntKind::I64);
        let (at, bt) = (a.ty.clone(), b.ty.clone());
        let va = self.new_var("a", at.clone());
        let vb = self.new_var("b", bt.clone());
        let len = |v: VarId, t: &Type| Expr::new(ExprKind::Len(Box::new(Expr::var(v, t.clone(), span))), Type::Int(IntKind::I64), span);
        let n = Expr::new(ExprKind::Binary(BinOp::Add, Box::new(len(va, &at)), Box::new(len(vb, &bt))), i64t.clone(), span);
        let idx = Expr::new(ExprKind::Iota(Box::new(n)), Type::Array(Box::new(i64t.clone()), crate::Fixed::ANY), span);
        let i = self.new_var("i", i64t.clone());
        let iv = || Expr::var(i, Type::Int(IntKind::I64), span);
        let et = ty.elem().clone();
        let first = Expr::new(ExprKind::Index(Box::new(Expr::var(va, at.clone(), span)), Box::new(iv())), et.clone(), span);
        let back = Expr::new(ExprKind::Binary(BinOp::Sub, Box::new(iv()), Box::new(len(va, &at))), i64t.clone(), span);
        let second = Expr::new(ExprKind::Index(Box::new(Expr::var(vb, bt.clone(), span)), Box::new(back)), et.clone(), span);
        let cond = Expr::new(ExprKind::Binary(BinOp::Lt, Box::new(iv()), Box::new(len(va, &at))), Type::Bool, span);
        let body = Expr::new(ExprKind::If(Box::new(cond), Box::new(first), Box::new(second)), et, span);
        let m = self.map1(idx, i, body, ty, span);
        let inner = self.let_(vb, b, m, false, span);
        self.let_(va, a, inner, false, span)
    }

    fn map1(&mut self, x: Expr, p: VarId, body: Expr, ty: Type, span: Span) -> Expr {
        let rty = Type::Array(Box::new(body.ty.clone()), crate::Fixed::ANY);
        let _ = ty;
        Expr::new(
            ExprKind::Map(vec![x], Lambda { params: vec![p], body: Box::new(body), drop_params: vec![false] }),
            rty,
            span,
        )
    }

    /// A scalar used inside a loop body: literals are used directly, other
    /// expressions are bound once outside the loop.
    fn share_scalar(&mut self, s: Expr) -> (Expr, Option<(VarId, Expr)>) {
        match s.kind {
            ExprKind::Lit(_) | ExprKind::Var(..) => (s, None),
            _ => {
                let v = self.new_var("s", s.ty.clone());
                let sp = s.span;
                let ty = s.ty.clone();
                (Expr::var(v, ty, sp), Some((v, s)))
            }
        }
    }

    fn wrap_shared(&mut self, shared: Option<(VarId, Expr)>, body: Expr, span: Span) -> Expr {
        match shared {
            None => body,
            Some((v, init)) => self.let_(v, init, body, false, span),
        }
    }

    /// A binary operator on IR values; arithmetic on arrays is element-wise.
    fn binary(&mut self, op: BinOp, l: Expr, r: Expr, span: Span) -> R<Expr> {
        let boolean = Type::Bool;
        // `bigint` (D156): its operations; equality by bytes, below
        if l.ty == Type::Big && !matches!(op, BinOp::Eq | BinOp::Ne | BinOp::And | BinOp::Or) {
            let (b, t) = match op {
                BinOp::Add => (BigOp::Add, Type::Big),
                BinOp::Sub => (BigOp::Sub, Type::Big),
                BinOp::Mul => (BigOp::Mul, Type::Big),
                BinOp::Div => (BigOp::Div, Type::Big),
                BinOp::Rem => (BigOp::Rem, Type::Big),
                BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => (BigOp::Cmp(op), boolean),
                // `min` and `max`: a comparison and a choice
                BinOp::Min | BinOp::Max => {
                    let (a, b) = (self.new_var("a", Type::Big), self.new_var("b", Type::Big));
                    let (av, bv) = (Expr::var(a, Type::Big, span), Expr::var(b, Type::Big, span));
                    let c = if op == BinOp::Min { BinOp::Le } else { BinOp::Ge };
                    let test = Expr::new(ExprKind::StrOp(StrOp::Big(BigOp::Cmp(c)), vec![av.clone(), bv.clone()]), Type::Bool, span);
                    let pick = Expr::new(ExprKind::If(Box::new(test), Box::new(av), Box::new(bv)), Type::Big, span);
                    let inner = self.let_(b, r, pick, false, span);
                    return Ok(self.let_(a, l, inner, false, span));
                }
                _ => return Err(Diagnostic::new(span, format!("`{}` does not apply to a `bigint`", op.text()))),
            };
            return Ok(Expr::new(ExprKind::StrOp(StrOp::Big(b), vec![l, r]), t, span));
        }
        match op {
            BinOp::And => Ok(Expr::new(
                ExprKind::If(Box::new(l), Box::new(r), Box::new(Expr::lit(Lit::Bool(false), Type::Bool, span))),
                boolean,
                span,
            )),
            BinOp::Or => Ok(Expr::new(
                ExprKind::If(Box::new(l), Box::new(Expr::lit(Lit::Bool(true), Type::Bool, span)), Box::new(r)),
                boolean,
                span,
            )),
            BinOp::Eq => self.equal(l, r, span),
            BinOp::Ne => {
                let e = self.equal(l, r, span)?;
                Ok(Expr::new(ExprKind::Unary(UnOp::Not, Box::new(e)), boolean, span))
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge if l.ty == Type::Str => {
                Ok(Expr::new(ExprKind::StrOp(StrOp::Cmp(op), vec![l, r]), boolean, span))
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                Ok(Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), boolean, span))
            }
            _ => {
                let (la, ra) = (l.ty.is_array(), r.ty.is_array());
                if la && ra {
                    let et = l.ty.elem().clone();
                    let p = self.new_var("x", et.clone());
                    let q = self.new_var("y", et.clone());
                    let body = Expr::new(
                        ExprKind::Binary(op, Box::new(Expr::var(p, et.clone(), span)), Box::new(Expr::var(q, et.clone(), span))),
                        et.clone(),
                        span,
                    );
                    let ty = l.ty.clone();
                    Ok(Expr::new(
                        ExprKind::Map(vec![l, r], Lambda { params: vec![p, q], body: Box::new(body), drop_params: vec![false, false] }),
                        ty,
                        span,
                    ))
                } else if la || ra {
                    let (arr, sc) = if la { (l, r) } else { (r, l) };
                    let et = arr.ty.elem().clone();
                    let p = self.new_var("x", et.clone());
                    let (s, shared) = self.share_scalar(sc);
                    let pv = Expr::var(p, et.clone(), span);
                    let (a, b) = if la { (pv, s) } else { (s, pv) };
                    let body = Expr::new(ExprKind::Binary(op, Box::new(a), Box::new(b)), et, span);
                    let ty = arr.ty.clone();
                    let m = self.map1(arr, p, body, ty, span);
                    Ok(self.wrap_shared(shared, m, span))
                } else {
                    let ty = l.ty.clone();
                    Ok(Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), ty, span))
                }
            }
        }
    }

    /// Value equality (section 5).
    fn equal(&mut self, l: Expr, r: Expr, span: Span) -> R<Expr> {
        let ty = l.ty.clone();
        let bt = Type::Bool;
        match &ty {
            Type::Int(_) | Type::F64 | Type::F32 | Type::Bool => {
                Ok(Expr::new(ExprKind::Binary(BinOp::Eq, Box::new(l), Box::new(r)), bt, span))
            }
            Type::Unit => {
                let a = self.new_var("_", Type::Unit);
                let b = self.new_var("_", Type::Unit);
                let t = Expr::lit(Lit::Bool(true), bt, span);
                let inner = self.let_(b, r, t, false, span);
                Ok(self.let_(a, l, inner, false, span))
            }
            // by bytes (D134); a `bigint` too: its bytes are canonical (D156)
            Type::Str | Type::Big => Ok(Expr::new(ExprKind::StrOp(StrOp::Cmp(BinOp::Eq), vec![l, r]), bt, span)),
            // structural, by a function of the code generator (D129); the
            // strings by bytes (D134)
            // a resource of the system is never compared (the checker
            // refuses it, D339)
            Type::Handle => internal(span, "a comparison of resources of the system"),
            Type::Rec(_) => {
                // both sides bound, then freed after the comparison, which
                // only reads them
                let a = self.new_var("a", ty.clone());
                let b = self.new_var("b", ty.clone());
                let cmp = Expr::new(
                    ExprKind::Binary(BinOp::Eq, Box::new(Expr::var(a, ty.clone(), span)), Box::new(Expr::var(b, ty.clone(), span))),
                    bt,
                    span,
                );
                let inner = self.let_(b, r, cmp, false, span);
                Ok(self.let_(a, l, inner, false, span))
            }
            Type::Tuple(ts) => {
                let a = self.new_var("a", ty.clone());
                let b = self.new_var("b", ty.clone());
                let mut acc = Expr::lit(Lit::Bool(true), bt.clone(), span);
                for (i, ct) in ts.iter().enumerate().rev() {
                    let pa = Expr::new(ExprKind::Proj(Box::new(Expr::var(a, ty.clone(), span)), i as u32), ct.clone(), span);
                    let pb = Expr::new(ExprKind::Proj(Box::new(Expr::var(b, ty.clone(), span)), i as u32), ct.clone(), span);
                    let eq = self.equal(pa, pb, span)?;
                    acc = Expr::new(
                        ExprKind::If(Box::new(eq), Box::new(acc), Box::new(Expr::lit(Lit::Bool(false), bt.clone(), span))),
                        bt.clone(),
                        span,
                    );
                }
                let inner = self.let_(b, r, acc, false, span);
                Ok(self.let_(a, l, inner, false, span))
            }
            Type::Array(et, _) => {
                let et = (**et).clone();
                let a = self.new_var("a", ty.clone());
                let b = self.new_var("b", ty.clone());
                let p = self.new_var("x", et.clone());
                let q = self.new_var("y", et.clone());
                let ev = self.equal(Expr::var(p, et.clone(), span), Expr::var(q, et.clone(), span), span)?;
                let m = Expr::new(
                    ExprKind::Map(
                        vec![Expr::var(a, ty.clone(), span), Expr::var(b, ty.clone(), span)],
                        Lambda { params: vec![p, q], body: Box::new(ev), drop_params: vec![false, false] },
                    ),
                    Type::Array(Box::new(bt.clone()), crate::Fixed::ANY),
                    span,
                );
                let x = self.new_var("p", bt.clone());
                let y = self.new_var("q", bt.clone());
                let and = Expr::new(
                    ExprKind::If(
                        Box::new(Expr::var(x, bt.clone(), span)),
                        Box::new(Expr::var(y, bt.clone(), span)),
                        Box::new(Expr::lit(Lit::Bool(false), bt.clone(), span)),
                    ),
                    bt.clone(),
                    span,
                );
                let red = Expr::new(
                    ExprKind::Reduce(
                        Box::new(m),
                        Lambda { params: vec![x, y], body: Box::new(and), drop_params: vec![false, false] },
                        Box::new(Expr::lit(Lit::Bool(true), bt.clone(), span)),
                    ),
                    bt.clone(),
                    span,
                );
                let i64t = Type::Int(IntKind::I64);
                let la = Expr::new(ExprKind::Len(Box::new(Expr::var(a, ty.clone(), span))), i64t.clone(), span);
                let lb = Expr::new(ExprKind::Len(Box::new(Expr::var(b, ty.clone(), span))), i64t, span);
                let same = Expr::new(ExprKind::Binary(BinOp::Eq, Box::new(la), Box::new(lb)), bt.clone(), span);
                let res = Expr::new(
                    ExprKind::If(Box::new(same), Box::new(red), Box::new(Expr::lit(Lit::Bool(false), bt.clone(), span))),
                    bt,
                    span,
                );
                let inner = self.let_(b, r, res, false, span);
                Ok(self.let_(a, l, inner, false, span))
            }
        }
    }

    fn block(&mut self, env: &mut Env<'p>, stmts: &'p [TStmt], r: &'p TExpr) -> R<Expr> {
        if stmts.is_empty() {
            return self.expr(env, r);
        }
        let stmt = !env.pure;
        match &stmts[0] {
            TStmt::Let(pat, v) => {
                if holds_fn(&v.ty) {
                    return match pat {
                        // the components of a tuple of functions (D114)
                        TPat::Tuple(ps, _) => {
                            let fv = self.fn_value(env, v)?;
                            let FuncVal::Tuple(cs) = fv else {
                                return poc(v.span, "destructuring a function chosen at run time");
                            };
                            for (q, c) in ps.iter().zip(cs) {
                                match q {
                                    TPat::Bind(l) => {
                                        env.locals.insert(*l, Binding::Func(c));
                                    }
                                    TPat::Wild(_) => {}
                                    _ => return poc(v.span, "a nested pattern of functions"),
                                }
                            }
                            self.block(env, &stmts[1..], r)
                        }
                        TPat::Bind(l) => {
                            let fv = self.fn_value(env, v)?;
                            // its conditions computed once, here (D107)
                            let mut tags = Vec::new();
                            let fv = hoist_choices(self, fv, &mut tags);
                            env.locals.insert(*l, Binding::Func(fv));
                            let mut out = self.block(env, &stmts[1..], r)?;
                            for (t, c) in tags.into_iter().rev() {
                                let sp = c.span;
                                out = self.let_(t, c, out, stmt, sp);
                            }
                            Ok(out)
                        }
                        TPat::Wild(_) => self.block(env, &stmts[1..], r),
                        TPat::Lit(..) | TPat::Ctor { .. } => poc(v.span, "destructuring a function"),
                    };
                }
                let init = self.expr(env, v)?;
                let mut q = VecDeque::new();
                q.push_back((pat, init));
                let rest = &stmts[1..];
                self.bind_list(env, q, stmt, &mut |this: &mut Self, env: &mut Env<'p>| this.block(env, rest, r))
            }
            TStmt::Expr(v) => {
                let init = self.expr(env, v)?;
                let var = self.new_var("_", init.ty.clone());
                let body = self.block(env, &stmts[1..], r)?;
                Ok(self.let_(var, init, body, true, v.span))
            }
        }
    }

    fn bind_list(
        &mut self,
        env: &mut Env<'p>,
        mut q: VecDeque<(&'p TPat, Expr)>,
        stmt: bool,
        k: &mut dyn FnMut(&mut Self, &mut Env<'p>) -> R<Expr>,
    ) -> R<Expr> {
        let (pat, val) = match q.pop_front() {
            None => return k(self, env),
            Some(x) => x,
        };
        let span = val.span;
        match pat {
            TPat::Bind(l) => {
                let local = &self.tfn(env).locals[*l as usize];
                let v = self.new_var(&local.name, val.ty.clone());
                env.locals.insert(*l, Binding::Var(v));
                let lt = local.ty.clone();
                let ty = val.ty.clone();
                self.bind_sizes(env, &lt, Expr::var(v, ty, span));
                let body = self.bind_list(env, q, stmt, k)?;
                Ok(self.let_(v, val, body, stmt, span))
            }
            // `_` of a component of a tuple binds nothing: the projection
            // has no effect, and a binding would keep the tuple alive to
            // the end of the scope (D117)
            TPat::Wild(_) if matches!(&val.kind, ExprKind::Proj(b, _) if matches!(b.kind, ExprKind::Var(..))) => {
                self.bind_list(env, q, stmt, k)
            }
            TPat::Wild(_) | TPat::Lit(..) => {
                let v = self.new_var("_", val.ty.clone());
                let body = self.bind_list(env, q, stmt, k)?;
                Ok(self.let_(v, val, body, stmt, span))
            }
            TPat::Ctor { variant, subs, ty, .. } => {
                // the payload of the variant: its slots of the IR tuple (of the
                // node, for a recursive type)
                let (comps, slots) = self.sum_of(env, ty, span)?;
                let rec = self.is_rec(env, ty);
                let t = self.new_var("sum", val.ty.clone());
                let fields = self.fn_fields(env, ty, *variant);
                for (k, (p, &slot)) in subs.iter().zip(slots[*variant as usize].iter()).enumerate().rev() {
                    let mut base = Expr::var(t, val.ty.clone(), span);
                    if rec {
                        base = Expr::new(ExprKind::Unbox(Box::new(base)), Type::Tuple(comps.clone()), span);
                    }
                    let pr = Expr::new(ExprKind::Proj(Box::new(base), slot), comps[slot as usize].clone(), span);
                    // a function: a branch on its code at each call (D153)
                    if let Some(key) = fields.get(k).cloned().flatten() {
                        match p {
                            TPat::Bind(l) => {
                                // its code: a field of the value, read at
                                // each call
                                let fv = self.stored_fn(&key, pr, span);
                                env.locals.insert(*l, Binding::Func(fv));
                            }
                            TPat::Wild(_) => {}
                            _ => return poc(span, "a pattern on a function stored in a value"),
                        }
                        continue;
                    }
                    q.push_front((p, pr));
                }
                let body = self.bind_list(env, q, stmt, k)?;
                Ok(self.let_(t, val, body, stmt, span))
            }
            TPat::Tuple(ps, _) => {
                if let ExprKind::Tuple(es) = val.kind {
                    for (p, e) in ps.iter().zip(es.into_iter()).rev() {
                        q.push_front((p, e));
                    }
                    return self.bind_list(env, q, stmt, k);
                }
                let t = self.new_var("tuple", val.ty.clone());
                let ts = match &val.ty {
                    Type::Tuple(ts) => ts.clone(),
                    _ => return internal(span, "tuple pattern on a non-tuple"),
                };
                for (i, p) in ps.iter().enumerate().rev() {
                    let pr = Expr::new(
                        ExprKind::Proj(Box::new(Expr::var(t, val.ty.clone(), span)), i as u32),
                        ts[i].clone(),
                        span,
                    );
                    q.push_front((p, pr));
                }
                let body = self.bind_list(env, q, stmt, k)?;
                Ok(self.let_(t, val, body, stmt, span))
            }
        }
    }

    /// The function `f` has no effect (D386): a `function`, a lambda without
    /// one, a primitive that is not an action, an operator; a choice or a
    /// table of such functions.
    fn fn_pure(&self, f: &FuncVal<'p>) -> bool {
        match f {
            FuncVal::Lambda { act, .. } => !act,
            FuncVal::Global { id, .. } => !self.p.fns[*id as usize].is_action,
            FuncVal::Prim(p) => !p.is_action(),
            FuncVal::Op(_) => true,
            FuncVal::Choice { then, other, .. } => self.fn_pure(then) && self.fn_pure(other),
            FuncVal::Tuple(fs) | FuncVal::Array(fs) | FuncVal::Table { fns: fs, .. } => fs.iter().all(|f| self.fn_pure(f)),
            FuncVal::Bound { inner, .. } => self.fn_pure(inner),
        }
    }

    fn fn_value(&mut self, env: &mut Env<'p>, e: &'p TExpr) -> R<FuncVal<'p>> {
        match &e.kind {
            TExprKind::Lambda(ps, body) => {
                let mut snap = env.clone();
                snap.pure = true;
                Ok(FuncVal::Lambda { params: ps, body, env: Rc::new(snap), act: !matches!(e.ty, Ty::Fn(_, _, false)) })
            }
            TExprKind::Local(l) => match env.locals.get(l) {
                Some(Binding::Func(f)) => Ok(f.clone()),
                _ => internal(e.span, "local is not a function"),
            },
            TExprKind::FnRef { id, type_args } => {
                Ok(FuncVal::Global { id: *id, type_args: type_args.iter().map(|t| self.conc(env, t)).collect() })
            }
            // `min` and `max` as values: the function of two numbers
            TExprKind::PrimRef(Prim::Min) => Ok(FuncVal::Op(BinOp::Min)),
            TExprKind::PrimRef(Prim::Max) => Ok(FuncVal::Op(BinOp::Max)),
            TExprKind::PrimRef(p) => Ok(FuncVal::Prim(*p)),
            TExprKind::OpRef(op) => Ok(FuncVal::Op(*op)),
            // structures of functions (D114)
            TExprKind::Tuple(es) => {
                let mut out = Vec::new();
                for x in es {
                    out.push(self.fn_value(env, x)?);
                }
                Ok(FuncVal::Tuple(out))
            }
            TExprKind::Array(es) => {
                let mut out = Vec::new();
                for x in es {
                    out.push(self.fn_value(env, x)?);
                }
                Ok(FuncVal::Array(out))
            }
            TExprKind::Field(x, i) => match self.fn_value(env, x)? {
                FuncVal::Tuple(cs) => Ok(cs[*i as usize].clone()),
                _ => internal(e.span, "a field of a value that is not a structure of functions"),
            },
            TExprKind::Index(a, i) => match self.fn_value(env, a)? {
                FuncVal::Array(fns) => {
                    let index = self.expr(env, i)?;
                    // a choice at run time, in the memory report (section 11)
                    self.choices.push(e.span);
                    Ok(FuncVal::Table { fns, index: Box::new(index) })
                }
                _ => internal(e.span, "an index of a value that is not an array of functions"),
            },
            // a function chosen at run time: a branch at each call (D107)
            TExprKind::If(c, t, f) => {
                let cond = self.expr(env, c)?;
                let then = self.fn_value(env, t)?;
                let other = self.fn_value(env, f)?;
                self.choices.push(e.span);
                Ok(FuncVal::Choice { cond: Box::new(cond), then: Box::new(then), other: Box::new(other) })
            }
            // a function chosen by `match`: the choices of `if`, one per arm,
            // the first arm that matches (D127)
            TExprKind::Match(x, arms) => {
                fn binds(p: &TPat) -> bool {
                    match p {
                        TPat::Bind(_) => true,
                        TPat::Tuple(ps, _) => ps.iter().any(binds),
                        TPat::Ctor { subs, .. } => subs.iter().any(binds),
                        TPat::Wild(_) | TPat::Lit(..) => false,
                    }
                }
                let val = self.expr(env, x)?;
                let mut tests = Vec::new();
                let mut fvs = Vec::new();
                for (pat, body) in arms {
                    tests.push(self.pattern_test(env, pat, &val, e.span)?);
                    if !binds(pat) {
                        fvs.push(self.fn_value(env, body)?);
                        continue;
                    }
                    // the variables of the pattern, at any depth, read from
                    // the value whatever its variant (the slots of the
                    // others are zero), bound where the function is bound
                    // (D132, D152)
                    let mut env2 = env.clone();
                    let mut bvals: Vec<(VarId, Expr)> = Vec::new();
                    let mut found = Vec::new();
                    self.pattern_values(env, pat, val.clone(), e.span, false, &mut found)?;
                    let test = tests.last().cloned().flatten();
                    for (l, x, below, slot) in found {
                        // a part read through a node of a recursive type
                        // below the first level exists only when the arm
                        // matches: read it then, else its zero
                        let x = match (&test, below) {
                            (Some(c), true) => {
                                let t = x.ty.clone();
                                let sp = x.span;
                                Expr::new(ExprKind::If(Box::new(c.clone()), Box::new(x), Box::new(Expr::new(ExprKind::Zero, t.clone(), sp))), t, sp)
                            }
                            _ => x,
                        };
                        // a function stored in the value (D153)
                        if let Some(key) = slot {
                            let fv = self.stored_fn(&key, x, e.span);
                            env2.locals.insert(l, Binding::Func(fv));
                            continue;
                        }
                        let name = self.tfn(&env2).locals[l as usize].name.clone();
                        let v = self.new_var(&name, x.ty.clone());
                        env2.locals.insert(l, Binding::Var(v));
                        bvals.push((v, x));
                    }
                    let inner = self.fn_value(&mut env2, body)?;
                    fvs.push(FuncVal::Bound { binds: bvals, inner: Box::new(inner) });
                }
                // from the last arm back: an arm that matches every value
                // ends the choice
                let mut acc = fvs.pop().unwrap();
                tests.pop();
                while let Some(f) = fvs.pop() {
                    acc = match tests.pop().unwrap() {
                        None => f,
                        Some(cond) => FuncVal::Choice { cond: Box::new(cond), then: Box::new(f), other: Box::new(acc) },
                    };
                }
                self.choices.push(e.span);
                Ok(acc)
            }
            // a function returned by a function (D152): the callee is
            // inlined, its arguments and its `const`s computed once
            TExprKind::Call { callee: Callee::Fn { id, type_args }, args, size_checks } if !self.p.recursive[*id] && size_checks.is_empty() => {
                let ta: Vec<Ty> = type_args.iter().map(|t| self.conc(env, t)).collect();
                let a = self.args(env, args)?;
                let (mut env2, lets, _, size_params) = self.callee_env_full(*id, ta, a, e.span);
                self.hoisted.extend(lets);
                // a size is positive or zero (SPEC section 4)
                for k in size_params {
                    let unit = Expr::lit(Lit::Unit, Type::Unit, e.span);
                    let check = Expr::new(ExprKind::CheckNonNegative(Box::new(k), Box::new(unit)), Type::Unit, e.span);
                    let v = self.new_var("_", Type::Unit);
                    self.hoisted.push((v, check));
                }
                let f = &self.p.fns[*id];
                self.fn_value(&mut env2, &f.body)
            }
            TExprKind::Call { callee: Callee::Fn { id, .. }, .. } if self.p.recursive[*id] => {
                poc(e.span, "a function returned by a recursive function (SPEC section 11)")
            }
            TExprKind::Block(stmts, last) => {
                let mut env2 = env.clone();
                for st in stmts.iter() {
                    match st {
                        TStmt::Let(pat, v) if holds_fn(&v.ty) => match pat {
                            TPat::Bind(l) => {
                                let fv = self.fn_value(&mut env2, v)?;
                                env2.locals.insert(*l, Binding::Func(fv));
                            }
                            TPat::Wild(_) => {}
                            _ => return poc(v.span, "destructuring a structure of functions in a function that returns a function"),
                        },
                        TStmt::Let(pat, v) => {
                            let init = self.expr(&mut env2, v)?;
                            self.hoist_pattern(&mut env2, pat, init, v.span)?;
                        }
                        TStmt::Expr(v) => return poc(v.span, "a statement in a function that returns a function"),
                    }
                }
                self.fn_value(&mut env2, last)
            }
            _ => poc(e.span, "a function computed by an expression"),
        }
    }

    /// The variables of a pattern and the parts of `val` they name, at any
    /// depth (D152): projections of tuples and of the slots of variants.
    /// `below`: the value is read through a node of a recursive type that
    /// may be absent (a part of a part).
    #[allow(clippy::type_complexity)]
    fn pattern_values(&mut self, env: &Env<'p>, pat: &'p TPat, val: Expr, span: Span, below: bool, out: &mut Vec<(LocalId, Expr, bool, Option<SlotKey>)>) -> R<()> {
        self.pattern_values_in(env, pat, val, span, below, None, out)
    }

    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    fn pattern_values_in(
        &mut self,
        env: &Env<'p>,
        pat: &'p TPat,
        val: Expr,
        span: Span,
        below: bool,
        slot: Option<SlotKey>,
        out: &mut Vec<(LocalId, Expr, bool, Option<SlotKey>)>,
    ) -> R<()> {
        match pat {
            TPat::Bind(l) => out.push((*l, val, below, slot)),
            TPat::Wild(_) | TPat::Lit(..) => {}
            TPat::Tuple(ps, _) => {
                let ts = match &val.ty {
                    Type::Tuple(ts) => ts.clone(),
                    _ => return internal(span, "a tuple pattern on a value that is not a tuple"),
                };
                for (k, q) in ps.iter().enumerate() {
                    let pr = Expr::new(ExprKind::Proj(Box::new(val.clone()), k as u32), ts[k].clone(), span);
                    self.pattern_values_in(env, q, pr, span, below, None, out)?;
                }
            }
            TPat::Ctor { variant, subs, ty, .. } => {
                let (comps, slots) = self.sum_of(env, ty, span)?;
                let fields = self.fn_fields(env, ty, *variant);
                let mut base = val;
                let mut below = below;
                if self.is_rec(env, ty) {
                    // the matched value itself is a node; a part of it may
                    // be the zero of another variant
                    below = below || !matches!(base.kind, ExprKind::Var(..));
                    base = Expr::new(ExprKind::Unbox(Box::new(base)), Type::Tuple(comps.clone()), span);
                }
                for (k, (q, &slot)) in subs.iter().zip(slots[*variant as usize].iter()).enumerate() {
                    let pr = Expr::new(ExprKind::Proj(Box::new(base.clone()), slot), comps[slot as usize].clone(), span);
                    let key = fields.get(k).cloned().flatten();
                    self.pattern_values_in(env, q, pr, span, below, key, out)?;
                }
            }
        }
        Ok(())
    }

    /// The variables of a `const` of a function that returns a function
    /// (D152): computed once, hoisted; a variable or a tuple of variables.
    fn hoist_pattern(&mut self, env: &mut Env<'p>, pat: &'p TPat, init: Expr, span: Span) -> R<()> {
        match pat {
            TPat::Wild(_) => Ok(()),
            TPat::Bind(l) => {
                let name = self.tfn(env).locals[*l as usize].name.clone();
                let v = self.new_var(&name, init.ty.clone());
                env.locals.insert(*l, Binding::Var(v));
                self.hoisted.push((v, init));
                Ok(())
            }
            TPat::Tuple(ps, _) => {
                let ts = match &init.ty {
                    Type::Tuple(ts) => ts.clone(),
                    _ => return internal(span, "a tuple pattern on a value that is not a tuple"),
                };
                let t = self.new_var("t", init.ty.clone());
                let tty = init.ty.clone();
                self.hoisted.push((t, init));
                for (k, q) in ps.iter().enumerate() {
                    let pr = Expr::new(ExprKind::Proj(Box::new(Expr::var(t, tty.clone(), span)), k as u32), ts[k].clone(), span);
                    self.hoist_pattern(env, q, pr, span)?;
                }
                Ok(())
            }
            TPat::Lit(..) | TPat::Ctor { .. } => poc(span, "this pattern in a function that returns a function"),
        }
    }

    fn args(&mut self, env: &mut Env<'p>, args: &'p [TExpr]) -> R<Vec<Arg<'p>>> {
        let mut out = Vec::new();
        for a in args {
            if holds_fn(&a.ty) {
                out.push(Arg::Func(self.fn_value(env, a)?));
            } else {
                out.push(Arg::Val(self.expr(env, a)?));
            }
        }
        Ok(out)
    }

    fn apply(&mut self, fv: &FuncVal<'p>, args: Vec<Arg<'p>>, ret: &Type, span: Span) -> R<Expr> {
        match fv {
            FuncVal::Lambda { params, body, env, .. } => {
                let mut env2: Env<'p> = (**env).clone();
                let mut lets = Vec::new();
                for (p, a) in params.iter().zip(args.into_iter()) {
                    match a {
                        Arg::Val(x) => {
                            let local = &self.p.fns[env2.fn_id].locals[*p as usize];
                            let v = self.new_var(&local.name, x.ty.clone());
                            env2.locals.insert(*p, Binding::Var(v));
                            let lt = local.ty.clone();
                            let xt = x.ty.clone();
                            self.bind_sizes(&mut env2, &lt, Expr::var(v, xt, span));
                            lets.push((v, x));
                        }
                        Arg::Func(f) => {
                            env2.locals.insert(*p, Binding::Func(f));
                        }
                    }
                }
                let mut b = self.expr(&mut env2, body)?;
                for (v, x) in lets.into_iter().rev() {
                    b = self.let_(v, x, b, false, span);
                }
                Ok(b)
            }
            FuncVal::Global { id, type_args } => self.inline_fn(*id, type_args.clone(), args, &[], span),
            // the variables of its arm bound around the call (D132)
            FuncVal::Bound { binds, inner } => {
                let mut b = self.apply(inner, args, ret, span)?;
                for (v, x) in binds.iter().rev() {
                    b = self.let_(*v, x.clone(), b, false, span);
                }
                Ok(b)
            }
            // the function at a run-time index: the index checked (the
            // bounds of the array: `iota(n)[index]`, fused, no array), then a
            // branch per function (D114)
            // no function stored in this slot (yet, or ever): unreachable
            FuncVal::Table { fns, .. } if fns.is_empty() => Ok(Expr::new(ExprKind::Zero, ret.clone(), span)),
            FuncVal::Table { fns, index } => {
                let n = fns.len() as i64;
                let i64t = Type::Int(IntKind::I64);
                let iota = Expr::new(ExprKind::Iota(Box::new(Expr::i64(n, span))), Type::Array(Box::new(i64t.clone()), crate::Fixed::ANY), span);
                let idx = Expr::new(ExprKind::Convert(index.clone()), i64t.clone(), index.span);
                let tag_e = Expr::new(ExprKind::Index(Box::new(iota), Box::new(idx)), i64t.clone(), index.span);
                let tag = self.new_var("tag", i64t.clone());
                let mut lets = vec![(tag, tag_e)];
                let mut copies: Vec<Vec<Arg<'p>>> = (0..fns.len()).map(|_| Vec::new()).collect();
                for a in args {
                    match a {
                        Arg::Val(x) => {
                            let ty = x.ty.clone();
                            let v = self.new_var("arg", ty.clone());
                            lets.push((v, x));
                            for c in copies.iter_mut() {
                                c.push(Arg::Val(Expr::var(v, ty.clone(), span)));
                            }
                        }
                        Arg::Func(f) => {
                            for c in copies.iter_mut() {
                                c.push(Arg::Func(f.clone()));
                            }
                        }
                    }
                }
                // from the last function back to the first
                let mut copies = copies.into_iter().rev();
                let last = copies.next().unwrap_or_default();
                let mut out = self.apply(&fns[fns.len() - 1], last, ret, span)?;
                for (k, c) in copies.enumerate() {
                    let j = fns.len() - 2 - k;
                    let call = self.apply(&fns[j], c, ret, span)?;
                    let is = Expr::new(
                        ExprKind::Binary(BinOp::Eq, Box::new(Expr::var(tag, i64t.clone(), span)), Box::new(Expr::i64(j as i64, span))),
                        Type::Bool,
                        span,
                    );
                    out = Expr::new(ExprKind::If(Box::new(is), Box::new(call), Box::new(out)), ret.clone(), span);
                }
                for (v, x) in lets.into_iter().rev() {
                    out = self.let_(v, x, out, false, span);
                }
                Ok(out)
            }
            FuncVal::Tuple(_) | FuncVal::Array(_) => internal(span, "a structure of functions called"),
            // the arguments once, then a branch on the condition (D107)
            FuncVal::Choice { cond, then, other } => {
                let mut lets = Vec::new();
                let mut a1 = Vec::new();
                let mut a2 = Vec::new();
                for a in args {
                    match a {
                        Arg::Val(x) => {
                            let ty = x.ty.clone();
                            let v = self.new_var("arg", ty.clone());
                            lets.push((v, x));
                            a1.push(Arg::Val(Expr::var(v, ty.clone(), span)));
                            a2.push(Arg::Val(Expr::var(v, ty, span)));
                        }
                        Arg::Func(f) => {
                            a1.push(Arg::Func(f.clone()));
                            a2.push(Arg::Func(f));
                        }
                    }
                }
                let t = self.apply(then, a1, ret, span)?;
                let f = self.apply(other, a2, ret, span)?;
                let mut out = Expr::new(ExprKind::If(cond.clone(), Box::new(t), Box::new(f)), ret.clone(), span);
                for (v, x) in lets.into_iter().rev() {
                    out = self.let_(v, x, out, false, span);
                }
                Ok(out)
            }
            FuncVal::Prim(Prim::Math(f)) => {
                let mut xs = Vec::new();
                for a in args {
                    match a {
                        Arg::Val(x) => xs.push(x),
                        Arg::Func(_) => return internal(span, "bad primitive argument"),
                    }
                }
                Ok(Expr::new(ExprKind::Math(*f, xs), ret.clone(), span))
            }
            // the functions of strings used as values (D135)
            FuncVal::Prim(
                p @ (Prim::Trim
                | Prim::StrLen
                | Prim::Concat
                | Prim::ToString
                | Prim::StringOf
                | Prim::Join
                | Prim::ToUpper
                | Prim::ToLower
                | Prim::TrimStart
                | Prim::TrimEnd),
            ) => {
                let op = match p {
                    Prim::Trim => StrOp::Trim,
                    Prim::ToUpper => StrOp::ToUpper,
                    Prim::ToLower => StrOp::ToLower,
                    Prim::TrimStart => StrOp::TrimStart,
                    Prim::TrimEnd => StrOp::TrimEnd,
                    Prim::StrLen => StrOp::Len,
                    Prim::Concat => StrOp::Concat,
                    Prim::ToString => StrOp::ToString,
                    Prim::StringOf => StrOp::FromBytes,
                    _ => StrOp::Join,
                };
                let mut xs = Vec::new();
                for a in args {
                    match a {
                        Arg::Val(x) => xs.push(x),
                        Arg::Func(_) => return internal(span, "bad primitive argument"),
                    }
                }
                // `toString` of a `bigint` (D156)
                let op = if op == StrOp::ToString && xs.first().is_some_and(|x| x.ty == Type::Big) { StrOp::Big(BigOp::ToString) } else { op };
                Ok(Expr::new(ExprKind::StrOp(op, xs), ret.clone(), span))
            }
            FuncVal::Prim(p) => {
                let x = match args.into_iter().next() {
                    Some(Arg::Val(x)) => x,
                    _ => return internal(span, "bad primitive argument"),
                };
                match p {
                    Prim::Sqrt => {
                        let t = x.ty.clone();
                        Ok(Expr::new(ExprKind::Sqrt(Box::new(x)), t, span))
                    }
                    Prim::Convert(_) => Ok(Expr::new(ExprKind::Convert(Box::new(x)), ret.clone(), span)),
                    // parsing as a function (D156): `lines(t).map(parseF64)`
                    Prim::ParseI64 => Ok(Expr::new(ExprKind::Action(ActionOp::ParseI64, vec![x]), ret.clone(), span)),
                    Prim::ParseF64 => Ok(Expr::new(ExprKind::Action(ActionOp::ParseF64, vec![x]), ret.clone(), span)),
                    Prim::ParseFloat => Ok(Expr::new(ExprKind::StrOp(StrOp::JsParseFloat, vec![x]), ret.clone(), span)),
                    Prim::JsNumber => Ok(Expr::new(ExprKind::StrOp(StrOp::JsNumber, vec![x]), ret.clone(), span)),
                    _ => internal(span, "primitive used as a value"),
                }
            }
            FuncVal::Op(op) => {
                let mut it = args.into_iter();
                match (it.next(), it.next()) {
                    (Some(Arg::Val(a)), Some(Arg::Val(b))) => self.binary(*op, a, b, span),
                    _ => internal(span, "bad operator arguments"),
                }
            }
        }
    }

    fn inline_fn(&mut self, id: FnId, type_args: Vec<Ty>, args: Vec<Arg<'p>>, checks: &[usize], span: Span) -> R<Expr> {
        if has_alloc_requirement(&self.p.fns[id]) && !self.required.iter().any(|(i, t)| *i == id && *t == type_args) {
            self.required.push((id, type_args.clone()));
        }
        // a recursive function is not inlined (SPEC section 11)
        if self.p.recursive[id] {
            return self.call_recursive(id, type_args, args, span);
        }
        let f = &self.p.fns[id];
        let (env, lets, pvars, size_params) = self.callee_env_full(id, type_args, args, span);
        let mut env = env;
        let mut body = self.expr(&mut env, &f.body)?;
        // a size is positive or zero (SPEC section 4)
        for k in size_params.into_iter().rev() {
            let ty = body.ty.clone();
            body = Expr::new(ExprKind::CheckNonNegative(Box::new(k), Box::new(body)), ty, span);
        }
        for &i in checks.iter().rev() {
            let Some((v, t)) = &pvars[i] else { continue };
            // the arrays of the parameter, in records and tuples too (D336)
            let mut arrays = Vec::new();
            array_paths(&f.param_tys[i], &mut Vec::new(), &mut arrays);
            for (path, p) in arrays.into_iter().rev() {
                if binds(&f.syms, i, &path, &p) {
                    continue;
                }
                let d = self.size(&env, &p, span)?;
                let actual = Expr::new(ExprKind::Len(Box::new(proj_path(*v, t, &path, span))), Type::Int(IntKind::I64), span);
                let ty = body.ty.clone();
                body = Expr::new(ExprKind::CheckSize(Box::new(d), Box::new(actual), Box::new(body)), ty, span);
            }
        }
        for (v, x) in lets.into_iter().rev() {
            body = self.let_(v, x, body, false, span);
        }
        let f = &self.p.fns[id];
        self.reassoc_body(f, &mut body);
        // a function of the prelude has no place in the program's file: its
        // positions (a panic inside it, D111) are those of the call
        if f.from_prelude {
            relocate_prelude(&mut body, span);
        }
        Ok(body)
    }

    /// The environment of an inlined call: its parameters bound to the
    /// arguments, the values in new variables (to bind around the body),
    /// the variables of the parameters, the `size` parameters.
    #[allow(clippy::type_complexity)]
    fn callee_env_full(
        &mut self,
        id: FnId,
        type_args: Vec<Ty>,
        args: Vec<Arg<'p>>,
        span: Span,
    ) -> (Env<'p>, Vec<(VarId, Expr)>, Vec<Option<(VarId, Type)>>, Vec<Expr>) {
        let f = &self.p.fns[id];
        let mut env = Env {
            fn_id: id,
            type_args: Rc::new(type_args),
            locals: HashMap::new(),
            sizes: HashMap::new(),
            pure: !f.is_action,
        };
        let mut lets = Vec::new();
        let mut pvars = Vec::new();
        for (i, a) in args.into_iter().enumerate() {
            match a {
                Arg::Val(x) => {
                    let name = f.locals[f.params[i] as usize].name.clone();
                    let v = self.new_var(&name, x.ty.clone());
                    env.locals.insert(f.params[i], Binding::Var(v));
                    pvars.push(Some((v, x.ty.clone())));
                    lets.push((v, x));
                }
                Arg::Func(fv) => {
                    env.locals.insert(f.params[i], Binding::Func(fv));
                    pvars.push(None);
                }
            }
        }
        let mut size_params = Vec::new();
        for (s, info) in f.syms.iter().enumerate() {
            if let tast::SymOrigin::Param(i) = info.origin {
                if let Some((v, t)) = &pvars[i] {
                    let len = Expr::new(ExprKind::Len(Box::new(Expr::var(*v, t.clone(), span))), Type::Int(IntKind::I64), span);
                    env.sizes.insert(s as SymId, len);
                }
            }
            if let tast::SymOrigin::ParamPath(i, path) = &info.origin {
                if let (Some((v, t)), true) = (&pvars[*i], tast::path_has_value(path)) {
                    let len = Expr::new(ExprKind::Len(Box::new(proj_path(*v, t, path, span))), Type::Int(IntKind::I64), span);
                    env.sizes.insert(s as SymId, len);
                }
            }
            if let tast::SymOrigin::SizeParam(i) = info.origin {
                if let Some((v, t)) = &pvars[i] {
                    env.sizes.insert(s as SymId, Expr::var(*v, t.clone(), span));
                    size_params.push(Expr::var(*v, t.clone(), span));
                }
            }
        }
        (env, lets, pvars, size_params)
    }

    fn call(
        &mut self,
        env: &mut Env<'p>,
        callee: &'p Callee,
        args: &'p [TExpr],
        size_checks: &[usize],
        e: &'p TExpr,
    ) -> R<Expr> {
        let span = e.span;
        match callee {
            Callee::Fn { id, type_args } => {
                let ta: Vec<Ty> = type_args.iter().map(|t| self.conc(env, t)).collect();
                let a = self.args(env, args)?;
                self.inline_fn(*id, ta, a, size_checks, span)
            }
            Callee::Value(f) => {
                let fv = self.fn_value(env, f)?;
                let a = self.args(env, args)?;
                let ret = self.ty(env, &e.ty, span)?;
                self.apply(&fv, a, &ret, span)
            }
            Callee::Prim(p) => self.prim(env, *p, args, e),
            // a C function (D125): an action, in its place, never removed
            Callee::Extern(i) => {
                let ty = self.ty(env, &e.ty, span)?;
                let mut out = Vec::new();
                for a in args {
                    out.push(self.expr(env, a)?);
                }
                Ok(Expr::new(ExprKind::Action(ActionOp::Extern(*i), out), ty, span))
            }
        }
    }

    /// A call of a recursive function: its specialization for `type_args`,
    /// lowered once (D102).
    fn call_recursive(&mut self, id: FnId, type_args: Vec<Ty>, args: Vec<Arg<'p>>, span: Span) -> R<Expr> {
        // a function argument: part of the specialization (D128)
        let mut vals = Vec::new();
        let mut funcs = Vec::new();
        let mut keys = Vec::new();
        let mut captured = Vec::new();
        for a in args {
            match a {
                Arg::Val(x) => {
                    vals.push(x);
                    funcs.push(None);
                    keys.push(None);
                }
                Arg::Func(f) => {
                    let Some(k) = fn_key(&f) else {
                        return poc(span, "a recursive function given a function that reads, of its context, a size or a function that reads values of its own (SPEC section 11)");
                    };
                    // the values it captures, the sizes it reads, then those
                    // of the functions it reads: arguments after the others
                    for c in captures(&f) {
                        match c {
                            Capture::Var(v) => {
                                let t = self.vars[v as usize].ty.clone();
                                captured.push(Expr::var(v, t, span));
                            }
                            Capture::Size(x) => captured.push(x),
                        }
                    }
                    keys.push(Some(k));
                    funcs.push(Some(f));
                }
            }
        }
        vals.extend(captured);
        let key = (id, type_args, keys);
        let idx = match self.specs.get(&key) {
            Some(&i) => i,
            None => self.lower_fn(key.0, key.1.clone(), funcs, key.2.clone(), span)?,
        };
        let ret = self.rets[idx as usize].clone();
        Ok(Expr::new(ExprKind::Call(idx, vals), ret, span))
    }

    /// `f` reading what it read of its context from new parameters of the
    /// specialization (pushed onto `params`), in the order of `captures`:
    /// its values, its sizes, then the functions it reads, made again the
    /// same way (D131, D152, D563).
    fn rebind(&mut self, f: &FuncVal<'p>, params: &mut Vec<VarId>, pvars: &mut Vec<Option<VarId>>, span: Span) -> FuncVal<'p> {
        let FuncVal::Lambda { params: lp, body: lb, env: le, act: la } = f else { return f.clone() };
        let mut bound: HashSet<LocalId> = lp.iter().copied().collect();
        let mut free = Vec::new();
        let mut sizes = Vec::new();
        free_locals(lb, &mut bound, &mut free, &mut sizes);
        let mut inner = (**le).clone();
        for l in &free {
            if let Some(Binding::Var(cv)) = le.locals.get(l) {
                let t = self.vars[*cv as usize].ty.clone();
                let name = self.vars[*cv as usize].name.clone();
                let p = self.new_var(&name, t);
                inner.locals.insert(*l, Binding::Var(p));
                params.push(p);
                pvars.push(None);
            }
        }
        for s in &sizes {
            if le.sizes.contains_key(s) {
                let p = self.new_var("size", Type::Int(IntKind::I64));
                inner.sizes.insert(*s, Expr::var(p, Type::Int(IntKind::I64), span));
                params.push(p);
                pvars.push(None);
            }
        }
        for l in &free {
            if let Some(Binding::Func(g)) = le.locals.get(l) {
                let g2 = self.rebind(g, params, pvars, span);
                inner.locals.insert(*l, Binding::Func(g2));
            }
        }
        FuncVal::Lambda { params: lp, body: lb, env: Rc::new(inner), act: *la }
    }

    fn lower_fn(&mut self, id: FnId, type_args: Vec<Ty>, funcs: Vec<Option<FuncVal<'p>>>, keys: Vec<Option<FnKey>>, span: Span) -> R<u32> {
        let f = &self.p.fns[id];
        let idx = self.fns.len() as u32;
        self.specs.insert((id, type_args.clone(), keys), idx);
        self.fns.push(None);
        let mut env = Env {
            fn_id: id,
            type_args: Rc::new(type_args),
            locals: HashMap::new(),
            sizes: HashMap::new(),
            pure: !f.is_action,
        };
        let mut params = Vec::new();
        // the variable of each parameter of the source (none for a function)
        let mut pvars: Vec<Option<VarId>> = Vec::new();
        for (i, &l) in f.params.iter().enumerate() {
            if is_fn_ty(&f.param_tys[i]) {
                // the function given: known in this specialization (D128)
                match funcs.get(i).cloned().flatten() {
                    Some(fv) => {
                        env.locals.insert(l, Binding::Func(fv));
                        pvars.push(None);
                        continue;
                    }
                    None => return poc(span, "a recursive function with a function parameter (SPEC section 11)"),
                }
            }
            let t = self.ty(&env, &f.param_tys[i], f.span)?;
            let v = self.new_var(&f.locals[l as usize].name, t);
            env.locals.insert(l, Binding::Var(v));
            params.push(v);
            pvars.push(Some(v));
        }
        // the captured values of the functions given: parameters after the
        // others, the functions bound to them in this specialization (D131),
        // those of the functions they read too (D563)
        for (i, &l) in f.params.iter().enumerate() {
            let Some(Some(fv)) = funcs.get(i) else { continue };
            if captures(fv).is_empty() {
                continue;
            }
            let g = self.rebind(fv, &mut params, &mut pvars, span);
            env.locals.insert(l, Binding::Func(g));
        }
        let ret = self.ty(&env, &f.ret, f.span)?;
        self.rets.push(ret);
        let fspan = f.span;
        let mut size_params = Vec::new();
        for (s, info) in f.syms.iter().enumerate() {
            let pv = |i: usize, me: &Self| {
                let v = pvars[i].expect("a size of a value parameter");
                (v, me.vars[v as usize].ty.clone())
            };
            if let tast::SymOrigin::Param(i) = info.origin {
                let (v, t) = pv(i, self);
                let len = Expr::new(ExprKind::Len(Box::new(Expr::var(v, t, fspan))), Type::Int(IntKind::I64), fspan);
                env.sizes.insert(s as SymId, len);
            }
            if let tast::SymOrigin::ParamPath(i, path) = &info.origin {
                if !tast::path_has_value(path) {
                    continue;
                }
                let (v, t) = pv(*i, self);
                let len = Expr::new(ExprKind::Len(Box::new(proj_path(v, &t, path, fspan))), Type::Int(IntKind::I64), fspan);
                env.sizes.insert(s as SymId, len);
            }
            if let tast::SymOrigin::SizeParam(i) = info.origin {
                let (v, t) = pv(i, self);
                env.sizes.insert(s as SymId, Expr::var(v, t.clone(), fspan));
                size_params.push(Expr::var(v, t, fspan));
            }
        }
        let mut body = self.expr(&mut env, &f.body)?;
        for k in size_params.into_iter().rev() {
            let ty = body.ty.clone();
            body = Expr::new(ExprKind::CheckNonNegative(Box::new(k), Box::new(body)), ty, fspan);
        }
        // the size of each array parameter not fixed by itself: tested on
        // entry (a call site cannot prove it for every call)
        // and of each array in a record or a tuple of a parameter that a
        // call cannot prove (D336)
        for (i, pt) in f.param_tys.iter().enumerate().rev() {
            let mut arrays = Vec::new();
            array_paths(pt, &mut Vec::new(), &mut arrays);
            for (path, p) in arrays.into_iter().rev() {
                if binds(&f.syms, i, &path, &p) || (!path.is_empty() && !self.unproved.contains(&(id, i))) {
                    continue;
                }
                let d = self.size(&env, &p, fspan)?;
                let Some(pi) = pvars[i] else { continue };
                let t = self.vars[pi as usize].ty.clone();
                let actual = Expr::new(ExprKind::Len(Box::new(proj_path(pi, &t, &path, fspan))), Type::Int(IntKind::I64), fspan);
                let ty = body.ty.clone();
                body = Expr::new(ExprKind::CheckSize(Box::new(d), Box::new(actual), Box::new(body)), ty, fspan);
            }
        }
        let require_tail = f.requires.iter().find(|(m, _)| *m == isc_syntax::ast::Modifier::RequireTail).map(|(_, s)| *s);
        self.reassoc_body(f, &mut body);
        let consumed = vec![false; params.len()];
        self.fns[idx as usize] = Some(Function { name: f.name.clone(), params, body, span: fspan, require_tail, consumed });
        Ok(idx)
    }

    fn lambda_of(&mut self, fv: &FuncVal<'p>, ptys: &[Type], ret: &Type, span: Span) -> R<Lambda> {
        // the names the program gives them, for the IR and the memory report
        let names: Vec<String> = (0..ptys.len())
            .map(|i| match fv {
                FuncVal::Lambda { params, env, .. } => params
                    .get(i)
                    .map(|l| self.p.fns[env.fn_id].locals[*l as usize].name.clone())
                    .unwrap_or_else(|| "p".into()),
                FuncVal::Global { id, .. } => {
                    let f = &self.p.fns[*id];
                    f.params.get(i).map(|l| f.locals[*l as usize].name.clone()).unwrap_or_else(|| "p".into())
                }
                _ => "p".into(),
            })
            .collect();
        let params: Vec<VarId> = ptys.iter().zip(&names).map(|(t, n)| self.new_var(n, t.clone())).collect();
        let args = params.iter().zip(ptys.iter()).map(|(v, t)| Arg::Val(Expr::var(*v, t.clone(), span))).collect();
        let body = self.apply(fv, args, ret, span)?;
        let n = params.len();
        Ok(Lambda { params, body: Box::new(body), drop_params: vec![false; n] })
    }

    fn prim(&mut self, env: &mut Env<'p>, p: Prim, args: &'p [TExpr], e: &'p TExpr) -> R<Expr> {
        let span = e.span;
        let ty = self.ty(env, &e.ty, span)?;
        let kind = match p {
            // a constant of each call compiled (D386)
            Prim::IsPure => {
                let fv = self.fn_value(env, &args[0])?;
                return Ok(Expr::new(ExprKind::Lit(Lit::Bool(self.fn_pure(&fv))), Type::Bool, span));
            }
            // `Map<K, V>` (D158): its three arrays
            Prim::MapOf => ExprKind::Tuple(args.iter().map(|a| self.expr(env, a)).collect::<R<_>>()?),
            Prim::MapParts => return self.expr(env, &args[0]),
            Prim::MapSize => {
                let m = self.expr(env, &args[0])?;
                let kt = match &m.ty {
                    Type::Tuple(ts) => ts[0].clone(),
                    _ => return internal(span, "a Map is not a tuple"),
                };
                let keys = Expr::new(ExprKind::Proj(Box::new(m), 0), kt, span);
                ExprKind::Len(Box::new(keys))
            }
            // strings (D134)
            // the functions of strings of TypeScript (D155)
            Prim::IndexOf
            | Prim::LastIndexOf
            | Prim::Includes
            | Prim::StartsWith
            | Prim::EndsWith
            | Prim::ReplaceAll
            | Prim::ToUpper
            | Prim::ToLower
            | Prim::PadStart
            | Prim::PadEnd
            | Prim::Repeat
            | Prim::TrimStart
            | Prim::TrimEnd
            | Prim::ToFixed
            | Prim::ArgSort
            | Prim::Group
            | Prim::GroupCounts
            | Prim::HashIndex
            | Prim::HashFind
            | Prim::ParseFloat
            | Prim::ParseInt
            | Prim::StrSlice => {
                let op = match p {
                    Prim::ArgSort => StrOp::ArgSort,
                    Prim::Group => StrOp::Group,
                    Prim::GroupCounts => StrOp::GroupCounts,
                    Prim::HashIndex => StrOp::HashIndex,
                    Prim::HashFind => StrOp::HashFind,
                    Prim::ParseFloat => StrOp::JsParseFloat,
                    Prim::ParseInt => StrOp::JsParseInt,
                    Prim::ReplaceAll if self.conc(env, &args[1].ty) == Ty::RegExp => StrOp::Rx(RxOp::ReplaceAll),
                    Prim::IndexOf => StrOp::IndexOf,
                    Prim::LastIndexOf => StrOp::LastIndexOf,
                    Prim::Includes => StrOp::Includes,
                    Prim::StartsWith => StrOp::StartsWith,
                    Prim::EndsWith => StrOp::EndsWith,
                    Prim::ReplaceAll => StrOp::ReplaceAll,
                    Prim::ToUpper => StrOp::ToUpper,
                    Prim::ToLower => StrOp::ToLower,
                    Prim::PadStart => StrOp::PadStart,
                    Prim::PadEnd => StrOp::PadEnd,
                    Prim::Repeat => StrOp::Repeat,
                    Prim::TrimStart => StrOp::TrimStart,
                    Prim::TrimEnd => StrOp::TrimEnd,
                    Prim::ToFixed => StrOp::ToFixed,
                    _ => StrOp::Slice,
                };
                let mut out = Vec::new();
                for a in args {
                    let mut x = self.expr(env, a)?;
                    // `toFixed` of a `f32`: its exact value, as a `f64`
                    if op == StrOp::ToFixed && x.ty == Type::F32 {
                        let sp = x.span;
                        x = Expr::new(ExprKind::Convert(Box::new(x)), Type::F64, sp);
                    }
                    out.push(x);
                }
                // the order of `bigint` (D156)
                if op == StrOp::ArgSort && out[0].ty.elem() == &Type::Big {
                    ExprKind::StrOp(StrOp::Big(BigOp::ArgSort), out)
                } else {
                    ExprKind::StrOp(op, out)
                }
            }
            Prim::Concat | Prim::StrLen | Prim::Bytes | Prim::StringOf | Prim::ToString | Prim::Lines | Prim::Split | Prim::Trim | Prim::Join => {
                let op = match p {
                    Prim::Concat => StrOp::Concat,
                    Prim::StrLen => StrOp::Len,
                    Prim::Bytes => StrOp::Bytes,
                    Prim::StringOf => StrOp::FromBytes,
                    Prim::Lines => StrOp::Lines,
                    Prim::Split => StrOp::Split,
                    Prim::Trim => StrOp::Trim,
                    Prim::Join => StrOp::Join,
                    _ => StrOp::ToString,
                };
                let mut out = Vec::new();
                for a in args {
                    out.push(self.expr(env, a)?);
                }
                if op == StrOp::ToString && out[0].ty == Type::Big {
                    ExprKind::StrOp(StrOp::Big(BigOp::ToString), out)
                } else if op == StrOp::Len && out[0].ty.is_array() {
                    ExprKind::Len(Box::new(out.remove(0)))
                } else if op == StrOp::Concat && out[0].ty.is_array() {
                    let b = out.pop().unwrap();
                    let a = out.pop().unwrap();
                    return Ok(self.concat_arrays(a, b, ty, span));
                } else if op == StrOp::Concat {
                    // `a + b + c` of strings, a template literal: one
                    // concatenation of all the parts, one block made (D342)
                    let mut parts = Vec::new();
                    for x in out {
                        match x.kind {
                            ExprKind::StrOp(StrOp::Concat, inner) => parts.extend(inner),
                            _ => parts.push(x),
                        }
                    }
                    ExprKind::StrOp(StrOp::Concat, parts)
                } else {
                    ExprKind::StrOp(op, out)
                }
            }
            Prim::ParseF64 | Prim::ReadText | Prim::WriteText | Prim::ParseCsvF64 | Prim::ReadCsvF64 | Prim::PrintBytes | Prim::OrError(_) => {
                let op = match p {
                    Prim::OrError(f) => ActionOp::OrError(f),
                    Prim::ParseF64 => ActionOp::ParseF64,
                    Prim::PrintBytes => ActionOp::PrintBytes,
                    Prim::ReadText => ActionOp::ReadText,
                    Prim::WriteText => ActionOp::WriteText,
                    Prim::ParseCsvF64 => ActionOp::ParseCsvF64,
                    _ => ActionOp::ReadCsvF64,
                };
                let mut out = Vec::new();
                for a in args {
                    out.push(self.expr(env, a)?);
                }
                ExprKind::Action(op, out)
            }
            Prim::Map => {
                let x = self.expr(env, &args[0])?;
                let fv = self.fn_value(env, &args[1])?;
                // an action: applied in order, once each, by `mapInOrder` of
                // the prelude (D387)
                if !self.fn_pure(&fv) {
                    let Some(id) = self.p.fns.iter().position(|f| f.from_prelude && f.name == "mapInOrder") else {
                        return internal(span, "`mapInOrder` is not in the prelude");
                    };
                    let elem = |t: Ty| match t {
                        Ty::Array(_, e) => Ok(*e),
                        _ => internal(span, "`map` of what is not an array"),
                    };
                    let type_args = vec![elem(self.conc(env, &args[0].ty))?, elem(self.conc(env, &e.ty))?];
                    return self.apply(&FuncVal::Global { id: id as FnId, type_args }, vec![Arg::Val(x), Arg::Func(fv)], &ty, span);
                }
                let et = x.ty.elem().clone();
                let lam = self.lambda_of(&fv, &[et], ty.elem(), span)?;
                ExprKind::Map(vec![x], lam)
            }
            // `mapParallel` (D373): its array bound to a variable, which the
            // tasks read
            Prim::MapParallel => {
                let x = self.expr(env, &args[0])?;
                let fv = self.fn_value(env, &args[1])?;
                let et = x.ty.elem().clone();
                let lam = self.lambda_of(&fv, &[et], ty.elem(), span)?;
                let xt = x.ty.clone();
                let xs = self.new_var("tasks", xt.clone());
                let t = Expr::new(ExprKind::Tasks(Box::new(Expr::var(xs, xt, span)), lam), ty.clone(), span);
                return Ok(self.let_(xs, x, t, true, span));
            }
            Prim::Min | Prim::Max => {
                let op = if p == Prim::Min { BinOp::Min } else { BinOp::Max };
                if args.len() == 2 {
                    let a = self.expr(env, &args[0])?;
                    let b = self.expr(env, &args[1])?;
                    return self.binary(op, a, b, span);
                }
                // `min(x)`: `reduce(x, min, e)` with the identity of `min`: +inf
                // for floats, the largest value of an integer type (an empty
                // array gives it: DECISIONS D89)
                let x = self.expr(env, &args[0])?;
                let et = x.ty.elem().clone();
                let low = p == Prim::Max;
                let z = match &et {
                    Type::F64 => Lit::F64(if low { f64::NEG_INFINITY } else { f64::INFINITY }),
                    Type::F32 => Lit::F32(if low { f32::NEG_INFINITY } else { f32::INFINITY }),
                    Type::Int(k) => {
                        let bits = k.bits();
                        let (lo, hi): (i128, i128) = if k.signed() {
                            (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
                        } else {
                            (0, (1i128 << bits) - 1)
                        };
                        Lit::Int(canon(*k, if low { lo } else { hi }))
                    }
                    _ => return internal(span, "min or max of an array of non-numbers"),
                };
                let a = self.new_var("a", et.clone());
                let b = self.new_var("b", et.clone());
                let body = Expr::new(
                    ExprKind::Binary(op, Box::new(Expr::var(a, et.clone(), span)), Box::new(Expr::var(b, et.clone(), span))),
                    et.clone(),
                    span,
                );
                let lam = Lambda { params: vec![a, b], body: Box::new(body), drop_params: vec![false, false] };
                ExprKind::Reduce(Box::new(x), lam, Box::new(Expr::lit(z, et, span)))
            }
            Prim::Zip => {
                // the pairs, element by element: a `Map` over both arrays,
                // which the consumers fuse like any other (F1)
                let a = self.expr(env, &args[0])?;
                let b = self.expr(env, &args[1])?;
                let (ta, tb) = (a.ty.elem().clone(), b.ty.elem().clone());
                let pt = Type::Tuple(vec![ta.clone(), tb.clone()]);
                self.plain_tuples.borrow_mut().insert(pt.clone());
                let x = self.new_var("x", ta.clone());
                let y = self.new_var("y", tb.clone());
                let body = Expr::new(ExprKind::Tuple(vec![Expr::var(x, ta, span), Expr::var(y, tb, span)]), pt, span);
                let lam = Lambda { params: vec![x, y], body: Box::new(body), drop_params: vec![false, false] };
                let checked = matches!(&e.kind, TExprKind::Call { size_checks, .. } if !size_checks.is_empty());
                if !checked {
                    ExprKind::Map(vec![a, b], lam)
                } else {
                    // sizes not proved equal: a run-time test, as for `a + b`
                    let (at, bt) = (a.ty.clone(), b.ty.clone());
                    let va = self.new_var("zip_a", at.clone());
                    let vb = self.new_var("zip_b", bt.clone());
                    let len = |v: VarId, t: Type| {
                        Expr::new(ExprKind::Len(Box::new(Expr::var(v, t, span))), Type::Int(IntKind::I64), span)
                    };
                    let map = Expr::new(
                        ExprKind::Map(vec![Expr::var(va, at.clone(), span), Expr::var(vb, bt.clone(), span)], lam),
                        ty.clone(),
                        span,
                    );
                    let chk = Expr::new(
                        ExprKind::CheckSize(Box::new(len(va, at)), Box::new(len(vb, bt)), Box::new(map)),
                        ty.clone(),
                        span,
                    );
                    let inner = self.let_(vb, b, chk, false, span);
                    return Ok(self.let_(va, a, inner, false, span));
                }
            }
            Prim::Reduce => {
                let x = self.expr(env, &args[0])?;
                let fv = self.fn_value(env, &args[1])?;
                let z = self.expr(env, &args[2])?;
                let et = x.ty.elem().clone();
                let lam = self.lambda_of(&fv, &[et.clone(), et.clone()], &et, span)?;
                ExprKind::Reduce(Box::new(x), lam, Box::new(z))
            }
            Prim::Window => {
                let x = self.expr(env, &args[0])?;
                let k = self.expr(env, &args[1])?;
                ExprKind::Window(Box::new(x), Box::new(k))
            }
            Prim::Filter => {
                let x = self.expr(env, &args[0])?;
                let fv = self.fn_value(env, &args[1])?;
                let et = x.ty.elem().clone();
                let lam = self.lambda_of(&fv, &[et], &Type::Bool, span)?;
                ExprKind::Filter(Box::new(x), lam)
            }
            Prim::Scan => {
                let x = self.expr(env, &args[0])?;
                let fv = self.fn_value(env, &args[1])?;
                let z = self.expr(env, &args[2])?;
                let et = x.ty.elem().clone();
                let lam = self.lambda_of(&fv, &[et.clone(), et.clone()], &et, span)?;
                ExprKind::Scan(Box::new(x), lam, Box::new(z))
            }
            Prim::Fold => {
                let x = self.expr(env, &args[0])?;
                let init = self.expr(env, &args[1])?;
                let fv = self.fn_value(env, &args[2])?;
                let et = x.ty.elem().clone();
                let at = init.ty.clone();
                let lam = self.lambda_of(&fv, &[at.clone(), et], &at, span)?;
                ExprKind::Fold(Box::new(x), Box::new(init), lam)
            }
            Prim::Zeros => {
                let n = match &e.ty {
                    Ty::Array(Size::Poly(p), _) => self.size(env, p, span)?,
                    _ => return internal(span, "zeros without size"),
                };
                ExprKind::Zeros(Box::new(n))
            }
            // a length known at run time (D155): checked positive or zero
            Prim::Iota | Prim::Fill if !matches!(args[0].kind, TExprKind::SizeVal(_)) => {
                let n = self.expr(env, &args[0])?;
                let i64t = Type::Int(IntKind::I64);
                let v = self.new_var("length", i64t.clone());
                let nv = Expr::var(v, i64t.clone(), span);
                let kind = if p == Prim::Iota {
                    ExprKind::Iota(Box::new(nv.clone()))
                } else {
                    let x = self.expr(env, &args[1])?;
                    if x.ty.has_heap() && !matches!(x.kind, ExprKind::Var(..)) || x.ty.is_array() {
                        let inner = self.fill_of(nv.clone(), x, ty.clone(), span);
                        let checked = Expr::new(ExprKind::CheckNonNegative(Box::new(nv), Box::new(inner)), ty, span);
                        return Ok(self.let_(v, n, checked, false, span));
                    }
                    ExprKind::Fill(Box::new(nv.clone()), Box::new(x))
                };
                let built = Expr::new(kind, ty.clone(), span);
                let checked = Expr::new(ExprKind::CheckNonNegative(Box::new(nv), Box::new(built)), ty, span);
                return Ok(self.let_(v, n, checked, false, span));
            }
            // `Number(x)` (D156): of a text, ECMAScript's conversion; of a
            // number, its `f64`
            Prim::JsNumber => {
                let x = self.expr(env, &args[0])?;
                if x.ty == Type::Str {
                    ExprKind::StrOp(StrOp::JsNumber, vec![x])
                } else if x.ty == Type::Big {
                    ExprKind::StrOp(StrOp::Big(BigOp::ToF64), vec![x])
                } else {
                    ExprKind::Convert(Box::new(x))
                }
            }
            Prim::Iota => ExprKind::Iota(Box::new(self.expr(env, &args[0])?)),
            Prim::Fill => {
                let n = self.expr(env, &args[0])?;
                let v = self.expr(env, &args[1])?;
                if v.ty.has_heap() && !matches!(v.kind, ExprKind::Var(..)) || v.ty.is_array() {
                    return Ok(self.fill_of(n, v, ty, span));
                }
                ExprKind::Fill(Box::new(n), Box::new(v))
            }
            Prim::Sqrt => ExprKind::Sqrt(Box::new(self.expr(env, &args[0])?)),
            // `abs` of an integer (D155): `x < 0 ? -x : x` (modulo 2^bits:
            // the minimum stays itself)
            Prim::Math(MathFn::Abs) if matches!(ty, Type::Int(_)) => {
                let x = self.expr(env, &args[0])?;
                if let Type::Int(k) = &ty {
                    if !k.signed() {
                        return Ok(x);
                    }
                }
                let v = self.new_var("x", ty.clone());
                let xv = Expr::var(v, ty.clone(), span);
                let zero = Expr::lit(Lit::Int(0), ty.clone(), span);
                let neg = Expr::new(ExprKind::Binary(BinOp::Lt, Box::new(xv.clone()), Box::new(zero)), Type::Bool, span);
                let minus = Expr::new(ExprKind::Unary(isc_syntax::ast::UnOp::Neg, Box::new(xv.clone())), ty.clone(), span);
                let body = Expr::new(ExprKind::If(Box::new(neg), Box::new(minus), Box::new(xv)), ty.clone(), span);
                return Ok(self.let_(v, x, body, false, span));
            }
            Prim::Math(f) => {
                let mut xs = Vec::new();
                for a in args {
                    xs.push(self.expr(env, a)?);
                }
                ExprKind::Math(f, xs)
            }
            // of a `bigint` (D156): modulo 2^64, or the nearest float
            Prim::Convert(_) if args[0].ty == Ty::BigInt || self.conc(env, &args[0].ty) == Ty::BigInt => {
                let x = self.expr(env, &args[0])?;
                let (op, t) = match &ty {
                    Type::F64 | Type::F32 => (BigOp::ToF64, Type::F64),
                    _ => (BigOp::ToI64, Type::Int(IntKind::I64)),
                };
                let y = Expr::new(ExprKind::StrOp(StrOp::Big(op), vec![x]), t.clone(), span);
                if t == ty {
                    return Ok(y);
                }
                ExprKind::Convert(Box::new(y))
            }
            Prim::Convert(_) => ExprKind::Convert(Box::new(self.expr(env, &args[0])?)),
            // regular expressions (D157)
            Prim::RegExpNew | Prim::RxTest | Prim::RxSearch | Prim::RxCount | Prim::RxMatches | Prim::Replace => {
                // a literal `/source/flags`, or texts known here: checked at
                // compilation (D157)
                if p == Prim::RegExpNew {
                    if let (TExprKind::Lit(tast::Lit::Str(src)), TExprKind::Lit(tast::Lit::Str(flags))) = (&args[0].kind, &args[1].kind) {
                        let mut sink = String::new();
                        if let Err(e) = isc_runtime_core::rx::value(src, flags, &mut sink) {
                            return Err(Diagnostic::new(span, format!("{}", e)));
                        }
                    }
                }
                let op = match p {
                    Prim::RegExpNew => StrOp::Rx(RxOp::New),
                    Prim::RxTest => StrOp::Rx(RxOp::Test),
                    Prim::RxSearch => StrOp::Rx(RxOp::Search),
                    Prim::RxCount => StrOp::Rx(RxOp::Count),
                    Prim::RxMatches => StrOp::Rx(RxOp::Matches),
                    _ if self.conc(env, &args[1].ty) == Ty::RegExp => StrOp::Rx(RxOp::Replace),
                    _ => StrOp::ReplaceFirst,
                };
                let mut out = Vec::new();
                for a in args {
                    out.push(self.expr(env, a)?);
                }
                ExprKind::StrOp(op, out)
            }
            // `BigInt(x)` (D156)
            Prim::ToBigInt => {
                let x = self.expr(env, &args[0])?;
                match &x.ty {
                    Type::Str => ExprKind::StrOp(StrOp::Big(BigOp::Parse), vec![x]),
                    _ => return Ok(self.to_big(x, span)),
                }
            }
            Prim::Bench => {
                let r = self.expr(env, &args[0])?;
                let fv = self.fn_value(env, &args[1])?;
                let body_ty = match &ty {
                    Type::Tuple(ts) => ts[0].clone(),
                    _ => return internal(span, "bench type"),
                };
                let body = self.apply(&fv, Vec::new(), &body_ty, span)?;
                ExprKind::Bench(Box::new(r), Box::new(body))
            }
            // the standard input (D155); its lines: `lines` of its text
            Prim::ReadStdin => ExprKind::Action(ActionOp::ReadStdin, Vec::new()),
            Prim::ReadStdinText => ExprKind::Action(ActionOp::ReadStdinText, Vec::new()),
            Prim::ReadStdinLines => {
                let t = Expr::new(ExprKind::Action(ActionOp::ReadStdinText, Vec::new()), Type::Str, span);
                ExprKind::StrOp(StrOp::Lines, vec![t])
            }
            Prim::ReadF64
            | Prim::ReadF32
            | Prim::ReadBytes
            | Prim::WriteF64
            | Prim::WriteF32
            | Prim::WriteBytes
            | Prim::Print
            | Prim::Clock
            | Prim::ParseI64
            | Prim::Sys(_) => {
                let op = match p {
                    Prim::Sys(o) => ActionOp::Sys(o),
                    Prim::ReadF64 => ActionOp::ReadF64,
                    Prim::ReadF32 => ActionOp::ReadF32,
                    Prim::ReadBytes => ActionOp::ReadBytes,
                    Prim::WriteF64 => ActionOp::WriteF64,
                    Prim::WriteF32 => ActionOp::WriteF32,
                    Prim::WriteBytes => ActionOp::WriteBytes,
                    Prim::Print => {
                        let sh = self.shape(env, &args[0].ty, span)?;
                        self.shapes.push(sh);
                        ActionOp::Print(self.shapes.len() as u32 - 1)
                    }
                    Prim::Clock => ActionOp::Clock,
                    _ => ActionOp::ParseI64,
                };
                let mut out = Vec::new();
                for a in args {
                    out.push(self.expr(env, a)?);
                }
                ExprKind::Action(op, out)
            }
        };
        Ok(Expr::new(kind, ty, span))
    }
}

/// The value at `path` (indices of fields and of elements of tuples) in the
/// variable `v` of type `t` (D336).
fn proj_path(v: VarId, t: &Type, path: &[u32], span: Span) -> Expr {
    let mut e = Expr::var(v, t.clone(), span);
    let mut ty = t.clone();
    for &k in path {
        let Type::Tuple(ts) = &ty else { unreachable!("a path through records and tuples") };
        let nt = ts[k as usize].clone();
        e = Expr::new(ExprKind::Proj(Box::new(e), k), nt.clone(), span);
        ty = nt;
    }
    e
}

/// The arrays of a value of type `t` with their paths, through records
/// and tuples (D336): the array itself (an empty path), or those of its
/// fields and elements.
fn array_paths(t: &Ty, path: &mut Vec<u32>, out: &mut Vec<(Vec<u32>, Poly)>) {
    match t {
        Ty::Array(Size::Poly(p), _) => out.push((path.clone(), p.clone())),
        Ty::Record(fs) => {
            for (i, (_, f)) in fs.iter().enumerate() {
                path.push(i as u32);
                array_paths(f, path, out);
                path.pop();
            }
        }
        Ty::Tuple(ts) => {
            for (i, f) in ts.iter().enumerate() {
                path.push(i as u32);
                array_paths(f, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

/// The size `p` of the array at `path` in the parameter `i` is the size
/// variable that this array binds: nothing to check.
fn binds(syms: &[tast::SymInfo], i: usize, path: &[u32], p: &Poly) -> bool {
    p.as_sym().is_some_and(|s| match &syms[s as usize].origin {
        tast::SymOrigin::Param(j) => *j == i && path.is_empty(),
        tast::SymOrigin::ParamPath(j, q) => *j == i && q.as_slice() == path,
        _ => false,
    })
}
