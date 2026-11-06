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
    let o = isc_codegen::Options { windows: true, ..isc_codegen::Options::default() };
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
    let bin = compile(src, &format!("winlinux_{}", name));
    let exe = compile_windows(src, &format!("win_{}", name));
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
    same_on_windows(&runner, BASICS, "basics", &d, &["one", "two words", ""], None);
    let d = dir("files");
    std::fs::write(d.join("big.txt"), "x".repeat(9 << 20)).unwrap();
    std::fs::write(d.join("in.txt"), "a\nb\nc\n").unwrap();
    same_on_windows(&runner, FILES, "files", &d, &["big.txt"], Some("in.txt"));
    let d = dir("panics");
    same_on_windows(&runner, PANICS, "panics", &d, &[], None);
    let d = dir("overflow");
    same_on_windows(&runner, OVERFLOW, "overflow", &d, &[], None);
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
    // the two servers, on free ports of that address
    let port = || std::net::TcpListener::bind((addr.as_str(), 0)).unwrap().local_addr().unwrap().port();
    let (pa, pb) = (port(), port());
    let mut servers = Vec::new();
    for (p, n) in [(pa, "ip"), (pb, "named")] {
        servers.push(
            std::process::Command::new("openssl")
                .args(["s_server", "-quiet", "-accept", &format!("{}:{}", addr, p), "-cert", &format!("../{}.pem", n), "-key", &format!("../{}.key", n), "-WWW"])
                .current_dir(certs.join("www"))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        while std::net::TcpStream::connect((addr.as_str(), p)).is_err() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    let big: u64 = (0..3_000_000u64).map(|i| i % 253).sum();
    let want = format!("200 hello over tls\n200 3000000 {}\ntls certificate\ntls certificate\ntls no certificate in the PEM text\ntls certificate\nrefused\n", big);
    let (sa, sb) = (pa.to_string(), pb.to_string());
    let args = [addr.as_str(), &sa, &sb, "ca.pem", "other.pem"];
    let out = same_on_windows(&runner, include_str!("data/std_https_remote_client.isc"), "https_client", &d, &args, None);
    for mut s in servers {
        let _ = s.kill();
        let _ = s.wait();
    }
    assert_eq!(String::from_utf8_lossy(&out), want);
}

/// D343: the server of HTTPS on Windows (SChannel), its client too, in
/// the real VM: the pair of programs of `data/std_https_pair_*.isc` (a
/// script of `cmd` starts the server, then the client), compiled, then
/// interpreted by `$ISC_WINDOWS_ISCC` when it is given; certificates made
/// by the test (EC); the same outputs as the pair on Linux.
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
    for (name, s, c) in ways {
        // the server in the background, its output in a file (written at
        // its end), the client when it listens
        let script = format!(
            "@echo off\r\nstart \"\" /b cmd /c \"{} 47970 server.pem server.key 3 > server.txt 2>&1\"\r\nping -n 5 127.0.0.1 > nul\r\n{} 47970 ca.pem other.pem\r\nping -n 3 127.0.0.1 > nul\r\ntype server.txt\r\nexit /b 0\r\n",
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
