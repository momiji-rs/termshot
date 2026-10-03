/* Production PNG decoder, malformed inputs and allocation bounds under sanitizers. */
#include <assert.h>
#include <stdio.h>
#include "../src/image.c"
#include "../src/deflate.c"
static const unsigned char png[] = {137,80,78,71,13,10,26,10,0,0,0,13,73,72,68,82,0,0,0,2,0,0,0,2,8,6,0,0,0,114,182,13,36,0,0,0,23,73,68,65,84,120,156,99,248,207,192,208,192,240,31,136,25,24,254,55,252,7,50,0,56,232,6,252,229,30,226,71,0,0,0,0,73,69,78,68,174,66,96,130};
// image_inflate reads up to 8 zero bytes past the stream.
static int inflates(const unsigned char *z, int len, unsigned char *out, int olen) {
    unsigned char *padded = calloc((size_t)len + 8, 1);
    assert(padded);
    memcpy(padded, z, (size_t)len);
    int ok = image_inflate(padded, len, out, olen);
    free(padded);
    return ok;
}

static void check_inflate(void) {
    // A stored block of "hello", then the same with a 64 KiB window field or
    // a preset dictionary, which zlib refuses.
    unsigned char hello[] = {0x78, 0x01, 1, 5, 0, 0xfa, 0xff, 'h', 'e', 'l', 'l', 'o', 0x06, 0x2c, 0x02, 0x15};
    unsigned char out[5];
    assert(inflates(hello, sizeof(hello), out, 5) && memcmp(out, "hello", 5) == 0);
    unsigned char header[][2] = {{0x88, 0x1c}, {0x78, 0x20}};
    for (int i = 0; i < 2; i++) {
        unsigned char bad[sizeof(hello)];
        memcpy(bad, hello, sizeof(hello));
        memcpy(bad, header[i], 2);
        assert(!inflates(bad, sizeof(bad), out, 5));
    }
    // Bytes summing to 65520 give an Adler-32 whose low half is 0: the zero
    // padding then matches a trailer cut short, and a zeroed extra output byte
    // leaves it unchanged, so only the bounds can refuse them.
    unsigned char sums[257];
    memset(sums, 255, 256);
    sums[256] = 240;
    int slen;
    unsigned char *sz = termshot_zlib_compress(sums, 257, &slen, 8);
    assert(sz && sz[slen - 1] == 0 && sz[slen - 2] == 0);
    unsigned char *zeroed = calloc(258, 1);
    assert(zeroed);
    assert(inflates(sz, slen, zeroed, 257) && memcmp(zeroed, sums, 257) == 0);
    assert(!inflates(sz, slen - 1, zeroed, 257));
    assert(!inflates(sz, slen - 2, zeroed, 257));
    memset(zeroed, 0, 258);
    assert(!inflates(sz, slen, zeroed, 258));
    free(zeroed);
    free(sz);
    // Streams from the renderer's compressor: noise, runs, and runs with noise,
    // around its block and window sizes.
    static const int sizes[] = {1, 2, 255, 4096, 32768, 70000};
    uint32_t seed = 0x5eedu;
    for (size_t s = 0; s < sizeof(sizes) / sizeof(sizes[0]); s++) {
        int n = sizes[s];
        for (int kind = 0; kind < 3; kind++) {
            unsigned char *raw = malloc((size_t)n), *got = malloc((size_t)n + 1);
            assert(raw && got);
            for (int i = 0; i < n; i++) {
                seed = seed * 1664525u + 1013904223u;
                unsigned char noise = (unsigned char)(seed >> 24), run = (unsigned char)(i / 7 & 3);
                raw[i] = kind == 0 ? noise : kind == 1 ? run : (seed >> 8) % 16 ? run : noise;
            }
            int zlen;
            unsigned char *z = termshot_zlib_compress(raw, n, &zlen, 8);
            assert(z);
            assert(inflates(z, zlen, got, n) && memcmp(got, raw, (size_t)n) == 0);
            // Exactly n bytes: one fewer or one more fails, as kitty's zlib does.
            assert(!inflates(z, zlen, got, n - 1));
            assert(!inflates(z, zlen, got, n + 1));
            // Bytes after the trailer are ignored; any change to it fails.
            unsigned char *longer = malloc((size_t)zlen + 3);
            assert(longer);
            memcpy(longer, z, (size_t)zlen);
            memcpy(longer + zlen, "xyz", 3);
            assert(inflates(longer, zlen + 3, got, n));
            for (int k = 1; k <= 4; k++) {
                longer[zlen - k] ^= 0x40;
                assert(!inflates(longer, zlen, got, n));
                longer[zlen - k] ^= 0x40;
            }
            free(longer);
            // Every truncation fails (only the last bytes for long streams).
            for (int t = zlen > 600 ? zlen - 64 : 0; t < zlen; t++) assert(!inflates(z, t, got, n));
            free(z);
            free(raw);
            free(got);
        }
    }
    // Damaged streams must fail or succeed cleanly; inflating never allocates.
    unsigned char raw[300];
    for (int i = 0; i < 300; i++) raw[i] = (unsigned char)(i * i % 7);
    int zlen;
    unsigned char *z = termshot_zlib_compress(raw, 300, &zlen, 8);
    assert(z);
    unsigned char *damaged = malloc((size_t)zlen), got[300];
    assert(damaged);
    for (int i = 0; i < 2000; i++) {
        memcpy(damaged, z, (size_t)zlen);
        seed = seed * 1664525u + 1013904223u;
        damaged[seed % (uint32_t)zlen] ^= (unsigned char)(1u << ((seed >> 16) % 8));
        inflates(damaged, zlen, got, 300);
        assert(allocated == 0);
    }
    free(damaged);
    free(z);
}

int main(void) {
    check_inflate();
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
    puts("ok, PNG pixels, all truncations, 2000 mutations, decoder quota and allocation cleanup; zlib round trips, exact sizes, trailers, truncations and mutations");
    return 0;
}
