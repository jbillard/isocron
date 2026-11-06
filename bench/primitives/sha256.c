// The floor of `sha256.isc`: the same work on the same bytes, the loop of
// FIPS 180-4 written in C the cheapest way known on one thread, without the
// instructions SHA of the processor (the module has none: SPEC section 16,
// D329). The lower bound with those instructions (SHA-NI, `digest.4k.shani`)
// is printed after, outside the table. Each line: a name, the nanoseconds of
// one call (the mean over `rounds`), a checksum.
//   sha256 <rounds>
#define _GNU_SOURCE
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static volatile long sink;

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

static const uint32_t K[64] = {
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
};

#define ROTR(x, n) (((x) >> (n)) | ((x) << (32 - (n))))

static void block(uint32_t h[8], const uint8_t *p) {
    uint32_t w[64];
    for (int t = 0; t < 16; t++) w[t] = (uint32_t)p[4 * t] << 24 | (uint32_t)p[4 * t + 1] << 16 | (uint32_t)p[4 * t + 2] << 8 | p[4 * t + 3];
    for (int t = 16; t < 64; t++) {
        uint32_t s0 = ROTR(w[t - 15], 7) ^ ROTR(w[t - 15], 18) ^ (w[t - 15] >> 3);
        uint32_t s1 = ROTR(w[t - 2], 17) ^ ROTR(w[t - 2], 19) ^ (w[t - 2] >> 10);
        w[t] = w[t - 16] + s0 + w[t - 7] + s1;
    }
    uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
    for (int t = 0; t < 64; t++) {
        uint32_t t1 = hh + (ROTR(e, 6) ^ ROTR(e, 11) ^ ROTR(e, 25)) + ((e & f) ^ (~e & g)) + K[t] + w[t];
        uint32_t t2 = (ROTR(a, 2) ^ ROTR(a, 13) ^ ROTR(a, 22)) + ((a & b) ^ (a & c) ^ (b & c));
        hh = g, g = f, f = e, e = d + t1, d = c, c = b, b = a, a = t1 + t2;
    }
    h[0] += a, h[1] += b, h[2] += c, h[3] += d, h[4] += e, h[5] += f, h[6] += g, h[7] += hh;
}

// the last blocks: the bytes left, 0x80, zeros, the length in bits
static void finish(uint32_t h[8], const uint8_t *m, size_t n, void (*f)(uint32_t *, const uint8_t *)) {
    uint8_t last[128] = {0};
    size_t rest = n % 64, full = n - rest;
    for (size_t i = 0; i < full; i += 64) f(h, m + i);
    memcpy(last, m + full, rest);
    last[rest] = 0x80;
    size_t len = rest + 9 <= 64 ? 64 : 128;
    uint64_t bits = (uint64_t)n * 8;
    for (int i = 0; i < 8; i++) last[len - 1 - i] = bits >> (8 * i);
    for (size_t i = 0; i < len; i += 64) f(h, last + i);
}

static void digest(const uint8_t *m, size_t n, uint8_t out[32], void (*f)(uint32_t *, const uint8_t *)) {
    uint32_t h[8] = {0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19};
    finish(h, m, n, f);
    for (int i = 0; i < 32; i++) out[i] = h[i / 4] >> (24 - 8 * (i % 4));
}

// a block by the instructions SHA (the lower bound, outside the table)
__attribute__((target("sha,sse4.1"))) static void block_ni(uint32_t h[8], const uint8_t *p) {
    const __m128i mask = _mm_set_epi64x(0x0c0d0e0f08090a0bULL, 0x0405060700010203ULL);
    __m128i tmp = _mm_loadu_si128((const __m128i *)&h[0]);
    __m128i st1 = _mm_loadu_si128((const __m128i *)&h[4]);
    tmp = _mm_shuffle_epi32(tmp, 0xB1);
    st1 = _mm_shuffle_epi32(st1, 0x1B);
    __m128i st0 = _mm_alignr_epi8(tmp, st1, 8);
    st1 = _mm_blend_epi16(st1, tmp, 0xF0);
    __m128i a0 = st0, b0 = st1, msg[4], m;
    for (int i = 0; i < 4; i++) msg[i] = _mm_shuffle_epi8(_mm_loadu_si128((const __m128i *)(p + 16 * i)), mask);
    for (int r = 0; r < 16; r++) {
        __m128i w = msg[r % 4];
        m = _mm_add_epi32(w, _mm_loadu_si128((const __m128i *)&K[4 * r]));
        st1 = _mm_sha256rnds2_epu32(st1, st0, m);
        st0 = _mm_sha256rnds2_epu32(st0, st1, _mm_shuffle_epi32(m, 0x0E));
        if (r < 12) {
            __m128i x = _mm_sha256msg1_epu32(msg[r % 4], msg[(r + 1) % 4]);
            x = _mm_add_epi32(x, _mm_alignr_epi8(msg[(r + 3) % 4], msg[(r + 2) % 4], 4));
            msg[r % 4] = _mm_sha256msg2_epu32(x, msg[(r + 3) % 4]);
        }
    }
    st0 = _mm_add_epi32(st0, a0);
    st1 = _mm_add_epi32(st1, b0);
    tmp = _mm_shuffle_epi32(st0, 0x1B);
    st1 = _mm_shuffle_epi32(st1, 0xB1);
    st0 = _mm_blend_epi16(tmp, st1, 0xF0);
    st1 = _mm_alignr_epi8(st1, tmp, 8);
    _mm_storeu_si128((__m128i *)&h[0], st0);
    _mm_storeu_si128((__m128i *)&h[4], st1);
}

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    static uint8_t big[4096], small[55];
    for (int i = 0; i < 4096; i++) big[i] = (uint8_t)(i * 7 + 3);
    for (int i = 0; i < 55; i++) small[i] = (uint8_t)(i * 7 + 3);
    uint8_t h[32];
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < r; k++) {
        digest(big, 4096, h, block);
        acc = h[0] + h[31] * 256;
    }
    printf("digest.4k %.3f %ld\n", (now() - t0) * 1e9 / r, acc);
    t0 = now();
    for (long k = 0; k < r; k++) {
        digest(small, 55, h, block);
        acc = h[0] + h[31] * 256;
    }
    printf("digest.55 %.3f %ld\n", (now() - t0) * 1e9 / r, acc);
    digest(big, 4096, h, block);
    static const char hexd[] = "0123456789abcdef";
    char text[64];
    t0 = now();
    for (long k = 0; k < r; k++) {
        // the 64 digits in AVX2: the nibbles, one shuffle of the 16 digits
        // each, interleaved (D392: the loop of the bytes was beaten)
        __m256i v = _mm256_loadu_si256((const __m256i *)h), lo = _mm256_and_si256(v, _mm256_set1_epi8(15));
        __m256i hi = _mm256_and_si256(_mm256_srli_epi16(v, 4), _mm256_set1_epi8(15));
        __m256i digits = _mm256_broadcastsi128_si256(_mm_loadu_si128((const __m128i *)hexd));
        __m256i hd = _mm256_shuffle_epi8(digits, hi), ld = _mm256_shuffle_epi8(digits, lo);
        __m256i a0 = _mm256_unpacklo_epi8(hd, ld), a1 = _mm256_unpackhi_epi8(hd, ld);
        _mm256_storeu_si256((__m256i *)text, _mm256_permute2x128_si256(a0, a1, 0x20));
        _mm256_storeu_si256((__m256i *)(text + 32), _mm256_permute2x128_si256(a0, a1, 0x31));
        sink += text[k % 64];
        acc = 64;
    }
    printf("hex.32 %.3f %ld\n", (now() - t0) * 1e9 / r, acc);
    // a text of 55 bytes (D392)
    static uint8_t words[55];
    for (int i = 0; i < 55; i++) words[i] = (uint8_t)('a' + i % 26);
    t0 = now();
    for (long k = 0; k < r; k++) {
        digest(words, 55, h, block);
        acc = h[0] + h[31] * 256;
    }
    printf("digestText.55 %.3f %ld\n", (now() - t0) * 1e9 / r, acc);
    t0 = now();
    for (long k = 0; k < r; k++) {
        digest(big, 4096, h, block_ni);
        acc = h[0] + h[31] * 256;
    }
    printf("digest.4k.shani %.3f %ld\n", (now() - t0) * 1e9 / r, acc);
    return 0;
}
