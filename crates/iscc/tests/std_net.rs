//! The module `net` of the standard library (SPEC section 18, D339):
//! connections of TCP on the local machine, the resolver of names asking a
//! server of DNS written here, the connections and listeners linear (closed
//! exactly once, checked at compile time), WebAssembly refused; the same
//! outputs in the interpreter and in the compiled program.

mod common;
use common::*;
use std::net::UdpSocket;

const SRC: &str = include_str!("data/std_net.isc");

/// A server of DNS on the local machine, for a while: `example.test` has
/// the record A 10.1.2.3 and AAAA 2001:db8::7, every other name none
/// (NXDOMAIN). Its port.
fn dns_server() -> u16 {
    let s = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = s.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = s.recv_from(&mut buf) {
            let q = &buf[..n];
            // the name of the question, its end, its type
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

/// D339: a listener on a port that the system chooses, a connection to it
/// by `localhost`, bytes both ways (70000 of them back, read to the end of
/// the stream), a read past its delay (`TimedOut`), a connection refused
/// once the listener is closed; the addresses of `localhost` and of a
/// numeric one; the records A and AAAA of a name asked to the server of
/// DNS of the test, and a name it does not have (`NotFound`); compiled
/// (1 and 4 threads) and interpreted, the same outputs; Valgrind.
#[test]
fn tcp_and_dns_on_the_local_machine() {
    let port = dns_server().to_string();
    let want = "true\nOk(())\ntrue\nhello\nOk(())\n(70000, 8746781)\nOk(())\ntimed out: read\ntrue\nOk([127.0.0.1, ::1])\nOk([::1])\nOk([10.1.2.3, 2001:db8::7])\nnot found: missing.test\n";
    assert_eq!(interp_ok(SRC, &[&port]), want, "interpreter");
    let bin = compile(SRC, "std_net");
    for t in [1, 4] {
        let r = run_bin_threads(&bin, &[&port], Some(t));
        assert_eq!((r.code, r.stdout.as_str()), (0, want), "compiled, {} threads: {}", t, r.stderr);
    }
    // nothing lost (Valgrind, the allocator of glibc)
    let vg = valgrind_path();
    if let Some(vg) = vg {
        let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
        let bin = compile_glibc(SRC, "std_net_vg", opts);
        let out = std::process::Command::new(vg)
            .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
            .arg(&bin)
            .arg(&port)
            .env("ISC_ALLOC", "glibc")
            .env("ISC_THREADS", "2")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "under valgrind:\n{}", String::from_utf8_lossy(&out.stderr));
    }
}

/// D339: a connection or a listener is closed (or given away) exactly once
/// on every path: a use after `close`, one never closed, one closed on one
/// path only, one left by `_`, one captured by a lambda, one made and lost,
/// a use after it was given to a function that closes it: each refused at
/// compile time, with its place.
#[test]
fn connections_are_closed_exactly_once() {
    let cases: &[(&str, &str)] = &[
        ("net.close(c);\n    print(net.writeText(c, \"x\"));", "`c` is used after it was closed or given away at line"),
        ("print(net.writeText(c, \"x\"));", "`c` is never closed"),
        ("if (args.length > 0) {\n        net.close(c);\n    }", "`c` is closed or given away on one path"),
        ("const f = (x: i64) => [c, x] as const;\n    net.close(c);", "a lambda cannot capture it"),
        ("net.close(c);\n    match (net.connect(\"127.0.0.1\", 1)) {\n        Ok(_) => print(1),\n        Err(e) => print(2),\n    };", "left by `_`"),
        ("net.close(c);\n    net.connect(\"127.0.0.1\", 1);", "never closed: bind it"),
        ("finish(c);\n    net.close(c);", "`c` is used after it was closed or given away at line"),
    ];
    for (body, want) in cases {
        let src = format!(
            "import * as net from \"net\";\n\naction finish(x: net.Conn): void {{\n    net.close(x);\n}}\n\naction use(c: net.Conn, l: net.Listener, args: string[k]): void {{\n    {}\n}}\n\naction main(args: string[k]): i32 {{\n    match (net.listen(\"127.0.0.1\", 0)) {{\n        Ok(l) => {{\n            match (net.connect(\"127.0.0.1\", net.port(l))) {{\n                Ok(c) => use(c, l, args),\n                Err(e) => print(0),\n            }};\n            net.closeListener(l);\n        }},\n        Err(e) => print(0),\n    }};\n    return 0;\n}}\n",
            body
        );
        let e = match iscc::frontend("bad.isc", &src) {
            Ok(_) => panic!("accepted: {}", body),
            Err(e) => format!("{}", e),
        };
        assert!(e.contains(want) && e.contains("bad.isc:"), "{}: {}", body, e);
    }
    // a function that only reads its connection: the caller closes it
    let ok = "import * as net from \"net\";\n\naction greet(c: net.Conn): void {\n    print(net.writeText(c, \"hi\"));\n}\n\naction main(args: string[k]): i32 {\n    match (net.listen(\"127.0.0.1\", 0)) {\n        Ok(l) => {\n            match (net.connect(\"127.0.0.1\", net.port(l))) {\n                Ok(c) => {\n                    greet(c);\n                    net.close(c);\n                },\n                Err(e) => print(0),\n            };\n            net.closeListener(l);\n        },\n        Err(e) => print(0),\n    };\n    return 0;\n}\n";
    assert_eq!(interp_ok(ok, &[]), "Ok(())\n");
}

/// D324, D339: WASI preview 1 has no sockets: a program that reaches one
/// is refused when built for WebAssembly, the path of its calls named; the
/// same program builds for Linux.
#[test]
fn net_refused_for_wasi() {
    let src = "import * as net from \"net\";\n\naction ask(port: i64): void {\n    match (net.connect(\"127.0.0.1\", port)) {\n        Ok(c) => net.close(c),\n        Err(e) => print(0),\n    };\n}\n\naction main(args: string[k]): i32 {\n    if (args.length > 99) {\n        ask(80);\n    }\n    print(1);\n    return 0;\n}\n";
    let p = iscc::frontend("wasi.isc", src).unwrap_or_else(|e| panic!("{}", e));
    let out = tmp("net_wasi");
    let opts = iscc::BuildOptions {
        output: out.clone(),
        emit_llvm: None,
        emit_asm: None,
        debug: false,
        codegen: isc_codegen::Options { isa: isc_codegen::Isa::Wasm32, ..isc_codegen::Options::default() },
        link: Vec::new(),
        glibc: false,
    };
    let e = iscc::build(&p, &opts).expect_err("a program with sockets built for WASI");
    assert!(e.contains("(WASI preview 1) has no `sys") && e.contains("(no sockets): reached by `main` -> `ask` -> `net."), "{}", e);
    let r = run_bin(&compile(src, "net_wasi_linux"), &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "1\n"));
}

/// D348: a port listened to twice: the second `listen` is
/// `AlreadyExists`, in the interpreter and compiled.
#[test]
fn a_port_listened_to_twice() {
    let src = include_str!("data/std_net_twice.isc");
    assert_eq!(interp_ok(src, &[]), "refused: already exists\n", "interpreter");
    let bin = compile(src, "std_net_twice");
    let r = run_bin(&bin, &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "refused: already exists\n"), "compiled: {}", r.stderr);
}
