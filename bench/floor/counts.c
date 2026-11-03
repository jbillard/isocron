// The work the programs cannot avoid, counted (DECISIONS D182):
//   counts fannkuch <n>       the flips of all the permutations of n
//   counts mandelbrot <n>     the iterations of the n x n pixels (at most 50)
//   counts kmers <fasta>      the k-mers of the sequence THREE, distinct ones
#include <omp.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int cmp64(const void *a, const void *b) {
    uint64_t x = *(const uint64_t *)a, y = *(const uint64_t *)b;
    return x < y ? -1 : x > y;
}

static void fannkuch(int n) {
    uint64_t total = 1;
    for (int i = 2; i <= n; i++) total *= i;
    uint64_t flips = 0, maxflips = 0;
    const int chunks = 4096;
#pragma omp parallel for schedule(dynamic) reduction(+ : flips) reduction(max : maxflips)
    for (int c = 0; c < chunks; c++) {
        uint64_t lo = total * c / chunks, hi = total * (c + 1) / chunks;
        // the permutation of rank lo (factorial number system, the order
        // of the reference programs)
        int perm[16], count[16], p[16];
        uint64_t r = lo, fact[16];
        fact[0] = 1;
        for (int i = 1; i <= n; i++) fact[i] = fact[i - 1] * i;
        for (int i = 0; i < n; i++) p[i] = i;
        for (int i = n - 1; i > 0; i--) {
            uint64_t d = r / fact[i];
            r %= fact[i];
            count[i] = (int)d;
            int t[16];
            for (int j = 0; j <= i; j++) t[j] = p[(j + d) % (i + 1)];
            for (int j = 0; j <= i; j++) p[j] = t[j];
        }
        for (uint64_t k = lo; k < hi; k++) {
            if (p[0] != 0) {
                for (int i = 0; i < n; i++) perm[i] = p[i];
                uint64_t f = 0;
                while (perm[0] != 0) {
                    int k2 = perm[0];
                    for (int a = 0, b = k2; a < b; a++, b--) {
                        int x = perm[a]; perm[a] = perm[b]; perm[b] = x;
                    }
                    f++;
                }
                flips += f;
                if (f > maxflips) maxflips = f;
            }
            // the next permutation
            for (int i = 1; i < n; i++) {
                int first = p[0];
                for (int j = 0; j < i; j++) p[j] = p[j + 1];
                p[i] = first;
                if (++count[i] <= i) break;
                count[i] = 0;
            }
        }
    }
    printf("fannkuch n=%d permutations %llu flips %llu maxflips %llu\n", n, (unsigned long long)total,
           (unsigned long long)flips, (unsigned long long)maxflips);
}

static void mandelbrot(int n) {
    uint64_t iters = 0, inside = 0;
#pragma omp parallel for schedule(dynamic) reduction(+ : iters, inside)
    for (int y = 0; y < n; y++) {
        double ci = 2.0 * y / n - 1.0;
        for (int x = 0; x < n; x++) {
            double cr = 2.0 * x / n - 1.5, zr = 0, zi = 0;
            int i = 0;
            for (; i < 50; i++) {
                double tr = zr * zr - zi * zi + cr, ti = 2 * zr * zi + ci;
                zr = tr;
                zi = ti;
                if (zr * zr + zi * zi > 4.0) { i++; break; }
            }
            iters += i;
            inside += (i >= 50);
        }
    }
    printf("mandelbrot n=%d pixels %llu iterations %llu at_50 %llu\n", n, (unsigned long long)n * n,
           (unsigned long long)iters, (unsigned long long)inside);
}

static void kmers(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) { perror(path); exit(1); }
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = malloc(size + 1);
    if (fread(buf, 1, size, f) != (size_t)size) exit(1);
    buf[size] = 0;
    char *s = strstr(buf, ">THREE");
    s = strchr(s, '\n') + 1;
    uint8_t *seq = malloc(size);
    long m = 0;
    for (char *q = s; q < buf + size; q++) {
        switch (*q) {
            case 'a': case 'A': seq[m++] = 0; break;
            case 'c': case 'C': seq[m++] = 1; break;
            case 'g': case 'G': seq[m++] = 2; break;
            case 't': case 'T': seq[m++] = 3; break;
        }
    }
    printf("kmers bases %ld\n", m);
    int ks[] = {1, 2, 3, 4, 6, 12, 18};
    uint64_t *codes = malloc(m * 8);
    for (int a = 0; a < 7; a++) {
        int k = ks[a];
        long c = m - k + 1;
        uint64_t code = 0, mask = k == 32 ? ~0ull : ((1ull << (2 * k)) - 1);
        for (long i = 0; i < k - 1; i++) code = (code << 2) | seq[i];
        for (long i = k - 1; i < m; i++) {
            code = ((code << 2) | seq[i]) & mask;
            codes[i - k + 1] = code;
        }
        qsort(codes, c, 8, cmp64);
        long distinct = c > 0;
        for (long i = 1; i < c; i++) distinct += codes[i] != codes[i - 1];
        printf("kmers k=%d windows %ld distinct %ld\n", k, c, distinct);
    }
}

int main(int argc, char **argv) {
    if (argc < 3) return 2;
    if (!strcmp(argv[1], "fannkuch")) fannkuch(atoi(argv[2]));
    else if (!strcmp(argv[1], "mandelbrot")) mandelbrot(atoi(argv[2]));
    else if (!strcmp(argv[1], "kmers")) kmers(argv[2]);
    return 0;
}
