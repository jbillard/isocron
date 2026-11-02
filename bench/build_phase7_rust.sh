#!/usr/bin/env bash
# Builds the Rust programs of the kernels of phase 7 (SPEC section 14) into
# bench/bin/, like the Rust part of build.sh:
#   rust_<kernel>_<variant>       cargo build --release, -C target-cpu=native
#   rust-par_<kernel>_<variant>   same, rayon (threads: RAYON_NUM_THREADS)
# Kernels: moving_average, filter_sum, prefix_sum, matvec.
# Variants: idiomatic, opt (hand-optimized, bit-identical to Isocron).
# Comparison B: rust_moving_average_b_idiomatic, rust-par_moving_average_b_idiomatic.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="$here/bin"
mkdir -p "$bin"
export PATH="$HOME/.cargo/bin:$PATH"
(cd "$here/rust" && RUSTFLAGS="-C target-cpu=native" cargo build --release -q)
for k in moving_average filter_sum prefix_sum matvec; do
    for v in idiomatic opt; do
        cp "$here/rust/target/release/${k}_$v" "$bin/rust_${k}_$v"
        cp "$here/rust/target/release/${k}_par_$v" "$bin/rust-par_${k}_$v"
    done
done
# comparison B of SPEC section 14 (the O(n) running sum), idiomatic only
cp "$here/rust/target/release/moving_average_b_idiomatic" "$bin/rust_moving_average_b_idiomatic"
cp "$here/rust/target/release/moving_average_b_par_idiomatic" "$bin/rust-par_moving_average_b_idiomatic"
