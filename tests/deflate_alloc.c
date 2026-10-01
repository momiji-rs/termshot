/* Fail each compressor allocation in turn: failure must terminate and free
   all earlier allocations, including an output buffer with pending bits. */
#include <assert.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>

static int calls, fail_at, live;
static void *checked_malloc(size_t n) {
    if (++calls == fail_at) return NULL;
    void *p = malloc(n);
    if (p) live++;
    return p;
}
static void *checked_calloc(size_t n, size_t size) {
    if (++calls == fail_at) return NULL;
    void *p = calloc(n, size);
    if (p) live++;
    return p;
}
static void *checked_realloc(void *old, size_t n) {
    if (++calls == fail_at) return NULL;
    int was_null = old == NULL;
    void *p = realloc(old, n);
    if (p && was_null) live++;
    return p;
}
static void checked_free(void *p) {
    if (p) live--;
    free(p);
}
#define malloc checked_malloc
#define calloc checked_calloc
#define realloc checked_realloc
#define free checked_free
#include "../src/deflate.c"

int main(void) {
    static unsigned char data[200000];
    uint32_t rng = 42;
    for (size_t i = 0; i < sizeof(data); i++) {
        rng ^= rng << 13; rng ^= rng >> 17; rng ^= rng << 5;
        data[i] = (unsigned char)rng;
    }
    const int lengths[] = {0, 1000, sizeof(data)};
    int failures = 0;
    for (size_t i = 0; i < sizeof(lengths) / sizeof(lengths[0]); i++) {
        calls = 0; fail_at = 0;
        int n;
        unsigned char *p = termshot_zlib_compress(data, lengths[i], &n, 8);
        assert(p && n > 0);
        int allocations = calls;
        free(p);
        assert(live == 0);
        for (int fail = 1; fail <= allocations; fail++) {
            calls = 0; fail_at = fail;
            p = termshot_zlib_compress(data, lengths[i], &n, 8);
            assert(p == NULL && live == 0);
            failures++;
        }
    }
    printf("%d compressor allocation failures handled\n", failures);
    return 0;
}
