/* The spatial prefetcher of L2 (D77, D81; D552): two threads on two cores,
 * each adding to its own counter of 8 bytes, the counters `gap` bytes
 * apart from a multiple of 128 (64: two lines of the same aligned pair of
 * 128 bytes; 128: lines of two pairs). If the prefetcher of a core fetches
 * the other line of the pair of a line it misses, the counters 64 bytes
 * apart are moved between the cores as if they shared a line.
 *   gcc -O2 -pthread bench/floor/line_pairs.c -o line_pairs
 *   ./line_pairs GAP CPU_A CPU_B   (nanoseconds a step) */
#define _GNU_SOURCE
#include <pthread.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#define STEPS 200000000L
static char *base;
static long gap;
struct arg { int cpu; volatile long *c; };
static void *run(void *p) {
    struct arg *a = p;
    cpu_set_t s; CPU_ZERO(&s); CPU_SET(a->cpu, &s);
    pthread_setaffinity_np(pthread_self(), sizeof s, &s);
    for (long i = 0; i < STEPS; i++) *a->c += 1;
    return 0;
}
int main(int argc, char **argv) {
    gap = atol(argv[1]);
    base = aligned_alloc(4096, 8192);
    struct arg a = {atoi(argv[2]), (volatile long *)(base + 1024)};
    struct arg b = {atoi(argv[3]), (volatile long *)(base + 1024 + gap)};
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    pthread_t x, y;
    pthread_create(&x, 0, run, &a);
    pthread_create(&y, 0, run, &b);
    pthread_join(x, 0);
    pthread_join(y, 0);
    clock_gettime(CLOCK_MONOTONIC, &t1);
    printf("%ld %.3f\n", gap, ((t1.tv_sec - t0.tv_sec) * 1e9 + (t1.tv_nsec - t0.tv_nsec)) / STEPS);
    return 0;
}
