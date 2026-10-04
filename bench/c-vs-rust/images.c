/* draw.c's image layers and backdrop as of the snapshot run.sh takes (the C,
   before #12 step 2b) against src/composite.rs (the Rust, linked as a static
   library by tests/rust_lib.sh): the same pixels, bit for bit, then the
   time each takes.

     images FONT [rounds]

   Pixels: random scenes, each painted by both over the same patterned
   canvas (a fresh raster is malloc's garbage, and the backdrop must cover
   it) and compared byte for byte, filter bytes included:
   - cells of random sizes, colours and ATTR_OPAQUE, and up to 80 views of a
     pool of random RGBA images, alpha 0, 255 or between: random crops, sizes
     (stretched, shrunk, 1:1), positions on and off the canvas, z-indexes at
     and around each layer's edges, row slices (clip_top/bottom) and column
     runs (clip_left/right) as scrolling and Unicode placeholders make them,
     empty and reversed clips, and solid 1x1 views as the cursor marks are;
   - the backdrop painted a row at a time and all at once, through the
     calls draw_png makes (one per row of cells, and one per glyph reaching
     down a random distance), then the images over the text;
   - whether a scene's backdrop goes by rows (16 MiB, 64 images) must agree;
   - then a few scenes at the CLI's sizes, past 16 MiB, so by rows.

   Time: the C and the Rust painting bench.py's image screens, in
   alternating rounds, median per screen. TIME_ONLY=1 skips the pixels. */
#include "draw.c"

/* The Rust side, as src/draw.c now declares it. Canvas, Cell and ImageView
   are the snapshot's, which have the same layout; Backdrop is RustBackdrop
   here, since the snapshot has its own. */
typedef struct {
    const Cell *cells;
    const ImageView *images;
    size_t image_count;
    int32_t cols, rows, cell_w, cell_h;
    int32_t done, whole;
    double ms;
} RustBackdrop;
void termshot_backdrop_init(RustBackdrop *bd, const Canvas *cv, const Cell *cells, int cols, int rows, int cell_w,
                            int cell_h, const ImageView *images, size_t image_count, size_t row_bytes);
int termshot_backdrop_through(const Canvas *cv, RustBackdrop *bd, int64_t y);
int termshot_paint_images(const Canvas *cv, const ImageView *images, size_t count, int layer);

static uint64_t seed = 0x9e3779b97f4a7c15ull;
static uint32_t next(uint32_t n) {
    seed ^= seed << 13;
    seed ^= seed >> 7;
    seed ^= seed << 17;
    return n ? (uint32_t)(seed % n) : 0;
}
static int64_t between(int64_t lo, int64_t hi) { return lo + (int64_t)next((uint32_t)(hi - lo + 1)); }

static void pattern(uint8_t *px, size_t n, uint32_t salt) {
    for (size_t i = 0; i < n; i++) px[i] = (uint8_t)(i * 37 + (i >> 9) + salt);
}

/* An RGBA image of random pixels; alpha is 255, 0 or between. */
typedef struct {
    unsigned char *px;
    uint32_t width, height;
} Picture;

static Picture picture(uint32_t width, uint32_t height) {
    Picture p = {malloc((size_t)width * height * 4), width, height};
    if (!p.px) exit(2);
    int kind = (int)next(4); /* opaque, mixed, translucent, sparse */
    for (size_t i = 0; i < (size_t)width * height; i++) {
        for (int c = 0; c < 3; c++) p.px[i * 4 + c] = (unsigned char)next(256);
        uint32_t r = next(8);
        unsigned a = kind == 0 ? 255 : kind == 2 ? next(256) : kind == 3 ? (r < 6 ? 0 : 255) : r < 3 ? 255 : r < 5 ? 0 : next(256);
        p.px[i * 4 + 3] = (unsigned char)a;
    }
    return p;
}

static const int32_t zs[] = {INT32_MIN, INT32_MIN / 2 - 1, INT32_MIN / 2, -1073741825, -5, -1, 0, 1, 3, INT32_MAX};
static unsigned char solid_pixels[4][4] = {{219, 231, 247, 255}, {205, 0, 0, 255}, {0, 0, 0, 255}, {17, 24, 35, 255}};

/* A view of one of the pictures over a W x H canvas of cw x ch cells. */
static ImageView random_view(const Picture *pool, int npool, int64_t W, int64_t H, int cw, int ch) {
    if (next(10) == 0) {
        /* A solid rectangle, as the underline and bar cursors are. */
        int64_t x = between(-2, W), y = between(-2, H), w = between(1, cw + 1), h = between(1, ch + 1);
        return (ImageView){.pixels = solid_pixels[next(4)], .width = 1, .height = 1, .x = x, .y = y, .w = w, .h = h,
                           .clip_top = y, .clip_bottom = y + h, .clip_left = x, .clip_right = x + w,
                           .src_w = 1, .src_h = 1, .z = INT32_MAX};
    }
    const Picture *p = &pool[next((uint32_t)npool)];
    ImageView v = {.pixels = p->px, .width = p->width, .height = p->height};
    v.src_x = next(p->width);
    v.src_y = next(p->height);
    v.src_w = 1 + next(p->width - v.src_x);
    v.src_h = 1 + next(p->height - v.src_y);
    switch (next(4)) {
    case 0: /* native size */
        v.w = v.src_w;
        v.h = v.src_h;
        break;
    case 1: /* whole cells */
        v.w = between(1, 12) * cw;
        v.h = between(1, 8) * ch;
        break;
    default:
        v.w = between(1, W + W / 2 + 2);
        v.h = between(1, H + H / 2 + 2);
    }
    v.x = between(-v.w, W + 2);
    v.y = between(-v.h, H + 2);
    v.clip_top = INT64_MIN;
    v.clip_bottom = INT64_MAX;
    v.clip_left = INT64_MIN;
    v.clip_right = INT64_MAX;
    if (next(3) == 0) { /* a slice, as scrolling leaves one */
        v.clip_top = v.y + between(-3, v.h);
        v.clip_bottom = next(8) == 0 ? v.clip_top - between(0, 3) : v.clip_top + between(0, v.h + 3);
    }
    if (next(3) == 0) { /* a placeholder run's columns */
        v.clip_left = v.x + between(-3, v.w);
        v.clip_right = next(8) == 0 ? v.clip_left - between(0, 3) : v.clip_left + between(0, v.w + 3);
    }
    v.z = zs[next(sizeof zs / sizeof zs[0])];
    return v;
}

static long long compared, scenes, pixels_compared;
static int failures;

/* Paint the scene with both and compare. whole: -1 as each decides, else
   forced. Returns nonzero on a difference. */
static int compare_scene(const Cell *cells, int cols, int rows, int cw, int ch, const ImageView *views, size_t n,
                         int whole, const char *what) {
    int64_t W = (int64_t)cols * cw, H = (int64_t)rows * ch;
    size_t stride = (size_t)W * 3 + 1, bytes = stride * (size_t)H;
    uint8_t *a = malloc(bytes), *b = malloc(bytes);
    if (!a || !b) {
        fprintf(stderr, "out of memory\n");
        exit(2);
    }
    pattern(a, bytes, (uint32_t)scenes);
    memcpy(b, a, bytes);
    Canvas ca = {.px = a + 1, .filtered = a, .w = (int)W, .h = (int)H, .stride = stride};
    Canvas cb = {.px = b + 1, .filtered = b, .w = (int)W, .h = (int)H, .stride = stride};
    Backdrop bc = backdrop_for(&ca, cells, cols, rows, cw, ch, views, n);
    RustBackdrop br;
    termshot_backdrop_init(&br, &cb, cells, cols, rows, cw, ch, views, n, BACKDROP_ROW_BYTES);
    int bad = 0;
    if (bc.whole != br.whole) {
        fprintf(stderr, "FAIL %s: C paints the backdrop %s, Rust %s\n", what, bc.whole ? "whole" : "by rows",
                br.whole ? "whole" : "by rows");
        bad = 1;
    }
    if (whole >= 0) bc.whole = br.whole = whole;
    /* draw_png's calls: each row of cells before its text, and glyphs
       reaching down into the rows below. */
    for (int r = 0; r < rows; r++) {
        int64_t y = (int64_t)(r + 1) * ch;
        backdrop_through(&ca, &bc, y);
        if (termshot_backdrop_through(&cb, &br, y) < 0) bad = 1;
        int glyphs = (int)next(3);
        for (int k = 0; k < glyphs; k++) {
            int64_t reach = (int64_t)r * ch + between(-ch, 3 * (int64_t)ch);
            backdrop_through(&ca, &bc, reach);
            if (termshot_backdrop_through(&cb, &br, reach) < 0) bad = 1;
        }
    }
    backdrop_through(&ca, &bc, H);
    if (termshot_backdrop_through(&cb, &br, H) < 0) bad = 1;
    if (bc.done != br.done) {
        fprintf(stderr, "FAIL %s: C painted %d rows, Rust %d\n", what, bc.done, br.done);
        bad = 1;
    }
    paint_images(&ca, views, n, LAYER_OVER_TEXT, NULL, cw, ch);
    if (termshot_paint_images(&cb, views, n, LAYER_OVER_TEXT) != 0) bad = 1;
    if (memcmp(a, b, bytes) != 0) {
        size_t i = 0;
        while (a[i] == b[i]) i++;
        fprintf(stderr, "FAIL %s: %dx%d cells of %dx%d, %zu views, %s: byte %zu (pixel %zu, %zu) differs: C %u, Rust %u\n",
                what, cols, rows, cw, ch, n, bc.whole ? "whole" : "rows", i, i % stride / 3, i / stride, a[i], b[i]);
        bad = 1;
    }
    compared += (long long)bytes;
    pixels_compared += W * H;
    scenes++;
    free(a);
    free(b);
    failures += bad;
    return bad;
}

static Cell *random_cells(int count) {
    Cell *cells = malloc(sizeof *cells * (size_t)count);
    if (!cells) exit(2);
    int opaque = (int)next(4); /* none, some, most, all */
    for (int i = 0; i < count; i++) {
        uint32_t r = next(4);
        int o = opaque == 0 ? 0 : opaque == 3 ? 1 : opaque == 1 ? r == 0 : r != 0;
        cells[i] = (Cell){.ch = ' ', .br = (uint8_t)next(256), .bg = (uint8_t)next(256), .bb = (uint8_t)next(256),
                          .attrs = (uint8_t)((o ? ATTR_OPAQUE : 0) | (next(2) ? ATTR_BOLD : 0))};
        if (!o && next(2)) cells[i].br = 17, cells[i].bg = 24, cells[i].bb = 35;
    }
    return cells;
}

static void random_scenes(int count) {
    for (int s = 0; s < count && failures < 10; s++) {
        int cols = (int)between(1, 30), rows = (int)between(1, 12), cw = (int)between(1, 16), ch = (int)between(1, 32);
        int64_t W = (int64_t)cols * cw, H = (int64_t)rows * ch;
        Picture pool[6];
        int npool = (int)between(1, 6);
        for (int k = 0; k < npool; k++) {
            uint32_t big = next(6) == 0;
            pool[k] = picture(1 + next(big ? 300 : 24), 1 + next(big ? 200 : 24));
        }
        size_t n = next(12) == 0 ? (size_t)between(60, 80) : (size_t)next(9);
        ImageView views[80];
        for (size_t k = 0; k < n; k++) views[k] = random_view(pool, npool, W, H, cw, ch);
        Cell *cells = random_cells(cols * rows);
        char what[32];
        snprintf(what, sizeof what, "scene %d", s);
        compare_scene(cells, cols, rows, cw, ch, views, n, (int)next(3) - 1, what);
        free(cells);
        for (int k = 0; k < npool; k++) free(pool[k].px);
    }
}

/* bench.py's image screens: 100x30 cells, a 128x128 RGBA image put over
   60x20 cells at z, and every third row of text on its own background. */
typedef struct {
    Cell *cells;
    int cols, rows, cw, ch;
    ImageView views[96];
    size_t n;
} Screen;

static Picture bench_picture;

static Screen image_screen(int cols, int rows, int cw, int ch, int32_t z) {
    Screen s = {random_cells(cols * rows), cols, rows, cw, ch, {{0}}, 0};
    for (int r = 0; r < rows; r++)
        for (int c = 0; c < cols; c++) {
            Cell *cell = &s.cells[r * cols + c];
            *cell = (Cell){.ch = 'x', .br = 17, .bg = 24, .bb = 35};
            if (r % 3 == 0) *cell = (Cell){.ch = 'x', .br = (uint8_t)(17 + r), .bg = 0, .bb = 95, .attrs = ATTR_OPAQUE};
        }
    s.views[s.n++] = (ImageView){.pixels = bench_picture.px, .width = 128, .height = 128, .x = 0, .y = 0,
                                 .w = 60 * (int64_t)cw, .h = 20 * (int64_t)ch, .clip_top = 0, .clip_bottom = 20 * (int64_t)ch,
                                 .clip_left = INT64_MIN, .clip_right = INT64_MAX, .src_w = 128, .src_h = 128, .z = z};
    return s;
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

/* The median ms each side takes to paint a screen's backdrop, a row at a
   time as draw_png does (or whole, as it decides), and its images over the
   text. */
static void time_screen(const char *name, const Screen *s, int rounds) {
    int64_t W = (int64_t)s->cols * s->cw, H = (int64_t)s->rows * s->ch;
    size_t stride = (size_t)W * 3 + 1, bytes = stride * (size_t)H;
    uint8_t *px = malloc(bytes);
    double *t[2] = {malloc(sizeof(double) * (size_t)rounds), malloc(sizeof(double) * (size_t)rounds)};
    if (!px || !t[0] || !t[1]) exit(2);
    pattern(px, bytes, 0);
    Canvas cv = {.px = px + 1, .filtered = px, .w = (int)W, .h = (int)H, .stride = stride};
    int whole = 0;
    for (int round = 0; round < rounds; round++) {
        for (int k = 0; k < 2; k++) {
            int side = (round + k) % 2; /* alternate which goes first */
            double start = now();
            if (side == 0) {
                Backdrop bd = backdrop_for(&cv, s->cells, s->cols, s->rows, s->cw, s->ch, s->views, s->n);
                for (int r = 0; r < s->rows; r++) backdrop_through(&cv, &bd, (int64_t)(r + 1) * s->ch);
                paint_images(&cv, s->views, s->n, LAYER_OVER_TEXT, NULL, s->cw, s->ch);
                whole = bd.whole;
            } else {
                RustBackdrop bd;
                termshot_backdrop_init(&bd, &cv, s->cells, s->cols, s->rows, s->cw, s->ch, s->views, s->n,
                                       BACKDROP_ROW_BYTES);
                for (int r = 0; r < s->rows; r++) termshot_backdrop_through(&cv, &bd, (int64_t)(r + 1) * s->ch);
                termshot_paint_images(&cv, s->views, s->n, LAYER_OVER_TEXT);
            }
            t[side][round] = now() - start;
        }
    }
    qsort(t[0], (size_t)rounds, sizeof(double), by_value);
    qsort(t[1], (size_t)rounds, sizeof(double), by_value);
    double c = t[0][rounds / 2], r = t[1][rounds / 2];
    printf("%-14s %4dx%-3d %3dx%-3d %2zu views %-5s  C %8.3f ms  Rust %8.3f ms  Rust/C %.3f\n", name, s->cols, s->rows,
           s->cw, s->ch, s->n, whole ? "whole" : "rows", c, r, r / c);
    free(px);
    free(t[0]);
    free(t[1]);
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: images FONT [rounds]\n");
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
    int cw, ch;
    if (!draw_cell_size(font, 0, 48, &cw, &ch)) return 1;
    bench_picture = picture(128, 128);
    for (size_t i = 0; i < 128 * 128; i++) bench_picture.px[i * 4 + 3] = (unsigned char)(i % 3 ? 255 : 128 + i % 97);

    if (!getenv("TIME_ONLY")) {
        random_scenes(20000);
        printf("%lld random scenes: %d failures so far\n", scenes, failures);
        fflush(stdout);
        /* At the CLI's sizes: 240x80 cells at --px 48 is over 16 MiB, so by
           rows, with images in every layer, and 80 placeholder runs (whole). */
        int32_t layers[] = {-1073741825, -1, 1};
        for (int k = 0; k < 3; k++) {
            Screen s = image_screen(240, 80, cw, ch, layers[k]);
            for (int v = 0; v < 6; v++) s.views[s.n++] = random_view(&bench_picture, 1, 240 * cw, 80 * ch, cw, ch);
            compare_scene(s.cells, s.cols, s.rows, cw, ch, s.views, s.n, -1, "240x80 at px 48");
            free(s.cells);
        }
        Screen s = image_screen(240, 80, cw, ch, -1);
        s.n = 0;
        for (int r = 0; r < 80; r++) {
            int64_t y = (int64_t)r * ch;
            s.views[s.n++] = (ImageView){.pixels = bench_picture.px, .width = 128, .height = 128, .x = 0, .y = 0,
                                         .w = 240 * (int64_t)cw, .h = 80 * (int64_t)ch, .clip_top = y, .clip_bottom = y + ch,
                                         .clip_left = (r % 7) * (int64_t)cw, .clip_right = (r % 7 + 30) * (int64_t)cw,
                                         .src_w = 128, .src_h = 128, .z = r % 2 ? -1 : INT32_MIN};
        }
        compare_scene(s.cells, s.cols, s.rows, cw, ch, s.views, s.n, -1, "80 placeholder runs");
        compare_scene(s.cells, s.cols, s.rows, cw, ch, s.views, s.n, 0, "80 placeholder runs, by rows");
        free(s.cells);
        if (failures) {
            printf("FAIL: %d scenes differ\n", failures);
            return 1;
        }
        printf("ok, %lld scenes painted alike by the C and the Rust (%lld pixels, %lld bytes compared)\n", scenes,
               pixels_compared, compared);
        fflush(stdout);
    }

    printf("== time per screen (backdrop and images over the text), median of %d alternating rounds\n", rounds);
    const char *names[] = {"image-below", "image-under", "image-over"};
    int32_t layers[] = {-1073741825, -1, 1};
    for (int k = 0; k < 3; k++) {
        Screen s = image_screen(100, 30, cw, ch, layers[k]);
        time_screen(names[k], &s, rounds);
        free(s.cells);
    }
    Screen s = image_screen(240, 80, cw, ch, -1073741825);
    time_screen("large-below", &s, rounds);
    s.n = 0;
    time_screen("large-none", &s, rounds);
    free(s.cells);
    free(bench_picture.px);
    free(font);
    return 0;
}
