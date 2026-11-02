#!/usr/bin/env bash
# Every measurement campaign of the repository on this machine, at fixed
# frequency on isolated cores (bench/machine.sh, needs sudo without a
# password), then the machine settings restored, even after an error:
#   bench/campaign.sh NAME
# writes results/NAME.md (one core, run.py), results/NAME-lanes.md (study of
# V, the instruction sets this CPU has), results/NAME-compile.md (compilation
# times) and results/NAME-parallel.md (1, 2, 4, 8 threads, up to the number
# of cores). Example on the Celeron of the POC: bench/campaign.sh poc
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
name="${1:?usage: bench/campaign.sh NAME}"
flags="$(LC_ALL=C lscpu | sed -n 's/^Flags://p')"
has() { [[ " $flags " == *" $1 "* ]]; }
if has avx512f; then isas="sse avx2 avx512"; elif has avx2; then isas="sse avx2"; else isas="sse"; fi

"$here/build.sh"
# only the programs of this run in bench/bin-lanes (lanes.py measures them all)
[ -d "$here/bin-lanes" ] && find "$here/bin-lanes" -mindepth 1 -delete
ISA_LIST="$isas" "$here/build_lanes.sh"

trap '"$here/machine.sh" restore' EXIT
"$here/machine.sh" apply single
"$here/machine.sh" run --name "$name"
"$here/machine.sh" run --script "$here/lanes.py" --name "$name-lanes"
"$here/machine.sh" run --script "$here/compile.py" --name "$name-compile"
"$here/machine.sh" apply parallel
"$here/machine.sh" run --script "$here/parallel.py" --name "$name-parallel"
echo "campaign $name done: results/$name.md, $name-lanes.md, $name-compile.md, $name-parallel.md"
