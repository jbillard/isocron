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
    let opts = iscc::BuildOptions { output: out.clone(), emit_llvm: None, emit_asm: None, debug: false, codegen: o, link: Vec::new() };
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
