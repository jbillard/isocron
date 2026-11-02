/* std_dev <input.f64> <output.f64> <reps>
 * Two passes: mean, then mean of squared deviations; sqrt.
 * VARIANT 0: sequential loops. VARIANT 1: fixed form (bit-identical). */
#include "common.h"

struct xm { const double *x; double m; };

#if VARIANT == 0
__attribute__((noinline)) static double kernel(const double *x, size_t n) {
    double s = 0.0;
#pragma omp parallel for reduction(+ : s)
    for (size_t i = 0; i < n; i++) s += x[i];
    double m = s / (double)n;
    double q = 0.0;
#pragma omp parallel for reduction(+ : q)
    for (size_t i = 0; i < n; i++) {
        double d = x[i] - m;
        q += d * d;
    }
    return sqrt(q / (double)n);
}
#else
FIXED_SUM_OF(sq_fixed, GROUP1,
             const double *restrict x = ((const struct xm *)ctx)->x; const double mu = ((const struct xm *)ctx)->m;,
             (x[k] - mu) * (x[k] - mu))
__attribute__((noinline)) static double kernel(const double *x, size_t n) {
    double m = fixed_sum_mem(x, n) / (double)n;
    struct xm c = {x, m};
    return sqrt(sq_fixed(n, &c) / (double)n);
}
#endif

int main(int argc, char **argv) {
    if (argc != 4) die("usage:", "std_dev <input.f64> <output.f64> <reps>");
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
