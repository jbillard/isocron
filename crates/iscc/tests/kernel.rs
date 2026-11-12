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
    // (the allocator of the program owns the start of the arena and of the
    // zone, D605: the words of this test are far in the arena, 512 GiB
    // after its start, and in the second half of the zone)
    let src = "action main(args: string[k]): i32 {
    const whole = 1099511627776u64;
    const start = $kStart();
    const base = start + 549755813888u64;
    print([start != 0u64 ? 1u64 : 0u64, $arenaBase() == start ? 1u64 : 0u64, $arenaSize(), start % 2097152u64]);
    print($arenaCommit(base, 2097152u64));
    print([$load64(base), $load8(base + 2097151u64)]);
    $store64(base + 64u64, 0x1122334455667788u64);
    $store8(base + 64u64, 0xFFu64);
    print([$load64(base + 64u64), $load8(base + 65u64), $load16(base + 70u64), $load32(base + 64u64)]);
    $store16(base + 2097150u64, 0xBEEFu64);
    $store32(base + 100u64, 0xFFFFFFFF01020304u64);
    print([$load16(base + 2097150u64), $load8(base + 2097151u64), $load64(base + 96u64)]);
    print([$zoneLoad64(2048), $zoneLoad16(4094)]);
    $zoneStore64(2056, 42u64);
    $zoneStore16(4094, 0xABCDu64);
    $zoneStore32(2148, 0x1FFFFFFFFu64);
    print([$zoneLoad64(2056), $zoneLoad16(4094), $zoneLoad32(2148), $zoneLoad64(2148), $zoneLoad32(2048)]);
    // pages outside the arena: refused by the commit, not read
    print([$arenaCommit(start + whole, 4096u64), $arenaCommit(start - 4096u64, 4096u64), $arenaCommit(start + whole - 4096u64, 4096u64)]);
    $store64(start + whole - 8u64, 7u64);
    print($load64(start + whole - 8u64));
    // a range of the arena: whole inside it, or not one of its ranges
    print([$arenaHas(start, 64u64), $arenaHas(start + whole - 64u64, 64u64), $arenaHas(start + whole - 63u64, 64u64), $arenaHas(start - 1u64, 1u64)]);
    print([$arenaHas(0u64, 64u64), $arenaHas(start, whole), $arenaHas(start, whole + 1u64), $arenaHas(start + whole, 0u64), $arenaHas(0u64 - 8u64, 64u64)]);
    return 0;
}
";
    let want = "[1, 1, 1099511627776, 0]\n0\n[0, 0]\n[1234605616436508671, 119, 4386, 1432778751]\n[48879, 190, 72623859706101760]\n[0, 0]\n[42, 43981, 4294967295, 4294967295, 0]\n[-22, -22, 0]\n7\n[true, true, false, false]\n[false, true, false, true, false]\n";
    assert_eq!(interpreted(src).unwrap(), want, "interpreted");
    let r = compiled(src, "kernel_words");
    assert_eq!((r.code, r.stdout.as_str()), (0, want), "{}", r.stderr);
}

/// An address outside the arena, a word across its end, a word of an atomic
/// operation not on 8 bytes, an offset past the zone, anything before the
/// arena is reserved: a panic with its place,
/// never a read or a write; the same interpreted and compiled.
#[test]
fn outside_the_arena_is_a_panic() {
    let cases = [
        ("print($load8(4096u64));", "the kernel reached outside its arena"),
        ("const b = $kStart();\n    print($load64(b - 8u64));", "the kernel reached outside its arena"),
        ("const b = $kStart();\n    print($load64(b + 1099511627776u64 - 7u64));", "the kernel reached outside its arena"),
        ("const b = $kStart();\n    $store8(b + 1099511627776u64, 1u64);", "the kernel reached outside its arena"),
        ("const b = $kStart();\n    $store64(0u64 - 4u64, 1u64);", "the kernel reached outside its arena"),
        ("const b = $kStart();\n    print($cas64(b + 4u64, 0u64, 1u64));", "the kernel reached outside its arena"),
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

/// D602, D603: the small blocks of the allocator written in Isocron over
/// the primitives of the kernel: 3000 blocks of 1 to 1024 bytes on 64
/// bytes, none over another, their marks whole; half of them freed and the
/// same requests again: nothing new cut, the blocks in use apart; the others
/// not touched; everything freed and every request again: nothing new cut. A
/// block freed twice is a panic. The same interpreted and compiled.
#[test]
fn small_blocks_written_in_isocron() {
    let src = include_str!("data/kernel_small.isc");
    let p = iscc::frontend_as_prelude("kernel.isc", src).unwrap_or_else(|e| panic!("{}", e));
    let mut out = Vec::new();
    let e = isc_interp::run(&p, &[], &mut out).unwrap_err().to_string();
    assert!(e.contains("invalid free"), "interpreted: {}", e);
    let want = "1\n1\n3000\n[1, 1]\n1500\n[1, 1]\n1\n";
    assert_eq!(String::from_utf8(out).unwrap(), want, "interpreted");
    let r = compiled(src, "kernel_small");
    assert_eq!((r.code, r.stdout.as_str()), (101, want), "{}", r.stderr);
    assert!(r.stderr.contains("invalid free"), "{}", r.stderr);
}

/// D603: the same allocator on several threads: 16 tasks, each 300 times
/// 200 blocks marked with its number, its marks checked, the blocks given
/// back; the lists of the threads overflow into the shared ones, under
/// their locks, and are refilled from them. No mark of a task is ever
/// another's, and the memory cut stays small: the blocks are taken again.
#[test]
fn small_blocks_on_several_threads() {
    let src = include_str!("data/kernel_threads.isc");
    let p = iscc::frontend_as_prelude("kernel.isc", src).unwrap_or_else(|e| panic!("{}", e));
    let out = tmp("kernel_threads");
    let opts = iscc::BuildOptions {
        output: out.clone(),
        emit_llvm: Some(out.with_extension("ll")),
        emit_asm: None,
        debug: false,
        codegen: isc_codegen::Options::default(),
        link: Vec::new(),
        glibc: false,
    };
    iscc::build(&p, &opts).unwrap_or_else(|e| panic!("{}", e));
    let want = "[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]\n1\n";
    for threads in [1, 2, 8, 16] {
        for _ in 0..3 {
            let r = run_bin_threads(&out, &[], Some(threads));
            assert_eq!((r.code, r.stdout.as_str()), (0, want), "{} threads: {}", threads, r.stderr);
        }
    }
}

/// D605: the generated code takes its small blocks from the allocator
/// written in Isocron. A program that builds texts moved the first byte
/// never given of the arena; with the allocator of the system asked for
/// (`ISC_ALLOC`), no arena is reserved, the blocks are the runtime's, and
/// the program gives the same texts.
#[test]
fn the_generated_code_allocates_in_the_arena() {
    let src = "action main(args: string[k]): i32 {
    const xs = map(iota(200), (i) => toString(i * 7) + \"-\" + toString(i));
    const base = $arenaBase();
    print([base != 0u64 ? 1 : 0, base != 0u64 ? ($load64(base) > base + 4096u64 ? 1 : 0) : 0 - 1, $arenaHas(base, 1u64) ? 1 : 0]);
    print([xs[0], xs[199]]);
    print(join(xs, \"\").length);
    return 0;
}
";
    let p = iscc::frontend_as_prelude("kernel.isc", src).unwrap_or_else(|e| panic!("{}", e));
    let texts = "[0-0, 1393-199]\n1330\n";
    // (a program linked without the C library has no other allocator than
    // Isocron's, D289: `ISC_ALLOC=glibc` changes nothing there; a program
    // built with `ISC_ALLOC=glibc` set, the second run of the suite, is
    // linked with it whatever it asks)
    let (ours, theirs) = ("[1, 1, 1]\n", "[0, -1, 0]\n");
    let linked_anyway = std::env::var("ISC_ALLOC").map(|v| v == "glibc").unwrap_or(false);
    let without = if linked_anyway { theirs } else { ours };
    let out = tmp("kernel_wired");
    // (linked with the module of the prelude compiled once, D611, whose
    // words of the arena are then the program's; then whole, the program
    // whose IR is read below)
    for (cache, glibc, modes) in [
        (iscc::pre_dir(), false, [(None, ours), (Some("glibc"), without), (Some("check"), theirs)]),
        (iscc::pre_dir(), true, [(None, ours), (Some("glibc"), theirs), (Some("check"), theirs)]),
        (None, false, [(None, ours), (Some("glibc"), without), (Some("check"), theirs)]),
        (None, true, [(None, ours), (Some("glibc"), theirs), (Some("check"), theirs)]),
    ] {
        let opts = iscc::BuildOptions {
            output: out.clone(),
            emit_llvm: Some(out.with_extension("ll")),
            emit_asm: None,
            debug: false,
            codegen: isc_codegen::Options::default(),
            link: Vec::new(),
            glibc,
        };
        iscc::build_in(&p, &opts, cache.as_deref()).unwrap_or_else(|e| panic!("{}", e));
        for (mode, first) in modes {
            let mut cmd = std::process::Command::new(&out);
            match mode {
                Some(m) => cmd.env("ISC_ALLOC", m),
                None => cmd.env_remove("ISC_ALLOC"),
            };
            let r = cmd.output().unwrap();
            assert_eq!(
                (r.status.code(), String::from_utf8_lossy(&r.stdout).as_ref()),
                (Some(0), format!("{}{}", first, texts).as_str()),
                "the module of the prelude {}, glibc {}, ISC_ALLOC={:?}: {}",
                cache.is_some(),
                glibc,
                mode,
                String::from_utf8_lossy(&r.stderr)
            );
        }
    }
    // every allocation and every free of the program through the entries
    // of the module; the runtime's allocator called by those entries alone
    let ll = std::fs::read_to_string(out.with_extension("ll")).unwrap();
    assert!(ll.contains("call noalias ptr @isc.late.alloc(") && ll.contains("call void @isc.late.free(ptr "), "the entries are not called");
    assert_eq!(ll.matches("@isc_alloc(i64 ").count(), 3, "the allocator of the runtime called outside the entries");
    assert_eq!(ll.matches("call void @isc_free(").count(), 3, "the free of the runtime called outside the entries");
}

/// D605: a block of the runtime's allocator is not one of the arena:
/// `$arenaHas` says so and `$kFree` leaves it (0). Interpreted, the
/// runtime's allocator is not there: a panic.
#[test]
fn a_block_of_the_runtime_is_not_of_the_arena() {
    let src = "action main(args: string[k]): i32 {
    const start = $kStart();
    const p = $runtimeAlloc(100);
    print([$arenaHas(p, 100u64) ? 1 : 0, $arenaHas(p - 64u64, 64u64) ? 1 : 0, $kFree(p), $kFree(0u64), $kFree(64u64)]);
    const q = $kAlloc(100);
    print([$arenaHas(q, 100u64) ? 1 : 0, $kFree(q)]);
    return 0;
}
";
    let e = interpreted(src).unwrap_err();
    assert!(e.contains("the allocator of the runtime is not the interpreter's"), "{}", e);
    // (the block of the runtime is not given back, no primitive does it:
    // run without the checked allocator, `ISC_ALLOC=check` of the third run
    // of the suite, which rightly says so)
    let _ = compiled(src, "kernel_runtime_block");
    let r = std::process::Command::new(tmp("kernel_runtime_block")).env_remove("ISC_ALLOC").output().unwrap();
    assert_eq!((r.status.code(), String::from_utf8_lossy(&r.stdout).as_ref()), (Some(0), "[0, 0, 0, 0, 0]\n[1, 1]\n"), "{}", String::from_utf8_lossy(&r.stderr));
}

/// D605: a block of the arena goes back to the kernel whoever frees it.
/// The texts of an array are freed by the module (`isc.late.free_strs`);
/// those of a large array out of the order of their places, and a text
/// given to a function of the runtime, by the runtime, which gives each
/// back through the entry of the program. After the first rounds the arena
/// grows by 64 KiB at most over 120 more. (Before, a block freed by the
/// runtime joined the lists of its allocator and the kernel cut new ones
/// for ever: 1.2 GiB for an array of 4096 texts made and freed 2400 times,
/// 0.5 MiB a round. Nothing new at all was asked before D651: since then a
/// thread may run the share of another that has not started, two shares at
/// once, its blocks then more than in the first rounds; measured under
/// load: one block of 8 KiB more once, then none over 760 rounds.)
#[test]
fn blocks_freed_by_the_runtime_come_back() {
    let src = "function texts(n: i64, r: i64): string[n] {
    return map(iota(n), (i) => toString(i * 7 + r) + \"-\" + toString(r));
}

function total(xs: string[n]): i64 {
    return sum(map(xs, (t) => t.length));
}

// an array of texts made, read twice, freed
action small(r: i64): i64 {
    const xs = texts(300, r);
    return total(xs) + xs[r % 300].length;
}

// 5000 texts sorted (their places out of order), freed
action sorted(r: i64): i64 {
    const xs = sortBy(texts(5000, r), (t) => 0 - t.length);
    return total(xs) + xs[r % 5000].length;
}

// texts given to functions of the runtime (`replace`, `split`)
action given(r: i64): i64 {
    const t = toString(r) + \"-a-b-c-\" + toString(r * 3);
    const parts = split(replace(t, \"-\", \"+\"), \"+\");
    return parts.length + total(parts);
}

action rounds(from: i64, count: i64): i64 {
    return sum(map(iota(count), (k) => small(from + k) + sorted(from + k) + given(from + k)));
}

action main(args: string[k]): i32 {
    const base = $arenaBase();
    // (the same count each time: the array of the results of the rounds is
    // a block too)
    const warm = rounds(0, 40);
    const cut = $load64(base);
    const more = rounds(40, 40) + rounds(80, 40) + rounds(120, 40);
    print([warm > 0 ? 1 : 0, more > 0 ? 1 : 0, $load64(base) - cut <= 65536u64 ? 1 : 0]);
    return 0;
}
";
    let p = iscc::frontend_as_prelude("kernel.isc", src).unwrap_or_else(|e| panic!("{}", e));
    let out = tmp("kernel_back");
    let opts = iscc::BuildOptions {
        output: out.clone(),
        emit_llvm: Some(out.with_extension("ll")),
        emit_asm: None,
        debug: false,
        codegen: isc_codegen::Options::default(),
        link: Vec::new(),
        glibc: false,
    };
    iscc::build(&p, &opts).unwrap_or_else(|e| panic!("{}", e));
    let ll = std::fs::read_to_string(out.with_extension("ll")).unwrap();
    assert!(ll.contains("call void @isc.late.free_strs(ptr "), "no array of texts freed by the module");
    for threads in ["1", "4"] {
        let r = std::process::Command::new(&out).env_remove("ISC_ALLOC").env("ISC_THREADS", threads).output().unwrap();
        assert_eq!((r.status.code(), String::from_utf8_lossy(&r.stdout).as_ref()), (Some(0), "[1, 1, 1]\n"), "{} threads: {}", threads, String::from_utf8_lossy(&r.stderr));
    }
}
