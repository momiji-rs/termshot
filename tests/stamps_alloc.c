/* Fail each allocation of the geometry caches in turn (the arc offsets and
   the Stamps of rounded corners and diagonals): a stroke whose cache can't
   grow is stamped afresh, so every pixel is the same as with no cache, and
   free_geometry leaves nothing allocated. Then shrink the cache's budget,
   STAMP_MAX_BYTES, from its default to nothing: the cache never holds more,
   and the pixels stay the same. */
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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
static size_t budget = (size_t)4 << 20;
#define STAMP_MAX_BYTES budget
#define malloc checked_malloc
#define calloc checked_calloc
#define realloc checked_realloc
#define free checked_free
#include "../src/draw.c"

enum { CELL_W = 22, CELL_H = 48, COLS = 14, ROWS = 6 };
static uint8_t want[COLS * CELL_W * ROWS * CELL_H * 3], got[sizeof want];

/* The seven strokes, plain and bold, over the grid; stamps or NULL. */
static void paint_grid(uint8_t *px, Stamps *stamps) {
    static const uint32_t strokes[] = {0x256D, 0x256E, 0x256F, 0x2570, 0x2571, 0x2572, 0x2573};
    memset(px, 0, sizeof want);
    Canvas cv = {.px = px, .filtered = px, .w = COLS * CELL_W, .h = ROWS * CELL_H,
                 .stride = (size_t)COLS * CELL_W * 3, .stamps = stamps};
    for (int row = 0; row < ROWS; row++)
        for (int col = 0; col < COLS; col++)
            paint_geometry(&cv, col, row, CELL_W, CELL_H, strokes[(row + col) % 7], (row / 3 + col) % 2, 255, 200,
                           100);
    free_geometry(&cv);
}

int main(void) {
    calls = 0, fail_at = 0;
    paint_grid(want, NULL);
    assert(live == 0);
    Stamps stamps = {0};
    calls = 0, fail_at = 0;
    paint_grid(got, &stamps);
    int allocations = calls;
    assert(live == 0 && memcmp(got, want, sizeof want) == 0 && stamps.hits > 0 && stamps.uncached == 0);
    for (int fail = 1; fail <= allocations; fail++) {
        stamps = (Stamps){0};
        calls = 0, fail_at = fail;
        paint_grid(got, &stamps);
        assert(live == 0);
        assert(memcmp(got, want, sizeof want) == 0);
    }
    calls = 0, fail_at = 0;
    size_t full = 0;
    int budgets = 0;
    for (size_t limit = budget; ; limit = limit * 3 / 4) {
        budget = limit;
        stamps = (Stamps){0};
        paint_grid(got, &stamps);
        assert(live == 0 && memcmp(got, want, sizeof want) == 0);
        /* bytes counts what was held when the render ended; peak tracks the
           most at any time. */
        assert(stamps.peak <= limit);
        if (!full) full = stamps.peak;
        budgets++;
        if (!limit) break;
    }
    assert(full > 0);
    printf("%d geometry cache allocation failures and %d budgets (%zu bytes needed) painted the same pixels\n",
           allocations, budgets, full);
    return 0;
}
