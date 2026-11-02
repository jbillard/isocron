/* histogram <input.u8> <output.f64> <reps>
 * VARIANT 0: h[x[i]]++. VARIANT 1: four sub-histograms (breaks the
 * store-to-load dependency on repeated values), merged at the end.
 * Integer counts: both are exact. */
#include "common.h"
#ifdef _OPENMP
#include <omp.h>
#endif

#if VARIANT == 0
__attribute__((noinline)) static void kernel(const uint8_t *x, size_t n, uint32_t *h) {
    memset(h, 0, 256 * sizeof(uint32_t));
#pragma omp parallel for reduction(+ : h[:256])
    for (size_t i = 0; i < n; i++) h[x[i]]++;
}
#else
static void sub_histograms(const uint8_t *x, size_t n, uint32_t *h) {
    uint32_t t[4][256];
    memset(t, 0, sizeof t);
    size_t i = 0;
    for (; i + 4 <= n; i += 4) {
        t[0][x[i]]++;
        t[1][x[i + 1]]++;
        t[2][x[i + 2]]++;
        t[3][x[i + 3]]++;
    }
    for (; i < n; i++) t[0][x[i]]++;
    for (int j = 0; j < 256; j++) h[j] += t[0][j] + t[1][j] + t[2][j] + t[3][j];
}

__attribute__((noinline)) static void kernel(const uint8_t *x, size_t n, uint32_t *h) {
    memset(h, 0, 256 * sizeof(uint32_t));
    /* with -fopenmp: one contiguous chunk per thread, merged at the end */
#pragma omp parallel reduction(+ : h[:256])
    {
        size_t t = 1, k = 0;
#ifdef _OPENMP
        t = (size_t)omp_get_num_threads();
        k = (size_t)omp_get_thread_num();
#endif
        size_t lo = n * k / t, hi = n * (k + 1) / t;
        sub_histograms(x + lo, hi - lo, h);
    }
}
#endif

int main(int argc, char **argv) {
    if (argc != 4) die("usage:", "histogram <input.u8> <output.f64> <reps>");
    size_t n;
    uint8_t *x = read_u8(argv[1], &n);
    long reps = reps_arg(argv[3]);
    uint32_t h[256];
    uint64_t t0 = now_ns();
    for (long k = 0; k < reps; k++) {
        KEEP_PTR(x);
        kernel(x, n, h);
        KEEP_PTR(h);
    }
    uint64_t t1 = now_ns();
    printf("%llu\n", (unsigned long long)(t1 - t0));
    double out[256];
    for (int j = 0; j < 256; j++) out[j] = (double)h[j];
    write_f64(argv[2], out, 256);
    return 0;
}
