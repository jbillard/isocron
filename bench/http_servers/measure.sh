#!/usr/bin/env bash
# Inside the image `isocron-http`: each server on the CPUs SERVER (4), wrk
# on the CPUs CLIENT (4), for each number of connections, DURATION seconds;
# one line each: server, connections, requests a second, the latency at
# 50 % and 99 % (wrk), the cores busy, the memory of the server (all its
# processes: the sum of their Pss at rest before the first run, then the
# highest sums of their Pss and of their Rss while wrk runs again as long,
# every 0.1 s (the first run of wrk, for the requests, the latency and the
# cores, alone: D743);
# the Pss counts a page shared by n processes for 1/n in each, so that the
# master and workers of nginx count their common pages once). The servers
# answer `hello, world!`, all started without huge pages (`nothp.c`: in
# pages of 4 KiB). The servers of several threads or processes have one
# for each CPU of SERVER, wrk one for each CPU of CLIENT (at most one a
# connection).
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
gcc -O2 -o /tmp/nothp nothp.c
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
# the processes named `$1`
pids_of() {
    local d
    for d in /proc/[0-9]*; do
        if [ "$(cat "$d/comm" 2>/dev/null)" = "$1" ]; then echo "${d#/proc/}"; fi
    done
}
# the sums of the Pss and of the Rss of the processes given, in KiB
mem_of() {
    local pss=0 rss=0 p a b
    for p in "$@"; do
        read -r a b < <(awk '/^Pss:/ {p = $2} /^Rss:/ {r = $2} END {print p + 0, r + 0}' "/proc/$p/smaps_rollup" 2>/dev/null || echo "0 0")
        pss=$((pss + a))
        rss=$((rss + b))
    done
    echo "$pss $rss"
}
# the highest sums of `mem_of` while the file `$1` exists, every 0.1 s,
# written into the file `$2`
sample() {
    local stop=$1 out=$2 mp=0 mr=0 p r
    shift 2
    while [ -e "$stop" ]; do
        read -r p r < <(mem_of "$@")
        if [ "$p" -gt "$mp" ]; then mp=$p; fi
        if [ "$r" -gt "$mr" ]; then mr=$r; fi
        sleep 0.1
    done
    echo "$mp $mr" > "$out"
}
run() {
    local name=$1 port=$2 path=$3 comm=$4
    wait_port "$port"
    # (the workers of nginx forked after its port is open)
    sleep 0.5
    local hz pids idle
    hz=$(getconf CLK_TCK)
    pids=$(pids_of "$comm")
    read -r idle _ < <(mem_of $pids)
    for c in 1 16 256 1024; do
        # (wrk gives each thread c / t connections, rounded down: the most
        # threads, one a CPU of the client at most, that divide c, so that
        # it opens c connections; 10 threads opened 10 of 16, D757)
        local t=$(( c < nclient ? c : nclient ))
        while (( c % t != 0 )); do t=$((t - 1)); done
        local t0 t1 spid mp mr
        # the requests a second, the latency and the cores: nothing else
        # running (D743: the sampler of the memory, on the CPUs of the
        # client, took from wrk, the limit on 4 cores, and moved the latency
        # of 99 % by up to 2 ms there)
        t0=$(ticks "$comm")
        out=$(taskset -c "$client_cpus" wrk -t"$t" -c"$c" -d"${duration}s" --latency "http://127.0.0.1:$port$path" 2>&1)
        t1=$(ticks "$comm")
        # the memory: a second run as long, sampled
        touch /tmp/sampling
        # (the sampler on the CPUs of the client: nothing taken from the
        # server's)
        sample /tmp/sampling /tmp/peak $pids &
        spid=$!
        taskset -cp "$client_cpus" "$spid" > /dev/null
        taskset -c "$client_cpus" wrk -t"$t" -c"$c" -d"${duration}s" "http://127.0.0.1:$port$path" > /dev/null 2>&1
        rm -f /tmp/sampling
        wait "$spid"
        read -r mp mr < /tmp/peak
        local cores
        cores=$(awk -v a="$t0" -v b="$t1" -v hz="$hz" -v d="$duration" 'BEGIN {printf "%.2f", (b - a) / hz / d}')
        rps=$(echo "$out" | awk '/Requests\/sec/ {print $2}')
        p50=$(echo "$out" | awk '$1 == "50%" {print $2}')
        p99=$(echo "$out" | awk '$1 == "99%" {print $2}')
        errs=$(echo "$out" | awk '/Socket errors|Non-2xx/ {print}' | tr '\n' ' ')
        printf '%s\t%s\t%s\t%s\t%s\t%s cores\t%s KiB at rest\t%s KiB Pss\t%s KiB Rss\t%s\n' "$name" "$c" "$rps" "$p50" "$p99" "$cores" "$idle" "$mp" "$mr" "$errs"
    done
}
ISC_THREADS=$nserver taskset -c "$server_cpus" /tmp/nothp /tmp/hello 8080 & pid=$!
run isocron-serve 8080 / hello
kill $pid; wait $pid 2>/dev/null || true
ISC_THREADS=$nserver taskset -c "$server_cpus" /tmp/nothp /tmp/hello 8083 action & pid=$!
run isocron-serve-action 8083 / hello
kill $pid; wait $pid 2>/dev/null || true
sed "s/^worker_processes 4;/worker_processes $nserver;/" /work/nginx.conf > /tmp/nginx.conf
taskset -c "$server_cpus" /tmp/nothp /usr/sbin/nginx -c /tmp/nginx.conf & pid=$!
run nginx 8081 / nginx
kill $pid; wait $pid 2>/dev/null || true
taskset -c "$server_cpus" /tmp/nothp /usr/sbin/lighttpd -D -f /work/lighttpd.conf > /dev/null 2>&1 & pid=$!
run lighttpd 8082 /index.html lighttpd
kill $pid; wait $pid 2>/dev/null || true
taskset -c "$server_cpus" /tmp/nothp /tmp/ceiling 8084 1 & pid=$!
run ceiling-1-thread 8084 / ceiling
kill $pid; wait $pid 2>/dev/null || true
taskset -c "$server_cpus" /tmp/nothp /tmp/ceiling 8085 "$nserver" & pid=$!
run "ceiling-$nserver-threads" 8085 / ceiling
kill $pid; wait $pid 2>/dev/null || true
