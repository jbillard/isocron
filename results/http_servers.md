# The server of HTTP against the ceiling of the machine

`bench/http_servers/` (DECISIONS D353): `hello, world!` (13 bytes) to every
request, by the server of Isocron (`http.serve`, a pure handler; and
`http.serve` given an action, D386), nginx 1.26.3 (a worker for each CPU of
the server, the text from its configuration) and lighttpd 1.4.79 (one
process, the text from a file); h2o is not in Debian trixie. In the Docker
image `isocron-http` (that of the Benchmarks Game, with nginx, lighttpd and
wrk), without network: the server on 2 or 4 CPUs, wrk on the others of the
8 of the measures, connections kept alive, one request at a time each, 10 s
for each count, after `bench/machine.sh apply parallel` (fixed frequency),
on an Intel Xeon E-2388G (`measure.sh SERVER CLIENT 10`). Requests a
second, the cores busy in brackets.

The reference is the machine: the **ceiling**, the requests a second of
the cheapest server known for the same work (`ceiling.c`, D355: a loop of
`epoll` for each thread, a listener for each (`SO_REUSEPORT`), a request
found by its end, the 130 bytes of the response of Isocron made once and
copied, one `recv` and one `send` for the requests a read brings). nginx
and lighttpd are landmarks. The cores: the CPU time of the server over the
time of the run (all its threads and processes). The servers of several
threads or processes have one for each CPU they are given, wrk one for
each CPU of the client (at most one a connection).

The memory (D732): the sum of the Pss of the processes of the server (a
page shared by n processes counts for 1/n in each, so that the master and
the workers of nginx count their common pages once), at rest before the
first run, then its highest while wrk runs (read every 0.1 s), their Rss
in brackets. wrk runs twice for each count (D743): the requests, the
latency and the cores from a first run with nothing beside it, the memory
from a second run sampled (the sampler, on the CPUs of the client, moved
the latency of 99 % by up to 2 ms on 4 cores, where wrk is the limit); every server started without huge pages (`nothp.c`), its
memory in pages of 4 KiB. The Pss of a server linked with the C library
falls a little when wrk starts, which maps the same library: its pages
then shared by one more process (a peak under the memory at rest). The
server of Isocron calls no C library: its Pss is its Rss.

Measured with the compiler of D755 (D754 and D755: the `iscc` built from
the sources of its commit, the server it makes the same byte for byte),
2026-11-24, 10 s a point; Isocron 0.9.0 (D732, and rounds alternated with
it, D743, D754, D755) in the text after each table.

## The server on 2 cores, the client on 6

The servers are the limit here (each keeps its 2 cores busy from 16
connections on): the measure of the cost of a request. (At 03b2847, D535:
`serve` 105.8, 111.8, 124.9 and 119.1 % of nginx, 92.8, 79.4, 82.5 and
78.6 % of the ceiling of 2 threads.)

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 74497 (0.51 cores) | 435207 (2.02 cores) | 491673 (2.00 cores) | 448111 (1.98 cores) |
| Isocron `serve`, an action | 75945 (0.51 cores) | 440777 (2.02 cores) | 487684 (2.00 cores) | 470795 (1.97 cores) |
| nginx (2 workers) | 71929 (0.57 cores) | 368946 (2.01 cores) | 380184 (2.00 cores) | 360652 (1.99 cores) |
| lighttpd | 70642 (0.63 cores) | 184342 (1.01 cores) | 189042 (1.00 cores) | 178138 (0.99 cores) |
| ceiling, 1 thread | 82289 (0.47 cores) | 265537 (1.01 cores) | 280548 (1.00 cores) | 260392 (0.99 cores) |
| ceiling, 2 threads | 81897 (0.47 cores) | 508790 (2.02 cores) | 555509 (1.99 cores) | 534178 (1.97 cores) |

| Isocron over | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| `serve` over nginx | 103.6 % | 118.0 % | 129.3 % | 124.3 % |
| `serve` over the ceiling of 2 threads | 91.0 % | 85.5 % | 88.5 % | 83.9 % |
| `serve`, an action over nginx | 105.6 % | 119.5 % | 128.3 % | 130.5 % |
| `serve`, an action over the ceiling of 2 threads | 92.7 % | 86.6 % | 87.8 % | 88.1 % |

The latency of 50 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 12.00us | 25.00us | 500.00us | 2.11ms |
| Isocron `serve`, an action | 12.00us | 25.00us | 503.00us | 2.09ms |
| nginx (2 workers) | 13.00us | 31.00us | 663.00us | 2.76ms |
| lighttpd | 13.00us | 64.00us | 1.32ms | 5.68ms |
| ceiling, 1 thread | 11.00us | 37.00us | 0.89ms | 3.77ms |
| ceiling, 2 threads | 11.00us | 24.00us | 443.00us | 1.83ms |

The latency of 99 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 18.00us | 50.00us | 748.00us | 4.26ms |
| Isocron `serve`, an action | 17.00us | 48.00us | 772.00us | 3.77ms |
| nginx (2 workers) | 18.00us | 48.00us | 720.00us | 3.46ms |
| lighttpd | 18.00us | 120.00us | 2.61ms | 11.67ms |
| ceiling, 1 thread | 15.00us | 79.00us | 0.97ms | 5.76ms |
| ceiling, 2 threads | 15.00us | 42.00us | 766.00us | 2.96ms |

The memory of the server (all its processes): the sum of their Pss at rest, then its highest while wrk runs (their Rss in brackets):

| server | at rest | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: | --: |
| Isocron `serve` | 580 KiB | 724 KiB (728 KiB) | 932 KiB (936 KiB) | 1008 KiB (1012 KiB) | 1.0 MiB (1.0 MiB) |
| Isocron `serve`, an action | 532 KiB | 612 KiB (616 KiB) | 820 KiB (824 KiB) | 896 KiB (900 KiB) | 924 KiB (928 KiB) |
| nginx (2 workers) | 6.1 MiB | 6.0 MiB (8.6 MiB) | 5.9 MiB (8.6 MiB) | 5.9 MiB (8.6 MiB) | 5.9 MiB (8.6 MiB) |
| lighttpd | 1.5 MiB | 1.5 MiB (2.7 MiB) | 1.6 MiB (2.8 MiB) | 2.5 MiB (3.7 MiB) | 5.4 MiB (6.6 MiB) |
| ceiling, 1 thread | 591 KiB | 486 KiB (1.6 MiB) | 595 KiB (1.7 MiB) | 1.5 MiB (2.6 MiB) | 4.5 MiB (5.6 MiB) |
| ceiling, 2 threads | 604 KiB | 518 KiB (1.7 MiB) | 615 KiB (1.7 MiB) | 2.1 MiB (3.3 MiB) | 7.4 MiB (8.6 MiB) |

Against 0.9.0 (D732: 75145, 410388, 449512 and 413544 requests a second,
the latency of 99 % 17 us, 56 us, 0.85 ms and 3.78 ms): the memory of the
server no longer grows with its connections, 2.7 MiB at 1024 connections
then, 1032 KiB now, 100 KiB more than at 16 (D736 to D741: the table of the
connections grown in place and holding only those with bytes pending, the
first blocks of a class of a thread taken from a run of 32 by its own
needs, 16 connections a turn at most). In 8 rounds alternated with 0.9.0,
nothing beside wrk (D754 at 1 and 16 connections, D755 at 256 and 1024,
where the tries of its loops act): +2.1, +7.5, +9.7 and +13.6 % requests
a second with 1, 16, 256 and 1024 connections, the latency of 50 % 12,
26, 497 us and 2.10 ms against 12, 26, 550 us and 2.42 ms, that of 99 %
17, 41.5, 836 us and 3.84 ms against 17.5, 56.5, 861 us and 4.33 ms, as
many cores busy, the CPU time of a request 6.8 to 11.7 % lower; given an
action +2.7, +7.5, +11.2 and +14.5 % requests, the latency within 5 % of
0.9.0 or below; with 4 threads on these 2 cores (`ISC_THREADS=4`) +11.7
and +13.2 % requests at 256 and 1024 connections, the latency of 99 %
8.26 and 9.13 ms against 8.29 and 9.36 (the slices of the system: each
loop asks for 100 us while the loops outnumber the processors, D743).
The latency of 99 % with 16 connections moves from one run to the next,
37 to 47 us in 8 rounds (50 us above), 0.9.0 44 to 63; in six rounds
alternated with nginx 40.5 us against 52 (D754): the kernel gave each connection
to the listener of one loop by a hash of its addresses (`SO_REUSEPORT`),
12 connections split 6 and 6 or 2 and 10; the loops now share one
listener, each connection given to the loop that has the fewest (D739).

`serve` runs a loop of events on each thread of the pool, all on one
listener of the port, the loop that the kernel wakes giving each new
connection to the loop that has the fewest (D739; a listener for each
loop before, D374). Against the measure of D374 on the same
cores (345784, 396260 and 321534 requests a second with 16, 256 and 1024
connections): D371 (the small blocks moved between the caches of the
threads by whole lists), D376 (the vectors of a wait kept from one wait to
the next), D377 and D379 (no block for an allocation of 0 bytes, 12.77 of
45.67 allocations a request). `serve` given an action (`serveAction` before D386) runs a loop on each thread too since D385, its responses written in one pass (at 070a376 `serveAction` answered 201043, 209652 and 191464 requests a second with 16, 256 and 1024 connections, on 1.07 to 1.30 cores).
The cost of a request with 16 connections (D376, `perf stat -p`): 18534
cycles, the kernel 12205 and the program 6329; the ceiling 13113, 12425
and 688; both make the same system calls (one `recv` and one `send` a
request, one `epoll_wait` for about 6 of them): the gap is in the program.

## The server on 4 cores, the client on 4

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 77604 (0.51 cores) | 737926 (3.80 cores) | 792245 (3.95 cores) | 769474 (3.87 cores) |
| Isocron `serve`, an action | 76549 (0.52 cores) | 719455 (3.79 cores) | 806614 (3.97 cores) | 774283 (3.80 cores) |
| nginx (4 workers) | 72801 (0.56 cores) | 629370 (3.82 cores) | 771332 (3.95 cores) | 726127 (3.96 cores) |
| lighttpd | 71852 (0.63 cores) | 188817 (1.01 cores) | 196322 (1.00 cores) | 180395 (0.99 cores) |
| ceiling, 1 thread | 81826 (0.47 cores) | 272710 (1.01 cores) | 297897 (1.00 cores) | 262750 (0.99 cores) |
| ceiling, 4 threads | 81475 (0.48 cores) | 738698 (3.17 cores) | 797187 (3.28 cores) | 761044 (3.06 cores) |

| Isocron over | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| `serve` over nginx | 106.6 % | 117.2 % | 102.7 % | 106.0 % |
| `serve` over the ceiling of 4 threads | 95.2 % | 99.9 % | 99.4 % | 101.1 % |
| `serve`, an action over nginx | 105.1 % | 114.3 % | 104.6 % | 106.6 % |
| `serve`, an action over the ceiling of 4 threads | 94.0 % | 97.4 % | 101.2 % | 101.7 % |

The latency of 50 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 12.00us | 19.00us | 151.00us | 633.00us |
| Isocron `serve`, an action | 12.00us | 20.00us | 149.00us | 628.00us |
| nginx (4 workers) | 13.00us | 20.00us | 286.00us | 1.29ms |
| lighttpd | 13.00us | 83.00us | 1.28ms | 5.62ms |
| ceiling, 1 thread | 11.00us | 51.00us | 842.00us | 3.70ms |
| ceiling, 4 threads | 11.00us | 19.00us | 156.00us | 644.00us |

The latency of 99 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 15.00us | 29.00us | 459.00us | 1.37ms |
| Isocron `serve`, an action | 17.00us | 31.00us | 390.00us | 3.71ms |
| nginx (4 workers) | 18.00us | 45.00us | 510.00us | 2.30ms |
| lighttpd | 18.00us | 160.00us | 2.55ms | 11.36ms |
| ceiling, 1 thread | 15.00us | 108.00us | 0.97ms | 6.20ms |
| ceiling, 4 threads | 15.00us | 31.00us | 324.00us | 1.27ms |

The memory of the server (all its processes): the sum of their Pss at rest, then its highest while wrk runs (their Rss in brackets):

| server | at rest | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: | --: |
| Isocron `serve` | 596 KiB | 740 KiB (744 KiB) | 1.1 MiB (1.1 MiB) | 1.4 MiB (1.4 MiB) | 1.4 MiB (1.4 MiB) |
| Isocron `serve`, an action | 572 KiB | 656 KiB (660 KiB) | 1.0 MiB (1.0 MiB) | 1.2 MiB (1.2 MiB) | 1.2 MiB (1.2 MiB) |
| nginx (4 workers) | 5.9 MiB | 5.6 MiB (8.5 MiB) | 5.6 MiB (8.5 MiB) | 5.6 MiB (8.5 MiB) | 5.6 MiB (8.5 MiB) |
| lighttpd | 1.5 MiB | 1.5 MiB (2.7 MiB) | 1.6 MiB (2.8 MiB) | 2.4 MiB (3.7 MiB) | 5.4 MiB (6.6 MiB) |
| ceiling, 1 thread | 635 KiB | 597 KiB (1.7 MiB) | 651 KiB (1.7 MiB) | 1.6 MiB (2.7 MiB) | 4.5 MiB (5.7 MiB) |
| ceiling, 4 threads | 711 KiB | 672 KiB (1.8 MiB) | 757 KiB (1.9 MiB) | 2.4 MiB (3.6 MiB) | 8.7 MiB (9.9 MiB) |

Here the ceiling keeps only 3.06 to 3.28 of its 4 cores busy: the
client limits them, wrk on its 4 CPUs (D742:
`perf stat` of wrk, 85 % of its cycles in the kernel, the same for every
server), and Isocron's 97 to 102 % of the ceiling from 16 connections on
says as much. The measure of 2 cores above is the one that compares the
servers. Against 0.9.0 (D732: 75693, 702499, 796872 and 766015 requests a
second, on 3.67 to 3.96 cores; 4.7 MiB at 1024 connections): 1.4 MiB at
1024 connections, and in 8 rounds alternated with 0.9.0, nothing beside
wrk (D754 at 1 and 16 connections, D755 at 256 and 1024): +2.6, +11.1,
+1.0 and +2.8 % requests a second with 1, 16, 256 and 1024 connections,
the latency of 99 % 16, 31, 517 us and 3.75 ms against 17, 39, 1840 us
and 4.54 ms (at 256 and 1024 it moves with the queue of wrk, 0.38 to
3.27 ms and 3.5 to 5.0 ms from one round to the next for both), that of
50 % 12, 19, 151 and 627 us against 12, 20.5, 153 and 608 us; given an
action the same but the latency of 50 % at 256 connections, 151 us at
each round against 142 (+6.3 %: every server since D739 at 149 to 153
us at each round, 0.9.0 at 139 to 151 or 180 to 264 by the round,
D755). The server needs 30 % fewer cycles of its own for a request than
0.9.0, and its loops slept between requests, each woken by wrk (D742: 200
thousand sleeps in 10 s at 1024 connections against 17 thousand, the
latency of 99 % 11 % above 0.9.0): a loop with at least 16 connections
now tries its set without sleeping before it sleeps, and a try that
found nothing leaves the next waits without one (D743); the cheaper
turns of D754 made those tries shorter (98 thousand sleeps, the latency
of 99 % 12 % above 0.9.0 in 16 rounds): a try now lasts up to twice the
mean work of the loop between two waits, and a wait left without one
whose events come within that time takes the tries again (D755: 42
thousand sleeps). The CPU time of the server for a request (the time of its threads on
a CPU, read from `schedstat`, 10 rounds alternated, D745): 5246 ns at
16 connections against 5321 for 0.9.0, 4902 at 256 against 4875 (the
difference within its standard error), given an action 5317 at 16
against 5273 (the same; perf attached, its tracepoints of the system
calls, made each call dearer and moved the requests a second by 6 to
8 %).

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
