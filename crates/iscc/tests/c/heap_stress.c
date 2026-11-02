/* Stress test of the allocator of the runtime (isc_alloc, isc_alloc_zeroed,
 * isc_free), run by tests/heap.rs with each allocator. Waves of threads
 * allocate blocks of random sizes, fill them with a pattern, check it before
 * freeing, check that zeroed blocks are zero, and pass some blocks to other
 * threads, which free them. Prints "ok" and exits with 0, or prints the
 * first error and exits with 1. */
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void *isc_alloc(int64_t bytes);
void *isc_alloc_zeroed(int64_t bytes);
void isc_free(void *p);

#define THREADS 8
#define WAVES 4
#define STEPS 40000
#define SLOTS 256
#define EXCHANGE 64

static _Atomic(uint64_t *) exchange[EXCHANGE];
static atomic_int failed;

static uint64_t next(uint64_t *s) {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    return *s;
}

static void fail(const char *what, uint64_t size) {
    if (atomic_exchange(&failed, 1) == 0) printf("error: %s (block of %llu bytes)\n", what, (unsigned long long)size);
}

/* block layout: [0] size, [1] seed, then bytes (seed + i) & 255 */
static void fill(uint64_t *b, uint64_t size, uint64_t seed) {
    b[0] = size;
    b[1] = seed;
    uint8_t *p = (uint8_t *)b;
    for (uint64_t i = 16; i < size; i++) p[i] = (uint8_t)(seed + i);
}

static void check_and_free(uint64_t *b) {
    uint64_t size = b[0], seed = b[1];
    uint8_t *p = (uint8_t *)b;
    if (((uintptr_t)b & 63) != 0) fail("block not aligned on 64 bytes", size);
    /* every byte of small blocks, a sample of large ones */
    uint64_t step = size <= 65536 ? 1 : 4093;
    for (uint64_t i = 16; i < size; i += step)
        if (p[i] != (uint8_t)(seed + i)) { fail("pattern overwritten", size); break; }
    if (size > 16 && p[size - 1] != (uint8_t)(seed + size - 1)) fail("last byte overwritten", size);
    isc_free(b);
}

static uint64_t random_size(uint64_t *s) {
    uint64_t r = next(s);
    if (r % 1000 == 0) return (4u << 20) + next(s) % (36u << 20); /* 4 to 40 MiB */
    int bits = 4 + (int)(next(s) % 17);                          /* 16 B to 1 MiB */
    uint64_t n = (1ull << bits) + next(s) % (1ull << bits);
    return n < 16 ? 16 : n;
}

static void *worker(void *arg) {
    uint64_t s = 0x9E3779B97F4A7C15ull * ((uintptr_t)arg + 1);
    uint64_t *slot[SLOTS] = {0};
    for (int k = 0; k < STEPS && !atomic_load(&failed); k++) {
        int i = (int)(next(&s) % SLOTS);
        if (slot[i]) {
            if (next(&s) % 4 == 0) {
                /* to another thread: whatever was in the exchange is freed here */
                uint64_t *other = atomic_exchange(&exchange[next(&s) % EXCHANGE], slot[i]);
                if (other) check_and_free(other);
            } else {
                check_and_free(slot[i]);
            }
            slot[i] = 0;
        }
        uint64_t size = random_size(&s);
        int zero = next(&s) % 3 == 0;
        uint64_t *b = zero ? isc_alloc_zeroed((int64_t)size) : isc_alloc((int64_t)size);
        if (zero) {
            uint8_t *p = (uint8_t *)b;
            uint64_t step = size <= 65536 ? 1 : 4093;
            for (uint64_t j = 0; j < size; j += step)
                if (p[j]) { fail("zeroed block not zero", size); break; }
            if (p[size - 1]) fail("zeroed block not zero at its end", size);
        }
        fill(b, size, next(&s));
        slot[i] = b;
    }
    for (int i = 0; i < SLOTS; i++)
        if (slot[i]) check_and_free(slot[i]);
    return 0;
}

int main(void) {
    for (int w = 0; w < WAVES; w++) {
        pthread_t t[THREADS];
        for (int i = 0; i < THREADS; i++) pthread_create(&t[i], 0, worker, (void *)(uintptr_t)(w * THREADS + i));
        for (int i = 0; i < THREADS; i++) pthread_join(t[i], 0);
    }
    for (int i = 0; i < EXCHANGE; i++) {
        uint64_t *b = atomic_exchange(&exchange[i], 0);
        if (b) check_and_free(b);
    }
    if (atomic_load(&failed)) return 1;
    printf("ok\n");
    return 0;
}
