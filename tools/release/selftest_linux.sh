#!/usr/bin/env bash
# Checks that an iscc for Linux needs nothing else (D248): in containers of
# minimal systems without compiler, linker or development files of glibc
# (Debian 12 slim, Ubuntu 20.04: glibc 2.31, AlmaLinux 8: glibc 2.28), and
# without network, it builds the ten programs of the Benchmarks Game, whose
# outputs must be those of the programs built here by REFERENCE (an iscc of
# this machine), and builds for Windows and WebAssembly.
#   selftest_linux.sh ISCC REFERENCE
set -euo pipefail
iscc=$(realpath "$1")
ref=$(realpath "$2")
root="$(cd "$(dirname "$0")/../.." && pwd)"
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
cp "$iscc" "$t/iscc"
cp "$root"/examples/benchmarks_game/*.isc "$t/"
"$ref" run "$root/examples/benchmarks_game/fasta.isc" 1000 > "$t/in.txt"
cat > "$t/go.sh" <<'IN'
cd /t
for p in binarytrees:10 fannkuchredux:7 fasta:1000 mandelbrot:200 nbody:1000 pidigits:27 spectralnorm:100 knucleotide:0 regexredux:0 revcomp:0; do
    n=${p%%:*}; a=${p##*:}
    ${ISCC:-./iscc} build $n.isc -o ${OUT:-/tmp}/$n > ${OUT:-/tmp}/$n.log 2>&1 || { echo "$n: build failed: $(tail -2 ${OUT:-/tmp}/$n.log)"; continue; }
    ${OUT:-/tmp}/$n $a < in.txt | md5sum | sed "s/  -/  $n/"
done
${ISCC:-./iscc} build fasta.isc --target x86_64-windows -o ${OUT:-/tmp}/f.exe && head -c 2 ${OUT:-/tmp}/f.exe | grep -q MZ && echo "windows ok"
${ISCC:-./iscc} build fasta.isc --target wasm32-wasi -o ${OUT:-/tmp}/f.wasm && head -c 4 ${OUT:-/tmp}/f.wasm | od -An -c | tr -d " " | grep -q asm && echo "wasm ok"
IN
mkdir -p "$t/ref"
(cd "$t" && ISCC="$ref" OUT="$t/ref" sh -c "sed 's#^cd /t##' go.sh | sh") > "$t/want"
fail=0
for img in debian:bookworm-slim ubuntu:20.04 almalinux:8; do
    docker run --rm --network none -v "$t":/t:ro "$img" sh /t/go.sh > "$t/got" 2>&1 || true
    if diff -q "$t/want" "$t/got" > /dev/null; then
        echo "$img: the same outputs ($(grep -c . "$t/got") lines)"
    else
        echo "$img: DIFFERENT"; diff "$t/want" "$t/got" | head -20; fail=1
    fi
done
exit $fail
