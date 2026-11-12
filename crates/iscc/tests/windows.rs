//! The Windows target (`iscc build --target x86_64-windows`, D246): each
//! program built for Linux and for Windows, run on both, with the same
//! standard output, error and exit code. The Windows machine is reached by
//! `$ISC_WINDOWS_RUNNER` (`tools/windows/run.py`, configured by its own
//! variables); without it, the checks are skipped with a message. The
//! linker needs MinGW-w64 (`$ISC_MINGW_DIR`, else `/usr`).

mod common;
use common::*;
use std::path::{Path, PathBuf};

fn runner() -> Option<PathBuf> {
    std::env::var("ISC_WINDOWS_RUNNER").ok().map(PathBuf::from)
}

fn compile_windows(src: &str, name: &str) -> PathBuf {
    compile_windows_with(src, name, false)
}

/// `compile_windows`, with Microsoft's ABI when `msvc` (D542).
fn compile_windows_with(src: &str, name: &str, msvc: bool) -> PathBuf {
    let o = isc_codegen::Options { windows: true, msvc, ..isc_codegen::Options::default() };
    let p = iscc::frontend(&format!("{}.isc", name), src).unwrap_or_else(|e| panic!("{}", e));
    let out = tmp(&format!("{}.exe", name));
    let opts = iscc::BuildOptions { output: out.clone(), emit_llvm: None, emit_asm: None, debug: false, codegen: o, link: Vec::new(), glibc: false };
    iscc::build(&p, &opts).unwrap_or_else(|e| panic!("{}: {}", name, e));
    out
}

/// A directory of its own for the files of one program.
fn dir(name: &str) -> PathBuf {
    let d = tmp(&format!("win_{}", name));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Runs `exe` on Windows: (exit code, stdout, stderr).
fn run_windows(runner: &Path, exe: &Path, d: &Path, args: &[&str], stdin: Option<&str>, threads: usize) -> (i32, Vec<u8>, Vec<u8>) {
    let mut c = std::process::Command::new(runner);
    c.arg("--dir").arg(d).arg("--env").arg(format!("ISC_THREADS={}", threads));
    if let Some(f) = stdin {
        c.arg("--stdin").arg(d.join(f));
    }
    let o = c.arg(exe).args(args).output().unwrap();
    (o.status.code().unwrap_or(-1), o.stdout, o.stderr)
}

/// Runs the Linux build in `d` (relative paths as on Windows).
fn run_linux(bin: &Path, d: &Path, args: &[&str], stdin: Option<&str>, threads: usize) -> (i32, Vec<u8>, Vec<u8>) {
    let mut c = std::process::Command::new(bin);
    c.current_dir(d).args(args).env("ISC_THREADS", threads.to_string());
    match stdin {
        Some(f) => c.stdin(std::fs::File::open(d.join(f)).unwrap()),
        None => c.stdin(std::process::Stdio::null()),
    };
    let o = c.output().unwrap();
    (o.status.code().unwrap_or(-1), o.stdout, o.stderr)
}

/// `src` built for both systems and run with `args` (files of `d`), on 1 and
/// 4 threads: the same results.
fn same_on_windows(runner: &Path, src: &str, name: &str, d: &Path, args: &[&str], stdin: Option<&str>) -> Vec<u8> {
    same_on_windows_with(runner, src, name, d, args, stdin, false)
}

/// `same_on_windows`, the program for Windows with Microsoft's ABI when
/// `msvc` (D542).
fn same_on_windows_with(runner: &Path, src: &str, name: &str, d: &Path, args: &[&str], stdin: Option<&str>, msvc: bool) -> Vec<u8> {
    // names of their own: a test and its variant for Microsoft's ABI run
    // at the same time
    let suffix = if msvc { "_msvc" } else { "" };
    let bin = compile(src, &format!("winlinux_{}{}", name, suffix));
    let exe = compile_windows_with(src, &format!("win_{}{}", name, suffix), msvc);
    let mut first = None;
    for t in [1, 4] {
        let l = run_linux(&bin, d, args, stdin, t);
        let w = run_windows(runner, &exe, d, args, stdin, t);
        assert_eq!(w.0, l.0, "{}: exit code on Windows ({} threads); stderr: {}", name, t, String::from_utf8_lossy(&w.2));
        assert!(w.1 == l.1, "{}: standard output on Windows ({} threads):\n{}\nLinux:\n{}", name, t, String::from_utf8_lossy(&w.1), String::from_utf8_lossy(&l.1));
        assert_eq!(String::from_utf8_lossy(&w.2), String::from_utf8_lossy(&l.2), "{}: standard error ({} threads)", name, t);
        first.get_or_insert(l.1);
    }
    first.unwrap()
}

const BASICS: &str = r#"function depth(n: i64): i64 {
    return n == 0 ? 0 : 1 + depth(n - 1);
}

action main(args: string[k]): i32 {
    print(args);
    print("été, 東京, 🦀");
    const xs = map(iota(3000000), (i) => f64(i) * 0.5);
    print(sum(xs));
    print([0.1 + 0.2, 1.0 / 3.0, 1e300 * 10.0, -0.0]);
    print(sortBy(["pear", "fig", "apple"], (s) => s));
    print(countBy(iota(100000), (i) => i % 7).entries());
    print(matches("a1 b22 c333", /[a-z][0-9]+/g));
    print(BigInt(2) ** 200n);
    print(depth(1000000));
    print(clock() > 0u64);
    return 0;
}
"#;

const FILES: &str = r#"action main(args: string[k]): i32 {
    writeBytes("bytes.bin", map(iota(70000), (i) => u8(i % 251)));
    const b = readBytes("bytes.bin");
    print([b.length, i64(b[69999]), sum(map(b, (x) => i64(x)))]);
    writeText("text.txt", "line one\nline two\n");
    print(readText("text.txt"));
    print(readText(args[0]).length);
    print(readStdinLines().length);
    const missing = tryReadText("missing.txt");
    print(match (missing) {
        Ok(t) => t,
        Err(e) => e,
    });
    return 0;
}
"#;

const PANICS: &str = r#"action main(args: string[k]): i32 {
    const xs = [1, 2, 3];
    print(xs[0]);
    print(xs[args.length + 5]);
    return 0;
}
"#;

const OVERFLOW: &str = r#"function down(n: i64): i64 {
    return n < 0 ? 0 : down(n + 1) + down(n + 2);
}

action main(args: string[k]): i32 {
    print(down(0));
    return 0;
}
"#;

/// The programs of `programs_on_windows` built with Microsoft's ABI (`--target
/// x86_64-windows-msvc`, D542): linked by LLD as Microsoft's `link`, without
/// a C library, the same outputs as on Linux.
#[test]
fn programs_on_windows_msvc() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let d = dir("basics_msvc");
    same_on_windows_with(&runner, BASICS, "basics", &d, &["one", "two words", "", "été 東京 🦀", "say \"hi\"", "C:\\dir\\file"], None, true);
    let d = dir("files_msvc");
    std::fs::write(d.join("big.txt"), "x".repeat(9 << 20)).unwrap();
    std::fs::write(d.join("in.txt"), "a\nb\nc\n").unwrap();
    same_on_windows_with(&runner, FILES, "files", &d, &["big.txt"], Some("in.txt"), true);
    let d = dir("panics_msvc");
    same_on_windows_with(&runner, PANICS, "panics", &d, &[], None, true);
    let d = dir("overflow_msvc");
    same_on_windows_with(&runner, OVERFLOW, "overflow", &d, &[], None, true);
}

/// A static library built by Visual Studio (`cl /GS`, `lib`) in the machine
/// of Windows (its Build Tools, D528) linked into a program of Isocron by
/// `iscc.exe build --target x86_64-windows-msvc --link` (D542): its
/// functions use what Microsoft's C runtime gives (the cookie of `/GS` of a
/// buffer on the stack, `snprintf`, `_fltused` of floats, `__chkstk` of a
/// frame of more than a page, `malloc`), so that the program is linked with
/// that runtime, static, found by the `LIB` of `vcvars64.bat`: the right
/// results, and no DLL but those of Windows.
#[test]
fn msvc_static_library_on_windows() {
    let (Some(runner), Ok(iscc)) = (runner(), std::env::var("ISC_WINDOWS_ISCC")) else {
        eprintln!("no Windows machine or no iscc.exe (ISC_WINDOWS_RUNNER, ISC_WINDOWS_ISCC): the static libraries of Visual Studio not run");
        return;
    };
    let vcvars = std::env::var("ISC_WINDOWS_VCVARS").unwrap_or_else(|_| "C:\\BuildTools\\VC\\Auxiliary\\Build\\vcvars64.bat".into());
    let llvm = std::env::var("ISC_WINDOWS_LLVM_BIN").unwrap_or_else(|_| "C:\\LLVM\\bin".into());
    let d = dir("msvc_static");
    std::fs::copy(&iscc, d.join("iscc.exe")).unwrap();
    std::fs::write(
        d.join("calc.c"),
        "#include <stdio.h>\n#include <stdlib.h>\n#include <stdint.h>\n\
int64_t calc_digits(int64_t n) { char buf[32]; return (int64_t)snprintf(buf, sizeof buf, \"%lld\", (long long)n); }\n\
double calc_scale(double x) { return x * 2.5; }\n\
int64_t calc_big(int64_t n) { volatile char a[20000]; a[0] = (char)n; a[19999] = (char)(n + 1); return a[0] + a[19999]; }\n\
int64_t calc_heap(int64_t n) { int64_t *p = malloc((size_t)n * sizeof *p); int64_t s = 0; for (int64_t i = 0; i < n; i++) p[i] = i; for (int64_t i = 0; i < n; i++) s += p[i]; free(p); return s; }\n",
    )
    .unwrap();
    std::fs::write(
        d.join("prog.isc"),
        "declare function calc_digits(n: i64): i64;\ndeclare function calc_scale(x: f64): f64;\ndeclare function calc_big(n: i64): i64;\ndeclare function calc_heap(n: i64): i64;\n\n\
action main(args: string[k]): i32 {\n    print(calc_digits(1234567));\n    print(calc_scale(4.0));\n    print(calc_big(20));\n    print(calc_heap(1000));\n    print(sum(map(iota(1000000), (i) => i)));\n    print(args);\n    return 0;\n}\n",
    )
    .unwrap();
    let script = format!(
        "@echo off\r\ncall \"{}\" > nul || exit /b 78\r\nset \"PATH={};%PATH%\"\r\ncl /nologo /c /O2 /GS calc.c > nul || exit /b 79\r\nlib /nologo /out:calc.lib calc.obj || exit /b 80\r\niscc.exe build prog.isc --target x86_64-windows-msvc --link calc.lib || exit /b 77\r\nprog.exe \"a b\" c\r\necho == %ERRORLEVEL%\r\ndumpbin /nologo /dependents prog.exe | findstr /i /c:\".dll\"\r\n",
        vcvars, llvm
    );
    std::fs::write(d.join("go.cmd"), script).unwrap();
    let o = std::process::Command::new(&runner).arg("--dir").arg(&d).arg(d.join("go.cmd")).output().unwrap();
    let (out, err) = (String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n"), String::from_utf8_lossy(&o.stderr).to_string());
    assert_eq!(o.status.code(), Some(0), "{}\n{}", out, err);
    let dlls: Vec<&str> = out.lines().skip(7).map(|l| l.trim()).collect();
    assert_eq!(out.lines().take(7).collect::<Vec<_>>(), ["7", "10", "41", "499500", "499999500000", "[a b, c]", "== 0"], "{}\n{}", out, err);
    assert!(!dlls.is_empty() && dlls.iter().all(|l| l.eq_ignore_ascii_case("kernel32.dll") || l.to_ascii_lowercase().starts_with("api-ms-win-core-")), "the DLLs of the program: {:?}", dlls);
}

/// Printing, threads, floats, strings in UTF-8, maps, sorting, regular
/// expressions, big integers, a recursion a million deep (the big stack of
/// `main`), the clock; files written and read (a large one: the reads in
/// parallel), the standard input, a missing file (the message of Linux);
/// panics (exit code 101, the message), a stack overflow.
#[test]
fn programs_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let d = dir("basics");
    // the arguments read from the command line in UTF-16 by the runtime
    // (D541): characters beyond the code page, quotes, backslashes
    same_on_windows(&runner, BASICS, "basics", &d, &["one", "two words", "", "été 東京 🦀", "say \"hi\"", "C:\\dir\\file"], None);
    let d = dir("files");
    std::fs::write(d.join("big.txt"), "x".repeat(9 << 20)).unwrap();
    std::fs::write(d.join("in.txt"), "a\nb\nc\n").unwrap();
    same_on_windows(&runner, FILES, "files", &d, &["big.txt"], Some("in.txt"));
    let d = dir("panics");
    same_on_windows(&runner, PANICS, "panics", &d, &[], None);
    let d = dir("overflow");
    same_on_windows(&runner, OVERFLOW, "overflow", &d, &[], None);
}

/// D664: the remainders of floats (`%`) of a program without a C library
/// (LLVM calls `fmod` and `fmodf`, which no library gave it: the program
/// did not link) computed by its own functions, exact: signed zeros,
/// infinities, NaN, subnormals, 20 000 pairs drawn over the exponents; the
/// same digits as on Linux (glibc's `fmod`), with both ABIs.
#[test]
fn float_remainders_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let src = include_str!("data/float_remainder.isc");
    let d = dir("float_remainder");
    let out = same_on_windows(&runner, src, "float_remainder", &d, &["20000"], None);
    assert_eq!(out.iter().filter(|b| **b == b'\n').count(), 20002);
    let d = dir("float_remainder_msvc");
    same_on_windows_with(&runner, src, "float_remainder", &d, &["20000"], None, true);
}

fn source(name: &str) -> String {
    let path = format!("{}/../../examples/benchmarks_game/{}.isc", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(path).unwrap()
}

/// The ten programs of the Benchmarks Game at the sizes of their reference
/// outputs, the FASTA input made by `fasta`: the same bytes on Windows.
#[test]
fn benchmarks_game_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let d = dir("bg");
    let input = same_on_windows(&runner, &source("fasta"), "fasta", &d, &["1000"], None);
    std::fs::write(d.join("input.fasta"), &input).unwrap();
    for (name, arg) in [("binarytrees", "10"), ("fannkuchredux", "7"), ("mandelbrot", "200"), ("nbody", "1000"), ("pidigits", "27"), ("spectralnorm", "100")] {
        same_on_windows(&runner, &source(name), name, &d, &[arg], None);
    }
    for name in ["knucleotide", "regexredux", "revcomp"] {
        same_on_windows(&runner, &source(name), name, &d, &["0"], Some("input.fasta"));
    }
}

const C_FUNCTIONS: &str = r#"declare function abs(v: i32): i32;
declare function strlen(s: string): u64;
declare function pow(x: f64, y: f64): f64;
declare function ldexp(x: f64, e: i32): f64;
declare function strncmp(a: string, b: string, n: u64): i32;
declare function MultiByteToWideChar(cp: u32, flags: u32, s: string, n: i32, wide: i64, room: i32): i32;

action main(args: string[k]): i32 {
    print(abs(-12i32));
    print(strlen("héllo"));
    print(pow(2.0, 10.0));
    print(ldexp(3.0, 4i32));
    print(strncmp("abcdef", "abcxyz", 3u64));
    print(MultiByteToWideChar(65001u32, 0u32, "héllo", -1i32, 0, 0i32));
    return 0;
}
"#;

/// iscc itself on Windows (D247, `$ISC_WINDOWS_ISCC`: an iscc.exe built by
/// `tools/windows/build_iscc.sh`), with LLVM 21 in `$ISC_WINDOWS_LLVM_BIN`
/// of the Windows machine (default `C:\LLVM\bin`): `iscc build` without a
/// target makes a Windows program, which prints what the Linux program
/// prints, and `iscc run` (the interpreter) prints the same; C functions
/// of Windows called by both, with arguments in the registers of both kinds
/// and on the stack (Win64).
#[test]
fn iscc_on_windows() {
    let (Some(runner), Ok(iscc)) = (runner(), std::env::var("ISC_WINDOWS_ISCC")) else {
        eprintln!("no Windows machine or no iscc.exe (ISC_WINDOWS_RUNNER, ISC_WINDOWS_ISCC): iscc on Windows not run");
        return;
    };
    let llvm = std::env::var("ISC_WINDOWS_LLVM_BIN").unwrap_or_else(|_| "C:\\LLVM\\bin".into());
    for (name, src, args, stdin, interpret) in [
        ("basics", BASICS, &["one", "two words"][..], None, false),
        ("files", FILES, &["big.txt"][..], Some("in.txt"), true),
        ("panics", PANICS, &[][..], None, true),
        ("cfunctions", C_FUNCTIONS, &[][..], None, true),
    ] {
        let d = dir(&format!("native_{}", name));
        std::fs::copy(&iscc, d.join("iscc.exe")).unwrap();
        std::fs::write(d.join("prog.isc"), src).unwrap();
        if name == "files" {
            std::fs::write(d.join("big.txt"), "x".repeat(9 << 20)).unwrap();
            std::fs::write(d.join("in.txt"), "a\nb\nc\n").unwrap();
        }
        let input = stdin.map(|f| format!(" < {}", f)).unwrap_or_default();
        let argv = args.iter().map(|a| format!("\"{}\"", a)).collect::<Vec<_>>().join(" ");
        // the compiled program, then the interpreter: each one's output,
        // error and exit code
        let mut script = format!("@echo off\r\nset \"PATH={};%PATH%\"\r\niscc.exe build prog.isc || exit /b 77\r\nprog.exe {}{}\r\necho == %ERRORLEVEL%\r\n", llvm, argv, input);
        if interpret {
            script += &format!("iscc.exe run prog.isc {}{}\r\necho == %ERRORLEVEL%\r\n", argv, input);
        }
        std::fs::write(d.join("go.cmd"), script).unwrap();
        let o = std::process::Command::new(&runner).arg("--dir").arg(&d).arg(d.join("go.cmd")).output().unwrap();
        let (out, err) = (String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string());
        assert_eq!(o.status.code(), Some(0), "{}: {}\n{}", name, out, err);
        let want = if name == "cfunctions" {
            "12\n6\n1024\n48\n0\n6\n== 0\n".to_string()
        } else {
            let bin = compile(src, &format!("native_linux_{}", name));
            let l = run_linux(&bin, &d, args, stdin, 4);
            format!("{}== {}\n", String::from_utf8_lossy(&l.1), l.0)
        };
        let want = if interpret { format!("{}{}", want, want) } else { want };
        // `echo` of cmd ends its lines with \r\n
        assert_eq!(out.replace("\r\n", "\n"), want, "{}: iscc on Windows", name);
    }
}

/// D331: the module `sys` on Windows (`kernel32`): every function in a new
/// folder `w`, the same outputs as on Linux (the folder, its files, the
/// errors of the six kinds).
#[test]
fn sys_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let d = dir("sys");
    let out = same_on_windows(&runner, include_str!("data/std_sys.isc"), "sys", &d, &["w"], None);
    assert!(String::from_utf8_lossy(&out).starts_with("ok\nok\nalready exists: w/a\n"), "{}", String::from_utf8_lossy(&out));
}

/// D334: the module `time` on Windows (its clock by
/// `GetSystemTimePreciseAsFileTime`): the same outputs as on Linux.
#[test]
fn time_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let d = dir("time");
    same_on_windows(&runner, include_str!("data/std_time.isc"), "time", &d, &[], None);
}

/// D335, D336, D338: the modules `json` and `sha256` on Windows (pure
/// Isocron, its threads those of Windows): reading, decoding into types
/// with their faults, writing; the hashes of the vectors of the NIST; the
/// same outputs as on Linux.
#[test]
fn json_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    for (src, name) in [
        (include_str!("data/std_json.isc"), "json"),
        (include_str!("data/std_json_types.isc"), "json_types"),
        (include_str!("data/std_json_decode.isc"), "json_decode"),
        (include_str!("data/std_json_strings.isc"), "json_strings"),
        (include_str!("data/std_sha256.isc"), "sha256"),
    ] {
        let d = dir(name);
        same_on_windows(&runner, src, name, &d, &[], None);
    }
}

/// D339, D340, D342: the modules `net` and `http` (client and server) on Windows
/// (Winsock): a listener, a connection,
/// bytes both ways, a read past its delay, a connection refused; the same
/// outputs as on Linux (the server of DNS of the test is on Linux: the
/// program asks it only when given its port).
#[test]
fn net_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let d = dir("net");
    same_on_windows(&runner, include_str!("data/std_net.isc"), "net", &d, &[], None);
    // the client of HTTP on responses of every form (D340)
    let d = dir("http");
    same_on_windows(&runner, include_str!("data/std_http.isc"), "http", &d, &[], None);
    // the server of HTTP on requests of every form (D342: `WSAPoll`)
    let d = dir("http_serve");
    same_on_windows(&runner, include_str!("data/std_http_serve.isc"), "http_serve", &d, &[], None);
    // a handler that is an action (D352): it prints and reads a file
    let d = dir("http_serve_action");
    same_on_windows(&runner, include_str!("data/std_http_serve_action.isc"), "http_serve_action", &d, &[], None);
}

/// D343: the client of HTTPS on Windows (SChannel) against two servers of
/// `openssl s_server` on this machine, at its address that the virtual
/// machine reaches (`ISC_WINDOWS_HOST_ADDR`, by default 172.17.0.1, the
/// bridge of Docker): certificates made by the test (an authority, a
/// certificate for that address, one for `localhost` only, another
/// authority), then a small body and 3000000 bytes, another authority, a
/// name the certificate has not, a `ca` that is not one, the authorities of
/// the system, a connection refused; the same outputs as on Linux (the
/// faults by their kinds: their texts are the library's).
#[test]
fn https_client_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let addr = std::env::var("ISC_WINDOWS_HOST_ADDR").unwrap_or_else(|_| "172.17.0.1".into());
    let d = dir("https_client");
    let certs = tmp("win_https_certs");
    let _ = std::fs::remove_dir_all(&certs);
    std::fs::create_dir_all(certs.join("www")).unwrap();
    let run = |args: &[&str]| assert!(std::process::Command::new("openssl").args(args).current_dir(&certs).output().unwrap().status.success(), "openssl {:?}", args);
    let ec = ["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes"];
    std::fs::write(certs.join("ip.cnf"), format!("subjectAltName=IP:{}\n", addr)).unwrap();
    std::fs::write(certs.join("named.cnf"), "subjectAltName=DNS:localhost\n").unwrap();
    run(&[&["req", "-x509"][..], &ec, &["-keyout", "ca.key", "-out", "ca.pem", "-days", "2", "-subj", "/CN=isocron test ca"]].concat());
    run(&[&["req", "-x509"][..], &ec, &["-keyout", "other.key", "-out", "other.pem", "-days", "2", "-subj", "/CN=another ca"]].concat());
    for n in ["ip", "named"] {
        run(&[&["req"][..], &ec, &["-keyout", &format!("{}.key", n), "-out", &format!("{}.csr", n), "-subj", "/CN=test server"]].concat());
        run(&["x509", "-req", "-in", &format!("{}.csr", n), "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", &format!("{}.pem", n), "-days", "2", "-extfile", &format!("{}.cnf", n)]);
    }
    std::fs::write(certs.join("www/hello.txt"), "hello over tls\n").unwrap();
    std::fs::write(certs.join("www/big.bin"), (0..3_000_000u32).map(|i| (i % 253) as u8).collect::<Vec<u8>>()).unwrap();
    for f in ["ca.pem", "other.pem"] {
        std::fs::copy(certs.join(f), d.join(f)).unwrap();
    }
    // the two servers, on ports of that address they choose themselves
    // (`common::s_server`), stopped when the test ends, a failure included
    // (two were found running hours after a run whose connection to the
    // machine failed)
    let mut servers = Servers(Vec::new());
    let mut ports = Vec::new();
    for n in ["ip", "named"] {
        let (c, p) = s_server(&certs.join("www"), &format!("{}:0", addr), &format!("../{}.pem", n), &format!("../{}.key", n));
        servers.0.push(c);
        ports.push(p);
    }
    let (pa, pb) = (ports[0], ports[1]);
    let big: u64 = (0..3_000_000u64).map(|i| i % 253).sum();
    let want = format!("200 hello over tls\n200 3000000 {}\ntls certificate\ntls certificate\ntls no certificate in the PEM text\ntls certificate\nrefused\n", big);
    let (sa, sb) = (pa.to_string(), pb.to_string());
    let args = [addr.as_str(), &sa, &sb, "ca.pem", "other.pem"];
    let out = same_on_windows(&runner, include_str!("data/std_https_remote_client.isc"), "https_client", &d, &args, None);
    drop(servers);
    assert_eq!(String::from_utf8_lossy(&out), want);
}

/// Servers of a test, killed when it ends, by a panic too.
struct Servers(Vec<std::process::Child>);

impl Drop for Servers {
    fn drop(&mut self) {
        for s in &mut self.0 {
            let _ = s.kill();
            let _ = s.wait();
        }
    }
}

/// D343: the server of HTTPS on Windows (SChannel), its client too, in
/// the real VM: the pair of programs of `data/std_https_pair_*.isc` (a
/// script of `cmd` starts the server, then the client), compiled, then
/// interpreted by `$ISC_WINDOWS_ISCC` when it is given; certificates made
/// by the test (EC); the same outputs as the pair on Linux; its key in the
/// store of the machine while it serves, removed after it (D485).
#[test]
fn https_server_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let certs = tmp("win_https_pair_certs");
    let _ = std::fs::remove_dir_all(&certs);
    std::fs::create_dir_all(&certs).unwrap();
    let run = |args: &[&str]| assert!(std::process::Command::new("openssl").args(args).current_dir(&certs).output().unwrap().status.success(), "openssl {:?}", args);
    let ec = ["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes"];
    std::fs::write(certs.join("ext.cnf"), "subjectAltName=DNS:localhost\n").unwrap();
    run(&[&["req", "-x509"][..], &ec, &["-keyout", "ca.key", "-out", "ca.pem", "-days", "2", "-subj", "/CN=isocron test ca"]].concat());
    run(&[&["req", "-x509"][..], &ec, &["-keyout", "other.key", "-out", "other.pem", "-days", "2", "-subj", "/CN=another ca"]].concat());
    run(&[&["req"][..], &ec, &["-keyout", "server.key", "-out", "server.csr", "-subj", "/CN=localhost"]].concat());
    run(&["x509", "-req", "-in", "server.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", "server.pem", "-days", "2", "-extfile", "ext.cnf"]);
    let (server_src, client_src) = (include_str!("data/std_https_pair_server.isc"), include_str!("data/std_https_pair_client.isc"));
    // the pair on Linux: what Windows must print
    let (lserver, lclient) = (compile(server_src, "winlinux_https_pair_server"), compile(client_src, "winlinux_https_pair_client"));
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port().to_string();
    let mut server = std::process::Command::new(&lserver).args([&port, "server.pem", "server.key", "3"]).current_dir(&certs).stdout(std::process::Stdio::piped()).spawn().unwrap();
    while std::net::TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap())).is_err() {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let l = run_linux(&lclient, &certs, &[&port, "ca.pem", "other.pem"], None, 4);
    let lo = server.wait_with_output().unwrap();
    let want = format!("{}{}", String::from_utf8_lossy(&l.1), String::from_utf8_lossy(&lo.stdout));
    assert!(want.ends_with("200 2000000 249996496\ndone\n"), "Linux: {}", want);
    // the pair on Windows: compiled, then interpreted
    let d = dir("https_pair");
    for f in ["ca.pem", "other.pem", "server.pem", "server.key"] {
        std::fs::copy(certs.join(f), d.join(f)).unwrap();
    }
    std::fs::copy(compile_windows(server_src, "win_https_pair_server"), d.join("server.exe")).unwrap();
    std::fs::copy(compile_windows(client_src, "win_https_pair_client"), d.join("client.exe")).unwrap();
    std::fs::write(d.join("server.isc"), server_src).unwrap();
    std::fs::write(d.join("client.isc"), client_src).unwrap();
    let mut ways = vec![("compiled", "server.exe".to_string(), "client.exe".to_string())];
    if let Ok(iscc) = std::env::var("ISC_WINDOWS_ISCC") {
        std::fs::copy(iscc, d.join("iscc.exe")).unwrap();
        ways.push(("interpreted", "iscc.exe run server.isc".to_string(), "iscc.exe run client.isc".to_string()));
    }
    // the port of the server open, at most 60 s (D365: a fixed wait of 4 s
    // was not always enough, the server opening its port after 1.1 to more
    // than 4 s on the first runs after the start of the machine); the
    // connection of this test is not a request of TLS, the server not
    // counting it
    std::fs::write(
        d.join("wait.ps1"),
        "param([int]$port)\r\n$s = Get-Date\r\nwhile (((Get-Date) - $s).TotalSeconds -lt 60) {\r\n  try { $c = New-Object System.Net.Sockets.TcpClient; $c.Connect('127.0.0.1', $port); $c.Close(); exit 0 } catch { Start-Sleep -Milliseconds 100 }\r\n}\r\nexit 1\r\n",
    )
    .unwrap();
    // D485: the key of the server in the store of the machine while it
    // serves (one file more in its folder), removed when it ends (as many
    // files as before); the user of the machine of the tests may write there.
    // `/a-d`: the files of keys are hidden and of the system, which `dir`
    // leaves out by default
    let count = "for /f %%c in ('dir /b /a-d C:\\ProgramData\\Microsoft\\Crypto\\Keys 2^>nul ^| find /c /v \"\"') do set";
    let keys = "set /a plus=before+1\r\nif \"%during%\"==\"%plus%\" (echo a key of the machine while serving) else (echo keys of the machine: %before% then %during% while serving)\r\nif \"%before%\"==\"%after%\" (echo keys of the machine as before) else (echo keys of the machine: %before% then %after%)\r\n";
    let want = format!("{}a key of the machine while serving\nkeys of the machine as before\n", want);
    for (name, s, c) in ways {
        // the server in the background, its output in a file (written at
        // its end), the client when it listens
        let script = format!(
            "@echo off\r\n{count} before=%%c\r\nstart \"\" /b cmd /c \"{} 47970 server.pem server.key 3 > server.txt 2>&1\"\r\npowershell -NoProfile -ExecutionPolicy Bypass -File wait.ps1 47970\r\n{count} during=%%c\r\n{} 47970 ca.pem other.pem\r\nping -n 3 127.0.0.1 > nul\r\ntype server.txt\r\n{count} after=%%c\r\n{keys}exit /b 0\r\n",
            s, c
        );
        std::fs::write(d.join("pair.cmd"), script).unwrap();
        let o = std::process::Command::new(&runner).arg("--dir").arg(&d).arg(d.join("pair.cmd")).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n"), want, "{} on Windows: {}", name, String::from_utf8_lossy(&o.stderr));
    }
}

/// D348: a port listened to twice on Windows: the second `listen` refused
/// (`SO_EXCLUSIVEADDRUSE`; with `SO_REUSEADDR` a second server took the same
/// port), compiled, then interpreted by `$ISC_WINDOWS_ISCC` when it is
/// given: the same output as on Linux.
#[test]
fn a_port_listened_to_twice_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let src = include_str!("data/std_net_twice.isc");
    let d = dir("net_twice");
    let out = same_on_windows(&runner, src, "net_twice", &d, &[], None);
    assert_eq!(String::from_utf8_lossy(&out), "refused: already exists\n");
    if let Ok(iscc) = std::env::var("ISC_WINDOWS_ISCC") {
        std::fs::copy(iscc, d.join("iscc.exe")).unwrap();
        std::fs::write(d.join("twice.isc"), src).unwrap();
        std::fs::write(d.join("twice.cmd"), "@echo off\r\niscc.exe run twice.isc\r\nexit /b %errorlevel%\r\n").unwrap();
        let o = std::process::Command::new(&runner).arg("--dir").arg(&d).arg(d.join("twice.cmd")).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n"), "refused: already exists\n", "interpreted on Windows: {}", String::from_utf8_lossy(&o.stderr));
    }
}

/// The output of `actions_parallel.isc` cut at each line printed outside
/// `mapParallel`: the lines of its tasks (in any order) sorted, then that
/// line (as in `actions.rs`).
fn parallel_segments(out: &[u8]) -> Vec<(Vec<String>, String)> {
    let (mut segs, mut cur) = (Vec::new(), Vec::new());
    for l in String::from_utf8_lossy(out).lines() {
        if (l.starts_with('[') && l.matches(',').count() == 11) || l.starts_with("word ") {
            cur.push(l.to_string());
        } else {
            cur.sort();
            segs.push((std::mem::take(&mut cur), l.to_string()));
        }
    }
    segs
}

/// D373: `mapParallel` under Windows: 16 tasks printing at the same time
/// (no line cut: the lock of `print` with the thread-local storage of
/// Windows), the same lines and results as under Linux on 1 and 4 threads;
/// two tasks that wait for each other through a socket, on 2 and 4 threads.
#[test]
fn actions_in_parallel_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let src = include_str!("data/actions_parallel.isc");
    let bin = compile(src, "winlinux_actions_parallel");
    let exe = compile_windows(src, "win_actions_parallel");
    let d = dir("actions_parallel");
    for t in [1, 4] {
        let l = run_linux(&bin, &d, &[], None, t);
        let w = run_windows(&runner, &exe, &d, &[], None, t);
        assert_eq!((w.0, l.0), (0, 0), "{} threads; stderr: {}", t, String::from_utf8_lossy(&w.2));
        assert!(parallel_segments(&w.1) == parallel_segments(&l.1), "{} threads: other lines on Windows", t);
        assert_eq!(parallel_segments(&w.1).len(), 5, "{} threads", t);
    }
    let src = include_str!("data/actions_parallel_wait.isc");
    let exe = compile_windows(src, "win_actions_parallel_wait");
    let d = dir("actions_parallel_wait");
    for t in [2, 4] {
        let w = run_windows(&runner, &exe, &d, &["18766"], None, t);
        assert_eq!((w.0, String::from_utf8_lossy(&w.1).into_owned()), (0, "[hello from the other task, sent]\n".to_string()), "{} threads; stderr: {}", t, String::from_utf8_lossy(&w.2));
    }
}

/// D422: every program of the guide that runs alone (no standard input, no
/// module of its own) built for Windows and run in the machine, on one
/// thread: the output that the guide gives (the test of the guide checks it
/// interpreted, compiled for Linux and as WebAssembly), so that each
/// function of the library its example calls runs on Windows too.
#[test]
fn the_guide_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the guide not run on Windows");
        return;
    };
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/language.md")).unwrap();
    let lines: Vec<&str> = doc.lines().collect();
    // the fenced blocks: (language, line of the fence, text, line of its end)
    let mut blocks: Vec<(String, usize, String, usize)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if let Some(lang) = lines[i].strip_prefix("```") {
            let mut j = i + 1;
            while j < lines.len() && !lines[j].starts_with("```") {
                j += 1;
            }
            let text: String = lines[i + 1..j].iter().map(|l| format!("{}\n", l)).collect();
            blocks.push((lang.trim().to_string(), i + 1, text, j));
            i = j + 1;
        } else {
            i += 1;
        }
    }
    let d = tmp("win_guide");
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mut ran = 0;
    for k in 0..blocks.len() {
        let (lang, line, src, end) = &blocks[k];
        let module_before = k > 0 && blocks[k - 1].0.starts_with("isc-file");
        if lang != "isc" || module_before {
            continue;
        }
        let Some((next, nline, want, _)) = blocks.get(k + 1) else { continue };
        if next != "output" || lines[end + 1..nline - 1].iter().any(|l| !l.trim().is_empty()) {
            continue;
        }
        // an example of the module `unix` (Linux only, D632): refused for
        // Windows, the primitive named
        let p = iscc::frontend("guide.isc", src).unwrap_or_else(|e| panic!("docs/language.md:{}: {}", line, e));
        if p.sys_paths[1].iter().any(|(op, _, _)| !op.on_windows()) {
            let opts = iscc::BuildOptions { output: tmp(&format!("win_guide_{}.exe", k)), emit_llvm: None, emit_asm: None, debug: false, codegen: isc_codegen::Options { windows: true, ..isc_codegen::Options::default() }, link: Vec::new(), glibc: false };
            let e = iscc::build(&p, &opts).expect_err("an example of the module unix built for Windows");
            assert!(e.contains("(the module `unix` is Linux only)"), "docs/language.md:{}: {}", line, e);
            continue;
        }
        let exe = compile_windows(src, &format!("win_guide_{}", k));
        let w = run_windows(&runner, &exe, &d, &[], None, 1);
        assert_eq!(w.0, 0, "docs/language.md:{}: exit code on Windows; stderr: {}", line, String::from_utf8_lossy(&w.2));
        assert_eq!(String::from_utf8_lossy(&w.1), want.as_str(), "docs/language.md:{}: the output on Windows", line);
        ran += 1;
    }
    assert!(ran >= 40, "only {} programs of the guide run", ran);
    eprintln!("{} programs of the guide run on Windows", ran);
}

/// D425: the MSVC ABI at the boundary of a DLL. C functions compiled by
/// Microsoft called by a program of the target of MinGW (GNU) through
/// `declare function`: `GetTickCount64`, `lstrlenA` and `MulDiv` of
/// kernel32, `strlen` of msvcrt (the convention of Win64 is the same for
/// both ABIs, D246); and those of `shlwapi.dll`, which iscc does not link,
/// through an import library in the format of Microsoft's `lib.exe` (short
/// import members, made here from a `.def` by `lld-link /lib`) given by
/// `--link`, as one given with a DLL built by Visual Studio.
#[test]
fn msvc_compiled_c_functions_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let d = dir("msvc_c");
    let def = d.join("shlwapi.def");
    std::fs::write(&def, "LIBRARY shlwapi.dll\nEXPORTS\n    StrToIntA\n    PathIsRelativeA\n    ChrCmpIA\n").unwrap();
    let lib = tmp("shlwapi_msvc.lib");
    let sysroot = std::process::Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into())).args(["--print", "sysroot"]).output().expect("cannot run rustc");
    let rustlib = PathBuf::from(String::from_utf8_lossy(&sysroot.stdout).trim()).join("lib/rustlib");
    let lld = std::fs::read_dir(&rustlib).unwrap().filter_map(|e| e.ok()).map(|e| e.path().join("bin/rust-lld")).find(|p| p.is_file()).expect("no rust-lld in the Rust toolchain");
    let st = std::process::Command::new(&lld).args(["-flavor", "link", "/lib", "/machine:x64"]).arg(format!("/def:{}", def.display())).arg(format!("/out:{}", lib.display())).status().unwrap();
    assert!(st.success(), "lld-link /lib failed");
    let bytes = std::fs::read(&lib).unwrap();
    assert!(bytes.windows(4).any(|w| w == [0, 0, 0xff, 0xff]), "no short import member in the library");
    let src = "declare function GetTickCount64(): u64;
declare function lstrlenA(s: string): i32;
declare function MulDiv(a: i32, b: i32, c: i32): i32;
declare function strlen(s: string): u64;
declare function StrToIntA(s: string): i32;
declare function PathIsRelativeA(s: string): i32;
declare function ChrCmpIA(a: u16, b: u16): i32;

action main(args: string[k]): i32 {
    print(GetTickCount64() > 0u64);
    print(lstrlenA(\"hello, MSVC\"));
    print(MulDiv(1000, 3, 7));
    print(strlen(\"été\"));
    print(StrToIntA(\"1234\"));
    print([PathIsRelativeA(\"dir\\\\file.txt\"), PathIsRelativeA(\"C:\\\\file.txt\")]);
    print([ChrCmpIA(97u16, 65u16) == 0, ChrCmpIA(97u16, 66u16) == 0]);
    return 0;
}
";
    let o = isc_codegen::Options { windows: true, ..isc_codegen::Options::default() };
    let p = iscc::frontend("win_msvc_c.isc", src).unwrap_or_else(|e| panic!("{}", e));
    let exe = tmp("win_msvc_c.exe");
    let opts = iscc::BuildOptions { output: exe.clone(), emit_llvm: None, emit_asm: None, debug: false, codegen: o.clone(), link: vec![lib.to_string_lossy().to_string()], glibc: false };
    iscc::build(&p, &opts).unwrap_or_else(|e| panic!("win_msvc_c: {}", e));
    // without the library, the functions of shlwapi are not found
    let without = iscc::BuildOptions { output: tmp("win_msvc_c_without.exe"), emit_llvm: None, emit_asm: None, debug: false, codegen: o, link: Vec::new(), glibc: false };
    assert!(iscc::build(&p, &without).is_err(), "shlwapi linked without its library");
    let w = run_windows(&runner, &exe, &d, &[], None, 1);
    assert_eq!(w.0, 0, "stderr: {}", String::from_utf8_lossy(&w.2));
    assert_eq!(String::from_utf8_lossy(&w.1), "true\n11\n429\n5\n1234\n[1, 0]\n[true, false]\n");
}

/// A server of DNS of this machine at `addr` (that the virtual machine
/// reaches), for the time of the test: `example.test` has the records A
/// 10.1.2.3 and AAAA 2001:db8::7, every other name none (NXDOMAIN), as the
/// one of `std_net.rs`. Its port.
fn dns_server_at(addr: &str) -> u16 {
    let s = std::net::UdpSocket::bind((addr, 0)).unwrap();
    let port = s.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = s.recv_from(&mut buf) {
            let q = &buf[..n];
            let mut at = 12;
            let mut labels = Vec::new();
            while at < n && q[at] != 0 {
                let len = q[at] as usize;
                labels.push(String::from_utf8_lossy(&q[at + 1..at + 1 + len]).to_string());
                at += 1 + len;
            }
            let end = at + 5;
            let kind = u16::from_be_bytes([q[at + 1], q[at + 2]]);
            let name = labels.join(".");
            let data: Option<Vec<u8>> = match (name.as_str(), kind) {
                ("example.test", 1) => Some(vec![10, 1, 2, 3]),
                ("example.test", 28) => Some("2001:db8::7".parse::<std::net::Ipv6Addr>().unwrap().octets().to_vec()),
                _ => None,
            };
            let mut r = q[..2].to_vec();
            let known = name == "example.test";
            r.extend_from_slice(&[0x81, if known { 0x80 } else { 0x83 }, 0, 1, 0, data.is_some() as u8, 0, 0, 0, 0]);
            r.extend_from_slice(&q[12..end]);
            if let Some(d) = data {
                r.extend_from_slice(&[0xC0, 0x0C]);
                r.extend_from_slice(&kind.to_be_bytes());
                r.extend_from_slice(&[0, 1, 0, 0, 0, 60]);
                r.extend_from_slice(&(d.len() as u16).to_be_bytes());
                r.extend_from_slice(&d);
            }
            let _ = s.send_to(&r, from);
        }
    });
    port
}

/// `net.lookup` on Windows (D439): the queries of DNS by UDP of the runtime
/// of Windows (`sysUdpQuery`, Winsock) to a server of this machine at its
/// address that the virtual machine reaches (`ISC_WINDOWS_HOST_ADDR`, by
/// default 172.17.0.1): the records A and AAAA of a name, a name without
/// any, a port where no server answers (the delay); the same outputs as on
/// Linux.
#[test]
fn lookup_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let addr = std::env::var("ISC_WINDOWS_HOST_ADDR").unwrap_or_else(|_| "172.17.0.1".into());
    let port = dns_server_at(&addr);
    let silent = std::net::UdpSocket::bind((addr.as_str(), 0)).unwrap();
    let quiet = silent.local_addr().unwrap().port();
    let src = "import * as net from \"net\";
import { message } from \"sys\";

action main(args: string[k]): i32 {
    print(net.lookup(\"example.test\", args[0], parseI64(args[1])));
    match (net.lookup(\"missing.test\", args[0], parseI64(args[1]))) {
        Ok(xs) => print(xs),
        Err(e) => print(message(e)),
    };
    match (net.lookup(\"example.test\", args[0], parseI64(args[2]))) {
        Ok(xs) => print(xs),
        Err(e) => print(message(e).startsWith(\"timed out\")),
    };
    return 0;
}
";
    let d = dir("lookup");
    let (sp, sq) = (port.to_string(), quiet.to_string());
    let out = same_on_windows(&runner, src, "lookup", &d, &[addr.as_str(), &sp, &sq], None);
    let text = String::from_utf8_lossy(&out).to_string();
    assert!(text.starts_with("Ok([10.1.2.3, 2001:db8::7])\n"), "{}", text);
    assert!(text.ends_with("true\n"), "{}", text);
    drop(silent);
}

/// Every program of these tests and the ten of the Benchmarks Game are
/// built for both targets of Windows here, without the machine of Windows:
/// a program that LLVM cannot compile for Windows is seen by the suite, not
/// only in that machine. (`indexOf` written in Isocron, D596, made LLVM
/// stop on every program that used it, D621: a tail call to a function
/// taking more arguments on the stack than its caller.)
#[test]
fn programs_build_for_windows() {
    if std::env::var_os("ISC_MINGW_DIR").is_none() && !std::path::Path::new("/usr/x86_64-w64-mingw32/lib").is_dir() {
        eprintln!("no MinGW-w64: the builds for Windows skipped");
        return;
    }
    // (the allocator of Linux changes nothing to a program for Windows: the
    // same builds in the three passes of the suite, made in its first)
    if std::env::var_os("ISC_ALLOC").is_some() {
        return;
    }
    let data = format!("{}/tests/data", env!("CARGO_MANIFEST_DIR"));
    let mut sources: Vec<(String, String)> = std::fs::read_dir(&data)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "isc"))
        // (those of the kernel name what only the prelude can: `kernel.rs`)
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("kernel_")))
        .map(|p| (p.file_stem().unwrap().to_string_lossy().to_string(), std::fs::read_to_string(&p).unwrap()))
        .collect();
    for name in ["binarytrees", "fannkuchredux", "fasta", "knucleotide", "mandelbrot", "nbody", "pidigits", "regexredux", "revcomp", "spectralnorm"] {
        sources.push((name.to_string(), source(name)));
    }
    sources.push(("search".to_string(), "action main(args: string[k]): i32 {\n    const t = \"a long text to look into, with a pattern near its end: abcabd\";\n    print([t.indexOf(args.length > 0 ? \"abcabd\" : \"x\"), t.includes(\"pattern near\") ? 1 : 0, t.lastIndexOf(\"a\")]);\n    return 0;\n}\n".to_string()));
    sources.sort();
    assert!(sources.len() > 20, "{} programs", sources.len());
    for (name, src) in &sources {
        // a program that reaches the module `unix` (Linux only, D632):
        // refused for Windows, the primitive named
        let p = iscc::frontend(&format!("{}.isc", name), src).unwrap_or_else(|e| panic!("{}", e));
        if p.sys_paths[1].iter().any(|(op, _, _)| !op.on_windows()) {
            let o = isc_codegen::Options { windows: true, ..isc_codegen::Options::default() };
            let opts = iscc::BuildOptions { output: tmp(&format!("built_{}.exe", name)), emit_llvm: None, emit_asm: None, debug: false, codegen: o, link: Vec::new(), glibc: false };
            let e = iscc::build(&p, &opts).expect_err("a program of the module unix built for Windows");
            assert!(e.contains("the target Windows has no `sys") && e.contains("(the module `unix` is Linux only)"), "{}: {}", name, e);
            continue;
        }
        for msvc in [false, true] {
            let exe = compile_windows_with(src, &format!("built_{}_{}", name, if msvc { "msvc" } else { "gnu" }), msvc);
            let head = std::fs::read(&exe).unwrap();
            assert!(head.starts_with(b"MZ"), "{} (msvc {}): not an executable of Windows", name, msvc);
        }
    }
}

/// D636: the module `win32` in the machine of Windows, run in the session on
/// its screen (`run.py --interactive`): a window of 320 by 200 pixels, an
/// image shown (fewer pixels than its size: refused), input played by
/// messages posted to it (the pointer, a click, a key and the character
/// Windows makes of it, a character, closing asked) read back in their
/// order as events (the character of the key in the same call, counted
/// apart: the thread of the window makes it when it reads the key, its place
/// among the messages posted meanwhile that thread's, D641, D645), a wait
/// without events that ends after its delay, what
/// the window shows captured: the image drawn, pixel for pixel; built with
/// both ABIs of Windows.
#[test]
fn a_window_of_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let src = include_str!("data/std_win32.isc");
    let want = "(320, 200)\nOk(())\nErr(std:sys.Other(22, show))\nOk(())\nOk(())\nOk(())\nOk(())\nOk(())\nOk(())\n[std:win32.Pointer(10, 20), std:win32.Pointer(11, 21), std:win32.Button(0, true), std:win32.Pointer(11, 21), std:win32.Button(0, false), std:win32.Key(65, true, 30), std:win32.Char(233), std:win32.Close]\n1\n0\n(64000, 64000)\n";
    for msvc in [false, true] {
        let exe = compile_windows_with(src, &format!("std_win32_{}", if msvc { "msvc" } else { "gnu" }), msvc);
        let d = dir("win32");
        let o = std::process::Command::new(&runner).arg("--dir").arg(&d).arg("--interactive").arg(&exe).output().unwrap();
        assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n")), (Some(0), want.to_string()), "msvc {}: {}", msvc, String::from_utf8_lossy(&o.stderr));
    }
}

/// D636, D324: the module `win32` is Windows only: a program that reaches it
/// is refused when built for Linux and for WebAssembly, the path of its
/// calls named; in a branch for Windows (D635), it builds for them.
#[test]
fn win32_refused_for_linux_and_wasi() {
    let src = "import * as win from \"win32\";\n\naction shown(): void {\n    match (win.open(\"x\", 320, 200)) {\n        Ok(w) => win.close(w),\n        Err(e) => print(0),\n    };\n}\n\naction main(args: string[k]): i32 {\n    if (args.length > 99) {\n        shown();\n    }\n    print(1);\n    return 0;\n}\n";
    let p = iscc::frontend("w.isc", src).unwrap_or_else(|e| panic!("{}", e));
    for (target, codegen) in [
        ("Linux", isc_codegen::Options::default()),
        ("WebAssembly (WASI preview 1)", isc_codegen::Options { isa: isc_codegen::Isa::Wasm32, ..isc_codegen::Options::default() }),
    ] {
        let opts = iscc::BuildOptions { output: tmp("win32_other"), emit_llvm: None, emit_asm: None, debug: false, codegen, link: Vec::new(), glibc: false };
        let e = iscc::build(&p, &opts).expect_err("the module win32 built for another system");
        assert!(e.contains(&format!("the target {} has no `sysWinOpen`", target)) && e.contains("reached by `main` -> `shown` -> `win32.open`"), "{}: {}", target, e);
    }
    let branch = src.replace("    if (args.length > 99) {", "    if (sys.platform() == \"windows\" && args.length > 99) {").replace("import * as win from \"win32\";\n", "import * as win from \"win32\";\nimport * as sys from \"sys\";\n");
    assert_eq!(interp_ok(&branch, &[]), "1\n");
    let r = run_bin(&compile(&branch, "win32_branch_linux"), &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "1\n"));
}

/// D640: `iscc build --subsystem windows`: a program of Windows of the
/// graphical subsystem, started without a console (a window of `win32`
/// launched from the explorer); the console subsystem by default. The
/// field `Subsystem` of its PE header (2: windows, 3: console), with the
/// start of the runtime and with MinGW's C runtime (a C function declared).
#[test]
fn a_program_of_windows_without_a_console() {
    let alone = "action main(args: string[k]): i32 {\n    print(1);\n    return 0;\n}\n";
    let with_c = "declare function abs(v: i32): i32;\n\naction main(args: string[k]): i32 {\n    print(abs(-1i32));\n    return 0;\n}\n";
    for (name, src) in [("alone", alone), ("with_c", with_c)] {
        let p = iscc::frontend(&format!("{}.isc", name), src).unwrap_or_else(|e| panic!("{}", e));
        for gui in [false, true] {
            let out = tmp(&format!("subsystem_{}_{}.exe", name, gui));
            let codegen = isc_codegen::Options { windows: true, windows_gui: gui, ..isc_codegen::Options::default() };
            let opts = iscc::BuildOptions { output: out.clone(), codegen, ..Default::default() };
            iscc::build(&p, &opts).unwrap_or_else(|e| panic!("{} {}: {}", name, gui, e));
            let b = std::fs::read(&out).unwrap();
            let pe = u32::from_le_bytes(b[0x3c..0x40].try_into().unwrap()) as usize;
            assert_eq!(&b[pe..pe + 4], b"PE\0\0");
            // the optional header (PE32+) after the 20 bytes of the COFF one
            assert_eq!(u16::from_le_bytes(b[pe + 24..pe + 26].try_into().unwrap()), 0x20b);
            let subsystem = u16::from_le_bytes(b[pe + 24 + 68..pe + 24 + 70].try_into().unwrap());
            assert_eq!(subsystem, if gui { 2 } else { 3 }, "{} {}", name, gui);
        }
    }
}

/// D643: `win32.programPath()`, the file of the program running
/// (`GetModuleFileNameW`, in UTF-8): its name, a path of a drive; and
/// `win32.alert` built and linked (`MessageBoxW` of user32; a box waits for
/// a hand: shown by the check of the library in the machine of Windows, not
/// here), in a branch for Windows: the program built for Linux too.
#[test]
fn the_path_of_the_program_on_windows() {
    let src = "import * as win from \"win32\";
import * as sys from \"sys\";

action main(args: string[k]): i32 {
    if (sys.platform() == \"windows\") {
        if (args.length > 0) {
            win.alert(\"Isocron\", args[0]);
        }
        match (win.programPath()) {
            Ok(p) => print([p.slice(p.lastIndexOf(\"\\\\\") + 1, p.length), p.indexOf(\":\\\\\") == 1] as const),
            Err(e) => print(sys.message(e)),
        };
    } else {
        print(0);
    }
    return 0;
}
";
    let r = run_bin(&compile(src, "program_path_linux"), &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "0\n"));
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    for msvc in [false, true] {
        let name = format!("program_path_{}", if msvc { "msvc" } else { "gnu" });
        let exe = compile_windows_with(src, &name, msvc);
        let d = dir(&name);
        let (code, out, err) = run_windows(&runner, &exe, &d, &[], None, 1);
        assert_eq!((code, String::from_utf8_lossy(&out).replace("\r\n", "\n")), (0, format!("({}.exe, true)\n", name)), "{}", String::from_utf8_lossy(&err));
    }
}

/// D646, D649: the density of the screen (96 dots per inch in the machine
/// of Windows, at 100 %), the shape of the pointer and the clipboard of
/// `win32`, in the session of the machine (`run.py --interactive`): a text with
/// an emoji put on the clipboard and read back, an empty text; the shapes
/// of the pointer set (a hand, a bar, an arrow) without failing; with both
/// ABIs.
#[test]
fn the_clipboard_and_the_pointer_on_windows() {
    let Some(runner) = runner() else {
        eprintln!("no Windows machine (ISC_WINDOWS_RUNNER): the Windows target not run");
        return;
    };
    let src = "import * as win from \"win32\";
import { message } from \"sys\";

action main(args: string[k]): i32 {
    match (win.open(\"clip\", 320, 200)) {
        Ok(w) => {
            const s = win.show(w, fill(320 * 200, 0xFF808080u32), 320, 200);
            const opening = win.events(w, 200);
            print(win.dpi(w));
            win.setCursor(w, 1);
            print(win.clipboardSet(w, \"Café 😀 du presse-papiers\"));
            print(win.clipboardGet(w));
            print(win.clipboardSet(w, \"\"));
            print(win.clipboardGet(w));
            win.setCursor(w, 2);
            win.setCursor(w, 0);
            win.close(w);
        },
        Err(e) => print(message(e)),
    };
    return 0;
}
";
    for msvc in [false, true] {
        let exe = compile_windows_with(src, &format!("clip_{}", if msvc { "msvc" } else { "gnu" }), msvc);
        let d = dir("clip");
        let o = std::process::Command::new(&runner).arg("--dir").arg(&d).arg("--interactive").arg(&exe).output().unwrap();
        assert_eq!(
            (o.status.code(), String::from_utf8_lossy(&o.stdout).replace("\r\n", "\n")),
            (Some(0), "96\nOk(())\nOk(Café 😀 du presse-papiers)\nOk(())\nOk()\n".to_string()),
            "msvc {}: {}",
            msvc,
            String::from_utf8_lossy(&o.stderr)
        );
    }
}
