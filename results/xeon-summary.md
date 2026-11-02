# Isocron on xeon — summary of the official campaign

Campaign of 2026-11-02 (10:44 to 11:29), machine xeon (Intel Xeon E-2388G, 8 cores, 16 logical CPUs),
every language in one session, with the changes D83 to D87 (large arrays placed 1088 or 3136 bytes after a
page, the same huge pages for every language during the measurements, the block results of `reduce` on the
stack, the last elements of a block through memory, four rows per iteration from memory, the runtime built
with LTO). Steps: `bench/build.sh`, `bench/run.py --name xeon-single` (one core), `bench/compile.py --name
xeon-compile`, `bench/parallel.py --name xeon` (1, 2, 4 and 8 threads), all through `bench/machine.sh`
(fixed frequency, turbo off, other processes confined, page cache dropped), then `bench/machine.sh restore`
(nginx, Plex and dockerd back on CPUs 0-15). No run failed; every Isocron result is bit-for-bit identical to
the reference and does not depend on the number of threads.

Detailed reports: [xeon-single.md](xeon-single.md) (one core), [xeon.md](xeon.md) (threads),
[xeon-compile.md](xeon-compile.md) (compilation). Verdict rule of SPEC section 14: *beats* when
Isocron's median is lower by more than the dispersion (the larger IQR of the two), *equals* within the
dispersion, *slower* otherwise. Medians of 30 interleaved measurements after 5 warm-up measurements.

**Isocron has the lowest median in all 50 cases**: 10 on one core (9 *beats*, 1 *equals*) and 40 with
threads (39 *beats*, 1 *equals*). The night before (campaign of 00:38), Rust was ahead on the large `sum`
and `std_dev` with one core and one thread, Zig on the small `std_dev`, Rust and C on the large `sum` with
4 threads; DECISIONS D83 to D87 give the causes measured and the changes.

## One core: Isocron against the best strict variant of every language

C (GCC and Clang), Rust, Go and Zig, idiomatic and hand-optimized variants; the reassociated C/Zig variants
are excluded because they change the floating-point result (they are compared with Isocron's
`allow_reassoc`, not implemented yet: the reassociated small `std_dev` takes 5.24 µs).

| Kernel | Size | Isocron | Best other | Difference | Verdict |
| --- | --- | --- | --- | --- | --- |
| sum | small | 1.78 µs | 1.85 µs (Zig, opt) | -3.68 % | **beats** |
| dot | small | 3.74 µs | 6.79 µs (Zig, opt) | -44.96 % | **beats** |
| std_dev | small | 5.69 µs | 5.79 µs (Zig, opt) | -1.83 % | equals |
| histogram | small | 90.80 µs | 94.36 µs (Rust, opt) | -3.77 % | **beats** |
| histogram_skewed | small | 91.49 µs | 97.14 µs (Rust, opt) | -5.82 % | **beats** |
| sum | large | 3.463 ms | 3.493 ms (Rust, opt) | -0.85 % | **beats** |
| dot | large | 6.477 ms | 6.887 ms (Clang, opt) | -5.95 % | **beats** |
| std_dev | large | 6.946 ms | 7.017 ms (Rust, opt) | -1.00 % | **beats** |
| histogram | large | 3.501 ms | 3.615 ms (Rust, opt) | -3.16 % | **beats** |
| histogram_skewed | large | 3.539 ms | 3.735 ms (Rust, opt) | -5.24 % | **beats** |

## Threads: Isocron against C (OpenMP), Rust (rayon), Go and Zig

Best parallel variant of the other languages for the same number of threads T; the difference is
Isocron's median against it.

| Kernel | Size | T=1 | T=2 | T=4 | T=8 |
| --- | --- | --- | --- | --- | --- |
| sum | small | beats (-44.7 %) | beats (-46.1 %) | beats (-49.7 %) | beats (-56.3 %) |
| dot | small | beats (-54.8 %) | beats (-37.7 %) | beats (-41.7 %) | beats (-48.8 %) |
| std_dev | small | beats (-24.4 %) | beats (-29.3 %) | beats (-38.3 %) | beats (-51.1 %) |
| histogram | small | beats (-3.5 %) | beats (-4.0 %) | beats (-5.9 %) | beats (-14.9 %) |
| histogram_skewed | small | beats (-4.8 %) | beats (-5.6 %) | beats (-7.6 %) | beats (-13.2 %) |
| sum | large | equals (-0.63 %, Rust) | beats (-4.2 %) | beats (-1.0 %) | beats (-2.3 %) |
| dot | large | beats (-7.2 %) | beats (-4.9 %) | beats (-4.1 %) | beats (-1.0 %) |
| std_dev | large | beats (-2.3 %) | beats (-4.8 %) | beats (-1.5 %) | beats (-2.3 %) |
| histogram | large | beats (-2.7 %) | beats (-2.9 %) | beats (-3.0 %) | beats (-3.3 %) |
| histogram_skewed | large | beats (-4.1 %) | beats (-4.0 %) | beats (-4.4 %) | beats (-4.3 %) |

On the large size the float kernels are limited by memory bandwidth from 4 threads on: the large dot product
is fastest at T=4 for Isocron (3.264 ms), and at T=8 the best variant of every implementation is between
3.41 and 3.97 ms.

## Compilation times

Median over the programs of each implementation; *warm*: caches filled, the source changed and built again
(the edit-compile cycle); *cold*: every cache empty (GCC and Isocron have no cache).

| Implementation | Warm wall | Warm CPU | Cold wall | Cold CPU |
| --- | --- | --- | --- | --- |
| Isocron | 0.07 s | 0.07 s | same (no cache) | same |
| C (GCC) | 0.07 s | 0.07 s | same (no cache) | same |
| C (Clang, zig cc) | 0.07 s | 0.08 s | 3.49 s | 4.89 s |
| Rust | 0.12 s | 0.12 s | 2.69 s | 4.52 s |
| Go | 0.11 s | 0.25 s | 2.76 s | 10.16 s |
| Zig | 0.79 s | 0.82 s | 4.15 s | 4.89 s |

Isocron per program, warm: `sum` 0.062 s, `dot` 0.062 s, `std_dev` 0.079 s, `histogram` 0.084 s, each
faster than the night before (0.065, 0.066, 0.081, 0.089 s).

## Open questions

* Why an array read from memory is about 3 % slower when it starts 0 or 64 bytes after a 4 KiB boundary
  (D83): measured with the same C loop, with or without blocks, each hardware prefetcher disabled in turn,
  the same volume read from memory; the mechanism is not identified.
* Why a large array filled by `read` is read about 1 % slower during its first ten passes (D83), and why
  four rows per iteration are faster from memory and slower from the caches (D86).
