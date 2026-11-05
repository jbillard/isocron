// The floor of `sys.isc`: the same work in C, the system called the
// cheapest way known (stat, getenv, open/read/close, open/write/close,
// getdents64 and a sort of the names, the paths by hand), on one thread.
// Each line: a name, the nanoseconds of one call (the mean over `rounds`),
// a checksum.
//   sys <rounds>
#define _GNU_SOURCE
#include <dirent.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e9 + t.tv_nsec;
}

static int cmp(const void *a, const void *b) {
    return strcmp(*(char *const *)a, *(char *const *)b);
}

// the names of the folder `dir`, sorted, without . and ..: their number
static long list(const char *dir) {
    static char buf[32768];
    static char *names[256];
    int fd = open(dir, O_RDONLY | O_DIRECTORY | O_CLOEXEC);
    if (fd < 0) return -1;
    long k = 0;
    for (;;) {
        long n = syscall(SYS_getdents64, fd, buf, sizeof buf);
        if (n <= 0) break;
        for (long at = 0; at < n;) {
            unsigned short len = *(unsigned short *)(buf + at + 16);
            char *name = buf + at + 19;
            if (strcmp(name, ".") && strcmp(name, "..") && k < 256) names[k++] = strdup(name);
            at += len;
        }
    }
    close(fd);
    qsort(names, k, sizeof *names, cmp);
    for (long i = 0; i < k; i++) free(names[i]);
    return k;
}

// the length of the normal form of `p` (no ., x/.., repeated /)
static long normalize(const char *p, char *out) {
    const char *parts[64];
    int lens[64], k = 0, ups = 0;
    const char *s = p;
    while (*s) {
        const char *e = strchr(s, '/');
        int len = e ? e - s : (int)strlen(s);
        if (len == 0 || (len == 1 && s[0] == '.')) {
        } else if (len == 2 && s[0] == '.' && s[1] == '.') {
            if (k > 0) k--;
            else ups++;
        } else {
            parts[k] = s;
            lens[k++] = len;
        }
        if (!e) break;
        s = e + 1;
    }
    long m = 0;
    for (int i = 0; i < ups; i++) m += sprintf(out + m, "../");
    for (int i = 0; i < k; i++) {
        if (i) out[m++] = '/';
        memcpy(out + m, parts[i], lens[i]);
        m += lens[i];
    }
    if (m == 0) out[m++] = '.';
    out[m] = 0;
    return m;
}

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    const char *dir = "/tmp/isc-bench-sys-c";
    char path[256], buf[8192], x[4096];
    mkdir(dir, 0777);
    for (int i = 0; i < 4096; i++) x[i] = i % 251;
    for (int i = 0; i < 64; i++) {
        sprintf(path, "%s/f%d.bin", dir, i);
        int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0666);
        write(fd, x, sizeof x);
        close(fd);
    }
    sprintf(path, "%s/f7.bin", dir);
    long acc = 0;
    double t0 = now();
    // 100 calls a round, as `sys.isc` (D346)
    for (long i = 0; i < 100 * r; i++) {
        const char *v = getenv("HOME");
        acc += v ? strlen(v) : 0;
    }
    double t1 = now();
    printf("env %.3f %ld\n", (t1 - t0) / (100 * r), acc / (100 * r));
    acc = 0;
    struct stat st;
    for (long i = 0; i < r; i++) acc += stat(path, &st) == 0;
    double t2 = now();
    printf("exists %.3f %ld\n", (t2 - t1) / r, acc / r);
    acc = 0;
    for (long i = 0; i < r; i++) {
        int fd = open(path, O_RDONLY | O_CLOEXEC);
        long n = 0, k;
        while ((k = read(fd, buf + n, sizeof buf - n)) > 0) n += k;
        close(fd);
        acc += n;
    }
    double t3 = now();
    printf("readFile.4k %.3f %ld\n", (t3 - t2) / r, acc / r);
    acc = 0;
    char wpath[256];
    sprintf(wpath, "%s/w.bin", dir);
    for (long i = 0; i < r; i++) {
        int fd = open(wpath, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0666);
        acc += write(fd, x, sizeof x) == sizeof x;
        close(fd);
    }
    double t4 = now();
    printf("writeFile.4k %.3f %ld\n", (t4 - t3) / r, acc / r);
    acc = 0;
    for (long i = 0; i < r; i++) acc += list(dir);
    double t5 = now();
    printf("listDir.65 %.3f %ld\n", (t5 - t4) / r, acc / r);
    static char ps[4096][40], out[64];
    for (int i = 0; i < 4096; i++) sprintf(ps[i], "a/./b%d/../c//d/e%d.txt", i % 10, i);
    acc = 0;
    double t6 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < 4096; i++) acc += normalize(ps[i], out);
    double t7 = now();
    printf("normalize %.3f %ld\n", (t7 - t6) / ((double)r * 4096), acc / r);
    acc = 0;
    for (long k = 0; k < r; k++)
        for (int i = 0; i < 4096; i++) {
            const char *b = strrchr(ps[i], '/');
            acc += strlen(b ? b + 1 : ps[i]);
        }
    double t8 = now();
    printf("basename %.3f %ld\n", (t8 - t7) / ((double)r * 4096), acc / r);
    for (int i = 0; i < 64; i++) {
        sprintf(path, "%s/f%d.bin", dir, i);
        unlink(path);
    }
    unlink(wpath);
    rmdir(dir);
    return 0;
}
