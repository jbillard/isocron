//! The memory check of the IR (`isc_ir::verify`, DECISIONS D96) rejects every
//! defect of the ownership pass that it is meant to catch. Real programs are
//! compiled to their optimized IR, which passes the check; then one decision
//! is changed (a free removed or added, a move turned into a borrow, a copy
//! removed, a temporary left unbound) and the check must fail.

use isc_ir::{Expr, ExprKind, Program, Use, VarId};

fn ir(src: &str) -> Program {
    iscc::frontend("v.isc", src).unwrap_or_else(|e| panic!("{}", e))
}

/// Applies `f` to the expressions of `e` in evaluation order until it returns
/// true; returns whether it did.
fn mutate_first(e: &mut Expr, f: &mut dyn FnMut(&mut Expr) -> bool) -> bool {
    if f(e) {
        return true;
    }
    for c in e.children_mut() {
        if mutate_first(c, f) {
            return true;
        }
    }
    false
}

fn rejected(p: &Program, want: &str) {
    match isc_ir::verify::verify(p) {
        Ok(()) => panic!("the mutated program passes the memory check (wanted: {})", want),
        Err(d) => assert!(d.message.contains(want), "{}", d.message),
    }
}

const UPDATE: &str = "action main(args: string[k]): i32 {
    const a = map(iota(10), (i) => f64(i));
    const b = a.with(0, 1.5);
    print(b);
    return 0;
}
";

const PRINT: &str = "action main(args: string[k]): i32 {
    print(map(iota(10), (i) => i * 2));
    return 0;
}
";

const SHARED: &str = "action main(args: string[k]): i32 {
    const a = map(iota(10), (i) => f64(i));
    const b = a.with(0, 1.5);
    print(a);
    print(b);
    return 0;
}
";

#[test]
fn the_compiled_programs_pass() {
    for src in [UPDATE, PRINT, SHARED] {
        isc_ir::verify::verify(&ir(src)).unwrap();
    }
}

#[test]
fn a_missing_free_is_rejected() {
    let mut p = ir(PRINT);
    let done = mutate_first(&mut p.body, &mut |e| match &mut e.kind {
        ExprKind::Let { drop, .. } if *drop => {
            *drop = false;
            true
        }
        _ => false,
    });
    assert!(done, "no freed variable in the program");
    rejected(&p, "is never freed");
}

#[test]
fn a_free_after_a_move_is_rejected() {
    let mut p = ir(UPDATE);
    // `a` is moved into the update: free it as well
    let a = p.vars.iter().position(|v| v.name == "a").unwrap() as VarId;
    let done = mutate_first(&mut p.body, &mut |e| match &mut e.kind {
        ExprKind::Let { var, drop, .. } if *var == a && !*drop => {
            *drop = true;
            true
        }
        _ => false,
    });
    assert!(done, "`a` is not moved");
    rejected(&p, "is moved here and also freed");
}

#[test]
fn a_move_turned_into_a_borrow_is_rejected() {
    let mut p = ir(UPDATE);
    let done = mutate_first(&mut p.body, &mut |e| match &mut e.kind {
        ExprKind::Var(_, u) if *u == Use::Move => {
            *u = Use::Borrow;
            true
        }
        _ => false,
    });
    assert!(done, "no move in the program");
    rejected(&p, "would be freed twice");
}

#[test]
fn a_missing_copy_is_rejected() {
    let mut p = ir(SHARED);
    // `a` is used again after the update, so the update works on a copy
    let done = mutate_first(&mut p.body, &mut |e| {
        if let ExprKind::Copy(x) = &mut e.kind {
            let sp = x.span;
            let inner = std::mem::replace(&mut **x, Expr::new(ExprKind::Zero, isc_ir::Type::Unit, sp));
            *e = inner;
            return true;
        }
        false
    });
    assert!(done, "no copy in the program");
    rejected(&p, "would be freed twice");
}

#[test]
fn an_unbound_temporary_is_rejected() {
    let mut p = ir(PRINT);
    // `let tmp = map(...) in print(tmp)` back to `print(map(...))`
    let done = mutate_first(&mut p.body, &mut |e| {
        let ExprKind::Let { var, init, body, drop: true, .. } = &mut e.kind else { return false };
        let ExprKind::Action(_, args) = &mut body.kind else { return false };
        let Some(slot) = args.iter_mut().find(|a| matches!(a.kind, ExprKind::Var(v, _) if v == *var)) else { return false };
        *slot = std::mem::replace(&mut **init, Expr::new(ExprKind::Zero, isc_ir::Type::Unit, slot.span));
        let b = std::mem::replace(&mut **body, Expr::new(ExprKind::Zero, isc_ir::Type::Unit, e.span));
        *e = b;
        true
    });
    assert!(done, "no bound temporary in the program");
    rejected(&p, "never freed");
}

#[test]
fn a_view_that_outlives_its_source_is_rejected() {
    let mut p = ir(PRINT);
    // `let tmp = map(...) in print(tmp)` becomes `let tmp = map(...) in tmp`:
    // the result borrows `tmp`, freed at the end of the `let`
    let done = mutate_first(&mut p.body, &mut |e| {
        let ExprKind::Let { var, init, body, drop: true, .. } = &mut e.kind else { return false };
        let ty = init.ty.clone();
        let sp = body.span;
        **body = Expr::var(*var, ty, sp);
        true
    });
    assert!(done, "no freed variable in the program");
    rejected(&p, "which is freed at the end of its scope");
}

#[test]
fn args_never_freed_is_rejected() {
    let mut p = ir(PRINT);
    assert!(p.drop_args, "`main` does not free `args`");
    p.drop_args = false;
    rejected(&p, "`args` is never freed");
}

#[test]
fn args_moved_and_freed_is_rejected() {
    // `args` given to an update at its last use: moved, so not freed
    let mut p = ir("action main(args: string[k]): i32 {
    const b = args.with(0, args[1]);
    print(b);
    return 0;
}
");
    assert!(!p.drop_args, "`args` is not moved");
    p.drop_args = true;
    rejected(&p, "`args` is moved here and also freed");
}

#[test]
fn a_slice_of_new_memory_is_rejected() {
    let mut p = ir("action main(args: string[k]): i32 {
    const x = map(iota(20), (i) => f64(i));
    print(sum(x.slice(2, 8)));
    return 0;
}
");
    // `x[2..8]` becomes `copy(x)[2..8]`: a view inside a new block (not a
    // producer, which a slice would compute: D101) that nothing frees
    let x = p.vars.iter().position(|v| v.name == "x").unwrap() as VarId;
    let done = mutate_first(&mut p.body, &mut |e| match &mut e.kind {
        ExprKind::Slice(s, _, _) if matches!(s.kind, ExprKind::Var(v, _) if v == x) => {
            let inner = (**s).clone();
            let (ty, sp) = (inner.ty.clone(), inner.span);
            **s = Expr::new(ExprKind::Copy(Box::new(inner)), ty, sp);
            true
        }
        _ => false,
    });
    assert!(done, "no slice of `x`");
    rejected(&p, "a slice of new memory");
}

#[test]
fn a_filter_read_by_index_is_rejected() {
    // `let filtered = filter(...) in filtered[0]` back to `filter(...)[0]`
    let mut p = ir("action main(args: string[k]): i32 {
    const x = read_f64(args[0]);
    print(filter(x, (v) => v > 0.5)[0]);
    return 0;
}
");
    let done = mutate_first(&mut p.body, &mut |e| {
        let ExprKind::Let { var, init, body, .. } = &mut e.kind else { return false };
        if !matches!(init.kind, ExprKind::Filter(..)) {
            return false;
        }
        let f = (**init).clone();
        let v = *var;
        let mut b = (**body).clone();
        let replaced = mutate_first(&mut b, &mut |x| {
            if matches!(x.kind, ExprKind::Var(w, _) if w == v) {
                *x = f.clone();
                return true;
            }
            false
        });
        *e = b;
        replaced
    });
    assert!(done, "no bound filter in the program");
    rejected(&p, "a `filter` read by index");
}

#[test]
fn a_function_returning_its_borrowed_parameter_is_rejected() {
    // the copy of `x` returned by `doubled` removed: the result would be the
    // memory of the caller
    let mut p = ir("function doubled(x: f64[n], k: i64): f64[n] {
    if (k == 0) {
        return x;
    } else {
        return doubled(map(x, (v) => v * 2.0), k - 1);
    }
}

action main(args: string[k]): i32 {
    print(doubled([1.0, 2.0], 3));
    return 0;
}
");
    let done = mutate_first(&mut p.fns[0].body, &mut |e| {
        if let ExprKind::Copy(x) = &mut e.kind {
            let sp = x.span;
            let inner = std::mem::replace(&mut **x, Expr::new(ExprKind::Zero, isc_ir::Type::Unit, sp));
            *e = inner;
            return true;
        }
        false
    });
    assert!(done, "no copy in `doubled`");
    // found by the test of the branches (new memory in one, borrowed in the
    // other) or of the result
    rejected(&p, "borrow");
}

#[test]
fn a_node_returned_without_a_new_owner_is_rejected() {
    // `drop_n` returns its parameter `l` (a list it borrows) with one owner
    // more (the count is incremented); without it the result would be the
    // node of the caller, freed twice
    let mut p = ir("type List = Nil | Cons(i64, List);

function drop_n(l: List, k: i64): List {
    if (k == 0) {
        return l;
    } else {
        return match (l) {
            Nil => Nil,
            Cons(h, t) => drop_n(t, k - 1),
        };
    }
}

action main(args: string[k]): i32 {
    print(drop_n(Cons(1, Cons(2, Nil)), 1));
    return 0;
}
");
    let done = mutate_first(&mut p.fns[0].body, &mut |e| {
        if let ExprKind::Copy(x) = &mut e.kind {
            let sp = x.span;
            let inner = std::mem::replace(&mut **x, Expr::new(ExprKind::Zero, isc_ir::Type::Unit, sp));
            *e = inner;
            return true;
        }
        false
    });
    assert!(done, "no new owner in `drop_n`");
    rejected(&p, "borrow");
}

#[test]
fn a_variable_consumed_in_one_branch_only_is_rejected() {
    // `a` is moved in one branch and freed at the end of the other: without
    // the free, it would be lost on that path
    let mut p = ir("function half(x: f64[n], c: boolean): f64 {
    const a = map(x, (v) => v * 2.0);
    if (c) {
        return sum(a.with(1, 5.0));
    } else {
        return sum(x);
    }
}

action main(args: string[k]): i32 {
    print(half([1.0, 2.0], parse_i64(args[0]) > 0));
    return 0;
}
");
    let done = mutate_first(&mut p.body, &mut |e| {
        if let ExprKind::Free(_, body) = &mut e.kind {
            let inner = std::mem::replace(&mut **body, Expr::new(ExprKind::Zero, isc_ir::Type::Unit, e.span));
            *e = inner;
            return true;
        }
        false
    });
    assert!(done, "no free in the program");
    rejected(&p, "in one branch of this `if` only");
}
