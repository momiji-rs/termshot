# Performance measurements

Third optimization round, measured on 2026-10-01 against **main at `bd726a6`**
(including the built-in font, CLI, and expanded color/text attributes after PR #10).
That baseline already includes optimized parsing, checked
fonts, glyph caching, RGB scanlines painted directly into the compressor input,
faster DEFLATE, Adler-32, and CRC-32. All comparisons below measure additional
changes; earlier reports remain in Git history.

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
| reply-sent | 9.49 / 10.40 | 9.03 / 10.14 | 205,774 |
| draft-ready | 9.86 / 11.13 | 9.57 / 10.65 | 200,974 |
| reply-24px | 5.70 / 6.29 | 5.55 / 6.16 | 79,300 |
| reply-128px | 32.67 / 37.91 | 30.92 / 34.04 | 954,758 |
| real-shell | 6.41 / 7.03 | 6.11 / 6.82 | 110,521 |
| real-less | 5.71 / 7.28 | 5.62 / 6.97 | 85,779 |
| real-vi | 8.02 / 9.23 | 7.66 / 10.08 | 187,698 |
| blank | 5.92 / 6.69 | 5.61 / 6.37 | 95,468 |
| color-grid | 18.47 / 20.98 | 17.53 / 18.42 | 649,294 |
| ascii-overflow | 21.68 / 27.01 | 9.68 / 15.01 | 34,560 |
| rounded-boxes | 17.12 / 19.52 | 16.89 / 17.96 | 141,124 |
| dense | 13.29 / 14.29 | 12.32 / 13.62 | 361,204 |
| ansi-replay | 20.91 / 24.29 | 19.86 / 23.77 | 205,774 |
| large | 46.52 / 57.40 | 43.21 / 53.07 | 1,362,101 |
| unicode | 9.10 / 9.97 | 7.40 / 8.18 | 154,584 |

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
| reply-sent | 1.054× [1.033, 1.070] | 1.032× [1.024, 1.051] |
| draft-ready | 1.050× [1.024, 1.071] | 1.036× [1.029, 1.048] |
| reply-24px | 1.029× [1.003, 1.050] | 1.018× [0.975, 1.052] |
| reply-128px | 1.073× [1.054, 1.090] | 1.049× [1.021, 1.080] |
| real-shell | 1.056× [1.033, 1.066] | 1.058× [1.033, 1.084] |
| real-less | 1.014× [0.990, 1.051] | 1.043× [0.989, 1.083] |
| real-vi | 1.045× [1.016, 1.074] | 1.008× [0.959, 1.065] |
| blank | 1.039× [1.005, 1.073] | 1.014× [0.932, 1.099] |
| color-grid | 1.059× [1.049, 1.072] | 1.085× [1.050, 1.100] |
| ascii-overflow | 2.337× [2.157, 2.484] | 2.064× [1.923, 2.244] |
| rounded-boxes | 1.019× [1.005, 1.054] | 1.000× [0.906, 1.113] |
| dense | 1.076× [1.051, 1.101] | 1.051× [0.975, 1.128] |
| ansi-replay | 1.046× [1.027, 1.060] | 1.057× [1.014, 1.096] |
| large | 1.069× [1.054, 1.096] | 1.098× [1.084, 1.112] |
| unicode | 1.246× [1.190, 1.258] | 1.216× [1.185, 1.257] |

The long ASCII and Unicode workloads show the largest gains. Small-case and
resolution-specific comparisons are more variable; some
intervals cross 1.0. Tail latency does not improve uniformly. Claims are based on
the repeated comparisons and stage evidence, not whichever run has the lowest
absolute time.

For example, batch B's blank-screen median rises from 10.09 to 10.78 ms;
its paired interval spans 1.0. Real-shell's median falls from 6.59 to 6.21 ms,
but p95 rises from 9.18 to 14.82 ms. Both outcomes remain in the report.

## CPU and memory

Batch A medians. CPU is milliseconds per process; RSS is process peak MiB
(1,048,576 bytes), measured independently with `/usr/bin/time -l`.

| Workload | Child CPU: main → optimized (ms) | Peak RSS: main → optimized (MiB) |
| --- | ---: | ---: |
| reply-sent | 8.13 → 7.67 | 13.58 → 13.56 |
| draft-ready | 8.33 → 7.96 | 13.55 → 13.58 |
| reply-24px | 4.53 → 4.34 | 6.69 → 6.69 |
| reply-128px | 30.31 → 28.18 | 69.12 → 69.11 |
| real-shell | 5.27 → 5.03 | 10.11 → 10.16 |
| real-less | 4.60 → 4.49 | 10.05 → 10.06 |
| real-vi | 6.80 → 6.42 | 10.27 → 10.28 |
| blank | 4.70 → 4.48 | 12.34 → 12.36 |
| color-grid | 17.16 → 16.16 | 7.97 → 7.89 |
| ascii-overflow | 19.27 → 7.20 | 10.14 → 7.88 |
| rounded-boxes | 15.56 → 15.19 | 12.58 → 12.59 |
| dense | 11.77 → 10.81 | 13.58 → 13.59 |
| ansi-replay | 19.02 → 18.14 | 18.05 → 18.08 |
| large | 43.34 → 39.56 | 65.98 → 66.03 |
| unicode | 8.00 → 6.23 | 7.14 → 7.03 |

Input bytes are now freed after parsing, before raster/compressor allocation.
For the long ASCII workload this reduces peak RSS from 10.14 to 7.88 MiB.
The ANSI replay's peak RSS barely changes: freeing a live buffer does not promise
an equal reduction in maximum resident memory, which also reflects earlier
allocations and allocator retention. `parse_ms` includes the input-buffer release.

Glyph cache metadata increases from 8 to 32 KiB on 64-bit builds; up to 1,024
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
| `input_read_ms` | 0.0336 | 0.0250 |
| `parse_ms` | 0.0569 | 0.0534 |
| `font_load_ms` | 0.2353 | 0.2337 |
| `font_read_ms` | 0.0560 | 0.0546 |
| `font_check_ms` | 0.1028 | 0.1022 |
| `font_padding_ms` | 0.0719 | 0.0722 |
| `font_setup_ms` | 0.0010 | 0.0010 |
| `allocate_ms` | 0.0030 | 0.0030 |
| `background_ms` | 0.8610 | 0.8760 |
| `foreground_ms` | 0.6540 | 0.6580 |
| `geometry_ms` | 0.0605 | 0.0650 |
| `glyph_ms` | 0.2590 | 0.2545 |
| `blend_ms` | 0.2740 | 0.2800 |
| `png_filter_ms` | 0.0000 | 0.0000 |
| `png_deflate_ms` | 4.1250 | 3.7625 |
| `deflate_allocate_ms` | 0.0050 | 0.0050 |
| `deflate_match_emit_ms` | 3.2065 | 2.8475 |
| `deflate_finalize_ms` | 0.0010 | 0.0010 |
| `deflate_checksum_ms` | 0.8810 | 0.8770 |
| `png_pack_ms` | 0.1080 | 0.1080 |
| `png_encode_ms` | 4.2335 | 3.8720 |
| `output_write_ms` | 0.1800 | 0.1880 |
| `cleanup_ms` | 0.0010 | 0.0010 |
| `total_ms` | 6.3291 | 5.9941 |

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
  Long-ASCII parsing: **11.943 → 1.343 ms**.
- Accumulate CSI digits in a wide integer and clamp once, preserving the same
  saturating u32 value with fewer overflow checks. Combined parser changes take
  ANSI replay parsing from **9.244 → 8.549 ms**.
- Increase the bounded glyph cache to 1,024 slots. The Unicode workload goes
  from 1,119 rasterizations / zero cache hits to 307 rasterizations / 2,200 hits;
  glyph time is **2.235 → 0.791 ms**. Cache-collision fixtures
  still exercise eviction at the new size, including missing glyphs.
- Flush DEFLATE bits in a bounded four-byte store, with one capacity check per
  token instead of per emitted byte. Compare matches in bounded 16-byte groups
  plus tails, preserving the first differing byte, match ordering, and stream
  bytes. Sample match/emission time is **3.207 → 2.848 ms**;
  the large case is **20.538 → 16.751 ms**. Allocation
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

Remaining batch-A costs include **3.872 ms** of sample PNG encoding,
**8.549 ms** of ANSI parsing, and
**6.711 ms** of rounded-grid geometry.
The large image still spends **5.482 ms** painting backgrounds and
**5.662 ms** blending. Optimizing small I/O and font stages further was
not justified by these profiles. More glyphs than the cache can hold, different
fonts, cold storage, and other architectures remain outside these measurements.
No absolute optimum or universal latency is claimed.

## Validation and reproduction

Local macOS validation passed:

- `SANITIZE=1 ./test.sh`: 76 unit tests, 3,000 byte-identical compressor
  comparisons, CLI checks, and portable pixel goldens.
- `SANITIZE=1 UBSAN_OPTIONS=halt_on_error=1 ./tests/run.sh`: 5,024 independent
  PNG/zlib round trips per compressor, 11 injected allocation failures,
  aligned/unaligned CRC checks, 18 pixel fixtures, and 8 concurrent profiled renders.
- A new parser test compares batch writes with individual prints across screen
  sizes, scrolling margins, wrap states, alternate screens, and run lengths.
  Another 10,000 generated streams match every output cell against `bd726a6`;
  5,000 font mutations retain the same acceptance/rejection result.
- New collision and distant-geometry fixtures were generated using `05f3319`
  and checked against `bd726a6`; all existing pixel hashes remain unchanged.
  Both benchmark batches match all 15 complete PNG hashes. The baseline build
  helper was exercised.

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
`bd726a6`, which already has all stage timers and embeds the bundled font.
Older instrumented baselines can still be built using explicit `--revision`
and `--profile-patch` options.

`TERMSHOT_PROFILE` is enabled by presence, including `0`, and emits two
`termshot-profile ` JSON records to stderr. Timers are thread-local and rendering
buffers/caches are per-call. Python is needed for benchmarks and extended tests,
not the build or `./test.sh`.

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
