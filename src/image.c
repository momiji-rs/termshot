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

/* Inflate a zlib stream (kitty's o=z) into exactly olen bytes, checking what
   zlib checks and stb does not: the window field and the Adler-32 trailer.
   Bytes after the trailer are ignored, as zlib ignores them. The caller pads
   the input with 8 zero bytes after len, so stb's read-ahead stays inside the
   buffer and the stream's end can be found from the bits it has buffered. A
   stream that reads into the padding ends past len - 4 and fails. */
int image_inflate(const unsigned char *in, int len, unsigned char *out, int olen) {
    if (len < 2 || in[0] >> 4 > 7) return 0;
    stbi__zbuf a;
    a.zbuffer = (stbi_uc *)in;
    a.zbuffer_end = (stbi_uc *)in + len + 8;
    if (!stbi__do_zlib(&a, (char *)out, olen, 0, 1)) return 0;
    if (a.zout - a.zout_start != olen) return 0;
    /* The trailer starts at the first whole byte stb has not consumed. */
    const unsigned char *end = a.zbuffer - a.num_bits / 8;
    if (end + 4 > in + len) return 0;
    uint32_t s1 = 1, s2 = 0;
    for (int i = 0; i < olen;) {
        int n = olen - i < 5552 ? olen - i : 5552;
        for (int j = 0; j < n; j++) {
            s1 += out[i + j];
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
        i += n;
    }
    uint32_t want = (uint32_t)end[0] << 24 | (uint32_t)end[1] << 16 | (uint32_t)end[2] << 8 | end[3];
    return want == (s2 << 16 | s1);
}
