// Links the LLVM 21 shared library (C API). Override the directory with
// ISC_LLVM_LIB_DIR and the library name with ISC_LLVM_LIB. For Windows
// (D247): `LLVM-C` (`LLVM-C.lib`, the import library of `LLVM-C.dll` of the
// official LLVM for Windows), in `C:\Program Files\LLVM\lib` by default.
fn main() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").map(|o| o == "windows").unwrap_or(false);
    let (dir, name) = if windows { ("C:\\Program Files\\LLVM\\lib", "LLVM-C") } else { ("/usr/lib/x86_64-linux-gnu", "LLVM-21") };
    let dir = std::env::var("ISC_LLVM_LIB_DIR").unwrap_or_else(|_| dir.into());
    let lib = std::env::var("ISC_LLVM_LIB").unwrap_or_else(|_| name.into());
    println!("cargo:rustc-link-search=native={}", dir);
    println!("cargo:rustc-link-lib=dylib={}", lib);
    println!("cargo:rerun-if-env-changed=ISC_LLVM_LIB_DIR");
    println!("cargo:rerun-if-env-changed=ISC_LLVM_LIB");
}
