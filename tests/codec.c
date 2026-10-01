/* Round-trip the encoders through an independent decoder. stb_image inflates
   and unfilters; it checks neither the Adler-32 trailer nor chunk CRCs, so
   those are checked here with bitwise reference implementations. */
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
#define STB_IMAGE_IMPLEMENTATION
#define STBI_ONLY_PNG
#define STBI_NO_STDIO
#define STBI_NO_LINEAR
#define STBI_NO_HDR
#include "stb_image.h"
#include "../src/png_crc.h"

static uint32_t state = 42;
static unsigned char random_byte(void) {
    state ^= state << 13; state ^= state >> 17; state ^= state << 5;
    return (unsigned char)state;
}
static int count;
static void fail(const char *what, char kind, uint32_t len) {
    fprintf(stderr, "FAIL %s: record %d kind %c length %u\n", what, count, kind, len);
    exit(1);
}
static uint32_t be32(const unsigned char *p) {
    return (uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 | (uint32_t)p[2] << 8 | p[3];
}
static uint32_t crc_bitwise(const unsigned char *p, uint32_t len) {
    uint32_t crc = ~0u;
    for (uint32_t i = 0; i < len; i++) {
        crc ^= p[i];
        for (int bit = 0; bit < 8; bit++) crc = crc >> 1 ^ (0xedb88320u & -(crc & 1));
    }
    return ~crc;
}
static uint32_t adler32(const unsigned char *p, uint32_t len) {
    uint32_t a = 1, b = 0;
    for (uint32_t i = 0; i < len; i++) {
        a = (a + p[i]) % 65521;
        b = (b + a) % 65521;
    }
    return b << 16 | a;
}
static void check_zlib(const unsigned char *raw, uint32_t len, unsigned char *packed, int size) {
    if (size < 6) fail("short zlib stream", 'Z', len);
    if (be32(packed + size - 4) != adler32(raw, len)) fail("adler-32", 'Z', len);
    int out_len;
    char *out = stbi_zlib_decode_malloc((const char *)packed, size, &out_len);
    if (!out || (uint32_t)out_len != len || memcmp(out, raw, len)) fail("inflate", 'Z', len);
    free(out);
}
static void check_png(const unsigned char *raw, uint32_t len, unsigned char *png, int size, int width, int height,
                      int channels) {
    if (size < 8 || memcmp(png, "\x89PNG\r\n\x1a\n", 8)) fail("signature", 'P', len);
    int pos = 8, ended = 0;
    while (pos < size) {
        if (ended || size - pos < 12) fail("chunk layout", 'P', len);
        uint32_t body = be32(png + pos);
        if (body > (uint32_t)(size - pos - 12)) fail("chunk length", 'P', len);
        if (crc_bitwise(png + pos + 4, body + 4) != be32(png + pos + 8 + body)) fail("chunk crc", 'P', len);
        ended = !memcmp(png + pos + 4, "IEND", 4);
        pos += (int)body + 12;
    }
    if (!ended) fail("missing IEND", 'P', len);
    int w, h, n;
    unsigned char *pixels = stbi_load_from_memory(png, size, &w, &h, &n, 0);
    if (!pixels || w != width || h != height || n != channels || memcmp(pixels, raw, len)) fail("decode", 'P', len);
    stbi_image_free(pixels);
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
                if (!encoded) fail("compress", 'Z', (uint32_t)len);
                check_zlib(data, (uint32_t)len, encoded, size);
                free(encoded);
                count++;
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
                if (!encoded) fail("encode", 'P', (uint32_t)len);
                check_png(expected, (uint32_t)len, encoded, size, width, height, n);
                free(encoded);
                count++;
            }
        }
    }
    if (count != 5024) {
        fprintf(stderr, "FAIL expected 5024 round trips, ran %d\n", count);
        return 1;
    }
    printf("%d codec round trips passed\n", count);
    return 0;
}
