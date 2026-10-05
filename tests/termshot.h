/* What the C harnesses call of termshot's Rust, which they link as a static
   library (tests/rust_lib.sh): the cell, the box-drawing painter
   (src/geometry.rs, for tests/boxes.c) and, in the library built with
   --cfg termshot_render, the cell-only render (src/render.rs, for
   tests/draw.c and tests/glyphs.c, which include src/stb_glue.c for the
   stb functions it calls). Each struct is asserted the size the Rust
   asserts. */
#ifndef TERMSHOT_TESTS_H
#define TERMSHOT_TESTS_H

#include <stddef.h>
#include <stdint.h>

/* As Cell in src/cell.rs. */
typedef struct {
    uint32_t ch;
    uint8_t fr, fg, fb;
    uint8_t br, bg, bb;
    uint8_t attrs;
} Cell;

_Static_assert(sizeof(Cell) == 12, "Cell ABI must match src/cell.rs");

/* Cell.attrs bits, as in src/cell.rs. */
#define ATTR_BOLD 1
#define ATTR_UNDERLINE 2
#define ATTR_DOUBLE_UNDERLINE 4
#define ATTR_STRIKE 8
#define ATTR_WIDE 16 /* the first of a wide character's two cells */
#define ATTR_TAIL 32 /* the second; ch is 0 */
#define ATTR_ITALIC 64
#define ATTR_OPAQUE 128

/* The most pixels an image may have, as MAX_PIXELS in src/render.rs. */
#define MAX_PIXELS (1 << 27)

/* The image being painted, as Canvas in src/geometry.rs: filtered is the
   PNG's filtered scanlines, a filter byte then w RGB pixels each, stride
   bytes apart, and px its first pixel; geometry the box-drawing state, or
   NULL to cache nothing. */
typedef struct Geometry Geometry;
typedef struct {
    uint8_t *px, *filtered;
    int32_t w, h;
    size_t stride;
    Geometry *geometry;
} Canvas;

_Static_assert(sizeof(Canvas) == 40, "Canvas ABI must match src/geometry.rs");

/* src/geometry.rs: the state of a render's box drawing, which reuses
   strokes when reuse_strokes is nonzero (NULL when memory runs out), and
   painting one cell's character into a canvas: 1 when painted, 0 for other
   characters, -1 if the painter failed. */
Geometry *termshot_geometry_new(int reuse_strokes);
void termshot_geometry_free(Geometry *geometry);
int termshot_paint_geometry(const Canvas *cv, int col, int row, int cell_w, int cell_h, uint32_t cp, int bold,
                            uint8_t r, uint8_t g, uint8_t b);
/* The stroke cache's counters, as GeometryStats there. */
typedef struct {
    size_t hits, misses, uncached, bytes, peak;
} GeometryStats;
void termshot_geometry_stats(const Geometry *geometry, GeometryStats *out);

/* src/render.rs: cols x rows cells drawn with the TrueType font ttf (its
   face at ttf_start) and the fallback, or NULL, to the PNG out_path.
   0 done; 1 an unusable font; 2 an image too large, memory ran out, or a
   painter failed; 3 the PNG could not be written. */
int draw_png(const Cell *cells, int cols, int rows, const unsigned char *ttf, int ttf_start,
             const unsigned char *fallback_ttf, int fallback_start, double font_px, const char *out_path,
             int verbose);

#endif
