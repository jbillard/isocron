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

Measured at a3b0a91 (D736 to D743: the `iscc` built from it, the same
byte for byte as the one measured), 2026-11-20, 10 s a point; Isocron
0.9.0 (D732, and rounds alternated with it, D743) in the text after each
table.

## The server on 2 cores, the client on 6

The servers are the limit here (each keeps its 2 cores busy from 16
connections on): the measure of the cost of a request. (At 03b2847, D535:
`serve` 105.8, 111.8, 124.9 and 119.1 % of nginx, 92.8, 79.4, 82.5 and
78.6 % of the ceiling of 2 threads.)

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 74801 (0.52 cores) | 418049 (2.02 cores) | 463504 (2.00 cores) | 453772 (1.99 cores) |
| Isocron `serve`, an action | 75333 (0.52 cores) | 422960 (2.02 cores) | 467966 (2.00 cores) | 445608 (1.97 cores) |
| nginx (2 workers) | 73363 (0.55 cores) | 365334 (2.02 cores) | 372041 (2.00 cores) | 358878 (2.00 cores) |
| lighttpd | 70906 (0.63 cores) | 185302 (1.01 cores) | 190408 (1.01 cores) | 181584 (1.00 cores) |
| ceiling, 1 thread | 82263 (0.47 cores) | 268477 (1.01 cores) | 284683 (1.00 cores) | 263554 (0.99 cores) |
| ceiling, 2 threads | 81789 (0.47 cores) | 514503 (2.02 cores) | 548318 (2.00 cores) | 545154 (1.97 cores) |

| Isocron over | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| `serve` over nginx | 102.0 % | 114.4 % | 124.6 % | 126.4 % |
| `serve` over the ceiling of 2 threads | 91.5 % | 81.3 % | 84.5 % | 83.2 % |
| `serve`, an action over nginx | 102.7 % | 115.8 % | 125.8 % | 124.2 % |
| `serve`, an action over the ceiling of 2 threads | 92.1 % | 82.2 % | 85.3 % | 81.7 % |

The latency of 50 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 12.00us | 27.00us | 534.00us | 2.19ms |
| Isocron `serve`, an action | 12.00us | 27.00us | 525.00us | 2.17ms |
| nginx (2 workers) | 13.00us | 34.00us | 659.00us | 2.77ms |
| lighttpd | 13.00us | 64.00us | 1.31ms | 5.58ms |
| ceiling, 1 thread | 11.00us | 36.00us | 0.87ms | 3.71ms |
| ceiling, 2 threads | 11.00us | 21.00us | 459.00us | 1.81ms |

The latency of 99 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 17.00us | 57.00us | 0.86ms | 3.05ms |
| Isocron `serve`, an action | 16.00us | 39.00us | 0.87ms | 3.95ms |
| nginx (2 workers) | 17.00us | 44.00us | 1.09ms | 4.84ms |
| lighttpd | 17.00us | 119.00us | 2.59ms | 11.05ms |
| ceiling, 1 thread | 15.00us | 79.00us | 1.01ms | 6.85ms |
| ceiling, 2 threads | 14.00us | 49.00us | 659.00us | 3.13ms |

The memory of the server (all its processes): the sum of their Pss at rest, then its highest while wrk runs (their Rss in brackets):

| server | at rest | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: | --: |
| Isocron `serve` | 536 KiB | 680 KiB (684 KiB) | 884 KiB (888 KiB) | 960 KiB (964 KiB) | 988 KiB (992 KiB) |
| Isocron `serve`, an action | 516 KiB | 596 KiB (600 KiB) | 800 KiB (804 KiB) | 876 KiB (880 KiB) | 904 KiB (908 KiB) |
| nginx (2 workers) | 6.1 MiB | 5.9 MiB (8.7 MiB) | 5.9 MiB (8.7 MiB) | 5.9 MiB (8.7 MiB) | 5.9 MiB (8.7 MiB) |
| lighttpd | 1.4 MiB | 1.3 MiB (2.5 MiB) | 1.4 MiB (2.5 MiB) | 2.3 MiB (3.4 MiB) | 5.2 MiB (6.4 MiB) |
| ceiling, 1 thread | 631 KiB | 586 KiB (1.7 MiB) | 629 KiB (1.7 MiB) | 1.6 MiB (2.7 MiB) | 4.6 MiB (5.7 MiB) |
| ceiling, 2 threads | 646 KiB | 631 KiB (1.7 MiB) | 668 KiB (1.8 MiB) | 2.1 MiB (3.4 MiB) | 7.3 MiB (8.6 MiB) |

Against 0.9.0 (D732: 75145, 410388, 449512 and 413544 requests a second,
the latency of 99 % 17 us, 56 us, 0.85 ms and 3.78 ms): the memory of the
server no longer grows with its connections, 2.7 MiB at 1024 connections
then, 988 KiB now, 104 KiB more than at 16 (D736 to D741: the table of the
connections grown in place and holding only those with bytes pending, the
first blocks of a class of a thread taken from a run of 32 by its own
needs, 16 connections a turn at most). In 8 rounds alternated with 0.9.0,
nothing beside wrk (D743): -0.3, +4.7, +4.8 and +9.1 % requests a second
with 1, 16, 256 and 1024 connections, the latency of 50 % 12, 26, 524 us
and 2.20 ms against 12, 25, 548 us and 2.40 ms, that of 99 % 17, 41, 860
us and 3.91 ms against 17, 56, 905 us and 3.84 ms, as many cores busy;
with 4 threads on these 2 cores (`ISC_THREADS=4`) +7.9 % requests at 256
and 1024 connections, the latency of 99 % 5.55 to 9.16 ms against 5.63 to
9.36 (the slices of the system: each loop asks for 100 us while the loops
outnumber the processors, D743). The latency of 99 % with 16 connections
moves from one run to the next, 39 to 71 us (57 us above); in six rounds
alternated with nginx 47 us against 50: the kernel gave each connection
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
| Isocron `serve` | 74867 (0.53 cores) | 737082 (3.90 cores) | 804735 (3.95 cores) | 771872 (3.84 cores) |
| Isocron `serve`, an action | 75412 (0.53 cores) | 734309 (3.89 cores) | 797995 (3.90 cores) | 783804 (3.85 cores) |
| nginx (4 workers) | 73877 (0.56 cores) | 604714 (3.61 cores) | 777796 (3.98 cores) | 748223 (3.95 cores) |
| lighttpd | 71588 (0.63 cores) | 187755 (1.01 cores) | 194926 (1.00 cores) | 175412 (0.99 cores) |
| ceiling, 1 thread | 82459 (0.47 cores) | 276660 (1.01 cores) | 282540 (1.00 cores) | 267454 (0.99 cores) |
| ceiling, 4 threads | 81088 (0.47 cores) | 671533 (2.87 cores) | 797139 (3.26 cores) | 782137 (3.12 cores) |

| Isocron over | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| `serve` over nginx | 101.3 % | 121.9 % | 103.5 % | 103.2 % |
| `serve` over the ceiling of 4 threads | 92.3 % | 109.8 % | 101.0 % | 98.7 % |
| `serve`, an action over nginx | 102.1 % | 121.4 % | 102.6 % | 104.8 % |
| `serve`, an action over the ceiling of 4 threads | 93.0 % | 109.3 % | 100.1 % | 100.2 % |

The latency of 50 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 12.00us | 19.00us | 150.00us | 625.00us |
| Isocron `serve`, an action | 12.00us | 19.00us | 151.00us | 624.00us |
| nginx (4 workers) | 13.00us | 19.00us | 287.00us | 1.27ms |
| lighttpd | 13.00us | 84.00us | 1.30ms | 5.79ms |
| ceiling, 1 thread | 11.00us | 52.00us | 0.88ms | 3.72ms |
| ceiling, 4 threads | 11.00us | 18.00us | 158.00us | 635.00us |

The latency of 99 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 17.00us | 30.00us | 398.00us | 2.43ms |
| Isocron `serve`, an action | 16.00us | 29.00us | 782.00us | 1.29ms |
| nginx (4 workers) | 18.00us | 50.00us | 483.00us | 1.83ms |
| lighttpd | 17.00us | 160.00us | 2.57ms | 13.58ms |
| ceiling, 1 thread | 14.00us | 108.00us | 1.42ms | 4.64ms |
| ceiling, 4 threads | 15.00us | 48.00us | 315.00us | 1.26ms |

The memory of the server (all its processes): the sum of their Pss at rest, then its highest while wrk runs (their Rss in brackets):

| server | at rest | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: | --: |
| Isocron `serve` | 564 KiB | 712 KiB (716 KiB) | 1.1 MiB (1.1 MiB) | 1.2 MiB (1.3 MiB) | 1.3 MiB (1.3 MiB) |
| Isocron `serve`, an action | 624 KiB | 644 KiB (648 KiB) | 1020 KiB (1.0 MiB) | 1.2 MiB (1.2 MiB) | 1.2 MiB (1.2 MiB) |
| nginx (4 workers) | 5.9 MiB | 5.7 MiB (8.7 MiB) | 5.6 MiB (8.7 MiB) | 5.6 MiB (8.7 MiB) | 5.6 MiB (8.7 MiB) |
| lighttpd | 1.5 MiB | 1.5 MiB (2.7 MiB) | 1.5 MiB (2.8 MiB) | 2.4 MiB (3.7 MiB) | 5.4 MiB (6.6 MiB) |
| ceiling, 1 thread | 625 KiB | 602 KiB (1.7 MiB) | 663 KiB (1.8 MiB) | 1.6 MiB (2.7 MiB) | 4.6 MiB (5.7 MiB) |
| ceiling, 4 threads | 703 KiB | 681 KiB (1.8 MiB) | 795 KiB (1.9 MiB) | 2.4 MiB (3.7 MiB) | 8.7 MiB (9.9 MiB) |

Here the ceiling keeps only 2.87 to 3.26 of its 4 cores busy, and nginx
3.61 with 16 connections: the client limits them, wrk on its 4 CPUs (D742:
`perf stat` of wrk, 85 % of its cycles in the kernel, the same for every
server), and Isocron's 99 to 110 % of the ceiling from 16 connections on
says as much. The measure of 2 cores above is the one that compares the
servers. Against 0.9.0 (D732: 75693, 702499, 796872 and 766015 requests a
second, on 3.67 to 3.96 cores; 4.7 MiB at 1024 connections): 1.3 MiB at
1024 connections, and in 8 rounds alternated with 0.9.0, nothing beside
wrk (D743): +1.1, +9.2, -0.6 and +1.2 % requests a second with 1, 16, 256
and 1024 connections, the latency of 99 % 17, 30, 573 us and 4.00 ms
against 17, 41, 684 us and 4.08 ms (at 256 and 1024 it moves with the
queue of wrk, 0.4 to 2.7 ms and 3.0 to 5.2 ms from one round to the next
for both), that of 50 % 12, 19, 150 and 624 us against 12, 20, 146 and
605 us. The server needs 30 % fewer cycles of its own for a request than
0.9.0, and its loops slept between requests, each woken by wrk (D742: 200
thousand sleeps in 10 s at 1024 connections against 17 thousand, the
latency of 99 % 11 % above 0.9.0): a loop with at least 16 connections
now tries its set without sleeping for at most its mean work between two
waits before it sleeps, and a try that found nothing leaves the next
waits without one (D743). The cycles of the server for a request at 16
connections, given an action (`perf stat -p`, 14 rounds alternated):
17370 now, 17431 for 0.9.0.

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
