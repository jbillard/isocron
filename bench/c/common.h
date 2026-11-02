/* Shared I/O, timing and the fixed reduction form of SPEC section 10.
 * Every program follows the interface of the Isocron POC programs:
 *   <kernel> <inputs...> <output.f64> <reps>
 * It times `reps` calls of the kernel only, prints the total in nanoseconds
 * and writes the result as little-endian f64 (canonical NaN). */
#ifndef ISC_BENCH_COMMON_H
#define ISC_BENCH_COMMON_H
#pragma GCC diagnostic ignored "-Wunused-function"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <math.h>

/* V of the fixed form; -DLANES=... for the experiments on V */
#ifndef LANES
#define LANES 32
#endif
#define BLOCK 4096

/* Accumulators per pass over a block (speed only, never results), the rule
   of Isocron's code generator: the accumulators of one pass, plus one
   register per array read (one less with AVX, which folds a load into the
   addition), must fit in the vector registers. GROUP1: one array read per
   element, GROUP2: two. Without AVX: 16 xmm registers of 2 doubles; with
   AVX2: 16 ymm of 4; with AVX-512: 32 registers, no pass needed up to
   V = 64. Each accumulator still receives its elements in increasing order. */
#if !defined(__AVX__)
#define GROUP_MAX1 16
#define GROUP_MAX2 16
#elif !defined(__AVX512F__)
#define GROUP_MAX1 64
#define GROUP_MAX2 32
#else
#define GROUP_MAX1 64
#define GROUP_MAX2 64
#endif
#define GROUP1 (LANES < GROUP_MAX1 ? LANES : GROUP_MAX1)
#define GROUP2 (LANES < GROUP_MAX2 ? LANES : GROUP_MAX2)

/* Keep the compiler from merging, hoisting or deleting the timed calls. */
#define CLOBBER() __asm__ volatile("" : : : "memory")
#define KEEP_PTR(p) __asm__ volatile("" : : "r"(p) : "memory")
#define KEEP_F64(x) __asm__ volatile("" : "+x"(x))

static void die(const char *m, const char *p) {
    fprintf(stderr, "error: %s %s\n", m, p);
    exit(1);
}

static void *read_file(const char *path, size_t *bytes) {
    FILE *f = fopen(path, "rb");
    if (!f) die("cannot read", path);
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    void *p = aligned_alloc(64, ((size_t)n + 64) / 64 * 64);
    if (n > 0 && fread(p, 1, (size_t)n, f) != (size_t)n) die("cannot read", path);
    fclose(f);
    *bytes = (size_t)n;
    return p;
}

static double *read_f64(const char *path, size_t *n) {
    size_t b;
    double *p = read_file(path, &b);
    *n = b / 8;
    return p;
}

static uint8_t *read_u8(const char *path, size_t *n) {
    return read_file(path, n);
}

static void write_f64(const char *path, const double *x, size_t n) {
    FILE *f = fopen(path, "wb");
    if (!f) die("cannot write", path);
    for (size_t i = 0; i < n; i++) {
        double v = x[i];
        uint64_t bits;
        memcpy(&bits, &v, 8);
        if (v != v) bits = 0x7FF8000000000000ull;
        fwrite(&bits, 8, 1, f);
    }
    fclose(f);
}

static uint64_t now_ns(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (uint64_t)t.tv_sec * 1000000000ull + (uint64_t)t.tv_nsec;
}

static long reps_arg(const char *s) {
    long r = strtol(s, NULL, 10);
    return r < 1 ? 1 : r;
}

/* Upper levels of the fixed form: reduce block results with `+`. */
static double fixed_sum_mem(const double *x, size_t n);

static double block_sum_mem(const double *x, size_t m) {
    double accs[LANES];
    size_t full = m / LANES * LANES;
    for (int p = 0; p < LANES / GROUP1; p++) {
        double acc[GROUP1];
        for (int j = 0; j < GROUP1; j++) acc[j] = 0.0;
        for (size_t i = 0; i < full; i += LANES)
            for (int j = 0; j < GROUP1; j++) acc[j] += x[i + p * GROUP1 + j];
        for (int j = 0; j < GROUP1; j++) accs[p * GROUP1 + j] = acc[j];
    }
    for (size_t i = full, j = 0; i < m; i++, j++) accs[j] += x[i];
    for (int d = LANES / 2; d >= 1; d /= 2)
        for (int j = 0; j < d; j++) accs[j] += accs[j + d];
    return accs[0];
}

static double fixed_sum_mem(const double *x, size_t n) {
    if (n <= BLOCK) return block_sum_mem(x, n);
    size_t c = (n + BLOCK - 1) / BLOCK;
    double *p = malloc(c * sizeof(double));
    /* with -fopenmp: blocks in parallel, each result in its own slot, so the
       result does not depend on the number of threads */
#pragma omp parallel for schedule(static)
    for (size_t j = 0; j < c; j++) {
        size_t m = n - j * BLOCK < BLOCK ? n - j * BLOCK : BLOCK;
        p[j] = block_sum_mem(x + j * BLOCK, m);
    }
    double r = fixed_sum_mem(p, c);
    free(p);
    return r;
}

/* Fixed form where the first level computes f(i) for element i. PREP runs
   once per block (to load the context into local restrict pointers, so that
   the compiler can vectorize ELEM); GROUP is GROUP1 or GROUP2. */
#define FIXED_SUM_OF(name, GROUP, PREP, ELEM)                                   \
    static double name##_block_impl(size_t start, size_t m, const void *ctx);  \
    static double name(size_t n, const void *ctx) {                            \
        size_t c = n <= BLOCK ? 1 : (n + BLOCK - 1) / BLOCK;                   \
        if (c == 1) return name##_block_impl(0, n, ctx);                       \
        double *p = malloc(c * sizeof(double));                                \
        _Pragma("omp parallel for schedule(static)")                           \
        for (size_t j = 0; j < c; j++) {                                       \
            size_t m = n - j * BLOCK < BLOCK ? n - j * BLOCK : BLOCK;          \
            p[j] = name##_block_impl(j * BLOCK, m, ctx);                       \
        }                                                                      \
        double r = fixed_sum_mem(p, c);                                        \
        free(p);                                                               \
        return r;                                                              \
    }                                                                          \
    static double name##_block_impl(size_t start, size_t m, const void *ctx) { \
        PREP                                                                   \
        double accs[LANES];                                                    \
        size_t full = m / LANES * LANES;                                       \
        for (int p = 0; p < LANES / (GROUP); p++) {                            \
            double acc[GROUP];                                                 \
            for (int j = 0; j < (GROUP); j++) acc[j] = 0.0;                    \
            for (size_t i = 0; i < full; i += LANES)                           \
                for (int j = 0; j < (GROUP); j++) {                            \
                    size_t k = start + i + p * (GROUP) + j;                    \
                    acc[j] += ELEM;                                            \
                }                                                              \
            for (int j = 0; j < (GROUP); j++) accs[p * (GROUP) + j] = acc[j];  \
        }                                                                      \
        for (size_t i = full, j = 0; i < m; i++, j++) {                        \
            size_t k = start + i;                                              \
            accs[j] += ELEM;                                                   \
        }                                                                      \
        for (int d = LANES / 2; d >= 1; d /= 2)                                \
            for (int j = 0; j < d; j++) accs[j] += accs[j + d];                \
        return accs[0];                                                        \
    }

#endif
