/* Frees a block twice: the allocator of Isocron must stop the program
 * with a panic (tests/heap.rs). */
#include <stdint.h>
void *isc_alloc(int64_t bytes);
void isc_free(void *p);
int main(void) {
    void *p = isc_alloc(100);
    isc_free(p);
    isc_free(p);
    return 0;
}
