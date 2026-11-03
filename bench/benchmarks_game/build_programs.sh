#!/usr/bin/env bash
# Inside the image (run.sh): builds the programs of programs.txt and the
# Isocron programs of examples/benchmarks_game, in two modes:
#   ivybridge  the options of the Benchmarks Game command lines (their
#              i5-3330: AVX without AVX2 or FMA); Isocron `--isa avx`
#   native     the same with the CPU of this machine; Isocron by default.
#              C and C++ add -ffp-contract=off: with AVX-512, -mno-fma no
#              longer stops gcc from contracting into FMAs, which changes
#              the output (their machine has no FMA); C++ #4 adds
#              -mno-avx512f: its AVX-512 branch stops a byte as soon as one
#              of its pixels escapes (its other branches when all do), a
#              wrong output their machine never runs (DECISIONS D142)
# into /work/bin/<mode>/<benchmark>.<tag>.
set -euo pipefail
cd /work
mkdir -p build/go build/rust
export GOCACHE=/work/build/go GOPATH=/work/build/gopath
for mode in ivybridge native; do
    out="/work/bin/$mode"
    mkdir -p "$out"
    march=$mode
    goamd64=v2
    cflags=""
    cxxflags=""
    if [ "$mode" = native ]; then
        goamd64=v3
        cflags="-ffp-contract=off"
        cxxflags="-ffp-contract=off -mno-avx512f"
    fi
    while read -r src lang extra; do
        case "$src" in ""|\#*) continue ;; esac
        name="$(basename "$src")"
        bench="${name%%.*}"
        tag="${name#*.}"; tag="${tag%.*}"
        f="/work/upstream/$src"
        # $extra: the options of this program alone (its libraries)
        # shellcheck disable=SC2086
        if ! case "$lang" in
            c++)
                cp "$f" "build/$tag.cpp"
                g++ -pipe -O3 -fomit-frame-pointer -march=$march -std=c++17 -mno-fma $cxxflags "build/$tag.cpp" -o "$out/$bench.$tag" -pthread $extra ;;
            c)
                cp "$f" "build/$tag.c"
                gcc -pipe -Wall -O3 -fomit-frame-pointer -march=$march -mno-fma -fno-finite-math-only -fopenmp $cflags "build/$tag.c" -o "$out/$bench.$tag" $extra -lm ;;
            rust)
                cp "$f" /opt/rust-bench/src/main.rs
                (cd /opt/rust-bench && CARGO_TARGET_DIR="/work/build/rust/$mode" RUSTFLAGS="-C target-cpu=$march" \
                    cargo build --release --offline --quiet) \
                    && cp "/work/build/rust/$mode/release/bg" "$out/$bench.$tag" ;;
            go)
                # in a module, with its dependencies fetched in the image
                rm -rf "build/gomod"
                cp -r /opt/go-bench "build/gomod"
                cp "$f" "build/gomod/main.go"
                (cd build/gomod && GOAMD64=$goamd64 GOFLAGS=-mod=mod GOPROXY=off go build -o "$out/$bench.$tag" .) ;;
        esac; then
            echo "FAILED $mode/$bench.$tag"
            continue
        fi
        echo "built $mode/$bench.$tag"
    done < /work/programs.txt
    for f in /work/isocron/*.isc; do
        bench="$(basename "$f" .isc)"
        isa=native
        [ "$mode" = ivybridge ] && isa=avx
        iscc build "$f" -o "$out/$bench.isocron" --isa $isa
        echo "built $mode/$bench.isocron"
    done
done
