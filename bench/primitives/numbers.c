// The floor of `numbers.isc` (D487): the same work in C, the cheapest way
// known, on one thread. Each line: a name, the nanoseconds of one call (the
// mean over `rounds`), a checksum.
//   numbers <rounds>
#define _GNU_SOURCE
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "f64_text.h"

// the bytes written into `p` kept: an empty `asm` that may read the memory
// (D456: GCC removed the writes of a text never read)
#define KEEP(p) __asm__ volatile("" : : "r"(p) : "memory")

#define N 4096

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e9 + t.tv_nsec;
}

// `x.toFixed(2)` for |x| < 1e17: the first three decimals of the exact value
// of |x|, from its integer and its power of two, rounded half away from
// zero (as JavaScript), then written
static int fixed2(double x, char *p) {
    uint64_t bits;
    memcpy(&bits, &x, 8);
    int be = (int)((bits >> 52) & 0x7ff);
    uint64_t m = bits & ((1ULL << 52) - 1);
    int e = be == 0 ? -1074 : be - 1075;
    if (be != 0) m |= 1ULL << 52;
    uint64_t q;
    if (e >= 0) {
        q = (m << e) * 100;
    } else {
        uint64_t t = m * 1000;
        uint64_t n = -e >= 64 ? 0 : t >> -e;
        q = n / 10 + (n % 10 >= 5);
    }
    char *o = p;
    if (x < 0) *o++ = '-';
    uint64_t ip = q / 100;
    unsigned d = (unsigned)(q % 100);
    char b[24];
    int k = 0;
    do {
        b[k++] = (char)('0' + ip % 10);
        ip /= 10;
    } while (ip);
    while (k) *o++ = b[--k];
    *o++ = '.';
    *o++ = (char)('0' + d / 10);
    *o++ = (char)('0' + d % 10);
    return (int)(o - p);
}

// `parseI64` of a text of `n` bytes (its length known, as in Isocron): an
// optional sign, then decimal digits, within 64 bits; up to 18 digits
// without a check of overflow (they cannot leave 63 bits)
static long parse_i64(const char *s, size_t n, int *ok) {
    int neg = *s == '-';
    if (*s == '-' || *s == '+') {
        s++;
        n--;
    }
    if (!n) {
        *ok = 0;
        return 0;
    }
    unsigned long v = 0;
    if (n <= 18) {
        for (size_t i = 0; i < n; i++) {
            unsigned c = (unsigned)(s[i] - '0');
            if (c > 9) {
                *ok = 0;
                return 0;
            }
            v = v * 10 + c;
        }
        return neg ? -(long)v : (long)v;
    }
    for (; *s; s++) {
        unsigned c = (unsigned)(*s - '0');
        if (c > 9 || __builtin_mul_overflow(v, 10UL, &v) || __builtin_add_overflow(v, c, &v)) {
            *ok = 0;
            return 0;
        }
    }
    if (v > (unsigned long)LONG_MAX + neg) {
        *ok = 0;
        return 0;
    }
    return neg ? (long)(0 - v) : (long)v;
}

// `parseF64` of a decimal of at most 15 digits and 22 decimals: its digits
// as an integer, divided by a power of ten (both exact: one rounding,
// Clinger's fast path); other texts by `strtod`
static const double pow10s[23] = {1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22};

static double parse_f64(const char *s, int *ok) {
    const char *p = s;
    int neg = *p == '-';
    if (*p == '-' || *p == '+') p++;
    uint64_t m = 0;
    int digits = 0, decimals = 0, point = 0;
    for (; *p; p++) {
        if (*p == '.' && !point) {
            point = 1;
            continue;
        }
        unsigned c = (unsigned)(*p - '0');
        if (c > 9) break;
        m = m * 10 + c;
        digits++;
        decimals += point;
    }
    if (!*p && digits > 0 && digits <= 15 && decimals <= 22) {
        double v = (double)m / pow10s[decimals];
        return neg ? -v : v;
    }
    char *end;
    double v = strtod(s, &end);
    if (*end || end == s) *ok = 0;
    return v;
}

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    static double xs[N];
    static char ints[N][24], floats[N][40];
    static size_t ilen[N];
    for (long i = 0; i < N; i++) {
        xs[i] = (i % 2 == 0 ? 1.0 : -1.0) * (double)(i * i * 7 + 1) / 3.0;
        ilen[i] = (size_t)snprintf(ints[i], 24, "%ld", (i * 7919 * 104729) % 1000000007 - 500000000);
        floats[i][f64_text((double)((i * 7919) % 1000003 - 500000) / 1000.0, floats[i])] = 0;
    }
    double each = (double)r * N;
    char t[64];
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < N; i++) {
            acc += fixed2(xs[i], t);
            KEEP(t);
        }
    printf("toFixed.2 %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < N; i++) {
            acc += f64_text(xs[i], t);
            KEEP(t);
        }
    printf("toString.f64 %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    int ok = 1;
    t0 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < N; i++) acc += parse_i64(ints[i], ilen[i], &ok);
    if (!ok) return 1;
    printf("parseI64 %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < N; i++) acc += (long)(parse_f64(floats[i], &ok) * 3.0);
    if (!ok) return 1;
    printf("parseF64 %.3f %ld\n", (now() - t0) / each, acc / r);
    return 0;
}
