# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

termshot replays a raw PTY log (bytes with ANSI escapes) into a cell grid and writes a PNG of the final screen. It has no crates.io dependencies and no Cargo: it is plain `rustc` plus a C compiler, and it links only libc and libm. MSRV is rustc 1.70 (CI checks it), so don't use newer std APIs or syntax.

## Commands

```sh
./build.sh                      # builds ./termshot (C objects → libtermshot_c.a, then rustc src/main.rs)
./test.sh                       # everything: unit tests, C harnesses, CLI checks, pixel goldens
./test.sh --update-goldens      # rewrite tests/goldens.txt after an intended pixel change
SANITIZE=1 ./test.sh            # draw.c under ASan + UBSan (macOS clang only)
SANITIZE=1 ./tests/run.sh       # extended: codec round trips, deflate alloc failures, draw.c pixels, profile
```

`./test.sh` builds the unit-test binary at `target/test/unit` (a libtest harness from `rustc --test src/main.rs`). After one full run you can run tests directly:

```sh
./target/test/unit <name-substring>   # a single test or a filter
./target/test/unit --ignored          # #[ignore] tests only
```

A test for a known bug states the correct behaviour and is marked `#[ignore = "#N: ..."]` with its issue, so it starts passing when the issue is fixed. None are open now. The bare `#[ignore]` on `poc_workloads` is different: that function is a benchmark helper, not a test. It writes inputs to `$TERMSHOT_POC_DIR` and does nothing when the variable is unset (see `docs/c-vs-rust.md`).

Rebuild it after editing Rust without running the whole suite:

```sh
./build.sh && rustc --edition 2021 --test src/main.rs -o target/test/unit -L native="$PWD" -l static=termshot_c
```

Renders from the test run stay in `target/test/` for inspection. When a change is meant to move pixels, look at them first, then update the goldens.

Other tools: `tests/vt/oracle.sh` compares `tests/vt/expected.txt` against tmux (not in CI); `tests/vt/record.sh` records real sessions; `tools/unicode-tables.sh` regenerates `src/unicode_tables.rs` from the UCD (don't edit that file by hand); `tools/rowcolumn-diacritics.sh` regenerates `src/rowcolumn_diacritics.rs`, kitty's Unicode placeholder diacritics, likewise; `scripts/bench.py` benchmarks; `TERMSHOT_PROFILE=1` prints stage timings as JSON on stderr; `scripts/release.sh <platform>` builds a release archive.

## Architecture

The pipeline is Rust in, C out:

1. **`src/main.rs`** holds the CLI (`parse_args`), the input handling, and the whole VT parser and screen model: `Screen`, `Pen`, `Cell`, `parse`/`parse_lf`, `csi`, and `utf8_at`. It follows xterm: autowrap, scroll regions, the alternate screen, DEC graphics, and so on. Output is a flat `Vec<Cell>` of `cols*rows` plus final image placements. `src/cast.rs` decodes asciinema `.cast` input (v2 and v3) into the output bytes and the recording's size (the header's, or the last resize's) before any of this runs; it holds the strict JSON reader, which bounds nesting at `MAX_DEPTH`. `src/graphics.rs` handles the supported kitty direct-transmission subset; `src/image.c` wraps the vendored PNG decoder with a bounded allocator. `src/sixel.rs` decodes Sixel images (DCS `q`) as xterm does and feeds each to the kitty image store as a kitty command (`Graphics::sixel`, which wraps `Graphics::command`), so Sixel shares its layering, scrolling and limits; unlike kitty's, its pixels belong to the cells, and text and ED 0/1 clear them.
2. **`src/font.rs`** validates every TrueType structure stb_truetype will touch before C ever sees the font, because stb trusts its input. A font that fails is refused with a reason (exit 1). Fonts reach C with zero padding after them. JetBrains Mono is embedded with `include_bytes!` as the default.
   **`src/cff.rs`** reads CFF and CFF2 outlines (`.otf`, Noto CJK; CFF2 at its default instance only). stb's CFF reader trusts its input, so stb never runs a charstring: `font.rs` checks the CFF table with `cff::Font::parse`, and draw.c asks Rust for each outline through the callback in the `#[repr(C)] Face` it is given (`font::Font::with_face`), then rasterizes it with `stbtt_Rasterize`. On a CFF face, draw.c must not call stb functions that read glyph outlines (`IsGlyphEmpty`, `GetGlyphBox`, `GetGlyphShape`, `MakeGlyphBitmap`); cmap and metrics calls are safe. `src/cff_tests.rs` compares every glyph with stb's and builds hostile fonts; `docs/cff-rust-vs-c.md` is why this is Rust.
3. **`src/unicode.rs`** and **`src/unicode_tables.rs`** give character widths (wide = 2 cells) and canonical composition for combining marks.
4. **`src/draw.c`** is the rasterizer, behind `draw_png_images` (and the cell-only `draw_png` test wrapper). `draw_cell_size` shares its exact font metrics with the parser so native-pixel images move the cursor correctly. It uses stb_truetype for glyphs and paints box drawing and block elements (U+2500–U+259F) as geometry. It writes the PNG through a locally modified `stb_image_write.h`, with `src/deflate.c` as the compressor (stb's deflate made faster, with byte-identical output) and `src/png_crc.h` for the CRC.

`Cell` is `#[repr(C)]` and shared across the FFI boundary: `draw.c` asserts `sizeof(Cell) == 12`, and the `ATTR_*` bits are defined in both languages. Change both sides together. All eight `attrs` bits are taken. Combining marks with no precomposed form live outside `Cell`, in `Screen::marks` (keyed by storage index, kept in step by every edit) and then `Grid::marks`, a sorted `CellMarks` list (cell index plus `MAX_MARKS` = 4 code points) that `draw.c` also reads; `marks_of` looks a cell up.

Unit tests live in `src/tests.rs` and `src/draw_tests.rs` (both `#[cfg(test)]` modules of main.rs). They check the parser against the screens in `tests/vt/expected.txt` and `tests/vt/real/`. Pixel goldens (`tests/golden.rs`) hash *decoded RGBA pixels*, not PNG bytes, so encoder changes that keep the pixels don't break them. The same runs write `--text` and `--json`, compared byte for byte with `tests/grids/<log>.txt` and `.json`; `--update-goldens` rewrites all of them, and `tests/grids/check.py` checks that the JSON agrees with the text. The text rows are also what the `tests/vt/` checks compare, so the tmux references cover `--text`.

## Constraints that aren't obvious

- **Determinism across platforms is a feature.** The same input must give the same pixels on macOS arm64, x86-64 Linux, and aarch64 Linux, with glibc or musl. `draw.c` is built with `-ffp-contract=off` so Apple clang doesn't fuse multiply-adds, which would change pixels (px 46 is in the goldens to catch exactly this). Never add `-ffast-math`.
- The C is linked as a static library (`-l static=termshot_c`), not as bare objects, so rustc orders it before libc and libm. That ordering is what makes static musl and glibc on aarch64 link. Keep `build.sh`, `test.sh`, and `tests/run.sh` consistent if you change the link line.
- Local changes to vendored stb are listed in `third_party/stb/CHANGES.md`; record any new ones there. `stb_truetype.h` is unmodified.
- CLI exit codes are part of the contract: 0 done, 1 unreadable or unwritable file, malformed `.cast`, or bad font, 2 bad arguments (including an image over 2^27 pixels, a cast larger than 500x200 without `--size`, and an output that names an input or another output) or an allocation failure in `draw.c`, whose return code `main` passes through. A failed run removes the output files it created (the PNG, `--text` and `--json`). It is quiet on success, except for the hint about `--lf-newline` and the warning for a character drawn as a box because a font maps it to an empty glyph (`draw.c` reports it in `EmptyGlyphs`; `main.rs` words it). `test.sh` checks all of this.
- A bare LF moves down without a carriage return, as in a real terminal. `--lf-newline` exists for logs not captured through a PTY, such as `tmux capture-pane`.
- User-visible changes go in `CHANGELOG.md` under `[Unreleased]`, in Keep a Changelog format.
