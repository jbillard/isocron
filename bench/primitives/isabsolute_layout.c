// The measure of D465: `isAbsolute` over 4096 paths, inline in one array
// with `strlen` (`sys.c` without it), or each in a
// block of its own of 64 bytes behind a pointer and a length (the layout of
// an array of strings of Isocron). Not a floor of `run.py`.
//   isabsolute_layout <rounds>
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec * 1e-9; }
static int abs_path(const char *p, long n) { return (n >= 1 && p[0] == '/') || (n >= 3 && p[1] == ':' && (p[2] == '/' || p[2] == '\\')); }
int main(int argc, char **argv) {
    long r = atol(argv[1]);
    static char qs[4096][48];
    static struct { char *p; long n; } ss[4096];
    for (int i = 0; i < 4096; i++) {
        char t[48];
        snprintf(t, sizeof t, "a/./b%d/../c//d/e%d.txt", i % 10, i);
        if (i % 3 == 0) snprintf(qs[i], 48, "/x/%s", t); else snprintf(qs[i], 48, "%s", t);
        long n = strlen(qs[i]);
        ss[i].p = aligned_alloc(64, 64);
        memcpy(ss[i].p, qs[i], n);
        ss[i].n = n;
    }
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < 4096; i++) acc += abs_path(qs[i], strlen(qs[i]));
    double t1 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < 4096; i++) acc += abs_path(ss[i].p, ss[i].n);
    double t2 = now();
    printf("inline %.3f\nblocks %.3f\n%ld\n", (t1 - t0) * 1e9 / (r * 4096.0), (t2 - t1) * 1e9 / (r * 4096.0), acc);
    return 0;
}
