// The floor of `strings.isc`: the same work on the same 4096 lines of 60
// bytes, written in C the cheapest way known, on one thread. A new string
// is made by a bump in an arena (given back at each round), its bytes
// copied: the cost of the machine, never of an allocator. Each line: a
// name and the nanoseconds of one call (the mean over `rounds`).
//   strings <rounds>
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define N 4096
#define W 60

typedef struct {
    const char *p;
    long n;
} str;

static char *arena, *bump;
static volatile long sink;

static char *take(long n) {
    char *p = bump;
    bump += (n + 15) & ~15L;
    return p;
}

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

static str ls[N], csv[N], nums[N], padded[N];
static char text[N * (W + 1)];

static long cmp(str a, str b) {
    long m = a.n < b.n ? a.n : b.n;
    int c = memcmp(a.p, b.p, m);
    return c ? c : (a.n > b.n) - (a.n < b.n);
}

static long digits(char *out, long v) {
    char b[24];
    int k = 0;
    unsigned long u = v < 0 ? -(unsigned long)v : (unsigned long)v;
    do {
        b[k++] = '0' + u % 10;
        u /= 10;
    } while (u);
    long m = 0;
    if (v < 0) out[m++] = '-';
    while (k) out[m++] = b[--k];
    return m;
}

#define TIME(name, ...)                                                         \
    do {                                                                        \
        long acc = 0;                                                           \
        double t0 = now();                                                      \
        for (long round = 0; round < r; round++) {                              \
            bump = arena;                                                       \
            __VA_ARGS__;                                                        \
        }                                                                       \
        double t = now() - t0;                                                  \
        sink += acc;                                                            \
        printf("%s %.3f %ld\n", name, t * 1e9 / ((double)r * N), acc / r);     \
    } while (0)

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    // the arena touched once: its pages made before any measure
    arena = malloc(64L * 1024 * 1024);
    memset(arena, 1, 64L * 1024 * 1024);
    static char lb[N][W], cb[N][64], nb[N][16], pb[N][W + 8];
    for (long i = 0; i < N; i++) {
        for (long j = 0; j < W; j++) lb[i][j] = 'A' + (i * 7 + j * 13 + j * j) % 26;
        ls[i] = (str){lb[i], W};
        long m = 0;
        for (long f = 0; f < 6; f++) {
            if (f) cb[i][m++] = ',';
            m += digits(cb[i] + m, i * 31 + f * 1000);
        }
        csv[i] = (str){cb[i], m};
        nums[i] = (str){nb[i], digits(nb[i], 1000000 + i * 997)};
        memcpy(pb[i], "   ", 3);
        memcpy(pb[i] + 3, lb[i], W);
        memcpy(pb[i] + 3 + W, "  ", 2);
        padded[i] = (str){pb[i], W + 5};
        memcpy(text + i * (W + 1), lb[i], W);
        if (i + 1 < N) text[i * (W + 1) + W] = '\n';
    }
    long tlen = N * (W + 1) - 1;
    TIME("startsWith.1", for (long i = 0; i < N; i++) acc += ls[i].n >= 1 && ls[i].p[0] == 'Z');
    TIME("startsWith.8", for (long i = 0; i < N; i++) acc += ls[i].n >= 8 && !memcmp(ls[i].p, "ABCDEFGH", 8));
    TIME("endsWith.2", for (long i = 0; i < N; i++) acc += ls[i].n >= 2 && !memcmp(ls[i].p + ls[i].n - 2, "XY", 2));
    // the first byte found by `memchr`, the rest compared (glibc's `memmem`
    // costs more on 60 bytes)
    TIME("includes.3", for (long i = 0; i < N; i++) {
        const char *s = ls[i].p, *e = s + ls[i].n - 2;
        int found = 0;
        while (s < e && (s = memchr(s, 'Q', e - s))) {
            if (s[1] == 'Q' && s[2] == 'Q') {
                found = 1;
                break;
            }
            s++;
        }
        acc += found;
    });
    TIME("indexOf.1", for (long i = 0; i < N; i++) {
        const char *q = memchr(ls[i].p, 'M', ls[i].n);
        acc += q ? q - ls[i].p : -1;
    });
    TIME("equal", for (long i = 0; i < N; i++) acc += ls[i].n == ls[i ^ 1].n && !memcmp(ls[i].p, ls[i ^ 1].p, ls[i].n));
    TIME("less", for (long i = 0; i < N; i++) acc += cmp(ls[i], ls[i ^ 1]) < 0);
    TIME("concat", for (long i = 0; i < N; i++) {
        char *p = take(ls[i].n + 1);
        memcpy(p, ls[i].p, ls[i].n);
        p[ls[i].n] = '!';
        acc += ls[i].n + 1 + (p[3] == 0);
    });
    TIME("slice", for (long i = 0; i < N; i++) {
        char *p = take(20);
        memcpy(p, ls[i].p + 5, 20);
        acc += 20 + (p[3] == 0);
    });
    TIME("toUpperCase", for (long i = 0; i < N; i++) {
        char *p = take(ls[i].n);
        for (long j = 0; j < ls[i].n; j++) {
            char c = ls[i].p[j];
            p[j] = c >= 'a' && c <= 'z' ? c - 32 : c;
        }
        acc += ls[i].n + (p[3] == 0);
    });
    TIME("split.6", for (long i = 0; i < N; i++) {
        const char *s = csv[i].p, *e = s + csv[i].n;
        long parts = 0;
        for (;;) {
            const char *q = memchr(s, ',', e - s);
            const char *end = q ? q : e;
            char *p = take(end - s);
            memcpy(p, s, end - s);
            parts++;
            if (!q) break;
            s = q + 1;
        }
        acc += parts;
    });
    TIME("join.line", {
        char *p = take(N * (W + 1));
        long m = 0;
        for (long i = 0; i < N; i++) {
            if (i) p[m++] = ',';
            memcpy(p + m, ls[i].p, ls[i].n);
            m += ls[i].n;
        }
        acc += m;
    });
    TIME("lines.line", {
        str *out = (str *)take(N * sizeof(str));
        long k = 0;
        const char *s = text, *e = text + tlen;
        while (s < e) {
            const char *q = memchr(s, '\n', e - s);
            const char *end = q ? q : e;
            char *p = take(end - s);
            memcpy(p, s, end - s);
            out[k++] = (str){p, end - s};
            if (!q) break;
            s = q + 1;
        }
        acc += k;
    });
    TIME("parseI64.7", for (long i = 0; i < N; i++) {
        long v = 0;
        for (long j = 0; j < nums[i].n; j++) v = v * 10 + (nums[i].p[j] - '0');
        acc += v;
    });
    TIME("toString.i64", for (long i = 0; i < N; i++) {
        char *p = take(24);
        acc += digits(p, i * 7919);
    });
    TIME("trim", for (long i = 0; i < N; i++) {
        const char *s = padded[i].p, *e = s + padded[i].n;
        while (s < e && *s == ' ') s++;
        while (e > s && e[-1] == ' ') e--;
        char *p = take(e - s);
        memcpy(p, s, e - s);
        acc += (e - s) + (p[0] == 0);
    });
    TIME("replaceAll.1", for (long i = 0; i < N; i++) {
        long count = 0;
        for (long j = 0; j < ls[i].n; j++) count += ls[i].p[j] == 'A';
        char *p = take(ls[i].n + count);
        long m = 0;
        for (long j = 0; j < ls[i].n; j++) {
            if (ls[i].p[j] == 'A') {
                p[m++] = 'a';
                p[m++] = 'a';
            } else {
                p[m++] = ls[i].p[j];
            }
        }
        acc += m;
    });
    TIME("template", for (long i = 0; i < N; i++) {
        char *p = take(48);
        long m = digits(p, i);
        p[m++] = '-';
        m += digits(p + m, ls[i].n);
        acc += m;
    });
    TIME("padStart", for (long i = 0; i < N; i++) {
        char *p = take(20);
        memset(p, '0', 10);
        memcpy(p + 10, ls[i].p, 10);
        acc += 20 + (p[11] == 0);
    });
    TIME("bytes", for (long i = 0; i < N; i++) {
        char *p = take(ls[i].n);
        memcpy(p, ls[i].p, ls[i].n);
        acc += (unsigned char)p[7];
    });
    return 0;
}
