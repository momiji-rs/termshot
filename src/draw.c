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
#define STB_IMAGE_WRITE_IMPLEMENTATION
#include "stb_image_write.h"

typedef struct {
    uint32_t ch;
    uint8_t fr, fg, fb;
    uint8_t br, bg, bb;
    uint8_t bold;
} Cell;

_Static_assert(sizeof(Cell) == 12, "Cell ABI must match the Rust side");

static uint8_t *g_img;
static int g_w, g_h;

static void put(int x, int y, uint8_t r, uint8_t g, uint8_t b) {
    if ((unsigned)x >= (unsigned)g_w || (unsigned)y >= (unsigned)g_h) return;
    uint8_t *p = g_img + ((size_t)y * g_w + x) * 4;
    p[0] = r;
    p[1] = g;
    p[2] = b;
    p[3] = 255;
}

static void fill_rect(int x0, int y0, int x1, int y1, uint8_t r, uint8_t g, uint8_t b) {
    if (x0 < 0) x0 = 0;
    if (y0 < 0) y0 = 0;
    if (x1 > g_w) x1 = g_w;
    if (y1 > g_h) y1 = g_h;
    for (int y = y0; y < y1; y++) {
        uint8_t *row = g_img + ((size_t)y * g_w + x0) * 4;
        for (int x = x0; x < x1; x++) {
            row[0] = r;
            row[1] = g;
            row[2] = b;
            row[3] = 255;
            row += 4;
        }
    }
}

static void hbar(int x0, int x1, int mid, int thick, uint8_t r, uint8_t g, uint8_t b) {
    fill_rect(x0, mid - thick / 2, x1, mid - thick / 2 + thick, r, g, b);
}

static void vbar(int mid, int y0, int y1, int thick, uint8_t r, uint8_t g, uint8_t b) {
    fill_rect(mid - thick / 2, y0, mid - thick / 2 + thick, y1, r, g, b);
}

/* Quarter ellipse. Angles are standard math angles with y growing downward. */
static void arc(float cx, float cy, float rx, float ry, float a0, float a1, float thick,
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
                if (dx * dx + dy * dy <= rad2) put(x, y, r, g, b);
            }
        }
    }
}

static int paint_geometry(int col, int row, int cell_w, int cell_h, uint32_t cp, int bold,
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
        hbar(x, right, cy, t, r, g, b);
        return 1;
    case 0x2502:
        vbar(cx, y, bottom, t, r, g, b);
        return 1;
    case 0x250C:
        hbar(jx, right, cy, t, r, g, b);
        vbar(cx, jy, bottom, t, r, g, b);
        return 1;
    case 0x2510:
        hbar(x, jx + t, cy, t, r, g, b);
        vbar(cx, jy, bottom, t, r, g, b);
        return 1;
    case 0x2514:
        hbar(jx, right, cy, t, r, g, b);
        vbar(cx, y, jy + t, t, r, g, b);
        return 1;
    case 0x2518:
        hbar(x, jx + t, cy, t, r, g, b);
        vbar(cx, y, jy + t, t, r, g, b);
        return 1;
    case 0x256D: /* ╭ arc down and right */
        arc((float)right, (float)bottom, cell_w * 0.5f, cell_h * 0.5f, pi, pi * 1.5f, (float)t, r, g, b);
        return 1;
    case 0x256E: /* ╮ */
        arc((float)x, (float)bottom, cell_w * 0.5f, cell_h * 0.5f, -pi * 0.5f, 0.0f, (float)t, r, g, b);
        return 1;
    case 0x256F: /* ╯ */
        arc((float)x, (float)y, cell_w * 0.5f, cell_h * 0.5f, 0.0f, pi * 0.5f, (float)t, r, g, b);
        return 1;
    case 0x2570: /* ╰ */
        arc((float)right, (float)y, cell_w * 0.5f, cell_h * 0.5f, pi * 0.5f, pi, (float)t, r, g, b);
        return 1;
    case 0x2588:
        fill_rect(x, y, right, bottom, r, g, b);
        return 1;
    case 0x2580:
        fill_rect(x, y, right, y + cell_h / 2, r, g, b);
        return 1;
    default:
        return 0;
    }
}

static void blend(int dx, int dy, const unsigned char *bm, int gw, int gh,
                   uint8_t r, uint8_t g, uint8_t b) {
    for (int y = 0; y < gh; y++) {
        int iy = dy + y;
        if ((unsigned)iy >= (unsigned)g_h) continue;
        for (int x = 0; x < gw; x++) {
            int ix = dx + x;
            if ((unsigned)ix >= (unsigned)g_w) continue;
            unsigned char a = bm[y * gw + x];
            if (a == 0) continue;
            uint8_t *p = g_img + ((size_t)iy * g_w + ix) * 4;
            if (a == 255) {
                p[0] = r;
                p[1] = g;
                p[2] = b;
            } else {
                p[0] = (uint8_t)((r * a + p[0] * (255 - a) + 127) / 255);
                p[1] = (uint8_t)((g * a + p[1] * (255 - a) + 127) / 255);
                p[2] = (uint8_t)((b * a + p[2] * (255 - a) + 127) / 255);
            }
            p[3] = 255;
        }
    }
}

int draw_png(const Cell *cells, int cols, int rows, const char *font_path, double font_px, const char *out_path) {
    FILE *fp = fopen(font_path, "rb");
    if (!fp) {
        fprintf(stderr, "font open failed: %s\n", font_path);
        return 1;
    }
    fseek(fp, 0, SEEK_END);
    long len = ftell(fp);
    fseek(fp, 0, SEEK_SET);
    if (len <= 0) {
        fclose(fp);
        fprintf(stderr, "font empty: %s\n", font_path);
        return 1;
    }
    unsigned char *ttf = (unsigned char *)malloc((size_t)len);
    if (!ttf || fread(ttf, 1, (size_t)len, fp) != (size_t)len) {
        free(ttf);
        fclose(fp);
        fprintf(stderr, "font read failed: %s\n", font_path);
        return 1;
    }
    fclose(fp);

    stbtt_fontinfo font;
    int offset = stbtt_GetFontOffsetForIndex(ttf, 0);
    if (offset < 0 || !stbtt_InitFont(&font, ttf, offset)) {
        free(ttf);
        fprintf(stderr, "font init failed: %s\n", font_path);
        return 1;
    }

    int ascent, descent, line_gap;
    stbtt_GetFontVMetrics(&font, &ascent, &descent, &line_gap);
    int adv = 0, lsb = 0;
    stbtt_GetCodepointHMetrics(&font, 'M', &adv, &lsb);
    if (adv <= 0 || ascent <= descent) {
        free(ttf);
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
    int width = cols * cell_w;
    int height = rows * cell_h;
    fprintf(stderr, "advance %d units scale %.5f cell %dx%d baseline %d image %dx%d\n",
            adv, scale, cell_w, cell_h, baseline, width, height);

    g_w = width;
    g_h = height;
    g_img = (uint8_t *)malloc((size_t)width * height * 4);
    if (!g_img) {
        free(ttf);
        return 2;
    }

    for (int r = 0; r < rows; r++) {
        for (int c = 0; c < cols; c++) {
            const Cell *cell = &cells[r * cols + c];
            fill_rect(c * cell_w, r * cell_h, (c + 1) * cell_w, (r + 1) * cell_h, cell->br, cell->bg, cell->bb);
        }
    }

    for (int r = 0; r < rows; r++) {
        for (int c = 0; c < cols; c++) {
            const Cell *cell = &cells[r * cols + c];
            uint32_t cp = cell->ch;
            if (cp == 0 || cp == ' ') continue;
            if (paint_geometry(c, r, cell_w, cell_h, cp, cell->bold, cell->fr, cell->fg, cell->fb)) continue;
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
            blend(dx, dy, bm, gw, gh, cell->fr, cell->fg, cell->fb);
            if (cell->bold) blend(dx + 1, dy, bm, gw, gh, cell->fr, cell->fg, cell->fb);
            free(bm);
        }
    }

    int ok = stbi_write_png(out_path, width, height, 4, g_img, width * 4);
    free(g_img);
    g_img = NULL;
    free(ttf);
    if (!ok) {
        fprintf(stderr, "png write failed: %s\n", out_path);
        return 2;
    }
    return 0;
}
