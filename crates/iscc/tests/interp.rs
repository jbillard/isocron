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

/// The program as `iscc run` runs it (D616): the functions of the prelude
/// that have a native form in the interpreter called in that form.
fn run_native(src: &str, args: &[&str]) -> Outcome {
    let p = iscc::frontend_for_run("test.isc", src).unwrap_or_else(|e| panic!("{}", e));
    let mut out = Vec::new();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    match isc_interp::run(&p, &args, &mut out) {
        Ok(c) => Outcome::Exit(c, String::from_utf8(out).unwrap()),
        Err(e) => Outcome::Panic(e.to_string(), String::from_utf8(out).unwrap()),
    }
}

/// The texts on which the native forms could differ from the prelude's:
/// numbers and what is none, the spaces of Unicode, patterns found at the
/// ends, never, empty, of several bytes a character.
const NATIVE_FORMS: &str = "function shown(s: string): string {
    return `<${s}>`;
}

action main(args: string[k]): i32 {
    const numbers = [\"0\", \"-0\", \"+5\", \"5\", \" 5\", \"5 \", \"\", \"-\", \"+\", \"--1\", \"0x10\", \"1e5\", \"1E-5\", \"1.5\", \".5\", \"5.\", \"inf\", \"-inf\",
        \"Infinity\", \"NaN\", \"nan\", \"1_000\", \"9223372036854775807\", \"9223372036854775808\", \"-9223372036854775808\", \"-9223372036854775809\",
        \"00012\", \"+0012\", \"1e309\", \"4.9e-324\", \"2.5e-324\", \"0.1\", \"123456789012345678901234567890\", \"12a\", \"a12\", \"١٢\", \"1e\", \"1e+\",
        \"1.7976931348623157e308\", \"9007199254740993\", \"0.30000000000000004\", \"-1.5e-7\", \"1,5\", \"1 000\", \"\\t7\", \"7\\n\"];
    print(map(numbers, (t) => match (tryParseI64(t)) {
        Ok(v) => toString(v),
        Err(e) => e,
    }));
    print(map(numbers, (t) => match (tryParseF64(t)) {
        Ok(v) => toString(v) + (1.0 / v < 0.0 ? \"-\" : \"+\"),
        Err(e) => e,
    }));
    print([parseI64(\"-42\") + 1, parseI64(\"+7\")]);
    print(parseF64(\"2.5\") * 2.0);
    const spaced = [\"\", \" \", \"a\", \" a\", \"a \", \"  a b  \", \"\\ta\\n\", \"\\r\\na\u{B}\u{C}\", \"\u{A0}a\u{A0}\", \"\u{1680}a\u{2000}\u{2001}\u{2002}\u{200A}\",
        \"\u{2028}a\u{2029}\", \"\u{202F}a\u{205F}\", \"\u{3000}a\u{3000}\", \"\u{FEFF}a\u{FEFF}\", \"\u{85}a\u{85}\", \"\u{200B}a\u{200B}\", \"é \", \" é\", \"   \",
        \"\u{180E}a\", \"a.\"];
    print(map(spaced, (t) => shown(t.trim()) + shown(t.trimStart()) + shown(t.trimEnd())));
    const text = \"un été à l'été, et après l'été : thé\";
    const patterns = [\"\", \"u\", \"un\", \"é\", \"été\", \"thé\", \"hé\", \"x\", \"été,\", \"l'été :\", \"été à l'été, et après l'été : thé\", \"un été à l'été, et après l'été : thé!\", \" \", \"  \", \"é \"];
    print(map(patterns, (q) => text.indexOf(q)));
    print(map(patterns, (q) => text.includes(q) ? 1 : 0));
    print([\"\".indexOf(\"\"), \"\".indexOf(\"a\"), \"aaa\".indexOf(\"aa\"), \"abcabc\".indexOf(\"cab\")]);
    print([args[0].trim().length, args[0].indexOf(args[1]), args[0].includes(args[1]) ? 1 : 0]);
    return 0;
}
";

/// D616: under `iscc run` the readings of numbers, `trim`, `indexOf` and
/// `includes` are native functions of the interpreter, not the functions of
/// the prelude run node by node (12 to 90 times slower): the same values as
/// the prelude's, and as the compiled program.
#[test]
fn native_forms_of_the_prelude() {
    let args = ["  le thé  ", "thé"];
    let want = interp_ok(NATIVE_FORMS, &args);
    assert!(want.contains("[-41, 7]\n5\n"), "{}", want);
    match run_native(NATIVE_FORMS, &args) {
        Outcome::Exit(0, out) => assert_eq!(out, want, "the native forms and the prelude's"),
        Outcome::Exit(c, out) => panic!("exit {} ({})", c, out),
        Outcome::Panic(m, _) => panic!("unexpected panic: {}", m),
    }
    let bin = compile(NATIVE_FORMS, "native_forms");
    let r = run_bin(&bin, &args);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "compiled: {}", r.stderr);
    // the native forms are called: the program run names them
    let p = iscc::frontend_for_run("test.isc", NATIVE_FORMS).unwrap();
    let named: Vec<&str> = p.fns.iter().filter_map(|f| f.name.strip_prefix(isc_interp::NATIVE)).collect();
    for name in ["$parseF64Value", "$parseI64Value", "trim", "trimStart", "trimEnd", "indexOf", "includes"] {
        assert!(named.contains(&name), "{} is not in its native form: {:?}", name, named);
    }
    assert!(iscc::frontend("test.isc", NATIVE_FORMS).unwrap().fns.iter().all(|f| !f.name.starts_with(isc_interp::NATIVE)));
}

/// A panic of a reading is the same message at the same place; a function
/// of a program that has the name of one of the prelude is the program's;
/// a requirement that holds for the program as compiled is not refused
/// because a function is called in the program as run.
#[test]
fn native_forms_keep_the_program() {
    let bad = "action main(args: string[k]): i32 {\n    print(parseI64(args[0]));\n    print(parseF64(args[1]));\n    return 0;\n}\n";
    for args in [["12x", "1.5"], ["12", "1.5x"]] {
        let (a, b) = (interp_panic(bad, &args), match run_native(bad, &args) {
            Outcome::Panic(m, _) => m,
            Outcome::Exit(c, out) => panic!("expected a panic, got exit {} ({})", c, out),
        });
        assert_eq!(a, b);
        assert!(a.contains("is not a valid"), "{}", a);
    }
    let own = "function trim(s: string): string {\n    return \"mine:\" + s;\n}\n\nfunction indexOf(s: string, t: string): i64 {\n    return 0 - s.length - t.length;\n}\n\naction main(args: string[k]): i32 {\n    print(trim(\" a \"));\n    print(indexOf(\"abc\", \"c\"));\n    print(\" b \".trimEnd() + \"|\");\n    return 0;\n}\n";
    let want = interp_ok(own, &[]);
    assert_eq!(want, "mine: a \n-4\n b|\n");
    assert!(matches!(run_native(own, &[]), Outcome::Exit(0, out) if out == want));
    let required = "@requireNoAlloc\nfunction place(s: string, t: string): i64 {\n    return s.trim().indexOf(t) + (s.includes(t) ? 1 : 0);\n}\n\naction main(args: string[k]): i32 {\n    print(place(args[0], args[1]));\n    return 0;\n}\n";
    let want = interp_ok(required, &["  le thé ", "thé"]);
    assert_eq!(want, "4\n");
    assert!(matches!(run_native(required, &["  le thé ", "thé"]), Outcome::Exit(0, out) if out == want));
}

/// D626: `lastIndexOf`, `replace` and `replaceAll` of texts,
/// `Number` of a text and the readers of CSV written in Isocron (the
/// prelude), with a native form each under `iscc run`: the function of the
/// runtime each replaced. On generated inputs (texts over small alphabets
/// and of characters of several bytes, needles cut at their ends or not or
/// empty, every `$` pattern and malformed ones, numbers of every form and
/// what is none, between the spaces of both standards; texts of CSV with
/// blank lines, a header, CRLF, a row of another length, fields that are no
/// number): the native forms (the runtime's code), the prelude's run node by
/// node, the compiled program (with the allocator checked too), and the
/// program in WebAssembly, the same outputs.
#[test]
fn native_forms_of_the_moved_texts() {
    let src = include_str!("data/strings_moved.isc");
    let file = tmp("strings_moved.csv").to_string_lossy().to_string();
    let bin = compile(src, "strings_moved");
    let wasm = wasi_runner().map(|r| (r, compile_wasm(src, "strings_moved_wasm")));
    for (mode, n, few) in [("last", "3000", "300"), ("replace", "3000", "300"), ("number", "2000", "200"), ("csv", "600", "120")] {
        let native = match run_native(src, &[mode, n, &file]) {
            Outcome::Exit(0, out) => out,
            Outcome::Exit(c, out) => panic!("{}: exit {} ({})", mode, c, out),
            Outcome::Panic(m, _) => panic!("{}: unexpected panic: {}", mode, m),
        };
        let r = run_bin(&bin, &[mode, n, &file]);
        assert!(r.code == 0 && r.stdout == native, "{}: the compiled program and the native forms differ: {}", mode, r.stderr);
        checked_clean_bin(&bin, &[mode, few, &file], None);
        // (node by node: fewer inputs)
        let short = match run_native(src, &[mode, few, &file]) {
            Outcome::Exit(0, out) => out,
            _ => panic!("{}", mode),
        };
        assert!(interp_ok(src, &[mode, few, &file]) == short, "{}: the prelude's run node by node and the native forms differ", mode);
        if let Some((runner, w)) = &wasm {
            let r = run_wasm(runner, w, &[mode, n, &file]);
            assert!(r.code == 0 && r.stdout == native, "{}: WebAssembly and the native forms differ: {}", mode, r.stderr);
        }
    }
    let p = iscc::frontend_for_run("test.isc", src).unwrap();
    let named: Vec<&str> = p.fns.iter().filter_map(|f| f.name.strip_prefix(isc_interp::NATIVE)).collect();
    for name in ["lastIndexOf", "$jsNumber", "$replaceOne", "$replaceEvery", "$csvParse", "$csvOrError"] {
        assert!(named.contains(&name), "{} is not in its native form: {:?}", name, named);
    }
    let ll = std::fs::read_to_string(bin.with_extension("ll")).unwrap();
    // (`isc_js_number` stays for `parseFloat` and `parseInt`: declared;
    // `isc_replace_all` for `replaceAll` of 3 to 16 bytes)
    for f in ["@isc_last_index_of(", "call double @isc_js_number(", "@isc_parse_csv_f64(", "@isc_read_csv_f64(", "@isc_try_read_csv_f64("] {
        assert!(!ll.contains(f), "{} still called", f);
    }
    // a function that must not allocate may call `lastIndexOf` and
    // `Number` of a text, as it could call the runtime's
    let required = "@requireNoAlloc\nfunction place(s: string, t: string): f64 {\n    return f64(s.lastIndexOf(t)) + Number(t);\n}\n\naction main(args: string[k]): i32 {\n    print(place(args[0], args[1]));\n    return 0;\n}\n";
    let want = interp_ok(required, &["a 12 b 12", " 12"]);
    assert_eq!(want, "18\n");
    assert!(matches!(run_native(required, &["a 12 b 12", " 12"]), Outcome::Exit(0, out) if out == want));
}

/// D626: the panics of `parseCsvF64` and `readCsvF64` written in
/// Isocron, at the place of their call, with the messages of the runtime's
/// (a field that is no number, its text trimmed; a row of another length,
/// one field or more; a file that cannot be read): compiled, run node by
/// node and in the native forms.
#[test]
fn panics_of_the_readers_of_csv() {
    let file = tmp("csv_panics.csv").to_string_lossy().to_string();
    let missing = tmp("csv_panics_missing.csv").to_string_lossy().to_string();
    let parse = "action main(args: string[k]): i32 {\n    const t = args[0].replaceAll(\";\", \"\\n\");\n    print(t.length);\n    print(parseCsvF64(t, parseI64(args[1])));\n    return 0;\n}\n";
    let read = "action main(args: string[k]): i32 {\n    writeText(args[2], args[0].replaceAll(\";\", \"\\r\\n\"));\n    print(\n        readCsvF64(args[3], parseI64(args[1])));\n    return 0;\n}\n";
    let cases: [(&str, &str, &str, &str); 7] = [
        (parse, "1,2;3, x ;4,5", "0", "panic at line 4, column 22: csv: line 2, field 2: `x` is not a number"),
        (parse, "1,2;3;4,5", "0", "panic at line 4, column 22: csv: line 2 has 1 field, the first row has 2"),
        (parse, "a,b;1;2,3,4", "1", "panic at line 4, column 22: csv: line 3 has 3 fields, the first row has 1"),
        (parse, "1;2,\u{3000}3\u{a0}", "0", "panic at line 4, column 22: csv: line 2 has 2 fields, the first row has 1"),
        (parse, "1,\u{feff}2", "0", "panic at line 4, column 22: csv: line 1, field 2: `\u{feff}2` is not a number"),
        (read, "1,2;;3,4,5", "0", "panic at line 4, column 19: csv: line 3 has 3 fields, the first row has 2"),
        (read, "1,2", "0", "panic at line 4, column 19: cannot read"),
    ];
    for (i, (src, text, skip, want)) in cases.iter().enumerate() {
        let target = if i == cases.len() - 1 { missing.as_str() } else { file.as_str() };
        let args = [*text, *skip, file.as_str(), target];
        let node = interp_panic(src, &args);
        assert!(node.starts_with(want), "case {}: {}", i, node);
        match run_native(src, &args) {
            Outcome::Panic(m, _) => assert_eq!(m, node, "case {}: the native form", i),
            Outcome::Exit(c, out) => panic!("case {}: expected a panic, got exit {} ({})", i, c, out),
        }
        let bin = compile(src, &format!("csv_panic_{}", i));
        let r = run_bin(&bin, &args);
        assert_eq!(r.code, 101, "case {}: {}", i, r.stderr);
        assert!(r.stderr.contains(want), "case {}: {}", i, r.stderr);
    }
}
