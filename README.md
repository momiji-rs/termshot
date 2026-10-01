# termshot

Turn raw terminal output (a PTY log with ANSI escapes) into a PNG of the final screen.
Headless: no window, no terminal, no crates.io dependencies. The same source builds
on macOS and Linux and writes the same pixels.

termshot reads bytes a terminal already emitted, rebuilds the cell grid, and paints it.
The screenshot below, 2200×1440, takes about 9 ms from start to finished file
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

It is not a terminal emulator. It draws one final frame, not an animation. Colors come from
24-bit SGR (`38;2;r;g;b` and `48;2;r;g;b`, or the `38:2::r:g:b` colon form); scrolling and the
16 and 256 color palettes are not interpreted yet ([#6](https://github.com/solcreek/termshot/issues/6)).
Cursor movement, tabs, erase, insert and delete of characters, save and restore of the cursor,
DEC line drawing (`ESC ( 0`), and escape and string sequences are parsed the way xterm does. A bare LF moves down without returning to column 0. Logs captured
through a PTY already have CR LF.

## Speed

Times below cover one whole run of `./termshot examples/reply-sent.pty out.png <font> <px>`:
process start, input, font validation, parse, rasterize, PNG encode, and file close.

| px | Image | PNG | Apple M3 median | p95 |
| --- | --- | ---: | ---: | ---: |
| 24 | 1100×720 | 79 KB | 5.34 ms | 5.98 ms |
| 48 | 2200×1440 | 206 KB | 9.07 ms | 12.55 ms |
| 128 | 5800×3840 | 955 KB | 31.76 ms | 34.38 ms |

Measured on macOS 26.3.1 with 24 GiB RAM, 2026-10-01: three warmups and 30 runs
per case, interleaved with baseline binaries. These are warm-filesystem measurements
on a shared machine. Current Linux timings have not been measured.

The latest round reduces median latency by 1.22–2.26× across 12 workloads against
main at `255fa3a`, with byte-identical PNGs. Painting directly into PNG scanlines
removes a full image copy and buffer; faster DEFLATE emission, Adler-32, and CRC-32
reduce encoding work. ASCII parsing, font validation, and repeated rounded corners
also improve. At 48 px, measured peak RSS falls from 22.67 to 13.52 MiB.

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

This builds termshot, runs the parser unit tests, checks the CLI exit codes, and compares the rendered samples against `tests/goldens.txt`. The goldens hash decoded pixels (stb_image decodes them, `tests/golden.rs` hashes them), so a change to how the PNG is encoded doesn't break them; only a change to the pixels does. They cover px 46 and 48, and CI runs them on Linux and macOS. px 46 is there because it is a size where a compiler that fuses multiply-adds would render different pixels.

When a change is meant to move pixels, look at the renders in `target/test/`, then run `./test.sh --update-goldens`. `SANITIZE=1 ./test.sh` builds draw.c with ASan and UBSan; this works on macOS only.

Tests marked `#[ignore]` describe the correct behaviour for a known bug and name its issue. Run them with `./target/test/unit --ignored`.

## Run

```sh
./termshot examples/reply-sent.pty reply-sent.png \
  third_party/jetbrains-mono/JetBrainsMono-Regular.ttf 48
```

`48` is the font pixel height. Columns and rows default to 100 and 30, which is the size of the sample logs. Pass them after the pixel height when the capture used another size:

```sh
./termshot session.pty session.png path/to/font.ttf 48 120 40
```

`M` is snapped to a whole number of pixels so box-drawing joints meet. `─ │ ┌ ┐ └ ┘ ╭ ╮ ╯ ╰ ▀ █` are painted as geometry. Other characters come from the font. An SGR reset uses foreground `#dbe7f7` on background `#111823`.

The font must be TrueType, meaning it has `glyf` outlines; CFF-based `.otf` fonts are rejected. stb_truetype trusts the file it reads, so termshot first checks every structure stb will use (`src/font.rs`). A damaged or hostile font is refused with a reason, and the run exits 1.

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
for benchmarks and tests.

See [performance measurements](docs/performance.md) for the before/after results,
baseline reproduction, timing boundaries, and remaining bottlenecks. Extended pixel tests compare against the renderer at `8e1110e`, including its
terminal parsing fixes and portable floating-point settings.

See [CHANGELOG.md](CHANGELOG.md) for notable changes.

## Vendored files

The program is MIT. Two things next to it keep their own terms:

- `third_party/stb/` is [stb](https://github.com/nothings/stb) `stb_truetype.h` 1.26 and `stb_image_write.h` 1.16, public domain. `stb_image.h` 2.30 is used only by the tests, to decode PNGs. Local writer hooks and fixes are documented in [CHANGES.md](third_party/stb/CHANGES.md).
- `third_party/jetbrains-mono/` is JetBrains Mono Regular, [SIL Open Font License 1.1](third_party/jetbrains-mono/OFL.txt).
