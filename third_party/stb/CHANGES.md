# Local changes to stb_image_write.h 1.16

The unmodified vendored header is available at commit `8e1110e`. Keep the
upstream license intact when updating. `stb_truetype.h` is unchanged.

- Add optional `STBIW_PNG_PROFILE(stage)` hooks at filter start, DEFLATE start,
  packaging start, and packaging end. The default hook does nothing.
- Preserve the final DEFLATE block for empty input. Upstream's stored-block
  fallback otherwise truncates it, producing an invalid zlib stream. The same
  fix is applied to `src/deflate.c`; nonempty compressed output is unchanged.
- Free compressed data if allocation of the final PNG buffer fails.
- Extract compression and packaging into the internal
  `stbiw__write_png_from_filtered` helper, which borrows scanlines including their
  filter bytes. The renderer paints those scanlines directly, avoiding another
  full image allocation and copy. The generic writer still supports all original
  filters, channels, strides, and flipping, and owns/frees its filter buffer.
- The renderer uses the existing `STBIW_CRC32` hook with portable slicing-by-eight
  IEEE CRC-32 (`src/png_crc.h`); the generic writer's default CRC is unchanged.

`tests/codec.c` independently decodes and checks 5,024 zlib/PNG round trips
for each of the stock and custom compressors, including all PNG filters,
1–4 channels, padded strides, vertical flipping, short inputs, DEFLATE window
boundaries, Adler-32 block boundaries, random bytes, and repeated runs. It also
checks PNG chunk CRCs and zlib Adler-32 trailers, and compares the renderer's CRC
with stb for aligned and unaligned inputs.
`tests/deflate_alloc.c` also injects failures at each allocation in the custom
compressor, checking termination and cleanup, including pending output bits.
Run `SANITIZE=1 ./tests/run.sh`
after changes. `./test.sh` additionally compares both compressors byte for byte
on 3,000 seeded inputs. Renderer pixel hashes provide end-to-end checks on macOS
and Linux CI.

# Local changes to stb_image.h

- `stbi__parse_uncompressed_block` compares the bytes left with a stored
  block's length (`a->zbuffer_end - a->zbuffer < len`) instead of forming
  `a->zbuffer + len`, a pointer that is undefined behaviour when it lands more
  than one past the input. The result is the same for every input. `image_inflate`
  passes its input unpadded, so a stored block longer than what is left can
  reach it. `tests/image.c` checks one.
