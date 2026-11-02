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
    let build = |triple: Option<&str>| -> bool {
        let mut cmd = Command::new(&cargo);
        cmd.args(["build", "--release", "--quiet", "--manifest-path"]).arg(&manifest).arg("--target-dir").arg(&target);
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
        cmd
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("CARGO_BUILD_TARGET")
            .status()
            .expect("cannot run cargo to build the runtime")
            .success()
    };
    assert!(build(None), "building crates/isc-runtime failed");
    let lib = target.join("release").join("libisc_runtime.a");
    assert!(lib.exists(), "{} was not produced", lib.display());
    println!("cargo:rustc-env=ISC_RUNTIME_LIB={}", lib.display());
    // the runtimes for `iscc build --target wasm32-wasi` and
    // `wasm32-wasi-threads`, when the Rust targets are installed (`rustup
    // target add wasm32-wasip1 wasm32-wasip1-threads`); otherwise an empty
    // file, and `iscc` says what is missing
    for (triple, var) in [("wasm32-wasip1", "ISC_RUNTIME_WASM_LIB"), ("wasm32-wasip1-threads", "ISC_RUNTIME_WASM_THREADS_LIB")] {
        let lib = target.join(triple).join("release").join("libisc_runtime.a");
        if !build(Some(triple)) || !lib.exists() {
            let empty = PathBuf::from(env::var("OUT_DIR").unwrap()).join(format!("no-runtime-{}.a", triple));
            std::fs::write(&empty, b"").unwrap();
            println!("cargo:warning=no {} runtime: rustup target add {}", triple, triple);
            println!("cargo:rustc-env={}={}", var, empty.display());
        } else {
            println!("cargo:rustc-env={}={}", var, lib.display());
        }
    }
}
