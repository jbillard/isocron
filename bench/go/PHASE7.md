# Go programs of the kernels of phase 7

The four kernels of SPEC section 14 that the POC did not cover, in Go, with the
conventions of the POC programs (`cmd/sum_*`, `cmd/dot_*`, package `common`).
Each one has the interface of its Isocron program in `examples/phase7/`:

```
moving_average <a.f64> <out.f64> <reps>
filter_sum     <a.f64> <out.f64> <reps>
prefix_sum     <i.f64> <out.f64> <reps>
matvec         <m.f64> <v.f64> <out.f64> <reps>
```

Each program loads its inputs, allocates its output and scratch buffers once,
times `reps` calls of a `//go:noinline` kernel (`common.Time`; a scalar result
goes to a package variable `sink`, an array result is written into the
output slice, then to the file), prints the total in nanoseconds and writes
the result as little-endian `f64` (`common.WriteF64`, canonical NaN).
`prefix_sum` converts the input to `int64` before the timed loop and the sums
back to `f64` after it.

## Files and build

| Path | Content |
| --- | --- |
| `cmd/<kernel>_idiomatic`, `cmd/<kernel>_opt` | sequential variants |
| `cmd/<kernel>_par_idiomatic`, `cmd/<kernel>_par_opt` | goroutines, `GOMAXPROCS` threads |
| `cmd/moving_average_b_idiomatic`, `cmd/moving_average_b_par_idiomatic` | comparison B of section 14: the running sum, O(n) |
| `common/phase7.go` | the shared building blocks of the opt variants, chunking, `int64` I/O |
| `../build_phase7_go.sh` | builds the 16 programs into `bench/bin/` |

`bench/build_phase7_go.sh` uses the options of the Go part of `bench/build.sh`
(GOAMD64=v3 if the CPU has AVX2, FMA and BMI2, else v2; `GOCACHE=bench/build/go-cache`;
`GOFLAGS=-mod=mod`) and produces `go_<kernel>_<variant>` and
`go-par_<kernel>_<variant>`, variants `idiomatic` and `opt`, and
`go_moving_average_b_idiomatic`, `go-par_moving_average_b_idiomatic`.
`bench/build.sh` calls it at its end (it runs every executable
`build_phase7_<lang>.sh`). Standard library only; `common.go` is unchanged.

## Variants

| Kernel | idiomatic | opt | par_idiomatic | par_opt |
| --- | --- | --- | --- | --- |
| moving_average | per window, sum from left to right, `/ 64` | per window, the fixed form as one tree of additions (`common.MovingAverageFixed`) | contiguous ranges of windows per goroutine, idiomatic loop | contiguous ranges of windows, `MovingAverageFixed` |
| filter_sum | `if v > 0 { s += v }` | compaction of the kept elements, 4096 at a time, without branch, then `BlockSum`, `FixedSum` | `common.ChunkSums`, idiomatic loop per chunk | count per chunk, then each block of 4096 kept elements reduced by the goroutine where it starts |
| prefix_sum | `s += v; out[i] = s` | the same, unrolled by 8 without bounds checks (`common.ScanI64`) | each chunk scanned, then the totals of the previous chunks added (reads `out` twice) | two-pass blocked scan: chunk sums (`common.SumI64`), exclusive prefix, then each chunk scanned from its start value |
| matvec | per row, `s += float64(a * v[j])` from left to right | per row, `common.Fixed` with `common.BlockDot` (`common.MatVecFixed`) | contiguous ranges of rows, idiomatic loop | contiguous ranges of rows, `MatVecFixed` |

Chunks: `common.Parts(n)` contiguous chunks (`GOMAXPROCS`, at most `n`), one
goroutine each through `common.ParallelFor`, bounds `n*k/p .. n*(k+1)/p`
(`common.ChunkOf`).

**FMA.** Go may fuse `x*y + z` into an FMA on amd64 (GOAMD64=v3 has it); an
explicit conversion `float64(x*y)` forbids it (Go specification, "Arithmetic
operators"). Every product is written that way, in the idiomatic variants too,
as in `cmd/dot_idiomatic`. Checked with `go tool objdump` on the 16 binaries:
no `VFMADD`, `VFNMADD`, `VFMSUB` or `VFNMSUB` in the functions of `main` and
`common`.

## How each opt variant reaches the fixed form of section 10

V = 32 accumulators, B = 4096 elements per block (`common.Lanes`, `common.Block`).

**moving_average.** A window has 64 elements, one block: lane `j` receives
`w[j]` then `w[j+32]`, so `acc[j] = (0.0 + w[j]) + w[j+32]`, then the tree
`acc[j] = acc[j] + acc[j+d]` for d = 16, 8, 4, 2, 1, then `/ 64.0`.
`MovingAverageFixed` writes these exact additions as one expression, depth
first (few live values, no array of accumulators in memory), with two
changes that keep the bits:

- the 32 additions `0.0 + w[j]` are replaced by one `+ 0.0` on the result.
  In round to nearest, `v + 0.0` is `v`, except that `-0.0` becomes `+0.0`.
  Let `z(v) = v + 0.0`. For all `a`, `b`: `z(a) + z(b) = z(a + b)` (a sum is
  `-0.0` only when both operands are `-0.0`; nonzero values and NaN are not
  changed by `z`), and `z(a) + b = z(a) + z(b)`. By induction over the tree,
  the fixed form equals `z(tree without the 0.0 additions)`.
  Checked on an input made of signed zeros (`ma_zeros` below), where the tree
  without them gives `-0.0` in 174 windows: the result has Isocron's `+0.0`.
- `/ 64.0` is compiled as `* 0x1p-6`; dividing by a power of two and
  multiplying by its exact inverse round the same real number, so the results
  are identical.

The assembly has 64 `ADDSD` per window (63 for the tree, one `+ 0.0` with
`XORPS X0, X0`: Go does not remove it), one `MULSD`, no FMA. Each window is
reduced alone, so `par_opt` gives the same result for any number of threads.

**filter_sum.** The fixed form of the array of kept elements (those
`> 0.0`; NaN and `-0.0` are not kept), as if it had been built: kept element
`t` goes to block `t / 4096`, lane `t mod 32`.
`common.CompactPositive` copies the elements into a buffer of 4096,
advancing the count only for the kept ones (`UCOMISD`, `SETA`, no branch on
the data), until 4096 are kept or the input ends; `common.BlockSum` of the
buffer is exactly `reduce_block` of that block. The block results are
reduced by `common.FixedSum` (recursively if there are more than 4096 of
them); 0 kept elements give `0.0` (the tree of 32 zeros), at most 4096 give
the single block result, as the fixed form does.

`par_opt`: (1) each goroutine counts the kept elements of its chunk
(`CountPositive`); (2) an exclusive prefix of the counts gives each chunk the
index, in the kept sequence, of its first kept element; (3) block `b` (kept
elements `4096*b ..`) is reduced by the goroutine whose chunk holds its first
element: it skips the kept elements that belong to the previous block
(`SkipPositive`), then compacts and reduces its blocks, the last one reading
into the following chunks if needed, into `parts[b]`; (4) `FixedSum(parts)`
in order. Every block is reduced from the same elements in the same order
whatever the chunks are, so the result is the sequential one for any number
of threads. The input is read twice (the counts, then the blocks).

**prefix_sum.** Integer addition (wrapping, as Go's `int64`) is
associative: any order gives the result of the sequential scan, which is
Isocron's. The `opt` variants only save work: unrolling, no bounds checks,
and in parallel the reduce-then-scan form (read `x` twice, write `out` once)
instead of the scan-then-add form of `par_idiomatic` (read and write `out`
twice).

**matvec.** Row `i` is `dot(m[i*cols .. i*cols+cols], v)`, the fixed form of
`row[k] * v[k]` for `k` in increasing order: `common.Fixed(cols, ...)` over
`common.BlockDot` (one block when `cols <= 4096`, the case of the data;
more blocks and the second level otherwise, checked with `cols = 5000`).
Each product is rounded, then added (`float64(a * b)`). Rows are
independent: `par_opt` gives the same result for any number of threads.

## Verification

Each program was run once (`reps = 1`) on `bench/data/small` and
`bench/data/large` and its output compared with Isocron's,
`bench/data/ref/<size>/<kernel>.out`: `cmp` for byte equality, otherwise the
largest relative difference `|go - isc| / |isc|` over the elements (and the
largest absolute difference). The parallel programs ran with `GOMAXPROCS=1`
and `GOMAXPROCS=4`.

| Kernel | Variant | small | large |
| --- | --- | --- | --- |
| moving_average | opt | identical | identical |
| moving_average | par_opt, P = 1 and 4 | identical | identical |
| moving_average | idiomatic | 1.42e-12 (26 744 of 32 705 values differ; abs. 7.1e-14) | 8.51e-09 (8 155 174 of 9 999 937; abs. 9.9e-14) |
| moving_average | par_idiomatic, P = 1 and 4 | same as idiomatic | same as idiomatic |
| filter_sum | opt | identical | identical |
| filter_sum | par_opt, P = 1 and 4 | identical | identical |
| filter_sum | idiomatic | identical | 1.49e-14 (abs. 5.9e-06) |
| filter_sum | par_idiomatic, P = 1 | identical | 1.49e-14 (abs. 5.9e-06) |
| filter_sum | par_idiomatic, P = 4 | 7.26e-16 (abs. 9.3e-10) | 1.59e-14 (abs. 6.3e-06) |
| prefix_sum | opt, idiomatic | identical | identical |
| prefix_sum | par_opt, par_idiomatic, P = 1 and 4 | identical | identical |
| matvec | opt | identical | identical |
| matvec | par_opt, P = 1 and 4 | identical | identical |
| matvec | idiomatic | 2.52e-14 (114 of 128 rows differ; abs. 1.2e-14) | 1.51e-12 (3 977 of 4 096; abs. 4.0e-13) |
| matvec | par_idiomatic, P = 1 and 4 | same as idiomatic | same as idiomatic |

The relative difference of the idiomatic moving average is large on a few
windows whose sum nearly cancels (the inputs lie in [-1000, 1000]):
the absolute difference stays below 1e-13. The idiomatic filter_sum on the
small input happens to round to the same value.

`par_opt` was also checked, byte for byte, with `GOMAXPROCS` = 2, 3, 7 and
16 on the small inputs, and the `opt` and `par_opt` programs (P = 1, 2, 3, 4,
7, 16) against `bench/bin/isc_<kernel>` on synthetic inputs, all identical:

- filter_sum: no kept element (negatives, `0.0`, `-0.0`, NaN), one kept
  element, exactly 4095, 4096, 4097, 8192 and 8193 kept, 1000 kept among
  10^6 (one block over every chunk), a first half without kept element,
  infinities and subnormals, and 16 779 216 kept elements (4 097 blocks: the
  second level of the fixed form);
- moving_average: signed zeros with windows summing to zero, 66 elements
  (3 windows, fewer than most of the thread counts), infinities and NaN;
- matvec: 7 rows of 5 000 columns (two blocks per row), 5 rows of 33;
- prefix_sum: 100 001 integers of both signs, 3 elements.

## Comparison B: the moving average by a running sum

Section 14, comparison B: each language uses the algorithm an experienced
developer writes, and the float results may differ. Isocron's program is
`examples/phase7/moving_average_b.isc` (prefix sums, then one difference per
window). The Go one is the running sum, O(n), what a Go developer writes:

```go
s := sum of x[0:64], left to right;  out[0] = s / 64
for each next window i:  s += x[i+64] - x[i];  out[i+1] = s / 64
```

`moving_average_b_par_idiomatic` gives each goroutine a contiguous range of
windows and restarts the sum at the start of its range: the results depend
on the number of goroutines (the error does not accumulate across ranges).

Differences against the exact sums of the windows (Isocron's
`bench/data/ref/<size>/moving_average.out`), `reps = 1`:

| Program | small: largest abs. | small: largest rel. | large: largest abs. | large: largest rel. |
| --- | --- | --- | --- | --- |
| moving_average_b_idiomatic | 2.45e-13 | 1.11e-11 | 8.89e-12 | 5.85e-06 |
| moving_average_b_par_idiomatic, P = 1 | 2.45e-13 | 1.11e-11 | 8.89e-12 | 5.85e-06 |
| moving_average_b_par_idiomatic, P = 4 | 2.74e-13 | 1.33e-11 | 5.73e-12 | 5.85e-06 |

Almost every value differs (31 970 of 32 705 on the small input, 9 999 871 of
9 999 937 on the large one). The absolute error grows along the array (it
is about 36 times larger on the large input than on the small one, against
1.4 times for the left-to-right sums of `moving_average_idiomatic`). The
largest relative difference is at the window whose mean is closest to zero
(window 1 099 105 on the large input, the same window as for
`moving_average_idiomatic`, mean 4.7e-7, difference 2.7e-12), which lies in the first
range when P = 4, hence the same value.

## Notes for the measurements

- **A faster exact moving average exists, not used here.** The levels of the
  tree are shared by neighbouring windows: the pair `j` of window `i`,
  `x[i+j] + x[i+j+32]`, is the pair `j-1` of window `i+1`. Six sliding passes
  (`P[k] = x[k] + x[k+32]`, then `P[k] + P[k+16]`, `+8`, `+4`, `+2`, `+1`,
  then `+ 0.0` and `/ 64`), tiled to stay in the L1 cache, give Isocron's
  bits with 6 additions per window instead of 63. It is a different
  algorithm (comparison B of section 14), so `opt` keeps the per-window
  tree; a scalar Go prototype of it matched Isocron byte for byte on both
  sizes. The same sharing is open to Isocron's compiler for
  `map(window(x, 64), sum)`.
- filter_sum `par_opt` reads its input twice (counts, then blocks); on one
  thread it is slower than `opt`.
- No measurement campaign was run; the programs were only run with
  `reps = 1` or `2` for the checks above.
