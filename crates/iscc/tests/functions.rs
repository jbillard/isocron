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
