#!/usr/bin/env bash
# Builds LLVM and LLD (21.1.8) as static libraries (D248), in the image of
# tools/release/<platform>/Dockerfile:
#   build_llvm.sh linux|windows SRC OUT
# SRC: the sources (llvm-project-21.1.8.src); OUT: the libraries, headers and
# llvm-config, and the build directory OUT.build. Only what iscc needs: the
# targets x86 and WebAssembly, LLD (ELF, COFF, MinGW, Wasm); no compression,
# XML, line editing, terminfo or RTTI; no tests, no tools but those of the
# build. For Windows, cross-compiled by MinGW-w64 (a native tablegen first).
set -euo pipefail
platform=$1
src=$2
out=$3
jobs=${JOBS:-$(nproc)}
args=(-G Ninja -S "$src/llvm" -B "$out.build" -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$out"
    -DLLVM_ENABLE_PROJECTS=lld "-DLLVM_TARGETS_TO_BUILD=X86;WebAssembly"
    -DLLVM_ENABLE_ZLIB=OFF -DLLVM_ENABLE_ZSTD=OFF -DLLVM_ENABLE_LIBXML2=OFF -DLLVM_ENABLE_LIBEDIT=OFF
    -DLLVM_ENABLE_LIBPFM=OFF -DLLVM_ENABLE_Z3_SOLVER=OFF -DLLVM_ENABLE_FFI=OFF -DLLVM_ENABLE_TERMINFO=OFF
    -DLLVM_ENABLE_BINDINGS=OFF -DLLVM_ENABLE_ASSERTIONS=OFF -DLLVM_ENABLE_RTTI=OFF
    -DLLVM_INCLUDE_TESTS=OFF -DLLVM_INCLUDE_EXAMPLES=OFF -DLLVM_INCLUDE_BENCHMARKS=OFF -DLLVM_INCLUDE_DOCS=OFF
    -DLLVM_BUILD_TOOLS=OFF -DLLD_BUILD_TOOLS=OFF)
case "$platform" in
    linux) ;;
    windows)
        args+=(-DCMAKE_SYSTEM_NAME=Windows -DCMAKE_C_COMPILER=x86_64-w64-mingw32-gcc-posix
            -DCMAKE_CXX_COMPILER=x86_64-w64-mingw32-g++-posix -DCMAKE_RC_COMPILER=x86_64-w64-mingw32-windres
            -DCMAKE_FIND_ROOT_PATH=/usr/x86_64-w64-mingw32 -DLLVM_HOST_TRIPLE=x86_64-w64-windows-gnu)
        ;;
    *) echo "usage: build_llvm.sh linux|windows SRC OUT" >&2; exit 2 ;;
esac
cmake "${args[@]}"
ninja -C "$out.build" -j"$jobs" install
# the libraries iscc links, in the order of their dependencies (the
# llvm-config of the build machine: the same components for both)
if [ "$platform" = linux ]; then
    ninja -C "$out.build" llvm-config
    mkdir -p "$out/bin"
    cp "$out.build/bin/llvm-config" "$out/bin/"
    "$out/bin/llvm-config" --libnames all > "$out/llvm-libs.txt"
fi
ls "$out/lib" | grep '^liblld.*\.a$' > "$out/lld-libs.txt"
echo "built: $(ls "$out/lib" | grep -c '\.a$') static libraries in $out/lib"
