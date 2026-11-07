// The floor of `http.isc`: the same exchanges on the local machine, in C on
// one thread: the response written by the server side, the request made by
// `snprintf` and sent, the response read to the end of its head, its status
// and its headers parsed, its body read by its length or by its chunks
// (into one buffer). The server: the same 8 connections of 200 requests,
// answered by a loop of `epoll` (each request found by its end, its line
// and headers checked, `Content-Length` and `Connection` read, the
// response made by `snprintf` with the date of the wake, sent by `send`),
// only it timed. Each line: a name, the nanoseconds of one exchange or
// request (the mean over `rounds`), a checksum.
//   http <rounds>
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/epoll.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

// the bytes written into `p` kept: an empty `asm` that may read the memory
// (D456: GCC removed the writes of a text never read, and the floor
// measured its length alone)
#define KEEP(p) __asm__ volatile("" : : "r"(p) : "memory")

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

static char buf[1 << 20], body[1 << 20];
static long have;

// at least `n` bytes in `buf`
static int need(int fd, long n) {
    while (have < n) {
        long r = read(fd, buf + have, sizeof buf - have);
        if (r <= 0) return 0;
        have += r;
    }
    return 1;
}

// one exchange: its status plus the length of its body, or -1
static long exchange(int c) {
    char req[512];
    int k = snprintf(req, sizeof req, "GET /a HTTP/1.1\r\nHost: example.test\r\nUser-Agent: isocron\r\nConnection: close\r\n\r\n");
    if (send(c, req, k, MSG_NOSIGNAL) != k) return -1;
    have = 0;
    char *end;
    for (;;) {
        if (!need(c, have + 1)) return -1;
        buf[have] = 0;
        if ((end = strstr(buf, "\r\n\r\n"))) break;
    }
    long head = end - buf + 4;
    if (strncmp(buf, "HTTP/1.", 7) != 0) return -1;
    long status = strtol(buf + 9, NULL, 10);
    long length = -1;
    int chunked = 0;
    for (char *l = strstr(buf, "\r\n") + 2; l < buf + head - 2; l = strstr(l, "\r\n") + 2) {
        if (!strncasecmp(l, "content-length:", 15)) length = strtol(l + 15, NULL, 10);
        if (!strncasecmp(l, "transfer-encoding:", 18) && strstr(l, "chunked") && strstr(l, "chunked") < strstr(l, "\r\n")) chunked = 1;
    }
    long n = 0, at = head;
    if (chunked) {
        for (;;) {
            char *eol;
            for (;;) {
                buf[have] = 0;
                if ((eol = strstr(buf + at, "\r\n"))) break;
                if (!need(c, have + 1)) return -1;
            }
            long size = strtol(buf + at, NULL, 16);
            at = eol - buf + 2;
            if (size == 0) break;
            if (!need(c, at + size + 2)) return -1;
            memcpy(body + n, buf + at, size);
            n += size;
            at += size + 2;
        }
    } else {
        if (!need(c, head + length)) return -1;
        memcpy(body, buf + head, length);
        n = length;
    }
    return status + n;
}

#define CONNS 8
#define PER_CONN 200

struct conn {
    int fd;
    long have;
    char buf[1 << 16];
};

// the requests complete at the start of `c`'s buffer answered: the number
// answered, or -1 when the connection is to be closed
static int answer(struct conn *c, const char *date) {
    int n = 0;
    long at = 0;
    for (;;) {
        char *start = c->buf + at;
        c->buf[c->have] = 0;
        char *end = strstr(start, "\r\n\r\n");
        if (!end) break;
        long head = end - start + 4;
        // the request line: a method, a target, the version
        char *sp1 = memchr(start, ' ', head), *sp2 = sp1 ? memchr(sp1 + 1, ' ', start + head - sp1 - 1) : NULL;
        if (!sp1 || !sp2 || strncmp(sp2 + 1, "HTTP/1.", 7) != 0) return -1;
        long length = 0;
        int host = 0, close_it = 0;
        for (char *l = strstr(start, "\r\n") + 2; l < end; l = strstr(l, "\r\n") + 2) {
            if (!strncasecmp(l, "content-length:", 15)) length = strtol(l + 15, NULL, 10);
            if (!strncasecmp(l, "host:", 5)) host = 1;
            if (!strncasecmp(l, "connection:", 11) && strstr(l, "close") && strstr(l, "close") < strstr(l, "\r\n")) close_it = 1;
        }
        if (!host) return -1;
        if (c->have - at < head + length) break;
        char out[512];
        const char *body = "hello, world!";
        int k = snprintf(out, sizeof out, "HTTP/1.1 200 OK\r\nDate: %s\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: %zu\r\n%s\r\n%s", date, strlen(body), close_it ? "Connection: close\r\n" : "", body);
        if (send(c->fd, out, k, MSG_NOSIGNAL) != k) return -1;
        at += head + length;
        n++;
        if (close_it) return -1;
    }
    memmove(c->buf, c->buf + at, c->have - at);
    c->have -= at;
    return n;
}

// one round of the server: its nanoseconds, the bytes of the responses
static double serve_round(long *bytes) {
    int l = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC | SOCK_NONBLOCK, 0);
    int one = 1;
    setsockopt(l, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
    struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = 0};
    inet_pton(AF_INET, "127.0.0.1", &a.sin_addr);
    bind(l, (struct sockaddr *)&a, sizeof a);
    listen(l, 128);
    socklen_t len = sizeof a;
    getsockname(l, (struct sockaddr *)&a, &len);
    static char requests[PER_CONN * 64];
    int m = 0;
    for (int i = 0; i < PER_CONN; i++) m += sprintf(requests + m, "GET /hello HTTP/1.1\r\nHost: bench\r\n\r\n");
    int clients[CONNS];
    for (int i = 0; i < CONNS; i++) {
        clients[i] = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
        connect(clients[i], (struct sockaddr *)&a, sizeof a);
        send(clients[i], requests, m, MSG_NOSIGNAL);
    }
    double t0 = now();
    int ep = epoll_create1(EPOLL_CLOEXEC);
    struct epoll_event ev = {.events = EPOLLIN, .data.ptr = NULL};
    epoll_ctl(ep, EPOLL_CTL_ADD, l, &ev);
    static struct conn conns[CONNS];
    int nconns = 0, answered = 0;
    while (answered < CONNS * PER_CONN) {
        struct epoll_event evs[64];
        int n = epoll_wait(ep, evs, 64, -1);
        time_t t = time(NULL);
        struct tm tm;
        gmtime_r(&t, &tm);
        char date[64];
        strftime(date, sizeof date, "%a, %d %b %Y %H:%M:%S GMT", &tm);
        for (int i = 0; i < n; i++) {
            if (evs[i].data.ptr == NULL) {
                int c;
                while ((c = accept4(l, NULL, NULL, SOCK_NONBLOCK | SOCK_CLOEXEC)) >= 0) {
                    setsockopt(c, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one);
                    struct conn *x = &conns[nconns++];
                    x->fd = c;
                    x->have = 0;
                    struct epoll_event e = {.events = EPOLLIN, .data.ptr = x};
                    epoll_ctl(ep, EPOLL_CTL_ADD, c, &e);
                }
                continue;
            }
            struct conn *x = evs[i].data.ptr;
            long r = read(x->fd, x->buf + x->have, sizeof x->buf - 1 - x->have);
            if (r <= 0) continue;
            x->have += r;
            int k = answer(x, date);
            if (k > 0) answered += k;
        }
    }
    for (int i = 0; i < nconns; i++) close(conns[i].fd);
    close(ep);
    close(l);
    double dt = now() - t0;
    for (int i = 0; i < CONNS; i++) {
        long r;
        while ((r = read(clients[i], buf, sizeof buf)) > 0) *bytes += r;
        close(clients[i]);
    }
    return dt;
}

// the pure functions of the module (D392): the cheapest way known, by hand
static long hfdiv(long a, long b) { return a / b - (a % b < 0); }

static void hdate_of(long z, long *y, long *m, long *d) {
    z += 719468;
    long era = hfdiv(z, 146097), doe = z - era * 146097;
    long yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    long doy = doe - (365 * yoe + yoe / 4 - yoe / 100), mp = (5 * doy + 2) / 153;
    *d = doy - (153 * mp + 2) / 5 + 1;
    *m = mp < 10 ? mp + 3 : mp - 9;
    *y = yoe + era * 400 + (*m <= 2);
}

static char *hput(char *p, long v, int n) {
    for (int i = n - 1; i >= 0; i--, v /= 10) p[i] = '0' + v % 10;
    return p + n;
}

static void pure_forms(long r) {
    enum { M = 4096 };
    static char urls[M][64], buf[2048];
    for (long i = 0; i < M; i++) sprintf(urls[i], "http://host%ld.example:%ld/path/%ld?q=%ld", i % 10, 8000 + i % 100, i, i);
    struct timespec t0, t1;
    long acc = 0;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (long k = 0; k < r; k++) {
        acc = 0;
        for (long i = 0; i < M; i++) {
            const char *u = urls[i], *sep = strstr(u, "://");
            if (!sep || (sep - u != 4 && sep - u != 5) || strncasecmp(u, sep - u == 4 ? "http" : "https", sep - u)) { acc--; continue; }
            const char *rest = sep + 3, *end = rest + strcspn(rest, "/?"), *colon = memchr(rest, ':', end - rest);
            long port = 0;
            for (const char *q = colon + 1; q < end; q++) port = port * 10 + (*q - '0');
            long tl = strlen(end);
            acc += port + (tl ? tl : 1);
        }
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    printf("parseUrl %.3f %ld\n", ((t1.tv_sec - t0.tv_sec) * 1e9 + (t1.tv_nsec - t0.tv_nsec)) / ((double)r * M), acc);
    static const char *hk[8] = {"Server", "Content-Type", "Content-Length", "Date", "Connection", "X-Id", "Cache-Control", "Vary"};
    static const char *hv[8] = {"isc", "text/plain", "13", "now", "keep-alive", "42", "no-cache", "Accept"};
    static const char *names[8] = {"content-length", "date", "x-id", "vary", "server", "missing", "CONNECTION", "cache-control"};
    long klen[8], vlen[8], nlen[8];
    for (int j = 0; j < 8; j++) klen[j] = strlen(hk[j]), vlen[j] = strlen(hv[j]), nlen[j] = strlen(names[j]);
    for (int form = 0; form < 2; form++) {
        clock_gettime(CLOCK_MONOTONIC, &t0);
        for (long k = 0; k < r; k++) {
            acc = 0;
            for (long i = 0; i < M; i++) {
                // the lengths first, then the bytes, the case of ASCII ignored
                // (D395: `strcasecmp` was beaten)
                const char *found = 0, *nm = names[i % 8];
                long nl = nlen[i % 8];
                for (int j = 0; j < 8; j++) {
                    if (klen[j] != nl) continue;
                    long q = 0;
                    while (q < nl && (hk[j][q] | 32) == (nm[q] | 32)) q++;
                    if (q == nl) found = hv[j];
                }
                acc += found ? (long)vlen[found == hv[0] ? 0 : found == hv[1] ? 1 : found == hv[2] ? 2 : found == hv[3] ? 3 : found == hv[4] ? 4 : found == hv[5] ? 5 : found == hv[6] ? 6 : 7] : 0;
            }
        }
        clock_gettime(CLOCK_MONOTONIC, &t1);
        printf("%s %.3f %ld\n", form ? "header" : "headerOf", ((t1.tv_sec - t0.tv_sec) * 1e9 + (t1.tv_nsec - t0.tv_nsec)) / ((double)r * M), acc);
    }
    static char body[1024];
    memset(body, 'x', sizeof body);
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (long k = 0; k < r; k++) {
        acc = 0;
        for (long i = 0; i < M; i++) {
            // checked as UTF-8 (ASCII: 8 bytes at a time), then a text made of it
            unsigned long bad = 0;
            for (int j = 0; j < 1024; j += 8) {
                unsigned long w;
                memcpy(&w, body + j, 8);
                bad |= w & 0x8080808080808080ul;
            }
            char *t = malloc(1024);
            memcpy(t, body, 1024);
            // the block kept (D455: GCC removed the `malloc`, the copy and
            // the `free` of a block never read, the floor without them)
            __asm__ volatile("" : : "r"(t) : "memory");
            acc += (bad ? -1 : 1024) + i % 2;
            free(t);
        }
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    printf("text.1k %.3f %ld\n", ((t1.tv_sec - t0.tv_sec) * 1e9 + (t1.tv_nsec - t0.tv_nsec)) / ((double)r * M), acc);
    static char bodies[M][32];
    for (long i = 0; i < M; i++) sprintf(bodies[i], "hello number %ld", i);
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (long k = 0; k < r; k++) {
        acc = 0;
        for (long i = 0; i < M; i++) {
            // the body copied into a block of its own, the header a constant
            long n = strlen(bodies[i]);
            char *b = malloc(n);
            memcpy(b, bodies[i], n);
            // the block kept (D455, as for `text.1k`)
            __asm__ volatile("" : : "r"(b) : "memory");
            acc += n;
            free(b);
        }
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    printf("textResponse %.3f %ld\n", ((t1.tv_sec - t0.tv_sec) * 1e9 + (t1.tv_nsec - t0.tv_nsec)) / ((double)r * M), acc);
    static const char days[7][4] = {"Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"};
    static const char months[12][4] = {"Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"};
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (long k = 0; k < r; k++) {
        acc = 0;
        for (long i = 0; i < M; i++) {
            long ns = 1767225600000000000L + i * 997123456789L, dz = hfdiv(ns, 86400000000000L), s = (ns - dz * 86400000000000L) / 1000000000, y, mo, d;
            hdate_of(dz, &y, &mo, &d);
            long wd = (dz + 3) - hfdiv(dz + 3, 7) * 7;
            char *p = buf;
            memcpy(p, days[wd], 3); p += 3; *p++ = ','; *p++ = ' ';
            p = hput(p, d, 2); *p++ = ' '; memcpy(p, months[mo - 1], 3); p += 3; *p++ = ' ';
            p = hput(p, y, 4); *p++ = ' '; p = hput(p, s / 3600, 2); *p++ = ':'; p = hput(p, s / 60 % 60, 2); *p++ = ':'; p = hput(p, s % 60, 2);
            memcpy(p, " GMT", 4); p += 4;
            KEEP(buf);
            acc += p - buf;
        }
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    printf("httpDate %.3f %ld\n", ((t1.tv_sec - t0.tv_sec) * 1e9 + (t1.tv_nsec - t0.tv_nsec)) / ((double)r * M), acc);
    // the errors as data, as the program reads them (D457: the number
    // 65536 a constant of the loop, GCC wrote its digits at compilation;
    // `r < 0` keeps it a value of the run)
    static struct { int kind, len; const char *s; long n; } errs[M];
    for (long i = 0; i < M; i++) {
        errs[i].kind = i % 3;
        errs[i].s = i % 3 == 0 ? "the head" : "ftp://x";
        errs[i].len = i % 3 == 0 ? 8 : 7;
        errs[i].n = 65536 + (r < 0);
    }
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (long k = 0; k < r; k++) {
        acc = 0;
        for (long i = 0; i < M; i++) {
            char *p = buf;
            if (errs[i].kind == 0) { memcpy(p, "malformed: ", 11); p += 11; memcpy(p, errs[i].s, errs[i].len); p += errs[i].len; }
            else if (errs[i].kind == 1) {
                long v = errs[i].n;
                int d = 1;
                for (long w = v; w >= 10; w /= 10) d++;
                memcpy(p, "too large: more than ", 21); p += 21; p = hput(p, v, d); memcpy(p, " bytes", 6); p += 6;
            }
            else { memcpy(p, "bad url: ", 9); p += 9; memcpy(p, errs[i].s, errs[i].len); p += errs[i].len; }
            KEEP(buf);
            acc += p - buf;
        }
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    printf("message %.3f %ld\n", ((t1.tv_sec - t0.tv_sec) * 1e9 + (t1.tv_nsec - t0.tv_nsec)) / ((double)r * M), acc);
}

// One request on a connection of its own (D412): the bytes of the request
// of `http.request` sent in one write, the response read to the end of its
// body (by its `Content-Length`), the connection closed; its status plus
// the length of its body.
static long client_request(int port, const char *req, size_t reqlen) {
    static char buf[1 << 17];
    int c = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = htons(port), .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
    if (connect(c, (struct sockaddr *)&a, sizeof a)) { close(c); return -1; }
    if (send(c, req, reqlen, 0) != (ssize_t)reqlen) { close(c); return -1; }
    size_t n = 0;
    long head = -1, length = -1;
    for (;;) {
        ssize_t k = recv(c, buf + n, sizeof buf - n, 0);
        if (k <= 0) break;
        n += k;
        if (head < 0) {
            char *e = memmem(buf, n, "\r\n\r\n", 4);
            if (!e) continue;
            head = e + 4 - buf;
            char *cl = memmem(buf, head, "Content-Length: ", 16);
            length = cl ? atol(cl + 16) : -1;
        }
        if (length >= 0 && (long)n - head >= length) break;
    }
    close(c);
    if (head < 0) return -1;
    return atol(buf + 9) + ((long)n - head);
}

static void client_forms(long r, int port) {
    char get_small[256], get_big[256], post[512];
    int l1 = snprintf(get_small, sizeof get_small, "GET / HTTP/1.1\r\nHost: 127.0.0.1:%d\r\nUser-Agent: isocron\r\nConnection: close\r\n\r\n", port);
    int l2 = snprintf(get_big, sizeof get_big, "GET /64k HTTP/1.1\r\nHost: 127.0.0.1:%d\r\nUser-Agent: isocron\r\nConnection: close\r\n\r\n", port);
    int l3 = snprintf(post, sizeof post, "POST /form HTTP/1.1\r\nHost: 127.0.0.1:%d\r\nUser-Agent: isocron\r\nConnection: close\r\nContent-Type: text/plain\r\nContent-Length: 64\r\n\r\n", port);
    for (int i = 0; i < 64; i++) post[l3 + i] = (char)(97 + i % 26);
    l3 += 64;
    const char *reqs[3] = {get_small, get_big, post};
    int lens[3] = {l1, l2, l3};
    const char *names[3] = {"get.small", "get.64k", "request.post"};
    for (int f = 0; f < 3; f++) {
        struct timespec t0, t1;
        long acc = 0;
        clock_gettime(CLOCK_MONOTONIC, &t0);
        for (long k = 0; k < r; k++) acc += client_request(port, reqs[f], lens[f]);
        clock_gettime(CLOCK_MONOTONIC, &t1);
        printf("%s %.3f %ld\n", names[f], ((t1.tv_sec - t0.tv_sec) * 1e9 + (t1.tv_nsec - t0.tv_nsec)) / (double)r, acc / r);
    }
}

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    pure_forms(r);
    int l = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    int one = 1;
    setsockopt(l, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
    struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = 0};
    inet_pton(AF_INET, "127.0.0.1", &a.sin_addr);
    bind(l, (struct sockaddr *)&a, sizeof a);
    listen(l, 16);
    socklen_t len = sizeof a;
    getsockname(l, (struct sockaddr *)&a, &len);
    int c = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    connect(c, (struct sockaddr *)&a, sizeof a);
    int s = accept4(l, NULL, NULL, SOCK_CLOEXEC);
    const char *small = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 13\r\n\r\nhello, world!";
    static char chunked[80000];
    int m = sprintf(chunked, "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
    for (int i = 0; i < 64; i++) {
        m += sprintf(chunked + m, "400\r\n");
        memset(chunked + m, 'x', 1024);
        m += 1024;
        m += sprintf(chunked + m, "\r\n");
    }
    m += sprintf(chunked + m, "0\r\n\r\n");
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < r; k++) {
        send(s, small, strlen(small), MSG_NOSIGNAL);
        acc += exchange(c);
    }
    printf("exchange.small %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        send(s, chunked, m, MSG_NOSIGNAL);
        acc += exchange(c);
    }
    printf("exchange.chunked64 %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    long sr = r / 10 > 1 ? r / 10 : 1, bytes = 0;
    double st = 0;
    for (long k = 0; k < sr; k++) st += serve_round(&bytes);
    printf("serve.pipelined %.3f %ld\n", st * 1e9 / (sr * CONNS * PER_CONN), bytes / (sr * CONNS * PER_CONN));
    // the same server measured again for `serveAction` (D352): the floor of
    // a server that answers one request at a time is this loop too
    st = 0;
    bytes = 0;
    for (long k = 0; k < sr; k++) st += serve_round(&bytes);
    printf("serve.action.pipelined %.3f %ld\n", st * 1e9 / (sr * CONNS * PER_CONN), bytes / (sr * CONNS * PER_CONN));
    if (argc > 2) client_forms(r, atoi(argv[2]));
    return 0;
}
