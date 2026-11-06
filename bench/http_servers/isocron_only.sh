#!/usr/bin/env bash
# Inside `isocron-http`: the server of Isocron alone (`serve`), its requests a
# second and its cores (the functions of `measure.sh`).
#   isocron_only.sh SERVER CLIENT DURATION
set -euo pipefail
source <(sed -n '/^set -euo pipefail/,/^ISC_THREADS=\$nserver taskset/p' /work/measure.sh | sed '1d;$d')
ISC_THREADS=$nserver taskset -c "$server_cpus" /tmp/hello 8080 & pid=$!
run isocron-serve 8080 / hello
kill $pid; wait $pid 2>/dev/null || true
