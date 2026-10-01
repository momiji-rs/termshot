#ifndef TERMSHOT_PNG_CRC_H
#define TERMSHOT_PNG_CRC_H
#include "crc32_table.h"

/* Each table consumes another byte of the CRC recurrence. Explicit little-
   endian assembly keeps the result portable and allows unaligned input. */
static uint32_t termshot_png_crc(unsigned char *p, int len) {
    uint32_t crc = ~0u;
    while (len >= 8) {
        crc ^= (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
               ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
        crc = crc32_table[7][crc & 255] ^ crc32_table[6][(crc >> 8) & 255] ^
              crc32_table[5][(crc >> 16) & 255] ^ crc32_table[4][crc >> 24] ^
              crc32_table[3][p[4]] ^ crc32_table[2][p[5]] ^
              crc32_table[1][p[6]] ^ crc32_table[0][p[7]];
        p += 8;
        len -= 8;
    }
    while (len-- > 0) crc = (crc >> 8) ^ crc32_table[0][(crc ^ *p++) & 255];
    return ~crc;
}
#endif
