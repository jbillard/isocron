// The floor of `arrays.isc` (D495): the same work in C, the cheapest way
// known, on one thread: sums in 32 lanes (4 vectors of 8, as Isocron: a
// loop of GCC sums in one vector and waits for it), a prefix sum within
// vectors of 8 and a carry, stores of whole vectors into arrays aligned on
// 64 bytes (first floors of GCC's loops were beaten by Isocron). Each
// line: a name, the nanoseconds of one element (the mean over `rounds`), a
// checksum.
//   arrays <rounds>
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <immintrin.h>

typedef long v8 __attribute__((vector_size(64)));

static inline v8 load8(const long *p) {
    v8 v;
    memcpy(&v, p, 64);
    return v;
}

static inline long hsum(v8 a, v8 b, v8 c, v8 d) {
    v8 s = a + b + c + d;
    long t = 0;
    for (int j = 0; j < 8; j++) t += s[j];
    return t;
}

#define N 4096
// the bytes written into `p` kept (D456)
#define KEEP(p) __asm__ volatile("" : : "r"(p) : "memory")

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e9 + t.tv_nsec;
}

int main(int argc, char **argv) {
    // 25 times the rounds asked, as `arrays.isc`
    long r = (argc > 1 ? atol(argv[1]) : 10) * 25;
    static long xs[N] __attribute__((aligned(64))), ys[N] __attribute__((aligned(64))), zs[2 * N] __attribute__((aligned(64)));
    for (long i = 0; i < N; i++) {
        xs[i] = (i * 7919 * 104729) % 1000000007 - 500000000;
        ys[i] = (i * 104729) % 1000003;
    }
    double each = (double)r * N;
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < r; k++) {
        v8 a = {0}, b = {0}, c = {0}, d = {0};
        for (int i = 0; i < N; i += 32) {
            v8 x0 = load8(xs + i), x1 = load8(xs + i + 8), x2 = load8(xs + i + 16), x3 = load8(xs + i + 24);
            // x + x, not x << 1: a shift of 512 bits has one port, an
            // addition two
            a += (x0 + x0) + x0 + 1;
            b += (x1 + x1) + x1 + 1;
            c += (x2 + x2) + x2 + 1;
            d += (x3 + x3) + x3 + 1;
        }
        acc += hsum(a, b, c, d);
        KEEP(xs);
    }
    printf("map.sum %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        for (int i = 0; i < N; i += 32) {
            v8 x0 = load8(xs + i), x1 = load8(xs + i + 8), x2 = load8(xs + i + 16), x3 = load8(xs + i + 24);
            v8 a = (x0 + x0) + x0 + 1, b = (x1 + x1) + x1 + 1, c = (x2 + x2) + x2 + 1, d = (x3 + x3) + x3 + 1;
            memcpy(zs + i, &a, 64);
            memcpy(zs + i + 8, &b, 64);
            memcpy(zs + i + 16, &c, 64);
            memcpy(zs + i + 24, &d, 64);
        }
        KEEP(zs);
        acc += zs[17] + zs[N - 1] + N;
    }
    printf("map.new %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        long c = 0;
        for (int i = 0; i < N; i++) c += xs[i] % 3 == 0;
        acc += c;
        KEEP(xs);
    }
    printf("filter.count %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        int m = 0;
        for (int i = 0; i < N; i++) {
            zs[m] = xs[i];
            m += xs[i] % 3 == 0;
        }
        KEEP(zs);
        long s = 0;
        for (int i = 0; i < m; i++) s += zs[i];
        acc += s;
    }
    printf("filter.sum %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        for (int i = 0; i < N; i += 32)
            for (int j = 0; j < 32; j += 8) {
                v8 a = load8(xs + i + j), b = load8(ys + i + j);
                memcpy(zs + i + j, &a, 64);
                memcpy(zs + N + i + j, &b, 64);
            }
        KEEP(zs);
        acc += zs[5] + zs[N + 7] + 2 * N;
    }
    printf("concat %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        v8 a = {0}, b = {0}, c = {0}, d = {0};
        for (int i = 0; i < N; i += 32) {
            a += load8(xs + i) ^ load8(ys + i);
            b += load8(xs + i + 8) ^ load8(ys + i + 8);
            c += load8(xs + i + 16) ^ load8(ys + i + 16);
            d += load8(xs + i + 24) ^ load8(ys + i + 24);
        }
        acc += hsum(a, b, c, d);
        KEEP(xs);
    }
    printf("zip.sum %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        long a = 0;
        for (int i = 0; i < N; i++) a = (a ^ xs[i]) * 31 % 1000000007;
        acc += a;
        KEEP(xs);
    }
    printf("fold %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    // the prefix sums of each vector of 8 (3 shifts by lanes), apart from
    // the vectors before it; its total broadcast from that (not from the
    // carry); then the carry added, and the total added to the carry: the
    // chain from one vector to the next one addition of vectors
    const __m512i z = _mm512_setzero_si512();
    const __m512i last = _mm512_set1_epi64(7);
    for (long k = 0; k < r; k++) {
        __m512i carry = z;
        for (int i = 0; i < N; i += 8) {
            __m512i v = _mm512_load_si512((const void *)(ys + i));
            v = _mm512_add_epi64(v, _mm512_alignr_epi64(v, z, 7));
            v = _mm512_add_epi64(v, _mm512_alignr_epi64(v, z, 6));
            v = _mm512_add_epi64(v, _mm512_alignr_epi64(v, z, 4));
            __m512i total = _mm512_permutexvar_epi64(last, v);
            _mm512_store_si512((void *)(zs + i), _mm512_add_epi64(v, carry));
            carry = _mm512_add_epi64(carry, total);
        }
        KEEP(zs);
        acc += zs[N - 1] + zs[N / 2];
    }
    printf("scan %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        v8 f = {r, r, r, r, r, r, r, r};
        for (int i = 0; i < N; i += 32) {
            memcpy(zs + i, &f, 64);
            memcpy(zs + i + 8, &f, 64);
            memcpy(zs + i + 16, &f, 64);
            memcpy(zs + i + 24, &f, 64);
        }
        KEEP(zs);
        acc += zs[3] + zs[N - 1] + N;
    }
    printf("fill %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        // 100 .. 4096 - 100: 3896 elements, 121 runs of 32 and 24 more
        v8 a = {0}, b = {0}, c = {0}, d = {0};
        int i = 100;
        for (; i + 32 <= N - 100; i += 32) {
            a += load8(xs + i);
            b += load8(xs + i + 8);
            c += load8(xs + i + 16);
            d += load8(xs + i + 24);
        }
        long s = hsum(a, b, c, d);
        for (; i < N - 100; i++) s += xs[i];
        acc += s;
        KEEP(xs);
    }
    printf("slice.sum %.3f %ld\n", (now() - t0) / each, acc / r);
    return 0;
}
