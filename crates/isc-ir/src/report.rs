//! The memory report (SPEC section 8, `iscc build --report`, DECISIONS D98):
//! every array the program allocates, with the rule that causes it (section
//! 7: M1 to M3), every copy with its reason, and the run-time tests that
//! remain (bounds, sizes). It reads the optimized IR, where every memory
//! decision is explicit (D96): an array expression is either fused into the
//! loop of its consumer, or evaluated into new memory.

use std::collections::HashMap;
use std::fmt::Write;

use crate::passes::{fusable_slots, is_producer, lambda_body_index};
use crate::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub line: u32,
    pub column: u32,
    /// `alloc`, `copy`, `bounds` or `size`.
    pub kind: &'static str,
    /// The size in bytes when it is known at compilation, else the type; the
    /// expression of a run-time test.
    pub what: String,
    pub bytes: Option<u64>,
    pub why: String,
    /// The rule of section 7 that causes an allocation (`M1`, `M2`, `M3`),
    /// or none.
    pub rule: Option<&'static str>,
}

/// The place an expression is evaluated in.
#[derive(Clone, Copy)]
enum Ctx<'e> {
    /// Bound to a variable: why it exists depends on its uses in `body`.
    Let(VarId, &'e Expr),
    /// An argument of an action.
    Action,
    /// The initial accumulator of a `fold`.
    FoldInit,
    /// The array a `with` updates.
    UpdateTarget,
    /// A part of a new value (a tuple component, a row of a literal, the new
    /// element of an update).
    Part,
    /// The array a consumer reads (not fused: it is not a producer).
    Read,
    /// The source of a view (a slice, a row).
    ViewSource,
    /// The source of a window, whose elements are read by several windows.
    WindowSource,
    /// An argument of a recursive function.
    CallArg,
    /// The result of the program or of a function (through `let` bodies and
    /// branches): the destination its caller gives (section 8).
    Result,
    /// The result of a lambda: a value at each iteration.
    LambdaResult,
    /// The value of the body of `bench`: given to its caller at each
    /// repetition.
    BenchResult,
}

struct Walk<'p> {
    p: &'p Program,
    out: Vec<Entry>,
    /// Lengths known at compilation.
    lens: HashMap<VarId, u64>,
    /// Variables bound by a `let`: whether it is a statement of an action.
    stmts: HashMap<VarId, bool>,
    /// The enclosing loops (the lambda of a `map`, `reduce`, ..., the body
    /// of a `bench`): what is evaluated there is evaluated many times.
    loops: Vec<String>,
    copy_reasons: HashMap<(u32, u32), Vec<String>>,
    /// The position, in the program's file, of the innermost expression
    /// being walked: an inlined function of the prelude (`sum`, `min`, ...)
    /// is reported at the place that calls it.
    user: isc_syntax::Span,
    /// The direct tail calls of the recursive functions, compiled as loops,
    /// and their tail calls to other functions, which reuse the frame.
    tails: std::collections::HashSet<*const Expr>,
    others: std::collections::HashSet<*const Expr>,
}

/// The entries of the report, in the order of the program.
pub fn entries(p: &Program) -> Vec<Entry> {
    entries_of(p, true)
}

/// The entries of the body of the program, and of its recursive functions
/// when `with_fns`.
pub fn entries_of(p: &Program, with_fns: bool) -> Vec<Entry> {
    let mut copy_reasons: HashMap<(u32, u32), Vec<String>> = HashMap::new();
    for (sp, why) in p.copy_reasons.iter().rev() {
        copy_reasons.entry((sp.line, sp.col)).or_default().push(why.clone());
    }
    let user = p.body.span;
    let mut tails = std::collections::HashSet::new();
    let mut others = std::collections::HashSet::new();
    for (i, f) in p.fns.iter().enumerate() {
        let own = crate::passes::tail_calls(i as u32, f);
        // as the code generator decides (D113)
        if own.values().all(|c| c.moved.is_empty()) {
            others.extend(crate::passes::other_tail_calls(i as u32, f).into_keys());
        }
        tails.extend(own.into_keys());
    }
    let mut w = Walk { p, out: Vec::new(), lens: HashMap::new(), stmts: HashMap::new(), loops: Vec::new(), copy_reasons, user, tails, others };
    // the functions whose results may vary (section 10: in the report)
    for (name, sp) in &p.reassoc_fns {
        w.out.push(Entry {
            line: sp.line,
            column: sp.col,
            kind: "reassoc",
            what: format!("`{}`", name),
            bytes: None,
            why: "allowReassoc: its floating-point operations may be reassociated and contracted into fma; the result may vary with the target".into(),
            rule: None,
        });
    }
    // a function chosen at run time (section 11: in the report)
    for sp in &p.choices {
        w.out.push(Entry {
            line: sp.line,
            column: sp.col,
            kind: "switch",
            what: "a function".into(),
            bytes: None,
            why: "chosen at run time: each call is a branch on the condition, no allocation".into(),
            rule: None,
        });
    }
    w.walk(&p.body, Ctx::Result);
    for f in p.fns.iter().filter(|_| with_fns) {
        w.user = f.span;
        w.walk(&f.body, Ctx::Result);
    }
    let mut out: Vec<Entry> = Vec::new();
    for e in w.out {
        // a function inlined at several places gives the same entry
        if !out.contains(&e) {
            out.push(e);
        }
    }
    out.sort_by_key(|e| (e.line, e.column));
    out
}

fn elem_bytes(t: &Type) -> u64 {
    match t {
        Type::Int(k) => (k.bits() as u64).max(8) / 8,
        Type::F64 => 8,
        Type::F32 => 4,
        Type::Bool | Type::Unit => 1,
        Type::Str | Type::Big => 16,
        Type::Tuple(ts) => ts.iter().map(elem_bytes).sum(),
        Type::Array(_, _) => 16,
        Type::Rec(_) | Type::Handle => 8,
    }
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A short rendering of a scalar expression, for the run-time tests.
fn show(e: &Expr, p: &Program) -> String {
    use ExprKind::*;
    match &e.kind {
        Var(v, _) => p.vars[*v as usize].name.clone(),
        Lit(crate::Lit::Int(v)) => v.to_string(),
        Lit(crate::Lit::F64(v)) => format!("{:?}", v),
        Lit(crate::Lit::Bool(b)) => b.to_string(),
        Binary(op, a, b) => format!("{} {} {}", show(a, p), op.text(), show(b, p)),
        Index(a, i) => format!("{}[{}]", show(a, p), show(i, p)),
        Proj(x, i) => format!("{}.{}", show(x, p), i),
        Convert(x) => format!("{}({})", crate::print::type_str(&e.ty), show(x, p)),
        Len(x) => format!("length of {}", show(x, p)),
        // a copy has the text of what it copies
        Copy(x) | CheckSize(_, _, x) | CheckNonNegative(_, x) => show(x, p),
        Let { body, .. } => show(body, p),
        Slice(x, a, b) => format!("{}.slice({}, {})", show(x, p), show(a, p), show(b, p)),
        Window(x, k) => format!("window({}, {})", show(x, p), show(k, p)),
        Map(ins, l) if matches!(&l.body.kind, Tuple(_)) && ins.len() > 1 => {
            format!("zip({})", ins.iter().map(|i| show(i, p)).collect::<Vec<_>>().join(", "))
        }
        Map(ins, _) => format!("map({}, …)", ins.iter().map(|i| show(i, p)).collect::<Vec<_>>().join(", ")),
        _ => "…".into(),
    }
}

impl<'p> Walk<'p> {
    fn len_of(&self, e: &Expr) -> Option<u64> {
        use ExprKind::*;
        let lit = |n: &Expr| match n.kind {
            Lit(crate::Lit::Int(v)) if v >= 0 => Some(v as u64),
            _ => None,
        };
        match &e.kind {
            Zeros(n) | Iota(n) | Fill(n, _) => lit(n),
            ArrayLit(es) => Some(es.len() as u64),
            Var(v, _) => self.lens.get(v).copied(),
            Map(ins, _) => ins.first().and_then(|i| self.len_of(i)),
            Copy(x) | CheckSize(_, _, x) | CheckNonNegative(_, x) => self.len_of(x),
            Update { target, .. } => self.len_of(target),
            Let { body, .. } => self.len_of(body),
            _ => None,
        }
    }

    fn size(&self, e: &Expr) -> (String, Option<u64>) {
        match (self.len_of(e), &e.ty) {
            (Some(n), Type::Array(t, _)) => {
                let b = n * elem_bytes(t);
                (format!("{} bytes", thousands(b)), Some(b))
            }
            (_, Type::Rec(i)) => (format!("`{}`", self.p.recs[*i as usize].name), None),
            _ => (crate::print::type_str(&e.ty), None),
        }
    }

    fn in_loop(&self) -> String {
        match self.loops.last() {
            Some(l) => format!(", {}", l),
            None => String::new(),
        }
    }

    /// The position of `e` in the program's file.
    fn place(&self, e: &Expr) -> isc_syntax::Span {
        if e.span.file == isc_types::USER_FILE {
            e.span
        } else {
            self.user
        }
    }

    fn loop_label(&self, e: &Expr) -> String {
        let line = self.place(e).line;
        match &e.kind {
            ExprKind::Bench(..) => format!("at each repetition of `bench` at line {}", line),
            _ => format!("at each iteration of the loop at line {}", line),
        }
    }

    fn push(&mut self, e: &Expr, kind: &'static str, what: String, bytes: Option<u64>, why: String, rule: Option<&'static str>) {
        let why = format!("{}{}", why, self.in_loop());
        let at = self.place(e);
        self.out.push(Entry { line: at.line, column: at.col, kind, what, bytes, why, rule });
    }

    /// Whether the `let` of `v` is a statement of an action.
    fn let_of(&self, v: VarId) -> Option<bool> {
        self.stmts.get(&v).copied()
    }

    /// Where the uses of `v` in `body` are: (line, inside a lambda, context).
    /// The uses of `v`, with the branches of `if`s each one is in: two uses
    /// in the two branches of one `if` never both run.
    #[allow(clippy::type_complexity)]
    fn uses_on_paths(&self, body: &Expr, v: VarId) -> Vec<(u32, bool, &'static str, Vec<(usize, u8)>)> {
        type Out = Vec<(u32, bool, &'static str, Vec<(usize, u8)>)>;
        fn go(e: &Expr, v: VarId, depth: u32, user: u32, path: &mut Vec<(usize, u8)>, out: &mut Out) {
            // a use inside a function of the prelude is at the line that
            // calls it
            let user = if e.span.file == isc_types::USER_FILE { e.span.line } else { user };
            let lam = lambda_body_index(e);
            let slots = fusable_slots(e);
            for (i, c) in e.children().into_iter().enumerate() {
                if let ExprKind::Var(w, _) = c.kind {
                    if w == v {
                        // a length reads no element
                        if matches!(e.kind, ExprKind::Len(_)) {
                            continue;
                        }
                        let place = match &e.kind {
                            ExprKind::Action(..) => "action",
                            ExprKind::Call(..) => "call",
                            ExprKind::Fold(_, init, _) if std::ptr::eq(&**init, c) => "fold",
                            ExprKind::Update { target, .. } if std::ptr::eq(&**target, c) => "update",
                            ExprKind::Window(..) => "window",
                            ExprKind::Slice(..) => "view",
                            ExprKind::Index(..) if e.ty.is_array() => "view",
                            _ if slots.contains(&i) => "read",
                            _ => "other",
                        };
                        let line = if c.span.file == isc_types::USER_FILE { c.span.line } else { user };
                        let mut here = path.clone();
                        if matches!(e.kind, ExprKind::If(..)) && i > 0 {
                            here.push((e as *const Expr as usize, i as u8));
                        }
                        out.push((line, depth > 0, place, here));
                        continue;
                    }
                }
                let branch = matches!(e.kind, ExprKind::If(..)) && i > 0;
                if branch {
                    path.push((e as *const Expr as usize, i as u8));
                }
                go(c, v, depth + u32::from(Some(i) == lam), user, path, out);
                if branch {
                    path.pop();
                }
            }
        }
        let mut out = Vec::new();
        if let ExprKind::Var(w, _) = body.kind {
            if w == v {
                out.push((body.span.line, false, "result", Vec::new()));
            }
        }
        go(body, v, 0, self.place(body).line, &mut Vec::new(), &mut out);
        out
    }

    /// Why an array is allocated in this context, and the rule.
    fn why(&self, e: &Expr, ctx: Ctx) -> (String, Option<&'static str>) {
        use ExprKind::*;
        // the result of a function is written into the destination of its
        // caller, whatever computes it (section 8)
        if matches!(ctx, Ctx::Result) && !matches!(e.kind, Action(..)) {
            return ("a result".into(), None);
        }
        match &e.kind {
            Action(..) => return ("read from a file: an input of the program".into(), None),
            ArrayLit(_) => return ("an array literal".into(), None),
            Scan(..) => return ("the result of `scan` (rule M2)".into(), Some("M2")),
            Fold(..) => return ("the result of `fold` (rule M2)".into(), Some("M2")),
            _ => {}
        }
        match ctx {
            Ctx::Let(v, body) => {
                let on_paths = self.uses_on_paths(body, v);
                // used more than once on one path (two uses in the two
                // branches of an `if` are one use on each path)
                let exclusive = |a: &Vec<(usize, u8)>, b: &Vec<(usize, u8)>| {
                    a.iter().any(|(x, s)| b.iter().any(|(y, t)| x == y && s != t))
                };
                let several = on_paths
                    .iter()
                    .enumerate()
                    .any(|(i, u)| on_paths[i + 1..].iter().any(|w| !exclusive(&u.3, &w.3)));
                let uses: Vec<(u32, bool, &'static str)> = on_paths.iter().map(|(l, d, p, _)| (*l, *d, *p)).collect();
                let name = &self.p.vars[v as usize].name;
                let shown = if name == "tmp" || name.starts_with("zip_") { "it".to_string() } else { format!("`{}`", name) };
                if several || uses.iter().any(|u| u.1) {
                    let mut lines: Vec<u32> = uses.iter().map(|u| u.0).collect();
                    lines.sort();
                    lines.dedup();
                    let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
                    let times = if uses.len() > 1 { format!("{} times", uses.len()) } else { "once, inside a loop".into() };
                    let at = if lines.len() > 1 { "lines" } else { "line" };
                    return (
                        format!("shared result (rule M1): {} is used {}, at {} {}", shown, times, at, lines.join(", ")),
                        Some("M1"),
                    );
                }
                // a statement of an action is computed entirely before the
                // next statement (section 12): it is not fused into its use
                let stmt = matches!(self.let_of(v), Some(true));
                // one use on each path: the one that asks the most of the
                // memory (a value of a branch says nothing by itself)
                let first = uses.iter().map(|u| u.2).find(|p| *p != "other").or(uses.first().map(|u| u.2));
                match first {
                    Some(u) if stmt && u != "action" => (
                        "a statement of an action: computed entirely before the next statement (section 12)".into(),
                        None,
                    ),
                    None => ("never used".into(), None),
                    // the value of the `let` itself: as its context says
                    Some("result") => ("a result".into(), None),
                    Some("call") => (
                        "an argument of a function that is called (recursive, or too large to inline, D336), which reads it in memory (rule M2: not inlined)".into(),
                        Some("M2"),
                    ),
                    Some("action") => ("given to an action (rule M3)".into(), Some("M3")),
                    Some("fold") => ("the accumulator of a `fold` (rule M2)".into(), Some("M2")),
                    Some("update") => ("the array of a `with`".into(), None),
                    Some("window") => (
                        "read through a window: each element is read by several windows (rule M1)".into(),
                        Some("M1"),
                    ),
                    Some("view") => ("read through a view".into(), None),
                    Some(_) => self.why(e, Ctx::Read),
                }
            }
            Ctx::Action => ("given to an action (rule M3)".into(), Some("M3")),
            Ctx::CallArg => (
                "an argument of a function that is called (recursive, or too large to inline, D336), which reads it in memory (rule M2: not inlined)".into(),
                Some("M2"),
            ),
            Ctx::FoldInit => ("the accumulator of a `fold` (rule M2)".into(), Some("M2")),
            Ctx::UpdateTarget => ("the array of a `with`".into(), None),
            Ctx::Part => ("a part of a new value (a tuple, a row of a literal)".into(), None),
            Ctx::ViewSource => ("read through a view".into(), None),
            Ctx::WindowSource => (
                "read through a window: each element is read by several windows (rule M1)".into(),
                Some("M1"),
            ),
            Ctx::Result => ("a result".into(), None),
            Ctx::LambdaResult => ("the value of a lambda, at each iteration: no rule".into(), None),
            Ctx::BenchResult => ("the value of `bench` (the result of its body)".into(), None),
            Ctx::Read => match &e.kind {
                Filter(..) => ("the result of `filter`: no rule (fusion F2 to come)".into(), None),
                _ => ("no rule".into(), None),
            },
        }
    }

    fn allocates(e: &Expr) -> bool {
        use ExprKind::*;
        match &e.kind {
            // a `map` that writes into its input moved into it (D207)
            Map(ins, _) if ins.iter().any(|i| matches!(i.kind, Var(_, Use::Move))) => false,
            Map(..) | Iota(_) | Zeros(_) | Fill(..) | ArrayLit(_) | Scan(..) | Filter(..) => true,
            Slice(x, _, _) => !e.ty.slice_is_view() || is_producer(x),
            Action(..) => e.ty.is_array(),
            _ => false,
        }
    }

    fn walk(&mut self, e: &'p Expr, ctx: Ctx<'p>) {
        let saved = self.user;
        if e.span.file == isc_types::USER_FILE {
            self.user = e.span;
        }
        self.walk_in(e, ctx);
        self.user = saved;
    }

    fn walk_in(&mut self, e: &'p Expr, ctx: Ctx<'p>) {
        use ExprKind::*;
        match &e.kind {
            Copy(_) => {
                let (what, bytes) = self.size(e);
                let why = self
                    .copy_reasons
                    .get_mut(&(e.span.line, e.span.col))
                    .and_then(|v| v.pop())
                    .unwrap_or_else(|| "a borrowed value must be owned here".into());
                if matches!(e.ty, Type::Rec(_)) {
                    // a node is not copied: it gets one owner more
                    self.push(e, "refcount", what, bytes, format!("one owner more (the count is incremented): {}", why), None);
                } else if matches!(ctx, Ctx::Result) {
                    // copied into the result: the destination of the caller
                    // (section 8), no array of its own
                    self.push(e, "copy", what, bytes, format!("{}; into the result, the destination of the caller", why), None);
                } else {
                    self.push(e, "copy", what, bytes, why, None);
                }
            }
            Index(a, i) => {
                let shown = format!("{}[{}]", show(a, self.p), show(i, self.p));
                self.push(e, "bounds", shown, None, "checked at run time".into(), None);
            }
            Update { target, index, .. } => {
                let shown = format!("{}.with({}, …)", show(target, self.p), show(index, self.p));
                self.push(e, "bounds", shown, None, "checked at run time".into(), None);
            }
            CheckSize(a, b, _) => {
                let shown = format!("{} = {}", show(a, self.p), show(b, self.p));
                self.push(e, "size", shown, None, "sizes not proved equal: checked at run time".into(), None);
            }
            // a node of a recursive type (SPEC section 8)
            Box(_) => {
                let name = match &e.ty {
                    Type::Rec(i) => self.p.recs[*i as usize].name.clone(),
                    _ => "?".into(),
                };
                self.push(e, "refcount", format!("`{}`", name), None, "a recursive type: a node on the heap, with a reference count".into(), None);
            }
            // recursion (SPEC section 11): a direct tail call is a loop, any
            // other call uses the stack; an array result is materialized (M2)
            Call(f, _) => {
                let name = self.p.fns[*f as usize].name.clone();
                let tail = self.tails.contains(&(e as *const Expr));
                if tail {
                    self.push(e, "loop", format!("`{}`", name), None, "a direct tail call: compiled as a loop".into(), None);
                } else if self.others.contains(&(e as *const Expr)) {
                    self.push(e, "tail", format!("`{}`", name), None, "a tail call: its frame replaces the frame of the caller".into(), None);
                } else {
                    self.push(e, "stack", format!("`{}`", name), None, "a call of a function that is not inlined (recursive, or too large, D336): it uses the stack".into(), None);
                }
                // a jump gives no result of its own
                if e.ty.has_heap() && !tail {
                    let (what, bytes) = self.size(e);
                    self.push(e, "alloc", what, bytes, format!("the result of the function `{}`, called (rule M2)", name), Some("M2"));
                }
            }
            CheckNonNegative(a, _) => {
                let shown = format!("{} >= 0", show(a, self.p));
                self.push(e, "size", shown, None, "size not proved non-negative: checked at run time".into(), None);
            }
            _ if Self::allocates(e) => {
                let (what, bytes) = self.size(e);
                let (why, rule) = self.why(e, ctx);
                self.push(e, "alloc", what, bytes, why, rule);
            }
            _ => {}
        }
        if let Let { var, init, stmt, .. } = &e.kind {
            self.stmts.insert(*var, *stmt);
            if let Some(n) = self.len_of(init) {
                self.lens.insert(*var, n);
            }
        }
        let slots = fusable_slots(e);
        let lam = lambda_body_index(e);
        for (i, c) in e.children().into_iter().enumerate() {
            // a filter left in a fusable slot is read in order (`place_streams`
            // binds the others): fused by rule F2
            let fused = slots.contains(&i) && (is_producer(c) || matches!(c.kind, Filter(..)));
            let cctx = match &e.kind {
                _ if fused => Ctx::Read,
                Let { var, body, .. } => {
                    if i == 0 {
                        Ctx::Let(*var, body)
                    } else {
                        ctx
                    }
                }
                If(..) if i > 0 => ctx,
                Free(..) => ctx,
                CheckSize(..) if i == 2 => ctx,
                CheckNonNegative(..) if i == 1 => ctx,
                Action(..) => Ctx::Action,
                Call(..) => Ctx::CallArg,
                // the accumulator of a `fold` that is the result, the array a
                // `with` that is the result updates: the result's memory
                Fold(_, init, _) if std::ptr::eq(&**init, c) && matches!(ctx, Ctx::Result) => Ctx::Result,
                Update { target, .. } if std::ptr::eq(&**target, c) && matches!(ctx, Ctx::Result) => Ctx::Result,
                Fold(_, init, _) if std::ptr::eq(&**init, c) => Ctx::FoldInit,
                Update { target, .. } if std::ptr::eq(&**target, c) => Ctx::UpdateTarget,
                Update { .. } | Tuple(_) | ArrayLit(_) => Ctx::Part,
                Window(..) if i == 0 => Ctx::WindowSource,
                Slice(..) if i == 0 => Ctx::ViewSource,
                Index(..) if i == 0 && e.ty.is_array() => Ctx::ViewSource,
                Copy(_) => Ctx::ViewSource,
                Bench(..) if i == 1 => Ctx::BenchResult,
                _ if Some(i) == lam => Ctx::LambdaResult,
                _ => Ctx::Read,
            };
            if fused {
                // evaluated element by element inside the consumer: its own
                // inputs and lambda are walked, it allocates nothing
                self.walk_fused(c, e);
                continue;
            }
            let looped = Some(i) == lam;
            if looped {
                let l = self.loop_label(e);
                self.loops.push(l);
            }
            self.walk(c, cctx);
            if looped {
                self.loops.pop();
            }
        }
    }

    /// A producer fused into the loop of `consumer`: its lambda runs at each
    /// iteration of that loop, which the report names after the producer.
    fn walk_fused(&mut self, e: &'p Expr, consumer: &'p Expr) {
        let saved = self.user;
        if e.span.file == isc_types::USER_FILE {
            self.user = e.span;
        }
        let slots = fusable_slots(e);
        let lam = lambda_body_index(e);
        for (i, c) in e.children().into_iter().enumerate() {
            if slots.contains(&i) && (is_producer(c) || matches!(c.kind, ExprKind::Filter(..))) {
                self.walk_fused(c, consumer);
                continue;
            }
            let looped = Some(i) == lam;
            if looped {
                let l = self.loop_label(e);
                self.loops.push(l);
            }
            self.walk(c, if looped { Ctx::LambdaResult } else { Ctx::Read });
            if looped {
                self.loops.pop();
            }
        }
        self.user = saved;
    }
}

/// The report as text (SPEC section 8).
pub fn text(file: &str, es: &[Entry]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "Memory report — {}", file);
    if es.is_empty() {
        let _ = writeln!(s, "  no allocation, no copy, no run-time test");
    }
    for e in es {
        let _ = writeln!(s, "  line {:<4} {:<11} {:<13} {}", e.line, e.kind, e.what, e.why);
    }
    s
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The report as JSON: `{"file": ..., "entries": [{"line", "column", "kind",
/// "what", "bytes", "rule", "why"}]}`.
pub fn json(file: &str, es: &[Entry]) -> String {
    let mut s = format!("{{\"file\": {}, \"entries\": [", json_str(file));
    for (i, e) in es.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        let bytes = e.bytes.map(|b| b.to_string()).unwrap_or_else(|| "null".into());
        let rule = e.rule.map(json_str).unwrap_or_else(|| "null".into());
        let _ = write!(
            s,
            "{{\"line\": {}, \"column\": {}, \"kind\": {}, \"what\": {}, \"bytes\": {}, \"rule\": {}, \"why\": {}}}",
            e.line,
            e.column,
            json_str(e.kind),
            json_str(&e.what),
            bytes,
            rule,
            json_str(&e.why)
        );
    }
    s.push_str("]}\n");
    s
}
