#!/usr/bin/env bash
# Inside the image: the output of every program of bin/<mode>/ at N = 200
# against the reference of the Benchmarks Game (upstream/<bench>/200_out),
# and at N = 16000 (their size) against the Isocron program of the same
# mode (whose output is the classic algorithm's, tests/bitwise.rs).
# C++ #4 rounds its number of rows up to a multiple of the number of CPUs
# (`hardware_concurrency`): at N = 200 its output differs on a machine of
# more than 8 CPUs (200 is a multiple of 4 and 8); N = 16000 is the check.
set -uo pipefail
cd /work
status=0
for mode in ivybridge native; do
    for bin in bin/$mode/*; do
        name="$(basename "$bin")"
        bench="${name%%.*}"
        ref="upstream/$bench/200_out"
        r200="different"
        "$bin" 200 | cmp -s - "$ref" && r200="same"
        ours="bin/$mode/$bench.isocron"
        r16="different"
        [ "$("$bin" 16000 | md5sum)" = "$("$ours" 16000 | md5sum)" ] && r16="same"
        printf '%-10s %-24s N=200 vs reference: %-9s N=16000 vs Isocron: %s\n' "$mode" "$name" "$r200" "$r16"
        [ "$r200" = same ] && [ "$r16" = same ] || status=1
    done
done
exit $status
