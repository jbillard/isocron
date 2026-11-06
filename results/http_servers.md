# The server of HTTP against the ceiling of the machine

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

The reference is the machine: the **ceiling**, the requests a second of
the cheapest server known for the same work (`ceiling.c`, D355: a loop of
`epoll` for each thread, a listener for each (`SO_REUSEPORT`), a request
found by its end, the 130 bytes of the response of Isocron made once and
copied, one `recv` and one `send` for the requests a read brings), on one
core and on the 4 of the server. nginx and lighttpd are landmarks. The
cores: the CPU time of the server over the time of the run (all its
threads and processes). Measured at fixed frequency, 10 s a point
(`measure.sh`), at 5d7f925 (D366, after D359 to D363; at D355 the server
of Isocron reached 69, 70 and 57 % of the ceiling of one loop with 16, 256
and 1024 connections, its pool 3.95 to 4.04 cores busy):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 70564 (0.58 cores) | 201587 (1.40 cores) | 206674 (2.10 cores) | 155344 (1.81 cores) |
| Isocron `serveAction` | 70573 (0.59 cores) | 196294 (1.12 cores) | 195828 (2.06 cores) | 151148 (1.75 cores) |
| lighttpd | 71401 (0.62 cores) | 187464 (1.01 cores) | 196662 (1.00 cores) | 182843 (0.99 cores) |
| nginx | 72784 (0.56 cores) | 630152 (3.63 cores) | 785171 (3.95 cores) | 753388 (3.96 cores) |
| ceiling, 1 core | 83536 (0.47 cores) | 275431 (1.01 cores) | 288547 (1.00 cores) | 265892 (0.99 cores) |
| ceiling, 4 cores | 83002 (0.47 cores) | 742655 (3.23 cores) | 808459 (3.28 cores) | 782530 (3.09 cores) |

| Isocron `serve` over | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| the ceiling of one loop | 84 % | 73 % | 72 % | 58 % |
| the ceiling of 4 cores (a bound below: the client saturates first) | 85 % | 27 % | 26 % | 20 % |

The ceiling of 4 cores is a bound below: it keeps 3.09 to 3.28 of its 4
cores busy, the client (wrk, 4 threads on 4 cores) saturating first; one
loop on one core is saturated (0.99 to 1.01 cores). Isocron has one loop
of events, as lighttpd: it reaches 58 to 73 % of the ceiling of one loop
with more connections (84 % with one), its pool now idle when it has
nothing to do (1.40 to 2.10 cores busy, D357; 3.95 to 4.04 at D355). D359
to D363 took 8.6 % of the time of a request away in the benchmark of one
thread (`serve.pipelined` 1491.87 -> 1363.77 ns, all in the program: the
kernel 3045.8 cycles a request against 2969.5 for the floor in C, the
program 2116.8 against 613.6 at D362): under wrk, +2.1, -0.2 and -0.5 %
against D357, a request taking 4.96 and 4.84 µs of the loop with 16 and
256 connections, of which the kernel takes about 59 %. The way to the ceiling: a loop of
events on each core (actions in parallel on the pool, D352), the cost of a
request of the loop down to that of the ceiling.

## Before: against nginx and lighttpd

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
