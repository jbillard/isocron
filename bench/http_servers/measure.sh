#!/usr/bin/env bash
# Inside the image `isocron-http`: each server on the CPUs SERVER (4), wrk
# on the CPUs CLIENT (4), for each number of connections, DURATION seconds;
# one line each: server, connections, requests a second, the latency at
# 50 % and 99 % (wrk). The servers answer `hello, world!`.
#   measure.sh SERVER CLIENT DURATION
set -euo pipefail
server_cpus=$1
client_cpus=$2
duration=$3
cd /work
iscc build hello.isc -o /tmp/hello > /dev/null
mkdir -p /tmp/http-root && printf 'hello, world!' > /tmp/http-root/index.html
wait_port() {
    for _ in $(seq 200); do
        if (exec 3<>/dev/tcp/127.0.0.1/$1) 2>/dev/null; then return 0; fi
        sleep 0.05
    done
    echo "no server on $1" >&2
    return 1
}
run() {
    local name=$1 port=$2 path=$3
    wait_port "$port"
    for c in 1 16 256 1024; do
        local t=$(( c < 4 ? c : 4 ))
        out=$(taskset -c "$client_cpus" wrk -t"$t" -c"$c" -d"${duration}s" --latency "http://127.0.0.1:$port$path" 2>&1)
        rps=$(echo "$out" | awk '/Requests\/sec/ {print $2}')
        p50=$(echo "$out" | awk '$1 == "50%" {print $2}')
        p99=$(echo "$out" | awk '$1 == "99%" {print $2}')
        errs=$(echo "$out" | awk '/Socket errors|Non-2xx/ {print}' | tr '\n' ' ')
        printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$c" "$rps" "$p50" "$p99" "$errs"
    done
}
ISC_THREADS=4 taskset -c "$server_cpus" /tmp/hello 8080 & pid=$!
run isocron-serve 8080 /
kill $pid; wait $pid 2>/dev/null || true
ISC_THREADS=4 taskset -c "$server_cpus" /tmp/hello 8083 action & pid=$!
run isocron-serveAction 8083 /
kill $pid; wait $pid 2>/dev/null || true
taskset -c "$server_cpus" /usr/sbin/nginx -c /work/nginx.conf & pid=$!
run nginx 8081 /
kill $pid; wait $pid 2>/dev/null || true
taskset -c "$server_cpus" /usr/sbin/lighttpd -D -f /work/lighttpd.conf > /dev/null 2>&1 & pid=$!
run lighttpd 8082 /index.html
kill $pid; wait $pid 2>/dev/null || true
