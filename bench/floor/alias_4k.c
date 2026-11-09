/* The aliasing of 4 KiB (D79, D81; D552): `b[i] = a[i] + 1.0` over 2048
 * doubles (16 KiB each, in L1), `b` 64 KiB after `a` plus `d` bytes, `d`
 * from 0 to 4095 by 16 (or one `d` given): the nanoseconds of an element.
 * A load whose address equals a store still in flight in its low 12 bits
 * waits for it (`ld_blocks_partial.address_alias`).
 *   gcc -O2 -march=native bench/floor/alias_4k.c -o alias_4k
 *   ./alias_4k [D] */
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#define N 2048
#define ROUNDS 20000
static double run(double *a, double *b) {
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (int r = 0; r < ROUNDS; r++) {
        for (int i = 0; i < N; i++) b[i] = a[i] + 1.0;
        __asm__ volatile("" ::: "memory");
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    return ((t1.tv_sec - t0.tv_sec) * 1e9 + (t1.tv_nsec - t0.tv_nsec)) / ((double)ROUNDS * N);
}
int main(int argc, char **argv) {
    char *buf = aligned_alloc(4096, 1 << 20);
    double *a = (double *)buf;
    for (int i = 0; i < N; i++) a[i] = i;
    if (argc > 1) {
        long d = atol(argv[1]);
        printf("%ld %.4f\n", d, run(a, (double *)(buf + 65536 + d)));
        return 0;
    }
    for (long d = 0; d < 4096; d += 16) printf("%ld %.4f\n", d, run(a, (double *)(buf + 65536 + d)));
    return 0;
}
