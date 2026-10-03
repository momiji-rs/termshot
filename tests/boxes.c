/* Box-drawing and block-element checks for draw.c's geometry (U+2500..U+259F).
   The expectations come from the Unicode character names below, parsed here,
   not from draw.c's own tables. Each character is painted alone into the
   middle cell of a 3x3 grid, at many cell sizes, plain and bold:

   - nothing is painted outside the cell;
   - on each cell edge, a line's cross-section matches the reference line of
     its weight (─ ━ ═ across, │ ┃ ║ down), so lines join with any neighbour;
     an edge without an arm is empty;
   - light and heavy line characters are one connected shape;
   - dashes have the named number of segments, and arcs and diagonals reach
     the edges they name;
   - blocks fill the named fraction from the named edge, complementary blocks
     tile the cell exactly, and shades are a flat mix of the two colours.

   Built and run by test.sh. */
#include "../src/draw.c"

static const char *NAMES[160] = {
    "LIGHT HORIZONTAL", /* 2500 */
    "HEAVY HORIZONTAL", /* 2501 */
    "LIGHT VERTICAL", /* 2502 */
    "HEAVY VERTICAL", /* 2503 */
    "LIGHT TRIPLE DASH HORIZONTAL", /* 2504 */
    "HEAVY TRIPLE DASH HORIZONTAL", /* 2505 */
    "LIGHT TRIPLE DASH VERTICAL", /* 2506 */
    "HEAVY TRIPLE DASH VERTICAL", /* 2507 */
    "LIGHT QUADRUPLE DASH HORIZONTAL", /* 2508 */
    "HEAVY QUADRUPLE DASH HORIZONTAL", /* 2509 */
    "LIGHT QUADRUPLE DASH VERTICAL", /* 250A */
    "HEAVY QUADRUPLE DASH VERTICAL", /* 250B */
    "LIGHT DOWN AND RIGHT", /* 250C */
    "DOWN LIGHT AND RIGHT HEAVY", /* 250D */
    "DOWN HEAVY AND RIGHT LIGHT", /* 250E */
    "HEAVY DOWN AND RIGHT", /* 250F */
    "LIGHT DOWN AND LEFT", /* 2510 */
    "DOWN LIGHT AND LEFT HEAVY", /* 2511 */
    "DOWN HEAVY AND LEFT LIGHT", /* 2512 */
    "HEAVY DOWN AND LEFT", /* 2513 */
    "LIGHT UP AND RIGHT", /* 2514 */
    "UP LIGHT AND RIGHT HEAVY", /* 2515 */
    "UP HEAVY AND RIGHT LIGHT", /* 2516 */
    "HEAVY UP AND RIGHT", /* 2517 */
    "LIGHT UP AND LEFT", /* 2518 */
    "UP LIGHT AND LEFT HEAVY", /* 2519 */
    "UP HEAVY AND LEFT LIGHT", /* 251A */
    "HEAVY UP AND LEFT", /* 251B */
    "LIGHT VERTICAL AND RIGHT", /* 251C */
    "VERTICAL LIGHT AND RIGHT HEAVY", /* 251D */
    "UP HEAVY AND RIGHT DOWN LIGHT", /* 251E */
    "DOWN HEAVY AND RIGHT UP LIGHT", /* 251F */
    "VERTICAL HEAVY AND RIGHT LIGHT", /* 2520 */
    "DOWN LIGHT AND RIGHT UP HEAVY", /* 2521 */
    "UP LIGHT AND RIGHT DOWN HEAVY", /* 2522 */
    "HEAVY VERTICAL AND RIGHT", /* 2523 */
    "LIGHT VERTICAL AND LEFT", /* 2524 */
    "VERTICAL LIGHT AND LEFT HEAVY", /* 2525 */
    "UP HEAVY AND LEFT DOWN LIGHT", /* 2526 */
    "DOWN HEAVY AND LEFT UP LIGHT", /* 2527 */
    "VERTICAL HEAVY AND LEFT LIGHT", /* 2528 */
    "DOWN LIGHT AND LEFT UP HEAVY", /* 2529 */
    "UP LIGHT AND LEFT DOWN HEAVY", /* 252A */
    "HEAVY VERTICAL AND LEFT", /* 252B */
    "LIGHT DOWN AND HORIZONTAL", /* 252C */
    "LEFT HEAVY AND RIGHT DOWN LIGHT", /* 252D */
    "RIGHT HEAVY AND LEFT DOWN LIGHT", /* 252E */
    "DOWN LIGHT AND HORIZONTAL HEAVY", /* 252F */
    "DOWN HEAVY AND HORIZONTAL LIGHT", /* 2530 */
    "RIGHT LIGHT AND LEFT DOWN HEAVY", /* 2531 */
    "LEFT LIGHT AND RIGHT DOWN HEAVY", /* 2532 */
    "HEAVY DOWN AND HORIZONTAL", /* 2533 */
    "LIGHT UP AND HORIZONTAL", /* 2534 */
    "LEFT HEAVY AND RIGHT UP LIGHT", /* 2535 */
    "RIGHT HEAVY AND LEFT UP LIGHT", /* 2536 */
    "UP LIGHT AND HORIZONTAL HEAVY", /* 2537 */
    "UP HEAVY AND HORIZONTAL LIGHT", /* 2538 */
    "RIGHT LIGHT AND LEFT UP HEAVY", /* 2539 */
    "LEFT LIGHT AND RIGHT UP HEAVY", /* 253A */
    "HEAVY UP AND HORIZONTAL", /* 253B */
    "LIGHT VERTICAL AND HORIZONTAL", /* 253C */
    "LEFT HEAVY AND RIGHT VERTICAL LIGHT", /* 253D */
    "RIGHT HEAVY AND LEFT VERTICAL LIGHT", /* 253E */
    "VERTICAL LIGHT AND HORIZONTAL HEAVY", /* 253F */
    "UP HEAVY AND DOWN HORIZONTAL LIGHT", /* 2540 */
    "DOWN HEAVY AND UP HORIZONTAL LIGHT", /* 2541 */
    "VERTICAL HEAVY AND HORIZONTAL LIGHT", /* 2542 */
    "LEFT UP HEAVY AND RIGHT DOWN LIGHT", /* 2543 */
    "RIGHT UP HEAVY AND LEFT DOWN LIGHT", /* 2544 */
    "LEFT DOWN HEAVY AND RIGHT UP LIGHT", /* 2545 */
    "RIGHT DOWN HEAVY AND LEFT UP LIGHT", /* 2546 */
    "DOWN LIGHT AND UP HORIZONTAL HEAVY", /* 2547 */
    "UP LIGHT AND DOWN HORIZONTAL HEAVY", /* 2548 */
    "RIGHT LIGHT AND LEFT VERTICAL HEAVY", /* 2549 */
    "LEFT LIGHT AND RIGHT VERTICAL HEAVY", /* 254A */
    "HEAVY VERTICAL AND HORIZONTAL", /* 254B */
    "LIGHT DOUBLE DASH HORIZONTAL", /* 254C */
    "HEAVY DOUBLE DASH HORIZONTAL", /* 254D */
    "LIGHT DOUBLE DASH VERTICAL", /* 254E */
    "HEAVY DOUBLE DASH VERTICAL", /* 254F */
    "DOUBLE HORIZONTAL", /* 2550 */
    "DOUBLE VERTICAL", /* 2551 */
    "DOWN SINGLE AND RIGHT DOUBLE", /* 2552 */
    "DOWN DOUBLE AND RIGHT SINGLE", /* 2553 */
    "DOUBLE DOWN AND RIGHT", /* 2554 */
    "DOWN SINGLE AND LEFT DOUBLE", /* 2555 */
    "DOWN DOUBLE AND LEFT SINGLE", /* 2556 */
    "DOUBLE DOWN AND LEFT", /* 2557 */
    "UP SINGLE AND RIGHT DOUBLE", /* 2558 */
    "UP DOUBLE AND RIGHT SINGLE", /* 2559 */
    "DOUBLE UP AND RIGHT", /* 255A */
    "UP SINGLE AND LEFT DOUBLE", /* 255B */
    "UP DOUBLE AND LEFT SINGLE", /* 255C */
    "DOUBLE UP AND LEFT", /* 255D */
    "VERTICAL SINGLE AND RIGHT DOUBLE", /* 255E */
    "VERTICAL DOUBLE AND RIGHT SINGLE", /* 255F */
    "DOUBLE VERTICAL AND RIGHT", /* 2560 */
    "VERTICAL SINGLE AND LEFT DOUBLE", /* 2561 */
    "VERTICAL DOUBLE AND LEFT SINGLE", /* 2562 */
    "DOUBLE VERTICAL AND LEFT", /* 2563 */
    "DOWN SINGLE AND HORIZONTAL DOUBLE", /* 2564 */
    "DOWN DOUBLE AND HORIZONTAL SINGLE", /* 2565 */
    "DOUBLE DOWN AND HORIZONTAL", /* 2566 */
    "UP SINGLE AND HORIZONTAL DOUBLE", /* 2567 */
    "UP DOUBLE AND HORIZONTAL SINGLE", /* 2568 */
    "DOUBLE UP AND HORIZONTAL", /* 2569 */
    "VERTICAL SINGLE AND HORIZONTAL DOUBLE", /* 256A */
    "VERTICAL DOUBLE AND HORIZONTAL SINGLE", /* 256B */
    "DOUBLE VERTICAL AND HORIZONTAL", /* 256C */
    "LIGHT ARC DOWN AND RIGHT", /* 256D */
    "LIGHT ARC DOWN AND LEFT", /* 256E */
    "LIGHT ARC UP AND LEFT", /* 256F */
    "LIGHT ARC UP AND RIGHT", /* 2570 */
    "LIGHT DIAGONAL UPPER RIGHT TO LOWER LEFT", /* 2571 */
    "LIGHT DIAGONAL UPPER LEFT TO LOWER RIGHT", /* 2572 */
    "LIGHT DIAGONAL CROSS", /* 2573 */
    "LIGHT LEFT", /* 2574 */
    "LIGHT UP", /* 2575 */
    "LIGHT RIGHT", /* 2576 */
    "LIGHT DOWN", /* 2577 */
    "HEAVY LEFT", /* 2578 */
    "HEAVY UP", /* 2579 */
    "HEAVY RIGHT", /* 257A */
    "HEAVY DOWN", /* 257B */
    "LIGHT LEFT AND HEAVY RIGHT", /* 257C */
    "LIGHT UP AND HEAVY DOWN", /* 257D */
    "HEAVY LEFT AND LIGHT RIGHT", /* 257E */
    "HEAVY UP AND LIGHT DOWN", /* 257F */
    "UPPER HALF BLOCK", /* 2580 */
    "LOWER ONE EIGHTH BLOCK", /* 2581 */
    "LOWER ONE QUARTER BLOCK", /* 2582 */
    "LOWER THREE EIGHTHS BLOCK", /* 2583 */
    "LOWER HALF BLOCK", /* 2584 */
    "LOWER FIVE EIGHTHS BLOCK", /* 2585 */
    "LOWER THREE QUARTERS BLOCK", /* 2586 */
    "LOWER SEVEN EIGHTHS BLOCK", /* 2587 */
    "FULL BLOCK", /* 2588 */
    "LEFT SEVEN EIGHTHS BLOCK", /* 2589 */
    "LEFT THREE QUARTERS BLOCK", /* 258A */
    "LEFT FIVE EIGHTHS BLOCK", /* 258B */
    "LEFT HALF BLOCK", /* 258C */
    "LEFT THREE EIGHTHS BLOCK", /* 258D */
    "LEFT ONE QUARTER BLOCK", /* 258E */
    "LEFT ONE EIGHTH BLOCK", /* 258F */
    "RIGHT HALF BLOCK", /* 2590 */
    "LIGHT SHADE", /* 2591 */
    "MEDIUM SHADE", /* 2592 */
    "DARK SHADE", /* 2593 */
    "UPPER ONE EIGHTH BLOCK", /* 2594 */
    "RIGHT ONE EIGHTH BLOCK", /* 2595 */
    "QUADRANT LOWER LEFT", /* 2596 */
    "QUADRANT LOWER RIGHT", /* 2597 */
    "QUADRANT UPPER LEFT", /* 2598 */
    "QUADRANT UPPER LEFT AND LOWER LEFT AND LOWER RIGHT", /* 2599 */
    "QUADRANT UPPER LEFT AND LOWER RIGHT", /* 259A */
    "QUADRANT UPPER LEFT AND UPPER RIGHT AND LOWER LEFT", /* 259B */
    "QUADRANT UPPER LEFT AND UPPER RIGHT AND LOWER RIGHT", /* 259C */
    "QUADRANT UPPER RIGHT", /* 259D */
    "QUADRANT UPPER RIGHT AND LOWER LEFT", /* 259E */
    "QUADRANT UPPER RIGHT AND LOWER LEFT AND LOWER RIGHT", /* 259F */
};

enum { LEFT, UP, RIGHT, DOWN };
enum { K_LINES, K_DASH, K_ARC, K_DIAGONAL };

typedef struct {
    int kind;
    int arm[4]; /* 0 none, 1 light or single, 2 heavy, 3 double */
    int dashes, vertical, heavy;
} Expect;

/* Whole-word search in a name. */
static int has_word(const char *s, const char *word) {
    size_t n = strlen(word);
    for (const char *p = strstr(s, word); p; p = strstr(p + 1, word)) {
        if ((p == s || p[-1] == ' ') && (p[n] == ' ' || p[n] == 0)) return 1;
    }
    return 0;
}

/* Parse "LIGHT DOWN AND HEAVY LEFT", "LEFT UP HEAVY AND RIGHT DOWN LIGHT",
   "LIGHT VERTICAL AND HORIZONTAL" (a clause without a weight takes the
   previous clause's), "DOWN SINGLE AND RIGHT DOUBLE", ... */
static Expect expect_box(const char *name) {
    Expect e = {0};
    if (has_word(name, "DASH")) {
        e.kind = K_DASH;
        e.dashes = has_word(name, "DOUBLE") ? 2 : has_word(name, "TRIPLE") ? 3 : 4;
        e.vertical = has_word(name, "VERTICAL");
        e.heavy = has_word(name, "HEAVY");
        return e;
    }
    e.kind = has_word(name, "ARC") ? K_ARC : has_word(name, "DIAGONAL") ? K_DIAGONAL : K_LINES;
    if (e.kind == K_DIAGONAL) return e;
    char buf[128];
    snprintf(buf, sizeof buf, "%s", name);
    int inherited = 0, weight = 0, dirs[4] = {0}, ndirs = 0;
    for (char *tok = strtok(buf, " "); ; tok = strtok(NULL, " ")) {
        if (!tok || !strcmp(tok, "AND")) {
            int w = weight ? weight : inherited;
            for (int i = 0; i < ndirs; i++) e.arm[dirs[i]] = w;
            if (weight) inherited = weight;
            weight = 0;
            ndirs = 0;
            if (!tok) break;
            continue;
        }
        if (!strcmp(tok, "LIGHT") || !strcmp(tok, "SINGLE")) weight = 1;
        else if (!strcmp(tok, "HEAVY")) weight = 2;
        else if (!strcmp(tok, "DOUBLE")) weight = 3;
        else if (!strcmp(tok, "LEFT")) dirs[ndirs++] = LEFT;
        else if (!strcmp(tok, "RIGHT")) dirs[ndirs++] = RIGHT;
        else if (!strcmp(tok, "UP")) dirs[ndirs++] = UP;
        else if (!strcmp(tok, "DOWN")) dirs[ndirs++] = DOWN;
        else if (!strcmp(tok, "HORIZONTAL")) { dirs[ndirs++] = LEFT; dirs[ndirs++] = RIGHT; }
        else if (!strcmp(tok, "VERTICAL")) { dirs[ndirs++] = UP; dirs[ndirs++] = DOWN; }
    }
    if (e.kind == K_ARC) {
        for (int i = 0; i < 4; i++) e.arm[i] = e.arm[i] ? 1 : 0;
    }
    return e;
}

/* ------------------------------------------------------------- painting */

static uint8_t *pixels;
static int cell_w, cell_h, grid_w, grid_h;

static void paint(uint32_t cp, int bold) {
    memset(pixels, 0, (size_t)grid_w * grid_h * 3);
    Canvas cv = {.px = pixels, .filtered = pixels, .w = grid_w, .h = grid_h, .stride = (size_t)grid_w * 3};
    paint_geometry(&cv, 1, 1, cell_w, cell_h, cp, bold, 255, 255, 255);
    for (int k = 0; k < 4; k++) free(cv.arc_offsets[k]);
}

/* A pixel of the grid; (x, y) is relative to the middle cell. */
static const uint8_t *at(int x, int y) {
    return pixels + ((size_t)(cell_h + y) * grid_w + (cell_w + x)) * 3;
}
static int lit(int x, int y) {
    const uint8_t *p = at(x, y);
    return p[0] | p[1] | p[2];
}

static int painted_outside(void) {
    for (int y = -cell_h; y < 2 * cell_h; y++) {
        for (int x = -cell_w; x < 2 * cell_w; x++) {
            int inside = x >= 0 && x < cell_w && y >= 0 && y < cell_h;
            if (!inside && lit(x, y)) return 1;
        }
    }
    return 0;
}

/* The lit pixels along one edge of the cell, as a string of 0 and 1. */
static void edge(int side, char *out) {
    int n = side == LEFT || side == RIGHT ? cell_h : cell_w;
    for (int i = 0; i < n; i++) {
        int x = side == LEFT ? 0 : side == RIGHT ? cell_w - 1 : i;
        int y = side == UP ? 0 : side == DOWN ? cell_h - 1 : i;
        out[i] = lit(x, y) ? '1' : '0';
    }
    out[n] = 0;
}

static int empty(const char *profile) {
    return strchr(profile, '1') == NULL;
}

/* Lit pixels in the cell form one 4-connected shape. */
static int connected(void) {
    static int stack[300 * 300][2];
    static uint8_t seen[300 * 300];
    memset(seen, 0, sizeof seen);
    int total = 0, sx = -1, sy = -1;
    for (int y = 0; y < cell_h; y++) {
        for (int x = 0; x < cell_w; x++) {
            if (lit(x, y)) {
                total++;
                if (sx < 0) sx = x, sy = y;
            }
        }
    }
    if (!total) return 0;
    int top = 0, reached = 0;
    stack[top][0] = sx, stack[top][1] = sy, top++;
    seen[sy * cell_w + sx] = 1;
    while (top) {
        top--;
        int x = stack[top][0], y = stack[top][1];
        reached++;
        const int d[4][2] = {{1, 0}, {-1, 0}, {0, 1}, {0, -1}};
        for (int k = 0; k < 4; k++) {
            int nx = x + d[k][0], ny = y + d[k][1];
            if (nx < 0 || ny < 0 || nx >= cell_w || ny >= cell_h) continue;
            if (seen[ny * cell_w + nx] || !lit(nx, ny)) continue;
            seen[ny * cell_w + nx] = 1;
            stack[top][0] = nx, stack[top][1] = ny, top++;
        }
    }
    return reached == total;
}

/* Runs of lit pixels along the middle row or column. */
static int runs(int vertical) {
    int count = 0, prev = 0, n = vertical ? cell_h : cell_w;
    for (int i = 0; i < n; i++) {
        int on = vertical ? lit(cell_w / 2, i) : lit(i, cell_h / 2);
        if (on && !prev) count++;
        prev = on;
    }
    return count;
}

/* -------------------------------------------------------------- blocks */

static uint8_t mask[300 * 300], mask2[300 * 300];

static void snapshot(uint8_t *m) {
    for (int y = 0; y < cell_h; y++)
        for (int x = 0; x < cell_w; x++) m[y * cell_w + x] = lit(x, y) ? 1 : 0;
}

/* The fraction in eighths a block name gives: "LOWER THREE EIGHTHS BLOCK" 3. */
static int eighths(const char *name) {
    if (has_word(name, "FULL")) return 8;
    if (has_word(name, "HALF")) return 4;
    if (strstr(name, "ONE EIGHTH")) return 1;
    if (strstr(name, "ONE QUARTER")) return 2;
    if (strstr(name, "THREE EIGHTHS")) return 3;
    if (strstr(name, "FIVE EIGHTHS")) return 5;
    if (strstr(name, "THREE QUARTERS")) return 6;
    if (strstr(name, "SEVEN EIGHTHS")) return 7;
    return -1;
}

static int failures, checks;

static void fail(uint32_t cp, int bold, const char *what) {
    failures++;
    if (failures <= 40 || getenv("ALL")) {
        fprintf(stderr, "FAIL U+%04X %s, cell %dx%d%s: %s\n", (unsigned)cp, NAMES[cp - 0x2500], cell_w,
                cell_h, bold ? " bold" : "", what);
    }
}

static void check(int ok, uint32_t cp, int bold, const char *what) {
    checks++;
    if (!ok) fail(cp, bold, what);
}

/* A block anchored at an edge covers whole rows (or columns) from that edge,
   between floor and ceil of the named fraction, and nothing else. */
static void check_anchored(uint32_t cp, const char *name) {
    int n = eighths(name);
    int vertical = has_word(name, "UPPER") || has_word(name, "LOWER") || has_word(name, "FULL");
    int from_end = has_word(name, "LOWER") || has_word(name, "RIGHT");
    int dim = vertical ? cell_h : cell_w, other = vertical ? cell_w : cell_h;
    int extent = 0;
    for (int i = 0; i < dim; i++) {
        int k = from_end ? dim - 1 - i : i, full = 1, none = 1;
        for (int j = 0; j < other; j++) {
            int on = vertical ? lit(j, k) : lit(k, j);
            full &= on;
            none &= !on;
        }
        if (full && extent == i) extent++;
        else check(none, cp, 0, "block is not a solid band from its edge");
    }
    int lo = dim * n / 8, hi = (dim * n + 7) / 8;
    check(extent >= lo && extent <= hi, cp, 0, "block size is not the named fraction");
}

static void check_blocks(void) {
    for (uint32_t cp = 0x2580; cp <= 0x259F; cp++) {
        const char *name = NAMES[cp - 0x2500];
        paint(cp, 0);
        check(!painted_outside(), cp, 0, "painted outside the cell");
        if (has_word(name, "SHADE")) {
            int k = has_word(name, "LIGHT") ? 1 : has_word(name, "MEDIUM") ? 2 : 3;
            int want = (255 * k + 2) / 4, flat = 1;
            for (int y = 0; y < cell_h; y++)
                for (int x = 0; x < cell_w; x++)
                    for (int c = 0; c < 3; c++) flat &= abs(at(x, y)[c] - want) <= 1;
            check(flat, cp, 0, "shade is not a flat mix of the colours");
        } else if (!has_word(name, "QUADRANT")) {
            check_anchored(cp, name);
        }
    }
    /* Complements tile the cell: union is everything, intersection nothing. */
    const uint32_t pairs[][2] = {{0x2580, 0x2584}, {0x2594, 0x2587}, {0x258C, 0x2590}, {0x2589, 0x2595}};
    for (int p = 0; p < 4; p++) {
        paint(pairs[p][0], 0);
        snapshot(mask);
        paint(pairs[p][1], 0);
        snapshot(mask2);
        int tiles = 1;
        for (int i = 0; i < cell_w * cell_h; i++) tiles &= mask[i] + mask2[i] == 1;
        check(tiles, pairs[p][0], 0, "does not tile the cell with its complement");
    }
    /* Quadrants: the four singles tile the cell, and each combination is the
       union of the quadrants its name lists. */
    const uint32_t single[4] = {0x2598, 0x259D, 0x2596, 0x2597}; /* UL UR LL LR */
    const char *words[4] = {"UPPER LEFT", "UPPER RIGHT", "LOWER LEFT", "LOWER RIGHT"};
    static uint8_t quadrant[4][300 * 300];
    for (int q = 0; q < 4; q++) {
        paint(single[q], 0);
        snapshot(quadrant[q]);
    }
    int tiles = 1;
    for (int i = 0; i < cell_w * cell_h; i++)
        tiles &= quadrant[0][i] + quadrant[1][i] + quadrant[2][i] + quadrant[3][i] == 1;
    check(tiles, 0x2596, 0, "the four quadrants do not tile the cell");
    for (uint32_t cp = 0x2596; cp <= 0x259F; cp++) {
        const char *name = NAMES[cp - 0x2500];
        paint(cp, 0);
        snapshot(mask);
        int same = 1;
        for (int i = 0; i < cell_w * cell_h; i++) {
            int want = 0;
            for (int q = 0; q < 4; q++) want |= strstr(name, words[q]) ? quadrant[q][i] : 0;
            same &= mask[i] == want;
        }
        check(same, cp, 0, "quadrant combination is not the union of its quadrants");
    }
}

/* --------------------------------------------------------------- lines */

static char ref_across[2][4][300], ref_down[2][4][300]; /* [bold][weight] */
static Expect expected[128];
static uint8_t shape[300 * 300], other[300 * 300];

/* The line character with these arms, or 0. */
static uint32_t with_arms(const int arm[4]) {
    for (int i = 0; i < 128; i++) {
        if (expected[i].kind == K_LINES && !memcmp(expected[i].arm, arm, sizeof expected[i].arm)) return 0x2500 + i;
    }
    return 0;
}

/* The first and last lit index of a profile, and its runs. */
static void runs_of(const char *p, int *first, int *last, int *count) {
    *first = -1, *last = -1, *count = 0;
    for (int i = 0; p[i]; i++) {
        if (p[i] == '1') {
            if (*first < 0) *first = i;
            if (i == 0 || p[i - 1] != '1') (*count)++;
            *last = i;
        }
    }
}

/* Joins, checked without draw.c's own arithmetic:
   - a pure light (or pure heavy) character is exactly the union of its half
     lines ╴╵╶╷ (╸╹╺╻);
   - a character mirrors onto its mirror image (┌ onto ┐), where the stroke
     width is odd so a centred stroke is symmetric;
   - in a light/heavy mix every arm runs from its edge through the centre
     line, and nothing lies outside the bands its arms define;
   - the gap of a double arm stays empty from the edge to the centre. */
static void check_joins(uint32_t cp, int bold) {
    Expect e = expected[cp - 0x2500];
    int heavy = 0, light = 0, doubled = 0;
    for (int s = 0; s < 4; s++) {
        heavy |= e.arm[s] == 2;
        light |= e.arm[s] == 1;
        doubled |= e.arm[s] == 3;
    }
    paint(cp, bold);
    snapshot(shape);
    int cx = cell_w / 2, cy = cell_h / 2;
    if (!doubled && (!heavy || !light)) {
        const uint32_t halves[2][4] = {{0x2574, 0x2575, 0x2576, 0x2577}, {0x2578, 0x2579, 0x257A, 0x257B}};
        memset(other, 0, sizeof other);
        for (int s = 0; s < 4; s++) {
            if (!e.arm[s]) continue;
            paint(halves[heavy][s], bold);
            snapshot(mask);
            for (int i = 0; i < cell_w * cell_h; i++) other[i] |= mask[i];
        }
        check(!memcmp(shape, other, (size_t)(cell_w * cell_h)), cp, bold, "is not the union of its half lines");
    }
    if (!doubled && heavy && light) {
        int inside = 1, reaches = 1;
        for (int y = 0; y < cell_h; y++) {
            for (int x = 0; x < cell_w; x++) {
                /* A pixel may be lit only in the horizontal band (if there is a
                   horizontal arm) or the vertical band. */
                int wide = 0;
                for (int s = 0; s < 4; s++) {
                    if (!e.arm[s]) continue;
                    const char *ref = s == LEFT || s == RIGHT ? ref_across[bold][e.arm[s]] : ref_down[bold][e.arm[s]];
                    wide |= (s == LEFT || s == RIGHT) ? ref[y] == '1' : ref[x] == '1';
                }
                inside &= !shape[y * cell_w + x] || wide;
            }
        }
        for (int s = 0; s < 4; s++) {
            if (!e.arm[s]) continue;
            const char *ref = s == LEFT || s == RIGHT ? ref_across[bold][e.arm[s]] : ref_down[bold][e.arm[s]];
            int n = s == LEFT || s == RIGHT ? cell_h : cell_w;
            for (int k = 0; k < n; k++) {
                if (ref[k] != '1') continue;
                int from = s == LEFT ? 0 : s == RIGHT ? cx : s == UP ? 0 : cy;
                int to = s == LEFT ? cx : s == RIGHT ? cell_w - 1 : s == UP ? cy : cell_h - 1;
                for (int i = from; i <= to; i++) {
                    reaches &= s == LEFT || s == RIGHT ? shape[k * cell_w + i] : shape[i * cell_w + k];
                }
            }
        }
        check(inside, cp, bold, "paints outside the bands of its arms");
        check(reaches, cp, bold, "an arm does not reach the centre");
    }
    if (doubled) {
        /* The gap rows of ═ (columns of ║) are empty from each double arm's
           edge to where the centre square starts. */
        int first, last, count, clear = 1;
        for (int s = 0; s < 4; s++) {
            if (e.arm[s] != 3) continue;
            const char *ref = s == LEFT || s == RIGHT ? ref_across[bold][3] : ref_down[bold][3];
            runs_of(ref, &first, &last, &count);
            int span = last - first + 1;
            int along = s == LEFT || s == RIGHT ? cx : cy;
            for (int k = first; k <= last; k++) {
                if (ref[k] == '1') continue; /* a gap row or column */
                int a = s == LEFT || s == UP ? 0 : along + span - span / 2;
                int b = s == LEFT || s == UP ? along - span / 2 : (s == RIGHT ? cell_w : cell_h);
                for (int i = a; i < b; i++) clear &= !(s == LEFT || s == RIGHT ? shape[k * cell_w + i] : shape[i * cell_w + k]);
            }
        }
        check(clear, cp, bold, "a double arm's gap is filled");
    }
    /* Mirror images, where a centred odd-width stroke is symmetric. */
    int t = cell_w / 12 < 1 ? 1 : cell_w / 12;
    if (bold) t++;
    if (!heavy && t % 2 == 1) {
        int flip_h[4] = {e.arm[RIGHT], e.arm[UP], e.arm[LEFT], e.arm[DOWN]};
        int flip_v[4] = {e.arm[LEFT], e.arm[DOWN], e.arm[RIGHT], e.arm[UP]};
        uint32_t mirror = with_arms(flip_h);
        if (mirror && cell_w % 2 == 1) {
            paint(mirror, bold);
            int same = 1;
            for (int y = 0; y < cell_h; y++)
                for (int x = 0; x < cell_w; x++) same &= shape[y * cell_w + x] == (lit(cell_w - 1 - x, y) ? 1 : 0);
            check(same, cp, bold, "is not the left-right mirror of its mirror character");
        }
        mirror = with_arms(flip_v);
        if (mirror && cell_h % 2 == 1) {
            paint(mirror, bold);
            int same = 1;
            for (int y = 0; y < cell_h; y++)
                for (int x = 0; x < cell_w; x++) same &= shape[y * cell_w + x] == (lit(x, cell_h - 1 - y) ? 1 : 0);
            check(same, cp, bold, "is not the top-bottom mirror of its mirror character");
        }
    }
}

static void check_lines(int bold) {
    /* Reference cross-sections: the left edge of ─ ━ ═, the top edge of │ ┃ ║. */
    const uint32_t across[4] = {0, 0x2500, 0x2501, 0x2550}, down[4] = {0, 0x2502, 0x2503, 0x2551};
    for (int w = 1; w <= 3; w++) {
        paint(across[w], bold);
        edge(LEFT, ref_across[bold][w]);
        paint(down[w], bold);
        edge(UP, ref_down[bold][w]);
    }
    char profile[300];
    for (uint32_t cp = 0x2500; cp <= 0x257F; cp++) {
        const char *name = NAMES[cp - 0x2500];
        Expect e = expect_box(name);
        paint(cp, bold);
        check(!painted_outside(), cp, bold, "painted outside the cell");
        /* Below 6 pixels wide (about px 12) a centred stroke already touches
           the cell edges, so only containment is checked there. */
        if (cell_w < 6) continue;
        if (e.kind == K_LINES) {
            int doubled = 0;
            for (int side = 0; side < 4; side++) {
                edge(side, profile);
                int weight = e.arm[side];
                doubled |= weight == 3;
                const char *want = side == LEFT || side == RIGHT ? ref_across[bold][weight] : ref_down[bold][weight];
                char what[64];
                snprintf(what, sizeof what, "%s edge does not match a %s line", (const char *[]){"left", "top", "right", "bottom"}[side],
                         (const char *[]){"missing", "light", "heavy", "double"}[weight]);
                check(weight ? !strcmp(profile, want) : empty(profile), cp, bold, what);
            }
            if (!doubled) check(connected(), cp, bold, "line character is not one connected shape");
            check_joins(cp, bold);
        } else if (e.kind == K_DASH) {
            int slot = (e.vertical ? cell_h : cell_w) / e.dashes;
            if (slot >= 3) {
                check(runs(e.vertical) == e.dashes, cp, bold, "wrong number of dash segments");
                edge(e.vertical ? UP : LEFT, profile);
                check(empty(profile), cp, bold, "dash touches the cell edge");
            }
        } else if (e.kind == K_ARC) {
            for (int side = 0; side < 4; side++) {
                edge(side, profile);
                if (!e.arm[side]) {
                    check(empty(profile), cp, bold, "arc reaches an edge it does not name");
                    continue;
                }
                /* The arc meets a straight light line: their cross-sections overlap. */
                const char *want = side == LEFT || side == RIGHT ? ref_across[bold][1] : ref_down[bold][1];
                int overlap = 0;
                for (int i = 0; profile[i]; i++) overlap |= profile[i] == '1' && want[i] == '1';
                check(overlap, cp, bold, "arc does not meet a light line at its edge");
            }
        } else if (e.kind == K_DIAGONAL) {
            int rising = has_word(name, "CROSS") || strstr(name, "UPPER RIGHT TO LOWER LEFT");
            int falling = has_word(name, "CROSS") || strstr(name, "UPPER LEFT TO LOWER RIGHT");
            check(!rising || (lit(cell_w - 1, 0) && lit(0, cell_h - 1)), cp, bold, "diagonal misses its corners");
            check(!falling || (lit(0, 0) && lit(cell_w - 1, cell_h - 1)), cp, bold, "diagonal misses its corners");
            check(lit(cell_w / 2, cell_h / 2), cp, bold, "diagonal misses the centre");
        }
    }
}

int main(void) {
    /* Odd and even sizes, from degenerate to px 255. */
    const int sizes[][2] = {{1, 2}, {2, 5}, {3, 7}, {4, 9}, {5, 11}, {6, 13}, {7, 15}, {8, 17}, {9, 19}, {10, 22},
                            {11, 24}, {12, 25}, {13, 27}, {14, 30}, {16, 34}, {22, 48}, {23, 49}, {29, 61},
                            {37, 80}, {58, 128}, {116, 255}};
    int nsizes = (int)(sizeof sizes / sizeof sizes[0]);
    for (int i = 0; i < 128; i++) expected[i] = expect_box(NAMES[i]);
    for (int s = 0; s < nsizes; s++) {
        cell_w = sizes[s][0];
        cell_h = sizes[s][1];
        grid_w = 3 * cell_w;
        grid_h = 3 * cell_h;
        pixels = malloc((size_t)grid_w * grid_h * 3);
        check_lines(0);
        check_lines(1);
        check_blocks();
        free(pixels);
    }
    if (failures) {
        printf("FAIL %d of %d box-drawing checks\n", failures, checks);
        return 1;
    }
    printf("ok, %d box-drawing checks over %d cell sizes\n", checks, nsizes);
    return 0;
}
