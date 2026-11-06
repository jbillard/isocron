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
| 1 | 67459 (14 µs, 19 µs) | 67363 (14 µs, 19 µs) | 71457 (13 µs, 19 µs) | 70701 (13 µs, 18 µs) |
| 16 | 197647 (74 µs, 150 µs) | 180816 (78 µs, 164 µs) | 628909 (23 µs, 45 µs) | 188598 (83 µs, 163 µs) |
| 256 | 151520 (1.65 ms, 2.40 ms) | 140039 (1.78 ms, 2.68 ms) | 769612 (291 µs, 491 µs) | 192416 (1.31 ms, 2.61 ms) |
| 1024 | 67277 (15.43 ms, 18.00 ms) | 65244 (15.77 ms, 21.35 ms) | 739676 (1.28 ms, 2.07 ms) | 174111 (5.80 ms, 12.07 ms) |

One connection: Isocron 94 % of nginx. More connections: Isocron falls
(67277 at 1024, 11 times below nginx, 2.6 times below lighttpd), where
lighttpd, one process as the loop of Isocron, stays near its level. The
cause, measured (D353): the profile of the server at 1024 connections
(`perf record -p`) gives 54.7 % of its cycles to the threads of its pool
waiting for work and 22.9 % to the allocator refilling the caches of
threads (blocks made by one thread, freed by another), a few percent to
the work; with one thread (`ISC_THREADS=1`) 83632 requests a second at
1024 connections, 206647 at 16: the cost of a request grows with the
connections, because each turn of the loop reads every pending connection
(`readyOf` of each, the filters of the closed ones) and matches each
arrival to its connection by a linear search (`arrive`). Open in TODO: a
table of the connections by their number, only those that received
bytes read.

Measured again by `bench/http_servers/measure.sh` (the four servers) and
`threads.sh` (the threads of Isocron), inside the image, the `iscc` of the
host mounted in it.
