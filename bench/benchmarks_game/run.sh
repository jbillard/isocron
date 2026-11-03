#!/usr/bin/env bash
# Builds then checks the programs inside the image `isocron-bg` (image.sh),
# without network, as the user of the host:
#   run.sh build     build_programs.sh (both modes) into bin/
#   run.sh check     check_programs.sh: outputs against the references
# The Isocron programs come from examples/benchmarks_game/.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(dirname "$(dirname "$here")")"
mkdir -p "$here/isocron"
cp "$root"/examples/benchmarks_game/*.isc "$here/isocron/"
in_image() {
    docker run --rm --network none --user "$(id -u):$(id -g)" -v "$here:/work" isocron-bg "$@"
}
case "${1:-}" in
    build) in_image bash /work/build_programs.sh ;;
    check) in_image bash /work/check_programs.sh ;;
    # the protocol of SPEC section 14: after `bench/machine.sh apply
    # parallel`, the container in the slice of the measurements, on T of its
    # CPUs (the first hardware thread of T physical cores)
    measure)
        t="${2:-4}"
        reps="${3:-3}"
        modes="${4:-native}"
        envf="$root/bench/build/machine.env"
        [ -f "$envf" ] || { echo "run 'bench/machine.sh apply parallel' first" >&2; exit 1; }
        # shellcheck disable=SC1090
        . "$envf"
        cpus="$(echo "$BENCH_CPUS" | tr ',' '\n' | head -n "$t" | paste -sd, -)"
        docker run --rm --network none --cgroup-parent=isc-bench.slice --cpuset-cpus="$cpus" \
            --user "$(id -u):$(id -g)" -v "$here:/work" isocron-bg bash /work/measure.sh "$t" "$reps" "$modes" ;;
    *) echo "usage: run.sh build|check|measure [T R MODES]" >&2; exit 2 ;;
esac
