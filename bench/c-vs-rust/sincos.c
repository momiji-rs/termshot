/* Whether this libm's sincos gives what its sinf and cosf give, for every
   float an arc's angle can be (-pi/2 to 3pi/2, with a margin).

     cc -O2 bench/c-vs-rust/sincos.c -lm -o sincos && ./sincos

   Compilers merge the sinf and cosf of one angle into one sincos call when
   they can: clang on macOS (__sincosf_stret), GCC on Linux (sincosf), and
   LLVM for Rust's f32::sin and f32::cos on both, but not at every call site
   alike. Box drawing's arcs (src/geometry.rs, and draw.c before #12 step
   2a) take the sine and cosine of the same angles, so their pixels don't
   depend on which form a build calls if the two agree on all of these. */
#define _GNU_SOURCE
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

/* Called through volatile pointers, so they stay two calls. */
static float (*volatile sin_f)(float) = sinf;
static float (*volatile cos_f)(float) = cosf;

static void both(float x, float *s, float *c) {
#ifdef __APPLE__
    struct __float2 r = __sincosf_stret(x);
    *s = r.__sinval;
    *c = r.__cosval;
#else
    sincosf(x, s, c);
#endif
}

static uint32_t bits(float x) {
    uint32_t u;
    memcpy(&u, &x, sizeof u);
    return u;
}

static float from(uint32_t u) {
    float x;
    memcpy(&x, &u, sizeof x);
    return x;
}

int main(void) {
    /* [0, 4.75] and [-1.6, -0]: bit patterns increase with magnitude. */
    const uint32_t ranges[2][2] = {{0, bits(4.75f)}, {0x80000000u, bits(-1.6f)}};
    unsigned long long checked = 0, differ = 0;
    for (int r = 0; r < 2; r++) {
        for (uint32_t u = ranges[r][0];; u++) {
            float x = from(u), s, c;
            both(x, &s, &c);
            float s1 = sin_f(x), c1 = cos_f(x);
            if (bits(s) != bits(s1) || bits(c) != bits(c1)) {
                if (differ++ < 10) printf("differ at %a: sincos %a %a, sinf %a cosf %a\n", x, s, c, s1, c1);
            }
            checked++;
            if (u == ranges[r][1]) break;
        }
    }
    printf("%s: %llu floats from -1.6 to 4.75, %llu where sincos differs from sinf and cosf\n",
#ifdef __APPLE__
           "__sincosf_stret",
#else
           "sincosf",
#endif
           checked, differ);
    return differ != 0;
}
