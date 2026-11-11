// The floor of `unix.isc`: the same work, the calls of the system made
// directly in C, on one thread. A round trip of 64 bytes on a Unix socket;
// 64 bytes sent with a descriptor (`sendmsg`, `SCM_RIGHTS`), received with
// it (`recvmsg`), the copy closed; a connection made (`connect`, `accept4`,
// both closed); a listener made and closed; `poll` of a socket that has
// bytes; a memory shared of 64 KiB made and given back (`memfd_create`,
// `ftruncate`, `mmap`, `munmap`, `close`); 64 KiB and 1 MiB written into
// it, 64 KiB read from it (into a buffer of the program); a descriptor
// projected read only and given back. Each line: a name, the nanoseconds of
// one (the mean over `rounds`), a checksum.
//   unix <rounds>
#define _GNU_SOURCE
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec + t.tv_nsec * 1e-9;
}

// the address of the name `name` of the abstract space: its length
static socklen_t address(struct sockaddr_un *a, const char *name) {
    memset(a, 0, sizeof *a);
    a->sun_family = AF_UNIX;
    size_t n = strlen(name);
    memcpy(a->sun_path + 1, name, n);
    return (socklen_t)(2 + 1 + n);
}

static int listen_on(const char *name) {
    struct sockaddr_un a;
    socklen_t len = address(&a, name);
    int l = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (bind(l, (struct sockaddr *)&a, len) != 0 || listen(l, 128) != 0) return -1;
    return l;
}

static int connect_to(const char *name) {
    struct sockaddr_un a;
    socklen_t len = address(&a, name);
    int c = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (connect(c, (struct sockaddr *)&a, len) != 0) return -1;
    return c;
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

static void send_fd(int c, const char *x, size_t n, int fd) {
    struct iovec iov = {.iov_base = (void *)x, .iov_len = n};
    union {
        char buf[CMSG_SPACE(sizeof(int))];
        struct cmsghdr align;
    } ctl;
    memset(&ctl, 0, sizeof ctl);
    struct msghdr m = {.msg_iov = &iov, .msg_iovlen = 1, .msg_control = ctl.buf, .msg_controllen = sizeof ctl.buf};
    struct cmsghdr *h = CMSG_FIRSTHDR(&m);
    h->cmsg_len = CMSG_LEN(sizeof(int));
    h->cmsg_level = SOL_SOCKET;
    h->cmsg_type = SCM_RIGHTS;
    memcpy(CMSG_DATA(h), &fd, sizeof(int));
    sendmsg(c, &m, MSG_NOSIGNAL);
}

// bytes and descriptors received (at most 4), the descriptors closed: their
// count, and the bytes' count
static long recv_fds(int s, char *buf, size_t max) {
    struct iovec iov = {.iov_base = buf, .iov_len = max};
    char ctl[CMSG_SPACE(4 * sizeof(int))];
    struct msghdr m = {.msg_iov = &iov, .msg_iovlen = 1, .msg_control = ctl, .msg_controllen = sizeof ctl};
    long r = recvmsg(s, &m, MSG_CMSG_CLOEXEC);
    if (r < 0) return -1;
    long fds = 0;
    for (struct cmsghdr *h = CMSG_FIRSTHDR(&m); h; h = CMSG_NXTHDR(&m, h)) {
        if (h->cmsg_level == SOL_SOCKET && h->cmsg_type == SCM_RIGHTS) {
            int n = (h->cmsg_len - CMSG_LEN(0)) / sizeof(int);
            for (int i = 0; i < n; i++) {
                int fd;
                memcpy(&fd, CMSG_DATA(h) + i * sizeof(int), sizeof fd);
                close(fd);
                fds++;
            }
        }
    }
    return r + fds;
}

int main(int argc, char **argv) {
    long r = atol(argv[1]);
    char name[64];
    snprintf(name, sizeof name, "isocron-bench-unix-c-%ld", (long)getpid());
    int l = listen_on(name);
    int c = connect_to(name);
    int s = accept4(l, NULL, NULL, SOCK_CLOEXEC);
    char small[64], buf[65536];
    for (int i = 0; i < 64; i++) small[i] = (char)i;

    double t0 = now();
    long a = 0;
    for (long i = 0; i < r; i++) {
        write(c, small, 64);
        a += read_n(s, buf, 64);
        write(s, small, 64);
        a += read_n(c, buf, 64);
    }
    double t1 = now();
    printf("roundtrip.64 %f %ld\n", (t1 - t0) * 1e9 / r, a / r);

    int mfd = memfd_create("isocron", MFD_CLOEXEC);
    ftruncate(mfd, 65536);
    double t2 = now();
    long p = 0;
    for (long i = 0; i < r; i++) {
        send_fd(c, small, 64, mfd);
        p += recv_fds(s, buf, 64);
    }
    double t3 = now();
    printf("sendWithFds.64 %f %ld\n", (t3 - t2) * 1e9 / r, p / r);
    close(mfd);

    write(c, small, 64);
    struct pollfd pf = {.fd = s, .events = POLLIN};
    double t4 = now();
    long q = 0;
    for (long i = 0; i < r; i++) {
        pf.revents = 0;
        poll(&pf, 1, 0);
        q += (pf.revents & POLLIN) ? 1 : 0;
    }
    double t5 = now();
    printf("poll.ready %f %ld\n", (t5 - t4) * 1e9 / r, q / r);
    close(s);
    close(c);

    double t6 = now();
    long n = 0;
    for (long i = 0; i < r; i++) {
        int cc = connect_to(name);
        int ss = accept4(l, NULL, NULL, SOCK_CLOEXEC);
        close(ss);
        close(cc);
        n += cc >= 0 && ss >= 0;
    }
    double t7 = now();
    printf("connect %f %ld\n", (t7 - t6) * 1e9 / r, n / r);
    close(l);

    char lname[80];
    snprintf(lname, sizeof lname, "%s-l", name);
    double t8 = now();
    long o = 0;
    for (long i = 0; i < r; i++) {
        int ll = listen_on(lname);
        o += ll >= 0;
        close(ll);
    }
    double t9 = now();
    printf("listen.close %f %ld\n", (t9 - t8) * 1e9 / r, o / r);

    double t10 = now();
    long sh = 0;
    for (long i = 0; i < r; i++) {
        int fd = memfd_create("isocron", MFD_CLOEXEC);
        ftruncate(fd, 65536);
        void *m = mmap(NULL, 65536, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        sh += m != MAP_FAILED ? 65536 : 0;
        munmap(m, 65536);
        close(fd);
    }
    double t11 = now();
    printf("shared.close.64k %f %ld\n", (t11 - t10) * 1e9 / r, sh / r);

    int fd = memfd_create("isocron", MFD_CLOEXEC);
    ftruncate(fd, 1048576);
    char *m = mmap(NULL, 1048576, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    uint8_t *bytes64k = malloc(65536);
    for (int i = 0; i < 65536; i++) bytes64k[i] = (uint8_t)(i % 251);
    double t12 = now();
    long wb = 0;
    for (long i = 0; i < r; i++) {
        memcpy(m, bytes64k, 65536);
        __asm__ volatile("" ::: "memory");
        wb += 1;
    }
    double t13 = now();
    printf("writeShared.64k %f %ld\n", (t13 - t12) * 1e9 / r, wb / r);
    uint32_t *words = malloc(1048576);
    for (int i = 0; i < 262144; i++) words[i] = (uint32_t)i;
    double t14 = now();
    long ww = 0;
    for (long i = 0; i < r; i++) {
        memcpy(m, words, 1048576);
        __asm__ volatile("" ::: "memory");
        ww += 1;
    }
    double t15 = now();
    printf("writeSharedU32.1m %f %ld\n", (t15 - t14) * 1e9 / r, ww / r);
    double t16 = now();
    long rd = 0;
    for (long i = 0; i < r; i++) {
        memcpy(buf, m, 65536);
        __asm__ volatile("" ::: "memory");
        rd += 65536 + (uint8_t)buf[100];
    }
    double t17 = now();
    printf("readShared.64k %f %ld\n", (t17 - t16) * 1e9 / r, rd / r);
    double t18 = now();
    long mp = 0;
    for (long i = 0; i < r; i++) {
        void *v = mmap(NULL, 1048576, PROT_READ, MAP_PRIVATE, fd, 0);
        mp += v != MAP_FAILED ? 1048576 : 0;
        munmap(v, 1048576);
    }
    double t19 = now();
    printf("mapShared %f %ld\n", (t19 - t18) * 1e9 / r, mp / r);
    munmap(m, 1048576);
    close(fd);
    return 0;
}
