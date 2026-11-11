//! The module `unix` of the standard library (D632): Unix sockets in one
//! program (a name of the abstract space), a memory shared whose descriptor
//! is sent with a byte and projected on the other side, the wait for a
//! descriptor; its primitives calls of the system made by the compiled
//! program itself; a `Shared` linear; Windows and WebAssembly refused; the
//! same outputs in the interpreter and in the compiled program.

mod common;
use common::*;

const SRC: &str = include_str!("data/std_unix.isc");

const WANT: &str = "Ok(())\nhello\n8192\nOk(())\nOk(())\nErr(std:sys.Other(22, writeShared))\nOk(())\nM\n1\nOk([112, 105, 120, 101, 108, 115])\nOk([68, 51, 34, 17, 0, 255, 0, 255])\nErr(std:sys.PermissionDenied(writeShared))\nErr(std:sys.Other(22, readShared))\nOk(())\n[0]\nOk(())\n(true, false, false)\nping\n(true, true)\n0\nrefused: @isocron-nothing-listens-here\nerror 22 of the system: /tmp/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\nerror 22 of the system: shared\nErr(std:sys.Other(9, closeFd))\n";

/// A name of this run for the socket of the program (two runs at the same
/// time never share it).
fn name(what: &str) -> String {
    format!("{}-{}", std::process::id(), what)
}

/// D632: a Unix socket listened to, connected and accepted; bytes both
/// ways; a memory shared of 8192 bytes written (bytes, words of 32 bits, a
/// write past its end refused), its descriptor sent with one byte, received,
/// projected read only on the other side (its bytes, a write refused, a
/// read past its end refused); `poll`: nothing within 20 ms, then bytes,
/// then the peer gone; a name where nothing listens, a path too long, an
/// empty memory, a descriptor that is none. Interpreted and compiled (1 and
/// 4 threads), the same outputs; the compiled program makes its calls of
/// the system itself (no function of the runtime); Valgrind.
#[test]
fn sockets_descriptors_and_shared_memory() {
    assert_eq!(interp_ok(SRC, &[&name("interp")]), WANT, "interpreter");
    let bin = compile(SRC, "std_unix");
    for t in [1, 4] {
        let r = run_bin_threads(&bin, &[&name(&format!("t{}", t))], Some(t));
        assert_eq!((r.code, r.stdout.as_str()), (0, WANT), "compiled, {} threads: {}", t, r.stderr);
    }
    // the calls of the system in the program: no function of the runtime
    let p = iscc::frontend("std_unix.isc", SRC).unwrap_or_else(|e| panic!("{}", e));
    let ir = isc_codegen::emit(&p).unwrap();
    assert!(ir.contains("asm sideeffect \"syscall\"") && ir.contains("@isc.unix.recvfds(") && !ir.contains("@isc_sys_unix"), "the calls of the system emitted");
    // nothing lost (Valgrind, the allocator of glibc)
    if let Some(vg) = valgrind_path() {
        let opts = isc_codegen::Options { isa: isc_codegen::Isa::Avx2, ..isc_codegen::Options::default() };
        let bin = compile_glibc(SRC, "std_unix_vg", opts);
        let out = std::process::Command::new(vg)
            .args(["--error-exitcode=9", "--leak-check=full", "--errors-for-leak-kinds=definite,indirect", "-q"])
            .arg(&bin)
            .arg(name("vg"))
            .env("ISC_ALLOC", "glibc")
            .env("ISC_THREADS", "2")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "under valgrind:\n{}", String::from_utf8_lossy(&out.stderr));
    }
}

/// D632, D339: a `Shared` is closed exactly once on every path, as a
/// connection: a use after `closeShared`, one never closed, one closed on
/// one path only, one made and lost: each refused at compile time.
#[test]
fn a_shared_memory_is_closed_exactly_once() {
    let cases: &[(&str, &str)] = &[
        ("unix.closeShared(m);\n    print(unix.sharedSize(m));", "`m` is used after it was closed or given away at line"),
        ("print(unix.sharedSize(m));", "`m` is never closed"),
        ("if (args.length > 0) {\n        unix.closeShared(m);\n    }", "`m` is closed or given away on one path"),
        ("unix.closeShared(m);\n    unix.shared(16);", "never closed: bind it"),
    ];
    for (body, want) in cases {
        let src = format!(
            "import * as unix from \"unix\";\n\naction use(m: unix.Shared, args: string[k]): void {{\n    {}\n}}\n\naction main(args: string[k]): i32 {{\n    match (unix.shared(16)) {{\n        Ok(m) => use(m, args),\n        Err(e) => print(0),\n    }};\n    return 0;\n}}\n",
            body
        );
        let e = match iscc::frontend("bad.isc", &src) {
            Ok(_) => panic!("accepted: {}", body),
            Err(e) => format!("{}", e),
        };
        assert!(e.contains(want) && e.contains("bad.isc:"), "{}: {}", body, e);
    }
    // a program calls the functions of the module, never its primitives
    let direct = "action main(args: string[k]): i32 {\n    const [m, code] = sysSharedOpen(16);\n    print(code);\n    return 0;\n}\n";
    let e = iscc::frontend("direct.isc", direct).map(|_| ()).expect_err("a primitive called by a program");
    assert!(format!("{}", e).contains("is a primitive of the standard library"), "{}", e);
}

/// D632, D324: the module `unix` is Linux only: a program that reaches it
/// is refused when built for Windows and for WebAssembly, the path of its
/// calls named; the same program builds for Linux.
#[test]
fn unix_refused_for_windows_and_wasi() {
    let src = "import * as unix from \"unix\";\nimport * as net from \"net\";\n\naction ask(path: string): void {\n    match (unix.connect(path)) {\n        Ok(c) => net.close(c),\n        Err(e) => print(0),\n    };\n}\n\naction main(args: string[k]): i32 {\n    if (args.length > 99) {\n        ask(\"@x\");\n    }\n    print(1);\n    return 0;\n}\n";
    let p = iscc::frontend("unix.isc", src).unwrap_or_else(|e| panic!("{}", e));
    for (target, codegen) in [
        ("Windows", isc_codegen::Options { windows: true, ..isc_codegen::Options::default() }),
        ("WebAssembly (WASI preview 1)", isc_codegen::Options { isa: isc_codegen::Isa::Wasm32, ..isc_codegen::Options::default() }),
    ] {
        let opts = iscc::BuildOptions { output: tmp("unix_other"), emit_llvm: None, emit_asm: None, debug: false, codegen, link: Vec::new(), glibc: false };
        let e = iscc::build(&p, &opts).expect_err("the module unix built for another system");
        assert!(
            e.contains(&format!("the target {} has no `sysUnixConnect` (the module `unix` is Linux only): reached by `main` -> `ask` -> `unix.connect`", target)),
            "{}: {}",
            target,
            e
        );
    }
    let r = run_bin(&compile(src, "unix_linux"), &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "1\n"));
}

/// D635: a program for several systems: the module `unix` called in a
/// branch of `sys.platform() == "linux"` alone builds for Windows and for
/// WebAssembly (the branch, known when the program is built, reaches
/// nothing there), and runs under Linux, interpreted and compiled; the
/// same call outside such a branch is still refused.
#[test]
fn a_branch_of_another_system_reaches_nothing() {
    let src = "import * as unix from \"unix\";\nimport * as net from \"net\";\nimport * as sys from \"sys\";\n\naction local(): void {\n    match (unix.connect(\"@isocron-no-one-here\")) {\n        Ok(c) => net.close(c),\n        Err(e) => print(\"no peer\"),\n    };\n}\n\naction main(args: string[k]): i32 {\n    print(sys.platform());\n    if (sys.platform() == \"linux\") {\n        local();\n    }\n    if (sys.platform() != \"linux\" && args.length > 99) {\n        print(\"elsewhere\");\n    }\n    return 0;\n}\n";
    assert_eq!(interp_ok(src, &[]), "linux\nno peer\n", "interpreter");
    let r = run_bin(&compile(src, "platform_linux"), &[]);
    assert_eq!((r.code, r.stdout.as_str()), (0, "linux\nno peer\n"), "compiled: {}", r.stderr);
    let p = iscc::frontend("platforms.isc", src).unwrap_or_else(|e| panic!("{}", e));
    assert!(p.sys_paths[0].iter().any(|(op, _, _)| op.is_unix()), "Linux reaches the module unix");
    assert!(!p.sys_paths[1].iter().any(|(op, _, _)| op.is_unix()) && !p.sys_paths[2].iter().any(|(op, _, _)| op.is_unix()), "Windows and WASI reach none of it");
    // built for WebAssembly (no C library needed)
    let opts = iscc::BuildOptions { output: tmp("platform_wasi.wasm"), emit_llvm: None, emit_asm: None, debug: false, codegen: isc_codegen::Options { isa: isc_codegen::Isa::Wasm32, ..isc_codegen::Options::default() }, link: Vec::new(), glibc: false };
    iscc::build(&p, &opts).unwrap_or_else(|e| panic!("built for WASI: {}", e));
    // and for Windows, when its libraries are here
    if std::env::var_os("ISC_MINGW_DIR").is_some() || std::path::Path::new("/usr/x86_64-w64-mingw32/lib").is_dir() {
        let opts = iscc::BuildOptions { output: tmp("platform_windows.exe"), emit_llvm: None, emit_asm: None, debug: false, codegen: isc_codegen::Options { windows: true, ..isc_codegen::Options::default() }, link: Vec::new(), glibc: false };
        iscc::build(&p, &opts).unwrap_or_else(|e| panic!("built for Windows: {}", e));
    }
    // outside a test of the system: refused for Windows
    let outside = src.replace("    if (sys.platform() == \"linux\") {\n        local();\n    }\n", "    local();\n");
    let q = iscc::frontend("outside.isc", &outside).unwrap_or_else(|e| panic!("{}", e));
    assert!(q.sys_paths[1].iter().any(|(op, _, _)| op.is_unix()), "reached outside the branch");
}
