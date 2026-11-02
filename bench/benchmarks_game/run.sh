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
    *) echo "usage: run.sh build|check" >&2; exit 2 ;;
esac
