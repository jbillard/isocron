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
    assert!(build(None), "building crates/isc-runtime failed");
    let lib = target.join("release").join("libisc_runtime.a");
    assert!(lib.exists(), "{} was not produced", lib.display());
    println!("cargo:rustc-env=ISC_RUNTIME_LIB={}", lib.display());
    // the same runtime for x86-64-v3 (AVX2, BMI2, FMA): the searches of
    // `memchr` and of the literals of regular expressions (Teddy) in AVX2,
    // which the runtime cannot detect at run time without the standard
    // library (D188); linked for the targets that have them
    let v3_dir = PathBuf::from(env::var("OUT_DIR").unwrap()).join("runtime-v3");
    let v3 = v3_dir.join("release").join("libisc_runtime.a");
    let x86 = env::var("CARGO_CFG_TARGET_ARCH").map(|a| a == "x86_64").unwrap_or(false);
    let v3_built = x86 && build_in(None, &v3_dir, Some("-Ctarget-cpu=x86-64-v3")) && v3.exists();
    if v3_built {
        println!("cargo:rustc-env=ISC_RUNTIME_V3_LIB={}", v3.display());
    } else {
        println!("cargo:rustc-env=ISC_RUNTIME_V3_LIB={}", lib.display());
    }
    // and for x86-64-v4 (AVX-512): the masks of 64 bytes of the searches of
    // regular expressions in one comparison (D241)
    let v4_dir = PathBuf::from(env::var("OUT_DIR").unwrap()).join("runtime-v4");
    let v4 = v4_dir.join("release").join("libisc_runtime.a");
    if v3_built && build_in(None, &v4_dir, Some("-Ctarget-cpu=x86-64-v4")) && v4.exists() {
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
    for (triple, var) in [("wasm32-wasip1", "ISC_RUNTIME_WASM_LIB"), ("wasm32-wasip1-threads", "ISC_RUNTIME_WASM_THREADS_LIB")] {
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
}
