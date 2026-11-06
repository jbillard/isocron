//! `Result` and the fallible actions (DECISIONS D138): `type Result<T, E> =
//! Ok(T) | Err(E)` in the prelude, the `try_` actions built on the
//! `_or_error` forms, the program's own `Result` taking precedence, and the
//! components of a new tuple taken without a copy (`Take`).

mod common;
use common::*;

fn valgrind_clean(src: &str, name: &str, args: &[&str]) {
    let vg = std::path::PathBuf::from("/usr/bin/valgrind");
    if !vg.is_file() {
        eprintln!("valgrind not installed: {} skipped", name);
        return;
    }
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

const TRY: &str = r#"function show<T>(r: Result<T, string>, f: (T) => string): string {
    return match (r) {
        Ok(v) => concat(concat("Ok(", f(v)), ")"),
        Err(e) => concat(concat("Err(", e), ")"),
    };
}

function done(r: Result<void, string>): string {
    return match (r) {
        Ok(u) => "Ok",
        Err(e) => concat(concat("Err(", e), ")"),
    };
}

action main(args: string[k]): i32 {
    const dir = args[0];
    print(show(tryParseI64("42"), toString));
    print(show(tryParseI64("4x2"), toString));
    print(show(tryParseF64("2.5e1"), toString));
    print(show(tryParseF64("virgule,5"), toString));
    print(show(tryReadF64(concat(dir, "/x.f64")), (v) => toString(sum(v))));
    print(show(tryReadF64(concat(dir, "/odd.f64")), (v) => toString(sum(v))));
    print(show(tryReadBytes(concat(dir, "/absent")), (v) => toString(len(v))));
    print(show(tryReadText(concat(dir, "/ok.csv")), (t) => toString(len(lines(t)))));
    print(show(tryReadText(concat(dir, "/x.f64")), (t) => t));
    match (tryReadCsvF64(concat(dir, "/ok.csv"), 0)) {
        Ok(m) => print(m),
        Err(e) => print(e),
    };
    match (tryReadCsvF64(concat(dir, "/bad.csv"), 0)) {
        Ok(m) => print(m),
        Err(e) => print(e),
    };
    print(done(tryWriteText(concat(dir, "/written.txt"), "bonjour\n")));
    print(done(tryWriteText(concat(dir, "/no/such/dir.txt"), "x")));
    print(done(tryWriteBytes(concat(dir, "/written.bin"), bytes("AB"))));
    print(concat(readText(concat(dir, "/written.txt")), toString(len(readBytes(concat(dir, "/written.bin"))))));
    return 0;
}
"#;

#[test]
fn fallible_actions() {
    let dir = tmp("result_dir");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ok.csv"), "1,2\n3,4\n").unwrap();
    std::fs::write(dir.join("bad.csv"), "1,2\n3\n").unwrap();
    let x: Vec<u8> = [1.5f64, 2.5, -1.0].iter().flat_map(|v| v.to_le_bytes()).collect();
    std::fs::write(dir.join("x.f64"), x).unwrap();
    std::fs::write(dir.join("odd.f64"), "abc").unwrap();
    let d = dir.to_string_lossy().to_string();
    let expected = format!(
        "Ok(42)\nErr(parseI64: `4x2` is not a valid integer)\nOk(25)\nErr(parseF64: `virgule,5` is not a valid number)\nOk(3)\n\
         Err(cannot read `{d}/odd.f64`: its size (3 bytes) is not a multiple of 8)\n\
         Err(cannot read `{d}/absent`: No such file or directory (os error 2))\nOk(2)\n\
         Err(readText: `{d}/x.f64` is not UTF-8 text (invalid from byte 6))\n[[1, 2], [3, 4]]\n\
         csv: line 2 has 1 field, the first row has 2\nOk\n\
         Err(cannot write `{d}/no/such/dir.txt`: No such file or directory (os error 2))\nOk\nbonjour\n2\n"
    );
    assert_eq!(interp_ok(TRY, &[&d]), expected, "interpreter");
    let bin = compile(TRY, "result_try");
    let r = run_bin(&bin, &[&d]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, expected, "compiled");
    valgrind_clean(TRY, "result_try", &[&d]);
}

#[test]
fn the_program_result_takes_precedence() {
    // a type `Result` of the program: the prelude's and its `try_` actions
    // are left out
    let a = "type Result = Ok(i64) | Err;\n\naction main(args: string[k]): i32 {\n    print(Ok(3));\n    return 0;\n}\n";
    assert_eq!(interp_ok(a, &[]), "Ok(3)\n");
    // a constructor `Ok` and a function `tryReadText` of the program
    let b = "type Answer = Ok | No;\n\nfunction tryReadText(p: string): i64 {\n    return 7;\n}\n\naction main(args: string[k]): i32 {\n    print([Ok, tryReadText(\"x\")] as const);\n    return 0;\n}\n";
    assert_eq!(interp_ok(b, &[]), "(Ok, 7)\n");
    // a function `len` of the program does not break the prelude
    let c = "function len(a: i64, b: i64): i64 {\n    return a + b;\n}\n\naction main(args: string[k]): i32 {\n    match (tryParseI64(\"5\")) {\n        Ok(v) => print(len(v, 1)),\n        Err(e) => print(e),\n    };\n    return 0;\n}\n";
    assert_eq!(interp_ok(c, &[]), "6\n");
}

#[test]
fn no_copy_of_what_is_read() {
    // the array read is moved into `Ok`, never copied
    let src = "action main(args: string[k]): i32 {\n    match (tryReadF64(args[0])) {\n        Ok(v) => print(sum(v)),\n        Err(e) => print(e),\n    };\n    return 0;\n}\n";
    let p = iscc::frontend("t.isc", src).unwrap();
    assert!(p.copy_reasons.is_empty(), "{:?}", p.copy_reasons);
    // the parts of a new tuple, destructured: moved, not copied
    let src = "function halves(x: f64[n]): [f64[n], f64[n]] {\n    return [x / 2.0, x * 2.0] as const;\n}\n\nfunction pick(b: boolean, x: f64[n]): f64[n] {\n    const [h, d] = halves(x);\n    if (b) {\n        return h;\n    } else {\n        return d;\n    }\n}\n\naction main(args: string[k]): i32 {\n    print(pick(true, [1.0, 2.0]));\n    print(pick(false, [1.0, 2.0]));\n    return 0;\n}\n";
    let p = iscc::frontend("t.isc", src).unwrap();
    assert!(p.copy_reasons.is_empty(), "{:?}", p.copy_reasons);
    assert_eq!(interp_ok(src, &[]), "[0.5, 1]\n[2, 4]\n");
    let bin = compile(src, "result_take");
    assert_eq!(run_bin(&bin, &[]).stdout, "[0.5, 1]\n[2, 4]\n");
    valgrind_clean(src, "result_take", &[]);
}

/// D367: the payload of a `match` on a temporary is taken out of it, not
/// copied then freed: `?? d`, `Ok(v) => v` with an error that owns memory
/// too (taken and freed in that branch), a match of three arms, a payload
/// that is a tuple, an arm that reads the temporary and frees it, the
/// threads of a `map`; the text of the interpreter with 1 and 4 threads,
/// Valgrind clean, the takes and the frees in the IR.
#[test]
fn match_payloads_taken() {
    let src = include_str!("data/match_payloads_taken.isc");
    let bin = compile(src, "match_payloads_taken");
    for a in ["0", "1", "2"] {
        let want = interp_ok(src, &[a]);
        for t in [1, 4] {
            let r = run_bin_threads(&bin, &[a], Some(t));
            assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "{} on {} threads: {}", a, t, r.stderr);
        }
    }
    valgrind_clean(src, "match_payloads_taken", &["1"]);
    let p = iscc::frontend("t.isc", src).unwrap();
    let ir = isc_ir::print::program(&p);
    assert!(ir.matches("take 1").count() >= 8, "the payloads not taken:\n{}", ir);
    assert!(ir.matches("free match").count() >= 8, "the other arms do not free the temporary:\n{}", ir);
}
