#!/usr/bin/env bash
# Builds the programs of the study of V (SPEC section 10; DECISIONS D55, D56)
# into bench/bin-lanes/:
#   <impl>_<kernel>_v<V>_<isa>   impl: isc, c-gcc, c-clang, rust, zig (the
#                                 hand-optimized fixed form), kernel: sum, dot,
#                                 std_dev, V: 16, 32, 64, isa: sse, avx2, avx512
# sse:    SSE4.2 without AVX, the code of the Celeron J3455 of the POC (CPU
#         `goldmont` for every compiler), run on this machine
# avx2:   no AVX-512 instruction at all
# avx512: 512-bit vectors allowed and preferred
# Go has no SIMD (without assembly): V does not change its code, it is left out.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(dirname "$here")"
out="$here/bin-lanes"
build="$here/build/lanes"
mkdir -p "$out" "$build"
export PATH="$HOME/.cargo/bin:$PATH"
ZIG="${ZIG:-$(command -v zig || echo "$HOME/.local/opt/zig-x86_64-linux-0.14.1/zig")}"
export ZIG_GLOBAL_CACHE_DIR="$here/build/zig-cache" ZIG_LOCAL_CACHE_DIR="$here/build/zig-cache"
kernels="sum dot std_dev"
lanes="${LANES_LIST:-16 32 64}"
strict="-O3 -march=native -ffp-contract=off -std=c11 -D_POSIX_C_SOURCE=200809L"

isas="${ISA_LIST:-sse avx2 avx512}"

c_isa() {
    case "$1" in
    sse) echo "-march=goldmont" ;;
    avx2) echo "-mno-avx512f" ;;
    *) echo "-mprefer-vector-width=512" ;;
    esac
}
rust_isa() {
    case "$1" in
    sse) echo "-C target-cpu=goldmont" ;;
    avx2) echo "-C target-cpu=native -C target-feature=-avx512f" ;;
    *) echo "-C target-cpu=native -C target-feature=-prefer-256-bit" ;;
    esac
}
zig_isa() {
    case "$1" in
    sse) echo "goldmont" ;;
    avx2) echo "native-avx512f" ;;
    *) echo "native-prefer_256_bit" ;;
    esac
}

(cd "$root" && cargo build --release -q)
for V in $lanes; do
    for isa in $isas; do
        tag="v${V}_$isa"
        echo "== V=$V $isa"
        for k in $kernels; do
            "$root/target/release/iscc" build "$root/examples/poc/$k.isc" -o "$out/isc_${k}_$tag" --lanes "$V" --isa "$isa"
            gcc $strict $(c_isa $isa) -DVARIANT=1 -DLANES="$V" -o "$out/c-gcc_${k}_$tag" "$here/c/$k.c" -lm
            "$ZIG" cc -target x86_64-linux-gnu $strict $(c_isa $isa) -DVARIANT=1 -DLANES="$V" -o "$out/c-clang_${k}_$tag" "$here/c/$k.c" -lm
        done
        # Rust: V is a compile-time constant read from ISC_LANES
        (cd "$here/rust" && ISC_LANES="$V" RUSTFLAGS="$(rust_isa $isa)" \
            cargo build --release -q --target-dir "$build/rust-$tag" 2>&1 | grep -v "unknown and unstable feature\|prefer-256-bit\|^ *=\|^$" || true)
        for k in $kernels; do cp "$build/rust-$tag/release/${k}_opt" "$out/rust_${k}_$tag"; done
        # Zig: V comes from lanes.zig, rewritten in a copy of the sources
        zdir="$build/zig-v$V"
        mkdir -p "$zdir"
        cp "$here"/zig/*.zig "$zdir/"
        printf 'pub const lanes = %s;\n' "$V" > "$zdir/lanes.zig"
        for k in $kernels; do
            (cd "$zdir" && "$ZIG" build-exe -O ReleaseFast -mcpu="$(zig_isa $isa)" -femit-bin="$zdir/zig_${k}_$tag" "${k}_opt.zig")
            cp "$zdir/zig_${k}_$tag" "$out/zig_${k}_$tag"
        done
    done
done
echo "built $(ls "$out" | wc -l) programs in $out"
