/* filter_sum <input.f64> <output.f64> <reps>
 * The sum of the elements > 0.0.
 * VARIANT 0 (idiomatic): one loop, `if (x[i] > 0.0) s += x[i]`.
 * VARIANT 1 (optimized): the fixed form of SPEC section 10 applied to the
 * kept elements in their order, as fixed_sum_mem would on the compacted
 * array (bit-identical to Isocron): the k-th kept element goes to
 * accumulator k mod 32 of block k / 4096 (blocks of 4096 kept elements),
 * each block is reduced by the tree, the block results by fixed_sum_mem.
 * The kept elements are compacted 1024 input elements at a time into a small
 * buffer (AVX-512 vcompresspd when available, else a branchless loop), and
 * the buffer is added 32 elements at a time to the 32 accumulators.
 * With -fopenmp: x is split into one chunk per thread; a first pass counts
 * the kept elements of each chunk, so that each chunk knows the index of its
 * first kept element. A chunk owns the blocks that start in its range of
 * kept indices: it skips the kept elements of a block started by a previous
 * chunk and reads past its end to finish its last block. Every block is
 * thus computed exactly as in one thread, whatever the number of threads. */
#include "common.h"
#ifdef _OPENMP
#include <omp.h>
#endif
#if VARIANT == 1 && defined(__AVX512F__)
#include <immintrin.h>
#endif

#if VARIANT == 0
__attribute__((noinline)) static double kernel(const double *x, size_t n, double *p, size_t *cnt) {
    (void)p;
    (void)cnt;
    double s = 0.0;
#pragma omp parallel for reduction(+ : s)
    for (size_t i = 0; i < n; i++)
        if (x[i] > 0.0) s += x[i];
    return s;
}
#else
#define SUB 1024 /* input elements compacted at a time */

/* Copies the elements > 0.0 of x[0 .. m] to buf, in order; returns their
   count. buf must have room for m elements. */
static size_t compact(const double *restrict x, size_t m, double *restrict buf) {
    size_t c = 0, i = 0;
#ifdef __AVX512F__
    const __m512d zero = _mm512_setzero_pd();
    for (; i + 8 <= m; i += 8) {
        __m512d v = _mm512_loadu_pd(x + i);
        __mmask8 k = _mm512_cmp_pd_mask(v, zero, _CMP_GT_OQ);
        _mm512_storeu_pd(buf + c, _mm512_maskz_compress_pd(k, v));
        c += (size_t)__builtin_popcount(k);
    }
#endif
    for (; i < m; i++) {
        buf[c] = x[i];
        c += x[i] > 0.0;
    }
    return c;
}

static size_t count_kept(const double *restrict x, size_t m) {
    size_t c = 0;
    for (size_t i = 0; i < m; i++) c += x[i] > 0.0;
    return c;
}

/* The tree of the fixed form, as in block_sum_mem. */
static double tree(double *acc) {
    for (int d = LANES / 2; d >= 1; d /= 2)
        for (int j = 0; j < d; j++) acc[j] += acc[j + d];
    return acc[0];
}

/* acc[j] += b[i * LANES + j] for i < g: element i * LANES + j goes to
   accumulator j, in increasing order of i. */
static void add_groups(double *restrict acc, const double *restrict b, size_t g) {
    double a[LANES];
    for (int j = 0; j < LANES; j++) a[j] = acc[j];
    for (size_t i = 0; i < g; i++)
        for (int j = 0; j < LANES; j++) a[j] += b[i * LANES + j];
    for (int j = 0; j < LANES; j++) acc[j] = a[j];
}

/* The blocks of the kept elements of indices [lo_k, hi_k): lo_k is a
   multiple of BLOCK, hi_k is a multiple of BLOCK or the number of kept
   elements (SIZE_MAX: up to the end of x). The reading starts at x[pos],
   whose kept elements have indices from kpos <= lo_k. Writes the result of
   block b into p[b]; returns the index of the block after the last one. */
static size_t run(const double *x, size_t n, size_t pos, size_t kpos, size_t lo_k, size_t hi_k, double *p) {
    double buf[LANES + SUB + 8];
    size_t len = 0; /* kept elements in buf, of indices kpos .. kpos + len */
    /* skip the kept elements of the block started before lo_k */
    while (kpos < lo_k && pos < n) {
        size_t m = n - pos < SUB ? n - pos : SUB;
        size_t c = compact(x + pos, m, buf);
        pos += m;
        if (kpos + c <= lo_k) {
            kpos += c;
            continue;
        }
        size_t drop = lo_k - kpos;
        len = c - drop;
        memmove(buf, buf + drop, len * sizeof(double));
        kpos = lo_k;
    }
    double acc[LANES];
    for (int j = 0; j < LANES; j++) acc[j] = 0.0;
    size_t blk = lo_k / BLOCK, inblk = 0;
    for (;;) {
        /* add the full groups of LANES elements below hi_k, block by block */
        size_t avail = len < hi_k - kpos ? len : hi_k - kpos;
        size_t g = avail / LANES, used = 0;
        while (g > 0) {
            size_t room = (BLOCK - inblk) / LANES;
            size_t h = g < room ? g : room;
            add_groups(acc, buf + used, h);
            used += h * LANES;
            inblk += h * LANES;
            g -= h;
            if (inblk == BLOCK) {
                p[blk++] = tree(acc);
                for (int j = 0; j < LANES; j++) acc[j] = 0.0;
                inblk = 0;
            }
        }
        kpos += used;
        len -= used;
        memmove(buf, buf + used, len * sizeof(double));
        if (len >= hi_k - kpos || pos == n) break;
        size_t m = n - pos < SUB ? n - pos : SUB;
        len += compact(x + pos, m, buf + len);
        pos += m;
    }
    /* the last, partial block: fewer than LANES elements left, which go to
       accumulators 0, 1, ... (blocks start at multiples of LANES) */
    size_t r = len < hi_k - kpos ? len : hi_k - kpos;
    if (r > 0 || inblk > 0) {
        for (size_t j = 0; j < r; j++) acc[j] += buf[j];
        p[blk++] = tree(acc);
    }
    return blk;
}

/* p: room for n / BLOCK + 1 block results; cnt: room for threads + 1 */
__attribute__((noinline)) static double kernel(const double *x, size_t n, double *p, size_t *cnt) {
    size_t blocks;
    int threads = 1;
#ifdef _OPENMP
    threads = omp_get_max_threads();
#endif
    if (threads == 1) {
        blocks = run(x, n, 0, 0, 0, SIZE_MAX, p);
    } else {
        size_t team = 1;
#pragma omp parallel
        {
            size_t t = 1, k = 0;
#ifdef _OPENMP
            t = (size_t)omp_get_num_threads();
            k = (size_t)omp_get_thread_num();
#endif
            size_t lo = n * k / t, hi = n * (k + 1) / t;
            cnt[k + 1] = count_kept(x + lo, hi - lo);
#pragma omp barrier
#pragma omp single
            {
                team = t;
                cnt[0] = 0;
                for (size_t c = 1; c <= t; c++) cnt[c] += cnt[c - 1];
            }
            /* kept indices of this chunk: [cnt[k], cnt[k + 1]); it owns the
               blocks that start there */
            size_t total = cnt[t];
            size_t own_lo = (cnt[k] + BLOCK - 1) / BLOCK * BLOCK;
            size_t own_hi = (cnt[k + 1] + BLOCK - 1) / BLOCK * BLOCK;
            if (own_hi > total) own_hi = total;
            if (own_lo < own_hi) run(x, n, lo, cnt[k], own_lo, own_hi, p);
        }
        blocks = (cnt[team] + BLOCK - 1) / BLOCK;
    }
    if (blocks == 0) return 0.0; /* nothing kept: the tree of 32 zeros */
    if (blocks == 1) return p[0];
    return fixed_sum_mem(p, blocks);
}
#endif

int main(int argc, char **argv) {
    if (argc != 4) die("usage:", "filter_sum <input.f64> <output.f64> <reps>");
    size_t n;
    double *x = read_f64(argv[1], &n);
    long reps = reps_arg(argv[3]);
    int threads = 1;
#ifdef _OPENMP
    threads = omp_get_max_threads();
#endif
    /* work space of the optimized variant, allocated once */
    double *p = malloc((n / BLOCK + 1) * sizeof(double));
    size_t *cnt = malloc(((size_t)threads + 1) * sizeof(size_t));
    double r = 0;
    uint64_t t0 = now_ns();
    for (long k = 0; k < reps; k++) {
        KEEP_PTR(x);
        r = kernel(x, n, p, cnt);
        KEEP_F64(r);
    }
    uint64_t t1 = now_ns();
    printf("%llu\n", (unsigned long long)(t1 - t0));
    write_f64(argv[2], &r, 1);
    return 0;
}
