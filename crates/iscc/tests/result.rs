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
    let bin = compile_with(src, &format!("{}_vg", name), opts);
    let out = std::process::Command::new(vg)
        .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
        .arg(&bin)
        .args(args)
        .env("ISC_ALLOC", "glibc")
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
    print(show(try_parse_i64("42"), to_string));
    print(show(try_parse_i64("4x2"), to_string));
    print(show(try_parse_f64("2.5e1"), to_string));
    print(show(try_parse_f64("virgule,5"), to_string));
    print(show(try_read_f64(concat(dir, "/x.f64")), (v) => to_string(sum(v))));
    print(show(try_read_f64(concat(dir, "/odd.f64")), (v) => to_string(sum(v))));
    print(show(try_read_bytes(concat(dir, "/absent")), (v) => to_string(len(v))));
    print(show(try_read_text(concat(dir, "/ok.csv")), (t) => to_string(len(lines(t)))));
    print(show(try_read_text(concat(dir, "/x.f64")), (t) => t));
    match (try_read_csv_f64(concat(dir, "/ok.csv"), 0)) {
        Ok(m) => print(m),
        Err(e) => print(e),
    };
    match (try_read_csv_f64(concat(dir, "/bad.csv"), 0)) {
        Ok(m) => print(m),
        Err(e) => print(e),
    };
    print(done(try_write_text(concat(dir, "/written.txt"), "bonjour\n")));
    print(done(try_write_text(concat(dir, "/no/such/dir.txt"), "x")));
    print(done(try_write_bytes(concat(dir, "/written.bin"), bytes("AB"))));
    print(concat(read_text(concat(dir, "/written.txt")), to_string(len(read_bytes(concat(dir, "/written.bin"))))));
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
        "Ok(42)\nErr(parse_i64: `4x2` is not a valid integer)\nOk(25)\nErr(parse_f64: `virgule,5` is not a valid number)\nOk(3)\n\
         Err(cannot read `{d}/odd.f64`: its size (3 bytes) is not a multiple of 8)\n\
         Err(cannot read `{d}/absent`: No such file or directory (os error 2))\nOk(2)\n\
         Err(read_text: `{d}/x.f64` is not UTF-8 text (invalid from byte 6))\n[[1, 2], [3, 4]]\n\
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
    // a constructor `Ok` and a function `try_read_text` of the program
    let b = "type Answer = Ok | No;\n\nfunction try_read_text(p: string): i64 {\n    return 7;\n}\n\naction main(args: string[k]): i32 {\n    print([Ok, try_read_text(\"x\")] as const);\n    return 0;\n}\n";
    assert_eq!(interp_ok(b, &[]), "(Ok, 7)\n");
    // a function `len` of the program does not break the prelude
    let c = "function len(a: i64, b: i64): i64 {\n    return a + b;\n}\n\naction main(args: string[k]): i32 {\n    match (try_parse_i64(\"5\")) {\n        Ok(v) => print(len(v, 1)),\n        Err(e) => print(e),\n    };\n    return 0;\n}\n";
    assert_eq!(interp_ok(c, &[]), "6\n");
}

#[test]
fn no_copy_of_what_is_read() {
    // the array read is moved into `Ok`, never copied
    let src = "action main(args: string[k]): i32 {\n    match (try_read_f64(args[0])) {\n        Ok(v) => print(sum(v)),\n        Err(e) => print(e),\n    };\n    return 0;\n}\n";
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
