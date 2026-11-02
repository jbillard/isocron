# Rust: the kernels of phase 7 (SPEC section 14)

Four kernels with the interface of `examples/phase7/*.isc`
(`<kernel> <inputs...> <output.f64> <reps>`): each program loads its inputs,
allocates its output buffer once, times `reps` calls of the kernel only
(inputs and outputs passed through `std::hint::black_box`, as in the POC
programs), prints the total in nanoseconds and writes the result as
little-endian `f64`.

```sh
bench/build_phase7_rust.sh   # -> bench/bin/rust_<kernel>_<variant>, rust-par_<kernel>_<variant>,
                             #    rust[-par]_moving_average_b_idiomatic (comparison B, below)
```

`build.sh` runs it at its end (the loop over `build_phase7_<lang>.sh`). The
shared code is `src/phase7.rs`; the programs are `src/bin/<kernel>_<variant>.rs`
and `src/bin/<kernel>_par_<variant>.rs` (auto-discovered bins, like the POC).

## Variants

| Kernel | `idiomatic` | `opt` | `par_idiomatic` (rayon) | `par_opt` (rayon) |
| --- | --- | --- | --- | --- |
| `moving_average` | `x.windows(64)`, each `.sum()` / 64 | fixed form, levels shared between windows, tiles of 1024 outputs | `par_windows(64)` zipped with `par_iter_mut` | `opt` on chunks of whole tiles, about 4 per thread |
| `filter_sum` | `.filter(v > 0).sum()` | AVX-512 compaction into a buffer of 1024, rows of 32 added to the 32 accumulators, tree per block of 4096 | `par_iter().filter().sum()` | count per chunk, then each chunk reduces the blocks that start in it |
| `prefix_sum` | loop, `wrapping_add` | AVX-512 in-register scan of rows of 8 plus a carry vector | two passes over one chunk per thread (sums, then scan from the offset) | same structure, vectorized chunk sums and the AVX-512 scan |
| `matvec` | per row `zip().map(p * q).sum()` | per row the fixed form (`block_map2`, as `dot_opt`) | `par_iter_mut` over rows | `par_iter_mut` over rows, fixed form per row |

`prefix_sum` converts the input to `i64` before the timed loop, and the sums
to `f64` after it. The idiomatic programs add from left to right. No variant
uses FMA: Rust never contracts `a * b + c`, and no program calls `mul_add`.

**Output buffers.** Allocated once before the timed loop by
`phase7::output_buffer`, which writes every element. `vec![0.0; n]` gets zero
pages from the system, mapped at the first write, which would put up to
80 MB of page faults (large `moving_average` and `prefix_sum`) into the first
timed call. Isocron allocates its output inside each call, so the cost of
reusing a buffer is not counted for Rust.

**AVX-512.** `filter_sum` (`vcompresspd`) and `prefix_sum` (`valignq`) use
`std::arch` intrinsics under `#[cfg(target_feature = "avx512f")]`. That is the
case with `-C target-cpu=native` on the test machine (Xeon E-2388G). Without
AVX-512 a portable path is compiled (branchless scalar compaction, scalar
scan). Both give the same results (checked below).

## How the `opt` variants reach the fixed form

The fixed form (section 10, `lib.rs`): V = 32 accumulators, blocks of
B = 4096, `acc[i mod V] += x[i]` from `0.0`, then the tree
`acc[j] += acc[j + d]` for d = 16, 8, 4, 2, 1; more than B elements: the
block results reduced the same way.

**moving_average.** For window `w = x[i..i+64]` the fixed form is
`acc[j] = (0.0 + w[j]) + w[j+32]`, then the tree, then `acc[0] / 64`. Every
intermediate value depends on `i + j` only:

```
L0[k] = (0.0 + x[k]) + x[k + 32]      acc[j] of window i   = L0[i + j]
L1[k] = L0[k] + L0[k + 16]            after d = 16         = L1[i + j]
L2[k] = L1[k] + L1[k + 8]             ...
L3[k] = L2[k] + L2[k + 4]
L4[k] = L3[k] + L3[k + 2]
out[i] = (L4[i] + L4[i + 1]) / 64.0   after d = 1, then the division
```

So each level is computed once for all the windows of a tile (1024 outputs,
two ping-pong buffers that stay in L1), as plain element-wise loops that LLVM
vectorizes. Every window still gets exactly the per-window operations, in the
same order, on the same operands; that includes `0.0 + w[j]`, which turns
`-0.0` into `+0.0`. The cost is 7 additions per output instead of 64 + 31.
Isocron's compiler uses the same shared-level cascade for sliding-window
reductions (DECISIONS D115), so `opt` against Isocron compares the same
algorithm (comparison A). The code is generic in `ISC_LANES` (64/V terms in
`L0`, log2(V) levels); for a V that does not divide 64 it falls back to the
fixed form window by window.

**filter_sum.** `sum(filter(x, p))` is the fixed form of the compacted array of
the kept elements. `phase7::kept_blocks` streams `x` by rows of 8:
`vcmppd` (`> 0.0`, ordered: NaN is not kept), `vcompresspd`, an unaligned
store at the fill level `k` of a small buffer, then `k += popcount`. When the
buffer holds 1024 = 32·V kept elements, they are added to the 32 accumulators
by rows of 32. The buffer size is a multiple of V that divides B, so kept
element k of a block lands in `acc[k mod 32]`, in order. After 4096 kept
elements, the tree gives the block result. The last block is partial. One block:
its result; several: `fixed_sum` over the block results (recursive above
4096 blocks), as `fixed` does. Why a buffer of 1024 rather than a whole block:
compacting into a 32 KiB block buffer and then calling `block_map` cost
6.5 µs per call on the small input (compaction alone 6.4 µs). The 32 KiB
buffer swept through L1 next to the input stream. With buffers of 512 to
2048 it cost 4.7 µs; Isocron (a 40-element buffer) cost 5.7 µs. These are quick
checks of a standalone harness, results identical. The parallel version
counts the kept elements of each chunk (pass 1, vectorized). The exclusive
cumulation gives each chunk the position `s` of its first kept element in the
kept sequence. The blocks that *start* in a chunk are its own:
`ceil(s / B)` up to `ceil(e / B)`, with their slots in the array of block
results. Pass 2: each chunk skips the `ceil(s / B)·B − s` kept elements that
belong to the block started before it, then reduces its blocks, reading past
its end to complete the last one. The block results, hence the result, do not
depend on the chunks or on the number of threads. The cost is two passes over
`x`.

**prefix_sum.** Integers, wrapping: any order gives the same result. One
row of 8 `i64` is scanned in its register with three shifts (`valignq` with
zero, by 1, 2, 4 lanes) and additions. The carry vector is then added, and
the carry advances by the broadcast of lane 7 (`vpermq`); its chain is one
vector addition per row. Parallel: a two-pass blocked scan. Pass 1 sums every
chunk but the last, then comes the exclusive cumulation of the sums, then
pass 2 scans each chunk from its offset.

**matvec.** cols ≤ 4096 here: one block per row, `block_map2(row, v, p * q)`,
each product rounded then added, accumulator k mod 32. For cols > 4096,
`row_dot` uses `fixed`, so it is still the fixed form (checked with cols = 5000,
8193, 9000).

## Comparison B: the moving average in O(n)

Comparison B of section 14: the best algorithm per language, results allowed
to differ. Isocron's program is `examples/phase7/moving_average_b.isc` (prefix
sums with `scan`, then one difference per window). The Rust programs are the
running sum an experienced Rust developer writes. The sum of the first window
is `.sum()`, then `s += x[i + 64] - x[i]` and `out[i] = s / 64.0`.

| Program | Source | Built as |
| --- | --- | --- |
| sequential | `src/bin/moving_average_b_idiomatic.rs` | `rust_moving_average_b_idiomatic` |
| rayon | `src/bin/moving_average_b_par_idiomatic.rs` | `rust-par_moving_average_b_idiomatic` |

The parallel program splits the output into chunks of 4096. Each chunk
restarts the running sum at its start, from the sum of its first window.
The chunks have a fixed size, so the result does not depend on the number of
threads (T = 1, 4 and 16 give identical outputs). Restarting costs 64
additions per 4096 outputs, and it limits how far the rounding errors of `s`
can drift. The sequential program never restarts.

Differences from the exact sums of the windows
(`bench/data/ref/<size>/moving_average.out`, Isocron's fixed form), with
reps = 1. Isocron's own comparison B program is shown for reference: built
from `moving_average_b.isc`, not part of this directory.

| Program | Size | Max abs diff | Max rel diff | Values that differ |
| --- | --- | --- | --- | --- |
| `rust_moving_average_b_idiomatic` | small | 2.45e-13 | 1.10e-11 | 31 970 / 32 705 |
| `rust-par_moving_average_b_idiomatic` (any T) | small | 1.74e-13 | 1.17e-11 | 31 411 / 32 705 |
| Isocron `moving_average_b` | small | 3.41e-13 | 1.48e-11 | 31 116 / 32 705 |
| `rust_moving_average_b_idiomatic` | large | 8.89e-12 | 5.85e-06 | 9 999 871 / 9 999 937 |
| `rust-par_moving_average_b_idiomatic` (any T) | large | 3.46e-13 | 3.26e-08 | 9 687 165 / 9 999 937 |
| Isocron `moving_average_b` | large | 1.61e-12 | 4.54e-08 | 9 729 002 / 9 999 937 |

The largest relative differences fall on windows whose exact mean is close to
zero. In the large input that is window 1 099 105 for all three programs:
the reference value is 4.69e-07, against window means of up to about 10^3.
There an absolute error of the order of the running error becomes a large
relative one. In the sequential running sum the error of `s` accumulates
over the 10^7 updates with nothing resetting it. That is why its large-size
differences are about 25 times larger in absolute terms than those of the
parallel version, which restarts every 4096 outputs.

## Verification

`reps = 1`, inputs `bench/data/{small,large}`, references
`bench/data/ref/<size>/<kernel>.out` (Isocron). `opt` and `par_opt` are compared
with `cmp`; the others by their largest relative difference
`|r − ref| / |ref|`. Threads: `RAYON_NUM_THREADS` = 1 and 4.

| Kernel | Size | `opt` | `par_opt` T=1 | `par_opt` T=4 | `idiomatic` | `par_idiomatic` T=1 | `par_idiomatic` T=4 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| moving_average | small | identical | identical | identical | 1.42e-12 | 1.42e-12 | 1.42e-12 |
| moving_average | large | identical | identical | identical | 8.51e-09 | 8.51e-09 | 8.51e-09 |
| filter_sum | small | identical | identical | identical | identical | 3.63e-16 | identical |
| filter_sum | large | identical | identical | identical | 1.49e-14 | 6.76e-15 | 1.4e-15 to 2.4e-15 (varies) |
| prefix_sum | small | identical | identical | identical | identical | identical | identical |
| prefix_sum | large | identical | identical | identical | identical | identical | identical |
| matvec | small | identical | identical | identical | 2.52e-14 | 2.52e-14 | 2.52e-14 |
| matvec | large | identical | identical | identical | 1.51e-12 | 1.51e-12 | 1.51e-12 |

- `moving_average idiomatic`: 82% of the outputs differ (26 744 of 32 705
  small, 8 155 174 of 9 999 937 large). The largest relative differences
  come from windows whose sum nearly cancels (inputs ±10^-3 to ±10^3).
- `filter_sum idiomatic` small is identical by chance (a single sum that
  rounds the same way). `filter_sum par_idiomatic`: rayon's reduction tree
  depends on work stealing, so its result changes from run to run
  (four runs at T=4: 1.35e-15, 1.65e-15, 1.8e-15, 2.4e-15).
- `matvec idiomatic`: 114 of 128 rows (small), 3 977 of 4 096 (large) differ.

Further checks (scripts outside the repository, against `bench/bin/isc_<kernel>`):

- 40 synthetic inputs × (`opt` + `par_opt` with 1, 2, 3, 4, 7, 16 threads):
  280 of 280 runs identical. The inputs: filter_sum with 0, 1, 31, 4095, 4096,
  4097, 8192, 12293, 20000 kept elements; 2·10^6 elements with 0.5% kept (a
  block spans several chunks); 97% kept; NaN, ±inf, subnormals, ±0.0.
  moving_average with n = 64, 65, 127, 1087 to 1089, 5000, 70001, all `-0.0`,
  mixed ±0.0, cancellation (1e16, 1, −1e16), ±inf. prefix_sum with
  n = 1, 7, 8, 9, 63, 1000, 100003, negatives, and sums that wrap around
  (|x| < 2^62). matvec with cols = 1, 31, 33, 100, 257, 4096, 5000, 9000.
- The crate built as `build_lanes.sh` builds it: V = 32 for `goldmont` (SSE,
  portable paths) and AVX2 without AVX-512, V = 16 and V = 64 native. `opt`
  and `par_opt` (1 and 3 threads) were run on the inputs above and the small
  data, against Isocron built with `--lanes 16/64` (`bench/bin/isc_*` for
  V = 32): 528 of 528 identical.
- The POC programs (`sum`, `dot`, `std_dev`, `histogram`, 16 bins) rebuilt
  after the change to `lib.rs` give the same outputs as before.
