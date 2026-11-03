//! Functions as values beyond D114 (the v1 of SPEC section 11): functions
//! returned by functions (D152), and the other holes of the guide's "Not
//! implemented yet". The interpreter and the compiled program print the
//! same text, with 1 and 4 threads, and the compiled program frees
//! everything (Valgrind, when installed).

mod common;
use common::*;

fn valgrind_clean(src: &str, name: &str, args: &[&str]) {
    let vg = std::path::PathBuf::from("/usr/bin/valgrind");
    if !vg.is_file() {
        eprintln!("valgrind not installed: {} skipped", name);
        return;
    }
    let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
    let bin = compile_with(src, &format!("{}_vg", name), opts);
    let out = std::process::Command::new(vg)
        .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
        .arg(&bin)
        .args(args)
        .env("ISC_ALLOC", "glibc")
        .env("ISC_THREADS", "2")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{} under valgrind:\n{}", name, err);
}

fn agree(src: &str, name: &str, args: &[&str], expected: &str) {
    assert_eq!(interp_ok(src, args), expected, "interpreter");
    let bin = compile(src, name);
    for t in [1, 4] {
        let r = run_bin_threads(&bin, args, Some(t));
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.stdout, expected, "compiled, {} threads", t);
    }
    valgrind_clean(src, name, args);
}

fn error(src: &str) -> String {
    match iscc::frontend("test.isc", src) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    }
}

const RETURNED: &str = r#"function adder(k: f64): (x: f64) => f64 {
    return (x) => x + k;
}

function compose(f: (x: f64) => f64, g: (x: f64) => f64): (x: f64) => f64 {
    return (x) => g(f(x));
}

function clamper(xs: f64[n]): (x: f64) => f64 {
    const m = sum(xs) / f64(n);
    const [lo, hi] = [m - 1.0, m + 1.0] as const;
    return (x) => x < lo ? lo : x > hi ? hi : x;
}

function pick(up: boolean): (x: f64) => f64 {
    if (up) return adder(1.0);
    return (x) => x * 10.0;
}

function repeat(k: size): (v: f64) => f64[k] {
    return (v) => fill(k, v);
}

action main(args: string[k]): i32 {
    const add3 = adder(3.0);
    print(add3(1.5));
    print(map(iota(4), (i) => adder(f64(i))(0.5)));
    const h = compose(adder(1.0), (x) => x * 2.0);
    print(map([1.0, 2.0, 3.0], h));
    print(map([0.0, 5.0, 10.0], clamper([4.0, 6.0])));
    print(map([1.0, 2.0], pick(args.length > 0)));
    print(map([1.0, 2.0], pick(args.length > 5)));
    print(iota(3).map((i) => f64(i)).map(adder(sum([1.0, 2.0]))));
    print(repeat(3)(0.5));
    return 0;
}
"#;

/// A function returned by a function (D152): the callee is inlined, the
/// values the returned function reads computed once.
#[test]
fn functions_returned_by_functions() {
    agree(RETURNED, "fn_returned", &["a"], "4.5\n[0.5, 1.5, 2.5, 3.5]\n[4, 6, 8]\n[4, 5, 6]\n[2, 3]\n[10, 20]\n[3, 4, 5]\n[0.5, 0.5, 0.5]\n");
    // the value read is computed once, outside the loop of `map`
    let src = "function adder(k: f64): (x: f64) => f64 {\n    return (x) => x + k;\n}\n\naction main(args: string[k]): i32 {\n    const x = readF64(args[0]);\n    print(sum(map(x, adder(sum(x)))));\n    return 0;\n}\n";
    let p = iscc::frontend("t.isc", src).unwrap();
    let ir = isc_ir::print::program(&p);
    let k = ir.find("let k%").expect("the value read is bound");
    let m = ir.find("map/1").unwrap();
    assert!(k < m, "{}", ir);
    // a recursive function cannot return a function: it is not inlined
    let e = error("function f(n: i64): (x: i64) => i64 {\n    if (n == 0) return (x) => x;\n    return f(n - 1);\n}\n\naction main(args: string[k]): i32 {\n    print(f(3)(1));\n    return 0;\n}\n");
    assert!(e.contains("a function returned by a recursive function"), "{}", e);
}

const DEEP: &str = r#"type Scale = By(f64) | Not;

type Op = Scaled([f64, Scale]) | Shift(Scale, f64) | Keep;

type List = Nil | Cons(f64, List);

action main(args: string[k]): i32 {
    const ops = [Scaled([1.0, By(3.0)] as const), Shift(By(2.0), 0.5), Shift(Not, 7.0), Keep];
    for_each(ops);
    const l = Cons(2.0, Cons(5.0, Nil));
    const f = match (l) {
        Cons(a, Cons(b, _)) => (v) => v * a + b,
        _ => (v) => v,
    };
    print(map([1.0, 2.0], f));
    return 0;
}

action for_each(ops: Op[n]): void {
    print(map(iota(n), (i) => {
        const f = match (ops[i]) {
            Scaled([w, By(c)]) => (v) => v * w * c,
            Shift(By(c), d) => (v) => v + c + d,
            Shift(Not, d) => (v) => v - d,
            _ => (v) => v,
        };
        return f(10.0);
    }));
}
"#;

const SHORT: &str = r#"type List = Nil | Cons(f64, List);

function build(n: i64): List {
    if (n == 0) return Nil;
    return Cons(f64(n), build(n - 1));
}

action main(args: string[k]): i32 {
    const l = build(parseI64(args[0]));
    const f = match (l) {
        Cons(a, Cons(b, _)) => (v) => v * a + b,
        _ => (v) => v,
    };
    print(map([1.0, 2.0], f));
    return 0;
}
"#;

/// A function chosen by `match` reads variables bound at any depth of its
/// pattern (D152); a part read through a node of a recursive type below the
/// first level is read only when the arm matches.
#[test]
fn functions_chosen_by_deep_patterns() {
    agree(DEEP, "fn_deep", &[], "[30, 12.5, 3, 10]\n[7, 9]\n");
    for (n, want) in [("0", "[1, 2]\n"), ("1", "[1, 2]\n"), ("2", "[3, 5]\n"), ("3", "[5, 8]\n")] {
        agree(SHORT, &format!("fn_short_{}", n), &[n], want);
    }
}

const REC_SIZES: &str = r#"function iterate(f: (x: f64) => f64, x: f64, k: i64): f64 {
    if (k == 0) return x;
    return iterate(f, f(x), k - 1);
}

function depth(f: (x: i64) => i64, x: i64): i64 {
    if (x <= 1) return 0;
    return 1 + depth(f, f(x));
}

function normalized(xs: f64[n]): f64 {
    return iterate((v) => v / f64(n), 1000.0, 3);
}

function spread(xs: f64[n], k: size): f64 {
    return iterate((v) => sum(fill(k, v)) + f64(n), 1.0, 2);
}

action main(args: string[k]): i32 {
    print(normalized([1.0, 2.0, 3.0, 4.0]));
    print(spread([1.0, 2.0], 3));
    const m = args.length;
    print(depth((x) => x / (m + 1), 1000));
    return 0;
}
"#;

/// A recursive function given a lambda that reads sizes of its context
/// (D152): the sizes, like the values it captures (D131), are parameters
/// of the specialization.
#[test]
fn recursive_functions_given_lambdas_that_read_sizes() {
    agree(REC_SIZES, "fn_rec_sizes", &["a", "b"], "15.625\n17\n6\n");
}

const IN_SUMS: &str = r#"type Option<T> = None | Some(T);

type Op = Apply((x: f64) => f64) | Const(f64);

type Pipe = End | Step((x: f64) => f64, Pipe);

function double(v: f64): f64 {
    return v * 2.0;
}

function mk(f: (x: f64) => f64): Op {
    return Apply(f);
}

function run(op: Op, x: f64): f64 {
    return match (op) {
        Apply(f) => f(x),
        Const(c) => c,
    };
}

function through(p: Pipe, x: f64): f64 {
    return match (p) {
        End => x,
        Step(f, rest) => through(rest, f(x)),
    };
}

function orElse(o: Option<(x: f64) => f64>, x: f64): f64 {
    return match (o) {
        Some(f) => f(x),
        None => -1.0,
    };
}

action main(args: string[k]): i32 {
    const up = args.length > 0;
    const ops = [Apply((v) => v + 1.0), Const(7.0), mk(Math.sqrt), Apply(up ? double : (v) => v * v), mk(double)];
    print(map(ops, (o) => run(o, 16.0)));
    const p = Step(double, Step((v) => v - 1.0, Step(Math.sqrt, End)));
    print(through(p, 12.5));
    print(orElse(Some((v) => v * 10.0), 2.0));
    print(orElse(None, 2.0));
    const g = match (ops[0]) {
        Apply(f) => f,
        Const(c) => (x) => c,
    };
    print(g(1.0));
    return 0;
}
"#;

/// Functions in sum types (D153): a value holds the code of its function
/// among those the program stores in that field; a call is a branch on
/// the code. A generic type, a recursive type walked by a recursive
/// function, a function chosen at run time, a function given to a helper.
#[test]
fn functions_in_sum_types() {
    agree(IN_SUMS, "fn_in_sums", &["a"], "[17, 7, 4, 32, 32]\n4.898979485566356\n20\n-1\n2\n");
    agree(IN_SUMS, "fn_in_sums_b", &[], "[17, 7, 4, 256, 32]\n4.898979485566356\n20\n-1\n2\n");
    let e = error("type Op = Apply((x: f64) => f64) | Const(f64);\n\naction main(args: string[k]): i32 {\n    const k = 3.0;\n    const o = Apply((v) => v * k);\n    return 0;\n}\n");
    assert!(e.contains("test.isc:5:21: error: a function stored in a value of a sum type cannot read a variable or a size of its context"), "{}", e);
    let e = error("type Op = Apply((x: f64) => f64) | Const(f64);\n\naction main(args: string[k]): i32 {\n    print(Apply((v) => v));\n    return 0;\n}\n");
    assert!(e.contains("a function cannot be printed"), "{}", e);
    let e = error("type Op = Apply((x: f64) => f64) | Const(f64);\n\naction main(args: string[k]): i32 {\n    print(Apply((v) => v) == Const(1.0));\n    return 0;\n}\n");
    assert!(e.contains("class Eq"), "{}", e);
}

/// D202: the base case of a recursive function (`if (c) base else ...`,
/// `c` and `base` without call, allocation or panic, `base` a scalar or a
/// constructor without payload) is computed by its callers, the call made
/// only for the other branch. A number, a tree, a list, mutual recursion;
/// left as calls: a base case that divides (it may panic, at its own line),
/// one that makes an array, a condition that calls. The interpreter's
/// results and panic.
#[test]
fn base_cases_computed_by_the_callers() {
    let src = "type Tree = Leaf | Node(Tree, i64, Tree);
type List = Nil | Cons(i64, List);
function fib(n: i64): i64 {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}
function make(d: i64, k: i64): Tree {
    if (d == 0) return Leaf;
    return Node(make(d - 1, 2 * k), k, make(d - 1, 2 * k + 1));
}
function weigh(t: Tree): i64 {
    return match (t) {
        Leaf => 0,
        Node(l, v, r) => v + weigh(l) - weigh(r),
    };
}
function upTo(k: i64): List {
    if (k == 0) return Nil;
    return Cons(k, upTo(k - 1));
}
function len(l: List): i64 {
    return match (l) {
        Nil => 0,
        Cons(x, rest) => 1 + len(rest),
    };
}
function isEven(n: i64): boolean {
    if (n == 0) return true;
    return isOdd(n - 1);
}
function isOdd(n: i64): boolean {
    if (n == 0) return false;
    return isEven(n - 1);
}
function share(n: i64, d: i64): i64 {
    if (n == 0) return 100 / d;
    return share(n - 1, d) + 1;
}
function ramp(n: i64): i64[2] {
    if (n == 0) return [7, 8];
    return map(ramp(n - 1), (x) => x + 1);
}
function deep(n: i64): i64 {
    if (fib(n) < 3) return 1;
    return deep(n - 1) + deep(n - 2);
}
action main(args: string[k]): i32 {
    print(fib(25));
    print(weigh(make(12, 1)));
    print(len(upTo(5000)));
    print([isEven(10), isOdd(7), isEven(3)]);
    print(share(5, 4));
    print(ramp(6));
    print(deep(15));
    print(share(3, args.length - 1));
    return 0;
}
";
    let bin = compile(src, "base_cases");
    let want = interp_panic(src, &["x"]);
    let r = run_bin(&bin, &["x"]);
    assert_eq!(r.code, 101, "{}", r.stderr);
    assert!(r.stderr.contains(&want), "{} / {}", r.stderr, want);
    let want_out = "75025\n";
    assert!(r.stdout.starts_with(want_out), "{}", r.stdout);
    let full = interp_ok(&src.replace("    print(share(3, args.length - 1));\n", ""), &[]);
    let bin2 = compile(&src.replace("    print(share(3, args.length - 1));\n", ""), "base_cases_ok");
    let r2 = run_bin(&bin2, &[]);
    assert_eq!((r2.code, r2.stdout.as_str()), (0, full.as_str()), "{}", r2.stderr);
    assert_eq!(r.stdout, full, "the output before the panic");
}

/// D203: `x.length` is the length whatever the program declares: a
/// function `len` of its own (which `len(x)` calls) does not change it.
#[test]
fn length_is_not_a_function_of_the_program() {
    let src = "function len(x: i64): i64 {
    return x + 1;
}
action main(args: string[k]): i32 {
    const xs = [4, 5, 6];
    print([xs.length, len(41), args.length, \"abc\".length]);
    return 0;
}
";
    agree(src, "own_len", &["a", "b"], "[3, 42, 2, 3]\n");
}

/// D204: the lengths fixed by the types (`f64[3]`) are constants of the
/// generated code, and every test of a size still reads the real one: an
/// array of a size known at run time given to a function or to a recursive
/// function (tested on entry) that expects `f64[3]` panics as before; an
/// index out of a fixed array too. A record of fixed arrays through a tail
/// loop, an array of arrays. The interpreter's outputs and panics.
#[test]
fn fixed_lengths_keep_every_test_of_a_size() {
    let src = "type Pair = { a: f64[3], b: f64[3] };
function norm3(x: f64[3]): f64 {
    return Math.sqrt(sum(x * x));
}
function spin(p: Pair, k: i64): Pair {
    if (k == 0) return p;
    return spin({ a: p.b, b: p.a + p.b * 0.5 }, k - 1);
}
function last(x: f64[3], k: i64): f64 {
    if (k == 0) return x[2];
    return last(x, k - 1);
}
action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const dyn = fill(n, 1.5);
    const p = spin({ a: [1.0, 2.0, 3.0], b: [0.5, 0.25, 0.125] }, 20);
    print(p.a);
    print(norm3(p.b));
    const m = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
    print(map(m, (r) => sum(r)));
    print(m[1][2]);
    const i = parseI64(args[1]);
    print([1, 2, 3, 4, 5][i]);
    print(parseI64(args[2]) == 0 ? norm3(dyn) : last(dyn, 3));
    return 0;
}
";
    let bin = compile(src, "fixed_lengths");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains(", i64 3, 1"), "no length fixed in the generated code");
    let want = interp_ok(src, &["3", "1", "0"]);
    let r = run_bin(&bin, &["3", "1", "0"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}", r.stderr);
    let want = interp_ok(src, &["3", "1", "1"]);
    let r = run_bin(&bin, &["3", "1", "1"]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}", r.stderr);
    for args in [["4", "1", "0"], ["4", "1", "1"], ["2", "1", "1"], ["3", "5", "0"]] {
        let m = interp_panic(src, &args);
        let r = run_bin(&bin, &args);
        assert_eq!(r.code, 101, "{:?}: {}", args, r.stderr);
        assert!(r.stderr.contains(&m), "{:?}: {} / {}", args, r.stderr, m);
    }
}

/// D205: a loop of a constant count of at most 16 (a `fold` over `iota(10)`,
/// a `map` over a fixed array) is unrolled whole (`llvm.loop.unroll.full`),
/// beside a `fold` of 17 steps. The interpreter's outputs.
#[test]
fn short_loops_unrolled_whole() {
    let src = "function pairs(x: f64[5]): f64 {
    const pi = [0, 0, 0, 0, 1, 1, 1, 2, 2, 3];
    const pj = [1, 2, 3, 4, 2, 3, 4, 3, 4, 4];
    return fold(iota(10), 0.0, (acc, k) => acc + Math.sqrt(1.0 + (x[pi[k]] - x[pj[k]]) * (x[pi[k]] - x[pj[k]])));
}
action main(args: string[k]): i32 {
    const x = [1.0, 2.5, -3.0, 4.25, 0.5];
    print(pairs(x));
    print(fold(iota(17), 1, (a, k) => a * 3 % 1000003 + k));
    print(map(x, (v) => v * 2.0 + 1.0));
    return 0;
}
";
    let bin = compile(src, "short_loops");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains("!{!\"llvm.loop.unroll.full\"}"), "no loop unrolled whole");
    let want = interp_ok(src, &[]);
    let r = run_bin(&bin, &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}", r.stderr);
}

/// D208: the allocations are declared `noalias` (a new block shares no
/// byte with another), so that LLVM keeps the arrays of a loop that does
/// not allocate in registers; the interpreter's outputs.
#[test]
fn allocations_are_new_memory() {
    let src = "type S = { a: f64[4], b: f64[4] };
function run(s: S, n: i64): S {
    if (n == 0) return s;
    return run({ a: s.a + s.b * 0.5, b: s.b * 0.75 + 0.125 }, n - 1);
}
action main(args: string[k]): i32 {
    const r = run({ a: [1.0, 2.0, 3.0, 4.0], b: [0.5, 0.25, 0.125, 1.0] }, 1000);
    print(r.a);
    print(r.b);
    return 0;
}
";
    let bin = compile(src, "noalias_allocs");
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    assert!(ll.contains("declare noalias ptr @isc_alloc(i64)"), "isc_alloc is not declared noalias");
    let want = interp_ok(src, &[]);
    let r = run_bin(&bin, &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{}", r.stderr);
}

/// D228: the length of an array known before running (the bytes of a
/// constant string, a literal array, `iota` and `fill` of a constant length,
/// a `map` of one) is a constant in the IR: `i % alu.length` a remainder by
/// a constant. The interpreter's results; no `len` of them left in the IR.
#[test]
fn lengths_known_before_running() {
    let src = "const ALU = \"GGCCGGGCGCGGTGGCTCACGCCTGTAATCCCAGCACTTTGGGAGGCCGAGGCGGGCGGATCACCTGAGG\";
action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const alu = bytes(ALU);
    const lit = [3, 1, 4, 1, 5];
    const ids = iota(7);
    const twice = map(fill(5, 2), (x) => x * 2);
    printBytes(map(iota(n), (i) => alu[i % alu.length]));
    print(\"\");
    print(sum(map(iota(n), (i) => lit[i % lit.length] + ids[i % ids.length] + twice[i % twice.length])));
    print([alu.length, lit.length, ids.length, twice.length] as const);
    return 0;
}
";
    let bin = compile(src, "constant_lengths");
    for n in ["0", "1", "71", "100000"] {
        let want = interp_ok(src, &[n]);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &[n], Some(t));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "n {} {} threads: {}", n, t, r.stderr);
        }
    }
    let p = iscc::frontend("constant_lengths.isc", src).unwrap();
    let ir = isc_ir::print::program(&p);
    assert!(!ir.lines().any(|l| l.trim() == "len"), "{}", ir);
}

/// D230: a function of an element of a small table, costly and unable to
/// panic whatever the element, computed once per element of the table when
/// the `map` has at least twice as many elements: each element of the `map`
/// reads the new table. The interpreter's results (the floats printed
/// whole: the same bits), below and above that size, 1 and 4 threads; not
/// done for a mere read; an index of the table past its end still panics.
#[test]
fn functions_of_table_elements_computed_once() {
    let src = "const P = [0.27, 0.12, 0.12, 0.27, 0.22];
const CODES = \"acgtN\";
action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const shift = parseI64(args[1]);
    const values = map(iota(1000), (i) => (i * 7919) % 1000);
    const cp = map(iota(5), (i) => fold(iota(i + 1), 0.0, (c, j) => c + P[j]));
    const codes = bytes(CODES);
    printBytes(map(iota(n), (t) => {
        const r = f64(values[(t + shift) % 1000]) / 1000.0;
        return codes[Math.min(count(cp, (c) => c <= r), 4)];
    }));
    print(\"\");
    print(sum(map(iota(n), (t) => Math.sqrt(f64(values[t % 1000])) / 3.0)));
    print(sum(map(iota(n), (t) => values[(t * 3) % 1000] + 1)));
    print(sum(map(iota(n), (t) => f64(codes[values[t % 1000] % 7]) / 2.0)));
    return 0;
}
";
    let bin = compile(src, "table_functions");
    for args in [["0", "0"], ["10", "3"], ["1999", "0"], ["2000", "7"], ["50000", "11"], ["3000", "-2001"]] {
        let want = interp(src, &args);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &args, Some(t));
            match &want {
                Outcome::Exit(code, out) => assert_eq!((r.code, r.stdout.as_str()), (*code, out.as_str()), "{:?} {} threads: {}", args, t, r.stderr),
                Outcome::Panic(m, _) => {
                    assert_eq!(r.code, 101, "{:?}: {}", args, r.stderr);
                    assert!(r.stderr.contains(m.as_str()), "{:?} {} threads: {} / {}", args, t, r.stderr, m);
                }
            }
        }
    }
    let p = iscc::frontend("table_functions.isc", src).unwrap();
    let ir = isc_ir::print::program(&p);
    // tables: the code of the bytes, the root, the half of an element of
    // `codes` (read at a remainder of 7, past its 5 bytes: the new table of
    // 5 panics where `codes` did); not the read alone (`+ 1`)
    assert_eq!(ir.lines().filter(|l| l.contains(" table%") && l.trim_start().starts_with("let")).count(), 3, "{}", ir);
}
