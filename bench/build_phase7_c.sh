#!/usr/bin/env bash
# Builds the C programs of the kernels of phase 7 (SPEC section 14) into
# bench/bin/, with the flags of bench/build.sh:
#   c-gcc_<kernel>_<variant>           -O3 -march=native -ffp-contract=off
#   c-clang_<kernel>_<variant>         same flags, clang 19 (zig cc, DECISIONS D2)
#   c-gcc-omp_<kernel>_<variant>       strict flags + -fopenmp (threads: OMP_NUM_THREADS)
#   c-gcc-reassoc_<kernel>_idiomatic   -O3 -march=native -fassociative-math -fno-signed-zeros -fno-trapping-math
#   c-clang-reassoc_<kernel>_idiomatic same, clang
# Kernels: moving_average, filter_sum, prefix_sum, matvec (the reassociated
# builds: the three float kernels). Variants: idiomatic (VARIANT=0), opt
# (VARIANT=1, bit-identical to Isocron). Comparison B of the moving average
# (running sum, O(n)): c-gcc_, c-clang_ and c-gcc-omp_moving_average_b_idiomatic.
# See bench/c/PHASE7.md.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="$here/bin"
mkdir -p "$bin" "$here/build"
ZIG="${ZIG:-$(command -v zig || echo "$HOME/.local/opt/zig-x86_64-linux-0.14.1/zig")}"
export ZIG_GLOBAL_CACHE_DIR="$here/build/zig-cache" ZIG_LOCAL_CACHE_DIR="$here/build/zig-cache"
kernels="moving_average filter_sum prefix_sum matvec"
float_kernels="moving_average filter_sum matvec"

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
# comparison B of section 14: the running sum (idiomatic only)
gcc $strict $common -o "$bin/c-gcc_moving_average_b_idiomatic" "$here/c/moving_average_b.c" -lm
gcc $strict $common -fopenmp -o "$bin/c-gcc-omp_moving_average_b_idiomatic" "$here/c/moving_average_b.c" -lm
"$ZIG" cc -target x86_64-linux-gnu $strict $common -o "$bin/c-clang_moving_average_b_idiomatic" "$here/c/moving_average_b.c" -lm
echo "built the C programs of phase 7 in $bin"
