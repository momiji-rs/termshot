# Contributing

termshot is plain `rustc` plus a C compiler: no Cargo, no crates.io dependencies, and MSRV
rustc 1.70 (CI checks it), so don't use newer std APIs or syntax. [CLAUDE.md](CLAUDE.md) maps
the architecture, module by module, and the constraints that aren't obvious (cross-platform
determinism, the link order, the exit codes).

## Build

```sh
./build.sh
```

This builds the C objects into `libtermshot_c.a`, the library `libtermshot.rlib` (with the C
inside), and `./termshot`. The binary links libc and libm.

## Test

```sh
./test.sh
```

This builds termshot, runs the parser unit tests, checks box drawing (`tests/boxes.c`) and
glyph placement (`tests/glyphs.c`: wide characters, missing glyphs), checks the CLI exit codes,
and compares the rendered samples against `tests/goldens.txt`. The goldens hash decoded pixels
(stb_image decodes them, `tests/golden.rs` hashes them), so a change to how the PNG is encoded
doesn't break them; only a change to the pixels does. They cover the two samples at px 46 and
48 and the edge cases in `tests/fixtures/` (clipping, missing glyphs, escapes, random colors,
box drawing from px 1 to 255), and CI runs them on Linux and macOS. px 46 is there because it
is a size where a compiler that fuses multiply-adds would render different pixels.

When a change is meant to move pixels, look at the renders in `target/test/`, then run
`./test.sh --update-goldens`. `SANITIZE=1 ./test.sh` builds the C (the stb glue and the PNG
decoder) with ASan and UBSan; this works on macOS only.

A test for a known bug describes the correct behaviour and is marked `#[ignore = "#N: ..."]`
with its issue. None are open now. Run them with `./target/test/unit --ignored`. Three other
tests are ignored. `poc_workloads` writes inputs for `bench/c-vs-rust/` and checks nothing.
`any_cff2_font_matches_harfbuzz` checks a CFF2 font of your own, at any instance, against the
outlines `tools/cff2-outlines.sh` recorded from HarfBuzz, and
`any_cff2_font_s_metrics_match_harfbuzz` its advances and extents against what
`tools/cff2-metrics.py` recorded; their doc comments give the commands.

## Measure

```sh
TERMSHOT_PROFILE=1 ./termshot examples/reply-sent.pty /tmp/reply.png \
  third_party/jetbrains-mono/JetBrainsMono-Regular.ttf
python3 scripts/bench.py --binary current=./termshot --runs 40 \
  --output /tmp/termshot-bench.json
SANITIZE=1 ./tests/run.sh
```

Profiling writes two `termshot-profile` JSON records to stderr, covering input, parsing, font
allocation/reading/validation/padding (for the built-in or given font and the fallback alike),
glyph cache and fallback counters, drawing, PNG filtering, compression
allocation/matching/emission/checksum, PNG packaging, and writing. The benchmark interleaves
ordinary and profiled CLI runs in the same rounds, plus optional peak RSS and (on Linux)
cold-cache runs; it records raw samples, means, medians, p95, child CPU time, paired
comparisons, profiling overhead, output size and hash, and host, toolchain, source and font
details. Each font-path case checks the profile counters that show it took its path
(`scripts/bench-report.py` summarizes a result file). Python 3 is needed only for optional
development scripts (benchmarks and CRC table generation); the tests need only a C compiler and
rustc.

See [performance measurements](docs/performance.md) for the before/after results, baseline
reproduction, timing boundaries, and remaining bottlenecks. Extended pixel tests retain main's
reference images and portable floating-point settings.


## Changes

User-visible changes go in [CHANGELOG.md](CHANGELOG.md) under `[Unreleased]`, in [Keep a
Changelog](https://keepachangelog.com/) format. Local changes to vendored stb are listed in
[third_party/stb/CHANGES.md](third_party/stb/CHANGES.md); record any new ones there.
