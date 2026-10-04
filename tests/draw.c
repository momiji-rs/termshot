/* Exercise the complete C renderer under ASan/UBSan, including cache eviction.
   Every raster's backdrop is painted a row at a time, however small. */
#define BACKDROP_ROW_BYTES 0
#include "../src/draw.c"
#include <assert.h>
/* The image layers and the backdrop are Rust (src/composite.rs), whose unit
   tests check what this file did of the C: layers, crops, clips, the mask of
   default backgrounds. This checks the backdrop through draw.c's own
   declarations of it, so the shared structs agree in use as in size. */
/* The backdrop painted a row of cells at a time and all at once (as for a
   small raster, or with more than 64 images under the
   text) is the same: cell
   backgrounds of both kinds, and images below them and under the text that
   overlap, cross rows of cells, hang off the canvas and blend. */
static void backdrop_rows(void) {
    enum { COLS = 5, ROWS = 4, CW = 3, CH = 5, W = COLS * CW, H = ROWS * CH, STRIDE = W * 3 + 1 };
    unsigned char pixels[3 * 2 * 4], rows[STRIDE * H], whole[STRIDE * H];
    for (size_t i = 0; i < sizeof pixels; i++) pixels[i] = (unsigned char)(i * 37 + (i % 4 == 3 ? 90 : 0));
    Cell cells[COLS * ROWS];
    for (int i = 0; i < COLS * ROWS; i++)
        cells[i] = (Cell){.ch = ' ', .br = (uint8_t)(i * 11), .bg = 40, .bb = (uint8_t)(255 - i), .attrs = i % 3 ? 0 : ATTR_OPAQUE};
    ImageView views[3];
    for (int k = 0; k < 3; k++)
        views[k] = (ImageView){.pixels = pixels, .width = 3, .height = 2, .x = -2 + 4 * k, .y = 3 + 2 * k, .w = 9, .h = 8 + k,
                               .clip_top = 0, .clip_bottom = H, .clip_left = INT64_MIN, .clip_right = INT64_MAX,
                               .src_w = 3, .src_h = 2, .z = k == 1 ? -1 : INT32_MIN};
    for (int mode = 0; mode < 2; mode++) {
        unsigned char *buffer = mode ? whole : rows;
        memset(buffer, 0xee, STRIDE * H);
        Canvas cv = {.filtered = buffer, .px = buffer + 1, .w = W, .h = H, .stride = STRIDE};
        Backdrop bd;
        termshot_backdrop_init(&bd, &cv, cells, COLS, ROWS, CW, CH, views, 3, BACKDROP_ROW_BYTES);
        assert(!bd.whole && bd.done == 0 && bd.cells == cells && bd.images == views && bd.image_count == 3);
        bd.whole = mode;
        for (int y = 1; y <= H; y += 3) backdrop_through(&cv, &bd, y);
        backdrop_through(&cv, &bd, H);
        assert(bd.done == ROWS);
    }
    assert(memcmp(rows, whole, sizeof rows) == 0);
    /* The images over the text, and a solid one, as the cursor marks are. */
    unsigned char red[4] = {200, 0, 0, 255};
    ImageView over[2] = {views[0], {.pixels = red, .width = 1, .height = 1, .x = 4, .y = 2, .w = 3, .h = 9,
                                    .clip_top = 2, .clip_bottom = 11, .clip_left = 4, .clip_right = 7,
                                    .src_w = 1, .src_h = 1, .z = INT32_MAX}};
    over[0].z = 0;
    Canvas cv = {.filtered = rows, .px = rows + 1, .w = W, .h = H, .stride = STRIDE};
    assert(termshot_paint_images(&cv, over, 2, LAYER_OVER_TEXT) == 0);
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++) {
            const unsigned char *p = rows + y * STRIDE + 1 + x * 3;
            if (x >= 4 && x < 7 && y >= 2 && y < 11) assert(p[0] == 200 && p[1] == 0 && p[2] == 0);
        }
    assert(!termshot_paint_failed());
}

int main(int argc, char **argv) {
    backdrop_rows();
    if (argc != 3) return 1;
    /* Only the trusted vendored fixture is used by this C harness. Production
       and the Rust draw tests validate and pad fonts with font::load. */
    FILE *fp = fopen(argv[1], "rb");
    if (!fp) return 1;
    if (fseek(fp, 0, SEEK_END) != 0) { fclose(fp); return 1; }
    long len = ftell(fp);
    if (len <= 0 || fseek(fp, 0, SEEK_SET) != 0) { fclose(fp); return 1; }
    unsigned char *font = malloc((size_t)len);
    if (!font || fread(font, 1, (size_t)len, fp) != (size_t)len) {
        free(font);
        fclose(fp);
        return 1;
    }
    fclose(fp);
    Cell cells[97 * 9];
    const uint32_t chars[] = {'j', 'A', 0x441, 0x841, 0xc41, 0x10ffff, 0x2500, 0x256d, 0x256e, 0x256f, 0x2570, 0x2588};
    for (int i = 0; i < 97*9; i++) {
        cells[i] = (Cell){.ch = chars[i % 12], .fr = i % 256, .fg = 255,
                         .fb = 100, .br = 17, .bg = i % 91, .bb = 35, .attrs = i % 16};
    }
    const double sizes[] = {1, 9, 47.5, 255};
    for (int i = 0; i < 4; i++) {
        int code = draw_png(cells, i == 3 ? 12 : 97, i == 3 ? 2 : 9, font, 0, NULL, 0, sizes[i], argv[2], 0);
        if (code) { free(font); return code; }
    }
    free(font);
    return 0;
}
