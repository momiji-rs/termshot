# termshot

Turn raw terminal output (a PTY log with ANSI escapes) into a PNG of the final screen.
Headless: no window, no terminal, no crates.io dependencies. The same source builds
on macOS and Linux and writes the same pixels.

termshot reads bytes a terminal already emitted, rebuilds the cell grid, and paints it.
The screenshot below, 2200×1440, takes about 19 ms from start to finished file
on the Apple M3 measured below.

![A 100 by 30 demo inbox, rasterized from examples/reply-sent.pty](docs/reply-sent.png)

## When to use it

We built it for TUI work, where the thing to check is what the screen ends up showing:

- **Developing a TUI.** Record what the app writes, render it, and look at the frame without
  opening a terminal. This also works on a remote box or inside an agent loop.
- **Verifying output.** Same bytes in, same PNG out. Commit a reference image and compare
  against it in CI to catch layout, color, or box-drawing regressions.
- **Screenshots for docs, READMEs, and PRs.** You get a crisp image at any pixel height. It
  doesn't depend on your terminal theme, font, or window size.
- **Bug reports.** Ask for the raw log, not a phone photo of the screen, and render it at your
  end.

It draws one final frame, not an animation. The screen model follows xterm and covers:

- autowrap, scrolling, scroll regions, inserting and deleting lines, and the alternate screen
  that full-screen programs use (`vi`, `less`)
- cursor movement, tabs, erase, inserting and deleting characters, saving the cursor, and DEC
  line drawing (`ESC ( 0`)

`tests/vt/` checks this against tmux, on short cases and on recorded `ls`, `less` and `vi`
sessions. A bare LF moves down without returning to column 0, as in a terminal; logs captured
through a PTY already have CR LF.

Colors are the 16 and 256 color palettes (xterm's defaults) and 24-bit color, in the `;` and
`:` forms. Bold, dim, underline, double underline, strike-through, reverse video and hidden
text are drawn; italic and blink are not. Wide characters (CJK, fullwidth forms, emoji) take two
cells. A combining mark is kept only when Unicode has a precomposed form for it; other marks are
dropped ([#14](https://github.com/solcreek/termshot/issues/14)).

## Speed

Times below cover one whole run of `./termshot examples/reply-sent.pty out.png <font> <px>`:
process start, input, font validation, parse, rasterize, PNG encode, and file close.

| px | Image | PNG | Apple M3 median | p95 |
| --- | --- | ---: | ---: | ---: |
| 24 | 1100×720 | 79 KB | 7.14 ms | 10.48 ms |
| 48 | 2200×1440 | 206 KB | 19.50 ms | 52.34 ms |
| 128 | 5800×3840 | 955 KB | 35.78 ms | 50.72 ms |

Measured on macOS 26.3.1 with 24 GiB RAM, 2026-10-01: three warmups and 30 runs
per case, interleaved with baseline binaries. These are warm-filesystem measurements
on a shared machine. Current Linux timings have not been measured.

The latest round reduces median latency by 1.26–2.10× across 12 workloads against
main at `1eaf7dd`, with byte-identical PNGs. Painting directly into PNG scanlines
removes a full image copy and buffer; faster DEFLATE emission, Adler-32, and CRC-32
reduce encoding work. ASCII parsing, font validation, and repeated rounded corners
also improve. At 48 px, measured peak RSS falls from 22.78 to 13.75 MiB.

See [performance measurements](docs/performance.md) for all workloads, per-stage
timings, raw samples, memory usage, validation, and remaining bottlenecks.

## Build


A C compiler and rustc 1.70 or newer are enough.

```sh
./build.sh
```

The binary links libc and libm.

## Test

```sh
./test.sh
```

This builds termshot, runs the parser unit tests, checks box drawing (`tests/boxes.c`) and glyph placement (`tests/glyphs.c`: wide characters, missing glyphs), checks the CLI exit codes, and compares the rendered samples against `tests/goldens.txt`. The goldens hash decoded pixels (stb_image decodes them, `tests/golden.rs` hashes them), so a change to how the PNG is encoded doesn't break them; only a change to the pixels does. They cover the two samples at px 46 and 48 and the edge cases in `tests/fixtures/` (clipping, missing glyphs, escapes, random colors, box drawing from px 1 to 255), and CI runs them on Linux and macOS. px 46 is there because it is a size where a compiler that fuses multiply-adds would render different pixels.

When a change is meant to move pixels, look at the renders in `target/test/`, then run `./test.sh --update-goldens`. `SANITIZE=1 ./test.sh` builds draw.c with ASan and UBSan; this works on macOS only.

Tests marked `#[ignore]` describe the correct behaviour for a known bug and name its issue. Run them with `./target/test/unit --ignored`.

## Run

```sh
./termshot examples/reply-sent.pty reply-sent.png
```

The binary carries JetBrains Mono, so it needs no other files. The grid defaults to 100×30
and the font pixel height to 48. Give the size the capture used:

```sh
./termshot --size 120x40 session.pty session.png
./termshot --px 24 --font path/to/font.ttf session.pty session.png
cat session.pty | ./termshot - - > screen.png   # stdin to stdout
./termshot --fallback-font /path/to/cjk.ttf session.pty session.png
```

| option | |
|---|---|
| `-f`, `--font FILE` | TrueType font (default: built-in JetBrains Mono) |
| `--fallback-font FILE` | TrueType font for characters the first lacks, such as CJK |
| `-p`, `--px N` | font pixel height, above 0 and below 256 (default 48) |
| `-s`, `--size CxR` | grid columns × rows, up to 500×200 (default 100x30) |
| `-v`, `--verbose` | print the cell and image size to stderr |
| `-h`, `--help`, `-V`, `--version` | |

It prints nothing on success. Exit status is 0 when done; 1 when a file can't be read or written,
or the font is unusable; and 2 for bad arguments, including an image over 2^27 pixels. termshot
won't write a PNG to a terminal. A failed run removes the output file it created.

The original form, `termshot <log> <out.png> <font.ttf> [px] [cols] [rows]`, still works.

`M` is snapped to a whole number of pixels so box-drawing joints meet. All box drawing and block elements (U+2500–U+259F: light, heavy, double and dashed lines, corners, tees, arcs, diagonals, eighths, shades and quadrants) are painted as geometry inside their cell, so lines join with any neighbour at any size; `tests/boxes.c` checks every one against its Unicode name. Other characters come from the font, then from `--fallback-font`, which is sized to the same height and centered in the cell; a character neither has is drawn as an outlined box, except for spaces. Wide characters (CJK, fullwidth forms, emoji, by Unicode 17 widths) take two cells and are centered over both; a combining mark merges into the character before it when Unicode has the precomposed form (e + U+0301 is é) and is otherwise dropped. An SGR reset uses foreground `#dbe7f7` on background `#111823`.

The font must be TrueType, meaning it has `glyf` outlines; CFF-based `.otf` fonts are rejected. Color emoji fonts are bitmaps, not outlines, so emoji need a monochrome outline font such as Noto Emoji. stb_truetype trusts the file it reads, so termshot first checks every structure stb will use (`src/font.rs`). A damaged or hostile font is refused with a reason, and the run exits 1.

## Samples

`examples/reply-sent.pty` and `examples/draft-ready.pty` are captures from the [crisp-tui](https://github.com/solcreek/crisp-tui) demo inbox. The customers and messages are fake. At pixel height 48 the image is 2200×1440.

## Measure and test

```sh
TERMSHOT_PROFILE=1 ./termshot examples/reply-sent.pty /tmp/reply.png \
  third_party/jetbrains-mono/JetBrainsMono-Regular.ttf
python3 scripts/bench.py --binary current=./termshot --runs 15 \
  --output /tmp/termshot-bench.json
SANITIZE=1 ./tests/run.sh
```

Profiling writes two `termshot-profile` JSON records to stderr, covering input,
parsing, font reading/validation/padding, drawing, PNG filtering, compression
allocation/matching/emission/checksum, PNG packaging, and writing. The benchmark
measures ordinary CLI runs separately from profiling and optional peak RSS runs;
it records raw samples, median, p95, output size and hash, and toolchain details. Python 3 is needed only
for optional development scripts (benchmarks and CRC table generation); the tests
need only a C compiler and rustc.

See [performance measurements](docs/performance.md) for the before/after results,
baseline reproduction, timing boundaries, and remaining bottlenecks. Extended
pixel tests retain main's reference images and portable floating-point settings.

See [CHANGELOG.md](CHANGELOG.md) for notable changes.

## Vendored files

The program is MIT. Two things next to it keep their own terms:

- `third_party/stb/` is [stb](https://github.com/nothings/stb) `stb_truetype.h` 1.26 and `stb_image_write.h` 1.16, public domain. `stb_image.h` 2.30 is used only by the tests, to decode PNGs. Local writer hooks and fixes are documented in [CHANGES.md](third_party/stb/CHANGES.md).
- `third_party/jetbrains-mono/` is JetBrains Mono Regular, [SIL Open Font License 1.1](third_party/jetbrains-mono/OFL.txt).
