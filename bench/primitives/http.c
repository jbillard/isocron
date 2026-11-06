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

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
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
    printf("serveAction.pipelined %.3f %ld\n", st * 1e9 / (sr * CONNS * PER_CONN), bytes / (sr * CONNS * PER_CONN));
    return 0;
}
