//! Text files and CSV (DECISIONS D135): `readText`, `writeText`, `lines`,
//! `split`, `trim`, `join`, `len` of an array, `parseCsvF64`,
//! `readCsvF64`. The interpreter and the compiled program print the same
//! text and write the same file, the compiled program frees everything
//! (Valgrind, when installed), and both panic with the same messages.

mod common;
use common::*;

fn valgrind_clean(src: &str, name: &str, args: &[&str]) {
    checked_clean(src, name, args);
    let Some(vg) = valgrind_path() else {
        eprintln!("valgrind not run (not installed, or ISC_TEST_NO_VALGRIND): {} skipped", name);
        return;
    };
    let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
    let bin = compile_glibc(src, &format!("{}_vg", name), opts);
    let out = std::process::Command::new(vg)
        .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
        .arg(&bin)
        .args(args)
        .env("ISC_ALLOC", "glibc")
        // 2 threads, as the other suites: under valgrind (one thread at a
        // time) each idle worker of the pool spins before it sleeps
        .env("ISC_THREADS", "2")
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{} under valgrind:\n{}", name, err);
}

const TEXT: &str = r#"function words(s: string): string[k] {
    return filter(map(split(s, " "), trim), (w) => len(w) > 0);
}

action main(args: string[k]): i32 {
    const out = args[0];
    writeText(out, "alpha beta\n  gamma   delta \nepsilon\n");
    const t = readText(out);
    const ls = lines(t);
    print(ls);
    print(map(ls, (l) => len(words(l))));
    print(join(words(join(ls, " ")), "+"));
    print(split("a,b,,c", ","));
    print(lines(""));
    print(lines("one"));
    print(lines("a\r\nb\n\nc"));
    print(concat(concat("[", trim("  \t padded \n")), "]"));
    print(join([], ","));
    print(reduce(map(ls, (l) => len(l)), (+), 0) == len(t) - len(ls));
    const m = readCsvF64(args[1], 1);
    print(m);
    print(map(m, (r) => sum(r)));
    print(map(iota(3), (j) => sum(map(m, (r) => r[j])) / f64(len(m))));
    print(map(split(lines(readText(args[1]))[0], ","), trim));
    print(parseCsvF64("1,2\n3,4", 0));
    print(parseCsvF64("", 0));
    return 0;
}
"#;

#[test]
fn text_and_csv() {
    let csv = tmp("text_data.csv");
    std::fs::write(&csv, "x, y ,z\r\n1, 2.5, -3\r\n4,5,6e1\r\n\r\n7 ,8, 9\r\n").unwrap();
    let csv = csv.to_string_lossy().to_string();
    let o1 = tmp("text_out_interp.txt").to_string_lossy().to_string();
    let o2 = tmp("text_out_native.txt").to_string_lossy().to_string();
    let expected = "[alpha beta,   gamma   delta , epsilon]\n[2, 2, 1]\nalpha+beta+gamma+delta+epsilon\n[a, b, , c]\n[]\n[one]\n[a, b, , c]\n[padded]\n\ntrue\n[[1, 2.5, -3], [4, 5, 60], [7, 8, 9]]\n[0.5, 69, 24]\n[4, 5.166666666666667, 22]\n[x, y, z]\n[[1, 2], [3, 4]]\n[]\n";
    assert_eq!(interp_ok(TEXT, &[&o1, &csv]), expected, "interpreter");
    let bin = compile(TEXT, "text_csv");
    let r = run_bin(&bin, &[&o2, &csv]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, expected, "compiled");
    let written = "alpha beta\n  gamma   delta \nepsilon\n";
    assert_eq!(std::fs::read_to_string(&o1).unwrap(), written);
    assert_eq!(std::fs::read_to_string(&o2).unwrap(), written);
    valgrind_clean(TEXT, "text_csv", &[&o2, &csv]);
}

#[test]
fn text_panics() {
    let bad = tmp("text_bad.txt");
    std::fs::write(&bad, b"ab\xffcd").unwrap();
    let ragged = tmp("text_ragged.csv");
    std::fs::write(&ragged, "1,2\n3\n").unwrap();
    let field = tmp("text_field.csv");
    std::fs::write(&field, "a,b\n1,x\n").unwrap();
    let none = tmp("text_none.txt");
    let _ = std::fs::remove_file(&none);
    let s = |p: &std::path::PathBuf| p.to_string_lossy().to_string();
    let cases: Vec<(String, String, String)> = vec![
        ("print(readText(args[0]))".into(), s(&bad), format!("panic at line 2, column 19: readText: `{}` is not UTF-8 text (invalid from byte 2)", s(&bad))),
        ("print(readCsvF64(args[0], 0))".into(), s(&ragged), "panic at line 2, column 21: csv: line 2 has 1 field, the first row has 2".into()),
        ("print(readCsvF64(args[0], 1))".into(), s(&field), "panic at line 2, column 21: csv: line 2, field 2: `x` is not a number".into()),
        ("print(split(args[0], \"\"))".into(), "abc".into(), "panic at line 2, column 16: split: the separator is empty".into()),
        ("print(readText(args[0]))".into(), s(&none), format!("panic at line 2, column 19: cannot read `{}`: No such file or directory (os error 2)", s(&none))),
    ];
    for (i, (body, arg, want)) in cases.iter().enumerate() {
        let src = format!("action main(args: string[k]): i32 {{\n    {};\n    return 0;\n}}\n", body);
        let m = interp_panic(&src, &[arg]);
        assert_eq!(m.trim(), want, "interpreter, case {}", i);
        let bin = compile(&src, &format!("text_panic_{}", i));
        let r = run_bin(&bin, &[arg]);
        assert_eq!(r.code, 101, "case {}", i);
        assert_eq!(r.stderr.trim(), want, "compiled, case {}", i);
    }
}

#[test]
fn text_errors() {
    let e = iscc::frontend("t.isc", "function f(p: string): string {\n    return readText(p);\n}\n\naction main(args: string[k]): i32 {\n    print(f(args[0]));\n    return 0;\n}\n").unwrap_err();
    assert!(e.contains("readText"), "{}", e);
    let e = iscc::frontend("t.isc", "action main(args: string[k]): i32 {\n    print(readCsvF64(args[0], 1.5));\n    return 0;\n}\n").unwrap_err();
    assert!(e.contains("i64") || e.contains("integer"), "{}", e);
    let e = iscc::frontend("t.isc", "action main(args: string[k]): i32 {\n    print(len(3));\n    return 0;\n}\n").unwrap_err();
    assert!(e.contains("string"), "{}", e);
}
