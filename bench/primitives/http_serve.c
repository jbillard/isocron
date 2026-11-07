// The floor of `http_serve.isc` (D431): a server of one thread on the
// system's calls, one connection at a time: each request read to the end of
// its head, the response of Isocron (made once: its date, its headers,
// `hello, world!`; `Connection: close` added when the request asks for it)
// written, until as many requests as the client makes.
//   http_serve <rounds> <port>
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

int main(int argc, char **argv) {
    long r = atol(argv[1]);
    long limit = (r / 2 > 1 ? r / 2 : 1) + 64 * (r / 8 > 1 ? r / 8 : 1);
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
        {
            int n = 0;
            for (;;) {
                int k = (int)recv(c, buf + n, sizeof buf - 1 - n, 0);
                if (k <= 0) break;
                n += k;
                buf[n] = 0;
                char *end;
                int closed = 0;
                // each request whole in the buffer answered
                while ((end = strstr(buf, "\r\n\r\n"))) {
                    int head = end + 4 - buf;
                    int close_it = memmem(buf, head, "Connection: close", 17) != NULL;
                    send(c, close_it ? closing : keep, close_it ? cl : kl, MSG_NOSIGNAL);
                    served++;
                    memmove(buf, buf + head, n - head + 1);
                    n -= head;
                    if (close_it) { closed = 1; break; }
                }
                if (closed) break;
            }
        }
        close(c);
    }
    return 0;
}
