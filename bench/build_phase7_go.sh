#!/usr/bin/env bash
# Builds the Go programs of the four kernels of phase 7 (SPEC section 14:
# moving average, filter then sum, prefix sum, matrix-vector product) into
# bench/bin/, with the options of the Go part of bench/build.sh (which calls
# this script):
#   go_<kernel>_<variant>       go build, GOAMD64=v3 if the CPU has AVX2/FMA/BMI2, else v2 (DECISIONS D4)
#   go-par_<kernel>_<variant>   goroutines, GOMAXPROCS threads
# Variants: idiomatic, opt (hand-optimized, bit-identical to Isocron).
# Comparison B of section 14 (best algorithm, results not bit-identical):
#   go_moving_average_b_idiomatic, go-par_moving_average_b_idiomatic (running sum, O(n))
# Sources: bench/go/cmd/<kernel>_<variant>, bench/go/cmd/<kernel>_par_<variant>;
# notes and verification: bench/go/PHASE7.md.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
bin="$here/bin"
mkdir -p "$bin" "$here/build"
kernels="moving_average filter_sum prefix_sum matvec"

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
go build -o "$bin/go_moving_average_b_idiomatic" ./cmd/moving_average_b_idiomatic
go build -o "$bin/go-par_moving_average_b_idiomatic" ./cmd/moving_average_b_par_idiomatic
