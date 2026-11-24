# The server of HTTP against the ceiling of the machine

`bench/http_servers/` (DECISIONS D353): `hello, world!` (13 bytes) to every
request, by the server of Isocron (`http.serve`, a pure handler; and
`http.serve` given an action, D386), nginx 1.26.3 (a worker for each CPU of
the server, the text from its configuration) and lighttpd 1.4.79 (one
process, the text from a file); h2o is not in Debian trixie. In the Docker
image `isocron-http` (that of the Benchmarks Game, with nginx, lighttpd and
wrk), without network: the server on the first hardware thread of 2 or 4
cores, wrk on the cores after them and their siblings (10 or 8 CPUs; every
other process on the siblings of the first 4 cores, `bench/machine.sh
apply http`, fixed frequency, D757), connections kept alive, one request
at a time each, 10 s for each count, on an Intel Xeon E-2388G (8 cores,
16 threads; `measure.sh SERVER CLIENT 10`). Requests a second, the cores
busy in brackets.

The reference is the machine: the **ceiling**, the requests a second of
the cheapest server known for the same work (`ceiling.c`, D355: a loop of
`epoll` for each thread, a listener for each (`SO_REUSEPORT`), a request
found by its end, the 130 bytes of the response of Isocron made once and
copied, one `recv` and one `send` for the requests a read brings). nginx
and lighttpd are landmarks. The cores: the CPU time of the server over the
time of the run (all its threads and processes). The servers of several
threads or processes have one for each CPU they are given; wrk the most
threads, one a CPU of the client at most, that divide the connections (8
for 16, 256 and 1024), so that it opens as many as asked (D757: with a
thread for each CPU, 16 connections were 12 on 6 threads).

The memory (D732): the sum of the Pss of the processes of the server (a
page shared by n processes counts for 1/n in each, so that the master and
the workers of nginx count their common pages once), at rest before the
first run, then its highest while wrk runs (read every 0.1 s), their Rss
in brackets. wrk runs twice for each count (D743): the requests, the
latency and the cores from a first run with nothing beside it, the memory
from a second run sampled (the sampler, on the CPUs of the client, moved
the latency of 99 % by up to 2 ms on 4 cores when wrk was the limit,
D743); every server started without huge pages (`nothp.c`), its
memory in pages of 4 KiB. The Pss of a server linked with the C library
falls a little when wrk starts, which maps the same library: its pages
then shared by one more process (a peak under the memory at rest). The
server of Isocron calls no C library: its Pss is its Rss.

Measured with the compiler of D758 (the `iscc` built from the sources of
its commit, the server it makes the same byte for byte), 2026-11-24, with
the client of D757, 10 s a point; Isocron 0.9.0 (rounds alternated with
it, D758) in the text after each table.

## The server on 2 cores, the client on 6

The servers are the limit here (each keeps its 2 cores busy from 16
connections on): the measure of the cost of a request. (At 03b2847, D535:
`serve` 105.8, 111.8, 124.9 and 119.1 % of nginx, 92.8, 79.4, 82.5 and
78.6 % of the ceiling of 2 threads.)

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 71198 (0.55 cores) | 430472 (2.02 cores) | 473946 (2.02 cores) | 443686 (2.00 cores) |
| Isocron `serve`, an action | 76144 (0.52 cores) | 429329 (2.01 cores) | 476599 (2.02 cores) | 448469 (1.99 cores) |
| nginx (2 workers) | 70602 (0.58 cores) | 353513 (2.02 cores) | 361692 (2.03 cores) | 308727 (2.01 cores) |
| lighttpd | 69246 (0.64 cores) | 180529 (1.01 cores) | 181134 (1.01 cores) | 168894 (1.00 cores) |
| ceiling, 1 thread | 80537 (0.48 cores) | 255945 (1.01 cores) | 277378 (1.01 cores) | 256796 (1.00 cores) |
| ceiling, 2 threads | 78572 (0.50 cores) | 505270 (2.02 cores) | 548052 (2.02 cores) | 480946 (2.00 cores) |

| Isocron over | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| `serve` over nginx | 100.8 % | 121.8 % | 131.0 % | 143.7 % |
| `serve` over the ceiling of 2 threads | 90.6 % | 85.2 % | 86.5 % | 92.3 % |
| `serve`, an action over nginx | 107.8 % | 121.4 % | 131.8 % | 145.3 % |
| `serve`, an action over the ceiling of 2 threads | 96.9 % | 85.0 % | 87.0 % | 93.2 % |

The latency of 50 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 12.00us | 35.00us | 515.00us | 2.14ms |
| Isocron `serve`, an action | 12.00us | 35.00us | 511.00us | 2.12ms |
| nginx (2 workers) | 13.00us | 45.00us | 686.00us | 2.87ms |
| lighttpd | 13.00us | 86.00us | 1.42ms | 6.10ms |
| ceiling, 1 thread | 11.00us | 52.00us | 0.88ms | 3.75ms |
| ceiling, 2 threads | 12.00us | 24.00us | 445.00us | 1.84ms |

The latency of 99 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 18.00us | 60.00us | 0.87ms | 3.95ms |
| Isocron `serve`, an action | 17.00us | 64.00us | 0.85ms | 3.90ms |
| nginx (2 workers) | 19.00us | 72.00us | 1.08ms | 5.43ms |
| lighttpd | 19.00us | 195.00us | 3.86ms | 15.27ms |
| ceiling, 1 thread | 15.00us | 118.00us | 1.45ms | 6.44ms |
| ceiling, 2 threads | 15.00us | 68.00us | 754.00us | 3.51ms |

The memory of the server (all its processes): the sum of their Pss at rest, then its highest while wrk runs (their Rss in brackets):

| server | at rest | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: | --: |
| Isocron `serve` | 520 KiB | 664 KiB (668 KiB) | 948 KiB (952 KiB) | 1016 KiB (1020 KiB) | 1.0 MiB (1.0 MiB) |
| Isocron `serve`, an action | 528 KiB | 608 KiB (612 KiB) | 824 KiB (828 KiB) | 892 KiB (896 KiB) | 916 KiB (920 KiB) |
| nginx (2 workers) | 6.1 MiB | 6.0 MiB (8.6 MiB) | 5.9 MiB (8.6 MiB) | 5.9 MiB (8.6 MiB) | 5.9 MiB (8.6 MiB) |
| lighttpd | 1.5 MiB | 1.5 MiB (2.6 MiB) | 1.6 MiB (2.7 MiB) | 2.4 MiB (3.6 MiB) | 5.4 MiB (6.5 MiB) |
| ceiling, 1 thread | 589 KiB | 537 KiB (1.6 MiB) | 592 KiB (1.7 MiB) | 1.5 MiB (2.6 MiB) | 4.5 MiB (5.6 MiB) |
| ceiling, 2 threads | 644 KiB | 641 KiB (1.7 MiB) | 709 KiB (1.8 MiB) | 2.0 MiB (3.2 MiB) | 7.4 MiB (8.6 MiB) |

Against 0.9.0 (D732: 75145, 410388, 449512 and 413544 requests a second,
the latency of 99 % 17 us, 56 us, 0.85 ms and 3.78 ms): the memory of the
server no longer grows with its connections, 2.7 MiB at 1024 connections
then, 1040 KiB now, 92 KiB more than at 16 (D736 to D741: the table of the
connections grown in place and holding only those with bytes pending, the
first blocks of a class of a thread taken from a run of 32 by its own
needs, 16 connections a turn at most). In 8 rounds alternated with 0.9.0,
nothing beside wrk, with the client of D757: +2.3, +5.5, +7.9 and +13.9 %
requests a second with 1, 16, 256 and 1024 connections, the latency of
50 % 12, 35, 514 us and 2.14 ms against 13, 35.5, 559 us and 2.45 ms, that
of 99 % 17, 63.5, 870 us and 4.10 ms against 18, 99, 955 us and 4.56 ms,
as many cores busy, the CPU time of a request 5.0 to 11.9 % lower; given
an action +1.7, +5.1, +7.3 and +11.7 % requests, the latency of 99 % 18,
61, 860 us and 3.97 ms against 18, 88, 1070 us and 4.38 ms; with 4
threads on these 2 cores (`ISC_THREADS=4`) +11.9 and +8.5 % requests at
256 and 1024 connections, the latency of 99 % 8.27 and 9.36 ms against
8.32 and 9.53 (the slices of the system: each loop asks for 100 us while
the loops outnumber the processors, D743). Against nginx at 16
connections, 6 rounds alternated: the latency of 50 % 35 against 43 us,
that of 99 % 61 against 71 (given an action 63, below nginx at each
round; D758: the connections watched edge-triggered, each answered in the
order of its request; 82 against 71 before, D757). In 0.9.0 the kernel gave
each connection to the listener of one loop by a hash of its addresses
(`SO_REUSEPORT`), 18 split 9 and 9 or 12 and 6; the loops now share one
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
| Isocron `serve` | 75889 (0.51 cores) | 734604 (3.94 cores) | 964271 (4.03 cores) | 929684 (3.99 cores) |
| Isocron `serve`, an action | 74299 (0.52 cores) | 764128 (3.93 cores) | 977180 (4.04 cores) | 848653 (3.99 cores) |
| nginx (4 workers) | 72048 (0.58 cores) | 618317 (3.71 cores) | 681954 (3.98 cores) | 714378 (4.00 cores) |
| lighttpd | 69740 (0.64 cores) | 178107 (1.01 cores) | 190215 (1.01 cores) | 173741 (1.00 cores) |
| ceiling, 1 thread | 81605 (0.48 cores) | 263583 (1.01 cores) | 285105 (1.01 cores) | 251036 (1.00 cores) |
| ceiling, 4 threads | 80600 (0.48 cores) | 790559 (3.47 cores) | 989466 (3.87 cores) | 990287 (3.84 cores) |

| Isocron over | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| `serve` over nginx | 105.3 % | 118.8 % | 141.4 % | 130.1 % |
| `serve` over the ceiling of 4 threads | 94.2 % | 92.9 % | 97.5 % | 93.9 % |
| `serve`, an action over nginx | 103.1 % | 123.6 % | 143.3 % | 118.8 % |
| `serve`, an action over the ceiling of 4 threads | 92.2 % | 96.7 % | 98.8 % | 85.7 % |

The latency of 50 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 12.00us | 18.00us | 239.00us | 1.01ms |
| Isocron `serve`, an action | 12.00us | 17.00us | 237.00us | 1.03ms |
| nginx (4 workers) | 13.00us | 23.00us | 345.00us | 1.34ms |
| lighttpd | 13.00us | 87.00us | 1.35ms | 5.92ms |
| ceiling, 1 thread | 11.00us | 51.00us | 0.87ms | 3.86ms |
| ceiling, 4 threads | 11.00us | 17.00us | 206.00us | 498.00us |

The latency of 99 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 17.00us | 32.00us | 422.00us | 1.83ms |
| Isocron `serve`, an action | 17.00us | 30.00us | 414.00us | 1.87ms |
| nginx (4 workers) | 19.00us | 49.00us | 2.23ms | 2.50ms |
| lighttpd | 19.00us | 218.00us | 3.33ms | 14.03ms |
| ceiling, 1 thread | 15.00us | 110.00us | 1.42ms | 6.49ms |
| ceiling, 4 threads | 15.00us | 30.00us | 3.40ms | 2.37ms |

The memory of the server (all its processes): the sum of their Pss at rest, then its highest while wrk runs (their Rss in brackets):

| server | at rest | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: | --: |
| Isocron `serve` | 620 KiB | 764 KiB (768 KiB) | 1.1 MiB (1.1 MiB) | 1.3 MiB (1.3 MiB) | 1.3 MiB (1.3 MiB) |
| Isocron `serve`, an action | 576 KiB | 660 KiB (664 KiB) | 1.0 MiB (1.0 MiB) | 1.2 MiB (1.2 MiB) | 1.2 MiB (1.2 MiB) |
| nginx (4 workers) | 5.8 MiB | 5.6 MiB (8.6 MiB) | 5.6 MiB (8.6 MiB) | 5.6 MiB (8.6 MiB) | 5.6 MiB (8.6 MiB) |
| lighttpd | 1.4 MiB | 1.4 MiB (2.5 MiB) | 1.4 MiB (2.6 MiB) | 2.4 MiB (3.5 MiB) | 5.3 MiB (6.4 MiB) |
| ceiling, 1 thread | 592 KiB | 530 KiB (1.6 MiB) | 588 KiB (1.7 MiB) | 1.6 MiB (2.6 MiB) | 4.5 MiB (5.6 MiB) |
| ceiling, 4 threads | 637 KiB | 624 KiB (1.7 MiB) | 685 KiB (1.8 MiB) | 2.3 MiB (3.5 MiB) | 8.5 MiB (9.7 MiB) |

With the client of D757 (wrk on 4 cores and their siblings) the servers
are the limit from 256 connections on: Isocron and the ceiling on 3.8 to
4.0 cores (the ceiling kept 3.1 to 3.3 when wrk ran on 4 CPUs, D742: 85 %
of wrk's cycles in the kernel); at 16 connections the ceiling and nginx
keep 3.5 to 3.7 of their cores busy, Isocron 3.94. Against 0.9.0 (D732:
75693, 702499, 796872 and 766015 requests a second, on 3.67 to 3.96 cores;
4.7 MiB at 1024 connections): 1.3 MiB at 1024 connections, and in 8
rounds alternated with 0.9.0, nothing beside wrk, with the client of
D757: +1.8, +22.5, +14.0 and +11.6 % requests a second with 1, 16, 256
and 1024 connections, the latency of 99 % 17, 29, 426 us and 1.85 ms
against 18, 50.5, 653 us and 2.12 ms, that of 50 % 12, 17, 239 us and
1.01 ms against 12, 22, 263.5 us and 1.18 ms, the CPU time of a request
8.1 to 12.0 % lower; given an action +2.2, +17.1, +12.2 and +13.1 %
requests, the latency of 50 % 236.5 against 262.5 us at 256 connections,
that of 99 % 421 against 651.5 us (with wrk the limit, before D757, 151
against 142 us: the shape of wrk's turns, D756). In some rounds two loops
share a core for 0.2 to 0.9 s while another is left without one, the
latency of 99 % then 2 to 3.5 ms (at 256 connections 3 rounds of 8 for
D758, 1 for 0.9.0, D758). The server needs 30 % fewer cycles of its own for a request than
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
