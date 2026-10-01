/* Headless cell-grid PNG. Same file on macOS and Linux.
   Raster is stb_truetype (public domain). PNG is stb_image_write (public domain,
   deflate included). No Core Text, FreeType, window, or distro package.
   Box-drawing is geometry so the joints meet at an integer cell size. */

#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define STB_TRUETYPE_IMPLEMENTATION
#include "stb_truetype.h"
/* stb's deflate, with a faster search that writes the same bytes (deflate.c). */
unsigned char *termshot_zlib_compress(unsigned char *data, int data_len, int *out_len, int quality);
#define STBIW_ZLIB_COMPRESS termshot_zlib_compress
#define STB_IMAGE_WRITE_IMPLEMENTATION
#include "stb_image_write.h"

typedef struct {
    uint32_t ch;
    uint8_t fr, fg, fb;
    uint8_t br, bg, bb;
    uint8_t bold;
} Cell;

_Static_assert(sizeof(Cell) == 12, "Cell ABI must match the Rust side");

/* The canvas is RGB: alpha would always be 255, and an opaque RGBA PNG is
   larger and blocks palette quantization in downstream optimizers. */
#define BPP 3

/* 2^27 pixels keeps the 3-byte rows plus filter bytes near 384 MiB, well under INT_MAX. */
#define MAX_PIXELS (1 << 27)

/* The image being painted. Passed explicitly so draw_png is reentrant. */
typedef struct {
    uint8_t *px;
    int w, h;
} Canvas;

static void put(Canvas *cv, int x, int y, uint8_t r, uint8_t g, uint8_t b) {
    if ((unsigned)x >= (unsigned)cv->w || (unsigned)y >= (unsigned)cv->h) return;
    uint8_t *p = cv->px + ((size_t)y * cv->w + x) * BPP;
    p[0] = r;
    p[1] = g;
    p[2] = b;
}

static void fill_rect(Canvas *cv, int x0, int y0, int x1, int y1, uint8_t r, uint8_t g, uint8_t b) {
    if (x0 < 0) x0 = 0;
    if (y0 < 0) y0 = 0;
    if (x1 > cv->w) x1 = cv->w;
    if (y1 > cv->h) y1 = cv->h;
    for (int y = y0; y < y1; y++) {
        uint8_t *row = cv->px + ((size_t)y * cv->w + x0) * BPP;
        for (int x = x0; x < x1; x++) {
            row[0] = r;
            row[1] = g;
            row[2] = b;
            row += BPP;
        }
    }
}

static void hbar(Canvas *cv, int x0, int x1, int mid, int thick, uint8_t r, uint8_t g, uint8_t b) {
    fill_rect(cv, x0, mid - thick / 2, x1, mid - thick / 2 + thick, r, g, b);
}

static void vbar(Canvas *cv, int mid, int y0, int y1, int thick, uint8_t r, uint8_t g, uint8_t b) {
    fill_rect(cv, mid - thick / 2, y0, mid - thick / 2 + thick, y1, r, g, b);
}

/* Quarter ellipse. Angles are standard math angles with y growing downward. */
static void arc(Canvas *cv, float cx, float cy, float rx, float ry, float a0, float a1, float thick,
                 uint8_t r, uint8_t g, uint8_t b) {
    int steps = (int)((rx + ry) * 2.0f);
    if (steps < 12) steps = 12;
    float rad = thick * 0.5f;
    float rad2 = (rad + 0.6f) * (rad + 0.6f);
    for (int i = 0; i <= steps; i++) {
        float a = a0 + (a1 - a0) * ((float)i / (float)steps);
        float px = cx + rx * cosf(a);
        float py = cy + ry * sinf(a);
        int x0 = (int)floorf(px - rad - 1.0f);
        int y0 = (int)floorf(py - rad - 1.0f);
        int x1 = (int)ceilf(px + rad + 1.0f);
        int y1 = (int)ceilf(py + rad + 1.0f);
        for (int y = y0; y <= y1; y++) {
            for (int x = x0; x <= x1; x++) {
                float dx = (x + 0.5f) - px;
                float dy = (y + 0.5f) - py;
                if (dx * dx + dy * dy <= rad2) put(cv, x, y, r, g, b);
            }
        }
    }
}

static int paint_geometry(Canvas *cv, int col, int row, int cell_w, int cell_h, uint32_t cp, int bold,
                           uint8_t r, uint8_t g, uint8_t b) {
    int x = col * cell_w;
    int y = row * cell_h;
    int right = x + cell_w;
    int bottom = y + cell_h;
    int cx = x + cell_w / 2;
    int cy = y + cell_h / 2;
    int t = cell_w / 12;
    if (t < 1) t = 1;
    if (bold) t += 1;
    int jx = cx - t / 2;
    int jy = cy - t / 2;
    const float pi = 3.14159265f;
    switch (cp) {
    case 0x2500:
        hbar(cv, x, right, cy, t, r, g, b);
        return 1;
    case 0x2502:
        vbar(cv, cx, y, bottom, t, r, g, b);
        return 1;
    case 0x250C:
        hbar(cv, jx, right, cy, t, r, g, b);
        vbar(cv, cx, jy, bottom, t, r, g, b);
        return 1;
    case 0x2510:
        hbar(cv, x, jx + t, cy, t, r, g, b);
        vbar(cv, cx, jy, bottom, t, r, g, b);
        return 1;
    case 0x2514:
        hbar(cv, jx, right, cy, t, r, g, b);
        vbar(cv, cx, y, jy + t, t, r, g, b);
        return 1;
    case 0x2518:
        hbar(cv, x, jx + t, cy, t, r, g, b);
        vbar(cv, cx, y, jy + t, t, r, g, b);
        return 1;
    case 0x256D: /* ╭ arc down and right */
        arc(cv, (float)right, (float)bottom, cell_w * 0.5f, cell_h * 0.5f, pi, pi * 1.5f, (float)t, r, g, b);
        return 1;
    case 0x256E: /* ╮ */
        arc(cv, (float)x, (float)bottom, cell_w * 0.5f, cell_h * 0.5f, -pi * 0.5f, 0.0f, (float)t, r, g, b);
        return 1;
    case 0x256F: /* ╯ */
        arc(cv, (float)x, (float)y, cell_w * 0.5f, cell_h * 0.5f, 0.0f, pi * 0.5f, (float)t, r, g, b);
        return 1;
    case 0x2570: /* ╰ */
        arc(cv, (float)right, (float)y, cell_w * 0.5f, cell_h * 0.5f, pi * 0.5f, pi, (float)t, r, g, b);
        return 1;
    case 0x2588:
        fill_rect(cv, x, y, right, bottom, r, g, b);
        return 1;
    case 0x2580:
        fill_rect(cv, x, y, right, y + cell_h / 2, r, g, b);
        return 1;
    default:
        return 0;
    }
}

static void blend(Canvas *cv, int dx, int dy, const unsigned char *bm, int gw, int gh,
                   uint8_t r, uint8_t g, uint8_t b) {
    for (int y = 0; y < gh; y++) {
        int iy = dy + y;
        if ((unsigned)iy >= (unsigned)cv->h) continue;
        for (int x = 0; x < gw; x++) {
            int ix = dx + x;
            if ((unsigned)ix >= (unsigned)cv->w) continue;
            unsigned char a = bm[y * gw + x];
            if (a == 0) continue;
            uint8_t *p = cv->px + ((size_t)iy * cv->w + ix) * BPP;
            if (a == 255) {
                p[0] = r;
                p[1] = g;
                p[2] = b;
            } else {
                p[0] = (uint8_t)((r * a + p[0] * (255 - a) + 127) / 255);
                p[1] = (uint8_t)((g * a + p[1] * (255 - a) + 127) / 255);
                p[2] = (uint8_t)((b * a + p[2] * (255 - a) + 127) / 255);
            }
        }
    }
}

/* stb_image_write keeps the forced filter in a global. Set it once at load
   time, before any thread can call draw_png, instead of on every call. */
__attribute__((constructor)) static void use_no_png_filter(void) {
    /* No row filter: on flat terminal colours it is both the smallest and the
       fastest choice, ahead of stb's per-row heuristic. */
    stbi_write_force_png_filter = 0;
}

/* Paint cells with the font in font (a TrueType file the caller has already
   checked; see src/font.rs) and write a PNG. Reentrant: all state is local.
   Returns 0, 1 for an unusable font, or 2 for an image or write failure. */
int draw_png(const Cell *cells, int cols, int rows, const unsigned char *ttf, double font_px,
             const char *out_path) {
    stbtt_fontinfo font;
    int offset = stbtt_GetFontOffsetForIndex(ttf, 0);
    if (offset < 0 || !stbtt_InitFont(&font, ttf, offset)) {
        fprintf(stderr, "font init failed\n");
        return 1;
    }

    int ascent, descent, line_gap;
    stbtt_GetFontVMetrics(&font, &ascent, &descent, &line_gap);
    int adv = 0, lsb = 0;
    stbtt_GetCodepointHMetrics(&font, 'M', &adv, &lsb);
    if (adv <= 0 || ascent <= descent) {
        fprintf(stderr, "font metrics unusable\n");
        return 1;
    }
    float scale = stbtt_ScaleForPixelHeight(&font, (float)font_px);
    int cell_w = (int)(adv * scale + 0.5f);
    if (cell_w < 1) cell_w = 1;
    scale = (float)cell_w / (float)adv;
    int body = (int)((ascent - descent) * scale + 0.5f);
    int gap = (int)(line_gap * scale + 0.5f);
    if (gap < 0) gap = 0;
    int cell_h = body + gap;
    if (cell_h < 1) cell_h = 1;
    int baseline = (int)(ascent * scale + 0.5f) + (cell_h - body) / 2;
    long long width = (long long)cols * cell_w;
    long long height = (long long)rows * cell_h;
    fprintf(stderr, "advance %d units scale %.5f cell %dx%d baseline %d image %lldx%lld\n",
            adv, scale, cell_w, cell_h, baseline, width, height);
    /* stb_image_write sizes its buffers with int: (width*BPP+1)*height must not wrap. */
    if (width * height > MAX_PIXELS) {
        fprintf(stderr, "image %lldx%lld is over %d pixels; lower px, cols or rows\n",
                width, height, MAX_PIXELS);
        return 2;
    }

    Canvas canvas = {(uint8_t *)malloc((size_t)(width * height * BPP)), (int)width, (int)height};
    Canvas *cv = &canvas;
    if (!cv->px) {
        fprintf(stderr, "out of memory for a %lldx%lld image\n", width, height);
        return 2;
    }

    for (int r = 0; r < rows; r++) {
        for (int c = 0; c < cols; c++) {
            const Cell *cell = &cells[r * cols + c];
            fill_rect(cv, c * cell_w, r * cell_h, (c + 1) * cell_w, (r + 1) * cell_h, cell->br, cell->bg, cell->bb);
        }
    }

    for (int r = 0; r < rows; r++) {
        for (int c = 0; c < cols; c++) {
            const Cell *cell = &cells[r * cols + c];
            uint32_t cp = cell->ch;
            if (cp == 0 || cp == ' ') continue;
            if (paint_geometry(cv, c, r, cell_w, cell_h, cp, cell->bold, cell->fr, cell->fg, cell->fb)) continue;
            if (stbtt_FindGlyphIndex(&font, (int)cp) == 0) continue;
            int ix0, iy0, ix1, iy1;
            stbtt_GetCodepointBitmapBox(&font, (int)cp, scale, scale, &ix0, &iy0, &ix1, &iy1);
            int gw = ix1 - ix0;
            int gh = iy1 - iy0;
            if (gw <= 0 || gh <= 0) continue;
            unsigned char *bm = (unsigned char *)malloc((size_t)gw * gh);
            if (!bm) continue;
            stbtt_MakeCodepointBitmap(&font, bm, gw, gh, gw, scale, scale, (int)cp);
            int dx = c * cell_w + ix0;
            int dy = r * cell_h + baseline + iy0;
            blend(cv, dx, dy, bm, gw, gh, cell->fr, cell->fg, cell->fb);
            if (cell->bold) blend(cv, dx + 1, dy, bm, gw, gh, cell->fr, cell->fg, cell->fb);
            free(bm);
        }
    }

    int ok = stbi_write_png(out_path, cv->w, cv->h, BPP, cv->px, cv->w * BPP);
    free(cv->px);
    if (!ok) {
        fprintf(stderr, "png write failed: %s\n", out_path);
        return 2;
    }
    return 0;
}
