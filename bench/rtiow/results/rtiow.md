# rtiow: Rust and Isocron

Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz, Debian GNU/Linux 13 (trixie), 6.12.115+deb13.1-amd64; rustc 1.99.1 (58cf8f9c3 2026-10-13); Isocron 55f7623.

## Builds

| Build | Wall (median) | Processor (median) |
| --- | ---: | ---: |
| Rust, clean, as shipped (with `images`) | 6.63 s | 21.77 s |
| Rust, clean (without `images`) | 6.26 s | 14.76 s |
| Rust, clean, `target-cpu=native` | 6.36 s | 14.83 s |
| Isocron (`iscc build`) | 2.66 s | 2.66 s |
| Rust, one file touched | 3.03 s | 6.64 s |

| Program | Size |
| --- | ---: |
| Rust, as shipped (with `images`) | 728 KiB |
| Rust | 721 KiB |
| Rust, `target-cpu=native` | 711 KiB |
| Isocron | 272 KiB |

## Renders, 8 threads

| Scene | Program | Wall (median) | Min | Q1 to Q3 | Processor (median) | Same bytes |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| sphere (100 samples, 384x216) | Rust | 2.789 s | 2.783 s | 2.788 to 2.794 s | 22.239 s | yes |
| sphere (100 samples, 384x216) | Rust, `target-cpu=native` | 2.801 s | 2.774 s | 2.783 to 2.803 s | 22.136 s | yes |
| sphere (100 samples, 384x216) | Isocron | 0.860 s | 0.843 s | 0.848 to 0.871 s | 6.668 s | yes |
| final (20 samples, 384x216) | Rust | 2.411 s | 2.361 s | 2.367 to 2.421 s | 19.212 s | yes |
| final (20 samples, 384x216) | Rust, `target-cpu=native` | 2.384 s | 2.335 s | 2.383 to 2.424 s | 19.002 s | yes |
| final (20 samples, 384x216) | Isocron | 1.368 s | 1.361 s | 1.362 to 1.417 s | 10.013 s | yes |

## Renders, 1 thread

| Scene | Program | Wall (median) | Min | Q1 to Q3 | Processor (median) | Same bytes |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| sphere (100 samples, 384x216) | Rust | 7.019 s | 7.001 s | 7.010 to 7.046 s | 7.017 s | yes |
| sphere (100 samples, 384x216) | Rust, `target-cpu=native` | 7.104 s | 7.104 s | 7.104 to 7.125 s | 7.102 s | yes |
| sphere (100 samples, 384x216) | Isocron | 6.970 s | 6.968 s | 6.969 to 6.979 s | 6.969 s | yes |
| final (20 samples, 384x216) | Rust | 10.709 s | 10.686 s | 10.698 to 10.712 s | 10.707 s | yes |
| final (20 samples, 384x216) | Rust, `target-cpu=native` | 10.363 s | 10.345 s | 10.354 to 10.365 s | 10.359 s | yes |
| final (20 samples, 384x216) | Isocron | 9.762 s | 9.758 s | 9.760 to 9.764 s | 9.760 s | yes |

## Peak resident memory (`VmHWM`, without huge pages)

| Scene | Threads | Rust | Isocron |
| --- | ---: | ---: | ---: |
| sphere (100 samples, 384x216) | 1 | 3.2 MiB | 0.9 MiB |
| final (20 samples, 384x216) | 1 | 3.8 MiB | 2.1 MiB |
| sphere (2000 samples, 2x2) | 1 | 3.0 MiB | 0.4 MiB |
| final (200 samples, 2x2) | 1 | 3.7 MiB | 1.7 MiB |
| sphere (100 samples, 384x216) | 8 | 3.6 MiB | 1.9 MiB |
| final (20 samples, 384x216) | 8 | 4.4 MiB | 3.4 MiB |
| sphere (2000 samples, 2x2) | 8 | 3.1 MiB | 0.6 MiB |
| final (200 samples, 2x2) | 8 | 3.8 MiB | 2.0 MiB |

## Allocations (Valgrind's DHAT, one thread)

| Scene | Program | Blocks | Bytes | Peak of the heap |
| --- | --- | ---: | ---: | ---: |
| sphere (10 samples, 96x54) | Rust | 858411 | 43690505 | 43034 |
| sphere (10 samples, 96x54) | Isocron | 5245 | 483664 | 114720 |
| final (4 samples, 96x54) | Rust | 1685022 | 83098286 | 601216 |
| final (4 samples, 96x54) | Isocron | 90369 | 25282704 | 1095520 |
