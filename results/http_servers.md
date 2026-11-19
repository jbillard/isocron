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

Measured with Isocron 0.9.0 (its archive, the `iscc` of a9088b5; D732),
2026-11-19, 10 s a point.

## The server on 2 cores, the client on 6

The servers are the limit here (each keeps its 2 cores busy from 16
connections on): the measure of the cost of a request. (At 03b2847, D535:
`serve` 105.8, 111.8, 124.9 and 119.1 % of nginx, 92.8, 79.4, 82.5 and
78.6 % of the ceiling of 2 threads.)

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 75145 (0.53 cores) | 410388 (2.02 cores) | 449512 (1.99 cores) | 413544 (1.99 cores) |
| Isocron `serve`, an action | 76063 (0.53 cores) | 413036 (2.02 cores) | 454529 (1.99 cores) | 407142 (1.97 cores) |
| nginx (2 workers) | 72645 (0.55 cores) | 365453 (2.02 cores) | 371892 (2.00 cores) | 346518 (2.00 cores) |
| lighttpd | 70113 (0.63 cores) | 184543 (1.01 cores) | 193535 (1.01 cores) | 175359 (0.99 cores) |
| ceiling, 1 thread | 81590 (0.47 cores) | 266514 (1.01 cores) | 280655 (1.00 cores) | 258765 (0.99 cores) |
| ceiling, 2 threads | 81708 (0.47 cores) | 517433 (2.02 cores) | 564517 (2.00 cores) | 548623 (1.98 cores) |

| Isocron over | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| `serve` over nginx | 103.4 % | 112.3 % | 120.9 % | 119.3 % |
| `serve` over the ceiling of 2 threads | 92.0 % | 79.3 % | 79.6 % | 75.4 % |
| `serve`, an action over nginx | 104.7 % | 113.0 % | 122.2 % | 117.5 % |
| `serve`, an action over the ceiling of 2 threads | 93.1 % | 79.8 % | 80.5 % | 74.2 % |

The latency of 99 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 17.00us | 56.00us | 0.85ms | 3.78ms |
| Isocron `serve`, an action | 16.00us | 52.00us | 0.89ms | 3.85ms |
| nginx (2 workers) | 18.00us | 44.00us | 1.06ms | 4.79ms |
| lighttpd | 18.00us | 120.00us | 2.54ms | 11.62ms |
| ceiling, 1 thread | 15.00us | 79.00us | 1.36ms | 5.01ms |
| ceiling, 2 threads | 15.00us | 42.00us | 0.87ms | 2.95ms |

The memory of the server (all its processes): the sum of their Pss at rest, then its highest while wrk runs (their Rss in brackets):

| server | at rest | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: | --: |
| Isocron `serve` | 556 KiB | 748 KiB (752 KiB) | 996 KiB (1000 KiB) | 1.6 MiB (1.6 MiB) | 2.7 MiB (2.7 MiB) |
| Isocron `serve`, an action | 508 KiB | 632 KiB (636 KiB) | 900 KiB (904 KiB) | 1.6 MiB (1.6 MiB) | 2.6 MiB (2.6 MiB) |
| nginx (2 workers) | 6.0 MiB | 5.9 MiB (8.5 MiB) | 5.8 MiB (8.5 MiB) | 5.7 MiB (8.5 MiB) | 5.7 MiB (8.5 MiB) |
| lighttpd | 1.5 MiB | 1.4 MiB (2.7 MiB) | 1.5 MiB (2.8 MiB) | 2.3 MiB (3.7 MiB) | 5.2 MiB (6.6 MiB) |
| ceiling, 1 thread | 648 KiB | 532 KiB (1.7 MiB) | 549 KiB (1.7 MiB) | 1.4 MiB (2.7 MiB) | 4.5 MiB (5.7 MiB) |
| ceiling, 2 threads | 649 KiB | 554 KiB (1.7 MiB) | 572 KiB (1.8 MiB) | 1.5 MiB (2.7 MiB) | 5.0 MiB (6.2 MiB) |

`serve` runs a loop of events on each thread of the pool, each on its own
listener of the port (D374). Against the measure of D374 on the same
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
| Isocron `serve` | 75693 (0.53 cores) | 702499 (3.67 cores) | 796872 (3.96 cores) | 766015 (3.95 cores) |
| Isocron `serve`, an action | 76379 (0.54 cores) | 696287 (3.74 cores) | 799649 (3.88 cores) | 773174 (3.95 cores) |
| nginx (4 workers) | 73136 (0.56 cores) | 554974 (3.00 cores) | 740402 (3.88 cores) | 703915 (3.81 cores) |
| lighttpd | 70274 (0.63 cores) | 186552 (1.01 cores) | 198298 (1.00 cores) | 175496 (0.99 cores) |
| ceiling, 1 thread | 81858 (0.47 cores) | 276061 (1.01 cores) | 295520 (0.99 cores) | 257873 (0.99 cores) |
| ceiling, 4 threads | 82414 (0.47 cores) | 652629 (2.72 cores) | 792018 (3.34 cores) | 766841 (3.75 cores) |

| Isocron over | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| `serve` over nginx | 103.5 % | 126.6 % | 107.6 % | 108.8 % |
| `serve` over the ceiling of 4 threads | 91.8 % | 107.6 % | 100.6 % | 99.9 % |
| `serve`, an action over nginx | 104.4 % | 125.5 % | 108.0 % | 109.8 % |
| `serve`, an action over the ceiling of 4 threads | 92.7 % | 106.7 % | 101.0 % | 100.8 % |

The latency of 99 % of the requests (wrk):

| server | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: |
| Isocron `serve` | 17.00us | 39.00us | 481.00us | 1.91ms |
| Isocron `serve`, an action | 17.00us | 40.00us | 2.32ms | 1.77ms |
| nginx (4 workers) | 20.00us | 81.00us | 2.97ms | 3.93ms |
| lighttpd | 18.00us | 161.00us | 2.51ms | 11.52ms |
| ceiling, 1 thread | 15.00us | 108.00us | 1.27ms | 6.23ms |
| ceiling, 4 threads | 15.00us | 74.00us | 629.00us | 2.00ms |

The memory of the server (all its processes): the sum of their Pss at rest, then its highest while wrk runs (their Rss in brackets):

| server | at rest | 1 conn. | 16 conn. | 256 conn. | 1024 conn. |
| :-- | --: | --: | --: | --: | --: |
| Isocron `serve` | 572 KiB | 812 KiB (816 KiB) | 1.3 MiB (1.3 MiB) | 3.7 MiB (3.7 MiB) | 4.7 MiB (4.7 MiB) |
| Isocron `serve`, an action | 576 KiB | 724 KiB (728 KiB) | 1.2 MiB (1.2 MiB) | 3.5 MiB (3.5 MiB) | 4.7 MiB (4.7 MiB) |
| nginx (4 workers) | 5.8 MiB | 5.7 MiB (8.6 MiB) | 5.6 MiB (8.6 MiB) | 5.5 MiB (8.6 MiB) | 5.5 MiB (8.6 MiB) |
| lighttpd | 1.4 MiB | 1.3 MiB (2.7 MiB) | 1.4 MiB (2.7 MiB) | 2.3 MiB (3.6 MiB) | 5.2 MiB (6.5 MiB) |
| ceiling, 1 thread | 629 KiB | 550 KiB (1.7 MiB) | 551 KiB (1.7 MiB) | 1.5 MiB (2.7 MiB) | 4.5 MiB (5.7 MiB) |
| ceiling, 4 threads | 704 KiB | 594 KiB (1.8 MiB) | 649 KiB (1.8 MiB) | 1.6 MiB (2.8 MiB) | 5.4 MiB (6.6 MiB) |

Here the ceiling keeps only 2.72 to 3.75 of its 4 cores busy, and nginx
3.00 with 16 connections: something outside the servers limits them,
most likely wrk on its 4 CPUs (its CPU time not measured: a hypothesis),
and Isocron's 100 to 108 % of the ceiling says as much. The measure of 2
cores above is the one that compares the servers.

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
