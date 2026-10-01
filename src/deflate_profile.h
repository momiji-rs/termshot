#ifndef TERMSHOT_DEFLATE_PROFILE_H
#define TERMSHOT_DEFLATE_PROFILE_H

/* One compressor call per thread. Disabled unless draw_png enables profiling. */
typedef struct {
    int enabled;
    double allocate_ms, match_emit_ms, finalize_ms, checksum_ms;
} DeflateProfile;
extern _Thread_local DeflateProfile termshot_deflate_profile;

#endif
