//! The allocator of the runtime (`crates/isc-runtime/src/heap.rs`), driven
//! from C programs linked with the runtime library: a stress test with many
//! threads (sizes from 16 bytes to 40 MiB, blocks freed by other threads,
//! threads that end, zeroed blocks, patterns checked), run with the
//! allocator of Isocron and with glibc's, and a double free, which must be
//! a panic.

use std::path::{Path, PathBuf};
use std::process::Command;

fn build(name: &str) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/c").join(format!("{}.c", name));
    let dir = std::env::temp_dir().join(format!("isc-heap-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(name);
    let cc = std::env::var("ISC_CC").unwrap_or_else(|_| "cc".into());
    let st = Command::new(&cc)
        .args(["-O2", "-o"])
        .arg(&exe)
        .arg(&src)
        .arg(env!("ISC_RUNTIME_LIB"))
        .arg("-lpthread")
        .status()
        .expect("cannot run the C compiler");
    assert!(st.success(), "cannot build {}", src.display());
    exe
}

fn run(exe: &Path, alloc: Option<&str>) -> std::process::Output {
    let mut cmd = Command::new(exe);
    match alloc {
        Some(a) => cmd.env("ISC_ALLOC", a),
        None => cmd.env_remove("ISC_ALLOC"),
    };
    cmd.output().unwrap()
}

#[test]
fn stress_with_both_allocators() {
    let exe = build("heap_stress");
    for alloc in [None, Some("glibc")] {
        let out = run(&exe, alloc);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success() && stdout.trim() == "ok", "{:?}: {} {}", alloc, stdout, String::from_utf8_lossy(&out.stderr));
    }
}

#[test]
fn a_double_free_is_a_panic() {
    let exe = build("heap_double_free");
    let out = run(&exe, None);
    assert_eq!(out.status.code(), Some(101));
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid free"), "{}", String::from_utf8_lossy(&out.stderr));
}
