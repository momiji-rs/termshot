/* In-memory PNG only. No filesystem access. stb is private to this TU so
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
