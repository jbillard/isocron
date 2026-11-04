//! Builds the runtime of compiled programs (`crates/isc-runtime`, without the
//! standard library, hence with `panic = "abort"`, in its own workspace) and
//! hands its path to `src/lib.rs`, which embeds it: `iscc` needs no file at
//! run time to link a program.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let here = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let manifest = here.join("../isc-runtime/Cargo.toml");
    let target = PathBuf::from(env::var("OUT_DIR").unwrap()).join("runtime");
    for p in ["../isc-runtime/src", "../isc-runtime/Cargo.toml", "../isc-runtime-core/src", "../isc-runtime-core/Cargo.toml", "../isc-mem/src", "../isc-mem/Cargo.toml"] {
        println!("cargo:rerun-if-changed={}", p);
    }
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    // the runtimes of native code are those of the target of iscc (D247:
    // an iscc for Windows built on Linux embeds the runtime of Windows)
    let native = env::var("TARGET").unwrap();
    let build_in = |triple: Option<&str>, dir: &PathBuf, flags: Option<&str>| -> bool {
        let mut cmd = Command::new(&cargo);
        cmd.args(["build", "--release", "--quiet", "--manifest-path"]).arg(&manifest).arg("--target-dir").arg(dir);
        if let Some(t) = triple {
            cmd.args(["--target", t]);
        }
        // the flags of the outer build do not apply to the runtime; the
        // threads target is told apart by `cfg(isc_wasm_threads)`: rustc
        // gives `wasm32-wasip1` and `wasm32-wasip1-threads` the same `cfg`
        // (`atomics` is an unstable target feature, never in `cfg` on stable)
        cmd.env_remove("CARGO_ENCODED_RUSTFLAGS");
        let mut rustflags: Vec<&str> = Vec::new();
        if triple == Some("wasm32-wasip1-threads") {
            rustflags.push("--cfg=isc_wasm_threads");
        }
        if let Some(f) = flags {
            rustflags.push(f);
        }
        // Functions and loops of the runtime on 64 bytes (D256): the place
        // of its hot loops in the windows of the decoders no longer depends
        // on the code before them (regex-redux 1.4 % slower when another
        // module was added to the runtime, with no change to its own code).
        rustflags.push("-Cllvm-args=-align-all-functions=6");
        rustflags.push("-Cllvm-args=-align-loops=64");
        // Rust puts the code of a crate in one section on the targets
        // `*-windows-gnu`: the linker then keeps all of the runtime or
        // nothing (a hello of 1.5 MB). One section per function, so that
        // only the functions used stay (D250); the flag is unstable,
        // allowed on a stable compiler for this build only.
        if triple.map(|t| t.contains("windows-gnu")).unwrap_or(false) {
            rustflags.push("-Zfunction-sections=yes");
            cmd.env("RUSTC_BOOTSTRAP", "1");
        }
        if !rustflags.is_empty() {
            cmd.env("CARGO_ENCODED_RUSTFLAGS", rustflags.join("\x1f"));
        }
        cmd
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("CARGO_BUILD_TARGET")
            .status()
            .expect("cannot run cargo to build the runtime")
            .success()
    };
    let build = |triple: Option<&str>| build_in(triple, &target, None);
    assert!(build(Some(&native)), "building crates/isc-runtime failed");
    let lib = target.join(&native).join("release").join("libisc_runtime.a");
    assert!(lib.exists(), "{} was not produced", lib.display());
    println!("cargo:rustc-env=ISC_RUNTIME_LIB={}", lib.display());
    // the same runtime for x86-64-v3 (AVX2, BMI2, FMA): the searches of
    // `memchr` and of the literals of regular expressions (Teddy) in AVX2,
    // which the runtime cannot detect at run time without the standard
    // library (D188); linked for the targets that have them
    let v3_dir = PathBuf::from(env::var("OUT_DIR").unwrap()).join("runtime-v3");
    let v3 = v3_dir.join(&native).join("release").join("libisc_runtime.a");
    let x86 = env::var("CARGO_CFG_TARGET_ARCH").map(|a| a == "x86_64").unwrap_or(false);
    let v3_built = x86 && build_in(Some(&native), &v3_dir, Some("-Ctarget-cpu=x86-64-v3")) && v3.exists();
    if v3_built {
        println!("cargo:rustc-env=ISC_RUNTIME_V3_LIB={}", v3.display());
    } else {
        println!("cargo:rustc-env=ISC_RUNTIME_V3_LIB={}", lib.display());
    }
    // and for x86-64-v4 (AVX-512): the masks of 64 bytes of the searches of
    // regular expressions in one comparison (D241)
    let v4_dir = PathBuf::from(env::var("OUT_DIR").unwrap()).join("runtime-v4");
    let v4 = v4_dir.join(&native).join("release").join("libisc_runtime.a");
    if v3_built && build_in(Some(&native), &v4_dir, Some("-Ctarget-cpu=x86-64-v4")) && v4.exists() {
        println!("cargo:rustc-env=ISC_RUNTIME_V4_LIB={}", v4.display());
    } else if v3_built {
        println!("cargo:rustc-env=ISC_RUNTIME_V4_LIB={}", v3.display());
    } else {
        println!("cargo:rustc-env=ISC_RUNTIME_V4_LIB={}", lib.display());
    }
    // the runtimes for `iscc build --target wasm32-wasi` and
    // `wasm32-wasi-threads`, when the Rust targets are installed (`rustup
    // target add wasm32-wasip1 wasm32-wasip1-threads`); otherwise an empty
    // file, and `iscc` says what is missing
    // A target that is installed must build: a runtime that no longer
    // compiles for it stops the build here, rather than leaving an empty
    // file that only the tests of WebAssembly would notice.
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let sysroot = Command::new(&rustc)
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim().to_string()));
    // and the runtime for `--target x86_64-windows` (D246)
    for (triple, var) in [
        ("wasm32-wasip1", "ISC_RUNTIME_WASM_LIB"),
        ("wasm32-wasip1-threads", "ISC_RUNTIME_WASM_THREADS_LIB"),
        ("x86_64-pc-windows-gnu", "ISC_RUNTIME_WINDOWS_LIB"),
    ] {
        let lib = target.join(triple).join("release").join("libisc_runtime.a");
        let installed = sysroot.as_ref().map(|r| r.join("lib").join("rustlib").join(triple).exists()).unwrap_or(false);
        let built = build(Some(triple)) && lib.exists();
        assert!(built || !installed, "building crates/isc-runtime for {} failed (the target is installed)", triple);
        if !built {
            let empty = PathBuf::from(env::var("OUT_DIR").unwrap()).join(format!("no-runtime-{}.a", triple));
            std::fs::write(&empty, b"").unwrap();
            println!("cargo:warning=no {} runtime: rustup target add {}", triple, triple);
            println!("cargo:rustc-env={}={}", var, empty.display());
        } else {
            println!("cargo:rustc-env={}={}", var, lib.display());
        }
    }
    mingw_files();
    link_files(sysroot.as_ref());
}

/// The other files a linker of its own needs (D248), embedded: the start
/// files of glibc (`Scrt1.o`, `crti.o`, `crtn.o`, `libc_nonshared.a`, from `$ISC_GLIBC_DIR`, else
/// the usual directories; programs are then linked with the `libc.so.6` of
/// the machine), and the C library of WASI of each WebAssembly target (in
/// the Rust sysroot). An empty list for what is not found.
fn link_files(sysroot: Option<&PathBuf>) {
    println!("cargo:rerun-if-env-changed=ISC_GLIBC_DIR");
    let mut dirs: Vec<PathBuf> = env::var("ISC_GLIBC_DIR").ok().map(PathBuf::from).into_iter().collect();
    dirs.extend(["/usr/lib/x86_64-linux-gnu", "/usr/lib64", "/usr/lib"].iter().map(PathBuf::from));
    let glibc = dirs.into_iter().find(|d| ["Scrt1.o", "crti.o", "crtn.o"].iter().all(|f| d.join(f).is_file()));
    let entry = |name: &str, f: &PathBuf| format!("    ({:?}, include_bytes!({:?})),", name, f.display().to_string());
    // and `libc_nonshared.a`: before glibc 2.34, `Scrt1.o` calls its
    // `__libc_csu_init` and `__libc_csu_fini`
    let glibc_entries: Vec<String> = match &glibc {
        Some(d) => ["Scrt1.o", "crti.o", "crtn.o", "libc_nonshared.a"].iter().filter(|f| d.join(f).is_file()).map(|f| entry(f, &d.join(f))).collect(),
        None => Vec::new(),
    };
    let mut wasi = Vec::new();
    for (triple, tag) in [("wasm32-wasip1", ""), ("wasm32-wasip1-threads", "threads-")] {
        if let Some(r) = sysroot {
            let d = r.join("lib/rustlib").join(triple).join("lib/self-contained");
            if d.join("libc.a").is_file() && d.join("crt1-command.o").is_file() {
                wasi.push(entry(&format!("{}crt1-command.o", tag), &d.join("crt1-command.o")));
                wasi.push(entry(&format!("{}libc.a", tag), &d.join("libc.a")));
            }
        }
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("link_files.rs");
    std::fs::write(
        &out,
        format!(
            "/// The start files of glibc for the linker of iscc (build.rs).\npub const GLIBC_FILES: &[(&str, &[u8])] = &[\n{}\n];\n/// The C library of WASI, `threads-` for wasm32-wasip1-threads.\npub const WASI_FILES: &[(&str, &[u8])] = &[\n{}\n];\n",
            glibc_entries.join("\n"),
            wasi.join("\n")
        ),
    )
    .unwrap();
}

/// The files of MinGW-w64 that the linker of Windows programs needs (D247),
/// from `$ISC_MINGW_DIR` (else `/usr`) when iscc is built: embedded, so that
/// an iscc on Windows needs only LLVM, and on Linux no MinGW to build for
/// Windows. Without them, an empty list: iscc then reads them from
/// `$ISC_MINGW_DIR` when it links.
fn mingw_files() {
    const LIB: [&str; 7] = ["crt2.o", "libmingw32.a", "libmingwex.a", "libmsvcrt.a", "libkernel32.a", "libsynchronization.a", "libmoldname.a"];
    const GCC: [&str; 4] = ["crtbegin.o", "crtend.o", "libgcc.a", "libgcc_eh.a"];
    println!("cargo:rerun-if-env-changed=ISC_MINGW_DIR");
    let root = PathBuf::from(env::var("ISC_MINGW_DIR").unwrap_or_else(|_| "/usr".into()));
    let lib = root.join("x86_64-w64-mingw32/lib");
    let gcc = std::fs::read_dir(root.join("lib/gcc/x86_64-w64-mingw32"))
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|d| d.ok())
        .map(|d| d.path())
        .filter(|p| p.join("libgcc.a").is_file())
        .min_by_key(|p| (!p.to_string_lossy().ends_with("posix"), p.clone()));
    let mut entries = Vec::new();
    if let Some(gcc) = gcc {
        let files: Vec<PathBuf> = LIB.iter().map(|f| lib.join(f)).chain(GCC.iter().map(|f| gcc.join(f))).collect();
        if files.iter().all(|f| f.is_file()) {
            for f in &files {
                println!("cargo:rerun-if-changed={}", f.display());
                entries.push(format!("    ({:?}, include_bytes!({:?})),", f.file_name().unwrap().to_string_lossy(), f.display().to_string()));
            }
        }
    }
    if entries.is_empty() {
        println!("cargo:warning=no MinGW-w64 in {}: the Windows target will read it from ISC_MINGW_DIR", root.display());
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("mingw_files.rs");
    std::fs::write(&out, format!("/// The files of MinGW-w64 for the linker (build.rs).\npub const MINGW_FILES: &[(&str, &[u8])] = &[\n{}\n];\n", entries.join("\n"))).unwrap();
}
