# The server of HTTP against the ceiling of the machine

`bench/http_servers/` (DECISIONS D353): `hello, world!` (13 bytes) to every
request, by the server of Isocron (`http.serve`, a pure handler; and
`http.serveAction`, an action), nginx 1.26 (a worker for each CPU of the
server, the text from its configuration) and lighttpd (one process, the
text from a file); h2o is not in Debian trixie. In the Docker image
`isocron-http` (that of the Benchmarks Game, with nginx, lighttpd and wrk),
without network: the server on 2 or 4 CPUs, wrk on the others of the 8 of
the measures, connections kept alive, one request at a time each, 10 s for
each count, after `bench/machine.sh apply parallel` (fixed frequency), on
an Intel Xeon E-2388G (`measure.sh SERVER CLIENT 10`). Requests a second,
the cores busy in brackets.

The reference is the machine: the **ceiling**, the requests a second of
the cheapest server known for the same work (`ceiling.c`, D355: a loop of
`epoll` for each thread, a listener for each (`SO_REUSEPORT`), a request
found by its end, the 130 bytes of the response of Isocron made once and
copied, one `recv` and one `send` for the requests a read brings). nginx
and lighttpd are landmarks. The cores: the CPU time of the server over the
time of the run (all its threads and processes). The servers of several
threads or processes have one for each CPU they are given, wrk one for
each CPU of the client (at most one a connection). Measured at 5712a39
(D377), 10 s a point.

## The server on 2 cores, the client on 6

The servers are the limit here (each keeps its 2 cores busy from 16
connections on): the measure of the cost of a request.

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 70306 (0.58 cores) | 381773 (2.02 cores) | 427716 (2.00 cores) | 352451 (1.98 cores) |
| Isocron `serveAction` | 70362 (0.59 cores) | 199283 (1.06 cores) | 203726 (1.20 cores) | 165795 (1.30 cores) |
| nginx (2 workers) | 72701 (0.56 cores) | 365982 (2.02 cores) | 373032 (2.00 cores) | 362714 (2.00 cores) |
| lighttpd | 71554 (0.62 cores) | 185351 (1.01 cores) | 189786 (1.00 cores) | 182385 (0.99 cores) |
| ceiling, 1 thread | 82950 (0.46 cores) | 267610 (1.01 cores) | 279379 (1.00 cores) | 262409 (0.99 cores) |
| ceiling, 2 threads | 82342 (0.47 cores) | 516809 (2.02 cores) | 548700 (2.00 cores) | 525840 (1.97 cores) |

| Isocron `serve` over | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| nginx | 96.7 % | 104.3 % | 114.7 % | 97.2 % |
| the ceiling of 2 threads | 85.4 % | 73.9 % | 78.0 % | 67.0 % |

`serve` runs a loop of events on each thread of the pool, each on its own
listener of the port (D374). Against the measure of D374 on the same
cores: +10.4, +7.9 and +9.6 % with 16, 256 and 1024 connections (D371:
the small blocks moved between the caches of the threads by whole lists;
D376: the vectors of a wait kept from one wait to the next). The ceiling
is 41 to 47 % above nginx: there is room to be far above it. The cost of
a request with 16 connections (D376, `perf stat -p`): 18534 cycles, the
kernel 12205 and the program 6329; the ceiling 13113, 12425 and 688. Both
make the same system calls (one `recv` and one `send` a request, one
`epoll_wait` for about 6 of them): the whole gap is in the program (about
43 allocations a request after D377, none in the ceiling).

## The server on 4 cores, the client on 4

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 72647 (0.57 cores) | 637367 (3.74 cores) | 822477 (3.98 cores) | 759964 (3.95 cores) |
| Isocron `serveAction` | 70325 (0.59 cores) | 207621 (1.47 cores) | 207162 (2.35 cores) | 166226 (2.00 cores) |
| nginx (4 workers) | 72435 (0.56 cores) | 519451 (2.92 cores) | 784310 (3.99 cores) | 747910 (3.94 cores) |
| lighttpd | 71627 (0.63 cores) | 188224 (1.01 cores) | 195987 (1.00 cores) | 184912 (0.99 cores) |
| ceiling, 1 thread | 82778 (0.48 cores) | 273205 (1.01 cores) | 296015 (1.00 cores) | 265645 (0.99 cores) |
| ceiling, 4 threads | 82519 (0.47 cores) | 742364 (3.19 cores) | 798387 (3.25 cores) | 795793 (3.11 cores) |

Here the ceiling keeps only 3.11 to 3.25 of its 4 cores busy and is 1.8 %
above nginx with 256 connections, 6.4 % with 1024: something outside the
servers limits them, most likely wrk (its CPU time not measured: a
hypothesis), and Isocron's 103.0 % of the ceiling with 256 connections
says as much. Isocron is 100.3 to 122.7 % of nginx there, but the measure
of 2 cores above is the one that compares the servers.

## Before: one loop of events (D366)

Measured at 5d7f925 (D366, after D359 to D363) on 4 cores for the server
and 4 for wrk:

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
| the ceiling of 4 cores (a bound below: it does not saturate its cores) | 85 % | 27 % | 26 % | 20 % |

The ceiling of 4 cores is a bound below: it keeps 3.09 to 3.28 of its 4
cores busy, so something outside it limits it, most likely the client (wrk,
4 threads on 4 cores; its CPU time not measured: a hypothesis); one
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
