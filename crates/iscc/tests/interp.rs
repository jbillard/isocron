//! Interpreter semantics (P3).

mod common;
use common::*;

fn sum_fixed(xs: &[f64]) -> f64 {
    fixed_reduce(xs, 0.0, &|a, b| a + b)
}

#[test]
fn sum_matches_fixed_form() {
    for (seed, n) in [(1u64, 0usize), (2, 1), (3, 17), (4, 4096), (5, 4097), (6, 100_000)] {
        let xs = random_f64s(seed, n);
        let inp = write_f64s(&format!("sum_in_{}.f64", n), &xs);
        let outp = tmp(&format!("sum_out_{}.f64", n)).to_string_lossy().to_string();
        let printed = interp_ok(SUM, &[&inp, &outp, "3"]);
        assert!(printed.trim().parse::<u64>().is_ok(), "{}", printed);
        let r = read_f64s(&outp);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].to_bits(), sum_fixed(&xs).to_bits(), "n = {}", n);
        if n > 1000 {
            // the sequential sum rounds differently on this data
            let seq: f64 = xs.iter().sum();
            assert_ne!(seq.to_bits(), r[0].to_bits());
        }
    }
}

#[test]
fn dot_matches_fixed_form() {
    let a = random_f64s(10, 10_000);
    let b = random_f64s(11, 10_000);
    let pa = write_f64s("dot_a.f64", &a);
    let pb = write_f64s("dot_b.f64", &b);
    let out = tmp("dot_out.f64").to_string_lossy().to_string();
    interp_ok(DOT, &[&pa, &pb, &out, "2"]);
    let prods: Vec<f64> = a.iter().zip(&b).map(|(x, y)| x * y).collect();
    assert_eq!(read_f64s(&out)[0].to_bits(), sum_fixed(&prods).to_bits());
    // different sizes: the run-time test of section 4
    let pc = write_f64s("dot_c.f64", &b[..9_999]);
    let m = interp_panic(DOT, &[&pa, &pc, &out, "1"]);
    assert!(m.contains("size mismatch: 10000 != 9999"), "{}", m);
}

#[test]
fn std_dev_matches_fixed_form() {
    let xs = random_f64s(20, 50_000);
    let p = write_f64s("std_in.f64", &xs);
    let out = tmp("std_out.f64").to_string_lossy().to_string();
    interp_ok(STD, &[&p, &out, "1"]);
    let n = xs.len() as f64;
    let m = sum_fixed(&xs) / n;
    let sq: Vec<f64> = xs.iter().map(|x| (x - m) * (x - m)).collect();
    let want = (sum_fixed(&sq) / n).sqrt();
    assert_eq!(read_f64s(&out)[0].to_bits(), want.to_bits());
}

#[test]
fn histogram_counts() {
    let mut r = Rng(30);
    let bytes: Vec<u8> = (0..70_000).map(|_| (r.next() % 251) as u8).collect();
    let p = tmp("hist_in.u8");
    std::fs::write(&p, &bytes).unwrap();
    let out = tmp("hist_out.f64").to_string_lossy().to_string();
    interp_ok(HIST, &[&p.to_string_lossy(), &out, "2"]);
    let mut want = vec![0f64; 256];
    for b in &bytes {
        want[*b as usize] += 1.0;
    }
    assert_eq!(read_f64s(&out), want);
}

const MAIN_PRINT: &str = "action main(args: string[k]): i32 {\n    print(";

fn eval_print(expr: &str, defs: &str) -> String {
    let src = format!("{}\n{}{});\n    return 0;\n}}\n", defs, MAIN_PRINT, expr);
    interp_ok(&src, &[]).trim_end().to_string()
}

#[test]
fn reduce_uses_the_fixed_tree_form() {
    // `-` is not associative: the result exposes the evaluation order.
    let defs = "function f(): i64 {\n    return reduce(iota(40), (a, b) => a - b, 0);\n}\n";
    let xs: Vec<i64> = (0..40).collect();
    let want = fixed_reduce(&xs, 0i64, &|a, b| a - b);
    assert_eq!(eval_print("f()", defs), want.to_string());
    // a non-neutral element is combined V = 32 times, then by the tree
    let defs = "function f(): i64 {\n    return reduce(iota(3), (+), 1);\n}\n";
    assert_eq!(eval_print("f()", defs), (32 + 3).to_string());
    // two levels: 5000 elements = 2 blocks
    let defs = "function f(): i64 {\n    return reduce(iota(5000), (a, b) => 2 * a - b, 7);\n}\n";
    let xs: Vec<i64> = (0..5000).collect();
    let want = fixed_reduce(&xs, 7i64, &|a: i64, b: i64| a.wrapping_mul(2).wrapping_sub(b));
    assert_eq!(eval_print("f()", defs), want.to_string());
}

#[test]
fn integer_semantics() {
    assert_eq!(eval_print("250u8 + 10u8", ""), "4");
    assert_eq!(eval_print("0u32 - 1u32", ""), "4294967295");
    assert_eq!(eval_print("-7 / 2", ""), "-3");
    assert_eq!(eval_print("-7 % 2", ""), "-1");
    assert_eq!(eval_print("7 % -2", ""), "1");
    assert_eq!(eval_print("f(-128i8)", "function f(x: i8): i8 {\n    return x / -1;\n}\n"), "-128");
    assert_eq!(eval_print("f(-128i8)", "function f(x: i8): i8 {\n    return x % -1;\n}\n"), "0");
    assert_eq!(eval_print("f(-9223372036854775808)", "function f(x: i64): i64 {\n    return x / -1;\n}\n"), "-9223372036854775808");
    assert_eq!(eval_print("2 ** 62 * 4", ""), "0");
    assert_eq!(eval_print("3u8 ** 5", ""), "243");
    assert_eq!(eval_print("3u8 ** 6", ""), "217");
    assert_eq!(eval_print("i32(3000000000u32)", ""), "-1294967296");
    assert_eq!(eval_print("u8(-1)", ""), "255");
    assert_eq!(eval_print("i32(1e10)", ""), "2147483647");
    assert_eq!(eval_print("u8(-3.5)", ""), "0");
    assert_eq!(eval_print("i64(-2.9)", ""), "-2");
    assert_eq!(eval_print("f(0.0)", "function f(z: f64): i64 {\n    return i64(z / z);\n}\n"), "0");
}

#[test]
fn float_semantics_and_printing() {
    assert_eq!(eval_print("0.1 + 0.2", ""), "0.30000000000000004");
    assert_eq!(eval_print("f(1.5) ** 2", "function f(x: f64): f64 {\n    return x;\n}\n"), "2.25");
    assert_eq!(eval_print("2.0 ** 0", ""), "1");
    assert_eq!(eval_print("f(0.0)", "function f(z: f64): f64 {\n    return -z / z;\n}\n"), "NaN");
    assert_eq!(eval_print("-0.0", ""), "-0");
    assert_eq!(eval_print("1.0 / 0.0", ""), "inf");
    assert_eq!(eval_print("sqrt(2.0)", ""), "1.4142135623730951");
    assert_eq!(eval_print("5.5 % 2.0", ""), "1.5");
    assert_eq!(eval_print("[1, true, 2.5] as const", ""), "(1, true, 2.5)");
    assert_eq!(eval_print("[1.0, 2.0] * 3.0", ""), "[3, 6]");
    assert_eq!(eval_print("iota(4) == iota(4)", ""), "true");
    assert_eq!(eval_print("iota(4) == iota(5)", ""), "false");
    assert_eq!(eval_print("[1, 2] == [1, 3]", ""), "false");
    assert_eq!(eval_print("[1, 2.0] as const != [1, 2.0] as const", ""), "false");
    assert_eq!(eval_print("mean([1.0, 2.0, 4.5])", ""), "2.5");
    assert_eq!(eval_print("dot(iota(4), iota(4))", ""), "14");
    assert_eq!(eval_print("product([1, 2, 3, 4])", ""), "24");
    assert_eq!(eval_print("map([1u8, 2u8], (v) => v * 200u8)", ""), "[200, 144]");
}

#[test]
fn panics_report_position() {
    let src = "action main(args: string[k]): i32 {\n    const x = iota(3);\n    print(x[3]);\n    return 0;\n}\n";
    let m = interp_panic(src, &[]);
    assert_eq!(m, "panic at line 3, column 12: index out of bounds: the index is 3 but the length is 3");
    let src = "function f(x: i64): i64 {\n    return 10 / x;\n}\n\naction main(args: string[k]): i32 {\n    print(f(parseI64(args[0])));\n    return 0;\n}\n";
    assert!(interp_panic(src, &["0"]).contains("integer division by zero"));
    assert!(interp_panic(src, &["zero"]).contains("`zero` is not a valid integer"));
    let src = "function f(x: i64): i64 {\n    return 2 ** x;\n}\n\naction main(args: string[k]): i32 {\n    print(f(parseI64(args[0])));\n    return 0;\n}\n";
    assert!(interp_panic(src, &["-1"]).contains("negative integer exponent: -1"));
    let src = "function f(x: f64[n], y: f64[m]): f64[n] {\n    return x + y;\n}\n\naction main(args: string[k]): i32 {\n    print(f([1.0], [1.0, 2.0]));\n    return 0;\n}\n";
    assert!(interp_panic(src, &[]).contains("size mismatch: 1 != 2"));
    let src = "action main(args: string[k]): i32 {\n    const [r, ns] = bench(0, () => 1);\n    return 0;\n}\n";
    assert!(interp_panic(src, &[]).contains("at least 1"));
    let src = "action main(args: string[k]): i32 {\n    const x = readF64(args[0]);\n    return 0;\n}\n";
    assert!(interp_panic(src, &["/nonexistent/file"]).contains("cannot read"));
}

#[test]
fn exit_code_and_actions_in_order() {
    let src = "action greet(n: i64): void {\n    print(n * 2);\n}\n\naction main(args: string[k]): i32 {\n    greet(1);\n    print(args[0]);\n    greet(2);\n    return 7;\n}\n";
    match interp(src, &["hello"]) {
        Outcome::Exit(7, out) => assert_eq!(out, "2\nhello\n4\n"),
        _ => panic!(),
    }
}

#[test]
fn updates_and_copies() {
    // `a` is used after the update: the update works on a copy
    let src = "action main(args: string[k]): i32 {\n    const a = [1, 2, 3];\n    const b = a.with(0, 9);\n    print(a);\n    print(b);\n    return 0;\n}\n";
    assert_eq!(interp_ok(src, &[]), "[1, 2, 3]\n[9, 2, 3]\n");
    // fold accumulator updated in place
    let src = "function f(x: i64[n]): i64[4] {\n    return fold(x, zeros(), (c, v) => c.with(v % 4, c[v % 4] + v));\n}\n\naction main(args: string[k]): i32 {\n    print(f(iota(10)));\n    return 0;\n}\n";
    assert_eq!(interp_ok(src, &[]), "[12, 15, 8, 10]\n");
    // a fold whose accumulator is returned unchanged, or replaced
    let src = "function f(x: i64[n]): i64[2] {\n    return fold(x, [0, 0], (c, v) => v > 5 ? [v, v] : c);\n}\n\naction main(args: string[k]): i32 {\n    print(f(iota(10)));\n    return 0;\n}\n";
    assert_eq!(interp_ok(src, &[]), "[9, 9]\n");
}

#[test]
fn generic_literals_are_range_checked() {
    // the type of `300` is known only once `g` is instantiated with `u8`
    let src = "function g<T extends Num>(x: T[n]): T {\n    return x[0] + 300;\n}\n\naction main(args: string[k]): i32 {\n    print(g([1u8]));\n    return 0;\n}\n";
    match iscc::frontend("t.isc", src) {
        Ok(_) => panic!("accepted"),
        Err(e) => assert!(e.contains("the literal `300` is out of range for `u8`"), "{}", e),
    }
    let ok = "function g<T extends Num>(x: T[n]): T {\n    return x[0] + 300;\n}\n\naction main(args: string[k]): i32 {\n    print(g([1i64]));\n    return 0;\n}\n";
    assert_eq!(interp_ok(ok, &[]), "301\n");
}
