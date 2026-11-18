# countwords: Rust and Isocron

Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz, Debian GNU/Linux 13 (trixie), 6.12.115+deb13.1-amd64; rustc 1.99.1 (58cf8f9c3 2026-10-13); Isocron ea9cca1. The input: the King James Bible ten times (43 MB).

## Builds

| Build | Wall (median) | Processor (median) |
| --- | ---: | ---: |
| Rust, simple, clean | 0.36 s | 0.73 s |
| Rust, optimized, clean | 0.57 s | 0.81 s |
| Isocron (`iscc build`) | 0.60 s | 0.60 s |

| Program | Size |
| --- | ---: |
| Rust, simple | 4916 KiB |
| Rust, optimized | 4864 KiB |
| Isocron | 152 KiB |

## Runs

| Program | Threads | Wall (median) | Min | Q1 to Q3 | Same output |
| --- | ---: | ---: | ---: | ---: | --- |
| Rust, simple | 1 | 0.572 s | 0.563 s | 0.566 to 0.574 s | yes |
| Rust, optimized | 1 | 0.253 s | 0.242 s | 0.252 to 0.255 s | yes |
| Isocron, one thread | 1 | 0.243 s | 0.239 s | 0.241 to 0.246 s | yes |
| Isocron, 8 threads | 8 | 0.206 s | 0.202 s | 0.204 to 0.209 s | yes |

## Peak resident memory (`VmHWM`)

| Program | Threads | Peak |
| --- | ---: | ---: |
| Rust, simple | 1 | 6.0 MiB |
| Rust, optimized | 1 | 6.0 MiB |
| Isocron, one thread | 1 | 307.6 MiB |
| Isocron, 8 threads | 8 | 307.8 MiB |
