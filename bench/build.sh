#!/usr/bin/env bash
# Builds every benchmark program of SPEC section 14 into bench/bin/.
#   isc_<kernel>                       Isocron (iscc, release build)
#   c-gcc_<kernel>_<variant>           -O3 -march=native -ffp-contract=off
#   c-clang_<kernel>_<variant>         same flags, clang 19 (zig cc, DECISIONS D2)
#   c-gcc-reassoc_<kernel>_idiomatic   -O3 -march=native -fassociative-math -fno-signed-zeros -fno-trapping-math
#   c-clang-reassoc_<kernel>_idiomatic same, clang
#   rust_<kernel>_<variant>            cargo build --release, -C target-cpu=native
#   go_<kernel>_<variant>              go build, GOAMD64=v3 if the CPU has AVX2/FMA/BMI2, else v2 (DECISIONS D4)
#   zig_<kernel>_<variant>             zig build-exe -O ReleaseFast -mcpu=native
#   zig-reassoc_<kernel>_idiomatic     same, @setFloatMode(.optimized)
# Parallel variants (threads: OMP_NUM_THREADS, RAYON_NUM_THREADS, GOMAXPROCS,
# ISC_THREADS; Isocron itself uses ISC_THREADS):
#   c-gcc-omp_<kernel>_<variant>       strict flags + -fopenmp (OpenMP)
#   rust-par_<kernel>_<variant>        rayon
#   go-par_<kernel>_<variant>          goroutines
#   zig-par_<kernel>_<variant>         std.Thread.Pool
# Variants: idiomatic, opt (hand-optimized).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(dirname "$here")"
bin="$here/bin"
mkdir -p "$bin" "$here/build"
export PATH="$HOME/.cargo/bin:$PATH"
ZIG="${ZIG:-$(command -v zig || echo "$HOME/.local/opt/zig-x86_64-linux-0.14.1/zig")}"
export ZIG_GLOBAL_CACHE_DIR="$here/build/zig-cache" ZIG_LOCAL_CACHE_DIR="$here/build/zig-cache"
kernels="sum dot std_dev histogram"
float_kernels="sum dot std_dev"

echo "== Isocron"
(cd "$root" && cargo build --release -q)
for k in $kernels; do
    "$root/target/release/iscc" build "$root/examples/poc/$k.isc" -o "$bin/isc_$k" \
        --emit-asm "$here/build/isc_$k.s" --emit-llvm "$here/build/isc_$k.ll"
done
# the kernels of phase 7 (SPEC section 14)
for k in moving_average moving_average_b filter_sum prefix_sum matvec; do
    "$root/target/release/iscc" build "$root/examples/phase7/$k.isc" -o "$bin/isc_$k" \
        --emit-asm "$here/build/isc_$k.s" --emit-llvm "$here/build/isc_$k.ll"
done
# `allow_reassoc`, compared with the reassociated variants (section 14, D123)
for k in sum dot std_dev moving_average filter_sum matvec; do
    "$root/target/release/iscc" build "$root/examples/reassoc/$k.isc" -o "$bin/isc-reassoc_$k" \
        --emit-asm "$here/build/isc-reassoc_$k.s"
done

echo "== C"
strict="-O3 -march=native -ffp-contract=off"
reassoc="-O3 -march=native -fassociative-math -fno-signed-zeros -fno-trapping-math"
common="-std=c11 -D_POSIX_C_SOURCE=200809L -Wall -Wno-unknown-pragmas"
for k in $kernels; do
    for v in 0 1; do
        name=$([ $v = 0 ] && echo idiomatic || echo opt)
        gcc $strict $common -DVARIANT=$v -o "$bin/c-gcc_${k}_$name" "$here/c/$k.c" -lm
        gcc $strict $common -fopenmp -DVARIANT=$v -o "$bin/c-gcc-omp_${k}_$name" "$here/c/$k.c" -lm
        "$ZIG" cc -target x86_64-linux-gnu $strict $common -DVARIANT=$v -o "$bin/c-clang_${k}_$name" "$here/c/$k.c" -lm
    done
done
for k in $float_kernels; do
    gcc $reassoc $common -DVARIANT=0 -o "$bin/c-gcc-reassoc_${k}_idiomatic" "$here/c/$k.c" -lm
    "$ZIG" cc -target x86_64-linux-gnu $reassoc $common -DVARIANT=0 -o "$bin/c-clang-reassoc_${k}_idiomatic" "$here/c/$k.c" -lm
done

echo "== Rust"
(cd "$here/rust" && RUSTFLAGS="-C target-cpu=native" cargo build --release -q)
for k in $kernels; do
    for v in idiomatic opt; do
        cp "$here/rust/target/release/${k}_$v" "$bin/rust_${k}_$v"
        cp "$here/rust/target/release/${k}_par_$v" "$bin/rust-par_${k}_$v"
    done
done

echo "== Go"
(
    cd "$here/go"
    if grep -qw avx2 /proc/cpuinfo && grep -qw fma /proc/cpuinfo && grep -qw bmi2 /proc/cpuinfo; then
        GOAMD64=v3
    else
        GOAMD64=v2
    fi
    echo "   GOAMD64=$GOAMD64"
    export GOAMD64 GOCACHE="$here/build/go-cache" GOFLAGS=-mod=mod
    for k in $kernels; do
        for v in idiomatic opt; do
            go build -o "$bin/go_${k}_$v" "./cmd/${k}_$v"
            go build -o "$bin/go-par_${k}_$v" "./cmd/${k}_par_$v"
        done
    done
)

echo "== Zig"
(
    cd "$here/zig"
    for k in $kernels; do
        for v in idiomatic opt; do
            "$ZIG" build-exe -O ReleaseFast -mcpu=native -femit-bin="$here/build/zig_${k}_$v" "${k}_$v.zig"
            cp "$here/build/zig_${k}_$v" "$bin/zig_${k}_$v"
            "$ZIG" build-exe -O ReleaseFast -mcpu=native -femit-bin="$here/build/zig-par_${k}_$v" "${k}_par_$v.zig"
            cp "$here/build/zig-par_${k}_$v" "$bin/zig-par_${k}_$v"
        done
    done
    for k in $float_kernels; do
        "$ZIG" build-exe -O ReleaseFast -mcpu=native -femit-bin="$here/build/zig-reassoc_$k" "${k}_reassoc.zig"
        cp "$here/build/zig-reassoc_$k" "$bin/zig-reassoc_${k}_idiomatic"
    done
)
# the other languages for the kernels of phase 7
for lang in c rust go zig; do
    if [ -x "$here/build_phase7_$lang.sh" ]; then
        echo "== $lang (phase 7)"
        "$here/build_phase7_$lang.sh"
    fi
done
echo "built $(ls "$bin" | wc -l) programs in $bin"
