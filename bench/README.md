# Benchmarks (SPEC section 14, tasks P5 and P6)

Four kernels — sum, dot product, histogram, mean then standard deviation — in
Isocron (`examples/poc`), C, Rust, Go and Zig. Every program has the same
interface as the Isocron POC programs:

```
<kernel> <inputs...> <output.f64> <reps>
```

It loads its inputs, times `reps` calls of the kernel only, prints the total
in nanoseconds and writes the result as little-endian `f64`.

| Directory | Variants |
| --- | --- |
| `c/` | `VARIANT=0` idiomatic, `VARIANT=1` hand-optimized; built with GCC and with Clang (`zig cc`), strict and reassociated |
| `rust/` | `src/bin/<kernel>_idiomatic.rs`, `<kernel>_opt.rs` |
| `go/` | `cmd/<kernel>_idiomatic`, `cmd/<kernel>_opt` |
| `zig/` | `<kernel>_idiomatic.zig`, `<kernel>_opt.zig`, `<kernel>_reassoc.zig` |

Parallel variants (`<kernel>_par_idiomatic`, `<kernel>_par_opt`; C: the same
files built with `-fopenmp`): OpenMP, rayon, goroutines, `std.Thread.Pool`.
The number of threads comes from `OMP_NUM_THREADS`, `RAYON_NUM_THREADS`,
`GOMAXPROCS` and `ISC_THREADS` (also used by Isocron).

The hand-optimized float variants implement the fixed reduction form of
section 10 and are bit-identical to Isocron; the idiomatic ones add from left
to right and differ by a few units in the last place (documented in
`results/poc.md`).

```sh
bench/campaign.sh NAME     # everything below at fixed frequency, then restore -> results/NAME*.md
bench/build.sh             # -> bench/bin/
bench/machine.sh apply [single|parallel]   # fixed frequency, isolated cores (needs sudo without password)
bench/machine.sh run       # bench/run.py on the isolated cores -> results/poc.md, results/poc.json
bench/machine.sh restore   # back to the previous settings
bench/run.py               # same measurements without the machine settings
bench/run.py --name xeon-single   # results/xeon-single.md (default name: poc)
bench/parallel.py          # 1, 2, 4, 8 threads -> results/xeon.md
bench/build_lanes.sh       # V = 16, 32, 64 x SSE4.2 / AVX2 / AVX-512 -> bench/bin-lanes/
bench/lanes.py             # the study of V -> results/xeon-lanes.md
bench/compile.py           # compilation times, cold and warm caches -> results/xeon-compile.md
bench/run.py --quick --sizes small --kernels sum    # quick check of the scripts
bench/run.py --report-only                          # rewrite poc.md from poc.json
```
