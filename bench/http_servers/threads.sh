#!/usr/bin/env bash
# Inside `isocron-http`: the server of Isocron with 1 to 4 threads of its
# pool, 16 and 1024 connections, DURATION seconds each.
#   threads.sh SERVER CLIENT DURATION
set -euo pipefail
cd /work
iscc build hello.isc -o /tmp/hello > /dev/null
for th in 1 2 4; do
    ISC_THREADS=$th taskset -c "$1" /tmp/hello 8080 & pid=$!
    until (exec 3<>/dev/tcp/127.0.0.1/8080) 2>/dev/null; do sleep 0.05; done
    for c in 16 1024; do
        rps=$(taskset -c "$2" wrk -t4 -c"$c" -d"$3s" http://127.0.0.1:8080/ 2>&1 | awk '/Requests\/sec/ {print $2}')
        printf 'threads %s\tconnections %s\t%s\n' "$th" "$c" "$rps"
    done
    kill $pid; wait $pid 2>/dev/null || true
done
