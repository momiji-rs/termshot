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
#include "deflate_profile.h"

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
/* stb's deflate, with faster search/emission and identical bytes (deflate.c). */
unsigned char *termshot_zlib_compress(unsigned char *data, int data_len, int *out_len, int quality);
#define STBIW_ZLIB_COMPRESS termshot_zlib_compress
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

/* Cell.attrs bits, as in src/main.rs. */
#define ATTR_BOLD 1
#define ATTR_UNDERLINE 2
#define ATTR_DOUBLE_UNDERLINE 4
#define ATTR_STRIKE 8
#define ATTR_WIDE 16 /* the first of a wide character's two cells */
#define ATTR_TAIL 32 /* the second; ch is 0 */

/* The canvas is RGB: alpha would always be 255, and an opaque RGBA PNG is
   larger and blocks palette quantization in downstream optimizers. */
#define BPP 3

/* 2^27 pixels keeps the 3-byte rows plus filter bytes near 384 MiB, well under INT_MAX. */
#define MAX_PIXELS (1 << 27)

/* Bounded per-render cache. Bold and colors reuse the same coverage bitmap.
   1024 slots avoid thrashing on mixed Unicode screens (800 distinct codepoints
   in the benchmark), while keeping metadata to 48 KiB on 64-bit builds. */
typedef struct {
    uint32_t cp;
    /* wide is part of the key: a wide glyph is centered over two cells. shift
       moves the glyph right within its cell (or cells); missing means neither
       font has it. */
    int valid, wide, missing, shift, ix0, iy0, w, h;
    unsigned char *bitmap;
} Glyph;
#define GLYPH_CACHE_SIZE 1024

/* The image being painted. Passed explicitly so draw_png is reentrant. */
typedef struct {
    uint8_t *px, *filtered;
    int w, h;
    size_t stride;
    float *arc_offsets[4];
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

static void fill_rect(Canvas *cv, int x0, int y0, int x1, int y1, uint8_t r, uint8_t g, uint8_t b) {
    if (x0 < 0) x0 = 0;
    if (y0 < 0) y0 = 0;
    if (x1 > cv->w) x1 = cv->w;
    if (y1 > cv->h) y1 = cv->h;
    if (cv->clipped) {
        if (x0 < cv->clip_x0) x0 = cv->clip_x0;
        if (y0 < cv->clip_y0) y0 = cv->clip_y0;
        if (x1 > cv->clip_x1) x1 = cv->clip_x1;
        if (y1 > cv->clip_y1) y1 = cv->clip_y1;
    }
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
   half) fill the cell exactly. Shades mix the colours instead of dithering. */
static int paint_block(Canvas *cv, int x, int y, int w, int h, uint32_t cp,
                       uint8_t r, uint8_t g, uint8_t b, uint8_t br, uint8_t bgc, uint8_t bb) {
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
        fill_rect(cv, x, y, right, bottom, (uint8_t)((r * k + br * (4 - k) + 2) / 4),
                  (uint8_t)((g * k + bgc * (4 - k) + 2) / 4), (uint8_t)((b * k + bb * (4 - k) + 2) / 4));
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
                               uint8_t r, uint8_t g, uint8_t b, uint8_t br, uint8_t bgc, uint8_t bb) {
    int x = col * cell_w;
    int y = row * cell_h;
    if (cp >= 0x2580 && cp <= 0x259F) return paint_block(cv, x, y, cell_w, cell_h, cp, r, g, b, br, bgc, bb);
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
                          uint8_t r, uint8_t g, uint8_t b, uint8_t br, uint8_t bgc, uint8_t bb) {
    if (cp < 0x2500 || cp > 0x259F) return 0;
    cv->clipped = 1;
    cv->clip_x0 = col * cell_w;
    cv->clip_y0 = row * cell_h;
    cv->clip_x1 = cv->clip_x0 + cell_w;
    cv->clip_y1 = cv->clip_y0 + cell_h;
    int painted = paint_cell_geometry(cv, col, row, cell_w, cell_h, cp, bold, r, g, b, br, bgc, bb);
    cv->clipped = 0;
    return painted;
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

static int init_font(stbtt_fontinfo *font, const unsigned char *ttf) {
    int offset = stbtt_GetFontOffsetForIndex(ttf, 0);
    return offset >= 0 && stbtt_InitFont(font, ttf, offset);
}

/* Unicode's space separators (Zs): blank even when no font has them. */
static int is_space(uint32_t cp) {
    return cp == 0x20 || cp == 0xa0 || cp == 0x1680 || (cp >= 0x2000 && cp <= 0x200a) || cp == 0x202f ||
           cp == 0x205f || cp == 0x3000;
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

/* Paint cells with the font in ttf (a TrueType file the caller has already
   checked; see src/font.rs) and write a PNG. fallback_ttf, checked the same
   way, or NULL, supplies the characters ttf lacks; characters neither has are
   drawn as an outlined box. The canvas and cache are local; timing hooks use
   thread-local state so concurrent renders remain independent.
   verbose prints the cell and image size to stderr.
   Returns 0; 1 for an unusable font; 2 when the image is too large or memory
   runs out; 3 when the PNG cannot be written. */
int draw_png(const Cell *cells, int cols, int rows, const unsigned char *ttf,
             const unsigned char *fallback_ttf, double font_px, const char *out_path, int verbose) {
    profiling = getenv("TERMSHOT_PROFILE") != NULL;
    termshot_deflate_profile.enabled = profiling;
    double started = now_ms();
    stbtt_fontinfo font, fallback;
    if (!init_font(&font, ttf) || (fallback_ttf && !init_font(&fallback, fallback_ttf))) {
        fprintf(stderr, "termshot: font init failed\n");
        return 1;
    }

    int ascent, descent, line_gap;
    stbtt_GetFontVMetrics(&font, &ascent, &descent, &line_gap);
    int adv = 0, lsb = 0;
    stbtt_GetCodepointHMetrics(&font, 'M', &adv, &lsb);
    if (adv <= 0 || ascent <= descent) {
        fprintf(stderr, "termshot: font metrics unusable\n");
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
    /* The fallback is sized to the same ascent-to-descent height and shares
       the baseline. */
    float fallback_scale = fallback_ttf ? stbtt_ScaleForPixelHeight(&fallback, (float)body) : 0;
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
    double allocated = now_ms();
    for (int r = 0; r < rows; r++) {
        int y = r * cell_h;
        uint8_t *scanline = cv->filtered + (size_t)y * cv->stride;
        scanline[0] = 0;
        for (int c = 0; c < cols; c++) {
            const Cell *cell = &cells[r * cols + c];
            fill_rect(cv, c * cell_w, y, (c + 1) * cell_w, y + 1, cell->br, cell->bg, cell->bb);
        }
        for (int dy = 1; dy < cell_h; dy++) {
            memcpy(cv->filtered + (size_t)(y + dy) * cv->stride, scanline, cv->stride);
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
            int geometry = paint_geometry(cv, c, r, cell_w, cell_h, cp, cell->attrs & ATTR_BOLD, cell->fr, cell->fg, cell->fb,
                                          cell->br, cell->bg, cell->bb);
            geometry_ms += now_ms() - tick;
            if (geometry) continue;
            tick = now_ms();
            int wide = (cell->attrs & ATTR_WIDE) != 0;
            int span = wide ? 2 * cell_w : cell_w;
            Glyph *entry = &cache[cp % GLYPH_CACHE_SIZE];
            if (entry->valid && entry->cp == cp && entry->wide == wide) {
                cache_hits++;
            } else {
                free(entry->bitmap);
                *entry = (Glyph){.cp = cp, .valid = 1, .wide = wide};
                const stbtt_fontinfo *face = &font;
                float s = scale;
                int glyph = stbtt_FindGlyphIndex(&font, (int)cp);
                if (glyph == 0 && fallback_ttf) {
                    face = &fallback;
                    s = fallback_scale;
                    glyph = stbtt_FindGlyphIndex(&fallback, (int)cp);
                }
                entry->missing = glyph == 0;
                if (glyph != 0) {
                    /* The primary font's narrow glyphs sit where the font puts
                       them. Wide and fallback glyphs are centered, and a
                       fallback glyph too wide for its cells is shrunk. */
                    int glyph_adv, glyph_lsb;
                    stbtt_GetGlyphHMetrics(face, glyph, &glyph_adv, &glyph_lsb);
                    float advance = glyph_adv * s;
                    if (face == &fallback && advance > span) {
                        s *= span / advance;
                        advance = (float)span;
                    }
                    if (wide || face == &fallback) entry->shift = (int)floorf((span - advance) / 2 + 0.5f);
                    int ix1, iy1;
                    stbtt_GetGlyphBitmapBox(face, glyph, s, s, &entry->ix0, &entry->iy0, &ix1, &iy1);
                    entry->w = ix1 - entry->ix0;
                    entry->h = iy1 - entry->iy0;
                    if (entry->w > 0 && entry->h > 0) {
                        entry->bitmap = (unsigned char *)malloc((size_t)entry->w * entry->h);
                        if (!entry->bitmap) {
                            for (int k = 0; k < GLYPH_CACHE_SIZE; k++) free(cache[k].bitmap);
                            for (int k = 0; k < 4; k++) free(cv->arc_offsets[k]);
                            free(cv->filtered);
                            cv->px = NULL;
                            fprintf(stderr, "termshot: glyph allocation failed\n");
                            return 2;
                        }
                        stbtt_MakeGlyphBitmap(face, entry->bitmap, entry->w, entry->h, entry->w, s, s, glyph);
                        glyphs++;
                    }
                }
            }
            glyph_ms += now_ms() - tick;
            tick = now_ms();
            if (entry->missing && !is_space(cp)) {
                paint_tofu(cv, c * cell_w, r * cell_h, span, cell_w, cell_h, cell->fr, cell->fg, cell->fb);
                blend_ms += now_ms() - tick;
                continue;
            }
            if (!entry->bitmap) continue;
            const unsigned char *bm = entry->bitmap;
            int gw = entry->w, gh = entry->h;
            int dx = c * cell_w + entry->shift + entry->ix0;
            int dy = r * cell_h + baseline + entry->iy0;
            blend(cv, dx, dy, bm, gw, gh, cell->fr, cell->fg, cell->fb);
            if (cell->attrs & ATTR_BOLD) blend(cv, dx + 1, dy, bm, gw, gh, cell->fr, cell->fg, cell->fb);
            blend_ms += now_ms() - tick;

        }
    }

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
    for (int k = 0; k < 4; k++) free(cv->arc_offsets[k]);
    double foreground = now_ms();
    int png_len = 0;
    STBIW_PNG_PROFILE(0);
    unsigned char *png = stbiw__write_png_from_filtered(cv->filtered, cv->w, cv->h, BPP, &png_len);
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
    free(cv->filtered);
    cv->px = NULL;
    if (profiling) {
        fprintf(stderr, "termshot-profile {\"deflate_allocate_ms\":%.6f,\"deflate_match_emit_ms\":%.6f,\"deflate_finalize_ms\":%.6f,\"deflate_checksum_ms\":%.6f,\"font_setup_ms\":%.6f,\"allocate_ms\":%.6f,\"background_ms\":%.6f,\"foreground_ms\":%.6f,\"geometry_ms\":%.6f,\"glyph_ms\":%.6f,\"blend_ms\":%.6f,\"png_filter_ms\":%.6f,\"png_deflate_ms\":%.6f,\"png_pack_ms\":%.6f,\"png_encode_ms\":%.6f,\"output_write_ms\":%.6f,\"cleanup_ms\":%.6f,\"glyph_rasterizations\":%zu,\"glyph_cache_hits\":%zu,\"png_bytes\":%d,\"pixel_bytes\":%zu}\n",
            termshot_deflate_profile.allocate_ms, termshot_deflate_profile.match_emit_ms,
            termshot_deflate_profile.finalize_ms, termshot_deflate_profile.checksum_ms,
            font_setup - started, allocated - font_setup,
            background - allocated, foreground - background, geometry_ms, glyph_ms, blend_ms,
            png_marks[1] - png_marks[0], png_marks[2] - png_marks[1], png_marks[3] - png_marks[2],
            encoded - foreground, written - encoded, now_ms() - written, glyphs, cache_hits, png_len, (size_t)(width * height * BPP));
    }
    if (!ok) {
        fprintf(stderr, "termshot: png write failed: %s\n", out_path);
        return 3;
    }
    return 0;
}
