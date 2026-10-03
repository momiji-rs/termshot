/* Stock stb_truetype, built as draw.c builds it (asserts on), as the
   reference the two CFF readers are checked against. */
#include <stdlib.h>
#define STB_TRUETYPE_IMPLEMENTATION
#include "stb_truetype.h"

int stb_faces(const unsigned char *data) { return stbtt_GetNumberOfFonts(data); }

stbtt_fontinfo *stb_open(const unsigned char *data, int face) {
    stbtt_fontinfo *font = malloc(sizeof *font);
    if (!font || !stbtt_InitFont(font, data, stbtt_GetFontOffsetForIndex(data, face))) {
        free(font);
        return NULL;
    }
    return font;
}

int stb_glyph_count(const stbtt_fontinfo *font) { return font->numGlyphs; }

int stb_glyph_index(const stbtt_fontinfo *font, int codepoint) { return stbtt_FindGlyphIndex(font, codepoint); }

/* The outline, as stbtt_GetGlyphShape gives it; free with stb_free_shape. */
int stb_shape(const stbtt_fontinfo *font, int glyph, stbtt_vertex **out) {
    return stbtt_GetGlyphShape(font, glyph, out);
}

void stb_free_shape(const stbtt_fontinfo *font, stbtt_vertex *v) { stbtt_FreeShape(font, v); }

void stb_box(const stbtt_fontinfo *font, int glyph, int box[4]) {
    stbtt_GetGlyphBox(font, glyph, &box[0], &box[1], &box[2], &box[3]);
}

/* What draw.c pays per glyph today: the bitmap box, then the bitmap (which
   runs the charstring three more times: twice for the shape, once for the
   box). Returns the bitmap's size in bytes. */
int stb_draw(const stbtt_fontinfo *font, int glyph, float scale, unsigned char *buf, int cap) {
    int x0, y0, x1, y1;
    stbtt_GetGlyphBitmapBox(font, glyph, scale, scale, &x0, &y0, &x1, &y1);
    int w = x1 - x0, h = y1 - y0;
    if (w <= 0 || h <= 0 || w * h > cap) return 0;
    stbtt_MakeGlyphBitmap(font, buf, w, h, w, scale, scale, glyph);
    return w * h;
}

/* Option C: an outline from cff.rs / cff.c, rasterised by stb exactly as
   stbtt_MakeGlyphBitmapSubpixel would. box is in font units; returns the
   bitmap's size in bytes. */
int stb_raster(stbtt_vertex *v, int n, const int box[4], float scale, unsigned char *buf, int cap) {
    int x0 = (int)STBTT_ifloor(box[0] * scale), y0 = (int)STBTT_ifloor(-box[3] * scale);
    int x1 = (int)STBTT_iceil(box[2] * scale), y1 = (int)STBTT_iceil(-box[1] * scale);
    int w = x1 - x0, h = y1 - y0;
    if (n == 0 || w <= 0 || h <= 0 || w * h > cap) return 0;
    stbtt__bitmap gbm = {w, h, w, buf};
    stbtt_Rasterize(&gbm, 0.35f, v, n, scale, scale, 0, 0, x0, y0, 1, NULL);
    return w * h;
}
