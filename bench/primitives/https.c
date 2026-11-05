// The floor of `https.isc`: the same requests in C on one thread, OpenSSL
// called directly (its functions declared here as its manual gives them:
// the machine has the library, not its headers): a socket connected, a
// context that trusts the authority of the file, the session with SNI and
// the check of the name, `SSL_connect`, the request written, the response
// read to its end, its status and its length parsed, the session and the
// socket closed. Each line: a name, the nanoseconds of one request (the
// mean over half of `rounds`), a checksum.
//   https <rounds> <port> <authority.pem>
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <netinet/in.h>
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
void SSL_CTX_free(SSL_CTX *c);
void SSL_CTX_set_verify(SSL_CTX *c, int mode, void *cb);
int SSL_CTX_load_verify_locations(SSL_CTX *c, const char *file, const char *dir);
unsigned long long SSL_CTX_set_options(SSL_CTX *c, unsigned long long op);
SSL *SSL_new(SSL_CTX *c);
void SSL_free(SSL *s);
int SSL_set_fd(SSL *s, int fd);
long SSL_ctrl(SSL *s, int cmd, long larg, void *parg);
int SSL_set1_host(SSL *s, const char *host);
int SSL_connect(SSL *s);
int SSL_read(SSL *s, void *buf, int num);
int SSL_write(SSL *s, const void *buf, int num);
int SSL_shutdown(SSL *s);

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

static char buf[1 << 20];

// one request of `path`: its status plus the length of its body, or -1
static long get(SSL_CTX *ctx, int port, const char *path) {
    int fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = htons(port)};
    inet_pton(AF_INET, "127.0.0.1", &a.sin_addr);
    if (connect(fd, (struct sockaddr *)&a, sizeof a) != 0) return -1;
    SSL *s = SSL_new(ctx);
    SSL_ctrl(s, 55, 0, (void *)"localhost");  // SNI
    SSL_set1_host(s, "localhost");
    SSL_set_fd(s, fd);
    long r = -1;
    if (SSL_connect(s) == 1) {
        char req[256];
        int k = snprintf(req, sizeof req, "GET %s HTTP/1.1\r\nHost: localhost:%d\r\nUser-Agent: isocron\r\nConnection: close\r\n\r\n", path, port);
        SSL_write(s, req, k);
        long have = 0, n;
        while ((n = SSL_read(s, buf + have, sizeof buf - 1 - have)) > 0) have += n;
        buf[have] = 0;
        char *end = strstr(buf, "\r\n\r\n");
        if (end && strncmp(buf, "HTTP/1.", 7) == 0) r = strtol(buf + 9, NULL, 10) + (have - (end + 4 - buf));
    }
    SSL_shutdown(s);
    SSL_free(s);
    close(fd);
    return r;
}

int main(int argc, char **argv) {
    long r = atol(argv[1]) / 2;
    if (r < 1) r = 1;
    int port = atoi(argv[2]);
    SSL_CTX *ctx = SSL_CTX_new(TLS_client_method());
    SSL_CTX_set_verify(ctx, 1, NULL);
    SSL_CTX_load_verify_locations(ctx, argv[3], NULL);
    SSL_CTX_set_options(ctx, 1ULL << 7);
    long acc = 0;
    double t0 = now();
    for (long k = 0; k < r; k++) acc += get(ctx, port, "/hello.txt");
    printf("get.small %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) acc += get(ctx, port, "/64k.bin");
    printf("get.64k %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    SSL_CTX_free(ctx);
    return 0;
}
