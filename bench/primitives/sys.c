// The floor of `sys.isc`: the same work in C, the system called the
// cheapest way known (stat, getenv, open/read/close, open/write/close,
// getdents64 and a sort of the names, the paths by hand), on one thread.
// Each line: a name, the nanoseconds of one call (the mean over `rounds`),
// a checksum.
//   sys <rounds>
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

// the bytes written into `p` kept: an empty `asm` that may read the memory
// (D456: GCC removed the writes of a text never read, and the floor
// measured its length alone)
#define KEEP(p) __asm__ volatile("" : : "r"(p) : "memory")

static double now(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1e9 + t.tv_nsec;
}

static int cmp(const void *a, const void *b) {
    return strcmp(*(char *const *)a, *(char *const *)b);
}

// an entry `name=value` and the length of its name (D435)
struct entry {
    const char *e;
    size_t n;
};

// by the bytes of the names, then by the whole entries, as `envAll` (D423)
static int by_name(const void *a, const void *b) {
    const struct entry *x = a, *y = b;
    size_t n = x->n < y->n ? x->n : y->n;
    int c = memcmp(x->e, y->e, n);
    if (c) return c;
    if (x->n != y->n) return x->n < y->n ? -1 : 1;
    return strcmp(x->e, y->e);
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

// the folder `p` and the folders above it that are not there: the full
// path first, its parent only when it is missing (D392)
static int make_dirs(char *p) {
    if (mkdir(p, 0777) == 0) return 0;
    struct stat st;
    if (errno == EEXIST) return stat(p, &st) == 0 && S_ISDIR(st.st_mode) ? 0 : -1;
    if (errno != ENOENT) return -1;
    char *slash = strrchr(p, '/');
    if (!slash || slash == p) return -1;
    *slash = 0;
    int r = make_dirs(p);
    *slash = '/';
    return r == 0 ? mkdir(p, 0777) : -1;
}

// the file or the folder `p` with all it holds (D392)
static int remove_all(const char *p) {
    struct stat st;
    if (lstat(p, &st) != 0) return -1;
    if (!S_ISDIR(st.st_mode)) return unlink(p);
    DIR *d = opendir(p);
    if (!d) return -1;
    struct dirent *e;
    char sub[512];
    while ((e = readdir(d))) {
        if (!strcmp(e->d_name, ".") || !strcmp(e->d_name, "..")) continue;
        snprintf(sub, sizeof sub, "%s/%s", p, e->d_name);
        remove_all(sub);
    }
    closedir(d);
    return rmdir(p);
}

// the bytes of `x` (n of them) written to `p`, appended or not
static int put_file(const char *p, const char *x, long n, int append) {
    int fd = open(p, O_WRONLY | O_CREAT | O_CLOEXEC | (append ? O_APPEND : O_TRUNC), 0666);
    if (fd < 0) return 0;
    int ok = write(fd, x, n) == n;
    close(fd);
    return ok;
}

extern char **environ;

int main(int argc, char **argv) {
    long r = argc > 1 ? atol(argv[1]) : 10;
    const char *dir = "/tmp/isc-bench-sys-c";
    char path[256], buf[8192], x[4096];
    // a new folder, as `sys.isc` (D392: the files of the runs before it
    // counted by `listDir`)
    remove_all(dir);
    mkdir(dir, 0777);
    for (int i = 0; i < 4096; i++) x[i] = i % 251;
    for (int i = 0; i < 64; i++) {
        sprintf(path, "%s/f%d.bin", dir, i);
        int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0666);
        write(fd, x, sizeof x);
        close(fd);
    }
    sprintf(path, "%s/f7.bin", dir);
    // the text of 4096 letters, in the folder before it is listed, as
    // `sys.isc` (D392)
    char words[4096], tpath[256];
    for (int i = 0; i < 4096; i++) words[i] = 'a' + i % 26;
    sprintf(tpath, "%s/t.txt", dir);
    put_file(tpath, words, sizeof words, 0);
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
    printf("listDir.66 %.3f %ld\n", (t5 - t4) / r, acc / r);
    static char ps[4096][40], out[64];
    for (int i = 0; i < 4096; i++) sprintf(ps[i], "a/./b%d/../c//d/e%d.txt", i % 10, i);
    acc = 0;
    double t6 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < 4096; i++) {
            acc += normalize(ps[i], out);
            KEEP(out);
        }
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
    // the other functions of `path` (D392): one path in three absolute
    static char qs[4096][48], jo[64];
    for (int i = 0; i < 4096; i++) sprintf(qs[i], i % 3 == 0 ? "/x/%s" : "%s", ps[i]);
    acc = 0;
    double t9 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < 4096; i++) {
            const char *p = qs[i];
            acc += p[0] == '/' || (p[0] && p[1] == ':' && (p[2] == '/' || p[2] == '\\'));
        }
    double t10 = now();
    printf("isAbsolute %.3f %ld\n", (t10 - t9) / ((double)r * 4096), acc / r);
    acc = 0;
    for (long k = 0; k < r; k++)
        for (int i = 0; i < 4096; i++) {
            const char *p = qs[i];
            size_t n = strlen(p);
            if (p[0] == '/') {
                memcpy(jo, p, n);
                KEEP(jo);
                acc += n;
            } else {
                memcpy(jo, "dir/sub/", 8);
                memcpy(jo + 8, p, n);
                KEEP(jo);
                acc += 8 + n;
            }
        }
    double t11 = now();
    printf("join %.3f %ld\n", (t11 - t10) / ((double)r * 4096), acc / r);
    acc = 0;
    for (long k = 0; k < r; k++)
        for (int i = 0; i < 4096; i++) {
            const char *p = ps[i], *e = strrchr(p, '/');
            long n = e ? e - p : -1;
            while (n > 0 && p[n - 1] == '/') n--;
            acc += n < 0 ? 1 : (n == 0 ? 1 : n);
        }
    double t12 = now();
    printf("dirname %.3f %ld\n", (t12 - t11) / ((double)r * 4096), acc / r);
    acc = 0;
    for (long k = 0; k < r; k++)
        for (int i = 0; i < 4096; i++) {
            const char *b = strrchr(ps[i], '/');
            b = b ? b + 1 : ps[i];
            const char *d = strrchr(b, '.');
            acc += d && d > b && strcmp(b, "..") ? (long)strlen(d) : 0;
        }
    double t13 = now();
    printf("extension %.3f %ld\n", (t13 - t12) / ((double)r * 4096), acc / r);
    // the other functions of `sys` (D392): the text of an error of each of
    // 8 kinds about `f.txt`
    static const char *kinds[6] = {"not found: ", "permission denied: ", "already exists: ", "not empty: ", "refused: ", "timed out: "};
    char msg[64];
    acc = 0;
    double u0 = now();
    for (long k = 0; k < r; k++)
        for (int i = 0; i < 4096; i++) {
            int code = i % 8 + 1;
            long n;
            if (code <= 6) {
                n = strlen(kinds[code - 1]);
                memcpy(msg, kinds[code - 1], n);
            } else {
                n = sprintf(msg, "error %d of the system: ", -code);
            }
            memcpy(msg + n, "f.txt", 5);
            KEEP(msg);
            acc += n + 5;
        }
    double u1 = now();
    printf("ioError.message %.3f %ld\n", (u1 - u0) / ((double)r * 4096), acc / r);
    char p2[256], p3[256];
    acc = 0;
    for (long i = 0; i < r; i++) {
        // the entries sorted by their names, as `sysEnviron` sorts them (the
        // floor without the sort did less than `envAll`, D400; by name since
        // D423, each name measured once, D435), then each a name and a value
        // made
        static struct entry es[1024];
        long m = 0;
        for (char **e = environ; *e && m < 1024; e++) {
            const char *eq = strchr(*e, '=');
            es[m].e = *e;
            es[m++].n = eq ? (size_t)(eq - *e) : strlen(*e);
        }
        qsort(es, m, sizeof *es, by_name);
        long k = 0;
        for (long j = 0; j < m; j++) {
            const char *eq = es[j].e + es[j].n;
            char *name = strndup(es[j].e, es[j].n), *value = strdup(*eq ? eq + 1 : "");
            free(name);
            free(value);
            k++;
        }
        acc += k;
    }
    double u2 = now();
    printf("envAll %.3f %ld\n", (u2 - u1) / r, acc / r);
    acc = 0;
    for (long i = 0; i < r; i++) acc += getcwd(p2, sizeof p2) ? (long)strlen(p2) : -1;
    double u3 = now();
    printf("cwd %.3f %ld\n", (u3 - u2) / r, acc / r);
    acc = 0;
    for (long i = 0; i < r; i++) acc += stat(path, &st) == 0 && S_ISREG(st.st_mode);
    double u4 = now();
    printf("isFile %.3f %ld\n", (u4 - u3) / r, acc / r);
    acc = 0;
    for (long i = 0; i < r; i++) acc += stat(dir, &st) == 0 && S_ISDIR(st.st_mode);
    double u5 = now();
    printf("isDir %.3f %ld\n", (u5 - u4) / r, acc / r);
    acc = 0;
    for (long i = 0; i < r; i++) acc += stat(path, &st) == 0 ? st.st_size : -1;
    double u6 = now();
    printf("fileSize %.3f %ld\n", (u6 - u5) / r, acc / r);
    acc = 0;
    for (long i = 0; i < r; i++) acc += stat(path, &st) == 0 ? st.st_mtim.tv_sec > 0 : -1;
    double u7 = now();
    printf("modified %.3f %ld\n", (u7 - u6) / r, acc / r);
    acc = 0;
    sprintf(p2, "%s/m", dir);
    for (long i = 0; i < r; i++) acc += (mkdir(p2, 0777) == 0) + (rmdir(p2) == 0);
    double u8 = now();
    printf("makeDir.remove %.3f %ld\n", (u8 - u7) / r, acc / r);
    acc = 0;
    for (long i = 0; i < r; i++) {
        sprintf(p2, "%s/a/b/c", dir);
        sprintf(p3, "%s/a", dir);
        acc += (make_dirs(p2) == 0) + (remove_all(p3) == 0);
    }
    double u9 = now();
    printf("makeDirs.removeAll %.3f %ld\n", (u9 - u8) / r, acc / r);
    acc = 0;
    sprintf(p2, "%s/g7.bin", dir);
    for (long i = 0; i < r; i++) acc += (rename(path, p2) == 0) + (rename(p2, path) == 0);
    double u10 = now();
    printf("rename.2 %.3f %ld\n", (u10 - u9) / r, acc / r);
    acc = 0;
    for (long i = 0; i < r; i++) {
        int fd = open(tpath, O_RDONLY | O_CLOEXEC);
        long n = 0, k;
        while ((k = read(fd, buf + n, sizeof buf - n)) > 0) n += k;
        close(fd);
        // the text checked as UTF-8: ASCII here, 8 bytes at a time
        long bad = 0;
        for (long j = 0; j + 8 <= n; j += 8) {
            unsigned long w;
            memcpy(&w, buf + j, 8);
            bad |= w & 0x8080808080808080ul;
        }
        acc += bad ? -1 : n;
    }
    double u11 = now();
    printf("readText.4k %.3f %ld\n", (u11 - u10) / r, acc / r);
    acc = 0;
    sprintf(p2, "%s/wt.txt", dir);
    for (long i = 0; i < r; i++) acc += put_file(p2, words, sizeof words, 0);
    double u12 = now();
    printf("writeText.4k %.3f %ld\n", (u12 - u11) / r, acc / r);
    acc = 0;
    sprintf(p2, "%s/ap.bin", dir);
    for (long i = 0; i < r; i++) acc += put_file(p2, x, 64, 1);
    double u13 = now();
    printf("appendFile.64 %.3f %ld\n", (u13 - u12) / r, acc / r);
    acc = 0;
    sprintf(p2, "%s/at.txt", dir);
    for (long i = 0; i < r; i++) acc += put_file(p2, words, 64, 1);
    double u14 = now();
    printf("appendText.64 %.3f %ld\n", (u14 - u13) / r, acc / r);
    for (int i = 0; i < 64; i++) {
        sprintf(path, "%s/f%d.bin", dir, i);
        unlink(path);
    }
    unlink(wpath);
    rmdir(dir);
    remove_all(dir);
    return 0;
}
