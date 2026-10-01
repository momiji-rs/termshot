/* Headless cell-grid PNG. Same file on macOS and Linux.
   Raster is stb_truetype (public domain). PNG is stb_image_write (public domain,
   deflate included). No Core Text, FreeType, window, or distro package.
   Box-drawing is geometry so the joints meet at an integer cell size. */

#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static int profiling;
static double png_marks[4];
static double now_ms(void) {
    if (!profiling) return 0;
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1000.0 + (double)ts.tv_nsec / 1000000.0;
}
#define STBIW_PNG_PROFILE(stage) (png_marks[stage] = now_ms())

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

/* Bounded per-render cache. Bold and colors reuse the same coverage bitmap. */
typedef struct {
    uint32_t cp;
    int valid, ix0, iy0, w, h;
    unsigned char *bitmap;
} Glyph;
#define GLYPH_CACHE_SIZE 256

static uint8_t *g_img;
static int g_w, g_h;

static void put(int x, int y, uint8_t r, uint8_t g, uint8_t b) {
    if ((unsigned)x >= (unsigned)g_w || (unsigned)y >= (unsigned)g_h) return;
    uint8_t *p = g_img + ((size_t)y * g_w + x) * BPP;
    p[0] = r;
    p[1] = g;
    p[2] = b;
}

static void fill_rect(int x0, int y0, int x1, int y1, uint8_t r, uint8_t g, uint8_t b) {
    if (x0 < 0) x0 = 0;
    if (y0 < 0) y0 = 0;
    if (x1 > g_w) x1 = g_w;
    if (y1 > g_h) y1 = g_h;
    for (int y = y0; y < y1; y++) {
        uint8_t *row = g_img + ((size_t)y * g_w + x0) * BPP;
        for (int x = x0; x < x1; x++) {
            row[0] = r;
            row[1] = g;
            row[2] = b;
            row += BPP;
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
    int x0 = dx < 0 ? -dx : 0;
    int y0 = dy < 0 ? -dy : 0;
    int x1 = gw < g_w - dx ? gw : g_w - dx;
    int y1 = gh < g_h - dy ? gh : g_h - dy;
    for (int y = y0; y < y1; y++) {
        int iy = dy + y;
        for (int x = x0; x < x1; x++) {
            int ix = dx + x;
            unsigned char a = bm[y * gw + x];
            if (a == 0) continue;
            uint8_t *p = g_img + ((size_t)iy * g_w + ix) * BPP;
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

int draw_png(const Cell *cells, int cols, int rows, const char *font_path, double font_px, const char *out_path) {
    profiling = getenv("TERMSHOT_PROFILE") != NULL;
    double started = now_ms();
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
    double font_read = now_ms();

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

    /* stb_image_write sizes its buffers with int: (width*BPP+1)*height must not wrap. */
    if ((long long)width * height > MAX_PIXELS) {
        free(ttf);
        fprintf(stderr, "image %dx%d is over %d pixels; lower px, cols or rows\n",
                width, height, MAX_PIXELS);
        return 2;
    }

    g_w = width;
    g_h = height;
    double font_setup = now_ms();
    g_img = (uint8_t *)malloc((size_t)width * height * BPP);
    if (!g_img) {
        free(ttf);
        return 2;
    }

    double allocated = now_ms();
    for (int r = 0; r < rows; r++) {
        int y = r * cell_h;
        for (int c = 0; c < cols; c++) {
            const Cell *cell = &cells[r * cols + c];
            fill_rect(c * cell_w, y, (c + 1) * cell_w, y + 1, cell->br, cell->bg, cell->bb);
        }
        const uint8_t *scanline = g_img + (size_t)y * width * BPP;
        for (int dy = 1; dy < cell_h; dy++) {
            memcpy(g_img + (size_t)(y + dy) * width * BPP, scanline, (size_t)width * BPP);
        }
    }

    double background = now_ms();
    double geometry_ms = 0, glyph_ms = 0, blend_ms = 0;
    size_t glyphs = 0, cache_hits = 0;
    Glyph cache[GLYPH_CACHE_SIZE] = {0};
    for (int r = 0; r < rows; r++) {
        for (int c = 0; c < cols; c++) {
            const Cell *cell = &cells[r * cols + c];
            uint32_t cp = cell->ch;
            if (cp == 0 || cp == ' ') continue;
            double tick = now_ms();
            int geometry = paint_geometry(c, r, cell_w, cell_h, cp, cell->bold, cell->fr, cell->fg, cell->fb);
            geometry_ms += now_ms() - tick;
            if (geometry) continue;
            tick = now_ms();
            Glyph *entry = &cache[cp % GLYPH_CACHE_SIZE];
            if (entry->valid && entry->cp == cp) {
                cache_hits++;
            } else {
                free(entry->bitmap);
                *entry = (Glyph){.cp = cp, .valid = 1};
                int glyph = stbtt_FindGlyphIndex(&font, (int)cp);
                if (glyph != 0) {
                    int ix1, iy1;
                    stbtt_GetGlyphBitmapBox(&font, glyph, scale, scale, &entry->ix0, &entry->iy0, &ix1, &iy1);
                    entry->w = ix1 - entry->ix0;
                    entry->h = iy1 - entry->iy0;
                    if (entry->w > 0 && entry->h > 0) {
                        entry->bitmap = (unsigned char *)malloc((size_t)entry->w * entry->h);
                        if (!entry->bitmap) {
                            for (int k = 0; k < GLYPH_CACHE_SIZE; k++) free(cache[k].bitmap);
                            free(g_img);
                            g_img = NULL;
                            free(ttf);
                            fprintf(stderr, "glyph allocation failed\n");
                            return 2;
                        }
                        stbtt_MakeGlyphBitmap(&font, entry->bitmap, entry->w, entry->h, entry->w, scale, scale, glyph);
                        glyphs++;
                    }
                }
            }
            glyph_ms += now_ms() - tick;
            tick = now_ms();
            if (!entry->bitmap) continue;
            const unsigned char *bm = entry->bitmap;
            int gw = entry->w, gh = entry->h;
            int dx = c * cell_w + entry->ix0;
            int dy = r * cell_h + baseline + entry->iy0;
            blend(dx, dy, bm, gw, gh, cell->fr, cell->fg, cell->fb);
            if (cell->bold) blend(dx + 1, dy, bm, gw, gh, cell->fr, cell->fg, cell->fb);
            blend_ms += now_ms() - tick;

        }
    }

    for (int k = 0; k < GLYPH_CACHE_SIZE; k++) free(cache[k].bitmap);
    double foreground = now_ms();
    /* Keep the no-filter encoding selected by upstream for terminal images. */
    stbi_write_force_png_filter = 0;
    int png_len = 0;
    unsigned char *png = stbi_write_png_to_mem(g_img, width * BPP, width, height, BPP, &png_len);
    double encoded = now_ms();
    int ok = 0;
    if (png) {
        FILE *out = fopen(out_path, "wb");
        if (out) {
            ok = fwrite(png, 1, (size_t)png_len, out) == (size_t)png_len;
            if (fclose(out) != 0) ok = 0;
        }
        free(png);
    }
    double written = now_ms();
    free(g_img);
    g_img = NULL;
    free(ttf);
    if (profiling) {
        fprintf(stderr, "termshot-profile {\"font_read_ms\":%.6f,\"font_setup_ms\":%.6f,\"allocate_ms\":%.6f,\"background_ms\":%.6f,\"foreground_ms\":%.6f,\"geometry_ms\":%.6f,\"glyph_ms\":%.6f,\"blend_ms\":%.6f,\"png_filter_ms\":%.6f,\"png_deflate_ms\":%.6f,\"png_pack_ms\":%.6f,\"png_encode_ms\":%.6f,\"output_write_ms\":%.6f,\"cleanup_ms\":%.6f,\"glyph_rasterizations\":%zu,\"glyph_cache_hits\":%zu,\"png_bytes\":%d,\"pixel_bytes\":%zu}\n",
            font_read - started, font_setup - font_read, allocated - font_setup,
            background - allocated, foreground - background, geometry_ms, glyph_ms, blend_ms,
            png_marks[1] - png_marks[0], png_marks[2] - png_marks[1], png_marks[3] - png_marks[2],
            encoded - foreground, written - encoded, now_ms() - written, glyphs, cache_hits, png_len, (size_t)width * height * BPP);
    }
    if (!ok) {
        fprintf(stderr, "png write failed: %s\n", out_path);
        return 2;
    }
    return 0;
}
