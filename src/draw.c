/* Headless cell-grid PNG. Same file on macOS and Linux.
   Raster is stb_truetype (public domain). PNG is stb_image_write (public domain,
   deflate included). No Core Text, FreeType, window, or distro package.
   Box-drawing is geometry so the joints meet at an integer cell size. */

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

/* Rounded corners and diagonals are stamped: a disc at each of a stroke's
   points. Stamps keeps which pixels of its cell a stroke covered, to paint
   the next one like it as runs; stamp_points says when two are alike. A
   render of the largest screen finds well under STAMP_SEQUENCES sequences
   per shape and axis (tests/boxes.c checks), and every allocation of the
   cache stays within STAMP_MAX_BYTES in all (tests/stamps_alloc.c checks);
   past either, strokes are stamped afresh. */
#define STAMP_SHAPES 6 /* the four corners, then the two diagonals */
#define STAMP_SEQUENCES 64
#define STAMP_MASKS 1024
#define STAMP_MAX_POINTS 4096
#ifndef STAMP_MAX_BYTES
#define STAMP_MAX_BYTES ((size_t)4 << 20)
#endif

typedef struct {
    uint32_t key; /* 0 for an empty slot */
    int run_count;
    uint16_t *runs; /* 3 per run: row, first column, past the last, in the cell */
} StampMask;

typedef struct {
    int w, h; /* the cell size, set by the first stroke */
    int n[STAMP_SHAPES]; /* each shape's point count */
    int lines[2]; /* columns and rows */
    /* Per axis (x, y), shape and column or row: the sequence of offsets the
       stroke there has, as 1 + its index; 0 not known yet, -1 none. */
    int16_t *ids[2][STAMP_SHAPES];
    /* Per axis and shape: the distinct sequences, n floats each. */
    float *sequences[2][STAMP_SHAPES];
    int sequence_count[2][STAMP_SHAPES], sequence_room[2][STAMP_SHAPES];
    StampMask *masks; /* STAMP_MASKS */
    float *points; /* the stroke being stamped: x, y */
    int capacity;
    uint8_t *mask; /* a cell's coverage, on a miss */
    size_t hits, misses, uncached, bytes, peak; /* bytes held now, and at most */
} Stamps;

/* The image being painted. Passed explicitly so draw_png is reentrant. */
typedef struct {
    uint8_t *px, *filtered;
    int w, h;
    size_t stride;
    float *arc_offsets[4];
    /* Strokes to reuse, or NULL to stamp each one afresh. */
    Stamps *stamps;
    /* While clipped, put and fill_rect stay inside [x0, x1) x [y0, y1):
       geometry never paints into a neighbouring cell. */
    int clipped, clip_x0, clip_y0, clip_x1, clip_y1;
} Canvas;

static void put(Canvas *cv, int x, int y, uint8_t r, uint8_t g, uint8_t b) {
    if ((unsigned)x >= (unsigned)cv->w || (unsigned)y >= (unsigned)cv->h) return;
    if (cv->clipped && (x < cv->clip_x0 || x >= cv->clip_x1 || y < cv->clip_y0 || y >= cv->clip_y1)) return;
    uint8_t *p = cv->px + (size_t)y * cv->stride + (size_t)x * BPP;
    p[0] = r;
    p[1] = g;
    p[2] = b;
}

/* Clip [x0, x1) x [y0, y1) to the canvas and, while clipped, to the clip. */
static int clip_rect(const Canvas *cv, int *x0, int *y0, int *x1, int *y1) {
    if (*x0 < 0) *x0 = 0;
    if (*y0 < 0) *y0 = 0;
    if (*x1 > cv->w) *x1 = cv->w;
    if (*y1 > cv->h) *y1 = cv->h;
    if (cv->clipped) {
        if (*x0 < cv->clip_x0) *x0 = cv->clip_x0;
        if (*y0 < cv->clip_y0) *y0 = cv->clip_y0;
        if (*x1 > cv->clip_x1) *x1 = cv->clip_x1;
        if (*y1 > cv->clip_y1) *y1 = cv->clip_y1;
    }
    return *x0 < *x1 && *y0 < *y1;
}

/* A shade: the colour at k quarters over what is painted, which is the cell's
   background unless an image under the text shows there. */
static void shade_rect(Canvas *cv, int x0, int y0, int x1, int y1, int k, uint8_t r, uint8_t g, uint8_t b) {
    if (!clip_rect(cv, &x0, &y0, &x1, &y1)) return;
    for (int y = y0; y < y1; y++) {
        uint8_t *p = cv->px + (size_t)y * cv->stride + (size_t)x0 * BPP;
        for (int x = x0; x < x1; x++, p += BPP) {
            p[0] = (uint8_t)((r * k + p[0] * (4 - k) + 2) / 4);
            p[1] = (uint8_t)((g * k + p[1] * (4 - k) + 2) / 4);
            p[2] = (uint8_t)((b * k + p[2] * (4 - k) + 2) / 4);
        }
    }
}

static void fill_rect(Canvas *cv, int x0, int y0, int x1, int y1, uint8_t r, uint8_t g, uint8_t b) {
    if (!clip_rect(cv, &x0, &y0, &x1, &y1)) return;
    for (int y = y0; y < y1; y++) {
        uint8_t *row = cv->px + (size_t)y * cv->stride + (size_t)x0 * BPP;
        for (int x = x0; x < x1; x++) {
            row[0] = r;
            row[1] = g;
            row[2] = b;
            row += BPP;
        }
    }
}

static void hbar(Canvas *cv, int x0, int x1, int mid, int thick, uint8_t r, uint8_t g, uint8_t b) {
    fill_rect(cv, x0, mid - thick / 2, x1, mid - thick / 2 + thick, r, g, b);
}

static void vbar(Canvas *cv, int mid, int y0, int y1, int thick, uint8_t r, uint8_t g, uint8_t b) {
    fill_rect(cv, mid - thick / 2, y0, mid - thick / 2 + thick, y1, r, g, b);
}

static void stamp_hold(Stamps *st, size_t bytes) {
    st->bytes += bytes;
    if (st->bytes > st->peak) st->peak = st->bytes;
}

/* Whether the cache may allocate bytes more. */
static int stamp_budget(const Stamps *st, size_t bytes) {
    return bytes <= STAMP_MAX_BYTES && st->bytes <= STAMP_MAX_BYTES - bytes;
}

/* a - b, and whether that is exact: Knuth's TwoSum error is zero. */
static int exact_difference(float a, float b, float *out) {
    float s = a - b;
    float bv = s - a;
    float av = s - bv;
    float err = (a - av) + (-b - bv);
    *out = s;
    return err == 0.0f;
}

/* The cell being stamped, the clip: its column and row in a grid of cells of
   the first stroke's size. 0 when strokes there are not reused. */
static int stamp_cell(Canvas *cv, int shape, int n, int *col, int *row) {
    Stamps *st = cv->stamps;
    int w = cv->clip_x1 - cv->clip_x0, h = cv->clip_y1 - cv->clip_y0;
    if (!st || !cv->clipped || n > STAMP_MAX_POINTS || w <= 0 || h <= 0 || w > 65535 || h > 65535) return 0;
    if (!st->w) {
        st->w = w;
        st->h = h;
        st->lines[0] = cv->w / w;
        st->lines[1] = cv->h / h;
    }
    if (w != st->w || h != st->h || (st->n[shape] && st->n[shape] != n)) return 0;
    st->n[shape] = n;
    if (cv->clip_x0 < 0 || cv->clip_y0 < 0 || cv->clip_x0 % w || cv->clip_y0 % h) return 0;
    *col = cv->clip_x0 / w;
    *row = cv->clip_y0 / h;
    return *col < st->lines[0] && *row < st->lines[1];
}

/* The key of shape's stroke at col and row, at thickness thick: 0 when it
   isn't known yet, -1 when it is not reused. */
static int64_t stamp_key(const Stamps *st, int shape, int col, int row, int thick) {
    if (!st->ids[0][shape] || !st->ids[1][shape]) return 0;
    int x = st->ids[0][shape][col], y = st->ids[1][shape][row];
    if (x < 0 || y < 0 || thick >= 256) return -1;
    if (!x || !y) return 0;
    return 1 + ((((int64_t)shape * 256 + thick) * (STAMP_SEQUENCES + 1) + x) * (STAMP_SEQUENCES + 1) + y);
}

static StampMask *stamp_slot(Stamps *st, uint32_t key) {
    return &st->masks[(key * 2654435761u) >> 22 & (STAMP_MASKS - 1)];
}

static void paint_runs(Canvas *cv, const StampMask *m, uint8_t r, uint8_t g, uint8_t b) {
    for (int k = 0; k < m->run_count; k++) {
        const uint16_t *run = m->runs + 3 * k;
        uint8_t *p = cv->px + (size_t)(cv->clip_y0 + run[0]) * cv->stride + (size_t)(cv->clip_x0 + run[1]) * BPP;
        for (int x = run[1]; x < run[2]; x++, p += BPP) {
            p[0] = r;
            p[1] = g;
            p[2] = b;
        }
    }
}

/* Paint shape's stroke in this cell from a stroke like it, if one was kept.
   1 when painted; 0 when the caller should put its points in
   cv->stamps->points (with room for n) and call stamp_points; -1 when the
   caller stamps them itself. */
static int stamp_find(Canvas *cv, int shape, int n, int thick, uint8_t r, uint8_t g, uint8_t b) {
    int col, row;
    if (!stamp_cell(cv, shape, n, &col, &row)) {
        if (cv->stamps) cv->stamps->uncached++;
        return -1;
    }
    Stamps *st = cv->stamps;
    int64_t key = stamp_key(st, shape, col, row, thick);
    if (key < 0) {
        st->uncached++;
        return -1;
    }
    if (key && st->masks) {
        StampMask *m = stamp_slot(st, (uint32_t)key);
        if (m->key == (uint32_t)key) {
            st->hits++;
            paint_runs(cv, m, r, g, b);
            return 1;
        }
    }
    if (n > st->capacity) {
        float *points = stamp_budget(st, (size_t)(n - st->capacity) * 2 * sizeof(float))
                            ? (float *)realloc(st->points, (size_t)n * 2 * sizeof(float))
                            : NULL;
        if (!points) {
            st->uncached++;
            return -1;
        }
        stamp_hold(st, (size_t)(n - st->capacity) * 2 * sizeof(float));
        st->points = points;
        st->capacity = n;
    }
    return 0;
}

/* The index + 1 of the sequence of offsets of the n points (stride 2) from
   origin, kept for shape and axis; -1 when an offset is inexact or there is
   no room. */
static int stamp_sequence(Stamps *st, int axis, int shape, const float *points, int n, float origin) {
    float *seen = st->sequences[axis][shape];
    int count = st->sequence_count[axis][shape], room = st->sequence_room[axis][shape];
    if (count + 1 > room) {
        /* Room for one more row than is kept, for the one being compared. */
        int more = room ? 2 * room : 4;
        if (more > STAMP_SEQUENCES + 1) more = STAMP_SEQUENCES + 1;
        if (!stamp_budget(st, (size_t)(more - room) * (size_t)n * sizeof(float))) return -1;
        seen = (float *)realloc(seen, (size_t)more * (size_t)n * sizeof(float));
        if (!seen) return -1;
        st->sequences[axis][shape] = seen;
        st->sequence_room[axis][shape] = more;
        stamp_hold(st, (size_t)(more - room) * (size_t)n * sizeof(float));
    }
    float *d = seen + (size_t)count * n;
    for (int i = 0; i < n; i++)
        if (!exact_difference(points[2 * i], origin, &d[i])) return -1;
    for (int k = 0; k < count; k++)
        if (memcmp(seen + (size_t)k * n, d, (size_t)n * sizeof(float)) == 0) return k + 1;
    if (count == STAMP_SEQUENCES) return -1;
    st->sequence_count[axis][shape] = count + 1;
    return count + 1;
}

/* Stamp the n points in cv->stamps->points with discs of squared radius rad2
   (radius rad) in the clip, which stamp_find found to be a cell, as put
   would, and keep what they cover. 1 when painted, 0 when the caller must.

   Which pixels a disc covers depends only on (x + 0.5f) - px and (y + 0.5f)
   - py for the pixel (x, y) and the point (px, py). Moved by whole pixels,
   x + 0.5f stays exact, so if each of a stroke's points is exactly as far
   from its cell's origin as the same point of a stroke kept before, each
   difference is the same real number, rounds the same, and the stroke covers
   the same pixels of its cell. That is checked, not assumed: how a point
   rounds depends on where its cell is. A stroke's x offsets depend only on
   its column, and its y offsets on its row, so each is found once. */
static int stamp_points(Canvas *cv, int shape, int n, int thick, float rad, float rad2, uint8_t r, uint8_t g,
                        uint8_t b) {
    Stamps *st = cv->stamps;
    int col = cv->clip_x0 / st->w, row = cv->clip_y0 / st->h, w = st->w, h = st->h;
    const float *pts = st->points;
    for (int axis = 0; axis < 2; axis++) {
        int16_t **ids = &st->ids[axis][shape];
        if (!*ids) {
            if (stamp_budget(st, (size_t)st->lines[axis] * sizeof(int16_t)))
                *ids = (int16_t *)calloc((size_t)st->lines[axis], sizeof(int16_t));
            if (!*ids) {
                st->uncached++;
                return 0;
            }
            stamp_hold(st, (size_t)st->lines[axis] * sizeof(int16_t));
        }
        int line = axis ? row : col;
        if (!(*ids)[line]) {
            float origin = (float)(axis ? cv->clip_y0 : cv->clip_x0);
            (*ids)[line] = (int16_t)stamp_sequence(st, axis, shape, pts + axis, n, origin);
        }
    }
    int64_t key = stamp_key(st, shape, col, row, thick);
    if (key <= 0) {
        st->uncached++;
        return 0;
    }
    if (!st->masks) {
        if (stamp_budget(st, STAMP_MASKS * sizeof(StampMask)))
            st->masks = (StampMask *)calloc(STAMP_MASKS, sizeof(StampMask));
        if (!st->masks) {
            st->uncached++;
            return 0;
        }
        stamp_hold(st, STAMP_MASKS * sizeof(StampMask));
    }
    StampMask *m = stamp_slot(st, (uint32_t)key);
    if (m->key == (uint32_t)key) {
        st->hits++;
        paint_runs(cv, m, r, g, b);
        return 1;
    }
    size_t size = (size_t)w * (size_t)h;
    if (!st->mask) {
        if (stamp_budget(st, size)) st->mask = (uint8_t *)malloc(size);
        if (!st->mask) {
            st->uncached++;
            return 0;
        }
        stamp_hold(st, size);
    }
    /* Stamp into a mask of the cell what the caller would into the canvas. */
    uint8_t *mask = st->mask;
    int cx = cv->clip_x0, cy = cv->clip_y0;
    memset(mask, 0, size);
    for (int i = 0; i < n; i++) {
        float px = pts[2 * i], py = pts[2 * i + 1];
        int x0 = (int)floorf(px - rad - 1.0f), y0 = (int)floorf(py - rad - 1.0f);
        int x1 = (int)ceilf(px + rad + 1.0f), y1 = (int)ceilf(py + rad + 1.0f);
        if (x0 < cx) x0 = cx;
        if (y0 < cy) y0 = cy;
        if (x1 > cx + w - 1) x1 = cx + w - 1;
        if (y1 > cy + h - 1) y1 = cy + h - 1;
        for (int y = y0; y <= y1; y++) {
            for (int x = x0; x <= x1; x++) {
                float dx = (x + 0.5f) - px;
                float dy = (y + 0.5f) - py;
                if (dx * dx + dy * dy <= rad2) mask[(size_t)(y - cy) * w + (x - cx)] = 1;
            }
        }
    }
    int count = 0;
    for (int y = 0; y < h; y++) {
        const uint8_t *line = mask + (size_t)y * w;
        for (int x = 0; x < w; x++) count += line[x] && (x == 0 || !line[x - 1]);
    }
    size_t bytes = (size_t)(count ? count : 1) * 3 * sizeof(uint16_t);
    uint16_t *runs = NULL;
    if (stamp_budget(st, bytes)) runs = (uint16_t *)malloc(bytes);
    StampMask fresh = {(uint32_t)key, count, runs};
    if (!runs) {
        /* Not kept: paint from the mask's runs all the same. */
        st->uncached++;
        for (int y = 0; y < h; y++) {
            const uint8_t *line = mask + (size_t)y * w;
            uint8_t *p = cv->px + (size_t)(cy + y) * cv->stride + (size_t)cx * BPP;
            for (int x = 0; x < w; x++, p += BPP) {
                if (!line[x]) continue;
                p[0] = r;
                p[1] = g;
                p[2] = b;
            }
        }
        return 1;
    }
    uint16_t *run = runs;
    for (int y = 0; y < h; y++) {
        const uint8_t *line = mask + (size_t)y * w;
        for (int x = 0; x < w; x++) {
            if (!line[x] || (x > 0 && line[x - 1])) continue;
            int end = x;
            while (end < w && line[end]) end++;
            run[0] = (uint16_t)y;
            run[1] = (uint16_t)x;
            run[2] = (uint16_t)end;
            run += 3;
        }
    }
    if (m->key) {
        st->bytes -= (size_t)(m->run_count ? m->run_count : 1) * 3 * sizeof(uint16_t);
        free(m->runs);
    }
    *m = fresh;
    stamp_hold(st, bytes);
    st->misses++;
    paint_runs(cv, m, r, g, b);
    return 1;
}

static void free_stamps(Stamps *st) {
    if (!st) return;
    for (int k = 0; st->masks && k < STAMP_MASKS; k++) free(st->masks[k].runs);
    free(st->masks);
    for (int axis = 0; axis < 2; axis++) {
        for (int shape = 0; shape < STAMP_SHAPES; shape++) {
            free(st->ids[axis][shape]);
            free(st->sequences[axis][shape]);
        }
    }
    free(st->points);
    free(st->mask);
}

/* What geometry holds: the arc offsets and the stamps. */
static void free_geometry(Canvas *cv) {
    for (int k = 0; k < 4; k++) free(cv->arc_offsets[k]);
    free_stamps(cv->stamps);
}

/* Quarter ellipse. Angles are standard math angles with y growing downward. */
static void arc(Canvas *cv, int corner, float cx, float cy, float rx, float ry, float a0, float a1, float thick,
                 uint8_t r, uint8_t g, uint8_t b) {
    int steps = (int)((rx + ry) * 2.0f);
    if (steps < 12) steps = 12;
    /* Cache only the translation-independent products. Add cx/cy at the
       original position, preserving the original float rounding of pixels.
       Cap storage for unusual font metrics; allocation failure uses the old path. */
    int cached = cv->arc_offsets[corner] != NULL;
    if (!cached && steps <= 2048) {
        cv->arc_offsets[corner] = malloc((size_t)(steps + 1) * 2 * sizeof(float));
    }
    float *offsets = cv->arc_offsets[corner];
    float rad = thick * 0.5f;
    float rad2 = (rad + 0.6f) * (rad + 0.6f);
    if (offsets && !cached) {
        for (int i = 0; i <= steps; i++) {
            float a = a0 + (a1 - a0) * ((float)i / (float)steps);
            offsets[2 * i] = rx * cosf(a);
            offsets[2 * i + 1] = ry * sinf(a);
        }
        cached = 1;
    }
    int found = offsets ? stamp_find(cv, corner, steps + 1, (int)thick, r, g, b) : -1;
    if (found == 1) return;
    if (found == 0) {
        float *pts = cv->stamps->points;
        for (int i = 0; i <= steps; i++) {
            pts[2 * i] = cx + offsets[2 * i];
            pts[2 * i + 1] = cy + offsets[2 * i + 1];
        }
        if (stamp_points(cv, corner, steps + 1, (int)thick, rad, rad2, r, g, b)) return;
    }
    for (int i = 0; i <= steps; i++) {
        float ox, oy;
        if (cached) {
            ox = offsets[2 * i];
            oy = offsets[2 * i + 1];
        } else {
            float a = a0 + (a1 - a0) * ((float)i / (float)steps);
            ox = rx * cosf(a);
            oy = ry * sinf(a);
            if (offsets) {
                offsets[2 * i] = ox;
                offsets[2 * i + 1] = oy;
            }
        }
        float px = cx + ox;
        float py = cy + oy;
        int x0 = (int)floorf(px - rad - 1.0f);
        int y0 = (int)floorf(py - rad - 1.0f);
        int x1 = (int)ceilf(px + rad + 1.0f);
        int y1 = (int)ceilf(py + rad + 1.0f);
        for (int y = y0; y <= y1; y++) {
            for (int x = x0; x <= x1; x++) {
                float dx = (x + 0.5f) - px;
                float dy = (y + 0.5f) - py;
                if (dx * dx + dy * dy <= rad2) put(cv, x, y, r, g, b);
            }
        }
    }
}

/* Box drawing, U+2500..U+257F: two bits per arm, (left, up, right, down),
   each 0 none, 1 light (or "single"), 2 heavy, 3 double. Dashes, arcs and
   diagonals are 0 here and drawn by their own code. tests/boxes.c checks this
   table against the Unicode character names. */
enum { NO = 0, LT = 1, HV = 2, DB = 3 };
#define ARMS(l, u, r, d) ((l) | (u) << 2 | (r) << 4 | (d) << 6)
static const uint8_t box_arms[128] = {
    ARMS(LT, NO, LT, NO), ARMS(HV, NO, HV, NO), ARMS(NO, LT, NO, LT), ARMS(NO, HV, NO, HV), /* 2500 ─━│┃ */
    0, 0, 0, 0, 0, 0, 0, 0,                                                               /* 2504 dashes */
    ARMS(NO, NO, LT, LT), ARMS(NO, NO, HV, LT), ARMS(NO, NO, LT, HV), ARMS(NO, NO, HV, HV), /* 250C ┌┍┎┏ */
    ARMS(LT, NO, NO, LT), ARMS(HV, NO, NO, LT), ARMS(LT, NO, NO, HV), ARMS(HV, NO, NO, HV), /* 2510 ┐┑┒┓ */
    ARMS(NO, LT, LT, NO), ARMS(NO, LT, HV, NO), ARMS(NO, HV, LT, NO), ARMS(NO, HV, HV, NO), /* 2514 └┕┖┗ */
    ARMS(LT, LT, NO, NO), ARMS(HV, LT, NO, NO), ARMS(LT, HV, NO, NO), ARMS(HV, HV, NO, NO), /* 2518 ┘┙┚┛ */
    ARMS(NO, LT, LT, LT), ARMS(NO, LT, HV, LT), ARMS(NO, HV, LT, LT), ARMS(NO, LT, LT, HV), /* 251C ├┝┞┟ */
    ARMS(NO, HV, LT, HV), ARMS(NO, HV, HV, LT), ARMS(NO, LT, HV, HV), ARMS(NO, HV, HV, HV), /* 2520 ┠┡┢┣ */
    ARMS(LT, LT, NO, LT), ARMS(HV, LT, NO, LT), ARMS(LT, HV, NO, LT), ARMS(LT, LT, NO, HV), /* 2524 ┤┥┦┧ */
    ARMS(LT, HV, NO, HV), ARMS(HV, HV, NO, LT), ARMS(HV, LT, NO, HV), ARMS(HV, HV, NO, HV), /* 2528 ┨┩┪┫ */
    ARMS(LT, NO, LT, LT), ARMS(HV, NO, LT, LT), ARMS(LT, NO, HV, LT), ARMS(HV, NO, HV, LT), /* 252C ┬┭┮┯ */
    ARMS(LT, NO, LT, HV), ARMS(HV, NO, LT, HV), ARMS(LT, NO, HV, HV), ARMS(HV, NO, HV, HV), /* 2530 ┰┱┲┳ */
    ARMS(LT, LT, LT, NO), ARMS(HV, LT, LT, NO), ARMS(LT, LT, HV, NO), ARMS(HV, LT, HV, NO), /* 2534 ┴┵┶┷ */
    ARMS(LT, HV, LT, NO), ARMS(HV, HV, LT, NO), ARMS(LT, HV, HV, NO), ARMS(HV, HV, HV, NO), /* 2538 ┸┹┺┻ */
    ARMS(LT, LT, LT, LT), ARMS(HV, LT, LT, LT), ARMS(LT, LT, HV, LT), ARMS(HV, LT, HV, LT), /* 253C ┼┽┾┿ */
    ARMS(LT, HV, LT, LT), ARMS(LT, LT, LT, HV), ARMS(LT, HV, LT, HV), ARMS(HV, HV, LT, LT), /* 2540 ╀╁╂╃ */
    ARMS(LT, HV, HV, LT), ARMS(HV, LT, LT, HV), ARMS(LT, LT, HV, HV), ARMS(HV, HV, HV, LT), /* 2544 ╄╅╆╇ */
    ARMS(HV, LT, HV, HV), ARMS(HV, HV, LT, HV), ARMS(LT, HV, HV, HV), ARMS(HV, HV, HV, HV), /* 2548 ╈╉╊╋ */
    0, 0, 0, 0,                                                                           /* 254C dashes */
    ARMS(DB, NO, DB, NO), ARMS(NO, DB, NO, DB), ARMS(NO, NO, DB, LT), ARMS(NO, NO, LT, DB), /* 2550 ═║╒╓ */
    ARMS(NO, NO, DB, DB), ARMS(DB, NO, NO, LT), ARMS(LT, NO, NO, DB), ARMS(DB, NO, NO, DB), /* 2554 ╔╕╖╗ */
    ARMS(NO, LT, DB, NO), ARMS(NO, DB, LT, NO), ARMS(NO, DB, DB, NO), ARMS(DB, LT, NO, NO), /* 2558 ╘╙╚╛ */
    ARMS(LT, DB, NO, NO), ARMS(DB, DB, NO, NO), ARMS(NO, LT, DB, LT), ARMS(NO, DB, LT, DB), /* 255C ╜╝╞╟ */
    ARMS(NO, DB, DB, DB), ARMS(DB, LT, NO, LT), ARMS(LT, DB, NO, DB), ARMS(DB, DB, NO, DB), /* 2560 ╠╡╢╣ */
    ARMS(DB, NO, DB, LT), ARMS(LT, NO, LT, DB), ARMS(DB, NO, DB, DB), ARMS(DB, LT, DB, NO), /* 2564 ╤╥╦╧ */
    ARMS(LT, DB, LT, NO), ARMS(DB, DB, DB, NO), ARMS(DB, LT, DB, LT), ARMS(LT, DB, LT, DB), /* 2568 ╨╩╪╫ */
    ARMS(DB, DB, DB, DB), 0, 0, 0,                                                        /* 256C ╬, arcs */
    0, 0, 0, 0,                                                                           /* 2570 arc, diagonals */
    ARMS(LT, NO, NO, NO), ARMS(NO, LT, NO, NO), ARMS(NO, NO, LT, NO), ARMS(NO, NO, NO, LT), /* 2574 ╴╵╶╷ */
    ARMS(HV, NO, NO, NO), ARMS(NO, HV, NO, NO), ARMS(NO, NO, HV, NO), ARMS(NO, NO, NO, HV), /* 2578 ╸╹╺╻ */
    ARMS(LT, NO, HV, NO), ARMS(NO, LT, NO, HV), ARMS(HV, NO, LT, NO), ARMS(NO, HV, NO, LT), /* 257C ╼╽╾╿ */
};

static int arm_width(int weight, int t) {
    return weight == NO ? 0 : weight == HV ? 2 * t : t;
}

/* Light and heavy lines meeting at the centre. Each arm runs from the cell
   edge across the centre to the far side of the widest crossing stroke, so
   joins are solid and lines continue straight into the next cell. */
static void paint_lines(Canvas *cv, int x, int y, int w, int h, int arms, int t,
                        uint8_t r, uint8_t g, uint8_t b) {
    int left = arms & 3, up = arms >> 2 & 3, right = arms >> 4 & 3, down = arms >> 6 & 3;
    int cx = x + w / 2, cy = y + h / 2;
    int wl = arm_width(left, t), wu = arm_width(up, t), wr = arm_width(right, t), wd = arm_width(down, t);
    int vmax = wu > wd ? wu : wd, hmax = wl > wr ? wl : wr;
    if (left) hbar(cv, x, vmax ? cx - vmax / 2 + vmax : cx - wl / 2 + wl, cy, wl, r, g, b);
    if (right) hbar(cv, vmax ? cx - vmax / 2 : cx - wr / 2, x + w, cy, wr, r, g, b);
    if (up) vbar(cv, cx, y, hmax ? cy - hmax / 2 + hmax : cy - wu / 2 + wu, wu, r, g, b);
    if (down) vbar(cv, cx, hmax ? cy - hmax / 2 : cy - wd / 2, y + h, wd, r, g, b);
}

/* Double lines, alone or meeting single (light) ones. A double arm is two
   strokes of width t with a gap of t, narrowed when needed so the pair keeps
   a clear pixel on each side of the cell. Where double arms meet they form
   inner and outer corners; a single arm meeting a double line stops at the
   near stroke, or crosses both when it turns a corner or crosses straight
   over. */
static void paint_double(Canvas *cv, int x, int y, int w, int h, int arms, int t,
                         uint8_t r, uint8_t g, uint8_t b) {
    int left = arms & 3, up = arms >> 2 & 3, right = arms >> 4 & 3, down = arms >> 6 & 3;
    int cx = x + w / 2, cy = y + h / 2;
    int room = (w < h ? w : h) - 2, dt = t, gap = t;
    while (2 * dt + gap > room && gap > 1) gap--;
    while (2 * dt + gap > room && dt > 1) dt--;
    int span = 2 * dt + gap;
    int bx = cx - span / 2, by = cy - span / 2;   /* first stroke of a double line */
    int bx2 = bx + dt + gap, by2 = by + dt + gap; /* second stroke */
    int st = cx - t / 2, sb = cy - t / 2;       /* a single line */
    int v2 = up == DB || down == DB, h2 = left == DB || right == DB;
    int v1 = up == LT || down == LT;
    if (left == DB || right == DB) {
        int stop[2], start[2]; /* per stroke: [0] upper, [1] lower */
        for (int s = 0; s < 2; s++) {
            int toward = s == 0 ? up : down; /* the vertical arm on this stroke's side */
            if (v2) {
                stop[s] = toward == DB ? bx + dt : bx + span;
                start[s] = toward == DB ? bx2 : bx;
            } else if (left == DB && right == DB) {
                stop[s] = bx + span;
                start[s] = bx;
            } else if (v1) {
                stop[s] = st + t;
                start[s] = st;
            } else {
                stop[s] = bx + span;
                start[s] = bx;
            }
        }
        if (left == DB) {
            fill_rect(cv, x, by, stop[0], by + dt, r, g, b);
            fill_rect(cv, x, by2, stop[1], by2 + dt, r, g, b);
        }
        if (right == DB) {
            fill_rect(cv, start[0], by, x + w, by + dt, r, g, b);
            fill_rect(cv, start[1], by2, x + w, by2 + dt, r, g, b);
        }
    }
    if (up == DB || down == DB) {
        int stop[2], start[2]; /* per stroke: [0] left, [1] right */
        for (int s = 0; s < 2; s++) {
            int toward = s == 0 ? left : right;
            if (h2) {
                stop[s] = toward == DB ? by + dt : by + span;
                start[s] = toward == DB ? by2 : by;
            } else if (up == DB && down == DB) {
                stop[s] = by + span;
                start[s] = by;
            } else if (left == LT || right == LT) {
                stop[s] = sb + t;
                start[s] = sb;
            } else {
                stop[s] = by + span;
                start[s] = by;
            }
        }
        if (up == DB) {
            fill_rect(cv, bx, y, bx + dt, stop[0], r, g, b);
            fill_rect(cv, bx2, y, bx2 + dt, stop[1], r, g, b);
        }
        if (down == DB) {
            fill_rect(cv, bx, start[0], bx + dt, y + h, r, g, b);
            fill_rect(cv, bx2, start[1], bx2 + dt, y + h, r, g, b);
        }
    }
    /* Single arms. */
    int cross_h = left == LT && right == LT, cross_v = up == LT && down == LT;
    int along_v = up == DB && down == DB, along_h = left == DB && right == DB;
    if (left == LT) hbar(cv, x, cross_h || !along_v ? bx + span : bx + dt, cy, t, r, g, b);
    if (right == LT) hbar(cv, cross_h || !along_v ? bx : bx2, x + w, cy, t, r, g, b);
    if (up == LT) vbar(cv, cx, y, cross_v || !along_h ? by + span : by + dt, t, r, g, b);
    if (down == LT) vbar(cv, cx, cross_v || !along_h ? by : by2, y + h, t, r, g, b);
}

/* Dashed lines: n segments, evenly spaced, with half a gap at each end so
   that a row of dashed cells keeps one rhythm. */
static void paint_dashes(Canvas *cv, int x, int y, int w, int h, int vertical, int n, int thick,
                         uint8_t r, uint8_t g, uint8_t b) {
    int len = vertical ? h : w;
    for (int i = 0; i < n; i++) {
        int a = len * i / n, e = len * (i + 1) / n, slot = e - a;
        /* At least 2, so each end of the cell keeps a clear pixel. */
        int gap = slot / 3 >= 2 ? slot / 3 : slot >= 3 ? 2 : slot - 1;
        int s0 = a + gap / 2, s1 = e - (gap - gap / 2);
        if (vertical) vbar(cv, x + w / 2, y + s0, y + s1, thick, r, g, b);
        else hbar(cv, x + s0, x + s1, y + h / 2, thick, r, g, b);
    }
}

/* A straight stroke from (ax, ay) to (bx, by), stamped like the arcs but
   clipped to the cell so it never paints a neighbour. */
static void paint_segment(Canvas *cv, int x, int y, int w, int h, float ax, float ay, float bx, float by,
                          float thick, uint8_t r, uint8_t g, uint8_t b) {
    float len = sqrtf((bx - ax) * (bx - ax) + (by - ay) * (by - ay));
    int steps = (int)(len * 2.0f) + 1;
    float rad = thick * 0.5f;
    float rad2 = (rad + 0.6f) * (rad + 0.6f);
    /* Shapes 4 and 5: from the top right (as ╱) or the top left. */
    int shape = ax > (float)x ? 4 : 5;
    int found = stamp_find(cv, shape, steps + 1, (int)thick, r, g, b);
    if (found == 1) return;
    if (found == 0) {
        float *pts = cv->stamps->points;
        for (int i = 0; i <= steps; i++) {
            pts[2 * i] = ax + (bx - ax) * ((float)i / (float)steps);
            pts[2 * i + 1] = ay + (by - ay) * ((float)i / (float)steps);
        }
        if (stamp_points(cv, shape, steps + 1, (int)thick, rad, rad2, r, g, b)) return;
    }
    for (int i = 0; i <= steps; i++) {
        float px = ax + (bx - ax) * ((float)i / (float)steps);
        float py = ay + (by - ay) * ((float)i / (float)steps);
        int x0 = (int)floorf(px - rad - 1.0f), y0 = (int)floorf(py - rad - 1.0f);
        int x1 = (int)ceilf(px + rad + 1.0f), y1 = (int)ceilf(py + rad + 1.0f);
        if (x0 < x) x0 = x;
        if (y0 < y) y0 = y;
        if (x1 > x + w - 1) x1 = x + w - 1;
        if (y1 > y + h - 1) y1 = y + h - 1;
        for (int yy = y0; yy <= y1; yy++) {
            for (int xx = x0; xx <= x1; xx++) {
                float dx = (xx + 0.5f) - px;
                float dy = (yy + 0.5f) - py;
                if (dx * dx + dy * dy <= rad2) put(cv, xx, yy, r, g, b);
            }
        }
    }
}

/* Block elements, U+2580..U+259F. Eighths round down from the top or left
   edge, so a block and its complement (upper and lower half, left and right
   half) fill the cell exactly. Shades blend the colour over what is painted instead of dithering. */
static int paint_block(Canvas *cv, int x, int y, int w, int h, uint32_t cp, uint8_t r, uint8_t g, uint8_t b) {
    int right = x + w, bottom = y + h, mx = x + w / 2, my = y + h / 2;
    if (cp == 0x2580) {
        fill_rect(cv, x, y, right, my, r, g, b);
    } else if (cp >= 0x2581 && cp <= 0x2588) { /* lower n/8, n = 1..8 */
        int n = (int)(cp - 0x2580);
        fill_rect(cv, x, y + h * (8 - n) / 8, right, bottom, r, g, b);
    } else if (cp >= 0x2589 && cp <= 0x258F) { /* left n/8, n = 7..1 */
        int n = 8 - (int)(cp - 0x2588);
        fill_rect(cv, x, y, x + w * n / 8, bottom, r, g, b);
    } else if (cp == 0x2590) {
        fill_rect(cv, mx, y, right, bottom, r, g, b);
    } else if (cp >= 0x2591 && cp <= 0x2593) { /* light, medium, dark shade */
        int k = (int)(cp - 0x2590);
        shade_rect(cv, x, y, right, bottom, k, r, g, b);
    } else if (cp == 0x2594) {
        fill_rect(cv, x, y, right, y + h / 8, r, g, b);
    } else if (cp == 0x2595) {
        fill_rect(cv, x + w * 7 / 8, y, right, bottom, r, g, b);
    } else if (cp >= 0x2596 && cp <= 0x259F) {
        /* Quadrant bits: 1 upper left, 2 upper right, 4 lower left, 8 lower right. */
        static const uint8_t quads[10] = {4, 8, 1, 1 | 4 | 8, 1 | 8, 1 | 2 | 4, 1 | 2 | 8, 2, 2 | 4, 2 | 4 | 8};
        int q = quads[cp - 0x2596];
        if (q & 1) fill_rect(cv, x, y, mx, my, r, g, b);
        if (q & 2) fill_rect(cv, mx, y, right, my, r, g, b);
        if (q & 4) fill_rect(cv, x, my, mx, bottom, r, g, b);
        if (q & 8) fill_rect(cv, mx, my, right, bottom, r, g, b);
    } else {
        return 0;
    }
    return 1;
}

static int paint_cell_geometry(Canvas *cv, int col, int row, int cell_w, int cell_h, uint32_t cp, int bold,
                               uint8_t r, uint8_t g, uint8_t b) {
    int x = col * cell_w;
    int y = row * cell_h;
    if (cp >= 0x2580 && cp <= 0x259F) return paint_block(cv, x, y, cell_w, cell_h, cp, r, g, b);
    if (cp < 0x2500 || cp > 0x257F) return 0;
    int right = x + cell_w;
    int bottom = y + cell_h;
    int t = cell_w / 12;
    if (t < 1) t = 1;
    if (bold) t += 1;
    const float pi = 3.14159265f;
    switch (cp) {
    case 0x256D: /* ╭ arc down and right */
        arc(cv, 0, (float)right, (float)bottom, cell_w * 0.5f, cell_h * 0.5f, pi, pi * 1.5f, (float)t, r, g, b);
        return 1;
    case 0x256E: /* ╮ */
        arc(cv, 1, (float)x, (float)bottom, cell_w * 0.5f, cell_h * 0.5f, -pi * 0.5f, 0.0f, (float)t, r, g, b);
        return 1;
    case 0x256F: /* ╯ */
        arc(cv, 2, (float)x, (float)y, cell_w * 0.5f, cell_h * 0.5f, 0.0f, pi * 0.5f, (float)t, r, g, b);
        return 1;
    case 0x2570: /* ╰ */
        arc(cv, 3, (float)right, (float)y, cell_w * 0.5f, cell_h * 0.5f, pi * 0.5f, pi, (float)t, r, g, b);
        return 1;
    case 0x2571: /* ╱ */
        paint_segment(cv, x, y, cell_w, cell_h, (float)right, (float)y, (float)x, (float)bottom, (float)t, r, g, b);
        return 1;
    case 0x2572: /* ╲ */
        paint_segment(cv, x, y, cell_w, cell_h, (float)x, (float)y, (float)right, (float)bottom, (float)t, r, g, b);
        return 1;
    case 0x2573: /* ╳ */
        paint_segment(cv, x, y, cell_w, cell_h, (float)right, (float)y, (float)x, (float)bottom, (float)t, r, g, b);
        paint_segment(cv, x, y, cell_w, cell_h, (float)x, (float)y, (float)right, (float)bottom, (float)t, r, g, b);
        return 1;
    /* Dashes: 2504..250B triple and quadruple, 254C..254F double;
       light then heavy, horizontal then vertical. */
    case 0x2504: case 0x2505: case 0x2506: case 0x2507:
    case 0x2508: case 0x2509: case 0x250A: case 0x250B:
    case 0x254C: case 0x254D: case 0x254E: case 0x254F: {
        int k = cp >= 0x254C ? (int)(cp - 0x254C) : (int)(cp - 0x2504) % 4;
        int n = cp >= 0x254C ? 2 : cp <= 0x2507 ? 3 : 4;
        paint_dashes(cv, x, y, cell_w, cell_h, k >= 2, n, k % 2 ? 2 * t : t, r, g, b);
        return 1;
    }
    default: {
        int arms = box_arms[cp - 0x2500];
        if (!arms) return 0;
        int doubled = (arms & 3) == DB || (arms >> 2 & 3) == DB || (arms >> 4 & 3) == DB || (arms >> 6 & 3) == DB;
        if (doubled) paint_double(cv, x, y, cell_w, cell_h, arms, t, r, g, b);
        else paint_lines(cv, x, y, cell_w, cell_h, arms, t, r, g, b);
        return 1;
    }
    }
}

/* Box drawing and blocks, clipped to the cell. Returns 0 for other characters. */
static int paint_geometry(Canvas *cv, int col, int row, int cell_w, int cell_h, uint32_t cp, int bold,
                          uint8_t r, uint8_t g, uint8_t b) {
    if (cp < 0x2500 || cp > 0x259F) return 0;
    cv->clipped = 1;
    cv->clip_x0 = col * cell_w;
    cv->clip_y0 = row * cell_h;
    cv->clip_x1 = cv->clip_x0 + cell_w;
    cv->clip_y1 = cv->clip_y0 + cell_h;
    int painted = paint_cell_geometry(cv, col, row, cell_w, cell_h, cp, bold, r, g, b);
    cv->clipped = 0;
    return painted;
}

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
        fill_rect(cv, x0, y0, x1, y1, r, g, b);
        return;
    }
    fill_rect(cv, x0, y0, x1, y0 + t, r, g, b);
    fill_rect(cv, x0, y1 - t, x1, y1, r, g, b);
    fill_rect(cv, x0, y0 + t, x0 + t, y1 - t, r, g, b);
    fill_rect(cv, x1 - t, y0 + t, x1, y1 - t, r, g, b);
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
            fill_rect(cv, c * bd->cell_w, top, (c + 1) * bd->cell_w, top + 1, cell->br, cell->bg, cell->bb);
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

/* Free what the glyph pass holds when memory runs out in it; returns 2. */
static int glyphs_failed(Canvas *cv, Glyph *cache, Outline *scratch) {
    for (int k = 0; k < GLYPH_CACHE_SIZE; k++) free(cache[k].bitmap);
    free_geometry(cv);
    free(scratch->v);
    free(cv->filtered);
    cv->px = NULL;
    fprintf(stderr, "termshot: glyph allocation failed\n");
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
   Returns 0; 1 for an unusable font; 2 when the image is too large or memory
   runs out; 3 when the PNG cannot be written. */
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
    Stamps stamps = {0};
    Canvas canvas = {.filtered = (uint8_t *)malloc(stride * (size_t)height),
                     .w = (int)width, .h = (int)height, .stride = stride, .stamps = &stamps};
    Canvas *cv = &canvas;
    if (!cv->filtered) {
        fprintf(stderr, "termshot: out of memory for a %lldx%lld image\n", width, height);
        return 2;
    }

    cv->px = cv->filtered + 1;
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
                int geometry = paint_geometry(cv, c, r, cell_w, cell_h, cp, cell->attrs & ATTR_BOLD, cell->fr, cell->fg, cell->fb);
                geometry_ms += now_ms() - tick;
                if (!geometry) {
                    tick = now_ms();
                    Glyph *entry = find_glyph(&g, cp, wide, (cell->attrs & ATTR_ITALIC) != 0, 0, span);
                    if (!entry) return glyphs_failed(cv, cache, &scratch);
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
                return glyphs_failed(cv, cache, &scratch);
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
                fill_rect(cv, x0, y + double_y, x1, y + double_y + line_t, cell->fr, cell->fg, cell->fb);
                fill_rect(cv, x0, y + double_y + 2 * line_t, x1, y + double_y + 3 * line_t, cell->fr, cell->fg, cell->fb);
            } else if (cell->attrs & ATTR_UNDERLINE) {
                fill_rect(cv, x0, y + under_y, x1, y + under_y + line_t, cell->fr, cell->fg, cell->fb);
            }
            if (cell->attrs & ATTR_STRIKE) {
                fill_rect(cv, x0, y + strike_y, x1, y + strike_y + line_t, cell->fr, cell->fg, cell->fb);
            }
        }
    }

    for (int k = 0; k < GLYPH_CACHE_SIZE; k++) free(cache[k].bitmap);
    free_geometry(cv);
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
