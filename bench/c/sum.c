/* sum <input.f64> <output.f64> <reps>
 * VARIANT 0 (idiomatic): one sequential loop.
 * VARIANT 1 (optimized): V = 32 accumulators, blocks of 4096 elements: the fixed
 * form of SPEC section 10, bit-identical to Isocron.
 * With -fopenmp both run in parallel (OpenMP reduction / blocks in parallel). */
#include "common.h"

#if VARIANT == 0
__attribute__((noinline)) static double kernel(const double *x, size_t n) {
    double s = 0.0;
#pragma omp parallel for reduction(+ : s)
    for (size_t i = 0; i < n; i++) s += x[i];
    return s;
}
#else
__attribute__((noinline)) static double kernel(const double *x, size_t n) {
    return fixed_sum_mem(x, n);
}
#endif

int main(int argc, char **argv) {
    if (argc != 4) die("usage:", "sum <input.f64> <output.f64> <reps>");
    size_t n;
    double *x = read_f64(argv[1], &n);
    long reps = reps_arg(argv[3]);
    double r = 0;
    uint64_t t0 = now_ns();
    for (long k = 0; k < reps; k++) {
        KEEP_PTR(x);
        r = kernel(x, n);
        KEEP_F64(r);
    }
    uint64_t t1 = now_ns();
    printf("%llu\n", (unsigned long long)(t1 - t0));
    write_f64(argv[2], &r, 1);
    return 0;
}
