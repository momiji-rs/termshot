# Changelog

Notable changes to termshot are recorded here for users and contributors.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Optional `TERMSHOT_PROFILE` JSON timings for input, parsing, font loading,
  rendering, PNG encoding, and file writing.
- Reproducible benchmarks with raw samples, median and p95 timings, output sizes,
  and a documented comparison against the previous implementation.
- Extended pixel regression tests and independent PNG/zlib round-trip checks,
  including sanitizer coverage on macOS and Linux CI.
- Detailed font reading, validation, padding, and DEFLATE stage timings.
- Benchmarks for high-resolution output, random colors, long ASCII input, and
  rounded boxes, with optional peak RSS measurements and exact PNG comparisons.

### Changed

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

### Fixed

- Report PNG short writes and close failures instead of reporting success.
- Produce a valid zlib stream for empty input in both compression implementations.
- Release compressed data if allocating the final PNG buffer fails.

[Unreleased]: https://github.com/solcreek/termshot/commits/main/
