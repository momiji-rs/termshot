/* CFF (Type 2 charstring) outlines, the C side of the #25 POC
   (docs/cff-rust-vs-c.md). Same design as cff.rs. */
#ifndef CFF_H
#define CFF_H
#include <stddef.h>
#include <stdint.h>

/* stbtt_vertex. */
typedef struct {
    int16_t x, y, cx, cy, cx1, cy1;
    uint8_t type, padding;
} cff_vertex;

/* A growable vertex buffer the caller keeps between glyphs. */
typedef struct {
    cff_vertex *v;
    size_t len, cap;
} cff_outline;

typedef struct cff_font cff_font;

/* Parse face `face` of a font. NULL with *error set on failure. */
cff_font *cff_parse(const uint8_t *data, size_t len, int face, const char **error);
int cff_glyph_count(const cff_font *font);
/* 1: drawn, out and box filled. 0: stb draws nothing. -1: *error says why
   stb would assert, hang, or run out of memory. */
int cff_glyph(const cff_font *font, int glyph, cff_outline *out, int box[4], const char **error);
void cff_free(cff_font *font);

#endif
