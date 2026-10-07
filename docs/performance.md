# Performance measurements

This file holds dated, versioned measurement rounds, newest first. The
[render-as-a-library round](#the-render-as-a-library-2026-10-06-35c62b9-85-part-2)
(2026-10-06, `35c62b9`, #85 part 2) made the render library API, returned
the PNG without a copy, remeasured every case against main `231f5e0` on the
same two machines, and measured the two-crate build again, which the CLI
now is. The
[parser-as-a-library round](#the-parser-as-a-library-2026-10-06-3c69f71-85-part-1)
(2026-10-06, `3c69f71`, #85 part 1) moved the parser and the screen model
out of `src/main.rs` into library modules, remeasured every case against
main `cd2b11d` on the same two machines, and measured the two-crate build
it does not use. The
[release size profile round](#release-size-profile-2026-10-05-66780fc-83)
(2026-10-05, `66780fc`, #83) measured size options for the release archives
and chose fat LTO. The
[render-driver-in-Rust round](#the-render-driver-in-rust-2026-10-04-cc29aed-12-step-2d)
(2026-10-04, `cc29aed`, #12 step 2d) moved the driver (the raster, the
passes, the PNG write and the profile record) from `src/draw.c` to
`src/render.rs`, leaving `src/stb_glue.c`, and remeasured every case against
main `48192f6` on the same two machines. The
[glyph-painting-in-Rust round](#glyph-painting-in-rust-2026-10-04-ab924ca-12-step-2c)
(2026-10-04, `ab924ca`, #12 step 2c) moved the text (glyphs, the glyph
cache, marks, lines) from `src/draw.c` to `src/glyphs.rs` and remeasured
every case against main `c0b7b02` on the same two machines. The
[image-layers-in-Rust round](#image-layers-in-rust-2026-10-04-82c3f3d-12-step-2b)
(2026-10-04, `82c3f3d`, #12 step 2b) moved image compositing and the
backdrop under the text from `src/draw.c` to `src/composite.rs` and
remeasured every case against main `431ed23` on the same two machines. The
[box-drawing-in-Rust round](#box-drawing-in-rust-2026-10-04-629a4d4-12-step-2a)
(2026-10-04, `629a4d4`, #12 step 2a) moved box drawing, blocks and the stroke
cache from `src/draw.c` to `src/geometry.rs` and remeasured every case against
main `24d71fe` on the same two machines. The
[deflate-in-Rust round](#png-compression-in-rust-2026-10-04-6396b8d-12-step-1)
(2026-10-04, `6396b8d`, #12 step 1) replaced `src/deflate.c` with
`src/deflate.rs` and remeasured every case against main `63d6de8` on the same
two machines; nothing else changed. The
[text-only pre-scan](#text-only-runs-one-pre-scan-instead-of-two-2026-10-03-macos-only)
(2026-10-03, after main `a8a95e0`) changed only how a run without a PNG
decides whether it needs fonts. The
[painting round](#painting-and-geometry-2026-10-03-721d3fe-22) (2026-10-03,
`721d3fe`, #22) changed only painting in `src/draw.c` and adds the `draw`
workloads. The
[ANSI replay parsing round](#ansi-replay-parsing-2026-10-03-10f1ea1-21)
(2026-10-03, `10f1ea1`, #21) changed only the parser, the screen model,
the width and composition lookups and cast detection, and remeasured every
case against main `76f18ee` on the same two machines. The
[PNG compression round](#png-compression-adler-32-and-deflate-matching-2026-10-03-3bf2ffc-20)
(2026-10-03, `3bf2ffc`, #20) changed only `src/deflate.c` and remeasured
every case on the same two machines; for everything outside painting, parsing
and PNG compression, the [font-path baseline](#current-baseline-font-paths-and-linux-2026-10-03-d83c8fd)
after it (main `d83c8fd`, Apple M2 Max and AMD Ryzen 7 8745HS) is still the
reference. The [third optimization round](#historical-third-optimization-round-2026-10-01-c44d83c-apple-m3)
below them (2026-10-01, `c44d83c`, Apple M3) is history: a different revision,
a different machine and a different harness. Do not subtract its numbers from
the current ones.

Figures published elsewhere (the repository's About description, issue #1, the
changelog) are traced, or marked unverified, in
[Published claims and their evidence](#published-claims-and-their-evidence-checked-2026-10-03)
at the end.

**Result files.** Since #83, `scripts/bench.py` writes a slim result by
default. It keeps every per-run wall, CPU, profiled-wall and peak-RSS
sample, the output hashes and sizes, and the metadata. Of the per-run
`TERMSHOT_PROFILE` records, it keeps only the stages named with `--stage`
(repeatable, e.g. `--stage parse_ms`). `--full-profile` keeps every stage
and counter, which `bench-report.py`'s stage table needs, at about ten times
the size. `bench.py --slim FULL.json --output SLIM.json [--stage KEY]`
slims an existing full file. Commit the slim form, with only the stages a
section cites. The files from earlier rounds are full; slimming them is a
possible follow-up.

## The render as a library (2026-10-06, `35c62b9`, #85 part 2)

#85's second part makes the render library API: `termshot::render` returns
the PNG's bytes, every failure is an `Error`, the render's preparation of
the cells (placeholders, the cursor, opaque backgrounds) moves from
`src/main.rs` to `src/prepare.rs`, and the CLI becomes a thin layer over
the public API. Nothing is meant to change in what the CLI makes or how
fast. In the release build (fat LTO) no case is slower on macOS, and on
Linux two large-image cases are 1-2.5% slower from code layout alone
(below); the dev build is the same. The CLI is now a crate of its own that
links the library (`two` below): measured against the one-unit build, it
is never slower beyond noise, so it is the layout used.

### Output

- `./test.sh` writes the same 996 PNG, `--text` and `--json` files as main
  `231f5e0`, byte for byte (sha256), on macOS.
- `tests/library.rs` draws all 68 pixel goldens' cases through
  `termshot::render`, and each PNG is the CLI's, byte for byte.
- `bench/c-vs-rust/run.sh full 1`: 4,135 renders byte-identical to the CLI
  at `48192f6`, with the same exit codes and stderr (macOS).
- Every case below gave the same output bytes from every binary
  (`--verify-identical`), dev and fat LTO builds alike.

### The PNG, returned without a copy

The first build of this round (`8d23873`) copied the PNG out of
stb_image_write's buffer into a `Vec` of its own, which cost the renders
with a large PNG on Linux, in both batches: dev `reply-128px` 0.978 /
0.972, `image-below` 0.976 / 0.984, `image-under` 0.981 / 0.982; fat LTO
`large` 0.971 / 0.971, `large-sparse` 0.971 / 0.960, `reply-128px` 0.967 /
0.977, `geometry-all` 0.979 / 0.977 (paired against main). A buffer the
PNG's size (1 to 6 MB here) is mapped fresh, its pages are faulted in, and
the bytes copied: 0.35 to 0.5 ms on a 20 ms render. `35c62b9` makes
`termshot_png_alloc`, the `STBIW_MALLOC` of the one buffer stb asks for, a
`Vec`'s (`try_reserve_exact`), and returns that `Vec` as it is. The
results below are `35c62b9`'s.

### What is left on Linux: code layout

In `35c62b9`, the cases slower in both batches are Linux's large images:
dev `large` 0.979 / 0.985 and `rounded-128px` 0.981 / 0.984, fat LTO
`large-color` 0.976 / 0.987 and `geometry-all` 0.991 / 0.990. A profiled
probe (`TERMSHOT_PROFILE`, fat LTO, 30 rounds) puts the difference in
`deflate_match_emit_ms`, the compressor's main loop: `large-color` 99.2 ms
on main, 100.1 ms on the branch, every other stage within 0.15 ms or
faster. The compressor is unchanged, and so is its machine code:
`termshot_zlib_compress` is 6,283 bytes in both binaries, but starts at
`0xca440` in main's (64-byte aligned) and `0xd45b0` in the branch's (48
past a 64-byte line). Built with every function 64-byte aligned
(`RUSTFLAGS='-C lto=fat -C llvm-args=-align-all-functions=6'`), the branch
is within noise of main on all of them, 40 rounds: `large` 0.994 [0.981,
1.013], `large-color` 1.007 [0.996, 1.018], `geometry-all` 0.993 [0.981,
1.004], `rounded-128px` 0.995 [0.981, 1.003], `reply-sent` 1.019 [0.992,
1.061] (95% intervals). That probe and the profiled one are not kept.

### Two crates or one compilation unit

#92 kept the CLI and the library in one compilation unit because, with the
CLI using the render's internals, two crates painted 1.6-2.6% slower with
fat LTO on macOS. With the CLI thin, `two` (`35c62b9`'s CLI as a crate of
its own, linking `libtermshot.rlib` with `--extern termshot`, every other
byte the same) was measured in every batch beside `branch` (one unit):

- against main, `two` is slower in both batches on one case of 192
  (Linux dev `large` 0.976 / 0.985, which `branch` shares), and `branch`
  on four;
- against `branch`, `two` is never slower in both batches: its paired
  speedup over `branch`'s ranges 0.974 to 1.033 (macOS dev), 0.977 to
  1.021 (macOS fat LTO), 0.943 to 1.085 (Linux dev) and 0.960 to 1.058
  (Linux fat LTO), and no case has both batches' intervals apart.

So the CLI is a crate of its own now (`62f0165`): build.sh builds the rlib
first, and the CLI links it, as an embedder does. The rlib is the
embedder's artifact either way.

### Results

Paired wall speedup of each binary against main (main's wall time / the
binary's, per interleaved round; above 1 is faster), batches A / B. "In
both" means the bootstrap 95% interval lies past 1 in both batches. Every
suite ran: 48 cases, the system Noto CJK collection included. "Every case"
is the range of all 96 medians.

`branch` (`35c62b9`, one compilation unit):

| case | macOS dev | macOS fat LTO | Linux dev | Linux fat LTO |
| --- | ---: | ---: | ---: | ---: |
| `ansi-replay` | 1.008 / 1.001 | 0.999 / 1.008 | 1.012 / 1.002 | 1.039 / 1.050 |
| `dense-sgr` | 1.007 / 1.002 | 1.009 / 1.002 | 1.021 / 1.015 | 0.993 / 0.992 |
| `thai-combining` | 1.023 / 1.024 | 1.016 / 1.024 | 1.038 / 1.038 | 1.018 / 1.018 |
| `cursor-moves` | 1.009 / 1.004 | 1.007 / 1.005 | 1.000 / 0.971 | 0.991 / 0.993 |
| `mixed-unicode` | 1.013 / 1.011 | 1.011 / 1.012 | 1.029 / 1.029 | 0.997 / 0.998 |
| `text-mixed-unicode` | 1.010 / 1.013 | 1.011 / 1.014 | 1.023 / 1.032 | 0.993 / 0.998 |
| `reply-sent` | 1.006 / 0.996 | 0.994 / 1.004 | 0.992 / 0.972 | 0.975 / 0.982 |
| `reply-128px` | 0.998 / 1.004 | 1.001 / 1.006 | 0.997 / 0.985 | 1.006 / 0.994 |
| `large` | 1.001 / 1.007 | 0.991 / 1.002 | 0.979 / 0.985 | 0.987 / 0.986 |
| `large-color` | 1.003 / 1.001 | 1.006 / 0.997 | 0.999 / 0.989 | 0.976 / 0.987 |
| `geometry-all` | 1.005 / 0.997 | 1.001 / 1.002 | 0.988 / 0.997 | 0.991 / 0.990 |
| `image-under` | 1.004 / 0.999 | 1.005 / 0.990 | 1.001 / 0.977 | 1.001 / 0.991 |
| `image-below` | 0.992 / 0.996 | 1.005 / 1.002 | 0.995 / 0.985 | 1.008 / 1.000 |
| `cjk-full` | 1.003 / 0.995 | 1.010 / 1.007 | 1.021 / 0.978 | 1.021 / 1.010 |
| slower in both | none | none | `large`, `rounded-128px` 0.981 / 0.984 | `geometry-all`, `large-color` |
| faster in both | `rounded-128px` 1.011 / 1.020, `mixed-unicode`, `thai-combining`, `text-mixed-unicode` | `thai-combining`, `text-ansi-replay` 1.010 / 1.014, `text-mixed-unicode` | `mixed-unicode`, `thai-combining`, `text-mixed-unicode` | `text-ansi-replay` 1.072 / 1.072 |
| every case | 0.987 to 1.024 | 0.982 to 1.024 | 0.952 to 1.039 | 0.956 to 1.072 |

`two` (`35c62b9` as two crates, the layout adopted):

| case | macOS dev | macOS fat LTO | Linux dev | Linux fat LTO |
| --- | ---: | ---: | ---: | ---: |
| `ansi-replay` | 1.008 / 1.012 | 1.002 / 1.007 | 1.011 / 0.988 | 1.031 / 1.054 |
| `dense-sgr` | 1.003 / 1.005 | 1.005 / 1.003 | 1.007 / 1.011 | 1.007 / 0.998 |
| `thai-combining` | 1.054 / 1.057 | 1.017 / 1.017 | 1.032 / 1.025 | 1.023 / 1.015 |
| `cursor-moves` | 1.005 / 1.005 | 0.996 / 0.999 | 1.018 / 1.002 | 0.995 / 0.994 |
| `mixed-unicode` | 1.017 / 1.013 | 0.998 / 0.996 | 1.007 / 1.007 | 0.978 / 0.993 |
| `text-mixed-unicode` | 1.019 / 1.018 | 1.000 / 0.991 | 1.007 / 1.023 | 0.984 / 0.998 |
| `reply-sent` | 1.001 / 0.992 | 1.002 / 1.002 | 0.972 / 0.992 | 0.979 / 0.994 |
| `reply-128px` | 1.001 / 1.002 | 1.004 / 1.001 | 0.996 / 0.985 | 0.994 / 0.999 |
| `large` | 0.999 / 1.007 | 0.997 / 1.001 | 0.976 / 0.985 | 0.985 / 0.990 |
| `large-color` | 0.997 / 0.999 | 1.002 / 1.001 | 0.997 / 0.998 | 0.979 / 0.990 |
| `geometry-all` | 1.004 / 1.001 | 0.998 / 1.003 | 0.989 / 0.991 | 0.995 / 0.993 |
| `image-under` | 0.996 / 1.003 | 1.005 / 0.998 | 1.009 / 0.991 | 0.997 / 1.000 |
| `image-below` | 1.008 / 0.997 | 0.997 / 0.993 | 0.998 / 0.979 | 0.991 / 0.982 |
| `cjk-full` | 1.007 / 0.998 | 1.010 / 1.009 | 1.016 / 0.957 | 1.026 / 1.022 |
| slower in both | none | none | `large` | none |
| faster in both | `mixed-unicode`, `thai-combining`, `text-mixed-unicode` | `thai-combining`, `text-ansi-replay` 1.020 / 1.016 | `thai-combining`, `text-dense-sgr` 1.031 / 1.038 | `ansi-replay`, `text-ansi-replay` 1.058 / 1.053 |
| every case | 0.984 to 1.057 | 0.989 to 1.022 | 0.957 to 1.073 | 0.959 to 1.058 |

Peak RSS against main's ranges from 0.935 to 1.079 over every case,
build and binary. The extremes are cases whose five peak-RSS runs land on
one of two levels about 0.7 MB apart, as main's own do (`color-grid` 8.0
or 8.7 MB, `image-below` 15.5 or 16.3 MB, macOS dev).

### The profile record

`TERMSHOT_PROFILE` prints the same keys as before, but `face_ms` (the
faces built for the render, the CFF tables parsed again) is in the
render's record now, beside `output_write_ms`, which the CLI measures and
adds to it: the render is the library's. A run without a PNG prints only
the CLI's record, with `face_ms` 0, as before (`tests/profile.rs`).

### What was measured

| | macOS arm64 | Linux x86-64 |
| --- | --- | --- |
| host | `lawrences-mac-studio`, Apple M2 Max, macOS 26.6.2 | `starship`, Ryzen 7 8745HS, governor `performance` |
| built with | rustc 1.98.1 (Homebrew), Apple clang 21.0.0 | rustc 1.98.1 (Arch), GCC 16.2.1 |
| load average (1 min), start → end | dev: A 6.84 → 5.13, B 5.04 → 5.12; fat: A 5.12 → 3.81, B 3.81 → 4.57 | dev: A 2.92 → 1.16, B 1.16 → 1.34; fat: A 1.34 → 1.03, B 1.03 → 1.13 |

The binaries: `main` is main `231f5e0`, `branch` is `35c62b9`, `two` is
`35c62b9` as two crates (`src/main.rs` without its module list, built with
`--extern termshot=libtermshot.rlib`), each with `build.sh`'s flags (dev)
and with `RUSTFLAGS='-C lto=fat'` added, as `scripts/release.sh` builds
them (fat LTO). Their sha256 values are in each file's `binary_sha256`.
`scripts/bench.py` ran 40 rounds and 5 warmups per batch, and 5 peak-RSS
runs, with seeds 71 (A) and 89 (B). The macOS host is a shared desktop
(other sessions kept its load near 4-5); the paired, interleaved rounds
are what make its numbers comparable. The first round (`8d23873`) is
quoted above and not kept. What changed after `35c62b9` (the parse's rows
and tab stops allocated as its cells are, two errors' variants,
stb_image_write's functions made static, and the two-crate layout itself,
`62f0165`) does not touch a hot path.

The files are slim, with no profile stage kept: dev
[A](performance-2026-10-06-lib-render-dev-macos-a.json) and
[B](performance-2026-10-06-lib-render-dev-macos-b.json), fat
[A](performance-2026-10-06-lib-render-fat-macos-a.json) and
[B](performance-2026-10-06-lib-render-fat-macos-b.json) on macOS; dev
[A](performance-2026-10-06-lib-render-dev-linux-a.json) and
[B](performance-2026-10-06-lib-render-dev-linux-b.json), fat
[A](performance-2026-10-06-lib-render-fat-linux-a.json) and
[B](performance-2026-10-06-lib-render-fat-linux-b.json) on Linux.

### Reproduce

```sh
# main, branch and two, dev and fat LTO; two is the branch's src/main.rs
# without its module list, linking the rlib build.sh makes:
#   rustc --edition 2021 main2.rs -o two-$flavor -C opt-level=2 $flags --extern termshot=libtermshot.rlib
cjk=/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc   # on macOS, a copy (same sha256)
for flavor in dev fat; do
  for batch in a:71 b:89; do
    python3 scripts/bench.py --binary main=/tmp/main-$flavor --binary branch=/tmp/branch-$flavor \
      --binary two=/tmp/two-$flavor --reference main --runs 40 --warmups 5 --memory-runs 5 \
      --verify-identical --cjk-font "$cjk" --seed "${batch#*:}" \
      --output "/tmp/lib-render-$flavor-${batch%%:*}.json"
  done
done
```

## The parser as a library (2026-10-06, `3c69f71`, #85 part 1)

#85's first part moves the parser and the screen model out of `src/main.rs`
into `src/vt.rs`, `src/screen.rs` and `src/grid.rs`, and adds `src/lib.rs`,
which builds them as a library (`libtermshot.rlib`). Nothing is meant to
change but where the code lives. Every output is the same. With fat LTO, as
the release archives are built, no case is slower on either machine; in the
dev build a few parser stress cases are 0.5-3% slower, from code layout once
five calls that `replay_with` used to inline are marked `#[inline]` again
(below). Building the binary as two crates, the CLI linking the library, was
measured too and is not used.

### Output

- `./test.sh` writes 1,732 files to `target/test/` (833 PNGs, 83 `--text`
  and 80 `--json` outputs, and the logs and fonts it generates). Every one
  has the same sha256 as main's, on macOS (Apple clang 21) and on starship
  (GCC 16.2.1).
- `tests/library.rs` links `libtermshot.rlib` as an embedder does and gets
  the 45 grids in `tests/grids/` byte for byte from `Grid::to_text` and
  `Grid::to_json`.
- Every case below gave the same output bytes from every binary
  (`--verify-identical`), dev and fat LTO builds alike.
- `bench/c-vs-rust/run.sh full 1`: 4,135 renders byte-identical to the CLI
  at `48192f6`, with the same exit codes and stderr (macOS).
- `scripts/release.sh macos-universal` (rustup 1.98.1): both slices,
  x86_64 under Rosetta, render the samples like the host build.

### The module boundary and inlining

The first build of the move (`7c5fc89`) ran the parser stress logs slower
in the dev build, in both batches on both machines: `cursor-moves` 0.971 /
0.967 on macOS and 0.947 / 0.948 on Linux, `text-dense-sgr` 0.970 / 0.967
and 0.956 / 0.937, `dense-sgr` 0.985 / 0.977 and 0.973 / 0.963 (paired
speedup against main; below 1 is slower). rustc starts a codegen unit per
module, and with `Screen` in `screen.rs` and `replay_with` in `vt.rs` it
stopped inlining `Screen::print_ascii`, the call that prints every run of
ASCII, and four calls made once per log or per Sixel image (`into_cells`,
`screen_marks`, `placeholders`, `sixel`). `#[inline]` on those five
(`3c69f71`) gives back main's inlining: every parser function in the binary
is the size it is on main, within 64 bytes (macOS: `replay_with` 8,684
bytes against 8,676, `Screen::csi` 5,608 against 5,600; Linux:
`replay_with` 10,219 against 10,155, which stores the grid's size and
background now, and `Screen::csi` the same). A probe of the parser and text
suites on macOS with `print_ascii` alone marked left `cursor-moves` at
0.989 / 0.994.

What is left in the dev build is code layout. On Linux, `dense-sgr` is
0.970 / 0.980 and `cursor-moves` 0.985 / 0.981. Built with every loop
aligned to 64 bytes (`RUSTFLAGS='-C llvm-args=-align-loops=64'`), main
itself moves by up to 2.5% on `cursor-moves` (0.975 / 0.996 against
the plain build), and aligned, the branch is within 1.2% of aligned main
on every case of the parser suite (`cursor-moves` 0.969 / 0.997 against
aligned main's 0.975 / 0.996, `dense-sgr` 0.992 / 0.995 against 1.004 /
0.999; all against plain main). That probe ran 30 rounds of the parser and
text suites on starship and is not kept.

### After review: `Arc` for image pixels

Review asked for a `Grid` that can cross threads, so `5f4a63e` shares image
pixels with `Arc` instead of `Rc` (`src/graphics.rs`,
`src/graphics/animation.rs`). Against `3c69f71`, on macOS, 30 rounds, two
batches: in the dev build the draw, text and parser suites are within noise
(`image-*` 0.989 to 1.006 against main, `3c69f71` 0.988 to 1.006), and with
fat LTO the draw and text suites too (`image-*` 0.996 to 1.006,
`text-kitty` 1.003 / 1.008). `./test.sh`'s outputs are unchanged. Those
probes are not kept.

### Two crates or one compilation unit

#85 asks for `src/lib.rs` as the crate root of everything but the CLI, with
`src/main.rs` a binary crate linking it, or one compilation unit if two
crates cost speed or the link order. The two-crate build was made as a
prototype from the same commit, for measurement only: every module public
in an rlib (the C bundled in it, `-l static=termshot_c`), and main.rs
linking it with `--extern`. It links on macOS and on glibc, and renders the
same bytes. It is the `two` binary in every table here. It is not used:

- With fat LTO on macOS it paints slower, in both batches: `large-color`
  0.974 / 0.977, `color-grid` 0.983 / 0.979, `geometry-all` 0.983 / 0.984,
  `image-under` 0.983 / 0.992, `image-below` 0.984 / 0.990, and four more.
  In the Linux dev build it parses slower: `text-mixed-unicode` 0.925 /
  0.940, `mixed-unicode` 0.959 / 0.947, `cursor-moves` 0.968 / 0.970. The
  one-unit build has none of these.
- The CLI needs the render, the fonts and the kitty image views, which are
  not part of the library until #85's second part. Linking them from an
  rlib would make every one of them public API first.

So `src/main.rs` declares the same modules as `src/lib.rs`, plus the
render's, and compiles them as one crate, as before; `src/lib.rs` is built
on its own as the rlib. Once the CLI is thin (#85 part 2), the two-crate
build is the one to measure again.

### Results

Paired wall speedup of each binary against main (main's wall time / the
binary's, per interleaved round; above 1 is faster), batches A / B. "In
both" means the bootstrap 95% interval lies past 1 in both batches. Every
suite ran: 48 cases, the system Noto CJK collection included. "Every case"
is the range of all 96 medians.

`branch` (`3c69f71`):

| case | macOS dev | macOS fat LTO | Linux dev | Linux fat LTO |
| --- | ---: | ---: | ---: | ---: |
| `ansi-replay` | 1.001 / 1.013 | 0.991 / 1.001 | 0.999 / 0.990 | 0.985 / 1.000 |
| `dense-sgr` | 0.995 / 0.999 | 0.995 / 0.999 | 0.970 / 0.980 | 0.999 / 1.009 |
| `thai-combining` | 1.013 / 1.007 | 1.019 / 1.018 | 1.004 / 1.001 | 1.010 / 1.006 |
| `ascii-overflow` | 1.004 / 1.009 | 0.996 / 0.982 | 1.007 / 0.972 | 0.985 / 1.006 |
| `cursor-moves` | 0.987 / 0.993 | 0.994 / 0.993 | 0.985 / 0.981 | 0.988 / 0.990 |
| `mixed-unicode` | 0.996 / 0.992 | 1.005 / 0.999 | 1.000 / 0.998 | 1.000 / 1.003 |
| `text-dense-sgr` | 0.994 / 0.997 | 1.001 / 0.998 | 0.970 / 0.993 | 1.014 / 1.014 |
| `text-mixed-unicode` | 0.992 / 0.991 | 1.011 / 1.010 | 0.984 / 0.994 | 1.013 / 1.013 |
| `reply-sent` | 1.001 / 1.003 | 0.998 / 0.996 | 0.993 / 0.998 | 0.994 / 0.999 |
| slower in both | `cursor-moves`, `text-mixed-unicode` (above) | none | `geometry-all` 0.991 / 0.986, `dense-sgr`, `cursor-moves` (above) | none |
| faster in both | `scrolling` 1.010 / 1.013, `thai-combining`, `text-ansi-replay` 1.006 / 1.013 | `thai-combining` | none | none |
| every case | 0.985 to 1.019 | 0.982 to 1.020 | 0.944 to 1.048 | 0.961 to 1.047 |

`two` (`3c69f71` as two crates):

| case | macOS dev | macOS fat LTO | Linux dev | Linux fat LTO |
| --- | ---: | ---: | ---: | ---: |
| `ansi-replay` | 0.992 / 0.998 | 1.000 / 1.004 | 0.984 / 0.984 | 0.978 / 0.996 |
| `dense-sgr` | 0.994 / 0.991 | 0.988 / 0.985 | 0.987 / 0.989 | 0.997 / 1.008 |
| `thai-combining` | 1.006 / 1.007 | 1.023 / 1.019 | 0.966 / 0.968 | 1.037 / 1.026 |
| `ascii-overflow` | 1.004 / 1.001 | 0.999 / 0.994 | 0.964 / 0.996 | 0.972 / 1.025 |
| `cursor-moves` | 0.980 / 0.982 | 0.988 / 0.990 | 0.968 / 0.970 | 0.995 / 0.994 |
| `mixed-unicode` | 0.998 / 0.991 | 0.997 / 0.995 | 0.959 / 0.947 | 1.033 / 1.024 |
| `text-dense-sgr` | 0.992 / 0.997 | 1.007 / 1.001 | 0.985 / 0.985 | 0.998 / 0.990 |
| `text-mixed-unicode` | 0.994 / 0.984 | 1.007 / 1.003 | 0.925 / 0.940 | 1.026 / 1.028 |
| `reply-sent` | 1.012 / 1.009 | 1.000 / 1.000 | 1.004 / 0.987 | 0.988 / 1.003 |
| slower in both | `geometry-all` 0.988 / 0.986, `dense-sgr`, `cursor-moves` | `mixed-full` 0.990 / 0.989, `color-grid` 0.983 / 0.979, `dense` 0.989 / 0.987, `geometry-all` 0.983 / 0.984, `large-color` 0.974 / 0.977, `image-below` 0.984 / 0.990, `image-under` 0.983 / 0.992, `dense-sgr`, `cursor-moves` | `geometry-all` 0.985 / 0.985, `dense-sgr`, `cursor-moves`, `mixed-unicode`, `thai-combining`, `text-ansi-replay` 0.964 / 0.963, `text-mixed-unicode` | none |
| faster in both | `thai-combining` | `thai-combining` | none | `geometry-all` 1.019 / 1.007, `mixed-unicode`, `thai-combining`, `text-mixed-unicode` |
| every case | 0.980 to 1.015 | 0.974 to 1.023 | 0.925 to 1.043 | 0.943 to 1.042 |

### What was measured

| | macOS arm64 | Linux x86-64 |
| --- | --- | --- |
| host | `lawrences-mac-studio`, Apple M2 Max, macOS 26.6.2 | `starship`, Ryzen 7 8745HS, governor `performance` |
| built with | rustc 1.98.1 (Homebrew), Apple clang 21.0.0 | rustc 1.98.1 (Arch), GCC 16.2.1 |
| load average (1 min), start → end | dev: A 4.09 → 4.72, B 4.72 → 3.33; fat: A 3.33 → 3.95, B 3.95 → 4.04 | dev: A 1.75 → 1.62, B 1.62 → 1.50; fat: A 1.46 → 1.98, B 1.98 → 2.07 |

The binaries: `main` is main `cd2b11d`, `branch` is `3c69f71`, `two` is
`3c69f71` as two crates, each with `build.sh`'s flags (dev) and with
`RUSTFLAGS='-C lto=fat'` added, as `scripts/release.sh` builds them (fat
LTO). Their sha256 values are in each file's `binary_sha256`.
`scripts/bench.py` ran 40 rounds and 5 warmups per batch, and 5 peak-RSS
runs, with seeds 53 (A) and 67 (B). A first Linux run of this round
overlapped other jobs on starship (load 3.7, then 19) and was run again;
it is not kept.

The files are slim, with no profile stage kept: dev
[A](performance-2026-10-06-lib-parse-dev-macos-a.json) and
[B](performance-2026-10-06-lib-parse-dev-macos-b.json), fat
[A](performance-2026-10-06-lib-parse-fat-macos-a.json) and
[B](performance-2026-10-06-lib-parse-fat-macos-b.json) on macOS; dev
[A](performance-2026-10-06-lib-parse-dev-linux-a.json) and
[B](performance-2026-10-06-lib-parse-dev-linux-b.json), fat
[A](performance-2026-10-06-lib-parse-fat-linux-a.json) and
[B](performance-2026-10-06-lib-parse-fat-linux-b.json) on Linux. The first
round (`7c5fc89`) and the probes are quoted above and not kept.

### Reproduce

```sh
# Each binary as build.sh builds it, and with fat LTO (in a checkout of main
# for main-dev and main-fat):
./build.sh && cp termshot /tmp/branch-dev
RUSTFLAGS='-C lto=fat' ./build.sh && cp termshot /tmp/branch-fat
cjk=/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc   # on macOS, a copy (same sha256)
for flavor in dev fat; do
  for batch in a:53 b:67; do
    python3 scripts/bench.py --binary main=/tmp/main-$flavor --binary branch=/tmp/branch-$flavor \
      --reference main --runs 40 --warmups 5 --memory-runs 5 --verify-identical --cjk-font "$cjk" \
      --seed "${batch#*:}" --output "/tmp/lib-parse-$flavor-${batch%%:*}.json"
  done
done
```

## Release size profile (2026-10-05, `66780fc`, #83)

#83 asks which size options the release archives should be built with, now
that the code is Rust but for `src/stb_glue.c`, `src/image.c` (the PNG
decoder's wrapper) and vendored stb. Release
archives were `build.sh`'s flags (C `-O2`, Rust `-C opt-level=2`) and then
`strip`. `scripts/release.sh` now adds `-C lto=fat` (through `build.sh`'s new
`RUSTFLAGS`) and nothing else: the binary is 5.0-8.7% smaller and the archive
3.4-4.6% smaller on the three release platforms. `reply-sent` is as fast. Two
or three cases are 1.5-3% slower in both batches (the trade-off is below),
and `thai-combining` is 2-6% faster. Every output is the same. Every other
size option was measured and is rejected, `panic=abort` among them.

### Result

| | macOS universal | Linux x86_64 musl | Linux aarch64 musl |
| --- | ---: | ---: | ---: |
| binary, before → after (bytes) | 2,478,016 → 2,262,304 (-8.7%) | 1,409,872 → 1,331,968 (-5.5%) | 1,315,160 → 1,249,528 (-5.0%) |
| archive (`.tar.gz`) | 1,229,023 → 1,172,809 (-4.6%) | 724,589 → 700,294 (-3.4%) | 690,678 → 665,747 (-3.6%) |
| `reply-sent`, paired speedup (before/after), batches A / B | 0.993 / 1.002 (arm64 slice) | 1.010 / 0.995 | not timed |
| slower with confidence in both batches | `text-dense-sgr` 0.974 / 0.975, `cursor-moves` 0.985 / 0.983 | `cursor-moves` 0.972 / 0.982, `rounded-128px` 0.971 / 0.985 | — |
| faster with confidence in both batches | `thai-combining` 1.021 / 1.017, `large-color` 1.006 / 1.006 | `thai-combining` 1.063 / 1.052 | — |

**The trade-off.** The cases that slow down are parser stress logs
(`cursor-moves`: 27.7 → 28.0 ms on macOS and 26.0 → 26.7 ms on Linux;
`text-dense-sgr`: 16.4 → 16.8 ms on macOS) and the 128 px rounded boxes
on Linux (22.4 → 23.0 ms). The common case, `reply-sent`, and every
other case are within noise or faster. That buys 5-9% of the binary. Every
candidate that saves materially more is slower on more cases, or breaks the
exit-code contract (`panic=abort`). `-C strip=symbols` on top saves 1,088
bytes (0.05%) on macOS and nothing on Linux, and is left out (Sizes).

### Sizes

Each candidate was built the way `scripts/release.sh` builds, from `66780fc`
(main): the C objects (`src/stb_glue.c` and `src/image.c`) with `CFLAGS`
added, so `-Os` applies to both, then rustc with flags added. For
macOS, both slices with `MACOSX_DEPLOYMENT_TARGET=11.0`, `lipo`, then
`strip -x`. For Linux, `musl-gcc` and the `*-unknown-linux-musl` target, then
`strip`. The archive is release.sh's `tar -czf` of the binary, licenses,
README and changelog. The binary is the stripped file. Each cell gives the
binary / the archive in bytes, with the change from `base`. Every candidate
rendered `reply-sent` and `draft-ready` byte for byte like the host build
(both macOS slices, x86_64 under Rosetta). macOS was built here; Linux by
`release.yml` itself, through `workflow_dispatch` on a throwaway branch
(runs 37265578364 and 37267462039, Ubuntu 24.04, musl-tools over GCC 13.3.0).

| candidate | flags on top of build.sh | macOS universal | Linux x86_64 musl | Linux aarch64 musl |
| --- | --- | ---: | ---: | ---: |
| `base` | none: the release flags before #83 | 2,478,016 (+0.0%) / 1,229,023 (+0.0%) | 1,409,872 (+0.0%) / 724,589 (+0.0%) | 1,315,160 (+0.0%) / 690,678 (+0.0%) |
| `base-rstrip` | strip=symbols, no external strip | 2,404,000 (-3.0%) / 1,212,089 (-1.4%) | 1,409,872 (+0.0%) / 724,592 (+0.0%) | 1,315,160 (+0.0%) / 690,680 (+0.0%) |
| `base-bstrip` | strip=symbols, then external strip | 2,404,032 (-3.0%) / 1,212,116 (-1.4%) | 1,409,872 (+0.0%) / 724,593 (+0.0%) | 1,315,160 (+0.0%) / 690,682 (+0.0%) |
| `base-abort` | panic=abort | 2,378,624 (-4.0%) / 1,186,977 (-3.4%) | 1,373,040 (-2.6%) / 706,145 (-2.5%) | 1,249,632 (-5.0%) / 673,521 (-2.5%) |
| `cos` | C -Os | 2,445,120 (-1.3%) / 1,208,848 (-1.6%) | 1,389,392 (-1.5%) / 713,747 (-1.5%) | 1,315,144 (-0.0%) / 682,358 (-1.2%) |
| `o2-cgu1` | codegen-units=1 | 2,411,312 (-2.7%) / 1,216,953 (-1.0%) | 1,389,392 (-1.5%) / 717,415 (-1.0%) | 1,315,160 (+0.0%) / 684,283 (-0.9%) |
| `o2-thin` | lto=thin | 2,435,424 (-1.7%) / 1,219,872 (-0.7%) | 1,385,320 (-1.7%) / 721,269 (-0.5%) | 1,249,632 (-5.0%) / 685,015 (-0.8%) |
| `o2-thin-cgu1` | lto=thin codegen-units=1 | 2,385,664 (-3.7%) / 1,208,178 (-1.7%) | 1,368,936 (-2.9%) / 714,545 (-1.4%) | 1,249,632 (-5.0%) / 678,610 (-1.7%) |
| `o2-fat` | lto=fat | 2,262,304 (-8.7%) / 1,172,809 (-4.6%) | 1,331,968 (-5.5%) / 700,294 (-3.4%) | 1,249,528 (-5.0%) / 665,747 (-3.6%) |
| `o2-fat-rstrip` | lto=fat strip=symbols, no external strip | 2,261,216 (-8.7%) / 1,172,193 (-4.6%) | 1,331,968 (-5.5%) / 700,297 (-3.4%) | 1,249,528 (-5.0%) / 665,744 (-3.6%) |
| `cos-o2-fat` | C -Os, lto=fat | 2,229,424 (-10.0%) / 1,152,415 (-6.2%) | 1,307,392 (-7.3%) / 688,661 (-5.0%) | 1,183,976 (-10.0%) / 655,488 (-5.1%) |
| `o2-fat-abort` | lto=fat panic=abort | 2,163,312 (-12.7%) / 1,139,744 (-7.3%) | 1,307,392 (-7.3%) / 686,992 (-5.2%) | 1,183,992 (-10.0%) / 652,070 (-5.6%) |
| `o2-fat-cgu1` | lto=fat codegen-units=1 | 2,245,824 (-9.4%) / 1,166,739 (-5.1%) | 1,327,872 (-5.8%) / 694,835 (-4.1%) | 1,249,528 (-5.0%) / 662,654 (-4.1%) |
| `o2-fat-cgu1-rstrip` | lto=fat codegen-units=1 strip=symbols, no external strip | 2,244,672 (-9.4%) / 1,166,040 (-5.1%) | 1,327,872 (-5.8%) / 694,838 (-4.1%) | 1,249,528 (-5.0%) / 662,652 (-4.1%) |
| `o2-fat-cgu1-bstrip` | lto=fat codegen-units=1 strip=symbols, then external strip | 2,244,704 (-9.4%) / 1,166,057 (-5.1%) | 1,327,872 (-5.8%) / 694,838 (-4.1%) | 1,249,528 (-5.0%) / 662,653 (-4.1%) |
| `cos-o2-fat-cgu1` | C -Os, lto=fat codegen-units=1 | 2,212,944 (-10.7%) / 1,146,031 (-6.8%) | 1,303,296 (-7.6%) / 684,259 (-5.6%) | 1,183,976 (-10.0%) / 652,755 (-5.5%) |
| `o2-fat-cgu1-abort` | lto=fat codegen-units=1 panic=abort | 2,163,280 (-12.7%) / 1,133,793 (-7.7%) | 1,295,104 (-8.1%) / 680,297 (-6.1%) | 1,183,992 (-10.0%) / 649,247 (-6.0%) |
| `os` | opt s | 2,380,752 (-3.9%) / 1,181,200 (-3.9%) | 1,352,528 (-4.1%) / 698,773 (-3.6%) | 1,249,624 (-5.0%) / 665,194 (-3.7%) |
| `os-cgu1` | opt s codegen-units=1 | 2,330,080 (-6.0%) / 1,165,864 (-5.1%) | 1,327,952 (-5.8%) / 690,878 (-4.7%) | 1,249,624 (-5.0%) / 657,000 (-4.9%) |
| `os-thin` | opt s lto=thin | 2,291,120 (-7.5%) / 1,148,677 (-6.5%) | 1,311,592 (-7.0%) / 683,851 (-5.6%) | 1,249,632 (-5.0%) / 651,041 (-5.7%) |
| `os-fat` | opt s lto=fat | 2,065,088 (-16.7%) / 1,092,243 (-11.1%) | 1,237,760 (-12.2%) / 658,407 (-9.1%) | 1,183,992 (-10.0%) / 627,796 (-9.1%) |
| `os-fat-cgu1` | opt s lto=fat codegen-units=1 | 2,065,024 (-16.7%) / 1,088,044 (-11.5%) | 1,237,760 (-12.2%) / 656,103 (-9.5%) | 1,183,992 (-10.0%) / 625,139 (-9.5%) |
| `cos-os-fat-cgu1` | C -Os, opt s lto=fat codegen-units=1 | 2,032,144 (-18.0%) / 1,067,397 (-13.2%) | 1,213,184 (-14.0%) / 644,266 (-11.1%) | 1,118,440 (-15.0%) / 615,649 (-10.9%) |
| `os-fat-cgu1-abort` | opt s lto=fat codegen-units=1 panic=abort | 1,982,480 (-20.0%) / 1,054,665 (-14.2%) | 1,204,992 (-14.5%) / 640,984 (-11.5%) | 1,118,456 (-15.0%) / 612,217 (-11.4%) |
| `oz` | opt z | 2,316,272 (-6.5%) / 1,156,310 (-5.9%) | 1,352,528 (-4.1%) / 693,934 (-4.2%) | 1,249,624 (-5.0%) / 667,300 (-3.4%) |
| `oz-cgu1` | opt z codegen-units=1 | 2,249,376 (-9.2%) / 1,137,222 (-7.5%) | 1,315,664 (-6.7%) / 682,369 (-5.8%) | 1,249,624 (-5.0%) / 656,384 (-5.0%) |
| `oz-thin` | opt z lto=thin | 2,228,704 (-10.1%) / 1,131,702 (-7.9%) | 1,315,688 (-6.7%) / 681,182 (-6.0%) | 1,249,632 (-5.0%) / 656,868 (-4.9%) |
| `oz-fat` | opt z lto=fat | 2,000,560 (-19.3%) / 1,063,991 (-13.4%) | 1,225,472 (-13.1%) / 649,126 (-10.4%) | 1,118,456 (-15.0%) / 627,649 (-9.1%) |
| `oz-fat-cgu1` | opt z lto=fat codegen-units=1 | 1,983,936 (-19.9%) / 1,056,073 (-14.1%) | 1,213,184 (-14.0%) / 644,494 (-11.1%) | 1,118,456 (-15.0%) / 623,387 (-9.7%) |
| `cos-oz-fat-cgu1` | C -Os, opt z lto=fat codegen-units=1 | 1,951,056 (-21.3%) / 1,035,599 (-15.7%) | 1,192,704 (-15.4%) / 633,615 (-12.6%) | 1,118,440 (-15.0%) / 613,822 (-11.1%) |

How to read it:

- **aarch64 sizes move in steps of about 64 KiB** (65,536 between `o2-fat`
  and `o2-fat-abort`, 65,632 between `base` and `o2-fat`): the segments are
  aligned to 64 KiB pages. Use the archive size there.
- **The archive is noisier than the binary** by about 20 bytes: tar records
  the files' times. `base`, built twice on macOS, gave 1,229,004 and
  1,229,023.
- **`-C strip=symbols` changes nothing on Linux** (the same binary bytes).
  On macOS universal it saves 3.0% without LTO, but only 1,088 bytes
  (0.05%) with fat LTO. Not adopted; the external `strip` stays.
- The same flags built on starship (Debian bookworm's musl-gcc over GCC
  12.2.0, the binaries benchmarked below) gave `base` and `o2-fat` 8 bytes
  smaller than CI's, and a few others one 4 KiB step apart
  (`performance-2026-10-05-release-sizes.json`). There, release.sh's
  `linux-x86_64-musl` build from the branch is byte-identical to the
  `o2-fat` binary timed (sha256 `b0aa617e6149…`).

### Speed

`scripts/bench.py`, every suite, 40 shuffled rounds of plain and profiled
runs per case and binary, 5 warmups, 5 peak-RSS runs, `--verify-identical`
(every binary gave the same PNG or text for every case), the full CJK
collection (sha256 `b76b0433…`; a copy on macOS), `base` as the reference.
Two rounds, each of two batches. On macOS, the arm64 slice of each
universal binary (`lipo -thin`); on Linux, the static musl binaries, run on
the host (glibc's allocator and libm play no part). A speedup is
before/after, so below 1 is slower; bold where both batches' 95% bootstrap
intervals exclude 1.

**Round 1** (seeds 17, 29): six binaries, `base` and the five
LTO-plus-one-codegen-unit candidates. Paired wall speedup, batch A / B:

macOS arm64:

| case | base wall A / B (ms) | `o2-fat-cgu1` | `cos-o2-fat-cgu1` | `os-fat-cgu1` | `oz-fat-cgu1` | `cos-oz-fat-cgu1` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 8.97 / 8.88 | 1.001 / 0.998 | 1.000 / 0.993 | **0.713 / 0.697** | **0.527 / 0.520** | **0.528 / 0.525** |
| `font-file` | 8.85 / 8.89 | 0.984 / 1.002 | 0.985 / 1.002 | **0.705 / 0.701** | **0.526 / 0.523** | **0.524 / 0.525** |
| `cjk-none` | 5.08 / 4.99 | 1.013 / 0.994 | 1.008 / 0.989 | **0.854 / 0.839** | **0.656 / 0.646** | **0.655 / 0.640** |
| `cjk-subset` | 9.55 / 9.64 | 0.992 / 0.996 | 0.990 / 1.002 | **0.891 / 0.893** | **0.637 / 0.640** | **0.639 / 0.642** |
| `cjk-cff-primary` | 9.36 / 9.20 | 1.000 / 1.004 | 0.994 / 0.994 | **0.879 / 0.876** | **0.642 / 0.636** | **0.644 / 0.643** |
| `mixed-subset` | 7.71 / 7.64 | 0.998 / 1.000 | 1.004 / 0.997 | **0.888 / 0.882** | **0.678 / 0.672** | **0.676 / 0.672** |
| `glyph-overflow` | 16.50 / 16.41 | 0.990 / 1.000 | **0.990 / 0.992** | **0.922 / 0.925** | **0.719 / 0.720** | **0.720 / 0.716** |
| `cjk-full` | 12.67 / 12.57 | 0.997 / 0.995 | 0.992 / 0.992 | **0.905 / 0.904** | **0.692 / 0.689** | **0.691 / 0.698** |
| `mixed-full` | 11.17 / 11.05 | 0.990 / 0.999 | 0.986 / 1.008 | **0.911 / 0.920** | **0.732 / 0.739** | **0.735 / 0.737** |
| `cjk-overflow-full` | 27.43 / 26.75 | 0.995 / 0.994 | 0.999 / 0.986 | **0.942 / 0.943** | **0.750 / 0.743** | **0.745 / 0.743** |
| `reply-sent` | 9.00 / 8.97 | 1.000 / 1.012 | 1.004 / 1.001 | **0.714 / 0.707** | **0.525 / 0.519** | **0.530 / 0.526** |
| `draft-ready` | 8.76 / 8.85 | 0.986 / 1.004 | 1.006 / 1.006 | **0.708 / 0.711** | **0.522 / 0.522** | **0.524 / 0.526** |
| `reply-24px` | 5.68 / 5.74 | 0.994 / 1.006 | 0.983 / 0.996 | **0.851 / 0.873** | **0.678 / 0.686** | **0.681 / 0.684** |
| `reply-128px` | 29.03 / 28.51 | 1.008 / 1.005 | 0.998 / 0.999 | **0.535 / 0.532** | **0.376 / 0.372** | **0.377 / 0.372** |
| `real-shell` | 6.34 / 6.44 | 0.997 / 1.007 | 1.003 / 1.001 | **0.733 / 0.727** | **0.560 / 0.564** | **0.563 / 0.562** |
| `real-less` | 5.94 / 5.98 | 1.002 / 1.012 | 1.020 / 1.019 | **0.733 / 0.730** | **0.558 / 0.561** | **0.554 / 0.559** |
| `real-vi` | 7.90 / 7.98 | 0.992 / 1.012 | 0.990 / 1.015 | **0.762 / 0.775** | **0.575 / 0.585** | **0.579 / 0.583** |
| `blank` | 6.05 / 5.94 | 1.023 / 0.995 | 1.026 / 1.000 | **0.641 / 0.621** | **0.486 / 0.471** | **0.479 / 0.468** |
| `color-grid` | 17.84 / 17.96 | 1.005 / 0.999 | 1.003 / 1.004 | **0.937 / 0.935** | **0.666 / 0.666** | **0.666 / 0.667** |
| `ascii-overflow` | 5.91 / 5.88 | 0.997 / 0.996 | 0.992 / 0.993 | **0.849 / 0.837** | **0.703 / 0.704** | **0.705 / 0.704** |
| `rounded-boxes` | 10.12 / 10.21 | 1.001 / 1.000 | 0.998 / 0.991 | **0.718 / 0.729** | **0.468 / 0.465** | **0.465 / 0.466** |
| `dense` | 11.93 / 12.00 | 0.996 / 1.005 | 0.997 / 1.005 | **0.744 / 0.752** | **0.538 / 0.541** | **0.538 / 0.545** |
| `ansi-replay` | 18.62 / 18.46 | 1.008 / 0.996 | 1.003 / 1.000 | **0.777 / 0.771** | **0.575 / 0.575** | **0.579 / 0.574** |
| `large` | 37.12 / 37.37 | 1.002 / 0.991 | 1.004 / 0.998 | **0.596 / 0.596** | **0.415 / 0.416** | **0.417 / 0.417** |
| `unicode` | 7.96 / 7.85 | 1.005 / 1.007 | 0.994 / 1.000 | **0.893 / 0.890** | **0.684 / 0.676** | **0.683 / 0.678** |
| `box-grid` | 7.52 / 7.32 | 0.992 / 0.991 | 1.010 / 0.988 | **0.658 / 0.659** | **0.454 / 0.451** | **0.454 / 0.448** |
| `block-grid` | 10.31 / 10.38 | 1.002 / 0.988 | 0.985 / 0.999 | **0.692 / 0.701** | **0.505 / 0.507** | **0.508 / 0.509** |
| `rounded-panes` | 9.70 / 9.60 | 0.996 / 1.003 | 1.001 / 0.994 | **0.705 / 0.705** | **0.505 / 0.502** | **0.510 / 0.503** |
| `rounded-24px` | 5.71 / 5.69 | 0.996 / 0.995 | 1.002 / 0.997 | **0.863 / 0.857** | **0.626 / 0.624** | **0.618 / 0.623** |
| `rounded-128px` | 32.29 / 32.06 | 1.011 / 1.006 | 1.009 / 1.002 | **0.553 / 0.550** | **0.349 / 0.345** | **0.347 / 0.345** |
| `geometry-all` | 90.26 / 90.34 | **0.987 / 0.984** | **0.988 / 0.987** | **0.786 / 0.786** | **0.445 / 0.445** | **0.445 / 0.445** |
| `large-sparse` | 20.23 / 20.58 | 1.004 / 1.002 | 1.006 / 0.994 | **0.477 / 0.481** | **0.336 / 0.336** | **0.338 / 0.338** |
| `large-color` | 144.22 / 145.40 | 1.002 / 1.001 | 1.002 / 1.001 | **0.839 / 0.839** | **0.560 / 0.562** | **0.561 / 0.563** |
| `image-below` | 21.61 / 21.86 | 1.005 / 0.996 | 1.006 / 1.000 | **0.824 / 0.824** | **0.595 / 0.597** | **0.597 / 0.593** |
| `image-under` | 25.17 / 25.35 | 1.007 / 1.001 | 1.010 / 0.999 | **0.845 / 0.847** | **0.611 / 0.608** | **0.610 / 0.607** |
| `image-over` | 19.77 / 20.19 | 0.999 / 1.001 | 0.998 / 0.999 | **0.813 / 0.813** | **0.588 / 0.588** | **0.583 / 0.586** |
| `dense-sgr` | 31.26 / 31.61 | **0.978 / 0.982** | **0.978 / 0.984** | **0.891 / 0.888** | **0.653 / 0.654** | **0.656 / 0.657** |
| `cursor-moves` | 27.47 / 27.83 | **0.961 / 0.965** | **0.959 / 0.969** | **0.901 / 0.909** | **0.595 / 0.598** | **0.597 / 0.600** |
| `scrolling` | 9.63 / 9.64 | 0.990 / 0.995 | 0.998 / 1.001 | **0.879 / 0.878** | **0.646 / 0.644** | **0.650 / 0.647** |
| `mixed-unicode` | 29.13 / 29.36 | **0.950 / 0.950** | **0.957 / 0.955** | **0.896 / 0.898** | **0.461 / 0.462** | **0.462 / 0.462** |
| `thai-combining` | 23.35 / 23.49 | **0.953 / 0.951** | **0.952 / 0.947** | **0.840 / 0.839** | **0.445 / 0.444** | **0.446 / 0.447** |
| `text-ansi-replay` | 13.47 / 13.43 | 0.995 / 0.996 | 0.998 / 0.995 | **0.891 / 0.885** | **0.641 / 0.638** | **0.636 / 0.637** |
| `text-ascii-overflow` | 4.86 / 4.88 | 0.988 / 1.019 | 0.991 / 1.005 | 0.977 / 1.017 | **0.734 / 0.743** | **0.737 / 0.747** |
| `text-dense-sgr` | 16.47 / 16.38 | **0.971 / 0.976** | **0.969 / 0.968** | **0.877 / 0.872** | **0.663 / 0.663** | **0.661 / 0.664** |
| `text-mixed-unicode` | 26.36 / 26.19 | **0.944 / 0.948** | **0.953 / 0.951** | **0.918 / 0.918** | **0.449 / 0.447** | **0.449 / 0.446** |
| `text-reply-sent` | 3.09 / 3.05 | 0.986 / 1.020 | 0.988 / 1.014 | 1.015 / 1.021 | 0.998 / 1.011 | 0.983 / 0.985 |
| `text-kitty` | 3.75 / 3.76 | 1.013 / 1.015 | 0.997 / 1.022 | 0.970 / 0.978 | **0.889 / 0.883** | **0.870 / 0.897** |
| `text-sixel` | 3.40 / 3.26 | 1.003 / 0.993 | 1.024 / 0.986 | 1.004 / 0.980 | **0.896 / 0.877** | **0.897 / 0.887** |

Linux x86-64:

| case | base wall A / B (ms) | `o2-fat-cgu1` | `cos-o2-fat-cgu1` | `os-fat-cgu1` | `oz-fat-cgu1` | `cos-oz-fat-cgu1` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 7.92 / 8.26 | 0.990 / 1.026 | 0.983 / 0.959 | 0.964 / 0.987 | **0.732 / 0.732** | **0.716 / 0.734** |
| `font-file` | 8.14 / 8.22 | 1.011 / 1.009 | 1.005 / 1.019 | 0.977 / 0.986 | **0.743 / 0.733** | **0.724 / 0.727** |
| `cjk-none` | 4.13 / 4.20 | 1.074 / 0.979 | 1.050 / 1.026 | 1.008 / 0.978 | **0.783 / 0.772** | **0.787 / 0.765** |
| `cjk-subset` | 9.82 / 9.70 | 1.021 / 1.000 | 1.014 / 0.992 | **0.960 / 0.947** | **0.737 / 0.717** | **0.724 / 0.712** |
| `cjk-cff-primary` | 8.85 / 9.00 | 0.980 / 0.959 | 0.997 / 1.025 | 0.947 / 0.978 | **0.721 / 0.698** | **0.720 / 0.713** |
| `mixed-subset` | 8.93 / 9.23 | 1.008 / 0.993 | 1.016 / 1.007 | 0.962 / 0.990 | **0.788 / 0.816** | **0.792 / 0.814** |
| `glyph-overflow` | 21.16 / 21.59 | 0.998 / 0.985 | 0.982 / 0.975 | **0.970 / 0.973** | **0.818 / 0.829** | **0.814 / 0.816** |
| `cjk-full` | 13.54 / 13.40 | 0.981 / 0.993 | 0.978 / 0.998 | 0.960 / 0.971 | **0.790 / 0.781** | **0.777 / 0.778** |
| `mixed-full` | 13.16 / 13.50 | 0.999 / 1.017 | 0.992 / 1.012 | 0.985 / 1.004 | **0.847 / 0.852** | **0.851 / 0.861** |
| `cjk-overflow-full` | 32.49 / 32.76 | 0.998 / 0.992 | **0.975 / 0.976** | **0.978 / 0.963** | **0.840 / 0.830** | **0.830 / 0.810** |
| `reply-sent` | 8.19 / 8.34 | 1.034 / 1.023 | 1.027 / 1.012 | 1.010 / 0.959 | **0.741 / 0.732** | **0.741 / 0.710** |
| `draft-ready` | 7.86 / 8.97 | 1.016 / 0.981 | 1.033 / 0.997 | 0.987 / 0.979 | **0.746 / 0.702** | **0.741 / 0.687** |
| `reply-24px` | 5.14 / 8.57 | 0.991 / 1.015 | 0.997 / 1.004 | 0.970 / 0.992 | **0.790 / 0.773** | **0.797 / 0.766** |
| `reply-128px` | 23.35 / 47.78 | 0.993 / 0.994 | 0.997 / 0.997 | **0.966 / 0.979** | **0.630 / 0.742** | **0.624 / 0.735** |
| `real-shell` | 5.40 / 5.43 | 1.008 / 1.011 | 0.991 / 0.982 | 0.994 / 0.982 | **0.748 / 0.749** | **0.757 / 0.738** |
| `real-less` | 4.70 / 4.87 | 1.002 / 0.996 | 0.989 / 0.982 | 0.989 / 0.993 | **0.738 / 0.766** | **0.774 / 0.753** |
| `real-vi` | 7.05 / 7.43 | 0.984 / 1.005 | 1.007 / 1.013 | 0.969 / 0.981 | **0.728 / 0.748** | **0.729 / 0.732** |
| `blank` | 3.99 / 4.07 | 0.989 / 0.969 | 0.955 / 0.968 | 0.990 / 1.000 | **0.698 / 0.646** | **0.713 / 0.678** |
| `color-grid` | 16.46 / 17.13 | 0.986 / 0.987 | 0.999 / 0.986 | **0.952 / 0.952** | **0.683 / 0.709** | **0.684 / 0.705** |
| `ascii-overflow` | 5.73 / 5.76 | 0.988 / 1.025 | 1.021 / 0.999 | 0.982 / 0.966 | **0.861 / 0.829** | **0.849 / 0.811** |
| `rounded-boxes` | 8.00 / 7.88 | 0.975 / 0.984 | 0.984 / 0.983 | 0.910 / 0.962 | **0.597 / 0.593** | **0.595 / 0.585** |
| `dense` | 10.39 / 10.54 | 0.979 / 1.022 | 0.972 / 0.998 | 0.980 / 0.929 | **0.677 / 0.680** | **0.676 / 0.670** |
| `ansi-replay` | 16.81 / 16.91 | 0.992 / 0.979 | 0.982 / 0.984 | **0.901 / 0.903** | **0.718 / 0.703** | **0.718 / 0.699** |
| `large` | 30.95 / 31.52 | 1.001 / 0.996 | 0.984 / 0.996 | **0.959 / 0.960** | **0.617 / 0.616** | **0.612 / 0.617** |
| `unicode` | 9.18 / 9.13 | 1.010 / 0.987 | 0.961 / 0.975 | 0.973 / 0.987 | **0.792 / 0.798** | **0.786 / 0.765** |
| `box-grid` | 5.94 / 5.93 | 1.000 / 1.028 | 0.985 / 1.026 | 0.964 / 0.975 | **0.636 / 0.649** | **0.634 / 0.633** |
| `block-grid` | 9.22 / 9.10 | 1.005 / 0.997 | 1.019 / 1.001 | **0.929 / 0.902** | **0.660 / 0.648** | **0.668 / 0.644** |
| `rounded-panes` | 8.44 / 9.03 | 1.008 / 0.996 | 1.004 / 0.972 | 0.945 / 0.971 | **0.681 / 0.695** | **0.698 / 0.691** |
| `rounded-24px` | 4.39 / 4.63 | 1.034 / 1.055 | 1.003 / 1.036 | 0.966 / 1.040 | **0.724 / 0.736** | **0.720 / 0.724** |
| `rounded-128px` | 22.88 / 45.80 | 0.987 / 1.008 | 0.982 / 0.986 | **0.929 / 0.966** | **0.516 / 0.667** | **0.520 / 0.680** |
| `geometry-all` | 77.71 / 79.04 | 1.000 / 0.995 | 1.000 / 0.996 | **0.957 / 0.952** | **0.525 / 0.521** | **0.520 / 0.523** |
| `large-sparse` | 14.83 / 14.75 | 0.979 / 0.981 | 0.998 / 0.981 | 0.990 / 1.001 | **0.615 / 0.612** | **0.618 / 0.616** |
| `large-color` | 127.17 / 129.31 | 0.995 / 0.994 | **0.993 / 0.994** | **0.945 / 0.949** | **0.631 / 0.633** | **0.623 / 0.631** |
| `image-below` | 20.18 / 19.91 | 1.002 / 1.007 | 0.990 / 1.015 | **0.962 / 0.961** | **0.694 / 0.687** | **0.680 / 0.692** |
| `image-under` | 23.06 / 22.82 | 0.991 / 0.988 | 0.993 / 0.979 | **0.956 / 0.949** | **0.691 / 0.686** | **0.685 / 0.687** |
| `image-over` | 18.90 / 18.49 | 0.983 / 0.994 | 0.995 / 1.005 | **0.963 / 0.957** | **0.689 / 0.682** | **0.698 / 0.695** |
| `dense-sgr` | 29.54 / 28.76 | **0.971 / 0.964** | **0.970 / 0.973** | **0.914 / 0.918** | **0.715 / 0.705** | **0.708 / 0.704** |
| `cursor-moves` | 26.23 / 25.51 | **0.925 / 0.923** | **0.924 / 0.929** | **0.932 / 0.926** | **0.663 / 0.664** | **0.656 / 0.664** |
| `scrolling` | 10.56 / 10.42 | 0.990 / 0.986 | 0.981 / 0.984 | **0.909 / 0.911** | **0.781 / 0.774** | **0.782 / 0.775** |
| `mixed-unicode` | 30.03 / 28.65 | **0.938 / 0.943** | **0.944 / 0.934** | **0.924 / 0.927** | **0.601 / 0.599** | **0.602 / 0.602** |
| `thai-combining` | 24.03 / 23.08 | 0.992 / 1.003 | 0.987 / 1.001 | **0.959 / 0.958** | **0.625 / 0.617** | **0.620 / 0.621** |
| `text-ansi-replay` | 10.59 / 10.31 | **0.961 / 0.965** | 0.947 / 0.976 | **0.870 / 0.879** | **0.657 / 0.660** | **0.654 / 0.669** |
| `text-ascii-overflow` | 3.47 / 3.23 | 0.994 / 0.997 | 1.061 / 0.979 | 1.008 / 0.963 | **0.677 / 0.676** | **0.701 / 0.662** |
| `text-dense-sgr` | 13.64 / 13.27 | **0.955 / 0.952** | **0.951 / 0.961** | **0.896 / 0.897** | **0.699 / 0.696** | **0.692 / 0.701** |
| `text-mixed-unicode` | 25.38 / 24.26 | **0.932 / 0.932** | **0.933 / 0.930** | **0.919 / 0.914** | **0.567 / 0.566** | **0.564 / 0.563** |
| `text-reply-sent` | 0.49 / 0.49 | 0.993 / 1.018 | 1.024 / 1.053 | 1.021 / 1.024 | **0.976 / 0.941** | 0.986 / 0.947 |
| `text-kitty` | 2.00 / 2.07 | 0.978 / 0.999 | 0.973 / 0.999 | 0.944 / 0.982 | **0.809 / 0.839** | **0.775 / 0.815** |
| `text-sixel` | 1.41 / 1.39 | 1.035 / 1.015 | 0.994 / 0.941 | 1.041 / 0.959 | **0.831 / 0.859** | **0.860 / 0.819** |

`opt-level` s and z are out on both hosts. With s, `reply-sent` takes 1.4x
as long on macOS (0.714 / 0.707) and the large renders up to 2.1x
(`large-sparse` 0.477). With z, every PNG case takes 1.2-3x as long. C
`-Os` costs at most 1% on macOS (`glyph-overflow` 0.990 / 0.992) but 2.5%
on Linux's CJK glyph renders (`cjk-overflow-full` 0.975 / 0.976, against
`o2-fat-cgu1`'s 0.998 / 0.992).
`o2-fat-cgu1` itself slows the parser-bound cases by 6-8% on both hosts
(`text-mixed-unicode` 0.944 / 0.948 and 0.932 / 0.932). The Linux batch B
of round 1 had an outside load for a while: `reply-128px` and
`rounded-128px` medians doubled for every binary alike, which the paired
ratios cancel.

**Which flag slows the parser**: a probe on Linux (seed 41, one batch, 12
cases, the same method without peak-RSS runs) with each flag alone:

| case | `base` wall median / p95 (ms) | `o2-thin` | `o2-cgu1` | `o2-fat` | `o2-fat-cgu1` | `cos` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `glyph-overflow` | 21.28 / 21.96 | 1.004 [0.989, 1.017] | 0.996 [0.974, 1.012] | 1.003 [0.992, 1.011] | 1.017 [0.991, 1.035] | **0.980 [0.970, 0.990]** |
| `cjk-overflow-full` | 32.76 / 35.10 | 0.993 [0.979, 1.002] | **0.985 [0.967, 0.999]** | 1.009 [0.977, 1.016] | 1.000 [0.994, 1.022] | **0.972 [0.952, 0.985]** |
| `reply-sent` | 7.86 / 9.35 | 0.984 [0.939, 1.037] | **0.956 [0.938, 0.997]** | 0.974 [0.940, 1.043] | 0.968 [0.931, 1.017] | 0.960 [0.930, 1.010] |
| `unicode` | 9.11 / 9.64 | **0.972 [0.954, 0.986]** | 0.987 [0.960, 1.014] | 0.958 [0.945, 1.004] | 1.007 [0.976, 1.028] | 0.984 [0.945, 1.016] |
| `geometry-all` | 75.24 / 76.55 | 0.999 [0.995, 1.003] | **0.987 [0.984, 0.991]** | 1.002 [0.994, 1.007] | **0.994 [0.991, 0.999]** | 1.004 [0.996, 1.007] |
| `large-color` | 123.79 / 125.97 | **0.996 [0.990, 1.000]** | **0.989 [0.986, 0.993]** | **1.010 [1.005, 1.015]** | 1.001 [0.993, 1.005] | 0.994 [0.989, 1.005] |
| `dense-sgr` | 28.82 / 30.11 | **0.988 [0.981, 0.998]** | **0.975 [0.963, 0.987]** | 0.989 [0.969, 1.003] | **0.974 [0.964, 0.986]** | 1.001 [0.991, 1.018] |
| `cursor-moves` | 25.34 / 26.63 | **0.977 [0.968, 0.983]** | **0.911 [0.896, 0.930]** | **0.978 [0.960, 0.994]** | **0.904 [0.895, 0.930]** | 0.995 [0.973, 1.009] |
| `mixed-unicode` | 29.23 / 30.09 | **0.956 [0.948, 0.966]** | **0.923 [0.909, 0.929]** | 1.001 [0.991, 1.014] | **0.940 [0.931, 0.949]** | 1.016 [0.994, 1.024] |
| `text-ansi-replay` | 10.36 / 10.97 | 0.976 [0.962, 1.001] | 0.991 [0.953, 1.010] | 1.001 [0.984, 1.019] | **0.962 [0.942, 0.989]** | 0.998 [0.980, 1.025] |
| `text-dense-sgr` | 13.38 / 14.05 | 1.003 [0.985, 1.020] | **0.967 [0.942, 0.980]** | 0.991 [0.976, 1.003] | **0.953 [0.936, 0.966]** | 0.984 [0.971, 1.013] |
| `text-mixed-unicode` | 24.61 / 26.20 | **0.945 [0.935, 0.961]** | **0.905 [0.894, 0.920]** | 0.986 [0.979, 1.004] | **0.932 [0.917, 0.935]** | 1.010 [0.999, 1.021] |

`-C codegen-units=1` is the cause (`cursor-moves` 0.911, `text-mixed-unicode`
0.905), and `lto=thin` costs up to 5.5%. Fat LTO with the default 16 codegen
units is within noise on 10 of the 12 cases (`cursor-moves` 0.978,
`large-color` 1.010). Round 2 timed it on every case.

**Round 2** (seeds 53, 67): `base`, `o2-fat` (the choice) and `cos-o2-fat`
(the same with C `-Os`). Wall median / p95 (ms) of `base` and `o2-fat` in
each batch, `o2-fat`'s paired speedup with its interval, and `cos-o2-fat`'s
speedup, batch A / B. The case is bold where `o2-fat` moved with confidence
in both batches.

macOS arm64:

| case | `base` A | `o2-fat` A | `base` B | `o2-fat` B | `o2-fat` speedup A | `o2-fat` speedup B | `cos-o2-fat` A / B |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 8.75 / 9.17 | 8.64 / 9.20 | 8.62 / 9.23 | 8.71 / 8.88 | 1.003 [0.980, 1.021] | 0.997 [0.986, 1.014] | 1.007 / 0.994 |
| `font-file` | 8.82 / 9.37 | 8.79 / 9.32 | 8.87 / 9.19 | 8.83 / 9.22 | 0.997 [0.983, 1.009] | 0.995 [0.982, 1.009] | 0.996 / 1.011 |
| `cjk-none` | 4.96 / 5.19 | 4.96 / 5.31 | 5.00 / 5.19 | 5.00 / 5.25 | 1.005 [0.984, 1.018] | 1.008 [0.983, 1.021] | 0.998 / 0.996 |
| `cjk-subset` | 9.45 / 10.03 | 9.50 / 10.04 | 9.51 / 10.06 | 9.52 / 10.06 | 1.002 [0.987, 1.015] | 0.998 [0.975, 1.014] | 1.000 / 1.015 |
| `cjk-cff-primary` | 9.21 / 9.81 | 9.12 / 9.69 | 9.26 / 9.87 | 9.19 / 9.80 | 1.003 [0.994, 1.017] | 1.011 [0.998, 1.020] | 0.998 / 1.011 |
| `mixed-subset` | 7.64 / 7.94 | 7.63 / 7.97 | 7.54 / 7.88 | 7.64 / 8.00 | 0.990 [0.983, 1.006] | 0.990 [0.978, 1.009] | 0.989 / 0.990 |
| `glyph-overflow` | 16.12 / 16.55 | 16.13 / 16.57 | 16.36 / 16.86 | 16.24 / 16.90 | 0.998 [0.988, 1.010] | 1.009 [1.002, 1.015] | 0.995 / 0.997 |
| `cjk-full` | 12.52 / 13.14 | 12.63 / 12.96 | 12.82 / 13.46 | 12.74 / 13.17 | 0.995 [0.987, 1.001] | 0.999 [0.996, 1.012] | 0.993 / 1.007 |
| `mixed-full` | 11.20 / 11.89 | 11.26 / 11.76 | 11.40 / 12.43 | 11.42 / 12.91 | 0.993 [0.986, 1.012] | 0.993 [0.981, 1.008] | 1.000 / 0.987 |
| `cjk-overflow-full` | 26.91 / 27.67 | 26.77 / 28.11 | 26.90 / 28.29 | 26.84 / 27.71 | 1.002 [0.992, 1.019] | 1.002 [0.996, 1.009] | 1.005 / 1.002 |
| `reply-sent` | 8.82 / 9.42 | 8.84 / 9.30 | 8.83 / 9.20 | 8.82 / 9.45 | 0.993 [0.987, 1.005] | 1.002 [0.983, 1.014] | 1.005 / 0.991 |
| `draft-ready` | 8.83 / 9.62 | 8.89 / 9.37 | 8.61 / 9.02 | 8.65 / 9.06 | 1.001 [0.980, 1.015] | 1.002 [0.982, 1.008] | 0.991 / 1.002 |
| `reply-24px` | 5.73 / 5.96 | 5.70 / 6.04 | 5.64 / 5.94 | 5.69 / 6.17 | 0.992 [0.975, 1.024] | 0.997 [0.980, 1.017] | 1.003 / 1.000 |
| `reply-128px` | 28.89 / 29.62 | 28.74 / 30.14 | 28.70 / 43.02 | 28.64 / 33.06 | 1.007 [0.993, 1.012] | 1.003 [0.983, 1.012] | 1.000 / 0.992 |
| `real-shell` | 6.39 / 6.74 | 6.33 / 6.61 | 6.22 / 6.74 | 6.24 / 6.75 | 1.011 [0.998, 1.039] | 1.006 [0.984, 1.022] | 1.003 / 1.005 |
| `real-less` | 5.90 / 6.30 | 5.87 / 6.28 | 5.72 / 6.01 | 5.69 / 6.00 | 1.002 [0.987, 1.026] | 1.010 [0.981, 1.021] | 0.995 / 1.007 |
| `real-vi` | 7.89 / 8.41 | 7.94 / 8.16 | 7.61 / 8.02 | 7.64 / 7.88 | 1.002 [0.994, 1.013] | 0.996 [0.991, 1.006] | 1.007 / 0.999 |
| `blank` | 5.97 / 6.51 | 5.88 / 6.28 | 5.76 / 6.07 | 5.75 / 6.26 | 1.018 [0.991, 1.030] | 0.999 [0.988, 1.026] | 1.000 / 1.012 |
| `color-grid` | 17.87 / 19.28 | 17.85 / 20.55 | 17.58 / 18.16 | 17.61 / 18.24 | 1.002 [0.994, 1.011] | 1.000 [0.993, 1.009] | 1.006 / 0.998 |
| `ascii-overflow` | 5.87 / 6.36 | 5.87 / 6.33 | 5.73 / 6.08 | 5.70 / 5.95 | 0.996 [0.979, 1.013] | 1.007 [0.997, 1.023] | 0.998 / 1.008 |
| `rounded-boxes` | 10.01 / 10.37 | 10.00 / 10.36 | 9.82 / 10.25 | 9.80 / 10.27 | 1.002 [0.987, 1.024] | 1.010 [0.987, 1.029] | 0.999 / 0.998 |
| `dense` | 11.77 / 12.44 | 11.83 / 12.38 | 11.73 / 12.09 | 11.65 / 12.04 | 0.991 [0.976, 1.000] | 1.013 [0.998, 1.024] | 0.990 / 1.009 |
| `ansi-replay` | 18.63 / 19.33 | 18.57 / 19.04 | 18.22 / 19.01 | 18.39 / 18.91 | 1.002 [0.996, 1.007] | 0.994 [0.986, 1.002] | 0.998 / 1.002 |
| `large` | 37.66 / 39.74 | 37.70 / 38.42 | 37.11 / 38.12 | 37.00 / 37.57 | 1.004 [0.995, 1.011] | 1.004 [1.001, 1.014] | 1.005 / 1.002 |
| `unicode` | 7.84 / 8.52 | 7.85 / 8.08 | 7.83 / 8.12 | 7.86 / 8.28 | 1.002 [0.989, 1.013] | 0.995 [0.987, 1.014] | 0.996 / 1.006 |
| `box-grid` | 7.34 / 7.67 | 7.40 / 7.74 | 7.25 / 7.86 | 7.22 / 7.55 | 1.001 [0.970, 1.009] | 1.002 [0.986, 1.021] | 0.990 / 1.009 |
| `block-grid` | 10.28 / 10.82 | 10.34 / 10.72 | 10.21 / 11.24 | 10.25 / 10.89 | 0.996 [0.977, 1.007] | 0.993 [0.981, 1.017] | 0.997 / 1.012 |
| `rounded-panes` | 9.86 / 10.64 | 9.68 / 10.44 | 9.44 / 10.06 | 9.37 / 10.29 | 1.011 [0.993, 1.039] | 1.010 [0.993, 1.022] | 1.020 / 1.010 |
| `rounded-24px` | 5.97 / 6.74 | 5.99 / 6.71 | 5.64 / 5.80 | 5.59 / 5.84 | 1.000 [0.986, 1.020] | 0.997 [0.985, 1.014] | 1.009 / 1.014 |
| `rounded-128px` | 31.69 / 33.47 | 31.50 / 32.79 | 32.12 / 33.34 | 31.81 / 33.59 | 1.011 [1.002, 1.024] | 1.008 [0.995, 1.022] | **1.015 / 1.014** |
| `geometry-all` | 90.66 / 97.19 | 90.64 / 98.48 | 89.76 / 91.29 | 89.67 / 90.94 | 0.997 [0.992, 1.003] | 1.000 [0.997, 1.006] | 0.999 / 1.004 |
| `large-sparse` | 21.09 / 28.96 | 20.97 / 26.70 | 20.16 / 20.98 | 20.14 / 20.48 | 1.013 [0.990, 1.029] | 0.999 [0.989, 1.006] | 1.013 / 1.002 |
| **`large-color`** | 144.99 / 148.92 | 143.90 / 147.15 | 144.20 / 150.09 | 143.13 / 145.82 | 1.006 [1.001, 1.010] | 1.006 [1.002, 1.010] | **1.005 / 1.008** |
| `image-below` | 21.62 / 22.18 | 21.64 / 22.20 | 21.65 / 22.43 | 21.62 / 22.42 | 0.999 [0.990, 1.006] | 1.001 [0.994, 1.006] | 1.001 / 1.001 |
| `image-under` | 25.05 / 25.85 | 25.05 / 25.92 | 24.96 / 25.60 | 24.87 / 25.56 | 1.000 [0.994, 1.009] | 1.003 [0.997, 1.009] | 0.998 / 0.993 |
| `image-over` | 19.70 / 20.45 | 19.65 / 20.62 | 20.04 / 20.70 | 19.98 / 20.75 | 1.005 [0.996, 1.008] | 1.003 [0.994, 1.010] | 1.002 / 1.003 |
| `dense-sgr` | 31.24 / 31.77 | 31.57 / 32.06 | 31.39 / 31.93 | 31.38 / 32.07 | 0.990 [0.988, 0.994] | 0.998 [0.993, 1.008] | **0.993 / 0.995** |
| **`cursor-moves`** | 27.67 / 28.34 | 28.01 / 28.95 | 27.54 / 28.16 | 28.02 / 28.68 | 0.985 [0.979, 0.996] | 0.983 [0.980, 0.989] | **0.982 / 0.982** |
| `scrolling` | 9.61 / 10.12 | 9.53 / 9.84 | 9.54 / 9.83 | 9.57 / 10.06 | 1.010 [0.995, 1.027] | 0.995 [0.982, 1.009] | 0.997 / 0.990 |
| `mixed-unicode` | 29.11 / 29.84 | 29.29 / 29.76 | 29.17 / 29.63 | 29.28 / 29.90 | 0.999 [0.994, 1.001] | 0.993 [0.987, 1.001] | 1.001 / 1.000 |
| **`thai-combining`** | 23.32 / 24.05 | 22.86 / 23.68 | 23.25 / 23.76 | 22.83 / 23.05 | 1.021 [1.016, 1.025] | 1.017 [1.013, 1.028] | **1.017 / 1.018** |
| `text-ansi-replay` | 13.45 / 14.13 | 13.41 / 14.16 | 13.57 / 13.84 | 13.47 / 13.81 | 1.004 [0.991, 1.011] | 1.006 [0.993, 1.012] | 0.993 / 1.004 |
| `text-ascii-overflow` | 4.92 / 5.18 | 4.96 / 5.18 | 4.96 / 5.39 | 4.99 / 5.30 | 0.998 [0.966, 1.019] | 0.994 [0.984, 1.016] | 0.996 / 0.999 |
| **`text-dense-sgr`** | 16.39 / 16.96 | 16.85 / 17.02 | 16.54 / 16.94 | 16.93 / 17.31 | 0.974 [0.966, 0.983] | 0.975 [0.968, 0.977] | **0.985 / 0.984** |
| `text-mixed-unicode` | 26.72 / 31.50 | 27.00 / 30.00 | 26.61 / 27.81 | 26.85 / 27.82 | 0.986 [0.981, 1.000] | 0.995 [0.987, 1.004] | 0.997 / 0.995 |
| `text-reply-sent` | 3.11 / 3.68 | 3.11 / 3.65 | 3.11 / 3.30 | 3.12 / 3.33 | 0.999 [0.987, 1.016] | 0.992 [0.982, 1.008] | 0.992 / 0.988 |
| `text-kitty` | 3.76 / 3.97 | 3.78 / 4.06 | 3.87 / 4.18 | 3.82 / 4.08 | 0.993 [0.972, 1.016] | 1.008 [0.987, 1.024] | 1.000 / 0.985 |
| `text-sixel` | 3.33 / 3.50 | 3.37 / 3.67 | 3.40 / 3.78 | 3.35 / 3.68 | 0.980 [0.975, 0.993] | 1.003 [0.995, 1.039] | 0.981 / 0.990 |

Linux x86-64:

| case | `base` A | `o2-fat` A | `base` B | `o2-fat` B | `o2-fat` speedup A | `o2-fat` speedup B | `cos-o2-fat` A / B |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 8.24 / 9.20 | 8.36 / 9.83 | 8.08 / 9.27 | 8.15 / 9.44 | 0.969 [0.931, 1.008] | 1.021 [0.961, 1.068] | 0.995 / 0.993 |
| `font-file` | 8.56 / 9.39 | 8.65 / 10.19 | 7.96 / 8.76 | 8.11 / 9.13 | 0.989 [0.942, 1.020] | 0.984 [0.946, 1.028] | 1.011 / 0.952 |
| `cjk-none` | 4.24 / 4.99 | 4.17 / 5.01 | 4.24 / 4.87 | 4.23 / 4.93 | 1.013 [0.954, 1.073] | 0.978 [0.920, 1.083] | 1.004 / 0.955 |
| `cjk-subset` | 9.79 / 11.08 | 9.86 / 11.29 | 9.97 / 10.98 | 9.55 / 10.29 | 0.985 [0.967, 1.015] | 1.037 [0.991, 1.077] | 0.996 / 1.009 |
| `cjk-cff-primary` | 9.03 / 10.07 | 9.04 / 9.82 | 9.35 / 10.01 | 9.01 / 10.02 | 0.991 [0.952, 1.026] | 1.033 [1.014, 1.065] | 0.992 / 1.032 |
| `mixed-subset` | 9.16 / 9.95 | 9.06 / 10.19 | 9.02 / 9.72 | 9.15 / 9.72 | 1.039 [0.998, 1.046] | 0.988 [0.958, 1.019] | 0.975 / 0.999 |
| `glyph-overflow` | 21.86 / 23.55 | 21.92 / 23.37 | 21.36 / 22.34 | 20.94 / 22.10 | 0.978 [0.963, 1.008] | 1.015 [0.985, 1.037] | 0.989 / 0.986 |
| `cjk-full` | 13.92 / 16.80 | 13.67 / 17.04 | 13.40 / 14.13 | 13.27 / 14.04 | 1.018 [0.966, 1.054] | 1.015 [0.985, 1.034] | 1.001 / 0.985 |
| `mixed-full` | 13.68 / 15.09 | 13.42 / 17.26 | 13.66 / 16.49 | 13.82 / 16.94 | 1.030 [0.972, 1.049] | 0.996 [0.960, 1.036] | 0.982 / 1.017 |
| `cjk-overflow-full` | 35.25 / 38.51 | 34.41 / 37.49 | 32.90 / 33.97 | 32.71 / 34.46 | 1.017 [1.001, 1.039] | 1.008 [0.997, 1.017] | 0.991 / 0.985 |
| `reply-sent` | 8.70 / 10.05 | 8.25 / 9.38 | 8.06 / 9.31 | 8.01 / 9.11 | 1.010 [0.973, 1.100] | 0.995 [0.921, 1.063] | 1.007 / 1.005 |
| `draft-ready` | 8.01 / 9.31 | 8.03 / 8.95 | 7.95 / 9.13 | 7.85 / 9.03 | 0.992 [0.963, 1.070] | 1.002 [0.950, 1.037] | 1.020 / 0.984 |
| `reply-24px` | 5.40 / 6.32 | 5.43 / 6.11 | 5.32 / 6.26 | 5.53 / 6.12 | 1.011 [0.959, 1.063] | 1.000 [0.957, 1.037] | 1.052 / 1.000 |
| `reply-128px` | 24.23 / 27.74 | 23.90 / 26.62 | 23.32 / 24.33 | 23.65 / 24.56 | 1.011 [0.980, 1.028] | 1.000 [0.978, 1.013] | 0.984 / 0.993 |
| `real-shell` | 5.31 / 6.52 | 5.32 / 6.08 | 5.52 / 6.18 | 5.59 / 6.26 | 1.001 [0.975, 1.039] | 0.991 [0.913, 1.039] | 0.994 / 0.993 |
| `real-less` | 4.96 / 5.66 | 4.92 / 6.18 | 4.90 / 5.51 | 4.94 / 5.94 | 1.014 [0.979, 1.043] | 0.989 [0.945, 1.014] | 1.003 / 1.010 |
| `real-vi` | 7.41 / 8.47 | 7.42 / 8.44 | 7.54 / 8.41 | 7.52 / 8.45 | 1.002 [0.966, 1.041] | 0.999 [0.980, 1.023] | 1.004 / 1.026 |
| `blank` | 4.16 / 5.05 | 4.17 / 4.90 | 4.01 / 5.06 | 4.12 / 4.95 | 0.992 [0.944, 1.047] | 1.012 [0.893, 1.075] | 0.981 / 1.023 |
| `color-grid` | 17.21 / 18.26 | 17.07 / 18.16 | 17.06 / 17.81 | 16.66 / 17.87 | 0.999 [0.984, 1.013] | 1.021 [1.008, 1.042] | 0.999 / 1.012 |
| `ascii-overflow` | 5.72 / 6.62 | 5.86 / 6.96 | 5.56 / 6.11 | 5.58 / 6.35 | 0.962 [0.912, 1.017] | 0.992 [0.955, 1.050] | 1.019 / 1.019 |
| `rounded-boxes` | 8.12 / 9.51 | 8.15 / 10.10 | 8.02 / 9.41 | 8.03 / 9.13 | 0.996 [0.939, 1.033] | 0.980 [0.962, 1.042] | 0.957 / 0.987 |
| `dense` | 11.60 / 12.89 | 11.07 / 11.87 | 10.51 / 11.81 | 10.67 / 11.71 | 1.056 [1.015, 1.105] | 1.000 [0.968, 1.028] | 1.059 / 0.995 |
| `ansi-replay` | 16.93 / 18.29 | 17.23 / 18.95 | 17.16 / 20.35 | 17.10 / 20.33 | 0.985 [0.947, 1.014] | 0.998 [0.967, 1.041] | 0.996 / 1.001 |
| `large` | 30.77 / 34.93 | 30.59 / 34.76 | 31.58 / 32.97 | 31.45 / 32.29 | 1.010 [0.998, 1.027] | 1.004 [0.996, 1.014] | 1.013 / 1.005 |
| `unicode` | 8.71 / 9.51 | 8.73 / 9.92 | 8.94 / 9.80 | 9.21 / 10.14 | 0.994 [0.969, 1.018] | 0.994 [0.959, 1.016] | 1.006 / 0.963 |
| `box-grid` | 5.75 / 6.48 | 5.44 / 6.30 | 5.87 / 6.83 | 5.87 / 6.88 | 1.019 [0.960, 1.061] | 0.986 [0.952, 1.059] | 1.014 / 1.001 |
| `block-grid` | 9.06 / 9.72 | 8.75 / 10.08 | 9.31 / 10.38 | 9.38 / 10.19 | 1.039 [0.968, 1.072] | 0.989 [0.967, 1.044] | 1.037 / 1.027 |
| `rounded-panes` | 8.37 / 9.23 | 8.40 / 9.47 | 8.54 / 9.19 | 8.51 / 9.71 | 0.984 [0.945, 1.045] | 1.008 [0.961, 1.037] | 0.998 / 0.969 |
| `rounded-24px` | 4.42 / 4.96 | 4.44 / 5.00 | 4.37 / 5.09 | 4.47 / 4.92 | 0.989 [0.958, 1.055] | 0.999 [0.949, 1.051] | 1.046 / 1.027 |
| **`rounded-128px`** | 22.43 / 23.92 | 23.03 / 26.02 | 22.66 / 23.69 | 23.00 / 24.13 | 0.971 [0.963, 0.985] | 0.985 [0.976, 0.999] | **0.967 / 0.974** |
| `geometry-all` | 79.38 / 87.49 | 78.72 / 85.07 | 75.92 / 77.71 | 75.92 / 78.19 | 1.002 [0.989, 1.013] | 1.000 [0.993, 1.008] | 1.006 / 0.999 |
| `large-sparse` | 14.71 / 16.28 | 14.81 / 17.61 | 14.67 / 15.39 | 14.75 / 15.68 | 0.994 [0.973, 1.015] | 0.990 [0.986, 1.003] | 0.997 / 0.985 |
| `large-color` | 128.32 / 136.05 | 127.05 / 134.27 | 125.59 / 127.49 | 124.53 / 126.66 | 1.009 [0.991, 1.025] | 1.008 [1.003, 1.011] | 1.005 / 1.004 |
| `image-below` | 20.81 / 23.01 | 20.51 / 22.26 | 19.95 / 21.18 | 20.11 / 21.27 | 0.998 [0.984, 1.044] | 0.999 [0.969, 1.035] | 1.019 / 1.010 |
| `image-under` | 23.96 / 25.50 | 24.33 / 26.76 | 23.24 / 24.32 | 23.00 / 24.40 | 0.988 [0.965, 1.002] | 1.006 [0.987, 1.024] | 1.010 / 1.008 |
| `image-over` | 19.74 / 21.79 | 20.18 / 22.73 | 18.84 / 20.15 | 18.89 / 20.28 | 0.999 [0.955, 1.018] | 0.995 [0.981, 1.014] | 1.007 / 1.000 |
| `dense-sgr` | 30.61 / 33.30 | 31.06 / 32.83 | 29.83 / 31.28 | 29.58 / 30.87 | 0.994 [0.969, 1.003] | 1.006 [0.997, 1.016] | 0.987 / 1.011 |
| **`cursor-moves`** | 25.98 / 27.06 | 26.75 / 28.04 | 26.22 / 27.42 | 26.56 / 27.93 | 0.972 [0.954, 0.985] | 0.982 [0.963, 0.998] | **0.962 / 0.977** |
| `scrolling` | 10.84 / 11.80 | 10.63 / 11.42 | 10.46 / 11.53 | 10.46 / 11.51 | 1.024 [0.984, 1.054] | 0.982 [0.961, 1.009] | 1.003 / 0.977 |
| `mixed-unicode` | 29.57 / 30.87 | 29.81 / 31.69 | 30.16 / 31.20 | 30.35 / 31.73 | 0.992 [0.981, 1.004] | 0.999 [0.986, 1.010] | 1.001 / 1.005 |
| **`thai-combining`** | 24.13 / 25.17 | 22.90 / 23.78 | 24.40 / 25.51 | 23.25 / 24.39 | 1.063 [1.045, 1.076] | 1.052 [1.044, 1.067] | **1.039 / 1.049** |
| `text-ansi-replay` | 10.42 / 11.09 | 10.82 / 11.52 | 10.35 / 11.23 | 10.49 / 11.48 | 0.965 [0.952, 0.980] | 0.975 [0.959, 1.010] | 0.966 / 0.979 |
| `text-ascii-overflow` | 3.29 / 3.91 | 3.20 / 3.92 | 3.43 / 3.82 | 3.22 / 3.84 | 0.990 [0.950, 1.036] | 1.060 [1.008, 1.080] | 1.016 / 0.989 |
| `text-dense-sgr` | 13.61 / 14.63 | 13.78 / 14.74 | 13.73 / 14.85 | 13.74 / 15.18 | 0.976 [0.967, 1.002] | 0.999 [0.979, 1.016] | 0.982 / 0.987 |
| `text-mixed-unicode` | 25.25 / 26.28 | 25.55 / 27.04 | 25.52 / 27.51 | 25.54 / 26.31 | 0.991 [0.981, 1.004] | 0.997 [0.984, 1.013] | 0.995 / 1.002 |
| `text-reply-sent` | 0.50 / 0.55 | 0.50 / 0.55 | 0.49 / 0.57 | 0.44 / 0.53 | 0.998 [0.974, 1.046] | 1.077 [0.980, 1.155] | 1.048 / 1.048 |
| `text-kitty` | 2.01 / 2.32 | 2.04 / 2.34 | 2.11 / 2.42 | 2.07 / 2.48 | 0.979 [0.926, 1.041] | 1.006 [0.912, 1.074] | 1.014 / 1.007 |
| `text-sixel` | 1.39 / 1.55 | 1.37 / 1.57 | 1.32 / 1.59 | 1.30 / 1.59 | 1.008 [0.951, 1.034] | 0.995 [0.846, 1.123] | 0.967 / 1.019 |

C `-Os` on top saves another 1.5-1.8% of the binary and 1.5-1.7% of the
archive. It slows `rounded-128px` further on Linux (0.967 / 0.974) and
`cursor-moves` (0.962 / 0.977), and Linux's CJK and glyph-heavy cases by up
to 3% (round 1, and `cos` alone in the probe: `cjk-overflow-full` 0.972,
`glyph-overflow` 0.980). It is rejected: the bar is no regression, and the
saving is small.

### `panic=abort`: rejected

On top of the chosen profile, `-C panic=abort` would save:

| | macOS universal | Linux x86_64 musl | Linux aarch64 musl |
| --- | ---: | ---: | ---: |
| binary | -98,992 (-4.4%) | -24,576 (-1.8%) | -65,536 (-5.2%, one 64 KiB step) |
| archive | -33,065 (-2.8%) | -13,302 (-1.9%) | -13,677 (-2.1%) |

It is rejected because it would break the exit-code contract. `catch_unwind` does not catch under
`panic=abort`. A scratch program whose `catch_unwind` maps a panic to exit 7
exits 7 when built with unwinding and 134 (SIGABRT) with `-C panic=abort`
(rustc 1.98.1, macOS). The FFI entry points rely on catching:

- **Some panics fail the render**: the compressor (`termshot_zlib_compress`
  returns NULL), the painters (`termshot_paint_text` returns 3,
  `termshot_paint_images` -1, geometry's `guarded` records it for
  `termshot_paint_failed`) and the render (`draw_png`). Each becomes exit 2
  with its own message, and the outputs are removed.
- **Others are recovered, and the render goes on**: a panic in a CFF outline
  (`font.rs`'s `outline` callback returns no outline), in an HVAR advance
  (`advance` falls back to the hmtx advance), in `termshot_geometry_new`
  (no cache) or `termshot_geometry_free`. These paths are defense in depth
  against hostile fonts. They exit 0 today.

A panic hook that removes the outputs and calls `_exit(2)` would keep the
first group's exit code, but not each one's message. It would turn the
second group's successful renders into failures, and it would also make a
panic outside the FFI paths exit 2 instead of 101. Keeping the contract would
take a hook that knows which boundary it is under. It would also take a
second build configuration with panic injection, to prove every path end to
end. That is not worth 1.8-5.2% of the binary (the 5.2% is one 64 KiB
step on aarch64), so unwinding stays.

### Output

- `./test.sh` with `RUSTFLAGS='-C lto=fat'` passes, and its 422 PNGs hash
  the same as a plain `./test.sh`'s, on macOS (Apple clang 21) and on starship
  (glibc, GCC 16.2.1).
- `bench/c-vs-rust/run.sh full`, with main's release binary as the old CLI
  and this branch's release.sh build as the new one, gave the same PNG
  bytes, exit codes and stderr. That was 4,265 renders on macOS (the arm64
  slices, with the system Noto CJK copy) and 4,266 on Linux (the x86_64 musl
  binaries). The script's old and new CLIs were swapped in a scratch copy;
  the script itself is unchanged.
- `scripts/release.sh macos-universal` (rustup 1.98.1) and
  `linux-x86_64-musl` (on starship, in Docker) pass their checks. Both
  macOS slices, x86_64 under Rosetta, render the samples like the host
  build.

### What was measured

| | macOS arm64 | Linux x86-64 |
| --- | --- | --- |
| host | `lawrences-mac-studio`, Apple M2 Max, macOS 26.6.2 | `starship`, Ryzen 7 8745HS, kernel 7.2.5-3-omarchy, governor `performance` |
| built with | rustc 1.98.1 (rustup, for both targets), Apple clang 21.0.0 | rustc 1.98.1, Debian 12 musl-gcc over GCC 12.2.0 (Docker `rust:1-bookworm` with `musl-tools`; starship has no musl-gcc) |
| `base` / `o2-fat` sha256 | `d33c3fc86098…` / `90aa06736ac5…` (arm64 slices) | `dc3b8ab5d2df…` / `b0aa617e6149…` |
| load average (1 min), start → end | round 1: A 4.33 → 3.53, B 3.53 → 3.55; round 2: A 3.12 → 4.12, B 4.12 → 3.69 | round 1: A 1.03 → 2.17, B 2.17 → 1.15; probe 0.59 → 1.30; round 2: A 0.26 → 2.23, B 2.23 → 2.46 |

The `toolchain` and `build_flags` fields in the JSON are bench.py's host
tools and `build.sh`'s defaults. Each binary's flags are in its `describe`
field, except in the Linux probe, which has none: its labels are the
candidates' names in the size table, and its `binary_sha256` values match
the binaries of the same name in the other files (`base` `dc3b8ab5d2df…`,
`o2-fat` `b0aa617e6149…`, `o2-fat-cgu1` `fba2f5b562df…`). On macOS the timed `o2-fat` slice was built in a scratch directory.
release.sh's build is 8 bytes larger with the same flags, and its PNGs match
(above).

Raw results: sizes for every candidate and platform in
[performance-2026-10-05-release-sizes.json](performance-2026-10-05-release-sizes.json).
Round 1: macOS [A](performance-2026-10-05-release-macos-a.json) and
[B](performance-2026-10-05-release-macos-b.json), Linux
[A](performance-2026-10-05-release-linux-a.json) and
[B](performance-2026-10-05-release-linux-b.json). The
[Linux probe](performance-2026-10-05-release-probe-linux.json). Round 2:
macOS [A](performance-2026-10-05-release-lto-macos-a.json) and
[B](performance-2026-10-05-release-lto-macos-b.json), Linux
[A](performance-2026-10-05-release-lto-linux-a.json) and
[B](performance-2026-10-05-release-lto-linux-b.json).

These files are slim (`bench.py --slim`): they keep every per-run wall, CPU,
profiled-wall and peak-RSS sample, each binary's output hash and size, and
all the metadata, but none of the per-run `TERMSHOT_PROFILE` records, which
this section does not cite. That is 3,589,881 bytes for the ten files,
against 30,343,230 for the full ones. Every summary, paired speedup and
interval recomputes from the samples (`summary` and `paired_ratio` are
deterministic), and every table row and number in this section was
recomputed from the slim files and matched. The full files are not in the
repository; ask for them by sha256:

| file | full sha256 |
| --- | --- |
| `release-macos-a` | `664f7c585ec38ea62d5c7172ef09fe952006f695d182928d1dea69368620f779` |
| `release-macos-b` | `f9ed47f582b532aaffbd829e9403ec6b66a861f05bccb9d6f40d0c305bfbc488` |
| `release-linux-a` | `e8f6ba21d528d49155a8112902329ace6456a3bb71da7a6b488d7d2b16541b39` |
| `release-linux-b` | `770e9a9939b6c9156aa006fbe39e8d0fffc8de6f1d4e0d6a587036eabe9a1792` |
| `release-probe-linux` | `9c916fc9e325c4ee979a5ac9814fd6a682d6c3401b646de34ba4bd402b26f517` |
| `release-lto-macos-a` | `643b27640fe11df0f1f4c09a2041ef11c2eb63848707f5460d44d4135cfd53f0` |
| `release-lto-macos-b` | `5866a77a0c0e8c642b5b1774aff682c1a2fd9316e0187dc82665546f9d9fb182` |
| `release-lto-linux-a` | `96df4b289929c0e24468dd0e5541f0058159b155bf046b4689a975676da1f787` |
| `release-lto-linux-b` | `2593680fd81d95b9a7e4efb64745b9f0d70d80d45ff426bd8b0713935971e672` |

No sample was discarded.

### Remaining limits

- aarch64 was measured for size only, not for speed.
- macOS x86_64 (the universal binary's other slice) was checked for output
  only, under Rosetta, not timed.
- `opt-level=3` was not tried; it is not a size option.
- One probe batch for the per-flag decomposition.

### Reproduce

```sh
# A candidate: release.sh's build with flags added (scratch harness: the
# same steps by hand). For example, on macOS:
RUSTFLAGS='-C lto=fat' CFLAGS='-arch arm64' TARGET=aarch64-apple-darwin ./build.sh
# The chosen profile end to end:
scripts/release.sh macos-universal
cjk=/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc   # on macOS, a copy (same sha256)
for batch in a:53 b:67; do
  python3 scripts/bench.py \
    --binary base=/tmp/base/termshot --binary o2-fat=/tmp/o2-fat/termshot \
    --reference base --runs 40 --warmups 5 --memory-runs 5 \
    --verify-identical --cjk-font "$cjk" \
    --seed "${batch#*:}" --output "/tmp/termshot-${batch%%:*}.json"
done
python3 scripts/bench-report.py /tmp/termshot-a.json /tmp/termshot-b.json --label o2-fat
```

## The render driver in Rust (2026-10-04, `cc29aed`, #12 step 2d)

#12 step 2d moves the driver from `src/draw.c` to `src/render.rs`: the font
setup call, the raster, the order of the passes, the errors, the PNG write
and this profile record. What is left of the C is `src/stb_glue.c`, stb's
font setup, metrics and PNG encoder behind three calls. The bar was no
regression beyond noise. Every output is the same: `bench/c-vs-rust/run.sh
full` renders 4,057 PNGs on macOS and 4,266 on Linux (with the system Noto
CJK) with the CLI built at main `48192f6` and with the branch, the same
bytes, exit codes and stderr; all 48 cases below gave one output per case
on both binaries and batches (`--verify-identical`); and every PNG
`./test.sh` writes hashes the same as on main on both hosts.

### Result

| | macOS arm64 (M2 Max) | Linux x86-64 (Ryzen 7 8745HS) |
| --- | --- | --- |
| end to end, 48 cases, paired wall speedup (main/branch) | 0.985-1.015 (A), 0.986-1.022 (B) | 0.960-1.077 (A), 0.933-1.055 (B) |
| slower with confidence in both batches | `mixed-unicode` 0.992 / 0.991 and `text-mixed-unicode` 0.990 / 0.991: the parser's time, see below | none |
| faster with confidence in both batches | none | `text-mixed-unicode` 1.020 / 1.011 |
| `output_write_ms`, the PNG write | 2-17% lower (one `write` instead of stdio's buffered ones) | within 0.01 ms |

The cases asked about, wall median (ms) of main and the branch from each
batch, the paired speedup (main/branch, above 1 is faster) with its 95%
bootstrap interval, and three stages the driver owns, batch A medians:
`allocate_ms` (the raster), `background_ms` (the backdrop, which first
touches the raster's pages) and `output_write_ms` (the PNG write):

macOS arm64:

| case | wall A | wall B | speedup A | speedup B | allocate_ms | background_ms | output_write_ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `reply-sent` | 8.72 / 8.78 | 8.70 / 8.68 | 0.996 [0.981, 1.006] | 1.000 [0.986, 1.018] | 0.004 → 0.004 | 0.845 → 0.865 | 0.176 → 0.146 |
| `reply-24px` | 5.61 / 5.62 | 5.64 / 5.65 | 0.993 [0.976, 1.011] | 0.995 [0.986, 1.015] | 0.004 → 0.004 | 0.210 → 0.211 | 0.111 → 0.102 |
| `reply-128px` | 27.91 / 27.91 | 28.25 / 28.08 | 1.000 [0.995, 1.004] | 1.007 [1.000, 1.022] | 0.004 → 0.004 | 5.249 → 5.230 | 0.401 → 0.356 |
| `glyph-overflow` | 16.23 / 16.11 | 16.19 / 16.26 | 1.002 [0.997, 1.012] | 1.001 [0.994, 1.010] | 0.004 → 0.004 | 0.216 → 0.218 | 0.175 → 0.160 |
| `cjk-subset` | 9.54 / 9.43 | 9.42 / 9.45 | 1.002 [0.989, 1.026] | 1.002 [0.991, 1.013] | 0.004 → 0.004 | 0.215 → 0.214 | 0.161 → 0.149 |
| `cjk-cff-primary` | 9.01 / 9.12 | 9.12 / 9.22 | 0.994 [0.972, 1.007] | 0.992 [0.984, 0.999] | 0.004 → 0.004 | 0.243 → 0.243 | 0.147 → 0.129 |
| `cjk-overflow-full` | 26.53 / 26.51 | 26.26 / 26.47 | 1.002 [0.992, 1.005] | 0.993 [0.986, 0.999] | 0.003 → 0.004 | 0.215 → 0.220 | 0.232 → 0.226 |
| `large` | 36.53 / 36.43 | 36.65 / 36.53 | 1.003 [0.998, 1.008] | 1.004 [0.994, 1.010] | 0.004 → 0.004 | 5.349 → 5.423 | 0.509 → 0.482 |
| `large-color` | 144.09 / 143.17 | 144.05 / 143.48 | 1.001 [0.996, 1.009] | 1.002 [0.998, 1.007] | 0.004 → 0.004 | 5.272 → 5.323 | 2.015 → 1.925 |
| `geometry-all` | 88.32 / 88.50 | 90.05 / 89.87 | 1.000 [0.997, 1.003] | 1.006 [1.000, 1.008] | 0.004 → 0.004 | 5.301 → 5.278 | 0.467 → 0.435 |
| `image-over` | 19.84 / 19.95 | 20.02 / 19.91 | 1.000 [0.990, 1.009] | 1.009 [0.996, 1.014] | 0.004 → 0.004 | 0.854 → 0.877 | 0.330 → 0.323 |
| `mixed-unicode` | 29.11 / 29.31 | 29.18 / 29.51 | 0.992 [0.989, 0.997] | 0.991 [0.985, 0.995] | 0.001 → 0.037 | 0.087 → 0.065 | 0.155 → 0.141 |
| `text-mixed-unicode` | 26.26 / 26.53 | 26.39 / 26.55 | 0.990 [0.984, 0.993] | 0.991 [0.988, 0.995] | — | — | — |

Linux x86-64:

| case | wall A | wall B | speedup A | speedup B | allocate_ms | background_ms | output_write_ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `reply-sent` | 6.96 / 6.99 | 7.31 / 7.31 | 0.985 [0.964, 1.006] | 0.988 [0.953, 1.043] | 0.007 → 0.006 | 0.789 → 0.767 | 0.095 → 0.093 |
| `reply-24px` | 4.64 / 4.70 | 5.06 / 4.83 | 1.013 [0.962, 1.081] | 1.041 [0.960, 1.080] | 0.007 → 0.006 | 0.800 → 0.811 | 0.050 → 0.045 |
| `reply-128px` | 23.09 / 23.25 | 22.84 / 23.40 | 0.994 [0.984, 1.003] | 0.995 [0.966, 1.013] | 0.007 → 0.007 | 4.080 → 4.096 | 0.378 → 0.369 |
| `glyph-overflow` | 15.43 / 15.42 | 15.63 / 15.50 | 1.016 [0.993, 1.036] | 1.003 [0.983, 1.031] | 0.008 → 0.008 | 0.831 → 0.832 | 0.164 → 0.155 |
| `cjk-subset` | 9.33 / 9.22 | 9.49 / 9.59 | 0.998 [0.973, 1.015] | 0.978 [0.961, 1.003] | 0.006 → 0.006 | 0.864 → 0.895 | 0.125 → 0.122 |
| `cjk-cff-primary` | 8.00 / 8.42 | 8.57 / 8.49 | 0.983 [0.945, 1.006] | 1.003 [0.953, 1.034] | 0.007 → 0.007 | 0.418 → 0.403 | 0.123 → 0.121 |
| `cjk-overflow-full` | 26.80 / 26.45 | 28.11 / 27.73 | 1.004 [0.988, 1.016] | 1.025 [0.983, 1.054] | 0.009 → 0.009 | 0.805 → 0.806 | 0.200 → 0.198 |
| `large` | 30.69 / 30.69 | 31.73 / 31.45 | 0.987 [0.981, 1.008] | 0.998 [0.986, 1.035] | 0.005 → 0.005 | 3.641 → 3.699 | 0.568 → 0.569 |
| `large-color` | 132.56 / 133.64 | 129.88 / 130.36 | 0.992 [0.985, 1.003] | 1.006 [0.984, 1.013] | 0.005 → 0.005 | 4.065 → 4.179 | 2.626 → 2.617 |
| `geometry-all` | 78.84 / 79.83 | 78.24 / 79.08 | 0.982 [0.971, 1.001] | 0.991 [0.979, 0.997] | 0.005 → 0.006 | 3.891 → 3.978 | 0.684 → 0.675 |
| `image-over` | 18.49 / 18.58 | 19.09 / 18.93 | 0.986 [0.969, 1.016] | 0.996 [0.988, 1.032] | 0.006 → 0.006 | 0.868 → 0.818 | 0.283 → 0.277 |
| `mixed-unicode` | 27.76 / 27.23 | 27.83 / 27.29 | 1.009 [0.996, 1.033] | 1.025 [1.005, 1.039] | 0.005 → 0.050 | 0.826 → 0.778 | 0.088 → 0.082 |
| `text-mixed-unicode` | 23.64 / 23.06 | 23.06 / 22.80 | 1.020 [1.009, 1.037] | 1.011 [1.003, 1.028] | — | — | — |

`bench-report.py` on the raw files gives every case. What moved, and why:

1. **The parser-bound cases on macOS** (`mixed-unicode`, and
   `text-mixed-unicode`, which writes no PNG and never reaches the render)
   read 0.8-1.0% slower in both batches: `parse_ms` 21.32 → 21.66 and
   21.19 → 21.56 ms. The parser is unchanged, and on Linux the same two
   cases read 1.1-2.5% faster (`parse_ms` 20.90 → 20.38), so this is where
   the binary's code landed, not the driver.
2. **The zeroed raster**: `allocate_ms` of `mixed-unicode` went 0.001 →
   0.037 ms (macOS) and 0.005 → 0.050 (Linux), and its `background_ms` down
   0.087 → 0.065 and 0.826 → 0.778. Its raster is small enough that calloc
   reuses heap memory the parser freed and clears it, where malloc did not;
   the backdrop then finds the pages mapped. A large raster is fresh pages,
   which calloc does not clear: every other case's `allocate_ms` is the
   same. The cost buys a raster whose every byte is initialized before
   Rust reads it.
3. **`geometry-all` on Linux**, 0.982 [0.971, 1.001] and 0.991 [0.979,
   0.997], and 0.986 [0.981, 0.995] in an 80-round recheck (raw JSON below):
   `deflate_match_emit_ms` 63.14 → 63.85 ms there. `src/deflate.rs` is
   unchanged, but rustc emits `termshot_zlib_compress` differently in the
   new crate (6,125 → 6,271 bytes on x86-64, the same calls, a 16-byte
   larger frame; 6,040 → 6,008 on arm64, which shows no change). Calling the
   deflate profile functions through `extern` declarations, as draw.c did,
   left it at 6,271 bytes. It is about 1% of the compressor on the largest
   PNGs on this host (`large-color` 0.992 / 1.006 overall).
4. **The PNG write** is one `write` of the whole PNG (`File::write_all`)
   instead of stdio's buffered `fwrite`: `output_write_ms` 2-17% lower on
   macOS (`reply-128px` 0.401 → 0.356), unchanged on Linux.

### What was measured

- **main**: `48192f6` (main after #80), built with `./build.sh`.
- **branch**: `cc29aed`, the step 2d commits on top of it. Later commits
  change documentation and tests, and read the result of the PNG file's
  `close`, which `File`'s drop made anyway: the same calls.

Method as in the
[step 2c round](#glyph-painting-in-rust-2026-10-04-ab924ca-12-step-2c):
`bench.py` with every suite, 5 warmups and 40 shuffled rounds of plain and
profiled runs per case and binary, 5 peak-RSS runs, seeds 17 (batch A) and
29 (batch B), `--verify-identical`, and the full CJK collection (sha256
`b76b0433…`, a copy on macOS) on both hosts. On Linux both binaries were
built from fresh clones of the pushed branch and of main in a temp dir,
since removed. The Linux recheck is `geometry-all`, `rounded-panes`,
`large-color` and `reply-sent`, 80 rounds, seed 41.

| | macOS arm64 | Linux x86-64 |
| --- | --- | --- |
| host | `lawrences-mac-studio`, macOS 26.6.2 | `starship`, kernel 7.2.5-3-omarchy, glibc 2.44, governor `performance` |
| compilers | rustc 1.98.1, Apple clang 21.0.0 (clang-2100.3.34.2) | rustc 1.98.1, GCC 16.2.1 20260810 |
| main / branch sha256 | `0b2a3291cd63…` / `fcbdafafe4e2…` | `abb45f3f1599…` / `3400376148c5…` |
| load average (1 min), start → end | A 5.97 → 3.74, B 3.74 → 5.85 | A 1.09 → 2.65, B 2.65 → 1.50, recheck 2.80 → 5.63 |

Raw results: macOS [batch A](performance-2026-10-04-driver-rust-macos-a.json)
and [batch B](performance-2026-10-04-driver-rust-macos-b.json); Linux
[batch A](performance-2026-10-04-driver-rust-linux-a.json),
[batch B](performance-2026-10-04-driver-rust-linux-b.json) and the
[recheck](performance-2026-10-04-driver-rust-linux-recheck.json). They hold
every sample; none was discarded.

### Validation

- `./test.sh`, `SANITIZE=1 ./test.sh`, `SANITIZE=1 ./tests/run.sh` and
  `./test.sh` under rustc 1.70.0 on macOS; `./test.sh`, `./tests/run.sh` and
  `bench/c-vs-rust/run.sh full` on starship (x86-64, GCC 16).
- `bench/c-vs-rust/run.sh full`: the glyphs matrix (every fixture,
  `tests/vt/real/`, `examples/`, `tests/perf/` and the generated logs, with
  12 font setups at `--px` 9, 24, 46, 47.5 and 128), plus `-v` and an image
  over 2^27 pixels with three font setups per log, and two unwritable
  outputs: 4,057 renders on macOS and 4,266 on Linux (a 13th setup with the
  system Noto CJK), PNG, exit code and stderr byte for byte.
- Every PNG `./test.sh` writes hashes the same as main's on both hosts: all
  412 names both write; the branch writes one more, the new
  `failed-render-allocation.png`.
- `tests/run.sh` fails the render's two allocations (the raster, stb's PNG
  buffer) in turn through the CLI, and the raster under `ulimit -v` on
  Linux: exit 2, the same message, no output. The first version of the
  raster dropped a `Raster` holding null when calloc failed, which is
  undefined, and the Linux check segfaulted; fixed in `cc29aed` with a unit
  test.
- `draw_png_is_reentrant` (four screens on twelve threads) and
  `tests/profile.rs`, which checks each concurrent profile record against
  its screen's, pass.

### Remaining limits

- One batch pair per host, and the Linux recheck for one case.
- The musl release builds were not run locally; `release.yml` builds and
  checks them on this pull request.

### Reproduce

```sh
git worktree add /tmp/termshot-main 48192f6 && (cd /tmp/termshot-main && ./build.sh)
./build.sh && cp termshot /tmp/termshot-branch
cjk=/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc   # on macOS, a copy (same sha256)
for batch in a:17 b:29; do
  python3 scripts/bench.py \
    --binary main=/tmp/termshot-main/termshot --binary branch=/tmp/termshot-branch \
    --describe main=48192f6 --describe branch=cc29aed \
    --reference main --runs 40 --warmups 5 --memory-runs 5 \
    --verify-identical --cjk-font "$cjk" \
    --seed "${batch#*:}" --output "/tmp/termshot-${batch%%:*}.json"
done
python3 scripts/bench-report.py /tmp/termshot-a.json /tmp/termshot-b.json
CJK_FONT="$cjk" bench/c-vs-rust/run.sh full 1
```

## Glyph painting in Rust (2026-10-04, `ab924ca`, #12 step 2c)

#12 step 2c moves the text from `src/draw.c` to `src/glyphs.rs`: the glyph
cache, the font and fallback lookups, the fallback's scaling, italic, bold,
combining marks, the box of a missing glyph, underlines and strike-through,
and the glyph blend; draw.c calls it once per render, and stb_truetype stays
C. The bar was no regression beyond noise. Every pixel is the same, but for
one C bug: `bench/c-vs-rust/run.sh glyphs` renders 3,640 PNGs with the CLI
built at main `c0b7b02` and with the branch, the same bytes, exit codes and
stderr on both hosts (glibc's malloc made to hand the old C zeroed blocks,
see Validation); all 48 cases below gave one output per case on both
binaries and batches; and every PNG `./test.sh` writes hashes the same as on
main, but for the reentrancy test's, whose screens changed.

### Result

| | macOS arm64 (M2 Max) | Linux x86-64 (Ryzen 7 8745HS) |
| --- | --- | --- |
| end to end, 48 cases, paired wall speedup (main/branch) | 0.989-1.022 (A), 0.988-1.017 (B) | 0.918-1.044 (A), 0.955-1.061 (B) |
| slower with confidence in both batches | none | none |
| faster with confidence in both batches | `large` 1.016 / 1.017, `large-color` 1.008 / 1.006, `image-over` 1.018 / 1.013 | `large` 1.027 / 1.034 |
| `blend_ms`, the glyph blend | up to 16% lower (`reply-128px`, `large`, `large-color`), within 4% at small cells | 6-23% lower |

The cases asked about, wall median (ms) of main and the branch from each
batch, the paired speedup (main/branch, above 1 is faster) with its 95%
bootstrap interval, and the glyph stages, batch A medians: `glyph_ms`
(finding and rasterizing glyphs), `blend_ms` (blending them, with the boxes
of missing ones) and `foreground_ms` (the text, box drawing and the images
over it):

macOS arm64:

| case | wall A | wall B | speedup A | speedup B | glyph_ms | blend_ms | foreground_ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `reply-sent` | 8.70 / 8.77 | 8.81 / 8.76 | 0.997 [0.988, 1.009] | 1.000 [0.982, 1.029] | 0.24 → 0.25 | 0.25 → 0.23 | 0.61 → 0.61 |
| `reply-128px` | 28.83 / 28.55 | 28.52 / 28.18 | 1.010 [0.994, 1.025] | 1.008 [1.000, 1.020] | 0.55 → 0.55 | 1.25 → 1.05 | 2.08 → 1.87 |
| `glyph-overflow` | 16.40 / 16.35 | 16.26 / 16.22 | 1.000 [0.992, 1.008] | 0.998 [0.990, 1.011] | 4.71 → 4.72 | 0.92 → 0.90 | 5.93 → 5.91 |
| `cjk-none` | 4.97 / 4.93 | 4.91 / 4.96 | 1.010 [0.989, 1.029] | 1.001 [0.987, 1.011] | 0.07 → 0.08 | 0.12 → 0.12 | 0.34 → 0.34 |
| `cjk-subset` | 9.51 / 9.57 | 9.64 / 9.45 | 1.001 [0.991, 1.022] | 1.015 [1.004, 1.028] | 0.29 → 0.30 | 0.53 → 0.51 | 0.98 → 0.95 |
| `cjk-cff-primary` | 9.21 / 9.37 | 9.13 / 9.05 | 1.001 [0.986, 1.012] | 1.016 [1.001, 1.023] | 0.31 → 0.31 | 0.51 → 0.51 | 0.98 → 0.96 |
| `cjk-full` | 12.68 / 12.63 | 12.53 / 12.51 | 1.007 [0.998, 1.012] | 0.999 [0.991, 1.010] | 0.32 → 0.32 | 0.52 → 0.51 | 0.99 → 0.98 |
| `cjk-overflow-full` | 26.69 / 26.61 | 26.83 / 26.92 | 1.004 [0.995, 1.011] | 0.996 [0.988, 1.010] | 9.61 → 9.54 | 1.05 → 1.03 | 10.81 → 10.78 |
| `mixed-subset` | 7.62 / 7.56 | 7.61 / 7.60 | 0.992 [0.980, 1.017] | 0.996 [0.987, 1.015] | 0.71 → 0.70 | 0.22 → 0.22 | 1.10 → 1.07 |
| `mixed-full` | 11.43 / 11.21 | 11.05 / 11.03 | 1.008 [0.999, 1.017] | 0.996 [0.987, 1.013] | 0.92 → 0.93 | 0.25 → 0.26 | 1.34 → 1.35 |
| `thai-combining` | 24.33 / 24.36 | 24.22 / 24.21 | 0.998 [0.989, 1.003] | 1.004 [0.995, 1.009] | 0.08 → 0.09 | 0.10 → 0.10 | 0.35 → 0.36 |
| `large` | 37.44 / 37.00 | 36.78 / 36.01 | 1.016 [1.006, 1.025] | 1.017 [1.012, 1.028] | 0.32 → 0.34 | 4.08 → 3.50 | 5.35 → 4.79 |
| `large-color` | 146.31 / 145.06 | 145.38 / 144.52 | 1.008 [1.005, 1.013] | 1.006 [1.003, 1.010] | 0.43 → 0.50 | 6.90 → 5.83 | 9.02 → 7.95 |

Linux x86-64:

| case | wall A | wall B | speedup A | speedup B | glyph_ms | blend_ms | foreground_ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `reply-sent` | 7.61 / 7.65 | 7.45 / 7.38 | 0.992 [0.968, 1.021] | 0.994 [0.974, 1.035] | 0.25 → 0.26 | 0.29 → 0.26 | 0.68 → 0.67 |
| `reply-128px` | 23.62 / 23.52 | 23.95 / 23.72 | 1.006 [0.994, 1.014] | 1.004 [0.998, 1.020] | 0.61 → 0.62 | 1.25 → 1.07 | 2.16 → 2.01 |
| `glyph-overflow` | 15.16 / 15.40 | 15.14 / 15.25 | 0.990 [0.962, 1.010] | 0.996 [0.977, 1.023] | 4.49 → 4.55 | 1.14 → 1.03 | 5.97 → 5.89 |
| `cjk-none` | 4.20 / 4.38 | 4.47 / 4.49 | 0.961 [0.865, 1.036] | 0.977 [0.945, 1.026] | 0.07 → 0.07 | 0.12 → 0.11 | 0.36 → 0.35 |
| `cjk-subset` | 9.75 / 9.75 | 9.79 / 9.67 | 1.011 [0.994, 1.038] | 1.012 [0.975, 1.047] | 0.28 → 0.28 | 0.81 → 0.64 | 1.25 → 1.09 |
| `cjk-cff-primary` | 8.65 / 8.72 | 8.75 / 8.60 | 0.983 [0.951, 1.026] | 0.989 [0.965, 1.033] | 0.32 → 0.31 | 0.79 → 0.61 | 1.28 → 1.09 |
| `cjk-full` | 13.83 / 13.62 | 13.51 / 13.47 | 1.009 [0.987, 1.050] | 0.978 [0.945, 1.039] | 0.29 → 0.30 | 0.80 → 0.63 | 1.27 → 1.09 |
| `cjk-overflow-full` | 27.21 / 27.57 | 27.92 / 27.91 | 0.993 [0.970, 1.017] | 1.014 [0.988, 1.026] | 9.62 → 9.78 | 1.16 → 1.02 | 10.98 → 11.01 |
| `mixed-subset` | 7.72 / 7.49 | 7.51 / 7.57 | 1.017 [1.006, 1.053] | 1.013 [0.982, 1.035] | 0.66 → 0.67 | 0.29 → 0.26 | 1.14 → 1.10 |
| `mixed-full` | 11.74 / 12.07 | 12.03 / 11.91 | 0.969 [0.946, 0.996] | 1.019 [0.983, 1.040] | 0.85 → 0.90 | 0.32 → 0.30 | 1.36 → 1.37 |
| `thai-combining` | 24.46 / 24.11 | 23.75 / 23.98 | 1.009 [1.000, 1.026] | 0.995 [0.975, 1.008] | 0.09 → 0.09 | 0.10 → 0.09 | 0.38 → 0.35 |
| `large` | 33.43 / 32.43 | 33.56 / 32.68 | 1.027 [1.012, 1.045] | 1.034 [1.016, 1.041] | 0.34 → 0.32 | 5.22 → 4.25 | 6.57 → 5.59 |
| `large-color` | 136.65 / 134.42 | 136.09 / 134.81 | 1.011 [1.008, 1.023] | 1.009 [0.997, 1.021] | 0.47 → 0.45 | 8.99 → 7.42 | 11.19 → 9.60 |

`bench-report.py` on the raw files gives every case. The blend is where the
time moved: `blend_ms` of `large` 4.08 → 3.50 (macOS) and 5.22 → 4.25 ms
(Linux), of the CJK cases 0.80 → 0.63 on Linux. On Linux `cjk-none` read
0.961 and 0.977 with intervals across 1 and the same `foreground_ms`, and
`mixed-full` 0.969 [0.946, 0.996] in batch A only (1.019 in B); the
text-only cases, which paint nothing, ranged 0.918-1.061, which is this
host's noise at these sizes.

`glyph_ms` on the M2 reads 0.01-0.07 ms more on screens of cache hits
(`large-color`, 17,271 hits: 0.43 → 0.50), about 3 ns a hit, with the same
or lower `foreground_ms`. Built with 64-byte loop alignment
(`CFLAGS=-falign-loops=64`, `-C llvm-args=-align-loops=64`), a 240x80
screen of random coloured ASCII still read 0.687 → 0.750 ms of `glyph_ms`
and 5.667 → 5.648 of `foreground_ms` (41 rounds, `bench/c-vs-rust/glyphs.rs
time`), so it is not alignment, and it is not in the wall time.

### Glyph stages alone

`bench/c-vs-rust/run.sh glyphs 61`, after its pixel check, renders a few
screens with both CLIs in alternating rounds and reports the median of each
stage of `TERMSHOT_PROFILE` (ms, Rust ÷ C):

| screen | macOS glyph | blend | foreground | Linux glyph | blend | foreground |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `reply-sent` `--px 48` | 0.243 → 0.244 (1.004) | 0.254 → 0.233 (0.917) | 0.619 → 0.599 (0.968) | 0.375 → 0.354 (0.944) | 0.973 → 0.942 (0.969) | 1.581 → 1.544 (0.977) |
| `reply-sent` `--px 128` | 0.538 → 0.545 (1.013) | 1.243 → 1.057 (0.850) | 2.057 → 1.873 (0.911) | 0.938 → 0.942 (1.005) | 2.122 → 2.060 (0.971) | 3.495 → 3.463 (0.991) |
| `glyph-overflow` `--px 24` | 4.756 → 4.738 (0.996) | 0.918 → 0.930 (1.013) | 6.026 → 6.000 (0.996) | 6.415 → 6.516 (1.016) | 2.682 → 2.425 (0.904) | 9.544 → 9.385 (0.983) |
| `cjk-dense`, CFF fallback | 0.287 → 0.290 (1.010) | 0.538 → 0.516 (0.959) | 0.982 → 0.953 (0.970) | 0.398 → 0.405 (1.018) | 1.379 → 1.176 (0.853) | 1.985 → 1.812 (0.913) |
| `cjk-dense`, CFF font | 0.298 → 0.306 (1.027) | 0.505 → 0.497 (0.984) | 0.955 → 0.960 (1.005) | 0.422 → 0.438 (1.037) | 1.297 → 1.084 (0.836) | 1.941 → 1.731 (0.892) |
| `italic` `--px 48` | 0.117 → 0.117 (1.000) | 0.064 → 0.061 (0.953) | 0.203 → 0.201 (0.990) | 0.145 → 0.153 (1.056) | 0.177 → 0.178 (1.007) | 0.384 → 0.407 (1.059) |
| marks, marks fallback | 0.207 → 0.209 (1.010) | 0.093 → 0.088 (0.946) | 0.348 → 0.346 (0.994) | 0.276 → 0.282 (1.025) | 0.297 → 0.286 (0.965) | 0.672 → 0.648 (0.964) |
| JetBrains Mono's cmap, italic | 3.622 → 3.622 (1.000) | 1.138 → 1.092 (0.960) | 4.876 → 4.876 (1.000) | 5.522 → 5.597 (1.014) | 2.845 → 2.876 (1.011) | 8.684 → 8.743 (1.007) |
| the CFF2 subset's cmap | 0.735 → 0.741 (1.008) | 0.103 → 0.106 (1.029) | 0.865 → 0.878 (1.015) | 1.077 → 1.101 (1.022) | 0.336 → 0.328 (0.976) | 1.486 → 1.485 (1.000) |

macOS ran at a load of 3.3-4.8. The Linux run began at a load of 12 and
ended at 33, under another session's compile, so its absolute numbers are
high; the alternating rounds keep the ratios paired. Two choices made the
port's numbers what they are, each measured:

1. **The blend checks a row once.** The glyph's row and the canvas's are
   sliced once per row, then blended through pointers by composite.rs's
   `blend_pixel`, the core it shares with the images (the C's `blend`, with
   the same 255 and 0 shortcuts).
2. **The profile reads the C's clock.** With std's `Instant`, the profiled
   `foreground_ms` of cache-hit screens read up to 15% over the C's
   (`cjk-dense` 0.34 → 0.39 ms) at the same wall time; reading
   `CLOCK_MONOTONIC` as draw.c's `now_ms` does gives 0.349 → 0.346. The
   cache's hit path is inline and its miss path a call of its own.

### What was measured

- **main**: `c0b7b02` (main after #78), built with `./build.sh`.
- **branch**: `ab924ca`, the step 2c commits on top of it, merged with
  `c0b7b02`. Later commits change only the differential test and
  documentation.

Method as in the
[step 2b round](#image-layers-in-rust-2026-10-04-82c3f3d-12-step-2b):
`bench.py` with every suite, 5 warmups and 40 shuffled rounds of plain and
profiled runs per case and binary, 5 peak-RSS runs, seeds 17 (batch A) and
29 (batch B), `--verify-identical`, and the full CJK collection (sha256
`b76b0433…`, a copy on macOS) on both hosts. On Linux both binaries were
built from fresh clones of the pushed branch and of main in a temp dir,
since removed.

| | macOS arm64 | Linux x86-64 |
| --- | --- | --- |
| host | `lawrences-mac-studio`, macOS 26.6.2 | `starship`, kernel 7.2.5-3-omarchy, glibc 2.44, governor `performance` |
| compilers | rustc 1.98.1, Apple clang 21.0.0 (clang-2100.3.34.2) | rustc 1.98.1, GCC 16.2.1 20260810 |
| main / branch sha256 | `242b467b8689…` / `0b2a3291cd63…` | `93cf9593e62f…` / `abb45f3f1599…` |
| load average (1 min), start → end | A 4.96 → 4.78, B 4.78 → 4.45 | A 3.60 → 2.11, B 2.11 → 3.96 (the 15-minute average was 14 → 11 after another session's compile) |

Raw results: macOS [batch A](performance-2026-10-04-glyphs-rust-macos-a.json)
and [batch B](performance-2026-10-04-glyphs-rust-macos-b.json); Linux
[batch A](performance-2026-10-04-glyphs-rust-linux-a.json) and
[batch B](performance-2026-10-04-glyphs-rust-linux-b.json). They hold every
sample; none was discarded.

### Validation

- `./test.sh`, `SANITIZE=1 ./test.sh`, `SANITIZE=1 ./tests/run.sh` and
  `./test.sh` under rustc 1.70.0 on macOS; `./test.sh`, `./tests/run.sh` and
  `bench/c-vs-rust/run.sh glyphs` on starship (x86-64, GCC 16).
- `bench/c-vs-rust/run.sh glyphs`: 3,640 renders, PNG, exit code and stderr
  byte for byte, on both hosts (with the full Noto CJK as a 13th font
  setup). A deliberate `+ 128` in the blend changes `reply-sent`'s PNG, and
  a slant of 0.2126 instead of 0.21256 changes `italic.pty`'s.
- One difference was a C bug, now fixed: a glyph with a box but no points (Α
  and А in the font `tests/glyphs.c` writes with an empty 'A', of which they
  are composites) makes stb write nothing into its bitmap. The C blended
  what malloc returned: zeros on macOS, but on glibc, in `glyph-overflow`,
  six cells of leftovers. The Rust's bitmap starts zeroed, so they draw
  nothing; the differential runs the old C with
  `GLIBC_TUNABLES=glibc.malloc.tcache_count=0:glibc.malloc.perturb=255`,
  which zeroes its blocks, and then they match.
- Every PNG `./test.sh` writes hashes the same as main's on both hosts: 395
  of the 403 names both write, and the other eight are the reentrancy
  test's `thread-N.png`, which now draws four other screens on twelve
  threads.
- `src/glyphs_tests.rs` drives the Rust with fake stb functions: the
  cache's slots, keys and conflicts, the lookups, missing and empty glyphs
  and their report, that a CFF face never reaches stb's outline readers, the
  outline scratch's growth, the fallback's scaling and centering, both mark
  rules, the slant and the lines, and a failure at each of its 9
  allocations (all 3 sites) in turn. `a_failed_glyph_allocation_fails_the_render`
  fails each of 8 with the vendored fonts through draw.c (2, no PNG), and
  `tests/run.sh` each of 10 in the CLI (exit 2, no PNG, no `--text`).
- `tests/glyphs.c` (168 placement checks) and `tests/draw.c` pass unchanged
  in what they check, now against the Rust.
- `scripts/release.sh macos-universal`: both slices render the samples like
  the host build, and 32 glyph renders each (italic, marks, CJK, missing
  glyphs, the overflow screen, random cells; at `--px` 46, 47.5, 24 and 9,
  with CFF, CFF2 at `wght=700` and the marks font) like it too, the x86_64
  slice under Rosetta.

### Remaining limits

- One batch pair per host; on Linux the differential's timing ran under
  another session's load.
- The musl release builds were not run locally; `release.yml` builds and
  checks them on this pull request.

### Reproduce

```sh
git worktree add /tmp/termshot-main c0b7b02 && (cd /tmp/termshot-main && ./build.sh)
./build.sh && cp termshot /tmp/termshot-branch
cjk=/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc   # on macOS, a copy (same sha256)
for batch in a:17 b:29; do
  python3 scripts/bench.py \
    --binary main=/tmp/termshot-main/termshot --binary branch=/tmp/termshot-branch \
    --describe main=c0b7b02 --describe branch=ab924ca \
    --reference main --runs 40 --warmups 5 --memory-runs 5 \
    --verify-identical --cjk-font "$cjk" \
    --seed "${batch#*:}" --output "/tmp/termshot-${batch%%:*}.json"
done
python3 scripts/bench-report.py /tmp/termshot-a.json /tmp/termshot-b.json
CJK_FONT="$cjk" bench/c-vs-rust/run.sh glyphs 61
```

## Image layers in Rust (2026-10-04, `82c3f3d`, #12 step 2b)

#12 step 2b moves image compositing (`paint_image_rows`: layers, crops,
clips, the mask of default backgrounds, the blend) and the backdrop under
the text (`backdrop_through`: the cell backgrounds and the two lower layers,
by rows of cells) from `src/draw.c` to `src/composite.rs`; draw.c calls it
per backdrop row or per layer. The bar was no regression beyond noise. Every
pixel is the same: `bench/c-vs-rust/run.sh images` paints 20,005 scenes
(386 million pixels) with draw.c as of `431ed23` and with the Rust, byte for
byte, and renders 132 fixture PNGs with both CLIs, the same bytes, on both
hosts; all 48 cases below gave one output per case on both binaries and
batches; and every PNG `./test.sh` writes (399 files) hashes the same as on
main, on both hosts.

### Result

| | macOS arm64 (M2 Max) | Linux x86-64 (Ryzen 7 8745HS) |
| --- | --- | --- |
| end to end, 48 cases, paired wall speedup (main/branch) | 0.979-1.021 over both batches | 0.960-1.050 over both batches (batch A under a falling load of 23 → 7) |
| slower with confidence in both batches | none | none |
| faster with confidence in both batches | `image-below` 1.019 / 1.018 | `image-below` 1.048 / 1.027, `image-under` 1.040 / 1.022, `image-over` 1.037 / 1.039, `large` 1.010 / 1.019, `geometry-all` 1.009 / 1.011, `rounded-128px` 1.017 / 1.023 |
| image layers alone (`run.sh images 61`), Rust ÷ C | 0.738-0.970 | 0.680-0.985 |

The cases asked about, wall median / p95 (ms) from batch A, the paired
speedup (main/branch, above 1 is faster) with its 95% bootstrap interval for
both batches, and the `background` stage (the backdrop: backgrounds and the
images under the text) and `foreground` stage (glyphs, geometry and the
images over the text), batch A medians:

macOS arm64:

| case | main wall | branch wall | speedup A | speedup B | background_ms | foreground_ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `image-below` | 22.11 / 22.84 | 21.73 / 22.62 | 1.019 [1.010, 1.029] | 1.018 [1.012, 1.022] | 2.37 → 1.89 | 1.60 → 1.63 |
| `image-under` | 25.20 / 26.08 | 25.14 / 25.97 | 0.998 [0.990, 1.008] | 1.000 [0.993, 1.006] | 2.48 → 2.40 | 1.56 → 1.64 |
| `image-over` | 20.18 / 20.95 | 20.12 / 21.06 | 1.004 [0.988, 1.008] | 1.006 [0.998, 1.015] | 0.89 → 0.90 | 3.17 → 3.12 |
| `large` | 37.10 / 38.15 | 37.00 / 37.93 | 1.001 [0.995, 1.006] | 0.998 [0.986, 1.007] | 5.40 → 5.45 | 5.16 → 5.34 |
| `large-color` | 147.05 / 153.38 | 147.28 / 151.92 | 0.998 [0.992, 1.006] | 1.002 [0.996, 1.004] | 5.53 → 5.55 | 8.69 → 9.00 |
| `reply-128px` | 28.61 / 29.09 | 28.64 / 29.58 | 0.998 [0.985, 1.005] | 1.001 [0.986, 1.008] | 5.76 → 5.65 | 1.97 → 2.05 |
| `geometry-all` | 89.48 / 90.23 | 89.36 / 90.00 | 1.002 [0.998, 1.005] | 1.004 [0.999, 1.007] | 5.56 → 5.51 | 3.85 → 3.86 |
| `reply-sent` | 8.79 / 9.31 | 8.76 / 9.36 | 0.998 [0.982, 1.010] | 0.997 [0.980, 1.004] | 0.91 → 0.90 | 0.60 → 0.62 |

Linux x86-64:

| case | main wall | branch wall | speedup A | speedup B | background_ms | foreground_ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `image-below` | 20.49 / 21.47 | 19.59 / 20.91 | 1.048 [1.026, 1.061] | 1.027 [1.012, 1.051] | 2.45 → 1.90 | 2.11 → 2.03 |
| `image-under` | 23.08 / 24.25 | 22.20 / 23.29 | 1.040 [1.014, 1.053] | 1.022 [1.010, 1.036] | 2.79 → 2.21 | 2.15 → 2.06 |
| `image-over` | 18.48 / 19.51 | 17.81 / 19.05 | 1.037 [1.026, 1.050] | 1.039 [1.024, 1.055] | 0.77 → 0.78 | 4.08 → 3.38 |
| `large` | 32.28 / 34.04 | 31.97 / 33.39 | 1.010 [1.003, 1.024] | 1.019 [1.006, 1.028] | 3.67 → 3.69 | 6.26 → 6.36 |
| `large-color` | 132.92 / 140.06 | 131.64 / 140.92 | 1.004 [0.999, 1.011] | 1.008 [1.003, 1.015] | 3.87 → 3.90 | 11.10 → 11.19 |
| `reply-128px` | 26.44 / 39.27 | 25.52 / 38.25 | 0.999 [0.993, 1.018] | 1.015 [0.996, 1.022] | 4.57 → 4.58 | 2.20 → 2.25 |
| `geometry-all` | 78.64 / 104.48 | 77.79 / 94.30 | 1.009 [1.004, 1.017] | 1.011 [1.001, 1.016] | 3.79 → 3.81 | 3.76 → 3.77 |
| `reply-sent` | 7.16 / 7.80 | 7.06 / 7.84 | 1.016 [0.990, 1.035] | 0.988 [0.970, 1.004] | 0.78 → 0.77 | 0.69 → 0.69 |

`bench-report.py` on the raw files gives every case. The images are where
the time moved: on Linux the backdrop of `image-below` and `image-under`
went 2.45 → 1.90 and 2.79 → 2.21 ms, and `image-over`'s foreground 4.08 →
3.38 ms. No case is slower with confidence in both batches on either host.
On Linux, `unicode` was 0.960 [0.932, 0.982] in batch A, at a load of 20
and falling, and 0.996 [0.967, 1.014] in batch B; it is parser-bound.

On the M2, `foreground_ms` of `large` and `large-color` is 3-4% higher in
both batches (5.16 → 5.34, 8.69 → 9.00 ms) with no wall difference (1.001 /
0.998, 0.998 / 1.002). It is their glyph `blend_ms` (3.87 → 4.04, 6.55 →
6.87), draw.c's `blend`, which this branch doesn't change. 30 alternating
profiled runs of `large-color` gave the same (6.55 → 6.82 ms); built with
`CFLAGS=-falign-loops=64`, main and the branch read 6.88 and 6.82 ms. So
the difference is the alignment of that C loop, which moving the image code
out of draw.c shifted, not the Rust; step 2c moves `blend` to Rust.

### Image layers alone

`bench/c-vs-rust/run.sh images 61` paints the backdrop (a row of cells at a
time, as draw_png calls it, or whole as it decides) and the images over the
text of bench.py's image screens with both, after the pixel check, in
alternating rounds, median ms per screen (cells 22x48, `--px 48`):

| screen | Apple clang 21 C | Rust | Rust ÷ C | GCC 16 C | Rust | Rust ÷ C |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `image-below` 100x30, whole | 2.335 | 1.724 | 0.738 | 2.432 | 1.654 | 0.680 |
| `image-under` 100x30, whole | 2.438 | 2.328 | 0.955 | 2.812 | 2.126 | 0.756 |
| `image-over` 100x30, whole | 2.431 | 2.358 | 0.970 | 2.808 | 2.128 | 0.758 |
| `large-below` 240x80, rows | 3.722 | 3.068 | 0.824 | 4.896 | 4.100 | 0.837 |
| `large-none` 240x80, rows, no image | 1.530 | 1.438 | 0.940 | 2.578 | 2.540 | 0.985 |

macOS ran at a load of 3.2, Linux at 1.6. Two differences from a straight
port, not measured apart:

1. **The sampler checks once and writes through pointers.** A row of the
   crop and a row of the canvas are sliced (checked) once per row, and the
   last column's source pixel once per run; the pixel loop then reads and
   writes through pointers, as the C does. Step 2a found a check per pixel
   cost up to 25% on short spans.
2. **The mask goes a cell at a time.** Below the backgrounds the C tested
   each pixel's cell; the Rust paints a run per cell and skips a run over an
   opaque one, recomputing the source column at the next run.

The cell backgrounds' first scanline is filled inside Rust now, not through
`termshot_fill_rect` once per cell; with no image (`large-none`) that is
0.94-0.99 of the C.

### What was measured

- **main**: `431ed23` (main after #75), built with `./build.sh`.
- **branch**: `82c3f3d`, the step 2b commits on top of it. Later commits
  change documentation and add one check per image outside the paint loops
  (`4fd0d28`: a crop must lie inside its image).

Method as in the
[step 2a round](#box-drawing-in-rust-2026-10-04-629a4d4-12-step-2a):
`bench.py` with every suite (the `draw` workloads included), 5 warmups and
40 shuffled rounds of plain and profiled runs per case and binary, 5 peak-RSS
runs, seeds 17 (batch A) and 29 (batch B), `--verify-identical`, and the full
CJK collection (sha256 `b76b0433…`) on both hosts. On Linux both binaries
were built from fresh clones of the pushed branch and of main in a temp dir,
since removed.

| | macOS arm64 | Linux x86-64 |
| --- | --- | --- |
| host | `lawrences-mac-studio`, macOS 26.6.2 | `starship`, kernel 7.2.5-3-omarchy, glibc 2.44, governor `performance` |
| compilers | rustc 1.98.1, Apple clang 21.0.0 (clang-2100.3.34.2) | rustc 1.98.1, GCC 16.2.1 20260810 |
| main / branch sha256 | `56398602405a…` / `16f1125b5205…` | `ebdef544b2b3…` / `041b9518130b…` |
| load average (1 min), start → end | A 4.49 → 5.83, B 5.83 → 4.09 | A 22.91 → 7.15, B 7.15 → 2.18: another session was compiling as batch A began |

Raw results: macOS [batch A](performance-2026-10-04-images-rust-macos-a.json)
and [batch B](performance-2026-10-04-images-rust-macos-b.json); Linux
[batch A](performance-2026-10-04-images-rust-linux-a.json) and
[batch B](performance-2026-10-04-images-rust-linux-b.json). They hold every
sample; none was discarded.

### Validation

- `./test.sh`, `SANITIZE=1 ./test.sh`, `SANITIZE=1 ./tests/run.sh` and
  `./test.sh` under rustc 1.70.0 on macOS; `./test.sh`, `./tests/run.sh` and
  `bench/c-vs-rust/run.sh images` on starship (x86-64, GCC 16).
- `tests/graphics.rs` (its 2.9 million kitty, Sixel, placeholder and
  relative-placement pixel checks) passes unchanged.
- The unit tests carry over `tests/draw.c`'s image checks (layers, crops,
  clips, the mask, rows against whole) and compare the Rust with a per-pixel
  transcription of the C on 3,000 random views; `tests/draw.c` still runs
  the backdrop and the layer over the text through draw.c's declarations.
- Nothing in the image layers allocates, in the C or the Rust, so there is
  no allocation to fail in turn. The raster they paint is draw.c's:
  `tests/run.sh` fails it on Linux with `ulimit -v` (exit 2, no PNG or
  `--text` left). A panic (a crop outside its image) fails the render in
  each layer with exit 2 and no PNG (`a_failed_image_layer_fails_the_render`).
- `scripts/release.sh macos-universal`: both slices render the samples like
  the host build, and the 22 kitty, Sixel and cursor fixtures at three sizes
  (66 PNGs) like it too, the x86_64 slice under Rosetta.

### Remaining limits

- One batch pair per host; on Linux batch A began under another session's
  compile.
- The musl release builds were not run locally; `release.yml` builds and
  checks them on this pull request.

### Reproduce

```sh
git worktree add /tmp/termshot-main 431ed23 && (cd /tmp/termshot-main && ./build.sh)
./build.sh && cp termshot /tmp/termshot-branch
cjk=/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc   # on macOS, a copy (same sha256)
for batch in a:17 b:29; do
  python3 scripts/bench.py \
    --binary main=/tmp/termshot-main/termshot --binary branch=/tmp/termshot-branch \
    --describe main=431ed23 --describe branch=82c3f3d \
    --reference main --runs 40 --warmups 5 --memory-runs 5 \
    --verify-identical --cjk-font "$cjk" \
    --seed "${batch#*:}" --output "/tmp/termshot-${batch%%:*}.json"
done
python3 scripts/bench-report.py /tmp/termshot-a.json /tmp/termshot-b.json
bench/c-vs-rust/run.sh images 61
```

## Box drawing in Rust (2026-10-04, `629a4d4`, #12 step 2a)

#12 step 2a moves box drawing, block elements and the stroke cache (`Stamps`)
from `src/draw.c` to `src/geometry.rs`; draw.c calls it once per cell. The
bar was no regression beyond noise on the geometry cases and `reply-sent`.
Every pixel is the same: `bench/c-vs-rust/run.sh geometry` paints all 160
characters with draw.c as of `24d71fe` and with the Rust (1,442,400 cells,
every cell size up to 40x100 and the widest row and tallest column at every
`--px`, cached and not) and the canvases are byte-identical on both hosts;
all 48 cases below gave one output per case on both binaries and batches, and
every PNG `./test.sh` writes (399 files) hashes the same as on main, on both
hosts.

### Result

| | macOS arm64 (M2 Max) | Linux x86-64 (Ryzen 7 8745HS) |
| --- | --- | --- |
| end to end, 48 cases, paired wall speedup (main/branch) | 0.977-1.037 over both batches | 0.823-1.084 over both batches (batch B under a rising load) |
| slower with confidence in both batches | none | `thai-combining` 0.973 / 0.972 (parsing; see below) |
| faster with confidence in both batches | none | `geometry-all` 1.010 / 1.010, `cursor-moves` 1.018 / 1.018 |
| geometry alone (`run.sh geometry 61`), Rust ÷ C | 0.970-1.008 | 0.860-1.039 |

The six cases asked about, wall median / p95 (ms) from batch A, the paired
speedup (main/branch, above 1 is faster) with its 95% bootstrap interval for
both batches, and the `geometry` stage (batch A medians):

macOS arm64:

| case | main wall | branch wall | speedup A | speedup B | geometry_ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| `rounded-boxes` | 10.81 / 17.11 | 11.00 / 17.71 | 0.983 [0.965, 1.020] | 1.004 [0.983, 1.018] | 1.38 → 1.32 |
| `rounded-128px` | 31.87 / 32.71 | 31.99 / 32.66 | 0.997 [0.990, 1.004] | 1.002 [0.993, 1.009] | 4.87 → 4.71 |
| `geometry-all` | 91.54 / 92.99 | 91.71 / 93.49 | 0.998 [0.993, 1.003] | 0.996 [0.993, 1.001] | 3.41 → 3.45 |
| `box-grid` | 7.54 / 8.06 | 7.55 / 7.89 | 0.998 [0.985, 1.017] | 1.004 [0.987, 1.024] | 0.17 → 0.18 |
| `block-grid` | 10.61 / 11.30 | 10.63 / 11.31 | 0.999 [0.985, 1.008] | 1.008 [0.988, 1.028] | 0.62 → 0.64 |
| `reply-sent` | 8.79 / 9.07 | 8.90 / 9.38 | 0.987 [0.976, 0.999] | 1.001 [0.985, 1.026] | 0.06 → 0.06 |

Linux x86-64:

| case | main wall | branch wall | speedup A | speedup B | geometry_ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| `rounded-boxes` | 8.18 / 8.79 | 8.23 / 8.94 | 0.993 [0.982, 1.012] | 1.016 [0.987, 1.043] | 1.38 → 1.36 |
| `rounded-128px` | 23.46 / 24.98 | 23.00 / 24.23 | 1.020 [1.009, 1.041] | 1.016 [0.997, 1.029] | 3.78 → 3.28 |
| `geometry-all` | 77.43 / 79.87 | 76.70 / 80.32 | 1.010 [1.004, 1.015] | 1.010 [1.003, 1.023] | 3.36 → 3.19 |
| `box-grid` | 5.96 / 6.37 | 5.93 / 6.44 | 1.005 [0.984, 1.031] | 0.994 [0.964, 1.028] | 0.19 → 0.18 |
| `block-grid` | 9.51 / 10.10 | 9.58 / 10.34 | 0.993 [0.972, 1.018] | 0.938 [0.863, 1.033] | 1.33 → 1.39 |
| `reply-sent` | 7.68 / 8.84 | 7.52 / 8.50 | 1.021 [0.986, 1.043] | 1.004 [0.976, 1.029] | 0.07 → 0.07 |

`bench-report.py` on the raw files gives every case. No geometry case is
slower with confidence in both batches on either host. `block-grid` batch B
on Linux (0.938, interval [0.863, 1.033]) ran while the load climbed from 3.5
to 5.6; its geometry stage there went 1.89 → 1.77 ms, faster.

`thai-combining` on Linux is 2.7% slower in both batches, and
`text-mixed-unicode` 1.4-1.6%; both are parser-bound (`parse` 17.88 → 18.26
ms in batch A), and `text-mixed-unicode` writes no PNG, so it never reaches
geometry. The parser's source is unchanged; what changed is the crate it is
compiled in, which gained a module, and so how rustc splits it into codegen
units. `perf stat -r 30 -e instructions:u,cycles:u` on starship, the same
logs (`thai-combining` with a PNG, `mixed-unicode` with `--text` only):

| log | main | branch | main, 1 codegen unit | branch, 1 codegen unit |
| --- | ---: | ---: | ---: | ---: |
| `thai-combining` instructions | 366,433,457 | 371,024,775 (+1.25%) | 386,479,352 | 386,280,724 (-0.05%) |
| `thai-combining` cycles | 149.3 M | 154.0 M (+3.1%) | 151.9 M | 145.0 M (-4.5%) |
| `mixed-unicode --text` instructions | 350,532,256 | 355,397,803 (+1.39%) | 410,217,965 | 410,217,532 (-0.0001%) |
| `mixed-unicode --text` cycles | 156.6 M | 164.4 M (+5.0%) | 174.5 M | 168.0 M (-3.7%) |

Built as one codegen unit, the two binaries run the same instructions
(cycles ±0.3% over 30 runs, at a load of 18). The shipped difference is
therefore rustc's partitioning of the crate, which the new module moved,
not anything the parser or geometry does; it is accepted, since
`-C codegen-units=1` itself costs 5-17% more instructions on these logs. macOS
shows neither case slower (1.000 / 0.997 and 1.000 / 1.000).

### Geometry alone

`bench/c-vs-rust/run.sh geometry 61` paints bench.py's geometry screens with
both after the pixel check, in alternating rounds, median ms per screen:

| screen | Apple clang 21 C | Rust | Rust ÷ C | GCC 16 C | Rust | Rust ÷ C |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `geometry-all` 240x80 | 4.039 | 4.026 | 0.997 | 8.887 | 8.929 | 1.005 |
| `rounded-boxes` | 0.985 | 0.983 | 0.998 | 1.044 | 1.033 | 0.989 |
| `rounded-128px` | 5.784 | 5.608 | 0.970 | 5.673 | 5.114 | 0.901 |
| `box-grid` | 0.196 | 0.192 | 0.980 | 0.191 | 0.164 | 0.860 |
| `block-grid` | 0.485 | 0.489 | 1.008 | 0.683 | 0.710 | 1.039 |

macOS ran at a load of 6.6 → 6.9, Linux at 2.6 → 2.6. Three changes from a
straight port were needed to get there; each was measured with this harness
and kept:

1. **Fills write through a pointer.** A rectangle, a kept stroke's run and a
   pixel are each checked to be on the canvas once (`clip_rect`, the cell, or
   `put`'s own test); the row loop then stores 3 bytes a pixel, as the C
   does. Slice indexing per pixel, or a checked span per row, was up to 25%
   slower than the C on `box-grid` (short bars) and 7% on `rounded-128px`
   (short runs).
2. **The disc test writes the mask through a row slice**, so the inner loop
   has no bounds check: `rounded-128px` went from 1.07 to 0.97-0.98 of the C
   on the M2.
3. **Shades blend 16 pixels at a time** against the colour repeated as often,
   in u16 (`629a4d4`). A pixel at a time the blend didn't vectorize on x86-64,
   and `block-grid` was 1.28 of GCC's C; now 1.04, and unchanged on arm64.
   Filling from a 16-pixel pattern was tried for plain fills too and
   rejected: on Linux it took `geometry-all` from 1.02 to 1.00-1.01 but
   `box-grid` from 0.85 to 1.06, and on the M2 `box-grid` and `block-grid`
   to 1.27 and 1.35.

### What was measured

- **main**: `24d71fe` (main after #73), built with `./build.sh`.
- **branch**: `629a4d4`, the step 2a commits on top of it. Later commits
  change documentation and add one check per call outside the paint loops
  (`f967bfc`: a canvas with a null pixel pointer paints nothing).

Each host built both binaries itself (on Linux from fresh clones of the
pushed branch and main in a temp dir, since removed). Method as in the
[step 1 round](#png-compression-in-rust-2026-10-04-6396b8d-12-step-1):
`bench.py` with every suite (the `draw` workloads included), 5 warmups and
40 shuffled rounds of plain and profiled runs per case and binary, 5 peak-RSS
runs, seeds 17 (batch A) and 29 (batch B), `--verify-identical`, and the full
CJK collection (sha256 `b76b0433…`) on both hosts.

| | macOS arm64 | Linux x86-64 |
| --- | --- | --- |
| host | `lawrences-mac-studio`, macOS 26.6.2 | `starship`, kernel 7.2.5-3-omarchy, glibc 2.44, governor `performance` |
| compilers | rustc 1.98.1, Apple clang 21.0.0 (clang-2100.3.34.2) | rustc 1.98.1, GCC 16.2.1 20260810 |
| main / branch sha256 | `a8832d9980d4…` / `627fc0ec3fc0…` | `78c0f613a4c1…` / `6f6bd8017f4d…` |
| load average (1 min), start → end | A 7.90 → 7.47, B 7.47 → 6.73: a shared desktop, other sessions busy | A 2.54 → 3.47, B 3.47 → 5.63 |

Raw results: macOS [batch A](performance-2026-10-04-geometry-rust-macos-a.json)
and [batch B](performance-2026-10-04-geometry-rust-macos-b.json); Linux
[batch A](performance-2026-10-04-geometry-rust-linux-a.json) and
[batch B](performance-2026-10-04-geometry-rust-linux-b.json). They hold every
sample; none was discarded.

### Validation

- `./test.sh`, `SANITIZE=1 ./test.sh`, `SANITIZE=1 ./tests/run.sh` and
  `./test.sh` under rustc 1.70.0 on macOS; `./test.sh`, `./tests/run.sh` and
  `bench/c-vs-rust/run.sh geometry` on starship (x86-64, GCC 16).
- `tests/boxes.c` (37,804 checks: each character against its Unicode name,
  and reused strokes against fresh ones in every cell of a 500-column row and
  a 200-row column) runs against the Rust, linked as a static library.
- The unit tests fail each of the cache's allocations in turn (the stroke
  grid that was `tests/stamps_alloc.c`; every site fails at least once) and
  shrink its budget from 4 MiB to 0: the pixels never change and the cache
  never holds more than its budget. `tests/run.sh` does the same through the
  CLI with a `--cfg termshot_alloc_faults` build: each of 71 failures draws
  the same PNG.
- `bench/c-vs-rust/sincos.c`: on macOS, `__sincosf_stret` (what clang makes
  of draw.c's `sinf` and `cosf` of one angle) differs from `cosf` in the last
  bit for 2,549,753 of the 2,154,089,679 floats from -1.6 to 4.75, and for
  2.6% of the angles arcs of 12 to 4,000 steps take. LLVM merged Rust's
  `f32::sin` and `f32::cos` at some call sites and not others, so
  `geometry.rs` calls `__sincosf_stret` (macOS) or `sincosf` (Linux) by name.
  glibc 2.44's `sincosf` agrees with its `sinf` and `cosf` on every one.
- `scripts/release.sh macos-universal`: both slices render the samples like
  the host build; the x86_64 slice (Rosetta), main and the branch also render
  `geometry-all` and a screen of every rounded corner and diagonal alike at
  13 sizes from px 1 to 255.

### Remaining limits

- One batch pair per host, and the Mac under a load of 6.7-7.9.
- `block-grid`'s geometry is 4% slower than GCC's C on x86-64 (0.7 ms of a
  9.5 ms run).
- The musl release builds were not run locally; `release.yml` builds and
  checks them on this pull request. musl's `sincosf` was not compared with
  its `sinf` and `cosf`; the release check renders the samples, which
  include rounded corners, like the host build.

### Reproduce

```sh
git worktree add /tmp/termshot-main 24d71fe && (cd /tmp/termshot-main && ./build.sh)
./build.sh && cp termshot /tmp/termshot-branch
cjk=/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc   # on macOS, a copy (same sha256)
for batch in a:17 b:29; do
  python3 scripts/bench.py \
    --binary main=/tmp/termshot-main/termshot --binary branch=/tmp/termshot-branch \
    --describe main=24d71fe --describe branch=629a4d4 \
    --reference main --runs 40 --warmups 5 --memory-runs 5 \
    --verify-identical --cjk-font "$cjk" \
    --seed "${batch#*:}" --output "/tmp/termshot-${batch%%:*}.json"
done
python3 scripts/bench-report.py /tmp/termshot-a.json /tmp/termshot-b.json
bench/c-vs-rust/run.sh geometry 61
cc -O2 bench/c-vs-rust/sincos.c -lm -o /tmp/sincos && /tmp/sincos
```

## PNG compression in Rust (2026-10-04, `6396b8d`, #12 step 1)

#12 step 1 moves the compressor from `src/deflate.c` to `src/deflate.rs`.
stb_image_write, still C, calls it through `STBIW_ZLIB_COMPRESS`. The bar was
no regression beyond noise on `reply-sent`. The stream is the same for every
input and quality (`tests/deflate_diff.c` against stock stb, now linked with
the Rust), so every PNG is byte-identical: all 48 cases below, on both binaries,
both batches and both hosts, gave one output each, and every PNG `./test.sh`
writes (399 files) hashes the same as on main.

### Result

| | macOS arm64 (M2 Max) | Linux x86-64 (Ryzen 7 8745HS) |
| --- | --- | --- |
| end to end, 48 cases, paired wall speedup (main/branch) | 0.965-1.052 (A), 0.980-1.047 (B) | 0.961-1.167 (A), 0.989-1.173 (B) |
| `reply-sent` wall median, main → branch | 8.96 → 8.87 ms, 1.014 [1.002, 1.028] / 1.004 [0.991, 1.029] | 7.71 → 7.29 ms, 1.052 [1.032, 1.073] / 1.060 [1.026, 1.082] |
| Adler-32, `reply-128px` (stage median) | 3.51 → 3.50 ms | 3.55 → 2.74 ms |
| matching and emission, `reply-128px` | 11.48 → 11.13 ms | 10.98 → 10.40 ms |
| slower with confidence in both batches | `cjk-cff-primary` 0.984 / 0.980, `cjk-overflow-full` 0.988 / 0.989, `large-color` 0.991 / 0.992 | none |

`reply-sent` is not slower on either host. On Linux it is 5-6% faster: the
SSE2 Adler-32 (0.50 → 0.38 ms) and the matcher. On macOS three cases are 1-2%
slower in both batches. All three are dominated by matching on glyph-heavy
images (`deflate_match_emit` +1.3-2.8%: 3.94 → 4.05, 7.73 → 7.91, 117.71 →
119.23 ms), which is the LLVM-against-LLVM matching gap of
[docs/c-vs-rust.md](c-vs-rust.md) that this round narrowed but did not
close. It is accepted: it is at most 2% of a run, it does not reach
`reply-sent`, and the geometry-heavy cases gain more (`geometry-all` 1.052 / 1.047,
`rounded-128px` 1.031 / 1.034).

### What was measured

- **main**: `63d6de8` (main after #72), built with `./build.sh`.
- **branch**: `6396b8d`, the compressor commits on top of it. The later
  commits change only `cfg`-gated test code (the fault hook's visibility for
  rustc 1.70), scripts and documentation, so the shipped binary is the one
  measured.

Each host built both binaries itself (on Linux from fresh clones of the
pushed branch and main in a temp dir, since removed). Method and harness are
those of the [#20 round](#png-compression-adler-32-and-deflate-matching-2026-10-03-3bf2ffc-20):
`bench.py` with every suite, 5 warmups and 40 shuffled rounds of plain and
profiled runs per case and binary, 5 peak-RSS runs, seeds 17 (batch A) and 29
(batch B), `--verify-identical`, and the full CJK collection (same sha256,
`b76b0433…`) on both hosts.

| | macOS arm64 | Linux x86-64 |
| --- | --- | --- |
| host | `lawrences-mac-studio`, macOS 26.6.2 | `starship`, kernel 7.2.5-3-omarchy, glibc 2.44, governor `performance` |
| compilers | rustc 1.98.1, Apple clang 21.0.0 (clang-2100.3.34.2) | rustc 1.98.1, GCC 16.2.1 20260810 |
| main / branch sha256 | `2fb0770600c5…` / `1df1d063be27…` | `e6512e8675b7…` / `a922cbe41e1c…` |
| load average (1 min), start → end | A 3.42 → 5.88, B 5.88 → 4.48: a shared desktop, other sessions busy | A 1.08 → 1.30, B 1.30 → 1.38 |

Raw results: macOS [batch A](performance-2026-10-04-deflate-rust-macos-a.json)
and [batch B](performance-2026-10-04-deflate-rust-macos-b.json); Linux
[batch A](performance-2026-10-04-deflate-rust-linux-a.json) and
[batch B](performance-2026-10-04-deflate-rust-linux-b.json). They hold every
sample; none was discarded. Peak RSS medians differ by at most 1.4 MiB
(-1.36 MiB `cjk-overflow-full`, Linux batch A); the compressor allocates as
the C did.

### End-to-end results

Wall median / p95 (ms) from batch A, paired speedup (main/branch, above 1 is
faster) with its 95% bootstrap interval for both batches, and the PNG stages
(batch A medians, main → branch). `png_encode` holds the other two. Text-only
cases write no PNG.

macOS arm64:

| case | main wall | branch wall | speedup A | speedup B | match_emit | checksum | png_encode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 9.29 / 10.07 | 9.23 / 10.02 | 0.997 [0.976, 1.028] | 1.012 [0.990, 1.029] | 2.80 → 2.74 | 0.48 → 0.48 | 3.40 → 3.35 |
| `font-file` | 8.89 / 9.48 | 8.96 / 9.84 | 0.996 [0.981, 1.006] | 1.001 [0.987, 1.017] | 2.72 → 2.71 | 0.47 → 0.48 | 3.31 → 3.33 |
| `cjk-none` | 5.12 / 5.66 | 5.09 / 5.67 | 1.004 [0.971, 1.036] | 1.007 [0.997, 1.019] | 0.90 → 0.86 | 0.12 → 0.12 | 1.06 → 1.01 |
| `cjk-subset` | 9.64 / 10.35 | 9.85 / 10.33 | 0.995 [0.967, 1.011] | 0.987 [0.968, 0.997] | 4.09 → 4.22 | 0.12 → 0.12 | 4.39 → 4.51 |
| `cjk-cff-primary` | 9.19 / 9.70 | 9.34 / 9.71 | 0.984 [0.969, 0.997] | 0.980 [0.970, 1.000] | 3.94 → 4.05 | 0.13 → 0.13 | 4.25 → 4.37 |
| `mixed-subset` | 7.74 / 7.94 | 7.80 / 8.17 | 0.990 [0.972, 1.015] | 1.002 [0.988, 1.014] | 2.28 → 2.29 | 0.12 → 0.12 | 2.50 → 2.51 |
| `glyph-overflow` | 16.63 / 17.57 | 16.82 / 17.24 | 0.991 [0.984, 1.001] | 0.994 [0.985, 1.003] | 6.04 → 6.04 | 0.12 → 0.12 | 6.39 → 6.37 |
| `cjk-full` | 12.99 / 13.40 | 13.11 / 13.53 | 0.987 [0.977, 1.002] | 0.994 [0.982, 1.004] | 4.09 → 4.20 | 0.12 → 0.12 | 4.40 → 4.50 |
| `mixed-full` | 11.50 / 11.95 | 11.43 / 11.88 | 1.008 [0.997, 1.018] | 1.005 [0.989, 1.018] | 2.46 → 2.45 | 0.12 → 0.12 | 2.68 → 2.66 |
| `cjk-overflow-full` | 27.06 / 27.54 | 27.39 / 28.21 | 0.988 [0.978, 0.994] | 0.989 [0.981, 0.995] | 7.73 → 7.91 | 0.12 → 0.12 | 8.16 → 8.34 |
| `reply-sent` | 8.96 / 9.57 | 8.87 / 9.22 | 1.014 [1.002, 1.028] | 1.004 [0.991, 1.029] | 2.73 → 2.74 | 0.47 → 0.48 | 3.33 → 3.35 |
| `draft-ready` | 9.02 / 9.69 | 9.04 / 9.97 | 1.001 [0.979, 1.017] | 1.000 [0.977, 1.020] | 2.73 → 2.70 | 0.48 → 0.48 | 3.34 → 3.31 |
| `reply-24px` | 5.78 / 6.18 | 5.77 / 6.13 | 1.013 [0.990, 1.024] | 1.003 [0.985, 1.041] | 1.25 → 1.25 | 0.12 → 0.12 | 1.42 → 1.42 |
| `reply-128px` | 29.56 / 30.32 | 29.24 / 29.79 | 1.019 [1.005, 1.028] | 1.021 [1.008, 1.028] | 11.48 → 11.13 | 3.51 → 3.50 | 15.56 → 15.16 |
| `real-shell` | 6.55 / 6.89 | 6.51 / 7.00 | 0.994 [0.979, 1.023] | 1.010 [0.991, 1.019] | 1.32 → 1.27 | 0.31 → 0.30 | 1.70 → 1.64 |
| `real-less` | 6.02 / 6.80 | 6.10 / 6.43 | 1.007 [0.990, 1.026] | 0.998 [0.982, 1.029] | 1.00 → 0.97 | 0.31 → 0.31 | 1.36 → 1.33 |
| `real-vi` | 8.03 / 8.43 | 8.06 / 8.37 | 1.000 [0.983, 1.009] | 0.996 [0.985, 1.013] | 2.53 → 2.51 | 0.30 → 0.31 | 2.95 → 2.93 |
| `blank` | 6.14 / 6.43 | 5.95 / 6.30 | 1.027 [0.997, 1.044] | 1.009 [0.988, 1.038] | 0.76 → 0.70 | 0.48 → 0.48 | 1.30 → 1.25 |
| `color-grid` | 18.03 / 18.76 | 18.20 / 18.79 | 0.991 [0.982, 0.997] | 1.001 [0.993, 1.008] | 11.65 → 11.89 | 0.12 → 0.12 | 12.19 → 12.39 |
| `ascii-overflow` | 6.05 / 6.33 | 6.01 / 6.33 | 1.005 [0.988, 1.024] | 1.013 [1.001, 1.040] | 0.40 → 0.38 | 0.12 → 0.12 | 0.55 → 0.53 |
| `rounded-boxes` | 10.44 / 11.22 | 10.37 / 10.80 | 1.011 [0.993, 1.037] | 1.028 [1.017, 1.043] | 3.57 → 3.34 | 0.48 → 0.48 | 4.15 → 3.92 |
| `dense` | 12.05 / 12.49 | 12.08 / 12.49 | 0.993 [0.983, 1.002] | 1.012 [0.998, 1.016] | 5.19 → 5.16 | 0.49 → 0.48 | 5.90 → 5.85 |
| `ansi-replay` | 18.79 / 19.35 | 18.88 / 19.29 | 0.996 [0.987, 1.007] | 1.004 [0.997, 1.007] | 2.78 → 2.74 | 0.48 → 0.48 | 3.40 → 3.37 |
| `large` | 38.45 / 39.23 | 38.16 / 38.98 | 1.012 [1.006, 1.020] | 1.015 [1.003, 1.019] | 18.35 → 17.94 | 3.15 → 3.15 | 22.36 → 21.95 |
| `unicode` | 7.99 / 8.49 | 7.95 / 8.36 | 1.010 [0.990, 1.024] | 1.006 [0.988, 1.024] | 2.55 → 2.50 | 0.12 → 0.12 | 2.76 → 2.73 |
| `box-grid` | 7.75 / 8.22 | 7.54 / 7.74 | 1.030 [1.020, 1.052] | 1.026 [1.009, 1.046] | 1.82 → 1.66 | 0.49 → 0.49 | 2.38 → 2.24 |
| `block-grid` | 10.75 / 11.19 | 10.69 / 11.14 | 0.999 [0.986, 1.020] | 1.015 [1.005, 1.034] | 4.20 → 4.09 | 0.47 → 0.47 | 4.81 → 4.71 |
| `rounded-panes` | 10.11 / 10.79 | 10.05 / 10.52 | 1.008 [0.996, 1.032] | 1.019 [1.009, 1.035] | 3.42 → 3.31 | 0.49 → 0.49 | 4.05 → 3.93 |
| `rounded-24px` | 5.80 / 6.06 | 5.70 / 6.06 | 1.013 [0.993, 1.031] | 1.025 [0.980, 1.038] | 1.25 → 1.19 | 0.12 → 0.12 | 1.40 → 1.34 |
| `rounded-128px` | 33.00 / 33.55 | 31.99 / 32.78 | 1.031 [1.023, 1.036] | 1.034 [1.027, 1.037] | 12.66 → 11.80 | 3.49 → 3.48 | 16.73 → 15.80 |
| `geometry-all` | 95.10 / 97.36 | 90.54 / 92.60 | 1.052 [1.047, 1.057] | 1.047 [1.044, 1.051] | 75.48 → 70.85 | 3.15 → 3.19 | 79.59 → 75.03 |
| `large-sparse` | 21.31 / 21.90 | 20.71 / 21.39 | 1.026 [1.011, 1.040] | 1.015 [1.009, 1.024] | 5.88 → 5.58 | 3.12 → 3.14 | 9.37 → 9.10 |
| `large-color` | 145.63 / 148.19 | 146.96 / 149.50 | 0.991 [0.990, 0.995] | 0.992 [0.989, 0.996] | 117.71 → 119.23 | 3.25 → 3.28 | 124.86 → 126.45 |
| `image-below` | 22.60 / 23.76 | 22.69 / 25.55 | 0.995 [0.983, 1.006] | 0.996 [0.986, 1.002] | 12.36 → 12.54 | 0.50 → 0.51 | 13.32 → 13.51 |
| `image-under` | 27.03 / 40.18 | 27.14 / 35.90 | 0.995 [0.981, 1.009] | 0.995 [0.986, 1.000] | 15.79 → 16.11 | 0.52 → 0.53 | 16.95 → 17.27 |
| `image-over` | 25.65 / 32.83 | 26.74 / 33.00 | 0.965 [0.937, 1.013] | 0.990 [0.984, 0.999] | 10.48 → 10.66 | 0.50 → 0.49 | 11.48 → 11.62 |
| `dense-sgr` | 31.35 / 32.43 | 31.50 / 32.47 | 0.998 [0.993, 1.004] | 0.998 [0.992, 1.001] | 12.51 → 12.68 | 0.12 → 0.12 | 13.02 → 13.17 |
| `cursor-moves` | 27.81 / 29.02 | 27.82 / 28.32 | 0.998 [0.992, 1.004] | 0.998 [0.993, 1.000] | 5.79 → 5.89 | 0.12 → 0.12 | 6.14 → 6.24 |
| `scrolling` | 9.50 / 9.89 | 9.63 / 9.91 | 0.987 [0.974, 1.005] | 0.993 [0.988, 1.005] | 1.43 → 1.40 | 0.12 → 0.12 | 1.60 → 1.58 |
| `mixed-unicode` | 29.58 / 30.67 | 29.56 / 30.13 | 1.004 [0.999, 1.008] | 1.000 [0.994, 1.007] | 2.32 → 2.32 | 0.12 → 0.12 | 2.53 → 2.51 |
| `thai-combining` | 24.01 / 24.72 | 24.05 / 25.46 | 0.999 [0.992, 1.007] | 1.001 [0.994, 1.007] | 0.94 → 0.88 | 0.12 → 0.12 | 1.09 → 1.04 |
| `text-ansi-replay` | 13.44 / 13.80 | 13.40 / 13.71 | 1.003 [0.992, 1.009] | 1.001 [0.985, 1.009] | — | — | — |
| `text-ascii-overflow` | 4.89 / 5.31 | 4.93 / 5.22 | 0.996 [0.981, 1.012] | 1.015 [0.994, 1.031] | — | — | — |
| `text-dense-sgr` | 16.49 / 16.97 | 16.47 / 16.85 | 1.000 [0.994, 1.007] | 0.992 [0.982, 1.002] | — | — | — |
| `text-mixed-unicode` | 26.70 / 27.28 | 26.64 / 27.06 | 1.006 [1.000, 1.010] | 1.000 [0.995, 1.006] | — | — | — |
| `text-reply-sent` | 3.12 / 3.33 | 3.13 / 3.60 | 0.997 [0.973, 1.037] | 1.014 [0.977, 1.030] | — | — | — |
| `text-kitty` | 3.84 / 4.31 | 3.89 / 4.26 | 0.989 [0.973, 1.011] | 1.009 [0.983, 1.032] | — | — | — |
| `text-sixel` | 3.36 / 3.64 | 3.31 / 3.57 | 1.013 [0.997, 1.033] | 1.037 [0.999, 1.059] | — | — | — |

Linux x86-64:

| case | main wall | branch wall | speedup A | speedup B | match_emit | checksum | png_encode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 7.55 / 8.21 | 7.23 / 7.78 | 1.047 [1.009, 1.087] | 1.037 [1.011, 1.063] | 3.43 → 2.93 | 0.50 → 0.38 | 4.18 → 3.56 |
| `font-file` | 7.64 / 8.28 | 7.42 / 7.90 | 1.041 [1.013, 1.067] | 1.063 [1.026, 1.081] | 3.28 → 2.94 | 0.50 → 0.38 | 4.02 → 3.54 |
| `cjk-none` | 4.24 / 4.80 | 4.18 / 4.65 | 1.006 [0.950, 1.098] | 1.087 [1.010, 1.143] | 1.14 → 1.08 | 0.12 → 0.10 | 1.37 → 1.26 |
| `cjk-subset` | 9.75 / 10.55 | 9.47 / 10.22 | 1.029 [1.009, 1.057] | 1.015 [0.998, 1.041] | 4.58 → 4.33 | 0.12 → 0.10 | 4.92 → 4.71 |
| `cjk-cff-primary` | 8.45 / 9.37 | 8.51 / 9.28 | 0.986 [0.953, 1.033] | 1.049 [1.010, 1.079] | 4.60 → 4.22 | 0.14 → 0.11 | 5.02 → 4.62 |
| `mixed-subset` | 7.45 / 8.07 | 7.14 / 7.82 | 1.041 [0.998, 1.065] | 1.029 [1.008, 1.065] | 2.70 → 2.47 | 0.12 → 0.10 | 2.96 → 2.74 |
| `glyph-overflow` | 15.29 / 16.02 | 14.74 / 15.32 | 1.048 [1.025, 1.063] | 1.033 [1.002, 1.055] | 6.34 → 5.89 | 0.12 → 0.10 | 6.74 → 6.27 |
| `cjk-full` | 13.74 / 14.49 | 13.56 / 14.57 | 1.002 [0.976, 1.022] | 1.036 [0.989, 1.062] | 4.59 → 4.35 | 0.12 → 0.09 | 4.95 → 4.71 |
| `mixed-full` | 11.56 / 12.29 | 11.70 / 12.54 | 0.984 [0.953, 1.009] | 1.015 [0.992, 1.038] | 2.82 → 2.60 | 0.12 → 0.09 | 3.10 → 2.85 |
| `cjk-overflow-full` | 26.64 / 27.78 | 26.28 / 27.56 | 1.019 [1.002, 1.030] | 1.007 [0.998, 1.025] | 8.16 → 7.80 | 0.12 → 0.10 | 8.65 → 8.28 |
| `reply-sent` | 7.71 / 8.35 | 7.29 / 8.01 | 1.052 [1.032, 1.073] | 1.060 [1.026, 1.082] | 3.16 → 2.91 | 0.50 → 0.38 | 3.90 → 3.62 |
| `draft-ready` | 7.70 / 8.53 | 7.34 / 7.95 | 1.041 [1.026, 1.089] | 1.055 [1.017, 1.074] | 3.15 → 2.94 | 0.50 → 0.38 | 3.92 → 3.65 |
| `reply-24px` | 5.11 / 5.68 | 4.74 / 5.50 | 1.064 [1.007, 1.128] | 1.060 [0.996, 1.119] | 1.58 → 1.44 | 0.12 → 0.09 | 1.88 → 1.66 |
| `reply-128px` | 24.11 / 24.89 | 22.53 / 23.19 | 1.066 [1.057, 1.075] | 1.067 [1.056, 1.086] | 10.98 → 10.40 | 3.55 → 2.74 | 15.50 → 14.06 |
| `real-shell` | 5.42 / 6.12 | 5.26 / 6.04 | 1.033 [1.003, 1.077] | 1.050 [1.023, 1.069] | 1.57 → 1.50 | 0.32 → 0.24 | 2.05 → 1.89 |
| `real-less` | 4.94 / 5.54 | 4.74 / 5.17 | 1.041 [1.012, 1.069] | 1.038 [0.993, 1.084] | 1.23 → 1.22 | 0.32 → 0.24 | 1.69 → 1.61 |
| `real-vi` | 7.15 / 7.69 | 6.79 / 7.29 | 1.054 [1.032, 1.090] | 1.066 [1.033, 1.098] | 3.02 → 2.71 | 0.32 → 0.24 | 3.51 → 3.15 |
| `blank` | 4.10 / 4.52 | 4.19 / 4.51 | 0.983 [0.955, 1.041] | 1.034 [0.981, 1.087] | 0.66 → 0.68 | 0.50 → 0.38 | 1.26 → 1.16 |
| `color-grid` | 16.77 / 17.74 | 16.10 / 16.98 | 1.032 [1.014, 1.051] | 1.033 [1.011, 1.049] | 11.61 → 11.19 | 0.12 → 0.10 | 12.20 → 11.77 |
| `ascii-overflow` | 6.14 / 7.14 | 6.10 / 6.63 | 1.002 [0.971, 1.045] | 0.993 [0.965, 1.000] | 0.49 → 0.46 | 0.12 → 0.09 | 0.66 → 0.64 |
| `rounded-boxes` | 8.38 / 9.27 | 7.83 / 8.28 | 1.076 [1.046, 1.101] | 1.099 [1.081, 1.123] | 3.68 → 3.09 | 0.50 → 0.39 | 4.27 → 3.59 |
| `dense` | 10.71 / 11.21 | 10.30 / 10.94 | 1.049 [1.022, 1.068] | 1.046 [1.030, 1.072] | 5.95 → 5.20 | 0.50 → 0.39 | 6.75 → 5.92 |
| `ansi-replay` | 15.88 / 16.83 | 15.52 / 17.08 | 1.021 [1.006, 1.036] | 1.033 [1.016, 1.050] | 3.06 → 2.79 | 0.50 → 0.38 | 3.65 → 3.31 |
| `large` | 33.96 / 35.36 | 31.87 / 33.22 | 1.076 [1.060, 1.086] | 1.073 [1.054, 1.085] | 17.72 → 16.43 | 3.24 → 2.50 | 22.22 → 20.11 |
| `unicode` | 7.18 / 7.94 | 6.99 / 7.64 | 1.038 [0.998, 1.077] | 1.022 [0.991, 1.069] | 2.90 → 2.73 | 0.12 → 0.09 | 3.17 → 3.04 |
| `box-grid` | 6.01 / 6.82 | 5.62 / 6.10 | 1.074 [1.026, 1.102] | 1.085 [1.070, 1.115] | 1.87 → 1.75 | 0.50 → 0.38 | 2.56 → 2.30 |
| `block-grid` | 9.55 / 10.16 | 8.87 / 9.44 | 1.083 [1.046, 1.106] | 1.078 [1.058, 1.102] | 4.79 → 4.23 | 0.50 → 0.39 | 5.66 → 4.87 |
| `rounded-panes` | 8.63 / 9.35 | 8.24 / 9.12 | 1.057 [1.016, 1.082] | 1.086 [1.067, 1.113] | 3.80 → 3.39 | 0.50 → 0.38 | 4.49 → 4.01 |
| `rounded-24px` | 4.66 / 5.38 | 4.59 / 4.97 | 1.022 [0.969, 1.089] | 1.057 [1.009, 1.101] | 1.37 → 1.07 | 0.12 → 0.10 | 1.58 → 1.22 |
| `rounded-128px` | 25.68 / 27.12 | 23.16 / 25.32 | 1.100 [1.088, 1.122] | 1.086 [1.075, 1.104] | 10.72 → 9.47 | 3.55 → 2.74 | 14.98 → 13.07 |
| `geometry-all` | 90.16 / 92.30 | 77.15 / 78.65 | 1.167 [1.157, 1.172] | 1.173 [1.165, 1.182] | 75.06 → 62.92 | 3.23 → 2.50 | 79.52 → 66.65 |
| `large-sparse` | 15.48 / 16.17 | 14.70 / 15.53 | 1.051 [1.042, 1.070] | 1.042 [1.020, 1.050] | 5.00 → 5.06 | 3.24 → 2.50 | 8.77 → 8.13 |
| `large-color` | 134.90 / 137.04 | 126.73 / 127.81 | 1.065 [1.060, 1.071] | 1.062 [1.055, 1.065] | 110.42 → 102.93 | 3.25 → 2.50 | 117.17 → 108.84 |
| `image-below` | 21.49 / 22.44 | 20.08 / 20.98 | 1.056 [1.046, 1.084] | 1.069 [1.061, 1.081] | 12.95 → 11.90 | 0.50 → 0.39 | 14.05 → 12.87 |
| `image-under` | 24.34 / 25.62 | 22.99 / 23.75 | 1.072 [1.054, 1.081] | 1.049 [1.037, 1.061] | 15.42 → 14.49 | 0.50 → 0.39 | 16.62 → 15.59 |
| `image-over` | 19.74 / 20.94 | 18.69 / 20.21 | 1.056 [1.039, 1.076] | 1.046 [1.039, 1.060] | 11.13 → 10.22 | 0.50 → 0.39 | 12.16 → 11.14 |
| `dense-sgr` | 29.12 / 30.36 | 28.38 / 29.26 | 1.024 [1.015, 1.051] | 1.032 [1.013, 1.049] | 12.52 → 11.55 | 0.12 → 0.10 | 12.87 → 11.91 |
| `cursor-moves` | 26.35 / 27.33 | 26.26 / 27.31 | 1.008 [0.987, 1.018] | 1.009 [1.000, 1.028] | 6.19 → 5.67 | 0.12 → 0.10 | 6.45 → 5.95 |
| `scrolling` | 9.16 / 9.78 | 9.02 / 9.68 | 1.003 [0.973, 1.047] | 1.043 [1.020, 1.075] | 1.69 → 1.55 | 0.12 → 0.10 | 1.87 → 1.75 |
| `mixed-unicode` | 26.25 / 28.25 | 25.88 / 28.08 | 1.023 [1.012, 1.035] | 1.025 [1.010, 1.040] | 2.65 → 2.36 | 0.12 → 0.10 | 2.85 → 2.58 |
| `thai-combining` | 22.38 / 23.70 | 21.96 / 23.03 | 1.024 [1.013, 1.037] | 1.023 [1.011, 1.038] | 1.11 → 0.98 | 0.12 → 0.10 | 1.26 → 1.15 |
| `text-ansi-replay` | 10.39 / 11.09 | 10.52 / 11.21 | 0.982 [0.961, 1.013] | 0.992 [0.967, 1.020] | — | — | — |
| `text-ascii-overflow` | 3.64 / 4.22 | 3.70 / 4.20 | 0.991 [0.954, 1.035] | 0.989 [0.944, 1.050] | — | — | — |
| `text-dense-sgr` | 13.99 / 15.19 | 13.81 / 14.65 | 1.019 [0.994, 1.037] | 1.008 [0.982, 1.023] | — | — | — |
| `text-mixed-unicode` | 22.64 / 23.53 | 22.10 / 23.62 | 1.017 [1.013, 1.033] | 1.013 [0.997, 1.033] | — | — | — |
| `text-reply-sent` | 0.90 / 1.01 | 0.91 / 0.99 | 1.030 [0.981, 1.076] | 1.030 [0.982, 1.066] | — | — | — |
| `text-kitty` | 2.03 / 2.41 | 2.00 / 2.39 | 1.070 [0.980, 1.096] | 1.015 [0.928, 1.055] | — | — | — |
| `text-sixel` | 1.54 / 1.77 | 1.55 / 1.78 | 0.961 [0.913, 1.118] | 1.042 [0.985, 1.178] | — | — | — |

### The compressor alone

`bench/c-vs-rust/run.sh deflate 61` now times a sixth variant, the shipped
`src/deflate.rs` through its C entry point, against `deflate.c` as of
`a8a95e0` (the C this replaced; the newest change to it). Every variant writes
the same bytes on 3,260 cases first. Medians of 61 rounds, ms; **ship** is
shipped Rust ÷ that C, and **safe** is the 2026-10-03 port without this
round's changes, from the same runs.

| input | Apple clang 21 C | shipped | ship | safe | GCC 16 C | shipped | ship | safe | clang 22 C | shipped | ship | safe |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1-reply-px48 | 3.132 | 3.112 | 0.99 | 1.05 | 3.155 | 2.807 | 0.89 | 0.99 | 2.983 | 2.774 | 0.93 | 1.05 |
| 2-reply-px128 | 13.319 | 13.178 | 0.99 | 1.03 | 13.569 | 12.003 | 0.88 | 1.03 | 13.521 | 11.889 | 0.88 | 1.03 |
| 3-attrs-px24 | 0.219 | 0.226 | 1.03 | 1.07 | 0.226 | 0.212 | 0.94 | 1.01 | 0.211 | 0.210 | 1.00 | 1.09 |
| 4-boxes-px48 | 1.758 | 1.734 | 0.99 | 0.99 | 1.685 | 1.466 | 0.87 | 1.03 | 1.697 | 1.453 | 0.86 | 1.03 |
| 5-dense-200x60-px16 | 24.737 | 25.521 | 1.03 | 1.08 | 24.117 | 22.673 | 0.94 | 1.00 | 21.577 | 22.078 | 1.02 | 1.10 |
| 6-blank | 1.158 | 1.145 | 0.99 | 1.03 | 1.132 | 1.012 | 0.89 | 1.11 | 1.204 | 1.000 | 0.83 | 1.04 |
| 6-color-grid | 11.536 | 11.877 | 1.03 | 1.07 | 11.167 | 10.510 | 0.94 | 1.01 | 10.302 | 10.292 | 1.00 | 1.08 |
| 6-large | 19.911 | 19.531 | 0.98 | 1.03 | 19.694 | 17.509 | 0.89 | 0.99 | 18.418 | 16.970 | 0.92 | 1.05 |
| 7-random-uniform | 59.323 | 59.945 | 1.01 | 1.06 | 56.280 | 53.853 | 0.96 | 1.08 | 49.635 | 49.695 | 1.00 | 1.13 |
| 8-random-4sym-skewed | 53.447 | 51.955 | 0.97 | 1.01 | 47.840 | 45.877 | 0.96 | 0.99 | 46.677 | 46.180 | 0.99 | 1.03 |
| Adler-32 alone, reply-px48 | 0.483 | 0.483 | 1.00 | 1.01 | 0.499 | 0.386 | 0.77 | 1.21 | 0.532 | 0.375 | 0.70 | 1.14 |
| Adler-32 alone, reply-px128 | 3.411 | 3.429 | 1.01 | 1.01 | 3.551 | 2.748 | 0.77 | 1.21 | 3.775 | 2.706 | 0.72 | 1.14 |

macOS ran at a load of 5.96 → 3.90; Linux at 2.58 → 1.81 (GCC) and 1.68 →
1.42 (clang). The two changes:

1. **SSE2 Adler-32 on x86-64.** The safe 16-lane loop loads 4 bytes per `movd`
   there (docs/c-vs-rust.md). `adler32_sse2` keeps the same block sums with
   one 16-byte `movdqu` per chunk: `psadbw` adds the chunk's bytes, `pmaddwd`
   weights them 16..1, and the sum of earlier byte sums is kept as before.
   SSE2 is baseline on x86-64, so there is no detection; the arithmetic is
   integer, so the value is exact. It is 23% faster than GCC 16's C and 28-30%
   faster than clang 22's, where the safe form was 14-21% slower. arm64 keeps
   the safe form, which matches Apple clang's NEON.
2. **Two-phase bucket scan** (`6396b8d`). Until a first match the scan takes
   any match of 3 or more with no rejection test; after it, only a longer one.
   Splitting the loop there removes a per-candidate branch on which phase it is
   in, and the current position's slice is cut once per step. Same matches, same
   bytes. On the M2 it took the match-heavy inputs from 1.06-1.08 of Apple
   clang's C to 1.01-1.03; against clang 22 on x86-64, from 1.08-1.13 to
   1.00-1.02. The rest of that gap is still unexplained.

### Validation

- `./test.sh`, `SANITIZE=1 ./test.sh`, `SANITIZE=1 ./tests/run.sh` and
  `./test.sh` under rustc 1.70.0 on macOS; `./test.sh` and `./tests/run.sh` on
  starship (x86-64, GCC 16), where `deflate_diff` also runs against a build with
  `--cfg termshot_portable_adler`, the safe Adler-32 form. Both forms agree with
  stb on all 3,060 cases.
- The x86-64 unit tests and `deflate_diff` also ran on the Mac under Rosetta
  (`--target x86_64-apple-darwin`): every Adler-32 form agrees with the
  definition.
- `tests/run.sh` builds a termshot with `--cfg termshot_alloc_faults` and fails
  each compressor allocation of a `reply-sent` run in turn (5: the hash table,
  its counts, the output buffer and two growths). Each exits 2, says it ran
  out of memory, and leaves neither the PNG nor `--text` output.
- `scripts/release.sh macos-universal` on the Mac: both slices render the
  samples byte for byte like the host build (x86_64 under Rosetta).

### Remaining limits

- Matching against Apple clang's C is still 1-3% slower on match-heavy inputs,
  and the cause is not known. Bounds checks are not it (the `unchecked`
  variant buys 0-2 points).
- One batch pair per host, the Mac under a load of 3.4-5.9.
- The musl release builds were not run locally; `release.yml` builds and checks
  them on this pull request (it runs when `build.sh` or `scripts/release.sh`
  changes).

### Reproduce

```sh
git worktree add /tmp/termshot-main 63d6de8 && (cd /tmp/termshot-main && ./build.sh)
./build.sh && cp termshot /tmp/termshot-branch
cjk=/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc   # on macOS, a copy (same sha256)
for batch in a:17 b:29; do
  python3 scripts/bench.py \
    --binary main=/tmp/termshot-main/termshot --binary branch=/tmp/termshot-branch \
    --describe main=63d6de8 --describe branch=6396b8d \
    --reference main --runs 40 --warmups 5 --memory-runs 5 \
    --verify-identical --cjk-font "$cjk" \
    --seed "${batch#*:}" --output "/tmp/termshot-${batch%%:*}.json"
done
python3 scripts/bench-report.py /tmp/termshot-a.json /tmp/termshot-b.json
bench/c-vs-rust/run.sh deflate 61        # CC=gcc or CC=clang on Linux
```

## Text-only runs: one pre-scan instead of two (2026-10-03, macOS only)

A `--text` or `--json` run without a PNG reads fonts only when the log has a
kitty command or a Sixel image that needs cell metrics. The
[parser round](#remaining-limits-1) found that deciding this took 5.9 ms on
`ansi-replay` against 8.1 ms of parsing: `graphics::needs_cell_metrics` and
`sixel::needs_cell_metrics` each walked the whole log a byte at a time.

Now one pass decides both (`needs_cell_metrics` in `main.rs`). A skipped
string passes no ESC but an ST's, so the old walks never skipped past an
`ESC P` or `ESC _`; the pass therefore only looks for those two pairs,
sixteen bytes at a time and without a branch on where the ESCs fall, and
hands each DCS and APC string, in order, to the same checks as before
(`graphics::CellMetricsScan`, `sixel::string_needs_cell_metrics`). The
decision is the same: `src/prescan_tests.rs` compares it, and each half,
with the old scans (kept as `needs_cell_metrics_reference`) on every log
in `tests/fixtures/`, `tests/vt/real/`, `examples/` and `tests/perf/`, on
every 64th prefix of each, on each with its `C=1` turned into `C=0`, and
on 20,000 generated or mutated logs (`TERMSHOT_FUZZ_ROUNDS`,
`TERMSHOT_FUZZ_SEED`). Fifteen deliberate breakages of the new code (the
numbered-image bound, relative and virtual puts, `C=1`, puts of images
never sent, ST, the scan's word boundaries and tail) each fail it.

`scripts/bench.py --suite text` (also in `all`) runs seven text-only cases (`--size 100x30
--text`, built-in font, no PNG); each checks `font_builtin`, so a case
that reads a font it should not, or skips one it needs, fails. Apple M2 Max,
macOS 26.6.2, rustc 1.98.1, Apple clang 21.0.0; main `a8a95e0` (sha256
`345dc4507209…`) against it with this change (`2fb0770600c5…`); 5 warmups,
40 interleaved rounds, 5 peak-RSS runs, seeds 17 (batch A) and 29 (B); load
average (1 min) 4.7 → 4.4 in A and 4.4 → 4.1 in B, a shared desktop. Batch
A, CLI wall time median / p95 (ms), the paired speedup with its 95%
bootstrap interval, and the `font_load_ms` and `parse_ms` stage medians:

| case | input bytes | main wall | branch wall | wall speedup A | `font_load_ms` | `parse_ms` | font read | peak RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `text-ansi-replay` | 4,716,750 | 18.82 / 19.56 | 13.61 / 14.24 | 1.378 [1.364, 1.393] | 5.941 → 0.663 | 8.17 → 8.22 | no | 6.5 → 6.5 |
| `text-ascii-overflow` | 4,000,000 | 8.14 / 8.69 | 5.02 / 5.35 | 1.618 [1.591, 1.650] | 3.604 → 0.533 | 0.34 → 0.35 | no | 5.9 → 5.9 |
| `text-dense-sgr` | 3,940,713 | 22.52 / 23.24 | 16.56 / 17.00 | 1.362 [1.356, 1.371] | 6.483 → 0.523 | 11.83 → 11.83 | no | 5.7 → 5.7 |
| `text-mixed-unicode` | 4,286,028 | 29.60 / 30.08 | 26.63 / 27.44 | 1.111 [1.108, 1.117] | 3.840 → 0.578 | 21.32 → 21.65 | no | 6.2 → 6.2 |
| `text-reply-sent` | 18,867 | 3.08 / 3.52 | 3.10 / 3.31 | 0.992 [0.972, 1.019] | 0.027 → 0.005 | 0.05 → 0.05 | no | 2.0 → 2.0 |
| `text-kitty` | 87,747 | 3.81 / 4.16 | 3.84 / 4.17 | 0.991 [0.959, 1.018] | 0.230 → 0.230 | 0.44 → 0.44 | yes | 3.8 → 3.8 |
| `text-sixel` | 386 | 3.42 / 3.67 | 3.36 / 3.75 | 1.014 [1.004, 1.031] | 0.231 → 0.233 | 0.04 → 0.05 | yes | 3.6 → 3.6 |

Batch B agrees: wall speedups 1.389, 1.611, 1.359, 1.109, 1.023, 0.971
and 0.991, `font_load_ms` 5.934 → 0.664 on `text-ansi-replay`. The pass
now costs 0.13-0.14 ns a byte on these logs, ESC-dense or not; the old walks
cost 0.9-1.6. A log whose first image needs metrics stops the scan there
(`text-kitty`, `text-sixel`), so those are unchanged; `text-kitty`'s 0.971
[0.943, 0.997] in batch B is outside every timer (its `total_ms` median is
0.863 → 0.851 ms), and its batch A interval holds 1. `text-mixed-unicode`'s `parse_ms`
is 1.5% higher in both batches although the parser did not change; the
wall time still falls by 3 ms.

Tried and dropped (`font_load_ms` medians of 31 runs, ms, on `ansi-replay`,
`dense-sgr` and 4 MB of `x`): the two old checks behind one pass that finds
each ESC eight bytes at a time and tests the byte after it, 0.79 / 1.84 /
0.32, slow where ESCs are dense; testing the pair eight bytes at a time,
0.86 / 0.70 / 0.71; skipping a 32-byte block with no ESC before testing
pairs, 1.20 / 1.02 / 0.43; four words a step instead of two, 1.44 / 1.16 /
1.18. The kept version gives 0.65 / 0.54 / 0.53. Deciding during the real
parse was not tried: the parser needs the cell size before it starts, so it
would have to replay the log again after the first image, and its answer
(the image really loaded) is not the conservative one the contract keeps.

Raw results: [batch A](performance-2026-10-03-prescan-macos-a.json),
[batch B](performance-2026-10-03-prescan-macos-b.json). Linux was not
measured. These two files predate `output_bytes` and `output_sha256`: their
`png_bytes` and `png_sha256` are the text file's (`workload.output` is
`text`); `bench.py` now writes `output_*` for every case and `png_*` only
for a PNG.

```sh
./build.sh && cp termshot /tmp/ts/branch   # and main a8a95e0 as /tmp/ts/main
for batch in a:17 b:29; do
  python3 scripts/bench.py --suite text \
    --binary main=/tmp/ts/main --binary branch=/tmp/ts/branch --reference main \
    --runs 40 --warmups 5 --memory-runs 5 --verify-identical \
    --seed "${batch#*:}" --output "/tmp/ts/text-${batch%%:*}.json"
done
```

## Painting and geometry (2026-10-03, `721d3fe`, #22)

Issue #22 asked to reprofile drawing (rounded, box and block grids, large
sparse and dense screens, other font sizes, ordinary text, images under and
over text), and to try bounded reuse of repeated geometry and cheaper
background and blend painting, without changing a pixel. Three changes to
`src/draw.c` are kept; six more experiments were measured and dropped.
Every PNG of every run matches main's byte for byte.

### Result

| | macOS arm64 (M2 Max, Apple clang 21) | Linux x86-64 (Ryzen 7 8745HS, GCC 16.2) |
| --- | --- | --- |
| `rounded-boxes` (3,000 corners, 2200×1440) wall | 15.33 → 10.13 ms (1.50-1.51×) | 14.08 → 8.67 ms (1.60-1.62×) |
| `rounded-128px` (5800×3840) wall | 69.94 → 32.85 ms (2.13-2.15×) | 63.17 → 25.56 ms (2.44-2.46×) |
| `rounded-24px` (1100×720) wall | 8.17 → 5.74 ms (1.43×) | 7.57 → 4.89 ms (1.52×) |
| `rounded-boxes` `geometry_ms` | 6.11 → 1.24 ms | 6.62 → 1.07 ms |
| `image-over` wall | 21.10 → 20.01 ms (1.05-1.06×) | 21.58 → 20.11 ms (1.08-1.09×) |
| `image-under` wall | 26.25 → 25.13 ms (1.05-1.06×) | 26.26 → 24.77 ms (1.07×) |
| `large` (5280×3840) wall | 38.67 → 38.14 ms (1.02×) | 36.90 → 34.26 ms (1.05-1.07×) |
| `reply-128px` (5800×3840) wall | 28.81 → 28.82 ms (1.00×) | 25.62 → 24.30 ms (1.05-1.06×) |
| `geometry-all` (5280×3840) wall | 97.23 → 94.40 ms (1.03×) | 98.49 → 91.32 ms (1.08-1.09×) |
| `large` `blend_ms` | 4.91 → 3.90 ms | 7.26 → 5.14 ms |
| paired wall speedup, every other case | 0.983-1.012 | 0.955-1.038 |

Speedups are main/branch paired wall ratios (above 1 is faster) of batches A
and B; stage figures are batch A medians. Rounded corners gain the most,
because each one used to be stamped afresh: on macOS 3,000 of them cost 6.1
ms of `geometry_ms` at 48 px and 42.6 ms at 128 px, and now 1.2 and 4.7 ms.
Images gain 3-6% on macOS and 7-9% on Linux. Rasters over 16 MiB gain up to
9% on Linux (`large` 1.05-1.07×, `geometry-all` 1.08-1.09×, `reply-128px`
1.05-1.06×) and up to 3% on macOS, where DEFLATE matching runs slower after
the row-at-a-time painting (see the rejected experiments). Everything else
is within noise on both hosts.

### What was measured

- **main**: `5a832cd` (main after #67, #68 and #70, the parser round),
  built with `scripts/build-baseline.py --revision 5a832cd`.
- **branch**: `721d3fe`, this PR merged with that main. The documentation
  commit after it changes no build input. The parser round's five long logs
  (`dense-sgr` and the rest) are in the suite and in the tables.

`bench.py` as in the rounds below: 5 warmups and 40 shuffled rounds of plain
and profiled runs per case and binary, 5 peak-RSS runs, seeds 17 (batch A)
and 29 (batch B), `--verify-identical`. A new `--suite draw` adds the
painting workloads (all at the default 48 px unless named; every row fits
its screen, so none wraps):

| case | screen | what it stresses |
| --- | --- | --- |
| `box-grid` | 100×30 | light, heavy and double lines: tables |
| `block-grid` | 100×30 | 30 block elements and shades in 256 colours |
| `rounded-panes` | 100×30 | four rounded panes of text, 48 corners: a realistic TUI |
| `rounded-24px`, `rounded-128px` | 100×30 | the `rounded-boxes` grid (3,000 corners) at other sizes |
| `geometry-all` | 240×80 | every U+2500-U+259F character, bold and plain, cycling |
| `large-sparse` | 240×80 | three lines of text on a large screen |
| `large-color` | 240×80 | text on 216 background colours |
| `image-below`, `image-under`, `image-over` | 100×30 | a 128×128 RGBA image with transparency over 60×20 cells, below the backgrounds (z < -2^30), under the text (z = -1) and over it, with every third row on a background of its own |

| | macOS arm64 | Linux x86-64 |
| --- | --- | --- |
| host | `lawrences-mac-studio`, macOS 26.6.2 | `starship`, kernel 7.2.5-3-omarchy, glibc 2.44, governor `performance` |
| compilers | rustc 1.98.1, Apple clang 21.0.0 (clang-2100.3.34.2) | rustc 1.98.1, GCC 16.2.1 20260810 |
| main / branch sha256 | `0880e7c6af32…` / `345dc4507209…` | `956b25423881…` / `8a57e678bec6…` |
| load average (1 min) during the batches | 4.3-4.8: a shared desktop, other sessions busy | 0.8-1.5 |

The macOS batches have no `*-full` cases: the full Noto CJK collection is not
on that host, and nothing in this round touches fonts.

Raw results: macOS [batch A](performance-2026-10-03-draw-macos-a.json),
[batch B](performance-2026-10-03-draw-macos-b.json) and
[a recheck](performance-2026-10-03-draw-macos-recheck.json); Linux
[batch A](performance-2026-10-03-draw-linux-a.json),
[batch B](performance-2026-10-03-draw-linux-b.json) and
[a recheck](performance-2026-10-03-draw-linux-recheck.json). They hold every
sample; none was discarded.

### End-to-end results

Wall median / p95 (ms) from batch A, then the paired speedup of each batch
with its 95% bootstrap interval, and peak RSS medians (MiB, batch A).

macOS arm64:

| case | main wall | branch wall | speedup A | speedup B | main RSS | branch RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 8.80 / 9.36 | 8.90 / 9.46 | 1.000 [0.980, 1.013] | 0.992 [0.977, 1.002] | 14.23 | 14.27 |
| `font-file` | 8.90 / 9.47 | 8.94 / 9.53 | 0.992 [0.968, 1.011] | 0.998 [0.969, 1.013] | 14.02 | 14.02 |
| `cjk-none` | 5.01 / 5.34 | 5.02 / 5.27 | 1.003 [0.990, 1.012] | 1.004 [0.988, 1.024] | 7.06 | 7.09 |
| `cjk-subset` | 9.58 / 10.15 | 9.54 / 10.04 | 1.006 [0.982, 1.024] | 1.003 [0.986, 1.024] | 8.62 | 8.61 |
| `cjk-cff-primary` | 9.31 / 9.87 | 9.29 / 9.87 | 1.000 [0.976, 1.015] | 0.985 [0.971, 1.009] | 7.38 | 7.31 |
| `mixed-subset` | 7.69 / 7.93 | 7.65 / 8.12 | 0.997 [0.982, 1.011] | 0.996 [0.984, 1.006] | 8.84 | 8.92 |
| `glyph-overflow` | 16.35 / 17.09 | 16.50 / 16.80 | 0.992 [0.983, 0.999] | 1.007 [0.988, 1.013] | 8.42 | 8.72 |
| `reply-sent` | 8.87 / 9.67 | 8.80 / 9.47 | 1.006 [0.990, 1.018] | 1.009 [1.000, 1.027] | 14.00 | 14.02 |
| `draft-ready` | 8.98 / 9.73 | 9.18 / 10.12 | 0.983 [0.962, 1.009] | 0.985 [0.967, 1.019] | 14.00 | 13.98 |
| `reply-24px` | 5.73 / 6.29 | 5.82 / 6.17 | 0.999 [0.981, 1.016] | 0.996 [0.974, 1.013] | 7.08 | 7.06 |
| `reply-128px` | 28.81 / 29.92 | 28.82 / 31.04 | 0.996 [0.987, 1.010] | 0.996 [0.985, 1.001] | 69.58 | 69.59 |
| `real-shell` | 6.24 / 6.67 | 6.28 / 6.58 | 0.990 [0.976, 1.021] | 0.998 [0.975, 1.006] | 10.48 | 10.52 |
| `real-less` | 5.88 / 6.06 | 5.79 / 6.17 | 1.004 [0.980, 1.029] | 1.000 [0.991, 1.019] | 10.45 | 10.44 |
| `real-vi` | 7.75 / 8.05 | 7.77 / 8.04 | 0.995 [0.979, 1.016] | 1.012 [0.987, 1.023] | 10.69 | 10.62 |
| `blank` | 5.94 / 6.32 | 5.97 / 6.27 | 0.999 [0.984, 1.018] | 1.003 [0.972, 1.015] | 12.81 | 12.84 |
| `color-grid` | 17.52 / 17.91 | 17.51 / 17.98 | 1.001 [0.994, 1.005] | 1.004 [0.995, 1.014] | 8.28 | 8.25 |
| `ascii-overflow` | 5.77 / 6.10 | 5.73 / 6.08 | 1.008 [0.998, 1.022] | 1.010 [0.992, 1.030] | 8.36 | 8.41 |
| `rounded-boxes` | 15.33 / 15.86 | 10.13 / 10.66 | 1.512 [1.493, 1.533] | 1.499 [1.479, 1.531] | 13.00 | 13.11 |
| `dense` | 11.81 / 12.03 | 11.69 / 12.31 | 1.003 [0.992, 1.020] | 1.009 [0.989, 1.015] | 14.28 | 13.97 |
| `ansi-replay` | 18.46 / 18.98 | 18.47 / 18.99 | 1.002 [0.985, 1.005] | 1.003 [0.994, 1.011] | 18.53 | 18.53 |
| `large` | 38.67 / 40.20 | 38.14 / 40.16 | 1.017 [1.008, 1.025] | 1.015 [1.007, 1.019] | 65.39 | 65.41 |
| `unicode` | 7.73 / 8.04 | 7.76 / 8.08 | 0.994 [0.985, 1.007] | 1.011 [0.987, 1.020] | 7.50 | 7.52 |
| `box-grid` | 7.31 / 7.63 | 7.35 / 7.66 | 1.000 [0.983, 1.013] | 1.011 [0.984, 1.030] | 13.97 | 13.95 |
| `block-grid` | 10.26 / 10.71 | 10.25 / 10.65 | 1.006 [0.992, 1.023] | 1.001 [0.988, 1.014] | 14.14 | 14.09 |
| `rounded-panes` | 9.67 / 10.11 | 9.56 / 10.05 | 1.002 [0.989, 1.029] | 1.001 [0.985, 1.020] | 14.27 | 14.12 |
| `rounded-24px` | 8.17 / 8.42 | 5.74 / 6.01 | 1.425 [1.402, 1.464] | 1.428 [1.421, 1.443] | 6.27 | 6.38 |
| `rounded-128px` | 69.94 / 74.62 | 32.85 / 35.01 | 2.127 [2.109, 2.146] | 2.150 [2.139, 2.163] | 69.17 | 69.36 |
| `geometry-all` | 97.23 / 100.11 | 94.40 / 99.06 | 1.029 [1.022, 1.033] | 1.027 [1.021, 1.034] | 65.48 | 65.61 |
| `large-sparse` | 20.40 / 22.72 | 20.70 / 22.65 | 1.000 [0.983, 1.010] | 0.995 [0.989, 1.016] | 64.19 | 64.17 |
| `large-color` | 145.30 / 147.67 | 143.91 / 147.06 | 1.004 [1.001, 1.009] | 1.003 [0.999, 1.010] | 77.12 | 77.14 |
| `image-below` | 22.83 / 23.20 | 22.01 / 22.63 | 1.032 [1.024, 1.042] | 1.033 [1.023, 1.040] | 14.80 | 15.56 |
| `image-under` | 26.25 / 27.08 | 25.13 / 25.80 | 1.050 [1.032, 1.058] | 1.056 [1.035, 1.065] | 15.78 | 15.08 |
| `image-over` | 21.10 / 21.76 | 20.01 / 20.74 | 1.059 [1.046, 1.063] | 1.047 [1.043, 1.054] | 15.36 | 15.45 |
| `dense-sgr` | 31.23 / 31.67 | 30.91 / 31.78 | 1.007 [1.004, 1.011] | 1.001 [0.994, 1.009] | 9.86 | 9.88 |
| `cursor-moves` | 27.52 / 28.14 | 27.49 / 28.12 | 1.000 [0.997, 1.006] | 1.001 [0.988, 1.010] | 9.14 | 9.23 |
| `scrolling` | 9.54 / 10.01 | 9.46 / 9.92 | 1.010 [0.984, 1.018] | 0.990 [0.977, 1.007] | 7.98 | 8.08 |
| `mixed-unicode` | 29.27 / 30.11 | 29.16 / 29.90 | 0.997 [0.994, 1.011] | 1.002 [0.998, 1.008] | 9.00 | 8.86 |
| `thai-combining` | 24.00 / 24.45 | 24.05 / 24.34 | 1.000 [0.996, 1.005] | 0.997 [0.990, 1.006] | 8.88 | 8.94 |

Linux x86-64:

| case | main wall | branch wall | speedup A | speedup B | main RSS | branch RSS |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 7.57 / 8.24 | 7.49 / 8.30 | 1.000 [0.977, 1.024] | 1.022 [0.993, 1.042] | 15.32 | 15.30 |
| `font-file` | 7.69 / 8.84 | 7.71 / 9.29 | 1.019 [0.967, 1.042] | 0.982 [0.967, 1.002] | 15.10 | 15.01 |
| `cjk-none` | 4.39 / 4.98 | 4.27 / 4.85 | 1.027 [0.949, 1.054] | 1.006 [0.967, 1.045] | 8.17 | 8.11 |
| `cjk-subset` | 9.97 / 10.46 | 9.79 / 10.67 | 1.024 [0.998, 1.041] | 1.004 [0.978, 1.026] | 9.38 | 9.46 |
| `cjk-cff-primary` | 8.56 / 9.88 | 9.03 / 9.83 | 0.955 [0.932, 0.995] | 0.982 [0.954, 1.026] | 8.27 | 8.23 |
| `mixed-subset` | 7.44 / 8.11 | 7.51 / 8.31 | 1.003 [0.981, 1.026] | 0.982 [0.963, 1.015] | 9.52 | 9.43 |
| `glyph-overflow` | 15.48 / 17.16 | 15.64 / 17.74 | 0.986 [0.971, 1.020] | 1.006 [0.979, 1.035] | 8.74 | 8.65 |
| `cjk-full` | 13.74 / 15.07 | 13.55 / 14.35 | 1.012 [0.991, 1.028] | 1.017 [0.992, 1.038] | 29.47 | 29.53 |
| `mixed-full` | 11.91 / 12.85 | 11.76 / 14.05 | 1.008 [0.975, 1.027] | 1.020 [0.981, 1.043] | 29.56 | 29.46 |
| `cjk-overflow-full` | 27.41 / 29.22 | 27.15 / 28.87 | 1.002 [0.985, 1.019] | 0.991 [0.971, 1.014] | 30.02 | 29.89 |
| `reply-sent` | 7.69 / 8.44 | 7.60 / 8.49 | 1.017 [0.996, 1.035] | 1.008 [0.978, 1.029] | 15.10 | 14.97 |
| `draft-ready` | 7.45 / 7.92 | 7.55 / 8.33 | 0.986 [0.967, 1.010] | 1.035 [0.987, 1.068] | 15.03 | 15.01 |
| `reply-24px` | 5.09 / 5.70 | 5.17 / 5.78 | 0.967 [0.897, 1.034] | 0.986 [0.936, 1.016] | 8.01 | 8.00 |
| `reply-128px` | 25.62 / 26.93 | 24.30 / 26.82 | 1.051 [1.036, 1.068] | 1.056 [1.045, 1.069] | 70.36 | 70.71 |
| `real-shell` | 5.58 / 6.08 | 5.38 / 6.04 | 1.038 [0.981, 1.064] | 0.993 [0.973, 1.028] | 11.45 | 11.48 |
| `real-less` | 5.02 / 5.71 | 5.02 / 5.36 | 1.018 [0.978, 1.072] | 0.969 [0.931, 1.000] | 11.47 | 11.38 |
| `real-vi` | 7.27 / 7.79 | 7.06 / 7.75 | 1.009 [0.997, 1.027] | 1.018 [1.008, 1.037] | 11.63 | 11.62 |
| `blank` | 4.12 / 4.68 | 4.07 / 4.60 | 1.035 [1.001, 1.061] | 1.008 [0.974, 1.029] | 13.82 | 13.72 |
| `color-grid` | 17.04 / 17.89 | 17.25 / 19.09 | 0.983 [0.966, 1.001] | 1.012 [0.986, 1.040] | 8.54 | 8.50 |
| `ascii-overflow` | 6.13 / 6.92 | 6.18 / 7.74 | 0.969 [0.938, 1.016] | 1.026 [0.993, 1.072] | 8.29 | 8.34 |
| `rounded-boxes` | 14.08 / 15.53 | 8.67 / 9.32 | 1.615 [1.593, 1.657] | 1.603 [1.553, 1.646] | 13.90 | 13.94 |
| `dense` | 10.94 / 11.54 | 10.69 / 11.50 | 1.013 [0.995, 1.055] | 0.992 [0.977, 1.020] | 15.10 | 14.91 |
| `ansi-replay` | 16.14 / 17.48 | 16.14 / 19.09 | 0.997 [0.978, 1.023] | 0.993 [0.983, 1.009] | 14.98 | 14.96 |
| `large` | 36.90 / 40.17 | 34.26 / 36.96 | 1.072 [1.051, 1.088] | 1.054 [1.042, 1.072] | 65.48 | 65.48 |
| `unicode` | 7.33 / 8.27 | 7.27 / 8.21 | 0.996 [0.970, 1.018] | 1.010 [0.984, 1.033] | 8.12 | 7.98 |
| `box-grid` | 5.90 / 6.70 | 5.94 / 6.96 | 1.011 [0.974, 1.043] | 1.001 [0.971, 1.037] | 14.72 | 14.87 |
| `block-grid` | 9.65 / 11.15 | 9.63 / 10.38 | 0.992 [0.960, 1.016] | 1.002 [0.972, 1.031] | 15.07 | 15.17 |
| `rounded-panes` | 8.70 / 9.42 | 8.90 / 9.64 | 0.991 [0.953, 1.023] | 1.009 [0.990, 1.031] | 15.34 | 15.29 |
| `rounded-24px` | 7.57 / 8.33 | 4.89 / 5.57 | 1.524 [1.498, 1.577] | 1.521 [1.479, 1.583] | 7.28 | 7.32 |
| `rounded-128px` | 63.17 / 67.99 | 25.56 / 28.59 | 2.456 [2.421, 2.507] | 2.444 [2.396, 2.520] | 70.18 | 70.24 |
| `geometry-all` | 98.49 / 103.17 | 91.32 / 95.69 | 1.076 [1.063, 1.096] | 1.088 [1.076, 1.098] | 66.07 | 65.91 |
| `large-sparse` | 15.49 / 16.53 | 15.43 / 16.49 | 0.986 [0.963, 1.013] | 1.015 [1.000, 1.039] | 64.66 | 64.74 |
| `large-color` | 141.00 / 150.00 | 136.36 / 145.23 | 1.026 [1.016, 1.045] | 1.028 [1.017, 1.049] | 76.21 | 76.76 |
| `image-below` | 23.18 / 24.68 | 21.37 / 23.37 | 1.075 [1.066, 1.093] | 1.081 [1.066, 1.090] | 15.79 | 15.76 |
| `image-under` | 26.26 / 28.01 | 24.77 / 26.77 | 1.068 [1.044, 1.087] | 1.070 [1.046, 1.088] | 15.84 | 15.95 |
| `image-over` | 21.58 / 23.15 | 20.11 / 21.30 | 1.080 [1.068, 1.109] | 1.086 [1.072, 1.111] | 15.77 | 15.69 |
| `dense-sgr` | 29.27 / 31.63 | 29.62 / 31.77 | 0.982 [0.973, 0.996] | 0.997 [0.987, 1.024] | 8.29 | 8.29 |
| `cursor-moves` | 26.96 / 28.80 | 26.77 / 29.97 | 1.007 [0.992, 1.023] | 0.998 [0.978, 1.018] | 8.25 | 8.23 |
| `scrolling` | 9.05 / 10.10 | 9.15 / 10.24 | 0.970 [0.944, 1.001] | 1.027 [0.974, 1.050] | 7.92 | 7.94 |
| `mixed-unicode` | 27.37 / 29.11 | 27.17 / 29.37 | 0.994 [0.984, 1.016] | 0.997 [0.987, 1.010] | 8.51 | 8.50 |
| `thai-combining` | 23.24 / 24.10 | 23.19 / 24.50 | 0.999 [0.993, 1.016] | 1.025 [0.993, 1.038] | 8.63 | 8.61 |

Four ratios are below 1 with confidence in one batch and not the other:
`glyph-overflow` on macOS batch A, 0.992 [0.983, 0.999], and on Linux
`cjk-cff-primary` batch A, 0.955 [0.932, 0.995], `dense-sgr` batch A, 0.982
[0.973, 0.996], and `real-less` batch B, 0.969 [0.931, 1.000]. None of them
takes a new path (rasters under 16 MiB, no corners or images), and 60-round
rechecks gave 0.991 [0.985, 1.001] (macOS), 0.989 [0.973, 1.011], 1.000
[0.983, 1.019] and 1.000 [0.969, 1.024] (Linux). Peak RSS medians differ by
-0.7 to +0.8 MiB on macOS, whose RSS jumps by about 0.7 MiB between runs of
one binary, and by -0.2 to +0.6 MiB on Linux.

### Where the time goes, main → branch

Batch A stage medians (ms). `background` is the cell backgrounds and the
images under the text; `foreground` holds `geometry`, `glyph` and `blend`
and the images over the text.

macOS arm64:

| case | background | geometry | glyph | blend | foreground |
| --- | ---: | ---: | ---: | ---: | ---: |
| `rounded-boxes` | 0.94 → 0.92 | 6.11 → 1.24 | 0.00 → 0.00 | 0.00 → 0.00 | 6.21 → 1.33 |
| `rounded-24px` | 0.25 → 0.26 | 2.94 → 0.49 | 0.00 → 0.00 | 0.00 → 0.00 | 3.04 → 0.59 |
| `rounded-128px` | 5.57 → 5.61 | 42.64 → 4.68 | 0.00 → 0.00 | 0.00 → 0.00 | 42.75 → 4.80 |
| `rounded-panes` | 0.90 → 0.89 | 0.21 → 0.23 | 0.11 → 0.11 | 0.58 → 0.57 | 1.02 → 1.05 |
| `geometry-all` | 5.64 → 5.67 | 8.27 → 3.27 | 0.00 → 0.00 | 0.00 → 0.00 | 8.85 → 3.87 |
| `box-grid` | 0.89 → 0.90 | 0.16 → 0.16 | 0.03 → 0.03 | 0.20 → 0.20 | 0.50 → 0.51 |
| `block-grid` | 0.90 → 0.90 | 0.61 → 0.61 | 0.00 → 0.00 | 0.00 → 0.00 | 0.71 → 0.70 |
| `large-sparse` | 5.64 → 5.56 | 0.00 → 0.00 | 0.12 → 0.10 | 0.07 → 0.04 | 0.24 → 0.20 |
| `large-color` | 5.76 → 5.80 | 0.36 → 0.37 | 0.44 → 0.53 | 7.85 → 6.52 | 9.95 → 8.83 |
| `large` | 5.76 → 5.79 | 0.21 → 0.21 | 0.35 → 0.38 | 4.91 → 3.90 | 6.22 → 5.30 |
| `image-below` | 2.98 → 2.39 | 0.05 → 0.05 | 0.20 → 0.22 | 1.15 → 1.16 | 1.60 → 1.64 |
| `image-under` | 3.57 → 2.51 | 0.05 → 0.05 | 0.20 → 0.21 | 1.17 → 1.13 | 1.63 → 1.60 |
| `image-over` | 0.95 → 0.94 | 0.05 → 0.05 | 0.20 → 0.21 | 1.19 → 1.15 | 4.26 → 3.21 |
| `reply-sent` | 0.94 → 0.94 | 0.06 → 0.07 | 0.26 → 0.26 | 0.25 → 0.25 | 0.63 → 0.64 |
| `reply-128px` | 5.69 → 5.78 | 0.69 → 0.21 | 0.55 → 0.55 | 1.54 → 1.15 | 2.86 → 1.98 |
| `dense` | 0.91 → 0.95 | 0.03 → 0.03 | 0.17 → 0.17 | 0.68 → 0.68 | 1.00 → 1.01 |
| `glyph-overflow` | 0.25 → 0.25 | 0.07 → 0.07 | 4.69 → 4.70 | 0.89 → 0.89 | 5.90 → 5.93 |

Linux x86-64:

| case | background | geometry | glyph | blend | foreground |
| --- | ---: | ---: | ---: | ---: | ---: |
| `rounded-boxes` | 0.80 → 0.81 | 6.62 → 1.07 | 0.00 → 0.00 | 0.00 → 0.00 | 6.72 → 1.18 |
| `rounded-24px` | 0.81 → 0.81 | 3.26 → 0.52 | 0.00 → 0.00 | 0.00 → 0.00 | 3.37 → 0.63 |
| `rounded-128px` | 3.80 → 3.80 | 40.95 → 3.72 | 0.00 → 0.00 | 0.00 → 0.00 | 41.05 → 3.84 |
| `rounded-panes` | 0.81 → 0.78 | 0.20 → 0.21 | 0.11 → 0.11 | 0.72 → 0.72 | 1.16 → 1.17 |
| `geometry-all` | 3.50 → 3.55 | 10.66 → 3.40 | 0.00 → 0.00 | 0.00 → 0.00 | 11.39 → 3.93 |
| `box-grid` | 0.77 → 0.80 | 0.14 → 0.14 | 0.03 → 0.03 | 0.25 → 0.26 | 0.53 → 0.56 |
| `block-grid` | 0.78 → 0.80 | 0.76 → 0.74 | 0.00 → 0.00 | 0.00 → 0.00 | 0.86 → 0.84 |
| `large-sparse` | 3.59 → 3.64 | 0.00 → 0.00 | 0.11 → 0.10 | 0.07 → 0.04 | 0.24 → 0.20 |
| `large-color` | 3.56 → 3.61 | 0.39 → 0.39 | 0.46 → 0.47 | 12.63 → 8.81 | 14.75 → 10.96 |
| `large` | 3.56 → 3.62 | 0.22 → 0.22 | 0.33 → 0.33 | 7.26 → 5.14 | 8.58 → 6.47 |
| `image-below` | 4.10 → 2.45 | 0.06 → 0.06 | 0.18 → 0.19 | 1.51 → 1.48 | 1.96 → 1.93 |
| `image-under` | 4.81 → 2.79 | 0.05 → 0.05 | 0.19 → 0.19 | 1.50 → 1.46 | 1.96 → 1.91 |
| `image-over` | 0.77 → 0.77 | 0.06 → 0.06 | 0.19 → 0.19 | 1.46 → 1.45 | 5.95 → 4.03 |
| `reply-sent` | 0.80 → 0.78 | 0.05 → 0.06 | 0.24 → 0.24 | 0.29 → 0.28 | 0.67 → 0.64 |
| `reply-128px` | 3.87 → 3.89 | 0.22 → 0.19 | 0.58 → 0.62 | 2.46 → 1.24 | 3.36 → 2.16 |
| `dense` | 0.78 → 0.78 | 0.03 → 0.03 | 0.16 → 0.16 | 0.82 → 0.81 | 1.13 → 1.12 |
| `glyph-overflow` | 0.82 → 0.83 | 0.07 → 0.07 | 4.42 → 4.45 | 1.08 → 1.10 | 5.85 → 5.91 |

What the profile showed on main, before any change (macOS, medians of 5-7
profiled runs):

- **Rounded corners and diagonals** cost 1.9 µs and 3.0 µs a cell at 48 px
  (240×80 screens of `╭` or `╱` alone: 36.6 and 57.6 ms of `geometry_ms`).
  Each stroke is a disc stamped at each of 71 points (a corner at 48 px) to
  several hundred (255 px), every pixel of every disc's box tested through
  `put`. The other geometry costs 30-560 ns a cell.
- **Cache misses, not arithmetic, set the rest of the geometry cost.** On a
  5280×3840 screen a cell of `│` (48 rows, a pixel wide) cost 297 ns, but a
  cell of `─` (one row, 22 pixels) 33 ns, and `█` 479 ns: each pixel row is a
  new cache line that the background pass wrote long before, so painting is
  bound by the rows it touches. Glyph blending behaves the same way on the
  large screens.
- **Background painting is page faults.** In a new process on macOS,
  `memset` of a fresh 61 MB buffer takes 5.8 ms and of the same buffer again
  1.0-3.2 ms: the first touch of each 16 KiB page is most of `background_ms`
  (5.4-5.9 ms) on the large screens, and no reordering of the painting
  removes it.
- **Image painting** divided `(x - im->x) * src_w / w` in 64 bits, and `x /
  cell_w` for the mask of opaque backgrounds, for every pixel: 2.7 ms for the
  1.27 million pixels of the image cases.
- **Glyphs**: `glyph-overflow` rasterizes 2,050 times for 1,116 distinct
  glyphs (1,225 evictions in the direct-mapped 1,024-slot cache), 4.7 ms. That
  is glyph caching, not painting; see the remaining limits.

### Retained changes

1. **Reuse rounded corners and diagonals that cover the same pixels**
   (`Stamps`, `stamp_find` and `stamp_points` in `draw.c`). Whether a disc
   covers pixel (x, y) depends only on `(x + 0.5f) - px` and `(y + 0.5f) -
   py`. Moved by whole pixels, `x + 0.5f` stays exact, so a stroke whose
   points are each exactly as far from its cell's origin as those of a stroke
   painted before has the same differences, as real numbers, which round the
   same: it covers the same pixels of its cell. Float rounding of the points
   does depend on where the cell is (a point's ulp grows with its
   coordinate), so this is checked, not assumed: each offset is computed with
   Knuth's TwoSum and kept only if the subtraction was exact; a column's x
   offsets and a row's y offsets are interned once per shape (at most 64
   sequences per shape and axis); a stroke is painted from the runs kept for
   its shape, thickness and pair of sequences (1,024 slots); and anything
   past the limits, past 4 MiB of allocations in all, or whose allocation
   fails is stamped as before. Letting columns share one sequence without
   the check changes pixels: `tests/boxes.c` then fails 7 of its 37,804
   checks (corners at 37×80, diagonals at 116×255, bold), so the check is
   what keeps the cache exact.
2. **Paint the backgrounds a row of cells ahead of the text, on rasters over
   16 MiB** (`Backdrop`, `backdrop_through`). There the backgrounds and the
   images below and under the text are painted a row of cells at a time,
   just before the row's own cells and before any glyph or mark that reaches
   down into it (the bracket pieces U+239B-U+23AD reach a pixel below their
   cell), so the row is still in the cache when the text goes over it, and
   each pixel is painted in the same order as before. A smaller raster, or
   one with more than 64 images under the text (each row would look at every
   one; Unicode placeholders make an image of each run), is painted at once,
   as before. `background_ms` is the sum of those rows, and `foreground_ms`
   excludes them.
3. **Step image source columns instead of dividing for each pixel.** The
   source column and the cell under the pixel are stepped as a quotient and
   remainder from one division per row; a source pixel of alpha 255 is
   copied and one of alpha 0 skipped, which is what the blend gives for them
   exactly.

### Geometry cache: hit rates and memory

From the branch's profile (`geometry_cache_hits`, `_misses`, `_uncached`,
`_bytes`; batch A; macOS and Linux report the same counts). A miss stamps
the stroke into a mask of its cell and keeps the runs; bytes are what the
cache holds at the end of the render, including its 16 KiB slot table. No
stroke went uncached in any case.

| case | strokes | hits | misses | hit rate | bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 4 | 0 | 4 | 0.0% | 28,736 |
| `font-file` | 4 | 0 | 4 | 0.0% | 28,736 |
| `mixed-subset` | 8 | 0 | 8 | 0.0% | 23,208 |
| `reply-sent` | 4 | 0 | 4 | 0.0% | 28,736 |
| `draft-ready` | 4 | 0 | 4 | 0.0% | 28,736 |
| `reply-24px` | 4 | 0 | 4 | 0.0% | 22,896 |
| `reply-128px` | 4 | 0 | 4 | 0.0% | 51,888 |
| `rounded-boxes` | 3,000 | 2,754 | 246 | 91.8% | 81,112 |
| `ansi-replay` | 4 | 0 | 4 | 0.0% | 28,736 |
| `rounded-panes` | 48 | 12 | 36 | 25.0% | 33,536 |
| `rounded-24px` | 3,000 | 2,754 | 246 | 91.8% | 49,896 |
| `rounded-128px` | 3,000 | 2,838 | 162 | 94.6% | 139,340 |
| `geometry-all` | 960 | 797 | 163 | 83.0% | 95,816 |

Worst cases, the seven strokes cycling with bold alternating in every cell
(3 profiled runs each, macOS, `geometry_ms` main `7892c11` → `e4be44b`, an
earlier state of this branch whose cache is this one without the allocation
budget):

| screen | px | strokes | misses | kept bytes | geometry ms |
| --- | ---: | ---: | ---: | ---: | --- |
| 500×200 | 9 | 114,284 | 1,186 | 74,846 | 50.63 → 4.80 |
| 500×200 | 24 | 114,284 | 1,984 | 171,562 | 166.40 → 11.31 |
| 240×80 | 64 | 21,942 | 948 | 313,072 | 134.02 → 12.51 |
| 100×30 | 255 | 3,428 | 667 | 993,028 | 376.56 → 74.15 |

No stroke went uncached in them, and every PNG matched main's. A screen with
only a few corners (`reply-sent`, `rounded-panes`) misses on most of them;
its geometry time is unchanged, and it holds 29-34 KB more for the render.

### Rejected experiments

| experiment | result | why rejected |
| --- | --- | --- |
| `fill_rect` fills its first row a pixel at a time, then `memcpy`s it to the others | `geometry_ms` 0.63 → 0.71 (`block-grid`), 6.40 → 6.92 (`geometry-all`), macOS, 7 runs | slower: most rectangles are a few pixels wide |
| unsigned arithmetic in `blend`'s divide by 255 (the same quotient) | `blend_ms` 3.86 → 4.15 (`large`), 6.67 → 6.48 (`large-color`), 0.63 → 0.65 (`dense`), macOS, 7 runs | no gain: clang already divides by a multiply |
| each glyph row's first and last covered columns stored after its bitmap, so `blend` skips the empty ends | macOS ([raw](performance-2026-10-03-draw-spans-macos.json)): `blend_ms` 5-24% lower, `glyph_ms` 0-8% higher, paired wall 0.984-1.017 with every interval spanning 1 (10 cases, 20 rounds). Linux at load 16-19 ([raw](performance-2026-10-03-draw-spans-linux.json)): `blend_ms` 0-12% lower, `glyph_ms` 1-5% higher, paired wall 0.974-1.059, only `unicode` above 1 with confidence | no end-to-end gain on either host, 4 more bytes a glyph row |
| the first version of the cache: a direct-mapped table keyed by a stroke's whole sequence of offsets, compared point by point on every stroke | `rounded-boxes`, macOS, 3-5 runs: 64 slots, 644 misses of 3,000 and `geometry_ms` 2.92; 4,096 slots, 300 misses, 2.13 ms and 167 KB | replaced: interning each axis once per column or row (retained: 246 misses, 1.2 ms, 81 KB) finds as many strokes alike and compares no points on a hit |
| the backdrop a row at a time on every raster, against all at once, the same binary otherwise (`c49a437`) | all-at-once/rows, 30 rounds. macOS ([raw](performance-2026-10-03-draw-rows-macos.json)): `draft-ready` 0.979 [0.974, 0.984], `cjk-none` 0.985 [0.972, 0.995], no raster of 9.5 MB or less above 1.010; `large` 1.010, `geometry-all` 1.008, `reply-128px` 0.989 [0.978, 1.001]. Linux ([raw](performance-2026-10-03-draw-rows-linux.json)): `large` 1.078, `rounded-128px` 1.075, `geometry-all` 1.069, `reply-128px` 1.048, `large-color` 1.030, nothing slower with confidence | kept only over 16 MiB (change 2). On macOS, after it, DEFLATE matching runs up to 1.5 ms slower on the same bytes (`reply-128px` 10.32 → 11.25 ms in the final batch A), which offsets most of the painting gain there |
| pre-faulting the raster in address order (a byte every 4 KiB) before painting rows, to undo that DEFLATE slowdown (on `c49a437`) | main (`76f18ee`)/variant, 20 rounds. macOS ([raw](performance-2026-10-03-draw-prefault-macos.json)): matching back to main's (`reply-128px` 11.22 → 10.10 ms); `reply-128px` 1.014 against 0.990 without it, `large` 1.001 against 1.007, `reply-sent` 1.000 against 0.998. Linux ([raw](performance-2026-10-03-draw-prefault-linux.json)): `reply-128px` 0.981 against 1.036, `large` 1.012 against 1.052, `reply-sent` 0.964 against 0.978 | slower on Linux, and mixed on macOS: the faults and the fill become two passes over memory |

Why matching slows on macOS is not established: its input is byte-identical,
and pre-faulting the pages in order restores it, so it follows the order in
which the raster's pages are first touched, not the data.

The issue's earlier arc approximation and uniform blending were not tried
again: the cache keeps the arcs' exact stamping, and the blend experiments
above found nothing to gain in the formula.

### Validation

On macOS and on Linux: `./test.sh`, and `SANITIZE=1 UBSAN_OPTIONS=halt_on_error=1
./tests/run.sh` (GCC's sanitizers on Linux); on macOS also `SANITIZE=1
./test.sh`. All pass, with every native golden unchanged. New checks:

- `tests/boxes.c` paints every corner and diagonal, plain and bold, at all
  21 cell sizes, through the cache and afresh, in every cell of a 500-column
  row, a 200-row column and a 20×10 grid, in random order, and requires the
  same pixels and no stroke left uncached.
- `tests/stamps_alloc.c` (`tests/run.sh`) fails each of the 136 allocations
  of a grid of strokes in turn, then shrinks the cache's budget from 4 MiB to
  nothing in 52 steps: the pixels match a render without the cache, the cache
  never holds more than its budget, and nothing is left allocated.
- `tests/draw.c` paints a backdrop of crossing, overlapping and blending
  images a row at a time and all at once and requires the same bytes; it
  paints every raster a row at a time, so the sanitizer runs cover that path.
- The `row-overlap` goldens (100×30 at 24 px, painted at once, and at 96 px,
  a 38 MB raster painted a row at a time) have glyphs that reach into the
  next row, stacked marks, italic, bold, shades and corners over coloured and
  default backgrounds, with images below and under the text; their PNGs are
  byte-identical to main's at px 9, 24, 46, 47.5, 96 and 128.
- 180 random renders of 1-4 scaled, cropped and offset images on all three
  layers, at px 9, 24 and 47.5, and 45 with 110-150 images, write main's PNG
  bytes.

CI runs the suite on macOS arm64 (Apple clang), Linux x86-64 and Linux
aarch64 (GCC).

### Remaining limits

- Page faults on the canvas's first touch are most of `background_ms` on the
  large screens (5.6-5.8 ms on macOS, 3.5-3.9 on Linux). Fewer faults need
  larger pages, which macOS arm64 does not offer to `malloc`; nothing here
  changes the allocation.
- Glyph rasterization when the glyph cache overflows (`glyph-overflow`, 4.7
  ms; `cjk-overflow-full`) is a cache-policy question: about 930 of its 2,050
  rasterizations are conflict misses in the direct-mapped cache. It is not
  painting and is left for a separate change.
- PNG compression is now the largest stage of nearly every case
  (`deflate_match_emit` 75 ms of `geometry-all`'s 90 on macOS).
- The Mac is a shared desktop (load 4.3-4.8 during the batches), so its p95
  values are noisy; the paired ratios are the figures to trust there.

### Reproduce

```sh
python3 scripts/build-baseline.py /tmp/termshot-main --revision 5a832cd
./build.sh && cp termshot /tmp/termshot-branch
for batch in a:17 b:29; do
  python3 scripts/bench.py \
    --binary main=/tmp/termshot-main/original --binary branch=/tmp/termshot-branch \
    --describe main=5a832cd --describe branch=721d3fe \
    --reference main --runs 40 --warmups 5 --memory-runs 5 --verify-identical \
    --cjk-font /usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc \
    --seed "${batch#*:}" --output "/tmp/termshot-${batch%%:*}.json"
done
python3 scripts/bench-report.py /tmp/termshot-a.json /tmp/termshot-b.json
```

`--suite draw` runs only the painting workloads.

## ANSI replay parsing (2026-10-03, `10f1ea1`, #21)

Issue #21 asked where the parse time of ANSI-heavy logs goes, and to make it
faster without changing what any log replays to. The starting point is main
`76f18ee` (#66's combining marks, #65's Sixel erasing and #67's kitty Unicode
placeholders included). Only the parser, the screen model, the width and
composition lookups and the cast detection changed; drawing and encoding
did not, and every PNG is the same.

### Result

| | macOS arm64 (M2 Max, rustc 1.98.1) | Linux x86-64 (Ryzen 7 8745HS, rustc 1.98.1) |
| --- | --- | --- |
| `ansi-replay` (4.7 MB) wall median, paired speedup | 21.86 → 18.32 ms, 1.193 | 19.24 → 16.10 ms, 1.206 |
| `ansi-replay` `parse_ms`, paired speedup | 10.20 → 8.21 ms, 1.249 | 9.12 → 7.15 ms, 1.265 |
| `ansi-replay` `input_read_ms` | 2.00 → 0.57 ms | 2.03 → 0.99 ms |
| parser cases, paired wall speedup, default build | 1.03-1.80 | 1.02-1.72 |
| parser cases, paired `parse_ms` speedup, default / aligned loops | 1.08-5.05 / 1.07-5.38 | 1.06-2.42 / 1.08-2.54 |
| other 23 cases, paired wall speedup (all four columns) | 0.981-1.034 | 0.943-1.070 |

Speedups are main/branch ratios per interleaved round (above 1 is faster),
the range over the cases and both batches. "Aligned" compares main and the
branch both built with `-C llvm-args=-align-loops=64`, the control the
[font-path baseline](#instrumentation-overhead) asked for after a 10% swing
of the ASCII loop on Zen 4 with no code change. The default and aligned
builds' parse speedups agree within 8% on every parser case and batch (the
largest gaps are `ascii-overflow`'s, 6.5% on macOS and 7.9% on Linux), so
none of the gains is a loop landing somewhere luckier. Main's ASCII loop is
still sensitive to it: in a first run of this round against `7892c11`, its
Linux `ascii-overflow` parse took 1.86 ms in the default build and 1.10 ms
aligned, while the branch took 0.45 ms in both.

### What was measured

Four binaries per host, built on that host from the same sources (on Linux
from a fresh clone of the pushed branch):

- **main**: `76f18ee` (the merge of #68), `scripts/build-baseline.py --revision 76f18ee`.
- **branch**: `10f1ea1`, this round's changes merged with `76f18ee`.
- **main-al**, **branch-al**: the same, with `RUSTC_LINK_ARGS='-C llvm-args=-align-loops=64'`
  (build.sh appends it to the rustc command).

`scripts/bench.py` gained a `parser` suite (in `all`): five generated logs
of 3.5-4.3 MB, the same bytes on every host (the JSON records their hashes):

| case | what each byte is |
| --- | --- |
| `dense-sgr` | 3,000 lines of 100 characters, each after its own SGR: palette, 256-colour, truecolour with `;` and `:`, attributes on and off, resets |
| `cursor-moves` | 700,000 moves (CUP, CUU, CUD, CUF, CUB, CHA, VPA, CR, BS, HT), each followed by a character |
| `scrolling` | full-screen lines, then a scroll region with IND, RI, IL, DL, SU, SD and bare LFs, 2,000 times |
| `mixed-unicode` | words of ASCII, Latin-1, Greek, Cyrillic, box drawing, CJK, Hangul and an emoji |
| `thai-combining` | Thai syllables with above and below vowels and tone marks (kept as marks), `café`, Hebrew with points |

With `ansi-replay` (`reply-sent.pty` × 250) and `ascii-overflow` (4 MB of
`x`) from the legacy suite they are the seven parser cases. Method as in the
earlier rounds: 5 warmups, 40 rounds, each running every binary plain and
with `TERMSHOT_PROFILE=1` in a shuffled order, 5 peak-RSS runs, seeds 17
(batch A) and 29 (batch B), the full Noto CJK collection (same sha256) on
both hosts, `--verify-identical`. Every run of a case, on all four binaries,
both batches and both hosts, wrote one PNG.

| | macOS arm64 | Linux x86-64 |
| --- | --- | --- |
| host | `lawrences-mac-studio`, macOS 26.6.2 | `starship`, Arch Linux, kernel 7.2.5-3-omarchy, glibc 2.44, governor `performance`; outputs on tmpfs |
| compilers | rustc 1.98.1, Apple clang 21.0.0 | rustc 1.98.1, GCC 16.2.1 |
| main / branch sha256 | `d3a3c621beb4…` / `0880e7c6af32…`, aligned `e066c9d86c8a…` / `288c57f75c84…` | `308474a7f13f…` / `956b25423881…`, aligned `1a814d8ceb6d…` / `f69e48bf110f…` |
| load average (1 min) during the batches | 4.4-5.6: a shared desktop, other sessions busy | 1.3-3.1: other sessions active, after a kernel build earlier in the day |

Raw results: macOS [batch A](performance-2026-10-03-parser-macos-a.json) and
[batch B](performance-2026-10-03-parser-macos-b.json); Linux
[batch A](performance-2026-10-03-parser-linux-a.json) and
[batch B](performance-2026-10-03-parser-linux-b.json), every sample of all
four binaries.

### Where the parse time went

Sampled with `samply` at 4 kHz, 50 `--text` runs per log and binary (main
`7892c11` and this branch merged with it, `88488d8`, before #67 and #68
were merged; both built with `-g` at `opt-level=2`; their
`parse_ms` speedups, 1.12 on `dense-sgr` and 1.25 on `ansi-replay` in 15
interleaved runs, match the release builds'). A sample counts as parsing
when its stack holds `replay_sized`; its stage is the innermost inlined
function (`atos -i`) that names one. Shares of the parse samples, main → branch:

| stage, % of parse samples | ansi | ascii | dense-sgr | cursor-moves | scrolling | mixed-unicode | thai-combining |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| CSI parameters | 41.9 → 42.7 | 0.0 → 0.0 | 52.6 → 48.7 | 32.8 → 33.0 | 0.9 → 1.7 | 0.0 → 0.0 | 0.0 → 0.0 |
| CSI dispatch | 8.9 → 11.5 | 0.0 → 0.0 | 7.3 → 8.6 | 26.3 → 29.1 | 2.6 → 7.0 | 0.5 → 0.7 | 0.1 → 0.1 |
| SGR and pen colours | 9.9 → 12.4 | 0.0 → 0.0 | 20.2 → 23.4 | 5.4 → 0.0 | 1.1 → 0.0 | 9.7 → 0.0 | 3.7 → 0.0 |
| grid writes | 16.0 → 12.2 | 0.8 → 1.1 | 12.1 → 10.3 | 16.6 → 17.7 | 72.5 → 54.1 | 37.5 → 38.9 | 15.0 → 31.8 |
| scrolling | 0.0 → 0.0 | 0.0 → 0.0 | 0.1 → 0.2 | 0.0 → 0.0 | 2.1 → 1.7 | 0.3 → 0.5 | 0.2 → 0.2 |
| marks | 0.1 → 0.2 | 0.0 → 0.0 | 0.8 → 0.7 | 0.9 → 0.0 | 0.5 → 0.1 | 0.4 → 1.8 | 21.7 → 21.5 |
| width lookup | 8.0 → 2.8 | 0.0 → 0.0 | 0.0 → 0.0 | 0.0 → 0.0 | 0.0 → 0.0 | 22.7 → 9.6 | 36.9 → 10.5 |
| UTF-8 decoding | 3.1 → 3.2 | 0.0 → 0.0 | 0.0 → 0.0 | 0.0 → 0.0 | 0.0 → 0.0 | 10.7 → 16.8 | 10.8 → 16.9 |
| replay loop and ASCII scan | 11.6 → 13.8 | 96.8 → 84.4 | 6.4 → 7.0 | 16.7 → 18.4 | 17.5 → 24.9 | 16.4 → 26.1 | 10.6 → 17.2 |
| strings and images | 0.4 → 1.1 | 0.0 → 1.1 | 0.3 → 0.8 | 1.2 → 1.7 | 0.3 → 1.2 | 0.6 → 2.0 | 0.4 → 0.2 |
| other | 0.2 → 0.1 | 2.4 → 13.3 | 0.3 → 0.5 | 0.1 → 0.1 | 2.5 → 9.2 | 1.2 → 3.6 | 0.6 → 1.7 |

- **CSI parameters** are the largest stage of every escape-heavy log. An
  ablation on the branch before change 7 below (scratch builds, `parse_ms`
  medians of 10 interleaved runs, M2 Max) splits them: on 200,000
  `ESC [ 38;2;100;100;100 m` + character, the whole parse took 7.14 ms,
  5.70 ms without `Screen::csi` (parameters still parsed) and 2.36 ms with
  the CSI skipped to its final byte; on `ansi-replay` 9.00, 7.88 and 4.34 ms.
  So parameters cost about 3.3-3.5 ms there, dispatch and SGR about
  1.1-1.4 ms (the skipped build also prints in other places, so this is an
  estimate). The same log with random colours took 8.28 ms against 7.15:
  mispredicted branches are a small part of it.
- **Grid writes** (printing, splitting wide characters, erasing) and the
  **width lookup** dominated the Unicode logs. The **marks** stage
  (`combine`, the composition search, the `Screen::marks` table) is Thai's
  second; change 5 below about halved its samples (its share stays, as the
  whole parse halved too). It is under 2% of every other log.
- **Scrolling** itself (rotating the row map) is small; what a scroll costs
  is erasing the row it opens, counted in grid writes.
- `(outside parse)` in `--text` mode is mostly `graphics::needs_cell_metrics`
  and `sixel::needs_cell_metrics`; see Remaining limits.

### Retained changes

Each was measured on its own against the state before it: `parse_ms` medians
of 15 interleaved `--text --json` runs per binary, default and aligned builds
(ratio of medians for aligned), M2 Max, load 3.3-5.6. Above 1 is faster.

| # | change | commit | default | aligned |
| --- | --- | --- | --- | --- |
| 1 | `cast::detect` tests the first byte before finding the first LF | `7624ec9` | `input_read_ms` 2.04 → 0.59 on `ansi-replay` | same |
| 2 | keep `Pen::cell()` in the screen, updated on SGR, DECRC and RIS | `36c5f1c` | ansi 1.092, mixed 1.122, Thai 1.035, SGR 1.002, ASCII 0.996 | 1.109, 1.121, 1.042, 0.997, 1.005 |
| 3 | erase a row by doubling `copy_within` instead of `fill` | `381673e` | scrolling 1.419, mixed 1.113, Thai 1.034, SGR 1.024, ansi 1.001 | 1.485, 1.108, 1.015, 1.018, 1.001 |
| 4 | widths below U+40000 from a two-level table (7.6 KB) | `602a11f` | Thai 1.490, mixed 1.421, ansi 1.059 | 1.502, 1.413, 1.042 |
| 5 | a mark not among the 72 that compose skips the composition search | `be90ca2` | Thai 1.290, mixed 0.989 | 1.216, 0.976 |
| 6 | test for a wide-character half inline, split out of line | `09597a9` | cursor 1.088, mixed 1.043, Thai 1.025, ansi 1.000 | 1.099, 1.082, 1.041, 1.019 |
| 7 | CSI digits, `:` and `;` before the general byte match | `26e5a6e` | SGR-only 1.195, ansi 1.106, SGR 1.074, cursor 1.007 | 1.108, 1.045, 1.037, 1.000 |
| 8 | printable ASCII runs past 16 bytes scanned eight bytes at a time | `6114db3` | ASCII 4.574, scrolling 1.132, cursor 1.025, mixed 1.010, ansi 1.006 | 4.600, 1.081, 1.042, 1.009, 1.024 |

Change 4 is generated: `tools/unicode-tables.rs` writes `WIDTH_BLOCKS` (a
byte per 64-code-point block) and `WIDTH_ROWS` (223 distinct rows of 2-bit
widths) from the same sets as the ranges, which stay for the code points
above. Change 5's list (`COMPOSING_MARKS`) comes from the same generator.
Mixed's 0.976 under change 5 does not come from the change: that log has no
mark that reaches the search. Change 1 also helps `ascii-overflow`, a raw
log with no LF (1.70 → 0.49 ms).

### Rejected experiments

Same harness, against the state before each.

| experiment | default | aligned | why rejected |
| --- | --- | --- | --- |
| an inner loop over a run of CSI digits, inlined (on main) | ansi 0.937, SGR 0.866, cursor 0.908 | 0.914, 0.878, 0.849 | slower everywhere |
| `#[inline(never)]` on the CSI parser, so its state stays in registers (on main) | ansi 0.983, SGR 0.994, cursor 0.956 | 0.950, 0.977, 0.953 | slower |
| `utf8_at` with `(low, high)` bytes instead of a `RangeInclusive` (after change 5) | Thai 1.001, mixed 0.999, ansi 0.994 | 1.003, 0.994, 1.003 | no effect |
| the u32 clamp as a sticky flag beside a wrapping u64, off the digit's dependency chain (after change 6) | SGR-only 1.007, SGR 1.003, cursor 1.008, ansi 0.993 | 0.987, 0.992, 0.998, 0.988 | no effect |
| change 7 with separate digit and `;` tests | SGR-only 1.170, SGR 1.045, cursor 0.998, ansi 1.083 | 1.108, 1.025, 0.973, 1.057 | change 7's single range test is as fast and does not slow cursor moves |
| change 7 plus `#[inline(never)]` on the CSI parser | SGR-only 1.264, SGR 1.035, cursor 0.963, ansi 1.064 | 1.265, 1.041, 0.965, 1.079 | cursor moves 3.5% slower: a call per short CSI |
| the flag clamp on top of change 7 | SGR-only 1.007, SGR 0.986, cursor 1.004, ansi 0.959 | 1.025, 1.016, 1.027, 1.025 | ansi 4% slower in the default build |
| change 8 from a run's first byte | ASCII 4.416, scrolling 1.233, ansi 0.983, mixed 0.988, cursor 0.957 | 4.500, 1.193, 1.025, 0.984, 0.991 | short runs (most of them) slower |
| `#[inline]` on `Screen::csi` (after change 8) | SGR 1.001, cursor 1.000, ansi 1.022 | 0.995, 0.998, 0.995 | no effect |

"SGR-only" is 200,000 `ESC [ 38;2;100;100;100 m x`; SGR is `dense-sgr`.

### End-to-end results

Batch A medians / p95 (ms) of plain wall time and of `parse_ms`, and the
paired speedups (main/branch) of both batches, default and aligned builds.

macOS arm64:

| case | input | main wall | branch wall | wall speedup A / B | aligned A / B | main parse | branch parse | parse speedup A / B | aligned A / B |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `ansi-replay` | 4.7 MB | 21.86 / 22.42 | 18.32 / 18.74 | 1.193 / 1.202 | 1.178 / 1.174 | 10.20 / 10.43 | 8.21 / 8.38 | 1.249 / 1.240 | 1.212 / 1.220 |
| `ascii-overflow` | 4.0 MB | 8.50 / 8.74 | 5.87 / 6.17 | 1.428 / 1.448 | 1.450 / 1.447 | 1.73 / 1.79 | 0.35 / 0.37 | 5.049 / 5.040 | 5.380 / 5.104 |
| `dense-sgr` | 3.9 MB | 32.69 / 33.85 | 31.62 / 32.42 | 1.037 / 1.031 | 1.034 / 1.029 | 13.29 / 13.66 | 12.20 / 12.46 | 1.089 / 1.082 | 1.071 / 1.073 |
| `cursor-moves` | 4.0 MB | 30.70 / 31.12 | 27.66 / 28.31 | 1.109 / 1.106 | 1.100 / 1.097 | 17.83 / 18.18 | 16.11 / 16.58 | 1.104 / 1.106 | 1.090 / 1.098 |
| `scrolling` | 3.5 MB | 15.09 / 15.69 | 9.69 / 10.25 | 1.558 / 1.543 | 1.560 / 1.562 | 8.80 / 9.07 | 3.45 / 3.57 | 2.558 / 2.544 | 2.529 / 2.545 |
| `mixed-unicode` | 4.3 MB | 47.16 / 48.39 | 29.84 / 31.08 | 1.574 / 1.574 | 1.560 / 1.561 | 39.34 / 40.24 | 22.00 / 22.34 | 1.784 / 1.790 | 1.760 / 1.754 |
| `thai-combining` | 4.3 MB | 44.28 / 45.36 | 24.73 / 25.81 | 1.788 / 1.804 | 1.792 / 1.789 | 38.13 / 38.70 | 18.49 / 18.81 | 2.066 / 2.074 | 2.069 / 2.069 |

Linux x86-64:

| case | input | main wall | branch wall | wall speedup A / B | aligned A / B | main parse | branch parse | parse speedup A / B | aligned A / B |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `ansi-replay` | 4.7 MB | 19.24 / 21.53 | 16.10 / 18.47 | 1.206 / 1.188 | 1.200 / 1.195 | 9.12 / 9.85 | 7.15 / 7.92 | 1.265 / 1.274 | 1.305 / 1.309 |
| `ascii-overflow` | 4.0 MB | 7.79 / 8.42 | 6.19 / 7.05 | 1.275 / 1.262 | 1.225 / 1.245 | 1.14 / 1.72 | 0.45 / 0.66 | 2.352 / 2.415 | 2.537 / 2.395 |
| `dense-sgr` | 3.9 MB | 30.92 / 33.03 | 30.49 / 32.37 | 1.020 / 1.024 | 1.026 / 1.032 | 11.70 / 12.34 | 11.08 / 11.94 | 1.055 / 1.058 | 1.081 / 1.100 |
| `cursor-moves` | 4.0 MB | 29.48 / 31.51 | 27.32 / 29.44 | 1.079 / 1.065 | 1.116 / 1.098 | 16.32 / 17.10 | 15.16 / 15.91 | 1.075 / 1.096 | 1.127 / 1.147 |
| `scrolling` | 3.5 MB | 13.08 / 14.12 | 9.20 / 9.63 | 1.425 / 1.385 | 1.405 / 1.384 | 6.93 / 7.49 | 3.14 / 3.60 | 2.191 / 2.177 | 2.238 / 2.266 |
| `mixed-unicode` | 4.3 MB | 42.50 / 45.08 | 27.00 / 28.74 | 1.575 / 1.565 | 1.577 / 1.561 | 35.52 / 37.13 | 20.11 / 21.59 | 1.759 / 1.758 | 1.758 / 1.780 |
| `thai-combining` | 4.3 MB | 40.07 / 43.34 | 23.32 / 25.41 | 1.718 / 1.724 | 1.698 / 1.760 | 35.01 / 37.78 | 18.17 / 19.46 | 1.939 / 1.911 | 1.927 / 1.926 |

The 95% intervals of the batch A wall speedups are narrow: the widest is
`ascii-overflow`'s on both hosts, 1.428 [1.423, 1.466] on macOS and 1.275
[1.255, 1.341] on Linux. One wall gain is within noise: Linux `dense-sgr`,
1.020 [0.995, 1.040] in batch A (1.024 [1.009, 1.039] in B), whose parse
gain is 1.055 [1.031, 1.067]; most of that run is drawing 3,000 cells in
3,000 colours. Every other case, paired wall speedup with its 95% bootstrap
interval:

| case | macOS default A | macOS default B | macOS aligned A | macOS aligned B | Linux default A | Linux default B | Linux aligned A | Linux aligned B |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 0.996 [0.969, 1.020] | 0.999 [0.987, 1.031] | 1.008 [0.984, 1.033] | 1.015 [0.995, 1.029] | 1.043 [0.973, 1.073] | 0.988 [0.953, 1.036] | 0.982 [0.950, 1.010] | 0.991 [0.964, 1.019] |
| `font-file` | 0.988 [0.977, 1.022] | 1.003 [0.984, 1.021] | 1.025 [0.997, 1.039] | 0.989 [0.968, 1.006] | 1.023 [1.003, 1.044] | 1.013 [0.971, 1.052] | 1.017 [0.995, 1.050] | 1.004 [0.961, 1.066] |
| `cjk-none` | 1.005 [0.991, 1.031] | 0.990 [0.973, 1.020] | 0.991 [0.967, 1.021] | 1.018 [0.984, 1.035] | 0.958 [0.929, 1.021] | 1.037 [0.989, 1.075] | 0.991 [0.954, 1.057] | 1.070 [1.018, 1.140] |
| `cjk-subset` | 1.006 [0.986, 1.023] | 0.990 [0.982, 1.019] | 1.022 [1.008, 1.031] | 1.011 [0.999, 1.021] | 1.005 [0.968, 1.027] | 0.998 [0.966, 1.013] | 1.006 [0.970, 1.042] | 0.994 [0.955, 1.043] |
| `cjk-cff-primary` | 0.994 [0.970, 1.003] | 1.006 [0.989, 1.034] | 1.016 [1.004, 1.024] | 1.001 [0.975, 1.029] | 0.998 [0.969, 1.034] | 1.022 [0.986, 1.066] | 1.003 [0.955, 1.032] | 1.002 [0.971, 1.024] |
| `mixed-subset` | 0.997 [0.985, 1.004] | 0.988 [0.972, 1.012] | 0.993 [0.987, 1.021] | 1.011 [0.991, 1.038] | 1.006 [0.981, 1.045] | 0.974 [0.929, 1.017] | 0.998 [0.969, 1.033] | 1.021 [0.979, 1.056] |
| `glyph-overflow` | 0.993 [0.983, 1.002] | 1.005 [0.993, 1.013] | 0.991 [0.986, 0.997] | 1.002 [0.988, 1.014] | 0.985 [0.966, 1.023] | 1.000 [0.966, 1.021] | 0.999 [0.983, 1.014] | 1.009 [0.992, 1.024] |
| `cjk-full` | 1.005 [0.995, 1.011] | 0.999 [0.982, 1.013] | 1.004 [0.994, 1.024] | 0.991 [0.972, 1.015] | 0.994 [0.970, 1.041] | 1.003 [0.969, 1.035] | 1.014 [1.000, 1.043] | 1.022 [0.988, 1.049] |
| `mixed-full` | 1.018 [0.991, 1.030] | 0.995 [0.975, 1.008] | 1.005 [0.990, 1.013] | 0.993 [0.965, 1.025] | 1.017 [0.983, 1.042] | 1.003 [0.971, 1.045] | 1.034 [0.983, 1.078] | 1.005 [0.970, 1.043] |
| `cjk-overflow-full` | 0.997 [0.982, 1.007] | 1.003 [0.985, 1.012] | 1.009 [0.994, 1.015] | 1.000 [0.992, 1.009] | 0.992 [0.971, 1.011] | 1.006 [0.982, 1.026] | 0.993 [0.984, 1.014] | 0.999 [0.977, 1.024] |
| `reply-sent` | 0.986 [0.979, 1.010] | 1.013 [1.002, 1.030] | 1.007 [0.983, 1.020] | 1.016 [0.982, 1.038] | 0.985 [0.966, 0.998] | 0.994 [0.976, 1.010] | 1.005 [0.964, 1.018] | 0.986 [0.962, 1.024] |
| `draft-ready` | 1.014 [0.999, 1.028] | 1.014 [0.988, 1.041] | 1.001 [0.982, 1.019] | 1.010 [0.989, 1.037] | 1.008 [0.991, 1.023] | 0.992 [0.959, 1.022] | 1.015 [0.987, 1.036] | 1.001 [0.972, 1.024] |
| `reply-24px` | 0.981 [0.960, 1.011] | 1.003 [0.989, 1.029] | 1.001 [0.983, 1.022] | 0.985 [0.971, 1.036] | 0.998 [0.973, 1.069] | 1.002 [0.947, 1.032] | 0.992 [0.934, 1.038] | 0.943 [0.899, 1.027] |
| `reply-128px` | 1.001 [0.985, 1.009] | 0.991 [0.982, 1.004] | 1.002 [0.994, 1.011] | 0.999 [0.990, 1.012] | 0.998 [0.978, 1.012] | 1.007 [0.991, 1.027] | 1.001 [0.983, 1.014] | 1.010 [0.988, 1.029] |
| `real-shell` | 0.989 [0.981, 1.009] | 1.000 [0.976, 1.016] | 1.013 [1.002, 1.028] | 1.007 [0.985, 1.024] | 0.997 [0.969, 1.019] | 0.982 [0.934, 1.029] | 0.988 [0.963, 1.016] | 1.003 [0.982, 1.026] |
| `real-less` | 0.996 [0.980, 1.014] | 0.991 [0.975, 1.014] | 1.005 [0.993, 1.012] | 1.006 [0.988, 1.030] | 1.024 [0.969, 1.070] | 0.996 [0.970, 1.025] | 0.993 [0.970, 1.019] | 1.018 [0.974, 1.054] |
| `real-vi` | 1.005 [0.995, 1.016] | 1.010 [1.002, 1.030] | 1.001 [0.988, 1.010] | 1.000 [0.986, 1.009] | 1.037 [0.983, 1.076] | 0.984 [0.947, 1.030] | 1.019 [0.990, 1.055] | 1.013 [0.993, 1.030] |
| `blank` | 0.999 [0.989, 1.014] | 1.004 [0.981, 1.015] | 1.000 [0.986, 1.016] | 0.982 [0.972, 1.005] | 1.005 [0.979, 1.053] | 1.009 [0.971, 1.042] | 1.023 [0.995, 1.050] | 0.989 [0.955, 1.041] |
| `color-grid` | 1.010 [1.003, 1.012] | 1.001 [0.988, 1.018] | 1.003 [0.995, 1.010] | 1.000 [0.991, 1.012] | 1.009 [0.989, 1.024] | 0.978 [0.969, 1.025] | 0.995 [0.971, 1.027] | 0.995 [0.981, 1.024] |
| `rounded-boxes` | 0.996 [0.991, 1.004] | 0.996 [0.983, 1.008] | 1.034 [1.025, 1.039] | 1.020 [1.005, 1.033] | 1.032 [0.977, 1.051] | 0.992 [0.985, 1.019] | 0.997 [0.978, 1.015] | 1.002 [0.991, 1.034] |
| `dense` | 1.005 [0.992, 1.015] | 1.001 [0.989, 1.012] | 0.998 [0.983, 1.026] | 1.019 [1.002, 1.035] | 1.017 [0.981, 1.039] | 1.002 [0.985, 1.014] | 1.010 [0.987, 1.051] | 1.036 [1.009, 1.047] |
| `large` | 0.997 [0.988, 1.004] | 0.996 [0.992, 1.003] | 0.995 [0.986, 1.006] | 0.997 [0.992, 1.001] | 0.999 [0.983, 1.016] | 0.999 [0.983, 1.015] | 1.006 [0.989, 1.016] | 1.001 [0.992, 1.018] |
| `unicode` | 0.998 [0.987, 1.003] | 1.009 [0.992, 1.025] | 1.005 [0.988, 1.024] | 1.010 [0.987, 1.035] | 1.048 [0.975, 1.083] | 0.958 [0.946, 1.001] | 1.000 [0.961, 1.031] | 1.021 [0.976, 1.046] |

Their parse is 0.01-0.27 ms; what moves them is start-up, drawing and
encoding, which this round does not touch.

Peak RSS medians move by about 1 MiB at most (macOS −0.9 to +1.0 MiB,
Linux −0.3 to +1.4 MiB), the page-granular noise of the earlier rounds; the width table adds 7.6 KB of
read-only data. `TERMSHOT_PROFILE` costs the same as before
(profiled/plain 0.99-1.08).

### Regressions, and what was checked

- **No parser case is slower** on either host, build or batch.
- Two other cases have an interval wholly below 1, each in one of its four
  columns, and neither inside termshot:
  - macOS `glyph-overflow`, aligned batch A: 0.991 [0.986, 0.997]. Its
    `total_ms` is lower on the branch (13.427 → 13.325 ms), its parse
    faster (0.057 → 0.041 ms); the other three columns are 0.993-1.005.
  - Linux `reply-sent`, default batch A: 0.985 [0.966, 0.998], and 0.994
    [0.976, 1.010] in batch B. Its `total_ms` is lower on the branch in both
    (6.653 → 6.596 and 6.783 → 6.631 ms), so the 0.1-0.2 ms is process
    start-up, loading and exit, which no timer covers; the aligned builds
    give 1.005 and 0.986.
- The widest intervals are Linux's 4-5 ms runs (`reply-24px` aligned B 0.943
  [0.899, 1.027], `cjk-none` default A 0.958 [0.929, 1.021], and the same
  case's 1.070 [1.018, 1.140] in aligned B), whose parse is 0.05 ms.
- `blank` parses an empty log; its 0.01 ms `parse_ms` moves by timer noise.

### Validation

- `./test.sh`, `SANITIZE=1 ./test.sh` and `rustup run 1.70 ./test.sh` pass
  on macOS arm64: the parser unit tests, the `tests/vt/` screens (malformed
  and truncated escapes, extended colours, margins, autowrap, wide
  characters, REP, combining marks) and the real sessions against tmux, the
  grid-size fuzz, and every pixel golden and `--text`/`--json` grid,
  unchanged. CI runs the same on Linux x86-64 and aarch64.
- New tests: the width table against the ranges for every code point up to
  U+110100; every composition through the mark filter; `printable_end`
  against a byte loop for every byte value at every offset of a 40-byte run.
- Differential replay (scratch, not committed): random logs over random
  grid sizes (1-119 columns by 1-39 rows, a quarter with `--lf-newline`, a
  tenth cut at a random byte), mixing SGR in every form, CSI with private
  markers, intermediates, huge and zero-padded parameters, C0 controls
  inside sequences, OSC, DCS, Sixel and kitty strings, wide characters,
  marks and invalid UTF-8. Main's and the branch's `--text`, `--json`,
  stderr and exit status are identical for all of them: 23,000 logs against
  `7892c11`, and 25,000 against `76f18ee`, 5,000 of those with kitty Unicode
  placeholders and SGR 58. A deliberately broken build (SGR dispatch
  removed) fails on the second log.

### Remaining limits

- **Text-only runs scan the log twice before parsing it** (since done in
  one pass: see [the text-only pre-scan](#text-only-runs-one-pre-scan-instead-of-two-2026-10-03-macos-only)). With `--text` or
  `--json` and no PNG, `graphics::needs_cell_metrics` and
  `sixel::needs_cell_metrics` look for an image that needs font metrics a
  byte at a time: 5.9 ms on `ansi-replay` against 8.1 ms of parsing (M2
  Max, `--text`, `10f1ea1`). They live in `graphics.rs` and `sixel.rs`,
  which other work is changing, so this round leaves them; finding each ESC
  eight bytes at a time, as change 8 does for printable runs, would remove
  most of it.
- CSI parameters are still about 40% of `ansi-replay`'s parse. None of the
  restructurings above moved them further; what is left is the per-byte
  classification itself.
- One batch pair per host; the Mac is a shared desktop.

### Reproduce

```sh
python3 scripts/build-baseline.py /tmp/ts/main --revision 76f18ee
RUSTC_LINK_ARGS='-C llvm-args=-align-loops=64' python3 scripts/build-baseline.py /tmp/ts/main-al --revision 76f18ee
./build.sh && cp termshot /tmp/ts/branch
RUSTC_LINK_ARGS='-C llvm-args=-align-loops=64' ./build.sh && cp termshot /tmp/ts/branch-al
cjk=/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc   # or a copy with the same sha256
for batch in a:17 b:29; do
  python3 scripts/bench.py \
    --binary main=/tmp/ts/main/original --binary branch=/tmp/ts/branch \
    --binary main-al=/tmp/ts/main-al/original --binary branch-al=/tmp/ts/branch-al \
    --reference main --runs 40 --warmups 5 --memory-runs 5 \
    --verify-identical --cjk-font "$cjk" \
    --seed "${batch#*:}" --output "/tmp/ts/${batch%%:*}.json"
done
python3 scripts/bench.py ... --suite parser   # the five new logs alone
```

## PNG compression: Adler-32 and DEFLATE matching (2026-10-03, `3bf2ffc`, #20)

Issue #20 asked what is left of PNG compression after #4 and #13, and to make
it faster without changing a byte of output. The starting point is the
[font-path baseline](#current-baseline-font-paths-and-linux-2026-10-03-d83c8fd)
below: matching was the largest stage in most cases, and at 128 px Linux
spent 13.6 ms in Adler-32 against 6.2 ms on macOS. Only `src/deflate.c`
changed; the stream it writes is the same for every input and quality
(`tests/deflate_diff.c`), so every PNG is too.

### Result

| | macOS arm64 (M2 Max, Apple clang 21) | Linux x86-64 (Ryzen 7 8745HS, GCC 16.2) |
| --- | --- | --- |
| end to end, 25 cases, paired wall speedup | 0.994-1.098 | 1.034-1.470 |
| `reply-sent` (2200×1440) wall median | 9.74 → 9.55 ms | 9.94 → 7.90 ms |
| `reply-128px` (5800×3840) wall median | 33.19 → 31.05 ms | 40.48 → 27.60 ms |
| `large` (5280×3840) wall median | 43.52 → 40.83 ms | 51.88 → 39.32 ms |
| Adler-32, `reply-128px` | 6.21 → 3.48 ms | 14.49 → 3.82 ms |
| matching and emission, `reply-128px` | 10.27 → 10.29 ms | 14.37 → 11.97 ms |

Speedups are main/branch paired wall ratios (above 1 is faster), batch A and
B together; stage figures are batch A medians. The Linux gain is mostly
Adler-32, which now costs the same on both machines, and 5-23% of
matching time. On macOS the gain is Adler-32 alone; matching is unchanged
within noise. No case is slower with confidence on either host: the lowest
95% bound is 0.935 (`glyph-overflow`, macOS batch B, median 0.994).

### What was measured

- **main**: `bb21b3c` (main when this work started), built with
  `scripts/build-baseline.py --revision bb21b3c`.
- **branch**: `3bf2ffc`, the three `deflate.c` commits on top of it. The later
  documentation commit does not change any build input.

Each host built both binaries from the same sources (on Linux from a fresh
clone of the pushed branch). Method, cases and harness are those of the
baseline below: `bench.py`, 5 warmups and 40 shuffled rounds of plain and
profiled runs per case and binary, 5 peak-RSS runs, seeds 17 (batch A) and
29 (batch B), the full CJK collection (same sha256) on both hosts. Every run
of a case, on both binaries, both batches and both hosts, gave one PNG.

| | macOS arm64 | Linux x86-64 |
| --- | --- | --- |
| host | `lawrences-mac-studio`, macOS 26.6.2 | `starship`, kernel 7.2.5-3-omarchy, glibc 2.44, governor `performance` |
| compilers | rustc 1.98.1, Apple clang 21.0.0 (clang-2100.3.34.2) | rustc 1.98.1, GCC 16.2.1 20260810 |
| main / branch sha256 | `e12d181701fc…` / `3517189e0999…` | `c559de35b33b…` / `2a13d3ed4d88…` |
| load average (1 min) during the batches | 5.9-8.1: a shared desktop, other sessions busy | 1.7-2.7 |

Raw results: macOS [batch A](performance-2026-10-03-deflate-macos-a.json) and
[batch B](performance-2026-10-03-deflate-macos-b.json); Linux
[batch A](performance-2026-10-03-deflate-linux-a.json) and
[batch B](performance-2026-10-03-deflate-linux-b.json). They hold every sample;
none was discarded. The Mac's load was higher than in the baseline round, so
its p95 values are noisier; the paired ratios are the figures to trust there.

### End-to-end results

Wall median / p95 (ms) from batch A, paired speedup with its 95% bootstrap
interval for both batches, and the three PNG stages (batch A medians, main →
branch). `png_encode` holds the other two.

macOS arm64:

| case | main wall | branch wall | speedup A | speedup B | match_emit | checksum | png_encode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 10.40 / 11.14 | 9.81 / 10.47 | 1.067 [1.031, 1.083] | 1.044 [1.026, 1.063] | 2.72 → 2.71 | 0.86 → 0.48 | 3.71 → 3.30 |
| `font-file` | 10.35 / 11.67 | 9.84 / 10.76 | 1.058 [1.040, 1.075] | 1.051 [1.033, 1.066] | 2.75 → 2.69 | 0.87 → 0.48 | 3.77 → 3.29 |
| `cjk-none` | 5.20 / 5.59 | 5.12 / 5.61 | 1.010 [0.998, 1.045] | 1.022 [0.995, 1.049] | 0.88 → 0.88 | 0.21 → 0.12 | 1.12 → 1.03 |
| `cjk-subset` | 10.54 / 12.36 | 10.30 / 11.61 | 1.025 [1.010, 1.053] | 1.004 [0.992, 1.030] | 4.19 → 4.03 | 0.22 → 0.12 | 4.58 → 4.33 |
| `cjk-cff-primary` | 10.20 / 12.28 | 9.72 / 11.35 | 1.031 [1.013, 1.065] | 1.023 [1.004, 1.045] | 4.03 → 3.93 | 0.24 → 0.14 | 4.45 → 4.26 |
| `mixed-subset` | 7.98 / 8.60 | 7.80 / 8.48 | 1.022 [1.005, 1.032] | 1.016 [1.007, 1.048] | 2.30 → 2.26 | 0.22 → 0.12 | 2.62 → 2.47 |
| `glyph-overflow` | 18.59 / 19.48 | 18.18 / 19.70 | 0.998 [0.984, 1.024] | 0.994 [0.935, 1.018] | 6.04 → 5.89 | 0.21 → 0.12 | 6.51 → 6.25 |
| `cjk-full` | 13.63 / 15.40 | 13.36 / 14.73 | 1.030 [1.002, 1.044] | 1.039 [1.014, 1.129] | 4.09 → 4.00 | 0.21 → 0.12 | 4.50 → 4.30 |
| `mixed-full` | 11.63 / 12.15 | 11.51 / 12.47 | 1.012 [0.992, 1.023] | 1.012 [0.994, 1.027] | 2.40 → 2.39 | 0.21 → 0.12 | 2.70 → 2.60 |
| `cjk-overflow-full` | 29.89 / 31.37 | 29.49 / 31.50 | 1.015 [0.998, 1.032] | 1.018 [1.004, 1.036] | 7.91 → 7.71 | 0.22 → 0.12 | 8.43 → 8.16 |
| `reply-sent` | 9.74 / 11.63 | 9.55 / 10.75 | 1.022 [0.995, 1.053] | 1.045 [1.028, 1.053] | 2.76 → 2.72 | 0.85 → 0.48 | 3.75 → 3.31 |
| `draft-ready` | 9.65 / 12.40 | 9.45 / 12.78 | 1.032 [1.006, 1.052] | 1.047 [1.031, 1.071] | 2.69 → 2.64 | 0.85 → 0.48 | 3.68 → 3.22 |
| `reply-24px` | 6.03 / 6.55 | 5.96 / 7.40 | 1.003 [0.991, 1.024] | 1.057 [1.031, 1.067] | 1.25 → 1.25 | 0.22 → 0.12 | 1.52 → 1.42 |
| `reply-128px` | 33.19 / 40.37 | 31.05 / 38.04 | 1.074 [1.056, 1.091] | 1.093 [1.075, 1.115] | 10.27 → 10.29 | 6.21 → 3.48 | 17.10 → 14.29 |
| `real-shell` | 6.92 / 9.48 | 6.55 / 7.22 | 1.064 [1.046, 1.085] | 1.047 [1.037, 1.067] | 1.32 → 1.28 | 0.55 → 0.30 | 1.93 → 1.64 |
| `real-less` | 6.45 / 7.86 | 6.14 / 7.08 | 1.051 [1.030, 1.090] | 1.034 [1.012, 1.067] | 0.99 → 0.99 | 0.56 → 0.31 | 1.60 → 1.36 |
| `real-vi` | 8.77 / 10.10 | 8.22 / 14.62 | 1.047 [1.029, 1.070] | 1.054 [1.036, 1.062] | 2.52 → 2.50 | 0.56 → 0.31 | 3.18 → 2.91 |
| `blank` | 7.08 / 8.21 | 6.57 / 7.40 | 1.098 [1.046, 1.128] | 1.040 [1.015, 1.056] | 0.72 → 0.73 | 0.87 → 0.48 | 1.66 → 1.28 |
| `color-grid` | 19.31 / 24.71 | 19.19 / 25.56 | 1.010 [0.999, 1.019] | 1.013 [1.002, 1.029] | 11.80 → 11.63 | 0.22 → 0.12 | 12.41 → 12.14 |
| `ascii-overflow` | 9.06 / 9.93 | 8.82 / 9.28 | 1.023 [1.007, 1.030] | 1.008 [0.993, 1.023] | 0.39 → 0.39 | 0.21 → 0.12 | 0.64 → 0.54 |
| `rounded-boxes` | 16.37 / 16.78 | 15.85 / 16.42 | 1.028 [1.018, 1.037] | 1.030 [1.013, 1.041] | 3.67 → 3.61 | 0.89 → 0.49 | 4.66 → 4.23 |
| `dense` | 13.77 / 17.59 | 13.17 / 17.51 | 1.029 [1.015, 1.047] | 1.028 [1.011, 1.049] | 5.43 → 5.25 | 0.90 → 0.50 | 6.57 → 6.04 |
| `ansi-replay` | 23.39 / 28.29 | 22.77 / 32.85 | 1.014 [1.004, 1.024] | 1.028 [1.015, 1.036] | 2.80 → 2.76 | 0.88 → 0.49 | 3.79 → 3.37 |
| `large` | 43.52 / 46.92 | 40.83 / 43.27 | 1.066 [1.057, 1.080] | 1.064 [1.058, 1.074] | 17.83 → 17.51 | 5.70 → 3.17 | 24.39 → 21.57 |
| `unicode` | 8.39 / 8.68 | 8.27 / 8.73 | 1.014 [0.996, 1.030] | 1.011 [1.003, 1.026] | 2.53 → 2.46 | 0.22 → 0.12 | 2.85 → 2.67 |

Linux x86-64:

| case | main wall | branch wall | speedup A | speedup B | match_emit | checksum | png_encode |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 10.08 / 10.67 | 7.92 / 8.79 | 1.247 [1.201, 1.318] | 1.262 [1.231, 1.283] | 3.80 → 3.34 | 2.01 → 0.53 | 6.04 → 4.07 |
| `font-file` | 10.16 / 11.26 | 8.10 / 8.79 | 1.259 [1.227, 1.277] | 1.261 [1.246, 1.288] | 3.97 → 3.44 | 2.03 → 0.53 | 6.28 → 4.23 |
| `cjk-none` | 4.92 / 5.38 | 4.36 / 4.86 | 1.108 [1.086, 1.221] | 1.087 [1.049, 1.168] | 1.34 → 1.19 | 0.50 → 0.13 | 2.02 → 1.41 |
| `cjk-subset` | 10.98 / 11.63 | 10.08 / 10.87 | 1.096 [1.076, 1.112] | 1.119 [1.090, 1.136] | 5.47 → 4.88 | 0.50 → 0.13 | 6.31 → 5.27 |
| `cjk-cff-primary` | 9.78 / 10.95 | 9.31 / 9.99 | 1.086 [1.040, 1.136] | 1.126 [1.075, 1.168] | 5.38 → 4.82 | 0.57 → 0.15 | 6.23 → 5.22 |
| `mixed-subset` | 8.46 / 9.07 | 7.75 / 8.35 | 1.078 [1.058, 1.114] | 1.109 [1.083, 1.136] | 3.09 → 2.82 | 0.50 → 0.13 | 3.75 → 3.10 |
| `glyph-overflow` | 16.79 / 18.20 | 15.90 / 17.17 | 1.062 [1.040, 1.088] | 1.079 [1.056, 1.100] | 7.43 → 6.70 | 0.50 → 0.13 | 8.24 → 7.17 |
| `cjk-full` | 15.60 / 16.92 | 14.49 / 15.32 | 1.093 [1.072, 1.110] | 1.063 [1.036, 1.105] | 5.33 → 4.83 | 0.50 → 0.13 | 6.08 → 5.23 |
| `mixed-full` | 13.00 / 14.08 | 12.31 / 13.84 | 1.055 [1.027, 1.092] | 1.034 [1.010, 1.084] | 3.28 → 2.94 | 0.50 → 0.13 | 3.94 → 3.22 |
| `cjk-overflow-full` | 30.49 / 33.34 | 29.44 / 30.53 | 1.041 [1.027, 1.064] | 1.075 [1.057, 1.092] | 9.67 → 8.74 | 0.51 → 0.13 | 10.62 → 9.34 |
| `reply-sent` | 9.94 / 10.78 | 7.90 / 8.50 | 1.276 [1.258, 1.294] | 1.245 [1.207, 1.275] | 3.92 → 3.43 | 2.02 → 0.52 | 6.13 → 4.15 |
| `draft-ready` | 9.88 / 10.31 | 7.89 / 8.69 | 1.251 [1.203, 1.274] | 1.270 [1.254, 1.301] | 3.79 → 3.44 | 2.03 → 0.52 | 6.09 → 4.17 |
| `reply-24px` | 5.72 / 6.45 | 5.14 / 5.61 | 1.113 [1.091, 1.159] | 1.115 [1.090, 1.166] | 1.80 → 1.64 | 0.50 → 0.13 | 2.45 → 1.90 |
| `reply-128px` | 40.48 / 42.79 | 27.60 / 29.44 | 1.459 [1.440, 1.482] | 1.305 [1.280, 1.426] | 14.37 → 11.97 | 14.49 → 3.82 | 29.84 → 16.65 |
| `real-shell` | 6.91 / 7.68 | 5.57 / 6.13 | 1.233 [1.199, 1.277] | 1.238 [1.199, 1.267] | 1.96 → 1.67 | 1.29 → 0.33 | 3.46 → 2.17 |
| `real-less` | 6.33 / 6.98 | 5.10 / 5.93 | 1.246 [1.202, 1.271] | 1.235 [1.215, 1.284] | 1.50 → 1.31 | 1.29 → 0.33 | 3.07 → 1.79 |
| `real-vi` | 9.00 / 10.80 | 7.60 / 8.24 | 1.199 [1.179, 1.222] | 1.225 [1.191, 1.249] | 3.61 → 3.15 | 1.30 → 0.34 | 5.12 → 3.68 |
| `blank` | 6.27 / 7.18 | 4.34 / 4.82 | 1.455 [1.396, 1.488] | 1.470 [1.437, 1.528] | 0.93 → 0.72 | 2.06 → 0.53 | 3.21 → 1.37 |
| `color-grid` | 19.96 / 21.89 | 17.81 / 19.07 | 1.114 [1.098, 1.147] | 1.110 [1.096, 1.125] | 14.14 → 12.46 | 0.50 → 0.13 | 15.23 → 13.15 |
| `ascii-overflow` | 8.23 / 8.81 | 7.84 / 8.92 | 1.038 [1.000, 1.062] | 1.063 [1.037, 1.083] | 0.60 → 0.53 | 0.50 → 0.13 | 1.15 → 0.70 |
| `rounded-boxes` | 16.32 / 17.52 | 14.45 / 15.47 | 1.107 [1.099, 1.129] | 1.136 [1.121, 1.147] | 4.10 → 3.89 | 2.03 → 0.53 | 6.25 → 4.55 |
| `dense` | 13.51 / 14.31 | 11.33 / 12.36 | 1.209 [1.158, 1.236] | 1.194 [1.184, 1.203] | 6.81 → 6.15 | 2.03 → 0.53 | 9.22 → 7.02 |
| `ansi-replay` | 21.18 / 22.73 | 19.39 / 20.63 | 1.100 [1.076, 1.121] | 1.105 [1.092, 1.112] | 3.84 → 3.27 | 2.03 → 0.53 | 6.00 → 3.91 |
| `large` | 51.88 / 54.30 | 39.32 / 42.89 | 1.325 [1.307, 1.350] | 1.312 [1.299, 1.340] | 22.12 → 19.16 | 13.04 → 3.55 | 36.51 → 23.92 |
| `unicode` | 8.14 / 9.17 | 7.31 / 8.11 | 1.124 [1.100, 1.155] | 1.095 [1.075, 1.127] | 3.39 → 3.05 | 0.51 → 0.13 | 4.06 → 3.33 |

Peak RSS medians differ by at most about 1 MiB (+1.02 MiB `large` on macOS
batch A, -0.89 MiB `reply-128px` on macOS batch B, within 0.22 MiB on Linux).
Two macOS p95 values rose (`real-vi` 10.10 → 14.62, `ansi-replay` 28.29 →
32.85) while their medians and paired ratios improved; with the host's load
at 6-8 these are single slow runs, not a pattern, and neither reproduces in
batch B or on Linux. After this change Adler-32 is no longer the largest
stage of any case; matching is the largest PNG stage everywhere.

### Where the time went, and why Linux paid twice

The old Adler-32 loop took 32 bytes at a time and computed
`sum((32 - k) * d[k])`, a weighted sum with 32-bit products. Apple clang
vectorizes it with NEON widening multiply-accumulates (`umull`/`umlal`).
GCC 16 vectorizes it too (`-fopt-info-vec`: "loop vectorized using 16 byte
vectors"), but baseline x86-64 is SSE2, which has no 32-bit lane multiply
(`pmulld` is SSE4.1): `objdump -d` shows every product emulated with
`pmuludq` plus `psrlq`/`pshufd`/`punpckldq` shuffles, and a horizontal
reduction every 32 bytes. That is the 2.2× gap.

Matching was attributed with `perf record` on Linux and with counters in a
scratch copy of the compressor (not committed). On these images a position
is cheap and long matches are common: `reply-128px` codes 67 MB of filtered
scanlines from 427,000 positions (average match 210 bytes, about one
`countm` per position), `color-grid` 2.4 MB from 435,000 positions (average
match 17.5). Time goes to loading the newest bucket entry, the
candidate-rejection byte test, `countm`, and per-token emission. Two GCC
specifics showed in the profile: `countm`, `zhash`, `huff` and `add_bits`
stayed out of line at `-O2`, and the Huffman bit reversal ran as a loop of
up to nine iterations per literal. Allocation and the stored-block check
are negligible (`deflate_allocate_ms` ≈ 0.005 ms, `deflate_finalize_ms`
≈ 0.001 ms at 128 px).

### Retained changes

1. **Adler-32 in 16 lanes** (`e634fa1`). Per 16-byte chunk, keep the byte sum
   of each lane `a[k]` and the sum of earlier sums `p[k]` (`p += a`, then
   `a += chunk`). At the end of a 5552-byte block, `s2 += 16n·s1 + 16·Σp +
   Σ(16 - k)·a[k]` and `s1 += Σa`: the inner loop is widening adds only, and
   the weights are applied once per block. 5552 is zlib's NMAX, so no sum
   can overflow before the modulo. GCC vectorizes this plain C loop at
   `-O2`; clang leaves it scalar (it is 1.8× slower than the old loop on the
   M2), so clang builds use the same arithmetic in its generic vector types
   (`__attribute__((vector_size))`, `__builtin_convertvector`). No
   intrinsics, nothing target-specific: the clang form lowers to NEON on
   arm64 and SSE2 on x86-64 macOS. `TERMSHOT_PORTABLE_ADLER` forces the plain
   loop, and `test.sh` runs `deflate_diff` both ways, so CI tests the plain
   loop on clang as well as on GCC.
2. **Huffman bit reversal by table** (`b4bd3a3`): a 256-byte table of
   reversed bytes, built by macros, instead of a bit loop.
3. **Inline the per-token helpers and carry the next hash** (`3bf2ffc`):
   force `countm`, `zhash`, `huff`, `add_bits` and `bitrev` inline (GCC and
   clang only; others get plain `static inline`). Lazy matching already
   hashes position `i + 1`; a literal that follows reuses it, and after a
   match the next position is hashed before the token is emitted.

Compressor alone, in a scratch harness that calls main's and the branch's
`termshot_zlib_compress` alternately on the deflate input of each image
(the decompressed IDAT of its PNG) and checks that the outputs are
identical: medians of 21 rounds (5 for the two synthetic inputs), ms, total
and the paired match/emit ratio.

| input | bytes | macOS total main → branch | macOS match ratio | Linux total main → branch | Linux match ratio |
| --- | ---: | ---: | ---: | ---: | ---: |
| `reply-sent` | 9,505,440 | 3.57 → 3.14 | 0.991 | 5.27 → 3.24 | 0.841 |
| `reply-24px` | 2,376,720 | 1.34 → 1.23 | 0.967 | 1.83 → 1.29 | 0.866 |
| `reply-128px` | 66,819,840 | 15.87 → 13.19 | 0.999 | 26.04 → 13.58 | 0.801 |
| `large` | 60,829,440 | 21.92 → 19.35 | 0.998 | 32.42 → 19.77 | 0.844 |
| `color-grid` | 2,376,720 | 11.92 → 11.61 | 0.994 | 13.06 → 10.95 | 0.863 |
| `dense` | 9,505,440 | 5.96 → 5.58 | 0.975 | 7.63 → 5.61 | 0.889 |
| `rounded-boxes` | 9,505,440 | 4.26 → 3.88 | 0.999 | 5.73 → 4.06 | 0.939 |
| `blank` | 9,505,440 | 1.53 → 1.16 | 1.025 | 2.74 → 1.12 | 0.762 |
| uniform random bytes | 9,505,440 | 249.5 → 245.3 | 0.969 | 270.8 → 227.7 | 0.851 |
| 4 symbols, skewed | 9,505,440 | 208.2 → 191.0 | 0.953 | 186.1 → 174.3 | 0.949 |

The two synthetic inputs vary entropy: random bytes (every position a
literal, about two hash-collision candidates each) and a skewed
four-symbol alphabet. They are slow for any fixed-Huffman stb-style
compressor and are not termshot images; they show that the changes hold
outside the image workloads. The one ratio above 1 is `blank` matching on
macOS (0.66 → 0.68 ms), outweighed by its Adler-32 saving.

Adler-32 variants, in a scratch benchmark (interleaved, medians of 31 runs,
ms; each checked against the old function on 3,000 random and all-0xff
buffers of random length and alignment). The macOS inputs are the
`reply-128px` and `reply-sent` scanlines; on Linux 67 MB of random bytes and
the `reply-sent` scanlines (the checksum's speed does not depend on the
data). On both hosts the two generic-vector rows come from a second run of
the same benchmark.

| variant | M2, clang, 67 MB | M2, clang, 9.5 MB | Ryzen, GCC 16, 67 MB | Ryzen, GCC 16, 9.5 MB |
| --- | ---: | ---: | ---: | ---: |
| old: 32-byte weighted sum | 6.15 | 0.87 | 15.26 | 2.16 |
| plain C, 16 lanes (kept for GCC) | 10.95 | 1.54 | 3.93 | 0.54 |
| plain C, 32 lanes | 8.47 | 1.19 | 7.56 | 1.06 |
| plain C, 8 lanes | 9.60 | 1.36 | 9.56 | 1.34 |
| plain C, 16 lanes, two chunks per step | 9.28 | 1.31 | 4.34 | 0.60 |
| generic vectors, 16 lanes (kept for clang) | 3.45 | 0.49 | 58.56 | 8.25 |
| generic vectors, 32 lanes | 4.82 | 0.69 | 29.73 | 4.16 |

GCC lowers a 64-byte generic vector without AVX-512 one lane at a time, so
the generic-vector form is clang-only. GCC 13.3 (Ubuntu 24.04, the CI
compiler) vectorizes the 16-lane plain loop the same way: 0.56 ms against
2.06 ms for the old loop on the 9.5 MB input.

### Rejected experiments

Match/emit time, paired median ratio of 21 interleaved rounds against the
state after change 1, over the six image inputs above (below 1 is faster).

| experiment | macOS | Linux | why rejected |
| --- | --- | --- | --- |
| buffer growth moved out of line from `add_bits` | — | match/emit medians 0.99-1.03 of main's (9 rounds) | no gain; marked `cold`, it also made GCC move part of the match loop to `termshot_zlib_compress.cold` |
| 16-bit bucket counts (`cnt`, 32 KiB instead of 64) | 0.970-1.003 | 1.028-1.051 | slower on Linux |
| `countm` 32 bytes per step, then 16 | 0.993-1.043 | 0.990-1.039 | no gain, as in the third round |
| `__builtin_prefetch` of the next bucket and count, on top of carrying the hash | 0.977-1.013 | 0.949-0.990 | no consistent gain over carrying the hash alone (0.975-0.994 / 0.969-0.987) |
| hash table aligned to 64 bytes, one cache line per bucket (against change 3) | 0.985-1.010 | 0.990-1.006 | no effect |
| Adler-32 fused into the match loop, while the data is in cache | — | — | not built: 67 MB checksummed as hot 64 KiB pieces takes 3.46-3.83 ms against 3.70-5.21 ms streamed on macOS, and 3.66-3.77 against 3.69-3.95 ms on Linux (5 runs each). The new loop is compute-bound, leaving at most about 0.4 ms per 67 MB for a restructured match loop |

For comparison, the retained bit-reversal table alone measured 0.996-1.016
on macOS and 0.897-0.965 on Linux, forced inlining alone 0.986-1.019 and
0.938-0.999, carrying the hash alone 0.975-0.994 and 0.969-0.987, and changes
2 and 3 together 0.954-0.992 and 0.806-0.931.

### Validation

On macOS: `./test.sh` (3,060 compressor cases byte-identical to stb, plus
the same with `TERMSHOT_PORTABLE_ADLER`), `SANITIZE=1 ./test.sh`, and
`SANITIZE=1 UBSAN_OPTIONS=halt_on_error=1 ./tests/run.sh` (round trips,
48 PNG integrity cases, 11 compressor allocation failures, 8 concurrent
renders) all pass. `deflate_diff` with 20,000 cases also passes. The new
all-0xff cases catch an Adler block too large for 32-bit sums: with the
block set to 4 × 5552, 20 of them fail. CI runs the suite on
macOS arm64, Linux x86-64 and Linux aarch64 (GCC there).

### Remaining limits

- Matching is now the cost: 10-12 ms at 128 px and for `color-grid`, on
  both hosts. What is left is the stb-compatible policy
  itself (fixed Huffman codes, a 16-entry bucket scan, lazy matching); a
  different policy would change the bytes and is outside #20.
- The Adler-32 loop is compute-bound at about 18 GB/s on both hosts. A
  faster one needs pairwise widening adds (`uadalp`, `psadbw`), which plain C
  and generic vectors do not express portably.
- One Mac batch pair, under a load average of 6-8.

### Reproduce

```sh
python3 scripts/build-baseline.py /tmp/termshot-main --revision bb21b3c
./build.sh && cp termshot /tmp/termshot-branch
# Arch's copy; on macOS, point this at a copy of the same file (same sha256).
cjk=/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc
for batch in a:17 b:29; do
  python3 scripts/bench.py \
    --binary main=/tmp/termshot-main/original --binary branch=/tmp/termshot-branch \
    --describe main=bb21b3c --describe branch=3bf2ffc \
    --reference main --runs 40 --warmups 5 --memory-runs 5 \
    --verify-identical --cjk-font "$cjk" \
    --seed "${batch#*:}" --output "/tmp/termshot-${batch%%:*}.json"
done
python3 scripts/bench-report.py /tmp/termshot-a.json /tmp/termshot-b.json
```

## Current baseline: font paths and Linux (2026-10-03, `d83c8fd`)

Issue #19 asked for the default built-in font, an explicit font, real CJK
fallback rendering, mixed scripts, a glyph working set larger than the glyph
cache, and Linux x86-64. Since the third round, main gained CFF (`.otf`) fonts,
`.ttc` face selection (`FILE#N`), the cursor and DECSCUSR shapes, italic, and
kitty graphics, so everything here was remeasured from scratch.

### What was measured

Two binaries per host, built on that host from the same sources:

- **main**: `d83c8fd` (the merge of PR #52), built with `scripts/build-baseline.py --revision d83c8fd`.
- **branch**: `d83c8fd` plus this round's profiling change (`22b77e8`): no change
  to parsing, drawing or encoding, only timers and counters.

Every run's PNG was hashed. Each case produced **one PNG across every warmup,
plain, profiled, RSS and cold run of both binaries** in every batch, and
**the same PNG on macOS arm64 and Linux x86-64** for all 25 cases.

| | macOS arm64 | Linux x86-64 |
| --- | --- | --- |
| host | `lawrences-mac-studio`: Apple M2 Max, 12 cores, 32 GiB | `starship`: AMD Ryzen 7 8745HS, 8 cores / 16 threads, 60 GiB |
| OS | macOS 26.6.2 | Arch Linux, kernel 7.2.5-3-omarchy, glibc 2.44, governor `performance` |
| compilers | rustc 1.98.1 (Homebrew), Apple clang 21.0.0 (clang-2100.3.34.2) | rustc 1.98.1 (Arch), GCC 16.2.1 20260810 |
| main binary sha256 | `530aafcfad36…` | `fefda4d0612d…` |
| branch binary sha256 | `b4e4c365fe97…` | `6a58fd112552…` |
| storage | APFS (internal SSD); outputs in `$TMPDIR` | btrfs on NVMe; outputs on the same disk (`TMPDIR`), not tmpfs |
| load average (1 min) | 3.7-5.0: a shared desktop with other sessions running | 0.5-1.8 |

Flags are those of `build.sh`: C `-O2`, plus `-ffp-contract=off` for `draw.c`;
Rust `--edition 2021 -C opt-level=2`. Each JSON records the full binary hashes,
the harness revision (`e3a8f67`, clean; the binary's build inputs have not
changed since `22b77e8`) with a hash of every build input, the toolchain, font
hashes, input hashes and dimensions.

Raw results: macOS [batch A](performance-2026-10-03-macos-a.json) and
[batch B](performance-2026-10-03-macos-b.json); Linux
[batch A](performance-2026-10-03-linux-a.json),
[batch B](performance-2026-10-03-linux-b.json) and
[cold cache](performance-2026-10-03-linux-cold.json). They keep every wall,
child-CPU, profiled-wall, profile-record and peak-RSS sample, with means,
medians, nearest-rank p95, min and max. No sample was discarded.

**Method.** Batches A and B use the same binaries and seeds 17 and 29. Per
case and binary: 5 warmups, then 40 rounds; every round runs each binary once
plain and once with `TERMSHOT_PROFILE=1`, the four runs shuffled. Both kinds
are spawned the same way (stdout discarded, stderr piped), and profile records
are parsed after the clock stops. Then 5 separate peak-RSS runs
(`/usr/bin/time -l` / `-v`). Nothing else from this
work ran on either host during a batch; other users' work did on the Mac.
Paired speedups are the median of per-round ratios with a 95% percentile
bootstrap (2,000 resamples of whole rounds, seed 42). They describe these
samples; they are not a guarantee for other machines. **Warm** means the page
cache holds the binary, inputs and fonts (the warmups load them); outputs are
rewritten and closed without `fsync`. The **cold** experiment is separate.

### Workloads and fonts

The 15 earlier workloads are kept as they were (positional CLI, explicit
JetBrains Mono file). Ten new ones cover the font paths. Their logs are in
`tests/perf/`, regenerated byte for byte by `scripts/perf-fixtures.py`
(`--check` verifies):

| case | log | grid / px | fonts | what it exercises |
| --- | --- | --- | --- | --- |
| `font-builtin` | `examples/reply-sent.pty` | 100×30 / 48 | built-in | the embedded-font path |
| `font-file` | same | 100×30 / 48 | `--font JetBrainsMono-Regular.ttf` | the same font read from a file |
| `cjk-none` | `tests/perf/cjk-dense.pty` | 100×30 / 24 | built-in only | control: every Han cell a missing-glyph box |
| `cjk-subset` | same | 100×30 / 24 | built-in + `--fallback-font` Noto subset | CFF fallback glyphs |
| `cjk-full` | same | 100×30 / 24 | built-in + `NotoSansCJK-Regular.ttc#3` | the same with a real 19 MB collection |
| `cjk-cff-primary` | same | 100×30 / 24 | `--font` Noto subset | a CFF primary font, no fallback |
| `mixed-subset` | `tests/perf/mixed-script.pty` | 100×30 / 24 | built-in + Noto subset | Latin, Greek, Cyrillic, Han, missing kana/Hangul/emoji, boxes, SGR |
| `mixed-full` | same | 100×30 / 24 | built-in + full Noto TC | the same; only the emoji are missing |
| `glyph-overflow` | `tests/perf/glyph-overflow.pty` | 100×30 / 24 | built-in | 1,116 distinct glyphs over 3,000 cells, beyond the 1,024-slot cache |
| `cjk-overflow-full` | `tests/perf/cjk-overflow.pty` | 100×30 / 24 | built-in + full Noto TC | 1,500 distinct fallback glyphs, no cache hits possible |

| font | version | bytes | sha256 | license |
| --- | --- | ---: | --- | --- |
| JetBrains Mono Regular (built in, and `third_party/jetbrains-mono/`) | 2.304 | 273,900 | `a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f` | OFL 1.1 |
| Noto Sans CJK TC subset (`third_party/noto-sans-cjk/`) | 2.004 | 32,620 | `f8923ab11a5f99924efcbd8b7903a633ccd1e51ac5ec5dcd074d3cd8cdc2ab45` | OFL 1.1 |
| `NotoSansCJK-Regular.ttc`, face 3 (Noto Sans CJK TC) | 2.004 | 19,484,784 | `b76b0433203017ca80401b2ee0dd69350349871c4b19d504c34dbdd80541690a` | OFL 1.1 |

The full collection is not in the repository: it is Arch's `noto-fonts-cjk
20240730-1` (`/usr/share/fonts/noto-cjk/`), the file the subset was cut from.
The Mac ran a copy of the same file (same hash) through `--cjk-font`. Without
one, `bench.py` skips the three `*-full` cases.

**Each case checks that it took its path.** `bench.py` reads the profile
counters of every profiled run and fails the batch if a case's checks do not
hold (for example `fallback_rasterizations > 0`, `glyph_missing == 0`,
`glyph_cache_evictions > 0`). Medians from the branch binary:

| case | glyphs rasterized | cache hits | evictions | missing | fallback lookups | fallback rasterized |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin`, `font-file` | 63 | 399 | 0 | 0 | 0 | 0 |
| `cjk-none` | 12 | 1,463 | 6 | 25 | 0 | 0 |
| `cjk-subset`, `cjk-full` | 37 | 1,463 | 6 | 0 | 25 | 25 |
| `cjk-cff-primary` | 37 | 1,463 | 6 | 0 | 0 | 0 |
| `mixed-subset` | 207 | 749 | 89 | 33 | 62 | 29 |
| `mixed-full` | 237 | 749 | 89 | 3 | 62 | 59 |
| `glyph-overflow` | 2,050 | 950 | 1,225 | 0 | 0 | 0 |
| `cjk-overflow-full` | 1,500 | 0 | 476 | 0 | 1,500 | 1,500 |

`font-builtin`, `font-file`, `reply-sent` and `ansi-replay` give one PNG, as
do `cjk-subset` and `cjk-full`: the subset's outlines are the full font's.

### End-to-end results

Branch binary, batch A. Wall is median / p95 elapsed ms per CLI run, CPU is the
median child user+system ms, RSS the median peak MiB.

| case | macOS wall | macOS CPU | macOS RSS | Linux wall | Linux CPU | Linux RSS | PNG bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `font-builtin` | 9.27 / 10.41 | 7.70 | 14.11 | 9.64 / 10.39 | 9.45 | 15.30 | 205,915 |
| `font-file` | 9.32 / 9.69 | 7.77 | 13.91 | 9.85 / 11.00 | 9.64 | 15.03 | 205,915 |
| `cjk-none` | 5.17 / 6.71 | 3.82 | 6.94 | 4.90 / 5.66 | 4.75 | 8.07 | 45,725 |
| `cjk-subset` | 9.75 / 10.42 | 8.22 | 8.47 | 11.03 / 12.13 | 10.84 | 9.43 | 296,925 |
| `cjk-cff-primary` | 9.17 / 9.92 | 7.84 | 7.45 | 9.88 / 11.20 | 9.65 | 8.11 | 293,397 |
| `mixed-subset` | 7.97 / 8.41 | 6.43 | 8.70 | 8.18 / 8.93 | 8.01 | 9.40 | 141,953 |
| `glyph-overflow` | 17.20 / 17.68 | 15.23 | 7.95 | 16.37 / 18.14 | 16.10 | 8.53 | 365,425 |
| `cjk-full` | 12.67 / 13.17 | 10.96 | 27.03 | 14.85 / 16.06 | 14.59 | 29.19 | 296,925 |
| `mixed-full` | 11.18 / 11.64 | 9.47 | 27.23 | 12.98 / 13.87 | 12.71 | 29.46 | 152,792 |
| `cjk-overflow-full` | 26.92 / 27.79 | 24.91 | 27.91 | 28.66 / 29.79 | 28.36 | 28.52 | 506,727 |
| `reply-sent` | 9.17 / 9.83 | 7.68 | 13.86 | 9.71 / 10.42 | 9.50 | 15.06 | 205,915 |
| `draft-ready` | 9.13 / 9.64 | 7.72 | 13.88 | 9.84 / 11.20 | 9.61 | 15.03 | 201,182 |
| `reply-24px` | 5.83 / 6.30 | 4.48 | 6.97 | 5.63 / 6.15 | 5.48 | 7.98 | 79,424 |
| `reply-128px` | 31.56 / 33.00 | 28.86 | 70.38 | 38.06 / 42.02 | 37.64 | 70.42 | 955,025 |
| `real-shell` | 6.79 / 7.30 | 5.36 | 10.42 | 6.84 / 7.43 | 6.65 | 11.31 | 110,584 |
| `real-less` | 6.30 / 6.88 | 4.87 | 10.30 | 6.50 / 6.89 | 6.10 | 11.35 | 85,844 |
| `real-vi` | 8.22 / 11.89 | 6.81 | 10.55 | 8.59 / 9.37 | 8.41 | 11.73 | 187,905 |
| `blank` | 6.31 / 6.63 | 4.97 | 12.69 | 6.18 / 6.76 | 5.87 | 13.74 | 95,475 |
| `color-grid` | 18.04 / 19.72 | 16.43 | 7.58 | 19.22 / 20.47 | 18.95 | 8.35 | 649,294 |
| `ascii-overflow` | 7.46 / 7.71 | 6.10 | 8.27 | 7.89 / 9.00 | 7.71 | 8.36 | 34,814 |
| `rounded-boxes` | 15.83 / 16.27 | 14.21 | 12.89 | 15.73 / 16.49 | 15.48 | 13.85 | 141,158 |
| `dense` | 12.15 / 12.53 | 10.65 | 13.88 | 13.12 / 14.46 | 12.88 | 15.10 | 361,314 |
| `ansi-replay` | 20.80 / 22.03 | 18.97 | 18.58 | 19.69 / 20.86 | 19.43 | 14.96 | 205,915 |
| `large` | 41.91 / 45.05 | 39.21 | 65.48 | 49.24 / 53.02 | 48.80 | 65.21 | 1,362,320 |
| `unicode` | 8.24 / 8.74 | 6.73 | 7.33 | 7.89 / 8.64 | 7.71 | 8.04 | 157,189 |

Medians differ by at most about 21% between the machines. Linux spends more time
inside termshot (`total_ms`) and macOS more outside it (start-up, below), so
small cases tend to be faster on Linux and the largest images on macOS. At
128 px Linux's Adler-32 (`deflate_checksum`) costs about twice macOS's (13.6
vs 6.2 ms).

What the font paths cost, from these medians:

- **Built-in vs file**: no measurable difference. Copying the embedded font
  (`font_read_ms` 0.046 ms macOS, 0.137 Linux) costs about the same as reading the
  file (0.047, 0.151); the paired `font-builtin`/`font-file` gap is inside run-to-run noise.
- **A fallback font is cheap until it is big.** Loading the subset takes
  0.12 ms on macOS and 0.36 ms on Linux, mostly padding. Between `cjk-none` and
  `cjk-subset` most of the difference is compressing a busier image
  (`deflate_match_emit` 0.9 → 4.2 ms on macOS), not the fallback lookups.
  With the real 19 MB collection `font_load` is **2.8 ms (macOS) and 4.8 ms
  (Linux)**, against 0.4 and 1.0 ms with the subset. Almost all of it is
  `fallback_read_ms` (2.4 / 3.7 ms); checking face 3 takes 0.09 / 0.08 ms and
  padding 0.11 / 0.36 ms. It also adds about 19 MiB of peak RSS.
- **Overflowing the glyph cache** costs rasterization, not lookup. With 1,116
  distinct glyphs, 1,225 evictions make `glyph_ms` 4.7 ms (macOS) / 4.1 ms (Linux),
  against 0.8 ms (macOS) for the 800-codepoint `unicode` case. With 1,500 distinct CJK
  fallback glyphs `glyph_ms` is 9.5 / 9.4 ms, the largest stage of that case.

### Where the time goes, per case (input for #20, #21, #22)

Branch binary, batch A, the three largest **non-overlapping** stages by median
(ms; `foreground_other` is `foreground` minus its three children, `font_load`
holds every font timer). `total` is the Rust `total_ms`.

| case | macOS total: top stages | Linux total: top stages |
| --- | --- | --- |
| `font-builtin` | 5.94: deflate_match_emit 2.73, background 0.92, deflate_checksum 0.87 | 8.59: deflate_match_emit 3.67, deflate_checksum 1.97, background 0.78 |
| `cjk-none` | 2.22: deflate_match_emit 0.88, background 0.25, font_load 0.24 | 4.26: deflate_match_emit 1.33, background 0.80, font_load 0.69 |
| `cjk-subset` | 6.57: deflate_match_emit 4.18, blend 0.53, font_load 0.36 | 9.96: deflate_match_emit 5.29, font_load 0.99, background 0.82 |
| `cjk-cff-primary` | 6.30: deflate_match_emit 4.02, blend 0.51, glyph 0.30 | 9.30: deflate_match_emit 5.16, background 0.92, blend 0.74 |
| `mixed-subset` | 4.93: deflate_match_emit 2.29, glyph 0.68, output_write 0.46 | 7.14: deflate_match_emit 2.96, font_load 0.99, background 0.81 |
| `glyph-overflow` | 14.02: deflate_match_emit 6.06, glyph 4.68, blend 0.93 | 15.82: deflate_match_emit 7.19, glyph 4.14, blend 1.06 |
| `cjk-full` | 9.09: deflate_match_emit 4.14, font_load 2.83, blend 0.53 | 13.70: deflate_match_emit 5.17, font_load 4.84, background 0.79 |
| `mixed-full` | 7.56: font_load 2.85, deflate_match_emit 2.41, glyph 0.89 | 11.55: font_load 4.81, deflate_match_emit 3.16, glyph 0.82 |
| `cjk-overflow-full` | 23.09: glyph 9.49, deflate_match_emit 7.95, font_load 2.92 | 27.65: glyph 9.37, deflate_match_emit 9.16, font_load 4.80 |
| `reply-128px` | 26.82: deflate_match_emit 10.32, deflate_checksum 6.17, background 5.72 | 37.17: deflate_checksum 13.63, deflate_match_emit 13.31, background 3.77 |
| `color-grid` | 14.90: deflate_match_emit 11.90, blend 0.48, png_pack 0.38 | 18.94: deflate_match_emit 13.99, background 0.79, blend 0.68 |
| `rounded-boxes` | 12.46: geometry 6.14, deflate_match_emit 3.67, deflate_checksum 0.88 | 14.63: geometry 6.36, deflate_match_emit 3.93, deflate_checksum 1.94 |
| `ascii-overflow` | 4.63: parse 1.76, input_read 0.48, blend 0.45 | 7.07: parse 2.01, input_read 1.03, background 0.69 |
| `ansi-replay` | 17.19: parse 10.09, deflate_match_emit 2.77, background 0.92 | 18.49: parse 9.06, deflate_match_emit 3.55, deflate_checksum 1.93 |
| `large` | 38.13: deflate_match_emit 17.89, deflate_checksum 5.67, background 5.65 | 49.45: deflate_match_emit 21.01, deflate_checksum 12.38, blend 7.24 |

`bench-report.py` prints this table, with the rest of the cases, from any result file.

- **PNG (#20)**: DEFLATE matching/emission is the largest stage in 19 of 25
  cases on macOS and 18 on Linux. Adler-32 is second at high resolution and
  on Linux the largest for `reply-128px` and `blank`, at about twice macOS's cost.
- **ANSI (#21)**: parsing dominates only the long-log cases (`ansi-replay`,
  `ascii-overflow`); see the code-alignment note below before trusting a small
  parser change on Linux.
- **Drawing (#22)**: glyph rasterization matters once the working set exceeds
  the cache or the fallback brings many distinct glyphs (`glyph-overflow`,
  `cjk-overflow-full`); box/arc geometry dominates `rounded-boxes`; background
  and blending scale with image size.
- **Outside `total_ms`**: process start-up, dynamic loading, exit and the
  harness's spawn take a median **3.0-5.0 ms on macOS but 1.0-1.5 ms on Linux**
  (profiled wall minus `total_ms`), the largest single cost of the small macOS
  cases. No timer inside termshot can see it.

### Cold page cache (Linux only)

`bench.py --cold-runs 15` drops the page cache (`sync; echo 3 >
/proc/sys/vm/drop_caches`, via `sudo -n`) before every run, so the binary, its
libraries, the input and the fonts come from the NVMe disk. Fifteen interleaved
runs per binary, then compared with the same file's 5 warm runs:

| case | cold median / p95 (main) | cold median / p95 (branch) | warm median (branch) |
| --- | ---: | ---: | ---: |
| `font-builtin` | 16.61 / 17.30 | 17.01 / 18.19 | 10.15 |
| `font-file` | 18.06 / 18.87 | 18.07 / 20.34 | 9.85 |
| `cjk-subset` | 18.66 / 21.03 | 19.00 / 19.88 | 11.12 |
| `cjk-full` | 27.18 / 28.90 | 27.18 / 31.23 | 14.81 |

A cold start adds 6.9-8.2 ms to the three small-font cases, and 12.4 ms with
the 19 MB collection. macOS has no unprivileged way to drop its cache (`purge` needs root,
which this host's account does not have), so no macOS cold result is claimed.

### Timer boundaries

`TERMSHOT_PROFILE` (any value, including `0`) makes termshot print two
`termshot-profile ` JSON records to stderr: one from `draw.c`, then one from
`main.rs`. Indentation below is nesting; **never add a parent to its
children**, and siblings at one level do not overlap.

- `total_ms`: from the start of `main` to the last record, excluding process
  start-up, Rust's final teardown and the record itself.
  - `input_read_ms`: reading the log, and for a `.cast`, decoding its events.
  - `font_load_ms`: deciding which fonts are needed and loading them. Each font
    (`font_*` for the built-in or `--font` font, `fallback_*` for `--fallback-font`)
    has the same four boundaries, in this order:
    - `*_allocate_ms`: an empty buffer the size of the font.
    - `*_read_ms`: filling it. For a file this includes opening and sizing it;
      for the built-in font, copying it out of the binary, page faults included.
    - `*_check_ms`: picking the face and validating it (`font::check`, the CFF parse).
    - `*_padding_ms`: appending the 1 MiB of zero padding stb needs, which may
      move the buffer.
    - `*_bytes` is the font's size before padding; `font_builtin` is 1 for the
      built-in font. A font not loaded reports zeros.
  - `parse_ms`: cell metrics (`draw_cell_size`), replaying the log, and freeing the input.
  - `face_ms`: building the faces `draw.c` gets, including the CFF table parsed
    again for each CFF font (`Font::with_face`).
  - The whole `draw.c` call, which prints its own record:
    - `font_setup_ms`: stb initialization and cell metrics; `allocate_ms`: the raster.
    - `background_ms`: painting backgrounds (first touch of the raster) and
      the images under the text. Since #22 these are painted a row of cells
      at a time, interleaved with the text, and this is the sum of the rows.
    - `foreground_ms`: glyphs, decorations, freeing caches and images over the
      text; since #22, the painting time less `background_ms`.
      - `geometry_ms`: box drawing and block elements, and dispatch for every
        non-blank cell; `glyph_ms`: cache lookup, outline and rasterization;
        `blend_ms`: blending glyphs and missing-glyph boxes.
    - `png_encode_ms`: `png_filter_ms` (a boundary only), `png_deflate_ms`
      (`deflate_allocate_ms`, `deflate_match_emit_ms`, `deflate_finalize_ms`,
      `deflate_checksum_ms`), `png_pack_ms` (PNG allocation, copy, CRC-32).
    - `output_write_ms`: open, write, close; `cleanup_ms`: freeing the raster.
- Counters, from `draw.c`: `glyph_rasterizations`, `glyph_cache_hits`,
  `glyph_cache_evictions` (a cache miss that replaced another glyph),
  `glyph_missing` (a cache miss that found the glyph in neither font),
  `fallback_lookups` (a miss that asked the fallback), `fallback_rasterizations`;
  since #22, `geometry_cache_hits`, `geometry_cache_misses`,
  `geometry_cache_uncached` (a rounded corner or diagonal stamped without the
  cache) and `geometry_cache_bytes`; also `png_bytes`, `pixel_bytes` and,
  from Rust, `input_bytes`.

What changed in this round: the built-in font's `font_check_ms` used to hold
copying, checking and padding; a fallback font was timed only inside
`font_load_ms`; `face_ms`, `*_allocate_ms`, `*_bytes`, `font_builtin` and the
glyph/fallback counters are new. `tests/profile.rs` (run by `tests/run.sh`)
checks that the font parts fit inside `font_load_ms`, that a render with the
built-in font and the CFF subset as fallback reports both fonts' sizes and
draws fallback glyphs, and that profiling does not change the PNG.

### Instrumentation overhead

Two separate questions, both measured in the same interleaved rounds:

- **Does the new instrumentation slow ordinary runs?** main/branch paired wall
  ratios, both batches, are inside **0.98-1.02 on every case on macOS** and
  0.97-1.03 on Linux, except `ascii-overflow` on Linux, 0.897 [0.873, 0.942]
  and 0.900 [0.885, 0.937]. That one is **code placement, not the timers**: the
  `replay_sized` function it spends its parse time in is unchanged, but it moved
  from an address 48 bytes past a 64-byte boundary to one on it. Building both
  revisions with `-C llvm-args=-align-loops=64` puts their parse times within
  2% (1.106 vs 1.122 ms, 55 interleaved profiled runs), while unaligned they
  are 1.087 vs 1.857 ms; moving only `main.rs`/`font.rs` reproduces it and
  restoring `fs::read` does not remove it. (These diagnostic builds were
  separate, ad-hoc profiled runs; only their medians are recorded here.)
  macOS shows no such shift (1.001 and 1.015). So on Zen 4 the ASCII parse loop
  alone can swing ~0.7 ms on an unrelated change. #21 should compare aligned
  builds, or several layouts, before claiming a parser gain or loss.
- **What does `TERMSHOT_PROFILE` itself cost?** The profiled/plain paired wall
  ratio (batch A, branch binary) is 1.00-1.05 on macOS and 0.99-1.08 on
  Linux. It is largest where many cells each read the clock several times relative to little other work
  (`cjk-none` 1.049 / 1.079, `ascii-overflow` 1.050 / 1.037) and near 1.00 for
  large images. Stage medians therefore slightly overstate per-cell stages; the
  plain runs, not the profiled ones, give the end-to-end numbers above.

### Remaining limits

- Each host ran two warm batches; the Mac is a shared desktop with a load
  average of 3.7-5.0 during them, so its p95 values in particular are noisy. No
  wall-clock threshold belongs in CI; the pixel and codec checks stay portable.
- Cold-cache results are Linux only, 15 runs per binary and case.
- The full Noto CJK collection comes from the system, not the repository; its
  hash is recorded and `bench.py` skips the cases without it.
- `glyph-overflow.pty` depends on Python's Unicode database for category and
  width; `perf-fixtures.py --check` detects a change. Version 3.14.7 made it.

### Reproduce

```sh
python3 scripts/build-baseline.py /tmp/termshot-main --revision d83c8fd
./build.sh && cp termshot /tmp/termshot-branch
python3 scripts/bench.py \
  --binary main=/tmp/termshot-main/original --binary branch=/tmp/termshot-branch \
  --describe main=d83c8fd --describe branch="$(git rev-parse --short HEAD)" \
  --unchecked main --reference main --runs 40 --warmups 5 --memory-runs 5 \
  --verify-identical --cjk-font /usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc \
  --seed 17 --output /tmp/termshot-a.json        # again with --seed 29
python3 scripts/bench.py ... --case font-builtin --case font-file \
  --case cjk-subset --case cjk-full --runs 5 --warmups 1 --cold-runs 15 \
  --seed 41 --output /tmp/termshot-cold.json      # Linux, passwordless sudo
python3 scripts/bench-report.py /tmp/termshot-a.json /tmp/termshot-b.json
```

`--suite legacy` or `--suite fonts` runs one half; `--case` and `--cold-case`
are repeatable. A path check whose counter is missing fails the batch;
`--unchecked main` exempts a binary that predates the counters, as `d83c8fd`
does, so only the branch binary's counters were checked.

## Historical: third optimization round (2026-10-01, `c44d83c`, Apple M3)

The section below is unchanged from that round, except for its heading level.
Its harness ran plain and profiled runs in separate loops, and its timers
charged the built-in font's preparation to `font_check_ms`.

### Method and limits

Apple M3, 24 GiB RAM, macOS 26.3.1 ARM64; rustc 1.98.1
(48a229cea 2026-09-01), Apple clang 17.0.0 (clang-1700.6.4.2), Python 3.14.7.
Both binaries retain C `-O2`, Rust `opt-level=2`, and `-ffp-contract=off`.
No runtime dependencies or architecture-specific intrinsics were added.

Two complete batches use **the same binary hashes**, with different execution
order seeds (17 and 29). Each workload/binary has five warmups, 40 ordinary CLI
runs, 40 separate profiled runs, and five separate peak-RSS runs per batch.
Baseline and candidate execution is shuffled within every round. Builds, tests,
and other benchmark batches did not run concurrently with these measurements.
This is still a shared machine: load-average snapshots are recorded, and no
claim is made that scheduling, thermal state, or other applications were fixed.

These workloads use an explicit external JetBrains Mono font via the compatible
positional CLI, so font timings include file reading; the built-in-font path is
not timed here. CLI wall time includes launch, input, validation, parsing, rendering, encoding,
file close, and exit. Child CPU time is the `RUSAGE_CHILDREN` user+system delta
around that same process; resource queries are outside the timed wall interval.
CPU time excludes waiting and the benchmark parent's work, so it is useful
corroboration, not a substitute for elapsed time. Filesystem caches are warm;
file writes stop at `fclose`, without `fsync`.

[Batch A](performance-results.json) and [batch B](performance-repeat.json) retain
all wall/CPU/profile/RSS samples, means, medians, nearest-rank p95, execution
seeds, toolchain metadata, and binary/input/font/PNG hashes. No samples were
discarded. All **15 PNGs match byte for byte** across both builds and batches.

### End-to-end results

Batch A, median / p95 elapsed milliseconds:

| Workload | Main | Optimized | PNG bytes (both) |
| --- | ---: | ---: | ---: |
| reply-sent | 9.09 / 9.79 | 8.66 / 9.38 | 205,783 |
| draft-ready | 9.82 / 14.06 | 9.38 / 12.33 | 200,967 |
| reply-24px | 5.91 / 8.56 | 5.66 / 7.23 | 79,295 |
| reply-128px | 32.40 / 37.63 | 30.50 / 36.24 | 954,758 |
| real-shell | 6.37 / 7.15 | 6.16 / 6.74 | 110,521 |
| real-less | 5.86 / 7.19 | 5.82 / 7.44 | 85,779 |
| real-vi | 8.41 / 17.34 | 8.10 / 13.86 | 187,698 |
| blank | 5.97 / 6.46 | 5.79 / 6.40 | 95,468 |
| color-grid | 19.51 / 23.50 | 18.27 / 22.73 | 649,294 |
| ascii-overflow | 17.02 / 18.39 | 6.34 / 6.95 | 34,560 |
| rounded-boxes | 16.47 / 22.16 | 16.10 / 24.23 | 140,976 |
| dense | 12.48 / 13.08 | 11.76 / 12.50 | 361,204 |
| ansi-replay | 21.11 / 24.33 | 20.13 / 22.09 | 205,783 |
| large | 43.36 / 52.46 | 39.98 / 45.98 | 1,362,101 |
| unicode | 9.71 / 10.31 | 7.84 / 8.27 | 157,153 |

The two sample logs, blank screen, dense ASCII, and rounded boxes use 100×30
cells at 48 px. Sample variants use 24 and 128 px. Real shell, less, and vi logs
come from `tests/vt/real/` at 80×24 / 48 px. Random colors use 100×30 / 24 px,
seed 13, per-cell foreground/background colors and printable ASCII.
`ascii-overflow` prints four million ASCII bytes with automatic wrap and scroll.
`ansi-replay` repeats the sample 250 times (4,716,750 bytes). `large` is 240×80 /
48 px (5280×3840 pixels); `unicode` cycles through 800 codepoints at 100×30 /
24 px, including missing glyphs. Generated multiline logs use CR LF.

### Repeated paired comparisons

For each round, divide baseline elapsed time by candidate elapsed time, then
report the median ratio and a descriptive 95% percentile bootstrap interval
(2,000 resamples of entire pairs, seed 42). These ratios need not equal ratios
of the independently computed medians above. Intervals describe these samples;
temporal correlation and shared-machine interference limit statistical inference.
They are not cross-platform guarantees or a correction for systematic bias.

| Workload | Batch A paired speedup [95% interval] | Batch B paired speedup [95% interval] |
| --- | ---: | ---: |
| reply-sent | 1.048× [1.031, 1.064] | 1.048× [1.040, 1.064] |
| draft-ready | 1.042× [1.026, 1.064] | 1.044× [1.028, 1.069] |
| reply-24px | 1.030× [1.012, 1.063] | 1.029× [1.018, 1.071] |
| reply-128px | 1.064× [1.043, 1.073] | 1.057× [1.034, 1.088] |
| real-shell | 1.040× [1.021, 1.056] | 1.043× [1.015, 1.069] |
| real-less | 1.016× [1.002, 1.031] | 1.025× [1.018, 1.039] |
| real-vi | 1.054× [1.026, 1.083] | 1.035× [1.019, 1.059] |
| blank | 1.019× [1.003, 1.046] | 1.045× [1.010, 1.062] |
| color-grid | 1.054× [1.032, 1.066] | 1.053× [1.044, 1.064] |
| ascii-overflow | 2.691× [2.629, 2.745] | 2.710× [2.679, 2.730] |
| rounded-boxes | 1.035× [1.016, 1.042] | 1.035× [1.017, 1.046] |
| dense | 1.066× [1.050, 1.082] | 1.066× [1.045, 1.079] |
| ansi-replay | 1.050× [1.035, 1.070] | 1.063× [1.043, 1.068] |
| large | 1.086× [1.070, 1.113] | 1.091× [1.079, 1.102] |
| unicode | 1.245× [1.226, 1.254] | 1.227× [1.217, 1.236] |

Long ASCII and Unicode workloads benefit most from skipping overwritten rows
and retaining glyphs. Small-case comparisons remain variable; some intervals
cross 1.0. Tail latency does not improve uniformly. All samples are retained,
and claims rely on both batches and stage evidence rather than the lowest time.

### CPU and memory

Batch A medians. CPU is milliseconds per process; RSS is process peak MiB
(1,048,576 bytes), measured independently with `/usr/bin/time -l`.

| Workload | Child CPU: main → optimized (ms) | Peak RSS: main → optimized (MiB) |
| --- | ---: | ---: |
| reply-sent | 7.79 → 7.39 | 13.64 → 13.67 |
| draft-ready | 8.32 → 7.93 | 13.64 → 13.64 |
| reply-24px | 4.66 → 4.44 | 6.72 → 6.73 |
| reply-128px | 30.09 → 28.01 | 69.22 → 69.20 |
| real-shell | 5.27 → 5.08 | 10.20 → 10.17 |
| real-less | 4.77 → 4.65 | 10.09 → 10.11 |
| real-vi | 7.17 → 6.86 | 10.34 → 10.31 |
| blank | 4.87 → 4.63 | 12.39 → 12.39 |
| color-grid | 17.92 → 16.90 | 7.38 → 7.31 |
| ascii-overflow | 15.74 → 5.16 | 10.20 → 7.92 |
| rounded-boxes | 14.86 → 14.57 | 12.59 → 12.61 |
| dense | 11.16 → 10.50 | 13.61 → 13.64 |
| ansi-replay | 19.58 → 18.60 | 18.12 → 18.16 |
| large | 40.70 → 37.26 | 66.05 → 66.06 |
| unicode | 8.56 → 6.67 | 7.23 → 7.20 |

Input bytes are now freed after parsing, before raster/compressor allocation.
For the long ASCII workload peak RSS changes from 10.20 to 7.92 MiB.
The ANSI replay's peak RSS barely changes: freeing a live buffer does not promise
an equal reduction in maximum resident memory, which also reflects earlier
allocations and allocator retention. `parse_ms` includes the input-buffer release.

Glyph cache metadata increases from 12 to 48 KiB on 64-bit builds; up to 1,024
coverage bitmaps are retained per render instead of 256. Large or diverse fonts
can therefore use more cache memory even though the measured Unicode workload
has fewer allocations and slightly lower RSS. Cache collisions still evict and
free old bitmaps. RGB raster sizes and the existing image-size guard are unchanged.

### Stage timings

Batch A, separately profiled medians for `reply-sent`. Row medians do not need to
sum to the median total, and per-operation profiling overhead is absent from
ordinary CLI measurements.

| Stage | Main (ms) | Optimized (ms) |
| --- | ---: | ---: |
| `input_read_ms` | 0.0308 | 0.0231 |
| `parse_ms` | 0.0611 | 0.0594 |
| `font_load_ms` | 0.2219 | 0.2254 |
| `font_read_ms` | 0.0499 | 0.0537 |
| `font_check_ms` | 0.1033 | 0.1037 |
| `font_padding_ms` | 0.0675 | 0.0692 |
| `font_setup_ms` | 0.0020 | 0.0020 |
| `allocate_ms` | 0.0030 | 0.0030 |
| `background_ms` | 0.8600 | 0.8720 |
| `foreground_ms` | 0.6525 | 0.6605 |
| `geometry_ms` | 0.0695 | 0.0710 |
| `glyph_ms` | 0.2585 | 0.2590 |
| `blend_ms` | 0.2675 | 0.2760 |
| `png_filter_ms` | 0.0000 | 0.0000 |
| `png_deflate_ms` | 4.1355 | 3.7605 |
| `deflate_allocate_ms` | 0.0040 | 0.0040 |
| `deflate_match_emit_ms` | 3.2280 | 2.8505 |
| `deflate_finalize_ms` | 0.0010 | 0.0010 |
| `deflate_checksum_ms` | 0.8825 | 0.9105 |
| `png_pack_ms` | 0.1080 | 0.1115 |
| `png_encode_ms` | 4.2475 | 3.8705 |
| `output_write_ms` | 0.1500 | 0.1735 |
| `cleanup_ms` | 0.0010 | 0.0010 |
| `total_ms` | 6.2940 | 6.0362 |

Timing boundaries and nesting are unchanged from the previous round:

- `font_read`, `font_check`, and `font_padding` are inside `font_load`.
- `geometry`, `glyph`, and `blend` are inside `foreground`. Geometry includes
  dispatch for ordinary characters; glyph timing covers lookup, allocation,
  rasterization, and cache handling. Decoration drawing and freeing glyph/arc
  caches also fall within `foreground`, outside those three child timers.
- `png_filter`, `png_deflate`, and `png_pack` are inside `png_encode`. Scanlines
  already contain zero filter bytes prepared in `background`; filtering is only
  a timer boundary. `png_pack` includes PNG allocation, copying, and CRC-32.
- `deflate_allocate`, `deflate_match_emit`, `deflate_finalize`, and
  `deflate_checksum` are inside `png_deflate`: setup/headers; search/emission;
  freeing search storage and any stored-block fallback; Adler-32/trailer.
- `font_setup` includes stb initialization and metrics (the optional diagnostic is disabled).
  `allocate` times raster allocation; first-touch costs fall under `background`.
  `output_write` includes open/write/close and freeing the PNG. `cleanup` frees
  the raster. Rust `total` includes C profile output, but excludes process startup,
  Rust's final teardown, and its own profile output.

Do not add parent and child values. CPU, CLI, and profiled medians come from
different boundaries or processes; subtracting them cannot isolate startup cost.
Font reading/validation/padding, background painting, alpha blending, Adler-32,
CRC-32, and file writing have no algorithmic changes this round. Their measured differences
must not be presented as optimizations to those stages.

### Retained changes

- Skip complete ASCII rows that will necessarily scroll out before the end of
  the current uninterrupted run. Rotate the row map by the equivalent count,
  retaining at least one full scrolling region of actual writes. Avoid clearing
  a row immediately before overwriting every cell. Attributes, pending wrap,
  disabled wrapping, alternate screens, margins, and REP state are preserved.
  Long-ASCII parsing: **11.841 → 1.290 ms**.
- Accumulate CSI digits in a wide integer and clamp once, preserving the same
  saturating u32 value with fewer overflow checks. Combined parser changes take
  ANSI replay parsing from **10.415 → 9.776 ms**.
- Increase the bounded glyph cache to 1,024 slots. The Unicode workload goes
  from 1,027 rasterizations / 0 cache hits to
  284 rasterizations / 1,888 hits; glyph time is **2.279 → 0.815 ms**.
  Cache-collision fixtures still exercise eviction at the new size, including
  missing glyphs. Wide-glyph cache keys retain main's behavior.
- Flush DEFLATE bits in a bounded four-byte store, with one capacity check per
  token instead of per emitted byte. Compare matches in bounded 16-byte groups
  plus tails, preserving the first differing byte, match ordering, and stream
  bytes. Sample match/emission time is **3.228 → 2.851 ms**;
  the large case is **19.470 → 16.006 ms**. Allocation
  failures still drain logical bits and return failure without leaking or looping.
- Release input storage before rendering; retain its length for profiling.

### Rejected experiments and remaining work

Before integration of the newer CLI/color commits, screening runs used
interleaved comparisons and exact PNG checks. A 32-byte
match-comparison variant increased match/emission time on random-color and
large images (about 12.175 → 13.212 ms and 16.694 → 17.632 ms in that screening
batch), so it was discarded. A circle-span variant slightly improved the rounded
grid but raised high-resolution geometry from about 0.778 to 1.565 ms. It was
also discarded. The final arc rasterizer is unchanged; rounded-grid end-to-end
improvements come from other stages. Screening numbers are separate runs and
must not be compared directly with the final tables.

Uniform alpha blending initially helped on the PR #10 base but regressed after
integrating the newer rendering features: large-image blending measured
6.095 → 9.485 ms against `bd726a6`. Restoring the branch-based formula reduced
that stage from 8.901 to 5.693 ms in a separate comparison batch.
The uniform formula was removed; the final blending code matches the baseline.
This is why the complete benchmark was repeated after integrating main.

Remaining batch-A costs include **3.870 ms** of sample PNG encoding,
**9.776 ms** of ANSI parsing, and **6.171 ms** of rounded-grid geometry.
The large image still spends **5.434 ms** painting backgrounds and
**5.372 ms** blending. Optimizing small I/O and font stages further was
not justified by these profiles. More glyphs than the cache can hold, different
fonts, fallback-font rendering, cold storage, and other architectures remain
outside these measurements. No absolute optimum or universal latency is claimed.

An integration check also reproduced main's existing panic when printing a
wide character (`界`) on a single-column screen (`--size 1x1`). The unchanged
baseline and this branch share that limitation; wide-character differential
coverage here uses grids with at least two columns.

### Validation and reproduction

Local macOS validation passed:

- `SANITIZE=1 ./test.sh`: 83 unit tests passed (the manual POC helper remains
  ignored), 3,000 byte-identical compressor comparisons, 36,901 box/block checks,
  56 glyph-placement checks, CLI checks, and 20 native pixel goldens.
- `SANITIZE=1 UBSAN_OPTIONS=halt_on_error=1 ./tests/run.sh`: 5,024 independent
  PNG/zlib round trips per compressor, 48 PNG integrity cases per compressor,
  11 injected allocation failures, aligned/unaligned CRC checks, and 8 concurrent
  profiled renders. The PNG cases check split IDAT streams, incorrect Adler-32
  trailers with valid chunk CRCs, and incorrect chunk CRCs.
- The parser test compares batch writes with individual prints across screen
  sizes, scrolling margins, wrap states, alternate screens, and run lengths.
  It also checks combining-character targets and overwrites of existing wide
  characters on grids with room for them. Another 10,000 generated streams,
  including wide/combining characters and long ASCII runs, match every output
  cell against `c44d83c`; 5,000 font mutations retain the same acceptance result.
- The two new collision/distant-geometry fixtures are stored in `tests/fixtures/`
  and run by `tests/golden.rs`. Their goldens come from an independent build of
  `c44d83c`; all 18 existing native goldens remain unchanged. Both benchmark
  batches match all 15 complete PNG hashes. The baseline helper was exercised.

Linux and Rust 1.70 execution have not been repeated locally this round. The
existing CI covers Linux/macOS, sanitizers, and the minimum Rust version; it has
no shared-runner timing thresholds.

Choose a baseline destination that does not exist:

```sh
python3 scripts/build-baseline.py /tmp/termshot-baseline
./build.sh
python3 scripts/bench.py \
  --binary baseline=/tmp/termshot-baseline/original \
  --binary optimized=./termshot --reference baseline \
  --runs 40 --warmups 5 --memory-runs 5 --verify-identical --seed 17 \
  --output /tmp/termshot-a.json
python3 scripts/bench.py \
  --binary baseline=/tmp/termshot-baseline/original \
  --binary optimized=./termshot --reference baseline \
  --runs 40 --warmups 5 --memory-runs 5 --verify-identical --seed 29 \
  --output /tmp/termshot-b.json
```

`--case` narrows workloads and is repeatable. Peak RSS uses `/usr/bin/time -l`
on macOS and `-v` on Linux, normalized to bytes. The default baseline is now
`c44d83c`, which already has all stage timers and embeds the bundled font.
Older instrumented baselines can still be built using explicit `--revision`
and `--profile-patch` options.

`TERMSHOT_PROFILE` is enabled by presence, including `0`, and emits two
`termshot-profile ` JSON records to stderr. Timers are thread-local and rendering
buffers/caches are per-call. Python is needed only for optional development
scripts, including benchmarks and `scripts/generate-crc32.py`, not the build
or tests.

### Where the historical “~20 ms” came from

[README at `fb714a5`](https://github.com/momiji-rs/termshot/blob/fb714a5/README.md)
recorded **21 ms mean** for the 2200×1440 sample on Apple M3, using 40 hyperfine
runs, and rounded this to “about 20 ms” in prose. Those original samples were
not checked into the repository. That mean and the later PR's 19.50 ms median
came from different measurement batches and cannot establish improvement by
simple subtraction.

A separate [historical audit](performance-history-audit.json) rebuilt `fb714a5`
and interleaved 60 runs against main `1eaf7dd` and PR #10 `34ceb1a`. Medians were
16.06, 13.76, and 8.77 ms respectively, with identical PNGs. The latter two
executables had the exact same hashes as the earlier 30.59 / 19.50 ms report.
This demonstrates why absolute timings from separate batches need context.
The current README omits a fixed millisecond claim in its introduction and
presents versioned, paired results with their measurement conditions instead.

## Published claims and their evidence (checked 2026-10-03)

This section records each latency figure published outside this report, where
it came from, and whether its evidence is in the repository. A figure is traceable only when its
raw samples, revision, input, font, image size, machine, statistic and timing
boundary are all recorded. The measurement rounds above, with their JSON files,
meet that bar. The historical “~20 ms” notes above and the figures below do
not, so none of them should be quoted as a current result.

### “~20 ms for 2200×1440” (the repository's About description)

**Provenance: partly documented, the measurement itself unverified.**

- **Where it first appears**: the README at
  [`fb714a5`](https://github.com/momiji-rs/termshot/blob/fb714a5/README.md)
  (2026-10-01): “The screenshot below, 2200×1440, takes about 20 ms from start
  to finished file”. Its Speed table gave 21 ms for px 48 on an Apple M3
  (macOS) and 15 ms on a Ryzen 7 8745HS (Linux).
- **Command and input**: `./termshot examples/reply-sent.pty out.png <font> 48`,
  the positional CLI, when a font file was still required. The font is not
  named; the README's example at that revision passes
  `third_party/jetbrains-mono/JetBrainsMono-Regular.ttf`. 100×30 cells, so a
  2200×1440 image.
- **Revision**: `fb714a5` changes only `README.md`, so the source it describes
  is its parent `c1ff1f9`'s (RGB PNGs with no row filter, on top of the faster
  deflate in `7a7eb19`), built with `build.sh`. No binary hash was kept, so
  which build was timed is inferred, not recorded.
- **Boundary and statistic**: one whole CLI process, “process start, parse,
  rasterize, PNG encode, and the file write”; the **mean** of 40 runs, with
  hyperfine on macOS and a shell loop on Linux (the commit message). Whether
  hyperfine used warmups or a shell is not recorded.
- **Not recorded anywhere**: the samples, the macOS version, the compilers,
  hyperfine's options and the machine's load. The 21 ms cannot be recomputed.
- **Later remeasurement**: [the historical audit](performance-history-audit.json)
  (2026-10-01T21:29Z, the same M3) has a binary labelled `historical`, sha256
  `c4fb34d1bbea…`, at a **16.06 ms median** of 60 runs. The JSON does not
  record that binary's revision; the third round's notes say it was rebuilt
  from `fb714a5`. Confirming that needs a rebuild with the audit's toolchain
  (Apple clang 17.0.0, rustc 1.98.1) on that machine; it was not repeated.
- **The “newer ~19–20 ms” results** #23 mentions are PR #10's
  **19.50 ms median** (30 runs) for `34ceb1a` against 30.59 ms for main
  `1eaf7dd`, kept in
  [`34ceb1a:docs/performance-results.json`](https://github.com/momiji-rs/termshot/blob/34ceb1a/docs/performance-results.json)
  (the file was overwritten by later rounds). That batch was disturbed: its
  means were 45.28 and 24.82 ms. The audit timed the same two binaries (sha256
  `a04a42c5ec96…` and `74314e5a09c0…`) at 13.76 and 8.77 ms one batch later.
  Neither 21 ms nor 19.50 ms is a stable property of the code.
- **Today's nearest equivalent** is `font-builtin` / `reply-sent` in the current
  baseline: 9.17-9.89 ms median on an Apple M2 Max and 9.58-9.81 ms on the
  Ryzen 7 8745HS (both cases, both batches, `22b77e8`). The machine, revision,
  harness and font path all differ from `fb714a5`'s, so the gap from 21 ms is
  not a measured speedup.

### “~130 ms render”, “about 90% is stb's PNG deflate” (issue #1)

From #4: reply-sent at px 48 on the Apple M3, 2026-10-01, about 128 ms
(128.6 ± 2.3 ms), with stb's `stbi_zlib_compress` 84-92% of it. That was the
code before `7a7eb19`; #4's own samples are not in the repository. It no longer
describes termshot. In the current baseline (`font-builtin`, the same log at
px 48, batch A profiled runs), the median `png_deflate_ms` is 3.60 ms against a
median `total_ms` of 5.94 ms on macOS and 5.79 against 8.59 ms on Linux; the
median per-run share is 61% and 68%, and `glyph_ms`'s 4% and 3%. These are
internal timers, which leave out process start-up and exit.

### “about 18 ms for a 2200×1440 frame instead of about 140 ms” (CHANGELOG, 0.1.0)

Added in `0454114`. No command, machine, statistic or samples are recorded
with it; it agrees with #4 and the `fb714a5` table only roughly.
**Provenance unverified.** It stays as written because it is part of a
released changelog entry.

### “a text-only run takes about 1 ms where a PNG takes 10” (README, CHANGELOG)

From `f2d0863`'s commit message (0.85 vs 9.9 ms for reply-sent), with no
machine or samples recorded. **Provenance unverified**; the README and the
unreleased changelog entry no longer give a ratio.
