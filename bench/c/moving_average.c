/* moving_average <input.f64> <output.f64> <reps>
 * out[i] = sum(x[i .. i + 64]) / 64.0 for i = 0 .. n - 64 (n - 63 values).
 * VARIANT 0 (idiomatic): for each window, a left-to-right sum of its 64
 * elements.
 * VARIANT 1 (optimized): the fixed form of SPEC section 10 for each window
 * (V = 32, one block): acc[j] = (0.0 + w[j]) + w[j + 32] for j < 32, then
 * acc[j] = acc[j] + acc[j + d] for j < d, d = 16, 8, 4, 2, 1; acc[0] / 64.0.
 * Bit-identical to Isocron. The windows share these values: at every level
 * of the tree, acc[j] of window i depends only on the position i + j,
 *   L32(k) = (0.0 + x[k]) + x[k + 32]
 *   L16(k) = L32(k) + L32(k + 16)   ...   L1(k) = L2(k) + L2(k + 1)
 * and out[i] = L1(i) / 64.0, so each level is computed once per position
 * (6 additions per window instead of 63, each with the same operands as in
 * the per-window form). The levels are computed by tiles of 1024 windows
 * that stay in the L1 cache.
 * With -fopenmp both split the windows between the threads. */
#include "common.h"

#define WIN 64

#if VARIANT == 0
__attribute__((noinline)) static void kernel(const double *x, size_t n, double *out) {
    size_t m = n - (WIN - 1);
#pragma omp parallel for
    for (size_t i = 0; i < m; i++) {
        double s = 0.0;
        for (size_t j = 0; j < WIN; j++) s += x[i + j];
        out[i] = s / 64.0;
    }
}
#else
#define TILE 1024

/* windows x[i .. i + 64] for i < m (reads x[0 .. m + 63]) */
static void tile(const double *restrict x, double *restrict out, size_t m) {
    double a[TILE + 32], b[TILE + 32];
    for (size_t k = 0; k < m + 31; k++) a[k] = (0.0 + x[k]) + x[k + 32]; /* L32 */
    for (size_t k = 0; k < m + 15; k++) b[k] = a[k] + a[k + 16];         /* L16 */
    for (size_t k = 0; k < m + 7; k++) a[k] = b[k] + b[k + 8];           /* L8 */
    for (size_t k = 0; k < m + 3; k++) b[k] = a[k] + a[k + 4];           /* L4 */
    for (size_t k = 0; k < m + 1; k++) a[k] = b[k] + b[k + 2];           /* L2 */
    for (size_t k = 0; k < m; k++) out[k] = (a[k] + a[k + 1]) / 64.0;    /* L1 / 64 */
}

__attribute__((noinline)) static void kernel(const double *x, size_t n, double *out) {
    size_t m = n - (WIN - 1);
    size_t tiles = (m + TILE - 1) / TILE;
#pragma omp parallel for schedule(static)
    for (size_t t = 0; t < tiles; t++) {
        size_t lo = t * TILE;
        size_t len = m - lo < TILE ? m - lo : TILE;
        tile(x + lo, out + lo, len);
    }
}
#endif

int main(int argc, char **argv) {
    if (argc != 4) die("usage:", "moving_average <input.f64> <output.f64> <reps>");
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
