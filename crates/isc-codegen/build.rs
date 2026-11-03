// Links LLVM 21 (C API). By default the shared library: override the
// directory with ISC_LLVM_LIB_DIR and the library name with ISC_LLVM_LIB;
// for Windows (D247) `LLVM-C` (`LLVM-C.lib`, the import library of
// `LLVM-C.dll` of the official LLVM for Windows).
//
// With ISC_LLVM_STATIC (D248), the static libraries of LLVM and LLD built by
// tools/release/build_llvm.sh (`lib`, `include`, `llvm-libs.txt`,
// `lld-libs.txt`), the C++ library of the compiler linked statically, and
// LLD called in the process (`cfg(isc_lld)`, `src/lld_shim.cpp` built by
// `$ISC_CXX`): an iscc that needs no LLVM and no linker.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=ISC_LLVM_LIB_DIR");
    println!("cargo:rerun-if-env-changed=ISC_LLVM_LIB");
    println!("cargo:rerun-if-env-changed=ISC_LLVM_STATIC");
    println!("cargo:rerun-if-env-changed=ISC_CXX");
    println!("cargo:rerun-if-changed=src/lld_shim.cpp");
    println!("cargo::rustc-check-cfg=cfg(isc_lld)");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").map(|o| o == "windows").unwrap_or(false);
    if let Ok(prefix) = std::env::var("ISC_LLVM_STATIC") {
        static_llvm(&PathBuf::from(prefix), windows);
        return;
    }
    let (dir, name) = if windows { ("C:\\Program Files\\LLVM\\lib", "LLVM-C") } else { ("/usr/lib/x86_64-linux-gnu", "LLVM-21") };
    let dir = std::env::var("ISC_LLVM_LIB_DIR").unwrap_or_else(|_| dir.into());
    let lib = std::env::var("ISC_LLVM_LIB").unwrap_or_else(|_| name.into());
    println!("cargo:rustc-link-search=native={}", dir);
    println!("cargo:rustc-link-lib=dylib={}", lib);
}

fn names(file: &PathBuf) -> Vec<String> {
    std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", file.display(), e))
        .split_whitespace()
        .map(|n| n.trim_start_matches("lib").trim_end_matches(".a").to_string())
        .collect()
}

fn static_llvm(prefix: &PathBuf, windows: bool) {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let cxx = std::env::var("ISC_CXX").unwrap_or_else(|_| if windows { "x86_64-w64-mingw32-g++-posix".into() } else { "c++".into() });
    let ar = if windows { "x86_64-w64-mingw32-ar".to_string() } else { "ar".to_string() };
    // the shim: its flags those of LLVM (C++17, no RTTI)
    let obj = out.join("lld_shim.o");
    let ok = Command::new(&cxx)
        .args(["-std=c++17", "-O2", "-fPIC", "-fno-rtti", "-fno-exceptions", "-c", "src/lld_shim.cpp", "-o"])
        .arg(&obj)
        .arg("-I")
        .arg(prefix.join("include"))
        .status()
        .expect("cannot run the C++ compiler (ISC_CXX)")
        .success();
    assert!(ok, "building src/lld_shim.cpp failed");
    let lib = out.join("libisc_lld.a");
    let _ = std::fs::remove_file(&lib);
    assert!(Command::new(&ar).arg("crs").arg(&lib).arg(&obj).status().expect("cannot run ar").success());
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native={}", prefix.join("lib").display());
    println!("cargo:rustc-link-lib=static:-bundle=isc_lld");
    // LLD's drivers before what they use, then LLVM in the order of
    // llvm-config (each library before those it needs)
    let lld = names(&prefix.join("lld-libs.txt"));
    for l in ["lldMinGW", "lldCOFF", "lldELF", "lldWasm", "lldCommon"] {
        assert!(lld.iter().any(|x| x == l), "no {} in {}", l, prefix.display());
        println!("cargo:rustc-link-lib=static:-bundle={}", l);
    }
    for l in names(&prefix.join("llvm-libs.txt")) {
        println!("cargo:rustc-link-lib=static:-bundle={}", l);
    }
    // the C++ library of the compiler, statically
    let o = Command::new(&cxx).arg("-print-file-name=libstdc++.a").output().expect("cannot run the C++ compiler");
    let stdcxx = PathBuf::from(String::from_utf8_lossy(&o.stdout).trim());
    if let Some(d) = stdcxx.parent() {
        println!("cargo:rustc-link-search=native={}", d.display());
    }
    println!("cargo:rustc-link-lib=static:-bundle=stdc++");
    if windows {
        for l in ["psapi", "shell32", "ole32", "uuid", "advapi32", "ws2_32", "ntdll"] {
            println!("cargo:rustc-link-lib=dylib={}", l);
        }
    } else {
        for l in ["pthread", "dl", "m", "rt"] {
            println!("cargo:rustc-link-lib=dylib={}", l);
        }
    }
    println!("cargo:rustc-cfg=isc_lld");
}
