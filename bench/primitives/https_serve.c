#define _GNU_SOURCE
// The floor of `https_serve.isc` (D416): a server of OpenSSL called directly,
// one thread, one connection at a time: the handshake, each request read to
// the end of its head, the response of Isocron (made once: its date, its
// headers, `hello, world!`; `Connection: close` added when the request asks
// for it) written, until as many requests as the client makes.
//   https_serve <rounds> <port> <cert.pem> <key.pem>
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
const SSL_METHOD *TLS_server_method(void);
SSL_CTX *SSL_CTX_new(const SSL_METHOD *m);
int SSL_CTX_use_certificate_chain_file(SSL_CTX *c, const char *f);
int SSL_CTX_use_PrivateKey_file(SSL_CTX *c, const char *f, int type);
SSL *SSL_new(SSL_CTX *c);
int SSL_set_fd(SSL *s, int fd);
int SSL_accept(SSL *s);
int SSL_read(SSL *s, void *b, int n);
int SSL_write(SSL *s, const void *b, int n);
int SSL_shutdown(SSL *s);
void SSL_free(SSL *s);

int main(int argc, char **argv) {
    long r = atol(argv[1]);
    long limit = (r / 2 > 1 ? r / 2 : 1) + 64 * (r / 8 > 1 ? r / 8 : 1);
    SSL_CTX *ctx = SSL_CTX_new(TLS_server_method());
    if (SSL_CTX_use_certificate_chain_file(ctx, argv[3]) != 1 || SSL_CTX_use_PrivateKey_file(ctx, argv[4], 1) != 1) return 1;
    char date[64], keep[256], closing[256];
    time_t now = time(NULL);
    strftime(date, sizeof date, "%a, %d %b %Y %H:%M:%S GMT", gmtime(&now));
    int kl = snprintf(keep, sizeof keep, "HTTP/1.1 200 OK\r\nDate: %s\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 13\r\n\r\nhello, world!", date);
    int cl = snprintf(closing, sizeof closing, "HTTP/1.1 200 OK\r\nDate: %s\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 13\r\nConnection: close\r\n\r\nhello, world!", date);
    int l = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0), one = 1;
    setsockopt(l, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
    struct sockaddr_in a = {.sin_family = AF_INET, .sin_port = htons(atoi(argv[2])), .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
    if (bind(l, (struct sockaddr *)&a, sizeof a) || listen(l, 128)) return 1;
    long served = 0;
    static char buf[16384];
    while (served < limit) {
        int c = accept4(l, NULL, NULL, SOCK_CLOEXEC);
        if (c < 0) continue;
        // as the runtime does for each connection it accepts
        setsockopt(c, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one);
        SSL *s = SSL_new(ctx);
        SSL_set_fd(s, c);
        if (SSL_accept(s) == 1) {
            int n = 0;
            for (;;) {
                int k = SSL_read(s, buf + n, sizeof buf - 1 - n);
                if (k <= 0) break;
                n += k;
                buf[n] = 0;
                char *end;
                int closed = 0;
                // each request whole in the buffer answered
                while ((end = strstr(buf, "\r\n\r\n"))) {
                    int head = end + 4 - buf;
                    int close_it = memmem(buf, head, "Connection: close", 17) != NULL;
                    SSL_write(s, close_it ? closing : keep, close_it ? cl : kl);
                    served++;
                    memmove(buf, buf + head, n - head + 1);
                    n -= head;
                    if (close_it) { closed = 1; break; }
                }
                if (closed) break;
            }
            SSL_shutdown(s);
        }
        SSL_free(s);
        close(c);
    }
    return 0;
}
