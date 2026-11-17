# countwords: Rust and Isocron

Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz, Debian GNU/Linux 13 (trixie), 6.12.115+deb13.1-amd64; rustc 1.99.1 (58cf8f9c3 2026-10-13); Isocron 5ad5776. The input: the King James Bible ten times (43 MB).

## Builds

| Build | Wall (median) | Processor (median) |
| --- | ---: | ---: |
| Rust, simple, clean | 0.35 s | 0.73 s |
| Rust, optimized, clean | 0.57 s | 0.81 s |
| Isocron (`iscc build`) | 0.45 s | 0.45 s |

| Program | Size |
| --- | ---: |
| Rust, simple | 4916 KiB |
| Rust, optimized | 4864 KiB |
| Isocron | 213 KiB |

## Runs

| Program | Threads | Wall (median) | Min | Q1 to Q3 | Same output |
| --- | ---: | ---: | ---: | ---: | --- |
| Rust, simple | 1 | 0.568 s | 0.561 s | 0.564 to 0.572 s | yes |
| Rust, optimized | 1 | 0.251 s | 0.250 s | 0.250 to 0.253 s | yes |
| Isocron, one thread | 1 | 0.307 s | 0.305 s | 0.306 to 0.308 s | yes |
| Isocron, 8 threads | 8 | 0.263 s | 0.258 s | 0.260 to 0.265 s | yes |

## Peak resident memory (`VmHWM`)

| Program | Threads | Peak |
| --- | ---: | ---: |
| Rust, simple | 1 | 6.0 MiB |
| Rust, optimized | 1 | 6.1 MiB |
| Isocron, one thread | 1 | 434.2 MiB |
| Isocron, 8 threads | 8 | 434.4 MiB |
