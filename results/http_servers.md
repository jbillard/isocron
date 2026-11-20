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
in brackets; every server started without huge pages (`nothp.c`), its
memory in pages of 4 KiB. The Pss of a server linked with the C library
falls a little when wrk starts, which maps the same library: its pages
then shared by one more process (a peak under the memory at rest). The
server of Isocron calls no C library: its Pss is its Rss.

Measured at d4bdcc1 (D736 to D742: the `iscc` built from it, the same
byte for byte as the one measured), 2026-11-20, 10 s a point; Isocron
0.9.0 (D732) in the text after each table.

## The server on 2 cores, the client on 6

The servers are the limit here (each keeps its 2 cores busy from 16
connections on): the measure of the cost of a request. (At 03b2847, D535:
`serve` 105.8, 111.8, 124.9 and 119.1 % of nginx, 92.8, 79.4, 82.5 and
78.6 % of the ceiling of 2 threads.)

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 75104 (0.52 cores) | 420630 (2.02 cores) | 467574 (1.99 cores) | 441773 (1.99 cores) |
| Isocron `serve`, an action | 74456 (0.53 cores) | 422076 (2.02 cores) | 467990 (2.00 cores) | 438041 (1.98 cores) |
| nginx (2 workers) | 71342 (0.57 cores) | 359502 (2.01 cores) | 368784 (2.00 cores) | 354052 (2.00 cores) |
| lighttpd | 71413 (0.63 cores) | 186938 (1.01 cores) | 189556 (1.00 cores) | 179407 (0.99 cores) |
| ceiling, 1 thread | 82443 (0.47 cores) | 266551 (1.01 cores) | 291944 (1.00 cores) | 256511 (1.00 cores) |
| ceiling, 2 threads | 82124 (0.48 cores) | 511675 (2.02 cores) | 558421 (1.99 cores) | 545860 (1.98 cores) |

| Isocron over | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| `serve` over nginx | 105.3 % | 117.0 % | 126.8 % | 124.8 % |
| `serve` over the ceiling of 2 threads | 91.5 % | 82.2 % | 83.7 % | 80.9 % |
| `serve`, an action over nginx | 104.4 % | 117.4 % | 126.9 % | 123.7 % |
| `serve`, an action over the ceiling of 2 threads | 90.7 % | 82.5 % | 83.8 % | 80.2 % |

The latency of 50 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 12.00us | 27.00us | 523.00us | 2.24ms |
| Isocron `serve`, an action | 12.00us | 27.00us | 516.00us | 2.23ms |
| nginx (2 workers) | 13.00us | 33.00us | 666.00us | 2.80ms |
| lighttpd | 13.00us | 63.00us | 1.32ms | 5.63ms |
| ceiling, 1 thread | 11.00us | 37.00us | 846.00us | 3.80ms |
| ceiling, 2 threads | 11.00us | 21.00us | 425.00us | 1.80ms |

The latency of 99 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 16.00us | 40.00us | 0.87ms | 3.02ms |
| Isocron `serve`, an action | 18.00us | 40.00us | 0.89ms | 3.91ms |
| nginx (2 workers) | 19.00us | 45.00us | 1.05ms | 4.72ms |
| lighttpd | 17.00us | 118.00us | 2.62ms | 11.36ms |
| ceiling, 1 thread | 14.00us | 80.00us | 1.09ms | 6.51ms |
| ceiling, 2 threads | 15.00us | 46.00us | 840.00us | 3.02ms |

The memory of the server (all its processes): the sum of their Pss at rest, then its highest while wrk runs (their Rss in brackets):

| server | at rest | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: | --: |
| Isocron `serve` | 492 KiB | 628 KiB (632 KiB) | 820 KiB (824 KiB) | 896 KiB (900 KiB) | 920 KiB (924 KiB) |
| Isocron `serve`, an action | 512 KiB | 584 KiB (588 KiB) | 780 KiB (784 KiB) | 852 KiB (856 KiB) | 880 KiB (884 KiB) |
| nginx (2 workers) | 6.0 MiB | 5.9 MiB (8.5 MiB) | 5.8 MiB (8.5 MiB) | 5.7 MiB (8.5 MiB) | 5.7 MiB (8.5 MiB) |
| lighttpd | 1.5 MiB | 1.3 MiB (2.6 MiB) | 1.4 MiB (2.7 MiB) | 2.3 MiB (3.6 MiB) | 5.2 MiB (6.5 MiB) |
| ceiling, 1 thread | 598 KiB | 500 KiB (1.6 MiB) | 496 KiB (1.7 MiB) | 1.4 MiB (2.6 MiB) | 4.5 MiB (5.6 MiB) |
| ceiling, 2 threads | 678 KiB | 578 KiB (1.7 MiB) | 608 KiB (1.8 MiB) | 1.5 MiB (2.7 MiB) | 5.2 MiB (6.4 MiB) |

Against 0.9.0 (D732: 75145, 410388, 449512 and 413544 requests a second,
the latency of 99 % 17 us, 56 us, 0.85 ms and 3.78 ms; and the A/B of
D742, two rounds alternated: -0.3, +5.1, +5.3 and +7.2 % requests a
second, the latency of 50 % the same or lower at every count): the memory
of the server no longer grows with its connections, 2.7 MiB at 1024
connections then, 920 KiB now, 100 KiB more than at 16 (D736 to D741: the
table of the connections grown in place and holding only those with bytes
pending, the first blocks of a class of a thread carved by its own needs,
16 connections a turn at most). The latency of 99 % with 16 connections,
56 us against 44 for nginx then: the kernel gave each connection to the
listener of one loop by a hash of its addresses (`SO_REUSEPORT`), 12
connections split 6 and 6 or 2 and 10; the loops now share one listener,
each connection given to the loop that has the fewest (D739): 40 us here,
and in six rounds alternated with nginx and 0.9.0 (D742) 45 to 49 us
against nginx's 42 to 55 and 0.9.0's 48 to 73.

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
| Isocron `serve` | 76621 (0.52 cores) | 714762 (3.82 cores) | 806270 (3.86 cores) | 747885 (3.51 cores) |
| Isocron `serve`, an action | 75865 (0.52 cores) | 734886 (3.88 cores) | 799407 (3.85 cores) | 762078 (3.61 cores) |
| nginx (4 workers) | 72932 (0.56 cores) | 641675 (3.68 cores) | 754055 (3.90 cores) | 716756 (3.85 cores) |
| lighttpd | 71686 (0.63 cores) | 188441 (1.01 cores) | 194435 (1.00 cores) | 181193 (0.99 cores) |
| ceiling, 1 thread | 82982 (0.47 cores) | 277464 (1.01 cores) | 295591 (1.00 cores) | 266602 (0.98 cores) |
| ceiling, 4 threads | 82167 (0.47 cores) | 727546 (3.19 cores) | 780342 (3.22 cores) | 762278 (3.06 cores) |

| Isocron over | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| `serve` over nginx | 105.1 % | 111.4 % | 106.9 % | 104.3 % |
| `serve` over the ceiling of 4 threads | 93.2 % | 98.2 % | 103.3 % | 98.1 % |
| `serve`, an action over nginx | 104.0 % | 114.5 % | 106.0 % | 106.3 % |
| `serve`, an action over the ceiling of 4 threads | 92.3 % | 101.0 % | 102.4 % | 100.0 % |

The latency of 50 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 12.00us | 20.00us | 149.00us | 639.00us |
| Isocron `serve`, an action | 12.00us | 19.00us | 215.00us | 628.00us |
| nginx (4 workers) | 13.00us | 22.00us | 272.00us | 642.00us |
| lighttpd | 13.00us | 84.00us | 1.30ms | 5.56ms |
| ceiling, 1 thread | 11.00us | 51.00us | 841.00us | 3.66ms |
| ceiling, 4 threads | 11.00us | 19.00us | 155.00us | 638.00us |

The latency of 99 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 16.00us | 33.00us | 497.00us | 4.78ms |
| Isocron `serve`, an action | 17.00us | 32.00us | 2.55ms | 3.92ms |
| nginx (4 workers) | 18.00us | 471.00us | 2.47ms | 3.99ms |
| lighttpd | 17.00us | 160.00us | 2.57ms | 11.10ms |
| ceiling, 1 thread | 14.00us | 108.00us | 1.41ms | 5.90ms |
| ceiling, 4 threads | 14.00us | 39.00us | 360.00us | 1.50ms |

The memory of the server (all its processes): the sum of their Pss at rest, then its highest while wrk runs (their Rss in brackets):

| server | at rest | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: | --: |
| Isocron `serve` | 524 KiB | 732 KiB (736 KiB) | 1.1 MiB (1.1 MiB) | 1.2 MiB (1.2 MiB) | 1.3 MiB (1.3 MiB) |
| Isocron `serve`, an action | 524 KiB | 596 KiB (600 KiB) | 952 KiB (956 KiB) | 1.1 MiB (1.1 MiB) | 1.2 MiB (1.2 MiB) |
| nginx (4 workers) | 5.9 MiB | 5.8 MiB (8.7 MiB) | 5.7 MiB (8.7 MiB) | 5.6 MiB (8.7 MiB) | 5.6 MiB (8.7 MiB) |
| lighttpd | 1.5 MiB | 1.4 MiB (2.7 MiB) | 1.4 MiB (2.8 MiB) | 2.3 MiB (3.7 MiB) | 5.3 MiB (6.6 MiB) |
| ceiling, 1 thread | 581 KiB | 496 KiB (1.6 MiB) | 511 KiB (1.7 MiB) | 1.4 MiB (2.6 MiB) | 4.5 MiB (5.7 MiB) |
| ceiling, 4 threads | 639 KiB | 545 KiB (1.7 MiB) | 618 KiB (1.8 MiB) | 1.6 MiB (2.8 MiB) | 5.3 MiB (6.5 MiB) |

Here the ceiling keeps only 3.06 to 3.22 of its 4 cores busy, and nginx
3.68 with 16 connections: the client limits them, wrk on its 4 CPUs (D742:
`perf stat` of wrk, 85 % of its cycles in the kernel, the same for every
server), and Isocron's 98 to 103 % of the ceiling from 16 connections on
says as much. The
measure of 2 cores above is the one that compares the servers. Against
0.9.0 (D732: 75693, 702499, 796872 and 766015 requests a second, on 3.67
to 3.96 cores; 4.7 MiB at 1024 connections): as many requests a second,
1.3 MiB at 1024 connections, and fewer cores busy at 1024 (3.51 against
3.95): the server needs 30 % fewer cycles of its own for a request, so its
loops sleep between requests, each woken by wrk (D742: 54 to 84 thousand
context switches in 4 s against 3 to 8 thousand). Its latency of 99 % at
256 and 1024 connections moves with the queue of wrk, from one run to the
next (D742, 11 and 15 runs of each: at 256 0.5 to 2.9 ms for 0.9.0, 0.8
to 3.4 now; at 1024 1.9 to 5.1 ms, 3.8 to 5.1 now): 0.5 and 0.8 ms higher
than 0.9.0 on the mean (2.03 against 1.55 ms, 4.75 against 3.96), the cost
of those wake-ups (a loop that tried its wait 200 times
before sleeping had it lower than 0.9.0, but kept a core busy for a
single connection on 2 cores and lost 1.7 % of the requests with 16:
refused).

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
