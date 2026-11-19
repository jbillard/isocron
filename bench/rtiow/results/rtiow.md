# rtiow: Rust and Isocron

Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz, Debian GNU/Linux 13 (trixie), 6.12.115+deb13.1-amd64; rustc 1.99.1 (58cf8f9c3 2026-10-13); Isocron 64f9888.

## Builds

| Build | Wall (median) | Processor (median) |
| --- | ---: | ---: |
| Rust, clean, as shipped (with `images`) | 6.45 s | 21.35 s |
| Rust, clean (without `images`) | 6.20 s | 14.59 s |
| Rust, clean, `target-cpu=native` | 6.21 s | 14.54 s |
| Isocron (`iscc build`) | 2.68 s | 2.66 s |
| Rust, one file touched | 3.00 s | 6.59 s |

| Program | Size |
| --- | ---: |
| Rust, as shipped (with `images`) | 728 KiB |
| Rust | 721 KiB |
| Rust, `target-cpu=native` | 711 KiB |
| Isocron | 271 KiB |

## Renders, 8 threads

| Scene | Program | Wall (median) | Min | Q1 to Q3 | Processor (median) | Same bytes |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| sphere (100 samples, 384x216) | Rust | 2.782 s | 2.778 s | 2.779 to 2.787 s | 22.206 s | yes |
| sphere (100 samples, 384x216) | Rust, `target-cpu=native` | 2.782 s | 2.780 s | 2.780 to 2.815 s | 22.192 s | yes |
| sphere (100 samples, 384x216) | Isocron | 0.860 s | 0.844 s | 0.851 to 0.865 s | 6.735 s | yes |
| final (20 samples, 384x216) | Rust | 2.392 s | 2.380 s | 2.390 to 2.442 s | 18.982 s | yes |
| final (20 samples, 384x216) | Rust, `target-cpu=native` | 2.397 s | 2.360 s | 2.362 to 2.405 s | 19.114 s | yes |
| final (20 samples, 384x216) | Isocron | 1.358 s | 1.356 s | 1.358 to 1.364 s | 9.902 s | yes |

## Renders, 1 thread

| Scene | Program | Wall (median) | Min | Q1 to Q3 | Processor (median) | Same bytes |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| sphere (100 samples, 384x216) | Rust | 7.040 s | 7.005 s | 7.023 to 7.045 s | 7.020 s | yes |
| sphere (100 samples, 384x216) | Rust, `target-cpu=native` | 7.102 s | 7.096 s | 7.099 to 7.105 s | 7.094 s | yes |
| sphere (100 samples, 384x216) | Isocron | 6.984 s | 6.964 s | 6.974 to 6.989 s | 6.983 s | yes |
| final (20 samples, 384x216) | Rust | 10.733 s | 10.712 s | 10.722 to 10.741 s | 10.730 s | yes |
| final (20 samples, 384x216) | Rust, `target-cpu=native` | 10.355 s | 10.341 s | 10.348 to 10.367 s | 10.352 s | yes |
| final (20 samples, 384x216) | Isocron | 9.751 s | 9.750 s | 9.750 to 9.757 s | 9.748 s | yes |

## Peak resident memory (`VmHWM`, without huge pages)

| Scene | Threads | Rust | Isocron |
| --- | ---: | ---: | ---: |
| sphere (100 samples, 384x216) | 1 | 3.2 MiB | 1.0 MiB |
| final (20 samples, 384x216) | 1 | 3.9 MiB | 2.3 MiB |
| sphere (2000 samples, 2x2) | 1 | 3.0 MiB | 0.4 MiB |
| final (200 samples, 2x2) | 1 | 3.6 MiB | 1.9 MiB |
| sphere (100 samples, 384x216) | 8 | 3.6 MiB | 1.9 MiB |
| final (20 samples, 384x216) | 8 | 4.3 MiB | 3.6 MiB |
| sphere (2000 samples, 2x2) | 8 | 3.0 MiB | 0.6 MiB |
| final (200 samples, 2x2) | 8 | 3.8 MiB | 2.3 MiB |

## Allocations (Valgrind's DHAT, one thread)

| Scene | Program | Blocks | Bytes | Peak of the heap |
| --- | --- | ---: | ---: | ---: |
| sphere (10 samples, 96x54) | Rust | 858411 | 43690505 | 43034 |
| sphere (10 samples, 96x54) | Isocron | 5245 | 483664 | 114720 |
| final (4 samples, 96x54) | Rust | 1685022 | 83098286 | 601216 |
| final (4 samples, 96x54) | Isocron | 90369 | 25282704 | 1095520 |
