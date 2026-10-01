# Performance measurements

Measured on 2026-10-01 against **main at `1eaf7dd`**, after the previous
performance PR and terminal controls, scrolling, and alternate-screen support
were merged. That reference already includes glyph caching,
background scanline reuse, faster DEFLATE match search, RGB output, no PNG row
filter, checked fonts, and concurrent rendering. These results measure the
additional improvements in this round.

Environment: Apple M3, 24 GiB RAM, macOS 26.3.1 ARM64; rustc 1.98.1
(48a229cea 2026-09-01), Apple clang 17.0.0 (clang-1700.6.4.2), Python 3.14.7.
Both builds retain C `-O2`, Rust `opt-level=2`, and `-ffp-contract=off`.
There are no new runtime dependencies or architecture-specific intrinsics.

## End-to-end results

Each case uses three warmups followed by 30 fresh processes per binary. Binary
order is shuffled with a fixed seed every round. Ordinary CLI timings include
launch, input, validation, parsing, rendering, encoding, file close, and process
exit; profiling is disabled. Another 30 interleaved runs measure stages, using
a third binary with only the additional instrumentation applied to the baseline.
Peak RSS uses five separate, interleaved `/usr/bin/time -l` runs per binary/case.

| Workload | Main median / p95 (ms) | Optimized median / p95 (ms) | Speedup | PNG bytes (both) |
| --- | ---: | ---: | ---: | ---: |
| reply-sent | 30.59 / 145.82 | 19.50 / 52.34 | 1.57× | 205,774 |
| draft-ready | 19.31 / 33.08 | 12.74 / 17.70 | 1.52× | 200,974 |
| reply-24px | 9.28 / 13.77 | 7.14 / 10.48 | 1.30× | 79,300 |
| reply-128px | 75.19 / 88.39 | 35.78 / 50.72 | 2.10× | 954,758 |
| blank | 12.59 / 16.37 | 8.03 / 17.01 | 1.57× | 95,468 |
| color-grid | 22.18 / 23.51 | 17.67 / 18.31 | 1.26× | 649,294 |
| ascii-overflow | 23.84 / 25.42 | 16.17 / 16.95 | 1.47× | 34,560 |
| rounded-boxes | 23.37 / 23.93 | 15.41 / 16.23 | 1.52× | 141,124 |
| dense | 18.32 / 19.15 | 11.96 / 12.78 | 1.53× | 361,204 |
| ansi-replay | 26.36 / 27.87 | 18.96 / 19.95 | 1.39× | 205,774 |
| large | 72.39 / 83.97 | 39.82 / 46.14 | 1.82× | 1,362,101 |
| unicode | 11.26 / 11.71 | 8.96 / 10.04 | 1.26× | 154,584 |

The sample logs, blank screen, dense ASCII, and rounded boxes use 100×30 cells
at 48 px (2200×1440 pixels). `reply-24px` and `reply-128px` use the same sample
at 1100×720 and 5800×3840 pixels. `color-grid` assigns seeded random foreground
and background colors and printable ASCII to every cell at 24 px.
`ascii-overflow` is four million printable ASCII bytes without explicit newlines
at 24 px, stressing automatic wrapping and scrolling. `rounded-boxes` fills the
grid with the four quarter-ellipse characters. `ansi-replay` repeats the sample
250 times (4,716,750 bytes). `large` uses 240×80 cells at 48 px (5280×3840 pixels).
`unicode` cycles through 800 codepoints at 100×30 / 24 px, including missing
glyphs and cache collisions. Generated multiline logs use CR LF.

All 12 resulting PNGs are **byte-identical** across the original, instrumented,
and optimized binaries. This preserves file size, compression choices, checksums,
and pixels. [performance-results.json](performance-results.json) contains every
raw latency, profile, and RSS sample, binary and PNG hashes, and toolchain details.
p95 uses nearest rank (the 29th of 30 sorted latency samples). No samples were
discarded. Filesystem caches are warm; writes end at `fclose`, without `fsync`.

These are measurements on a shared machine, not speed guarantees. Tail latency
and differences between the original and instrumentation-only CLI samples show
the limits of precision; all slow samples remain in the raw report. Blank-screen
p95 rose from 16.37 to 17.01 ms despite a lower median. The old
report against `8e1110e` remains in Git history; its timings came from a different
run and are not comparable here.

## Memory

Median **process peak RSS**, in MiB (1,048,576 bytes), from five independent runs:

| Workload | Main (MiB) | Optimized (MiB) | Reduction |
| --- | ---: | ---: | ---: |
| reply-sent | 22.78 | 13.75 | 39.6% |
| draft-ready | 22.72 | 13.58 | 40.2% |
| reply-24px | 9.00 | 6.66 | 26.0% |
| reply-128px | 132.95 | 69.11 | 48.0% |
| blank | 21.45 | 12.30 | 42.7% |
| color-grid | 9.69 | 7.28 | 24.8% |
| ascii-overflow | 12.47 | 10.12 | 18.8% |
| rounded-boxes | 21.69 | 12.55 | 42.1% |
| dense | 22.69 | 13.48 | 40.6% |
| ansi-replay | 27.22 | 18.03 | 33.8% |
| large | 124.05 | 65.97 | 46.8% |
| unicode | 9.44 | 7.16 | 24.2% |

The renderer now paints directly into PNG scanlines, including one zero filter
marker per row. This removes the second full image buffer and encoding copy.
The sample's RGB payload is still 9,504,000 bytes; the large case is still
60,825,600 bytes. Those payload counts (`pixel_bytes`) exclude scanline markers,
font storage, compressor storage, caches, and process overhead; they are not RSS.
The read-only CRC table adds 8 KiB. Rounded-corner offsets use at most four
arrays of 2,049 pairs of floats (about 64 KiB), allocated only when needed and
freed within the render. Larger arcs or allocation failure use the original path.

## Stage timings

Separate profiled-run medians for `reply-sent`; the baseline is `1eaf7dd` with
only the additional timers. Row medians need not sum to the median total.

| Stage | Instrumented main (ms) | Optimized (ms) |
| --- | ---: | ---: |
| `input_read_ms` | 0.0641 | 0.0625 |
| `parse_ms` | 0.0737 | 0.0604 |
| `font_load_ms` | 0.4460 | 0.3775 |
| `font_read_ms` | 0.0828 | 0.1103 |
| `font_check_ms` | 0.2012 | 0.1277 |
| `font_padding_ms` | 0.1432 | 0.1065 |
| `font_setup_ms` | 0.0115 | 0.0120 |
| `allocate_ms` | 0.0050 | 0.0045 |
| `background_ms` | 1.2790 | 1.2720 |
| `foreground_ms` | 1.7050 | 1.2695 |
| `geometry_ms` | 0.2415 | 0.1985 |
| `glyph_ms` | 0.4215 | 0.3605 |
| `blend_ms` | 0.6360 | 0.5520 |
| `png_filter_ms` | 2.6290 | 0.0000 |
| `png_deflate_ms` | 12.9665 | 6.3715 |
| `deflate_allocate_ms` | 0.0110 | 0.0070 |
| `deflate_match_emit_ms` | 9.1455 | 5.3400 |
| `deflate_finalize_ms` | 0.0070 | 0.0060 |
| `deflate_checksum_ms` | 3.7120 | 1.1050 |
| `png_pack_ms` | 0.5860 | 0.1310 |
| `png_encode_ms` | 16.7405 | 6.4920 |
| `output_write_ms` | 0.4435 | 0.4010 |
| `cleanup_ms` | 0.0020 | 0.0030 |
| `total_ms` | 21.4322 | 10.8914 |

Timing boundaries:

- `font_read`, `font_check`, and `font_padding` are inside `font_load`. Reading
  covers Rust file I/O; checking validates every font structure needed by stb;
  padding resizes the owned buffer. The original binary has only `font_load`.
- `geometry`, `glyph`, and `blend` are inside `foreground`. Geometry includes
  dispatch for ordinary characters. Glyph time includes cache lookup, allocation,
  and rasterization. The sample still rasterizes 63 glyphs with 399 cache hits.
- `png_filter`, `png_deflate`, and `png_pack` are inside `png_encode`.
  `png_filter` is now just the timer boundary: filter markers are prepared during
  `background`, and there is no separate scanline copy or filter selection.
- `deflate_allocate`, `deflate_match_emit`, `deflate_finalize`, and
  `deflate_checksum` are inside `png_deflate`. They cover initial allocation and
  stream headers; match search and bit emission; freeing search buffers and any
  stored-block fallback; then Adler-32 and trailer emission. `png_pack` includes
  final PNG allocation, compressed-data copying, and CRC-32 checksums.
- `font_setup` includes stb font initialization, metrics, and the existing stderr
  message. `allocate` is raster allocation; first-touch page costs fall under
  `background`. `output_write` covers open/write/close and freeing the PNG buffer.
  `cleanup` frees the raster; glyph and arc caches are freed in `foreground`.
- Rust `total` includes C profile output but excludes startup, Rust's final
  teardown, and its own profile output. CLI wall time includes the entire process.
  Foreground profiling uses per-operation clocks, so separate CLI/profile runs
  cannot be subtracted to estimate exact startup cost.

Do not add parent and child timings together. Input/output, background painting,
glyph rasterization, and blending have no algorithmic changes this round; their
small timing differences are not evidence of faster implementations.

## Changes and workload-specific findings

- Batch printable ASCII runs within each physical row, preserving pending wraps,
  scrolling regions, alternate-screen row maps, attributes, Unicode, DEC character
  sets, REP, and control-sequence boundaries. Reuse the CSI parameter
  array across sequences. Parsing falls from **17.381 to 11.082 ms** for the long
  ASCII line and **11.777 to 9.143 ms** for the ANSI replay.
- Validate simple glyph coordinate lengths directly from repeated flag runs,
  avoiding an expanded flag allocation and two scans. Sample validation falls
  from **0.201 to 0.128 ms**; all checks and font padding are retained.
- Cache only translation-independent arc offsets per render, applying the
  original floating-point translation at each cell. Rounded-grid geometry falls
  from **9.230 to 6.219 ms**, with identical pixels.
- Render directly into zero-filter PNG scanlines, eliminating a full image copy
  and allocation. The generic stb writer retains filtering, channels, strides,
  and flipping support through a borrowed-scanline packaging helper.
- Calculate DEFLATE length/distance indexes directly and emit each token in one
  operation using a 64-bit bit buffer. Match selection remains unchanged.
  Sample match/emission time falls from **9.146 to 5.340 ms**.
- Express Adler-32 as independent weighted reductions that the compiler can
  vectorize. Sample checksum time falls from **3.712 to 1.105 ms**. Portable
  slicing-by-eight IEEE CRC-32 through stb's existing hook reduces sample PNG
  packaging from **0.586 to 0.131 ms**. `scripts/generate-crc32.py` reproduces the
  checked-in table; unaligned inputs use explicit byte assembly.

## Reproduce

Choose a destination that does not exist. The helper reads historical source
without switching the checkout, builds the reference, applies
[round-two-baseline.patch](round-two-baseline.patch), and builds an instrumented
baseline. The repository must contain commit `1eaf7dd`.

```sh
python3 scripts/build-baseline.py /tmp/termshot-baseline
./build.sh
python3 scripts/bench.py \
  --binary original=/tmp/termshot-baseline/original \
  --binary baseline=/tmp/termshot-baseline/baseline \
  --binary optimized=./termshot \
  --runs 30 --warmups 3 --memory-runs 5 --verify-identical \
  --output /tmp/termshot-comparison.json
```

Use `--case reply-sent` (repeatable) to narrow a run. `--memory-runs` uses
`/usr/bin/time -l` on macOS or `-v` on Linux, normalizing peak RSS to bytes.
Memory and profile runs are separate from CLI latency runs. Prefer an otherwise
idle machine. Raw output paths identify temporary measurement artifacts; the
helper and checked-in patch reproduce their sources.

```sh
TERMSHOT_PROFILE=1 ./termshot examples/reply-sent.pty /tmp/reply.png \
  third_party/jetbrains-mono/JetBrainsMono-Regular.ttf
SANITIZE=1 ./test.sh
SANITIZE=1 UBSAN_OPTIONS=halt_on_error=1 ./tests/run.sh
```

The profile variable is enabled by its presence, including a value of `0`.
It emits two JSON records prefixed `termshot-profile ` on stderr. Clocks are
opt-in for the new fine-grained stages; timing hooks use thread-local storage,
and rendering buffers and caches belong to each render. The extended suite
checks eight concurrent profiled renders for identical pixels and finite timings.
Python 3 is needed for optional benchmarks and extended tests, not the build or
`./test.sh`.

## Validation and remaining bottlenecks

Local macOS validation passed 69 parser/font/drawing unit tests, 3,000
byte-for-byte compressor comparisons, CLI checks, and portable pixel goldens.
The extended suite passed **5,024 independent zlib/PNG round trips per compressor**,
including all PNG filters, 1–4 channels, padded strides, flipping, Adler-32 block
boundaries, random bytes, repetitive inputs, and DEFLATE window boundaries.
It compares the new CRC against stb on aligned and unaligned buffers. Both
compressors and the C renderer ran under ASan/UBSan. Sixteen extended pixel
fixtures and eight concurrent profiled renders passed. Extended fixtures retain
the updated CR LF expectations from main. The unit suite also includes main's
terminal reference screens and recorded shell, less, and vi sessions.

Additional local differential checks against `1eaf7dd` matched every cell field
for 10,000 generated parser streams and matched acceptance/rejection for 5,000
mutated fonts. All 12 benchmark outputs match the full original PNG SHA-256.
The baseline rebuild helper was also exercised. Linux and Rust 1.70 execution
have not been repeated locally this round; existing CI covers those environments.
Shared-runner CI has no timing thresholds.

The remaining bottleneck depends on the input. PNG encoding is **6.492 of
10.891 ms** of profiled sample time. Random-color compression still spends
**12.387 ms** finding/emitting matches. ANSI replay spends **9.143 of 15.602 ms**
parsing. Rounded boxes spend **6.219 ms** in geometry; the Unicode case spends
**2.179 ms** rasterizing glyphs and has no cache hits. Large-image blending still
costs **4.805 ms**. Input reads, output writes, and font padding are smaller here
and depend on the filesystem and allocator.

Further gains should target those workloads independently: compressor search
cost, ANSI dispatch, arc rasterization, and glyph-cache collisions/blending.
Changing compressors or geometric rasterization needs an explicit output
compatibility tradeoff; this round retains identical bytes and pixels. These
measurements do not establish an absolute optimum or cover every font, storage
device, and architecture. Vendored changes are documented in
[stb/CHANGES.md](../third_party/stb/CHANGES.md).
