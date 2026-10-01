# Performance measurements

Measured on 2026-10-01 against **main at `255fa3a`**, after the previous
performance PR was merged. That reference already includes glyph caching,
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
| reply-sent | 14.03 / 20.49 | 9.07 / 12.55 | 1.55× | 205,774 |
| draft-ready | 14.14 / 15.92 | 8.93 / 10.71 | 1.58× | 200,974 |
| reply-24px | 6.78 / 7.45 | 5.34 / 5.98 | 1.27× | 79,300 |
| reply-128px | 64.87 / 70.63 | 31.76 / 34.38 | 2.04× | 954,758 |
| blank | 10.71 / 14.38 | 5.89 / 8.37 | 1.82× | 95,468 |
| color-grid | 22.43 / 26.04 | 17.67 / 18.95 | 1.27× | 649,294 |
| ascii-overflow | 12.65 / 15.02 | 5.59 / 7.25 | 2.26× | 24,837 |
| rounded-boxes | 23.17 / 25.05 | 15.17 / 17.84 | 1.53× | 141,124 |
| dense | 23.43 / 32.11 | 15.32 / 16.86 | 1.53× | 474,803 |
| ansi-replay | 24.20 / 26.73 | 17.29 / 18.38 | 1.40× | 205,774 |
| large | 98.08 / 158.37 | 53.33 / 137.94 | 1.84× | 1,501,397 |
| unicode | 11.16 / 12.43 | 9.14 / 12.60 | 1.22× | 154,584 |

The sample logs, blank screen, dense ASCII, and rounded boxes use 100×30 cells
at 48 px (2200×1440 pixels). `reply-24px` and `reply-128px` use the same sample
at 1100×720 and 5800×3840 pixels. `color-grid` assigns seeded random foreground
and background colors and printable ASCII to every cell at 24 px.
`ascii-overflow` is four million printable ASCII bytes on one logical line at
24 px, stressing cursor advancement and clipping. `rounded-boxes` fills the
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

These are measurements on a shared machine, not speed guarantees. In particular,
Unicode p95 increased slightly, large-image tail latency remains noisy, and an
ASCII run had a 134.53 ms outlier despite its lower median and p95. The original
and instrumentation-only CLI medians also differ (98.08 vs 104.09 ms for `large`),
showing the limits of precision. The old report against `8e1110e` remains in Git
history; its timings came from a different run and are not comparable here.

## Memory

Median **process peak RSS**, in MiB (1,048,576 bytes), from five independent runs:

| Workload | Main (MiB) | Optimized (MiB) | Reduction |
| --- | ---: | ---: | ---: |
| reply-sent | 22.67 | 13.52 | 40.4% |
| draft-ready | 22.64 | 13.52 | 40.3% |
| reply-24px | 8.97 | 6.59 | 26.5% |
| reply-128px | 132.92 | 69.06 | 48.0% |
| blank | 21.48 | 12.30 | 42.8% |
| color-grid | 9.67 | 7.31 | 24.4% |
| ascii-overflow | 12.41 | 10.05 | 19.0% |
| rounded-boxes | 21.72 | 12.56 | 42.2% |
| dense | 22.78 | 13.64 | 40.1% |
| ansi-replay | 27.16 | 18.02 | 33.7% |
| large | 124.34 | 66.22 | 46.7% |
| unicode | 9.45 | 7.11 | 24.8% |

The renderer now paints directly into PNG scanlines, including one zero filter
marker per row. This removes the second full image buffer and encoding copy.
The sample's RGB payload is still 9,504,000 bytes; the large case is still
60,825,600 bytes. Those payload counts (`pixel_bytes`) exclude scanline markers,
font storage, compressor storage, caches, and process overhead; they are not RSS.
The read-only CRC table adds 8 KiB. Rounded-corner offsets use at most four
arrays of 2,049 pairs of floats (about 64 KiB), allocated only when needed and
freed within the render. Larger arcs or allocation failure use the original path.

## Stage timings

Separate profiled-run medians for `reply-sent`; the baseline is `255fa3a` with
only the additional timers. Row medians need not sum to the median total.

| Stage | Instrumented main (ms) | Optimized (ms) |
| --- | ---: | ---: |
| `input_read_ms` | 0.0309 | 0.0224 |
| `parse_ms` | 0.0436 | 0.0374 |
| `font_load_ms` | 0.2872 | 0.2160 |
| `font_read_ms` | 0.0489 | 0.0485 |
| `font_check_ms` | 0.1767 | 0.0973 |
| `font_padding_ms` | 0.0653 | 0.0667 |
| `font_setup_ms` | 0.0060 | 0.0040 |
| `allocate_ms` | 0.0030 | 0.0030 |
| `background_ms` | 0.7610 | 0.7790 |
| `foreground_ms` | 0.6000 | 0.5900 |
| `geometry_ms` | 0.0575 | 0.0555 |
| `glyph_ms` | 0.2405 | 0.2430 |
| `blend_ms` | 0.2550 | 0.2405 |
| `png_filter_ms` | 0.9965 | 0.0000 |
| `png_deflate_ms` | 7.2555 | 3.9135 |
| `deflate_allocate_ms` | 0.0050 | 0.0045 |
| `deflate_match_emit_ms` | 4.3700 | 3.0440 |
| `deflate_finalize_ms` | 0.0010 | 0.0010 |
| `deflate_checksum_ms` | 2.8395 | 0.8390 |
| `png_pack_ms` | 0.5165 | 0.1015 |
| `png_encode_ms` | 8.7670 | 4.0150 |
| `output_write_ms` | 0.1750 | 0.1615 |
| `cleanup_ms` | 0.0010 | 0.0010 |
| `total_ms` | 10.7824 | 5.8513 |

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

- Batch printable ASCII runs, clipping once and preserving cursor saturation,
  attributes, Unicode, and control-sequence boundaries. Reuse the CSI parameter
  array across sequences. Parsing falls from **7.199 to 1.202 ms** for the long
  ASCII line and **9.763 to 7.849 ms** for the ANSI replay.
- Validate simple glyph coordinate lengths directly from repeated flag runs,
  avoiding an expanded flag allocation and two scans. Sample validation falls
  from **0.177 to 0.097 ms**; all checks and font padding are retained.
- Cache only translation-independent arc offsets per render, applying the
  original floating-point translation at each cell. Rounded-grid geometry falls
  from **9.135 to 6.160 ms**, with identical pixels.
- Render directly into zero-filter PNG scanlines, eliminating a full image copy
  and allocation. The generic stb writer retains filtering, channels, strides,
  and flipping support through a borrowed-scanline packaging helper.
- Calculate DEFLATE length/distance indexes directly and emit each token in one
  operation using a 64-bit bit buffer. Match selection remains unchanged.
  Sample match/emission time falls from **4.370 to 3.044 ms**.
- Express Adler-32 as independent weighted reductions that the compiler can
  vectorize. Sample checksum time falls from **2.840 to 0.839 ms**. Portable
  slicing-by-eight IEEE CRC-32 through stb's existing hook reduces sample PNG
  packaging from **0.517 to 0.102 ms**. `scripts/generate-crc32.py` reproduces the
  checked-in table; unaligned inputs use explicit byte assembly.

## Reproduce

Choose a destination that does not exist. The helper reads historical source
without switching the checkout, builds the reference, applies
[round-two-baseline.patch](round-two-baseline.patch), and builds an instrumented
baseline. The repository must contain commit `255fa3a`.

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

Local macOS validation passed 48 parser/font/drawing unit tests, 3,000
byte-for-byte compressor comparisons, CLI checks, and portable pixel goldens.
The extended suite passed **5,024 independent zlib/PNG round trips per compressor**,
including all PNG filters, 1–4 channels, padded strides, flipping, Adler-32 block
boundaries, random bytes, repetitive inputs, and DEFLATE window boundaries.
It compares the new CRC against stb on aligned and unaligned buffers. Both
compressors and the C renderer ran under ASan/UBSan. Sixteen extended pixel
fixtures and eight concurrent profiled renders passed. The original extended
goldens were recorded from `8e1110e`; all remain valid.

Additional local differential checks against `255fa3a` matched every cell field
for 10,000 generated parser streams and matched acceptance/rejection for 5,000
mutated fonts. All 12 benchmark outputs match the full original PNG SHA-256.
The baseline rebuild helper was also exercised. Linux and Rust 1.70 execution
have not been repeated locally this round; existing CI covers those environments.
Shared-runner CI has no timing thresholds.

The remaining bottleneck depends on the input. PNG encoding is **4.015 of
5.851 ms** of profiled sample time. Random-color compression still spends
**12.385 ms** finding/emitting matches. ANSI replay spends **7.849 of 15.525 ms**
parsing. Rounded boxes spend **6.160 ms** in geometry; the Unicode case spends
**2.185 ms** rasterizing glyphs and has no cache hits. Large-image blending still
costs **8.623 ms**. Input reads, output writes, and font padding are smaller here
and depend on the filesystem and allocator.

Further gains should target those workloads independently: compressor search
cost, ANSI dispatch, arc rasterization, and glyph-cache collisions/blending.
Changing compressors or geometric rasterization needs an explicit output
compatibility tradeoff; this round retains identical bytes and pixels. These
measurements do not establish an absolute optimum or cover every font, storage
device, and architecture. Vendored changes are documented in
[stb/CHANGES.md](../third_party/stb/CHANGES.md).
