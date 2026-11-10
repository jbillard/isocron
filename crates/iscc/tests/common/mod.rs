#![allow(dead_code)]
//! Helpers shared by the integration tests.

use std::path::PathBuf;

pub const SUM: &str = include_str!("../../../../examples/poc/sum.isc");
pub const DOT: &str = include_str!("../../../../examples/poc/dot.isc");
pub const HIST: &str = include_str!("../../../../examples/poc/histogram.isc");
pub const STD: &str = include_str!("../../../../examples/poc/std_dev.isc");

pub fn tmp(name: &str) -> PathBuf {
    // `ISC_TEST_TMP`: a directory of its own for a run of the suites at the
    // same time as another (both allocators at once)
    let d = match std::env::var_os("ISC_TEST_TMP") {
        Some(t) => PathBuf::from(t),
        None => PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("isc-tests"),
    };
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

/// Deterministic pseudo-random generator (xorshift64*).
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    /// Floats of varied magnitudes and signs, so that the order of a sum
    /// changes its rounding.
    pub fn f64(&mut self) -> f64 {
        let m = (self.next() >> 11) as f64 / (1u64 << 53) as f64;
        let e = (self.next() % 40) as i32 - 20;
        let s = if self.next() & 1 == 0 { 1.0 } else { -1.0 };
        s * m * 2f64.powi(e)
    }
}

pub fn write_f64s(name: &str, xs: &[f64]) -> String {
    let p = tmp(name);
    std::fs::write(&p, isc_interp::rt::f64_to_le_canonical(xs)).unwrap();
    p.to_string_lossy().to_string()
}

pub fn read_f64s(path: &str) -> Vec<f64> {
    isc_interp::rt::read_f64_file(path).unwrap()
}

pub fn random_f64s(seed: u64, n: usize) -> Vec<f64> {
    let mut r = Rng(seed);
    (0..n).map(|_| r.f64()).collect()
}

pub enum Outcome {
    Exit(i32, String),
    Panic(String, String),
}

/// Runs a program with the interpreter, returning its exit code and output.
pub fn interp(src: &str, args: &[&str]) -> Outcome {
    let p = iscc::frontend("test.isc", src).unwrap_or_else(|e| panic!("{}", e));
    let mut out = Vec::new();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    match isc_interp::run(&p, &args, &mut out) {
        Ok(c) => Outcome::Exit(c, String::from_utf8(out).unwrap()),
        Err(e) => Outcome::Panic(e.to_string(), String::from_utf8(out).unwrap()),
    }
}

/// The interpreter with another V; panics unless the program exits with 0.
pub fn interp_lanes_ok(src: &str, args: &[&str], lanes: usize) {
    let p = iscc::frontend("test.isc", src).unwrap_or_else(|e| panic!("{}", e));
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    assert_eq!(isc_interp::run_with(&p, &args, &mut out, lanes).unwrap(), 0);
}

pub fn interp_ok(src: &str, args: &[&str]) -> String {
    match interp(src, args) {
        Outcome::Exit(0, out) => out,
        Outcome::Exit(c, out) => panic!("exit {} ({})", c, out),
        Outcome::Panic(m, _) => panic!("unexpected panic: {}", m),
    }
}

pub fn interp_panic(src: &str, args: &[&str]) -> String {
    match interp(src, args) {
        Outcome::Panic(m, _) => m,
        Outcome::Exit(c, out) => panic!("expected a panic, got exit {} ({})", c, out),
    }
}

/// Independent implementation of the fixed reduction form of section 10.
pub fn fixed_reduce<T: Copy>(xs: &[T], e: T, op: &dyn Fn(T, T) -> T) -> T {
    const V: usize = 32;
    const B: usize = 4096;
    let block = |b: &[T]| {
        let mut acc = [e; V];
        for (i, x) in b.iter().enumerate() {
            acc[i % V] = op(acc[i % V], *x);
        }
        let mut d = V / 2;
        while d >= 1 {
            for j in 0..d {
                acc[j] = op(acc[j], acc[j + d]);
            }
            d /= 2;
        }
        acc[0]
    };
    if xs.len() <= B {
        return block(xs);
    }
    let partials: Vec<T> = xs.chunks(B).map(block).collect();
    fixed_reduce(&partials, e, op)
}

/// Compiles a program to a native executable in the test directory.
pub fn compile(src: &str, name: &str) -> PathBuf {
    compile_with(src, name, isc_codegen::Options::default())
}

/// `compile` with another V or instruction set.
pub fn compile_with(src: &str, name: &str, codegen: isc_codegen::Options) -> PathBuf {
    compile_linked(src, name, codegen, &[])
}

/// `compile_with` and the libraries of the `extern` functions (`--link`).
pub fn compile_linked(src: &str, name: &str, codegen: isc_codegen::Options, link: &[&str]) -> PathBuf {
    compile_built(src, name, codegen, link, false, false)
}

/// `compile`, every function the program reaches compiled in it, none
/// taken from the module of the prelude compiled once (D611): for the
/// tests that read the IR of a whole program.
pub fn compile_whole(src: &str, name: &str) -> PathBuf {
    compile_built(src, name, isc_codegen::Options::default(), &[], false, true)
}

/// `compile_with`, the program linked with glibc (D289): for Valgrind,
/// which sees the blocks of glibc's allocator (`ISC_ALLOC=glibc`).
pub fn compile_glibc(src: &str, name: &str, codegen: isc_codegen::Options) -> PathBuf {
    compile_built(src, name, codegen, &[], true, false)
}

/// The memory of every program checked (D578): with `ISC_TEST_MEMCHECK` set,
/// the programs of the tests are linked with glibc for AVX2 (Valgrind decodes
/// no AVX-512) and run under Valgrind's memcheck on glibc's allocator
/// (`run_bin`): a read or a write outside the blocks of the heap, or of
/// memory never written, is an error, the exit code 99.
pub fn memcheck() -> bool {
    std::env::var_os("ISC_TEST_MEMCHECK").is_some()
}

/// The programs built for the memory check (`compile_built`): only they run
/// under Valgrind (a test may build its program another way, for the native
/// instructions, which Valgrind may not decode).
static MEMCHECKED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

fn compile_built(src: &str, name: &str, mut codegen: isc_codegen::Options, link: &[&str], mut glibc: bool, whole: bool) -> PathBuf {
    let checked = memcheck() && !codegen.windows;
    if checked {
        if matches!(codegen.isa, isc_codegen::Isa::Native | isc_codegen::Isa::Avx512) {
            codegen.isa = isc_codegen::Isa::Avx2;
        }
        glibc = true;
    }
    let p = iscc::frontend(&format!("{}.isc", name), src).unwrap_or_else(|e| panic!("{}", e));
    let out = tmp(name);
    let opts = iscc::BuildOptions {
        output: out.clone(),
        emit_llvm: Some(out.with_extension("ll")),
        emit_asm: Some(out.with_extension("s")),
        debug: false,
        codegen,
        link: link.iter().map(|s| s.to_string()).collect(),
        glibc,
    };
    let cache = if whole { None } else { iscc::pre_dir() };
    iscc::build_in(&p, &opts, cache.as_deref()).unwrap_or_else(|e| panic!("{}: {}", name, e));
    if checked {
        MEMCHECKED.lock().unwrap().push(out.clone());
    }
    out
}

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn run_bin(bin: &std::path::Path, args: &[&str]) -> Run {
    run_bin_threads(bin, args, None)
}

/// Runs a compiled program with `ISC_THREADS` set (None: the default).
pub fn run_bin_threads(bin: &std::path::Path, args: &[&str], threads: Option<usize>) -> Run {
    let mut cmd = if memcheck() && MEMCHECKED.lock().unwrap().iter().any(|b| b == bin) {
        let mut c = std::process::Command::new("valgrind");
        // 2 threads, as the other runs under Valgrind (it runs one at a
        // time; the threads of the pool spin before they sleep)
        c.args(["-q", "--error-exitcode=99"]).arg(bin).env("ISC_ALLOC", "glibc").env("ISC_THREADS", "2");
        c
    } else {
        std::process::Command::new(bin)
    };
    cmd.args(args);
    if let Some(t) = threads {
        cmd.env("ISC_THREADS", t.to_string());
    }
    let o = cmd.output().unwrap();
    Run {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).to_string(),
        stderr: String::from_utf8_lossy(&o.stderr).to_string(),
    }
}

/// A WASI engine to run WebAssembly programs: `$ISC_WASI_RUNNER` (a
/// `wasmtime`-compatible command), else `wasmtime` in the `PATH` or in
/// `~/.wasmtime/bin` (its installer's default). None: the WebAssembly checks
/// are skipped, with a message.
pub fn wasi_runner() -> Option<PathBuf> {
    if let Ok(r) = std::env::var("ISC_WASI_RUNNER") {
        return Some(r.into());
    }
    let mut dirs: Vec<PathBuf> = std::env::var("PATH").unwrap_or_default().split(':').map(PathBuf::from).collect();
    dirs.push(PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".wasmtime/bin"));
    dirs.into_iter().map(|d| d.join("wasmtime")).find(|p| p.is_file())
}

/// Compiles for `wasm32-wasi`.
pub fn compile_wasm(src: &str, name: &str) -> PathBuf {
    compile_wasm_isa(src, name, isc_codegen::Isa::Wasm32)
}

/// Compiles for `wasm32-wasi` (`Isa::Wasm32`) or `wasm32-wasi-threads`.
pub fn compile_wasm_isa(src: &str, name: &str, isa: isc_codegen::Isa) -> PathBuf {
    let mut o = isc_codegen::Options::default();
    o.isa = isa;
    let p = iscc::frontend(&format!("{}.isc", name), src).unwrap_or_else(|e| panic!("{}", e));
    let out = tmp(&format!("{}.wasm", name));
    let opts = iscc::BuildOptions { output: out.clone(), emit_llvm: None, emit_asm: None, debug: false, codegen: o, link: Vec::new(), glibc: false };
    iscc::build(&p, &opts).unwrap_or_else(|e| panic!("{}: {}", name, e));
    out
}

/// Runs a WebAssembly program with the WASI engine; the whole file system is
/// visible to it at the same paths.
pub fn run_wasm(runner: &std::path::Path, wasm: &std::path::Path, args: &[&str]) -> Run {
    run_wasm_threads(runner, wasm, args, None)
}

/// Runs a `wasm32-wasi-threads` module with `ISC_THREADS = threads` (the
/// engine's threads enabled), or a `wasm32-wasi` one (None).
pub fn run_wasm_threads(runner: &std::path::Path, wasm: &std::path::Path, args: &[&str], threads: Option<usize>) -> Run {
    let mut cmd = std::process::Command::new(runner);
    cmd.arg("run").arg("--dir=/::/");
    if let Some(t) = threads {
        cmd.args(["-W", "threads=y", "-S", "threads=y", "--env"]).arg(format!("ISC_THREADS={}", t));
    }
    let o = cmd.arg(wasm).args(args).output().unwrap();
    Run {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).to_string(),
        stderr: String::from_utf8_lossy(&o.stderr).to_string(),
    }
}

/// `src` built as a static program (without glibc) and run with the
/// allocator checked (`ISC_ALLOC=check`, D546) on 2 threads: no fault and
/// no block left at the end of `main` (the code 0), as its version linked
/// with glibc under Valgrind.
pub fn checked_clean(src: &str, name: &str, args: &[&str]) {
    let bin = compile(src, &format!("{}_chk", name));
    checked_clean_bin(&bin, args, None);
}

/// `bin` run with the allocator checked (D546), `input` its standard input.
pub fn checked_clean_bin(bin: &std::path::Path, args: &[&str], input: Option<&[u8]>) {
    use std::io::Write;
    let mut c = std::process::Command::new(bin)
        .args(args)
        .env("ISC_ALLOC", "check")
        .env("ISC_THREADS", "2")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(input.unwrap_or(&[])).unwrap();
    let o = c.wait_with_output().unwrap();
    assert_eq!(o.status.code(), Some(0), "{:?} with the allocator checked: {}", bin, String::from_utf8_lossy(&o.stderr));
}

