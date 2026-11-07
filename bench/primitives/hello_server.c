// The server of the clients of `http` measured (D412): one connection at a
// time, its request read to the end of its head and its body (by its
// `Content-Length`), answered `hello, world!` (or 64 KiB for `/64k`) with
// `Connection: close`, then closed. Started by `run.py` for a group that
// declares `// bench-server: http`; the clients of Isocron and of C talk to
// it alike.
//   hello_server PORT
#include <arpa/inet.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

static char big[65536];

static void answer(int c) {
    char buf[8192];
    size_t n = 0;
    char *end = NULL;
    while (!end) {
        ssize_t k = recv(c, buf + n, sizeof buf - 1 - n, 0);
        if (k <= 0) return;
        n += k;
        buf[n] = 0;
        end = strstr(buf, "\r\n\r\n");
    }
    size_t head = end + 4 - buf;
    long length = 0;
    char *cl = strstr(buf, "Content-Length: ");
    if (cl && cl < end) length = atol(cl + 16);
    long have = (long)(n - head);
    while (have < length) {
        ssize_t k = recv(c, buf, sizeof buf, 0);
        if (k <= 0) return;
        have += k;
    }
    int large = strncmp(buf, "GET /64k ", 9) == 0;
    const char *body = large ? big : "hello, world!";
    size_t blen = large ? sizeof big : 13;
    char h[128];
    int hl = snprintf(h, sizeof h, "HTTP/1.1 200 OK\r\nContent-Length: %zu\r\nConnection: close\r\n\r\n", blen);
    send(c, h, hl, MSG_MORE);
    for (size_t at = 0; at < blen;) {
        ssize_t k = send(c, body + at, blen - at, 0);
        if (k <= 0) return;
        at += k;
    }
}

int main(int argc, char **argv) {
    for (size_t i = 0; i < sizeof big; i++) big[i] = (char)(i % 251);
    int l = socket(AF_INET, SOCK_STREAM, 0), one = 1;
    setsockopt(l, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
    struct sockaddr_in a = { .sin_family = AF_INET, .sin_port = htons(atoi(argv[1])), .sin_addr.s_addr = htonl(INADDR_LOOPBACK) };
    if (bind(l, (struct sockaddr *)&a, sizeof a) || listen(l, 128)) return 1;
    for (;;) {
        int c = accept(l, NULL, NULL);
        if (c < 0) continue;
        setsockopt(c, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one);
        answer(c);
        close(c);
    }
}
