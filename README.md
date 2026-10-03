# termshot

Turn raw terminal output (a PTY log with ANSI escapes) into a PNG of the final screen.
Headless: no window, no terminal, no crates.io dependencies. The same source builds
on macOS and Linux and writes the same pixels.

termshot reads bytes a terminal already emitted, rebuilds the cell grid, and paints it.
The screenshot below is 2200×1440.

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
- the cursor, drawn where the log leaves it, unless the log hides it (`ESC [ ? 25 l`): a block
  in reverse video, or the underline or bar a program picks with DECSCUSR (`ESC [ 5 SP q`, as
  shells in vi mode and editors do for insert mode), drawn steady

`tests/vt/` checks this against tmux, on short cases and on recorded `ls`, `less` and `vi`
sessions. A bare LF moves down without returning to column 0, as in a terminal; logs captured
through a PTY already have CR LF. For output that has bare LFs (a text file, `cmd > out.log`),
pass `--lf-newline`.

Colors are the 16 and 256 color palettes (xterm's defaults) and 24-bit color, in the `;` and
`:` forms. Bold, dim, italic, underline, double underline, strike-through, reverse video and
hidden text are drawn; blink is not. Italic is the font's own glyph slanted by 12 degrees, and
box drawing stays upright in it. Wide characters (CJK, fullwidth forms, emoji) take two
cells. A combining mark is kept only when Unicode has a precomposed form for it; other marks are
dropped ([#14](https://github.com/momiji-rs/termshot/issues/14)).

## Speed

How long a run takes depends on the log, the image size, the fonts, the disk
cache and the machine, so termshot has no single latency figure. The current
figures below come from the **[versioned benchmark report](docs/performance.md)**,
whose JSON files keep every raw sample of the round with the binary hashes,
toolchains, fonts and inputs. The historical figure at the end of this section
has no retained samples and is quoted only to say where it came from.

Current baseline: measured 2026-10-03 with `scripts/bench.py`, on termshot
built from `22b77e8` (main `d83c8fd` plus profiling timers that change no
output; on main since #57) with `build.sh`'s flags. Each figure is the median
of 40 **whole CLI runs**, wall time from spawn to exit: start-up, reading the
log and the fonts, parsing, drawing, PNG encoding and closing the file, with a
warm page cache and no `fsync`. They are not `TERMSHOT_PROFILE`'s internal
stage timers, which leave out process start-up and exit. Two batches with
different run orders are shown as A / B.

| workload | grid / px | image | fonts | Apple M2 Max, macOS 26.6.2 (ms) | Ryzen 7 8745HS, Arch Linux (ms) |
| --- | --- | --- | --- | ---: | ---: |
| `examples/reply-sent.pty` (`font-builtin`) | 100×30 / 48 | 2200×1440 | built-in JetBrains Mono | 9.27 / 9.24 | 9.64 / 9.58 |
| same log (`reply-24px`) | 100×30 / 24 | 1100×720 | JetBrains Mono file | 5.83 / 5.97 | 5.63 / 5.59 |
| same log (`reply-128px`) | 100×30 / 128 | 5800×3840 | JetBrains Mono file | 31.56 / 33.73 | 38.06 / 38.42 |
| `large` (generated) | 240×80 / 48 | 5280×3840 | JetBrains Mono file | 41.91 / 41.96 | 49.24 / 49.57 |
| `tests/perf/cjk-dense.pty` (`cjk-full`) | 100×30 / 24 | 1100×720 | built-in + `--fallback-font NotoSansCJK-Regular.ttc#3` (19 MB) | 12.67 / 13.53 | 14.85 / 14.67 |

The report covers 25 workloads; their batch-A medians range from 5.17 ms
(macOS) and 4.90 ms (Linux) for `cjk-none` to 41.91 and 49.24 ms for `large`.
It also gives p95, child CPU time, peak RSS, a per-stage breakdown and a
Linux cold-cache run, which adds 6.9-8.2 ms to three small-font cases. These
are two machines with warm caches; a different log, font, disk or a busy
machine can take longer. The release archives use the same compiler flags
but link musl statically on Linux; they were not measured.

Earlier rounds (2026-10-01, Apple M3, revisions up to `c44d83c`) are kept in
the report as history. They used a different machine, revision and harness, so
their numbers must not be subtracted from these. The "~20 ms for 2200×1440"
quoted in older descriptions was a 21 ms mean of 40 hyperfine runs on that M3
at `fb714a5`, whose samples were not kept; the report records
[what is known about it](docs/performance.md#published-claims-and-their-evidence-checked-2026-10-03).

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

A test for a known bug describes the correct behaviour and is marked `#[ignore = "#N: ..."]` with its issue. None are open now. Run them with `./target/test/unit --ignored`. The other ignored test, `poc_workloads`, writes inputs for `bench/c-vs-rust/` and checks nothing.

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

To screenshot a TUI that is still running, drive it in tmux and capture the pane. tmux ends
each row with a bare LF, so pass `--lf-newline` and the pane's size. The capture doesn't say
where the cursor is, so ask tmux and pass it with `--cursor`:

```sh
tmux new-session -d -s app -x 100 -y 30 top
cursor=$(tmux display -p -t app '#{?cursor_flag,#{cursor_x}#,#{cursor_y},none}')
tmux capture-pane -t app -e -p | ./termshot --lf-newline --size 100x30 --cursor "$cursor" - top.png
```

To check what a screen shows rather than how it looks (in a test, or as an agent), write it as
text. It is laid out as `tmux capture-pane -p` prints it: a line per row, trailing spaces
trimmed. For logs without graphics, omitting the PNG skips font loading, drawing and PNG
encoding; how much time that saves depends on the log, and the benchmark report does not
time text-only runs. Kitty graphics still need font metrics to replay cursor movement,
even for text/JSON-only output; images themselves are not included in these formats:

```sh
./termshot --text - session.pty | grep -q 'Saved'        # text only, to stdout
./termshot --text screen.txt session.pty screen.png     # both
```

`--json` adds the colours, the attributes and the cursor. Each row is a line of runs of cells
that look alike, with the column each starts at (a wide character takes two):

```json
{"cols":100,"rows":30,"cursor":{"col":2,"row":5,"shape":"block"},"lines":[
[{"col":0,"text":"ok","fg":"#00cd00","bg":"#111823","bold":true},{"col":2,"text":" done","fg":"#dbe7f7","bg":"#111823"}],
...
]}
```

`cursor` is null when the log hides it; its `shape` is `block`, `underline` or `bar`. `bold`, `italic`, `underline`, `double_underline` and `strike`
appear only when set. Blank cells that end a row are left out unless their background or a line
shows. Colours are as drawn: reverse video and dim are already applied, and concealed text has
`fg` equal to `bg`.

| option | |
|---|---|
| `-f`, `--font FILE` | TrueType or OpenType (CFF or CFF2) font (default: built-in JetBrains Mono); `FILE#N` or `FILE#NAME` picks a face of a collection |
| `--fallback-font FILE` | TrueType or OpenType (CFF or CFF2) font for characters the first lacks, such as CJK; faces as for `--font` |
| `-p`, `--px N` | font pixel height, above 0 and below 256 (default 48) |
| `-s`, `--size CxR` | grid columns × rows, up to 500×200 (default 100x30) |
| `--lf-newline` | treat each bare LF as CR LF, for logs not captured through a PTY; a final bare LF ends the last line instead of scrolling |
| `--cursor COL,ROW` or `none` | draw the cursor there, counting from 0 as tmux's `#{cursor_x},#{cursor_y}` do, or not at all (default: where the log leaves it, unless it hides it) |
| `--cursor-shape block`, `underline` or `bar` | draw the cursor as that shape (default: the one the log sets with DECSCUSR, or a block) |
| `--text FILE` | write the screen as text, a line per row with trailing spaces trimmed; the PNG is then optional |
| `--json FILE` | write the screen as JSON: the cursor and its shape, and per row the runs of cells alike in colour and attributes; the PNG is then optional |
| `-v`, `--verbose` | print the cell and image size to stderr |
| `-h`, `--help`, `-V`, `--version` | |

It prints nothing on success, except a hint on stderr when the log has line feeds but no CR,
which means it was probably not captured through a PTY and needs `--lf-newline`, and a warning
when a font maps a character to an empty glyph, so it was drawn as a box. The warning names the
first such cell, the font, and what to pass instead. Exit status is 0 when done; 1 when a file can't be read or written,
or the font is unusable; and 2 for bad arguments, including an image over 2^27 pixels. termshot
won't write a PNG to a terminal, and only one output can be `-`. A failed run removes the output
files it created.

The original form, `termshot <log> <out.png> <font.ttf> [px] [cols] [rows]`, still works.

`M` is snapped to a whole number of pixels so box-drawing joints meet. All box drawing and block elements (U+2500–U+259F: light, heavy, double and dashed lines, corners, tees, arcs, diagonals, eighths, shades and quadrants) are painted as geometry inside their cell, so lines join with any neighbour at any size; `tests/boxes.c` checks every one against its Unicode name. Other characters come from the font, then from `--fallback-font`, which is sized to the same height and centered in the cell; a character neither has is drawn as an outlined box, except for spaces, the line and paragraph separators, and the blank Braille pattern U+2800. Wide characters (CJK, fullwidth forms, emoji, by Unicode 17 widths) take two cells and are centered over both (on a one-column screen, where no row can hold two, they take the one cell); a combining mark merges into the character before it when Unicode has the precomposed form (e + U+0301 is é) and is otherwise dropped. An SGR reset uses foreground `#dbe7f7` on background `#111823`.

The font may have TrueType (`glyf`), CFF or CFF2 outlines, so `.ttf`, `.otf` and collections such as Noto Sans CJK's `.ttc` all work. A variable font with CFF2 outlines (such as `NotoSansCJKtc-VF.otf`) is drawn at its default instance, which for Noto Sans CJK is the Thin weight; choosing another instance is not supported yet. Color emoji fonts are bitmaps, not outlines, so emoji need a monochrome outline font such as Noto Emoji. A glyph with no outline counts as missing, so the emoji of a color font that has `glyf` (Apple Color Emoji) go on to `--fallback-font` or are drawn as boxes rather than left blank, with a warning. stb_truetype trusts the file it reads, so termshot first checks every structure stb will use (`src/font.rs`), and runs CFF and CFF2 charstrings itself (`src/cff.rs`), with a limit on the work a glyph may take; stb only rasterizes the outline. A damaged or hostile font is refused with a reason, and the run exits 1.

A font collection (`.ttc`) holds several faces, often one per script or region, and termshot
draws with one. Pick it after a `#`: `FILE#3` by number, counting from 0 as
`fc-list : file index family` does, or `"FILE#Family Name"` by its full or family name, ignoring
case. Without a `#`, the first face is used and a hint on stderr lists the others; `FILE#0`
uses the first without the hint. A face that doesn't exist, or a name that two faces share, is
refused with the list (exit 1), and `-v` prints the face used. If the file's own name has a
`#` in it, it is read as that file.

## Images in PTY logs

Kitty graphics sent inline (`t=d`) render above the text: RGB (`f=24`),
RGBA (`f=32`, the default), and PNG (`f=100`), including `m=1`/`m=0` chunks
and zlib-compressed payloads (`o=z`; a compressed PNG gives its size in `S`).
`a=T` transmits and places an image; `a=t` only stores it, under an id `i` or
a number `I`, and each `a=p` places a stored one again, sharing its pixels.
Images start at the cursor, use their native pixel size or fit a `c`/`r` cell
rectangle while preserving aspect ratio, and blend alpha over the existing
screen. Scaling uses deterministic nearest-neighbor sampling. Cell dimensions
come from the selected font and `--px`. `C=1` keeps the cursor in place;
otherwise it advances by the placement's columns and rows, clamped to the
screen/scroll area's bottom and right edges.

A placement id `p` names one placement of an image: putting the same `i,p`
again moves it. Deletes follow kitty's selectors: all (`d=a`, the default), by
id (`i`, with `p`), by number (`n`), by id range (`r`), at the cursor (`c`), at
a cell (`p`, `q` with a z-index), in a column (`x`), a row (`y`), or by z-index
(`z`). Lowercase keeps the image data for another `a=p`; uppercase also frees
the images it leaves without a placement. Retransmitting an id replaces its
image and removes its placements. Nonnegative `z` orders overlays, then the
order images and placements were made. Only images wholly inside a scrolling
region move and clip at its edges; images crossing a margin stay stationary.
Full-screen erase and reset remove every placement and free every stored image,
as kitty does. Explicit image ids and numbers must be nonzero; omitting `i` is
valid. Main and alternate screens keep separate images. See
[the regression evidence](docs/kitty-graphics.md).

This is a subset, not full kitty emulation: Sixel, file/shared-memory transfer,
source cropping, pixel offsets, negative z-index, animation, relative placements and Unicode
placeholders are not supported. Unsupported or malformed commands are ignored
without printing their payload. PNG images may be compressed internally as usual.
The log must contain the original escape sequences and image bytes; a plain
`tmux capture-pane` text capture cannot recover them. This does not make every
image-using TUI capture compatible automatically. Sixel remains tracked in
[#41](https://github.com/momiji-rs/termshot/issues/41); advanced kitty work is
tracked in [#44](https://github.com/momiji-rs/termshot/issues/44).

Limits per screen are 1,024 placements, and 4,096 stored images holding 16 MiB
of RGBA pixels. Past the image limits, an upload first frees every image
without a placement, then the least recently placed ones, as kitty's storage
quota does; a placement past its limit is discarded. Each upload is limited to 16 MiB of decoded payload and 8,192 pixels per source
axis (at most 4,194,304 source pixels). A display rectangle is limited to
16,777,216 pixels per axis. The PNG decoder has a separate 64 MiB allocation
budget, including inflation. A compressed (`o=z`) RGB or RGBA payload may be
at most 1,024 bytes over its decoded size, as in kitty; a compressed PNG at
most 16 MiB. Over-limit commands are discarded.

## Samples

`examples/reply-sent.pty` and `examples/draft-ready.pty` are captures from the [crisp-tui](https://github.com/solcreek/crisp-tui) demo inbox. The customers and messages are fake. At pixel height 48 the image is 2200×1440.

## Measure and test

```sh
TERMSHOT_PROFILE=1 ./termshot examples/reply-sent.pty /tmp/reply.png \
  third_party/jetbrains-mono/JetBrainsMono-Regular.ttf
python3 scripts/bench.py --binary current=./termshot --runs 40 \
  --output /tmp/termshot-bench.json
SANITIZE=1 ./tests/run.sh
```

Profiling writes two `termshot-profile` JSON records to stderr, covering input,
parsing, font allocation/reading/validation/padding (for the built-in or given
font and the fallback alike), glyph cache and fallback counters, drawing, PNG filtering, compression
allocation/matching/emission/checksum, PNG packaging, and writing. The benchmark
interleaves ordinary and profiled CLI runs in the same rounds, plus optional
peak RSS and (on Linux) cold-cache runs; it records raw samples, means, medians,
p95, child CPU time, paired comparisons, profiling overhead, output size and
hash, and host, toolchain, source and font details. Each font-path case checks
the profile counters that show it took its path (`scripts/bench-report.py`
summarizes a result file). Python 3 is needed only for optional
development scripts (benchmarks and CRC table generation); the tests need only
a C compiler and rustc.

See [performance measurements](docs/performance.md) for the before/after results,
baseline reproduction, timing boundaries, and remaining bottlenecks. Extended
pixel tests retain main's reference images and portable floating-point settings.

See [CHANGELOG.md](CHANGELOG.md) for notable changes.

## Vendored files

The program is MIT. Three things next to it keep their own terms:

- `third_party/stb/` is [stb](https://github.com/nothings/stb) `stb_truetype.h` 1.26 and `stb_image_write.h` 1.16, public domain. `stb_image.h` 2.30 decodes inline PNG images and test output; production enables only in-memory PNG decoding with bounded allocations. Local writer hooks and fixes are documented in [CHANGES.md](third_party/stb/CHANGES.md).
- `third_party/jetbrains-mono/` is JetBrains Mono Regular, [SIL Open Font License 1.1](third_party/jetbrains-mono/OFL.txt).
- `third_party/noto-sans-cjk/` is a subset of Noto Sans CJK TC, used only by the tests, [SIL Open Font License 1.1](third_party/noto-sans-cjk/OFL.txt).
