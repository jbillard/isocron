// The ceiling of the machine for the work of `hello.isc` (DECISIONS D355):
// the cheapest way known to answer `hello, world!` to each request of HTTP/1.1
// kept alive, the same 130 bytes as the server of Isocron. Each of THREADS
// threads its own listener on PORT (`SO_REUSEPORT`: the kernel spreads the
// connections), its own `epoll`; a request is found by its end (CR LF CR
// LF), the response made once and copied (its date made again each second);
// one `recv` and one `send` for the requests a read brings. Nothing else: no
// parsing of the head, no allocation. The floor of the server, its ceiling
// of requests a second.
//   ceiling PORT THREADS
#define _GNU_SOURCE
#include <errno.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/epoll.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

enum { MAXFD = 65536, BUF = 4096 };
static int port;

typedef struct {
    char *buf[MAXFD];
    int len[MAXFD];
} State;

static int response(char *out, time_t *made) {
    static const char *days[] = { "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat" };
    static const char *months[] = { "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec" };
    time_t now = time(NULL);
    struct tm t;
    gmtime_r(&now, &t);
    *made = now;
    return sprintf(out, "HTTP/1.1 200 OK\r\nDate: %s, %02d %s %d %02d:%02d:%02d GMT\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 13\r\n\r\nhello, world!",
                   days[t.tm_wday], t.tm_mday, months[t.tm_mon], t.tm_year + 1900, t.tm_hour, t.tm_min, t.tm_sec);
}

static void *serve(void *arg) {
    (void)arg;
    State *st = calloc(1, sizeof(State));
    int l = socket(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0), one = 1;
    setsockopt(l, SOL_SOCKET, SO_REUSEPORT, &one, sizeof one);
    struct sockaddr_in a = { .sin_family = AF_INET, .sin_port = htons(port), .sin_addr.s_addr = htonl(0x7f000001) };
    if (bind(l, (struct sockaddr *)&a, sizeof a) != 0 || listen(l, 4096) != 0) {
        perror("listen");
        exit(1);
    }
    int ep = epoll_create1(0);
    struct epoll_event ev = { .events = EPOLLIN, .data.fd = l };
    epoll_ctl(ep, EPOLL_CTL_ADD, l, &ev);
    char resp[256], out[256 * 64];
    time_t made;
    int rl = response(resp, &made);
    struct epoll_event evs[512];
    for (;;) {
        int n = epoll_wait(ep, evs, 512, 1000);
        if (time(NULL) != made) rl = response(resp, &made);
        for (int i = 0; i < n; i++) {
            int fd = evs[i].data.fd;
            if (fd == l) {
                int c;
                while ((c = accept4(l, NULL, NULL, SOCK_NONBLOCK)) >= 0) {
                    setsockopt(c, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one);
                    if (!st->buf[c]) st->buf[c] = malloc(BUF);
                    st->len[c] = 0;
                    struct epoll_event ce = { .events = EPOLLIN, .data.fd = c };
                    epoll_ctl(ep, EPOLL_CTL_ADD, c, &ce);
                }
                continue;
            }
            char *b = st->buf[fd];
            ssize_t got = recv(fd, b + st->len[fd], BUF - st->len[fd], 0);
            if (got <= 0) {
                if (got < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) continue;
                close(fd);
                continue;
            }
            int len = st->len[fd] + (int)got, at = 0, k = 0;
            // each request by the end of its head
            for (int j = 0; j + 3 < len; j++) {
                if (b[j] == '\r' && b[j + 1] == '\n' && b[j + 2] == '\r' && b[j + 3] == '\n') {
                    if (k < 64) memcpy(out + k * rl, resp, rl);
                    k++;
                    at = j + 4;
                    j += 3;
                }
            }
            if (k > 0) send(fd, out, (size_t)(k < 64 ? k : 64) * rl, MSG_NOSIGNAL);
            memmove(b, b + at, len - at);
            st->len[fd] = len - at;
        }
    }
    return NULL;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: ceiling PORT THREADS\n");
        return 2;
    }
    port = atoi(argv[1]);
    int threads = atoi(argv[2]);
    pthread_t t[64];
    for (int i = 0; i < threads; i++) pthread_create(&t[i], NULL, serve, NULL);
    for (int i = 0; i < threads; i++) pthread_join(t[i], NULL);
    return 0;
}
