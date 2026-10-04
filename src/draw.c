/* Headless cell-grid PNG. Same file on macOS and Linux.
   Raster is stb_truetype (public domain). PNG is stb_image_write (public domain,
   deflate included). No Core Text, FreeType, window, or distro package.
   Box drawing and blocks are geometry, so the joints meet at an integer cell
   size; that painter is Rust (src/geometry.rs), and so are the image layers
   and the backdrop under the text (src/composite.rs), the text itself
   (src/glyphs.rs, which calls back into stb here for cmaps, metrics and
   rasterizing) and the compressor. Font setup, cell metrics, the order of
   the passes and the PNG write stay here. */

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

/* As Face in src/glyphs.rs (and src/font.rs, which makes it). */
_Static_assert(sizeof(Face) == 32, "Face ABI must match the Rust side");
_Static_assert(sizeof(stbtt_vertex) == 14, "stbtt_vertex ABI must match cff::Vertex");

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

   The text (src/glyphs.rs) paints box drawing, underlines and the boxes of
   missing glyphs through these from Rust; draw.c declares the two for the
   C harnesses (tests/boxes.c, bench/c-vs-rust/geometry.c).

   termshot_paint_failed: nonzero if a painter failed (a bug) on this thread
   since termshot_geometry_new; the render then fails. */
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

/* As ImageView in src/composite.rs, which asserts the same size. */
_Static_assert(sizeof(ImageView) == 104, "ImageView ABI must match the Rust side");

/* kitty's three image layers, by z-index: under the cell backgrounds that are
   not the default (z below INT32_MIN / 2), over every background but under
   the text (other negative z), and over the text. As LAYER_* in
   src/composite.rs, which decides an image's layer. */
enum { LAYER_BELOW, LAYER_UNDER_TEXT, LAYER_OVER_TEXT };

/* The cells drawn as a box because a font maps the character to an empty
   glyph, as color bitmap fonts do: how many, and the first one, its
   character, and which fonts did. The caller says so; painting stays quiet.
   As EmptyGlyphs in src/glyphs.rs. */
#define EMPTY_IN_FONT 1
#define EMPTY_IN_FALLBACK 2
typedef struct {
    uint32_t cp, fonts;
    int32_t col, row;
    size_t cells;
} EmptyGlyphs;

/* The images and the backdrop under the text, composited in Rust
   (src/composite.rs), which samples each image nearest-neighbour in
   integers, so screenshots are reproducible.

   The backdrop is what is painted under the text, a row of cells at a time,
   so that the text is painted while its rows are still in the cache: the
   cell backgrounds, then the images below them, which show only through the
   default ones (no ATTR_OPAQUE), and those over every background. A row is
   painted before anything over it (the row's own cells, or a glyph or mark
   reaching down into it), so each pixel is painted in the same order as if
   every row were painted first. The whole backdrop is painted at once
   instead, as it was before rows:
   - for a raster under BACKDROP_ROW_BYTES, which stays in the last-level
     cache anyway. Rows gained nothing at 2200x1440 (9.5 MB) on either host
     measured, and on macOS they cost 1-2% there, but up to 8% faster on
     Linux at 61-67 MB (docs/performance.md);
   - with more than 64 images under the text (Unicode placeholders make one a
     run), since each row looks at every image.

   termshot_backdrop_init: fill in bd for the cells (cols x rows of cell_w x
   cell_h pixels, the whole canvas) and the images, which it keeps.

   termshot_backdrop_through: paint the backdrop of every row of cells above
   pixel row y not painted yet. 1 when it painted, 0 when nothing was due, -1
   if the painter failed (a bug).

   termshot_paint_images: paint the images of one layer, in their order, over
   the whole canvas. 0, or -1 if the painter failed (a bug).

   A failure is also remembered for termshot_paint_failed. Nothing here
   allocates, so nothing here runs out of memory. */
#ifndef BACKDROP_ROW_BYTES
#define BACKDROP_ROW_BYTES ((size_t)16 << 20)
#endif
typedef struct {
    const Cell *cells;
    const ImageView *images;
    size_t image_count;
    int32_t cols, rows, cell_w, cell_h;
    int32_t done; /* rows painted */
    int32_t whole; /* paint every row at the first call */
} Backdrop;

_Static_assert(sizeof(Backdrop) == 48, "Backdrop ABI must match the Rust side");

void termshot_backdrop_init(Backdrop *bd, const Canvas *cv, const Cell *cells, int cols, int rows, int cell_w,
                            int cell_h, const ImageView *images, size_t image_count, size_t row_bytes);
int termshot_backdrop_through(const Canvas *cv, Backdrop *bd, int64_t y);
int termshot_paint_images(const Canvas *cv, const ImageView *images, size_t count, int layer);

/* The text, painted in Rust (src/glyphs.rs): each cell's glyph from the font
   or else the fallback, cached, slanted for italic and doubled for bold; box
   drawing and blocks as geometry; an outlined box for a character neither
   font has; combining marks; then underlines and strike-through. It paints
   the backdrop a row of cells ahead of the text, and all of it by the end.

   stb_truetype stays here, and Rust calls only the stb functions it is
   handed. GlyphFace is one face: stb's font, the Face, its scale, and the
   four stb functions that read glyph outlines, which glyph_face sets for a
   TrueType face only. A CFF or CFF2 face gets NULL for them, so stb never
   runs its charstrings: its outlines come from Face.outline (src/cff.rs)
   and go to stbtt_Rasterize. TextFonts is both faces (fallback.info NULL
   for none), the stb functions any face may be asked (cmap, metrics, the
   rasterizer, freeing a shape), the italic pivot and the baseline. As in
   src/glyphs.rs, which asserts the same sizes.

   termshot_paint_text: paint the text of bd's cells, with marks (mark_count
   long, sorted by cell, or NULL); empty, if not NULL, is filled in as
   EmptyGlyphs says; profiling times the stages into stats. 0, or one of
   TEXT_* with the render unfinished. A panic in the box painter or the
   backdrop is also remembered for termshot_paint_failed. */
typedef struct {
    const stbtt_fontinfo *info;
    const Face *face;
    float scale;
    int (*is_glyph_empty)(const stbtt_fontinfo *info, int glyph);
    int (*get_glyph_shape)(const stbtt_fontinfo *info, int glyph, stbtt_vertex **vertices);
    void (*get_glyph_bitmap_box)(const stbtt_fontinfo *info, int glyph, float scale_x, float scale_y, int *ix0,
                                 int *iy0, int *ix1, int *iy1);
    void (*make_glyph_bitmap)(const stbtt_fontinfo *info, unsigned char *output, int out_w, int out_h,
                              int out_stride, float scale_x, float scale_y, int glyph);
} GlyphFace;

typedef struct {
    GlyphFace font, fallback;
    int (*find_glyph_index)(const stbtt_fontinfo *info, int unicode_codepoint);
    void (*get_glyph_h_metrics)(const stbtt_fontinfo *info, int glyph, int *advance, int *left_side_bearing);
    void (*rasterize)(stbtt__bitmap *result, float flatness_in_pixels, stbtt_vertex *vertices, int num_verts,
                      float scale_x, float scale_y, float shift_x, float shift_y, int x_off, int y_off, int invert,
                      void *userdata);
    void (*free_shape)(const stbtt_fontinfo *info, stbtt_vertex *vertices);
    float italic_pivot; /* the middle of the body, in pixels above the baseline */
    int32_t baseline;
} TextFonts;

/* The time spent painting the backdrop, box drawing, finding glyphs and
   blending them, and the glyph cache's counts, for TERMSHOT_PROFILE. */
typedef struct {
    double backdrop_ms, geometry_ms, glyph_ms, blend_ms;
    size_t glyphs, cache_hits, evictions, missing, fallback_lookups, fallback_glyphs;
} TextStats;

_Static_assert(sizeof(GlyphFace) == 56, "GlyphFace ABI must match the Rust side");
_Static_assert(sizeof(TextFonts) == 152, "TextFonts ABI must match the Rust side");
_Static_assert(sizeof(TextStats) == 80, "TextStats ABI must match the Rust side");

enum { TEXT_OUT_OF_MEMORY = 1, TEXT_BOX_DRAWING = 2, TEXT_PANICKED = 3 };
int termshot_paint_text(const Canvas *cv, Backdrop *bd, const CellMarks *marks, size_t mark_count,
                        const TextFonts *fonts, int profiling, EmptyGlyphs *empty, TextStats *stats);

/* A face for termshot_paint_text, with stb's outline readers for TrueType
   only (init_face has checked that a face without Face.outline has glyf);
   info NULL for none. */
static GlyphFace glyph_face(const stbtt_fontinfo *info, const Face *face, float scale) {
    GlyphFace f = {info, face, scale, NULL, NULL, NULL, NULL};
    if (info && !face->outline && info->glyf) {
        f.is_glyph_empty = stbtt_IsGlyphEmpty;
        f.get_glyph_shape = stbtt_GetGlyphShape;
        f.get_glyph_bitmap_box = stbtt_GetGlyphBitmapBox;
        f.make_glyph_bitmap = stbtt_MakeGlyphBitmap;
    }
    return f;
}

/* Free what the render holds when painting fails: memory ran out, or a
   painter failed. Says why; returns 2. */
static int paint_failed(Canvas *cv, const char *why) {
    termshot_geometry_free(cv->geometry);
    free(cv->filtered);
    cv->px = NULL;
    fprintf(stderr, "termshot: %s\n", why);
    return 2;
}

/* Paint cells with the face font and write a PNG. fallback_face, or NULL,
   supplies the characters font_face lacks; characters neither has are drawn
   as an outlined box. marks, mark_count long and sorted by cell, gives the
   cells' combining marks, drawn over them (src/glyphs.rs); NULL when none.
   The canvas, and the glyph cache in Rust, are the render's own, and the
   timing hooks and fault counters are per thread, so concurrent renders
   remain independent.
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
       cells ahead of the text over them (src/glyphs.rs); background_ms is
       their time, and foreground_ms the rest. */
    Backdrop backdrop;
    termshot_backdrop_init(&backdrop, cv, cells, cols, rows, cell_w, cell_h, images, image_count, BACKDROP_ROW_BYTES);
    double background = now_ms();
    TextFonts fonts = {glyph_face(&font, font_face, scale),
                       glyph_face(fallback_face ? &fallback : NULL, fallback_face, fallback_scale),
                       stbtt_FindGlyphIndex, stbtt_GetGlyphHMetrics, stbtt_Rasterize, stbtt_FreeShape,
                       italic_pivot, baseline};
    TextStats text;
    int painted = termshot_paint_text(cv, &backdrop, marks, mark_count, &fonts, profiling, empty, &text);
    if (painted != 0)
        return paint_failed(cv, painted == TEXT_OUT_OF_MEMORY ? "glyph allocation failed"
                                : painted == TEXT_BOX_DRAWING ? "box drawing failed"
                                                              : "painting failed");

    termshot_paint_images(cv, images, image_count, LAYER_OVER_TEXT);
    /* The fills and the backdrop have no result of their own: a fill or an
       image that failed says so now, before an incomplete image is written. */
    if (termshot_paint_failed()) return paint_failed(cv, "painting failed");
    GeometryStats stamps;
    termshot_geometry_stats(cv->geometry, &stamps);
    termshot_geometry_free(cv->geometry);
    cv->geometry = NULL;
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
            background - allocated + text.backdrop_ms, foreground - background - text.backdrop_ms, text.geometry_ms,
            text.glyph_ms, text.blend_ms,
            png_marks[1] - png_marks[0], png_marks[2] - png_marks[1], png_marks[3] - png_marks[2],
            encoded - foreground, written - encoded, now_ms() - written, stamps.hits, stamps.misses, stamps.uncached, stamps.bytes, text.glyphs, text.cache_hits, text.evictions, text.missing, text.fallback_lookups, text.fallback_glyphs, png_len, (size_t)(width * height * BPP));
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
