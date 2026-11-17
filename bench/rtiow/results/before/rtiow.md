# rtiow: Rust and Isocron

Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz, Debian GNU/Linux 13 (trixie), 6.12.115+deb13.1-amd64; rustc 1.99.1 (58cf8f9c3 2026-10-13); Isocron c1c4107.

## Builds

| Build | Wall (median) | Processor (median) |
| --- | ---: | ---: |
| Rust, clean, as shipped (with `images`) | 6.52 s | 21.70 s |
| Rust, clean (without `images`) | 6.29 s | 14.70 s |
| Rust, clean, `target-cpu=native` | 6.34 s | 14.64 s |
| Isocron (`iscc build`) | 2.91 s | 2.91 s |
| Rust, one file touched | 3.02 s | 6.78 s |

| Program | Size |
| --- | ---: |
| Rust, as shipped (with `images`) | 728 KiB |
| Rust | 721 KiB |
| Rust, `target-cpu=native` | 711 KiB |
| Isocron | 363 KiB |

## Renders, 8 threads

| Scene | Program | Wall (median) | Min | Q1 to Q3 | Processor (median) | Same bytes |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| sphere (100 samples, 384x216) | Rust | 2.779 s | 2.773 s | 2.778 to 2.793 s | 22.188 s | yes |
| sphere (100 samples, 384x216) | Rust, `target-cpu=native` | 2.792 s | 2.775 s | 2.781 to 2.794 s | 22.285 s | yes |
| sphere (100 samples, 384x216) | Isocron | 2.725 s | 2.712 s | 2.716 to 2.727 s | 21.407 s | yes |
| final (20 samples, 384x216) | Rust | 2.404 s | 2.370 s | 2.402 to 2.415 s | 19.169 s | yes |
| final (20 samples, 384x216) | Rust, `target-cpu=native` | 2.373 s | 2.348 s | 2.372 to 2.376 s | 18.924 s | yes |
| final (20 samples, 384x216) | Isocron | 3.478 s | 3.391 s | 3.469 to 3.537 s | 25.027 s | yes |

## Renders, 1 thread

| Scene | Program | Wall (median) | Min | Q1 to Q3 | Processor (median) | Same bytes |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| sphere (100 samples, 384x216) | Rust | 7.006 s | 7.000 s | 7.003 to 7.036 s | 7.005 s | yes |
| sphere (100 samples, 384x216) | Rust, `target-cpu=native` | 7.117 s | 7.112 s | 7.114 to 7.122 s | 7.110 s | yes |
| sphere (100 samples, 384x216) | Isocron | 17.785 s | 17.770 s | 17.778 to 17.801 s | 17.772 s | yes |
| final (20 samples, 384x216) | Rust | 10.769 s | 10.734 s | 10.751 to 10.781 s | 10.766 s | yes |
| final (20 samples, 384x216) | Rust, `target-cpu=native` | 10.383 s | 10.358 s | 10.371 to 10.384 s | 10.380 s | yes |
| final (20 samples, 384x216) | Isocron | 23.706 s | 23.699 s | 23.702 to 23.733 s | 23.700 s | yes |

## Peak resident memory (`VmHWM`)

| Scene | Threads | Rust | Isocron |
| --- | ---: | ---: | ---: |
| sphere (100 samples, 384x216) | 1 | 3.3 MiB | 20.7 MiB |
| final (20 samples, 384x216) | 1 | 3.8 MiB | 28.7 MiB |
| sphere (2000 samples, 2x2) | 1 | 2.8 MiB | 2.6 MiB |
| final (200 samples, 2x2) | 1 | 3.7 MiB | 11.0 MiB |
| sphere (100 samples, 384x216) | 8 | 3.6 MiB | 21.9 MiB |
| final (20 samples, 384x216) | 8 | 4.4 MiB | 30.2 MiB |
| sphere (2000 samples, 2x2) | 8 | 3.1 MiB | 2.8 MiB |
| final (200 samples, 2x2) | 8 | 3.8 MiB | 11.4 MiB |

## Allocations (Valgrind's DHAT, one thread)

| Scene | Program | Blocks | Bytes | Peak of the heap |
| --- | --- | ---: | ---: | ---: |
| sphere (10 samples, 96x54) | Rust | 858411 | 43690505 | 43034 |
| sphere (10 samples, 96x54) | Isocron | 5237 | 641552 | 640992 |
| final (4 samples, 96x54) | Rust | 1685022 | 83098286 | 601216 |
| final (4 samples, 96x54) | Isocron | 90611 | 30514192 | 6258976 |
