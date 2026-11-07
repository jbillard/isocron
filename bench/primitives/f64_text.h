// A double as the shortest decimal that reads back to it (D415): Schubfach
// (R. Giulietti, "The Schubfach way to render doubles", the algorithm of
// Java's `Double.toString`), laid out as Rust's `Display` writes it (the
// text of Isocron: no exponent, `-0`, digits then zeros, or a point). The
// floor of the writing of numbers of `json` (`encode`, `stringify`): the
// cheapest way known, where `%.17g` of `snprintf` made Isocron faster than
// its floor.
#include <stdint.h>
#include <string.h>
#include "pow10_table.h"

#define F64_K_MIN (-324)

static inline uint64_t f64t_mulhi(uint64_t a, uint64_t b) { return (uint64_t)(((unsigned __int128)a * b) >> 64); }
static inline int f64t_flog10pow2(int q) { return (int)(((int64_t)q * 661971961083LL) >> 41); }
static inline int f64t_flog10three4(int q) { return (int)(((int64_t)q * 661971961083LL - 274743187321LL) >> 41); }
static inline int f64t_flog2pow10(int e) { return (int)(((int64_t)e * 913124641741LL) >> 38); }

static inline uint64_t f64t_rop(uint64_t g1, uint64_t g0, uint64_t cp) {
    const uint64_t m63 = 0x7fffffffffffffffULL;
    uint64_t x1 = f64t_mulhi(g0, cp);
    uint64_t y0 = g1 * cp;
    uint64_t y1 = f64t_mulhi(g1, cp);
    uint64_t z = (y0 >> 1) + x1;
    uint64_t vbp = y1 + (z >> 63);
    return vbp | (((z & m63) + m63) >> 63);
}

// the digits f and the exponent e of the shortest decimal of c 2^q
static inline void f64t_decimal(int q, uint64_t c, int dk, uint64_t *f, int *e) {
    const uint64_t c_min = 1ULL << 52;
    uint64_t out = c & 1, cb = c << 2, cbr = cb + 2, cbl;
    int k;
    if (c != c_min || q == -1074) {
        cbl = cb - 2;
        k = f64t_flog10pow2(q);
    } else {
        cbl = cb - 1;
        k = f64t_flog10three4(q);
    }
    int h = q + f64t_flog2pow10(-k) + 2;
    uint64_t g1 = POW10_G[k - F64_K_MIN][0], g0 = POW10_G[k - F64_K_MIN][1];
    uint64_t vb = f64t_rop(g1, g0, cb << h);
    uint64_t vbl = f64t_rop(g1, g0, cbl << h);
    uint64_t vbr = f64t_rop(g1, g0, cbr << h);
    uint64_t s = vb >> 2;
    if (s >= 100) {
        uint64_t sp10 = 10 * f64t_mulhi(s, 115292150460684698ULL << 4);
        uint64_t tp10 = sp10 + 10;
        int upin = vbl + out <= sp10 << 2;
        int wpin = (tp10 << 2) + out <= vbr;
        if (upin != wpin) {
            *f = upin ? sp10 : tp10;
            *e = k;
            return;
        }
    }
    uint64_t t = s + 1;
    int uin = vbl + out <= s << 2;
    int win = (t << 2) + out <= vbr;
    if (uin != win) {
        *f = uin ? s : t;
        *e = k + dk;
        return;
    }
    int64_t cmp = (int64_t)(vb - ((s + t) << 1));
    *f = cmp < 0 || (cmp == 0 && (s & 1) == 0) ? s : t;
    *e = k + dk;
}

// writes the text of a finite v at p; its length
static inline int f64_text(double v, char *p) {
    uint64_t bits;
    memcpy(&bits, &v, 8);
    char *start = p;
    if (bits >> 63) *p++ = '-';
    uint64_t t = bits & ((1ULL << 52) - 1);
    int bq = (int)((bits >> 52) & 0x7ff);
    uint64_t f;
    int e;
    if (bq == 0 && t == 0) {
        *p++ = '0';
        return (int)(p - start);
    }
    if (bq != 0) {
        int mq = bq - 1075;
        uint64_t c = (1ULL << 52) | t;
        if (mq < 0 && -mq < 53 && ((c >> -mq) << -mq) == c) {
            f = c >> -mq;
            e = 0;
        } else {
            f64t_decimal(mq, c, 0, &f, &e);
        }
    } else if (t < 3) {
        // the two smallest values: Schubfach gives them two digits there (as
        // Java writes 4.9E-324); their shortest forms are 5e-324 and 1e-323
        f = t == 1 ? 5 : 1;
        e = t == 1 ? -324 : -323;
    } else {
        f64t_decimal(-1074, t, 0, &f, &e);
    }
    while (f % 10 == 0) {
        f /= 10;
        e++;
    }
    char d[20];
    int n = 0;
    for (uint64_t x = f; x; x /= 10) d[n++] = (char)('0' + x % 10);
    if (e >= 0) {
        for (int i = n - 1; i >= 0; i--) *p++ = d[i];
        for (int i = 0; i < e; i++) *p++ = '0';
    } else if (n + e > 0) {
        for (int i = n - 1; i >= -e; i--) *p++ = d[i];
        *p++ = '.';
        for (int i = -e - 1; i >= 0; i--) *p++ = d[i];
    } else {
        *p++ = '0';
        *p++ = '.';
        for (int i = 0; i < -(n + e); i++) *p++ = '0';
        for (int i = n - 1; i >= 0; i--) *p++ = d[i];
    }
    return (int)(p - start);
}
