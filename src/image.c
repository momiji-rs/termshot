/* In-memory PNG and zlib only. No filesystem access. stb is private to this TU so
   test decoders can coexist. Bound all decoder allocations, including inflate. */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stddef.h>
#define DECODE_BUDGET (64u * 1024u * 1024u)
typedef union { size_t size; max_align_t align; } Allocation;
static _Thread_local size_t allocated;
static void *image_alloc(size_t n) {
    if (n > DECODE_BUDGET - allocated) return NULL;
    Allocation *p = malloc(sizeof(*p) + n);
    if (!p) return NULL;
    p->size = n;
    allocated += n;
    return p + 1;
}
static void image_free(void *ptr) {
    if (!ptr) return;
    Allocation *p = (Allocation *)ptr - 1;
    allocated -= p->size;
    free(p);
}
static void *image_realloc(void *ptr, size_t n) {
    if (!ptr) return image_alloc(n);
    Allocation *p = (Allocation *)ptr - 1;
    size_t old = p->size;
    if (n > DECODE_BUDGET - (allocated - old)) return NULL;
    p = realloc(p, sizeof(*p) + n);
    if (!p) return NULL;
    p->size = n;
    allocated = allocated - old + n;
    return p + 1;
}
#define STBI_MALLOC image_alloc
#define STBI_REALLOC image_realloc
#define STBI_FREE image_free
#define STB_IMAGE_STATIC
#define STB_IMAGE_IMPLEMENTATION
#define STBI_ONLY_PNG
#define STBI_NO_STDIO
#define STBI_MAX_DIMENSIONS 8192
#include "stb_image.h"

/* The caller owns out and limits its size to 16 MiB. */
int image_png_size(const unsigned char *data, int len, int *w, int *h) {
    int channels;
    return stbi_info_from_memory(data, len, w, h, &channels) &&
           *w > 0 && *h > 0 && *w <= 8192 && *h <= 8192 &&
           (int64_t)*w * *h <= 4 * 1024 * 1024;
}
int image_png_decode(const unsigned char *data, int len, unsigned char *out, int w, int h) {
    int x, y, channels;
    unsigned char *p = stbi_load_from_memory(data, len, &x, &y, &channels, 4);
    if (!p) return 0;
    int ok = x == w && y == h;
    if (ok) memcpy(out, p, (size_t)w * h * 4);
    stbi_image_free(p);
    return ok;
}

/* Kitty's o=z: inflate a zlib stream (RFC 1950) to exactly size bytes. The
   output buffer is not expandable, so inflation stops at the bound instead of
   inflating everything and checking afterwards, and nothing is allocated.
   stb neither checks Adler-32 nor reports where the stream ended, so this does
   both: the DEFLATE data must end in the byte before the 4-byte trailer, and
   the trailer must match. Every read stays within data[0, len). */
static uint32_t image_adler32(const unsigned char *p, size_t n) {
    uint32_t a = 1, b = 0;
    while (n) {
        size_t k = n < 5552 ? n : 5552; /* the most bytes before b can overflow */
        n -= k;
        while (k--) {
            a += *p++;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    return b << 16 | a;
}
int image_zlib_decode(const unsigned char *data, int len, unsigned char *out, int size) {
    /* Two header bytes, at least one of DEFLATE, four of trailer. stb checks
       the method, the check bits and the preset dictionary; a window over
       32 KiB (CINFO > 7) is invalid too. */
    if (len < 7 || size <= 0 || (data[0] >> 4) > 7) return 0;
    stbi__zbuf z;
    z.zbuffer = (stbi_uc *)data;
    z.zbuffer_end = (stbi_uc *)data + len;
    if (!stbi__do_zlib(&z, (char *)out, size, 0, 1)) return 0;
    if (z.zout - z.zout_start != size) return 0;
    /* At the end of input stb pads with zero bits it never fetched, which
       would make the count below wrong. A stream that ends before the trailer
       reads at most four bytes ahead, so it never gets there. */
    if (z.hit_zeof_once || z.zbuffer >= z.zbuffer_end || z.num_bits < 0) return 0;
    size_t bits = (size_t)(z.zbuffer - (const stbi_uc *)data) * 8 - (size_t)z.num_bits;
    if ((bits + 7) / 8 != (size_t)len - 4) return 0;
    const unsigned char *t = data + len - 4;
    uint32_t adler = (uint32_t)t[0] << 24 | (uint32_t)t[1] << 16 | (uint32_t)t[2] << 8 | t[3];
    return image_adler32(out, (size_t)size) == adler;
}
