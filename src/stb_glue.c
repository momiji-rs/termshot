/* The C that is left: stb_truetype (public domain) and the PNG writer of
   stb_image_write (public domain, locally modified; third_party/stb/CHANGES.md),
   and the glue that reaches into them. Same file on macOS and Linux; no Core
   Text, FreeType, window, or distro package.

   The render itself is Rust (src/render.rs): the canvas, the order of the
   passes, the profile record and the errors. It calls here for two things
   that need stb's own structs:
   - termshot_font_setup: stb's fonts for both faces, the cell metrics, and
     the stb functions the text (src/glyphs.rs) may call, with the four that
     read outlines for a TrueType face only;
   - termshot_png_encode: the PNG of the canvas's filtered scanlines, with
     Rust's compressor (src/deflate.rs) behind STBIW_ZLIB_COMPRESS.
   main.rs asks draw_cell_size for the cell before replaying the log.
   Everything here exists because it touches stb's internals or its types. */

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
/* The PNG's own buffer comes from malloc through Rust (src/render.rs), so
   the fault tests can fail it as they fail the compressor's; the
   compressor's buffer, which stb frees, is malloc's too. */
void *termshot_png_alloc(size_t size);
#define STBIW_MALLOC(size) termshot_png_alloc(size)
#define STBIW_REALLOC(p, size) realloc(p, size)
#define STBIW_FREE(p) free(p)
#include "png_crc.h"
#define STBIW_CRC32 termshot_png_crc
#define STB_IMAGE_WRITE_IMPLEMENTATION
#include "stb_image_write.h"

/* As FontInfo's storage in src/render.rs, which holds two in FontSetup. */
_Static_assert(sizeof(stbtt_fontinfo) == 160 && _Alignof(stbtt_fontinfo) == 8,
               "stbtt_fontinfo must match FontInfoStorage in src/render.rs");

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

/* As CellMetrics in src/render.rs. */
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

/* What the text (src/glyphs.rs) may ask stb, as GlyphFace and TextFonts
   there, which assert the same sizes. GlyphFace is one face: stb's font, the
   Face, its scale, and the four stb functions that read glyph outlines,
   which glyph_face sets for a TrueType face only. A CFF or CFF2 face gets
   NULL for them, so stb never runs its charstrings: its outlines come from
   Face.outline (src/cff.rs) and go to stbtt_Rasterize. TextFonts is both
   faces (fallback.info NULL for none), the stb functions any face may be
   asked (cmap, metrics, the rasterizer, freeing a shape), the italic pivot
   and the baseline. */
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

_Static_assert(sizeof(GlyphFace) == 56, "GlyphFace ABI must match the Rust side");
_Static_assert(sizeof(TextFonts) == 152, "TextFonts ABI must match the Rust side");

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

/* A render's fonts, as FontSetup in src/render.rs, which keeps it in its
   frame: stb's font for each face, the cell metrics of the font, and text,
   which points into font and fallback, so the setup must not move once
   filled in. */
typedef struct {
    stbtt_fontinfo font, fallback;
    TextFonts text;
    CellMetrics metrics;
} FontSetup;

_Static_assert(sizeof(CellMetrics) == 28, "CellMetrics ABI must match the Rust side");
_Static_assert(sizeof(FontSetup) == 504, "FontSetup ABI must match the Rust side");

/* Set up the fonts of a render in s: font_face to draw with, and
   fallback_face, or NULL, for the characters it lacks. 1, having said why,
   when a face can't be used or the font's metrics are unusable; 0 when
   done. */
int termshot_font_setup(const Face *font_face, const Face *fallback_face, double font_px, FontSetup *s) {
    stbtt_fontinfo *font = &s->font, *fallback = &s->fallback;
    if (!init_face(font, font_face) || (fallback_face && !init_face(fallback, fallback_face))) {
        fprintf(stderr, "termshot: font init failed\n");
        return 1;
    }

    CellMetrics metrics;
    if (!cell_metrics(font, font_px, &metrics)) return 1;
    /* The fallback is sized to the same ascent-to-descent height and shares
       the baseline. */
    float fallback_scale = fallback_face ? stbtt_ScaleForPixelHeight(fallback, (float)metrics.body) : 0;
    /* Italic slants around the middle of the body, in pixels above the
       baseline, for both fonts. */
    s->text = (TextFonts){glyph_face(font, font_face, metrics.scale),
                          glyph_face(fallback_face ? fallback : NULL, fallback_face, fallback_scale),
                          stbtt_FindGlyphIndex, stbtt_GetGlyphHMetrics, stbtt_Rasterize, stbtt_FreeShape,
                          metrics.italic_pivot, metrics.baseline};
    s->metrics = metrics;
    return 0;
}

/* The PNG of w x h RGB pixels, h filtered scanlines (a filter byte, then
   the row) stride 3 * w + 1 apart, from stb_image_write: malloc'd, *len
   bytes long, or NULL when an allocation failed, stb's or the compressor's.
   With profiling, marks gets the clock (CLOCK_MONOTONIC, in ms) when it
   began, began compressing, began packing and was done. */
unsigned char *termshot_png_encode(unsigned char *filtered, int w, int h, int profile, double marks[4], int *len) {
    profiling = profile;
    *len = 0;
    STBIW_PNG_PROFILE(0);
    unsigned char *png = stbiw__write_png_from_filtered(filtered, w, h, 3, len);
    memcpy(marks, png_marks, sizeof png_marks);
    return png;
}
