# Changelog

Notable changes to termshot are recorded here for users and contributors.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

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

[Unreleased]: https://github.com/solcreek/termshot/commits/main/
