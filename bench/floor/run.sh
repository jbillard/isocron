#!/usr/bin/env bash
# The constants of the machine (floor.c) on 1 and 4 CPUs of the slice of the
# measurements, at fixed frequency; then the machine put back.
#   bench/floor/run.sh > results/machine_floor.txt
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
mkdir -p "$here/build"
gcc -O2 -march=native -fopenmp "$here/floor.c" -o "$here/build/floor"
trap '"$root/bench/machine.sh" restore > /dev/null 2>&1' EXIT
"$root/bench/machine.sh" apply parallel > /dev/null
. "$root/bench/build/machine.env"
for t in 1 4; do
    cpus="$(echo "$BENCH_CPUS" | tr ',' '\n' | head -n "$t" | paste -sd, -)"
    echo "# $t thread(s), CPUs $cpus"
    sudo -n systemd-run --quiet --scope --slice=isc-bench -p AllowedCPUs="$cpus" --uid="$(id -u)" --gid="$(id -g)" \
        env OMP_PROC_BIND=true "$here/build/floor" "$t"
done
