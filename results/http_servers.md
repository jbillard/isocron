# The server of HTTP against servers of C made for production

`bench/http_servers/` (DECISIONS D353): `hello, world!` (13 bytes) to every
request, by the server of Isocron (`http.serve`, a pure handler; and
`http.serveAction`, an action), nginx 1.26 (4 workers, the text from its
configuration) and lighttpd (one process, the text from a file); h2o is
not in Debian trixie. In the Docker image `isocron-http` (that of the
Benchmarks Game, with nginx, lighttpd and wrk), without network: the
server on 4 CPUs, wrk on 4 others (4 threads, or as many as connections),
connections kept alive, one request at a time each, 10 s for each count,
after `bench/machine.sh apply parallel` (fixed frequency), on an Intel
Xeon E-2388G. Requests a second; the latency at 50 % and 99 % in
brackets.

| connections | Isocron `serve` | Isocron `serveAction` | nginx | lighttpd |
| --: | --: | --: | --: | --: |
| 1 | 67841 (14 µs, 20 µs) | 67764 (14 µs, 19 µs) | 71700 (13 µs, 18 µs) | 69494 (13 µs, 19 µs) |
| 16 | 189023 (82 µs, 152 µs) | 174816 (88 µs, 167 µs) | 662884 (22 µs, 34 µs) | 187235 (84 µs, 162 µs) |
| 256 | 202600 (1.24 ms, 1.43 ms) | 184129 (1.35 ms, 2.15 ms) | 771854 (289 µs, 461 µs) | 194181 (1.30 ms, 2.60 ms) |
| 1024 | 158735 (6.26 ms, 9.74 ms) | 146131 (6.81 ms, 9.56 ms) | 722790 (1.32 ms, 2.20 ms) | 177463 (5.71 ms, 11.78 ms) |

After D354 (above): the server of Isocron as lighttpd, both one loop of
events (101 to 104 % of it up to 256 connections, 89 % at 1024); nginx
3.5 to 4.5 times above with more connections, by its 4 processes, each
its own loop and its own connections. Before D354 (D353), the cost of a
request grew with the connections:

| connections | Isocron `serve` | Isocron `serveAction` | nginx | lighttpd |
| --: | --: | --: | --: | --: |
| 1 | 67459 | 67363 | 71457 | 70701 |
| 16 | 197647 | 180816 | 628909 | 188598 |
| 256 | 151520 | 140039 | 769612 | 192416 |
| 1024 | 67277 | 65244 | 739676 | 174111 |

Before D354, with more connections Isocron fell (67277 at 1024, 11 times
below nginx, 2.6 times below lighttpd), where lighttpd, one process as the
loop of Isocron, stays near its level. The cause, measured (D353): the
profile of the server at 1024 connections
(`perf record -p`) gives 54.7 % of its cycles to the threads of its pool
waiting for work and 22.9 % to the allocator refilling the caches of
threads (blocks made by one thread, freed by another), a few percent to
the work; with one thread (`ISC_THREADS=1`) 83632 requests a second at
1024 connections, 206647 at 16: the cost of a request grows with the
connections, because each turn of the loop reads every pending connection
(`readyOf` of each, the filters of the closed ones) and matches each
arrival to its connection by a linear search (`arrive`). D354: the
connections in a table sorted by their numbers, each arrival found by
halving and written in place, only the connections that received bytes
read: 158735 at 1024 connections (2.36 times), the threads no longer
costing (1 thread 153792, 4 threads 156128).

Measured again by `bench/http_servers/measure.sh` (the four servers) and
`threads.sh` (the threads of Isocron), inside the image, the `iscc` of the
host mounted in it.
