/* The deflate.c snapshot under a per-build prefix, so run.sh can link several
   builds of it (default, -DTERMSHOT_PORTABLE_ADLER, and SHIM_ZEROED_TABLE)
   into one process. run.sh renames termshot_zlib_compress and
   termshot_deflate_profile with -D and names the exported Adler-32 wrapper
   with SHIM_ADLER32.

   SHIM_ZEROED_TABLE allocates the hash table with calloc instead of malloc,
   as the safe Rust port must (vec! zeroes), to measure what that costs. The
   table is deflate.c's only malloc. */
#ifdef SHIM_ZEROED_TABLE
#include <stdlib.h>
#define malloc(n) calloc(1, (n))
#endif
#include "deflate.c"

uint32_t SHIM_ADLER32(const unsigned char *d, size_t len) { return adler32(d, len); }
