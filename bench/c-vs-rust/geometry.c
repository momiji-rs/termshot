/* draw.c's box drawing and blocks as of the snapshot run.sh takes (the C,
   before #12 step 2a) against src/geometry.rs (the Rust, linked as a static
   library by tests/rust_lib.sh): the same pixels, bit for bit, then the
   time each takes.

     geometry FONT [rounds]

   Pixels: every character U+2500..U+259F, plain and bold, is painted by
   both over a patterned canvas (shades blend over it), with the stroke
   cache and without it, and the canvases are compared byte for byte:
   - at every cell size from 1x1 to 40x100, one cell of each character in a
     row;
   - at the cell size of every --px from 1 to 255 with FONT, in every cell
     of the widest row and the tallest column the CLI allows (500 columns,
     200 rows) that fit in its 2^27 pixels, the characters cycling, so the
     strokes cover every binade a screen's coordinates do;
   and the caches' hit, miss and uncached counts must agree.

   Time: the C and the Rust painting the same screens of geometry, in
   alternating rounds, median per screen. TIME_ONLY=1 skips the pixels. */
#include "draw.c"

/* The Rust side, as src/draw.c now declares it; Canvas there is RustCanvas
   here, since the snapshot has its own. */
typedef struct {
    uint8_t *px, *filtered;
    int32_t w, h;
    size_t stride;
    void *geometry;
} RustCanvas;
void *termshot_geometry_new(int reuse_strokes);
void termshot_geometry_free(void *geometry);
int termshot_paint_geometry(const RustCanvas *cv, int col, int row, int cell_w, int cell_h, uint32_t cp, int bold,
                            uint8_t r, uint8_t g, uint8_t b);
void termshot_fill_rect(const RustCanvas *cv, int x0, int y0, int x1, int y1, uint8_t r, uint8_t g, uint8_t b);
typedef struct {
    size_t hits, misses, uncached, bytes, peak;
} GeometryStats;
void termshot_geometry_stats(const void *geometry, GeometryStats *out);

static uint32_t seed = 0x9e3779b9u;
static uint32_t next(void) {
    seed ^= seed << 13;
    seed ^= seed >> 17;
    seed ^= seed << 5;
    return seed;
}

/* A cell of a screen: its character, weight and colour. */
typedef struct {
    uint32_t cp;
    int bold;
    uint8_t r, g, b;
} Stroke;

static long long compared, cells_painted;
static int failures;

static void pattern(uint8_t *px, size_t n) {
    for (size_t i = 0; i < n; i++) px[i] = (uint8_t)(i * 37 + (i >> 7));
}

/* Paint strokes (one per cell, row-major, cols x rows of w x h) with both,
   cached or not, and compare. */
static void compare(const Stroke *strokes, int cols, int rows, int w, int h, int cached) {
    int width = cols * w, height = rows * h;
    size_t stride = (size_t)width * 3, n = stride * (size_t)height;
    uint8_t *a = malloc(n), *b = malloc(n);
    if (!a || !b) {
        fprintf(stderr, "out of memory\n");
        exit(2);
    }
    pattern(a, n);
    memcpy(b, a, n);
    Stamps stamps = {0};
    Canvas cv = {.px = a, .filtered = a, .w = width, .h = height, .stride = stride, .stamps = cached ? &stamps : NULL};
    RustCanvas rv = {.px = b, .filtered = b, .w = width, .h = height, .stride = stride,
                     .geometry = termshot_geometry_new(cached)};
    for (int row = 0; row < rows; row++) {
        for (int col = 0; col < cols; col++) {
            const Stroke *s = &strokes[row * cols + col];
            int c = paint_geometry(&cv, col, row, w, h, s->cp, s->bold, s->r, s->g, s->b);
            int r = termshot_paint_geometry(&rv, col, row, w, h, s->cp, s->bold, s->r, s->g, s->b);
            if (c != r) {
                fprintf(stderr, "FAIL U+%04X at %d,%d of %dx%d: C says %d, Rust %d\n", (unsigned)s->cp, col, row, w, h,
                        c, r);
                failures++;
            }
        }
    }
    cells_painted += (long long)cols * rows;
    compared += (long long)n;
    if (memcmp(a, b, n) != 0) {
        size_t i = 0;
        while (a[i] == b[i]) i++;
        size_t y = i / stride, x = i % stride / 3;
        const Stroke *s = &strokes[(y / h) * cols + x / w];
        fprintf(stderr, "FAIL %dx%d cells, %dx%d, %s: pixel (%zu, %zu) differs, in U+%04X%s\n", w, h, cols, rows,
                cached ? "cached" : "uncached", x, y, (unsigned)s->cp, s->bold ? " bold" : "");
        failures++;
    }
    GeometryStats rs;
    termshot_geometry_stats(rv.geometry, &rs);
    if (cached && (rs.hits != stamps.hits || rs.misses != stamps.misses || rs.uncached != stamps.uncached)) {
        fprintf(stderr, "FAIL %dx%d cells, %dx%d: cache counts C %zu/%zu/%zu, Rust %zu/%zu/%zu\n", w, h, cols, rows,
                stamps.hits, stamps.misses, stamps.uncached, rs.hits, rs.misses, rs.uncached);
        failures++;
    }
    free_geometry(&cv);
    termshot_geometry_free(rv.geometry);
    free(a);
    free(b);
}

static Stroke stroke(uint32_t cp) {
    uint32_t v = next();
    return (Stroke){cp, (int)(v >> 31), (uint8_t)v, (uint8_t)(v >> 8), (uint8_t)(v >> 16)};
}

/* Every character, then cycling, over cols x rows. */
static Stroke *cycle(int cols, int rows, int phase) {
    Stroke *s = malloc(sizeof *s * (size_t)cols * (size_t)rows);
    if (!s) exit(2);
    for (int i = 0; i < cols * rows; i++) s[i] = stroke(0x2500 + (uint32_t)((i * 7 + phase) % 160));
    return s;
}

/* fill_rect, clipped to the canvas, on random rectangles in and around it. */
static void compare_fill_rect(void) {
    enum { W = 37, H = 23 };
    static uint8_t a[W * H * 3 + 1], b[W * H * 3 + 1];
    pattern(a, sizeof a);
    memcpy(b, a, sizeof a);
    Canvas cv = {.px = a + 1, .filtered = a, .w = W, .h = H, .stride = W * 3};
    RustCanvas rv = {.px = b + 1, .filtered = b, .w = W, .h = H, .stride = W * 3};
    for (int k = 0; k < 20000; k++) {
        int x0 = (int)(next() % 61) - 12, y0 = (int)(next() % 47) - 12, x1 = (int)(next() % 61) - 12,
            y1 = (int)(next() % 47) - 12;
        uint32_t c = next();
        fill_rect(&cv, x0, y0, x1, y1, (uint8_t)c, (uint8_t)(c >> 8), (uint8_t)(c >> 16));
        termshot_fill_rect(&rv, x0, y0, x1, y1, (uint8_t)c, (uint8_t)(c >> 8), (uint8_t)(c >> 16));
    }
    if (memcmp(a, b, sizeof a) != 0) {
        fprintf(stderr, "FAIL fill_rect\n");
        failures++;
    }
}

static double now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1e3 + (double)ts.tv_nsec / 1e6;
}

static int by_value(const void *a, const void *b) {
    double x = *(const double *)a, y = *(const double *)b;
    return x < y ? -1 : x > y;
}

/* The median ms each side takes to paint strokes over a screen, cached as
   draw_png does. */
static void time_screen(const char *name, const Stroke *strokes, int cols, int rows, int w, int h, int rounds) {
    int width = cols * w, height = rows * h;
    size_t stride = (size_t)width * 3 + 1, n = stride * (size_t)height;
    uint8_t *px = calloc(n, 1);
    double *t[2] = {malloc(sizeof(double) * (size_t)rounds), malloc(sizeof(double) * (size_t)rounds)};
    if (!px || !t[0] || !t[1]) exit(2);
    for (int round = 0; round < rounds; round++) {
        for (int k = 0; k < 2; k++) {
            int side = (round + k) % 2; /* alternate which goes first */
            double start = now();
            if (side == 0) {
                Stamps stamps = {0};
                Canvas cv = {.px = px + 1, .filtered = px, .w = width, .h = height, .stride = stride, .stamps = &stamps};
                for (int i = 0; i < cols * rows; i++) {
                    const Stroke *s = &strokes[i];
                    paint_geometry(&cv, i % cols, i / cols, w, h, s->cp, s->bold, s->r, s->g, s->b);
                }
                free_geometry(&cv);
            } else {
                RustCanvas rv = {.px = px + 1, .filtered = px, .w = width, .h = height, .stride = stride,
                                 .geometry = termshot_geometry_new(1)};
                for (int i = 0; i < cols * rows; i++) {
                    const Stroke *s = &strokes[i];
                    termshot_paint_geometry(&rv, i % cols, i / cols, w, h, s->cp, s->bold, s->r, s->g, s->b);
                }
                termshot_geometry_free(rv.geometry);
            }
            t[side][round] = now() - start;
        }
    }
    qsort(t[0], (size_t)rounds, sizeof(double), by_value);
    qsort(t[1], (size_t)rounds, sizeof(double), by_value);
    double c = t[0][rounds / 2], r = t[1][rounds / 2];
    printf("%-22s %5dx%-4d %3dx%-3d  C %9.3f ms  Rust %9.3f ms  Rust/C %.3f\n", name, cols, rows, w, h, c, r, r / c);
    free(px);
    free(t[0]);
    free(t[1]);
}

/* The pixel comparison; 0 when every canvas is the same. */
static int check_pixels(const unsigned char *font) {
    compare_fill_rect();
    /* Every small cell size, each character once. */
    for (int w = 1; w <= 40; w++) {
        for (int h = 1; h <= 100; h++) {
            Stroke *s = cycle(160, 1, 0);
            for (int i = 0; i < 160; i++) s[i].cp = 0x2500 + (uint32_t)i;
            compare(s, 160, 1, w, h, 1);
            compare(s, 160, 1, w, h, 0);
            free(s);
        }
    }
    printf("small sizes: %d failures so far\n", failures);
    fflush(stdout);
    /* The CLI's cell sizes, across the largest screens. */
    int sizes[256][2], nsizes = 0;
    for (int px = 1; px <= 255; px++) {
        int w, h;
        if (!draw_cell_size(font, 0, px, &w, &h)) return 1;
        if (nsizes && sizes[nsizes - 1][0] == w && sizes[nsizes - 1][1] == h) continue;
        sizes[nsizes][0] = w;
        sizes[nsizes][1] = h;
        nsizes++;
    }
    for (int k = 0; k < nsizes; k++) {
        int w = sizes[k][0], h = sizes[k][1];
        int cols = 500, rows = 200;
        while ((long long)cols * w * h > MAX_PIXELS) cols--;
        while ((long long)rows * w * h > MAX_PIXELS) rows--;
        for (int cached = 0; cached < 2; cached++) {
            Stroke *row = cycle(cols, 1, k), *column = cycle(1, rows, k + 1);
            compare(row, cols, 1, w, h, cached);
            compare(column, 1, rows, w, h, cached);
            free(row);
            free(column);
        }
    }
    printf("%d CLI cell sizes (px 1..255)\n", nsizes);
    if (failures) {
        printf("FAIL: %d differences\n", failures);
        return 1;
    }
    printf("ok, %lld cells painted alike by the C and the Rust (%lld bytes compared), cached and not\n", cells_painted,
           compared);
    fflush(stdout);
    return 0;
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: geometry FONT [rounds]\n");
        return 2;
    }
    int rounds = argc > 2 ? atoi(argv[2]) : 31;
    if (rounds < 1) rounds = 1;
    FILE *fp = fopen(argv[1], "rb");
    if (!fp) return 1;
    fseek(fp, 0, SEEK_END);
    long len = ftell(fp);
    fseek(fp, 0, SEEK_SET);
    unsigned char *font = malloc((size_t)len);
    if (!font || fread(font, 1, (size_t)len, fp) != (size_t)len) return 1;
    fclose(fp);

    /* TIME_ONLY=1 skips to the timing. */
    if (!getenv("TIME_ONLY") && check_pixels(font)) return 1;

    /* Time: screens of bench.py's draw suite, painted by each. */
    int w48, h48, w128, h128;
    draw_cell_size(font, 0, 48, &w48, &h48);
    draw_cell_size(font, 0, 128, &w128, &h128);
    Stroke *all = malloc(sizeof *all * 240 * 80), *rounded = malloc(sizeof *rounded * 100 * 30),
           *boxes = malloc(sizeof *boxes * 100 * 30), *blocks = malloc(sizeof *blocks * 100 * 30);
    if (!all || !rounded || !boxes || !blocks) return 2;
    static const char *box_chars = "\x00\x02\x0c\x10\x14\x18\x1c\x24\x2c\x34\x3c\x50\x51\x6c";
    for (int r = 0; r < 80; r++)
        for (int c = 0; c < 240; c++) all[r * 240 + c] = (Stroke){0x2500 + (uint32_t)((r * 240 + c) % 160), (r + c) % 2, 219, 231, 247};
    for (int i = 0; i < 100 * 30; i++) {
        rounded[i] = (Stroke){0x256D + (uint32_t)(i % 4 == 2 ? 3 : i % 4 == 3 ? 2 : i % 4), 0, 219, 231, 247};
        boxes[i] = (Stroke){0x2500 + (uint32_t)(unsigned char)box_chars[i % 14], 0, 219, 231, 247};
        blocks[i] = (Stroke){0x2580 + (uint32_t)((i * 7) % 32), 0, (uint8_t)next(), (uint8_t)next(), 200};
    }
    printf("== time per screen, median of %d alternating rounds\n", rounds);
    time_screen("geometry-all", all, 240, 80, w48, h48, rounds);
    time_screen("rounded-boxes", rounded, 100, 30, w48, h48, rounds);
    time_screen("rounded-128px", rounded, 100, 30, w128, h128, rounds);
    time_screen("box-grid", boxes, 100, 30, w48, h48, rounds);
    time_screen("block-grid", blocks, 100, 30, w48, h48, rounds);
    free(all);
    free(rounded);
    free(boxes);
    free(blocks);
    free(font);
    return 0;
}
