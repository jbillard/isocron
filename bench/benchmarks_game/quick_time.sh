#!/usr/bin/env bash
# Inside the image: a rough timing (not the protocol of SPEC section 14):
# the lowest of 3 runs at N = 16000, output to /dev/null, with T threads
# (the programs that read it: OMP_NUM_THREADS, RAYON_NUM_THREADS,
# GOMAXPROCS, ISC_THREADS; C++ #4 starts one thread per CPU of the machine).
set -euo pipefail
cd /work
t="$1"
export OMP_NUM_THREADS=$t RAYON_NUM_THREADS=$t GOMAXPROCS=$t ISC_THREADS=$t
for mode in ivybridge native; do
    for bin in bin/$mode/*; do
        best=999999
        for r in 1 2 3; do
            s=$(date +%s%N); "$bin" 16000 > /dev/null; e=$(( ($(date +%s%N) - s) / 1000000 ))
            [ $e -lt $best ] && best=$e
        done
        printf 'T=%s %-10s %-22s %6s ms\n' "$t" "$mode" "$(basename "$bin")" "$best"
    done
done
