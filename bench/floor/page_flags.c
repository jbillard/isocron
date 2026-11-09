/* The flags of the pages of a file (/proc/kpageflags, as root): mapped
 * privately and read one byte a page, then each page's frame from
 * /proc/self/pagemap and its flags counted (D551).
 *   gcc -O2 bench/floor/page_flags.c -o page_flags
 *   sudo systemd-run --quiet --pipe --wait ./page_flags FILE */
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>
int main(int argc, char **argv) {
    int fd = open(argv[1], O_RDONLY);
    struct stat st; fstat(fd, &st);
    size_t pages = (st.st_size + 4095) / 4096;
    volatile unsigned char *p = mmap(0, st.st_size, PROT_READ, MAP_PRIVATE, fd, 0);
    unsigned long sum = 0;
    for (size_t i = 0; i < pages; i++) sum += p[i * 4096];
    int pm = open("/proc/self/pagemap", O_RDONLY), kf = open("/proc/kpageflags", O_RDONLY);
    size_t count[64] = {0}, present = 0;
    for (size_t i = 0; i < pages; i++) {
        uint64_t e, f;
        pread(pm, &e, 8, ((uintptr_t)p / 4096 + i) * 8);
        if (!(e >> 63)) continue;
        present++;
        pread(kf, &f, 8, (e & ((1ULL << 55) - 1)) * 8);
        for (int b = 0; b < 64; b++) count[b] += (f >> b) & 1;
    }
    const char *names[] = {"LOCKED","ERROR","REFERENCED","UPTODATE","DIRTY","LRU","ACTIVE","SLAB","WRITEBACK","RECLAIM","BUDDY","MMAP","ANON","SWAPCACHE","SWAPBACKED","COMPOUND_HEAD","COMPOUND_TAIL","HUGE","UNEVICTABLE","HWPOISON","NOPAGE","KSM","THP","OFFLINE","ZERO_PAGE","IDLE","PGTABLE"};
    printf("%zu pages, %zu present:", pages, present);
    for (int b = 0; b < 27; b++) if (count[b]) printf(" %s %zu", names[b], count[b]);
    printf(" (sum %lu)\n", sum);
    return 0;
}
