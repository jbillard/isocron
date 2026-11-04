//! Compiler pipeline shared by the `iscc` binary and the tests.

use std::path::{Path, PathBuf};

/// Parses, type-checks, lowers and optimizes a program and the modules it
/// imports. Errors are rendered as `file:line:col: error: message`.
pub fn frontend(path: &str, src: &str) -> Result<isc_ir::Program, String> {
    let mut files = isc_types::source_files(path);
    let mut user = isc_syntax::parse(src, isc_types::USER_FILE).map_err(|d| d.render(&files))?;
    let loaded = isc_types::modules::load_modules(&mut user, Path::new(path), &mut files);
    let render = |d: isc_syntax::Diagnostic| d.render(&files);
    loaded.map_err(render)?;
    let t = isc_types::check_parsed(&user).map_err(render)?;
    let mut p = isc_ir::lower::lower(&t).map_err(render)?;
    isc_ir::passes::optimize(&mut p).map_err(render)?;
    Ok(p)
}

#[derive(Default, Clone, Debug)]
pub struct BuildOptions {
    pub output: PathBuf,
    pub emit_llvm: Option<PathBuf>,
    pub emit_asm: Option<PathBuf>,
    /// Keep the symbols in the executable (default: stripped).
    pub debug: bool,
    /// V and target instruction set (experiments; defaults: V = 32, host CPU).
    pub codegen: isc_codegen::Options,
    /// The libraries of the `extern` functions (`--link`, D125): a name
    /// (`-lname`) or the path of a library or an object file.
    pub link: Vec<String>,
}

/// Compiles the IR to a native executable.
pub fn build(p: &isc_ir::Program, opts: &BuildOptions) -> Result<(), String> {
    let ir = isc_codegen::emit_with(p, &opts.codegen)?;
    if let Some(f) = &opts.emit_llvm {
        std::fs::write(f, &ir).map_err(|e| format!("cannot write `{}`: {}", f.display(), e))?;
    }
    let isa = opts.codegen.isa;
    let wasm = matches!(isa, isc_codegen::Isa::Wasm32 | isc_codegen::Isa::Wasm32Threads);
    let threads = isa == isc_codegen::Isa::Wasm32Threads;
    // (WebAssembly from an iscc on Windows too)
    let windows = opts.codegen.windows && !wasm;
    let obj = opts.output.with_extension("o");
    isc_codegen::backend::compile_for(&ir, &obj, opts.emit_asm.as_deref(), opts.codegen.isa, windows)?;
    // the embedded runtime, written next to the object file for the linker
    let (var, bytes, triple) = if windows {
        ("ISC_RUNTIME_WINDOWS", RUNTIME_WINDOWS, "x86_64-pc-windows-gnu")
    } else if threads {
        ("ISC_RUNTIME_WASM_THREADS", RUNTIME_WASM_THREADS, "wasm32-wasip1-threads")
    } else if wasm {
        ("ISC_RUNTIME_WASM", RUNTIME_WASM, "wasm32-wasip1")
    } else if runtime_v4(isa) {
        ("ISC_RUNTIME", RUNTIME_V4, "")
    } else if runtime_v3(isa) {
        ("ISC_RUNTIME", RUNTIME_V3, "")
    } else {
        ("ISC_RUNTIME", RUNTIME, "")
    };
    let (rt, temporary) = match std::env::var(var) {
        Ok(p) => (PathBuf::from(p), false),
        Err(_) => {
            if bytes.is_empty() {
                let _ = std::fs::remove_file(&obj);
                return Err(format!(
                    "this iscc has no runtime for {}: install the Rust target (`rustup target add {}`) and build iscc again",
                    triple, triple
                ));
            }
            let f = opts.output.with_extension("rt.a");
            std::fs::write(&f, bytes).map_err(|e| format!("cannot write `{}`: {}", f.display(), e))?;
            (f, true)
        }
    };
    let linked = if windows {
        link_windows(&obj, &rt, &opts.output, opts.debug, &opts.link)
    } else if wasm {
        link_wasm(&obj, &rt, &opts.output, opts.debug, triple, threads, &opts.link)
    } else {
        link(&obj, &rt, &opts.output, opts.debug, &opts.link)
    };
    let _ = std::fs::remove_file(&obj);
    if temporary {
        let _ = std::fs::remove_file(&rt);
    }
    linked
}

/// The runtime of compiled programs (`crates/isc-runtime`), built by
/// `build.rs` and embedded in `iscc`. `$ISC_RUNTIME` may name another
/// `libisc_runtime.a`.
const RUNTIME: &[u8] = include_bytes!(env!("ISC_RUNTIME_LIB"));

/// The same runtime built for x86-64-v3 (AVX2, BMI2, FMA, D188); the one
/// above when the host is not an x86-64.
const RUNTIME_V3: &[u8] = include_bytes!(env!("ISC_RUNTIME_V3_LIB"));

/// The same runtime built for x86-64-v4 (AVX-512, D241); the one above when
/// it could not be built.
const RUNTIME_V4: &[u8] = include_bytes!(env!("ISC_RUNTIME_V4_LIB"));

/// The runtime for x86-64-v4 when the target has its instructions: `native`
/// and `avx512` on a host with AVX-512 F, BW, CD, DQ and VL (not `avx2`: the
/// tests under valgrind, which has no AVX-512, ask for it, D241).
pub fn runtime_v4(isa: isc_codegen::Isa) -> bool {
    use isc_codegen::Isa;
    if !matches!(isa, Isa::Native | Isa::Avx512) || !runtime_v3(isa) {
        return false;
    }
    let host = isc_codegen::backend::host_features();
    let has = |f: &str| host.split(',').any(|x| x == f);
    ["+avx512f", "+avx512bw", "+avx512cd", "+avx512dq", "+avx512vl"].iter().all(|f| has(f))
}

/// The runtime for x86-64-v3 when the target has its instructions: `avx2`
/// and `avx512` are the host's CPU, and so is `native` (D188).
pub fn runtime_v3(isa: isc_codegen::Isa) -> bool {
    use isc_codegen::Isa;
    if !matches!(isa, Isa::Native | Isa::Avx2 | Isa::Avx512) || !cfg!(target_arch = "x86_64") {
        return false;
    }
    let host = isc_codegen::backend::host_features();
    let has = |f: &str| host.split(',').any(|x| x == f);
    ["+avx", "+avx2", "+bmi", "+bmi2", "+fma", "+lzcnt", "+movbe", "+f16c", "+popcnt", "+sse4.2", "+ssse3"].iter().all(|f| has(f))
}

/// The same runtime for WebAssembly (`wasm32-wasip1`); empty when the Rust
/// target was not installed when `iscc` was built. `$ISC_RUNTIME_WASM` may
/// name another one.
const RUNTIME_WASM: &[u8] = include_bytes!(env!("ISC_RUNTIME_WASM_LIB"));

/// The same runtime with threads (`wasm32-wasip1-threads`).
const RUNTIME_WASM_THREADS: &[u8] = include_bytes!(env!("ISC_RUNTIME_WASM_THREADS_LIB"));

/// The same runtime for Windows (`x86_64-pc-windows-gnu`, D246); empty when
/// the Rust target was not installed when `iscc` was built.
const RUNTIME_WINDOWS: &[u8] = include_bytes!(env!("ISC_RUNTIME_WINDOWS_LIB"));

/// The files of MinGW-w64 the linker needs for Windows (D246), under a
/// prefix like `/usr` (`x86_64-w64-mingw32/lib` and
/// `lib/gcc/x86_64-w64-mingw32/<version>`): `$ISC_MINGW_DIR`, else `/usr`.
fn mingw_dirs() -> Result<(PathBuf, PathBuf), String> {
    let root = PathBuf::from(std::env::var("ISC_MINGW_DIR").unwrap_or_else(|_| "/usr".into()));
    let lib = root.join("x86_64-w64-mingw32/lib");
    if !lib.join("crt2.o").is_file() {
        return Err(format!("no MinGW-w64 in `{}` (crt2.o): install it (Debian: mingw-w64-x86-64-dev, gcc-mingw-w64-x86-64-posix) or set ISC_MINGW_DIR", lib.display()));
    }
    let gcc_root = root.join("lib/gcc/x86_64-w64-mingw32");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(&gcc_root)
        .map_err(|e| format!("cannot read `{}`: {}", gcc_root.display(), e))?
        .filter_map(|d| d.ok())
        .map(|d| d.path())
        .filter(|p| p.join("libgcc.a").is_file())
        .collect();
    // the threads of POSIX or of Win32: the runtime uses neither, the
    // POSIX one first
    versions.sort_by_key(|p| (!p.to_string_lossy().ends_with("posix"), p.clone()));
    let gcc = versions.into_iter().next().ok_or_else(|| format!("no libgcc.a under `{}`", gcc_root.display()))?;
    Ok((lib, gcc))
}

// The files of MinGW-w64 embedded when iscc was built (build.rs, D247),
// the start files of glibc and the C library of WASI (D248).
include!(concat!(env!("OUT_DIR"), "/mingw_files.rs"));
include!(concat!(env!("OUT_DIR"), "/link_files.rs"));

/// The linker of Windows programs (D247), and whether it needs `-flavor
/// gnu` (`lld`, `rust-lld`): `$ISC_LLD`; else the `rust-lld` of Rust when
/// there is one (iscc on Linux); else `ld.lld` of LLVM, in the `PATH` or in
/// `C:\Program Files\LLVM\bin` (iscc on Windows: the LLVM it uses).
fn windows_linker() -> Result<(PathBuf, bool), String> {
    let flavor = |p: &Path| matches!(p.file_stem().and_then(|s| s.to_str()), Some("lld" | "rust-lld"));
    if let Ok(p) = std::env::var("ISC_LLD") {
        let p = PathBuf::from(p);
        let f = flavor(&p);
        return Ok((p, f));
    }
    if let Ok(sysroot) = rust_sysroot() {
        let rustlib = sysroot.join("lib/rustlib");
        if let Some(p) = std::fs::read_dir(&rustlib).ok().into_iter().flatten().filter_map(|d| d.ok()).map(|d| d.path().join("bin").join(if cfg!(windows) { "rust-lld.exe" } else { "rust-lld" })).find(|p| p.is_file()) {
            return Ok((p, true));
        }
    }
    let exe = if cfg!(windows) { "ld.lld.exe" } else { "ld.lld" };
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    if cfg!(windows) {
        dirs.push(PathBuf::from("C:\\Program Files\\LLVM\\bin"));
    }
    dirs.into_iter()
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
        .map(|p| (p, false))
        .ok_or_else(|| "no linker for Windows: install LLVM 21 (its ld.lld), or set ISC_LLD".to_string())
}

/// Writes `files` into the new directory `dir`, for the linker.
fn write_files(dir: &Path, files: &[(&str, &[u8])]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make `{}`: {}", dir.display(), e))?;
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).map_err(|e| format!("cannot write `{}`: {}", dir.join(name).display(), e))?;
    }
    Ok(())
}

/// Runs a linker on `args`: LLD in the process when iscc has it (D248, its
/// flavor given by `argv0`), else `external` (the program, and whether it
/// needs `-flavor flavor`).
fn run_linker(argv0: &str, flavor: &str, args: Vec<std::ffi::OsString>, external: &dyn Fn() -> Result<(PathBuf, bool), String>) -> Result<(), String> {
    if isc_codegen::lld::available() {
        let mut all = vec![std::ffi::OsString::from(argv0)];
        all.extend(args);
        return isc_codegen::lld::link(&all);
    }
    let (prog, needs_flavor) = external()?;
    let mut cmd = std::process::Command::new(&prog);
    if needs_flavor {
        cmd.args(["-flavor", flavor]);
    }
    let status = cmd.args(&args).status().map_err(|e| format!("cannot run the linker `{}`: {}", prog.display(), e))?;
    if !status.success() {
        return Err(format!("the linker `{}` failed ({})", prog.display(), status));
    }
    Ok(())
}

/// The path or `-lname` of a library given by `--link`.
fn lib_arg(l: &str) -> std::ffi::OsString {
    if l.contains('/') || l.contains('\\') || l.ends_with(".a") || l.ends_with(".o") || l.ends_with(".lib") || l.ends_with(".so") || l.contains(".so.") {
        l.into()
    } else {
        format!("-l{}", l).into()
    }
}

/// Links a program for Windows as the linker of MinGW (`-m i386pep`): the
/// start files and the libraries of MinGW-w64 (`msvcrt`, `kernel32`,
/// `synchronization` for `WaitOnAddress`), those embedded in iscc (written
/// next to the program for the linker), else those of `$ISC_MINGW_DIR`; a
/// console executable whose main thread has a stack of 8 MiB, as on Linux.
/// The linker: LLD in iscc (D248), else `windows_linker`.
fn link_windows(obj: &Path, rt: &Path, out: &Path, debug: bool, libs: &[String]) -> Result<(), String> {
    let tmp = out.with_extension("mingw");
    let (lib, gcc) = if MINGW_FILES.is_empty() {
        mingw_dirs()?
    } else {
        write_files(&tmp, MINGW_FILES)?;
        (tmp.clone(), tmp.clone())
    };
    let mut a: Vec<std::ffi::OsString> = ["-m", "i386pep", "--subsystem", "console", "--stack", "8388608", "--gc-sections", "-o"].iter().map(|x| x.into()).collect();
    a.push(out.into());
    a.push(lib.join("crt2.o").into());
    a.push(gcc.join("crtbegin.o").into());
    a.push(obj.into());
    a.extend(libs.iter().map(|l| lib_arg(l)));
    a.push(rt.into());
    a.extend(["-L".into(), lib.clone().into(), "-L".into(), gcc.clone().into(), "--start-group".into()]);
    for l in ["mingw32", "gcc", "gcc_eh", "mingwex", "msvcrt", "moldname", "kernel32", "synchronization"] {
        a.push(format!("-l{}", l).into());
    }
    a.push("--end-group".into());
    a.push(gcc.join("crtend.o").into());
    if !debug {
        a.push("-s".into());
    }
    let r = run_linker("ld.lld", "gnu", a, &windows_linker);
    if !MINGW_FILES.is_empty() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    r
}

/// The Rust sysroot, which holds the WebAssembly linker (`rust-lld`) and the
/// WASI C library of each target (`lib/rustlib/<target>/lib/self-contained`):
/// `$ISC_RUST_SYSROOT`, else `rustc --print sysroot`.
fn rust_sysroot() -> Result<PathBuf, String> {
    if let Ok(s) = std::env::var("ISC_RUST_SYSROOT") {
        return Ok(s.into());
    }
    let rustc = std::env::var("HOME").map(|h| PathBuf::from(h).join(".cargo/bin/rustc")).ok().filter(|p| p.is_file());
    let rustc = rustc.unwrap_or_else(|| "rustc".into());
    let o = std::process::Command::new(&rustc)
        .args(["--print", "sysroot"])
        .output()
        .map_err(|e| format!("cannot run `{}` to find the WebAssembly linker: {}", rustc.display(), e))?;
    Ok(String::from_utf8_lossy(&o.stdout).trim().into())
}

/// Links a WebAssembly program for WASI preview 1 (wasm-ld: LLD in iscc,
/// D248, else the `rust-lld` of Rust), the runtime and the wasi-libc of the
/// Rust target (`crt1-command.o`, `libc.a`: embedded in iscc, else those of
/// the Rust sysroot): the result is a WASI command module, run by any WASI
/// engine. With threads (wasi-threads), the memory is shared and imported,
/// and `wasi_thread_start` is exported, as the proposal requires.
fn link_wasm(obj: &Path, rt: &Path, out: &Path, debug: bool, triple: &str, threads: bool, libs: &[String]) -> Result<(), String> {
    let tag = if threads { "threads-" } else { "" };
    let embedded: Vec<(&str, &[u8])> = WASI_FILES.iter().filter(|(n, _)| n.strip_prefix(tag).map(|r| !r.starts_with("threads-") && (r == "libc.a" || r == "crt1-command.o")).unwrap_or(false)).map(|(n, b)| (n.strip_prefix(tag).unwrap(), *b)).collect();
    let tmp = out.with_extension("wasi");
    let libc_dir = if isc_codegen::lld::available() && embedded.len() == 2 {
        write_files(&tmp, &embedded)?;
        tmp.clone()
    } else {
        let d = rust_sysroot()?.join("lib/rustlib").join(triple).join("lib/self-contained");
        if !d.join("libc.a").is_file() {
            return Err(format!("no WASI C library in `{}`: rustup target add {}", d.display(), triple));
        }
        d
    };
    let mut a: Vec<std::ffi::OsString> = vec!["-o".into(), out.into(), libc_dir.join("crt1-command.o").into(), obj.into()];
    // the libraries of the `extern` functions and the C library before the
    // runtime, as natively (D125)
    a.extend(libs.iter().map(|l| lib_arg(l)));
    a.extend([libc_dir.join("libc.a").into(), rt.into(), libc_dir.join("libc.a").into()]);
    // the stack of the main thread: 8 MiB, as on Linux (wasm-ld gives 64 KiB)
    a.extend(["--gc-sections", "-z", "stack-size=8388608"].iter().map(|x| x.into()));
    if threads {
        a.extend(["--shared-memory", "--import-memory", "--export-memory", "--max-memory=4294967296", "--export=wasi_thread_start"].iter().map(|x| x.into()));
    }
    if !debug {
        a.push("--strip-all".into());
    }
    let external = || -> Result<(PathBuf, bool), String> {
        let rustlib = rust_sysroot()?.join("lib/rustlib");
        let lld = std::fs::read_dir(&rustlib)
            .map_err(|e| format!("cannot read `{}`: {}", rustlib.display(), e))?
            .filter_map(|d| d.ok())
            .map(|d| d.path().join("bin/rust-lld"))
            .find(|p| p.is_file())
            .ok_or_else(|| format!("no `rust-lld` under `{}`", rustlib.display()))?;
        Ok((lld, true))
    };
    let r = run_linker("wasm-ld", "wasm", a, &external);
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

/// The shared libraries of glibc of this machine, for programs linked by
/// LLD in iscc (D248): `libc.so.6`, `libm.so.6`, and `libpthread.so.0` when
/// it exists (its functions are in libc from glibc 2.34 on).
fn glibc_libraries() -> Result<Vec<PathBuf>, String> {
    let dirs = ["/lib/x86_64-linux-gnu", "/usr/lib/x86_64-linux-gnu", "/lib64", "/usr/lib64", "/usr/lib", "/lib"];
    let find = |name: &str| dirs.iter().map(|d| Path::new(d).join(name)).find(|p| p.is_file());
    let mut v = Vec::new();
    for name in ["libm.so.6", "libc.so.6"] {
        v.push(find(name).ok_or_else(|| format!("no `{}` on this machine: iscc links programs with the glibc of the system", name))?);
    }
    if let Some(p) = find("libpthread.so.0") {
        v.push(p);
    }
    Ok(v)
}

/// Links a program for Linux with LLD in iscc (D248): a position-independent
/// executable, the start files of glibc embedded in iscc, the `libc.so.6`
/// and `libm.so.6` of this machine (no development files needed): the
/// program uses the glibc of the system, as when linked by `cc`.
fn link_linux_lld(obj: &Path, rt: &Path, out: &Path, debug: bool, libs: &[String]) -> Result<(), String> {
    let sys = glibc_libraries()?;
    let tmp = out.with_extension("crt");
    write_files(&tmp, GLIBC_FILES)?;
    let mut a: Vec<std::ffi::OsString> = ["-pie", "-m", "elf_x86_64", "--hash-style=gnu", "--build-id", "--eh-frame-hdr", "-z", "relro", "-z", "now", "--dynamic-linker", "/lib64/ld-linux-x86-64.so.2", "--gc-sections", "--allow-shlib-undefined", "-o"].iter().map(|x| x.into()).collect();
    a.push(out.into());
    a.push(tmp.join("Scrt1.o").into());
    a.push(tmp.join("crti.o").into());
    a.push(obj.into());
    for d in ["/usr/local/lib", "/usr/lib/x86_64-linux-gnu", "/lib/x86_64-linux-gnu", "/usr/lib64", "/lib64", "/usr/lib", "/lib"] {
        if Path::new(d).is_dir() {
            a.extend(["-L".into(), d.into()]);
        }
    }
    // the libraries of the `extern` functions, then the C library, before
    // the runtime (D125)
    a.extend(libs.iter().map(|l| lib_arg(l)));
    a.push("--as-needed".into());
    a.extend(sys.iter().map(|p| p.into()));
    a.push(rt.into());
    a.extend(sys.iter().map(|p| p.into()));
    a.push("--no-as-needed".into());
    // `__libc_csu_init` and `__libc_csu_fini` of the `Scrt1.o` of a glibc
    // older than 2.34; from 2.34 on, `__libc_start_main` ignores them and
    // runs the constructors itself: they run once either way
    if tmp.join("libc_nonshared.a").is_file() {
        a.push(tmp.join("libc_nonshared.a").into());
    }
    a.push(tmp.join("crtn.o").into());
    if !debug {
        a.push("-s".into());
    }
    let r = isc_codegen::lld::link(&{
        let mut all = vec![std::ffi::OsString::from("ld.lld")];
        all.extend(a);
        all
    });
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

/// Links the program with the runtime (which needs only the C library, and
/// `libm` for `fmod`). Unused sections are dropped and, unless `debug`, the
/// symbols are stripped: the code that runs is the same, the file is smaller.
fn link(obj: &Path, rt: &Path, out: &Path, debug: bool, libs: &[String]) -> Result<(), String> {
    // LLD in iscc and the start files of glibc: no compiler needed (D248);
    // `$ISC_CC` asks for a C compiler as the linker all the same
    if isc_codegen::lld::available() && !GLIBC_FILES.is_empty() && std::env::var("ISC_CC").is_err() {
        return link_linux_lld(obj, rt, out, debug, libs);
    }
    let cc = std::env::var("ISC_CC").unwrap_or_else(|_| "cc".into());
    let mut cmd = std::process::Command::new(&cc);
    cmd.arg("-o").arg(out).arg(obj).arg("-Wl,--gc-sections");
    // the libraries of the `extern` functions, then the C library, before
    // the runtime: the runtime's archive holds weak copies of C functions
    // (Rust's `compiler_builtins`, `cbrt` among them) that would otherwise
    // replace those of the C library for an `extern` (D125)
    for l in libs {
        if l.contains('/') || l.ends_with(".so") || l.ends_with(".a") || l.ends_with(".o") {
            cmd.arg(l);
        } else {
            cmd.arg(format!("-l{}", l));
        }
    }
    cmd.args(["-lm", "-lc"]).arg(rt);
    if !debug {
        cmd.arg("-s");
    }
    let status = cmd
        .args(["-lm", "-lc"])
        .status()
        .map_err(|e| format!("cannot run the linker `{}`: {}", cc, e))?;
    if !status.success() {
        return Err(format!("the linker `{}` failed ({})", cc, status));
    }
    Ok(())
}
