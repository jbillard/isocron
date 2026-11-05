// The constants of the machine that the floors of the programs are made of
// (DECISIONS D182, D222), measured without Isocron: the best of R runs of
// each. A constant is the cost of a primitive of the machine done the
// cheapest way known, never the time of one program.
//   floor <threads> [file]
// `file`: a file read through the page cache (the input of
// reverse-complement and k-nucleotide).
// Each line: `name value unit`. Run after `bench/machine.sh apply parallel`
// (fixed frequency) through `bench/floor/run.sh`.
#define _GNU_SOURCE
#include <immintrin.h>
#include <omp.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

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

// a node visited by an explicit stack: every node once, its pointers followed
static long walk(const struct node *root) {
    const struct node *stack[128];
    int sp = 0;
    long c = 0;
    stack[sp++] = root;
    while (sp) {
        const struct node *n = stack[--sp];
        c++;
        if (n->l) {
            stack[sp++] = n->r;
            stack[sp++] = n->l;
        }
    }
    return c;
}

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

    // ---- D222: the primitives of the floors redone ----

    // a file of the page cache read by every thread, each its part with
    // `pread`: into new pages, into pages already written, and by blocks of
    // 256 KiB into a buffer of each thread (the bytes stay in its caches)
    if (argc > 2) {
        int fd = open(argv[2], O_RDONLY);
        struct stat st;
        if (fd < 0 || fstat(fd, &st) != 0) { perror(argv[2]); exit(1); }
        size_t n = st.st_size;
        unsigned char *w = mapped(n + 4096);
        memset(w, 3, n);
        for (size_t o = 0; o < n;) { ssize_t r = pread(fd, w, n - o < (64u << 20) ? n - o : (64u << 20), o); if (r <= 0) break; o += r; }
        const size_t BLOCK = 256u << 10;
        unsigned char *small = mapped(BLOCK * t);
        memset(small, 0, BLOCK * t);
        for (int mode = 0; mode < 3; mode++) {
            best = 1e9;
            for (int r = 0; r < REPS; r++) {
                unsigned char *dst = mode == 0 ? mapped(n + 4096) : w;
                double s = now();
#pragma omp parallel for
                for (int k = 0; k < t; k++) {
                    size_t lo = n / t * k, hi = k == t - 1 ? n : n / t * (k + 1);
                    while (lo < hi) {
                        size_t m = hi - lo;
                        if (mode == 2 && m > BLOCK) m = BLOCK;
                        ssize_t got = pread(fd, mode == 2 ? small + (size_t)k * BLOCK : dst + lo, m, lo);
                        if (got <= 0) break;
                        lo += got;
                    }
                }
                double e = now() - s;
                if (e < best) best = e;
                if (mode == 0) munmap(dst, n + 4096);
            }
            printf("%s %.1f GB/s\n", mode == 0 ? "file_read_new" : mode == 1 ? "file_read_written" : "file_read_blocks", n / best / 1e9);
        }
        munmap(small, BLOCK * t);
        munmap(w, n + 4096);
        close(fd);
    }

    // an array of 256 MiB changed in place: each line loaded, a table
    // lookup of its bytes (vpshufb), stored back
    {
        best = 1e9;
        __m512i tbl = _mm512_set_epi8(15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6,
                                      5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10,
                                      9, 8, 7, 6, 5, 4, 3, 2, 1, 0);
        for (int r = 0; r < REPS; r++) {
            double s = now();
#pragma omp parallel for
            for (int k = 0; k < t; k++) {
                __m512i *p = (__m512i *)(a + k * chunk);
                for (size_t i = 0; i < chunk / 64; i++) _mm512_store_si512(p + i, _mm512_shuffle_epi8(tbl, _mm512_load_si512(p + i)));
            }
            double e = now() - s;
            if (e < best) best = e;
        }
        printf("update_in_place %.1f GB/s\n", BYTES / best / 1e9);
    }

    // fannkuch-redux: flips of permutations of 12, 4 permutations of 16 bytes
    // in a vector of 64, 8 vectors interleaved per thread: the index of a
    // flip computed in the vector (f - i below f, i above), one vpshufb; a
    // permutation done (first element 0) replaced by the next of a pool
    // (an expanding load). ns per real flip (first element not 0).
    {
        const size_t POOL = 1u << 20;
        unsigned char *pool = mapped(POOL * 16 + 4096);
        uint64_t h = 0x2545f4914f6cdd1dull;
        for (size_t i = 0; i < POOL; i++) {
            unsigned char *q = pool + i * 16;
            for (int j = 0; j < 16; j++) q[j] = j;
            for (int j = 11; j > 0; j--) {
                h ^= h << 13; h ^= h >> 7; h ^= h << 17;
                int x = h % (j + 1);
                unsigned char tmp = q[j]; q[j] = q[x]; q[x] = tmp;
            }
        }
        const __m512i iota = _mm512_set_epi8(15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8, 7,
                                             6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 15, 14, 13, 12,
                                             11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0);
        const __m512i zero = _mm512_setzero_si512();
        const __mmask64 firsts = 0x0001000100010001ull;
        const long STEPS = 1l << 27;
        best = 1e9;
        long flips_best = 1;
        for (int r = 0; r < 3; r++) {
            long flips = 0;
            double s = now();
#pragma omp parallel for reduction(+ : flips)
            for (int k = 0; k < t; k++) {
                const unsigned char *src = pool + (size_t)k * (POOL / t) * 16;
                size_t next = 16;
                __m512i v0 = _mm512_loadu_si512(src), v1 = _mm512_loadu_si512(src + 64), v2 = _mm512_loadu_si512(src + 128),
                        v3 = _mm512_loadu_si512(src + 192), v4 = _mm512_loadu_si512(src + 256), v5 = _mm512_loadu_si512(src + 320),
                        v6 = _mm512_loadu_si512(src + 384), v7 = _mm512_loadu_si512(src + 448);
                next = 32;
                long fl = 0;
#define FLIP(v)                                                                              \
    {                                                                                        \
        __m512i f = _mm512_shuffle_epi8(v, zero);                                            \
        __mmask64 below = _mm512_cmple_epu8_mask(iota, f);                                   \
        __m512i idx = _mm512_mask_sub_epi8(iota, below, f, iota);                            \
        v = _mm512_shuffle_epi8(v, idx);                                                     \
        __mmask64 live = _mm512_mask_cmpneq_epu8_mask(firsts, f, zero);                      \
        fl += __builtin_popcountll(live);                                                    \
        __mmask8 done = (__mmask8)_pext_u64(~live & firsts, firsts) ;                        \
        __mmask8 dq = (__mmask8)_pdep_u64(done, 0x55) | (__mmask8)(_pdep_u64(done, 0x55) << 1); \
        v = _mm512_mask_expandloadu_epi64(v, dq, src + next * 16);                           \
        next = (next + __builtin_popcount(done)) & (POOL / t - 1);                           \
    }
                for (long i = 0; i < STEPS / t / 32; i++) { FLIP(v0) FLIP(v1) FLIP(v2) FLIP(v3) FLIP(v4) FLIP(v5) FLIP(v6) FLIP(v7) }
#undef FLIP
                __m512i all = _mm512_add_epi64(_mm512_add_epi64(_mm512_add_epi64(v0, v1), _mm512_add_epi64(v2, v3)),
                                               _mm512_add_epi64(_mm512_add_epi64(v4, v5), _mm512_add_epi64(v6, v7)));
                flips += fl + _mm512_reduce_add_epi64(all) % 2;
            }
            double e = now() - s;
            if (e / flips < best / flips_best) { best = e; flips_best = flips; }
        }
        printf("fannkuch_flip %.3f ns_per_flip_per_core\n", best * t / flips_best * 1e9);
        munmap(pool, POOL * 16 + 4096);
    }

    // fannkuch-redux: the next permutation (the counting order of the
    // program), 8 independent generators interleaved per thread, each a
    // rotation of a prefix by a vpshufb from a table. ns per permutation
    {
        __m128i rot[16];
        for (int r = 0; r < 16; r++) {
            unsigned char m[16];
            for (int i = 0; i < 16; i++) m[i] = i < r ? i + 1 : i == r ? 0 : i;
            rot[r] = _mm_loadu_si128((const __m128i *)m);
        }
        const long PERMS = 1l << 26;
        best = 1e9;
        for (int r = 0; r < 3; r++) {
            long chk = 0;
            double s = now();
#pragma omp parallel for reduction(+ : chk)
            for (int k = 0; k < t; k++) {
                __m128i p[8];
                unsigned char cnt[8][16];
                for (int g = 0; g < 8; g++) {
                    p[g] = _mm_set_epi8(15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0);
                    memset(cnt[g], 0, 16);
                    for (int j = 0; j < g + k; j++) p[g] = _mm_shuffle_epi8(p[g], rot[11]);
                }
                long c = 0;
                for (long i = 0; i < PERMS / t / 8; i++)
                    for (int g = 0; g < 8; g++) {
                        int q = 1;
                        p[g] = _mm_shuffle_epi8(p[g], rot[1]);
                        while (++cnt[g][q] > q) {
                            cnt[g][q] = 0;
                            q++;
                            p[g] = _mm_shuffle_epi8(p[g], rot[q]);
                        }
                        c += _mm_cvtsi128_si32(p[g]) & 0xff;
                    }
                chk += c;
            }
            sink += chk;
            double e = now() - s;
            if (e < best) best = e;
        }
        printf("fannkuch_next_permutation %.3f ns_per_permutation_per_core\n", best * t / PERMS * 1e9);
    }

    // a big integer times a word, in assembly: mulx (no flags) and one chain
    // of adc, by 4 words, a load and a store each; ns per word
    {
        static uint64_t x[1024], out[1025];
        for (int i = 0; i < 1024; i++) x[i] = 0x9e3779b97f4a7c15ull * (i + 1);
        const long R = 1l << 17;
        double best_w = 1e9;
        for (int rep = 0; rep < 5; rep++) {
            double s = now();
            for (long r = 0; r < R; r++) {
                uint64_t w = 0xfedcba9876543210ull + r;
                const uint64_t *src = x;
                uint64_t *dst = out;
                long n4 = 1024 / 4;
                __asm__ volatile(
                    "xor %%r8d, %%r8d\n"
                    "1:\n"
                    "mulx (%[s]), %%rax, %%r9\n"
                    "mulx 8(%[s]), %%r10, %%r11\n"
                    "adc %%r8, %%rax\n"
                    "mov %%rax, (%[d])\n"
                    "adc %%r9, %%r10\n"
                    "mov %%r10, 8(%[d])\n"
                    "mulx 16(%[s]), %%rax, %%r9\n"
                    "mulx 24(%[s]), %%r10, %%r8\n"
                    "adc %%r11, %%rax\n"
                    "mov %%rax, 16(%[d])\n"
                    "adc %%r9, %%r10\n"
                    "mov %%r10, 24(%[d])\n"
                    "lea 32(%[s]), %[s]\n"
                    "lea 32(%[d]), %[d]\n"
                    "dec %[n]\n"
                    "jnz 1b\n"
                    "adc $0, %%r8\n"
                    "mov %%r8, (%[d])\n"
                    : [s] "+r"(src), [d] "+r"(dst), [n] "+r"(n4)
                    : "d"(w)
                    : "rax", "r8", "r9", "r10", "r11", "cc", "memory");
            }
            double e = now() - s;
            if (e < best_w) best_w = e;
        }
        sink += out[100];
        printf("mul_word_asm %.3f ns_per_word\n", best_w / R / 1024 * 1e9);
    }

    // counting k-mers in a table of 4096 counters (k = 6): the keys of 4
    // parts of a sequence of random bases, each part its own table (no
    // dependence through a counter), rolling keys; ns per update per core
    {
        const size_t BASES = 1u << 24;
        unsigned char *seq = mapped(BASES);
        uint64_t h = 0x9e3779b97f4a7c15ull;
        for (size_t i = 0; i < BASES; i++) { h ^= h << 13; h ^= h >> 7; h ^= h << 17; seq[i] = h & 3; }
        best = 1e9;
        for (int r = 0; r < REPS; r++) {
            long chk = 0;
            double s = now();
#pragma omp parallel for reduction(+ : chk)
            for (int k = 0; k < t; k++) {
                static __thread uint32_t tab[4][4096];
                memset(tab, 0, sizeof tab);
                size_t part = BASES / 4;
                uint32_t k0 = 0, k1 = 0, k2 = 0, k3 = 0;
                for (size_t i = 0; i < part; i++) {
                    k0 = ((k0 << 2) | seq[i]) & 4095; tab[0][k0]++;
                    k1 = ((k1 << 2) | seq[part + i]) & 4095; tab[1][k1]++;
                    k2 = ((k2 << 2) | seq[2 * part + i]) & 4095; tab[2][k2]++;
                    k3 = ((k3 << 2) | seq[3 * part + i]) & 4095; tab[3][k3]++;
                }
                chk += tab[0][5] + tab[3][7];
            }
            sink += chk;
            double e = now() - s;
            if (e < best) best = e;
        }
        printf("count_table_4096 %.3f ns_per_update_per_core\n", best / BASES * 1e9);
        munmap(seq, BASES);
    }

    // binary trees with pointers, made in an arena in breadth-first order
    // (a loop, no recursion), visited by an explicit stack; depth 12 (in the
    // L2 cache) and 20 (in memory); ns per node per core
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
                    struct node *n = arena + k * nodes;
                    long inner = nodes / 2;
                    for (long j = 0; j < inner; j++) { n[j].l = n + 2 * j + 1; n[j].r = n + 2 * j + 2; }
                    for (long j = inner; j < nodes; j++) n[j].l = n[j].r = 0;
                    total += walk(n);
                }
            double e = now() - s;
            sink += total;
            if (e < best) best = e;
        }
        printf("tree_loop_depth%d %.3f ns_per_node_per_core\n", d, best * t / ((trees / t) * t * nodes) * 1e9);
        munmap(arena, sizeof(struct node) * nodes * t);
    }

    // fasta: the generator of the program (x = (x * 3877 + 29573) mod
    // 139968) in 4 vectors of 8 lanes, each lane 32 numbers ahead (the
    // multiplier and the increment of 32 steps; 4 chains interleaved), the
    // remainder by a multiplication in doubles; each number a symbol by
    // comparisons with integer thresholds (15: the IUB table, 3: homo
    // sapiens) on 16 lanes of 32 bits, packed to bytes, stored in a buffer of
    // 64 KiB per thread. ns per number per core
    for (int nth = 15; nth >= 3; nth -= 12) {
        const uint64_t M = 139968, A = 3877, C = 29573;
        uint64_t A32 = 1, C32 = 0;
        for (int i = 0; i < 32; i++) { A32 = A32 * A % M; C32 = (C32 * A + C) % M; }
        const long NUMS = 1l << 28;
        best = 1e9;
        for (int r = 0; r < REPS; r++) {
            long chk = 0;
            double s = now();
#pragma omp parallel for reduction(+ : chk)
            for (int k = 0; k < t; k++) {
                static __thread unsigned char outb[65536 + 64];
                uint64_t x0[32];
                uint64_t y = 42 + k;
                for (int j = 0; j < 32; j++) { y = (y * A + C) % M; x0[j] = y; }
                __m512i x[4];
                for (int v = 0; v < 4; v++) x[v] = _mm512_loadu_si512(x0 + 8 * v);
                const __m512i a32 = _mm512_set1_epi64(A32), c32 = _mm512_set1_epi64(C32), mm = _mm512_set1_epi64(M), z = _mm512_setzero_si512();
                const __m512d inv = _mm512_set1_pd(1.0 / (double)M), md = _mm512_set1_pd((double)M);
                __m512i th[15];
                for (int j = 0; j < 15; j++) th[j] = _mm512_set1_epi32((int)(M * (j + 1) / 16));
                size_t at = 0;
                for (long i = 0; i < NUMS / t / 32; i++) {
                    for (int v = 0; v < 4; v++) {
                        __m512i p = _mm512_add_epi64(_mm512_mul_epu32(x[v], a32), c32);
                        __m512d q = _mm512_roundscale_pd(_mm512_mul_pd(_mm512_cvtepu64_pd(p), inv), _MM_FROUND_TO_NEG_INF);
                        __m512i rr = _mm512_sub_epi64(p, _mm512_cvtpd_epu64(_mm512_mul_pd(q, md)));
                        rr = _mm512_mask_add_epi64(rr, _mm512_cmplt_epi64_mask(rr, z), rr, mm);
                        x[v] = _mm512_mask_sub_epi64(rr, _mm512_cmpge_epi64_mask(rr, mm), rr, mm);
                    }
                    for (int h = 0; h < 2; h++) {
                        __m512i w = _mm512_inserti64x4(_mm512_castsi256_si512(_mm512_cvtepi64_epi32(x[2 * h])), _mm512_cvtepi64_epi32(x[2 * h + 1]), 1);
                        __m512i sym = _mm512_setzero_si512();
                        for (int j = 0; j < nth; j++) sym = _mm512_mask_sub_epi32(sym, _mm512_cmpge_epu32_mask(w, th[j]), sym, _mm512_set1_epi32(-1));
                        _mm_storeu_si128((__m128i *)(outb + at), _mm512_cvtepi32_epi8(sym));
                        at = (at + 16) & 65535;
                    }
                }
                chk += outb[100] + outb[60000];
            }
            sink += chk;
            double e = now() - s;
            if (e < best) best = e;
        }
        printf("fasta_number_%s %.3f ns_per_number_per_core\n", nth == 15 ? "iub" : "homo", best * t / NUMS * 1e9);
    }

    // regex-redux: the matches of two literals of 8 bytes in a text of random
    // bases (agggtaaa|tttaccct), in blocks of 256 KiB in the caches of each
    // thread: per 64 bytes, the masks of the 4 bases (4 comparisons); a
    // literal at position p is the and of the masks of its bytes shifted by
    // their places, across two blocks (shrd), in general registers; ns per
    // byte per core
    {
        const size_t BLOCK = 256u << 10;
        unsigned char *txt = mapped(BLOCK * t + 128);
        const char *bases = "acgt";
        uint64_t h = 0x9e3779b97f4a7c15ull;
        for (size_t i = 0; i < BLOCK * t + 128; i++) { h ^= h << 13; h ^= h >> 7; h ^= h << 17; txt[i] = bases[h & 3]; }
        const long PASSES = 1l << 12;
        best = 1e9;
        long found_check = 0;
        for (int r = 0; r < REPS; r++) {
            long found = 0;
            double s = now();
#pragma omp parallel for reduction(+ : found)
            for (int k = 0; k < t; k++) {
                const unsigned char *b = txt + (size_t)k * BLOCK;
                long f = 0;
                const __m512i va = _mm512_set1_epi8('a'), vc = _mm512_set1_epi8('c'), vg = _mm512_set1_epi8('g'), vt = _mm512_set1_epi8('t');
                for (long pass = 0; pass < PASSES / t; pass++) {
                    __m512i v = _mm512_loadu_si512(b);
                    uint64_t pa = _mm512_cmpeq_epi8_mask(v, va), pc = _mm512_cmpeq_epi8_mask(v, vc), pg = _mm512_cmpeq_epi8_mask(v, vg),
                             pt = _mm512_cmpeq_epi8_mask(v, vt);
                    for (size_t i = 64; i <= BLOCK; i += 64) {
                        v = _mm512_loadu_si512(b + i);
                        uint64_t na = _mm512_cmpeq_epi8_mask(v, va), nc = _mm512_cmpeq_epi8_mask(v, vc), ng = _mm512_cmpeq_epi8_mask(v, vg),
                                 nt = _mm512_cmpeq_epi8_mask(v, vt);
#define SH(m, n, j) (j == 0 ? m : (m >> j) | (n << (64 - j)))
                        // agggtaaa
                        uint64_t m1 = SH(pa, na, 0) & SH(pg, ng, 1) & SH(pg, ng, 2) & SH(pg, ng, 3) & SH(pt, nt, 4) & SH(pa, na, 5) &
                                      SH(pa, na, 6) & SH(pa, na, 7);
                        // tttaccct
                        uint64_t m2 = SH(pt, nt, 0) & SH(pt, nt, 1) & SH(pt, nt, 2) & SH(pa, na, 3) & SH(pc, nc, 4) & SH(pc, nc, 5) &
                                      SH(pc, nc, 6) & SH(pt, nt, 7);
#undef SH
                        f += __builtin_popcountll(m1 | m2);
                        pa = na; pc = nc; pg = ng; pt = nt;
                    }
                }
                found += f;
            }
            sink += found;
            found_check = found;
            double e = now() - s;
            if (e < best) best = e;
        }
        // the same count by bytes: the scan finds every match
        long naive = 0;
        for (int k = 0; k < t; k++)
            for (size_t i = (size_t)k * BLOCK; i < (size_t)k * BLOCK + BLOCK; i++)
                naive += memcmp(txt + i, "agggtaaa", 8) == 0 || memcmp(txt + i, "tttaccct", 8) == 0;
        if (found_check != naive * (PASSES / t)) { fprintf(stderr, "literal_pair_scan: %ld matches, %ld expected\n", found_check, naive * (PASSES / t)); exit(1); }
        printf("literal_pair_scan %.4f ns_per_byte_per_core\n", best * t / ((double)(PASSES / t) * t * BLOCK) * 1e9);
        munmap(txt, BLOCK * t + 128);
    }

    // regex-redux: the same matches by groups of 8 blocks (D311), cheaper:
    // the classes of each byte looked up in a table (bit c: base c), the bits
    // of each lane of 8 bytes transposed (gf2p8affineqb), so that a block's
    // classes are the lanes of one vector; the 8 vectors of a group
    // transposed, a lane per block; each place of a literal the lane of its
    // base shifted by its place with the next block's (the lanes moved by
    // one), anded lane by lane: no reduction of lanes; ns per byte per core
    {
        const size_t BLOCK = 256u << 10;
        unsigned char *txt = mapped(BLOCK * t + 1024);
        const char *bases = "acgt";
        uint64_t h = 0x9e3779b97f4a7c15ull;
        for (size_t i = 0; i < BLOCK * t + 1024; i++) { h ^= h << 13; h ^= h >> 7; h ^= h << 17; txt[i] = bases[h & 3]; }
        unsigned char table[128] = {0};
        table['a'] = 1, table['c'] = 2, table['g'] = 4, table['t'] = 8;
        const __m512i lo = _mm512_loadu_si512(table), hi = _mm512_loadu_si512(table + 64);
        const __m512i rev = _mm512_set_epi64(0x38393a3b3c3d3e3fll, 0x3031323334353637ll, 0x28292a2b2c2d2e2fll, 0x2021222324252627ll,
                                             0x18191a1b1c1d1e1fll, 0x1011121314151617ll, 0x08090a0b0c0d0e0fll, 0x0001020304050607ll);
        const __m512i sel = _mm512_set1_epi64(0x8040201008040201ll);
        const __m512i gather = _mm512_set_epi64(0x3f372f271f170f07ll, 0x3e362e261e160e06ll, 0x3d352d251d150d05ll, 0x3c342c241c140c04ll,
                                                0x3b332b231b130b03ll, 0x3a322a221a120a02ll, 0x3931292119110901ll, 0x3830282018100800ll);
        const __m512i i0 = _mm512_set_epi64(13, 12, 5, 4, 9, 8, 1, 0), i1 = _mm512_set_epi64(15, 14, 7, 6, 11, 10, 3, 2);
        const __m512i j0 = _mm512_set_epi64(11, 10, 9, 8, 3, 2, 1, 0);
        const long PASSES = 1l << 12;
        best = 1e9;
        long found_check = 0;
        for (int r = 0; r < REPS; r++) {
            long found = 0;
            double s = now();
#pragma omp parallel for reduction(+ : found)
            for (int k = 0; k < t; k++) {
                const unsigned char *b = txt + (size_t)k * BLOCK;
                long f = 0;
                for (long pass = 0; pass < PASSES / t; pass++) {
                    __m512i cur[4], nxt[4];
#define LANES(at, out)                                                                                          \
    do {                                                                                                        \
        __m512i l[8];                                                                                           \
        for (int q = 0; q < 8; q++) {                                                                           \
            __m512i x = _mm512_loadu_si512((at) + 64 * q);                                                      \
            __m512i c = _mm512_permutex2var_epi8(lo, x, hi);                                                    \
            l[q] = _mm512_permutexvar_epi8(gather, _mm512_gf2p8affine_epi64_epi8(sel, _mm512_permutexvar_epi8(rev, c), 0)); \
        }                                                                                                       \
        __m512i t0 = _mm512_unpacklo_epi64(l[0], l[1]), t1 = _mm512_unpackhi_epi64(l[0], l[1]);                \
        __m512i t2 = _mm512_unpacklo_epi64(l[2], l[3]), t3 = _mm512_unpackhi_epi64(l[2], l[3]);                \
        __m512i t4 = _mm512_unpacklo_epi64(l[4], l[5]), t5 = _mm512_unpackhi_epi64(l[4], l[5]);                \
        __m512i t6 = _mm512_unpacklo_epi64(l[6], l[7]), t7 = _mm512_unpackhi_epi64(l[6], l[7]);                \
        __m512i u0 = _mm512_permutex2var_epi64(t0, i0, t2), u1 = _mm512_permutex2var_epi64(t1, i0, t3);        \
        __m512i u2 = _mm512_permutex2var_epi64(t0, i1, t2), u3 = _mm512_permutex2var_epi64(t1, i1, t3);        \
        __m512i u4 = _mm512_permutex2var_epi64(t4, i0, t6), u5 = _mm512_permutex2var_epi64(t5, i0, t7);        \
        __m512i u6 = _mm512_permutex2var_epi64(t4, i1, t6), u7 = _mm512_permutex2var_epi64(t5, i1, t7);        \
        (out)[0] = _mm512_permutex2var_epi64(u0, j0, u4), (out)[1] = _mm512_permutex2var_epi64(u1, j0, u5);    \
        (out)[2] = _mm512_permutex2var_epi64(u2, j0, u6), (out)[3] = _mm512_permutex2var_epi64(u3, j0, u7);    \
    } while (0)
                    LANES(b, cur);
                    for (size_t i = 0; i + 512 <= BLOCK; i += 512) {
                        LANES(b + i + 512, nxt);
                        __m512i nb[4];
                        for (int c = 0; c < 4; c++) nb[c] = _mm512_alignr_epi64(nxt[c], cur[c], 1);
#define SH(c, j) (j == 0 ? cur[c] : _mm512_or_si512(_mm512_srli_epi64(cur[c], j), _mm512_slli_epi64(nb[c], 64 - j)))
                        // agggtaaa, tttaccct: a 0, c 1, g 2, t 3
                        __m512i m1 = _mm512_and_si512(_mm512_and_si512(_mm512_and_si512(SH(0, 0), SH(2, 1)), _mm512_and_si512(SH(2, 2), SH(2, 3))),
                                                      _mm512_and_si512(_mm512_and_si512(SH(3, 4), SH(0, 5)), _mm512_and_si512(SH(0, 6), SH(0, 7))));
                        __m512i m2 = _mm512_and_si512(_mm512_and_si512(_mm512_and_si512(SH(3, 0), SH(3, 1)), _mm512_and_si512(SH(3, 2), SH(0, 3))),
                                                      _mm512_and_si512(_mm512_and_si512(SH(1, 4), SH(1, 5)), _mm512_and_si512(SH(1, 6), SH(3, 7))));
#undef SH
                        f += _mm512_reduce_add_epi64(_mm512_popcnt_epi64(_mm512_or_si512(m1, m2)));
                        for (int c = 0; c < 4; c++) cur[c] = nxt[c];
                    }
#undef LANES
                }
                found += f;
            }
            sink += found;
            found_check = found;
            double e = now() - s;
            if (e < best) best = e;
        }
        // the same count by bytes: the scan finds every match
        long naive = 0;
        for (int k = 0; k < t; k++)
            for (size_t i = (size_t)k * BLOCK; i < (size_t)k * BLOCK + BLOCK; i++)
                naive += memcmp(txt + i, "agggtaaa", 8) == 0 || memcmp(txt + i, "tttaccct", 8) == 0;
        if (found_check != naive * (PASSES / t)) { fprintf(stderr, "literal_lanes_scan: %ld matches, %ld expected\n", found_check, naive * (PASSES / t)); exit(1); }
        printf("literal_lanes_scan %.4f ns_per_byte_per_core\n", best * t / ((double)(PASSES / t) * t * BLOCK) * 1e9);
        munmap(txt, BLOCK * t + 1024);
    }

    // fasta: lines of 60 bytes and a newline made from a table of 139968
    // bytes read in turn (one period of the generator, its symbols), into a
    // buffer of 256 KiB per thread (the output written by blocks that stay
    // in the caches); ns per byte of output per core
    {
        const size_t TABLE = 139968, BLOCK = 256u << 10;
        unsigned char *tab = mapped(TABLE + 64);
        for (size_t i = 0; i < TABLE; i++) tab[i] = "acgt"[(i * 2654435761u >> 7) & 3];
        unsigned char *outs = mapped(BLOCK * t + 128);
        memset(outs, 0, BLOCK * t);
        const long BYTES_OUT = 1l << 30;
        best = 1e9;
        for (int r = 0; r < REPS; r++) {
            long chk = 0;
            double s = now();
#pragma omp parallel for reduction(+ : chk)
            for (int k = 0; k < t; k++) {
                unsigned char *o = outs + (size_t)k * BLOCK;
                size_t at = (size_t)k * 1000 % TABLE, made = 0;
                while (made < (size_t)(BYTES_OUT / t)) {
                    size_t pos = 0;
                    while (pos + 61 <= BLOCK) {
                        if (at + 60 <= TABLE) {
                            memcpy(o + pos, tab + at, 60);
                            at += 60;
                        } else {
                            size_t a = TABLE - at;
                            memcpy(o + pos, tab + at, a);
                            memcpy(o + pos + a, tab, 60 - a);
                            at = 60 - a;
                        }
                        o[pos + 60] = '\n';
                        pos += 61;
                    }
                    __asm__ volatile("" : : "r"(o) : "memory");
                    made += pos;
                }
                chk += o[100] + o[5000];
            }
            sink += chk;
            double e = now() - s;
            if (e < best) best = e;
        }
        printf("table_rows %.4f ns_per_byte_per_core\n", best * t / BYTES_OUT * 1e9);
        munmap(tab, TABLE + 64);
        munmap(outs, BLOCK * t + 128);
    }

    return (int)(sink & 0);
}
