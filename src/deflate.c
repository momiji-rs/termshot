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
   - Matches are compared 16 bytes at a time, with bounded tail loads.
   - Buckets are fixed arrays of positions instead of per-bucket stretchy
     buffers; stb caps a bucket at 2*quality entries anyway.
   - Length/distance indexes are calculated directly and each token is emitted
     in one operation.
   - Adler-32 keeps per-lane sums and applies the weights once per block.
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

/* GCC at -O2 keeps some per-token helpers out of line, which cost it up to
   6% of matching time on x86-64; clang inlines them anyway. */
#if defined(__GNUC__) || defined(__clang__)
#define HOT static inline __attribute__((always_inline))
#else
#define HOT static inline
#endif

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

HOT void add_bits(Out *o, unsigned int code, int bits) {
    o->bitbuf |= (uint64_t)code << o->bitcount;
    o->bitcount += bits;
    /* At most 31 new bits plus seven trailing bits: reserve four bytes once,
       then store the little-endian word without a capacity branch per byte.
       Bytes beyond n are scratch and overwritten by the next token. */
    if (!o->failed && o->cap - o->n < 4) {
        size_t cap = o->cap ? o->cap * 2 : 65536;
        unsigned char *p = (unsigned char *)realloc(o->p, cap);
        if (!p) {
            o->failed = 1;
        } else {
            o->p = p;
            o->cap = cap;
        }
    }
    unsigned int bytes = (unsigned int)o->bitcount >> 3;
    uint32_t word = (uint32_t)o->bitbuf;
    unsigned char packed[4] = {(unsigned char)word, (unsigned char)(word >> 8),
                              (unsigned char)(word >> 16), (unsigned char)(word >> 24)};
    if (!o->failed) {
        memcpy(o->p + o->n, packed, 4);
        o->n += bytes;
    }
    o->bitbuf >>= bytes * 8;
    o->bitcount &= 7;
}

/* Every byte with its bits reversed. */
#define R2(n) n, n + 2 * 64, n + 1 * 64, n + 3 * 64
#define R4(n) R2(n), R2(n + 2 * 16), R2(n + 1 * 16), R2(n + 3 * 16)
#define R6(n) R4(n), R4(n + 2 * 4), R4(n + 1 * 4), R4(n + 3 * 4)
static const unsigned char reversed_byte[256] = {R6(0), R6(2), R6(1), R6(3)};
#undef R2
#undef R4
#undef R6

/* code (below 2^bits, bits <= 9) with its bits reversed, Huffman codes being
   sent most significant bit first. A bit-at-a-time loop here cost GCC 3-10%
   of matching time. */
HOT unsigned int bitrev(unsigned int code, int bits) {
    unsigned int r9 = ((unsigned int)reversed_byte[code & 0xff] << 1) | ((code >> 8) & 1);
    return r9 >> (9 - bits);
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
HOT void huff(Out *o, int n) {
    if (n <= 143) add_bits(o, bitrev(0x30 + n, 8), 8);
    else if (n <= 255) add_bits(o, bitrev(0x190 + n - 144, 9), 9);
    else if (n <= 279) add_bits(o, bitrev(n - 256, 7), 7);
    else add_bits(o, bitrev(0xc0 + n - 280, 8), 8);
}

HOT unsigned int zhash(const unsigned char *d) {
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
HOT int countm(const unsigned char *a, const unsigned char *b, int limit) {
    int i = 0;
#if (defined(__GNUC__) || defined(__clang__)) && defined(__BYTE_ORDER__)
    while (i + 16 <= limit) {
        uint64_t x0, x1, y0, y1;
        memcpy(&x0, a + i, 8);
        memcpy(&y0, b + i, 8);
        memcpy(&x1, a + i + 8, 8);
        memcpy(&y1, b + i + 8, 8);
        uint64_t d0 = x0 ^ y0, d1 = x1 ^ y1;
        if (d0 | d1) {
            int offset = d0 ? 0 : 8;
            uint64_t diff = d0 ? d0 : d1;
#if __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
            return i + offset + (__builtin_ctzll(diff) >> 3);
#else
            return i + offset + (__builtin_clzll(diff) >> 3);
#endif
        }
        i += 16;
    }
    if (i + 8 <= limit) {
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

/* Adler-32 in 16 lanes. Within a block of n 16-byte chunks, byte k of chunk c
   is added to s2 (16 * (n - 1 - c) + 16 - k) times, so it is enough to keep,
   per lane k, the byte sum a[k] and the sum of earlier byte sums p[k] (p += a
   before each a += chunk): s2 += 16n * s1 + 16 * sum(p) + sum((16 - k) * a[k]).
   The inner loop is then only widening adds, with no multiply, which plain
   SSE2 and NEON both do well. A block of 5552 bytes (347 chunks) is the
   largest whose sums cannot overflow 32 bits before the modulo, as in zlib.

   GCC vectorizes the plain C loop at -O2; clang leaves it scalar, so clang
   gets the same arithmetic in its generic vector types. Both give the value
   of the scalar definition, which tests/deflate_diff.c checks against stb;
   TERMSHOT_PORTABLE_ADLER forces the plain loop so it is tested on clang too. */
#define ADLER_BLOCK 5552
#if defined(__clang__) && defined(__has_builtin) && !defined(TERMSHOT_PORTABLE_ADLER)
#if __has_builtin(__builtin_convertvector)
#define ADLER_VECTOR 1
#endif
#endif

#ifdef ADLER_VECTOR
typedef unsigned char AdlerBytes __attribute__((vector_size(16)));
typedef uint32_t AdlerLanes __attribute__((vector_size(64)));
#endif

static uint32_t adler32(const unsigned char *d, size_t len) {
    uint32_t s1 = 1, s2 = 0;
    while (len) {
        size_t block = len < ADLER_BLOCK ? len : ADLER_BLOCK;
        size_t chunks = block / 16;
        len -= block;
        if (chunks) {
#ifdef ADLER_VECTOR
            AdlerLanes a = {0}, p = {0};
            for (size_t c = 0; c < chunks; c++) {
                AdlerBytes x;
                memcpy(&x, d, 16);
                p += a;
                a += __builtin_convertvector(x, AdlerLanes);
                d += 16;
            }
#else
            uint32_t a[16] = {0}, p[16] = {0};
            for (size_t c = 0; c < chunks; c++) {
                for (unsigned k = 0; k < 16; k++) {
                    p[k] += a[k];
                    a[k] += d[k];
                }
                d += 16;
            }
#endif
            uint32_t sum = 0, prefix = 0, weighted = 0;
            for (unsigned k = 0; k < 16; k++) {
                sum += a[k];
                prefix += p[k];
                weighted += (16 - k) * a[k];
            }
            /* No sum here exceeds the s2 the scalar loop would reach. */
            s2 += (uint32_t)(chunks * 16) * s1 + 16 * prefix + weighted;
            s1 += sum;
            block -= chunks * 16;
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
    /* The hash of position i, carried over from the previous step: lazy
       matching has already hashed i + 1, and after a match it is computed
       before the token is emitted. */
    unsigned int h = data_len > 3 ? zhash(data) : 0;
    while (i < data_len - 3) {
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

        int next_hashed = 0;
        if (bestpos >= 0) {
            /* Lazy matching: if the match at i+1 is longer, emit i as a literal. */
            int limit1 = data_len - i - 1 < MAX_MATCH ? data_len - i - 1 : MAX_MATCH;
            if (best < limit1) {
                unsigned int h1 = zhash(data + i + 1);
                h = h1; /* i + 1 < data_len - 3, as limit1 > best >= 3 */
                next_hashed = 1;
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
            i += best;
            if (i < data_len - 3) h = zhash(data + i);
            add_bits(&o, code, (int)bits);
        } else {
            huff(&o, data[i]);
            i++;
            if (!next_hashed && i < data_len - 3) h = zhash(data + i);
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
