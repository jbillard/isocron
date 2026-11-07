// The server of DNS of the benchmark of `net.lookup` (D414), on the local
// machine: each query answered with one record, 127.0.0.1 for the type A,
// ::1 for AAAA, nothing for the others. Started by `run.py` for a group
// that declares `// bench-server: dns`.
//   dns_server PORT
#include <arpa/inet.h>
#include <netinet/in.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

int main(int argc, char **argv) {
    int s = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in a = { .sin_family = AF_INET, .sin_port = htons(atoi(argv[1])), .sin_addr.s_addr = htonl(INADDR_LOOPBACK) };
    if (bind(s, (struct sockaddr *)&a, sizeof a)) return 1;
    unsigned char q[512], r[600];
    for (;;) {
        struct sockaddr_in from;
        socklen_t fl = sizeof from;
        ssize_t n = recvfrom(s, q, sizeof q, 0, (struct sockaddr *)&from, &fl);
        if (n < 12) continue;
        // the end of the question: its name, then its type and class
        size_t at = 12;
        while (at < (size_t)n && q[at] != 0) at += q[at] + 1;
        at += 5;
        if (at > (size_t)n) continue;
        int type = q[at - 3];
        memcpy(r, q, at);
        r[2] = 0x81; r[3] = 0x80;
        r[6] = 0; r[7] = (type == 1 || type == 28) ? 1 : 0;
        size_t m = at;
        if (r[7]) {
            unsigned char rec[12] = {0xc0, 0x0c, 0, (unsigned char)type, 0, 1, 0, 0, 0, 60, 0, type == 1 ? 4 : 16};
            memcpy(r + m, rec, 12);
            m += 12;
            if (type == 1) { unsigned char v4[4] = {127, 0, 0, 1}; memcpy(r + m, v4, 4); m += 4; }
            else { unsigned char v6[16] = {0}; v6[15] = 1; memcpy(r + m, v6, 16); m += 16; }
        }
        sendto(s, r, m, 0, (struct sockaddr *)&from, fl);
    }
}
