/* Production PNG and zlib decoders, malformed inputs and allocation bounds under sanitizers. */
#include <assert.h>
#include <stdio.h>
#include "../src/image.c"
static const unsigned char png[] = {137,80,78,71,13,10,26,10,0,0,0,13,73,72,68,82,0,0,0,2,0,0,0,2,8,6,0,0,0,114,182,13,36,0,0,0,23,73,68,65,84,120,156,99,248,207,192,208,192,240,31,136,25,24,254,55,252,7,50,0,56,232,6,252,229,30,226,71,0,0,0,0,73,69,78,68,174,66,96,130};
/* kitty's o=z: the four pixels above as RGB, compressed by src/deflate.c. */
static const unsigned char zrgb[] = {0x78,0x5e,0xfb,0xcf,0xc0,0xc0,0x00,0xc6,0xff,0xff,0x33,0x00,0x00,0x1c,0xef,0x04,0xfc};
static const unsigned char rgb[] = {255,0,0, 0,255,0, 0,0,255, 255,255,0};
/* Inflate into a heap buffer of exactly size bytes, so ASan sees any overrun. */
static int inflate_exact(const unsigned char *z, size_t len, int size, const unsigned char *want) {
    unsigned char *out = malloc((size_t)size);
    assert(out);
    int ok = image_zlib_decode(z, (int)len, out, size);
    if (ok && want) assert(memcmp(out, want, (size_t)size) == 0);
    free(out);
    assert(allocated == 0);
    return ok;
}
/* A fixed-Huffman stream: one zero literal, then copies of 258 at distance 1. */
typedef struct { unsigned char *p; size_t n; unsigned bits, count; } Bits;
static void put(Bits *b, unsigned value, int n) {
    for (int i = 0; i < n; i++) {
        b->bits |= ((value >> i) & 1u) << b->count;
        if (++b->count == 8) { b->p[b->n++] = (unsigned char)b->bits; b->bits = b->count = 0; }
    }
}
static void code(Bits *b, unsigned value, int n) { /* Huffman codes go most significant bit first */
    for (int i = n - 1; i >= 0; i--) put(b, value >> i & 1u, 1);
}
static size_t bomb(unsigned char *p, size_t copies) {
    Bits b = {p, 0, 0, 0};
    p[b.n++] = 0x78;
    p[b.n++] = 0x01;
    put(&b, 1, 1);           /* final */
    put(&b, 1, 2);           /* fixed codes */
    code(&b, 0x30, 8);       /* literal 0 */
    for (size_t i = 0; i < copies; i++) {
        code(&b, 0xc5, 8);   /* length 258 (code 285) */
        code(&b, 0, 5);      /* distance 1 */
    }
    code(&b, 0, 7);          /* end of block */
    if (b.count) put(&b, 0, 8 - b.count);
    uint32_t n = (uint32_t)(1 + 258 * copies);
    uint32_t adler = (n % 65521) << 16 | 1; /* all zeros: a stays 1, b sums it n times */
    for (int i = 3; i >= 0; i--) p[b.n++] = (unsigned char)(adler >> (8 * i));
    return b.n;
}
static void zlib_checks(void) {
    assert(inflate_exact(zrgb, sizeof(zrgb), 12, rgb));
    /* The size must be exact: one byte less stops at the bound, one more is short. */
    assert(!inflate_exact(zrgb, sizeof(zrgb), 11, NULL));
    assert(!inflate_exact(zrgb, sizeof(zrgb), 13, NULL));
    /* Every truncation, and bytes after the trailer. */
    for (size_t n = 0; n < sizeof(zrgb); n++) assert(!inflate_exact(zrgb, n, 12, NULL));
    unsigned char longer[sizeof(zrgb) + 4];
    memcpy(longer, zrgb, sizeof(zrgb));
    for (size_t extra = 1; extra <= 4; extra++) {
        memcpy(longer + sizeof(zrgb), zrgb + sizeof(zrgb) - 4, 4); /* even a second trailer */
        assert(!inflate_exact(longer, sizeof(zrgb) + extra, 12, NULL));
        memset(longer + sizeof(zrgb), 0, 4);
        assert(!inflate_exact(longer, sizeof(zrgb) + extra, 12, NULL));
    }
    /* RFC 1950 allows windows up to 32 KiB (CINFO 7); 8 is invalid even
       with valid check bits. */
    unsigned char window[sizeof(zrgb)];
    memcpy(window, zrgb, sizeof(zrgb));
    window[0] = 0x88;
    window[1] = (unsigned char)(31 - 0x8800 % 31);
    assert(!inflate_exact(window, sizeof(window), 12, NULL));
    window[0] = 0x78;
    window[1] = (unsigned char)(31 - 0x7800 % 31);
    assert(inflate_exact(window, sizeof(window), 12, rgb));
    /* A stream one byte short of the size, into a zeroed buffer. Its bytes
       sum to 65,520, so Adler-32 is the same with a zero byte appended:
       only the length tells them apart. */
    unsigned char shorter[2 + 5 + 257 + 4] = {0x78, 0x01, 0x01, 0x01, 0x01, 0xfe, 0xfe};
    memset(shorter + 7, 255, 256);
    shorter[7 + 256] = 240;
    uint32_t sum = image_adler32(shorter + 7, 257);
    for (int i = 0; i < 4; i++) shorter[7 + 257 + i] = (unsigned char)(sum >> (24 - 8 * i));
    unsigned char *zeroed = calloc(258, 1);
    assert(zeroed);
    assert(image_zlib_decode(shorter, sizeof(shorter), zeroed, 257));
    memcpy(zeroed, shorter + 7, 257);
    assert(image_adler32(zeroed, 258) == sum);
    memset(zeroed, 0, 258);
    assert(!image_zlib_decode(shorter, sizeof(shorter), zeroed, 258));
    free(zeroed);
    /* A single-bit flip is refused, or lands in the padding after the last
       code, which no inflater reads; and 2,000 multi-byte mutations at any
       output size never read or write out of bounds. */
    int accepted = 0;
    for (size_t i = 0; i < sizeof(zrgb) * 8; i++) {
        unsigned char damaged[sizeof(zrgb)];
        memcpy(damaged, zrgb, sizeof(zrgb));
        damaged[i / 8] ^= (unsigned char)(1u << (i % 8));
        accepted += inflate_exact(damaged, sizeof(damaged), 12, rgb);
    }
    assert(accepted < 8);
    uint32_t seed = 0x2b0du;
    for (int i = 0; i < 2000; i++) {
        unsigned char damaged[sizeof(zrgb)];
        memcpy(damaged, zrgb, sizeof(zrgb));
        for (int k = 0; k < 3; k++) {
            seed = seed * 1664525u + 1013904223u;
            damaged[seed % sizeof(zrgb)] = (unsigned char)(seed >> 24);
        }
        inflate_exact(damaged, sizeof(damaged), 1 + (int)(seed >> 8) % 24, NULL);
    }
    /* A 104 KiB stream of 16,908,289 zeros: inflation stops at the bound,
       whatever it is, and only the exact size succeeds. */
    unsigned char *z = malloc(128 * 1024);
    assert(z);
    size_t len = bomb(z, 65536);
    assert(len < 128 * 1024);
    int full = 1 + 258 * 65536;
    assert(!inflate_exact(z, len, 12, NULL));
    assert(!inflate_exact(z, len, 16 * 1024 * 1024, NULL));
    assert(!inflate_exact(z, len, full + 1, NULL));
    unsigned char *zeros = calloc((size_t)full, 1);
    assert(zeros);
    assert(inflate_exact(z, len, full, zeros));
    free(zeros);
    free(z);
    /* Inflation allocates nothing, so it works with the quota spent. */
    void *p = image_alloc(DECODE_BUDGET);
    assert(p);
    unsigned char out[12];
    assert(image_zlib_decode(zrgb, sizeof(zrgb), out, 12) && memcmp(out, rgb, 12) == 0);
    assert(allocated == DECODE_BUDGET);
    image_free(p);
    assert(allocated == 0);
    puts("ok, zlib pixels, exact sizes, all truncations, trailing bytes, all bit flips, 2000 mutations, bounded inflation without allocation");
}
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
    zlib_checks();
    return 0;
}
