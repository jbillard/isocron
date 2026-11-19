/* nothp COMMAND ARGS...: runs COMMAND without transparent huge pages
 * (PR_SET_THP_DISABLE, kept across fork and execve: the workers of nginx
 * too), so that the memory of every server is counted in pages of 4 KiB
 * (with huge pages a peak jumps by 2 MiB at once). */
#include <stdio.h>
#include <sys/prctl.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: nothp COMMAND ARGS...\n");
        return 2;
    }
    if (prctl(PR_SET_THP_DISABLE, 1, 0, 0, 0) != 0) {
        perror("prctl");
        return 1;
    }
    execvp(argv[1], argv + 1);
    perror("execvp");
    return 1;
}
