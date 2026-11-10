//! Stage 2, phase 5 (SPEC sections 6 and 10): the array primitives beyond
//! the POC. Each is compared with the interpreter bit for bit, with 1 and 4
//! threads, and its errors are checked.

mod common;
use common::*;

const SIZES: [usize; 7] = [0, 1, 31, 33, 4096, 4097, 70_001];

/// Compiles `src`, runs it with the interpreter and compiled with 1 and 4
/// threads on `inputs` (files) plus an output file, and compares the outputs
/// and the printed text.
fn same_everywhere(src: &str, name: &str, inputs: &[&str]) {
    let bin = compile(src, name);
    let o_ref = tmp(&format!("{}_ref.out", name)).to_string_lossy().to_string();
    let mut a = inputs.to_vec();
    a.push(&o_ref);
    let printed = interp_ok(src, &a);
    for t in [1, 4] {
        let o = tmp(&format!("{}_{}.out", name, t)).to_string_lossy().to_string();
        let mut a = inputs.to_vec();
        a.push(&o);
        let r = run_bin_threads(&bin, &a, Some(t));
        assert_eq!(r.code, 0, "{} with {} threads: {}", name, t, r.stderr);
        assert_eq!(r.stdout, printed, "{} with {} threads: printed text", name, t);
        assert_eq!(std::fs::read(&o).unwrap(), std::fs::read(&o_ref).unwrap(), "{} with {} threads", name, t);
    }
}

const ZIP: &str = "function weighted(p: [f64, f64]): f64 {
    const [v, w] = p;
    return v * w;
}

function larger(p: [f64, f64]): [f64, f64] {
    const [v, w] = p;
    if (v > w) {
        return [v, w] as const;
    } else {
        return [w, v] as const;
    }
}

action main(args: string[k]): i32 {
    const a = readF64(args[0]);
    const b = readF64(args[1]);
    const z = zip(a, b);
    const s = reduce(map(z, weighted), (x, y) => x + y, 0.0);
    const m = map(z, larger);
    const firsts = map(m, (p) => {
        const [v, w] = p;
        return v;
    });
    writeF64(args[2], firsts);
    print(s);
    print(z[0]);
    return 0;
}
";

#[test]
fn zip_matches_the_interpreter() {
    for (i, n) in SIZES.iter().enumerate() {
        if *n == 0 {
            continue; // `z[0]` would be out of bounds
        }
        let a = write_f64s(&format!("zip_a{}.f64", n), &random_f64s(700 + i as u64, *n));
        let b = write_f64s(&format!("zip_b{}.f64", n), &random_f64s(800 + i as u64, *n));
        same_everywhere(ZIP, &format!("zip_{}", n), &[&a, &b]);
    }
}

#[test]
fn zip_sizes_are_checked() {
    // proved different: a compilation error naming both types
    let e = iscc::frontend("z.isc", "action main(args: string[k]): i32 {\n    print(zip(iota(3), iota(4)));\n    return 0;\n}\n")
        .err()
        .expect("zip of [3] and [4] must be rejected");
    assert!(e.contains("size mismatch: `zip` of `i64[3]` and `i64[4]`"), "{}", e);
    // not provable: a panic at run time, compiled and interpreted
    let src = "action main(args: string[k]): i32 {\n    const tags = map(args, (s) => 1);\n    print(zip(tags, iota(2)));\n    return 0;\n}\n";
    let bin = compile(src, "zip_panic");
    let r = run_bin(&bin, &["x"]);
    assert_eq!(r.code, 101, "{}", r.stderr);
    assert!(r.stderr.contains("size mismatch: 1 != 2"), "{}", r.stderr);
    assert!(interp_panic(src, &["x"]).contains("size mismatch: 1 != 2"));
    // equal sizes known only at run time: no panic
    let r = run_bin(&bin, &["x", "y"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout.trim(), "[(1, 0), (1, 1)]");
}

const MINMAX: &str = "action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const y = readF64(args[1]);
    writeF64(args[2], [min(x), max(x), reduce(x, min, 1e300), reduce(y, max, -1e300), min(y), max(y)]);
    print(map(zip(x, y), (p) => {
        const [a, b] = p;
        return [min(a, b), max(a, b)] as const;
    }));
    print(min(map(x, (v) => i64(v * 1000.0))));
    print(max(map(x, (v) => u8(i64(v * 10.0) % 200 + 50))));
    return 0;
}
";

/// Random values with NaNs, zeros of both signs and infinities among them.
fn special_f64s(seed: u64, n: usize) -> Vec<f64> {
    let mut v = random_f64s(seed, n);
    let specials = [f64::NAN, 0.0, -0.0, f64::INFINITY, f64::NEG_INFINITY];
    for (j, x) in v.iter_mut().enumerate() {
        if (j * 7 + seed as usize) % 13 == 0 {
            *x = specials[(j + seed as usize) % specials.len()];
        }
    }
    v
}

#[test]
fn min_max_follow_ieee_754_2019() {
    for (i, n) in [1usize, 2, 31, 33, 4097, 70_001].iter().enumerate() {
        let x = write_f64s(&format!("mm_x{}.f64", n), &special_f64s(900 + i as u64, *n));
        // a second array without NaN, so that the reductions mix signs of zero
        let y: Vec<f64> = special_f64s(950 + i as u64, *n).into_iter().map(|v| if v.is_nan() { -0.0 } else { v }).collect();
        let y = write_f64s(&format!("mm_y{}.f64", n), &y);
        same_everywhere(MINMAX, &format!("mm_{}", n), &[&x, &y]);
    }
    // the operations of the specification, on values chosen by hand
    let src = "action main(args: string[k]): i32 {\n    const nan = 0.0 / 0.0;\n    print([min(nan, 1.0), max(1.0, nan), 1.0 / min(0.0, -0.0), 1.0 / max(-0.0, 0.0)]);\n    print([min(3, -7), max(-3, 7)]);\n    print(min(map(args, (s) => 1.0)));\n    return 0;\n}\n";
    let bin = compile(src, "mm_hand");
    let r = run_bin(&bin, &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, "[NaN, NaN, -inf, inf]\n[-7, 7]\ninf\n");
    assert_eq!(interp_ok(src, &[]), r.stdout);
}

#[test]
fn min_max_reject_non_numbers() {
    for (src, want) in [
        ("action main(args: string[k]): i32 {\n    print(min(1.0, i64(2)));\n    return 0;\n}\n", "found `i64`"),
        ("action main(args: string[k]): i32 {\n    print(max([true, false]));\n    return 0;\n}\n", "bool"),
        ("action main(args: string[k]): i32 {\n    print(min(1.0));\n    return 0;\n}\n", "`min(x)` takes an array"),
        ("action main(args: string[k]): i32 {\n    print(min(1, 2, 3));\n    return 0;\n}\n", "argument"),
    ] {
        let e = iscc::frontend("m.isc", src).err().unwrap_or_else(|| panic!("must be rejected: {}", src));
        assert!(e.contains(want), "{}: {}", src, e);
    }
}

const COUNT: &str = "function positive(v: f64): boolean {
    return v > 0.0;
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const c = count(x, positive);
    const d = count(map(x, (v) => v * v), (v) => v < 0.25);
    writeF64(args[1], [f64(c), f64(d)]);
    print(c + d);
    return 0;
}
";

#[test]
fn count_matches_the_interpreter() {
    for (i, n) in SIZES.iter().enumerate() {
        let x = write_f64s(&format!("count_x{}.f64", n), &random_f64s(1000 + i as u64, *n));
        same_everywhere(COUNT, &format!("count_{}", n), &[&x]);
    }
}

const SLICE: &str = "action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const n = parseI64(args[1]);
    const a = n / 3;
    const b = n - n / 4;
    const s = x.slice(a, b);
    const pairs = zip(x, map(x, (v) => v * 2.0));
    const t = pairs.slice(a, b);
    const second = map(t, (p) => {
        const [u, v] = p;
        return v - u;
    });
    writeF64(args[2], s);
    print(sum(x.slice(a, b)));
    print(sum(second));
    print(x.slice(b, b));
    return 0;
}
";

#[test]
fn slices_match_the_interpreter() {
    for (i, n) in SIZES.iter().enumerate() {
        let x = write_f64s(&format!("slice_x{}.f64", n), &random_f64s(1100 + i as u64, *n));
        let ns = n.to_string();
        same_everywhere(SLICE, &format!("slice_{}", n), &[&x, &ns]);
    }
}

#[test]
fn slice_bounds_are_checked() {
    let src = "action main(args: string[k]): i32 {\n    const x = [1.0, 2.0, 3.0, 4.0];\n    print(x.slice(parseI64(args[0]), parseI64(args[1])));\n    return 0;\n}\n";
    let bin = compile(src, "slice_panic");
    for (a, b, ok) in [("0", "4", true), ("4", "4", true), ("-1", "2", false), ("3", "2", false), ("1", "5", false)] {
        let r = run_bin(&bin, &[a, b]);
        if ok {
            assert_eq!(r.code, 0, "{}..{}: {}", a, b, r.stderr);
            assert_eq!(r.stdout, interp_ok(src, &[a, b]));
        } else {
            let want = format!("slice out of bounds: [{}..{}] of an array of length 4", a, b);
            assert_eq!(r.code, 101, "{}..{}", a, b);
            assert!(r.stderr.contains(&want), "{}", r.stderr);
            assert!(interp_panic(src, &[a, b]).contains(&want));
        }
    }
}

const SCAN: &str = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const x = readF64(args[1]);
    const v = map(x, (f) => i64(f * 1000.0));
    const s1 = scan(v, (+), 7);
    const s2 = scan(v, (a, b) => b * a, 3);
    const s3 = scan(v, max, -5000);
    const s4 = scan(map(v, (i) => u32(i % 1000 + 1000)), min, u32(4000000000));
    const b = map(v, (i) => i % 5 != 0);
    const s5 = scan(b, (p, q) => p && q, true);
    const s6 = scan(b, (p, q) => q || p, false);
    const s7 = scan(x, (a, c) => a + c, 0.0);
    print([s1[n - 1], s2[n - 1], s3[n - 1], i64(s4[n - 1])]);
    print([count(s5, (t) => t), count(s6, (t) => t)]);
    print(sum(s1) + sum(s2));
    print(s7[n - 1]);
    writeF64(args[2], map(s1, (i) => f64(i)));
    return 0;
}
";

#[test]
fn scan_matches_the_interpreter_with_any_number_of_threads() {
    for (i, n) in [1usize, 4096, 4097, 70_001, 300_007].iter().enumerate() {
        let x = write_f64s(&format!("scan_x{}.f64", n), &random_f64s(1200 + i as u64, *n));
        let ns = n.to_string();
        same_everywhere(SCAN, &format!("scan_{}", n), &[&ns, &x]);
        let bin = compile(SCAN, &format!("scan8_{}", n));
        let o = tmp(&format!("scan8_{}.out", n)).to_string_lossy().to_string();
        let r = run_bin_threads(&bin, &[&ns, &x, &o], Some(8));
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(r.stdout, interp_ok(SCAN, &[&ns, &x, &tmp("scan8_ref.out").to_string_lossy()]));
    }
}

const FILTER: &str = "function positive(v: f64): boolean {
    return v > 0.0;
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const p = filter(x, positive);
    const q = filter(zip(x, map(x, (v) => v * v)), (t) => {
        const [v, w] = t;
        return w < 0.25;
    });
    const squares = map(q, (t) => {
        const [v, w] = t;
        return w;
    });
    writeF64(args[1], p);
    print(sum(p));
    print(sum(squares));
    print(sum(filter(map(x, (v) => v * 3.0), (v) => v < -1.0)));
    return 0;
}
";

#[test]
fn filter_matches_the_interpreter() {
    for (i, n) in SIZES.iter().enumerate() {
        let x = write_f64s(&format!("filter_x{}.f64", n), &random_f64s(1300 + i as u64, *n));
        same_everywhere(FILTER, &format!("filter_{}", n), &[&x]);
    }
}

const EXISTENTIAL: &str = "function positives(x: f64[n]): f64[k <= n] {
    return filter(x, (v) => v > 0.0);
}

function kept<T>(x: T[n], keep: (T) => boolean): T[m] {
    return filter(x, keep);
}

function inner(x: f64[n]): f64[k] {
    return x.slice(1, n - 1);
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const p = positives(x);
    writeF64(args[1], p);
    print(mean(p));
    print(sum(kept(iota(1000), (i) => i % 7 == 3)));
    print(sum(positives(inner(x))));
    return 0;
}
";

#[test]
fn existential_result_sizes_of_functions() {
    for (i, n) in [3usize, 33, 4097, 70_001].iter().enumerate() {
        let x = write_f64s(&format!("exist_x{}.f64", n), &random_f64s(1400 + i as u64, *n));
        same_everywhere(EXISTENTIAL, &format!("exist_{}", n), &[&x]);
    }
    for (sig, want) in [
        ("function f(x: f64[n]): f64[k <= m] {\n    return filter(x, (v) => v > 0.0);\n}\n", "must be made of sizes of the parameters"),
        ("function f(x: f64[k <= n]): f64 {\n    return 0.0;\n}\n", "can only be the type of a result"),
        ("function f(x: f64[n]): f64[2 * k] {\n    return x;\n}\n", "must appear alone"),
        ("function f(x: f64[n]): f64[n <= n] {\n    return x;\n}\n", "must be a new size variable"),
    ] {
        let src = format!("{}\n\naction main(args: string[k]): i32 {{\n    return 0;\n}}\n", sig);
        let e = iscc::frontend("e.isc", &src).err().unwrap_or_else(|| panic!("must be rejected: {}", sig));
        assert!(e.contains(want), "{}: {}", sig, e);
    }
}

const WINDOW: &str = "function moving_average(x: f64[n], k: size): f64[n - k + 1] {
    return map(window(x, k), (w) => sum(w) / f64(k));
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const m = moving_average(x, 64);
    writeF64(args[1], m);
    const w = window(x, 3);
    print(w[0]);
    print(sum(map(w, (r) => r[2] - r[0])));
    print(map([[1.5, 2.0], [3.0, 4.5]], (r) => r[0] * r[1]));
    return 0;
}
";

#[test]
fn window_matches_the_interpreter() {
    for (i, n) in [64usize, 65, 4097, 70_001].iter().enumerate() {
        let x = write_f64s(&format!("window_x{}.f64", n), &random_f64s(1500 + i as u64, *n));
        same_everywhere(WINDOW, &format!("window_{}", n), &[&x]);
    }
    let src = "action main(args: string[k]): i32 {\n    const x = map(args, (s) => 1.0);\n    print(window(x, 3));\n    return 0;\n}\n";
    let bin = compile(src, "window_panic");
    let r = run_bin(&bin, &["a", "b"]);
    assert_eq!(r.code, 101, "{}", r.stderr);
    assert!(r.stderr.contains("window of 3 elements over an array of length 2"), "{}", r.stderr);
    assert!(interp_panic(src, &["a", "b"]).contains("window of 3 elements over an array of length 2"));
    assert_eq!(run_bin(&bin, &["a", "b", "c"]).stdout, "[[1, 1, 1]]\n");
}

/// Rows given to a literal, borrowed (copied) or at their last use (moved).
#[test]
fn rows_given_to_a_literal() {
    let src = "action main(args: string[k]): i32 {
    const a = [1, 2];
    const m = [a, [3, 4]];
    print(m);
    print(a);
    const b = map(iota(3), (i) => i * 10);
    const n = [[7, 8, 9], b];
    print(n);
    writeF64(args[0], map(n[1], (v) => f64(v)));
    return 0;
}
";
    same_everywhere(src, "rows_literal", &[]);
}

/// Slices as views (rule F4, D97), against the interpreter.
#[test]
fn slices_as_views() {
    let src = "action main(args: string[k]): i32 {
    const x = map(iota(20), (i) => f64(i) * 1.5);
    const s = x.slice(2, 8);
    print(sum(s) + sum(x.slice(10, 20)));
    print(x.slice(3, 9).slice(1, 4));
    const t = s.with(0, 99.0);
    print(t);
    print(s);
    const p = [x.slice(0, 2), 7] as const;
    print(p);
    const y = x.with(5, -1.0);
    print(y.slice(4, 7));
    print(map(iota(6), (i) => [i, f64(i)] as const).slice(1, 3));
    writeF64(args[0], map(s, (v) => v * 2.0));
    return 0;
}
";
    same_everywhere(src, "slice_views", &[]);
}

/// A shared zip is never materialized (rule F4, D97): copied at each use,
/// read inside a lambda, over a `map` computed once, zipped again.
const SHARED_ZIP: &str = "function prod(p: [f64, f64]): f64 {
    const [u, v] = p;
    return u * v;
}

action main(args: string[k]): i32 {
    const a = readF64(args[0]);
    const b = readF64(args[1]);
    const z = zip(a, b);
    const s1 = sum(map(z, prod));
    const s2 = sum(map(iota(3), (i) => prod(z[i])));
    const c = map(a, (v) => v + 1.0);
    const w = zip(map(b, (v) => v * 3.0), c);
    const s3 = sum(map(w, prod));
    const s4 = sum(map(w, (p) => prod(p) + 1.0));
    const zz = zip(z, c);
    print(zz[1]);
    print(z[2]);
    writeF64(args[2], [s1, s2, s3, s4]);
    return 0;
}
";

#[test]
fn shared_zip() {
    for (i, n) in [3usize, 1000, 70_001].iter().enumerate() {
        let a = write_f64s(&format!("szip_a{}.f64", n), &random_f64s(1100 + i as u64, *n));
        let b = write_f64s(&format!("szip_b{}.f64", n), &random_f64s(1200 + i as u64, *n));
        same_everywhere(SHARED_ZIP, &format!("shared_zip{}", n), &[&a, &b]);
    }
    // sizes that differ: the same panic, once
    let a = write_f64s("szip_bad_a.f64", &random_f64s(1300, 5));
    let b = write_f64s("szip_bad_b.f64", &random_f64s(1301, 4));
    let o = tmp("szip_bad.out").to_string_lossy().to_string();
    let bin = compile(SHARED_ZIP, "shared_zip_bad");
    let r = run_bin_threads(&bin, &[&a, &b, &o], Some(1));
    assert_eq!(r.code, 101, "{}", r.stderr);
    assert!(r.stderr.contains("size mismatch: 5 != 4"), "{}", r.stderr);
}

/// Rule F2 (D99): a filter read in order by a `reduce`, a `fold` or a `map`,
/// in one pass; a `reduce` of an exact operator with its identity, masked;
/// the other reductions in the form of section 10 on the kept elements, at
/// the levels of the blocks (sizes beyond B kept elements); a filter read by
/// index, materialized.
const FILTERS: &str = "function pos(x: f64[n]): f64 {
    return sum(filter(x, (v) => v > 0.25));
}

function pos_sq(x: f64[n]): f64 {
    return sum(map(filter(x, (v) => v > 0.25), (v) => v * v));
}

function big(x: f64[n]): f64[k <= n] {
    return map(filter(x, (v) => v > 0.5), (v) => v * 10.0);
}

function cnt(x: f64[n]): i64 {
    return fold(filter(x, (v) => v < 0.1), 0, (c, v) => c + 1);
}

function mx(x: i64[n]): i64 {
    return max(filter(x, (v) => v % 3 == 0));
}

function odd(x: f64[n]): f64 {
    return reduce(filter(x, (v) => v > 0.3), (a, b) => a * 0.5 + b, 1.0);
}

function lo(x: f64[n]): f64 {
    return min(map(filter(x, (v) => v > 0.2), (v) => v * 2.0));
}

function third(x: f64[n]): f64 {
    return filter(x, (v) => v > 0.5)[2];
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const y = map(x, (v) => i64(v * 1000.0));
    const b = big(x);
    const t = count(x, (v) => v > 0.5) > 2 ? third(x) : 0.0;
    writeF64(args[1], [pos(x), pos_sq(x), sum(b), f64(cnt(x)), f64(mx(y)), odd(x), lo(x), t]);
    return 0;
}
";

#[test]
fn filters_fused() {
    for (i, n) in [0usize, 1, 100, 5000, 70_001, 1_000_003].iter().enumerate() {
        let x = write_f64s(&format!("f2_x{}.f64", n), &random_f64s(1400 + i as u64, *n));
        same_everywhere(FILTERS, &format!("filters{}", n), &[&x]);
    }
}

/// Rule F3 (D100): independent `reduce`s of one array in one pass, each with
/// the result of its own `reduce`; one that depends on another is left
/// alone.
const COLUMNS: &str = "function stats(x: f64[n]): [f64, f64, f64, f64] {
    const s = sum(x);
    const q = sum(map(x, (v) => v * v));
    const hi = max(x);
    const lo = min(map(x, (v) => v - 1.0));
    return [s, q, hi, lo] as const;
}

function spread(x: f64[n]): f64 {
    return max(x) - min(x);
}

function dep(x: f64[n]): f64 {
    const m = sum(x);
    return sum(map(x, (v) => v - m));
}

function ints(y: i64[n]): i64 {
    return sum(y) * 3 + max(y) - min(map(y, (v) => v * 2));
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const y = map(x, (v) => i64(v * 1000.0));
    const [s, q, hi, lo] = stats(x);
    writeF64(args[1], [s, q, hi, lo, spread(x), dep(x), f64(ints(y)), sum(x) + max(x)]);
    return 0;
}
";

#[test]
fn reductions_in_columns() {
    for (i, n) in [0usize, 1, 100, 5000, 70_001, 1_000_003].iter().enumerate() {
        let x = write_f64s(&format!("f3_x{}.f64", n), &random_f64s(1500 + i as u64, *n));
        same_everywhere(COLUMNS, &format!("columns{}", n), &[&x]);
    }
}

/// A slice of a producer computes only its elements (rule F1 for views,
/// D101); a window of a producer reads it several times (M1).
const SLICED_PRODUCERS: &str = "function mid(x: f64[n]): f64 {
    return sum(map(x, (v) => v * 3.0).slice(2, n - 2));
}

function part(x: f64[n], a: i64, b: i64): f64[k] {
    return map(x, (v) => v + 1.0).slice(a, b);
}

function pairs(x: f64[n]): f64 {
    const q = map(x, (v) => [v, v * 2.0] as const).slice(1, 3);
    return fold(q, 0.0, (c, p) => {
        const [u, w] = p;
        return c + u * w;
    });
}

function wsum(x: f64[n]): f64 {
    return sum(map(window(map(x, (v) => v * 0.5), 3), (r) => sum(r)));
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const p = part(x, 1, 4);
    const sq = map(iota(10), (i) => i * i).slice(3, 6);
    writeF64(args[1], [mid(x), sum(p), pairs(x), wsum(x), f64(sum(sq))]);
    return 0;
}
";

#[test]
fn slices_of_producers() {
    for (i, n) in [5usize, 5000, 70_001].iter().enumerate() {
        let x = write_f64s(&format!("sp_x{}.f64", n), &random_f64s(1600 + i as u64, *n));
        same_everywhere(SLICED_PRODUCERS, &format!("sliced{}", n), &[&x]);
    }
    // out of bounds: the same panic
    let x = write_f64s("sp_short.f64", &random_f64s(1700, 3));
    let o = tmp("sp_short.out").to_string_lossy().to_string();
    let bin = compile(SLICED_PRODUCERS, "sliced_short");
    let r = run_bin_threads(&bin, &[&x, &o], Some(1));
    assert_eq!(r.code, 101, "{}", r.stderr);
    assert!(r.stderr.contains("slice out of bounds: [1..4] of an array of length 3"), "{}", r.stderr);
}

/// The mathematical functions of the runtime (SPEC section 10, D108): the
/// same bits compiled (natively and as WebAssembly) and interpreted.
pub const MATH: &str = "action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const y = map(x, (v) => v * 10.0 - 5.0);
    const a = map(y, exp);
    const b = map(y, (v) => log(v * v + 1.0));
    const c = map(y, (v) => sin(v) + cos(v));
    const d = map(y, (v) => pow(v * v, 0.75));
    const e = map(y, (v) => fma(v, v, 1.0));
    const f = map(y, (v) => (v * v) ** 1.5);
    const g = map(y, (v) => f64(exp(f32(v)) + sin(f32(v)) * pow(f32(2.0), f32(v))));
    writeF64(args[1], [sum(a), sum(b), sum(c), sum(d), sum(e), sum(f), sum(g), exp(1.0), log(0.0), pow(-8.0, 1.0 / 3.0), sin(1e300)]);
    print(exp(1.0));
    print(log(-1.0));
    print(fma(0.1, 10.0, -1.0));
    return 0;
}
";

#[test]
fn math_functions() {
    for (i, n) in [0usize, 1, 1000, 70_001].iter().enumerate() {
        let x = write_f64s(&format!("math_x{}.f64", n), &random_f64s(2000 + i as u64, *n));
        same_everywhere(MATH, &format!("math{}", n), &[&x]);
    }
    let wasm = compile_wasm(MATH, "math_wasm");
    let Some(runner) = wasi_runner() else {
        eprintln!("no WASI engine (wasmtime): not run as WebAssembly");
        return;
    };
    let x = write_f64s("math_wx.f64", &random_f64s(2100, 5000));
    let (ow, on) = (tmp("math_w.out"), tmp("math_n.out"));
    let r = run_wasm(&runner, &wasm, &[&x, &ow.to_string_lossy()]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let r = run_bin_threads(&compile(MATH, "math_native"), &[&x, &on.to_string_lossy()], Some(1));
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(std::fs::read(&ow).unwrap(), std::fs::read(&on).unwrap(), "WebAssembly and native");
}

/// D613: `floor`, `ceil`, `trunc`, `round` and `abs` of floats are
/// instructions of the processor in the generated code, on vectors as on
/// scalars, not calls of the runtime: the same bits as the interpreter (whose
/// versions are musl's) for every instruction set, on 1 and 4 threads and as
/// WebAssembly, on the values where they could differ (the zeros and their
/// signs, the halves, the floats around 2^52, the least floats, the
/// infinities, NaN).
pub const ROUNDINGS: &str = "// every function on the values where it could differ: the same bits
// interpreted and compiled, scalar and in a `map`
action main(args: string[k]): i32 {
    const base = [0.0, -0.0, 0.25, -0.25, 0.5, -0.5, 0.49999999999999994, -0.49999999999999994, 1.5, -1.5, 2.5, -2.5, 3.5, -3.5,
        0.75, -0.75, 1.0, -1.0, 4503599627370495.5, -4503599627370495.5, 4503599627370496.0, -4503599627370496.0, 9007199254740993.0,
        1e300, -1e300, 5e-324, -5e-324, 2.2250738585072014e-308, 0.1, -0.1, 123456.5, -123456.5, 0.9999999999999999, -0.9999999999999999];
    const special = [parseF64(\"NaN\"), parseF64(\"inf\"), parseF64(\"-inf\")];
    const n = parseI64(args[0]);
    const xs = map(iota(n), (i) => i < 34 ? base[i] : i < 37 ? special[i - 34] : (i % 2 == 0 ? 1.0 : -1.0) * f64(i * i * 7 + 1) / 16.0);
    const ys = map(xs, (x) => f32(x));
    print(map(xs, (x) => floor(x)));
    print(map(xs, (x) => ceil(x)));
    print(map(xs, (x) => trunc(x)));
    print(map(xs, (x) => round(x)));
    print(map(xs, (x) => abs(x)));
    print(map(ys, (y) => floor(y)));
    print(map(ys, (y) => ceil(y)));
    print(map(ys, (y) => trunc(y)));
    print(map(ys, (y) => round(y)));
    print(map(ys, (y) => abs(y)));
    // scalars, one at a time, and their signs of zero
    print([floor(xs[3]), ceil(xs[3]), trunc(xs[3]), round(xs[3]), abs(xs[1]), round(xs[5]), round(xs[6]), round(xs[11])]);
    print(map(xs, (x) => 1.0 / round(x) < 0.0 ? 1 : 0));
    print(map(xs, (x) => 1.0 / ceil(x) < 0.0 ? 1 : 0));
    print(sum(map(xs, (x) => x == x ? abs(x - floor(x) - 0.5) : 0.0)));
    print(map(iota(n), (i) => round(f64(i) * 0.5 - 4.0) + floor(f64(i) / 3.0)));
    // as values
    print(map([1.5, -1.5, -0.25], round));
    print(map([1.5, -1.5, -0.25], floor));
    print(map([1.5f32, -1.5f32, -0.25f32], abs));
    return 0;
}
";

#[test]
fn roundings_are_instructions() {
    let want = interp_ok(ROUNDINGS, &["300"]);
    let lines: Vec<&str> = want.lines().collect();
    // (the first 20 values of each, checked by hand; `round` as ECMAScript:
    // a half toward +inf, the sign of a zero kept)
    assert!(lines[0].starts_with("[0, -0, 0, -1, 0, -1, 0, -1, 1, -2, 2, -3, 3, -4, 0, -1, 1, -1, 4503599627370495, -4503599627370496, "), "floor: {}", lines[0]);
    assert!(lines[1].starts_with("[0, -0, 1, -0, 1, -0, 1, -0, 2, -1, 3, -2, 4, -3, 1, -0, 1, -1, 4503599627370496, -4503599627370495, "), "ceil: {}", lines[1]);
    assert!(lines[2].starts_with("[0, -0, 0, -0, 0, -0, 0, -0, 1, -1, 2, -2, 3, -3, 0, -0, 1, -1, 4503599627370495, -4503599627370495, "), "trunc: {}", lines[2]);
    assert!(lines[3].starts_with("[0, -0, 0, -0, 1, -0, 0, -0, 2, -1, 3, -2, 4, -3, 1, -1, 1, -1, 4503599627370496, -4503599627370495, "), "round: {}", lines[3]);
    assert!(lines[4].starts_with("[0, 0, 0.25, 0.25, 0.5, 0.5, 0.49999999999999994, 0.49999999999999994, 1.5, 1.5, "), "abs: {}", lines[4]);
    assert_eq!(lines[10], "[-1, -0, -0, -0, 0, -0, 0, -2]");
    assert_eq!(&lines[lines.len() - 3..], ["[2, -1, -0]", "[1, -2, -1]", "[1.5, 1.5, 0.25]"]);
    use isc_codegen::Isa;
    for (isa, name) in [(Isa::Native, "native"), (Isa::Sse, "sse"), (Isa::Avx, "avx"), (Isa::Avx2, "avx2")] {
        let mut o = isc_codegen::Options::default();
        o.isa = isa;
        let bin = compile_with(ROUNDINGS, &format!("roundings_{}", name), o);
        for threads in [1, 4] {
            let r = run_bin_threads(&bin, &["300"], Some(threads));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{} on {} threads: {}", name, threads, r.stderr);
        }
        // the code: no function of the runtime, the instructions on vectors
        let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
        for gone in ["@isc_floor", "@isc_ceil", "@isc_trunc", "@isc_round", "@isc_abs"] {
            assert!(!ll.contains(gone), "{}: {} called", name, gone);
        }
        for made in ["@llvm.floor.v", "@llvm.ceil.v", "@llvm.trunc.v", "@llvm.fabs.v", "@llvm.copysign.v"] {
            assert!(ll.contains(made), "{}: no {} on vectors", name, made);
        }
    }
    match wasi_runner() {
        Some(runner) => {
            let r = run_wasm(&runner, &compile_wasm(ROUNDINGS, "roundings_wasm"), &["300"]);
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "WebAssembly: {}", r.stderr);
        }
        None => eprintln!("no WASI engine (wasmtime): not run as WebAssembly"),
    }
}

/// The reductions of sliding windows by their shared levels (D115): the
/// same bits as one `reduce` per window (the interpreter), for widths that
/// take the cascade with one pass per lane (32, 64, 96, 128), with one pass
/// per element (160, 256), a non-associative operator, and widths that do
/// not (40, 16), with 1 and 4 threads, around the blocks of 1024 windows.
pub const WINDOWS: &str = "action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const a = map(window(x, 64), (w) => sum(w) / 64.0);
    const b = map(window(x, 96), (w) => max(w) - 1.0);
    const c = map(window(x, 40), (w) => sum(w));
    const d = map(window(x, 32), (w) => reduce(w, (p, q) => p * 0.5 + q, 1.0) + 2.0);
    const f = map(window(x, 16), (w) => sum(w));
    const g = map(window(x, 128), (w) => min(w));
    const h = map(window(x, 160), (w) => sum(w) * 0.5);
    const u = map(window(x, 256), (w) => reduce(w, (p, q) => p * 0.5 + q, 1.0));
    writeF64(args[1], [sum(a), sum(b), sum(c), sum(d), sum(f), sum(g), sum(h), sum(u)]);
    return 0;
}
";

#[test]
fn window_reductions() {
    for (i, n) in [256usize, 1087, 1088, 3000, 70_001].iter().enumerate() {
        let x = write_f64s(&format!("win_x{}.f64", n), &random_f64s(2500 + i as u64, *n));
        same_everywhere(WINDOWS, &format!("windows{}", n), &[&x]);
    }
}

/// The scans of integers computed in vector registers (D116): every width
/// (64, 32, 16 and 8 lanes), the four operators, wrapping, lengths around
/// the multiples of the lanes; the same values as the interpreter.
const INT_SCANS: &str = "action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const v = map(x, (f) => i64(f * 1000.0));
    const a = scan(map(v, (i) => i8(i % 100)), (+), 5i8);
    const b = scan(map(v, (i) => u8((i % 200 + 200) % 200 + 50)), max, 0u8);
    const c = scan(map(v, (i) => i16(i % 3000)), min, 32767i16);
    const d = scan(map(v, (i) => i32(i % 7 + 1)), (*), 1i32);
    const e = scan(map(v, (i) => u64(i % 1000 + 1000)), (+), 18446744073709551000u64);
    const f = scan(v, min, 0);
    print([sum(map(a, (t) => i64(t))), sum(map(b, (t) => i64(t))), sum(map(c, (t) => i64(t)))]);
    print([sum(map(d, (t) => i64(t))), i64(sum(map(e, (t) => t % 1000000))), sum(f)]);
    print(a);
    writeF64(args[1], map(d, (t) => f64(t)));
    return 0;
}
";

#[test]
fn integer_scans_in_vectors() {
    for (i, n) in [0usize, 1, 7, 8, 9, 15, 17, 63, 64, 65, 1000, 4097, 70_001].iter().enumerate() {
        let x = write_f64s(&format!("iscan_x{}.f64", n), &random_f64s(2600 + i as u64, *n));
        same_everywhere(INT_SCANS, &format!("int_scans_{}", n), &[&x]);
    }
}

/// `reduce` of a filter on several threads (D120): each chunk owns the
/// blocks of kept elements that start in it; the same bits as the
/// interpreter with any number of threads, for densities from none to all
/// kept elements, a non-associative operator, a map after the filter,
/// negative zeros, and numbers of kept elements around the blocks.
const PARALLEL_STREAMS: &str = "action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const a = sum(filter(x, (v) => v > 0.25));
    const b = sum(filter(x, (v) => v > 100000.0));
    const c = sum(filter(x, (v) => v > -1000000000.0));
    const d = sum(filter(x, (v) => v > 1000000000.0));
    const e = reduce(filter(x, (v) => v < 0.0), (p, q) => p * 0.5 + q, 1.0);
    const f = sum(map(filter(x, (v) => v > 0.0), (v) => v * 3.0));
    const g = max(filter(x, (v) => v < 0.5));
    const h = sum(filter(map(x, (v) => v * 0.0 - 0.0), (v) => v < 0.5));
    writeF64(args[1], [a, b, c, d, e, f, g, h]);
    return 0;
}
";

#[test]
fn parallel_streams() {
    let bin = compile(PARALLEL_STREAMS, "par_streams");
    for (i, n) in [8193usize, 8200, 12_289, 16_385, 70_001, 300_007, 1_000_003].iter().enumerate() {
        let x = write_f64s(&format!("pstream_x{}.f64", n), &random_f64s(2700 + i as u64, *n));
        let o_ref = tmp(&format!("pstream_{}_ref.out", n)).to_string_lossy().to_string();
        interp_ok(PARALLEL_STREAMS, &[&x, &o_ref]);
        for t in [1, 2, 3, 4, 7, 8, 16] {
            let o = tmp(&format!("pstream_{}_{}.out", n, t)).to_string_lossy().to_string();
            let r = run_bin_threads(&bin, &[&x, &o], Some(t));
            assert_eq!(r.code, 0, "n = {} with {} threads: {}", n, t, r.stderr);
            assert_eq!(std::fs::read(&o).unwrap(), std::fs::read(&o_ref).unwrap(), "n = {} with {} threads", n, t);
        }
    }
}

/// A `map` whose function returns an array: an array of arrays, its rows in
/// one block (D126): rows that are literals, `map`s, slices, a variable of
/// the program, a `let` of the function; integers, floats and booleans; no
/// row; the same bits as the interpreter with 1 and 4 threads.
const ROWS: &str = "function outer(a: f64[n], b: f64[m]): f64[n][m] {
    return map(a, (u) => map(b, (v) => u * v));
}

action main(args: string[k]): i32 {
    const x = readF64(args[0]);
    const s = map(iota(7), (i) => x.slice(i, i + 4));
    print(map(s, (r) => sum(r)));
    const y = map(x, (v) => v + 1.0);
    const same = map(iota(3), (i) => y);
    print(map(same, (r) => max(r)));
    const ints = map(iota(7), (i) => [i, i * i, -i]);
    print(ints);
    const flags = map(iota(7), (i) => map(iota(5), (j) => (i + j) % 2 == 0));
    print(flags);
    const t = outer(x, x.slice(0, 3));
    const u = map(t, (r) => {
        const c = sum(r);
        return map(r, (v) => v - c);
    });
    print(sum(map(u, (r) => sum(r))));
    print(map(iota(0), (i) => [f64(i)]));
    const named = map(iota(4), (i) => {
        const r = map(iota(3), (j) => f64(i + j) * 0.25);
        return r;
    });
    print(named);
    writeF64(args[2], map(t, (r) => r[1]));
    return 0;
}
";

#[test]
fn map_builds_arrays_of_arrays() {
    let x = write_f64s("rows_x.f64", &random_f64s(2900, 5000));
    same_everywhere(ROWS, "rows", &[&x, "7"]);
}
