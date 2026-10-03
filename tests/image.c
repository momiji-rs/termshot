/* Production PNG decoder, malformed inputs and allocation bounds under sanitizers. */
#include <assert.h>
#include <stdio.h>
#include "../src/image.c"
static const unsigned char png[] = {137,80,78,71,13,10,26,10,0,0,0,13,73,72,68,82,0,0,0,2,0,0,0,2,8,6,0,0,0,114,182,13,36,0,0,0,23,73,68,65,84,120,156,99,248,207,192,208,192,240,31,136,25,24,254,55,252,7,50,0,56,232,6,252,229,30,226,71,0,0,0,0,73,69,78,68,174,66,96,130};
int main(void) {
    int w, h;
    unsigned char out[16];
    assert(image_png_size(png, sizeof(png), &w, &h) && w == 2 && h == 2);
    assert(image_png_decode(png, sizeof(png), out, w, h));
    const unsigned char expected[] = {255,0,0,128,0,255,0,128,0,0,255,128,255,255,0,128};
    assert(memcmp(out, expected, sizeof(out)) == 0);
    assert(allocated == 0);
    // Every truncation and 2,000 deterministic mutations must release all memory.
    for (size_t n = 0; n < sizeof(png); n++) {
        image_png_decode(png, (int)n, out, 2, 2);
        assert(allocated == 0);
    }
    uint32_t seed = 0x7139u;
    for (int i = 0; i < 2000; i++) {
        unsigned char damaged[sizeof(png)];
        memcpy(damaged, png, sizeof(png));
        seed = seed * 1664525u + 1013904223u;
        damaged[seed % sizeof(png)] ^= (unsigned char)(1u << ((seed >> 16) % 8));
        if (image_png_size(damaged, sizeof(damaged), &w, &h)) {
            unsigned char *pixels = malloc((size_t)w * h * 4);
            assert(pixels);
            image_png_decode(damaged, sizeof(damaged), pixels, w, h);
            free(pixels);
        }
        assert(allocated == 0);
    }
    void *p = image_alloc(16);
    assert(p && allocated == 16);
    assert(!image_alloc(DECODE_BUDGET));
    assert(!image_realloc(p, DECODE_BUDGET + 1));
    assert(allocated == 16);
    p = image_realloc(p, 32);
    assert(p && allocated == 32);
    image_free(p);
    // Force production decoding to fail its allocation quota, then recover.
    p = image_alloc(DECODE_BUDGET);
    assert(p);
    assert(!image_png_decode(png, sizeof(png), out, 2, 2));
    assert(allocated == DECODE_BUDGET);
    image_free(p);
    assert(image_png_decode(png, sizeof(png), out, 2, 2));
    assert(allocated == 0);
    puts("ok, PNG pixels, all truncations, 2000 mutations, decoder quota and allocation cleanup");
    return 0;
}
