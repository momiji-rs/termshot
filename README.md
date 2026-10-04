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
cells. A combining mark composes with the character before it when Unicode has a precomposed
form (e + U+0301 is é); otherwise the cell keeps it, up to four marks, and draws it over the
character, so Thai, Hebrew points and stacked Latin accents show
([#14](https://github.com/momiji-rs/termshot/issues/14)). There is no shaping: a mark sits where
its font draws it, stacked marks may overlap, and Indic scripts are only approximate. Emoji
sequences (ZWJ, skin tones, VS16) are not joined into one picture.

## Speed

How long a run takes depends on the log, the image size, the fonts, the disk
cache and the machine, so termshot has no single latency figure. The current
figures below come from the **[versioned benchmark report](docs/performance.md)**,
whose JSON files keep every raw sample of each round with the binary hashes,
toolchains, fonts and inputs. Older figures at the end of this section are
history: other revisions, and one with no retained samples.

Current figures: measured 2026-10-03 with `scripts/bench.py` in the
[painting round](docs/performance.md#painting-and-geometry-2026-10-03-721d3fe-22)
(#22), on termshot built from `721d3fe` with `build.sh`'s flags. That is
main after the PNG compression (#20), parser (#21) and painting (#22) work;
main `a8a95e0` has the same build inputs and builds the same macOS binary
(sha256 `345dc4507209…`). Each figure is the median of 40 **whole CLI
runs**, wall time from spawn to exit: start-up, reading the log and the
fonts, parsing, drawing, PNG encoding and closing the file, with a warm page
cache and no `fsync`. They are not `TERMSHOT_PROFILE`'s internal stage
timers, which leave out process start-up and exit. Two batches with
different run orders are shown as A / B.

| workload | grid / px | image | fonts | Apple M2 Max, macOS 26.6.2 (ms) | Ryzen 7 8745HS, Arch Linux (ms) |
| --- | --- | --- | --- | ---: | ---: |
| `examples/reply-sent.pty` (`font-builtin`) | 100×30 / 48 | 2200×1440 | built-in JetBrains Mono | 8.90 / 8.92 | 7.49 / 7.56 |
| same log (`reply-24px`) | 100×30 / 24 | 1100×720 | JetBrains Mono file | 5.82 / 5.79 | 5.17 / 5.15 |
| same log (`reply-128px`) | 100×30 / 128 | 5800×3840 | JetBrains Mono file | 28.82 / 29.17 | 24.30 / 24.34 |
| `large` (generated) | 240×80 / 48 | 5280×3840 | JetBrains Mono file | 38.14 / 38.25 | 34.26 / 34.94 |
| `ansi-replay` (`reply-sent.pty` × 250, 4.7 MB) | 100×30 / 48 | 2200×1440 | JetBrains Mono file | 18.47 / 18.54 | 16.14 / 16.03 |
| `tests/perf/cjk-dense.pty` (`cjk-full`) | 100×30 / 24 | 1100×720 | built-in + `--fallback-font NotoSansCJK-Regular.ttc#3` (19 MB) | 12.38 / 12.62 ¹ | 13.55 / 13.57 |

¹ The painting round's macOS batches had no full Noto CJK collection; this
is the [parser round](docs/performance.md#ansi-replay-parsing-2026-10-03-10f1ea1-21)'s
(#21, built from `10f1ea1`, the same Mac and harness), from before #22,
whose changes are for rounded corners, diagonals, images and rasters over
16 MiB, none of which this case has.

That round covers 38 workloads on macOS and 41 on Linux; their batch-A
medians range from 5.02 ms (`cjk-none`, macOS) and 4.07 ms (`blank`,
Linux) to 143.91 and 136.36 ms for `large-color` (240×80 on 216 background
colours). The report also gives p95, child CPU time, peak RSS and a
per-stage breakdown. These are two machines with warm caches; a different
log, font, disk or a busy machine can take longer. The release archives use
the same compiler flags but link musl statically on Linux; they were not
measured.

Without a PNG, `--text` and `--json` read no font unless the log has an
image that needs cell metrics. The 4.7 MB `ansi-replay` log as text
(`--size 100x30 --text`) takes 13.61 / 13.53 ms on the M2 Max, CLI wall
medians of 40 runs, built from `ad35b1e`; main `a8a95e0` took 18.82 / 18.77
([text-only pre-scan](docs/performance.md#text-only-runs-one-pre-scan-instead-of-two-2026-10-03-macos-only),
macOS only).

History. The first 2026-10-03 baseline, built from `22b77e8` before #20,
#21 and #22 (the report's
[font-path baseline](docs/performance.md#current-baseline-font-paths-and-linux-2026-10-03-d83c8fd)),
measured the same six rows at 9.27, 5.83, 31.56, 41.91, 20.80 and 12.67 ms
on the M2 Max and 9.64, 5.63, 38.06, 49.24, 19.69 and 14.85 ms on the Ryzen
(batch-A CLI wall medians). Its Linux cold-cache run, the
only one, found that dropping the page cache adds 6.9-8.2 ms to three
small-font cases. Earlier rounds (2026-10-01, Apple M3, revisions up to
`c44d83c`) used a different machine, revision and harness, so their numbers
must not be subtracted from these. The "~20 ms for 2200×1440" quoted in
older descriptions was a 21 ms mean of 40 hyperfine runs on that M3 at
`fb714a5`, whose samples were not kept; the report records
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

A test for a known bug describes the correct behaviour and is marked `#[ignore = "#N: ..."]` with its issue. None are open now. Run them with `./target/test/unit --ignored`. Three other tests are ignored. `poc_workloads` writes inputs for `bench/c-vs-rust/` and checks nothing. `any_cff2_font_matches_harfbuzz` checks a CFF2 font of your own, at any instance, against the outlines `tools/cff2-outlines.sh` recorded from HarfBuzz, and `any_cff2_font_s_metrics_match_harfbuzz` its advances and extents against what `tools/cff2-metrics.py` recorded; their doc comments give the commands.

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

An [asciinema](https://asciinema.org) recording works as the log too, in either format,
asciicast [v2](https://docs.asciinema.org/manual/asciicast/v2/) or
[v3](https://docs.asciinema.org/manual/asciicast/v3/). The grid takes the recording's size,
so `--size` isn't needed:

```sh
asciinema rec demo.cast
./termshot demo.cast demo.png
```

A log is read as a cast when its first line, from the first byte, is a JSON object with a
`"version"` member at its top level; the rest of the line need not be valid, so a damaged
header is refused rather than drawn as text. Terminal output rarely starts that way; when it does, `--raw` reads the
log as raw output, and a log taken for a cast that fails to read says so. `--cast` reads the
log as a cast whatever it starts with, which matters only for the error you get. termshot replays the data of the
output (`"o"`) events, in the order the file has them, and ignores input (`"i"`), markers
(`"m"`), exit (`"x"`) and other events. The size is the header's (`width` and `height` in v2,
`term.cols` and `term.rows` in v3), or that of the last resize (`"r"`, `"COLSxROWS"`) event, as
the final screen is the one drawn. The grid has that one size from the start rather than
changing mid-replay: full-screen programs redraw when resized, so their last frame is right,
but text printed before a resize may wrap where it did not in the terminal. `--size`, or the
original form's cols and rows, override it, each dimension on its own. A recording larger
than 500×200 needs `--size` (exit 2). Timing is not used, so the replay doesn't depend on it.

The JSON is read strictly: UTF-8, with every escape including `\uXXXX` surrogate pairs, no
duplicate keys, nesting at most 16 deep, and every line after the header an event (in v3, or
a `#` comment), so a blank line is refused too. A lone surrogate, invalid UTF-8, a truncated line,
a wrong type, a size that is not a whole number, or a negative or infinite time is refused
with its line, and for bad JSON or UTF-8 its column (exit 1), as for an unusable font. For a raw log, the size isn't
guessed from where the cursor went; give it with `--size`.

To check what a screen shows rather than how it looks (in a test, or as an agent), write it as
text. It is laid out as `tmux capture-pane -p` prints it: a line per row, trailing spaces
trimmed, each character followed by its combining marks. For logs without graphics, omitting
the PNG skips font loading, drawing and PNG encoding; how much time that saves depends on the log, and the benchmark report does not
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
shows. A run's `text` has each character followed by its combining marks, as in `--text`,
so a mark takes no column of its own. Colours are as drawn: reverse video and dim are already applied, and concealed text has
`fg` equal to `bg`.

| option | |
|---|---|
| `-f`, `--font FILE` | TrueType or OpenType (CFF or CFF2) font (default: built-in JetBrains Mono); `FILE#N` or `FILE#NAME` picks a face of a collection, `FILE#wght=700` an instance of a CFF2 variable font |
| `--fallback-font FILE` | TrueType or OpenType (CFF or CFF2) font for characters the first lacks, such as CJK; faces as for `--font` |
| `-p`, `--px N` | font pixel height, above 0 and below 256 (default 48) |
| `-s`, `--size CxR` | grid columns × rows, up to 500×200 (default: a cast's size, else 100x30) |
| `--cast` | read the log as an asciinema `.cast` (v2 or v3), even if its first line isn't a header |
| `--raw` | read the log as raw PTY output, even if its first line looks like a cast's header |
| `--lf-newline` | treat each bare LF as CR LF, for logs not captured through a PTY; a final bare LF ends the last line instead of scrolling |
| `--cursor COL,ROW` or `none` | draw the cursor there, counting from 0 as tmux's `#{cursor_x},#{cursor_y}` do, or not at all (default: where the log leaves it, unless it hides it) |
| `--cursor-shape block`, `underline` or `bar` | draw the cursor as that shape (default: the one the log sets with DECSCUSR, or a block) |
| `--text FILE` | write the screen as text, a line per row with trailing spaces trimmed; the PNG is then optional |
| `--json FILE` | write the screen as JSON: the cursor and its shape, and per row the runs of cells alike in colour and attributes; the PNG is then optional |
| `-v`, `--verbose` | print the cell and image size, the face of each collection and the instance of each variable font to stderr |
| `-h`, `--help`, `-V`, `--version` | |

It prints nothing on success, except a hint on stderr when the log (for a cast, its output) has line feeds but no CR,
which means it was probably not captured through a PTY and needs `--lf-newline`, and a warning
when a font maps a character to an empty glyph, so it was drawn as a box. The warning names the
first such cell, the font, and what to pass instead. Exit status is 0 when done; 1 when a file can't be read or written,
a `.cast` is malformed, or the font is unusable; and 2 for bad arguments, including an image
over 2^27 pixels and a cast larger than 500×200 without `--size`. termshot
won't write a PNG to a terminal, and only one output can be `-`. A failed run removes the output
files it created.

The original form, `termshot <log> <out.png> <font.ttf> [px] [cols] [rows]`, still works.

`M` is snapped to a whole number of pixels so box-drawing joints meet. All box drawing and block elements (U+2500–U+259F: light, heavy, double and dashed lines, corners, tees, arcs, diagonals, eighths, shades and quadrants) are painted as geometry inside their cell, so lines join with any neighbour at any size; `tests/boxes.c` checks every one against its Unicode name. Other characters come from the font, then from `--fallback-font`, which is sized to the same height and centered in the cell; a character neither has is drawn as an outlined box, except for spaces, the line and paragraph separators, and the blank Braille pattern U+2800. Wide characters (CJK, fullwidth forms, emoji, by Unicode 17 widths) take two cells and are centered over both (on a one-column screen, where no row can hold two, they take the one cell); a combining mark merges into the character before it when Unicode has the precomposed form (e + U+0301 is é); otherwise the cell keeps up to four marks, and each is drawn over the character in its colours, from the font or else `--fallback-font` (a mark neither has is left out, not boxed). Without shaping (no GPOS anchors), a mark its font draws left of its origin, as most fonts do, is drawn from where the character ends; one drawn right of its origin, as in right-to-left fonts, is centered over the character. Joiners, variation selectors, Hangul fillers and the other default-ignorable characters are kept in `--text` and `--json` but draw nothing. An SGR reset uses foreground `#dbe7f7` on background `#111823`.

The font may have TrueType (`glyf`), CFF or CFF2 outlines, so `.ttf`, `.otf` and collections such as Noto Sans CJK's `.ttc` all work. A variable font with CFF2 outlines (such as `NotoSansCJKtc-VF.otf`) is drawn at its default instance, which for Noto Sans CJK is the Thin weight, unless you choose another after a `#` (see below). Color emoji fonts are bitmaps, not outlines, so emoji need a monochrome outline font such as Noto Emoji. A glyph with no outline counts as missing, so the emoji of a color font that has `glyf` (Apple Color Emoji) go on to `--fallback-font` or are drawn as boxes rather than left blank, with a warning. stb_truetype trusts the file it reads, so termshot first checks every structure stb will use (`src/font.rs`), and runs CFF and CFF2 charstrings itself (`src/cff.rs`), with a limit on the work a glyph may take; stb only rasterizes the outline. A damaged or hostile font is refused with a reason, and the run exits 1.

A font collection (`.ttc`) holds several faces, often one per script or region, and termshot
draws with one. Pick it after a `#`: `FILE#3` by number, counting from 0 as
`fc-list : file index family` does, or `"FILE#Family Name"` by its full or family name, ignoring
case. Without a `#`, the first face is used and a hint on stderr lists the others; `FILE#0`
uses the first without the hint. A face that doesn't exist, or a name that two faces share, is
refused with the list (exit 1), and `-v` prints the face used. If the file's own name has a
`#` in it, it is read as that file.

A variable font with CFF2 outlines can be drawn at another instance: give its axis settings in a
last `#` part, `TAG=VALUE` separated by commas, as in `NotoSansCJKtc-VF.otf#wght=700` or
`FILE.ttc#1#wght=700,wdth=90`. Values are in the axis's own units (`hb-info --list-variations
FILE` lists them), clamped to its range, and the outlines match HarfBuzz's (`hb-view
--variations`). An axis left out stays at its default, and `-v` prints the instance. Bad syntax
exits 2. An axis the font doesn't have exits 1 and lists those it has; a TrueType or CFF
font, whose outlines don't vary here, exits 1 with that reason. The metrics vary with the
instance too, as in HarfBuzz: each glyph's advance by `HVAR`, so a heavy weight gets wider
cells, and the ascender, descender and line gap by `MVAR`.

## Images in PTY logs

Kitty graphics sent inline (`t=d`) render above the text: RGB (`f=24`),
RGBA (`f=32`, the default), and PNG (`f=100`), including `m=1`/`m=0` chunks
and zlib-compressed payloads (`o=z`; a compressed PNG gives its size in `S`).
`a=T` transmits and places an image; `a=t` only stores it, under an id `i` or
a number `I`, and each `a=p` places a stored one again, sharing its pixels.
Images start at the cursor, or `X`/`Y` pixels into its cell (at most a pixel
short of the cell's edge), use their native pixel size or fit a `c`/`r` cell
rectangle while preserving aspect ratio, and blend alpha over the existing
screen. `x`, `y`, `w`, `h` choose a source rectangle in pixels, and the part of
it inside the image is shown; that crop's aspect ratio is the one kept, and an
empty crop draws nothing. Scaling uses deterministic nearest-neighbor sampling. Cell dimensions
come from the selected font and `--px`. `C=1` keeps the cursor in place;
otherwise it moves as kitty moves it: right by the placement's columns and
down by its rows less one, to the next row's start if that reaches the right
edge, scrolling the region up if it passes the bottom margin.

A placement id `p` names one placement of an image: putting the same `i,p`
again moves it. Deletes follow kitty's selectors: all (`d=a`, the default), by
id (`i`, with `p`), by number (`n`), by id range (`r`), at the cursor (`c`), at
a cell (`p`, `q` with a z-index), in a column (`x`), a row (`y`), or by z-index
(`z`). Lowercase keeps the image data for another `a=p`; uppercase also frees
the images it leaves without a placement. Retransmitting an id replaces its
image and removes its placements. Images draw by `z`, then the order images
and placements were made: from 0 over the text, below 0 under the text but over
every cell background, and below -1,073,741,824 under the backgrounds that are
not the default colour, so they show only through default ones. Reverse-video
cells and the block cursor are opaque there, as in kitty. The underline and
bar cursors are drawn with the text, as kitty draws them: over the images under
it, under those of `z` 0 and up. Only images wholly inside a scrolling
region move and clip at its edges; images crossing a margin stay stationary.
Full-screen erase and reset remove every placement but the virtual ones and
free every stored image left without one, as kitty does. A relative placement (`P` and `Q` name a parent image and
placement) starts `H`, `V` cells from its parent's top left cell, follows the
parent when it moves or scrolls, and goes when the parent goes; it never moves
the cursor. A missing parent, a cycle, or a chain of more than 8 links refuses
the put. Explicit image ids and numbers must be nonzero; omitting `i` is
valid. Main and alternate screens keep separate images. See
[the regression evidence](docs/kitty-graphics.md).

Unicode placeholders work as in kitty (`kitten icat --unicode-placeholder`,
tmux, editors): `U=1` makes a virtual placement of `c` x `r` cells, which
draws nothing itself, and each U+10EEEE cell shows the part of the image
under it. The cell's foreground colour is the image id (`38;5;n` is `n`,
`38;2;r;g;b` is 0xRRGGBB, a third diacritic the high byte), its underline
colour (`58`) the placement id, and its first two diacritics, from kitty's
`rowcolumn-diacritics.txt`, the row and column; missing ones are inherited
from the cell to the left. The image is fitted into the placement's cells
keeping its aspect ratio, centered, at z-index -1, over the cell's
background. The placeholders are ordinary text, so the image moves, scrolls
and is erased with them. A placeholder whose image or placement does not
exist draws nothing. `d=i`, `n` and `r` delete virtual placements; the
other selectors, full-screen erase and reset leave them. A relative
placement may name a virtual parent: it starts from the top row and the
leftmost column the image shows in. Placeholder cells are drawn blank;
`--text` and `--json` keep U+10EEEE and its diacritics.

This is a subset, not full kitty emulation: file/shared-memory transfer
and animation are not supported. Unsupported or malformed commands are ignored
without printing their payload. PNG images may be compressed internally as usual.
The log must contain the original escape sequences and image bytes; a plain
`tmux capture-pane` text capture cannot recover them. This does not make every
image-using TUI capture compatible automatically. Advanced kitty work is
tracked in [#44](https://github.com/momiji-rs/termshot/issues/44).

Sixel images (`ESC P P1;P2;P3 q … ESC \`) are drawn too, following xterm's
decoder (its `graphics_sixel.c`, patch 412) and the VT340 it emulates:

- Colour introducers `#Pc` select a register and `#Pc;Pu;Px;Py;Pz` define it,
  in HLS (`Pu` 1, DEC hues: 0 is blue, 120 red, 240 green) or RGB (`Pu` 2), in
  percent, rounded half up to 8 bits. Each image has its own 1,024 registers,
  starting from the VT340's 16 colours (the rest black), as xterm's default
  private colour registers; register numbers wrap at 1,024. Drawing starts in
  register 3, as in xterm. Pixels keep their register, so redefining one
  recolours what it drew: the palette at the end of the image applies.
  A definition out of range or with the wrong number of parameters is ignored.
- Repeats (`!Pn`), graphics carriage return (`$`) and next line (`-`), and raster
  attributes (`"Pan;Pad;Ph;Pv`). Pixels are square device pixels at their
  native size, as in xterm, which also ignores `P1` and `Pan;Pad`; cell
  dimensions come from the font and `--px`, as for kitty's native sizing.
  The image is as large as `Ph`x`Pv` or its pixels reach, whichever is larger;
  an extent given as 0 or left empty counts as 1, as in xterm.
- `P2` 1 leaves pixels no sixel set transparent. `P2` 0 or 2 paints the area
  the raster attributes declared before the first sixel with register 0, as
  xterm does; pixels past it stay transparent.
- With Sixel scrolling on (the default), the image starts at the cursor, and
  the cursor ends on the last text row the image covers, in the same column,
  so the program's next newline goes below it. An image that would pass the
  bottom margin first scrolls the region up; its top is cut off if it is
  taller than the region. DECSDM (`CSI ? 80 h`) instead draws at the top left
  corner, clipped at the bottom, without scrolling or moving the cursor.
- The image joins the kitty image store as an unnamed image drawn above the
  text, so it shares the layering, scrolling, erase and storage limits above.
  Unlike a kitty image, its pixels belong to the cells, as in xterm: a
  character written there later clears them in the cells it takes, and
  erasing below or above the cursor (ED 0 or 1) clears them in the rows
  below or above the cursor's, though not in its own row.
  Only `ESC \` commits an image; BEL, CAN, SUB, another escape or the end of
  the log discard it, and C1 controls (such as the 8-bit ST) are not
  recognised. A DCS that is not Sixel (`DECRQSS`, `XTGETTCAP`, ...) is
  skipped. An image wider or taller than 8,192 pixels or over 4,194,304
  pixels is refused whole, before its pixels are allocated, and a repeat
  costs nothing beyond those bounds. Since a few bytes can declare a large
  image or draw over the same pixels again and again, a log may write at most
  16,777,216 Sixel pixels plus 256 for each byte of Sixel data, counting every
  pixel a sixel sets, each time it sets it, and every pixel of each finished
  image; an image past that budget is refused.

EL, ECH, ICH and DCH leave the pixels, as in xterm; IL and DL move images as
they move kitty's, where xterm leaves them. An image with no pixels moves
nothing. Non-square pixels from `P1` or `Pan;Pad`
(xterm ignores them too), DECSET 8452 (the cursor to the right of the image),
shared colour registers (`CSI ? 1070 l`) and ReGIS are not supported.

Limits per screen are 1,024 placements, and 4,096 stored images holding 16 MiB
of RGBA pixels. Past the image limits, an upload first frees every image
without a placement, then the least recently placed ones, as kitty's storage
quota does; a placement past its limit is discarded. Each upload is limited to 16 MiB of decoded payload and 8,192 pixels per source
axis (at most 4,194,304 source pixels). A display rectangle is limited to
16,777,216 pixels per axis. The PNG decoder has a separate 64 MiB allocation
budget, including inflation. As in kitty, an RGB or RGBA payload may be at
most 10 bytes over its decoded size, which are ignored, or 1,024 bytes if
compressed (`o=z`); a PNG payload at most 16 MiB. Over-limit commands are
discarded.

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
