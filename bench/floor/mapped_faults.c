/* The pages of a file in memory (mincore), then its minor and major
 * faults when one byte a page is read through a private mapping, as the
 * runtime maps a large standard input (D257, D551).
 *   gcc -O2 bench/floor/mapped_faults.c -o mapped_faults; ./mapped_faults FILE */
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <unistd.h>
int main(int argc, char **argv) {
    int fd = open(argv[1], O_RDONLY);
    struct stat st; fstat(fd, &st);
    size_t pages = (st.st_size + 4095) / 4096;
    unsigned char *p = mmap(0, st.st_size, PROT_READ, MAP_PRIVATE, fd, 0);
    unsigned char *v = malloc(pages);
    mincore(p, st.st_size, v);
    size_t in = 0;
    for (size_t i = 0; i < pages; i++) in += v[i] & 1;
    struct rusage a, b; getrusage(RUSAGE_SELF, &a);
    unsigned long sum = 0;
    for (off_t i = 0; i < st.st_size; i += 4096) sum += p[i];
    getrusage(RUSAGE_SELF, &b);
    printf("%zu pages, %zu in memory before; %ld minor and %ld major faults (sum %lu)\n", pages, in, b.ru_minflt - a.ru_minflt, b.ru_majflt - a.ru_majflt, sum);
    return 0;
}
