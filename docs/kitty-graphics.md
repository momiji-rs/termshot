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

- `RUSTUP_TOOLCHAIN=1.70.0 ./test.sh`: 125 unit tests passed; one existing
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


## Partial scroll-region regression

[Review finding](https://github.com/momiji-rs/termshot/pull/43#discussion_r4170723891)
confirmed against `af189d8`: a picture crossing either scroll margin lost pixels
outside the scrolled region. Each placement now retains disjoint visible slices
sharing its original pixels and identity. Only the intersecting slices move;
outside slices stay fixed, clipped pixels stay gone, and adjacent slices with
the same source mapping are coalesced. Splitting does not multiply image storage
or consume additional placement IDs/quota.

| Before the scroll fix | After |
| --- | --- |
| ![Lost outside pixels](kitty-scroll-before.png) | ![Outside pixels preserved](kitty-scroll-after.png) |

```sh
./termshot --cursor none --size 12x8 --px 24 \
  tests/fixtures/kitty-scroll.pty /tmp/kitty-scroll.png
```

The end-to-end raster oracle fails on `af189d8` with:

```text
assertion `left == right` failed: scroll up at (11, 41)
  left: [17, 24, 35, 255]
 right: [255, 0, 0, 255]
```

Seven pixel comparisons cover scrolling up/down, a completely cleared region,
insert/delete line, index and reverse index. Three additional unit tests cover
both margins, reverse scrolling without resurrection, deletion/replacement and
quota semantics after splitting, and 2,000 partial scroll operations compared
against an independent pixel-row model. The existing 25 golden hashes are
unchanged; the new scroll fixture adds one hash.

Remaining protocol work: Sixel is already tracked by #41; advanced kitty
features now have a dedicated follow-up, #44.
