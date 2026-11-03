#!/usr/bin/env bash
# Inside the image (run.sh measure): every program of bin/<mode>/ at the size
# of the Benchmarks Game, the lowest of R runs (elapsed time, output to
# /dev/null), with T threads (OMP_NUM_THREADS, RAYON_NUM_THREADS,
# GOMAXPROCS, ISC_THREADS; C++ #4 of mandelbrot starts one thread per CPU).
# The inputs of k-nucleotide, reverse-complement and regex-redux are the
# output of fasta (25 000 000 and 5 000 000), made once. Output: lines
# `mode bench program milliseconds` separated by tabs.
#   measure.sh T R MODES
set -uo pipefail
cd /work
t="${1:-4}"
reps="${2:-3}"
modes="${3:-native}"
export OMP_NUM_THREADS=$t RAYON_NUM_THREADS=$t GOMAXPROCS=$t ISC_THREADS=$t
mkdir -p build/inputs
[ -s build/inputs/fasta25m.txt ] || bin/native/fasta.isocron 25000000 > build/inputs/fasta25m.txt
[ -s build/inputs/fasta5m.txt ] || bin/native/fasta.isocron 5000000 > build/inputs/fasta5m.txt
size() {
    case "$1" in
        binarytrees) echo "21 -" ;;
        fannkuchredux) echo "12 -" ;;
        fasta) echo "25000000 -" ;;
        knucleotide) echo "0 build/inputs/fasta25m.txt" ;;
        mandelbrot) echo "16000 -" ;;
        nbody) echo "50000000 -" ;;
        pidigits) echo "10000 -" ;;
        regexredux) echo "0 build/inputs/fasta5m.txt" ;;
        revcomp) echo "0 build/inputs/fasta25m.txt" ;;
        spectralnorm) echo "5500 -" ;;
    esac
}
for mode in $modes; do
    for bin in bin/$mode/*; do
        name="$(basename "$bin")"
        bench="${name%%.*}"
        [ "$name" = mandelbrot.simple.isocron ] && continue
        read -r n input <<< "$(size "$bench")"
        in=/dev/null
        [ "$input" != - ] && in="$input"
        best=""
        for _ in $(seq "$reps"); do
            s=$(date +%s%N)
            timeout 900 "$bin" "$n" < "$in" > /dev/null 2>&1
            code=$?
            e=$(( ($(date +%s%N) - s) / 1000000 ))
            if [ $code -ne 0 ]; then best="failed($code)"; break; fi
            if [ -z "$best" ] || [ "$e" -lt "$best" ]; then best=$e; fi
        done
        printf '%s\t%s\t%s\t%s\n' "$mode" "$bench" "${name#*.}" "$best"
    done
done
