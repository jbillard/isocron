use isc_ir::{lower::lower, passes::optimize, print, ExprKind, Program};

fn build(src: &str) -> Program {
    let t = isc_types::check_source(src).unwrap_or_else(|e| panic!("{}", e));
    let mut p = lower(&t).unwrap_or_else(|e| panic!("{}", e));
    optimize(&mut p).unwrap_or_else(|e| panic!("{}", e));
    p
}

fn count(e: &isc_ir::Expr, f: &dyn Fn(&ExprKind) -> bool) -> usize {
    let mut n = if f(&e.kind) { 1 } else { 0 };
    for c in e.children() {
        n += count(c, f);
    }
    n
}

const SUM: &str = include_str!("../../../examples/poc/sum.isc");
const DOT: &str = include_str!("../../../examples/poc/dot.isc");
const HIST: &str = include_str!("../../../examples/poc/histogram.isc");
const STD: &str = include_str!("../../../examples/poc/std_dev.isc");

#[test]
fn dump() {
    for src in [SUM, DOT, HIST, STD] {
        let p = build(src);
        println!("{}", print::program(&p));
    }
}

/// Number of `map` nodes that are materialized: bound by a `let` or passed to
/// an action, i.e. not sitting in a consumer slot.
fn materialized_maps(e: &isc_ir::Expr) -> usize {
    let mut n = 0;
    if let ExprKind::Let { init, .. } = &e.kind {
        if matches!(init.kind, ExprKind::Map(..)) {
            n += 1;
        }
    }
    for c in e.children() {
        n += materialized_maps(c);
    }
    n
}

#[test]
fn std_dev_is_fused() {
    let p = build(STD);
    // two traversals of x: one reduce for the mean, one for the sum of squares
    assert_eq!(count(&p.body, &|k| matches!(k, ExprKind::Reduce(..))), 2);
    // (x - m) ^ 2 is never materialized (rule F1)
    assert_eq!(materialized_maps(&p.body), 0, "{}", print::program(&p));
}

#[test]
fn dot_is_fused_and_checked() {
    let p = build(DOT);
    assert_eq!(count(&p.body, &|k| matches!(k, ExprKind::CheckSize(..))), 1);
    assert_eq!(materialized_maps(&p.body), 0, "{}", print::program(&p));
}

#[test]
fn histogram_updates_in_place() {
    let p = build(HIST);
    let txt = print::program(&p);
    // the accumulator is moved into the update: no copy, not freed
    assert_eq!(count(&p.body, &|k| matches!(k, ExprKind::Copy(_))), 0, "{}", txt);
    assert!(txt.contains("(move)"), "{}", txt);
    assert!(!txt.contains("!drop"), "{}", txt);
}

#[test]
fn require_unique_fails_when_shared() {
    let src = "function f(c: u32[n]): u32[n] {\n    const d = requireUnique(c.with(0, 1));\n    return d;\n}\n\naction main(args: string[k]): i32 {\n    return 0;\n}\n";
    let t = isc_types::check_source(src).unwrap();
    // `f` is never called: nothing to analyze, accepted
    let mut p = lower(&t).unwrap();
    optimize(&mut p).unwrap();
    let src = "action main(args: string[k]): i32 {\n    const c = readBytes(args[0]);\n    const d = requireUnique(c.with(0, 1));\n    print(c[0]);\n    return 0;\n}\n";
    let t = isc_types::check_source(src).unwrap();
    let mut p = lower(&t).unwrap();
    let e = optimize(&mut p).unwrap_err();
    assert!(e.message.contains("requireUnique not satisfied"), "{}", e);
    assert!(e.message.contains("used again at line 4"), "{}", e);
    // last use: in place
    let src = "action main(args: string[k]): i32 {\n    const c = readBytes(args[0]);\n    const d = requireUnique(c.with(0, 1));\n    print(d[0]);\n    return 0;\n}\n";
    let t = isc_types::check_source(src).unwrap();
    let mut p = lower(&t).unwrap();
    optimize(&mut p).unwrap();
}

/// Rule F4 (D97): a slice of an array of scalars is a view, never copied nor
/// freed, even shared; it is copied only when a value must own it (the
/// target of an update, a component of a tuple). A slice of an array of
/// tuples is a copy.
#[test]
fn slices_are_views() {
    let p = build(
        "action main(args: string[k]): i32 {
    const x = map(iota(20), (i) => f64(i));
    const s = x.slice(2, 8);
    print(sum(s) + sum(x.slice(10, 20)));
    print(x.slice(3, 9).slice(1, 4));
    print(s);
    return 0;
}
",
    );
    assert_eq!(count(&p.body, &|k| matches!(k, ExprKind::Copy(_))), 0, "{}", print::program(&p));
    let freed_slices = |e: &isc_ir::Expr| {
        count(e, &|k| matches!(k, ExprKind::Let { init, drop: true, .. } if matches!(init.kind, ExprKind::Slice(..))))
    };
    assert_eq!(freed_slices(&p.body), 0, "{}", print::program(&p));
    let p = build(
        "action main(args: string[k]): i32 {
    const x = map(iota(20), (i) => f64(i));
    const s = x.slice(2, 8);
    const t = s.with(0, 1.5);
    const u = [x.slice(0, 2), 7] as const;
    print(t);
    print(u);
    const q = map(iota(6), (i) => [i, f64(i)] as const);
    print(q.slice(1, 3));
    return 0;
}
",
    );
    // the update and the tuple each copy their slice; the slice of tuples is
    // a new array, freed after `print`
    assert_eq!(count(&p.body, &|k| matches!(k, ExprKind::Copy(_))), 2, "{}", print::program(&p));
    assert_eq!(freed_slices(&p.body), 1, "{}", print::program(&p));
}

/// Rule F4 (D97): a shared zip is never bound to a `let`; only an action
/// that receives it materializes it (rule M3).
#[test]
fn shared_zip_is_not_materialized() {
    let p = build(
        "function prod(p: [f64, f64]): f64 {
    const [u, v] = p;
    return u * v;
}

action main(args: string[k]): i32 {
    const a = map(iota(10), (i) => f64(i));
    const b = map(iota(10), (i) => f64(i) * 2.0);
    const z = zip(a, b);
    print(sum(map(z, prod)));
    print(sum(map(iota(3), (i) => prod(z[i]))));
    print(z);
    return 0;
}
",
    );
    let bound_tuples = count(&p.body, &|k| {
        matches!(k, ExprKind::Let { init, .. } if matches!(&init.ty, isc_ir::Type::Array(t) if matches!(**t, isc_ir::Type::Tuple(_))))
    });
    // `print(z)` only
    assert_eq!(bound_tuples, 1, "{}", print::program(&p));
}

/// The other use is inside `sum`, a function of the prelude: the error names
/// the line of the program that calls it, not a line of the prelude (D98).
#[test]
fn require_unique_names_the_line_of_the_program() {
    let src = "action main(args: string[k]): i32 {
    const y = map(iota(5), (i) => f64(i));
    const t = requireUnique(y.with(0, 5.0));
    print(t);
    print(sum(y));
    return 0;
}
";
    let t = isc_types::check_source(src).unwrap();
    let mut p = lower(&t).unwrap();
    let e = optimize(&mut p).unwrap_err();
    assert!(e.message.contains("`y` is used again at line 5"), "{}", e);
}

/// Rule F2 (D99): no filter read in order is bound to a `let`; a `reduce` of
/// `max` with its identity becomes masked (no filter left); a filter read by
/// index is bound (materialized).
#[test]
fn filters_are_fused() {
    let p = build(
        "function s(x: f64[n]): f64 {
    return sum(filter(x, (v) => v > 0.25));
}

function m(x: f64[n]): f64 {
    return max(filter(x, (v) => v > 0.25));
}

function t(x: f64[n]): f64 {
    return filter(x, (v) => v > 0.25)[0];
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    print(s(x) + m(x) + t(x));
    return 0;
}
",
    );
    let bound_filters = count(&p.body, &|k| matches!(k, ExprKind::Let { init, .. } if matches!(init.kind, ExprKind::Filter(..))));
    let filters = count(&p.body, &|k| matches!(k, ExprKind::Filter(..)));
    // `s`: a stream; `m`: masked, no filter; `t`: bound
    assert_eq!(filters, 2, "{}", print::program(&p));
    assert_eq!(bound_filters, 1, "{}", print::program(&p));
}

/// Rule F3 (D100): the four `reduce`s of `stats` become one; the two of
/// `dep` stay apart (the second uses the first); in an action, two
/// statements are not grouped (section 12), one expression is.
#[test]
fn independent_reductions_are_grouped() {
    let p = build(
        "function stats(x: f64[n]): f64 {
    const s = sum(x);
    const q = sum(map(x, (v) => v * v));
    const hi = max(x);
    return s + q + hi + min(x);
}

function dep(x: f64[n]): f64 {
    const m = sum(x);
    return sum(map(x, (v) => v - m));
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    print(stats(x));
    print(dep(x));
    const a = sum(x);
    const b = max(x);
    print(a + b + sum(x) + min(x));
    return 0;
}
",
    );
    let reduces = count(&p.body, &|k| matches!(k, ExprKind::Reduce(..)));
    // stats: 1; dep: 2; the statements `a`, `b`: 2; the last expression: 1
    assert_eq!(reduces, 6, "{}", print::program(&p));
}

/// A recursive function is not inlined (SPEC section 11, D102): one function
/// per type arguments, calls in the IR; one with a function parameter is
/// specialized per function given (D128), its captured values as
/// parameters (D131); not yet a lambda that reads a size of its context.
#[test]
fn recursive_functions_are_functions() {
    let p = build(
        "function count<T>(x: T[n], i: i64): i64 {
    if (i == n) {
        return 0;
    } else {
        return 1 + count(x, i + 1);
    }
}

action main(args: string[k]): i32 {
    print(count([1.0, 2.0], 0) + count([1, 2, 3], 0));
    return 0;
}
",
    );
    assert_eq!(p.fns.len(), 2, "{}", print::program(&p));
    assert_eq!(count(&p.body, &|k| matches!(k, ExprKind::Call(..))), 2);
    let p = build(
        "function apply(f: (i64) => i64, n: i64): i64 {
    if (n == 0) {
        return 0;
    } else {
        return f(n) + apply(f, n - 1);
    }
}

function twice(v: i64): i64 {
    return v * 2;
}

action main(args: string[k]): i32 {
    print(apply((v) => v * 2, 3) + apply(twice, 4) + apply(twice, 5));
    return 0;
}
",
    );
    // one specialization per function given
    assert_eq!(p.fns.len(), 2, "{}", print::program(&p));
    let t = isc_types::check_source(
        "function apply(f: (i64) => i64, n: i64): i64 {
    if (n == 0) {
        return 0;
    } else {
        return f(n) + apply(f, n - 1);
    }
}

function sized(x: f64[m]): i64 {
    return apply((v) => v * m, 3);
}

action main(args: string[k]): i32 {
    print(sized([1.0, 2.0]));
    return 0;
}
",
    )
    .unwrap();
    let e = lower(&t).unwrap_err();
    assert!(e.message.contains("a recursive function given a function that reads a size or a function of its context"), "{}", e);
}

/// Uniqueness in the branches of an `if` (D105): the last use of a variable
/// on each path moves it; a branch that does not use it frees it; nothing
/// is copied; `requireUnique` holds in a branch.
#[test]
fn moves_in_branches() {
    let p = build(
        "function pick(x: f64[n], c: boolean): f64[n] {
    const a = map(x, (v) => v + 1.0);
    if (c) {
        return requireUnique(a.with(0, 100.0));
    } else {
        return a;
    }
}

function half(x: f64[n], c: boolean): f64 {
    const a = map(x, (v) => v * 2.0);
    if (c) {
        return sum(requireUnique(a.with(1, 5.0)));
    } else {
        return sum(x);
    }
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    print(pick(x, true));
    print(half(x, false));
    return 0;
}
",
    );
    assert_eq!(count(&p.body, &|k| matches!(k, ExprKind::Copy(_))), 0, "{}", print::program(&p));
    assert_eq!(count(&p.body, &|k| matches!(k, ExprKind::Free(..))), 1, "{}", print::program(&p));
    // a use after the `if`: the branches cannot move
    let t = isc_types::check_source(
        "action main(args: string[k]): i32 {
    const a = readF64(args[0]);
    const b = parseI64(args[1]) > 0 ? requireUnique(a.with(0, 1.0)) : a;
    print(a);
    print(b);
    return 0;
}
",
    )
    .unwrap();
    let mut p = lower(&t).unwrap();
    let e = optimize(&mut p).unwrap_err();
    assert!(e.message.contains("`a` is used again at line 4"), "{}", e);
}
