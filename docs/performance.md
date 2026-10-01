# Performance measurements

Third optimization round, remeasured on 2026-10-01 against **main at `c44d83c`**
(the merge of PR #11). Both builds include the current CLI, complete box/block
geometry, wide and combining characters, and fallback-font support. The tests
now use native C/Rust harnesses. All comparisons below measure the additional
optimizations in this PR; earlier reports remain in Git history.

## Method and limits

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

## End-to-end results

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

## Repeated paired comparisons

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

## CPU and memory

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

## Stage timings

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

## Retained changes

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

## Rejected experiments and remaining work

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

## Validation and reproduction

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

## Where the historical “~20 ms” came from

[README at `fb714a5`](https://github.com/solcreek/termshot/blob/fb714a5/README.md)
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
