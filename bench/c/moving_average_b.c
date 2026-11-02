/* moving_average_b <input.f64> <output.f64> <reps>
 * Comparison B of SPEC section 14: the O(n) algorithm an experienced C
 * developer writes for out[i] = sum(x[i .. i + 64]) / 64.0 (n - 63 values),
 * a running sum: s = the sum of the first 64 elements, then, from one window
 * to the next, s += x[i + 64] - x[i]. Its results differ from the sums of
 * the windows by the rounding accumulated in s (see PHASE7.md).
 * Only an idiomatic variant. With -fopenmp: one contiguous chunk of windows
 * per thread, each restarting the running sum at the start of its chunk (the
 * results then depend on the number of threads). */
#include "common.h"
#ifdef _OPENMP
#include <omp.h>
#endif

#define WIN 64

/* windows lo .. hi - 1 */
static void running(const double *x, double *out, size_t lo, size_t hi) {
    if (lo >= hi) return;
    double s = 0.0;
    for (size_t j = 0; j < WIN; j++) s += x[lo + j];
    for (size_t i = lo; i < hi; i++) {
        out[i] = s / 64.0;
        if (i + 1 < hi) s += x[i + WIN] - x[i];
    }
}

__attribute__((noinline)) static void kernel(const double *x, size_t n, double *out) {
    size_t m = n - (WIN - 1);
#pragma omp parallel
    {
        size_t t = 1, k = 0;
#ifdef _OPENMP
        t = (size_t)omp_get_num_threads();
        k = (size_t)omp_get_thread_num();
#endif
        running(x, out, m * k / t, m * (k + 1) / t);
    }
}

int main(int argc, char **argv) {
    if (argc != 4) die("usage:", "moving_average_b <input.f64> <output.f64> <reps>");
    size_t n;
    double *x = read_f64(argv[1], &n);
    if (n < WIN) die("input shorter than the window:", argv[1]); /* as Isocron */
    size_t m = n - (WIN - 1);
    double *out = aligned_alloc(64, (m * sizeof(double) + 64) / 64 * 64);
    /* the pages of the output touched before the timer, in every language
       (DECISIONS D117); not zeros, which GCC may turn into calloc */
    memset(out, 0xff, (m * sizeof(double) + 64) / 64 * 64);
    long reps = reps_arg(argv[3]);
    uint64_t t0 = now_ns();
    for (long k = 0; k < reps; k++) {
        KEEP_PTR(x);
        KEEP_PTR(out);
        kernel(x, n, out);
        KEEP_PTR(out);
    }
    uint64_t t1 = now_ns();
    printf("%llu\n", (unsigned long long)(t1 - t0));
    write_f64(argv[2], out, m);
    return 0;
}
