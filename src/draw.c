/* Headless cell-grid PNG. Same file on macOS and Linux.
   Raster is stb_truetype (public domain). PNG is stb_image_write (public domain,
   deflate included). No Core Text, FreeType, window, or distro package.
   Box drawing and blocks are geometry, so the joints meet at an integer cell
   size; that painter is Rust (src/geometry.rs), and so is the compressor. */

#include <limits.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

/* PNG hooks have no context argument; keep timings independent per thread. */
static _Thread_local int profiling;
static _Thread_local double png_marks[4];
static double now_ms(void) {
    if (!profiling) return 0;
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1000.0 + (double)ts.tv_nsec / 1000000.0;
}
#define STBIW_PNG_PROFILE(stage) (png_marks[stage] = now_ms())

#define STB_TRUETYPE_IMPLEMENTATION
#include "stb_truetype.h"
/* stb's deflate, with faster search/emission and identical bytes, in Rust
   (src/deflate.rs). It returns a malloc'd buffer, which stb frees, or NULL
   when memory runs out. */
unsigned char *termshot_zlib_compress(unsigned char *data, int data_len, int *out_len, int quality);
#define STBIW_ZLIB_COMPRESS termshot_zlib_compress
/* Its stage timings, per thread: profiling on or off, then the last call's. */
typedef struct {
    double allocate_ms, match_emit_ms, finalize_ms, checksum_ms;
} DeflateTimings;
void termshot_deflate_profiling(int enabled);
void termshot_deflate_timings(DeflateTimings *out);
#include "png_crc.h"
#define STBIW_CRC32 termshot_png_crc
#define STB_IMAGE_WRITE_IMPLEMENTATION
#include "stb_image_write.h"

typedef struct {
    uint32_t ch;
    uint8_t fr, fg, fb;
    uint8_t br, bg, bb;
    uint8_t attrs;
} Cell;

_Static_assert(sizeof(Cell) == 12, "Cell ABI must match the Rust side");

/* One cell's combining marks, which a cell's one code point can't hold: the
   cell's index (row * cols + col) and up to MAX_MARKS code points, ending at
   the first 0. A list of them is sorted by cell. As CellMarks in src/main.rs. */
#define MAX_MARKS 4
typedef struct {
    uint32_t cell;
    uint32_t marks[MAX_MARKS];
} CellMarks;

_Static_assert(sizeof(CellMarks) == 20, "CellMarks ABI must match the Rust side");

/* Cell.attrs bits, as in src/main.rs. */
#define ATTR_BOLD 1
#define ATTR_UNDERLINE 2
#define ATTR_DOUBLE_UNDERLINE 4
#define ATTR_STRIKE 8
#define ATTR_WIDE 16 /* the first of a wide character's two cells */
#define ATTR_TAIL 32 /* the second; ch is 0 */
#define ATTR_ITALIC 64
/* The background hides an image placed below the cell backgrounds. main.rs
   sets it on every background that is not the default colour, and on reverse
   video and the block cursor, as kitty treats them. */
#define ATTR_OPAQUE 128

/* The canvas is RGB: alpha would always be 255, and an opaque RGBA PNG is
   larger and blocks palette quantization in downstream optimizers. */
#define BPP 3

/* 2^27 pixels keeps the 3-byte rows plus filter bytes near 384 MiB, well under INT_MAX. */
#define MAX_PIXELS (1 << 27)

/* Bounded per-render cache. Bold and colors reuse the same coverage bitmap.
   1024 slots avoid thrashing on mixed Unicode screens (800 distinct codepoints
   in the benchmark), while keeping metadata to 64 KiB on 64-bit builds. */
typedef struct {
    uint32_t cp;
    /* wide, italic and mark are part of the key: a wide glyph is centered
       over two cells, an italic one is slanted, and a combining mark is
       neither centered nor shrunk, as the font places it. shift moves the
       glyph right within its cell (or cells), and advance is how far the
       glyph, so drawn, moves the pen, in pixels; missing means neither font
       has it, and empty which fonts (EMPTY_IN_*) map it to an empty glyph. */
    int valid, wide, italic, mark, missing, empty, shift, ix0, iy0, w, h;
    float advance;
    unsigned char *bitmap;
} Glyph;
#define GLYPH_CACHE_SIZE 1024

/* A checked font (see src/font.rs) and the face of it to draw with: start is
   0 for a single font, its offset in a collection. stb_truetype runs CFF
   charstrings without bounds, so a CFF face brings its own outlines: outline
   fills out[0..capacity) as stbtt_GetGlyphShape would and box as
   stbtt_GetGlyphBox would, and returns the vertex count, which may be over
   capacity (call again with room for that many); 0 is no outline, or one that
   can't be drawn. NULL for a TrueType face. */
typedef struct {
    const unsigned char *ttf;
    int start;
    int (*outline)(const void *cff, int glyph, stbtt_vertex *out, int capacity, int box[4]);
    const void *cff;
} Face;

_Static_assert(sizeof(stbtt_vertex) == 14, "stbtt_vertex ABI must match cff::Vertex");

/* The outline last fetched from a CFF face, so the check for an empty glyph
   and the drawing share one run of its charstring. */
typedef struct {
    stbtt_vertex *v;
    int n, capacity;
    const Face *face; /* whose glyph v holds, or NULL */
    int glyph;
    int box[4];
} Outline;

/* Fetch glyph of a CFF face into o, unless o holds it already. Returns the
   vertex count, or -1 when memory runs out. A glyph with more vertices than
   o has room for runs its charstring twice, so o starts with room for most
   glyphs and doubles. */
static int cff_outline(Outline *o, const Face *face, int glyph) {
    if (o->face == face && o->glyph == glyph) return o->n;
    o->face = NULL;
    int want = o->capacity ? 0 : 512;
    for (;;) {
        if (want > o->capacity) {
            stbtt_vertex *v = (stbtt_vertex *)realloc(o->v, (size_t)want * sizeof *v);
            if (!v) return -1;
            o->v = v;
            o->capacity = want;
        }
        int n = face->outline(face->cff, glyph, o->v, o->capacity, o->box);
        if (n <= o->capacity) {
            o->n = n < 0 ? 0 : n;
            o->face = face;
            o->glyph = glyph;
            return o->n;
        }
        want = o->capacity > INT_MAX / 2 || n > 2 * o->capacity ? n : 2 * o->capacity;
    }
}

/* stbtt_IsGlyphEmpty, asked of the face's own outlines for CFF. -1 when
   memory runs out. */
static int glyph_empty(const stbtt_fontinfo *info, const Face *face, int glyph, Outline *o) {
    if (!face->outline) return stbtt_IsGlyphEmpty(info, glyph);
    int n = cff_outline(o, face, glyph);
    return n < 0 ? -1 : n == 0;
}

/* The image being painted. Passed explicitly so draw_png is reentrant, and
   shared with src/geometry.rs, which paints box drawing and blocks into it:
   as Canvas there, which asserts the same size. filtered is the PNG's
   filtered scanlines, a filter byte then w RGB pixels each, stride bytes
   apart, and px its first pixel. geometry is the render's box-drawing state
   (the arcs' offsets, and the strokes it reuses), or NULL to cache nothing. */
typedef struct Geometry Geometry;
typedef struct {
    uint8_t *px, *filtered;
    int32_t w, h;
    size_t stride;
    Geometry *geometry;
} Canvas;

_Static_assert(sizeof(Canvas) == 40, "Canvas ABI must match the Rust side");

/* Box drawing (U+2500..U+257F) and block elements (U+2580..U+259F), as
   geometry so the joints meet at an integer cell size, in Rust
   (src/geometry.rs). Rounded corners and diagonals are stamped, and strokes
   that cover the same pixels of their cells are painted again from a cache
   whose allocations stay within 4 MiB; running out of memory there only
   means a stroke is stamped afresh, so geometry never fails a render.

   termshot_geometry_new: the state of a render's geometry, which reuses
   strokes when reuse_strokes is nonzero; NULL when memory runs out, which
   paints the same pixels with no cache. termshot_geometry_free frees it.

   termshot_paint_geometry: paint the character cp of the cell at col, row
   (of cell_w x cell_h pixels) inside the cell, a pixel thicker when bold.
   1 when painted, 0 for other characters, -1 if the painter failed (a bug).

   termshot_fill_rect: fill [x0, x1) x [y0, y1), clipped to the canvas.

   termshot_paint_failed: nonzero if a call above failed (a bug) on this
   thread since termshot_geometry_new; the render then fails. */
Geometry *termshot_geometry_new(int reuse_strokes);
void termshot_geometry_free(Geometry *geometry);
int termshot_paint_geometry(const Canvas *cv, int col, int row, int cell_w, int cell_h, uint32_t cp, int bold,
                            uint8_t r, uint8_t g, uint8_t b);
void termshot_fill_rect(const Canvas *cv, int x0, int y0, int x1, int y1, uint8_t r, uint8_t g, uint8_t b);
int termshot_paint_failed(void);
/* The cache's counters, for TERMSHOT_PROFILE: strokes painted from it, kept
   in it, and stamped without it, and the bytes it holds now and held at
   most. Zeros with no cache. */
typedef struct {
    size_t hits, misses, uncached, bytes, peak;
} GeometryStats;
void termshot_geometry_stats(const Geometry *geometry, GeometryStats *out);

/* Italic is synthetic: the outline is slanted by 12 degrees before it is
   rasterized, so its edges are as smooth as upright ones. The slant pivots on
   the middle of the body, pivot font units above the baseline, so a glyph
   stays centered in its cell and leans as far into each neighbour. Slants
   the outline in place and returns its pixel box at scale s. */
#define ITALIC_SLANT 0.21256f /* tan(12 degrees) */
static void slant_outline(stbtt_vertex *v, int n, float pivot, float s, int *ix0, int *iy0, int *ix1, int *iy1) {
    float min_x = 0, min_y = 0, max_x = 0, max_y = 0;
    for (int i = 0; i < n; i++) {
        /* Control points are inside the box of the curve's points, so the
           box of all of them holds the outline. A line has none. */
        int points = v[i].type == STBTT_vcubic ? 3 : v[i].type == STBTT_vcurve ? 2 : 1;
        stbtt_vertex_type *xs[3] = {&v[i].x, &v[i].cx, &v[i].cx1};
        stbtt_vertex_type ys[3] = {v[i].y, v[i].cy, v[i].cy1};
        for (int k = 0; k < points; k++) {
            float x = *xs[k] + ITALIC_SLANT * (ys[k] - pivot);
            x = x < -32768 ? -32768 : x > 32767 ? 32767 : x;
            *xs[k] = (stbtt_vertex_type)lroundf(x);
            if ((i == 0 && k == 0) || *xs[k] < min_x) min_x = *xs[k];
            if ((i == 0 && k == 0) || *xs[k] > max_x) max_x = *xs[k];
            if ((i == 0 && k == 0) || ys[k] < min_y) min_y = ys[k];
            if ((i == 0 && k == 0) || ys[k] > max_y) max_y = ys[k];
        }
    }
    /* As stbtt_GetGlyphBitmapBox rounds the glyph's own box; y grows down. */
    *ix0 = (int)floorf(min_x * s);
    *iy0 = (int)floorf(-max_y * s);
    *ix1 = (int)ceilf(max_x * s);
    *iy1 = (int)ceilf(-min_y * s);
    if (n <= 0) *ix0 = *iy0 = *ix1 = *iy1 = 0;
}

static void blend(Canvas *cv, int dx, int dy, const unsigned char *bm, int gw, int gh,
                   uint8_t r, uint8_t g, uint8_t b) {
    int x0 = dx < 0 ? -dx : 0;
    int y0 = dy < 0 ? -dy : 0;
    int x1 = gw < cv->w - dx ? gw : cv->w - dx;
    int y1 = gh < cv->h - dy ? gh : cv->h - dy;
    for (int y = y0; y < y1; y++) {
        int iy = dy + y;
        for (int x = x0; x < x1; x++) {
            int ix = dx + x;
            unsigned char a = bm[y * gw + x];
            if (a == 0) continue;
            uint8_t *p = cv->px + (size_t)iy * cv->stride + (size_t)ix * BPP;
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

/* stbtt_InitFont for a CFF2 face, which stb refuses: with no glyf it wants a
   CFF table to read outlines from. A CFF2 face's outlines come from Rust
   (Face.outline), so only the rest of InitFont is done, as stb does it: the
   metrics tables and the cmap subtable it would choose. font.rs has checked
   all of them. No outline table is set, so stb can't read one. */
static int init_cff2(stbtt_fontinfo *info, unsigned char *data, int start) {
    stbtt_uint32 cmap = stbtt__find_table(data, start, "cmap");
    memset(info, 0, sizeof *info);
    info->data = data;
    info->fontstart = start;
    info->head = stbtt__find_table(data, start, "head");
    info->hhea = stbtt__find_table(data, start, "hhea");
    info->hmtx = stbtt__find_table(data, start, "hmtx");
    info->kern = stbtt__find_table(data, start, "kern");
    info->gpos = stbtt__find_table(data, start, "GPOS");
    if (!cmap || !info->head || !info->hhea || !info->hmtx || !stbtt__find_table(data, start, "CFF2")) return 0;
    stbtt_uint32 maxp = stbtt__find_table(data, start, "maxp");
    info->numGlyphs = maxp ? ttUSHORT(data + maxp + 4) : 0xffff;
    info->svg = -1;
    for (stbtt_int32 i = 0, n = ttUSHORT(data + cmap + 2); i < n; ++i) {
        stbtt_uint32 record = cmap + 4 + 8 * i;
        stbtt_uint16 platform = ttUSHORT(data + record), encoding = ttUSHORT(data + record + 2);
        if ((platform == STBTT_PLATFORM_ID_MICROSOFT &&
             (encoding == STBTT_MS_EID_UNICODE_BMP || encoding == STBTT_MS_EID_UNICODE_FULL)) ||
            platform == STBTT_PLATFORM_ID_UNICODE)
            info->index_map = cmap + ttULONG(data + record + 4);
    }
    if (info->index_map == 0) return 0;
    info->indexToLocFormat = ttUSHORT(data + info->head + 50);
    return 1;
}

static int init_font(stbtt_fontinfo *font, const unsigned char *ttf, int start) {
    if (start < 0) return 0;
    if (stbtt_InitFont(font, ttf, start)) return 1;
    /* stb finds glyf or CFF before CFF2, as font.rs does. */
    return !stbtt__find_table((unsigned char *)ttf, start, "glyf") &&
           !stbtt__find_table((unsigned char *)ttf, start, "CFF ") && init_cff2(font, (unsigned char *)ttf, start);
}

/* A CFF face without outlines of its own would have stb run its charstrings. */
static int init_face(stbtt_fontinfo *font, const Face *face) {
    return init_font(font, face->ttf, face->start) && (font->glyf || face->outline);
}

/* Characters that draw nothing by design, so blank even when no font has
   them: Unicode's space separators (Zs), the line and paragraph separators,
   and the blank Braille pattern that TUIs use as an empty dot graph. */
static int is_blank(uint32_t cp) {
    return cp == 0x20 || cp == 0xa0 || cp == 0x1680 || (cp >= 0x2000 && cp <= 0x200a) || cp == 0x2028 ||
           cp == 0x2029 || cp == 0x202f || cp == 0x205f || cp == 0x2800 || cp == 0x3000;
}

/* An outlined box for a character neither font has, inset in its cells. */
static void paint_tofu(Canvas *cv, int x, int y, int span, int cell_w, int cell_h,
                       uint8_t r, uint8_t g, uint8_t b) {
    int t = cell_w / 12 < 1 ? 1 : cell_w / 12;
    int x0 = x + cell_w / 6, x1 = x + span - cell_w / 6;
    int y0 = y + cell_h / 6, y1 = y + cell_h - cell_h / 6;
    if (x1 - x0 <= 2 * t || y1 - y0 <= 2 * t) {
        termshot_fill_rect(cv, x0, y0, x1, y1, r, g, b);
        return;
    }
    termshot_fill_rect(cv, x0, y0, x1, y0 + t, r, g, b);
    termshot_fill_rect(cv, x0, y1 - t, x1, y1, r, g, b);
    termshot_fill_rect(cv, x0, y0 + t, x0 + t, y1 - t, r, g, b);
    termshot_fill_rect(cv, x1 - t, y0 + t, x1, y1 - t, r, g, b);
}

typedef struct {
    int adv, cell_w, cell_h, body, baseline;
    float scale, italic_pivot;
} CellMetrics;

static int cell_metrics(const stbtt_fontinfo *font, double font_px, CellMetrics *m) {
    int ascent, descent, line_gap;
    stbtt_GetFontVMetrics(font, &ascent, &descent, &line_gap);
    int adv = 0, lsb = 0;
    stbtt_GetCodepointHMetrics(font, 'M', &adv, &lsb);
    if (adv <= 0 || ascent <= descent) {
        fprintf(stderr, "termshot: font metrics unusable\n");
        return 0;
    }
    float scale = stbtt_ScaleForPixelHeight(font, (float)font_px);
    int cell_w = (int)(adv * scale + 0.5f);
    if (cell_w < 1) cell_w = 1;
    scale = (float)cell_w / (float)adv;
    int body = (int)((ascent - descent) * scale + 0.5f);
    int gap = (int)(line_gap * scale + 0.5f);
    if (gap < 0) gap = 0;
    int cell_h = body + gap;
    if (cell_h < 1) cell_h = 1;
    int baseline = (int)(ascent * scale + 0.5f) + (cell_h - body) / 2;
    *m = (CellMetrics){adv, cell_w, cell_h, body, baseline, scale, (ascent + descent) * scale / 2};
    return 1;
}

/* Uses the exact same metrics as the renderer, including custom fonts. */
int draw_cell_size(const unsigned char *ttf, int ttf_start, double px, int *w, int *h) {
    stbtt_fontinfo font;
    CellMetrics m;
    if (!init_font(&font, ttf, ttf_start) || !cell_metrics(&font, px, &m)) return 0;
    *w = m.cell_w;
    *h = m.cell_h;
    return 1;
}

typedef struct {
    const unsigned char *pixels;
    uint32_t width, height;
    int64_t x, y, w, h, clip_top, clip_bottom, clip_left, clip_right;
    /* The source rectangle sampled: the crop, inside width x height. */
    uint32_t src_x, src_y, src_w, src_h;
    int32_t z;
} ImageView;

/* As ImageView in src/graphics.rs, which asserts the same size. */
_Static_assert(sizeof(ImageView) == 104, "ImageView ABI must match the Rust side");

/* kitty's three image layers, by z-index: under the cell backgrounds that are
   not the default (z below INT32_MIN / 2), over every background but under
   the text (other negative z), and over the text. */
enum { LAYER_BELOW, LAYER_UNDER_TEXT, LAYER_OVER_TEXT };
static int image_layer(int32_t z) {
    return z < INT32_MIN / 2 ? LAYER_BELOW : z < 0 ? LAYER_UNDER_TEXT : LAYER_OVER_TEXT;
}

/* The cells drawn as a box because a font maps the character to an empty
   glyph, as color bitmap fonts do: how many, and the first one, its
   character, and which fonts did. The caller says so; draw.c stays quiet.
   As EmptyGlyphs in src/main.rs. */
#define EMPTY_IN_FONT 1
#define EMPTY_IN_FALLBACK 2
typedef struct {
    uint32_t cp, fonts;
    int32_t col, row;
    size_t cells;
} EmptyGlyphs;

/* Whether the cell's background is the default one, which shows an image of
   LAYER_BELOW through it. */
static int clear_background(const Cell *cell) { return !(cell->attrs & ATTR_OPAQUE); }

/* Rust validates dimensions and owns each RGBA buffer. Clip before looping,
   and use integer nearest-neighbor sampling for reproducible screenshots.
   Paints the images of one layer, in their order. With cells, of cell_w x
   cell_h pixels and cv->w / cell_w to a row, only over clear backgrounds. */
static void paint_image_rows(Canvas *cv, const ImageView *images, size_t count, int layer, const Cell *cells,
                             int cell_w, int cell_h, int64_t top, int64_t bottom) {
    for (size_t i = 0; i < count; i++) {
        const ImageView *im = &images[i];
        if (image_layer(im->z) != layer) continue;
        int64_t x0 = im->x > im->clip_left ? im->x : im->clip_left;
        if (x0 < 0) x0 = 0;
        int64_t y0 = im->y > im->clip_top ? im->y : im->clip_top;
        if (y0 < top) y0 = top;
        int64_t x1 = im->x + im->w;
        int64_t y1 = im->y + im->h;
        if (x1 > im->clip_right) x1 = im->clip_right;
        if (x1 > cv->w) x1 = cv->w;
        if (y1 > im->clip_bottom) y1 = im->clip_bottom;
        if (y1 > bottom) y1 = bottom;
        if (x0 >= x1) continue;
        /* The source column of x is src_x + (x - im->x) * src_w / w: its
           quotient and remainder, stepped from x0 a column at a time. */
        int64_t first = (x0 - im->x) * (int64_t)im->src_w, step = (int64_t)im->src_w / im->w,
                carry = (int64_t)im->src_w % im->w;
        for (int64_t y = y0; y < y1; y++) {
            size_t sy = im->src_y + (size_t)((y - im->y) * im->src_h / im->h);
            const Cell *row = cells ? cells + (size_t)(y / cell_h) * (size_t)(cv->w / cell_w) : NULL;
            const unsigned char *line = im->pixels + (sy * im->width + im->src_x) * 4;
            unsigned char *dst = cv->px + (size_t)y * cv->stride + (size_t)x0 * BPP;
            int64_t sx = first / im->w, rem = first % im->w, col = x0 / cell_w, sub = x0 % cell_w;
            for (int64_t x = x0; x < x1; x++, dst += BPP) {
                if (!row || clear_background(&row[col])) {
                    const unsigned char *src = line + (size_t)sx * 4;
                    unsigned a = src[3];
                    /* At 255 and 0 the blend below is the source and the
                       destination exactly. */
                    if (a == 255) {
                        dst[0] = src[0];
                        dst[1] = src[1];
                        dst[2] = src[2];
                    } else if (a) {
                        for (int c = 0; c < 3; c++)
                            dst[c] = (unsigned char)((src[c] * a + dst[c] * (255 - a) + 127) / 255);
                    }
                }
                sx += step;
                rem += carry;
                if (rem >= im->w) {
                    sx++;
                    rem -= im->w;
                }
                if (++sub == cell_w) {
                    sub = 0;
                    col++;
                }
            }
        }
    }
}

static void paint_images(Canvas *cv, const ImageView *images, size_t count, int layer, const Cell *cells,
                         int cell_w, int cell_h) {
    paint_image_rows(cv, images, count, layer, cells, cell_w, cell_h, 0, cv->h);
}

/* What is painted under the text, a row of cells at a time, so that the text
   is painted while its rows are still in the cache: the cell backgrounds,
   then the images below them, which show only through the default ones, and
   those over every background. A row is painted before anything over it
   (the row's own cells, or a glyph or mark reaching down into it), so each
   pixel is painted in the same order as if every row were painted first.
   The whole backdrop is painted at once instead, as it was before rows:
   - for a raster under BACKDROP_ROW_BYTES, which stays in the last-level
     cache anyway. Rows gained nothing at 2200x1440 (9.5 MB) on either host
     measured, and on macOS they cost 1-2% there, but up to 8% faster on
     Linux at 61-67 MB (docs/performance.md);
   - with more than BACKDROP_ROW_IMAGES images under the text (Unicode
     placeholders make one a run), since each row looks at every image. */
#ifndef BACKDROP_ROW_BYTES
#define BACKDROP_ROW_BYTES ((size_t)16 << 20)
#endif
#define BACKDROP_ROW_IMAGES 64
typedef struct {
    const Cell *cells;
    int cols, rows, cell_w, cell_h, done; /* rows painted */
    const ImageView *images;
    size_t image_count;
    int whole; /* paint every row at the first call */
    double ms;
} Backdrop;

static Backdrop backdrop_for(const Canvas *cv, const Cell *cells, int cols, int rows, int cell_w, int cell_h,
                             const ImageView *images, size_t image_count) {
    size_t under = 0;
    for (size_t i = 0; i < image_count; i++) under += image_layer(images[i].z) != LAYER_OVER_TEXT;
    int whole = under > BACKDROP_ROW_IMAGES || cv->stride * (size_t)cv->h < BACKDROP_ROW_BYTES;
    return (Backdrop){cells, cols, rows, cell_w, cell_h, 0, images, image_count, whole, 0};
}

/* Paint the backdrop of every row of cells above pixel row y. */
static void backdrop_through(Canvas *cv, Backdrop *bd, int64_t y) {
    if (bd->done >= bd->rows || y <= (int64_t)bd->done * bd->cell_h) return;
    double tick = now_ms();
    int64_t last = bd->whole ? bd->rows : (y + bd->cell_h - 1) / bd->cell_h;
    if (last > bd->rows) last = bd->rows;
    for (int r = bd->done; r < last; r++) {
        int top = r * bd->cell_h;
        uint8_t *scanline = cv->filtered + (size_t)top * cv->stride;
        scanline[0] = 0;
        for (int c = 0; c < bd->cols; c++) {
            const Cell *cell = &bd->cells[(size_t)r * bd->cols + c];
            termshot_fill_rect(cv, c * bd->cell_w, top, (c + 1) * bd->cell_w, top + 1, cell->br, cell->bg, cell->bb);
        }
        for (int dy = 1; dy < bd->cell_h; dy++) {
            memcpy(cv->filtered + (size_t)(top + dy) * cv->stride, scanline, cv->stride);
        }
        if (bd->whole) continue;
        paint_image_rows(cv, bd->images, bd->image_count, LAYER_BELOW, bd->cells, bd->cell_w, bd->cell_h, top,
                         top + bd->cell_h);
        paint_image_rows(cv, bd->images, bd->image_count, LAYER_UNDER_TEXT, NULL, bd->cell_w, bd->cell_h, top,
                         top + bd->cell_h);
    }
    if (bd->whole) {
        paint_images(cv, bd->images, bd->image_count, LAYER_BELOW, bd->cells, bd->cell_w, bd->cell_h);
        paint_images(cv, bd->images, bd->image_count, LAYER_UNDER_TEXT, NULL, bd->cell_w, bd->cell_h);
    }
    bd->done = (int)last;
    bd->ms += now_ms() - tick;
}

/* Free what the glyph pass holds when it fails: memory ran out, or the box
   painter failed. Says why; returns 2. */
static int paint_failed(Canvas *cv, Glyph *cache, Outline *scratch, const char *why) {
    for (int k = 0; k < GLYPH_CACHE_SIZE; k++) free(cache[k].bitmap);
    termshot_geometry_free(cv->geometry);
    free(scratch->v);
    free(cv->filtered);
    cv->px = NULL;
    fprintf(stderr, "termshot: %s\n", why);
    return 2;
}

/* What finding a glyph needs: both fonts (fallback NULL when there is none),
   their scales, the cache and outline scratch, and the profile's counters. */
typedef struct {
    const stbtt_fontinfo *font, *fallback;
    const Face *font_face, *fallback_face;
    float scale, fallback_scale, italic_pivot;
    Glyph *cache;
    Outline *scratch;
    size_t glyphs, cache_hits, evictions, missing, fallback_lookups, fallback_glyphs;
} Glyphs;

/* The cached glyph of cp, rasterized on a miss, from the font or else the
   fallback; span is the width of its cells in pixels. A mark is placed as its
   font places it, never centered or shrunk. NULL when memory runs out. */
static Glyph *find_glyph(Glyphs *g, uint32_t cp, int wide, int italic, int mark, int span) {
    /* An italic glyph has its own slot, so mixed text doesn't evict. */
    Glyph *entry = &g->cache[(cp ^ (italic ? GLYPH_CACHE_SIZE / 2 : 0)) % GLYPH_CACHE_SIZE];
    if (entry->valid && entry->cp == cp && entry->wide == wide && entry->italic == italic && entry->mark == mark) {
        g->cache_hits++;
        return entry;
    }
    g->evictions += entry->valid;
    free(entry->bitmap);
    *entry = (Glyph){.cp = cp, .valid = 1, .wide = wide, .italic = italic, .mark = mark};
    const stbtt_fontinfo *face = g->font;
    const Face *source = g->font_face;
    float s = g->scale;
    /* A glyph with no outline counts as missing unless the
       character is blank by design: color emoji fonts (sbix,
       CBDT) map characters to empty glyphs and draw them from
       bitmaps, which stb_truetype cannot. */
    int blank = is_blank(cp);
    int glyph = stbtt_FindGlyphIndex(g->font, (int)cp), hollow = 0;
    /* bare is 1 for an empty glyph, -1 when memory ran out. */
    int bare = glyph != 0 && !blank ? glyph_empty(g->font, g->font_face, glyph, g->scratch) : 0;
    if (bare > 0) {
        glyph = 0;
        hollow = EMPTY_IN_FONT;
    }
    if (glyph == 0 && g->fallback && bare >= 0) {
        g->fallback_lookups++;
        face = g->fallback;
        source = g->fallback_face;
        s = g->fallback_scale;
        glyph = stbtt_FindGlyphIndex(g->fallback, (int)cp);
        bare = glyph != 0 && !blank ? glyph_empty(g->fallback, g->fallback_face, glyph, g->scratch) : 0;
        if (bare > 0) {
            glyph = 0;
            hollow |= EMPTY_IN_FALLBACK;
        }
    }
    if (bare < 0) return NULL;
    entry->missing = glyph == 0;
    if (!mark) g->missing += entry->missing;
    entry->empty = hollow;
    if (glyph != 0) {
        /* The primary font's narrow glyphs sit where the font puts
           them. Wide and fallback glyphs are centered, and a
           fallback glyph too wide for its cells is shrunk. A
           mark is neither (paint_marks places it). */
        int glyph_adv, glyph_lsb;
        stbtt_GetGlyphHMetrics(face, glyph, &glyph_adv, &glyph_lsb);
        float advance = glyph_adv * s;
        if (!mark && face == g->fallback && advance > span) {
            s *= span / advance;
            advance = (float)span;
        }
        entry->advance = advance;
        if (!mark && (wide || face == g->fallback)) entry->shift = (int)floorf((span - advance) / 2 + 0.5f);
        int ix1, iy1;
        /* stb's outline, to free; or the CFF face's, in scratch. */
        stbtt_vertex *shape = NULL, *outline = NULL;
        int verts = 0;
        if (source->outline) {
            verts = cff_outline(g->scratch, source, glyph);
            if (verts < 0) return NULL;
            outline = g->scratch->v;
        } else if (italic) {
            verts = stbtt_GetGlyphShape(face, glyph, &shape);
            outline = shape;
        }
        if (italic) {
            slant_outline(outline, verts, g->italic_pivot / s, s, &entry->ix0, &entry->iy0, &ix1, &iy1);
            g->scratch->face = NULL; /* now slanted */
        } else if (source->outline) {
            /* As stbtt_GetGlyphBitmapBox rounds the glyph's box. */
            const int *box = g->scratch->box;
            entry->ix0 = (int)floorf(box[0] * s);
            entry->iy0 = (int)floorf(-box[3] * s);
            ix1 = (int)ceilf(box[2] * s);
            iy1 = (int)ceilf(-box[1] * s);
            if (verts == 0) entry->ix0 = entry->iy0 = ix1 = iy1 = 0;
        } else {
            stbtt_GetGlyphBitmapBox(face, glyph, s, s, &entry->ix0, &entry->iy0, &ix1, &iy1);
        }
        entry->w = ix1 - entry->ix0;
        entry->h = iy1 - entry->iy0;
        if (entry->w > 0 && entry->h > 0) {
            entry->bitmap = (unsigned char *)malloc((size_t)entry->w * entry->h);
            if (!entry->bitmap) {
                stbtt_FreeShape(face, shape);
                return NULL;
            }
            if (outline) {
                stbtt__bitmap out = {entry->w, entry->h, entry->w, entry->bitmap};
                stbtt_Rasterize(&out, 0.35f, outline, verts, s, s, 0, 0, entry->ix0, entry->iy0, 1, face->userdata);
            } else {
                stbtt_MakeGlyphBitmap(face, entry->bitmap, entry->w, entry->h, entry->w, s, s, glyph);
            }
            g->glyphs++;
            g->fallback_glyphs += face == g->fallback;
        }
        stbtt_FreeShape(face, shape);
    }
    return entry;
}

/* Default_Ignorable_Code_Point, past U+00AD: joiners, direction marks,
   variation selectors, fillers and tags, which draw nothing even when a font
   has a glyph for them. Most are zero width, and so marks; the Hangul fillers
   U+115F, U+3164 and U+FFA0 take cells of their own. */
static int is_ignorable(uint32_t cp) {
    return cp == 0x034f || cp == 0x061c || (cp >= 0x115f && cp <= 0x1160) || (cp >= 0x17b4 && cp <= 0x17b5) ||
           (cp >= 0x180b && cp <= 0x180f) || (cp >= 0x200b && cp <= 0x200f) || (cp >= 0x202a && cp <= 0x202e) ||
           (cp >= 0x2060 && cp <= 0x206f) || cp == 0x3164 || (cp >= 0xfe00 && cp <= 0xfe0f) || cp == 0xfeff ||
           cp == 0xffa0 || (cp >= 0xfff0 && cp <= 0xfff8) || (cp >= 0x1bca0 && cp <= 0x1bca3) ||
           (cp >= 0x1d173 && cp <= 0x1d17a) || (cp >= 0xe0000 && cp <= 0xe0fff);
}

/* Draw a cell's combining marks over its character, in the cell's colours
   and attributes, from the font or else the fallback; a mark neither has is
   left out. There is no shaping (no GPOS anchors), so a mark goes where its
   own outline puts it. Most fonts draw a mark to the left of its origin, with
   no advance, to overlay the character before it: that mark is drawn from
   where the character ends (base_x plus its advance). A mark drawn to the
   right of its origin, as in right-to-left fonts, is centered over the
   character instead. y is the baseline. Adds the time spent finding glyphs
   and blending them to glyph_ms and blend_ms. Returns 0, or 2 when memory
   runs out. */
static int paint_marks(Canvas *cv, Backdrop *backdrop, Glyphs *g, const CellMarks *marks, const Cell *cell, int base_x,
                       float base_advance, int y, double *glyph_ms, double *blend_ms) {
    int italic = (cell->attrs & ATTR_ITALIC) != 0;
    for (int k = 0; k < MAX_MARKS && marks->marks[k]; k++) {
        uint32_t cp = marks->marks[k];
        if (is_ignorable(cp)) continue;
        double tick = now_ms();
        Glyph *mark = find_glyph(g, cp, 0, italic, 1, 0);
        *glyph_ms += now_ms() - tick;
        if (!mark) return 2;
        if (!mark->bitmap) continue;
        backdrop_through(cv, backdrop, (int64_t)y + mark->iy0 + mark->h);
        tick = now_ms();
        int dx = 2 * mark->ix0 + mark->w < 0 ? base_x + (int)floorf(base_advance + 0.5f) + mark->ix0
                                             : base_x + (int)floorf((base_advance - (float)mark->w) / 2 + 0.5f);
        int dy = y + mark->iy0;
        blend(cv, dx, dy, mark->bitmap, mark->w, mark->h, cell->fr, cell->fg, cell->fb);
        if (cell->attrs & ATTR_BOLD) blend(cv, dx + 1, dy, mark->bitmap, mark->w, mark->h, cell->fr, cell->fg, cell->fb);
        *blend_ms += now_ms() - tick;
    }
    return 0;
}

/* Paint cells with the face font and write a PNG. fallback_face, or NULL,
   supplies the characters font_face lacks; characters neither has are drawn
   as an outlined box. marks, mark_count long and sorted by cell, gives the
   cells' combining marks, drawn over them (paint_marks); NULL when none.
   The canvas and cache are local; timing hooks use
   thread-local state so concurrent renders remain independent.
   verbose prints the cell and image size to stderr. empty, if not NULL,
   is filled in as EmptyGlyphs says.
   Returns 0; 1 for an unusable font; 2 when the image is too large, memory
   runs out or the box painter fails; 3 when the PNG cannot be written. */
int draw_png_images(const Cell *cells, const CellMarks *marks, size_t mark_count, int cols, int rows,
                    const Face *font_face, const Face *fallback_face,
                    double font_px, const char *out_path, int verbose, const ImageView *images,
                    size_t image_count, EmptyGlyphs *empty) {
    if (empty) *empty = (EmptyGlyphs){0};
    profiling = getenv("TERMSHOT_PROFILE") != NULL;
    termshot_deflate_profiling(profiling);
    double started = now_ms();
    stbtt_fontinfo font, fallback;
    if (!init_face(&font, font_face) || (fallback_face && !init_face(&fallback, fallback_face))) {
        fprintf(stderr, "termshot: font init failed\n");
        return 1;
    }

    CellMetrics metrics;
    if (!cell_metrics(&font, font_px, &metrics)) return 1;
    int adv = metrics.adv, cell_w = metrics.cell_w, cell_h = metrics.cell_h;
    int body = metrics.body, baseline = metrics.baseline;
    float scale = metrics.scale;
    /* Italic slants around the middle of the body, in pixels above the
       baseline, for both fonts. */
    float italic_pivot = metrics.italic_pivot;
    /* The fallback is sized to the same ascent-to-descent height and shares
       the baseline. */
    float fallback_scale = fallback_face ? stbtt_ScaleForPixelHeight(&fallback, (float)body) : 0;
    long long width = (long long)cols * cell_w;
    long long height = (long long)rows * cell_h;
    if (verbose) {
        fprintf(stderr, "advance %d units scale %.5f cell %dx%d baseline %d image %lldx%lld\n",
                adv, scale, cell_w, cell_h, baseline, width, height);
    }
    /* stb_image_write sizes its buffers with int: (width*BPP+1)*height must not wrap. */
    if (width * height > MAX_PIXELS) {
        fprintf(stderr, "termshot: image %lldx%lld is over %d pixels; lower px, cols or rows\n",
                width, height, MAX_PIXELS);
        return 2;
    }

    double font_setup = now_ms();
    /* One leading zero per scanline is PNG's None filter. Paint directly into
       the compressor input instead of copying a second full image later. */
    size_t stride = (size_t)width * BPP + 1;
    Canvas canvas = {.filtered = (uint8_t *)malloc(stride * (size_t)height),
                     .w = (int)width, .h = (int)height, .stride = stride};
    Canvas *cv = &canvas;
    if (!cv->filtered) {
        fprintf(stderr, "termshot: out of memory for a %lldx%lld image\n", width, height);
        return 2;
    }

    cv->px = cv->filtered + 1;
    /* NULL only means nothing is cached; the pixels are the same. */
    cv->geometry = termshot_geometry_new(1);
    double allocated = now_ms();
    /* The backgrounds and the images under the text are painted a row of
       cells ahead of the text over them (backdrop_through); background_ms
       is their time, and foreground_ms the rest. */
    Backdrop backdrop = backdrop_for(cv, cells, cols, rows, cell_w, cell_h, images, image_count);
    double background = now_ms();
    double geometry_ms = 0, glyph_ms = 0, blend_ms = 0;
    Glyph cache[GLYPH_CACHE_SIZE] = {0};
    Outline scratch = {0};
    Glyphs g = {&font, fallback_face ? &fallback : NULL, font_face, fallback_face, scale, fallback_scale,
                italic_pivot, cache, &scratch, 0, 0, 0, 0, 0, 0};
    size_t next_mark = 0;
    for (int r = 0; r < rows; r++) {
        backdrop_through(cv, &backdrop, (int64_t)(r + 1) * cell_h);
        for (int c = 0; c < cols; c++) {
            size_t index = (size_t)r * (size_t)cols + (size_t)c;
            const Cell *cell = &cells[index];
            uint32_t cp = cell->ch;
            /* The cell's marks, if any; the list is sorted by cell. */
            const CellMarks *cell_marks = NULL;
            while (next_mark < mark_count && marks[next_mark].cell < index) next_mark++;
            if (next_mark < mark_count && marks[next_mark].cell == index) cell_marks = &marks[next_mark];
            if ((cp == 0 || cp == ' ') && !cell_marks) continue;
            int wide = (cell->attrs & ATTR_WIDE) != 0;
            int span = wide ? 2 * cell_w : cell_w;
            /* Where the character starts and how far it advances, for its
               marks: its cells, unless a glyph says otherwise. */
            int base_x = c * cell_w;
            float base_advance = (float)span;
            /* A Hangul filler draws nothing, though its marks still do. */
            if (cp != 0 && cp != ' ' && !is_ignorable(cp)) {
                double tick = now_ms();
                /* Box drawing and blocks, painted in Rust: one call per cell. */
                int geometry = cp >= 0x2500 && cp <= 0x259F
                                   ? termshot_paint_geometry(cv, c, r, cell_w, cell_h, cp, cell->attrs & ATTR_BOLD,
                                                             cell->fr, cell->fg, cell->fb)
                                   : 0;
                geometry_ms += now_ms() - tick;
                if (geometry < 0) return paint_failed(cv, cache, &scratch, "box drawing failed");
                if (!geometry) {
                    tick = now_ms();
                    Glyph *entry = find_glyph(&g, cp, wide, (cell->attrs & ATTR_ITALIC) != 0, 0, span);
                    if (!entry) return paint_failed(cv, cache, &scratch, "glyph allocation failed");
                    glyph_ms += now_ms() - tick;
                    tick = now_ms();
                    double ahead = backdrop.ms;
                    if (entry->missing && !is_blank(cp)) {
                        if (entry->empty && empty && empty->cells++ == 0) {
                            *empty = (EmptyGlyphs){.cp = cp, .fonts = (uint32_t)entry->empty, .col = c, .row = r, .cells = 1};
                        }
                        paint_tofu(cv, c * cell_w, r * cell_h, span, cell_w, cell_h, cell->fr, cell->fg, cell->fb);
                    } else if (entry->bitmap) {
                        const unsigned char *bm = entry->bitmap;
                        int gw = entry->w, gh = entry->h;
                        int dx = c * cell_w + entry->shift + entry->ix0;
                        int dy = r * cell_h + baseline + entry->iy0;
                        backdrop_through(cv, &backdrop, (int64_t)dy + gh);
                        blend(cv, dx, dy, bm, gw, gh, cell->fr, cell->fg, cell->fb);
                        if (cell->attrs & ATTR_BOLD) blend(cv, dx + 1, dy, bm, gw, gh, cell->fr, cell->fg, cell->fb);
                    }
                    if (!entry->missing) {
                        base_x += entry->shift;
                        base_advance = entry->advance;
                    }
                    blend_ms += now_ms() - tick - (backdrop.ms - ahead);
                }
            }
            if (cell_marks &&
                paint_marks(cv, &backdrop, &g, cell_marks, cell, base_x, base_advance, r * cell_h + baseline, &glyph_ms,
                            &blend_ms) != 0)
                return paint_failed(cv, cache, &scratch, "glyph allocation failed");
        }
    }

    backdrop_through(cv, &backdrop, (int64_t)rows * cell_h);
    /* Underlines and strike-through, over the glyphs, as thick as box-drawing
       strokes and kept inside the cell. */
    int line_t = cell_w / 12 < 1 ? 1 : cell_w / 12;
    int under_y = baseline + (cell_h - baseline) / 3;
    if (under_y > cell_h - line_t) under_y = cell_h - line_t;
    int double_y = under_y + 3 * line_t <= cell_h ? under_y : cell_h - 3 * line_t;
    if (double_y < 0) double_y = 0;
    int strike_y = baseline - baseline * 3 / 10;
    for (int r = 0; r < rows; r++) {
        for (int c = 0; c < cols; c++) {
            const Cell *cell = &cells[r * cols + c];
            if (!(cell->attrs & (ATTR_UNDERLINE | ATTR_DOUBLE_UNDERLINE | ATTR_STRIKE))) continue;
            int x0 = c * cell_w, x1 = x0 + cell_w, y = r * cell_h;
            if (cell->attrs & ATTR_DOUBLE_UNDERLINE) {
                termshot_fill_rect(cv, x0, y + double_y, x1, y + double_y + line_t, cell->fr, cell->fg, cell->fb);
                termshot_fill_rect(cv, x0, y + double_y + 2 * line_t, x1, y + double_y + 3 * line_t, cell->fr, cell->fg, cell->fb);
            } else if (cell->attrs & ATTR_UNDERLINE) {
                termshot_fill_rect(cv, x0, y + under_y, x1, y + under_y + line_t, cell->fr, cell->fg, cell->fb);
            }
            if (cell->attrs & ATTR_STRIKE) {
                termshot_fill_rect(cv, x0, y + strike_y, x1, y + strike_y + line_t, cell->fr, cell->fg, cell->fb);
            }
        }
    }

    /* termshot_fill_rect has no result of its own: a fill that failed says
       so here, before an incomplete image is written. */
    if (termshot_paint_failed()) return paint_failed(cv, cache, &scratch, "box drawing failed");
    for (int k = 0; k < GLYPH_CACHE_SIZE; k++) free(cache[k].bitmap);
    GeometryStats stamps;
    termshot_geometry_stats(cv->geometry, &stamps);
    termshot_geometry_free(cv->geometry);
    cv->geometry = NULL;
    free(scratch.v);
    paint_images(cv, images, image_count, LAYER_OVER_TEXT, NULL, cell_w, cell_h);
    double foreground = now_ms();
    int png_len = 0;
    STBIW_PNG_PROFILE(0);
    unsigned char *png = stbiw__write_png_from_filtered(cv->filtered, cv->w, cv->h, BPP, &png_len);
    double encoded = now_ms();
    int ok = 0, encode_failed = !png;
    /* stb returns NULL only when an allocation failed, its own or the compressor's. */
    if (encode_failed) {
        fprintf(stderr, "termshot: out of memory encoding a %lldx%lld PNG\n", width, height);
    } else {
        FILE *out = fopen(out_path, "wb");
        if (out) {
            ok = fwrite(png, 1, (size_t)png_len, out) == (size_t)png_len;
            if (fclose(out) != 0) ok = 0;
        }
        free(png);
    }
    double written = now_ms();
    free(cv->filtered);
    cv->px = NULL;
    if (profiling) {
        DeflateTimings deflate;
        termshot_deflate_timings(&deflate);
        fprintf(stderr, "termshot-profile {\"deflate_allocate_ms\":%.6f,\"deflate_match_emit_ms\":%.6f,\"deflate_finalize_ms\":%.6f,\"deflate_checksum_ms\":%.6f,\"font_setup_ms\":%.6f,\"allocate_ms\":%.6f,\"background_ms\":%.6f,\"foreground_ms\":%.6f,\"geometry_ms\":%.6f,\"glyph_ms\":%.6f,\"blend_ms\":%.6f,\"png_filter_ms\":%.6f,\"png_deflate_ms\":%.6f,\"png_pack_ms\":%.6f,\"png_encode_ms\":%.6f,\"output_write_ms\":%.6f,\"cleanup_ms\":%.6f,\"geometry_cache_hits\":%zu,\"geometry_cache_misses\":%zu,\"geometry_cache_uncached\":%zu,\"geometry_cache_bytes\":%zu,\"glyph_rasterizations\":%zu,\"glyph_cache_hits\":%zu,\"glyph_cache_evictions\":%zu,\"glyph_missing\":%zu,\"fallback_lookups\":%zu,\"fallback_rasterizations\":%zu,\"png_bytes\":%d,\"pixel_bytes\":%zu}\n",
            deflate.allocate_ms, deflate.match_emit_ms, deflate.finalize_ms, deflate.checksum_ms,
            font_setup - started, allocated - font_setup,
            background - allocated + backdrop.ms, foreground - background - backdrop.ms, geometry_ms, glyph_ms, blend_ms,
            png_marks[1] - png_marks[0], png_marks[2] - png_marks[1], png_marks[3] - png_marks[2],
            encoded - foreground, written - encoded, now_ms() - written, stamps.hits, stamps.misses, stamps.uncached, stamps.bytes, g.glyphs, g.cache_hits, g.evictions, g.missing, g.fallback_lookups, g.fallback_glyphs, png_len, (size_t)(width * height * BPP));
    }
    if (encode_failed) return 2;
    if (!ok) {
        fprintf(stderr, "termshot: png write failed: %s\n", out_path);
        return 3;
    }
    return 0;
}

/* The cell-only entry point for the C rasterizer tests, for TrueType faces. */
int draw_png(const Cell *cells, int cols, int rows, const unsigned char *ttf, int ttf_start,
             const unsigned char *fallback_ttf, int fallback_start, double font_px, const char *out_path,
             int verbose) {
    Face font = {ttf, ttf_start, NULL, NULL}, fallback = {fallback_ttf, fallback_start, NULL, NULL};
    return draw_png_images(cells, NULL, 0, cols, rows, &font, fallback_ttf ? &fallback : NULL, font_px, out_path, verbose,
                           NULL, 0, NULL);
}
