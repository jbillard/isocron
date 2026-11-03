//! Stage 2, phase 4 (SPEC section 9): `requireNoAlloc`, `requireFused`,
//! `requireTail`, with positive and negative tests for each (DECISIONS
//! D106). An accepted program is also run, compiled and interpreted.

mod common;
use common::*;

fn rejected(src: &str, want: &[&str]) {
    let e = iscc::frontend("req.isc", src).unwrap_err();
    for w in want {
        assert!(e.contains(w), "wanted `{}` in:\n{}", w, e);
    }
}

fn accepted(src: &str, name: &str) {
    if let Err(e) = iscc::frontend("req.isc", src) {
        panic!("rejected:\n{}", e);
    }
    let bin = compile(src, name);
    let r = run_bin_threads(&bin, &[], Some(2));
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, interp_ok(src, &[]));
}

#[test]
fn require_tail() {
    // the example of the specification
    accepted(
        "@requireTail
function gcd(a: u64, b: u64): u64 {
    if (b == 0) {
        return a;
    } else {
        return gcd(b, a % b);
    }
}

@requireTail
function steps(x: f64[n], k: i64): f64 {
    if (k == 0) {
        return sum(x);
    } else {
        return steps(map(x, (v) => v * 0.5), k - 1);
    }
}

action main(args: string[k]): i32 {
    print(gcd(48, 18));
    print(steps([1.0, 2.0], 100000));
    return 0;
}
",
        "req_tail_ok",
    );
    rejected(
        "@requireTail
function fact(n: i64): i64 {
    if (n <= 1) {
        return 1;
    } else {
        return n * fact(n - 1);
    }
}

action main(args: string[k]): i32 {
    print(fact(5));
    return 0;
}
",
        &["requireTail not satisfied (line 2)", "the call of `fact` at line 6 is not in tail position"],
    );
    // a mutual recursion in tail position reuses the frame (D113); not in
    // tail position, it is refused
    rejected(
        "@requireTail
function even(n: i64): boolean {
    if (n == 0) {
        return true;
    } else if (odd(n - 1)) {
        return true;
    } else {
        return false;
    }
}

function odd(n: i64): boolean {
    if (n == 0) {
        return false;
    } else {
        return even(n - 1);
    }
}

action main(args: string[k]): i32 {
    print(even(4));
    return 0;
}
",
        &["the call of `odd` at line 5 (a mutual recursion) is not in tail position"],
    );
    rejected(
        "@requireTail
function swap(x: f64[n], y: f64[n], k: i64): f64 {
    if (k == 0) {
        return sum(x);
    } else {
        return swap(y, x, k - 1);
    }
}

action main(args: string[k]): i32 {
    print(swap([1.0], [2.0], 3));
    return 0;
}
",
        &["passes an array (line 6) that is neither the same parameter nor a new array"],
    );
}

#[test]
fn require_fused() {
    accepted(
        "@requireFused
function scale(x: f64[n]): f64[n] {
    return map(x, (v) => v * 2.0) + 1.0;
}

@requireFused
function shared(x: f64[n]): f64 {
    const y = map(x, (v) => v * 2.0);
    return sum(y) + max(y);
}

@requireFused
function positive_sum(x: f64[n]): f64 {
    return sum(filter(x, (v) => v > 0.0));
}

action main(args: string[k]): i32 {
    const x = [1.0, -2.0, 3.0];
    print(scale(x));
    print(shared(x));
    print(positive_sum(x));
    return 0;
}
",
        "req_fused_ok",
    );
    // the example of the specification
    rejected(
        "@requireFused
function center(x: f64[n]): f64[n] {
    const y = x * 2.0;
    return y - mean(y);
}

action main(args: string[k]): i32 {
    print(center([1.0, 2.0]));
    return 0;
}
",
        &["requireFused not satisfied (line 2)", "line 3: f64[] shared result (rule M1): `y` is used 2 times, at line 4"],
    );
    rejected(
        "@requireFused
function running(x: i64[n]): i64 {
    return sum(scan(x, (+), 0));
}

action main(args: string[k]): i32 {
    print(running([1, 2, 3]));
    return 0;
}
",
        &["the result of `scan` (rule M2)"],
    );
}

#[test]
fn require_no_alloc() {
    accepted(
        "@requireNoAlloc
function norm2(x: f64[n]): f64 {
    return sqrt(sum(x * x));
}

@requireNoAlloc
function counts(x: u8[n]): u32[256] {
    return fold(x, zeros(), (c, v) => requireUnique(c.with(v, c[v] + 1)));
}

@requireNoAlloc
function set0(x: f64[n]): f64[n] {
    return x.with(0, 1.0);
}

@requireNoAlloc
function sum_of<T extends Num>(x: T[n]): T {
    return sum(x);
}

action main(args: string[k]): i32 {
    print(norm2([3.0, 4.0]));
    print(sum(map(counts([1, 1, 7]), (c) => i64(c))));
    print(set0([5.0, 6.0]));
    print(sum_of([1, 2]) + i64(sum_of([0.5, 1.5])));
    return 0;
}
",
        "req_no_alloc_ok",
    );
    rejected(
        "@requireNoAlloc
function lit(x: f64[n]): f64 {
    return sum(x) + sum([x[0], 2.0]);
}

action main(args: string[k]): i32 {
    print(lit([1.0]));
    return 0;
}
",
        &["requireNoAlloc not satisfied (line 2)", "line 3: 16 bytes an array literal"],
    );
    // a literal of constants is data of the program, not allocated (D173)
    accepted(
        "@requireNoAlloc
function lit(x: f64[n]): f64 {
    return sum(x) + sum([1.0, 2.0]) + [5.0, 7.0][1];
}

action main(args: string[k]): i32 {
    print(lit([1.0]));
    return 0;
}
",
        "req_no_alloc_const",
    );
    // a copy that is not the result
    rejected(
        "@requireNoAlloc
function bumped(x: f64[n]): f64 {
    return sum(x.with(0, 1.0));
}

action main(args: string[k]): i32 {
    print(bumped([1.0, 2.0]));
    return 0;
}
",
        &["`x` is shared"],
    );
    // a node of a recursive type, and the result of a recursive function
    rejected(
        "type List = Nil | Cons(i64, List);

@requireNoAlloc
function one(x: i64): i64 {
    return match (Cons(x, Nil)) {
        Nil => 0,
        Cons(h, t) => h,
    };
}

action main(args: string[k]): i32 {
    print(one(3));
    return 0;
}
",
        &["a recursive type: a node on the heap"],
    );
    rejected(
        "function up(x: f64[n], k: i64): f64[n] {
    if (k == 0) {
        return map(x, (v) => v);
    } else {
        return map(up(x, k - 1), (v) => v + 1.0);
    }
}

@requireNoAlloc
function total(x: f64[n]): f64 {
    return sum(up(x, 2));
}

action main(args: string[k]): i32 {
    print(total([1.0]));
    return 0;
}
",
        &["the result of the recursive function `up` (rule M2)"],
    );
}

#[test]
fn requirements_are_for_functions() {
    rejected(
        "@requireFused
action main(args: string[k]): i32 {
    return 0;
}
",
        &["a decorator goes on a `function`, not on an `action`"],
    );
}

/// `allowReassoc` (SPEC section 10, D109): the floating-point operations of
/// the function (and of what it inlines) carry the flags of
/// `-fassociative-math` (`reassoc nsz contract`), the others none; the
/// result is close to the strict one; the memory report lists the function.
#[test]
fn allow_reassoc() {
    let src = "@allowReassoc
function fast_dot(a: f64[n], b: f64[n]): f64 {
    return sum(a * b);
}

function strict_dot(a: f64[n], b: f64[n]): f64 {
    return sum(a * b) + 0.0;
}

action main(args: string[k]): i32 {
    const a = readF64(args[0]);
    const b = map(a, (v) => 1.0 - v);
    writeF64(args[1], [fast_dot(a, b), strict_dot(a, b)]);
    return 0;
}
";
    let p = iscc::frontend("r.isc", src).unwrap();
    let ir = isc_codegen::emit_with(&p, &isc_codegen::Options::default()).unwrap();
    assert!(ir.contains("fmul reassoc nsz contract"), "no flags");
    assert!(ir.contains("fmul double") || ir.contains("fmul <"), "the strict function lost its strict operations");
    let es = isc_ir::report::entries(&p);
    assert_eq!(es.iter().filter(|e| e.kind == "reassoc").map(|e| e.what.as_str()).collect::<Vec<_>>(), vec!["`fast_dot`"]);
    let x = write_f64s("reassoc_x.f64", &random_f64s(2200, 70_001));
    let o = tmp("reassoc.out").to_string_lossy().to_string();
    let oi = tmp("reassoc_i.out").to_string_lossy().to_string();
    let r = run_bin_threads(&compile(src, "reassoc"), &[&x, &o], Some(4));
    assert_eq!(r.code, 0, "{}", r.stderr);
    interp_ok(src, &[&x, &oi]);
    let (c, i) = (read_f64s(&o), read_f64s(&oi));
    // the strict function: the same bits; the other: close
    assert_eq!(c[1].to_bits(), i[1].to_bits());
    assert!(((c[0] - i[0]) / i[0]).abs() < 1e-12, "{} {}", c[0], i[0]);
}

/// Under `allowReassoc`, a `+` of floats over a filter is a masked
/// reduction: a rejected element counts as 0.0, no compaction (D123); the
/// result is close to the strict one, which keeps its stream.
#[test]
fn reassociated_filters_are_masked() {
    let src = "@allowReassoc
function fast(x: f64[n]): f64 {
    return sum(filter(x, (v) => v > 0.0)) + reduce(map(filter(x, (v) => v < -0.5), (v) => 1.0 + v * 1e-6), (*), 1.0);
}

function strict(x: f64[n]): f64 {
    return sum(filter(x, (v) => v > 0.0)) + reduce(map(filter(x, (v) => v < -0.5), (v) => 1.0 + v * 1e-6), (*), 1.0);
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    writeF64(args[1], [fast(x), strict(x)]);
    return 0;
}
";
    let p = iscc::frontend("m.isc", src).unwrap();
    let filters = isc_ir::print::program(&p).matches("filter").count();
    // the strict function keeps its two filters, the other has none
    assert_eq!(filters, 2, "{}", isc_ir::print::program(&p));
    let x = write_f64s("masked_x.f64", &random_f64s(2800, 70_001).iter().map(|v| v.clamp(-1.5, 1.5)).collect::<Vec<_>>());
    let bin = compile(src, "masked");
    let oi = tmp("masked_i.out").to_string_lossy().to_string();
    interp_ok(src, &[&x, &oi]);
    let i = read_f64s(&oi);
    for t in [1, 4] {
        let o = tmp(&format!("masked_{}.out", t)).to_string_lossy().to_string();
        let r = run_bin_threads(&bin, &[&x, &o], Some(t));
        assert_eq!(r.code, 0, "{}", r.stderr);
        let c = read_f64s(&o);
        assert_eq!(c[1].to_bits(), i[1].to_bits(), "the strict function, {} threads", t);
        assert!(((c[0] - i[0]) / i[0]).abs() < 1e-12, "{} {}", c[0], i[0]);
    }
}
