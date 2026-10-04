/* Exercise the complete C renderer under ASan/UBSan, including cache eviction. */
#include "../src/draw.c"
#include <assert.h>
static void images(void) {
    unsigned char pixels[4 * 4] = {255,0,0,255, 0,255,0,128, 0,0,255,0, 255,255,0,255};
    unsigned char buffer[4 * 13] = {0};
    Canvas cv = {.filtered = buffer, .px = buffer + 1, .w = 4, .h = 4, .stride = 13};
    // Negative origins, vertical clipping and partial alpha exercise the actual
    // painter with guarded scanlines. Source quadrants expand to 2x2 pixels.
    ImageView image = {.pixels = pixels, .width = 2, .height = 2,
        .x = -1, .y = -1, .w = 4, .h = 4, .clip_top = 0, .clip_bottom = 2, .clip_left = INT64_MIN, .clip_right = INT64_MAX,
        .src_w = 2, .src_h = 2};
    paint_images(&cv, &image, 1, LAYER_OVER_TEXT, NULL, 1, 1);
    assert(buffer[1] == 255); // red at (0,0)
    assert(buffer[5] == 128); // half-alpha green at (1,0)
    assert(buffer[14] == 0);  // transparent blue at (0,1)
    assert(buffer[17] == 255 && buffer[18] == 255); // yellow at (1,1)
    for (int y = 0; y < 4; y++) {
        assert(buffer[y * 13] == 0); // filter/guard byte
        for (int c = 0; c < 3; c++) assert(buffer[y * 13 + 10 + c] == 0);
    }
    for (int i = 26; i < 52; i++) assert(buffer[i] == 0);
}

/* Crops, layers and the mask of default backgrounds, on a 4x4 canvas of two
   2x4 cells: the first in the default background, the second red. */
static void layered_images(void) {
    unsigned char pixels[4 * 4] = {255,0,0,255, 0,255,0,255, 0,0,255,255, 255,255,0,255};
    unsigned char buffer[4 * 13];
    Cell cells[2] = {{.ch = ' ', .br = 17, .bg = 24, .bb = 35}, {.ch = ' ', .br = 205, .attrs = ATTR_OPAQUE}};
    Canvas cv = {.filtered = buffer, .px = buffer + 1, .w = 4, .h = 4, .stride = 13};
    /* The bottom right source pixel, yellow, stretched over the canvas. */
    ImageView image = {.pixels = pixels, .width = 2, .height = 2, .x = 0, .y = 0, .w = 4, .h = 4,
                       .clip_top = 0, .clip_bottom = 4, .clip_left = INT64_MIN, .clip_right = INT64_MAX, .src_x = 1, .src_y = 1, .src_w = 1, .src_h = 1};
    for (int32_t z = -3; z <= 3; z++) assert(image_layer(z) == (z < 0 ? LAYER_UNDER_TEXT : LAYER_OVER_TEXT));
    assert(image_layer(INT32_MIN / 2) == LAYER_UNDER_TEXT);
    assert(image_layer(INT32_MIN / 2 - 1) == LAYER_BELOW && image_layer(INT32_MIN) == LAYER_BELOW);
    assert(image_layer(INT32_MAX) == LAYER_OVER_TEXT);
    for (int layer = LAYER_BELOW; layer <= LAYER_OVER_TEXT; layer++) {
        image.z = layer == LAYER_BELOW ? INT32_MIN : layer == LAYER_UNDER_TEXT ? -1 : 0;
        for (int mask = 0; mask < 2; mask++) {
            memset(buffer, 0, sizeof buffer);
            /* Another layer paints nothing. */
            paint_images(&cv, &image, 1, (layer + 1) % 3, mask ? cells : NULL, 2, 4);
            for (size_t i = 0; i < sizeof buffer; i++) assert(buffer[i] == 0);
            paint_images(&cv, &image, 1, layer, mask ? cells : NULL, 2, 4);
            for (int y = 0; y < 4; y++) {
                for (int x = 0; x < 4; x++) {
                    const unsigned char *p = buffer + y * 13 + 1 + x * 3;
                    int shown = !mask || x < 2;
                    assert(p[0] == (shown ? 255 : 0) && p[1] == (shown ? 255 : 0) && p[2] == 0);
                }
            }
        }
    }
    /* Only ATTR_OPAQUE decides, whatever the colour or other attributes. */
    cells[0].attrs = ATTR_OPAQUE;
    assert(!clear_background(&cells[0]) && !clear_background(&cells[1]));
    cells[1].attrs = ATTR_BOLD | ATTR_ITALIC;
    assert(clear_background(&cells[1]));
    /* A crop of the top row, sampled across: red then green. */
    image = (ImageView){.pixels = pixels, .width = 2, .height = 2, .x = 0, .y = 0, .w = 4, .h = 1,
                        .clip_top = 0, .clip_bottom = 4, .clip_left = INT64_MIN, .clip_right = INT64_MAX, .src_x = 0, .src_y = 0, .src_w = 2, .src_h = 1};
    memset(buffer, 0, sizeof buffer);
    paint_images(&cv, &image, 1, LAYER_OVER_TEXT, NULL, 2, 4);
    assert(buffer[1] == 255 && buffer[4] == 255 && buffer[8] == 255 && buffer[11] == 255);
    assert(buffer[2] == 0 && buffer[7] == 0 && buffer[10] == 0);
    for (int i = 13; i < 52; i++) assert(buffer[i] == 0);
}

/* A Unicode placeholder run shows the columns of its cells only: the image
   is sampled as a whole and cut at clip_left and clip_right, so runs side by
   side join without a seam. */
static void clipped_across(void) {
    unsigned char pixels[4 * 4] = {255,0,0,255, 0,255,0,255, 0,0,255,255, 255,255,0,255};
    unsigned char buffer[4 * 13], whole[4 * 13];
    Canvas cv = {.filtered = buffer, .px = buffer + 1, .w = 4, .h = 4, .stride = 13};
    ImageView image = {.pixels = pixels, .width = 2, .height = 2, .x = -1, .y = 0, .w = 5, .h = 4,
                       .clip_top = 0, .clip_bottom = 4, .clip_left = INT64_MIN, .clip_right = INT64_MAX,
                       .src_w = 2, .src_h = 2};
    memset(whole, 0, sizeof whole);
    cv.px = whole + 1;
    paint_images(&cv, &image, 1, LAYER_OVER_TEXT, NULL, 1, 1);
    memset(buffer, 0, sizeof buffer);
    cv.px = buffer + 1;
    for (int64_t left = 0; left < 4; left += 2) {
        image.clip_left = left;
        image.clip_right = left + 2;
        paint_images(&cv, &image, 1, LAYER_OVER_TEXT, NULL, 1, 1);
    }
    assert(memcmp(buffer, whole, sizeof buffer) == 0);
    /* One run alone leaves the other columns untouched. */
    memset(buffer, 0, sizeof buffer);
    image.clip_left = 1;
    image.clip_right = 3;
    paint_images(&cv, &image, 1, LAYER_OVER_TEXT, NULL, 1, 1);
    for (int y = 0; y < 4; y++) {
        for (int x = 0; x < 4; x++) {
            const unsigned char *p = buffer + y * 13 + 1 + x * 3, *q = whole + y * 13 + 1 + x * 3;
            for (int c = 0; c < 3; c++) assert(p[c] == (x >= 1 && x < 3 ? q[c] : 0));
        }
    }
    /* An empty or reversed clip draws nothing. */
    memset(buffer, 0, sizeof buffer);
    image.clip_left = 3;
    image.clip_right = 3;
    paint_images(&cv, &image, 1, LAYER_OVER_TEXT, NULL, 1, 1);
    image.clip_left = 4;
    image.clip_right = 1;
    paint_images(&cv, &image, 1, LAYER_OVER_TEXT, NULL, 1, 1);
    for (size_t i = 0; i < sizeof buffer; i++) assert(buffer[i] == 0);
}
int main(int argc, char **argv) {
    images();
    layered_images();
    clipped_across();
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
