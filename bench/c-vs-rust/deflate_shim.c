/* The deflate.c snapshot under a per-build prefix, so run.sh can link two
   builds of it (default, and -DTERMSHOT_PORTABLE_ADLER) into one process.
   run.sh renames termshot_zlib_compress and termshot_deflate_profile with -D
   and names the exported Adler-32 wrapper with SHIM_ADLER32. */
#include "deflate.c"

uint32_t SHIM_ADLER32(const unsigned char *d, size_t len) { return adler32(d, len); }
