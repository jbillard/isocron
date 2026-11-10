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

/// D351: `path.normalize` in one pass, its text grown in place: the same
/// results as the algorithm of 0.3.1 on 7812 paths, interpreted and
/// compiled (1 and 4 threads).
#[test]
fn normalize_as_before() {
    let src = include_str!("data/std_path_normalize.isc");
    let want = "7812 paths, 0 different\nnone\n[a/c/d/e.txt, /x, ../../a, .., ., /, /, ., .., ../c]\n";
    assert_eq!(interp_ok(src, &[]), want, "interpreter");
    let bin = compile(src, "std_path_normalize");
    for t in [1, 4] {
        let r = run_bin_threads(&bin, &[], Some(t));
        assert_eq!((r.code, r.stdout.as_str()), (0, want), "compiled, {} threads: {}", t, r.stderr);
    }
}

/// D360: `time.now()` and `clock()` read by the `clock_gettime` of the vDSO
/// (no system call: `strace` sees none), by the system call without a vDSO
/// (Valgrind gives none): the second of the system both ways, never back in
/// a million readings.
#[test]
fn clocks_read_through_the_vdso() {
    let src = include_str!("data/std_clock.isc");
    let second = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let check = |out: &str, before: i64, how: &str| {
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!((lines.len(), lines[0], lines[2]), (3, "0", "true"), "{}: {}", how, out);
        let s: i64 = lines[1].parse().unwrap();
        assert!(s >= before && s <= second(), "{}: second {} not that of the system ({})", how, s, before);
    };
    let bin = compile(src, "std_clock");
    let before = second();
    let r = run_bin(&bin, &["1000000"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    check(&r.stdout, before, "compiled");
    let strace = std::path::Path::new("/usr/bin/strace");
    if strace.is_file() {
        let log = bin.with_extension("strace");
        let out = std::process::Command::new(strace).args(["-f", "-e", "trace=clock_gettime", "-o"]).arg(&log).arg(&bin).arg("1000").output().unwrap();
        assert_eq!(out.status.code(), Some(0));
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(!calls.contains("clock_gettime("), "the clocks read by system calls:\n{}", calls);
    }
    if let Some(vg) = valgrind_path() {
        let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
        let bin = compile_glibc(src, "std_clock_vg", opts);
        let before = second();
        let out = std::process::Command::new(vg).args(["--error-exitcode=9", "-q"]).arg(&bin).arg("1000").env("ISC_ALLOC", "glibc").output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
        check(&String::from_utf8_lossy(&out.stdout), before, "without a vDSO (Valgrind)");
    }
}

/// D370: `env` compares the name with each variable where it is: a name
/// that is the start of another, a name longer than the variable that
/// starts like it, an empty value, a value that is not UTF-8, names that
/// name none; compiled and interpreted, the same text.
#[test]
fn environment_names_compared_where_they_are() {
    use std::os::unix::ffi::OsStrExt;
    let src = include_str!("data/std_env_names.isc");
    let vars: [(&str, &[u8]); 4] = [("ISC_ENV_AB", b"two"), ("ISC_ENV_ABC", b"three"), ("ISC_ENV_EMPTY", b""), ("ISC_ENV_BAD", b"x\xffy")];
    let want = "[absent, two, three, absent, , x\u{fffd}y, absent, absent, absent, absent]\n[0, 3, 5, 0, 0, 5, 0, 0, 0, 0]\n";
    let bin = compile(src, "std_env_names");
    let mut c = std::process::Command::new(&bin);
    for (k, v) in vars {
        c.env(k, std::ffi::OsStr::from_bytes(v));
    }
    let o = c.output().unwrap();
    assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want.to_string()), "compiled: {}", String::from_utf8_lossy(&o.stderr));
    let file = tmp("std_env_names.isc");
    std::fs::write(&file, src).unwrap();
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_iscc"));
    c.arg("run").arg(&file);
    for (k, v) in vars {
        c.env(k, std::ffi::OsStr::from_bytes(v));
    }
    let o = c.output().unwrap();
    assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want.to_string()), "interpreted: {}", String::from_utf8_lossy(&o.stderr));
}

/// D400: `envAll` with its entries measured by aligned vectors and checked
/// by `from_utf8`: names and values of every length from 0 to 130 (their
/// ends at every place of a vector), a value that is not UTF-8 (its bytes
/// replaced), in the order of the bytes of the names; the environment
/// cleared before, so that the expected text is known; compiled and
/// interpreted, the same text.
#[test]
fn all_the_environment_by_vectors() {
    use std::os::unix::ffi::OsStrExt;
    let src = "import * as sys from \"sys\";

action main(args: string[k]): i32 {
    print(join(map(sys.envAll(), (e) => {
        const [k, v] = e;
        return `${k}=${v.length}:${v}`;
    }), \"\\n\"));
    return 0;
}
";
    let mut vars: Vec<(String, Vec<u8>)> = (0..131usize).map(|n| (format!("V{:03}{}", n, "n".repeat(n % 37)), (0..n).map(|i| b'a' + (i % 26) as u8).collect())).collect();
    vars.push(("V_BAD".to_string(), b"\xffab\xfe".to_vec()));
    // names whose order differs from that of their entries `name=value`
    // (a byte below `=` after a name, D423)
    for (k, v) in [("ISC_SORT", "a"), ("ISC_SORT1", "b"), ("ISC_SORT(x86)", "c"), ("ISC_SORT-", "d")] {
        vars.push((k.to_string(), v.as_bytes().to_vec()));
    }
    let mut want: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.clone(), String::from_utf8_lossy(v).into_owned())).collect();
    want.sort();
    let want: String = want.iter().map(|(k, v)| format!("{}={}:{}\n", k, v.len(), v)).collect();
    let bin = compile(src, "std_env_all");
    let mut c = std::process::Command::new(&bin);
    c.env_clear();
    for (k, v) in &vars {
        c.env(k, std::ffi::OsStr::from_bytes(v));
    }
    let o = c.output().unwrap();
    assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want.clone()), "compiled: {}", String::from_utf8_lossy(&o.stderr));
    let file = tmp("std_env_all.isc");
    std::fs::write(&file, src).unwrap();
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_iscc"));
    c.arg("run").arg(&file).env_clear();
    for (k, v) in &vars {
        c.env(k, std::ffi::OsStr::from_bytes(v));
    }
    let o = c.output().unwrap();
    assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want), "interpreted: {}", String::from_utf8_lossy(&o.stderr));
}

/// D451: `readText` refuses the bytes that the runtime would not take as
/// UTF-8 (an error, not a panic of `stringOf`): an encoded surrogate, an
/// overlong form of three and of four bytes, a character above U+10FFFF,
/// a character cut at the end; it takes the largest characters of each
/// length before them; the interpreter and compiled, the same outputs.
#[test]
fn read_text_refuses_bytes_not_utf8() {
    let src = "import * as sys from \"sys\";

action main(args: string[k]): i32 {
    print(join(map(iota(parseI64(args[1])), (i) => match (sys.readText(`${args[0]}/f${i}`)) {
        Ok(t) => `ok ${t.length}`,
        Err(e) => sys.message(e).endsWith(\"not UTF-8\") ? \"not UTF-8\" : \"other\",
    }), \" | \"));
    return 0;
}
";
    let files: [&[u8]; 9] = [
        b"a\xed\xa0\x80",
        b"a\xe0\x80\x80",
        b"a\xf0\x80\x80\x80",
        b"a\xf4\x90\x80\x80",
        b"a\xe2\x82",
        b"\xed\x9f\xbf\xee\x80\x80",
        b"\xf4\x8f\xbf\xbf\xf0\x90\x80\x80",
        b"\xdf\xbf\xc2\x80",
        b"\xef\xbf\xbf",
    ];
    let dir = std::env::temp_dir().join(format!("isc_read_text_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (i, f) in files.iter().enumerate() {
        std::fs::write(dir.join(format!("f{}", i)), f).unwrap();
    }
    let want = files.iter().map(|f| match std::str::from_utf8(f) {
        Ok(t) => format!("ok {}", t.len()),
        Err(_) => "not UTF-8".to_string(),
    }).collect::<Vec<_>>().join(" | ") + "\n";
    let d = dir.to_str().unwrap().to_string();
    let n = files.len().to_string();
    assert_eq!(interp_ok(src, &[&d, &n]), want, "interpreter");
    let r = run_bin(&compile(src, "read_text_not_utf8"), &[&d, &n]);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "compiled: {}", r.stderr);
    let _ = std::fs::remove_dir_all(&dir);
}

/// D464: `path.normalize` reads each part where it is, by its place, not
/// as a string of a `split`: every path of 0 to 4 parts among "", ".",
/// "..", "a", "bc", ".x" and "...", with and without a root `/`, against a reference
/// written here (the rules of the version before it: no `.`, no empty
/// part, `x/..` removed, `..` kept at the start of a relative path and
/// dropped at the root, `.` for nothing); compiled and interpreted.
#[test]
fn normalize_by_the_places_of_its_parts() {
    let src = "import * as path from \"path\";

function pathOf(k: i64, n: i64): string {
    const segs = [\"\", \".\", \"..\", \"a\", \"bc\", \".x\", \"...\"];
    return join(map(iota(n), (j) => segs[(k / [1, 7, 49, 343][j]) % 7]), \"/\");
}

action main(args: string[k]): i32 {
    const all = concat(concat(map(iota(1), (k) => pathOf(k, 0)), map(iota(7), (k) => pathOf(k, 1))), concat(map(iota(49), (k) => pathOf(k, 2)), concat(map(iota(343), (k) => pathOf(k, 3)), map(iota(2401), (k) => pathOf(k, 4)))));
    print(join(map(all, (p) => `${p}|${path.normalize(p)}|${path.normalize(\"/\" + p)}`), \"\\n\"));
    return 0;
}
";
    fn normalize(p: &str) -> String {
        let abs = p.starts_with('/');
        let mut out = if abs { "/".to_string() } else { String::new() };
        let mut names = 0;
        for s in p.split('/') {
            if s.is_empty() || s == "." {
                continue;
            }
            if s == ".." {
                if names > 0 {
                    out = match out.rfind('/') {
                        None => String::new(),
                        Some(0) => "/".to_string(),
                        Some(k) => out[..k].to_string(),
                    };
                    names -= 1;
                } else if !abs {
                    out = if out.is_empty() { "..".to_string() } else { out + "/.." };
                }
                continue;
            }
            out = if out.is_empty() || out == "/" { out + s } else { out + "/" + s };
            names += 1;
        }
        if out.is_empty() { ".".to_string() } else { out }
    }
    let segs = ["", ".", "..", "a", "bc", ".x", "..."];
    let mut want = String::new();
    for n in 0..=4usize {
        for k in 0..7usize.pow(n as u32) {
            let p = (0..n).map(|j| segs[(k / 7usize.pow(j as u32)) % 7]).collect::<Vec<_>>().join("/");
            want.push_str(&format!("{}|{}|{}\n", p, normalize(&p), normalize(&format!("/{}", p))));
        }
    }
    let r = run_bin(&compile(src, "normalize_places"), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout == want, "compiled: {:?}", r.stdout.lines().zip(want.lines()).find(|(a, b)| a != b));
    let got = interp_ok(src, &[]);
    assert!(got == want, "interpreter: {:?}", got.lines().zip(want.lines()).find(|(a, b)| a != b));
}

/// D479: `dirname`, `basename` and `extension` with their last `/` and the
/// `/` at their end found from the end: every path of 0 to 3 parts among
/// 9 (empty, `.`, `..`, names, dots, extensions), with and without a root,
/// with and without `/` at the end; the texts of the module of 0.4.5,
/// written in `std_path_parts.out`; compiled and interpreted.
#[test]
fn path_parts_as_before() {
    let src = include_str!("data/std_path_parts.isc");
    let want = include_str!("data/std_path_parts.out");
    let first = |got: &str| got.lines().zip(want.lines()).enumerate().find(|(_, (a, b))| a != b).map(|(i, (a, b))| format!("line {}: {:?} against {:?}", i + 1, a, b));
    let got = interp_ok(src, &[]);
    assert!(got == want, "interpreter: {:?}", first(&got));
    let r = run_bin(&compile(src, "path_parts"), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout == want, "compiled: {:?}", first(&r.stdout));
}
