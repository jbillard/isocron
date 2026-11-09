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
fn run_with(bin: &std::path::Path, args: &[&str], alloc: Option<&str>, threads: usize) -> (i32, String, String) {
    let mut c = std::process::Command::new(bin);
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
