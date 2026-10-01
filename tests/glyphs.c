/* Placement checks for draw.c's glyphs, at several sizes:

   - a character the font lacks is an outlined box inset in its cell, and a
     wide one's box spans both cells;
   - a space separator the font lacks (U+3000) stays blank;
   - a wide character's glyph is centered over its two cells: the same glyph
     narrow and wide differs only by half a cell, also when both are in one render.

   Renders with draw_png, decodes with tests/png_read.c. Built and run by
   test.sh with the vendored font: ./glyphs <font.ttf> <scratch.png>. */
#include "../src/draw.c"

unsigned char *png_read_rgba(const char *path, int *width, int *height);
void png_read_free(unsigned char *pixels);

static const unsigned char *font;
static const char *scratch;
static int failures;

typedef struct {
    int x0, y0, x1, y1; /* bounding box of the ink, x1 and y1 exclusive; empty when x1 == 0 */
    int cell_w, cell_h;
    int hollow;         /* the center of the box is background */
    int sides;          /* how many of the box's four sides have ink at their middle */
} Ink;

static Cell cell(uint32_t ch, int attrs) {
    return (Cell){.ch = ch, .fr = 255, .fg = 255, .fb = 255, .br = 0, .bg = 0, .bb = 0, .attrs = attrs};
}

/* The ink in cells [from, to) of a one-row render. */
static Ink ink_in(const Cell *cells, int cols, double px, int from, int to) {
    Ink box = {0};
    if (draw_png(cells, cols, 1, font, NULL, px, scratch, 0) != 0) {
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

int main(int argc, char **argv) {
    if (argc != 3) return 2;
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
        checks += 7;
    }
    free(data);
    if (failures) return 1;
    printf("ok, %d glyph placement checks over %zu sizes\n", checks, sizeof sizes / sizeof sizes[0]);
    return 0;
}
