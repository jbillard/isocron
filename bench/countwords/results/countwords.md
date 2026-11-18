# countwords: Rust and Isocron

Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz, Debian GNU/Linux 13 (trixie), 6.12.115+deb13.1-amd64; rustc 1.99.1 (58cf8f9c3 2026-10-13); Isocron e0b480b. The input: the King James Bible ten times (43 MB).

## Builds

| Build | Wall (median) | Processor (median) |
| --- | ---: | ---: |
| Rust, simple, clean | 0.36 s | 0.72 s |
| Rust, optimized, clean | 0.57 s | 0.81 s |
| Isocron (`iscc build`) | 0.66 s | 0.66 s |

| Program | Size |
| --- | ---: |
| Rust, simple | 4916 KiB |
| Rust, optimized | 4864 KiB |
| Isocron | 157 KiB |

## Runs

| Program | Threads | Wall (median) | Min | Q1 to Q3 | Same output |
| --- | ---: | ---: | ---: | ---: | --- |
| Rust, simple | 1 | 0.570 s | 0.560 s | 0.566 to 0.575 s | yes |
| Rust, optimized | 1 | 0.251 s | 0.250 s | 0.250 to 0.253 s | yes |
| Isocron, one thread | 1 | 0.225 s | 0.223 s | 0.224 to 0.228 s | yes |
| Isocron, 8 threads | 8 | 0.232 s | 0.226 s | 0.231 to 0.235 s | yes |

## Peak resident memory (`VmHWM`)

| Program | Threads | Peak |
| --- | ---: | ---: |
| Rust, simple | 1 | 6.1 MiB |
| Rust, optimized | 1 | 6.1 MiB |
| Isocron, one thread | 1 | 15.3 MiB |
| Isocron, 8 threads | 8 | 15.5 MiB |
