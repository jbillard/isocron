//! The module `https` of the standard library (SPEC section 18, D343): HTTP
//! over TLS by OpenSSL loaded at run time, on the local machine, with
//! certificates made by the test (`openssl req`: an authority, a server
//! for `localhost`, another authority): the client against `openssl
//! s_server`, the server against `curl` and against the client of the
//! module; the same outputs in the interpreter and in the compiled program;
//! the programs linked with glibc without OpenSSL among their libraries;
//! WebAssembly refused.

mod common;
use common::*;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The certificates of the tests in `dir`: `ca.pem` (an authority),
/// `server.pem` and `server.key` (for `localhost`, signed by it),
/// `other.pem` (another authority); `www/` the files that `s_server`
/// serves. None when there is no `openssl` on the machine.
fn certificates(name: &str) -> Option<PathBuf> {
    let dir = tmp(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("www")).unwrap();
    let run = |args: &[&str]| Command::new("openssl").args(args).current_dir(&dir).output().map(|o| o.status.success()).unwrap_or(false);
    let ec = ["-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes"];
    std::fs::write(dir.join("ext.cnf"), "subjectAltName=DNS:localhost\n").unwrap();
    let ok = run(&[&["req", "-x509"][..], &ec, &["-keyout", "ca.key", "-out", "ca.pem", "-days", "2", "-subj", "/CN=isocron test ca"]].concat())
        && run(&[&["req"][..], &ec, &["-keyout", "server.key", "-out", "server.csr", "-subj", "/CN=localhost"]].concat())
        && run(&["x509", "-req", "-in", "server.csr", "-CA", "ca.pem", "-CAkey", "ca.key", "-CAcreateserial", "-out", "server.pem", "-days", "2", "-extfile", "ext.cnf"])
        && run(&[&["req", "-x509"][..], &ec, &["-keyout", "other.key", "-out", "other.pem", "-days", "2", "-subj", "/CN=another ca"]].concat());
    if !ok {
        eprintln!("no openssl: the tests of https skipped");
        return None;
    }
    std::fs::write(dir.join("www/hello.txt"), "hello over tls\n").unwrap();
    let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(dir.join("www/big.bin"), big).unwrap();
    Some(dir)
}

/// A free port of the local machine.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// `openssl s_server` on `port`, serving the files of `dir/www` (`-WWW`),
/// accepting connections until it is killed.
fn s_server(dir: &Path, port: u16) -> std::process::Child {
    let c = Command::new("openssl")
        .args(["s_server", "-quiet", "-accept", &port.to_string(), "-cert", "../server.pem", "-key", "../server.key", "-WWW"])
        .current_dir(dir.join("www"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // its port open
    for _ in 0..500 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    c
}

fn valgrind_bin() -> Option<PathBuf> {
    let vg = PathBuf::from("/usr/bin/valgrind");
    vg.is_file().then_some(vg)
}

/// D343: the client against `openssl s_server`: a small body and 3000000
/// bytes read, the certificate of another authority refused, a name the
/// certificate does not have refused (`127.0.0.1`), a `ca` that is not one
/// refused, a connection refused, the authorities of the system that do
/// not know the test's; compiled, interpreted and under Valgrind, the same
/// outputs.
#[test]
fn client_against_openssl() {
    let Some(dir) = certificates("std_https_client_certs") else { return };
    let port = free_port();
    let mut server = s_server(&dir, port);
    let src = include_str!("data/std_https_client.isc");
    let big: u64 = (0..3_000_000u64).map(|i| i % 253).sum();
    let want = format!(
        "200 hello over tls\n200 3000000 {}\ntls certificate\ntls certificate\ntls no certificate in the PEM text\nrefused 127.0.0.1:1\ntls certificate\n",
        big
    );
    let (ca, other) = (dir.join("ca.pem"), dir.join("other.pem"));
    let args = [port.to_string(), ca.to_string_lossy().to_string(), other.to_string_lossy().to_string()];
    let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    assert_eq!(interp_ok(src, &args), want, "interpreter");
    let bin = compile(src, "std_https_client");
    let r = run_bin(&bin, &args);
    assert_eq!((r.code, r.stdout.as_str()), (0, want.as_str()), "compiled: {}", r.stderr);
    if let Some(vg) = valgrind_bin() {
        let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
        let vbin = compile_glibc(src, "std_https_client_vg", opts);
        let o = Command::new(vg)
            .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
            .arg(&vbin)
            .args(&args)
            .env("ISC_ALLOC", "glibc")
            .env("ISC_THREADS", "2")
            .output()
            .unwrap();
        assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()), (Some(0), want.clone()), "valgrind:\n{}", String::from_utf8_lossy(&o.stderr));
    }
    let _ = server.kill();
    let _ = server.wait();
    // linked with glibc (TLS is loaded at run time), OpenSSL not among its
    // libraries: a program that does not reach TLS keeps none of it
    let d = Command::new("readelf").arg("-d").arg(&bin).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    if !d.is_empty() {
        assert!(d.contains("libc.so.6") && !d.contains("libssl") && !d.contains("libcrypto"), "{}", d);
    }
    // (the run of the tests with the allocator of glibc links every
    // program with glibc)
    let all_glibc = std::env::var("ISC_ALLOC").map(|v| v == "glibc").unwrap_or(false) || std::env::var("ISC_CC").is_ok();
    if !all_glibc {
        let plain = compile("import * as http from \"http\";\n\naction main(args: string[k]): i32 {\n    print(http.parseUrl(\"https://x/\"));\n    return 0;\n}\n", "std_https_plain");
        let d = Command::new("readelf").arg("-d").arg(&plain).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        assert!(!d.contains("NEEDED"), "a program without TLS: {}", d);
    }
}

/// The server of `data/std_https_server.isc` started by `cmd` (its other
/// arguments after the files of `dir`): its child and its port.
fn start_server(mut cmd: Command, dir: &Path, limit: usize) -> (std::process::Child, u16, std::io::BufReader<std::process::ChildStdout>) {
    cmd.arg(dir.join("server.pem")).arg(dir.join("server.key")).arg(limit.to_string());
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let port: u16 = line.trim().parse().unwrap_or_else(|_| panic!("no port: {:?}", line));
    (child, port, out)
}

/// `curl` with the authority of the test: its output.
fn curl(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("curl").arg("-s").arg("--cacert").arg(dir.join("ca.pem")).args(args).output().unwrap();
    assert!(o.status.success(), "curl {:?}: {}", args, String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).to_string()
}

/// D343: `serveTlsOn` in a program of its own against `curl` (two
/// requests on one connection, a body sent, `Connection: close`, 4
/// clients at once, 2000000 bytes back) and against the client of the
/// module (a POST of 1 MiB sent back); the server ends after the answers
/// it was given; compiled, interpreted (`iscc run`) and under Valgrind.
#[test]
fn server_against_curl_and_the_client() {
    let Some(dir) = certificates("std_https_server_certs") else { return };
    if Command::new("curl").arg("--version").output().is_err() {
        eprintln!("no curl: the test of the server of https skipped");
        return;
    }
    let src = include_str!("data/std_https_server.isc");
    let bin = compile(src, "std_https_server");
    let file = dir.join("server.isc");
    std::fs::write(&file, src).unwrap();
    // the client of the module, for the POST
    let client_src = "import * as https from \"https\";\nimport * as http from \"http\";\n\naction main(args: string[k]): i32 {\n    const body = map(iota(1048576), (i) => u8(i % 249));\n    match (https.requestWith(readText(args[1]), \"POST\", `https://localhost:${args[0]}/echo`, [], body)) {\n        Ok(r) => print(`${r.status} ${r.body.length} ${r.body == body}`),\n        Err(e) => print(http.message(e)),\n    };\n    return 0;\n}\n";
    let client = compile(client_src, "std_https_server_client");
    let mut runs: Vec<(&str, Command)> = Vec::new();
    let mut c = Command::new(&bin);
    c.env("ISC_THREADS", "4");
    runs.push(("compiled", c));
    let mut c = Command::new(env!("CARGO_BIN_EXE_iscc"));
    c.arg("run").arg(&file);
    runs.push(("interpreted", c));
    if let Some(vg) = valgrind_bin() {
        let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
        let vbin = compile_glibc(src, "std_https_server_vg", opts);
        let mut c = Command::new(vg);
        c.args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"]).arg(&vbin).env("ISC_ALLOC", "glibc").env("ISC_THREADS", "2");
        runs.push(("valgrind", c));
    }
    for (name, cmd) in runs {
        // 2 + 1 + 1 + 4 + 1 + 1 answers
        let (mut child, port, mut out) = start_server(cmd, &dir, 10);
        let base = format!("https://localhost:{}", port);
        assert_eq!(curl(&dir, &[&format!("{}/a?x=1", base), &format!("{}/b", base)]), "secure GET /a x=1 0secure GET /b  0", "{}", name);
        assert_eq!(curl(&dir, &["-d", "a body", &format!("{}/c", base)]), "secure POST /c  6", "{}", name);
        assert_eq!(curl(&dir, &["-H", "Connection: close", &format!("{}/d", base)]), "secure GET /d  0", "{}", name);
        let threads: Vec<_> = (0..4)
            .map(|i| {
                let dir = dir.clone();
                let base = base.clone();
                std::thread::spawn(move || curl(&dir, &[&format!("{}/t?{}", base, i)]))
            })
            .collect();
        for (i, t) in threads.into_iter().enumerate() {
            assert_eq!(t.join().unwrap(), format!("secure GET /t {} 0", i), "{}", name);
        }
        let big = Command::new("curl").arg("-s").arg("--cacert").arg(dir.join("ca.pem")).arg(format!("{}/big?2000000", base)).output().unwrap().stdout;
        assert!(big.len() == 2000000 && big.iter().enumerate().all(|(i, &x)| x == (i % 251) as u8), "{}: {} bytes", name, big.len());
        let r = run_bin(&client, &[&port.to_string(), &dir.join("ca.pem").to_string_lossy()]);
        assert_eq!(r.stdout, "200 1048576 true\n", "{}: {}", name, r.stderr);
        let o = child.wait_with_output().unwrap();
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut out, &mut rest).unwrap();
        assert_eq!((o.status.code(), rest.as_str()), (Some(0), "done\n"), "{}: {}", name, String::from_utf8_lossy(&o.stderr));
    }
    // a certificate that is not one: the server refuses to start
    let l = interp_ok("import * as https from \"https\";\nimport * as http from \"http\";\nimport * as net from \"net\";\nimport { message } from \"sys\";\n\nfunction h(r: http.Request): http.Response {\n    return http.textResponse(200, \"\");\n}\n\naction main(args: string[k]): i32 {\n    match (net.listen(\"127.0.0.1\", 0)) {\n        Ok(l) => match (https.serveTlsOn(l, \"no certificate\", \"no key\", h, 1)) {\n            Ok(_) => print(\"served\"),\n            Err(e) => print(message(e).startsWith(\"tls: \")),\n        },\n        Err(e) => print(\"listen failed\"),\n    };\n    return 0;\n}\n", &[]);
    assert_eq!(l, "true\n");
    let _ = std::io::stdout().flush();
}

/// D343: a program that reaches TLS is refused when built for WebAssembly
/// (WASI preview 1 has no sockets), the path of its calls named.
#[test]
fn https_refused_for_wasi() {
    let src = "import * as https from \"https\";\nimport * as http from \"http\";\n\naction main(args: string[k]): i32 {\n    match (https.get(\"https://localhost:1/\")) {\n        Ok(r) => print(r.status),\n        Err(e) => print(http.message(e)),\n    };\n    return 0;\n}\n";
    let p = iscc::frontend("https.isc", src).unwrap();
    let o = isc_codegen::Options { isa: isc_codegen::Isa::Wasm32, ..isc_codegen::Options::default() };
    let opts = iscc::BuildOptions { output: tmp("https_wasi.wasm"), emit_llvm: None, emit_asm: None, debug: false, codegen: o, link: Vec::new(), glibc: false };
    let e = iscc::build(&p, &opts).expect_err("TLS built for WASI");
    assert!(e.contains("(WASI preview 1) has no") && e.contains("main"), "{}", e);
}
