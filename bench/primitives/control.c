// The floor of `control.isc` (D497): the same work in C, the cheapest way
// known, on one thread: the recursions and the `match` as C writes them
// (a recursive call, a loop, a `switch` on a tag); Euclid's division in 32
// bits when both numbers fit; the steps of Collatz of 32 numbers at once in
// four vectors of 8 lanes (Isocron runs the loops of a `map` so: a scalar
// loop was 4 times slower). Each
// line: a name, the nanoseconds of one call or element (the mean over
// `rounds`), a checksum.
//   control <rounds>
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

__attribute__((noinline)) static long fib(long n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }

static inline long gcd(long a, long b) {
    if (((unsigned long)a | (unsigned long)b) >> 32 == 0) {
        uint32_t x = (uint32_t)a, y = (uint32_t)b;
        while (y) {
            uint32_t t = x % y;
            x = y;
            y = t;
        }
        return x;
    }
    while (b) {
        long t = a % b;
        a = b;
        b = t;
    }
    return a;
}

#include <immintrin.h>

typedef struct {
    int tag;
    long a, b;
} Shape;

static inline long area(const Shape *s) {
    switch (s->tag) {
    case 0: return 3 * s->a * s->a;
    case 1: return s->a * s->a;
    default: return s->a * s->b;
    }
}

// the steps to 1 of 32 numbers at once (4 vectors of 8), as Isocron runs
// them: every lane stepped at each turn, the turn written into a lane when
// it reaches 1, until all have (cheaper than giving a lane done its next
// number: a reduction and a load at each turn, twice as slow)
static long collatz32(const long *start) {
    const __m512i one = _mm512_set1_epi64(1);
    __m512i x[4], res[4];
    for (int j = 0; j < 4; j++) {
        x[j] = _mm512_loadu_si512((const void *)(start + 8 * j));
        res[j] = _mm512_setzero_si512();
    }
    __mmask8 live[4] = {0xff, 0xff, 0xff, 0xff};
    for (long t = 0; live[0] | live[1] | live[2] | live[3]; t++) {
        __m512i tv = _mm512_set1_epi64(t);
        for (int j = 0; j < 4; j++) {
            __mmask8 at1 = _mm512_cmpeq_epi64_mask(x[j], one) & live[j];
            res[j] = _mm512_mask_mov_epi64(res[j], at1, tv);
            live[j] &= (__mmask8)~at1;
            __mmask8 odd = _mm512_test_epi64_mask(x[j], one);
            __m512i half = _mm512_srli_epi64(x[j], 1);
            __m512i up = _mm512_add_epi64(_mm512_add_epi64(_mm512_add_epi64(x[j], x[j]), x[j]), one);
            x[j] = _mm512_mask_mov_epi64(half, odd, up);
        }
    }
    __m512i s = _mm512_add_epi64(_mm512_add_epi64(res[0], res[1]), _mm512_add_epi64(res[2], res[3]));
    return _mm512_reduce_add_epi64(s);
}

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    static long xs[N], ys[N];
    static Shape shapes[N];
    for (long i = 0; i < N; i++) {
        xs[i] = (i * 7919 * 104729) % 1000003 + 1;
        ys[i] = (i * 104729) % 999983 + 1;
        if (i % 3 == 0) shapes[i] = (Shape){0, i % 7, 0};
        else if (i % 3 == 1) shapes[i] = (Shape){1, i % 11, 0};
        else shapes[i] = (Shape){2, i % 5, i % 13};
    }
    double each = (double)r * N;
    // the argument through a volatile: fib(25) not computed at compile time
    volatile long arg = 25;
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < r; k++) acc += fib(arg);
    printf("fib.call %.3f %ld\n", (now() - t0) / ((double)r * 242785), acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        long s = 0;
        for (int i = 0; i < N; i++) s += gcd(xs[i], ys[i]);
        acc += s;
        __asm__ volatile("" : : "r"(xs) : "memory");
    }
    printf("gcd %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    // 25 times the rounds, as `control.isc`
    for (long k = 0; k < r * 25; k++) {
        long s = 0;
        for (int i = 0; i < N; i++) s += area(&shapes[i]);
        acc += s;
        __asm__ volatile("" : : "r"(shapes) : "memory");
    }
    printf("match.area %.3f %ld\n", (now() - t0) / (each * 25), acc / (r * 25));
    acc = 0;
    t0 = now();
    static long starts[N] __attribute__((aligned(64)));
    for (int i = 0; i < N; i++) starts[i] = i % 1000 + 1;
    for (long k = 0; k < r; k++) {
        long s = 0;
        for (int i = 0; i < N; i += 32) s += collatz32(starts + i);
        acc += s;
        __asm__ volatile("" : : "r"(starts) : "memory");
    }
    printf("collatz %.3f %ld\n", (now() - t0) / each, acc / r);
    return 0;
}
