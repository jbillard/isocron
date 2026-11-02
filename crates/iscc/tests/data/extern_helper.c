#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
double sum_f64(const double *x, int64_t n) { double s = 0; for (int64_t i = 0; i < n; i++) s += x[i]; return s; }
float sum_f32(const float *x, int64_t n) { float s = 0; for (int64_t i = 0; i < n; i++) s += x[i]; return s; }
int64_t sum_i32(const int32_t *x, int64_t n) { int64_t s = 0; for (int64_t i = 0; i < n; i++) s += x[i]; return s; }
int64_t count_true(const bool *x, int64_t n) { int64_t c = 0; for (int64_t i = 0; i < n; i++) c += x[i]; return c; }
int8_t neg8(int8_t v) { return (int8_t)-v; }
uint16_t twice16(uint16_t v) { return (uint16_t)(v * 2); }
bool is_even(int64_t v) { return v % 2 == 0; }
double mix(int64_t a, double b, int32_t c, float d, bool e) { return (double)a + b * c + d + (e ? 100.0 : 0.0); }
