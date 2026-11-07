// The client of the benchmark of the servers of TLS (D416), the same for
// the server of Isocron and its floor: OpenSSL, the authority given, the
// name `localhost` checked. `serve.handshake`: requests on a connection of
// their own (`Connection: close`: the handshake, the request, the response,
// the end); `serve.keepalive`: requests on one connection kept, 64 a
// connection. Each line: the name, the nanoseconds of one request, the
// checksum (the status plus the length of the body).
//   https_client <rounds> <port> <authority.pem>
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

typedef struct ssl_st SSL;
typedef struct ssl_ctx_st SSL_CTX;
typedef struct ssl_method_st SSL_METHOD;
const SSL_METHOD *TLS_client_method(void);
SSL_CTX *SSL_CTX_new(const SSL_METHOD *m);
void SSL_CTX_set_verify(SSL_CTX *c, int mode, void *cb);
int SSL_CTX_load_verify_locations(SSL_CTX *c, const char *f, const char *d);
SSL *SSL_new(SSL_CTX *c);
long SSL_ctrl(SSL *s, int cmd, long larg, void *parg);
int SSL_set1_host(SSL *s, const char *h);
int SSL_set_fd(SSL *s, int fd);
int SSL_connect(SSL *s);
int SSL_read(SSL *s, void *b, int n);
int SSL_write(SSL *s, const void *b, int n);
int SSL_shutdown(SSL *s);
void SSL_free(SSL *s);

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

static SSL *open_tls(SSL_CTX *ctx, int port, int *fd) {
    *fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = htons(port), .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
    if (connect(*fd, (struct sockaddr *)&a, sizeof a)) return NULL;
    int one = 1;
    setsockopt(*fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one);
    SSL *s = SSL_new(ctx);
    SSL_ctrl(s, 55, 0, (void *)"localhost");
    SSL_set1_host(s, "localhost");
    SSL_set_fd(s, *fd);
    if (SSL_connect(s) != 1) { SSL_free(s); return NULL; }
    return s;
}

// one response read: its status plus the length of its body, or -1
static long response(SSL *s) {
    static char buf[16384];
    int n = 0;
    char *end = NULL;
    while (!end) {
        int k = SSL_read(s, buf + n, sizeof buf - 1 - n);
        if (k <= 0) return -1;
        n += k;
        buf[n] = 0;
        end = strstr(buf, "\r\n\r\n");
    }
    int head = end + 4 - buf;
    char *cl = memmem(buf, head, "Content-Length: ", 16);
    long length = cl ? atol(cl + 16) : 0;
    while (n - head < length) {
        int k = SSL_read(s, buf + n, sizeof buf - 1 - n);
        if (k <= 0) return -1;
        n += k;
    }
    return atol(buf + 9) + length;
}

int main(int argc, char **argv) {
    long r = atol(argv[1]);
    long n1 = r / 2 > 1 ? r / 2 : 1, c2 = r / 8 > 1 ? r / 8 : 1;
    int port = atoi(argv[2]);
    SSL_CTX *ctx = SSL_CTX_new(TLS_client_method());
    SSL_CTX_set_verify(ctx, 1, NULL);
    SSL_CTX_load_verify_locations(ctx, argv[3], NULL);
    char close_req[256], keep_req[256];
    int ql = snprintf(close_req, sizeof close_req, "GET / HTTP/1.1\r\nHost: localhost:%d\r\nConnection: close\r\n\r\n", port);
    int kl = snprintf(keep_req, sizeof keep_req, "GET / HTTP/1.1\r\nHost: localhost:%d\r\n\r\n", port);
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < n1; k++) {
        int fd;
        SSL *s = open_tls(ctx, port, &fd);
        if (!s) { acc -= 1; close(fd); continue; }
        SSL_write(s, close_req, ql);
        acc += response(s);
        SSL_shutdown(s);
        SSL_free(s);
        close(fd);
    }
    printf("serve.handshake %.3f %ld\n", (now() - t0) * 1e9 / n1, acc / n1);
    acc = 0;
    t0 = now();
    for (long c = 0; c < c2; c++) {
        int fd;
        SSL *s = open_tls(ctx, port, &fd);
        if (!s) { acc -= 64; close(fd); continue; }
        for (int k = 0; k < 64; k++) {
            SSL_write(s, keep_req, kl);
            acc += response(s);
        }
        SSL_shutdown(s);
        SSL_free(s);
        close(fd);
    }
    printf("serve.keepalive %.3f %ld\n", (now() - t0) * 1e9 / (c2 * 64), acc / (c2 * 64));
    return 0;
}
