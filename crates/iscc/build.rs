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
    for p in ["../isc-runtime/src", "../isc-runtime/Cargo.toml", "../isc-runtime-core/src", "../isc-runtime-core/Cargo.toml"] {
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
        if triple == Some("wasm32-wasip1-threads") {
            cmd.env("CARGO_ENCODED_RUSTFLAGS", "--cfg=isc_wasm_threads");
        }
        if let Some(f) = flags {
            cmd.env("CARGO_ENCODED_RUSTFLAGS", f);
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
