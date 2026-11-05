//! The module `sys` of the standard library (SPEC section 18, D322 to
//! D324, D330): shipped inside `iscc`, imported by its bare name, over the
//! primitives `sys*` that only the standard modules call; the environment,
//! folders and files, the errors as `IoError` values; the same outputs in
//! the interpreter and in the compiled program.

mod common;
use common::*;

const SRC: &str = include_str!("data/std_sys.isc");

/// Every function of `sys` in a new folder, compiled (1 and 4 threads) and
/// interpreted: the same outputs, the folder named BASE in them.
#[test]
fn sys_folders_files_and_environment() {
    let bin = compile(SRC, "std_sys");
    let run = |dir: &std::path::Path, interp: bool| -> String {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let arg = dir.to_str().unwrap().to_string();
        let out = if interp {
            std::env::set_var("ISC_SYS_TEST", "oui");
            interp_ok(SRC, &[&arg])
        } else {
            let o = std::process::Command::new(&bin).arg(&arg).env("ISC_SYS_TEST", "oui").output().unwrap();
            assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).to_string()
        };
        let _ = std::fs::remove_dir_all(dir);
        out.replace(&arg, "BASE")
    };
    let want = "ok\nok\nalready exists: BASE/a\nok\nok\nok\nun deux\ntwo.bin is not text\n[b, one.txt]\n[true, true, true, false]\n7\nnot empty: BASE/a/b\nok\nnot found: BASE/a/one.txt\nok\nfalse\noui\n1\n";
    assert_eq!(run(&tmp("std_sys_interp"), true), want, "interpreter");
    assert_eq!(run(&tmp("std_sys_compiled"), false), want, "compiled");
}

/// A primitive of the system is refused in a program, with the module that
/// offers its function; a file `sys.isc` of `$ISC_PATH` does not hide the
/// standard module, and one next to the program is imported by its path.
#[test]
fn primitives_only_in_the_standard_modules() {
    let bad = "action main(args: string[k]): i32 {\n    const [v, f] = sysGetenv(\"HOME\");\n    print(f);\n    return 0;\n}\n";
    let e = match iscc::frontend("bad.isc", bad) {
        Ok(_) => panic!("a primitive of the system accepted in a program"),
        Err(e) => format!("{}", e),
    };
    assert!(e.contains("is a primitive of the standard library") && e.contains("import { env } from \"sys\""), "{}", e);
    let dir = tmp("std_sys_shadow");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("sys.isc"), "export action env(name: string): Option<string> {\n    return Some(\"shadow\");\n}\n").unwrap();
    let prog = dir.join("p.isc");
    std::fs::write(&prog, "import { env } from \"sys\";\nimport * as mine from \"./sys\";\n\naction main(args: string[k]): i32 {\n    print(env(\"ISC_NO_SUCH_VARIABLE\") ?? \"absent\");\n    print(mine.env(\"x\") ?? \"absent\");\n    return 0;\n}\n").unwrap();
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_iscc")).arg("run").arg(&prog).env("ISC_PATH", &dir).output().unwrap();
    assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), "absent\nshadow\n".to_string()), "{}", String::from_utf8_lossy(&o.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}

/// D332: `sys` under WASI (wasi-libc, the folders the host opens): the same
/// outputs as compiled for Linux, the environment of the engine without the
/// variable of the test.
#[test]
fn sys_under_wasi() {
    let Some(runner) = wasi_runner() else {
        eprintln!("no WASI engine: `sys` under WASI not run");
        return;
    };
    let wasm = compile_wasm(SRC, "std_sys_wasi");
    let dir = tmp("std_sys_wasi_dir");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let arg = dir.to_str().unwrap().to_string();
    let r = run_wasm(&runner, &wasm, &[&arg]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let want = "ok\nok\nalready exists: BASE/a\nok\nok\nok\nun deux\ntwo.bin is not text\n[b, one.txt]\n[true, true, true, false]\n7\nnot empty: BASE/a/b\nok\nnot found: BASE/a/one.txt\nok\nfalse\nabsent\n0\n";
    assert_eq!(r.stdout.replace(&arg, "BASE"), want);
}

/// D333: the module `path`, pure functions on the text of paths: the last
/// part, the folder, the extension, the normal form, roots and joins, on
/// paths with `/` at their ends, doubled, `.` and `..`, a drive; the same
/// in the interpreter and compiled.
#[test]
fn paths_as_text() {
    let src = include_str!("data/std_path.isc");
    let want = "[c.txt, b, c.txt, /, , .profile, b.tar.gz, b, C:\\x, c, y, a, .., ., c]\n[/a/b, a, ., /, ., ., a, //a, ., a/./b/.., ../x/../.., /.., a, ., a//b/.]\n[.txt, , .txt, , , , .gz, , , , , , , , ]\n[/a/b/c.txt, a/b, c.txt, /, ., .profile, a/b.tar.gz, /a/b, C:\\x, a/c, ../../y, /a, ., ., a/b/c]\n[true, false, false, true, false, false, false, true, true, false, false, true, false, false, false]\n[a/b, a/b, /b, b, C:/b]\n";
    assert_eq!(interp_ok(src, &[]), want, "interpreter");
    let bin = compile(src, "std_path");
    let r = run_bin(&bin, &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want), "compiled: {}", r.stderr);
}

/// D334: the module `time`: instants in RFC 3339 (before 1970, a leap day,
/// fractions), dates and times at an offset, the days of dates and their
/// weekdays (checked against the `datetime` of Python), months and days
/// added, the leap years, a pattern, dates read (offsets, a fraction of 12
/// digits, a day, an hour, a text that are not), the clock of the system;
/// the same in the interpreter, compiled and under WASI.
#[test]
fn time_dates_and_instants() {
    let src = include_str!("data/std_time.isc");
    let want = include_str!("data/std_time.out");
    assert_eq!(interp_ok(src, &[]), want, "interpreter");
    let bin = compile(src, "std_time");
    let r = run_bin(&bin, &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want), "compiled: {}", r.stderr);
    if let Some(runner) = wasi_runner() {
        let wasm = compile_wasm(src, "std_time_wasi");
        let r = run_wasm(&runner, &wasm, &[]);
        assert_eq!((r.code, r.stdout.as_str()), (0, want), "WASI: {}", r.stderr);
    }
}
