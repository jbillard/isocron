#!/usr/bin/env bash
# Builds the Zig programs of the phase 7 kernels (SPEC section 14) into
# bench/bin/, with the flags and caches of build.sh:
#   zig_<kernel>_<variant>             zig build-exe -O ReleaseFast -mcpu=native
#   zig-par_<kernel>_<variant>         same, std.Thread.Pool ($ISC_THREADS threads)
#   zig-reassoc_<kernel>_idiomatic     same, @setFloatMode(.optimized)
# Kernels: moving_average filter_sum prefix_sum matvec; variants: idiomatic,
# opt (hand-optimized, bit-identical to Isocron). Comparison B of the moving
# average (O(n) running sum): zig_moving_average_b_idiomatic,
# zig-par_moving_average_b_idiomatic. See bench/zig/PHASE7.md.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="$here/bin"
mkdir -p "$bin" "$here/build"
ZIG="${ZIG:-$(command -v zig || echo "$HOME/.local/opt/zig-x86_64-linux-0.14.1/zig")}"
export ZIG_GLOBAL_CACHE_DIR="$here/build/zig-cache" ZIG_LOCAL_CACHE_DIR="$here/build/zig-cache"
kernels="moving_average filter_sum prefix_sum matvec"
float_kernels="moving_average filter_sum matvec"

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
# comparison B (SPEC section 14): the O(n) moving average, a running sum
"$ZIG" build-exe -O ReleaseFast -mcpu=native -femit-bin="$here/build/zig_moving_average_b_idiomatic" moving_average_b_idiomatic.zig
cp "$here/build/zig_moving_average_b_idiomatic" "$bin/zig_moving_average_b_idiomatic"
"$ZIG" build-exe -O ReleaseFast -mcpu=native -femit-bin="$here/build/zig-par_moving_average_b_idiomatic" moving_average_b_par_idiomatic.zig
cp "$here/build/zig-par_moving_average_b_idiomatic" "$bin/zig-par_moving_average_b_idiomatic"
