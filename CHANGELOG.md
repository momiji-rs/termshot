# Changelog

Notable changes to termshot are recorded here for users and contributors.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Read asciinema recordings (`.cast`, asciicast v2 and v3) as the log (#1).
  The output events are replayed in order; input, markers and exit events are
  ignored. Without `--size`, the grid takes the recording's size: its header's,
  or its last resize event's. A first line that is a JSON object with a
  `"version"` member marks a cast; `--cast` reads one whatever it starts
  with, and `--raw` reads a log as raw output whatever it starts with. The JSON is read strictly; a malformed cast is refused with its line
  (exit 1), and one larger than 500x200 needs `--size` (exit 2).

- Replay kitty inline graphics (`a=T,t=d`) in RGB, RGBA and PNG formats,
  including chunked uploads, alpha blending, cell-sized placements, native
  pixel sizing, image-ID replacement and deletion (#41). Images track scrolling
  and alternate screens. Unsupported graphics features, including
  external-file transfers, remain ignored; see the README for the supported
  subset and resource limits.

- Draw Sixel images (`ESC P … q … ESC \`, #41) as xterm decodes them: HLS and
  RGB colour registers, private to each image and starting from the VT340's
  16 colours, repeats, `$` and `-`, raster attributes, and `P2` for an opaque
  or transparent background. Pixels are square and native size, measured in
  the font's cells like kitty's native sizing. The image starts at the cursor,
  scrolls the screen when it passes the bottom margin, and leaves the cursor on
  the last row it covers; DECSDM (`CSI ? 80 h`) draws it at the top left
  instead. Images share the kitty image store, its layering and its limits;
  one over 8,192 pixels on a side or 4,194,304 in all is refused, as are
  images past a per-log budget of 16,777,216 pixel writes plus 256 per byte
  of Sixel data, so a short log cannot demand unbounded work, overdrawing
  included. Text and
  JSON output load the font for a log with Sixel, since the cell height moves
  the cursor. As in xterm, whose Sixel pixels belong to the cells, a
  character written over the image later clears its pixels in the cells it
  takes, and ED 0 and 1 clear them in the rows below or above the cursor's
  (not in its own row); EL, ECH, ICH and DCH leave them, as xterm does.
  Kitty images, a layer of their own, keep their pixels.

- Kitty graphics store images apart from their placements (#44): `a=t`
  transmits without placing, `a=p` places a stored image again, `I` names
  images by number, and a placement id moves one placement. Deletes support
  kitty's `a`, `i`, `n`, `r`, `c`, `p`, `q`, `x`, `y` and `z` selectors, and
  uppercase frees the image data. Over the storage limit, images without a
  placement and then the least recently placed are freed instead of refusing
  the new image. Draw order now breaks z-index ties by creation, as kitty does.

- Kitty graphics accept zlib-compressed payloads (`o=z`, #44) in every format.
  As in kitty, the data must inflate to exactly its size, which a compressed
  PNG gives in `S`. A compressed RGB or RGBA upload may be at most 1 KiB over
  its decoded size, as in kitty, checked as each chunk arrives, and its
  dimensions are checked before anything is inflated.

- Kitty placements crop and layer as kitty does (#44): `x`, `y`, `w`, `h`
  pick the part of the image shown, `X`, `Y` start it inside its first cell,
  and a negative z-index draws it under the text, or with `z` below
  -1,073,741,824 under every background that is not the default one.
  Reverse video and the block cursor count as non-default there. A crop keeps
  its own aspect ratio when fitted to `c` and `r`. Commands with these keys
  were ignored before.

- Kitty relative placements (#44): `P` and `Q` name a parent placement, and
  the image starts `H`, `V` cells from its top left cell. Children move and
  scroll with their parent and are deleted with it, by every delete
  selector; a child's image left without placements is freed. The cursor
  does not move after a relative put. A missing parent, a cycle, or a chain
  of more than 8 links refuses the put, as kitty does. Commands with these
  keys were ignored before.

- Kitty Unicode placeholders (#44): `U=1` makes a virtual placement, which
  draws nothing and moves no cursor, and each U+10EEEE cell shows the part
  of its image under it. The foreground colour names the image (a palette
  colour by its index, a 24-bit one as 0xRRGGBB, the third diacritic as the
  high byte), the underline colour (SGR 58, otherwise not drawn) names the
  placement, and the first two diacritics give the row and column, inherited
  from the cell to the left as kitty does. The image is fitted into the
  placement's `c` x `r` cells and letterboxed. Placeholders are text, so the
  image scrolls, is erased and is overwritten with them; a virtual placement
  survives full-screen erase and reset, and only `d=i`, `n` and `r` delete
  it. A relative placement may have a virtual parent. The placeholder cells
  are drawn blank; `--text` and `--json` keep their code points.
  Commands with `U=1` were ignored before.

- `--lf-newline` treats each bare LF as CR LF, as a terminal with `onlcr`
  does, for logs not captured through a PTY (#28). A bare LF that ends the
  input ends the last line instead of scrolling, so
  `tmux capture-pane -e -p | termshot --lf-newline --size <pane size>`
  keeps the top row; a final CR LF still scrolls, so a PTY log renders
  the same with the flag as without.
- A hint on stderr when a log has line feeds but no CR and `--lf-newline`
  is not given. The image is still written and the exit status is 0.
- A warning on stderr when a character is drawn as a box because a font maps
  it to an empty glyph, as color bitmap fonts such as Apple Color Emoji do
  (#38). It names the first such cell, the font (and whether it is a color
  bitmap font), and what to pass instead. The image is still written and the
  exit status is 0. A character no font has at all is drawn as a box
  silently, as before.
- The cursor is drawn where the log leaves it, as a block in reverse video
  (#33). On a wide character it covers both cells. With a wrap pending it
  stays on the last column, as terminals draw it.
- The cursor takes the shape a program sets with DECSCUSR (`CSI Ps SP q`,
  #39): an underline (3, 4) or a bar (5, 6) in the default foreground, over
  the cell's own colours, or a block (0 to 2). As in kitty, which draws it
  with the text, it is over images under the text and under images of
  z-index 0 and up (Sixel images too). Blinking
  shapes are drawn steady. Like tmux, the shape survives DECSC/DECRC, DECSTR
  and the alternate screen; unlike tmux, and like xterm, RIS resets it.
  `--cursor-shape block|underline|bar` overrides it, and `--json` reports it
  as the cursor's `shape`.
- `--cursor COL,ROW` draws the cursor there instead, counting from 0 as
  tmux's `#{cursor_x},#{cursor_y}` do, and `--cursor none` leaves it out.
  A tmux capture-pane has no cursor, so the README's tmux example now asks
  tmux for it. COL may be the column count, which is how tmux reports a
  pending wrap.
- `--text FILE` writes the screen as text, as `tmux capture-pane -p` prints
  it: a line per row, trailing spaces trimmed, a wide character once (#32).
  `<out.png>` is optional with it; without a PNG nothing is drawn or
  encoded, and no font is read unless the log has kitty graphics, whose
  placements need the font's cell size to move the cursor. `tests/grids/` holds
  the text of every golden fixture, and the tmux references in `tests/vt/`
  check it too.
- `--json FILE` writes the screen with its colours and the cursor (#32):
  per row, runs of cells alike in colour (`#rrggbb`, as drawn) and
  attributes, each with the column it starts at, and the cursor as
  `{"col","row"}` or null. Like `--text`, it needs no PNG, and a font only
  for a log with kitty graphics.
  `tests/grids/` holds it for every golden fixture, and
  `tests/grids/check.py` checks that it parses and agrees with `--text`.
- A face of a font collection (`.ttc`) can be picked for `--font` and
  `--fallback-font` (part of #25): `FILE#N` by number from 0, or
  `FILE#NAME` by its full or family name, ignoring case. A face that
  doesn't exist, or a name that two faces share, is refused with the list
  of faces (exit 1). A file whose name has a `#` in it is still read as
  that file. `-v` prints the face used.
- A hint on stderr when a collection of several faces is given without
  picking one: which face was used (the first, as before) and the list of
  the others. `FILE#0` uses the first without the hint.
- Fonts with CFF outlines, such as `.otf` files and the Noto Sans CJK
  collections, for `--font` and `--fallback-font` (#25). termshot reads the
  CFF table and runs its charstrings in Rust (`src/cff.rs`), limiting the
  work one glyph may take, and stb_truetype only rasterizes the outline;
  stb's own CFF reader hangs, asserts or reads out of bounds on damaged
  fonts. A damaged CFF table is refused with a reason (exit 1), and a
  glyph that can't be drawn is drawn as the box for a missing glyph.
- Variable fonts with CFF2 outlines, such as the variable Noto Sans CJK and
  Source Han Sans builds, are drawn at their default instance (#51), where
  they were refused. `src/cff.rs` reads the CFF2 table with the same checks
  and the same per-glyph limit as CFF; `blend` keeps its default values.
  Choosing another instance (a weight, say) is not supported yet. A damaged
  CFF2 table is refused with a reason (exit 1).
- Italic (SGR 3, cleared by 23), which vim comments, `bat` and `delta` use,
  is drawn, and `--json` reports it as `"italic": true` (#26). The glyph's
  outline is slanted 12 degrees before it is rasterized, so it is as smooth
  as upright text with any font. Box drawing, block elements and the box
  for a missing glyph stay upright.
- Combining marks with no precomposed form are kept and drawn (#14): Thai
  vowel and tone marks, Hebrew points, stacked or uncommon Latin accents
  (q + U+0301), and, approximately, Indic vowel signs and viramas. A cell
  keeps up to four marks after its character, in a side table beside the
  cells, and each is drawn over the character in its colours, from the font
  or else `--fallback-font`, where its font puts it (there is no shaping).
  `--text` and `--json` write a cell's marks after its character, as
  `tmux capture-pane -p` does. Marks join the last printed character,
  wherever the cursor has gone since, as in xterm, and go with it when the
  line is edited or scrolled; one whose character was erased or overwritten
  is dropped. REP repeats a character with its marks. Emoji sequences (ZWJ,
  skin tones, VS16) are kept in the text but still drawn one code point per
  cell. Joiners, variation selectors and the Hangul fillers (U+115F, U+3164,
  U+FFA0) draw nothing; a filler used to be drawn as a box.

### Changed

- The PNG compressor is Rust (`src/deflate.rs`) instead of C, the first step
  of #12; stb_image_write, still C, calls it. PNGs are byte for byte the same.
  On x86-64 its Adler-32 uses SSE2, and the whole compressor is 4-13% faster
  than GCC's build of the C; `reply-sent` renders 5-6% faster on Linux x86-64
  and the same on macOS arm64, where a few glyph-heavy cases are 1-2% slower
  (docs/performance.md).
- Box drawing, block elements and the cache of rounded corners and diagonals
  are Rust (`src/geometry.rs`) instead of C, step 2a of #12; draw.c calls it
  once per cell. Every pixel is the same: `bench/c-vs-rust/run.sh geometry`
  compares it with the C it replaced at every cell size, and CI runs that on
  all three hosts. Geometry renders take the same time on macOS arm64 and
  Linux x86-64 (docs/performance.md). `TERMSHOT_PROFILE`'s `geometry_cache_bytes` is
  16 KiB higher once a stroke is kept: a cache slot is 32 bytes in Rust, 16
  in C.
- After a kitty placement, the cursor moves as kitty moves it
  (`handle_put_command`, `screen_handle_graphics_command`): right by the
  placement's columns and down by its rows less one, so it ends beside the
  image's last row, not below it. Reaching the right edge goes to the start
  of the next row, and passing the bottom margin scrolls the region up by the
  overshoot instead of clamping the cursor there. An image is no longer cut
  at the screen's bottom when placed: as in kitty, scrolling without margins
  brings the rest into view. `C=1` still leaves the cursor in place.
- A font that can't be used is reported as "not a usable font", no longer
  "not a usable TrueType font".
- Renders now show the cursor unless the log hides it with `ESC [ ? 25 l`
  (DECTCEM), as full-screen programs and progress bars often do.
- `TERMSHOT_PROFILE` times the built-in font, a `--font` file and a
  `--fallback-font` on the same allocate/read/check/padding boundaries
  (`font_*_ms`, `fallback_*_ms`, with their sizes and `font_builtin`), adds
  `face_ms`, and counts glyph cache evictions, missing glyphs and fallback
  lookups and rasterizations (#19). The built-in font's `font_check_ms` no
  longer includes copying and padding it.
- The README's Speed section quotes the latest measured round (Apple M2 Max
  and Ryzen 7 8745HS, `721d3fe`, after #20, #21 and #22) with its
  workloads, fonts, image sizes, revision and statistic, adds a text-only
  run, links the versioned report, and keeps the first 2026-10-03 baseline
  (`22b77e8`), the 2026-10-01 Apple M3 rounds and the "~20 ms" figure apart
  as history (#23).
- PNG compression is faster and writes the same bytes (#20). Adler-32 no
  longer needs a 32-bit vector multiply, which baseline x86-64 lacks, and
  the match loop inlines its per-token helpers and reverses Huffman codes
  by table, which helps GCC builds. On a Ryzen 7 8745HS a 2200×1440 render
  takes 7.9 ms instead of 9.9 and a 5800×3840 one 27.6 instead of 40.5; on
  an Apple M2 Max up to 9% less, most at high resolution.
  `docs/performance.md` has the measurements.
- Rounded corners, diagonals, large screens and images are drawn faster, to
  the same pixels (#22). A corner or diagonal is painted from one rasterized
  before it when its points round the same way, which is checked exactly;
  on rasters over 16 MiB, backgrounds are painted a row of cells ahead of the
  text over them, while the row is in the cache; and images find their
  source columns without a division per pixel. The 3,000-corner benchmark
  takes 10.1 ms instead of 15.3 at 48 px and 32.8 instead of 69.9 at 128 px
  on an Apple M2 Max, and 8.7 instead of 14.1 and 25.6 instead of 63.2 on a
  Ryzen 7 8745HS, where 5280×3840 screens are also up to 9% faster;
  `docs/performance.md` has the measurements.
- Replaying a log is faster and leaves the same screen (#21). Character
  widths come from a two-level table instead of two binary searches, marks
  that compose with nothing skip the composition search, the pen's colours
  are mixed once per SGR rather than once per character, erasing a row
  copies instead of filling cell by cell, CSI digits skip the general byte
  match, and long ASCII runs are scanned eight bytes at a time. Deciding
  whether a log is a cast no longer searches it for its first LF unless it
  starts with `{`; a raw log without a LF used to be searched to its end. The
  4.7 MB ANSI replay renders in 18.3 ms instead of 21.9 on an
  Apple M2 Max and in 16.1 instead of 19.2 on a Ryzen 7 8745HS, and Thai,
  mixed-script and scrolling logs parse 1.8-2.6 times as fast.
  `docs/performance.md` has the measurements.
- `--text` and `--json` runs without a PNG decide whether they need fonts in
  one pass over the log, sixteen bytes at a time, instead of two passes a
  byte at a time; they read a font for exactly the same logs as before. The
  4.7 MB ANSI replay's text takes 13.6 ms instead of 18.8 on an Apple M2 Max,
  and the decision 0.66 ms instead of 5.9. `scripts/bench.py --suite text`
  measures these runs; `docs/performance.md` has the measurements.

### Fixed

- A kitty RGB or RGBA payload up to 10 bytes longer than its pixels loads,
  as in kitty, which ignores the excess (#55). termshot required the exact
  length, and accepted up to 16 MiB of payload before refusing a longer one.
- Running out of memory while compressing the PNG exits 2, as other
  allocation failures do, with an "out of memory" message. It exited 1 and
  said the PNG could not be written.
- A font with no `glyf` table is refused for what it has instead (#38). A
  color bitmap font such as Noto Color Emoji (`CBDT` or `sbix`, no outlines)
  is named as one, with a pointer to an outline font such as Noto Emoji,
  instead of being called a CFF font.
- Output collision checks follow dangling symlinks and compare existing file
  identities, rejecting hard-link aliases of outputs, logs or fonts before
  writing (#43 review).

- ASCII autowrap and skipped-row batching now scroll images alongside text;
  whole-region skips discard contained image pixels even when row-storage rotation is
  zero modulo the region height (#43 review).

- Kitty scrolling moves only placements wholly inside the affected region,
  clipping them at its edges. Images crossing either margin remain stationary,
  including insert/delete line and reverse index (#43 review).
- Explicit kitty image ID `i=0` is rejected; omitting the ID remains valid.

- U+2800 BRAILLE PATTERN BLANK, which TUIs such as btop use for empty graph
  dots, and the line and paragraph separators U+2028 and U+2029 drew as
  boxes when the font lacked them. They are blank, like space separators.
- A glyph with no outline left its cell blank (#24). Color emoji fonts such
  as Apple Color Emoji map every emoji to one, and some text fonts map a few
  characters to one (µ in iA Writer Duospace). It now counts as missing:
  the fallback font draws the character, or it is drawn as a box. Blank
  characters keep their empty glyphs.
- An output that names the log or a font overwrote it, and two outputs
  that name one file overwrote each other. Both are refused with exit 2,
  however the paths are spelled (`a`, `./a`, a symlink).
- A kitty graphics payload in base64 without its `=` padding was refused, so
  `kitten icat` without `--place`, which sends small PNGs that way, drew
  nothing. As in kitty, each chunk is decoded on its own and may end in a
  partial group, padded or not; a padded chunk no longer has to be the last.
  Invalid characters, a length of 4n+1, wrong padding and data after it are
  still refused.

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

[Unreleased]: https://github.com/momiji-rs/termshot/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/momiji-rs/termshot/releases/tag/v0.1.0
