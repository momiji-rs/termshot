# Kitty direct graphics: issue #41

The report is reproducible on `d65b1af`: `replay` passes APC and DCS to
`skip_string`, so a valid inline image produces only the background. Existing
string-skipping tests check that these payloads do not leak into text, but do
not assert image pixels. They remain valid and pass with the new image layer.

These are actual CLI outputs for the same committed 2×2 RGB fixture, with
`c=4,r=2`, a 6×4 screen, `--px 48`, and `--cursor none`:

| Before (`d65b1af`) | After |
| --- | --- |
| ![Missing image](kitty-before.png) | ![Four-color inline image](kitty-after.png) |

```sh
./termshot --cursor none --size 6x4 --px 48 \
  tests/fixtures/kitty-rgb.pty /tmp/kitty.png
```

The [kitty protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
requires preserving image aspect ratio when fitting a cell rectangle. With
this font, 4×2 cells are 88×96 pixels: the square image is 88×88 with four pixels
of empty space above and below. Stretching to cover the entire rectangle would
not follow that rule. The renderer uses integer nearest-neighbor sampling for
cross-platform reproducibility; it does not promise GPU-filter-identical output.

## Validation and reproduction

Local validation passed on Linux x86-64:

- `RUSTUP_TOOLCHAIN=1.70.0 ./test.sh`: 132 unit tests passed; one existing
  benchmark helper is ignored. All CLI, pixel, codec and geometry checks passed.
- `SANITIZE=1 UBSAN_OPTIONS=halt_on_error=1 ./tests/run.sh`: passed, including
  the image decoder/compositor, codec round trips and concurrent render checks.

`./test.sh` runs the complete suite, including:

- Parser/state tests: RGB/RGBA, chunks and final-chunk cursor position, native
  and cell sizing, cursor policy, deletion/replacement, z order, scrolling and
  clipping, alternate screens, resets, malformed input and resource limits.
- `tests/graphics.rs`: explicit expected RGBA pixels for RGB, RGBA, PNG and
  PNG-with-alpha at five pixel heights (1, 9, 24, 47.5, 128): **843,072 pixel
  checks**, plus native clipping, text layering, transparency and deletion.
  Expected colors and geometry are computed independently of production code.
- Six new decoded-pixel goldens; all 20 existing hashes stay unchanged.
- `tests/image.c`: production PNG decoding, every truncation of a valid PNG,
  2,000 deterministic byte mutations, quota exhaustion and recovery, and
  allocation accounting. The Rust tests also decode PNGs concurrently.

After a test build, run the same pixel assertions against a pre-fix binary:

```sh
target/test/graphics /path/to/termshot-before
```

The baseline fails with:

```text
assertion `left == right` failed: rgb px=1 at (0,0)
  left: [17, 24, 35]
 right: [255, 0, 0]
```

`SANITIZE=1 UBSAN_OPTIONS=halt_on_error=1 ./tests/run.sh` additionally checks the
production image decoder and C image compositor under AddressSanitizer and
UndefinedBehaviorSanitizer. CI runs the pixel suite on Linux x86-64, Linux
arm64 and macOS, and tests Rust 1.70 compatibility.

## Scope

This implements the issue's first stage, direct `a=T` transmission, with an
independent image layer. The PNG decoder reuses vendored stb and adds no
external library dependency. Sixel remains deferred. See the README's supported
subset before using this to test a TUI: Unicode-placeholder placements and
external-file transfers remain unsupported. Separate transmit and put came
with #44; see [Stored images and placements](#stored-images-and-placements).
It has not been validated against captures of AgentAmp or yazi, or compared
pixel-for-pixel with a live kitty terminal.


## Scroll margins and explicit IDs

[Protocol review](https://github.com/momiji-rs/termshot/pull/43#pullrequestreview-5398124081)
was verified against the [kitty specification's terminal interactions](https://sw.kovidgoyal.net/kitty/graphics-protocol/#interaction-with-other-terminal-actions).
A placement crossing either margin stays stationary as a whole. Only placements
entirely inside the region scroll; clipping at an edge is permanent, including
when scrolling reverses. Eligibility uses the remaining visible bounds.

| Before (`5090104`): image split into bands | After: whole crossing image stays stationary |
| --- | --- |
| ![Torn crossing image](kitty-scroll-before.png) | ![Stationary whole image](kitty-scroll-after.png) |

```sh
./termshot --cursor none --size 12x8 --px 24 \
  tests/fixtures/kitty-scroll.pty /tmp/kitty-scroll.png
```

The corrected unit tests fail on `5090104`: scrolling a placement spanning
pixel rows 20–80 through a region starting at 40 incorrectly leaves only 20–40.
The end-to-end oracle also rejects that binary. Fourteen full-raster comparisons
cover crossing and contained placements, both directions, region clearing,
IL/DL and IND/RI. Unit tests cover each margin separately, both margins together,
exact-edge containment, reverse scrolling without resurrection, deletion and
replacement at the quota limit, plus 2,000 modeled scroll operations. Only the
`kitty-scroll` PNG golden changes; the other 25 PNG hashes and all text/JSON
goldens remain unchanged.

The [image-ID rule](https://sw.kovidgoyal.net/kitty/graphics-protocol/#querying-support-and-available-transmission-mediums)
requires supplied IDs to be nonzero. Tests reject `i=0`, `i=000` and overflow,
without placing an image, advancing the cursor or leaking payload into text;
omission, `i=1` and `i=4294967295` remain valid. The zero-ID test fails on
`5090104` before the parser fix.

Remaining protocol work: Sixel is already tracked by #41; advanced kitty
features now have a dedicated follow-up, #44.


## Stored images and placements

The first stage of #44 separates stored images from their placements, with
the semantics taken from kitty's `docs/graphics-protocol.rst` and
`kitty/graphics.c` (master, read 2026-10-02) rather than from the earlier
`a=T`-only code:

- `a=t` stores an image, `a=p` places a stored one at the cursor, and `a=T`
  does both. Placements share the image's pixels (`Rc`), so putting one image
  many times costs no pixel copies. `a=t` with neither `i` nor `I` stores
  nothing, as kitty trims such an image at once.
- `I` names the newest image with that number. Such an image gets the lowest
  free id (kitty's `get_free_client_id`). Giving both `i` and `I` is ignored.
- `(i, p)` with a nonzero `p` names one placement; putting it again moves it.
  `p` is ignored on an image without an id.
- Retransmitting an id removes the old image and its placements as the
  transmission starts, so a failed retransmission leaves nothing to place.
- Draw order is kitty's: z-index, then image creation, then placement
  creation. The earlier code broke ties by image id, which kitty does not.
- Delete selectors `a i n r c p q x y z` follow `handle_delete_command` and
  `filter_refs`: lowercase removes placements and frees only images without
  an id; uppercase also frees the images it emptied. `d=I`/`d=N` without `p`
  and `d=R` also free matching images that already had no placement; `d=A`
  keeps stored images it did not touch. Cell selectors use the cells each
  placement covers, which follow scrolling and are clipped at the margins as
  kitty's `scroll_filter_margins_func` clips them. `f` (frames) is ignored.
- ED 2, RIS and entering the alternate screen act as `grman_clear`: every
  placement goes and every image left without one is freed, stored ones too.
- Over 16 MiB or 4,096 images, an upload frees every image without a
  placement (except itself), then the least recently placed, with their
  placements, as `apply_storage_quota` does. The image count limit is
  termshot's: it bounds the linear lookups. kitty's own quota is 320 MB.

`src/graphics/tests.rs` covers each selector in both cases against a fixed
scene, freeing of stored images, the quota order, numbers, placement moves,
retransmission and scrolled cell bounds. Seventeen hand-made mutants of this
logic all fail the tests. `tests/graphics.rs` checks one full raster with two
stored images put in seven cells: draw order by z-index and creation, a moved
placement and a delete by column, against pixels it computes itself.


## Compressed payloads

`o=z` follows kitty's `inflate_zlib` and `initialize_load_data` (master, read
2026-10-03). The payload, after base64 and chunking, is an RFC 1950 zlib
stream that must inflate to exactly the data's size: `w*h*3` or `w*h*4` for
raw pixels, and for PNG the `S` key, or 100 KiB without it, as kitty assumes.
Any other `o` is ignored, as kitty refuses it. `S` changes nothing on an
uncompressed transmission.

The inflater is stb_image's, already built into `src/image.c` for PNG, so
nothing new is vendored. `image_inflate` adds what kitty gets from zlib and
stb skips: the Adler-32 trailer, the window field (at most 32 KiB) and the
exact output size; bytes after the trailer are ignored, as zlib ignores them.
It finds the trailer from the bits stb has buffered, and the caller pads the
input with 8 zero bytes so that read-ahead stays in the buffer. The output is
the image buffer itself, sized before inflating and limited to 16 MiB, so a
stream cannot allocate more than its size promises.

`tests/image.c`, also run under ASan and UBSan, round-trips noise, runs and
mixtures up to 70,000 bytes through the renderer's compressor, and checks
sizes one short and one long, trailing bytes, every change to the trailer,
truncations, the window and dictionary fields, and 2,000 mutations. It
includes data whose Adler-32 has a zero low half, where the zero padding
matches a trailer cut short, so only the bounds checks can refuse it.
`src/graphics/tests.rs` checks each format, `S`, the 100 KiB default, chunks
and the 16 MiB limit at its edge. `tests/graphics.rs` renders the four
fixture images again from `tests/fixtures/kitty-*-z.pty`, compressed by
Python's zlib, against the same expected pixels. Ten hand-made mutants of
the checks all fail the tests.


### Payload cap and early checks

kitty sizes a direct upload's buffer at the decoded size plus 1,024 bytes
for a compressed one (`initialize_load_data`) and refuses an RGB or RGBA
payload that would overflow it (`load_image_data`, `EFBIG`); only PNG may
grow. termshot applies the same cap to compressed RGB and RGBA, over all the
chunks of an upload and checked as each arrives, so an oversized upload is
dropped before it is fully buffered. A compressed PNG keeps the 16 MiB
payload limit. Raw dimensions are checked against the decoded limits (8,192
pixels per axis, 16 MiB of RGBA) before inflating, so a stream for an image
that would be refused anyway is never inflated.

`src/graphics/zlib_tests.rs` checks the cap at 1,024, 1,027, 1,028 and 1,029
bytes, alone and chunked; a stream cut at every chunk size, for stored and
Huffman-coded streams; interrupted uploads; that bytes after the trailer are
ignored; the early dimension checks; and 100,000 or 16 MiB of zeros sent for
smaller, near and exact images. `tests/image.c` inflates a hand-built
104 KiB fixed-Huffman stream of 16,908,289 zeros into heap buffers of exactly
12 bytes, 16 MiB, and the exact size and one either side under ASan, and
inflates with the allocation quota spent. `kitty-rgb-z-chunks.pty` is
`src/deflate.c`'s compression of `kitty-rgb`'s pixels, cut across three
chunks inside the DEFLATE data. Goldens for it and the four `-z` fixtures hash
the same as the uncompressed renders; the existing goldens are unchanged.


## Crops, offsets and layers

The third stage of #44 follows the spec's "Controlling displayed image
layout" section and kitty's `handle_put_command`, `update_dest_rect`,
`grman_update_layers`, `shaders.c` and `background.slang` (master, read
2026-10-03):

- **Crop.** `x, y, w, h` (0 meaning the rest of the image) are intersected
  with the image, as `handle_put_command` clamps them. A crop starting past
  the image is empty: the put draws nothing, but still replaces the
  placement it names and moves the cursor past the cells `c`, `r` and the
  offset span. termshot does not keep an empty placement, which kitty does;
  only the delete selectors and the storage quota could tell them apart.
- **Offsets.** `X, Y` move the image inside its first cell. The spec says
  they must be smaller than the cell; kitty clamps them to a pixel short of
  its edge, and so does termshot. `c` and `r` still count cells from the
  cell's edge, so an offset shrinks the space they give (the spec: "it is not
  added to the number of rows/columns"; kitty's destination rectangle ends at
  the `c`-th column's edge).
- **Aspect ratio.** The crop's, not the image's. With one of `c` and `r`, the
  other side follows it; with both, the crop is fitted inside and centered,
  as the spec's "letterboxed/pillarboxed" says. (kitty's own code fills the
  rectangle for an ordinary placement and letterboxes only Unicode-placeholder
  ones; termshot keeps the spec's rule, as it did before this stage.)
- **Cells covered** run from the cell to the far edge of that space. kitty
  computes the width for an `r`-only placement from `r` rows *plus* the
  vertical offset, which is not the width it draws; termshot uses the drawn
  extent. This changes the cursor only when `r`, no `c` and `Y` are given.
- **Layers.** kitty draws the default background, then images with `z` below
  `INT32_MIN / 2` (-1,073,741,824), then the cell backgrounds that are not the
  default, then other negative `z`, then the text and cursor shapes, then
  `z >= 0`. A background is the default one when its colour *value* is the
  default background (`cell_has_default_bg` compares colours), except for
  reverse video, the block cursor and selections, which kitty makes opaque.
  termshot marks reverse-video cells and the cells under the block cursor
  with a new `Cell` bit, `ATTR_OPAQUE` (128, in both languages), and paints
  the lowest layer only over pixels of the cells left clear. The block cursor
  is a background in kitty, so an image with a negative `z` above
  -1,073,741,824 covers it, and only the cursor's text colour stays on top.
- **Shades.** U+2591–2593 now blend their colour over what is painted, not
  over the cell's background colour, so an image under the text shows
  through them as through a glyph. Without such an image the pixels are the
  same, and every existing golden hash is unchanged.
- The bar and underline cursors stay above every image, as before. In kitty
  they are drawn with the text, so a `z >= 0` image covers them; that is a
  separate question from this stage.

Tests: `src/graphics/geometry_tests.rs` covers crops in and past each edge,
empty crops, offsets at and past the cell size, offsets with `c`, `r` and
both, the crop's aspect ratio, the signed `z` parser at its limits, draw and
delete order by negative `z`, moving a placement with new geometry, scrolling
and clipping an offset image, and extreme cell metrics. `tests/draw.c`, also
under ASan and UBSan, checks the layer boundaries, the clear-background mask
and crop sampling in `paint_images`. `tests/graphics.rs` checks every pixel
against rasters it builds from the rules above: a crop scene at px 9, 24 and
47.5, plain and scrolled, and the six layer boundaries (`z` = -2^31,
-2^30 - 1, -2^30, -1, 0, 7) over a row of text, a red, a default-valued and
a reverse-video background and a shade, opaque and half transparent, with no
cursor, a block and a bar, at px 9, 24 and 47.5 (292,896 pixel checks). The
crop checks fail against `origin/main`. `kitty-crop` and `kitty-layers` add
four goldens; the existing goldens are unchanged.


## ASCII autowrap regression

[Copilot review](https://github.com/momiji-rs/termshot/pull/43#discussion_r4170788572)
identified two optimized ASCII paths that rotated cell storage without moving
images. Confirmed against `b125bc9`. All row rotations now use `rotate_rows`,
which updates graphics from the logical scroll distance before taking the
storage rotation modulo. Image clipping saturates at one whole region, so
very large skips remain safe and cannot leave stale images.

The existing fast-versus-scalar printing test now compares placements as well
as cells and cursor state across grid sizes, scroll margins, wrapping modes,
alternate screens and run lengths up to 4,097 bytes. A separate check covers
whole-region multiples and extreme counts in both directions. Three additional
end-to-end pixel comparisons match ASCII autowrap against explicit scroll-up:
a single row, multiple skipped regions, and a trailing partial row. The contained-image fixture ensures these checks still exercise actual image
movement under the scroll-margin rule.


## Main integration

Merged italic support from `main` at `5bd367f`. The renderer keeps the shared
cell metrics used by graphics and the italic pivot used by both fonts. Both
italic PNG goldens retain their upstream hashes alongside all 26 graphics-era
goldens (28 total). The glyph-placement harness also checks italic and fallback
font geometry. Text/JSON retain upstream italic attributes.
