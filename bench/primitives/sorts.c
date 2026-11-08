// The floor of `sorts.isc` (D488): the same work in C, the cheapest way
// known, on one thread: integers and floats by a stable sort of their
// bytes (radix, least significant first, as many bytes as their range
// needs), texts by a quicksort of their bytes one place at a time
// (Bentley and Sedgewick), a key of few values by counting, a reversed
// copy. Each line: a name, the nanoseconds of one element (the mean over
// `rounds`), a checksum of the order.
//   sorts <rounds>
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define N 4096

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e9 + t.tv_nsec;
}

// the values weighted by their places modulo 8
static long placed(const long *y, int n) {
    long a = 0;
    for (int i = 0; i < n; i++) a += y[i] * (i % 8);
    return a;
}

// the `n` keys of 64 bits sorted (the order of unsigned integers): by their
// bytes from the lowest, the bytes above the range of the keys skipped, a
// byte that every key shares skipped; the keys alone (a number is its
// key), the result in `k`
static void radix(uint64_t *k, uint64_t *k2, int n) {
    uint64_t lo = k[0], hi = k[0];
    for (int i = 1; i < n; i++) {
        lo = k[i] < lo ? k[i] : lo;
        hi = k[i] > hi ? k[i] : hi;
    }
    uint64_t range = hi - lo;
    int passes = 0;
    while (passes < 8 && (range >> (8 * passes)) != 0) passes++;
    static int count[8][256];
    memset(count, 0, sizeof(int) * 256 * (size_t)passes);
    for (int i = 0; i < n; i++) {
        uint64_t d = k[i] - lo;
        for (int b = 0; b < passes; b++) count[b][(d >> (8 * b)) & 255]++;
    }
    uint64_t *src = k, *dst = k2;
    for (int b = 0; b < passes; b++) {
        int s = 0, one = 0;
        for (int j = 0; j < 256; j++) {
            int c = count[b][j];
            one |= c == n;
            count[b][j] = s;
            s += c;
        }
        if (one) continue;
        for (int i = 0; i < n; i++) dst[count[b][((src[i] - lo) >> (8 * b)) & 255]++] = src[i];
        uint64_t *t = src;
        src = dst;
        dst = t;
    }
    if (src != k) memcpy(k, src, sizeof(uint64_t) * (size_t)n);
}

typedef struct {
    const unsigned char *p;
    int n;
} Str;

static inline int at(const Str *s, int d) { return d < s->n ? s->p[d] : -1; }

static inline int cmp_from(const Str *a, const Str *b, int d) {
    int m = (a->n < b->n ? a->n : b->n) - d;
    int c = m > 0 ? memcmp(a->p + d, b->p + d, (size_t)m) : 0;
    return c ? c : a->n - b->n;
}

// the texts in the order of their bytes, all equal before the place `d`
static void mkqs(Str *a, int n, int d) {
    while (n > 1) {
        if (n < 12) {
            for (int i = 1; i < n; i++)
                for (int j = i; j > 0 && cmp_from(&a[j - 1], &a[j], d) > 0; j--) {
                    Str t = a[j];
                    a[j] = a[j - 1];
                    a[j - 1] = t;
                }
            return;
        }
        Str t = a[0];
        a[0] = a[n / 2];
        a[n / 2] = t;
        int v = at(&a[0], d), lt = 0, i = 1, gt = n - 1;
        while (i <= gt) {
            int c = at(&a[i], d);
            if (c < v) {
                t = a[lt];
                a[lt++] = a[i];
                a[i++] = t;
            } else if (c > v) {
                t = a[i];
                a[i] = a[gt];
                a[gt--] = t;
            } else {
                i++;
            }
        }
        mkqs(a, lt, d);
        if (v >= 0) mkqs(a + lt, gt - lt + 1, d + 1);
        a += gt + 1;
        n -= gt + 1;
    }
}

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    static long xs[N], ys[N];
    static double fs[N], gs[N];
    static char text[N][16];
    static Str ws[N], sorted[N];
    static uint64_t k1[N], k2[N];
    for (long i = 0; i < N; i++) {
        xs[i] = (i * 7919 * 104729) % 1000000007 - 500000000;
        fs[i] = (double)xs[i] / 7.0;
        ws[i].n = snprintf(text[i], 16, "%ld", xs[i] % 1000003);
        ws[i].p = (const unsigned char *)text[i];
    }
    double each = (double)r * N;
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < r; k++) {
        for (int i = 0; i < N; i++) k1[i] = (uint64_t)xs[i] ^ (1ULL << 63);
        radix(k1, k2, N);
        for (int i = 0; i < N; i++) ys[i] = (long)(k1[i] ^ (1ULL << 63));
        acc += placed(ys, N);
    }
    printf("sort.i64 %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        for (int i = 0; i < N; i++) {
            uint64_t b;
            memcpy(&b, &fs[i], 8);
            // the total order of doubles (-0 before +0) as unsigned integers
            k1[i] = b >> 63 ? ~b : b ^ (1ULL << 63);
        }
        radix(k1, k2, N);
        for (int i = 0; i < N; i++) {
            // back from the key: a key with its top bit set was a positive
            uint64_t b = k1[i] >> 63 ? k1[i] ^ (1ULL << 63) : ~k1[i];
            memcpy(&gs[i], &b, 8);
            ys[i] = (long)gs[i];
        }
        acc += placed(ys, N);
    }
    printf("sort.f64 %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        memcpy(sorted, ws, sizeof(ws));
        mkqs(sorted, N, 0);
        for (int i = 0; i < N; i++) ys[i] = sorted[i].p[sorted[i].n - 1];
        acc += placed(ys, N);
    }
    printf("sort.string %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        // a stable sort by counting: the lengths are below 16
        int count[17] = {0};
        for (int i = 0; i < N; i++) count[ws[i].n + 1]++;
        for (int j = 1; j < 17; j++) count[j] += count[j - 1];
        for (int i = 0; i < N; i++) sorted[count[ws[i].n]++] = ws[i];
        for (int i = 0; i < N; i++) ys[i] = sorted[i].p[0];
        acc += placed(ys, N);
    }
    printf("sortBy.length %.3f %ld\n", (now() - t0) / each, acc / r);
    // with the representation of Isocron (`ISC_SORTS_OWNED=1`, D488): each
    // text of the sorted array in a block of its own, copied, the blocks
    // freed with the array (a text of an array owns its bytes)
    if (getenv("ISC_SORTS_OWNED")) {
        static char *own[N];
        for (int pass = 0; pass < 2; pass++) {
            acc = 0;
            t0 = now();
            for (long k = 0; k < r; k++) {
                if (pass == 0) {
                    memcpy(sorted, ws, sizeof(ws));
                    mkqs(sorted, N, 0);
                } else {
                    int count[17] = {0};
                    for (int i = 0; i < N; i++) count[ws[i].n + 1]++;
                    for (int j = 1; j < 17; j++) count[j] += count[j - 1];
                    for (int i = 0; i < N; i++) sorted[count[ws[i].n]++] = ws[i];
                }
                for (int i = 0; i < N; i++) {
                    own[i] = malloc((size_t)sorted[i].n);
                    memcpy(own[i], sorted[i].p, (size_t)sorted[i].n);
                }
                for (int i = 0; i < N; i++) ys[i] = (unsigned char)own[i][pass == 0 ? sorted[i].n - 1 : 0];
                acc += placed(ys, N);
                for (int i = 0; i < N; i++) free(own[i]);
            }
            printf("%s.owned %.3f %ld\n", pass == 0 ? "sort.string" : "sortBy.length", (now() - t0) / each, acc / r);
        }
    }
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        for (int i = 0; i < N; i++) ys[i] = xs[N - 1 - i];
        acc += placed(ys, N);
    }
    printf("toReversed %.3f %ld\n", (now() - t0) / each, acc / r);
    return 0;
}
