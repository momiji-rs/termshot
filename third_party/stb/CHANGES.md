# Local changes to stb_image_write.h 1.16

The original vendored header is available at commit `f0a7993`. Keep the upstream
license intact when updating. `stb_truetype.h` is unchanged.

- Compare DEFLATE matches eight bytes at a time using unaligned-safe `memcpy` loads.
- Search match candidates newest first, retaining the original tie preference;
  stop at the maximum possible match length. Reject candidates that cannot beat
  the best length before comparing their prefixes. Skip lazy matching when a
  longer match is impossible.
- Emit an all-zero Up filter for identical PNG scanlines, and stop filter search
  when its score reaches zero. Explicit filter selection and vertical flipping
  still work. PNG encoding can change; decoded pixels do not.
- Add optional `STBIW_PNG_PROFILE(stage)` hooks at filter start, DEFLATE start,
  packaging start, and packaging end. The default hook does nothing.
- Preserve the final DEFLATE block for empty input. Upstream's stored-block
  fallback otherwise truncates it, producing an invalid zlib stream.
- Free compressed data if allocation of the final PNG buffer fails.

`tests/codec.c` and `tests/codec.py` check 4,976 independent zlib/PNG round trips,
including all PNG filters, 1–4 channels, padded strides, vertical flipping, short
inputs, DEFLATE window boundaries, random bytes, and repeated runs. Run
`SANITIZE=1 ./tests/run.sh` after changes. The renderer's original pixel hashes
provide an additional end-to-end check on both macOS and Linux CI.
