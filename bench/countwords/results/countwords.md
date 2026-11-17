# countwords: Rust and Isocron

Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz, Debian GNU/Linux 13 (trixie), 6.12.115+deb13.1-amd64; rustc 1.99.1 (58cf8f9c3 2026-10-13); Isocron b8e39b7. The input: the King James Bible ten times (43 MB).

## Builds

| Build | Wall (median) | Processor (median) |
| --- | ---: | ---: |
| Rust, simple, clean | 0.36 s | 0.73 s |
| Rust, optimized, clean | 0.57 s | 0.81 s |
| Isocron (`iscc build`) | 0.34 s | 0.34 s |

| Program | Size |
| --- | ---: |
| Rust, simple | 4916 KiB |
| Rust, optimized | 4864 KiB |
| Isocron | 1740 KiB |

## Runs

| Program | Threads | Wall (median) | Min | Q1 to Q3 | Same output |
| --- | ---: | ---: | ---: | ---: | --- |
| Rust, simple | 1 | 0.570 s | 0.565 s | 0.568 to 0.576 s | yes |
| Rust, optimized | 1 | 0.252 s | 0.251 s | 0.252 to 0.257 s | yes |
| Isocron, one thread | 1 | 1.399 s | 1.394 s | 1.396 to 1.406 s | yes |
| Isocron, 8 threads | 8 | 0.848 s | 0.823 s | 0.839 to 0.859 s | yes |

## Peak resident memory (`VmHWM`)

| Program | Threads | Peak |
| --- | ---: | ---: |
| Rust, simple | 1 | 6.0 MiB |
| Rust, optimized | 1 | 6.1 MiB |
| Isocron, one thread | 1 | 1339.0 MiB |
| Isocron, 8 threads | 8 | 1433.4 MiB |
