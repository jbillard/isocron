// The floor of `maps.isc` (D492): the same work in C, the cheapest way
// known, on one thread: a table of hashes with open addressing (linear
// probing, a power of two at most half full), the keys also in the order
// of their first insertion (that of the iterators of a map); integers
// hashed by a multiplication, texts by FNV-1a. Each line: a name, the
// nanoseconds of one element (the mean over `rounds`), a checksum.
//   maps <rounds>
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define N 4096
#define K 1000
#define SLOTS 4096

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e9 + t.tv_nsec;
}

static inline uint32_t hash_i(long k) { return (uint32_t)(((uint64_t)k * 0x9e3779b97f4a7c15ULL) >> 52); }

static inline uint32_t hash_s(const char *p, int n) {
    uint64_t h = 1469598103934665603ULL;
    for (int i = 0; i < n; i++) h = (h ^ (unsigned char)p[i]) * 1099511628211ULL;
    return (uint32_t)(h >> 52);
}

// a map of integers: the slot of each key (-1 empty) in `key`, `val`; the
// keys in the order of their first insertion in `order`
typedef struct {
    long key[SLOTS], val[SLOTS];
    char used[SLOTS];
    long order[N];
    int size;
} MapI;

static inline int find_i(const MapI *m, long k) {
    uint32_t s = hash_i(k);
    while (m->used[s] && m->key[s] != k) s = (s + 1) & (SLOTS - 1);
    return m->used[s] ? (int)s : -1;
}

static inline long *slot_i(MapI *m, long k) {
    uint32_t s = hash_i(k);
    while (m->used[s] && m->key[s] != k) s = (s + 1) & (SLOTS - 1);
    if (!m->used[s]) {
        m->used[s] = 1;
        m->key[s] = k;
        m->val[s] = 0;
        m->order[m->size++] = k;
    }
    return &m->val[s];
}

typedef struct {
    const char *key[SLOTS];
    int len[SLOTS];
    long val[SLOTS];
} MapS;

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    static long xs[N];
    static char names[K][16];
    static int nlen[K];
    for (long i = 0; i < N; i++) xs[i] = (i * 7919 * 104729) % 1000000007;
    for (int k = 0; k < K; k++) nlen[k] = snprintf(names[k], 16, "key%ld", ((long)k * 7919) % 100003);
    static MapI m, b;
    memset(&m, 0, sizeof m);
    for (int i = 0; i < N; i++) *slot_i(&m, xs[i] % 1000) = i;
    static MapS ms;
    memset(&ms, 0, sizeof ms);
    for (int k = 0; k < K; k++) {
        uint32_t s = hash_s(names[k], nlen[k]);
        while (ms.key[s]) s = (s + 1) & (SLOTS - 1);
        ms.key[s] = names[k];
        ms.len[s] = nlen[k];
        ms.val[s] = k;
    }
    double each = (double)r * N;
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < r; k++) {
        memset(&b.used, 0, sizeof b.used);
        b.size = 0;
        for (int i = 0; i < N; i++) (*slot_i(&b, xs[i] % 1000))++;
        int s7 = find_i(&b, 7), s999 = find_i(&b, 999);
        acc += b.size * 100000L + (s7 < 0 ? 0 : b.val[s7]) * 100 + (s999 < 0 ? 0 : b.val[s999]);
    }
    printf("countBy.i64 %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        memset(&b.used, 0, sizeof b.used);
        b.size = 0;
        for (int i = 0; i < N; i++) *slot_i(&b, xs[i] % 1000) = i;
        int s7 = find_i(&b, 7);
        acc += b.size * 100000L + (s7 < 0 ? 0 : b.val[s7]);
    }
    printf("Map.build.i64 %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < N; i++) {
            int s = find_i(&m, xs[i] % 1000);
            acc += s < 0 ? 0 : m.val[s];
        }
    printf("get.i64 %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < N; i++) {
            const char *p = names[xs[i] % 1000];
            int n = nlen[xs[i] % 1000];
            uint32_t s = hash_s(p, n);
            while (ms.key[s] && !(ms.len[s] == n && memcmp(ms.key[s], p, (size_t)n) == 0)) s = (s + 1) & (SLOTS - 1);
            acc += ms.key[s] ? ms.val[s] : 0;
        }
    printf("get.string %.3f %ld\n", (now() - t0) / each, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < N; i++) acc += find_i(&m, xs[i] % 1000 + 1000) >= 0;
    printf("has.miss.i64 %.3f %ld\n", (now() - t0) / each, acc / r);
    return 0;
}
