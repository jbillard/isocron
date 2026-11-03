#!/usr/bin/env bash
# Builds an iscc that needs nothing else (D248), in the image of
# tools/release/<platform>/Dockerfile, from the sources mounted at /src:
#   build_iscc.sh linux|windows LLVM OUT
# LLVM: the static LLVM and LLD of build_llvm.sh for that platform; OUT: the
# directory of the cargo build. For Linux, the MinGW-w64 files embedded for
# --target x86_64-windows come from $ISC_MINGW_DIR.
set -euo pipefail
platform=$1
llvm=$2
out=$3
export ISC_LLVM_STATIC="$llvm"
cd /src
case "$platform" in
    linux)
        export ISC_GLIBC_DIR=/usr/lib64
        CARGO_PROFILE_RELEASE_STRIP=symbols cargo build --release --locked -p iscc --bin iscc --target-dir "$out"
        echo "$out/release/iscc"
        ;;
    windows)
        export ISC_MINGW_DIR=/usr CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc-posix
        CARGO_PROFILE_RELEASE_STRIP=symbols cargo build --release --locked -p iscc --bin iscc --target x86_64-pc-windows-gnu --target-dir "$out"
        echo "$out/x86_64-pc-windows-gnu/release/iscc.exe"
        ;;
    *) echo "usage: build_iscc.sh linux|windows LLVM OUT" >&2; exit 2 ;;
esac
