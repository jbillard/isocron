#!/usr/bin/env bash
# Builds iscc for Windows on Linux (D247): target/x86_64-pc-windows-gnu/release/iscc.exe.
#   ISC_MINGW_DIR    MinGW-w64 under a prefix like /usr (its gcc is the linker of iscc.exe,
#                    its files are embedded for the programs iscc links)
#   ISC_LLVM_LIB_DIR the directory of LLVM-C.lib (lib\ of the official LLVM 21 for Windows)
# The Rust target: rustup target add x86_64-pc-windows-gnu. On Windows, iscc.exe needs
# LLVM 21 (LLVM-C.dll and ld.lld.exe in the PATH, or in C:\Program Files\LLVM\bin).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(dirname "$(dirname "$here")")"
mingw="${ISC_MINGW_DIR:-/usr}"
: "${ISC_LLVM_LIB_DIR:?the directory of LLVM-C.lib}"
gcc=""
for g in "$mingw"/bin/x86_64-w64-mingw32-gcc-posix "$mingw"/bin/x86_64-w64-mingw32-gcc; do
    if [ -x "$g" ]; then gcc="$g"; break; fi
done
[ -n "$gcc" ] || { echo "no x86_64-w64-mingw32-gcc in $mingw/bin" >&2; exit 1; }
export ISC_MINGW_DIR="$mingw" ISC_LLVM_LIB_DIR CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$gcc"
cd "$root"
cargo build --release -p iscc --bin iscc --target x86_64-pc-windows-gnu
echo "$root/target/x86_64-pc-windows-gnu/release/iscc.exe"
