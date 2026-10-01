# termshot

Turn raw terminal output (a PTY log with ANSI escapes) into a PNG of the final screen.
Headless: no window, no terminal, no crates.io dependencies. The same source builds
on macOS and Linux and writes the same pixels.

termshot reads bytes a terminal already emitted, rebuilds the cell grid, and paints it.
The screenshot below, 2200×1440, takes about 20 ms from start to finished file.

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
Cursor movement, erase in line and in display, and escape and string sequences are parsed the
way a VT terminal does. A bare LF moves down without returning to column 0. Logs captured
through a PTY already have CR LF.

## Speed

A frame costs milliseconds, which is cheap enough to render one per test or on every save of a watch loop. Times are for one whole run of `./termshot examples/reply-sent.pty out.png <font> <px>`: process start, parse, rasterize, PNG encode, and the file write.

| px | image | PNG | Apple M3, macOS | Ryzen 7 8745HS, Linux |
|---|---|---|---|---|
| 24 | 1100×720 | 79 KB | 10 ms | 8 ms |
| 48 | 2200×1440 | 206 KB | 21 ms | 15 ms |
| 128 | 5800×3840 | 955 KB | 94 ms | 64 ms |

Each figure is the mean of 40 runs, measured 2026-10-01: hyperfine on macOS, a shell loop on Linux.

Nearly all the time goes to PNG compression. termshot uses stb_image_write's deflate with a faster match search (`src/deflate.c`). The search writes exactly the bytes stock stb would for nonempty input, and `./test.sh` compares both implementations on 3000 inputs. An empty-input defect is fixed in both implementations. Together with RGB output and no PNG row filter, a run is 7.7× faster than one that uses stock stb at px 48, and 13× faster at px 128. Rendering the cells themselves takes a few milliseconds.

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
parsing, font loading, drawing, PNG filtering, compression, and writing. The
benchmark measures ordinary CLI runs separately from profiling and records raw
samples, median, p95, output size, and toolchain details. Python 3 is needed only
for benchmarks and tests.

See [performance measurements](docs/performance.md) for the before/after results,
baseline reproduction, timing boundaries, and remaining bottlenecks. Extended pixel tests compare against the renderer at `8e1110e`, including its
terminal parsing fixes and portable floating-point settings.

See [CHANGELOG.md](CHANGELOG.md) for notable changes.

## Vendored files

The program is MIT. Two things next to it keep their own terms:

- `third_party/stb/` is [stb](https://github.com/nothings/stb) `stb_truetype.h` 1.26 and `stb_image_write.h` 1.16, public domain. `stb_image.h` 2.30 is used only by the tests, to decode PNGs. Local writer hooks and fixes are documented in [CHANGES.md](third_party/stb/CHANGES.md).
- `third_party/jetbrains-mono/` is JetBrains Mono Regular, [SIL Open Font License 1.1](third_party/jetbrains-mono/OFL.txt).
