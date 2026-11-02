/* prefix_sum <input.f64> <output.f64> <reps>
 * The inclusive prefix sums of i64 (wrapping): the input holds integers
 * stored as f64, converted to int64 before the timed loop; the n sums are
 * written as f64 after it. Integer sums: both variants are exact.
 * VARIANT 0 (idiomatic): one loop `s += x[i]; out[i] = s`; with -fopenmp,
 * the OpenMP 5.0 scan (`reduction(inscan, +)` and `#pragma omp scan`).
 * VARIANT 1 (optimized): AVX-512 when available: the prefix sums of 8
 * elements in a register (3 shifts and adds), plus the carry broadcast from
 * the previous vector. With -fopenmp: a two-pass blocked scan, one chunk per
 * thread: the sum of each chunk, then each chunk scanned from the sum of the
 * chunks before it. */
#include "common.h"
#ifdef _OPENMP
#include <omp.h>
#endif
#if VARIANT == 1 && defined(__AVX512F__)
#include <immintrin.h>
#endif

#if VARIANT == 0
__attribute__((noinline)) static void kernel(const int64_t *x, int64_t *out, size_t n) {
    uint64_t s = 0;
    /* a signed index: with a size_t index and n = 0, GCC 14.2 -fopenmp
       writes out of bounds in its lowering of the inscan reduction */
#pragma omp parallel for reduction(inscan, + : s)
    for (int64_t i = 0; i < (int64_t)n; i++) {
        s += (uint64_t)x[i];
#pragma omp scan inclusive(s)
        out[i] = (int64_t)s;
    }
}
#else
/* out[i] = carry + x[0] + ... + x[i] */
static void scan_chunk(const int64_t *restrict x, int64_t *restrict out, size_t n, uint64_t carry) {
    size_t i = 0;
#ifdef __AVX512F__
    const __m512i zero = _mm512_setzero_si512();
    const __m512i last = _mm512_set1_epi64(7);
    __m512i c = _mm512_set1_epi64((long long)carry);
    for (; i + 8 <= n; i += 8) {
        __m512i v = _mm512_loadu_si512(x + i);
        v = _mm512_add_epi64(v, _mm512_alignr_epi64(v, zero, 7)); /* + lane i - 1 */
        v = _mm512_add_epi64(v, _mm512_alignr_epi64(v, zero, 6)); /* + lane i - 2 */
        v = _mm512_add_epi64(v, _mm512_alignr_epi64(v, zero, 4)); /* + lane i - 4 */
        v = _mm512_add_epi64(v, c);
        _mm512_storeu_si512(out + i, v);
        c = _mm512_permutexvar_epi64(last, v);
    }
    if (i > 0) carry = (uint64_t)out[i - 1];
#endif
    for (; i < n; i++) {
        carry += (uint64_t)x[i];
        out[i] = (int64_t)carry;
    }
}

static uint64_t chunk_sum(const int64_t *restrict x, size_t n) {
    uint64_t s = 0;
    for (size_t i = 0; i < n; i++) s += (uint64_t)x[i];
    return s;
}

/* sums: room for threads + 1 */
__attribute__((noinline)) static void kernel(const int64_t *x, int64_t *out, size_t n, uint64_t *sums) {
#pragma omp parallel
    {
        size_t t = 1, k = 0;
#ifdef _OPENMP
        t = (size_t)omp_get_num_threads();
        k = (size_t)omp_get_thread_num();
#endif
        size_t lo = n * k / t, hi = n * (k + 1) / t;
        uint64_t carry = 0;
        if (t > 1) {
            sums[k] = chunk_sum(x + lo, hi - lo);
#pragma omp barrier
            for (size_t c = 0; c < k; c++) carry += sums[c];
        }
        scan_chunk(x + lo, out + lo, hi - lo, carry);
    }
}
#endif

int main(int argc, char **argv) {
    if (argc != 4) die("usage:", "prefix_sum <input.f64> <output.f64> <reps>");
    size_t n;
    double *xf = read_f64(argv[1], &n);
    size_t bytes = (n * sizeof(int64_t) + 64) / 64 * 64;
    int64_t *x = aligned_alloc(64, bytes);
    int64_t *out = aligned_alloc(64, bytes);
    /* the pages of the output touched before the timer, in every language
       (DECISIONS D117); not zeros, which GCC may turn into calloc */
    memset(out, 0xff, bytes);
    for (size_t i = 0; i < n; i++) x[i] = (int64_t)xf[i];
    long reps = reps_arg(argv[3]);
#if VARIANT == 1
    int threads = 1;
#ifdef _OPENMP
    threads = omp_get_max_threads();
#endif
    uint64_t *sums = malloc(((size_t)threads + 1) * sizeof(uint64_t));
#endif
    uint64_t t0 = now_ns();
    for (long k = 0; k < reps; k++) {
        KEEP_PTR(x);
        KEEP_PTR(out);
#if VARIANT == 0
        kernel(x, out, n);
#else
        kernel(x, out, n, sums);
#endif
        KEEP_PTR(out);
    }
    uint64_t t1 = now_ns();
    printf("%llu\n", (unsigned long long)(t1 - t0));
    for (size_t i = 0; i < n; i++) xf[i] = (double)out[i];
    write_f64(argv[2], xf, n);
    return 0;
}
