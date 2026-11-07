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
#include <fcntl.h>
#include <sys/epoll.h>
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

int SSL_CTX_set_default_verify_paths(SSL_CTX *c);
const SSL_METHOD *TLS_server_method(void);
typedef struct bio_st BIO;
typedef struct x509_st X509;
typedef struct evp_pkey_st EVP_PKEY;
BIO *BIO_new_mem_buf(const void *buf, int len);
int BIO_free(BIO *b);
X509 *PEM_read_bio_X509(BIO *b, X509 **x, void *cb, void *u);
EVP_PKEY *PEM_read_bio_PrivateKey(BIO *b, EVP_PKEY **x, void *cb, void *u);
void X509_free(X509 *x);
void EVP_PKEY_free(EVP_PKEY *k);
void ERR_clear_error(void);
int SSL_CTX_use_certificate(SSL_CTX *c, X509 *x);
int SSL_CTX_use_PrivateKey(SSL_CTX *c, EVP_PKEY *k);
int SSL_CTX_check_private_key(const SSL_CTX *c);
long SSL_CTX_ctrl(SSL_CTX *c, int cmd, long larg, void *parg);

// the file `name` next to the authority `ca` into `out`: its length
static long read_file(const char *ca, const char *name, char *out, char *path) {
    size_t n = strlen(ca);
    while (n > 0 && ca[n - 1] != '/') n--;
    snprintf(path, 4096, "%.*s%s", (int)n, ca, name);
    FILE *f = fopen(path, "rb");
    if (!f) return 0;
    long k = (long)fread(out, 1, 4095, f);
    fclose(f);
    return k;
}

// a server of TLS on a new listener (D431): its context with the
// certificate and the key read from their texts, its `epoll` watching the
// listener, then all closed: 1, or 0
static long tls_server_once(const char *cert, long cn, const char *key, long kn) {
    int l = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0), one = 1;
    setsockopt(l, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
    struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = 0, .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
    bind(l, (struct sockaddr *)&a, sizeof a);
    listen(l, 128);
    SSL_CTX *c = SSL_CTX_new(TLS_server_method());
    SSL_CTX_ctrl(c, 123, 0x303, NULL);
    SSL_CTX_set_options(c, 1ULL << 7);
    SSL_CTX_ctrl(c, 33, 2, NULL);
    BIO *b = BIO_new_mem_buf(cert, (int)cn);
    X509 *x = PEM_read_bio_X509(b, NULL, NULL, NULL);
    while (PEM_read_bio_X509(b, NULL, NULL, NULL)) {
    }
    ERR_clear_error();
    BIO_free(b);
    int ok = x && SSL_CTX_use_certificate(c, x) == 1;
    X509_free(x);
    BIO *kb = BIO_new_mem_buf(key, (int)kn);
    EVP_PKEY *k = PEM_read_bio_PrivateKey(kb, NULL, NULL, NULL);
    BIO_free(kb);
    ok = ok && k && SSL_CTX_use_PrivateKey(c, k) == 1 && SSL_CTX_check_private_key(c) == 1;
    EVP_PKEY_free(k);
    int ep = epoll_create1(EPOLL_CLOEXEC);
    fcntl(l, F_SETFL, fcntl(l, F_GETFL) | O_NONBLOCK);
    struct epoll_event e = {.events = EPOLLIN, .data.u64 = 0};
    epoll_ctl(ep, EPOLL_CTL_ADD, l, &e);
    close(ep);
    SSL_CTX_free(c);
    close(l);
    return ok;
}

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

static char buf[1 << 20];

// one request of `path` with the header lines `extra`: its status plus the
// length of its body, or -1
static long get_with(SSL_CTX *ctx, int port, const char *path, const char *extra);

// one request of `path`: its status plus the length of its body, or -1
static long get(SSL_CTX *ctx, int port, const char *path) {
    return get_with(ctx, port, path, "");
}

static long get_with(SSL_CTX *ctx, int port, const char *path, const char *extra) {
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
        int k = snprintf(req, sizeof req, "GET %s HTTP/1.1\r\nHost: localhost:%d\r\nUser-Agent: isocron\r\nConnection: close\r\n%s\r\n", path, port, extra);
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
    // the request with a header (D424), and the handshake alone
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) acc += get_with(ctx, port, "/hello.txt", "Accept: text/plain\r\n");
    printf("requestWith.small %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) {
        int fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
        struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = htons(port)};
        inet_pton(AF_INET, "127.0.0.1", &a.sin_addr);
        if (connect(fd, (struct sockaddr *)&a, sizeof a) == 0) {
            SSL *s = SSL_new(ctx);
            SSL_ctrl(s, 55, 0, (void *)"localhost");
            SSL_set1_host(s, "localhost");
            SSL_set_fd(s, fd);
            acc += SSL_connect(s) == 1;
            SSL_shutdown(s);
            SSL_free(s);
        }
        close(fd);
    }
    printf("startTls %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    SSL_CTX_free(ctx);
    // the authorities of the system: `SSL_CERT_FILE`, as for Isocron (D431)
    SSL_CTX *sys = SSL_CTX_new(TLS_client_method());
    SSL_CTX_set_verify(sys, 1, NULL);
    SSL_CTX_set_default_verify_paths(sys);
    SSL_CTX_set_options(sys, 1ULL << 7);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) acc += get(sys, port, "/hello.txt");
    printf("get.system.small %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) acc += get_with(sys, port, "/hello.txt", "Accept: text/plain\r\n");
    printf("request.system.small %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    SSL_CTX_free(sys);
    // a server of TLS on a listener, opened and closed: the calls of the
    // runtime (`server_ctx`, then `serverOpen`)
    char cert[4096], key[4096], path[4096];
    long cn = read_file(argv[3], "server.pem", cert, path), kn = read_file(argv[3], "server.key", key, path);
    acc = 0;
    t0 = now();
    for (long k = 0; k < r; k++) acc += tls_server_once(cert, cn, key, kn);
    printf("serverOfTls.close %.3f %ld\n", (now() - t0) * 1e9 / r, acc / r);
    return 0;
}
