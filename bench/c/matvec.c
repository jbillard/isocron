/* matvec <matrix.f64> <vector.f64> <output.f64> <reps>
 * y = A v: cols = the length of v, rows = the length of A / cols, A stored
 * row after row; y[i] = dot(row i, v).
 * VARIANT 0 (idiomatic): for each row, a left-to-right sum of the products.
 * VARIANT 1 (optimized): for each row, the fixed form of the sum of the
 * products row[k] * v[k] (each product rounded, then added by 32
 * accumulators, then the tree), as dot.c: bit-identical to Isocron.
 * With -fopenmp both split the rows between the threads. */
#include "common.h"

#if VARIANT == 0
__attribute__((noinline)) static void kernel(const double *a, const double *v, double *y, size_t rows, size_t cols) {
#pragma omp parallel for
    for (size_t i = 0; i < rows; i++) {
        const double *row = a + i * cols;
        double s = 0.0;
        for (size_t k = 0; k < cols; k++) s += row[k] * v[k];
        y[i] = s;
    }
}
#else
struct ab { const double *a, *b; };

FIXED_SUM_OF(dot_fixed, GROUP2,
             const double *restrict a = ((const struct ab *)ctx)->a; const double *restrict b = ((const struct ab *)ctx)->b;,
             a[k] * b[k])

__attribute__((noinline)) static void kernel(const double *a, const double *v, double *y, size_t rows, size_t cols) {
#pragma omp parallel for schedule(static)
    for (size_t i = 0; i < rows; i++) {
        struct ab c = {a + i * cols, v};
        y[i] = dot_fixed(cols, &c);
    }
}
#endif

int main(int argc, char **argv) {
    if (argc != 5) die("usage:", "matvec <matrix.f64> <vector.f64> <output.f64> <reps>");
    size_t na, cols;
    double *a = read_f64(argv[1], &na);
    double *v = read_f64(argv[2], &cols);
    if (cols == 0) die("empty vector:", argv[2]);
    size_t rows = na / cols;
    double *y = aligned_alloc(64, (rows * sizeof(double) + 64) / 64 * 64);
    /* the pages of the output touched before the timer, in every language
       (DECISIONS D117); not zeros, which GCC may turn into calloc */
    memset(y, 0xff, (rows * sizeof(double) + 64) / 64 * 64);
    long reps = reps_arg(argv[4]);
    uint64_t t0 = now_ns();
    for (long k = 0; k < reps; k++) {
        KEEP_PTR(a);
        KEEP_PTR(v);
        KEEP_PTR(y);
        kernel(a, v, y, rows, cols);
        KEEP_PTR(y);
    }
    uint64_t t1 = now_ns();
    printf("%llu\n", (unsigned long long)(t1 - t0));
    write_f64(argv[3], y, rows);
    return 0;
}
