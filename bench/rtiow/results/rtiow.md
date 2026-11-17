# rtiow: Rust and Isocron

Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz, Debian GNU/Linux 13 (trixie), 6.12.115+deb13.1-amd64; rustc 1.99.1 (58cf8f9c3 2026-10-13); Isocron 3e66f95.

## Builds

| Build | Wall (median) | Processor (median) |
| --- | ---: | ---: |
| Rust, clean, as shipped (with `images`) | 6.48 s | 21.55 s |
| Rust, clean (without `images`) | 6.30 s | 14.75 s |
| Rust, clean, `target-cpu=native` | 6.34 s | 14.71 s |
| Isocron (`iscc build`) | 2.61 s | 2.61 s |
| Rust, one file touched | 3.04 s | 6.63 s |

| Program | Size |
| --- | ---: |
| Rust, as shipped (with `images`) | 728 KiB |
| Rust | 721 KiB |
| Rust, `target-cpu=native` | 711 KiB |
| Isocron | 267 KiB |

## Renders, 8 threads

| Scene | Program | Wall (median) | Min | Q1 to Q3 | Processor (median) | Same bytes |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| sphere (100 samples, 384x216) | Rust | 2.798 s | 2.769 s | 2.785 to 2.803 s | 22.235 s | yes |
| sphere (100 samples, 384x216) | Rust, `target-cpu=native` | 2.803 s | 2.790 s | 2.791 to 2.806 s | 22.271 s | yes |
| sphere (100 samples, 384x216) | Isocron | 0.867 s | 0.861 s | 0.863 to 0.883 s | 6.752 s | yes |
| final (20 samples, 384x216) | Rust | 2.411 s | 2.380 s | 2.410 to 2.460 s | 19.228 s | yes |
| final (20 samples, 384x216) | Rust, `target-cpu=native` | 2.401 s | 2.384 s | 2.401 to 2.412 s | 19.037 s | yes |
| final (20 samples, 384x216) | Isocron | 1.395 s | 1.377 s | 1.380 to 1.397 s | 10.067 s | yes |

## Renders, 1 thread

| Scene | Program | Wall (median) | Min | Q1 to Q3 | Processor (median) | Same bytes |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| sphere (100 samples, 384x216) | Rust | 7.039 s | 7.014 s | 7.027 to 7.048 s | 7.013 s | yes |
| sphere (100 samples, 384x216) | Rust, `target-cpu=native` | 7.109 s | 7.108 s | 7.109 to 7.117 s | 7.107 s | yes |
| sphere (100 samples, 384x216) | Isocron | 6.646 s | 6.643 s | 6.644 to 6.653 s | 6.644 s | yes |
| final (20 samples, 384x216) | Rust | 10.717 s | 10.709 s | 10.713 to 10.762 s | 10.715 s | yes |
| final (20 samples, 384x216) | Rust, `target-cpu=native` | 10.370 s | 10.369 s | 10.370 to 10.376 s | 10.368 s | yes |
| final (20 samples, 384x216) | Isocron | 9.849 s | 9.842 s | 9.845 to 9.850 s | 9.847 s | yes |

## Peak resident memory (`VmHWM`)

| Scene | Threads | Rust | Isocron |
| --- | ---: | ---: | ---: |
| sphere (100 samples, 384x216) | 1 | 3.1 MiB | 1.3 MiB |
| final (20 samples, 384x216) | 1 | 3.9 MiB | 3.6 MiB |
| sphere (2000 samples, 2x2) | 1 | 2.9 MiB | 0.5 MiB |
| final (200 samples, 2x2) | 1 | 3.6 MiB | 3.2 MiB |
| sphere (100 samples, 384x216) | 8 | 3.6 MiB | 2.3 MiB |
| final (20 samples, 384x216) | 8 | 4.3 MiB | 4.9 MiB |
| sphere (2000 samples, 2x2) | 8 | 3.1 MiB | 0.6 MiB |
| final (200 samples, 2x2) | 8 | 3.8 MiB | 3.5 MiB |

## Allocations (Valgrind's DHAT, one thread)

| Scene | Program | Blocks | Bytes | Peak of the heap |
| --- | --- | ---: | ---: | ---: |
| sphere (10 samples, 96x54) | Rust | 858411 | 43690505 | 43034 |
| sphere (10 samples, 96x54) | Isocron | 5245 | 483664 | 118688 |
| final (4 samples, 96x54) | Rust | 1685022 | 83098286 | 601216 |
| final (4 samples, 96x54) | Isocron | 90369 | 25282704 | 1185440 |
