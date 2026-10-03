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
        .x = -1, .y = -1, .w = 4, .h = 4, .clip_top = 0, .clip_bottom = 2};
    paint_images(&cv, &image, 1);
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
int main(int argc, char **argv) {
    images();
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
        int code = draw_png(cells, i == 3 ? 12 : 97, i == 3 ? 2 : 9, font, NULL, sizes[i], argv[2], 0);
        if (code) { free(font); return code; }
    }
    free(font);
    return 0;
}
