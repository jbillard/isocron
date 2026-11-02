// Links the LLVM 21 shared library (C API). Override the directory with
// ISC_LLVM_LIB_DIR and the library name with ISC_LLVM_LIB.
fn main() {
    let dir = std::env::var("ISC_LLVM_LIB_DIR").unwrap_or_else(|_| "/usr/lib/x86_64-linux-gnu".into());
    let lib = std::env::var("ISC_LLVM_LIB").unwrap_or_else(|_| "LLVM-21".into());
    println!("cargo:rustc-link-search=native={}", dir);
    println!("cargo:rustc-link-lib=dylib={}", lib);
    println!("cargo:rerun-if-env-changed=ISC_LLVM_LIB_DIR");
    println!("cargo:rerun-if-env-changed=ISC_LLVM_LIB");
}
