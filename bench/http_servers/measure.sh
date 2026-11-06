#!/usr/bin/env bash
# Inside the image `isocron-http`: each server on the CPUs SERVER (4), wrk
# on the CPUs CLIENT (4), for each number of connections, DURATION seconds;
# one line each: server, connections, requests a second, the latency at
# 50 % and 99 % (wrk). The servers answer `hello, world!`. The servers of
# several threads or processes have one for each CPU of SERVER, wrk one for
# each CPU of CLIENT (at most one a connection).
#   measure.sh SERVER CLIENT DURATION
set -euo pipefail
server_cpus=$1
client_cpus=$2
duration=$3
nserver=$(echo "$server_cpus" | tr ',' '\n' | wc -l)
nclient=$(echo "$client_cpus" | tr ',' '\n' | wc -l)
cd /work
iscc build hello.isc -o /tmp/hello > /dev/null
gcc -O2 -o /tmp/ceiling ceiling.c -lpthread
mkdir -p /tmp/http-root && printf 'hello, world!' > /tmp/http-root/index.html
wait_port() {
    for _ in $(seq 200); do
        if (exec 3<>/dev/tcp/127.0.0.1/$1) 2>/dev/null; then return 0; fi
        sleep 0.05
    done
    echo "no server on $1" >&2
    return 1
}
# the CPU time of the processes named `$1` (their threads with them), in
# ticks: every worker of nginx counted
ticks() {
    local sum=0 t
    for d in /proc/[0-9]*; do
        if [ "$(cat "$d/comm" 2>/dev/null)" = "$1" ]; then
            t=$(awk '{print $14 + $15}' "$d/stat" 2>/dev/null || echo 0)
            sum=$((sum + t))
        fi
    done
    echo "$sum"
}
run() {
    local name=$1 port=$2 path=$3 comm=$4
    wait_port "$port"
    local hz
    hz=$(getconf CLK_TCK)
    for c in 1 16 256 1024; do
        local t=$(( c < nclient ? c : nclient ))
        local t0
        t0=$(ticks "$comm")
        out=$(taskset -c "$client_cpus" wrk -t"$t" -c"$c" -d"${duration}s" --latency "http://127.0.0.1:$port$path" 2>&1)
        local cores
        cores=$(awk -v a="$t0" -v b="$(ticks "$comm")" -v hz="$hz" -v d="$duration" 'BEGIN {printf "%.2f", (b - a) / hz / d}')
        rps=$(echo "$out" | awk '/Requests\/sec/ {print $2}')
        p50=$(echo "$out" | awk '$1 == "50%" {print $2}')
        p99=$(echo "$out" | awk '$1 == "99%" {print $2}')
        errs=$(echo "$out" | awk '/Socket errors|Non-2xx/ {print}' | tr '\n' ' ')
        printf '%s\t%s\t%s\t%s\t%s\t%s cores\t%s\n' "$name" "$c" "$rps" "$p50" "$p99" "$cores" "$errs"
    done
}
ISC_THREADS=$nserver taskset -c "$server_cpus" /tmp/hello 8080 & pid=$!
run isocron-serve 8080 / hello
kill $pid; wait $pid 2>/dev/null || true
ISC_THREADS=$nserver taskset -c "$server_cpus" /tmp/hello 8083 action & pid=$!
run isocron-serve-action 8083 / hello
kill $pid; wait $pid 2>/dev/null || true
sed "s/^worker_processes 4;/worker_processes $nserver;/" /work/nginx.conf > /tmp/nginx.conf
taskset -c "$server_cpus" /usr/sbin/nginx -c /tmp/nginx.conf & pid=$!
run nginx 8081 / nginx
kill $pid; wait $pid 2>/dev/null || true
taskset -c "$server_cpus" /usr/sbin/lighttpd -D -f /work/lighttpd.conf > /dev/null 2>&1 & pid=$!
run lighttpd 8082 /index.html lighttpd
kill $pid; wait $pid 2>/dev/null || true
taskset -c "$server_cpus" /tmp/ceiling 8084 1 & pid=$!
run ceiling-1-thread 8084 / ceiling
kill $pid; wait $pid 2>/dev/null || true
taskset -c "$server_cpus" /tmp/ceiling 8085 "$nserver" & pid=$!
run "ceiling-$nserver-threads" 8085 / ceiling
kill $pid; wait $pid 2>/dev/null || true
