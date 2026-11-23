# countwords: Rust and Isocron

Intel(R) Xeon(R) E-2388G CPU @ 3.20GHz, Debian GNU/Linux 13 (trixie), 6.12.115+deb13.1-amd64; rustc 1.99.1 (58cf8f9c3 2026-10-13); Isocron 55f7623. The input: the King James Bible ten times (43 MB).

## Builds

| Build | Wall (median) | Processor (median) |
| --- | ---: | ---: |
| Rust, simple, clean | 0.35 s | 0.72 s |
| Rust, optimized, clean | 0.57 s | 0.81 s |
| Isocron (`iscc build`) | 0.91 s | 0.89 s |

| Program | Size |
| --- | ---: |
| Rust, simple | 4916 KiB |
| Rust, optimized | 4864 KiB |
| Isocron | 165 KiB |

## Runs

| Program | Threads | Wall (median) | Min | Q1 to Q3 | Same output |
| --- | ---: | ---: | ---: | ---: | --- |
| Rust, simple | 1 | 0.574 s | 0.566 s | 0.568 to 0.581 s | yes |
| Rust, optimized | 1 | 0.253 s | 0.243 s | 0.251 to 0.255 s | yes |
| Isocron, one thread | 1 | 0.169 s | 0.168 s | 0.168 to 0.170 s | yes |
| Isocron, 8 threads | 8 | 0.172 s | 0.171 s | 0.171 to 0.174 s | yes |

## Peak resident memory (`VmHWM`, without huge pages)

| Program | Threads | Peak |
| --- | ---: | ---: |
| Rust, simple | 1 | 6.0 MiB |
| Rust, optimized | 1 | 6.1 MiB |
| Isocron, one thread | 1 | 4.6 MiB |
| Isocron, 8 threads | 8 | 4.7 MiB |
