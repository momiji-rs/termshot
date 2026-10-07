# Changelog

Notable changes to termshot are recorded here for users and contributors.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.3.0] - 2026-10-06

termshot 0.3.0 can be used as a Rust library: `termshot::parse` replays a
log into a grid of cells and `termshot::render` draws it as a PNG in memory,
with errors as values, built and linked with plain rustc and no Cargo. The
CLI now takes a colour palette (`--palette`, `--fg`, `--bg`) and a margin
(`--padding`), and draws kitty animations as a still. The CLI is a thin
layer over the library; given the same log and options, it writes what
0.2.0 wrote, byte for byte, except as listed below.

Some 0.2.0 renders and runs change. Upgrading, expect:

- **kitty animation commands take effect.** 0.2.0 ignored frame uploads
  (`a=f`), composition (`a=c`), animation control (`a=a`) and frame
  deletion (`d=f`, `d=F`), and drew the image as transmitted. Now an image
  shows the frame an explicit `a=a` with `c` last made current, else its
  root frame, as frames, edits and compositions built it. A log that never
  makes another frame current, as `kitten icat` with a GIF, still shows the
  root frame, but edits of the root, `a=c` onto it, a current frame chosen
  with `a=a` and frame deletion now change the picture. Frames count against the
  16 MiB image quota, so a log with many frames can evict images 0.2.0 kept.
- **Out of memory exits 2 instead of aborting** when the screens of a large
  grid or a font file's bytes can't be allocated, with "out of memory ..."
  on stderr, and the outputs the run created are removed.
- **"font metrics unusable" is printed once**, not twice.
- **A font file named `-` is protected.** 0.2.0 overwrote it when an output
  named the same file (`--font - log ./-`); now that is refused with exit 2,
  as for any other input.
- **`TERMSHOT_PROFILE`'s `face_ms` moved** from the CLI's record (the one
  with `total_ms`) to the render's (the one with `output_write_ms`). A run
  without a PNG still reports it in the CLI's record, as 0. The keys are
  otherwise the same.

### Added

- **termshot as a Rust library** (#85). `build.sh` also builds
  `libtermshot.rlib` from `src/lib.rs`, with no dependencies and the C it
  needs inside; link it with `rustc --edition 2021 app.rs --extern
  termshot=libtermshot.rlib`, using the rustc that built it. The release
  archives hold the CLI only; the library is built from source.
  - `termshot::parse` replays a log into a `Grid` (`ParseOptions`: the LF
    mode and the palette), whose cells, cursor and shape can be read, and
    whose `to_text` and `to_json` are byte for byte what `--text` and
    `--json` write. `parse_with_cell_size` parses with a font's cell
    (`Font::cell_size`), for logs whose images move the cursor by pixels
    (`needs_cell_size`); `decode_cast` and `is_cast` read asciicasts.
  - `termshot::render` draws a grid and returns the PNG's bytes, byte for
    byte what the CLI writes with the same options; `render_rgba` returns
    the pixels. `RenderOptions` takes the pixel size, a `Font` (the built-in
    one, a file with the CLI's `#N`, `#NAME` and `#wght=...`, or bytes), a
    fallback font, padding, and the cursor and its shape. A character drawn
    as a box for an empty glyph comes back as data
    (`Rendered::empty_glyph`).
  - Errors are values (`termshot::Error`, with the CLI's messages): the
    library prints nothing of its own and never exits. Where an allocation
    grows with the input it returns `Error::OutOfMemory` rather than abort
    (the crate docs list what is still bounded otherwise), and a panic in a
    render is caught as `Error::Internal`. A grid with no cells, a side over 65,535, or more
    than 4,194,304 (2048 x 2048) cells is `Error::GridSize`.
  - Grids, fonts and renders can be used from many threads at once.
- **kitty animation, drawn as a still** (#44): frame uploads (`a=f`: new
  frames over a base frame `c` or a background colour `Y`, edits of frame
  `r`, blended or overwriting with `X`, in every format and chunked),
  composition between frames (`a=c`), animation control (`a=a`) and frame
  deletion (`d=f`, `d=F`), following kitty's `graphics.c`. A log has no
  timeline, so each image shows the frame an explicit `a=a` with `c` last
  made current, else its root frame; gaps, the animation state and loop
  counts are parsed and change nothing. Every placement, relative placement
  and Unicode placeholder shows the current frame. Frames count against the
  16 MiB image quota; an image may have 1,024 frames and a screen 16,384
  past the roots, and a screen may compose 2^28 pixels in all. See
  [docs/kitty-graphics.md](docs/kitty-graphics.md#animation).
- `--palette FILE` sets the default foreground and background and the 16
  named colours, in kitty's colour keys (`foreground`, `background`,
  `color0` to `color15`, each with `#rrggbb`), so those lines of a kitty
  theme work as they are; `--fg #RRGGBB` and `--bg #RRGGBB` set the default
  colours on their own, over the file's. Colours 16 to 255 and 24-bit
  colours keep their values. The palette applies as the log is replayed, so
  `--json` reports the colours it resolves to, and the cursor, dim, conceal
  and kitty's images below the cell backgrounds follow its default colours.
  A malformed file (an unknown or repeated key, a bad colour, text that
  isn't UTF-8, or over 64 KiB) exits 2 with its line; one that can't be
  read exits 1 (#87).
- `--padding N` or `--padding X,Y` draws a margin of 0 to 1024 pixels
  around the cells in the default background. Everything drawn moves by it,
  cut at the cells' edges as before; the cell size, `--text` and `--json`
  don't change, and the margin counts towards the 2^27-pixel limit. `-v`
  prints the padded size (#87).

### Changed

- The CLI (`src/main.rs`) is a crate of its own over the library's public
  API, linking `libtermshot.rlib` as an embedder does, with the same output
  bytes, messages and exit codes; no existing golden changed. Moving the
  parser and the render into the library (#92, #93) was timed against the
  main each started from: with fat LTO, as the release archives are built,
  no case was slower on macOS, and on Linux two large-image cases were
  1-2.5% slower, from code layout alone
  ([docs/performance.md](docs/performance.md#the-render-as-a-library-2026-10-06-35c62b9-85-part-2)).

### Fixed

- A font whose metrics can't make a cell printed "font metrics unusable"
  twice; it is said once.
- Running out of memory for the screens of a large grid, or for the font
  file's bytes, aborted; the CLI exits 2 with "out of memory ..." and
  removes the outputs it created, as for the render's own allocations.
- A `--font` or `--fallback-font` file named `-` was not checked against
  the outputs, so `./-` as an output overwrote it; it is refused with
  exit 2, as for the log and the other inputs.

## [0.2.0] - 2026-10-06

termshot 0.2.0 draws images (kitty graphics and Sixel), reads asciinema
recordings, takes OpenType fonts with CFF and CFF2 outlines, draws the
cursor, italic and combining marks, and can write the screen as text or
JSON. Drawing and PNG compression moved from C to Rust, release binaries
are smaller, and most renders are faster.

Some 0.1.0 renders change. Upgrading, expect:

- **The cursor is drawn.** 0.1.0 never drew it; 0.2.0 draws it where the
  log leaves it unless the log hides it (`ESC [ ? 25 l`). `--cursor none`
  renders without it, as 0.1.0 did.
- **Combining marks are drawn.** 0.1.0's notes say a mark with no
  precomposed form is "otherwise dropped"; that no longer holds. It is
  kept, drawn over its character, and written by `--text` and `--json`.
- **Italic (SGR 3) is drawn**, slanted; 0.1.0 drew it upright.
- **Images and their cursor movement.** 0.1.0 ignored kitty graphics and
  Sixel. Now the images are drawn and move the cursor as kitty and xterm
  move it, so text written after an image can land elsewhere.
- **Glyph fixes that move pixels:** a character whose glyph has no outline
  falls back or is drawn as a box instead of blank; U+2800, U+2028, U+2029
  and the Hangul fillers no longer draw boxes; a glyph with a box but no
  points no longer draws stray pixels.
- **A log whose first line is a JSON object with a `"version"` member is
  read as an asciinema cast.** `--raw` reads it as raw output, as 0.1.0 did.
- **Exit codes:** running out of memory while compressing the PNG exits 2,
  not 1, and an output that names an input or another output is refused
  with 2 instead of overwriting it.

### Added

- **Kitty graphics** (#41, #44). Images sent inline (`t=d`) in RGB
  (`f=24`), RGBA (`f=32`) and PNG (`f=100`), in `m=1` chunks, in base64
  with or without `=` padding (each chunk decoded on its own, as in kitty,
  so `kitten icat` works), and zlib-compressed (`o=z`; a compressed PNG
  gives its size in `S`). They are blended over the screen at their native
  pixel size or fitted into `c` x `r` cells keeping the aspect ratio, with
  deterministic nearest-neighbour scaling.
  - `a=T` transmits and places; `a=t` stores an image under an id `i` or a
    number `I`, and `a=p` places a stored one again. A placement id `p`
    moves one placement. Retransmitting an id replaces the image and
    removes its placements. Explicit ids and numbers must be nonzero.
  - Deletes take kitty's `a`, `i`, `n`, `r`, `c`, `p`, `q`, `x`, `y` and
    `z` selectors; uppercase also frees the data of images left without a
    placement.
  - `x`, `y`, `w`, `h` crop the source, and the crop's aspect ratio is the
    one kept; `X`, `Y` offset the image inside its first cell. A negative
    `z` draws under the text, and below -1,073,741,824 under every cell
    background that is not the default one (reverse video and the block
    cursor count as non-default). Ties in `z` go by creation order.
  - Relative placements (`P`, `Q`, `H`, `V`) start from a parent
    placement, move and scroll with it and are deleted with it. A missing
    parent, a cycle or a chain of more than 8 links refuses the put.
  - Unicode placeholders (`U=1`, U+10EEEE cells): the cell's foreground
    colour names the image, its underline colour (SGR 58) the placement, and
    its diacritics the row and column, inherited from the cell to the left
    as in kitty. The image is fitted into the placement's cells and
    letterboxed, and scrolls, is erased and is overwritten with the
    placeholders. A virtual placement survives full-screen erase and reset;
    only `d=i`, `n` and `r` delete it. A relative placement may have a
    virtual parent. Placeholder cells are drawn blank;
    `--text` and `--json` keep their code points.
  - After a placement the cursor moves as kitty moves it: right by its
    columns and down by its rows less one, to the next row's start at the
    right edge, scrolling the region up past the bottom margin. `C=1`,
    relative and virtual placements leave it in place. An image is not cut
    at the screen's bottom when placed; scrolling brings the rest into view.
  - Images track scrolling, and the main and alternate screens keep their
    own. Only placements wholly inside a scrolling region move, clipped at
    its edges; images that cross a margin stay put, including under IL, DL
    and RI.
  - Limits, per screen: 1,024 placements, and 4,096 stored images holding
    16 MiB of RGBA pixels. Past them, an upload frees images without a
    placement, then the least recently placed, as kitty's quota does. An
    upload may hold 16 MiB of decoded payload, 8,192 pixels per axis and
    4,194,304 pixels; a display rectangle at most 16,777,216 pixels per
    axis; the PNG decoder has a 64 MiB allocation budget. As in kitty, an
    RGB or RGBA payload may run at most 10 bytes over its size, which are
    ignored, or 1,024 bytes if compressed, checked as each chunk arrives and
    with the dimensions checked before anything is inflated; a compressed
    payload must inflate to exactly its size. Over-limit and malformed
    commands are discarded without printing their payload.
  - Not supported: file and shared-memory transfers, and animation.
- **Sixel images** (`ESC P … q … ESC \`, #41), decoded as xterm (patch 412)
  decodes them: HLS and RGB colour registers, 1,024 per image starting from
  the VT340's 16 colours, repeats, `$` and `-`, raster attributes, and `P2`
  for an opaque or transparent background. Pixels are square and native
  size. The image starts at the cursor, scrolls the screen when it passes
  the bottom margin, and leaves the cursor on the last row it covers;
  DECSDM (`CSI ? 80 h`) draws it at the top left instead. Sixel images share
  the kitty image store, its layering and its limits. As in xterm, a
  character written over the image clears its pixels in the cells it takes,
  and ED 0 and 1 clear them in the rows below or above the cursor's; EL,
  ECH, ICH and DCH leave them. An image over 8,192 pixels on a side or
  4,194,304 in all is refused before its pixels are allocated, and a log may
  write at most 16,777,216 Sixel pixels plus 256 per byte of Sixel data,
  overdrawing included. Only `ESC \` commits an image. Not supported:
  non-square pixels (`P1`, `Pan;Pad`, which xterm ignores too), DECSET 8452,
  shared colour registers and ReGIS.
- **asciinema recordings** (`.cast`, asciicast v2 and v3) as the log (#1).
  Output events are replayed in order; input, marker, exit and other events
  are ignored, and so is timing. Without `--size`, the grid takes the
  recording's size: its last resize event's, or its header's. A log is read
  as a cast when its first line is a JSON object with a `"version"` member;
  `--cast` forces it, and `--raw` reads the log as raw output. The JSON is
  read strictly (UTF-8, every escape, no duplicate keys, at most 16 deep);
  a malformed cast is refused with its line (exit 1), and one larger than
  500x200 needs `--size` (exit 2).
- **Fonts with CFF outlines** (`.otf`, Noto Sans CJK) for `--font` and
  `--fallback-font` (#25), where 0.1.0 refused them. termshot reads the CFF
  table and runs its charstrings in Rust (`src/cff.rs`) with a limit on the
  work one glyph may take; stb_truetype only rasterizes the outline. A
  damaged table is refused with a reason (exit 1); a glyph that can't be
  drawn is drawn as the box for a missing glyph.
- **Variable fonts with CFF2 outlines** (#51, #77), such as the variable
  Noto Sans CJK and Source Han Sans. They are drawn at their default
  instance, or at the one chosen after a `#`: `NotoSansCJKtc-VF.otf#wght=700`,
  or `FILE.ttc#1#wght=700,wdth=90` with a face. Each setting is clamped to
  its axis, mapped through `avar`, and blended as HarfBuzz blends it;
  advances vary by `HVAR` (a heavy instance as the main font gets wider
  cells) and the ascender, descender and line gap by `MVAR`. Bad syntax
  exits 2. An axis the font lacks exits 1 and lists those it has, and so
  does a TrueType or CFF font, whose outlines don't vary here. A damaged
  CFF2, `HVAR` or `MVAR` table is refused with a reason (exit 1). `-v`
  prints the instance.
- **A face of a font collection** (`.ttc`) for `--font` and
  `--fallback-font` (#25): `FILE#N` by number from 0, or `FILE#NAME` by its
  full or family name, ignoring case. A face that doesn't exist, or a name
  two faces share, is refused with the list of faces (exit 1). Without a
  `#`, the first face is used, as before, with a hint on stderr listing the
  others; `FILE#0` skips the hint. A file whose own name has a `#` is still
  read as that file. `-v` prints the face used.
- **The cursor** (#33, #39), drawn where the log leaves it: a block in
  reverse video, over both cells of a wide character, on the last column
  with a wrap pending. The shape a program sets with DECSCUSR
  (`CSI Ps SP q`) is drawn steady: an underline (3, 4) or a bar (5, 6) in
  the default foreground, or a block (0 to 2). As in kitty, the underline
  and bar are over images under the text and under images of z-index 0 and
  up. The shape survives DECSC/DECRC, DECSTR and the alternate screen, as in
  tmux; RIS resets it, as in xterm.
  - `--cursor COL,ROW` draws it there, counting from 0 as tmux's
    `#{cursor_x},#{cursor_y}` do (COL may be the column count, tmux's pending
    wrap), and `--cursor none` leaves it out.
  - `--cursor-shape block|underline|bar` overrides the shape.
- **`--text FILE`** writes the screen as text, as `tmux capture-pane -p`
  prints it: a line per row, trailing spaces trimmed, a wide character
  once, each character followed by its combining marks (#32).
- **`--json FILE`** writes the screen with its colours and the cursor
  (#32): per row, runs of cells alike in colour (`#rrggbb`, as drawn) and
  attributes (`bold`, `italic`, `underline`, `double_underline`,
  `strike`), each with the column it starts at, and the cursor as
  `{"col","row","shape"}` or null.
  - With `--text` or `--json`, `<out.png>` is optional. Without a PNG
    nothing is drawn or encoded, and no font is read unless the log has a
    kitty placement or a Sixel image that can move the cursor by the font's
    cells. `tests/grids/` holds both outputs for every golden fixture.
- **Combining marks with no precomposed form** are kept and drawn (#14):
  Thai vowel and tone marks, Hebrew points, stacked or uncommon Latin
  accents, and, approximately, Indic vowel signs and viramas. A cell keeps
  up to four, each drawn over the character in its colours, from the font
  or else `--fallback-font`, where the font puts it (there is no shaping).
  Marks join the last printed character, wherever the cursor has gone since,
  as in xterm, and go with it when the line is edited or scrolled; REP
  repeats them. Emoji sequences (ZWJ, skin tones, VS16) are kept in the text
  but drawn one code point per cell. Joiners, variation selectors and the
  Hangul fillers draw nothing.
- **Italic** (SGR 3, cleared by 23), which vim comments, `bat` and `delta`
  use (#26): the glyph's outline slanted 12 degrees before it is
  rasterized, as smooth as upright text with any font. Box drawing, block
  elements and the box for a missing glyph stay upright. `--json` reports
  it as `"italic": true`.
- **`--lf-newline`** treats each bare LF as CR LF, as a terminal with
  `onlcr` does, for logs not captured through a PTY (#28). A bare LF that
  ends the input ends the last line instead of scrolling, so
  `tmux capture-pane -e -p | termshot --lf-newline --size <pane size>` keeps
  the top row; a PTY log renders the same with the flag as without.
- Hints and warnings on stderr, which leave the image written and the exit
  status 0:
  - a hint when a log has line feeds but no CR and `--lf-newline` is not
    given;
  - a warning when a character is drawn as a box because a font maps it to
    an empty glyph, as color bitmap fonts such as Apple Color Emoji do
    (#38), naming the first such cell, the font, and what to pass instead.
    A character no font has at all is drawn as a box silently, as before.

### Changed

- Renders show the cursor unless the log hides it with `ESC [ ? 25 l`
  (DECTCEM), as full-screen programs and progress bars often do.
  `--cursor none` renders without it, as 0.1.0 did.
- **Faster, with the same output** (#20, #21, #22, #12, #83). Whole CLI
  runs, median wall time of 40, on an Apple M2 Max (macOS 26.6.2) and a
  Ryzen 7 8745HS (Arch Linux); every round is in `docs/performance.md`:
  - PNG compression (#20): a 2200×1440 render takes 7.9 ms instead of 9.9
    on the Ryzen, and a 5800×3840 one 27.6 instead of 40.5; up to 9% less
    on the M2 Max, most at high resolution.
  - Replay (#21): the 4.7 MB ANSI log renders in 18.3 ms instead of 21.9 on
    the M2 Max and 16.1 instead of 19.2 on the Ryzen; Thai, mixed-script and
    scrolling logs parse 1.8-2.6 times as fast.
  - Painting (#22): 3,000 rounded corners take 10.1 ms instead of 15.3 at
    48 px and 32.8 instead of 69.9 at 128 px on the M2 Max, and 8.7 instead
    of 14.1 and 25.6 instead of 63.2 on the Ryzen; images are 3-9% faster,
    and screens over 16 MiB up to 9% on the Ryzen.
  - Text-only runs decide whether they need a font in one pass instead of
    two: the 4.7 MB log's `--text` takes 13.6 ms instead of 18.8 on the M2
    Max.
  - The move to Rust (#12) slowed no case by more than 2% with confidence:
    `reply-sent` is 5-6% faster on Linux x86-64, and the glyph blend up to
    16% faster on macOS and 23% on Linux.
  - Release binaries are built with fat LTO (`-C lto=fat`, #83): 8.7%
    smaller on macOS universal, 5.5% on Linux x86_64 and 5.0% on Linux
    aarch64, and the archives 3.4-4.6% smaller, with the same output.
    `reply-sent` is as fast; `cursor-moves` and two other cases are 1.5-3%
    slower and `thai-combining` 2-6% faster.
  - With the 0.2.0 release binaries, `examples/reply-sent.pty` at
    2200×1440 takes 8.6-8.7 ms on the M2 Max (the universal binary's arm64
    slice) and 8.2-8.4 ms on the Ryzen (the static x86_64 musl binary).
- **Drawing and PNG compression are Rust** (#12): the compressor
  (`src/deflate.rs`), box drawing and blocks (`src/geometry.rs`), image
  compositing (`src/composite.rs`), the text (`src/glyphs.rs`) and the render
  driver (`src/render.rs`). `src/draw.c` and `src/deflate.c` are gone; the C
  left is stb_truetype's font setup and metrics, stb_image_write's PNG
  packaging (`src/stb_glue.c`) and the PNG decoder's wrapper. Every PNG,
  warning and exit code is the same as the C's, but for the fixes below,
  checked by `bench/c-vs-rust/run.sh` on every fixture on macOS and Linux. A
  CFF or CFF2 face is never handed stb's outline readers. A bug while
  painting fails the render with exit 2 ("painting failed") instead of
  reading past an image; running out of memory for the raster or a glyph
  still exits 2.
- Running out of memory while compressing the PNG exits 2 with an "out of
  memory" message, as other allocation failures do. It exited 1 and said the
  PNG could not be written.
- A font that can't be used is reported as "not a usable font", no longer
  "not a usable TrueType font". A font with no `glyf` table is refused for
  what it has (#38): a color bitmap font such as Noto Color Emoji (`CBDT` or
  `sbix`, no outlines) is named as one, with a pointer to an outline font
  such as Noto Emoji, instead of being called a CFF font.
- `TERMSHOT_PROFILE` times the built-in font, a `--font` file and a
  `--fallback-font` on the same allocate/read/check/padding boundaries
  (`font_*_ms`, `fallback_*_ms`, with their sizes and `font_builtin`), adds
  `face_ms`, and counts glyph cache evictions, missing glyphs and fallback
  lookups and rasterizations (#19). The built-in font's `font_check_ms` no
  longer includes copying and padding it.
- `build.sh` takes extra rustc flags from `RUSTFLAGS`.
- `docs/performance.md` is a versioned report: each round keeps its raw
  samples, binary hashes, toolchains, fonts and inputs, and the README's
  Speed section quotes it (#23). `scripts/bench.py --suite text` times
  text-only runs.

### Fixed

- An output that names the log or a font overwrote it, and two outputs that
  name one file overwrote each other. Both are refused with exit 2, however
  the path is spelled: `a`, `./a`, a symlink, a dangling symlink, or a hard
  link.
- A glyph with no outline left its cell blank (#24). Color emoji fonts such
  as Apple Color Emoji map every emoji to one, and some text fonts map a few
  characters to one (µ in iA Writer Duospace). It now counts as missing:
  the fallback font draws the character, or it is drawn as a box. Blank
  characters keep their empty glyphs.
- A glyph with a box but no points, such as a composite of an empty glyph,
  draws nothing. It drew whatever its bitmap's memory held, which on Linux
  was not always zeros, so its pixels could depend on what was drawn before.
- U+2800 BRAILLE PATTERN BLANK, which TUIs such as btop use for empty graph
  dots, and the line and paragraph separators U+2028 and U+2029 drew as
  boxes when the font lacked them. They are blank, like space separators.
  The Hangul fillers (U+115F, U+3164, U+FFA0) drew as boxes too; they draw
  nothing.
- A font whose `hhea` ascender is not above its descender is refused at load
  with that reason (exit 1). As `--fallback-font` it was scaled by a height
  of 0 or less, and as `--font` it was refused only when drawing, as "font
  metrics unusable".

## [0.1.0] - 2026-10-01

### Added

- Release archives on GitHub: a universal macOS binary (arm64 and x86_64,
  macOS 11 or newer) and static Linux binaries for x86_64 and aarch64, with
  SHA256SUMS. Each binary renders the samples byte for byte like the build
  the pixel goldens check.
- Wide characters (CJK, Hangul, fullwidth forms, emoji) take two cells, by
  the Unicode 17.0 widths in `src/unicode_tables.rs` (`tools/unicode-tables.sh`
  regenerates them). They wrap instead of splitting, and an edit that cuts
  one in half blanks the rest, as in xterm. Combining marks compose into the
  character before them when Unicode has the precomposed form, and are
  otherwise dropped; zero-width characters take no cell.
- `--fallback-font FILE` for the characters the main font lacks, centered in
  their cells.
- A character no font has is drawn as an outlined box instead of nothing.
- Every box-drawing and block-element character (U+2500–U+259F) is drawn as
  geometry: light, heavy, double and dashed lines with all their corners,
  tees and crosses, arcs, diagonals, eighth blocks, shades and quadrants.
  Previously only 12 were, and the rest came from the font, overflowing
  their cells and not lining up.
- 16 and 256 colors (xterm's palette, including `38;5;n` and `38:5:n`),
  dim, underline, double underline (`21`, `4:2`), strike-through, reverse
  video and hidden text.
- A built-in font: `termshot <log> <out.png>` needs no other files.
- Options `--font`, `--px`, `--size COLSxROWS`, `--verbose`, `--help` and
  `--version`, with `-` for stdin and stdout. The original positional form
  still works. Unknown options and extra arguments are errors.
- The screen model of a VT terminal, following xterm: autowrap, scrolling,
  scroll regions, inserting and deleting lines, origin mode, and the
  alternate screen (1049, 1047, 47, 1048).
- Cursor and editing sequences: tabs and tab stops, BS, CHA/HPA/VPA,
  HPR/VPR, CNL/CPL, ICH/DCH/ECH, REP, DECSC/DECRC with attributes,
  IND/NEL/RI, RIS, and DEC Special Graphics line drawing.
- `tests/vt/`: VT cases and recorded `ls`, `less` and `vi` sessions,
  checked against tmux.
- Optional `TERMSHOT_PROFILE` JSON timings for input, parsing, font loading,
  rendering, PNG encoding, and file writing.
- Reproducible benchmarks with raw samples, median and p95 timings, output sizes,
  and a documented comparison against the previous implementation.
- Extended pixel regression tests and independent PNG/zlib round-trip checks,
  including sanitizer coverage on macOS and Linux CI.
- Detailed font reading, validation, padding, and DEFLATE stage timings.
- Benchmarks for high-resolution output, random colors, long ASCII input, and
  rounded boxes, with optional peak RSS measurements and exact PNG comparisons.
- Repeated benchmarks with child CPU timings, paired speedup intervals, input
  fingerprints, and real shell, less, and vi recordings.
- Regression coverage for batched scrolling, larger glyph-cache collisions,
  distant rounded corners, and compressor allocation failures.

### Changed

- Quiet by default: the metrics line moved behind `--verbose`. Exit status
  is 1 for unreadable or unwritable files and unusable fonts, and 2 for bad
  arguments. termshot refuses to write a PNG to a terminal, and removes the
  output file it created when a run fails.
- A bare LF moves down without returning to column 0, as in a terminal.
- PNGs are RGB with no row filter: the same pixels, about 23% smaller.
- A faster deflate that writes stb's exact bytes, and a faster parser for
  large logs: about 18 ms for a 2200×1440 frame instead of about 140 ms.
- CFF (`.otf`) fonts are rejected; TrueType fonts are checked before use.
- Reuse rasterized glyphs within each render, copy repeated background scanlines,
  and clip glyph bounds before blending to reduce drawing work.
- Reject DEFLATE candidates that cannot improve the current match while
  preserving compressed output for nonempty input.
- Preserve concurrent rendering and checked font loading when profiling is
  enabled; `font_load_ms` covers font reading, validation, and padding.
- Paint directly into PNG scanlines to remove a full image allocation and copy.
- Batch printable ASCII runs and reuse CSI parameter storage while preserving
  terminal parsing behavior.
- Validate glyph coordinate lengths without expanding repeated flags, and reuse
  rounded-corner offsets within each render.
- Speed up DEFLATE token emission, Adler-32, and PNG CRC-32 while preserving
  compressed bytes and pixels.
- Skip ASCII rows that cannot survive the current run's scrolling, avoid
  clearing rows before fully overwriting them, and simplify CSI digit saturation.
- Retain up to 1,024 glyph bitmaps per render to reduce Unicode rasterization
  without changing pixels.
- Compare DEFLATE matches in 16-byte groups and flush bits with one capacity
  check per token while retaining byte-identical output and failure handling.
- Release input logs before rendering to reduce overlapping buffer lifetimes.
- Replace the README's fixed latency headline with paired, versioned results
  and document the source and limits of the historical “~20 ms” measurement.

### Fixed

- Rounded corners (╭╮╯╰) painted a pixel into the neighbouring cell.
- A heap overflow in stb_image_write for very large images (#2).
- Pixels differing between macOS and Linux at some sizes (#3).
- Out-of-bounds reads in stb_truetype on damaged fonts, and a
  use-after-free when rendering on several threads (#8).
- Parser bugs that corrupted ordinary output: stray characters after
  `ESC ( B`, erase modes, `ESC [2J` homing the cursor, `38;5;n` turning
  on bold, unterminated strings, invalid UTF-8 (#5).
- Report PNG short writes and close failures instead of reporting success.
- Produce a valid zlib stream for empty input in both compression implementations.
- Release compressed data if allocating the final PNG buffer fails.
- termshot did not build on aarch64 Linux, where C `char` is unsigned and
  the system libraries need the C code linked before them.
- A wide character on a one-column screen panicked (exit 101). It now
  takes the one cell as a narrow character (#18).

[Unreleased]: https://github.com/momiji-rs/termshot/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/momiji-rs/termshot/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/momiji-rs/termshot/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/momiji-rs/termshot/releases/tag/v0.1.0
