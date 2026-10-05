/* Placement checks for the glyphs, at several sizes:

   - a character the font lacks is an outlined box inset in its cell, and a
     wide one's box spans both cells;
   - a space separator the font lacks (U+3000) stays blank, as do the line and
     paragraph separators and the blank Braille pattern;
   - a glyph with no outline counts as missing: the fallback font draws it,
     or it is a box when no font has an outline for it;
   - the fallback's glyph is scaled to the main font's height and centered in
     its cell or cells, and shrunk to fit when it is too wide;
   - a wide character's glyph is centered over its two cells: the same glyph
     narrow and wide differs only by half a cell, also when both are in one render;
   - an italic glyph keeps its height and leans right by 12 degrees about the
     middle of its cell, so it stays centered in it, also when the fallback
     draws it at its own scale or shrinks it to fit; the same glyph upright
     and italic in one render are each drawn their own way, and italic leaves
     box drawing and the box for a missing character upright.

   Renders with draw_png (src/render.rs, through the static library
   tests/rust_lib.sh builds with --cfg termshot_render), decodes with
   tests/png_read.c. It includes src/stb_glue.c, which the render calls,
   for stb's own functions too. Built and run by
   test.sh with the vendored font:

       ./glyphs <font.ttf> <scratch.png> [<hollow.ttf> <reference.png>]

   It writes the font with no outline for 'A' to hollow.ttf, and 'A' drawn
   from it with font.ttf as the fallback, in the CLI's default colours and
   size, to reference.png. test.sh checks that the CLI's --fallback-font
   renders that same PNG. */
#include "../src/stb_glue.c"
#include "termshot.h"

unsigned char *png_read_rgba(const char *path, int *width, int *height);
void png_read_free(unsigned char *pixels);

static const unsigned char *font, *fallback_font;
static const char *scratch;
static int failures;

typedef struct {
    int x0, y0, x1, y1; /* bounding box of the ink, x1 and y1 exclusive; empty when x1 == 0 */
    int cell_w, cell_h;
    int hollow;         /* the center of the box is background */
    int sides;          /* how many of the box's four sides have ink at their middle */
    int top_x0, bottom_x0; /* the leftmost ink in the box's top and bottom rows */
} Ink;

static Cell cell(uint32_t ch, int attrs) {
    return (Cell){.ch = ch, .fr = 255, .fg = 255, .fb = 255, .br = 0, .bg = 0, .bb = 0, .attrs = attrs};
}

/* The ink in cells [from, to) of a one-row render. */
static Ink ink_in(const Cell *cells, int cols, double px, int from, int to) {
    Ink box = {0};
    if (draw_png(cells, cols, 1, font, 0, fallback_font, 0, px, scratch, 0) != 0) {
        fprintf(stderr, "draw_png failed at px %g\n", px);
        exit(1);
    }
    int w, h;
    unsigned char *rgba = png_read_rgba(scratch, &w, &h);
    if (!rgba) {
        fprintf(stderr, "cannot decode %s\n", scratch);
        exit(1);
    }
    box.cell_w = w / cols;
    box.cell_h = h;
    box.x0 = w, box.y0 = h;
    for (int y = 0; y < h; y++) {
        for (int x = from * box.cell_w; x < to * box.cell_w; x++) {
            const unsigned char *p = rgba + ((size_t)y * w + x) * 4;
            if (p[0] | p[1] | p[2]) {
                if (x < box.x0) box.x0 = x;
                if (y < box.y0) box.y0 = y;
                if (x + 1 > box.x1) box.x1 = x + 1;
                if (y + 1 > box.y1) box.y1 = y + 1;
            }
        }
    }
    if (box.x1 == 0) box.x0 = box.y0 = 0;
    if (box.x1) {
#define LIT(x, y) (rgba[((size_t)(y) * w + (x)) * 4] != 0)
        int mx = (box.x0 + box.x1) / 2, my = (box.y0 + box.y1) / 2;
        box.hollow = !LIT(mx, my);
        box.sides = LIT(box.x0, my) + LIT(box.x1 - 1, my) + LIT(mx, box.y0) + LIT(mx, box.y1 - 1);
        for (box.top_x0 = box.x0; !LIT(box.top_x0, box.y0); box.top_x0++) {}
        for (box.bottom_x0 = box.x0; !LIT(box.bottom_x0, box.y1 - 1); box.bottom_x0++) {}
#undef LIT
    }
    png_read_free(rgba);
    return box;
}

static Ink ink(const Cell *cells, int cols, double px) {
    return ink_in(cells, cols, px, 0, cols);
}

static void expect(int ok, const char *what, double px) {
    if (!ok) {
        printf("FAIL px %g: %s\n", px, what);
        failures++;
    }
}

/* The tofu box in a span of cells starting at cell 1, as paint_tofu draws it. */
static void expect_tofu(Ink got, int cells, double px, const char *what) {
    int cw = got.cell_w, ch = got.cell_h, t = cw / 12 < 1 ? 1 : cw / 12;
    int x0 = cw + cw / 6, x1 = cw + cells * cw - cw / 6, y0 = ch / 6, y1 = ch - ch / 6;
    if (x1 <= x0 || y1 <= y0) {
        expect(got.x1 == 0, what, px);
        return;
    }
    expect(got.x0 == x0 && got.x1 == x1 && got.y0 == y0 && got.y1 == y1, what, px);
    if (x1 - x0 > 2 * t && y1 - y0 > 2 * t) expect(got.hollow && got.sides == 4, what, px);
}

/* A copy of a TrueType font whose glyph for cp is empty, as a color emoji
   font's glyphs are: loca gives it no bytes. The next glyph then starts at
   its outline, which is harmless for these checks. */
static unsigned char *with_empty_glyph(const unsigned char *ttf, long len, uint32_t cp) {
    unsigned char *copy = malloc((size_t)len);
    stbtt_fontinfo info;
    if (!copy || !init_font(&info, ttf, 0)) exit(1);
    memcpy(copy, ttf, (size_t)len);
    int g = stbtt_FindGlyphIndex(&info, (int)cp), size = info.indexToLocFormat ? 4 : 2;
    if (g == 0 || g + 1 >= info.numGlyphs) exit(1);
    memcpy(copy + info.loca + (g + 1) * size, ttf + info.loca + g * size, (size_t)size);
    if (!init_font(&info, copy, 0) || !stbtt_IsGlyphEmpty(&info, stbtt_FindGlyphIndex(&info, (int)cp))) exit(1);
    return copy;
}

/* A copy of a TrueType font with its hhea ascender and descender times
   factor, so a fallback scales its glyphs by 1 / factor: smaller than the main
   font's for 2, and for 0.5 too wide for a cell, so they are shrunk. */
static unsigned char *with_height(const unsigned char *ttf, long len, double factor) {
    unsigned char *copy = malloc((size_t)len);
    stbtt_fontinfo info;
    if (!copy || !init_font(&info, ttf, 0)) exit(1);
    memcpy(copy, ttf, (size_t)len);
    for (int at = info.hhea + 4; at <= info.hhea + 6; at += 2) {
        int v = (int)lround(ttSHORT(copy + at) * factor);
        copy[at] = (unsigned char)((unsigned)v >> 8);
        copy[at + 1] = (unsigned char)v;
    }
    return copy;
}

/* Slanting moves each row of 'l' right by tan(12 degrees) times its height
   above the middle of the cell, and a row below it left. The upright and
   italic inks go to *u and *it. */
static void expect_slant(double px, const char *what, Ink *u, Ink *it) {
    Cell upright_l[] = {cell(' ', 0), cell('l', 0), cell(' ', 0)};
    Cell italic_l[] = {cell(' ', 0), cell('l', ATTR_ITALIC), cell(' ', 0)};
    *u = ink(upright_l, 3, px), *it = ink(italic_l, 3, px);
    double mid = u->cell_h / 2.0, tan12 = 0.2126;
    double top = tan12 * (mid - (u->y0 + 0.5)), bottom = tan12 * (mid - (u->y1 - 0.5));
    expect(it->x1 && it->y0 == u->y0 && it->y1 == u->y1 && fabs(it->top_x0 - u->top_x0 - top) <= 1 &&
               fabs(it->bottom_x0 - u->bottom_x0 - bottom) <= 1,
           what, px);
}

int main(int argc, char **argv) {
    if (argc != 3 && argc != 5) return 2;
    FILE *fp = fopen(argv[1], "rb");
    if (!fp) return 1;
    fseek(fp, 0, SEEK_END);
    long len = ftell(fp);
    fseek(fp, 0, SEEK_SET);
    unsigned char *data = malloc((size_t)len);
    if (!data || fread(data, 1, (size_t)len, fp) != (size_t)len) return 1;
    fclose(fp);
    font = data;
    scratch = argv[2];
    stbtt_fontinfo info;
    if (!init_font(&info, font, 0)) return 1;
    /* The blank checks are about characters no font has. */
    const uint32_t lacked[] = {0x3000, 0x2800, 0x2028, 0x2029};
    for (size_t i = 0; i < sizeof lacked / sizeof lacked[0]; i++) {
        if (stbtt_FindGlyphIndex(&info, (int)lacked[i])) {
            printf("FAIL: the font has U+%04X; pick a character it lacks\n", lacked[i]);
            return 1;
        }
    }
    /* The font itself maps U+16910 (Bamum) to an empty glyph. */
    int bamum = stbtt_FindGlyphIndex(&info, 0x16910);
    if (bamum == 0 || !stbtt_IsGlyphEmpty(&info, bamum)) {
        printf("FAIL: the font no longer maps U+16910 to an empty glyph; pick one it does\n");
        return 1;
    }
    unsigned char *hollow = with_empty_glyph(data, len, 'A');
    if (argc == 5) {
        FILE *out = fopen(argv[3], "wb");
        if (!out || fwrite(hollow, 1, (size_t)len, out) != (size_t)len || fclose(out) != 0) {
            fprintf(stderr, "cannot write %s\n", argv[3]);
            return 1;
        }
        /* main.rs's DEFAULT_FG and DEFAULT_BG, at the default px 48. */
        Cell a = {.ch = 'A', .fr = 219, .fg = 231, .fb = 247, .br = 17, .bg = 24, .bb = 35};
        if (draw_png(&a, 1, 1, hollow, 0, data, 0, 48, argv[4], 0) != 0) return 1;
    }
    unsigned char *hollow_l = with_empty_glyph(data, len, 'l');
    unsigned char *short_font = with_height(data, len, 2), *tall_font = with_height(data, len, 0.5);

    const double sizes[] = {1, 5, 9, 16, 23, 47.5, 128, 255};
    int checks = 0;
    for (size_t i = 0; i < sizeof sizes / sizeof sizes[0]; i++) {
        double px = sizes[i];
        Cell narrow_missing[] = {cell(' ', 0), cell(0x10ffff, 0), cell(' ', 0)};
        expect_tofu(ink(narrow_missing, 3, px), 1, px, "a missing character is a box in its cell");
        Cell wide_missing[] = {cell(' ', 0), cell(0x1f600, ATTR_WIDE), cell(0, ATTR_TAIL), cell(' ', 0)};
        expect_tofu(ink(wide_missing, 4, px), 2, px, "a missing wide character is a box over both cells");
        Cell bold_missing[] = {cell(' ', 0), cell(0x10ffff, ATTR_BOLD), cell(' ', 0)};
        expect_tofu(ink(bold_missing, 3, px), 1, px, "bold does not change the box");

        Cell spaces[] = {cell(0x3000, ATTR_WIDE), cell(0, ATTR_TAIL), cell(0xa0, 0), cell(0x2003, 0)};
        expect(ink(spaces, 4, px).x1 == 0, "space separators are blank", px);
        Cell blanks[] = {cell(0x2800, 0), cell(0x2028, 0), cell(0x2029, 0)};
        expect(ink(blanks, 3, px).x1 == 0, "U+2800, U+2028 and U+2029 are blank", px);

        Cell narrow[] = {cell(' ', 0), cell('A', 0), cell(' ', 0)};
        Cell wide[] = {cell(' ', 0), cell('A', ATTR_WIDE), cell(0, ATTR_TAIL), cell(' ', 0)};
        Ink a = ink(narrow, 3, px), b = ink(wide, 4, px);
        int shift = b.x0 - a.x0;
        expect(a.x1 && b.x1, "A is drawn", px);
        expect((shift == a.cell_w / 2 || shift == (a.cell_w + 1) / 2) && b.x1 - b.x0 == a.x1 - a.x0 &&
                   b.y0 == a.y0 && b.y1 == a.y1,
               "a wide glyph is the narrow one moved half a cell", px);
        /* The cache must not hand the narrow glyph to the wide one. */
        Cell mixed[] = {cell(' ', 0), cell('A', 0), cell(' ', 0), cell('A', ATTR_WIDE), cell(0, ATTR_TAIL), cell(' ', 0)};
        Ink c = ink_in(mixed, 6, px, 3, 6);
        expect(c.x0 - 2 * c.cell_w == b.x0 && c.x1 - 2 * c.cell_w == b.x1, "narrow and wide A in one render", px);

        Cell bamum_cells[] = {cell(' ', 0), cell(0x16910, 0), cell(' ', 0)};
        expect_tofu(ink(bamum_cells, 3, px), 1, px, "a glyph with no outline is a box");
        /* An 'A' with no outline in the main font: the fallback's is drawn.
           It is centered, and scaled by the fallback's own scale, so its
           edges may be a pixel off the main font's 'A'; a box or a blank
           cell is far further off. */
        const unsigned char *main_font = font;
        font = hollow;
        expect_tofu(ink(narrow, 3, px), 1, px, "an empty glyph and no fallback is a box");
        fallback_font = main_font;
        Ink f = ink(narrow, 3, px);
        expect(f.x1 && abs(f.x0 - a.x0) <= 1 && abs(f.x1 - a.x1) <= 1 && abs(f.y0 - a.y0) <= 1 &&
                   abs(f.y1 - a.y1) <= 1,
               "the fallback draws a glyph the main font has no outline for", px);
        Ink fw = ink(wide, 4, px);
        expect(fw.x1 && abs(fw.x0 - b.x0) <= 1 && abs(fw.x1 - b.x1) <= 1 && abs(fw.y0 - b.y0) <= 1 &&
                   abs(fw.y1 - b.y1) <= 1,
               "the fallback's wide glyph is centered over both cells", px);
        /* At twice the size the fallback's 'A' is two cells wide, so it is
           shrunk back to one; at half the size it is centered, on the
           baseline. */
        fallback_font = tall_font;
        Ink fs = ink(narrow, 3, px);
        expect(fs.x1 && fs.x0 >= a.cell_w && fs.x1 <= 2 * a.cell_w && abs(fs.x0 - a.x0) <= 1 && abs(fs.x1 - a.x1) <= 1 &&
                   abs(fs.y0 - a.y0) <= 1 && abs(fs.y1 - a.y1) <= 1,
               "a fallback glyph too wide for its cell is shrunk to fit", px);
        fallback_font = short_font;
        Ink fh = ink(narrow, 3, px);
        expect((px < 9 && fh.x1 == 0) || (fh.x1 && abs(fh.x0 + fh.x1 - a.x0 - a.x1) <= 2 && abs(fh.y1 - a.y1) <= 1 &&
                              abs(2 * (fh.x1 - fh.x0) - (a.x1 - a.x0)) <= 2),
               "a smaller fallback glyph is centered on the baseline", px);
        fallback_font = hollow;
        expect_tofu(ink(narrow, 3, px), 1, px, "an empty glyph in both fonts is a box");
        font = main_font;
        fallback_font = NULL;

        Ink u, it, fu, fi;
        expect_slant(px, "an italic glyph keeps its height and leans 12 degrees about the middle of its cell", &u, &it);
        /* The fallback slants about the same middle, in its own units. */
        font = hollow_l;
        fallback_font = short_font;
        expect_slant(px, "the fallback's smaller italic glyph leans about the middle of the cell", &fu, &fi);
        fallback_font = tall_font;
        expect_slant(px, "the fallback's shrunk italic glyph leans about the middle of the cell", &fu, &fi);
        font = main_font;
        fallback_font = NULL;
        /* The cache must not hand one to the other. */
        Cell both_l[] = {cell(' ', 0), cell('l', ATTR_ITALIC), cell(' ', 0), cell('l', 0), cell(' ', 0)};
        Ink bi = ink_in(both_l, 5, px, 0, 3), bu = ink_in(both_l, 5, px, 3, 5);
        expect(bi.x0 == it.x0 && bi.x1 == it.x1 && bu.x0 - 2 * bu.cell_w == u.x0 && bu.x1 - 2 * bu.cell_w == u.x1,
               "upright and italic l in one render", px);
        Cell italic_missing[] = {cell(' ', 0), cell(0x10ffff, ATTR_ITALIC), cell(' ', 0)};
        expect_tofu(ink(italic_missing, 3, px), 1, px, "italic does not slant the box");
        Cell box_upright[] = {cell(0x253c, 0), cell(0x2588, 0)}, box_italic[] = {cell(0x253c, ATTR_ITALIC), cell(0x2588, ATTR_ITALIC)};
        Ink gu = ink(box_upright, 2, px), gi = ink(box_italic, 2, px);
        expect(gu.x0 == gi.x0 && gu.x1 == gi.x1 && gu.y0 == gi.y0 && gu.y1 == gi.y1 && gu.top_x0 == gi.top_x0 &&
                   gu.bottom_x0 == gi.bottom_x0,
               "box drawing stays upright in italic", px);
        checks += 21;
    }
    free(hollow);
    free(hollow_l);
    free(short_font);
    free(tall_font);
    free(data);
    if (failures) return 1;
    printf("ok, %d glyph placement checks over %zu sizes\n", checks, sizeof sizes / sizeof sizes[0]);
    return 0;
}
