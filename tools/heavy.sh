#!/usr/bin/env bash
# The heavy jobs of this machine, two places (D619). The machine also runs
# the services of its owner; two jobs of unknown memory together froze it
# once (2026-11-06) and ran it out of memory once (2026-11-09).
#
#   tools/heavy.sh one LABEL COMMAND...   a build or one test file, in one
#                                         place, capped at 6 GiB (the kernel
#                                         stops it alone past its cap)
#   tools/heavy.sh all LABEL COMMAND...   the full suite, a measure of cycles,
#                                         a build of images or the VM: both
#                                         places, nothing else running
#
# A place is a file `target/heavy.lock` or `target/heavy.lock.2`, taken in
# one step (`noclobber`: the write fails if another took it), given back at
# the end only if its line is still the job's. `all` takes the first place,
# then waits for the second and for no build of another (`rustc`, `cargo`)
# outside its own; `one` takes either place. Nothing runs that did not get
# its place. Waits print nothing.
set -uo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
places=("$root/target/heavy.lock" "$root/target/heavy.lock.2")
kind=${1:-}
label=${2:-}
[ "$kind" = one ] || [ "$kind" = all ] || { echo "usage: heavy.sh one|all LABEL COMMAND..." >&2; exit 2; }
[ -n "$label" ] && [ $# -ge 3 ] || { echo "usage: heavy.sh one|all LABEL COMMAND..." >&2; exit 2; }
shift 2
line="$label (pid $$) since $(date +%H:%M)"
take() { ( set -o noclobber; echo "$line" > "$1" ) 2>/dev/null; }
mine() { [ "$(cat "$1" 2>/dev/null)" = "$line" ]; }
give() { mine "$1" && rm -f "$1"; }
held=()
trap 'for p in "${held[@]}"; do give "$p"; done' EXIT
if [ "$kind" = one ]; then
    while true; do
        for p in "${places[@]}"; do
            if take "$p"; then held=("$p"); break 2; fi
        done
        sleep 5
    done
    systemd-run --user --scope --quiet -p MemoryMax=6G -p MemorySwapMax=0 "$@"
    exit $?
fi
until take "${places[0]}"; do sleep 5; done
held=("${places[0]}")
until take "${places[1]}"; do sleep 5; done
held+=("${places[1]}")
# (a build begun before the places were taken: let it end)
while [ "$(ps -eo comm | grep -cE '^(rustc|cargo)$')" != 0 ]; do sleep 5; done
"$@"
