// The floor of `net.isc`: the same work on the local machine, the calls of
// the system made directly in C, on one thread: a round trip of 64 bytes on
// a connection of TCP, 64 KiB sent and received, a connection made
// (`connect`, `accept`, both closed). Each line: a name, the nanoseconds of
// one (the mean over `rounds`), a checksum.
//   net <rounds>
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <sys/epoll.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

// `n` bytes read: their number, or -1
static long read_n(int fd, char *buf, long n) {
    long got = 0;
    while (got < n) {
        long r = read(fd, buf + got, n - got);
        if (r <= 0) return -1;
        got += r;
    }
    return got;
}

static int connect_to(int port) {
    struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = htons(port)};
    inet_pton(AF_INET, "127.0.0.1", &a.sin_addr);
    int fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (connect(fd, (struct sockaddr *)&a, sizeof a) != 0) return -1;
    return fd;
}

// The functions of a `Server` and of a connection kept (D424), by the calls
// of the runtime: `setsockopt` twice, `shutdown`, `epoll_create1`, `fcntl`
// and `epoll_ctl`, `epoll_wait` then `accept4` or `recv`, `send`, `close`.
static int listener_on(int *port) {
    int l = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0), one = 1;
    setsockopt(l, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
    struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = 0, .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
    bind(l, (struct sockaddr *)&a, sizeof a);
    listen(l, 128);
    socklen_t len = sizeof a;
    getsockname(l, (struct sockaddr *)&a, &len);
    *port = ntohs(a.sin_port);
    return l;
}

static int server_open(int l) {
    int ep = epoll_create1(EPOLL_CLOEXEC);
    fcntl(l, F_SETFL, fcntl(l, F_GETFL) | O_NONBLOCK);
    struct epoll_event e = {.events = EPOLLIN, .data.u64 = 0};
    epoll_ctl(ep, EPOLL_CTL_ADD, l, &e);
    return ep;
}

// the bytes of one connection waited for (the listener's arrivals accepted
// on the way): its descriptor, the count read into `buf`
static long server_wait(int ep, int l, char *buf, int *fd) {
    struct epoll_event evs[64];
    for (;;) {
        int n = epoll_wait(ep, evs, 64, 1000);
        if (n <= 0) return -1;
        for (int i = 0; i < n; i++) {
            if (evs[i].data.u64 == 0) {
                int c;
                while ((c = accept4(l, NULL, NULL, SOCK_NONBLOCK | SOCK_CLOEXEC)) >= 0) {
                    struct epoll_event e = {.events = EPOLLIN, .data.u64 = (uint64_t)c + 1};
                    epoll_ctl(ep, EPOLL_CTL_ADD, c, &e);
                }
            } else {
                int c = (int)(evs[i].data.u64 - 1);
                long k = recv(c, buf, 65536, 0);
                if (k > 0) {
                    *fd = c;
                    return k;
                }
            }
        }
    }
}

static void server_forms(long r) {
    static char buf[65536], small[64];
    for (int i = 0; i < 64; i++) small[i] = i;
    int port, l = listener_on(&port);
    int c = connect_to(port), s = accept4(l, NULL, NULL, SOCK_CLOEXEC);
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < r; k++) {
        struct timeval tv = {1, 0};
        acc += setsockopt(c, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv) == 0 && setsockopt(c, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv) == 0;
    }
    printf("setTimeout %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    close(s);
    close(c);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        int x = connect_to(port), y = accept4(l, NULL, NULL, SOCK_CLOEXEC);
        acc += shutdown(x, SHUT_WR) == 0;
        close(y);
        close(x);
    }
    printf("connect.closeWrite %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    close(l);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        int p2, l2 = listener_on(&p2);
        int ep = server_open(l2);
        acc += ep >= 0;
        close(ep);
        close(l2);
    }
    printf("server.open.close %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    l = listener_on(&port);
    int ep = server_open(l);
    c = connect_to(port);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        send(c, small, 64, MSG_NOSIGNAL);
        int fd = -1;
        long got = server_wait(ep, l, buf, &fd);
        send(fd, small, 64, MSG_NOSIGNAL);
        acc += got + read_n(c, buf, 64);
    }
    printf("server.roundtrip.64 %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    close(c);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        int x = connect_to(port);
        send(x, "x", 1, MSG_NOSIGNAL);
        int fd = -1;
        long got = server_wait(ep, l, buf, &fd);
        if (fd >= 0) {
            epoll_ctl(ep, EPOLL_CTL_DEL, fd, NULL);
            close(fd);
        }
        close(x);
        acc += got;
    }
    printf("server.conn %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    close(ep);
    close(l);
}

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    int l = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    int one = 1;
    setsockopt(l, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
    struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = 0};
    inet_pton(AF_INET, "127.0.0.1", &a.sin_addr);
    bind(l, (struct sockaddr *)&a, sizeof a);
    listen(l, 128);
    socklen_t len = sizeof a;
    getsockname(l, (struct sockaddr *)&a, &len);
    int port = ntohs(a.sin_port);
    int c = connect_to(port);
    int s = accept4(l, NULL, NULL, SOCK_CLOEXEC);
    static char small[64], big[65536], buf[65536];
    for (int i = 0; i < 64; i++) small[i] = i;
    for (int i = 0; i < 65536; i++) big[i] = i % 251;
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < r; k++) {
        send(c, small, 64, MSG_NOSIGNAL);
        acc += read_n(s, buf, 64);
        send(s, small, 64, MSG_NOSIGNAL);
        acc += read_n(c, buf, 64);
    }
    printf("roundtrip.64 %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        long at = 0;
        while (at < 65536) {
            long w = send(c, big + at, 65536 - at, MSG_NOSIGNAL);
            if (w <= 0) return 1;
            at += w;
        }
        acc += read_n(s, buf, 65536);
    }
    printf("send.64k %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    close(s);
    close(c);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        int x = connect_to(port);
        int y = accept4(l, NULL, NULL, SOCK_CLOEXEC);
        close(y);
        close(x);
        acc += x >= 0 && y >= 0;
    }
    printf("connect %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    close(l);
    // a listener on a port of the system, closed (D412)
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        int x = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
        int one = 1;
        setsockopt(x, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
        struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = 0, .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
        acc += bind(x, (struct sockaddr *)&a, sizeof a) == 0 && listen(x, 128) == 0;
        close(x);
    }
    printf("listen.close %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    // a name of /etc/hosts: the file read, the lines that name it, their
    // first word (D412)
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        static char buf[1 << 16];
        int f = open("/etc/hosts", O_RDONLY | O_CLOEXEC);
        ssize_t n = read(f, buf, sizeof buf - 1);
        close(f);
        if (n < 0) n = 0;
        buf[n] = 0;
        long found = 0, lens = 0;
        for (char *line = buf; *line;) {
            char *end = strchr(line, '\n');
            if (!end) end = line + strlen(line);
            char *hash = memchr(line, '#', end - line);
            char *stop = hash ? hash : end;
            char *w = line, *first = NULL;
            size_t firstlen = 0;
            int idx = 0, hit = 0;
            while (w < stop) {
                while (w < stop && (*w == ' ' || *w == '\t')) w++;
                char *s = w;
                while (w < stop && *w != ' ' && *w != '\t') w++;
                if (w > s) {
                    if (idx == 0) { first = s; firstlen = w - s; }
                    else if ((size_t)(w - s) == 13 && memcmp(s, "ip6-localhost", 13) == 0) hit = 1;
                    idx++;
                }
            }
            if (hit && first) { found++; lens += firstlen; }
            line = *end ? end + 1 : end;
        }
        acc += 100 * found + lens;
    }
    printf("resolve.hosts %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    // a name asked to the server of DNS (D414): one socket, the queries A
    // then AAAA (the bytes of `net.lookup`), each answer read, its records
    // as texts
    if (argc > 2) {
        int dport = atoi(argv[2]);
        unsigned char qa[64], ans[512];
        static const char name[] = "\005bench\004test";
        size_t ql = 0;
        unsigned char head[12] = {0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0};
        memcpy(qa, head, 12);
        memcpy(qa + 12, name, 11);
        ql = 23;
        unsigned char tail[5] = {0, 0, 1, 0, 1};
        memcpy(qa + ql, tail, 5);
        ql += 5;
        acc = 0;
        t0 = now();
        for (long k = 0; k < r; k++) {
            int u = socket(AF_INET, SOCK_DGRAM | SOCK_CLOEXEC, 0);
            struct timeval tv = {2, 0};
            setsockopt(u, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv);
            struct sockaddr_in d = {.sin_family = AF_INET, .sin_port = htons(dport), .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
            long found = 0;
            for (int t = 0; t < 2; t++) {
                qa[ql - 3] = t == 0 ? 1 : 28;
                sendto(u, qa, ql, 0, (struct sockaddr *)&d, sizeof d);
                ssize_t n = recv(u, ans, sizeof ans, 0);
                if (n < 12) continue;
                int count = ans[6] * 256 + ans[7];
                size_t at = 12;
                while (at < (size_t)n && ans[at] != 0) at += ans[at] + 1;
                at += 5;
                for (int i = 0; i < count && at + 12 <= (size_t)n; i++) {
                    int type = ans[at + 3], len = ans[at + 10] * 256 + ans[at + 11];
                    char text[64];
                    if (type == 1 && len == 4 && inet_ntop(AF_INET, ans + at + 12, text, sizeof text)) found += 100 + strlen(text);
                    if (type == 28 && len == 16 && inet_ntop(AF_INET6, ans + at + 12, text, sizeof text)) found += 100 + strlen(text);
                    at += 12 + len;
                }
            }
            close(u);
            acc += found;
        }
        printf("lookup %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    }
    server_forms(r);
    return 0;
}
