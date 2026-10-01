/* Test-only PNG decoder for tests/golden.rs. Wraps vendored stb_image
   (public domain), PNG support only. Always returns 8-bit RGBA, so an RGB and
   an RGBA file with the same pixels decode to the same bytes. */

#define STB_IMAGE_IMPLEMENTATION
#define STBI_ONLY_PNG
#define STBI_NO_STDIO_WARNINGS
#include "stb_image.h"

unsigned char *png_read_rgba(const char *path, int *width, int *height) {
    int channels;
    return stbi_load(path, width, height, &channels, 4);
}

const char *png_read_error(void) {
    return stbi_failure_reason();
}

void png_read_free(unsigned char *pixels) {
    stbi_image_free(pixels);
}
