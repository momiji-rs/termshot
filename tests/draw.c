/* Exercise the complete C renderer under ASan/UBSan, including cache eviction. */
#include "../src/draw.c"
int main(int argc, char **argv) {
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
    const uint32_t chars[] = {'j', 'A', 0x141, 0x241, 0x341, 0x10ffff, 0x2500, 0x256d, 0x256e, 0x256f, 0x2570, 0x2588};
    for (int i = 0; i < 97*9; i++) {
        cells[i] = (Cell){.ch = chars[i % 12], .fr = i % 256, .fg = 255,
                         .fb = 100, .br = 17, .bg = i % 91, .bb = 35, .attrs = i % 16};
    }
    const double sizes[] = {1, 9, 47.5, 255};
    for (int i = 0; i < 4; i++) {
        int code = draw_png(cells, i == 3 ? 12 : 97, i == 3 ? 2 : 9, font, sizes[i], argv[2], 0);
        if (code) { free(font); return code; }
    }
    free(font);
    return 0;
}
