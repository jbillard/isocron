# C programs of the kernels of phase 7

The four kernels added to SPEC section 14 by phase 7, in C, with the
conventions of the POC programs (`sum.c`, `dot.c`, `common.h`): one file per
kernel, `VARIANT=0` idiomatic, `VARIANT=1` hand-optimized, the same file
built with `-fopenmp` for the parallel variants (threads from
`OMP_NUM_THREADS`). Interface of `examples/phase7/*.isc`:

```
moving_average <a.f64> <out.f64> <reps>
filter_sum     <a.f64> <out.f64> <reps>
prefix_sum     <i.f64> <out.f64> <reps>
matvec         <m.f64> <v.f64> <out.f64> <reps>
```

Each program loads its inputs and allocates its output and work buffers once.
It then times `reps` calls of the kernel only (`noinline`, `KEEP_PTR` and
`KEEP_F64` around each call), prints the total in nanoseconds and writes the
result as little-endian `f64`.

`bench/build_phase7_c.sh` builds them into `bench/bin/` with the flags of
`bench/build.sh`, which calls it at its end:

| Program | Flags |
| --- | --- |
| `c-gcc_<k>_<v>` | `-O3 -march=native -ffp-contract=off` |
| `c-clang_<k>_<v>` | same, `zig cc -target x86_64-linux-gnu` (clang 19) |
| `c-gcc-omp_<k>_<v>` | same as gcc + `-fopenmp` |
| `c-gcc-reassoc_<k>_idiomatic`, `c-clang-reassoc_<k>_idiomatic` | `-O3 -march=native -fassociative-math -fno-signed-zeros -fno-trapping-math` (moving_average, filter_sum, matvec) |

`<v>` is `idiomatic` or `opt`. All the builds also use `-std=c11 -D_POSIX_C_SOURCE=200809L -Wall -Wno-unknown-pragmas`.
That makes 30 programs. Comparison B of the moving average adds 3 more
(`c-gcc_`, `c-clang_` and `c-gcc-omp_moving_average_b_idiomatic`, below),
33 in all.

## The variants

### moving_average

`out[i] = sum(x[i .. i + 64]) / 64.0`, n - 63 values (n < 64 is refused,
as Isocron panics).

* **idiomatic**: for each window, a left-to-right sum of its 64 elements,
  then `/ 64.0`; OpenMP splits the windows. It is the same algorithm as
  Isocron (comparison A of section 14), so its cost is O(64 n). The O(n)
  running sum is comparison B, `moving_average_b.c` below.
* **opt**: the fixed form of each window. It is one block, so it starts with
  `acc[j] = (0.0 + w[j]) + w[j + 32]` for j < 32, then applies
  `acc[j] = acc[j] + acc[j + d]` for d = 16, 8, 4, 2, 1, and returns
  `acc[0] / 64.0`. At every level of this tree, `acc[j]` of window i
  depends only on the position i + j. Windows that overlap therefore share
  their values:

  ```
  L32(k) = (0.0 + x[k]) + x[k + 32]
  L16(k) = L32(k) + L32(k + 16)      L8(k) = L16(k) + L16(k + 8)
  L4(k)  = L8(k) + L8(k + 4)         L2(k) = L4(k) + L4(k + 2)
  L1(k)  = L2(k) + L2(k + 1)         out[i] = L1(i) / 64.0
  ```

  `acc[j]` of window i after level d is exactly `Ld(i + j)`: the same
  operands, added in the same order. The result is therefore bit-identical
  to Isocron. Each level is computed once per position, so a window costs 6
  additions instead of 63. The levels are computed by tiles of 1024 windows
  in two stack buffers that stay in L1, using vectorized loops; OpenMP
  splits the tiles. `0.0 + x[k]` is kept because the strict flags keep
  signed zeros. `/ 64.0` becomes a multiplication by 2^-6, which gives the
  same result. Isocron evaluates `sum(w)` separately for each window (63
  additions). Sharing the levels is a legal rewriting of the fixed form,
  and the Isocron compiler could apply it too.

### moving_average_b (comparison B of section 14)

`moving_average_b.c`: the O(n) algorithm an experienced C developer writes, a
running sum. It starts with s = the left-to-right sum of the first 64
elements. Then, for each window, it sets `out[i] = s / 64.0` followed by
`s += x[i + 64] - x[i]`. There is only an idiomatic variant:
`c-gcc_moving_average_b_idiomatic`, `c-clang_moving_average_b_idiomatic`,
and `c-gcc-omp_moving_average_b_idiomatic`. In the OpenMP build each thread
takes one contiguous chunk of windows and restarts the running sum at the
start of its chunk, so the results depend on the number of threads.
Isocron's comparison B is `examples/phase7/moving_average_b.isc`: the prefix
sums (`scan`, left to right), then `(p[i + 63] - p[i - 1]) / 64.0`.

Neither is exact. The largest differences to the exact window sums
(`bench/data/ref/<size>/moving_average.out`, the fixed form) are below.
`isc_moving_average_b` is not in `bench/bin`; for this table it was built
from the example with `iscc build`, outside the repository.

| program | size | largest absolute difference | largest relative difference |
| --- | --- | --- | --- |
| c-gcc, c-clang, c-gcc-omp T = 1 | small | 2.45e-13 | 1.10e-11 |
| c-gcc-omp T = 4 | small | 2.74e-13 | 1.33e-11 |
| isc_moving_average_b | small | 3.41e-13 | 1.48e-11 |
| c-gcc, c-clang, c-gcc-omp T = 1 | large | 8.89e-12 | 5.85e-06 |
| c-gcc-omp T = 4 | large | 5.73e-12 | 5.85e-06 |
| isc_moving_average_b | large | 1.61e-12 | 4.54e-08 |

For comparison, the idiomatic O(64 n) window sum differs by at most 8.51e-09
relative on the large size (table below).

The running sum carries its rounding errors from window to window. Over 10^7
windows they add up like a random walk to about 9e-12 on the mean. Restarting
the sum per thread shortens that walk, which is why T = 4 gives 5.7e-12. The
largest relative difference, 5.85e-06, falls on a window whose mean almost
cancels. Isocron's prefix sums grow large (|p| up to about 10^6), but
p[i + 63] and p[i - 1] carry nearly the same accumulated error, and it
cancels in their difference. What remains is the rounding of the window's
own 64 additions and of the subtraction, so on the large size its error is
about 5 times smaller in absolute terms and 100 times smaller in relative
terms. `python3 bench/c/verify_phase7.py` prints these rows for the programs
present in `bench/bin`.

### filter_sum

The sum of the elements > 0.0.

* **idiomatic**: `if (x[i] > 0.0) s += x[i];`, with an OpenMP
  `reduction(+ : s)`.
* **opt**: applies the fixed form to the kept elements in their order, as
  `fixed_sum_mem` does on the compacted array. The kept elements are
  compacted 1024 input elements at a time into a stack buffer. With AVX-512
  this uses `vcmppd` + `vcompresspd` into a register, then a store; the
  memory form of `vcompresspd` is slow on Intel. Without AVX-512 it uses a
  branchless loop. The buffer is added 32 elements at a time to the 32
  accumulators, so kept element k goes to accumulator k mod 32. Every 4096
  kept elements, the block is reduced by the tree into `p[b]`. At the end, the
  last partial group goes to accumulators 0, 1, ... and the last block is
  closed. The block results are then reduced:
  * 0 kept elements: +0.0, the tree of 32 zeros;
  * at most 4096: the block result itself;
  * more: `fixed_sum_mem(p, blocks)`, which applies the upper levels
    recursively.
* **opt, OpenMP**: one chunk of x per thread. A first pass counts the kept
  elements of each chunk; the prefix sums of the counts give each chunk the
  index of its first kept element. A chunk owns the blocks that *start* in
  its range of kept indices. It skips the kept elements of a block started
  by an earlier chunk, and reads past its own end to finish its last block.
  Every block is thus computed by one thread exactly as in the sequential
  form, and the result does not depend on the number of threads. One thread
  skips the counting pass.

### prefix_sum

The input (integers stored as `f64`) is converted to `int64_t` before the
timed loop. After it, the n sums are written as `f64`. The arithmetic is done
in `uint64_t`, which wraps like Isocron's `i64` with no undefined behaviour.
Both variants are exact.

* **idiomatic**: `s += x[i]; out[i] = s;`. With OpenMP it uses the OpenMP 5.0
  scan: `parallel for reduction(inscan, + : s)` with `#pragma omp scan
  inclusive(s)`. The loop index is signed. With a `size_t` index and n = 0,
  GCC 14.2 writes out of bounds in its lowering of the inscan reduction
  (found by the edge tests and reproduced with a minimal program).
* **opt**: with AVX-512, the prefix sums of 8 elements are computed in a
  register with three `valignq` shifts and adds. The carry is then added and
  broadcast from lane 7 (`vpermq`). Without AVX-512 it is the scalar loop.
  With OpenMP it is a two-pass blocked scan, one chunk per thread: the sum
  of each chunk, a barrier, then each chunk scanned from the sum of the
  chunks before it.

### matvec

`cols` is the length of v and `rows` is `len(m) / cols`, as `rows_of` in
`examples/phase7/matvec.isc`. m is stored row after row.

* **idiomatic**: for each row, a left-to-right sum of `row[k] * v[k]`; OpenMP
  splits the rows.
* **opt**: for each row, the fixed form of the products, using
  `FIXED_SUM_OF` of `common.h` as in `dot.c`. Each product is rounded, then
  added to accumulator k mod 32, with passes of `GROUP2` accumulators, then
  the tree. `cols` above 4096 takes the multi-block form, which is checked
  by the edge tests. OpenMP splits the rows.

## Verification

`python3 bench/c/verify_phase7.py --edge` (about 15 s) runs every program
with reps = 1 on `bench/data/small` and `bench/data/large`. It compares each
output with Isocron's (`bench/data/ref/<size>/<kernel>.out`): first byte for
byte, and otherwise by the largest relative difference |c - r| / |r|.

**opt**: all identical byte for byte to Isocron, on both sizes, for
`c-gcc`, `c-clang`, and `c-gcc-omp` with `OMP_NUM_THREADS` = 1 and 4:

| kernel | c-gcc opt | c-clang opt | c-gcc-omp opt, T = 1 | c-gcc-omp opt, T = 4 |
| --- | --- | --- | --- | --- |
| moving_average | identical | identical | identical | identical |
| filter_sum | identical | identical | identical | identical |
| prefix_sum | identical | identical | identical | identical |
| matvec | identical | identical | identical | identical |

**idiomatic**: the largest relative difference to Isocron (identical: byte for
byte). The parentheses give how many values differ.

| kernel | size | c-gcc, c-clang | c-gcc-omp T = 1 | c-gcc-omp T = 4 | c-gcc-reassoc | c-clang-reassoc |
| --- | --- | --- | --- | --- | --- | --- |
| moving_average | small | 1.42e-12 (26 744 / 32 705) | 1.42e-12 | 1.42e-12 | 1.52e-12 (20 429) | 8.97e-13 (19 165) |
| moving_average | large | 8.51e-09 (8 155 174 / 9 999 937) | 8.51e-09 | 8.51e-09 | 1.14e-08 (6 241 381) | 2.19e-10 (5 877 395) |
| filter_sum | small | identical | identical | 7.26e-16 | identical | identical |
| filter_sum | large | 1.49e-14 | 1.49e-14 | 1.59e-14 | 5.26e-15 | 1.49e-14 |
| prefix_sum | small | identical | identical | identical | — | — |
| prefix_sum | large | identical | identical | identical | — | — |
| matvec | small | 2.52e-14 (114 / 128) | 2.52e-14 | 2.52e-14 | 1.25e-14 (103) | 2.21e-14 (88) |
| matvec | large | 1.51e-12 (3 977 / 4 096) | 1.51e-12 | 1.51e-12 | 1.05e-12 (3 897) | 5.52e-13 (3 720) |

* moving_average, large: the largest relative differences come from windows
  whose sum nearly cancels. The worst window has a mean of 4.69e-7, from
  values of magnitude up to 1000. The largest absolute difference is
  9.9e-14.
* filter_sum, small: the left-to-right sum happens to round to the same
  double as the fixed form (1282095.680329701). A Python recomputation of
  both orders confirms it.
* The OpenMP idiomatic results depend on the number of threads (filter_sum
  T = 4). The opt results do not.

**Edge cases** (`--edge`): synthetic inputs, run through Isocron
(`bench/bin/isc_<k>`) and compared byte for byte. The opt builds are checked
with gcc and clang, and with OpenMP at T = 1, 2, 3, 5 and 16; the
idiomatic prefix_sum is checked the same way. The inputs:

* filter_sum: 0, 1, 31, 32, 33, 4095, 4096, 4097, 8191, 8192, 8193, 12305
  and 40000 kept elements, with negatives, -0.0, +0.0, NaN and -inf between
  them. Values of magnitudes 1e-8 to 1e8, so that any change of order shows.
  Also a sparse input (5000 kept in 200000), all positive, and empty.
* moving_average: n = 64, 65, 100, 1087, 1088, 1089 (tile boundaries), 2111
  and 5000, and runs of -0.0.
* prefix_sum: n = 0, 1, 7, 8, 9, 17 and 1000003, and sums that wrap past
  2^63.
* matvec: 3×5, 7×31, 5×32, 9×100, 3×4096, 2×5000 and 1×9000 (cols above
  4096: several blocks).

Result: 336 checks, 0 failures. reps = 2 also gives the same output, so the
work buffers are reused correctly.

## Notes

* **Vector width**: at `-march=native` on the test machine (Xeon E-2388G,
  Rocket Lake), GCC 14 and clang 19 auto-vectorize with 256-bit `ymm`
  registers. Their tuning for this CPU prefers 256 bits, and the POC
  programs behave the same way (no `zmm` in `c-gcc_sum_opt`). Isocron uses
  `zmm` (512 bits). Only the explicit intrinsics use `zmm` here:
  `vcompresspd` in filter_sum and the scan of prefix_sum. The flags are
  those of `bench/build.sh` and are not changed. A build with
  `-mprefer-vector-width=512` would be an extra variant; it was not made.
* **Choices** (DECISIONS.md is outside the scope of this task):
  * the idiomatic moving_average is the direct O(64 n) window sum; the
    running sum is the separate comparison B program `moving_average_b.c`;
  * n < 64 is refused as by Isocron;
  * i64 wrapping is done through `uint64_t` in both variants;
  * the OpenMP idiomatic prefix_sum uses `omp scan`;
  * the work buffers (block results, per-thread counts or sums) are
    allocated once, outside the timed loop.
* No timing was taken beyond the single verification runs; the measurements
  are left to the benchmark campaign.
