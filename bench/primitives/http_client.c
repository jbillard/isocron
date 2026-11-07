// The client of `http_serve.isc` and of its floor (D431): `https_client.c`
// without TLS, the same requests: one by connection, then 64 on each of
// connections kept.
//   http_client <rounds> <port>
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

static int open_tcp(int port) {
    int fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = htons(port), .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
    if (connect(fd, (struct sockaddr *)&a, sizeof a)) { close(fd); return -1; }
    int one = 1;
    setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one);
    return fd;
}

// one response read: its status plus the length of its body, or -1
static long response(int s) {
    static char buf[16384];
    int n = 0;
    char *end = NULL;
    while (!end) {
        int k = (int)recv(s, buf + n, sizeof buf - 1 - n, 0);
        if (k <= 0) return -1;
        n += k;
        buf[n] = 0;
        end = strstr(buf, "\r\n\r\n");
    }
    int head = end + 4 - buf;
    char *cl = memmem(buf, head, "Content-Length: ", 16);
    long length = cl ? atol(cl + 16) : 0;
    while (n - head < length) {
        int k = (int)recv(s, buf + n, sizeof buf - 1 - n, 0);
        if (k <= 0) return -1;
        n += k;
    }
    return atol(buf + 9) + length;
}

int main(int argc, char **argv) {
    long r = atol(argv[1]);
    long n1 = r / 2 > 1 ? r / 2 : 1, c2 = r / 8 > 1 ? r / 8 : 1;
    int port = atoi(argv[2]);
    char close_req[256], keep_req[256];
    int ql = snprintf(close_req, sizeof close_req, "GET / HTTP/1.1\r\nHost: localhost:%d\r\nConnection: close\r\n\r\n", port);
    int kl = snprintf(keep_req, sizeof keep_req, "GET / HTTP/1.1\r\nHost: localhost:%d\r\n\r\n", port);
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < n1; k++) {
        int s = open_tcp(port);
        if (s < 0) { acc -= 1; continue; }
        send(s, close_req, ql, MSG_NOSIGNAL);
        acc += response(s);
        close(s);
    }
    printf("serve.connection %.3f %ld\n", (now() - t0) * 1e9 / n1, acc / n1);
    acc = 0;
    t0 = now();
    for (long c = 0; c < c2; c++) {
        int s = open_tcp(port);
        if (s < 0) { acc -= 64; continue; }
        for (int k = 0; k < 64; k++) {
            send(s, keep_req, kl, MSG_NOSIGNAL);
            acc += response(s);
        }
        close(s);
    }
    printf("serve.keepalive %.3f %ld\n", (now() - t0) * 1e9 / (c2 * 64), acc / (c2 * 64));
    return 0;
}
