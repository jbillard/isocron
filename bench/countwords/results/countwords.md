# countwords: Rust and Isocron

Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz, Debian GNU/Linux 13 (trixie), 6.12.115+deb13.1-amd64; rustc 1.99.1 (58cf8f9c3 2026-10-13); Isocron 27129dd. The input: the King James Bible ten times (43 MB).

## Builds

| Build | Wall (median) | Processor (median) |
| --- | ---: | ---: |
| Rust, simple, clean | 0.35 s | 0.72 s |
| Rust, optimized, clean | 0.57 s | 0.81 s |
| Isocron (`iscc build`) | 0.58 s | 0.57 s |

| Program | Size |
| --- | ---: |
| Rust, simple | 4916 KiB |
| Rust, optimized | 4864 KiB |
| Isocron | 150 KiB |

## Runs

| Program | Threads | Wall (median) | Min | Q1 to Q3 | Same output |
| --- | ---: | ---: | ---: | ---: | --- |
| Rust, simple | 1 | 0.571 s | 0.560 s | 0.566 to 0.575 s | yes |
| Rust, optimized | 1 | 0.253 s | 0.239 s | 0.253 to 0.255 s | yes |
| Isocron, one thread | 1 | 0.180 s | 0.177 s | 0.177 to 0.180 s | yes |
| Isocron, 8 threads | 8 | 0.184 s | 0.180 s | 0.183 to 0.187 s | yes |

## Peak resident memory (`VmHWM`)

| Program | Threads | Peak |
| --- | ---: | ---: |
| Rust, simple | 1 | 6.1 MiB |
| Rust, optimized | 1 | 6.1 MiB |
| Isocron, one thread | 1 | 17.1 MiB |
| Isocron, 8 threads | 8 | 17.2 MiB |
