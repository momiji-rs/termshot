# Changelog

Notable changes to termshot are recorded here for users and contributors.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Replay kitty inline graphics (`a=T,t=d`) in RGB, RGBA and PNG formats,
  including chunked uploads, alpha blending, cell-sized placements, native
  pixel sizing, image-ID replacement and deletion (#41). Images track scrolling
  and alternate screens. Unsupported graphics features, including Sixel and
  external-file transfers, remain ignored; see the README for the supported
  subset and resource limits.

- `--lf-newline` treats each bare LF as CR LF, as a terminal with `onlcr`
  does, for logs not captured through a PTY (#28). A bare LF that ends the
  input ends the last line instead of scrolling, so
  `tmux capture-pane -e -p | termshot --lf-newline --size <pane size>`
  keeps the top row; a final CR LF still scrolls, so a PTY log renders
  the same with the flag as without.
- A hint on stderr when a log has line feeds but no CR and `--lf-newline`
  is not given. The image is still written and the exit status is 0.
- The cursor is drawn where the log leaves it, as a block in reverse video
  (#33). On a wide character it covers both cells. With a wrap pending it
  stays on the last column, as terminals draw it.
- `--cursor COL,ROW` draws the cursor there instead, counting from 0 as
  tmux's `#{cursor_x},#{cursor_y}` do, and `--cursor none` leaves it out.
  A tmux capture-pane has no cursor, so the README's tmux example now asks
  tmux for it. COL may be the column count, which is how tmux reports a
  pending wrap.
- `--text FILE` writes the screen as text, as `tmux capture-pane -p` prints
  it: a line per row, trailing spaces trimmed, a wide character once (#32).
  `<out.png>` is optional with it; without a PNG no font is read, so a
  text-only run takes about 1 ms where a PNG takes 10. `tests/grids/` holds
  the text of every golden fixture, and the tmux references in `tests/vt/`
  check it too.
- `--json FILE` writes the screen with its colours and the cursor (#32):
  per row, runs of cells alike in colour (`#rrggbb`, as drawn) and
  attributes, each with the column it starts at, and the cursor as
  `{"col","row"}` or null. Like `--text`, it needs no PNG and no font.
  `tests/grids/` holds it for every golden fixture, and
  `tests/grids/check.py` checks that it parses and agrees with `--text`.

### Changed

- Renders now show the cursor unless the log hides it with `ESC [ ? 25 l`
  (DECTCEM), as full-screen programs and progress bars often do.

### Fixed

- Output collision checks follow dangling symlinks and compare existing file
  identities, rejecting hard-link aliases of outputs, logs or fonts before
  writing (#43 review).

- ASCII autowrap and skipped-row batching now scroll images alongside text;
  whole-region skips discard image pixels even when row-storage rotation is
  zero modulo the region height (#43 review).

- Partial-region scrolling preserves image pixels outside the affected rows,
  including images that cross either margin, insert/delete line and reverse
  index (#43 review).

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
