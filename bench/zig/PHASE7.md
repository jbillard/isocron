# Zig: the kernels of phase 7 (SPEC section 14)

Four kernels with the interface of `examples/phase7/*.isc`
(`<kernel> <inputs...> <output.f64> <reps>`): moving average (window 64),
filter then sum, `i64` prefix sum, matrix-vector product. Built by
`bench/build_phase7_zig.sh` (called by `bench/build.sh`) with the flags and
caches of the other Zig programs (`zig build-exe -O ReleaseFast
-mcpu=native`, caches in `bench/build/zig-cache`):

| Source | Binary in `bench/bin/` |
| --- | --- |
| `<k>_idiomatic.zig`, `<k>_opt.zig` | `zig_<k>_idiomatic`, `zig_<k>_opt` |
| `<k>_par_idiomatic.zig`, `<k>_par_opt.zig` | `zig-par_<k>_idiomatic`, `zig-par_<k>_opt` (`std.Thread.Pool`, `$ISC_THREADS` threads) |
| `<k>_reassoc.zig` (moving_average, filter_sum, matvec) | `zig-reassoc_<k>_idiomatic` (`@setFloatMode(.optimized)`) |
| `moving_average_b_idiomatic.zig`, `moving_average_b_par_idiomatic.zig` | `zig_moving_average_b_idiomatic`, `zig-par_moving_average_b_idiomatic` (comparison B, below) |

`phase7.zig` holds the helpers of the optimized variants; the fixed reduction
form itself (`blockSum`, `blockDot`, `fixed`, `fixedSum`, V from `lanes.zig`)
is the one of `common.zig`. Every program loads its inputs, allocates its
output buffer once (and writes it, so that its pages are mapped before the
timing), times `reps` calls of a `noinline` kernel between
`std.mem.doNotOptimizeAway` barriers on the inputs and the output, prints the
total in nanoseconds and writes the result as little-endian `f64`.

## Variants

| Kernel | idiomatic (and reassoc) | opt |
| --- | --- | --- |
| moving_average | for each window, a left-to-right sum of its 64 elements, `/ 64.0` | the fixed form of each window, SIMD across 8 consecutive windows (AVX-512; 4 with AVX2) |
| filter_sum | `if (v > 0.0) s += v`, left to right | kept elements compacted (`vcompresspd`) into a buffer of one block, each full block reduced by `blockSum` |
| prefix_sum | `s +%= v; o.* = s` | in-register SIMD scan (shifts and adds), running total added to the vector |
| matvec | per row, `s += p * q` left to right | per row, `fixed` + `blockDot` of `common.zig` (the `dot` kernel of the POC) |

Parallel variants, contiguous chunks, one per thread:

* moving_average, matvec: the outputs (windows, rows) are split; each output
  is computed by one thread, so `par_opt` is the fixed form for any number of
  threads.
* filter_sum, idiomatic: `chunkSums` of `common.zig` (left-to-right sum of
  each chunk, then the partial sums in order; depends on the thread count).
  opt: two passes. (1) Each thread counts the kept elements of its chunk;
  their prefix sums give the rank of the first kept element of every chunk.
  (2) Each thread reduces the blocks of 4096 kept elements that *start* in
  its chunk: it skips the kept elements that belong to a block started
  before, and reads on into the following chunks to complete its last block.
  Each block result has its own slot, then they are combined as the fixed
  form does: the result does not depend on the number of threads.
* prefix_sum: two-pass blocked scan, both variants: (1) the sum of each chunk
  (wrapping), (2) each chunk scanned from the sum of the chunks before it
  (plain loops in the idiomatic variant, `sumI64` and the SIMD `scanFrom` in
  opt). Integer addition: any order gives the same result.

## How the opt variants reach the fixed form

* **moving_average.** Lane t of every vector belongs to window i + t, so each
  window gets exactly the operations of the fixed form, in the same order:
  acc[j] = (0.0 + w[j]) + w[j+32], then the tree d = 16, 8, 4, 2, 1. The
  tree is written as a depth-first recursion (`node` in `phase7.zig`), which
  computes the same additions with few live registers. The 32 additions of
  the neutral 0.0 are replaced by a single `0.0 +` at the root: (0 + a) + b
  and 0 + (a + b) are equal bit for bit (0 + a differs from a only for
  a = -0, and then both sides give b, or +0 when b is a zero too), and so are
  (0 + p) + (0 + q) and 0 + (p + q); by induction the whole tree equals
  0 + (the tree without the zeros). The division by 64 is exact as a
  multiplication by 2^-6 (LLVM makes that change; it does not change the
  result).
* **filter_sum.** The fixed form of the filtered array, built block by block:
  `compact` writes the elements > 0.0 of 512 input elements at a time into a
  buffer of 4096 + 520 doubles, in their order; as soon as the buffer holds
  4096, `blockSum` reduces them (reduce_block) and the rest moves to the
  front. At the end, the last (shorter) block is reduced. One block (or
  none): the result is reduce_block itself; more: `fixedSum` of the block
  results (recursively beyond 4096 blocks), exactly as `reduce` on the
  compacted array.
* **matvec.** Per row, the fixed form of row[k] * v[k] for k in increasing
  order (`blockDot`: each product rounded, then added; Zig does not contract
  into FMA outside `@setFloatMode(.optimized)`). cols <= 4096 here (one
  block); `fixed` also handles longer rows.
* **prefix_sum.** Integers: identical by construction.

### A pitfall: Zig folds `0.0 + x`

Zig 0.14.1 replaces a comptime-known `0.0 + x` by `x` during semantic
analysis (also in Debug builds), which is not an IEEE identity: +0 + -0 = +0.
The first version of the moving average, with `const zero = @splat(0.0)`,
gave -0.0 where Isocron gives +0.0 on a window of negative zeros (found by the
edge cases below). The zero is now a `var` (runtime for Zig's analysis); LLVM
itself keeps `+0.0 + x` in strict mode. `x + 0.0` is folded the same way. The
accumulators of `common.zig` are `var`s and are not affected.

## Verification

Each program run once (reps = 1) on `bench/data/{small,large}`, output
compared with Isocron's (`bench/data/ref/<size>/<kernel>.out`): `cmp` byte
for byte, otherwise the largest relative difference |zig - isc| / |isc|.
Parallel variants with `ISC_THREADS=1` and `4`.

| Size | Kernel | opt | par_opt (1 / 4 threads) | idiomatic | par_idiomatic (1 / 4) | reassoc |
| --- | --- | --- | --- | --- | --- | --- |
| small | moving_average | identical | identical / identical | 1.42e-12 | 1.42e-12 / 1.42e-12 | 1.42e-12 |
| small | filter_sum | identical | identical / identical | identical (1) | identical (1) / 7.26e-16 | identical (1) |
| small | prefix_sum | identical | identical / identical | identical | identical / identical | — |
| small | matvec | identical | identical / identical | 2.52e-14 | 2.52e-14 / 2.52e-14 | 2.21e-14 |
| large | moving_average | identical | identical / identical | 8.51e-09 (2) | 8.51e-09 / 8.51e-09 | 8.51e-09 |
| large | filter_sum | identical | identical / identical | 1.49e-14 | 1.49e-14 / 1.59e-14 | 4.06e-15 |
| large | prefix_sum | identical | identical / identical | identical | identical / identical | — |
| large | matvec | identical | identical / identical | 1.51e-12 | 1.51e-12 / 1.51e-12 | 5.52e-13 |

(1) A coincidence of rounding: a left-to-right sum in Python gives the same
value. (2) The largest absolute difference is 9.95e-14 (inputs in
[-1000, 1000]); the relative difference is large only where the mean is
close to zero (|mean| = 4.7e-7 at the worst window).

`par_opt` with 2, 3, 7 and 16 threads: identical on both sizes for the four
kernels (40 runs).

Edge cases on synthetic inputs, `zig_<k>_opt` and `zig-par_<k>_opt`
(1, 3 and 16 threads) against `isc_<k>`, all identical:

* filter_sum: nothing kept (all negative; only zeros and negatives), 13
  elements, exactly 4095, 4096, 4097, 8192 and 8193 kept elements, NaN, -inf,
  subnormals and -0.0 mixed in, +inf kept, 10^7 elements with one kept in 997
  (blocks spanning many chunks), 2 x 10^7 kept elements (4883 blocks: two
  levels of block results);
* moving_average: 64 elements (one window), 70, 1003 elements with exponents
  from 2^-60 to 2^60, signed zeros and cancelling pairs, only -0.0, NaN and
  infinities;
* prefix_sum: 3 elements, 100003 elements of both signs, values near 2^62
  (wrapping);
* matvec: 1, 37, 5000 and 8193 columns (two and three blocks per row).

## Comparison B: the moving average in O(n)

SPEC section 14, comparison B: the best algorithm per language. What an
experienced Zig developer writes is a running sum: s = the sum of the first
64 elements (left to right), out[0] = s / 64.0, then for each next window
s += x[i+63] - x[i-1] and out[i] = s / 64.0 (one subtraction and one
addition per window instead of 63 additions). The parallel variant splits
the windows into contiguous chunks and restarts the sum at the start of each
chunk, so its results depend on the number of threads. Isocron's comparison
B program is `examples/phase7/moving_average_b.isc` (prefix sums by `scan`,
then one difference per window).

The results differ from the exact sums of the windows by the rounding
accumulated in the running sum. Against Isocron's fixed form
(`bench/data/ref/<size>/moving_average.out`, reps = 1):

| Size | Program | Threads | Largest absolute difference | Largest relative difference |
| --- | --- | --- | --- | --- |
| small | `zig_moving_average_b_idiomatic` | 1 | 2.45e-13 | 1.10e-11 |
| small | `zig-par_moving_average_b_idiomatic` | 1 / 4 / 16 | 2.45e-13 / 2.74e-13 / 1.71e-13 | 1.10e-11 / 1.33e-11 / 9.25e-12 |
| large | `zig_moving_average_b_idiomatic` | 1 | 8.89e-12 | 5.85e-06 |
| large | `zig-par_moving_average_b_idiomatic` | 1 / 4 / 16 | 8.89e-12 / 5.73e-12 / 3.86e-12 | 5.85e-06 / 5.85e-06 / 2.09e-06 |

Inputs in [-1000, 1000]. The absolute error grows with the length of the
running sum: 2.45e-13 over 32705 windows, 8.89e-12 over 10^7; more threads
mean shorter sums, hence smaller errors. The largest relative differences are
where the mean is close to zero (|mean| = 4.7e-7 on the large input). For
comparison, the comparison A idiomatic loop (a left-to-right sum per window)
stays within 9.95e-14 absolute on the large input.

## Notes

* A finding for comparison B and for Isocron's code generator: in the fixed
  form, node (d, j) of the tree of the window at i depends only on i + j
  (level 32: x[i+j] + x[i+j+32]; level d: node(2d, j) + node(2d, j+d)). So
  consecutive windows share their partial sums, and the exact Isocron result
  is out[i] = (0 + N1[i]) / 64 with N32[k] = x[k] + x[k+32] and
  Nd[k] = N2d[k] + N2d[k+d]: 6 additions per output instead of 63. A scratch
  version (not in this directory) gave output identical to Isocron's on both
  sizes; untiled, it is twice as fast as `opt` on the small input but slower
  on the large one (six passes through memory). A tiled version would be
  needed.
* filter_sum: the compaction is limited by store traffic, not by
  `vcompresspd`. On the small input, the register form followed by a 64-byte
  store takes 0.20 ns per element, a table of indices with `vpermpd` the
  same, the loop without the store 0.11, and `vcompresspd` to memory (which
  writes only the kept elements) 0.11: `opt` uses the memory form, except on
  AMD Zen 4 and 5, where it is microcoded (`compress_to_memory` in
  `phase7.zig`).
