# Isocron — summary of the campaign `stage2`

One core: 2026-11-02T16:07:22; threads: 2026-11-02T16:56:02; compilation: 2026-11-02T16:43:15 (machine xeon). Detailed reports:
[stage2-single.md](stage2-single.md), [stage2.md](stage2.md), [stage2-compile.md](stage2-compile.md). Verdict rule of SPEC
section 14: *beats* when Isocron's median is lower by more than the dispersion (the larger IQR of the
two), *equals* within the dispersion, *slower* otherwise.

Note: the matvec kernel was measured again on 2026-11-02, after DECISIONS D121 (the threads start at different rows of their chunks); every other kernel comes from the campaign.

## One core: Isocron against the best strict variant of the other languages

| Kernel | Size | Isocron | Best other | Difference | Verdict |
| --- | --- | --- | --- | --- | --- |
| sum | small | 1.78 µs | 1.85 µs (Zig, opt) | -3.8 % | 🟡 equals |
| dot | small | 3.74 µs | 6.95 µs (Zig, opt) | -46.2 % | 🟢 **beats** |
| std_dev | small | 5.68 µs | 5.80 µs (Zig, opt) | -2.0 % | 🟡 equals |
| histogram | small | 90.79 µs | 94.47 µs (Rust, opt) | -3.9 % | 🟢 **beats** |
| histogram_skewed | small | 91.49 µs | 96.84 µs (Rust, opt) | -5.5 % | 🟢 **beats** |
| moving_average | small | 19.89 µs | 30.86 µs (C (GCC), opt) | -35.6 % | 🟢 **beats** |
| moving_average_b | small | 19.88 µs | 44.64 µs (Rust, idiomatic) | -55.5 % | 🟢 **beats** |
| filter_sum | small | 5.75 µs | 5.82 µs (C (GCC), opt) | -1.1 % | 🟡 equals |
| prefix_sum | small | 6.62 µs | 8.18 µs (C (Clang), opt) | -19.1 % | 🟢 **beats** |
| matvec | small | 2.88 µs | 3.22 µs (Zig, opt) | -10.4 % | 🟢 **beats** |
| sum | large | 3.473 ms | 3.493 ms (Rust, opt) | -0.6 % | 🟡 equals |
| dot | large | 6.484 ms | 6.893 ms (C (Clang), opt) | -5.9 % | 🟢 **beats** |
| std_dev | large | 6.944 ms | 7.013 ms (Rust, opt) | -1.0 % | 🟢 **beats** |
| histogram | large | 3.502 ms | 3.616 ms (Rust, opt) | -3.2 % | 🟢 **beats** |
| histogram_skewed | large | 3.539 ms | 3.731 ms (Rust, opt) | -5.2 % | 🟢 **beats** |
| moving_average | large | 9.171 ms | 14.405 ms (C (GCC), opt) | -36.3 % | 🟢 **beats** |
| moving_average_b | large | 9.167 ms | 14.205 ms (Rust, idiomatic) | -35.5 % | 🟢 **beats** |
| filter_sum | large | 4.017 ms | 4.357 ms (Rust, opt) | -7.8 % | 🟢 **beats** |
| prefix_sum | large | 5.544 ms | 7.871 ms (Zig, opt) | -29.6 % | 🟢 **beats** |
| matvec | large | 6.439 ms | 6.738 ms (C (Clang), opt) | -4.4 % | 🟢 **beats** |

Isocron has the lowest median in 20 of the 20 cases (16 *beats*, 4 *equals*, 0 *slower*).

## One core: `allow_reassoc` against the reassociated variants

| Kernel | Size | Isocron allow_reassoc | Best reassociated | Difference | Verdict |
| --- | --- | --- | --- | --- | --- |
| sum | small | 1.78 µs | 2.58 µs (Zig (reassociated)) | -30.9 % | 🟢 **beats** |
| dot | small | 3.74 µs | 7.56 µs (C (Clang, reassociated)) | -50.5 % | 🟢 **beats** |
| std_dev | small | 4.41 µs | 5.25 µs (C (Clang, reassociated)) | -16.0 % | 🟢 **beats** |
| moving_average | small | 20.05 µs | 117.29 µs (C (GCC, reassociated)) | -82.9 % | 🟢 **beats** |
| filter_sum | small | 1.77 µs | 2.64 µs (Zig (reassociated)) | -33.0 % | 🟢 **beats** |
| matvec | small | 2.74 µs | 3.92 µs (C (Clang, reassociated)) | -30.2 % | 🟢 **beats** |
| sum | large | 3.474 ms | 3.554 ms (Zig (reassociated)) | -2.2 % | 🟢 **beats** |
| dot | large | 6.484 ms | 6.896 ms (C (Clang, reassociated)) | -6.0 % | 🟢 **beats** |
| std_dev | large | 7.052 ms | 7.178 ms (Zig (reassociated)) | -1.8 % | 🟢 **beats** |
| moving_average | large | 9.160 ms | 36.364 ms (C (GCC, reassociated)) | -74.8 % | 🟢 **beats** |
| filter_sum | large | 3.514 ms | 3.614 ms (Zig (reassociated)) | -2.8 % | 🟢 **beats** |
| matvec | large | 6.394 ms | 6.654 ms (C (Clang, reassociated)) | -3.9 % | 🟢 **beats** |

Isocron's `allow_reassoc` programs have the lowest median in 12 of the 12 cases.

## Comparison B: the moving average, distance to the exact sums of the windows

| Size | Implementation | Variant | Largest absolute difference | Largest relative difference |
| --- | --- | --- | --- | --- |
| small | Isocron | - | 0.00e+00 | 0.00e+00 |
| small | C (Clang) | idiomatic | 2.45e-13 | 1.10e-11 |
| small | C (GCC) | idiomatic | 2.45e-13 | 1.10e-11 |
| small | Go | idiomatic | 2.45e-13 | 1.10e-11 |
| small | Rust | idiomatic | 2.45e-13 | 1.10e-11 |
| small | Zig | idiomatic | 2.45e-13 | 1.10e-11 |
| large | Isocron | - | 0.00e+00 | 0.00e+00 |
| large | C (Clang) | idiomatic | 8.89e-12 | 5.85e-06 |
| large | C (GCC) | idiomatic | 8.89e-12 | 5.85e-06 |
| large | Go | idiomatic | 8.89e-12 | 5.85e-06 |
| large | Rust | idiomatic | 8.89e-12 | 5.85e-06 |
| large | Zig | idiomatic | 8.89e-12 | 5.85e-06 |

## Threads: Isocron against C (OpenMP), Rust (rayon), Go and Zig

Best parallel variant of the other languages for the same number of threads T; the difference is
Isocron's median against it.

| Kernel | Size | T=1 | T=2 | T=4 | T=8 |
| --- | --- | --- | --- | --- | --- |
| sum | small | beats (-44.6 %) | beats (-45.7 %) | beats (-49.4 %) | beats (-55.9 %) |
| dot | small | beats (-53.4 %) | beats (-37.9 %) | beats (-41.9 %) | beats (-48.3 %) |
| std_dev | small | beats (-24.3 %) | beats (-29.2 %) | beats (-38.6 %) | beats (-51.0 %) |
| histogram | small | beats (-3.9 %) | beats (-4.3 %) | beats (-6.4 %) | beats (-14.6 %) |
| histogram_skewed | small | beats (-5.1 %) | beats (-6.1 %) | beats (-8.1 %) | beats (-13.4 %) |
| moving_average | small | beats (-35.2 %) | beats (-30.8 %) | beats (-34.4 %) | beats (-41.1 %) |
| moving_average_b | small | beats (-58.1 %) | beats (-57.0 %) | beats (-58.2 %) | beats (-58.8 %) |
| filter_sum | small | equals (-0.5 %) | beats (-23.7 %) | beats (-26.2 %) | beats (-36.9 %) |
| prefix_sum | small | beats (-27.3 %) | beats (-15.8 %) | beats (-18.8 %) | beats (-22.8 %) |
| matvec | small | beats (-53.3 %) | beats (-49.8 %) | beats (-49.4 %) | beats (-52.1 %) |
| sum | large | equals (-0.7 %) | beats (-3.9 %) | equals (-1.0 %) | beats (-2.3 %) |
| dot | large | beats (-7.4 %) | beats (-4.2 %) | beats (-4.3 %) | equals (-0.5 %) |
| std_dev | large | beats (-2.6 %) | beats (-5.2 %) | equals (-1.5 %) | beats (-1.9 %) |
| histogram | large | beats (-2.7 %) | beats (-2.9 %) | beats (-3.0 %) | beats (-3.6 %) |
| histogram_skewed | large | beats (-4.2 %) | beats (-4.0 %) | beats (-4.3 %) | beats (-4.2 %) |
| moving_average | large | beats (-37.8 %) | beats (-34.6 %) | beats (-29.2 %) | beats (-32.3 %) |
| moving_average_b | large | beats (-38.0 %) | beats (-28.6 %) | beats (-24.8 %) | beats (-35.3 %) |
| filter_sum | large | beats (-8.5 %) | beats (-32.0 %) | beats (-35.4 %) | equals (+4.0 %) |
| prefix_sum | large | beats (-34.5 %) | beats (-23.0 %) | beats (-25.6 %) | beats (-28.3 %) |
| matvec | large | equals (-4.7 %) | equals (-4.5 %) | equals (-2.0 %) | equals (-1.1 %) |

Isocron has the lowest median in 79 of the 80 cases.
Not in: filter_sum, large, T=8: Isocron 1.814 ms, c-gcc-omp_filter_sum_idiomatic 1.744 ms (dispersion 85.78 µs).

## Compilation times

Median over the programs of each implementation; *warm*: the source changed and built again; *cold*:
every cache empty (GCC and Isocron have no cache).

| Implementation | Programs | Warm wall | Cold wall |
| --- | --- | --- | --- |
| Isocron | 8 | 0.08 s | same (no cache) |
| C (GCC) | 16 | 0.07 s | same (no cache) |
| C (Clang, zig cc) | 16 | 0.07 s | 3.49 s |
| Rust | 16 | 0.13 s | 2.77 s |
| Go | 16 | 0.11 s | 2.76 s |
| Zig | 16 | 0.79 s | 4.16 s |

Isocron per program, warm: `sum` 0.062 s, `dot` 0.062 s, `std_dev` 0.079 s, `histogram` 0.082 s, `moving_average` 0.156 s, `filter_sum` 0.282 s, `prefix_sum` 0.117 s, `matvec` 0.088 s.

