/* The latency and the throughput of vaddpd on this core (D549): 1e8 adds
 * in K chains of independent registers (zmm_K, ymm_K); the cycles of an
 * add are the latency over K until the adders are full. The reason of
 * V = 32 (D55): latency times adders times the doubles of a register.
 *   gcc -O1 -mavx512f bench/floor/fadd_chains.c -o fadd_chains
 *   perf stat -e cycles:u ./fadd_chains zmm_4   (between machine.sh apply
 *   and restore, on the CPU of the measurements) */
#include <stdio.h>
#include <stdlib.h>
#define REP 100000000L
#define CH(w, n) \
  static void w##_##n(void) { \
    for (long i = 0; i < REP / n; i++) { \
      __asm__ volatile(ADDS_##n(w) ::: "xmm0","xmm1","xmm2","xmm3","xmm4","xmm5","xmm6","xmm7","xmm8","xmm9","xmm10","xmm11","xmm12","xmm13","xmm14","xmm15"); \
    } \
  }
#define A(w, r) "vaddpd %%" #w "15, %%" #w #r ", %%" #w #r "\n\t"
#define ADDS_1(w) A(w,0)
#define ADDS_2(w) A(w,0) A(w,1)
#define ADDS_4(w) A(w,0) A(w,1) A(w,2) A(w,3)
#define ADDS_8(w) ADDS_4(w) A(w,4) A(w,5) A(w,6) A(w,7)
#define ADDS_12(w) ADDS_8(w) A(w,8) A(w,9) A(w,10) A(w,11)
CH(zmm, 1) CH(zmm, 2) CH(zmm, 4) CH(zmm, 8) CH(zmm, 12)
CH(ymm, 1) CH(ymm, 2) CH(ymm, 4) CH(ymm, 8) CH(ymm, 12)
int main(int argc, char **argv) {
    const char *k = argv[1];
#define RUN(w, n) if (!__builtin_strcmp(k, #w "_" #n)) w##_##n();
    RUN(zmm,1) RUN(zmm,2) RUN(zmm,4) RUN(zmm,8) RUN(zmm,12)
    RUN(ymm,1) RUN(ymm,2) RUN(ymm,4) RUN(ymm,8) RUN(ymm,12)
    return 0;
}
