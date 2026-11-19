# countwords: Rust and Isocron

Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz, Debian GNU/Linux 13 (trixie), 6.12.115+deb13.1-amd64; rustc 1.99.1 (58cf8f9c3 2026-10-13); Isocron 64f9888. The input: the King James Bible ten times (43 MB).

## Builds

| Build | Wall (median) | Processor (median) |
| --- | ---: | ---: |
| Rust, simple, clean | 0.35 s | 0.72 s |
| Rust, optimized, clean | 0.58 s | 0.80 s |
| Isocron (`iscc build`) | 0.88 s | 0.88 s |

| Program | Size |
| --- | ---: |
| Rust, simple | 4916 KiB |
| Rust, optimized | 4864 KiB |
| Isocron | 165 KiB |

## Runs

| Program | Threads | Wall (median) | Min | Q1 to Q3 | Same output |
| --- | ---: | ---: | ---: | ---: | --- |
| Rust, simple | 1 | 0.567 s | 0.560 s | 0.564 to 0.572 s | yes |
| Rust, optimized | 1 | 0.250 s | 0.237 s | 0.244 to 0.252 s | yes |
| Isocron, one thread | 1 | 0.167 s | 0.166 s | 0.167 to 0.167 s | yes |
| Isocron, 8 threads | 8 | 0.173 s | 0.171 s | 0.171 to 0.173 s | yes |

## Peak resident memory (`VmHWM`, without huge pages)

| Program | Threads | Peak |
| --- | ---: | ---: |
| Rust, simple | 1 | 6.1 MiB |
| Rust, optimized | 1 | 6.1 MiB |
| Isocron, one thread | 1 | 4.6 MiB |
| Isocron, 8 threads | 8 | 4.7 MiB |
