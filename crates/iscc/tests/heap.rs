//! The allocator of the runtime (`crates/isc-runtime/src/heap.rs`) and the
//! allocator checked (`ISC_ALLOC=check`, D546), without C (they were
//! programs of C): a load of strings of every size made and checked by the
//! threads of a program of Isocron; the faults that no program of Isocron
//! can make (a block freed twice, a write past the end of a block or into a
//! block freed, a block never freed), written in LLVM IR.

mod common;
use common::*;

/// `bin` run with `args` on `threads` threads and `ISC_ALLOC` set to `alloc`
/// (or unset): its exit code, its output and its error.
/// The command that runs `bin` without transparent huge pages (D735):
/// `prctl(PR_SET_THP_DISABLE)` before its `exec`, by Python as the measures
/// of `bench/` do; its peaks counted in pages of 4 KiB. With huge pages
/// (`always` on this machine) a peak jumps by 2 MiB at once, as the memory
/// of the moment allows: the program of
/// `a_medium_block_kept_too_large_is_cut`, the same bytes, peaked at 4392
/// KiB in one series and 2416 in the next.
fn command_of(bin: &std::path::Path) -> std::process::Command {
    let mut c = std::process::Command::new("python3");
    c.args(["-c", "import ctypes, os, sys; ctypes.CDLL(None).prctl(41, 1, 0, 0, 0); os.execv(sys.argv[1], sys.argv[1:])"]).arg(bin);
    c
}

fn run_with(bin: &std::path::Path, args: &[&str], alloc: Option<&str>, threads: usize) -> (i32, String, String) {
    let mut c = command_of(bin);
    c.args(args).env("ISC_THREADS", threads.to_string());
    match alloc {
        Some(a) => c.env("ISC_ALLOC", a),
        None => c.env_remove("ISC_ALLOC"),
    };
    let o = c.output().unwrap();
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into(), String::from_utf8_lossy(&o.stderr).into())
}

/// Strings of 16 bytes to 40 MiB made by the threads of a `map`, checked
/// by others, freed by `main`: nothing wrong with the allocator of Isocron
/// and with the allocator checked (no block written past its end or after
/// its free, none left at the end), on 1 and 4 threads.
#[test]
fn stress_with_every_allocator() {
    let bin = compile(include_str!("data/heap_stress.isc"), "heap_stress");
    for alloc in [None, Some("check")] {
        for t in [1, 4] {
            let (code, out, err) = run_with(&bin, &["6", "1200"], alloc, t);
            assert_eq!((code, out.as_str()), (0, "0\n"), "{:?}, {} threads: {}", alloc, t, err);
        }
    }
}

/// A static program written in LLVM IR, the language the compiler emits
/// (D546): `body` between the arguments read (`isc_args`, which notes the
/// environment, `ISC_ALLOC` among it) and the end of `main` (`isc_main_end`),
/// `%p` a block of 64 bytes of `isc_alloc`.
fn ir_program(name: &str, body: &str) -> std::path::PathBuf {
    let ir = format!(
        "declare ptr @isc_alloc(i64)\ndeclare void @isc_free(ptr)\ndeclare void @isc_args(ptr, i32, ptr)\ndeclare i32 @isc_main_end(i32)\n\
         define i32 @main(i32 %argc, ptr %argv) {{\n  %a = alloca {{ ptr, i64 }}\n  call void @isc_args(ptr %a, i32 %argc, ptr %argv)\n  %p = call ptr @isc_alloc(i64 64)\n{}  %r = call i32 @isc_main_end(i32 0)\n  ret i32 %r\n}}\n",
        body
    );
    let obj = tmp(&format!("{}.o", name));
    let out = tmp(name);
    isc_codegen::backend::compile_for(&ir, &obj, None, isc_codegen::Isa::Native, false, false).unwrap();
    iscc::link_static_object(&obj, &out).unwrap();
    out
}

/// A block freed twice: a panic of the allocator of Isocron (`invalid
/// free`), and of the allocator checked (it was a program of C).
#[test]
fn a_double_free_is_a_panic() {
    let bin = ir_program("heap_double_free", "  call void @isc_free(ptr %p)\n  call void @isc_free(ptr %p)\n");
    for (alloc, what) in [(None, "invalid free"), (Some("check"), "a block freed twice")] {
        let (code, _, err) = run_with(&bin, &[], alloc, 1);
        assert_eq!(code, 101, "{:?}: {}", alloc, err);
        assert!(err.contains(what), "{:?}: {}", alloc, err);
    }
}

/// The faults the allocator checked finds: a byte written past the end of a
/// block (seen at its free), a byte written into a block freed (seen when
/// it leaves the quarantine, at the latest at the end of `main`), a block
/// never freed (the code 9 at the end of `main`, as the tests under
/// Valgrind); the leak goes unseen without the check (the other two would
/// damage the lists of the allocator of Isocron).
#[test]
fn the_checked_allocator_finds_the_faults() {
    for (name, body, code, what) in [
        ("past_end", "  %q = getelementptr i8, ptr %p, i64 64\n  store i8 1, ptr %q\n  call void @isc_free(ptr %p)\n", 101, "a block written past its end"),
        ("after_free", "  call void @isc_free(ptr %p)\n  %q = getelementptr i8, ptr %p, i64 8\n  store volatile i8 1, ptr %q\n", 101, "a block written after its free"),
        ("leak", "", 9, "1 blocks of 64 bytes not freed at the end of main"),
    ] {
        let bin = ir_program(&format!("heap_check_{}", name), body);
        let (c, _, err) = run_with(&bin, &[], Some("check"), 1);
        assert_eq!(c, code, "{}: {}", name, err);
        assert!(err.contains(what), "{}: {}", name, err);
        if name == "leak" {
            let (c, _, err) = run_with(&bin, &[], None, 1);
            assert_eq!(c, 0, "{} without the check: {}", name, err);
        }
    }
}

/// The last line a program prints, a number (KiB), and the lines before it.
fn lines_and_last(bin: &std::path::Path, args: &[&str]) -> (String, u64) {
    let (code, out, err) = run_with(bin, args, None, 1);
    assert_eq!(code, 0, "{}", err);
    let mut ls: Vec<&str> = out.lines().collect();
    let n = ls.pop().unwrap_or("").chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or_else(|_| panic!("{}", out));
    (ls.join("\n"), n)
}

/// D721: the first refill of a small class takes one block, twice as many
/// at each one after; it took half of what the thread keeps of the class
/// at once, each block's header written: one block of each of 20 classes
/// of 1100 to 35000 bytes (205 KB of texts) added 1152 KiB of resident
/// memory, 372 now.
#[test]
fn a_small_class_used_once_takes_one_block() {
    if std::env::var("ISC_ALLOC").is_ok() {
        return;
    }
    let src = "action main(args: string[k]): i32 {
    const before = rss();
    const sizes = map(iota(20), (k) => i64(1100.0 * pow(1.2, f64(k))));
    const texts = map(sizes, (s) => \"x\".repeat(s));
    const after = rss();
    print(sum(map(texts, (t) => t.length)));
    print(after - before);
    return 0;
}

action rss(): i64 {
    const status = lines(readText(\"/proc/self/status\"));
    const l = filter(status, (l) => l.startsWith(\"VmRSS\"))[0];
    return parseI64(trim(l.slice(6, l.length - 3)));
}
";
    let bin = compile(src, "heap_ramp");
    let (out, grown) = lines_and_last(&bin, &[]);
    assert_eq!(out, "205347");
    assert!(grown < 700, "{} KiB more resident", grown);
}

/// D737: the blocks a thread carves at its first refills of a class are its
/// own count's, 1, 2, 4..., not those of the thread that used the class
/// most: one thread holds 64 texts of each of 27 sizes (15 classes of the
/// arena, 100 to 996 bytes; 12 of the runtime, 1100 to 8800), then the
/// threads of the pool take one text of each size each. On 4 threads that
/// second part added 1384 KiB of resident memory (each thread's first
/// blocks of a class carved by 16 or 32 at once), 68 to 72 now; 44 to 48
/// on 1 thread before and after.
#[test]
fn the_first_blocks_of_a_class_of_a_thread_are_its_own() {
    if std::env::var("ISC_ALLOC").is_ok() {
        return;
    }
    let src = "action main(args: string[k]): i32 {
    const sizes = concat(map(iota(15), (c) => 100 + 64 * c), map(iota(12), (k) => 1100 + 700 * k));
    const warm = fold(sizes, 0, (n, s) => n + fold(iota(64), fill(64, \"\"), (acc, i) => acc.with(i, \"x\".repeat(s))).length);
    const before = rss();
    const lens = map(iota(64), (i) => sum(map(sizes, (s) => \"y\".repeat(s + i % 3).length)));
    const after = rss();
    print(warm);
    print(sum(lens));
    print(after - before);
    return 0;
}

action rss(): i64 {
    const status = lines(readText(\"/proc/self/status\"));
    const l = filter(status, (l) => l.startsWith(\"VmRSS\"))[0];
    return parseI64(trim(l.slice(6, l.length - 3)));
}
";
    let bin = compile(src, "heap_thread_ramp");
    let (code, out, err) = run_with(&bin, &[], None, 4);
    assert_eq!(code, 0, "{}", err);
    let ls: Vec<&str> = out.lines().collect();
    assert_eq!(&ls[..2], &["1728", "4329381"], "{}", out);
    let grown: u64 = ls[2].parse().unwrap();
    assert!(grown < 400, "{} KiB more resident", grown);
}

/// D721: a medium block a thread keeps (freed last) that a request finds too
/// large goes back to the lists, where that request is cut from it: an
/// array of 2 MiB freed (at its last read, D720), then three of 720 KB
/// take its pages (4392 KiB at the most), as many as when the array is
/// read after them, and new ones (6440 KiB; 6452 for both before, the
/// block kept with its pages while the three took new ones).
#[test]
fn a_medium_block_kept_too_large_is_cut() {
    if std::env::var("ISC_ALLOC").is_ok() {
        return;
    }
    let src = "action main(args: string[k]): i32 {
    const n = parseI64(args[0]);
    const big = map(iota(262144), (i) => u64(i * n));
    const s = sum(big) + big[n];
    const t = big[2 * n];
    const p = map(iota(90000), (i) => u64(i) + s % 3u64);
    const q = map(iota(90000), (i) => u64(i * 2) + t % 3u64);
    const r = map(iota(90000), (i) => u64(i * 3) + s % 5u64);
    print(s + t + p[n] + q[2 * n] + r[3 * n] + p[89999] + q[89998] + r[89997]);
    const status = lines(readText(\"/proc/self/status\"));
    print(filter(status, (l) => l.startsWith(\"VmHWM\")));
    return 0;
}
";
    let kept = src.replace("    print(s + t + p[n]", "    print(big[5] + s + t + p[n]");
    let (out, most) = lines_and_last(&compile(src, "heap_medium_cut"), &["7"]);
    let (out_k, most_k) = lines_and_last(&compile(&kept, "heap_medium_kept"), &["7"]);
    assert_eq!(out, "240517791311");
    assert_eq!(out_k, "240517791346");
    assert!(most + 1024 < most_k, "freed {} KiB, kept {} KiB", most, most_k);
}
