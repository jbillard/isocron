// The constants of the machine that the floors of the programs are made of
// (DECISIONS D182), measured without Isocron: the best of R runs of each.
//   floor <threads>
// Each line: `name value unit`. Run after `bench/machine.sh apply parallel`
// (fixed frequency) through `bench/floor/run.sh`.
#define _GNU_SOURCE
#include <immintrin.h>
#include <omp.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>

// a node of a binary tree (two pointers), made in an arena by a bump
struct node {
    struct node *l, *r;
};

static struct node *make(struct node **bump, int d) {
    struct node *n = (*bump)++;
    if (d > 0) {
        n->l = make(bump, d - 1);
        n->r = make(bump, d - 1);
    } else {
        n->l = n->r = 0;
    }
    return n;
}

static long check(const struct node *n) {
    return n->l ? 1 + check(n->l) + check(n->r) : 1;
}

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

#define BYTES (256ul << 20)
#define REPS 7

static unsigned char *mapped(size_t n) {
    unsigned char *p = mmap(NULL, n, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) { perror("mmap"); exit(1); }
    madvise(p, n, MADV_HUGEPAGE);
    return p;
}

int main(int argc, char **argv) {
    int t = argc > 1 ? atoi(argv[1]) : 1;
    omp_set_num_threads(t);
    unsigned char *a = mapped(BYTES), *b = mapped(BYTES);
    memset(a, 1, BYTES);
    memset(b, 2, BYTES);
    double best;
    size_t chunk = BYTES / t;

    // reading: the sum of 256 MiB (in memory, not in the caches)
    best = 1e9;
    volatile uint64_t sink = 0;
    for (int r = 0; r < REPS; r++) {
        double s = now();
        uint64_t total = 0;
#pragma omp parallel for reduction(+ : total)
        for (int k = 0; k < t; k++) {
            const __m256i *p = (const __m256i *)(a + k * chunk);
            __m256i acc = _mm256_setzero_si256();
            for (size_t i = 0; i < chunk / 32; i++) acc = _mm256_add_epi64(acc, _mm256_load_si256(p + i));
            total += _mm256_extract_epi64(acc, 0) + _mm256_extract_epi64(acc, 1);
        }
        sink += total;
        double e = now() - s;
        if (e < best) best = e;
    }
    printf("read %.1f GB/s\n", BYTES / best / 1e9);

    // copying: 256 MiB from one array to another (both faulted in)
    best = 1e9;
    for (int r = 0; r < REPS; r++) {
        double s = now();
#pragma omp parallel for
        for (int k = 0; k < t; k++) memcpy(b + k * chunk, a + k * chunk, chunk);
        double e = now() - s;
        if (e < best) best = e;
    }
    printf("copy %.1f GB/s\n", BYTES / best / 1e9);

    // writing new memory: the first write of each page, zeroed by the kernel
    best = 1e9;
    for (int r = 0; r < REPS; r++) {
        unsigned char *c = mapped(BYTES);
        double s = now();
#pragma omp parallel for
        for (int k = 0; k < t; k++)
            for (size_t i = 0; i < chunk; i += 4096) c[k * chunk + i] = 1;
        double e = now() - s;
        if (e < best) best = e;
        munmap(c, BYTES);
    }
    printf("new_pages %.1f GB/s\n", BYTES / best / 1e9);

    // the floating-point divider: independent divisions of 4 doubles
    best = 1e9;
    const long DIVS = 1l << 27;
    for (int r = 0; r < REPS; r++) {
        double s = now();
        double out = 0;
#pragma omp parallel for reduction(+ : out)
        for (int k = 0; k < t; k++) {
            __m256d x[8], one = _mm256_set1_pd(1.0);
            for (int j = 0; j < 8; j++) x[j] = _mm256_set1_pd(1.5 + j);
            for (long i = 0; i < DIVS / 32 / t; i++)
                for (int j = 0; j < 8; j++) x[j] = _mm256_div_pd(one, _mm256_add_pd(x[j], one));
            for (int j = 0; j < 8; j++) out += _mm256_cvtsd_f64(x[j]);
        }
        sink += (uint64_t)out;
        double e = now() - s;
        if (e < best) best = e;
    }
    printf("divide_f64 %.3f ns_per_division_per_core\n", best * t / DIVS * 1e9);

    // the square root: independent square roots of 4 doubles
    best = 1e9;
    for (int r = 0; r < REPS; r++) {
        double s = now();
        double out = 0;
#pragma omp parallel for reduction(+ : out)
        for (int k = 0; k < t; k++) {
            __m256d x[8], one = _mm256_set1_pd(1.0);
            for (int j = 0; j < 8; j++) x[j] = _mm256_set1_pd(2.5 + j);
            for (long i = 0; i < DIVS / 32 / t; i++)
                for (int j = 0; j < 8; j++) x[j] = _mm256_sqrt_pd(_mm256_add_pd(x[j], one));
            for (int j = 0; j < 8; j++) out += _mm256_cvtsd_f64(x[j]);
        }
        sink += (uint64_t)out;
        double e = now() - s;
        if (e < best) best = e;
    }
    printf("sqrt_f64 %.3f ns_per_root_per_core\n", best * t / DIVS * 1e9);

    // fused multiply-adds: 10 independent chains of 4 doubles, in registers
    best = 1e9;
    const long FMAS = 1l << 32;
    for (int r = 0; r < REPS; r++) {
        double s = now();
        double out = 0;
#pragma omp parallel for reduction(+ : out)
        for (int k = 0; k < t; k++) {
            __m256d m = _mm256_set1_pd(0.999999), c = _mm256_set1_pd(1e-7);
            __m256d x0 = _mm256_set1_pd(1), x1 = _mm256_set1_pd(2), x2 = _mm256_set1_pd(3), x3 = _mm256_set1_pd(4),
                    x4 = _mm256_set1_pd(5), x5 = _mm256_set1_pd(6), x6 = _mm256_set1_pd(7), x7 = _mm256_set1_pd(8),
                    x8 = _mm256_set1_pd(9), x9 = _mm256_set1_pd(10);
            for (long i = 0; i < FMAS / 40 / t; i++) {
                x0 = _mm256_fmadd_pd(x0, m, c); x1 = _mm256_fmadd_pd(x1, m, c);
                x2 = _mm256_fmadd_pd(x2, m, c); x3 = _mm256_fmadd_pd(x3, m, c);
                x4 = _mm256_fmadd_pd(x4, m, c); x5 = _mm256_fmadd_pd(x5, m, c);
                x6 = _mm256_fmadd_pd(x6, m, c); x7 = _mm256_fmadd_pd(x7, m, c);
                x8 = _mm256_fmadd_pd(x8, m, c); x9 = _mm256_fmadd_pd(x9, m, c);
            }
            __m256d y = _mm256_add_pd(_mm256_add_pd(_mm256_add_pd(x0, x1), _mm256_add_pd(x2, x3)),
                                      _mm256_add_pd(_mm256_add_pd(_mm256_add_pd(x4, x5), _mm256_add_pd(x6, x7)), _mm256_add_pd(x8, x9)));
            out += _mm256_cvtsd_f64(y);
        }
        sink += (uint64_t)out;
        double e = now() - s;
        if (e < best) best = e;
    }
    printf("fma_f64 %.4f ns_per_lane_per_core\n", best * t / FMAS * 1e9);

    // latencies: one chain of dependent scalar divisions, square roots
    {
        const long L = 1l << 24;
        double x = 1.7, s = now();
        for (long i = 0; i < L; i++) x = 1.0 / (x + 1.0);
        double e = now() - s;
        sink += (uint64_t)x;
        printf("divide_latency %.3f ns\n", e / L * 1e9);
        x = 1.7;
        s = now();
        for (long i = 0; i < L; i++) x = __builtin_sqrt(x + 1.0);
        e = now() - s;
        sink += (uint64_t)x;
        printf("sqrt_latency %.3f ns\n", e / L * 1e9);
    }

    // random increments of 32-bit counters, a private table per thread of
    // 64 MiB (in memory) and of 256 KiB (in the L2 cache), plain additions
    for (size_t tb = 64ul << 20; tb >= (256ul << 10); tb /= (tb == (64ul << 20) ? 32 : 8)) {
        uint32_t *c = (uint32_t *)mapped(tb * t);
        memset(c, 0, tb * t);
        size_t mask = tb / 4 - 1;
        const long UPD = 1l << 27;
        best = 1e9;
        for (int r = 0; r < 3; r++) {
            double s = now();
#pragma omp parallel for
            for (int k = 0; k < t; k++) {
                uint32_t *mine = c + k * (tb / 4);
                // four independent generators: the updates, not one chain of
                // shifts, are measured
                uint64_t a0 = 0x9e3779b97f4a7c15ull * (4 * k + 1), a1 = 0x9e3779b97f4a7c15ull * (4 * k + 2),
                         a2 = 0x9e3779b97f4a7c15ull * (4 * k + 3), a3 = 0x9e3779b97f4a7c15ull * (4 * k + 4);
#define STEP(h) h ^= h << 13; h ^= h >> 7; h ^= h << 17; mine[(h >> 3) & mask]++;
                for (long i = 0; i < UPD / t / 4; i++) {
                    STEP(a0) STEP(a1) STEP(a2) STEP(a3)
                }
#undef STEP
            }
            double e = now() - s;
            if (e < best) best = e;
        }
        printf("random_increment_%zuKiB %.3f ns_per_update_per_core\n", tb >> 10, best * t / UPD * 1e9);
        munmap(c, tb * t);
    }

    // the nodes of binary trees: made in an arena (a bump, freed at once),
    // then visited; trees of depth 12 (in the L2 cache) and 20 (in memory)
    for (int d = 12; d <= 20; d += 8) {
        long nodes = (2l << d) - 1;
        long trees = (1l << 24) / nodes + 1;
        struct node *arena = (struct node *)mapped(sizeof(struct node) * nodes * t);
        best = 1e9;
        for (int r = 0; r < 3; r++) {
            double s = now();
            long total = 0;
#pragma omp parallel for reduction(+ : total)
            for (int k = 0; k < t; k++)
                for (long i = 0; i < trees / t; i++) {
                    struct node *bump = arena + k * nodes;
                    total += check(make(&bump, d));
                }
            double e = now() - s;
            sink += total;
            if (e < best) best = e;
        }
        printf("tree_node_depth%d %.3f ns_per_node_per_core\n", d, best * t / ((trees / t) * t * nodes) * 1e9);
        munmap(arena, sizeof(struct node) * nodes * t);
    }

    // latency of an addition of doubles, of a load from the L1 cache (a
    // chain through 16 KiB)
    {
        const long L = 1l << 26;
        double x = 1.0, y = 1e-300, s = now();
        for (long i = 0; i < L; i++) __asm__ volatile("vaddsd %1, %0, %0" : "+x"(x) : "x"(y));
        double e = now() - s;
        sink += (uint64_t)x;
        printf("add_latency %.3f ns\n", e / L * 1e9);
        uint32_t *ring = (uint32_t *)mapped(16384);
        size_t m = 16384 / 64;
        for (size_t i = 0; i < m; i++) ring[i * 16] = (uint32_t)(((i * 7 + 3) % m) * 16);
        uint32_t q = 0;
        s = now();
        for (long i = 0; i < L; i++) q = ring[q];
        e = now() - s;
        sink += q;
        printf("l1_latency %.3f ns\n", e / L * 1e9);
    }

    // a number of 512 words times a word, the inner loop of the
    // multiplications of big integers: `mulx` (the high and low words, no
    // flags) and a chain of `adc`, by 4 words: ns per word
    {
        uint64_t x[512], out[513];
        for (int i = 0; i < 512; i++) x[i] = 0x9e3779b97f4a7c15ull * (i + 1);
        const long R = 1l << 18;
        double best_w = 1e9;
        for (int rep = 0; rep < 3; rep++) {
            double s = now();
            for (long r = 0; r < R; r++) {
                unsigned long long w = 0xfedcba9876543210ull + r, hi0, hi1, hi2, hi3, carry = 0;
                unsigned char cf = 0;
                for (int i = 0; i < 512; i += 4) {
                    unsigned long long l0 = _mulx_u64(x[i], w, &hi0), l1 = _mulx_u64(x[i + 1], w, &hi1);
                    unsigned long long l2 = _mulx_u64(x[i + 2], w, &hi2), l3 = _mulx_u64(x[i + 3], w, &hi3);
                    unsigned long long o0, o1, o2, o3;
                    cf = _addcarry_u64(cf, l0, carry, &o0);
                    cf = _addcarry_u64(cf, l1, hi0, &o1);
                    cf = _addcarry_u64(cf, l2, hi1, &o2);
                    cf = _addcarry_u64(cf, l3, hi2, &o3);
                    out[i] = o0; out[i + 1] = o1; out[i + 2] = o2; out[i + 3] = o3;
                    carry = hi3;
                }
                out[512] = carry + cf;
                __asm__ volatile("" : : "r"(out) : "memory");
            }
            double e = now() - s;
            if (e < best_w) best_w = e;
        }
        sink += out[100];
        printf("mul_word %.3f ns_per_word\n", best_w / R / 512 * 1e9);
    }

    // memory latency: a chain of dependent loads over 256 MiB
    {
        size_t n = BYTES / 64;
        uint64_t *next = (uint64_t *)a;
        // a random cycle through the lines (Sattolo)
        uint64_t *perm = malloc(n * 8);
        for (size_t i = 0; i < n; i++) perm[i] = i;
        uint64_t h = 88172645463325252ull;
        for (size_t i = n - 1; i > 0; i--) {
            h ^= h << 13; h ^= h >> 7; h ^= h << 17;
            size_t j = h % i;
            uint64_t tmp = perm[i]; perm[i] = perm[j]; perm[j] = tmp;
        }
        for (size_t i = 0; i < n; i++) next[perm[i] * 8] = perm[(i + 1) % n] * 8;
        const long L = 1l << 22;
        uint64_t p = 0;
        double s = now();
        for (long i = 0; i < L; i++) p = next[p];
        double e = now() - s;
        sink += p;
        printf("memory_latency %.1f ns\n", e / L * 1e9);
        free(perm);
    }
    return (int)(sink & 0);
}
