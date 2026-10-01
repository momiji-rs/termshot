/* POC: draw_png's painting section, verbatim but without timers, so a Rust
   port (poc.rs) can be compared against it on the same inputs. Includes
   src/draw.c for its static helpers (fill_rect, blend, paint_geometry, ...).
   run.sh builds it against draw.c as of the commit the port was taken from,
   so later changes to draw.c don't break the comparison. */
#include "draw.c"

size_t poc_fontinfo_size(void) { return sizeof(stbtt_fontinfo); }

/* Font metrics exactly as draw_png computes them. */
void poc_metrics(const unsigned char *ttf, double font_px, float *scale_out, int *cell_w_out,
                 int *cell_h_out, int *baseline_out) {
    stbtt_fontinfo font;
    stbtt_InitFont(&font, ttf, stbtt_GetFontOffsetForIndex(ttf, 0));
    int ascent, descent, line_gap;
    stbtt_GetFontVMetrics(&font, &ascent, &descent, &line_gap);
    int adv = 0, lsb = 0;
    stbtt_GetCodepointHMetrics(&font, 'M', &adv, &lsb);
    float scale = stbtt_ScaleForPixelHeight(&font, (float)font_px);
    int cell_w = (int)(adv * scale + 0.5f);
    if (cell_w < 1) cell_w = 1;
    scale = (float)cell_w / (float)adv;
    int body = (int)((ascent - descent) * scale + 0.5f);
    int gap = (int)(line_gap * scale + 0.5f);
    if (gap < 0) gap = 0;
    int cell_h = body + gap;
    if (cell_h < 1) cell_h = 1;
    *scale_out = scale;
    *cell_w_out = cell_w;
    *cell_h_out = cell_h;
    *baseline_out = (int)(ascent * scale + 0.5f) + (cell_h - body) / 2;
}

/* Paint into a caller-provided filtered buffer (stride = width*3+1). */
void c_paint(const Cell *cells, int cols, int rows, const unsigned char *ttf, float scale, int cell_w,
             int cell_h, int baseline, uint8_t *filtered) {
    stbtt_fontinfo font;
    stbtt_InitFont(&font, ttf, stbtt_GetFontOffsetForIndex(ttf, 0));
    long long width = (long long)cols * cell_w;
    long long height = (long long)rows * cell_h;
    size_t stride = (size_t)width * BPP + 1;
    Canvas canvas = {.filtered = filtered, .w = (int)width, .h = (int)height, .stride = stride};
    Canvas *cv = &canvas;
    cv->px = cv->filtered + 1;
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
    Glyph cache[GLYPH_CACHE_SIZE] = {0};
    for (int r = 0; r < rows; r++) {
        for (int c = 0; c < cols; c++) {
            const Cell *cell = &cells[r * cols + c];
            uint32_t cp = cell->ch;
            if (cp == 0 || cp == ' ') continue;
            if (paint_geometry(cv, c, r, cell_w, cell_h, cp, cell->attrs & ATTR_BOLD, cell->fr, cell->fg, cell->fb)) continue;
            Glyph *entry = &cache[cp % GLYPH_CACHE_SIZE];
            if (!(entry->valid && entry->cp == cp)) {
                free(entry->bitmap);
                *entry = (Glyph){.cp = cp, .valid = 1};
                int glyph = stbtt_FindGlyphIndex(&font, (int)cp);
                if (glyph != 0) {
                    int ix1, iy1;
                    stbtt_GetGlyphBitmapBox(&font, glyph, scale, scale, &entry->ix0, &entry->iy0, &ix1, &iy1);
                    entry->w = ix1 - entry->ix0;
                    entry->h = iy1 - entry->iy0;
                    if (entry->w > 0 && entry->h > 0) {
                        entry->bitmap = (unsigned char *)malloc((size_t)entry->w * entry->h);
                        stbtt_MakeGlyphBitmap(&font, entry->bitmap, entry->w, entry->h, entry->w, scale, scale, glyph);
                    }
                }
            }
            if (!entry->bitmap) continue;
            int dx = c * cell_w + entry->ix0;
            int dy = r * cell_h + baseline + entry->iy0;
            blend(cv, dx, dy, entry->bitmap, entry->w, entry->h, cell->fr, cell->fg, cell->fb);
            if (cell->attrs & ATTR_BOLD) blend(cv, dx + 1, dy, entry->bitmap, entry->w, entry->h, cell->fr, cell->fg, cell->fb);
        }
    }
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
}
