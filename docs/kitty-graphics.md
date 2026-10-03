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

- `RUSTUP_TOOLCHAIN=1.70.0 ./test.sh`: 131 unit tests passed; one existing
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
subset before using this to test a TUI: Unicode-placeholder placements,
separate transmit/put commands and external-file transfers remain unsupported.
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
