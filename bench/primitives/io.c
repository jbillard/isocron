// bench-io
// The floor of `io.isc` (D498): the same work in C, the cheapest way known,
// on one thread: the digits of each number written into a buffer of 64 KiB
// written by `write` when full; the 64 KiB written at once; the standard
// input read whole, its lines found by `memchr` and parsed as they come.
// The results written into the file `argv[2]`.
//   io <rounds> <results> < input > output
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#define N 4096
#define BUF (64 * 1024)

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e9 + t.tv_nsec;
}

static char out[BUF];
static size_t used;

static void flush(void) {
    size_t done = 0;
    while (done < used) {
        ssize_t k = write(1, out + done, used - done);
        if (k <= 0) exit(1);
        done += (size_t)k;
    }
    used = 0;
}

// a number and a newline into the buffer
static inline void put(long x) {
    if (used + 24 > BUF) flush();
    char t[24];
    int k = 0;
    unsigned long u = x < 0 ? 0 - (unsigned long)x : (unsigned long)x;
    do {
        t[k++] = (char)('0' + u % 10);
        u /= 10;
    } while (u);
    if (x < 0) out[used++] = '-';
    while (k) out[used++] = t[--k];
    out[used++] = '\n';
}

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    static long xs[N];
    static char block[65536];
    long sx = 0;
    for (long i = 0; i < N; i++) {
        xs[i] = (i * 7919 * 104729) % 1000000007 - 500000000;
        sx += xs[i];
    }
    for (int i = 0; i < 65536; i++) block[i] = (char)(65 + i % 26);
    double t0 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < N; i++) put(xs[i]);
    flush();
    double t1 = now();
    for (long k = 0; k < r; k++) {
        size_t done = 0;
        while (done < sizeof block) {
            ssize_t w = write(1, block + done, sizeof block - done);
            if (w <= 0) return 1;
            done += (size_t)w;
        }
    }
    double t2 = now();
    // the input whole, then its lines
    size_t cap = 1 << 24, len = 0;
    char *in = malloc(cap);
    for (;;) {
        if (len == cap) in = realloc(in, cap *= 2);
        ssize_t k = read(0, in + len, cap - len);
        if (k < 0) return 1;
        if (k == 0) break;
        len += (size_t)k;
    }
    long s = 0, m = 0;
    char *p = in, *end = in + len;
    while (p < end) {
        char *e = memchr(p, '\n', (size_t)(end - p));
        if (!e) e = end;
        int neg = *p == '-';
        long v = 0;
        for (char *q = p + neg; q < e; q++) v = v * 10 + (*q - '0');
        s += neg ? -v : v;
        m++;
        p = e + 1;
    }
    double t3 = now();
    FILE *f = fopen(argv[2], "w");
    if (!f) return 1;
    fprintf(f, "print.line %.3f %ld\nprintBytes.byte %.3f %ld\nreadStdinLines.line %.3f %ld\n", (t1 - t0) / ((double)r * N), sx * r, (t2 - t1) / ((double)r * 65536), r * 65536, (t3 - t2) / (double)m, s);
    fclose(f);
    return 0;
}
