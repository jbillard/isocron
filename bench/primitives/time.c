// The floor of `time.isc`: the same work in C, the cheapest way known
// (clock_gettime, the algorithms of the civil days of Howard Hinnant, the
// text written by hand), on one thread. Each line: a name, the nanoseconds
// of one call (the mean over `rounds`), a checksum.
//   time <rounds>
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define N 4096

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e9 + t.tv_nsec;
}

static long fdiv(long a, long b) { return a / b - (a % b < 0); }

static long days_of(long y, long m, long d) {
    y -= m <= 2;
    long era = fdiv(y, 400), yoe = y - era * 400;
    long doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    return era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
}

static void date_of(long z, long *y, long *m, long *d) {
    z += 719468;
    long era = fdiv(z, 146097), doe = z - era * 146097;
    long yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    long doy = doe - (365 * yoe + yoe / 4 - yoe / 100), mp = (5 * doy + 2) / 153;
    *d = doy - (153 * mp + 2) / 5 + 1;
    *m = mp < 10 ? mp + 3 : mp - 9;
    *y = yoe + era * 400 + (*m <= 2);
}

static char *put(char *p, long v, int n) {
    for (int i = n - 1; i >= 0; i--, v /= 10) p[i] = '0' + v % 10;
    return p + n;
}

// RFC 3339 in UTC, the fraction without its zeros at the end
static long format_iso(long ns, char *out) {
    long days = fdiv(ns, 86400000000000L), r = ns - days * 86400000000000L, s = r / 1000000000, f = r % 1000000000, y, m, d;
    date_of(days, &y, &m, &d);
    char *p = out;
    p = put(p, y, 4); *p++ = '-'; p = put(p, m, 2); *p++ = '-'; p = put(p, d, 2); *p++ = 'T';
    p = put(p, s / 3600, 2); *p++ = ':'; p = put(p, s / 60 % 60, 2); *p++ = ':'; p = put(p, s % 60, 2);
    if (f) {
        *p++ = '.';
        int k = 9;
        while (f % 10 == 0) f /= 10, k--;
        p = put(p, f, k);
    }
    *p++ = 'Z';
    return p - out;
}

// the number of the `n` digits at `s`, or -1
static long digits(const char *s, int n) {
    long v = 0;
    for (int i = 0; i < n; i++) {
        if (s[i] < '0' || s[i] > '9') return -1;
        v = v * 10 + (s[i] - '0');
    }
    return v;
}

static long dim(long y, long m) {
    static const char d[13] = {0, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31};
    return m == 2 && y % 4 == 0 && (y % 100 || y % 400 == 0) ? 29 : d[m];
}

// the instant of an RFC 3339 text in UTC (as `parseIso`: its separators
// and the bounds of its fields checked, a fraction, `Z`), or -1
static long parse_iso(const char *s, long *second) {
    long y = digits(s, 4), mo = digits(s + 5, 2), d = digits(s + 8, 2), h = digits(s + 11, 2), mi = digits(s + 14, 2), se = digits(s + 17, 2);
    if (y < 0 || mo < 1 || mo > 12 || d < 1 || s[4] != '-' || s[7] != '-' || d > dim(y, mo) || s[10] != 'T' || h < 0 || h > 23 || mi < 0 || mi > 59 || se < 0 || se > 60 || s[13] != ':' || s[16] != ':') return -1;
    long f = 0, k = 0, at = 19;
    if (s[at] == '.') {
        at++;
        while (s[at] >= '0' && s[at] <= '9') {
            if (k < 9) f = f * 10 + (s[at] - '0'), k++;
            at++;
        }
        while (k < 9) f *= 10, k++;
    }
    if (s[at] != 'Z' || s[at + 1]) return -1;
    *second = se;
    return days_of(y, mo, d) * 86400000000000L + ((h * 60 + mi) * 60 + se) * 1000000000L + f;
}

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    static long ins[N];
    static char texts[N][40];
    for (long i = 0; i < N; i++) {
        ins[i] = 1767225600000000000L + i * 997123456789L;
        texts[i][format_iso(ins[i], texts[i])] = 0;
    }
    long acc = 0;
    double t0 = now();
    struct timespec ts;
    for (long i = 0; i < r; i++) {
        clock_gettime(CLOCK_REALTIME, &ts);
        acc += ts.tv_sec > 0;
    }
    double t1 = now();
    printf("now %.3f %ld\n", (t1 - t0) / r, acc / r);
    acc = 0;
    for (long k = 0; k < r; k++)
        for (long i = 0; i < N; i++) {
            long t = ins[i] + 3600000000000L, days = fdiv(t, 86400000000000L), s = (t - days * 86400000000000L) / 1000000000;
            acc += s / 60 % 60;
        }
    double t2 = now();
    printf("toDateTime %.3f %ld\n", (t2 - t1) / ((double)r * N), acc / r);
    acc = 0;
    char out[40];
    for (long k = 0; k < r; k++)
        for (long i = 0; i < N; i++) acc += format_iso(ins[i], out);
    double t3 = now();
    printf("formatIso %.3f %ld\n", (t3 - t2) / ((double)r * N), acc / r);
    acc = 0;
    for (long k = 0; k < r; k++)
        for (long i = 0; i < N; i++) {
            long se = -1;
            acc += parse_iso(texts[i], &se) >= 0 ? se : -1;
        }
    double t4 = now();
    printf("parseIso %.3f %ld\n", (t4 - t3) / ((double)r * N), acc / r);
    acc = 0;
    for (long k = 0; k < r; k++)
        for (long i = 0; i < N; i++) {
            long y, m, d;
            date_of(i * 37 - 70000, &y, &m, &d);
            acc += days_of(y, m, d);
        }
    double t5 = now();
    printf("dateOf.daysOf %.3f %ld\n", (t5 - t4) / ((double)r * N), acc / r);
    return 0;
}
