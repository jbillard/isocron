//! The kernel of the runtime written in Isocron (SPEC section 17, D599):
//! its primitives, read as a part of the prelude (`$…`), the same
//! interpreted and compiled.

mod common;
use common::*;

/// The program run by the interpreter: its output, or the message of its
/// panic.
fn interpreted(src: &str) -> Result<String, String> {
    let p = iscc::frontend_as_prelude("kernel.isc", src).unwrap_or_else(|e| panic!("{}", e));
    let mut out = Vec::new();
    match isc_interp::run(&p, &[], &mut out) {
        Ok(0) => Ok(String::from_utf8(out).unwrap()),
        Ok(c) => panic!("exit {}", c),
        Err(e) => Err(e.to_string()),
    }
}

/// The program compiled and run: its code, its output, its errors.
fn compiled(src: &str, name: &str) -> Run {
    let p = iscc::frontend_as_prelude("kernel.isc", src).unwrap_or_else(|e| panic!("{}", e));
    let out = tmp(name);
    let opts = iscc::BuildOptions {
        output: out.clone(),
        emit_llvm: Some(out.with_extension("ll")),
        emit_asm: None,
        debug: false,
        codegen: isc_codegen::Options::default(),
        link: Vec::new(),
        glibc: false,
    };
    iscc::build(&p, &opts).unwrap_or_else(|e| panic!("{}: {}", name, e));
    run_bin(&out, &[])
}

/// D601: the arena reserved (1 TiB of addresses, nothing committed), 2 MiB
/// of it committed, words of 1, 2, 4 and 8 bytes written and read back at
/// its start and its end; the zone of the thread, zeros at first, the
/// same. The same output interpreted and compiled.
#[test]
fn arena_and_zone_words() {
    let src = "action main(args: string[k]): i32 {
    print([$arenaBase(), $arenaSize()]);
    const whole = 1099511627776u64;
    const base = $arenaReserve(whole);
    print([base != 0u64 ? 1u64 : 0u64, $arenaBase() == base ? 1u64 : 0u64, $arenaSize(), base % 2097152u64]);
    print($arenaCommit(base, 2097152u64));
    print([$load64(base), $load8(base + 2097151u64)]);
    $store64(base + 64u64, 0x1122334455667788u64);
    $store8(base + 64u64, 0xFFu64);
    print([$load64(base + 64u64), $load8(base + 65u64), $load16(base + 70u64), $load32(base + 64u64)]);
    $store16(base + 2097150u64, 0xBEEFu64);
    $store32(base + 100u64, 0xFFFFFFFF01020304u64);
    print([$load16(base + 2097150u64), $load8(base + 2097151u64), $load64(base + 96u64)]);
    print([$zoneLoad64(0), $zoneLoad16(4094)]);
    $zoneStore64(8, 42u64);
    $zoneStore16(4094, 0xABCDu64);
    $zoneStore32(100, 0x1FFFFFFFFu64);
    print([$zoneLoad64(8), $zoneLoad16(4094), $zoneLoad32(100), $zoneLoad64(100), $zoneLoad32(0)]);
    // pages outside what was committed: refused by the commit, not read
    print([$arenaCommit(base + whole, 4096u64), $arenaCommit(base - 4096u64, 4096u64), $arenaCommit(base + whole - 4096u64, 4096u64)]);
    $store64(base + whole - 8u64, 7u64);
    print($load64(base + whole - 8u64));
    return 0;
}
";
    let want = "[0, 0]\n[1, 1, 1099511627776, 0]\n0\n[0, 0]\n[1234605616436508671, 119, 4386, 1432778751]\n[48879, 190, 72623859706101760]\n[0, 0]\n[42, 43981, 4294967295, 4294967295, 0]\n[-22, -22, 0]\n7\n";
    assert_eq!(interpreted(src).unwrap(), want, "interpreted");
    let r = compiled(src, "kernel_words");
    assert_eq!((r.code, r.stdout.as_str()), (0, want), "{}", r.stderr);
}

/// An address outside the arena, a word across its end, an offset past the
/// zone, anything before the arena is reserved: a panic with its place,
/// never a read or a write; the same interpreted and compiled.
#[test]
fn outside_the_arena_is_a_panic() {
    let cases = [
        ("print($load8(4096u64));", "the kernel reached outside its arena"),
        ("const b = $arenaReserve(4194304u64);\n    print($load64(b - 8u64));", "the kernel reached outside its arena"),
        ("const b = $arenaReserve(4194304u64);\n    print($arenaCommit(b, 4096u64));\n    print($load64(b + 4194304u64 - 7u64));", "the kernel reached outside its arena"),
        ("const b = $arenaReserve(4194304u64);\n    $store8(b + 4194304u64, 1u64);", "the kernel reached outside its arena"),
        ("const b = $arenaReserve(4194304u64);\n    $store64(0u64 - 4u64, 1u64);", "the kernel reached outside its arena"),
        ("print($zoneLoad64(4089));", "the kernel reached outside the zone of its thread"),
        ("$zoneStore16(4095, 1u64);", "the kernel reached outside the zone of its thread"),
        ("print($zoneLoad32(0 - 1));", "the kernel reached outside the zone of its thread"),
    ];
    for (k, (body, message)) in cases.iter().enumerate() {
        let src = format!("action main(args: string[k]): i32 {{\n    {}\n    return 0;\n}}\n", body);
        let e = interpreted(&src).unwrap_err();
        assert!(e.contains(message), "{}: interpreted: {}", body, e);
        let r = compiled(&src, &format!("kernel_outside_{}", k));
        assert_eq!(r.code, 101, "{}: {}", body, r.stderr);
        assert!(r.stderr.contains(message), "{}: compiled: {}", body, r.stderr);
    }
}

/// The primitives of the kernel are the prelude's: a program cannot name
/// them, and a function (not an action) of the prelude cannot call them.
#[test]
fn only_the_prelude_and_only_actions() {
    let e = iscc::frontend("t.isc", "action main(args: string[k]): i32 {\n    print($arenaBase());\n    return 0;\n}\n").unwrap_err();
    assert!(e.contains("$") || e.contains("unexpected"), "{}", e);
    let e = iscc::frontend_as_prelude("t.isc", "function f(a: u64): u64 {\n    return $load64(a);\n}\n\naction main(args: string[k]): i32 {\n    print(f(0u64));\n    return 0;\n}\n").unwrap_err();
    assert!(e.contains("cannot be called here") || e.contains("cannot call the action"), "{}", e);
}
