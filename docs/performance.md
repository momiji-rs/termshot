# Performance measurements

These archived measurements predate the merge of `670cbbb`, which added font
validation and concurrent rendering. They compare against `8e1110e`, not the
latest main. Current profiling reports Rust font reading, validation, and padding
as `font_load_ms`; the historical `font_read_ms` field below covered C file I/O
only. Rerun the benchmark to measure the combined implementation.

Measured on 2026-10-01, macOS ARM64 (`macOS-26.3.1-arm64-arm-64bit-Mach-O`).

Toolchain: rustc 1.98.1 (48a229cea 2026-09-01) (Homebrew), Apple clang version 17.0.0 (clang-1700.6.4.2), Python 3.14.7. Both builds use the existing `-O2` / `opt-level=2` and `-ffp-contract=off` settings. No new runtime dependencies.

## End-to-end results

The reference is **main at `8e1110e`**, which already includes the faster custom compressor, RGB output, no PNG row filter, allocation-free CSI parsing, terminal parsing fixes, and an image size limit. The results below measure the additional changes in this PR.

Each case uses three warmups, then 15 fresh processes per binary. Binary order is shuffled with a fixed seed every round. CLI timings include process launch, replay, rendering, encoding, and file close, with profiling disabled. A third, instrumented baseline separates per-stage timings from ordinary CLI timing.

| Workload | Main median / p95 (ms) | Optimized median / p95 (ms) | Speedup | PNG bytes (both) |
| --- | ---: | ---: | ---: | ---: |
| reply-sent | 31.29 / 69.91 | 26.78 / 56.86 | 1.17× | 205,774 |
| draft-ready | 27.71 / 69.36 | 22.34 / 50.49 | 1.24× | 200,974 |
| blank | 17.79 / 24.47 | 16.89 / 27.22 | 1.05× | 95,468 |
| dense | 42.45 / 51.01 | 30.67 / 33.13 | 1.38× | 474,803 |
| ansi-replay | 37.78 / 48.20 | 34.97 / 56.46 | 1.08× | 205,774 |
| large | 195.49 / 315.32 | 116.98 / 225.06 | 1.67× | 1,501,397 |
| unicode | 15.88 / 16.87 | 14.51 / 21.15 | 1.09× | 154,584 |

The two sample logs use 100×30 cells / 48 px. `blank` uses the same dimensions.
`dense` fills the grid with repeated ASCII. `ansi-replay` repeats the sample 250
times (4,716,750 bytes). `large` uses 240×80 cells / 48 px (5280×3840 pixels).
`unicode` cycles through 800 codepoints at 100×30 / 24 px, including missing glyphs
and cache collisions. Generated multiline fixtures use CR LF, matching PTY logs.

[performance-results.json](performance-results.json) retains every raw sample,
profile record, binary hash, output size, and toolchain version. p95 uses nearest
rank; with 15 samples it is the slowest run. No samples were discarded. This was
a shared machine with visible scheduling outliers and warm filesystem caches;
these values are not cold-disk or cross-platform speed guarantees. For example,
blank and Unicode p95 increased even though their medians fell.

## Stage timings

These are separate profiled-run medians for `reply-sent`. The baseline is
`8e1110e` with the same timers and a split between PNG encoding and file writing.
Row medians need not sum to the median total.

| Stage | Instrumented main (ms) | Optimized (ms) |
| --- | ---: | ---: |
| input_read | 0.1171 | 0.0506 |
| parse | 0.0914 | 0.0738 |
| font_read | 0.1190 | 0.0940 |
| font_setup | 0.0170 | 0.0140 |
| allocate | 0.0060 | 0.0050 |
| background | 3.3420 | 1.4330 |
| foreground | 4.1280 | 1.0900 |
| geometry | 0.2750 | 0.1360 |
| glyph | 2.7080 | 0.4230 |
| blend | 0.8020 | 0.4300 |
| png_filter | 2.5930 | 2.0690 |
| png_deflate | 19.7690 | 12.0470 |
| png_pack | 0.8740 | 0.8180 |
| png_encode | 23.4370 | 15.3990 |
| output_write | 0.4410 | 0.3520 |
| cleanup | 0.0020 | 0.0020 |
| total | 33.3813 | 18.6185 |

`geometry`, `glyph`, and `blend` are nested inside `foreground`; `png_filter`,
`png_deflate`, and `png_pack` are nested inside `png_encode`. Do not add parent
and child values together. `glyph` includes lookup, cache handling, allocation,
and rasterization. Geometry includes dispatch for ordinary characters.

`font_setup` includes font initialization, metrics, and the existing stderr
message. `allocate` is the image `malloc`; first-touch page costs appear in
`background`. `output_write` includes opening, writing, closing, and freeing the
PNG buffer. `cleanup` freed the raster and font in the recorded build; current C
cleanup frees only the raster, as Rust now owns the font. `total` starts in Rust `main`
and includes C profile output. Process startup, final teardown, and scheduling
are only in CLI wall time. Writes stop at `fclose`; they do not include `fsync`.

Parsing, font I/O, allocation, and PNG filter selection were unchanged between
the two measured builds.
Differences in those stages reflect run-to-run noise, not an optimization. The
sample profile batches show especially visible variation; use interleaved CLI
medians for overall comparisons. Profile times and CLI times come from separate
runs and cannot be subtracted to estimate exact process-startup time.

## Changes

- Cache up to 256 coverage bitmaps for one render, sharing them across colors and
  bold text. Collisions evict safely and missing glyphs are cached. The sample
  goes from 462 rasterizations to 63; the large case goes from 17,280 to 22.
  Unique or colliding Unicode can still miss; the cache does not grow with the
  number of codepoints.
- Paint one background scanline per terminal row and copy it through the cell
  height. Clip glyph bounds once before blending. Blend rounding and geometric
  drawing remain unchanged.
- Reject DEFLATE candidates whose byte at the current best length differs, since
  they cannot produce a longer match. The existing newest-match tie preference
  makes this safe; the same check speeds lazy matching. Nonempty compressed
  output remains identical to the reference compressor.
- Check short PNG writes and `fclose` errors, release compressed data on PNG
  allocation failure, and repair the existing empty-input zlib defect in both
  compressors. Local stb hooks and fixes are documented in
  [CHANGES.md](../third_party/stb/CHANGES.md).

The raster remains RGB: 9,504,000 bytes for the sample and 60,825,600 bytes for the
large case, unchanged from main. These are buffer sizes, not peak RSS. The glyph
cache adds bounded per-render storage. RGB output, the no-filter PNG setting,
terminal semantics, image size guard, portable floating-point flags, and the
existing custom compressor are retained from main.

## Reproduce

Choose a destination that does not exist. The helper reads historical source
without switching the checkout, builds the reference, applies the checked-in
profiling patch, and builds an instrumented baseline. The repository must contain
commit `8e1110e`.

```sh
python3 scripts/build-baseline.py /tmp/termshot-baseline
./build.sh
python3 scripts/bench.py \
  --binary main=/tmp/termshot-baseline/original \
  --binary baseline=/tmp/termshot-baseline/baseline \
  --binary optimized=./termshot \
  --runs 15 --warmups 3 --output /tmp/termshot-comparison.json
```

Use `--case reply-sent` (repeatable) to narrow a run. Run on an otherwise idle
machine, and compare output sizes along with timings.

```sh
TERMSHOT_PROFILE=1 ./termshot examples/reply-sent.pty /tmp/reply.png \
  third_party/jetbrains-mono/JetBrainsMono-Regular.ttf
./test.sh
SANITIZE=1 UBSAN_OPTIONS=halt_on_error=1 ./tests/run.sh
```

The profile variable is enabled by its presence, including a value of `0`. It
emits two JSON records prefixed `termshot-profile ` on stderr. Foreground
profiling uses per-operation clocks; ordinary CLI benchmark runs disable these.
Timing hooks use thread-local storage, and the canvas and glyph cache belong to
each render. The extended suite checks eight concurrent profiled renders for
identical pixels and independent, finite timing records.

Python 3 is needed for the optional benchmarks and extended tests, not the build
or the existing `./test.sh` suite.

## Validation and remaining bottlenecks

The 16 extended goldens were generated from **main at `8e1110e`**, preserving its
corrected bare-LF, erase, SGR, and escape behavior. Tests independently decode PNG
filters with Python's standard library, verify CRCs, and compare dimensions and
RGB SHA-256 hashes. Coverage includes sample logs, empty input, geometry at six
sizes, clipping, bold, cache collisions, missing glyphs, control sequences, and
seeded random colors/cursor motion. Profiling must produce valid finite timings
without changing pixels; unwritable destinations must fail.

Each compressor has 4,976 independent zlib/PNG round trips, covering all PNG
filters, 1–4 channels, padded strides, flipping, random bytes, repetitive inputs,
short inputs, and window boundaries. Both codec variants and the C renderer run
under ASan/UBSan in the extended suite. Existing coverage also passes: 43 parser/font/drawing
unit tests, 3,000 compressor differential cases, CLI checks, and portable pixel
goldens. Local validation ran on macOS; CI includes Linux, macOS, sanitizer, and
Rust 1.70 jobs. Timing thresholds are excluded from shared-runner CI.

PNG encoding still dominates (15.40 ms of the sample's 18.62 ms profiled total).
The long replay spends 12.52 ms of 29.73 ms parsing, which is unchanged from main.
Further work should measure alternate compressors or a parser fast path against
these distributions while retaining output-size, terminal-semantics, and
portability constraints. This benchmark does not establish an absolute optimum
or cover every font, storage device, and architecture.
