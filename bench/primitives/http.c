// The floor of `http.isc`: the same exchanges on the local machine, in C on
// one thread: the response written by the server side, the request made by
// `snprintf` and sent, the response read to the end of its head, its status
// and its headers parsed, its body read by its length or by its chunks
// (into one buffer). Each line: a name, the nanoseconds of one exchange
// (the mean over `rounds`), a checksum.
//   http <rounds>
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <netinet/in.h>
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
    return 0;
}
