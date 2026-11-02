/* dot <a.f64> <b.f64> <output.f64> <reps>
 * VARIANT 0: sequential loop. VARIANT 1: fixed form (bit-identical to
 * Isocron): products are rounded, then added by V accumulators. */
#include "common.h"

struct ab { const double *a, *b; };

#if VARIANT == 0
__attribute__((noinline)) static double kernel(const double *a, const double *b, size_t n) {
    double s = 0.0;
#pragma omp parallel for reduction(+ : s)
    for (size_t i = 0; i < n; i++) s += a[i] * b[i];
    return s;
}
#else
FIXED_SUM_OF(dot_fixed, GROUP2,
             const double *restrict a = ((const struct ab *)ctx)->a; const double *restrict b = ((const struct ab *)ctx)->b;,
             a[k] * b[k])
__attribute__((noinline)) static double kernel(const double *a, const double *b, size_t n) {
    struct ab c = {a, b};
    return dot_fixed(n, &c);
}
#endif

int main(int argc, char **argv) {
    if (argc != 5) die("usage:", "dot <a.f64> <b.f64> <output.f64> <reps>");
    size_t n, m;
    double *a = read_f64(argv[1], &n);
    double *b = read_f64(argv[2], &m);
    if (n != m) die("size mismatch", "");
    long reps = reps_arg(argv[4]);
    double r = 0;
    uint64_t t0 = now_ns();
    for (long k = 0; k < reps; k++) {
        KEEP_PTR(a);
        KEEP_PTR(b);
        r = kernel(a, b, n);
        KEEP_F64(r);
    }
    uint64_t t1 = now_ns();
    printf("%llu\n", (unsigned long long)(t1 - t0));
    write_f64(argv[3], &r, 1);
    return 0;
}
