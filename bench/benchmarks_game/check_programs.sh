#!/usr/bin/env bash
# Inside the image: in both modes,
# 1. each of the ten Isocron programs (references.txt) against the
#    reference output of the Benchmarks Game, byte for byte, with its
#    argument and its input;
# 2. every other program of bin/<mode>/ (programs.txt) at N = 200 against
#    the reference of the Benchmarks Game (upstream/<bench>/200_out), and at
#    N = 16000 (their size) against the Isocron program of the same mode
#    (whose output is the classic algorithm's, tests/bitwise.rs).
# C++ #4 rounds its number of rows up to a multiple of the number of CPUs
# (`hardware_concurrency`): at N = 200 its output differs on a machine of
# more than 8 CPUs (200 is a multiple of 4 and 8); N = 16000 is the check.
set -uo pipefail
cd /work
status=0
ours=" $(grep -v '^#' references.txt | awk 'NF { printf "%s ", $1 }')"
for mode in ivybridge native; do
    while read -r bench n input; do
        case "$bench" in ""|\#*) continue ;; esac
        in=/dev/null
        [ -n "${input:-}" ] && in="upstream/$bench/$input"
        r="different"
        "bin/$mode/$bench.isocron" "$n" < "$in" | cmp -s - "upstream/$bench/${n}_out" && r="same"
        printf '%-10s %-24s N=%-5s vs reference: %s\n' "$mode" "$bench.isocron" "$n" "$r"
        [ "$r" = same ] || status=1
    done < references.txt
    for bin in bin/$mode/*; do
        name="$(basename "$bin")"
        bench="${name%%.*}"
        # the ten Isocron programs are checked above
        [ "$name" = "$bench.isocron" ] && [[ "$ours" == *" $bench "* ]] && continue
        if [ "$bench" = mandelbrot ]; then
            ref="upstream/$bench/200_out"
            r200="different"
            "$bin" 200 | cmp -s - "$ref" && r200="same"
            mine="bin/$mode/$bench.isocron"
            r16="different"
            [ "$("$bin" 16000 | md5sum)" = "$("$mine" 16000 | md5sum)" ] && r16="same"
            printf '%-10s %-24s N=200 vs reference: %-9s N=16000 vs Isocron: %s\n' "$mode" "$name" "$r200" "$r16"
            # C++ #4 at N = 200: the known difference above, N = 16000 checks it
            [ "$name" = mandelbrot.gpp-4 ] && r200=same
            [ "$r200" = same ] && [ "$r16" = same ] || status=1
            continue
        fi
        # Go #4 of pidigits writes no last line shorter than 10 digits (the
        # Benchmarks Game runs it with N = 10000): checked at N = 10000
        # against the Isocron program
        if [ "$name" = pidigits.go-4 ]; then
            r="different"
            [ "$("$bin" 10000 | md5sum)" = "$("bin/$mode/pidigits.isocron" 10000 | md5sum)" ] && r="same"
            printf '%-10s %-24s N=10000 vs Isocron: %s\n' "$mode" "$name" "$r"
            [ "$r" = same ] || status=1
            continue
        fi
        # the others: the argument and the input of the reference output
        read -r _ n input <<< "$(grep -E "^$bench[[:space:]]" references.txt)"
        in=/dev/null
        [ -n "${input:-}" ] && in="upstream/$bench/$input"
        r="different"
        "$bin" "$n" < "$in" 2>/dev/null | cmp -s - "upstream/$bench/${n}_out" && r="same"
        printf '%-10s %-24s N=%-5s vs reference: %s\n' "$mode" "$name" "$n" "$r"
        [ "$r" = same ] || status=1
    done
done
exit $status
