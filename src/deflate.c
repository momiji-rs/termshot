/* zlib compressor for stb_image_write, plugged in with STBIW_ZLIB_COMPRESS.
   It is stb's own algorithm (public domain, Sean Barrett) and writes the same
   bytes; tests/deflate_diff.c checks that against stock stb. Search, emission,
   and checksums are faster (both implementations fix an invalid empty stream):

   - stb scans a hash bucket oldest first and keeps a match when d >= best, so
     among equally long matches the newest wins. Scanning newest first and
     taking a match only when it is strictly longer picks the same one, and
     lets the scan stop at the longest possible length or at the first entry
     outside the window (entries are stored in position order).
   - Lazy matching asks only whether any match at i+1 is longer, so it is
     skipped when the current one cannot be beaten.
   - Candidates that cannot beat the best length are rejected before comparing
     their prefixes.
   - Matches are compared 8 bytes at a time.
   - Buckets are fixed arrays of positions instead of per-bucket stretchy
     buffers; stb caps a bucket at 2*quality entries anyway.
   - Length/distance indexes are calculated directly and each token is emitted
     in one operation. Independent Adler-32 reductions permit vectorization.
   - Allocation failures return NULL, as stb's
     hash-table failure does, instead of asserting. */

#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "deflate_profile.h"

_Thread_local DeflateProfile termshot_deflate_profile;

static double profile_now(void) {
    if (!termshot_deflate_profile.enabled) return 0;
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1000.0 + (double)ts.tv_nsec / 1000000.0;
}

#define ZHASH 16384
#define WINDOW 32768
#define MAX_MATCH 258

typedef struct {
    unsigned char *p;
    size_t n, cap;
    int failed;
    uint64_t bitbuf;
    int bitcount;
} Out;

static void put_byte(Out *o, unsigned char b) {
    if (o->n == o->cap) {
        if (o->failed) return;
        size_t cap = o->cap ? o->cap * 2 : 65536;
        unsigned char *p = (unsigned char *)realloc(o->p, cap);
        if (!p) {
            o->failed = 1;
            return;
        }
        o->p = p;
        o->cap = cap;
    }
    o->p[o->n++] = b;
}

static void add_bits(Out *o, unsigned int code, int bits) {
    o->bitbuf |= (uint64_t)code << o->bitcount;
    o->bitcount += bits;
    while (o->bitcount >= 8) {
        put_byte(o, (unsigned char)o->bitbuf);
        o->bitbuf >>= 8;
        o->bitcount -= 8;
    }
}

static unsigned int bitrev(unsigned int code, int bits) {
    unsigned int r = 0;
    while (bits--) {
        r = (r << 1) | (code & 1);
        code >>= 1;
    }
    return r;
}

/* Callers pass nonzero values. Compilers map this to a leading-zero count. */
static unsigned int log2_floor(unsigned int value) {
#if defined(__GNUC__) || defined(__clang__)
    return 31u - (unsigned int)__builtin_clz(value);
#else
    unsigned int result = 0;
    while (value >>= 1) result++;
    return result;
#endif
}

/* Fixed Huffman codes, as in stb. */
static void huff(Out *o, int n) {
    if (n <= 143) add_bits(o, bitrev(0x30 + n, 8), 8);
    else if (n <= 255) add_bits(o, bitrev(0x190 + n - 144, 9), 9);
    else if (n <= 279) add_bits(o, bitrev(n - 256, 7), 7);
    else add_bits(o, bitrev(0xc0 + n - 280, 8), 8);
}

static unsigned int zhash(const unsigned char *d) {
    uint32_t h = d[0] + (d[1] << 8) + (d[2] << 16);
    h ^= h << 3;
    h += h >> 5;
    h ^= h << 4;
    h += h >> 17;
    h ^= h << 25;
    h += h >> 6;
    return h & (ZHASH - 1);
}

/* Length of the common prefix of a and b, at most limit (limit <= 258). */
static int countm(const unsigned char *a, const unsigned char *b, int limit) {
    int i = 0;
#if (defined(__GNUC__) || defined(__clang__)) && defined(__BYTE_ORDER__)
    while (i + 8 <= limit) {
        uint64_t x, y;
        memcpy(&x, a + i, 8);
        memcpy(&y, b + i, 8);
        if (x != y) {
#if __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
            return i + (__builtin_ctzll(x ^ y) >> 3);
#else
            return i + (__builtin_clzll(x ^ y) >> 3);
#endif
        }
        i += 8;
    }
#endif
    while (i < limit && a[i] == b[i]) i++;
    return i;
}

static uint32_t adler32(const unsigned char *d, size_t len) {
    uint32_t s1 = 1, s2 = 0;
    while (len) {
        size_t block = len < 5552 ? len : 5552; /* largest n with no 32-bit overflow */
        len -= block;
        /* Independent reductions let the compiler vectorize the weighted
           checksum; expanding s2's recurrence gives 32*s1 + sum((32-k)*d[k]). */
        while (block >= 32) {
            uint32_t sum = 0, weighted = 0;
            for (unsigned k = 0; k < 32; k++) {
                sum += d[k];
                weighted += (32 - k) * d[k];
            }
            s2 += 32 * s1 + weighted;
            s1 += sum;
            d += 32;
            block -= 32;
        }
        while (block--) {
            s1 += *d++;
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
    }
    return (s2 << 16) | s1;
}

unsigned char *termshot_zlib_compress(unsigned char *data, int data_len, int *out_len, int quality) {
    double started = profile_now();
    static const unsigned short lengthc[] = {3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27,
                                             31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258, 259};
    static const unsigned char lengtheb[] = {0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2,
                                             2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0};
    static const unsigned short distc[] = {1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193,
                                           257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
                                           8193, 12289, 16385, 24577, 32768};
    static const unsigned char disteb[] = {0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6,
                                           6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13};
    if (quality < 5) quality = 5;
    const int cap = 2 * quality;
    int32_t *tab = (int32_t *)malloc(sizeof(int32_t) * (size_t)ZHASH * cap);
    int *cnt = (int *)calloc(ZHASH, sizeof(int));
    Out o = {0};
    if (!tab || !cnt) {
        free(tab);
        free(cnt);
        return NULL;
    }

    put_byte(&o, 0x78); /* DEFLATE 32K window */
    put_byte(&o, 0x5e); /* FLEVEL = 1 */
    add_bits(&o, 1, 1); /* BFINAL = 1 */
    add_bits(&o, 1, 2); /* BTYPE = 1, fixed Huffman */

    double allocated = profile_now();
    int i = 0;
    while (i < data_len - 3) {
        unsigned int h = zhash(data + i);
        int32_t *list = tab + (size_t)h * cap;
        int n = cnt[h];
        int limit = data_len - i < MAX_MATCH ? data_len - i : MAX_MATCH;
        int best = 3, bestpos = -1;
        for (int j = n - 1; j >= 0; j--) {
            if (list[j] <= i - WINDOW) break;
            /* A newer candidate already won ties. Only a longer match helps. */
            if (bestpos >= 0 && data[list[j] + best] != data[i + best]) continue;
            int d = countm(data + list[j], data + i, limit);
            if (bestpos < 0 ? d >= best : d > best) {
                best = d;
                bestpos = list[j];
                if (best == limit) break;
            }
        }
        if (n == cap) {
            memmove(list, list + quality, sizeof(int32_t) * quality);
            n = quality;
        }
        list[n] = i;
        cnt[h] = n + 1;

        if (bestpos >= 0) {
            /* Lazy matching: if the match at i+1 is longer, emit i as a literal. */
            int limit1 = data_len - i - 1 < MAX_MATCH ? data_len - i - 1 : MAX_MATCH;
            if (best < limit1) {
                unsigned int h1 = zhash(data + i + 1);
                int32_t *list1 = tab + (size_t)h1 * cap;
                for (int j = cnt[h1] - 1; j >= 0; j--) {
                    if (list1[j] <= i - (WINDOW - 1)) break;
                    if (data[list1[j] + best] != data[i + 1 + best]) continue;
                    if (countm(data + list1[j], data + i + 1, limit1) > best) {
                        bestpos = -1;
                        break;
                    }
                }
            }
        }

        if (bestpos >= 0) {
            int d = i - bestpos;
            int j;
            if (best <= 10) j = best - 3;
            else if (best == MAX_MATCH) j = 28;
            else {
                unsigned int magnitude = log2_floor((unsigned int)best - 3);
                j = (int)(4 * (magnitude - 1) + (((unsigned int)best - 3) >> (magnitude - 2) & 3));
            }
            unsigned int bits = j <= 22 ? 7 : 8;
            unsigned int code = j <= 22 ? bitrev((unsigned int)j + 1, 7) : bitrev(0xc0u + j - 23, 8);
            code |= (unsigned int)(best - lengthc[j]) << bits;
            bits += lengtheb[j];
            if (d <= 4) j = d - 1;
            else {
                unsigned int magnitude = log2_floor((unsigned int)d - 1);
                j = (int)(2 * magnitude + (((unsigned int)d - 1) >> (magnitude - 1) & 1));
            }
            code |= bitrev((unsigned int)j, 5) << bits;
            bits += 5;
            code |= (unsigned int)(d - distc[j]) << bits;
            bits += disteb[j];
            /* A complete length/distance token fits in 31 bits. The 64-bit
               accumulator also holds the previous token's trailing bits. */
            add_bits(&o, code, (int)bits);
            i += best;
        } else {
            huff(&o, data[i]);
            i++;
        }
    }
    for (; i < data_len; i++) huff(&o, data[i]);
    huff(&o, 256); /* end of block */
    while (o.bitcount) add_bits(&o, 0, 1);
    double matched = profile_now();
    free(tab);
    free(cnt);

    /* Store uncompressed instead if compression was worse, as stb does. */
    /* Empty input still needs the final fixed-Huffman block above. */
    if (!o.failed && data_len > 0 && o.n > (size_t)data_len + 2 + ((data_len + 32766) / 32767) * 5) {
        o.n = 2;
        for (int j = 0; j < data_len;) {
            int blocklen = data_len - j > 32767 ? 32767 : data_len - j;
            put_byte(&o, data_len - j == blocklen);
            put_byte(&o, (unsigned char)blocklen);
            put_byte(&o, (unsigned char)(blocklen >> 8));
            put_byte(&o, (unsigned char)~blocklen);
            put_byte(&o, (unsigned char)(~blocklen >> 8));
            for (int k = 0; k < blocklen; k++) put_byte(&o, data[j + k]);
            j += blocklen;
        }
    }

    double finalized = profile_now();
    uint32_t adler = adler32(data, (size_t)data_len);
    put_byte(&o, (unsigned char)(adler >> 24));
    put_byte(&o, (unsigned char)(adler >> 16));
    put_byte(&o, (unsigned char)(adler >> 8));
    put_byte(&o, (unsigned char)adler);
    if (o.failed) {
        free(o.p);
        return NULL;
    }
    *out_len = (int)o.n;
    if (termshot_deflate_profile.enabled) {
        termshot_deflate_profile.allocate_ms = allocated - started;
        termshot_deflate_profile.match_emit_ms = matched - allocated;
        termshot_deflate_profile.finalize_ms = finalized - matched;
        termshot_deflate_profile.checksum_ms = profile_now() - finalized;
    }
    return o.p;
}
