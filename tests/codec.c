/* Emit original data and encoded records for independent Python/zlib decoding. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef TEST_CUSTOM_DEFLATE
unsigned char *termshot_zlib_compress(unsigned char *, int, int *, int);
#define STBIW_ZLIB_COMPRESS termshot_zlib_compress
#endif
#define STB_IMAGE_WRITE_IMPLEMENTATION
#include "stb_image_write.h"
#include "../src/png_crc.h"

static uint32_t state = 42;
static unsigned char random_byte(void) {
    state ^= state << 13; state ^= state >> 17; state ^= state << 5;
    return (unsigned char)state;
}
static void record(char kind, const unsigned char *raw, uint32_t len, unsigned char *encoded, int size) {
    if (!encoded || size <= 0) exit(1);
    fwrite(&kind, 1, 1, stdout);
    fwrite(&len, 4, 1, stdout);
    uint32_t packed = (uint32_t)size;
    fwrite(&packed, 4, 1, stdout);
    fwrite(raw, 1, len, stdout);
    fwrite(encoded, 1, packed, stdout);
    free(encoded);
}
int main(void) {
    const int big[] = {5551, 5552, 5553, 32766, 32767, 32768, 32769, 65535, 65536, 100000};
    for (int test = 0; test < 301 + (int)(sizeof(big) / sizeof(big[0])); test++) {
        int len = test <= 300 ? test : big[test - 301];
        unsigned char *data = malloc((size_t)len + 1);
        if (!data) return 1;
        for (int pattern = 0; pattern < 4; pattern++) {
            for (int i = 0; i < len; i++)
                data[i] = pattern == 0 ? 0 : pattern == 1 ? (unsigned char)(i % 251) : pattern == 2 ? random_byte() : (i % 259 == 0 ? random_byte() : 65);
            if (termshot_png_crc(data, len) != stbiw__crc32(data, len)) return 2;
            if (len > 0 && termshot_png_crc(data + 1, len - 1) != stbiw__crc32(data + 1, len - 1)) return 2;
            for (int quality = 5; quality <= 20; quality += 5) {
                int size;
                unsigned char *encoded = stbi_zlib_compress(data, len, &size, quality);
                record('Z', data, (uint32_t)len, encoded, size);
            }
        }
        free(data);
    }
    for (int n = 1; n <= 4; n++) {
        int width = 31, height = 29, stride = width*n + 7, len = width*height*n;
        unsigned char data[29 * (31*4+7)], expected[29*31*4];
        memset(data, 0, sizeof(data));
        for (int y = 1; y < height; y++) {
            if (y % 3) memcpy(data+y*stride, data+(y-1)*stride, width*n);
            else for (int x = 0; x < width*n; x++) data[y*stride+x] = random_byte();
        }
        for (int flip = 0; flip <= 1; flip++) {
            stbi_flip_vertically_on_write(flip);
            for (int y = 0; y < height; y++) memcpy(expected+y*width*n, data+(flip ? height-1-y : y)*stride, width*n);
            for (int filter = -1; filter < 5; filter++) {
                stbi_write_force_png_filter = filter;
                int size;
                unsigned char *encoded = stbi_write_png_to_mem(data, stride, width, height, n, &size);
                record('P', expected, (uint32_t)len, encoded, size);
            }
        }
    }
    return ferror(stdout) ? 1 : 0;
}
