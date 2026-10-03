/* Differential test: src/deflate.c must write the same bytes as stock
   stb_image_write's stbi_zlib_compress for every input and quality.
   Inputs are pseudorandom but cover what matters for the search: long runs,
   short repeats, data longer than the 32 KiB window, low- and high-entropy
   bytes, and tiny lengths. Built and run by test.sh. */

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define STB_IMAGE_WRITE_IMPLEMENTATION
#include "stb_image_write.h"

unsigned char *termshot_zlib_compress(unsigned char *data, int data_len, int *out_len, int quality);

static uint64_t rng = 0x9e3779b97f4a7c15ull;

static uint32_t next(void) {
    rng ^= rng << 13;
    rng ^= rng >> 7;
    rng ^= rng << 17;
    return (uint32_t)(rng >> 16);
}

/* Fill buf with one of several shapes. */
static void fill(unsigned char *buf, int len, int shape) {
    int alphabet = 1 + (int)(next() % 255);
    int i = 0;
    while (i < len) {
        switch (shape) {
        case 0: /* uniform noise over a small or large alphabet */
            buf[i++] = (unsigned char)(next() % alphabet);
            break;
        case 1: { /* runs of one byte, like image background */
            int run = 1 + (int)(next() % 600);
            unsigned char b = (unsigned char)(next() % alphabet);
            while (run-- && i < len) buf[i++] = b;
            break;
        }
        case 2: { /* copies of earlier data at a random distance, like repeated glyphs */
            if (i < 4) {
                buf[i++] = (unsigned char)next();
                break;
            }
            int dist = 1 + (int)(next() % (i < 40000 ? i : 40000));
            int run = 3 + (int)(next() % 300);
            while (run-- && i < len) {
                buf[i] = buf[i - dist];
                i++;
            }
            if (i < len && next() % 3 == 0) buf[i++] = (unsigned char)next();
            break;
        }
        default: { /* image-like rows: a repeating pixel pattern with sparse edits */
            int stride = 4 * (1 + (int)(next() % 64));
            if (i < stride) {
                buf[i] = (unsigned char)(next() % alphabet);
                i++;
            } else {
                buf[i] = next() % 50 == 0 ? (unsigned char)next() : buf[i - stride];
                i++;
            }
            break;
        }
        }
    }
}

int main(int argc, char **argv) {
    int cases = argc > 1 ? atoi(argv[1]) : 3000;
    int fails = 0;
    unsigned char *buf = (unsigned char *)malloc(200000);
    if (!buf) return 2;
    for (int c = 0; c < cases; c++) {
        int len;
        if (c < 12) len = c; /* 0..11 bytes */
        else if (c % 50 == 0) len = 70000 + (int)(next() % 120000); /* past the 32 KiB window, twice */
        else len = (int)(next() % 20000);
        int shape = (int)(next() % 4);
        int quality = 5 + (int)(next() % 12);
        fill(buf, len, shape);
        int stb_len = 0, our_len = 0;
        unsigned char *want = stbi_zlib_compress(buf, len, &stb_len, quality);
        unsigned char *got = termshot_zlib_compress(buf, len, &our_len, quality);
        if (!want || !got || stb_len != our_len || memcmp(want, got, (size_t)our_len) != 0) {
            if (fails < 5)
                fprintf(stderr, "case %d: len %d shape %d quality %d: stb %d bytes, ours %d bytes\n",
                        c, len, shape, quality, stb_len, our_len);
            fails++;
        }
        free(want);
        free(got);
    }
    /* Adler-32 worst case: all 0xff makes every sum as large as it can be,
       at lengths around its 16-byte lanes and 5552-byte blocks, from
       unaligned starts. */
    static const int adler_lens[] = {15, 16, 17, 31, 32, 33, 5551, 5552, 5553, 5568,
                                     11103, 11104, 11105, 65536, 199000};
    for (size_t l = 0; l < sizeof adler_lens / sizeof adler_lens[0]; l++) {
        for (int offset = 0; offset < 4; offset++) {
            int len = adler_lens[l] - offset;
            memset(buf, 0xff, 200000);
            int stb_len = 0, our_len = 0;
            unsigned char *want = stbi_zlib_compress(buf + offset, len, &stb_len, 8);
            unsigned char *got = termshot_zlib_compress(buf + offset, len, &our_len, 8);
            if (!want || !got || stb_len != our_len || memcmp(want, got, (size_t)our_len) != 0) {
                fprintf(stderr, "all-0xff: len %d offset %d: stb %d bytes, ours %d bytes\n",
                        len, offset, stb_len, our_len);
                fails++;
            }
            free(want);
            free(got);
            cases++;
        }
    }
    free(buf);
    if (fails) {
        printf("FAIL %d of %d cases differ from stb\n", fails, cases);
        return 1;
    }
    printf("ok, %d cases byte-identical to stb\n", cases);
    return 0;
}
