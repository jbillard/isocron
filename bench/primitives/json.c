// The floor of `json.isc`: the same work on the same document of 1000
// records, written in C the cheapest way known, on one thread. `parse`
// reads any JSON into a tape (an array of values, strings unescaped into one
// buffer, both kept from round to round: no allocation per value), checking
// the grammar and UTF-8 as it goes; `decode` reads the records into
// structures by the names of their members, a name by a bump in an arena
// (given back at each round), any other member checked and skipped;
// `stringify` and `encode` write them back (numbers by Schubfach, the
// shortest form, D415, strings escaped where they must be). Each
// line: a name, the nanoseconds of one record (the mean over `rounds`), a
// checksum.
//   json <rounds>
#define _GNU_SOURCE
#include "f64_text.h"
#include <immintrin.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

// the bytes written into `p` kept: an empty `asm` that may read the memory
// (D456: GCC removed the writes of a text never read, and the floor
// measured its length alone)
#define KEEP(p) __asm__ volatile("" : : "r"(p) : "memory")

#define N 1000

static char *arena, *bump;

static char *take(long n) {
    char *p = bump;
    bump += (n + 15) & ~15L;
    return p;
}

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

enum { NUL, BOOL, NUM, STR, ARR, OBJ };

// a value of the tape: a number or a boolean; the place and length of a
// string in `chars`; the number of elements or members of a container (its
// keys and values follow it)
typedef struct {
    int kind;
    double num;
    long at, len, count;
} Node;

typedef struct {
    const char *p, *end;
    Node *nodes;
    long n, cap;
    char *chars;
    long nc, ccap;
    int bad;
} P;

static void ws(P *s) {
    while (s->p < s->end && (*s->p == ' ' || *s->p == '\t' || *s->p == '\n' || *s->p == '\r')) s->p++;
}

static long node(P *s, int kind) {
    if (s->n == s->cap) {
        s->cap = s->cap ? s->cap * 2 : 1024;
        s->nodes = realloc(s->nodes, s->cap * sizeof(Node));
    }
    s->nodes[s->n].kind = kind;
    return s->n++;
}

static void put(P *s, char c) {
    if (s->nc == s->ccap) {
        s->ccap = s->ccap ? s->ccap * 2 : 4096;
        s->chars = realloc(s->chars, s->ccap);
    }
    s->chars[s->nc++] = c;
}

static int hex(char c) {
    return c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 87 : c >= 'A' && c <= 'F' ? c - 55 : -1;
}

static void utf8(P *s, unsigned u) {
    if (u < 0x80) {
        put(s, u);
    } else if (u < 0x800) {
        put(s, 0xC0 | u >> 6);
        put(s, 0x80 | (u & 0x3F));
    } else {
        put(s, 0xE0 | u >> 12);
        put(s, 0x80 | (u >> 6 & 0x3F));
        put(s, 0x80 | (u & 0x3F));
    }
}

// the string at `s->p` (its quote), unescaped at the end of `chars`
static void string(P *s, long *at, long *len) {
    s->p++;
    *at = s->nc;
    for (;;) {
        if (s->p >= s->end || (unsigned char)*s->p < 32) {
            s->bad = 1;
            return;
        }
        char c = *s->p++;
        if (c == '"') break;
        if (c != '\\') {
            put(s, c);
            continue;
        }
        if (s->p >= s->end) {
            s->bad = 1;
            return;
        }
        char e = *s->p++;
        switch (e) {
        case '"': put(s, '"'); break;
        case '\\': put(s, '\\'); break;
        case '/': put(s, '/'); break;
        case 'b': put(s, 8); break;
        case 'f': put(s, 12); break;
        case 'n': put(s, '\n'); break;
        case 'r': put(s, '\r'); break;
        case 't': put(s, '\t'); break;
        case 'u': {
            if (s->end - s->p < 4) {
                s->bad = 1;
                return;
            }
            int a = hex(s->p[0]), b = hex(s->p[1]), c2 = hex(s->p[2]), d = hex(s->p[3]);
            if ((a | b | c2 | d) < 0) {
                s->bad = 1;
                return;
            }
            s->p += 4;
            utf8(s, a << 12 | b << 8 | c2 << 4 | d);
            break;
        }
        default: s->bad = 1; return;
        }
    }
    *len = s->nc - *at;
}

// a number of JSON: its grammar checked, then `strtod`
static int number(P *s, double *v) {
    const char *q = s->p;
    if (q < s->end && *q == '-') q++;
    if (q >= s->end || *q < '0' || *q > '9') return 0;
    if (*q == '0') q++;
    else
        while (q < s->end && *q >= '0' && *q <= '9') q++;
    if (q < s->end && *q == '.') {
        q++;
        if (q >= s->end || *q < '0' || *q > '9') return 0;
        while (q < s->end && *q >= '0' && *q <= '9') q++;
    }
    if (q < s->end && (*q == 'e' || *q == 'E')) {
        q++;
        if (q < s->end && (*q == '+' || *q == '-')) q++;
        if (q >= s->end || *q < '0' || *q > '9') return 0;
        while (q < s->end && *q >= '0' && *q <= '9') q++;
    }
    *v = strtod(s->p, NULL);
    s->p = q;
    return 1;
}

static void value(P *s, int depth);

static void container(P *s, int depth, int obj) {
    long me = node(s, obj ? OBJ : ARR), count = 0;
    s->p++;
    ws(s);
    if (s->p < s->end && *s->p == (obj ? '}' : ']')) {
        s->p++;
        s->nodes[me].count = 0;
        return;
    }
    for (;;) {
        ws(s);
        if (obj) {
            if (s->p >= s->end || *s->p != '"') {
                s->bad = 1;
                return;
            }
            long k = node(s, STR), at, len;
            string(s, &at, &len);
            s->nodes[k].at = at;
            s->nodes[k].len = len;
            ws(s);
            if (s->p >= s->end || *s->p != ':') {
                s->bad = 1;
                return;
            }
            s->p++;
        }
        value(s, depth + 1);
        if (s->bad) return;
        count++;
        ws(s);
        if (s->p < s->end && *s->p == ',') {
            s->p++;
            continue;
        }
        if (s->p < s->end && *s->p == (obj ? '}' : ']')) {
            s->p++;
            break;
        }
        s->bad = 1;
        return;
    }
    s->nodes[me].count = count;
}

static void value(P *s, int depth) {
    ws(s);
    if (s->p >= s->end || depth > 512) {
        s->bad = 1;
        return;
    }
    char c = *s->p;
    if (c == '{' || c == '[') {
        container(s, depth, c == '{');
        return;
    }
    if (c == '"') {
        long k = node(s, STR), at, len;
        string(s, &at, &len);
        s->nodes[k].at = at;
        s->nodes[k].len = len;
        return;
    }
    if (s->end - s->p >= 4 && !memcmp(s->p, "true", 4)) {
        s->nodes[node(s, BOOL)].num = 1;
        s->p += 4;
        return;
    }
    if (s->end - s->p >= 5 && !memcmp(s->p, "false", 5)) {
        s->nodes[node(s, BOOL)].num = 0;
        s->p += 5;
        return;
    }
    if (s->end - s->p >= 4 && !memcmp(s->p, "null", 4)) {
        node(s, NUL);
        s->p += 4;
        return;
    }
    double v;
    if (!number(s, &v)) {
        s->bad = 1;
        return;
    }
    s->nodes[node(s, NUM)].num = v;
}

// the text is UTF-8: each byte the continuation expected, or a start
static int utf8_ok(const unsigned char *b, long n) {
    long i = 0;
    while (i < n) {
        unsigned char x = b[i];
        if (x < 0x80) {
            i++;
            continue;
        }
        int want = (x & 0xE0) == 0xC0 && x >= 0xC2 ? 1 : (x & 0xF0) == 0xE0 ? 2 : (x & 0xF8) == 0xF0 && x <= 0xF4 ? 3 : -1;
        if (want < 0 || i + want >= n) return 0;
        for (int k = 1; k <= want; k++)
            if ((b[i + k] & 0xC0) != 0x80) return 0;
        i += want + 1;
    }
    return 1;
}

static int parse(P *s, const char *text, long n) {
    s->p = text;
    s->end = text + n;
    s->n = 0;
    s->nc = 0;
    s->bad = 0;
    if (!utf8_ok((const unsigned char *)text, n)) return 0;
    value(s, 0);
    ws(s);
    return !s->bad && s->p == s->end;
}

typedef struct {
    char *b;
    long n, cap;
} Out;

static void out(Out *o, const char *p, long n) {
    if (o->n + n > o->cap) {
        while (o->n + n > o->cap) o->cap = o->cap ? o->cap * 2 : 4096;
        o->b = realloc(o->b, o->cap);
    }
    memcpy(o->b + o->n, p, n);
    o->n += n;
}

static void quote(Out *o, const char *p, long n) {
    out(o, "\"", 1);
    long from = 0;
    for (long i = 0; i < n; i++) {
        unsigned char c = p[i];
        if (c >= 32 && c != '"' && c != '\\') continue;
        out(o, p + from, i - from);
        char e[8];
        int k = c == '"' ? snprintf(e, 8, "\\\"") : c == '\\' ? snprintf(e, 8, "\\\\") : c == '\n' ? snprintf(e, 8, "\\n") : snprintf(e, 8, "\\u%04x", c);
        out(o, e, k);
        from = i + 1;
    }
    out(o, p + from, n - from);
    out(o, "\"", 1);
}

// a number as Isocron writes it, by the cheapest way known: the shortest
// decimal that reads back to it (Schubfach, `f64_text.h`, D415; `%.17g` of
// `snprintf` before, slower than Isocron itself)
static void num(Out *o, double v) {
    char t[400];
    int k = f64_text(v == 0 ? 0.0 : v, t);
    out(o, t, k);
}

// the value at node `i` written; the node after it
static long write(Out *o, P *s, long i) {
    Node *x = &s->nodes[i];
    switch (x->kind) {
    case NUL: out(o, "null", 4); return i + 1;
    case BOOL: x->num ? out(o, "true", 4) : out(o, "false", 5); return i + 1;
    case NUM: num(o, x->num); return i + 1;
    case STR: quote(o, s->chars + x->at, x->len); return i + 1;
    default: {
        int obj = x->kind == OBJ;
        long count = x->count, j = i + 1;
        out(o, obj ? "{" : "[", 1);
        for (long k = 0; k < count; k++) {
            if (k) out(o, ",", 1);
            if (obj) {
                quote(o, s->chars + s->nodes[j].at, s->nodes[j].len);
                out(o, ":", 1);
                j++;
            }
            j = write(o, s, j);
        }
        out(o, obj ? "}" : "]", 1);
        return j;
    }
    }
}

typedef struct {
    long id;
    double x;
    char *name;
    long nlen;
    int ok;
} Rec;

#define WS() \
    while (p < end && (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r')) p++

// the records of `text` (an array of objects) by the names of their
// members: an integer checked whole, a name into the arena, any other
// member checked and skipped; their count, or -1
static long decode(const char *text, long n, Rec *rs, P *skip) {
    const char *p = text, *end = text + n;
    WS();
    if (p >= end || *p != '[') return -1;
    p++;
    long count = 0;
    WS();
    if (p < end && *p == ']') return 0;
    for (;;) {
        WS();
        if (p >= end || *p != '{' || count >= N) return -1;
        p++;
        Rec *r = &rs[count];
        int seen = 0;
        for (;;) {
            WS();
            if (p >= end || *p != '"') return -1;
            const char *k = ++p;
            while (p < end && *p != '"' && *p != '\\') p++;
            if (p >= end || *p != '"') return -1;
            long kl = p - k;
            p++;
            WS();
            if (p >= end || *p != ':') return -1;
            p++;
            WS();
            if (kl == 2 && !memcmp(k, "id", 2)) {
                int neg = p < end && *p == '-';
                const char *d = p + neg, *q = d;
                long v = 0;
                while (q < end && *q >= '0' && *q <= '9') v = v * 10 + (*q++ - '0');
                if (q == d || (*d == '0' && q - d > 1) || q - d > 18 || (q < end && (*q == '.' || *q == 'e' || *q == 'E'))) return -1;
                r->id = neg ? -v : v;
                p = q;
                seen |= 1;
            } else if (kl == 1 && *k == 'x') {
                skip->p = p;
                skip->end = end;
                if (!number(skip, &r->x)) return -1;
                p = skip->p;
                seen |= 2;
            } else if (kl == 4 && !memcmp(k, "name", 4)) {
                if (p >= end || *p != '"') return -1;
                skip->p = p;
                skip->end = end;
                skip->bad = 0;
                skip->nc = 0;
                long at, len;
                string(skip, &at, &len);
                if (skip->bad) return -1;
                r->name = take(len);
                memcpy(r->name, skip->chars + at, len);
                r->nlen = len;
                p = skip->p;
                seen |= 4;
            } else if (kl == 2 && !memcmp(k, "ok", 2)) {
                if (end - p >= 4 && !memcmp(p, "true", 4)) {
                    r->ok = 1;
                    p += 4;
                } else if (end - p >= 5 && !memcmp(p, "false", 5)) {
                    r->ok = 0;
                    p += 5;
                } else {
                    return -1;
                }
                seen |= 8;
            } else {
                skip->p = p;
                skip->end = end;
                skip->bad = 0;
                skip->n = 0;
                skip->nc = 0;
                value(skip, 2);
                if (skip->bad) return -1;
                p = skip->p;
            }
            WS();
            if (p < end && *p == ',') {
                p++;
                continue;
            }
            if (p < end && *p == '}') {
                p++;
                break;
            }
            return -1;
        }
        if (seen != 15) return -1;
        count++;
        WS();
        if (p < end && *p == ',') {
            p++;
            continue;
        }
        if (p < end && *p == ']') {
            p++;
            break;
        }
        return -1;
    }
    WS();
    return p == end ? count : -1;
}

// the records written as `encode` does: their fields in the order of their
// names
static long encode(Out *o, Rec *rs, long n) {
    o->n = 0;
    out(o, "[", 1);
    for (long i = 0; i < n; i++) {
        char t[32];
        if (i) out(o, ",", 1);
        out(o, "{\"id\":", 6);
        out(o, t, snprintf(t, 32, "%ld", rs[i].id));
        out(o, ",\"name\":", 8);
        quote(o, rs[i].name, rs[i].nlen);
        out(o, ",\"ok\":", 6);
        rs[i].ok ? out(o, "true", 4) : out(o, "false", 5);
        out(o, ",\"x\":", 5);
        num(o, rs[i].x);
        out(o, "}", 1);
    }
    out(o, "]", 1);
    return o->n;
}

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    // the arena touched once: its pages made before any measure
    arena = malloc(64L * 1024 * 1024);
    memset(arena, 1, 64L * 1024 * 1024);
    Out doc = {0};
    out(&doc, "[", 1);
    for (int i = 0; i < N; i++) {
        char t[96];
        int k = snprintf(t, 96, "%s{\"id\":%d,\"x\":%.17g,\"name\":\"n%d\",\"ok\":%s}", i ? "," : "", i, i * 0.5, i, i % 2 == 0 ? "true" : "false");
        out(&doc, t, k);
    }
    out(&doc, "]", 1);
    double each = (double)r * N;
    P s = {0}, skip = {0};
    long a1 = 0;
    double t0 = now();
    for (long k = 0; k < r; k++) a1 = parse(&s, doc.b, doc.n) ? s.nodes[0].count : -1;
    printf("parse %.3f %ld\n", (now() - t0) * 1e9 / each, a1);
    static Rec rs[N];
    long a2 = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        bump = arena;
        long c = decode(doc.b, doc.n, rs, &skip), sum = 0;
        for (long i = 0; i < c; i++) sum += rs[i].id;
        a2 = c < 0 ? -1 : sum;
    }
    printf("decode %.3f %ld\n", (now() - t0) * 1e9 / each, a2);
    parse(&s, doc.b, doc.n);
    Out o = {0};
    long a3 = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        o.n = 0;
        write(&o, &s, 0);
        a3 = o.n;
    }
    printf("stringify %.3f %ld\n", (now() - t0) * 1e9 / each, a3);
    bump = arena;
    long c = decode(doc.b, doc.n, rs, &skip), a4 = 0;
    t0 = now();
    for (long k = 0; k < r; k++) a4 = encode(&o, rs, c);
    printf("encode %.3f %ld\n", (now() - t0) * 1e9 / each, a4);
    // the other functions of the module (D392)
    enum { M = 4096 };
    static char buf[128];
    long a5 = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        a5 = 0;
        for (long i = 0; i < M; i++) {
            // `line L, column C: message`, the digits by hand
            char *p = buf;
            memcpy(p, "line ", 5); p += 5;
            long l = i % 97 + 1, c = i % 41 + 1;
            if (l >= 10) *p++ = '0' + l / 10;
            *p++ = '0' + l % 10;
            memcpy(p, ", column ", 9); p += 9;
            if (c >= 10) *p++ = '0' + c / 10;
            *p++ = '0' + c % 10;
            memcpy(p, ": unexpected token", 18); p += 18;
            KEEP(buf);
            a5 += p - buf;
        }
    }
    printf("message %.3f %ld\n", (now() - t0) * 1e9 / ((double)r * M), a5);
    static char plain[M][32], marked[M][32];
    static size_t lens[2][M];
    for (long i = 0; i < M; i++) {
        sprintf(plain[i], "name number %ld", i);
        sprintf(marked[i], "say \"hi\"\nto %ld", i);
        lens[0][i] = strlen(plain[i]);
        lens[1][i] = strlen(marked[i]);
    }
    for (int form = 0; form < 2; form++) {
        long a = 0;
        t0 = now();
        for (long k = 0; k < r; k++) {
            a = 0;
            for (long i = 0; i < M; i++) {
                const char *s = form ? marked[i] : plain[i];
                char *p = buf;
                *p++ = '"';
                // nothing to escape: the 32 bytes of the text (its array has
                // them) tested in one vector, then written in one store
                // (D392: the floor byte by byte was beaten)
                size_t n = lens[form][i];
                __m256i v = _mm256_loadu_si256((const __m256i *)s);
                __m256i bad = _mm256_or_si256(_mm256_or_si256(_mm256_cmpeq_epi8(v, _mm256_set1_epi8('"')), _mm256_cmpeq_epi8(v, _mm256_set1_epi8('\\'))),
                                              _mm256_cmpeq_epi8(_mm256_max_epu8(v, _mm256_set1_epi8(31)), _mm256_set1_epi8(31)));
                if (((unsigned)_mm256_movemask_epi8(bad) & ((1u << n) - 1)) == 0) {
                    _mm256_storeu_si256((__m256i *)p, v);
                    p += n;
                    *p++ = '"';
                    KEEP(buf);
                    a += p - buf;
                    continue;
                }
                for (; *s; s++) {
                    unsigned char ch = *s;
                    if (ch == '"' || ch == '\\') { *p++ = '\\'; *p++ = ch; }
                    else if (ch == '\n') { *p++ = '\\'; *p++ = 'n'; }
                    else if (ch == '\r') { *p++ = '\\'; *p++ = 'r'; }
                    else if (ch == '\t') { *p++ = '\\'; *p++ = 't'; }
                    else if (ch < 32) { p += sprintf(p, "\\u%04x", ch); }
                    else *p++ = ch;
                }
                *p++ = '"';
                KEEP(buf);
                a += p - buf;
            }
        }
        printf("%s %.3f %ld\n", form ? "quote.escaped" : "quote.plain", (now() - t0) * 1e9 / ((double)r * M), a);
    }
    return 0;
}
