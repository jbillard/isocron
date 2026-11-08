// The measure of D530: the cost model of two representations of texts
// (the item (6) of TODO). Not a floor of `run.py`.
//   text_repr <rounds>
// The model of two representations of texts (the item (6) of TODO):
// R0, the one of Isocron: a pointer and a length, each text that a value
// owns in a block of its own (64 bytes of header, then its bytes rounded
// to 64); R1: 16 bytes, a length of 32 bits, the first 4 bytes of the text,
// then its bytes 4 to 12 when it has at most 12 (inline) or a pointer to
// all its bytes. The same work in each, 4096 elements in the caches.
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
typedef struct { const unsigned char *p; long n; } r0;
typedef struct { uint32_t len; uint8_t pre[4]; union { const unsigned char *p; uint8_t in[8]; } u; } r1;
_Static_assert(sizeof(r1) == 16, "16 bytes");
static inline const unsigned char *r1_bytes(const r1 *s) { return s->len <= 12 ? s->pre : s->u.p; }
// a text of at most 12 bytes from a source with 16 readable bytes: one
// load of 16 and a mask, as the compiler would write it
static inline r1 r1_short(const unsigned char *b, uint32_t n) {
    uint64_t lo, hi;
    memcpy(&lo, b, 8);
    memcpy(&hi, b + 8, 8);
    uint64_t w0 = (uint64_t)n | (lo << 32), w1 = (lo >> 32) | (hi << 32);
    // the bytes past the text cleared: byte k of the text kept when k < n
    uint64_t m0 = n >= 4 ? ~0ULL : ((1ULL << (32 + 8 * n)) - 1);
    uint64_t m1 = n >= 12 ? ~0ULL : (n <= 4 ? 0 : ((1ULL << (8 * (n - 4))) - 1));
    r1 s;
    w0 &= m0;
    w1 &= m1;
    memcpy(&s, &w0, 8);
    memcpy((char *)&s + 8, &w1, 8);
    return s;
}
static r1 r1_make(const unsigned char *b, uint32_t n, const unsigned char *owned) {
    r1 s;
    memset(&s, 0, sizeof s);
    s.len = n;
    if (n <= 12) memcpy(s.pre, b, n);
    else { memcpy(s.pre, b, 4); s.u.p = owned; }
    return s;
}
// a block as the allocator of Isocron lays them: 64 bytes of header, the
// bytes rounded to 64 (a bump here: the cost of the layout, not of the
// allocator, which R0 also pays per text in `own`)
static unsigned char *arena;
static size_t used;
static unsigned char *block(size_t n) {
    unsigned char *p = arena + used + 64;
    used += 64 + ((n + 63) / 64) * 64;
    return p;
}
static volatile long sink;
int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 2000;
    arena = aligned_alloc(64, 64 << 20);
    static char text[N][16];
    static r0 w0[N], s0[N];
    static r1 w1[N], s1[N];
    static long xs[N];
    for (int i = 0; i < N; i++) {
        xs[i] = ((long)i * 7919 * 104729) % 1000000007 - 500000000;
        int n = snprintf(text[i], 16, "%ld", xs[i] % 1000003);
        unsigned char *b = block(n);
        memcpy(b, text[i], n);
        w0[i] = (r0){b, n};
        w1[i] = r1_make((unsigned char *)text[i], n, b);
    }
    double each = (double)r * N;
    // sortBy(ws, length): a stable sort by counting, the result an array
    // that owns its texts: R0 copies each into a block of its own (malloc,
    // freed with the array), R1 moves 16 bytes (every text inline)
    for (int pass = 0; pass < 2; pass++) {
        long acc = 0;
        double t0 = now();
        for (long k = 0; k < r; k++) {
            int count[18] = {0};
            if (pass == 0) {
                for (int i = 0; i < N; i++) count[w0[i].n + 1]++;
                for (int j = 1; j < 18; j++) count[j] += count[j - 1];
                for (int i = 0; i < N; i++) {
                    unsigned char *q = malloc(w0[i].n);
                    memcpy(q, w0[i].p, w0[i].n);
                    s0[count[w0[i].n]++] = (r0){q, w0[i].n};
                }
                for (int i = 0; i < N; i++) acc += s0[i].p[0] * (i % 8);
                for (int i = 0; i < N; i++) free((void *)s0[i].p);
            } else {
                for (int i = 0; i < N; i++) count[w1[i].len + 1]++;
                for (int j = 1; j < 18; j++) count[j] += count[j - 1];
                for (int i = 0; i < N; i++) s1[count[w1[i].len]++] = w1[i];
                for (int i = 0; i < N; i++) acc += r1_bytes(&s1[i])[0] * (i % 8);
            }
        }
        printf("sortBy.length.%s %.3f %ld\n", pass ? "r1" : "r0", (now() - t0) / each, acc / r);
    }
    // isAbsolute of paths of 21 to 30 bytes: R0 reads each text in its
    // block, R1 its first bytes in the element
    static r0 q0[N];
    static r1 q1[N];
    // the paths after the texts (their blocks kept)
    for (int i = 0; i < N; i++) {
        char t[48];
        int n = sprintf(t, i % 3 == 0 ? "/x/a/./b%d/../c//d/e%d.txt" : "a/./b%d/../c//d/e%d.txt", i % 10, i);
        unsigned char *b = block(n);
        memcpy(b, t, n);
        q0[i] = (r0){b, n};
        q1[i] = r1_make(b, n, b);
    }
    for (int pass = 0; pass < 2; pass++) {
        long acc = 0;
        double t0 = now();
        for (long k = 0; k < r; k++)
            for (int i = 0; i < N; i++) {
                const unsigned char *b = pass ? q1[i].pre : q0[i].p;
                long n = pass ? q1[i].len : q0[i].n;
                acc += (n >= 1 && b[0] == '/') || (n >= 3 && b[1] == ':' && (b[2] == '/' || b[2] == '\\'));
            }
        printf("isAbsolute.%s %.3f %ld\n", pass ? "r1" : "r0", (now() - t0) / each, acc / r);
    }
    // `indexOf` of a byte in paths of 21 to 30 bytes: R1 chooses where the
    // bytes are (inline or by its pointer) before the search
    for (int pass = 0; pass < 2; pass++) {
        long acc = 0;
        double t0 = now();
        for (long k = 0; k < r; k++)
            for (int i = 0; i < N; i++) {
                const unsigned char *b = pass ? r1_bytes(&q1[i]) : q0[i].p;
                long n = pass ? q1[i].len : q0[i].n;
                const unsigned char *e = memchr(b, 'e', n);
                acc += e ? e - b : -1;
            }
        printf("indexOf.%s %.3f %ld\n", pass ? "r1" : "r0", (now() - t0) / each, acc / r);
    }
    // equality of two texts of 1 to 8 bytes (each against its neighbour)
    for (int pass = 0; pass < 2; pass++) {
        long acc = 0;
        double t0 = now();
        for (long k = 0; k < r; k++)
            for (int i = 0; i < N; i++) {
                int j = (i * 17 + k) & (N - 1);
                if (pass == 0) acc += w0[i].n == w0[j].n && memcmp(w0[i].p, w0[j].p, w0[i].n) == 0;
                else acc += memcmp(&w1[i], &w1[j], 16) == 0;
            }
        printf("equal.%s %.3f %ld\n", pass ? "r1" : "r0", (now() - t0) / each, acc / r);
    }
    // a text of at most 8 bytes made then freed (`toString`, the key of an
    // object of `json`): R0 a block, R1 the element
    for (int pass = 0; pass < 2; pass++) {
        long acc = 0;
        double t0 = now();
        for (long k = 0; k < r; k++) {
            for (int i = 0; i < N; i++) {
                if (pass == 0) {
                    unsigned char *q = malloc(w0[i].n);
                    memcpy(q, text[i], w0[i].n);
                    s0[i] = (r0){q, w0[i].n};
                } else {
                    s1[i] = r1_short((unsigned char *)text[i], w1[i].len);
                }
            }
            acc += pass ? s1[k & (N - 1)].pre[0] : s0[k & (N - 1)].p[0];
            if (pass == 0)
                for (int i = 0; i < N; i++) free((void *)s0[i].p);
        }
        printf("make.%s %.3f %ld\n", pass ? "r1" : "r0", (now() - t0) / each, acc / r);
    }
    // the lines of a text of 4096 lines of 60 bytes: R0 copies each into
    // the block of the lines (8 bytes of address before it, rounded to 16,
    // as `lines`), R1 a view: its length, its first 4 bytes, a pointer
    static unsigned char in[N * 61];
    for (int i = 0; i < N * 61; i++) in[i] = (i % 61 == 60) ? '\n' : "ACGT"[(i * 7) % 4];
    unsigned char *lb = aligned_alloc(64, N * 80 + 64);
    for (int pass = 0; pass < 2; pass++) {
        long acc = 0;
        double t0 = now();
        for (long k = 0; k < r; k++) {
            const unsigned char *p = in, *end = in + N * 61;
            size_t at = 64;
            int i = 0;
            while (p < end) {
                const unsigned char *e = memchr(p, '\n', end - p);
                long n = e - p;
                if (pass == 0) {
                    memcpy(lb + at, &lb, 8);
                    memcpy(lb + at + 8, p, n);
                    s0[i++] = (r0){lb + at + 8, n};
                    at += (8 + n + 15) & ~15;
                } else {
                    s1[i++] = r1_make(p, n, p);
                }
                p = e + 1;
            }
            acc += pass ? s1[N - 1].pre[1] : s0[N - 1].p[1];
        }
        printf("lines.%s %.3f %ld\n", pass ? "r1" : "r0", (now() - t0) / each, acc / r);
    }
    sink = (long)s0[0].p + s1[0].len;
    return 0;
}
