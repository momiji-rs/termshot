/* CFF (Type 2 charstring) outlines, the C side of the #25 POC
   (docs/cff-rust-vs-c.md). For a well-formed font it produces exactly the
   vertices and glyph box stb_truetype 1.26 does; for anything else it returns
   an error where stb would read out of bounds, assert, or run without end.
   cff.rs is the same design in Rust. */
#include "cff.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

/* See cff.rs. */
#define MAX_OPS 20000
#define MAX_STACK 48
#define MAX_SUBR_DEPTH 10

enum { MOVE = 1, LINE = 2, CUBIC = 4 };

/* A byte range; every one made here lies inside the table. */
typedef struct {
    const uint8_t *data;
    size_t len;
} span;

static int u16_at(span d, size_t at, size_t *out) {
    if (at > d.len || d.len - at < 2) return 0;
    *out = (size_t)d.data[at] << 8 | d.data[at + 1];
    return 1;
}

static int u32_at(span d, size_t at, size_t *out) {
    if (at > d.len || d.len - at < 4) return 0;
    *out = (size_t)d.data[at] << 24 | (size_t)d.data[at + 1] << 16 | (size_t)d.data[at + 2] << 8 | d.data[at + 3];
    return 1;
}

static int sub(span d, size_t at, size_t len, span *out) {
    if (at > d.len || len > d.len - at) return 0;
    out->data = d.data + at;
    out->len = len;
    return 1;
}

/* A CFF INDEX. Its extent is checked to lie inside the table; each entry is
   checked when it is fetched, so one bad offset costs one glyph, as in stb. */
typedef struct {
    span cff;
    size_t count, offsize;
    size_t offsets; /* where the offset array starts */
    size_t base;    /* the byte before the first object */
    size_t end;     /* just past the last object */
} cff_index;

static size_t index_offset(const cff_index *x, size_t i) {
    size_t v = 0, at = x->offsets + i * x->offsize;
    for (size_t k = 0; k < x->offsize; k++) v = v << 8 | x->cff.data[at + k];
    return v;
}

/* Read the INDEX at `at`; *end is the offset just past it. */
static const char *index_read(span cff, size_t at, cff_index *x, size_t *end) {
    size_t count;
    memset(x, 0, sizeof *x);
    x->cff = cff;
    if (!u16_at(cff, at, &count)) return "INDEX truncated";
    if (count == 0) {
        if (end) *end = at + 2;
        return NULL;
    }
    if (at + 2 >= cff.len) return "INDEX truncated";
    size_t offsize = cff.data[at + 2];
    if (offsize < 1 || offsize > 4) return "INDEX offset size";
    x->count = count;
    x->offsize = offsize;
    x->offsets = at + 3;
    x->base = x->offsets + (count + 1) * offsize - 1;
    if (x->base >= cff.len) return "INDEX runs past the table";
    size_t last = index_offset(x, count);
    if (last > cff.len - x->base) return "INDEX runs past the table";
    x->end = x->base + last;
    if (end) *end = x->end;
    return NULL;
}

/* Entry i; empty where its offsets are out of order or out of range, as
   stbtt__cff_index_get gives it. */
static int index_get(const cff_index *x, size_t i, span *out) {
    if (i >= x->count) return 0;
    size_t start = x->base + index_offset(x, i), stop = x->base + index_offset(x, i + 1);
    int ok = start <= stop && stop <= x->end;
    out->data = x->cff.data + (ok ? start : 0); /* no pointer past the table */
    out->len = ok ? stop - start : 0;
    return 1;
}

/* The integer operands of the first `key` entry of a DICT, read the way
   stbtt__dict_get does; zero where there are fewer than n. stb asserts on
   bytes 31 and 255 and on a real number where it reads an integer. */
static const char *dict_ints(span d, int key, size_t n, long long *out, int *found) {
    size_t at = 0;
    for (size_t k = 0; k < n; k++) out[k] = 0;
    if (found) *found = 0;
    while (at < d.len) {
        long long ints[48];
        int reals[48];
        size_t operands = 0;
        for (;;) {
            int b0 = at < d.len ? d.data[at] : 0;
            long long v;
            size_t len;
            int real = 0;
            if (b0 < 28) break;
            if (b0 == 28) {
                size_t u;
                if (!u16_at(d, at + 1, &u)) return "DICT truncated";
                v = (long long)u;
                len = 3;
            } else if (b0 == 29) {
                size_t u;
                if (!u32_at(d, at + 1, &u)) return "DICT truncated";
                v = (long long)u;
                len = 5;
            } else if (b0 == 30) {
                size_t end = at + 1;
                while (end < d.len) {
                    int nibbles = d.data[end++];
                    if ((nibbles & 0xf) == 0xf || nibbles >> 4 == 0xf) break;
                }
                v = 0;
                len = end - at;
                real = 1;
            } else if (b0 >= 32 && b0 <= 246) {
                v = b0 - 139;
                len = 1;
            } else if (b0 >= 247 && b0 <= 254) {
                if (at + 1 >= d.len) return "DICT truncated";
                int b1 = d.data[at + 1];
                v = b0 <= 250 ? (b0 - 247) * 256 + b1 + 108 : -(b0 - 251) * 256 - b1 - 108;
                len = 2;
            } else {
                return "DICT operand byte";
            }
            if (operands < 48) {
                ints[operands] = v;
                reals[operands] = real;
                operands++;
            }
            at += len;
        }
        int op = at < d.len ? d.data[at] : 0;
        at++;
        if (op == 12) {
            op = 0x100 | (at < d.len ? d.data[at] : 0);
            at++;
        }
        if (op == key) {
            for (size_t k = 0; k < n && k < operands; k++) {
                if (reals[k]) return "DICT value is a real number";
                if (ints[k] < 0) return "DICT value is negative";
                out[k] = ints[k];
            }
            if (found) *found = operands > 0;
            return NULL;
        }
    }
    return NULL;
}

/* The local Subrs of a Top DICT or Font DICT, as stbtt__get_subrs finds them. */
static const char *read_subrs(span cff, span dict, cff_index *out) {
    long long private[2], local;
    span pdict;
    const char *error;
    memset(out, 0, sizeof *out);
    out->cff = cff;
    if ((error = dict_ints(dict, 18, 2, private, NULL))) return error;
    if (private[0] == 0 || private[1] == 0) return NULL;
    if (!sub(cff, (size_t)private[1], (size_t)private[0], &pdict)) return "Private DICT runs past the table";
    if ((error = dict_ints(pdict, 19, 1, &local, NULL))) return error;
    if (local == 0) return NULL;
    return index_read(cff, (size_t)private[1] + (size_t)local, out, NULL);
}

struct cff_font {
    int glyphs;
    cff_index charstrings, gsubrs, subrs;
    cff_index *fd_subrs;
    size_t fds;
    /* 0: not a CID font. Otherwise the FDSelect format, 0 or 3. */
    int cid, fdformat;
    span fd0;               /* format 0: one byte per glyph */
    size_t *firsts, *range_fd, ranges;  /* format 3 */
};

/* The first table with this tag, as stbtt__find_table picks it. */
static const char *find_table(span d, size_t start, const char *tag, span *out, int *found) {
    size_t count;
    *found = 0;
    if (!u16_at(d, start + 4, &count)) return "table directory truncated";
    for (size_t i = 0; i < count; i++) {
        size_t record = start + 12 + 16 * i, offset, length;
        if (!u32_at(d, record + 8, &offset) || !u32_at(d, record + 12, &length))
            return "table directory runs past the end of the file";
        if (memcmp(d.data + record, tag, 4) != 0) continue;
        if (offset == 0) return NULL;
        if (!sub(d, offset, length, out)) return "table runs past the end of the file";
        *found = 1;
        return NULL;
    }
    return NULL;
}

static const char *parse(cff_font *f, span d, int face) {
    size_t start = 0, word, at, hdr;
    span cff, maxp, ignored, top;
    int found;
    const char *error;
    if (d.len < 4) return "file is shorter than a font header";
    if (memcmp(d.data, "ttcf", 4) == 0) {
        size_t faces;
        if (!u32_at(d, 4, &word) || (word != 0x10000 && word != 0x20000)) return "not a TrueType or OpenType file";
        if (!u32_at(d, 8, &faces)) return "collection header truncated";
        if (face < 0 || (size_t)face >= faces) return "no such face in the collection";
        if (!u32_at(d, 12 + 4 * (size_t)face, &start)) return "collection header truncated";
    } else if (memcmp(d.data, "OTTO", 4) == 0 || memcmp(d.data, "\0\1\0\0", 4) == 0 || memcmp(d.data, "true", 4) == 0 ||
               memcmp(d.data, "typ1", 4) == 0 || memcmp(d.data, "1\0\0\0", 4) == 0) {
        if (face != 0) return "the file has one face";
    } else {
        return "not a TrueType or OpenType file";
    }
    if ((error = find_table(d, start, "glyf", &ignored, &found))) return error;
    if (found) return "TrueType outlines, not CFF";
    if ((error = find_table(d, start, "CFF2", &ignored, &found))) return error;
    if (found) return "CFF2 (variable) outlines are not supported";
    if ((error = find_table(d, start, "CFF ", &cff, &found))) return error;
    if (!found) return "no CFF table";
    if ((error = find_table(d, start, "maxp", &maxp, &found))) return error;
    f->glyphs = 0xffff;
    if (found) {
        if (!u16_at(maxp, 4, &word)) return "maxp truncated";
        f->glyphs = (int)word;
    }
    if (cff.len < 3) return "CFF header truncated";
    hdr = cff.data[2];
    cff_index names, tops, strings;
    if ((error = index_read(cff, hdr, &names, &at))) return error;
    if ((error = index_read(cff, at, &tops, &at))) return error;
    if (!index_get(&tops, 0, &top)) return "no Top DICT";
    if ((error = index_read(cff, at, &strings, &at))) return error;
    if ((error = index_read(cff, at, &f->gsubrs, NULL))) return error;
    long long charstrings, cstype = 2, fdarray, fdselect;
    if ((error = dict_ints(top, 17, 1, &charstrings, NULL))) return error;
    if ((error = dict_ints(top, 0x106, 1, &cstype, &found))) return error;
    if (!found) cstype = 2;
    if ((error = dict_ints(top, 0x124, 1, &fdarray, NULL))) return error;
    if ((error = dict_ints(top, 0x125, 1, &fdselect, NULL))) return error;
    if ((error = read_subrs(cff, top, &f->subrs))) return error;
    if (cstype != 2) return "charstring type is not 2";
    if (charstrings == 0) return "no CharStrings";
    if ((error = index_read(cff, (size_t)charstrings, &f->charstrings, NULL))) return error;
    if (f->charstrings.count < (size_t)f->glyphs) return "fewer charstrings than glyphs";
    if (fdarray == 0) return NULL;

    if (fdselect == 0) return "FDArray without FDSelect";
    cff_index dicts;
    if ((error = index_read(cff, (size_t)fdarray, &dicts, NULL))) return error;
    f->fds = dicts.count;
    f->fd_subrs = calloc(dicts.count ? dicts.count : 1, sizeof *f->fd_subrs);
    if (!f->fd_subrs) return "out of memory";
    for (size_t i = 0; i < dicts.count; i++) {
        span dict;
        index_get(&dicts, i, &dict);
        if ((error = read_subrs(cff, dict, &f->fd_subrs[i]))) return error;
    }
    f->cid = 1;
    size_t sel = (size_t)fdselect;
    if (sel >= cff.len) return "FDSelect is past the table";
    f->fdformat = cff.data[sel];
    if (f->fdformat == 0) {
        if (!sub(cff, sel + 1, (size_t)f->glyphs, &f->fd0)) return "FDSelect runs past the table";
        for (size_t g = 0; g < f->fd0.len; g++)
            if (f->fd0.data[g] >= f->fds) return "FDSelect names a missing font dict";
    } else if (f->fdformat == 3) {
        size_t n, sentinel;
        if (!u16_at(cff, sel + 1, &n)) return "FDSelect truncated";
        f->firsts = malloc((n ? n : 1) * sizeof *f->firsts);
        f->range_fd = malloc((n ? n : 1) * sizeof *f->range_fd);
        if (!f->firsts || !f->range_fd) return "out of memory";
        for (size_t i = 0; i < n; i++) {
            size_t r = sel + 3 + 3 * i, first;
            if (!u16_at(cff, r, &first) || r + 2 >= cff.len) return "FDSelect truncated";
            size_t fd = cff.data[r + 2];
            if (fd >= f->fds || (i == 0 ? first != 0 : first <= f->firsts[i - 1])) return "FDSelect range is invalid";
            f->firsts[i] = first;
            f->range_fd[i] = fd;
        }
        f->ranges = n;
        if (!u16_at(cff, sel + 3 + 3 * n, &sentinel)) return "FDSelect truncated";
        if (n == 0 || sentinel <= f->firsts[n - 1] || sentinel < (size_t)f->glyphs)
            return "FDSelect does not cover every glyph";
    } else {
        return "FDSelect format";
    }
    return NULL;
}

cff_font *cff_parse(const uint8_t *data, size_t len, int face, const char **error) {
    cff_font *f = calloc(1, sizeof *f);
    span d = {data, len};
    if (!f) {
        *error = "out of memory";
        return NULL;
    }
    if ((*error = parse(f, d, face))) {
        cff_free(f);
        return NULL;
    }
    return f;
}

int cff_glyph_count(const cff_font *f) { return f->glyphs; }

void cff_free(cff_font *f) {
    if (!f) return;
    free(f->fd_subrs);
    free(f->firsts);
    free(f->range_fd);
    free(f);
}

static const cff_index *local_subrs(const cff_font *f, int glyph) {
    if (!f->cid) return &f->subrs;
    if (f->fdformat == 0) return &f->fd_subrs[f->fd0.data[glyph]];
    size_t lo = 0, hi = f->ranges; /* the last range whose first <= glyph */
    while (hi - lo > 1) {
        size_t mid = lo + (hi - lo) / 2;
        if (f->firsts[mid] <= (size_t)glyph) lo = mid; else hi = mid;
    }
    return &f->fd_subrs[f->range_fd[lo]];
}

/* stbtt__csctx, with both of stb's passes in one. */
typedef struct {
    cff_outline *out;
    int started, failed;
    float first_x, first_y, x, y;
    int32_t min_x, max_x, min_y, max_y;
} pen;

/* (int)f, without C's undefined behaviour when f is out of range. */
static int32_t to_int(float f) {
    if (!(f > -2147483648.0f)) return f != f ? 0 : INT32_MIN;
    if (!(f < 2147483648.0f)) return INT32_MAX;
    return (int32_t)f;
}

static void track(pen *c, int32_t x, int32_t y) {
    if (x > c->max_x || !c->started) c->max_x = x;
    if (y > c->max_y || !c->started) c->max_y = y;
    if (x < c->min_x || !c->started) c->min_x = x;
    if (y < c->min_y || !c->started) c->min_y = y;
    c->started = 1;
}

static void vertex(pen *c, int type, int32_t x, int32_t y, int32_t cx, int32_t cy, int32_t cx1, int32_t cy1) {
    track(c, x, y);
    if (type == CUBIC) {
        track(c, cx, cy);
        track(c, cx1, cy1);
    }
    cff_outline *o = c->out;
    if (o->len == o->cap) {
        size_t cap = o->cap ? o->cap * 2 : 64;
        cff_vertex *v = realloc(o->v, cap * sizeof *v);
        if (!v) {
            c->failed = 1;
            return;
        }
        o->v = v;
        o->cap = cap;
    }
    /* Conversions to int16_t keep the low 16 bits, as stb's casts do on
       every compiler we build with. */
    cff_vertex v = {(int16_t)x, (int16_t)y, (int16_t)cx, (int16_t)cy, (int16_t)cx1, (int16_t)cy1, (uint8_t)type, 0};
    o->v[o->len++] = v;
}

static void close_shape(pen *c) {
    if (c->first_x != c->x || c->first_y != c->y) vertex(c, LINE, to_int(c->first_x), to_int(c->first_y), 0, 0, 0, 0);
}

static void move_to(pen *c, float dx, float dy) {
    close_shape(c);
    c->first_x = c->x = c->x + dx;
    c->first_y = c->y = c->y + dy;
    vertex(c, MOVE, to_int(c->x), to_int(c->y), 0, 0, 0, 0);
}

static void line_to(pen *c, float dx, float dy) {
    c->x += dx;
    c->y += dy;
    vertex(c, LINE, to_int(c->x), to_int(c->y), 0, 0, 0, 0);
}

static void curve_to(pen *c, float dx1, float dy1, float dx2, float dy2, float dx3, float dy3) {
    float cx1 = c->x + dx1, cy1 = c->y + dy1;
    float cx2 = cx1 + dx2, cy2 = cy1 + dy2;
    c->x = cx2 + dx3;
    c->y = cy2 + dy3;
    vertex(c, CUBIC, to_int(c->x), to_int(c->y), to_int(cx1), to_int(cy1), to_int(cx2), to_int(cy2));
}

/* stbtt__buf over one charstring: reads past the end give 0 and a seek past
   the end stops at it, as in stb (where the seek also asserts). */
typedef struct {
    span s;
    size_t at;
} cursor;

static uint32_t get8(cursor *b) { return b->at < b->s.len ? b->s.data[b->at++] : 0; }

static uint32_t getn(cursor *b, int n) {
    uint32_t v = 0;
    for (int i = 0; i < n; i++) v = v << 8 | get8(b);
    return v;
}

static int get_subr(const cff_index *x, int n, span *out) {
    long long bias = x->count >= 33900 ? 32768 : x->count >= 1240 ? 1131 : 107;
    long long k = (long long)n + bias;
    if (k < 0 || !index_get(x, (size_t)k, out)) return 0;
    return out->len != 0;
}

/* stbtt__run_charstring: 1 drawn, 0 where stb returns 0, -1 over budget. */
static int run(const cff_font *f, int glyph, pen *c) {
    int in_header = 1, sp = 0, depth = 0;
    size_t maskbits = 0;
    unsigned ops = 0;
    float s[MAX_STACK];
    cursor calls[MAX_SUBR_DEPTH], b = {{NULL, 0}, 0};
    const cff_index *local = NULL;
    index_get(&f->charstrings, (size_t)glyph, &b.s);
    while (b.at < b.s.len) {
        if (++ops > MAX_OPS) return -1;
        int i = 0, clear = 1;
        uint32_t b0 = get8(&b);
        switch (b0) {
        case 0x13: /* hintmask */
        case 0x14: /* cntrmask */
            if (in_header) maskbits += (size_t)(sp / 2);
            in_header = 0;
            {
                size_t skip = (maskbits + 7) / 8;
                b.at = skip > b.s.len - b.at ? b.s.len : b.at + skip;
            }
            break;
        case 0x01: case 0x03: case 0x12: case 0x17: /* hstem, vstem, hstemhm, vstemhm */
            maskbits += (size_t)(sp / 2);
            break;
        case 0x15:
            in_header = 0;
            if (sp < 2) return 0;
            move_to(c, s[sp - 2], s[sp - 1]);
            break;
        case 0x04:
            in_header = 0;
            if (sp < 1) return 0;
            move_to(c, 0, s[sp - 1]);
            break;
        case 0x16:
            in_header = 0;
            if (sp < 1) return 0;
            move_to(c, s[sp - 1], 0);
            break;
        case 0x05:
            if (sp < 2) return 0;
            for (; i + 1 < sp; i += 2) line_to(c, s[i], s[i + 1]);
            break;
        case 0x06: /* hlineto */
        case 0x07: /* vlineto */
            if (sp < 1) return 0;
            for (int horizontal = b0 == 0x06; i < sp; i++, horizontal = !horizontal) {
                if (horizontal) line_to(c, s[i], 0);
                else line_to(c, 0, s[i]);
            }
            break;
        case 0x1e: /* vhcurveto */
        case 0x1f: /* hvcurveto */
            if (sp < 4) return 0;
            for (int vertical = b0 == 0x1e; i + 3 < sp; i += 4, vertical = !vertical) {
                float last = sp - i == 5 ? s[i + 4] : 0.0f;
                if (vertical) curve_to(c, 0, s[i], s[i + 1], s[i + 2], s[i + 3], last);
                else curve_to(c, s[i], 0, s[i + 1], s[i + 2], last, s[i + 3]);
            }
            break;
        case 0x08:
            if (sp < 6) return 0;
            for (; i + 5 < sp; i += 6) curve_to(c, s[i], s[i + 1], s[i + 2], s[i + 3], s[i + 4], s[i + 5]);
            break;
        case 0x18: /* rcurveline */
            if (sp < 8) return 0;
            for (; i + 5 < sp - 2; i += 6) curve_to(c, s[i], s[i + 1], s[i + 2], s[i + 3], s[i + 4], s[i + 5]);
            if (i + 1 >= sp) return 0;
            line_to(c, s[i], s[i + 1]);
            break;
        case 0x19: /* rlinecurve */
            if (sp < 8) return 0;
            for (; i + 1 < sp - 6; i += 2) line_to(c, s[i], s[i + 1]);
            if (i + 5 >= sp) return 0;
            curve_to(c, s[i], s[i + 1], s[i + 2], s[i + 3], s[i + 4], s[i + 5]);
            break;
        case 0x1a: /* vvcurveto */
        case 0x1b: /* hhcurveto */
        {
            if (sp < 4) return 0;
            float lead = 0;
            if (sp & 1) lead = s[i++];
            for (; i + 3 < sp; i += 4) {
                if (b0 == 0x1b) curve_to(c, s[i], lead, s[i + 1], s[i + 2], s[i + 3], 0);
                else curve_to(c, lead, s[i], s[i + 1], s[i + 2], 0, s[i + 3]);
                lead = 0;
            }
            break;
        }
        case 0x0a: /* callsubr */
        case 0x1d: /* callgsubr */
        {
            const cff_index *subrs = &f->gsubrs;
            if (b0 == 0x0a) {
                if (!local) local = local_subrs(f, glyph);
                subrs = local;
            }
            if (sp < 1) return 0;
            float n = s[--sp];
            if (depth >= MAX_SUBR_DEPTH) return 0;
            calls[depth++] = b;
            span body;
            if (!get_subr(subrs, to_int(n), &body)) return 0;
            b.s = body;
            b.at = 0;
            clear = 0;
            break;
        }
        case 0x0b: /* return */
            if (depth <= 0) return 0;
            b = calls[--depth];
            clear = 0;
            break;
        case 0x0e: /* endchar */
            close_shape(c);
            return 1;
        case 0x0c: {
            uint32_t b1 = get8(&b);
            if (b1 == 0x22) { /* hflex */
                if (sp < 7) return 0;
                curve_to(c, s[0], 0, s[1], s[2], s[3], 0);
                curve_to(c, s[4], 0, s[5], -s[2], s[6], 0);
            } else if (b1 == 0x23) { /* flex */
                if (sp < 13) return 0;
                curve_to(c, s[0], s[1], s[2], s[3], s[4], s[5]);
                curve_to(c, s[6], s[7], s[8], s[9], s[10], s[11]);
            } else if (b1 == 0x24) { /* hflex1 */
                if (sp < 9) return 0;
                curve_to(c, s[0], s[1], s[2], s[3], s[4], 0);
                curve_to(c, s[5], 0, s[6], s[7], s[8], -(s[1] + s[3] + s[7]));
            } else if (b1 == 0x25) { /* flex1 */
                if (sp < 11) return 0;
                float dx = s[0] + s[2] + s[4] + s[6] + s[8];
                float dy = s[1] + s[3] + s[5] + s[7] + s[9];
                float dx6 = s[10], dy6 = s[10];
                if (fabs(dx) > fabs(dy)) dy6 = -dy;
                else dx6 = -dx;
                curve_to(c, s[0], s[1], s[2], s[3], s[4], s[5]);
                curve_to(c, s[6], s[7], s[8], s[9], dx6, dy6);
            } else {
                return 0;
            }
            break;
        }
        default: {
            float v;
            if (b0 != 255 && b0 != 28 && b0 < 32) return 0; /* reserved operator */
            if (b0 == 255) v = (float)(int32_t)getn(&b, 4) / 0x10000;
            else if (b0 == 28) v = (float)(int16_t)getn(&b, 2);
            else if (b0 <= 246) v = (float)(int16_t)(b0 - 139);
            else if (b0 <= 250) v = (float)(int16_t)((b0 - 247) * 256 + get8(&b) + 108);
            else v = (float)(int16_t)(-(int)(b0 - 251) * 256 - (int)get8(&b) - 108);
            if (sp >= MAX_STACK) return 0;
            s[sp++] = v;
            clear = 0;
            break;
        }
        }
        if (c->failed) return -1;
        if (clear) sp = 0;
    }
    return 0; /* no endchar */
}

int cff_glyph(const cff_font *f, int glyph, cff_outline *out, int box[4], const char **error) {
    pen c;
    memset(&c, 0, sizeof c);
    c.out = out;
    out->len = 0;
    box[0] = box[1] = box[2] = box[3] = 0;
    if (glyph < 0 || glyph >= f->glyphs) {
        *error = "glyph out of range";
        return -1;
    }
    int r = run(f, glyph, &c);
    if (r < 0) {
        out->len = 0;
        *error = c.failed ? "out of memory" : "too many charstring operators";
        return -1;
    }
    if (r == 0 || out->len == 0) {
        out->len = 0;
        return 0;
    }
    box[0] = c.min_x;
    box[1] = c.min_y;
    box[2] = c.max_x;
    box[3] = c.max_y;
    return 1;
}
