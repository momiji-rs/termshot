# C vs Rust for termshot's own C code

This file holds dated comparisons, newest first, for
[#12](https://github.com/momiji-rs/termshot/issues/12): should termshot's own C
(`src/draw.c` painting and `src/deflate.c` compression) move to Rust? It is
moving: compression and box drawing are Rust now. The vendored stb
libraries are not part of it. Each comparison ports the C as of one commit to Rust,
checks that both write the same bytes, and times both in one process.

- [Step 2a shipped: box drawing is Rust (2026-10-04)](#step-2a-shipped-box-drawing-is-rust-2026-10-04).
- [Step 1 shipped: deflate is Rust (2026-10-04)](#step-1-shipped-deflate-is-rust-2026-10-04).
- [Deflate, current code (2026-10-03, `a8a95e0`)](#deflate-current-code-2026-10-03-a8a95e0):
  `deflate.c` after #20 (16-lane Adler-32, bit reversal by table, inlined matcher).
- [Painting and deflate POC (2026-10-01, `bd726a6`), history](#painting-and-deflate-poc-2026-10-01-bd726a6-history):
  the first port of both files. Its deflate C is two optimization rounds old and its
  painting C predates #61, #66 and #69, so do not compare its numbers with the current code.

## Step 2a shipped: box drawing is Rust (2026-10-04)

Painting moves bottom-up in four steps, each a layer the remaining C calls
through FFI: 2a box drawing and blocks, 2b image compositing, 2c glyphs
(cache, blending, italic, marks), 2d the driver loop and the PNG write call,
leaving thin stb glue in C. Step 2a shipped: the canvas primitives (`put`,
`clip_rect`, `fill_rect`, `shade_rect`, the bars), box drawing (lines, heavy
and double, dashes, arcs and diagonals), block elements and shades, and the
`Stamps` stroke cache are `src/geometry.rs`.

- **FFI surface**: `termshot_paint_geometry(canvas, col, row, cell_w, cell_h,
  cp, bold, r, g, b)` per cell (1 painted, 0 not geometry, -1 a caught
  panic, which fails the render with exit 2); `termshot_fill_rect` for the
  C's own rectangles (the first scanline of each cell's background, underlines,
  missing-glyph boxes); `termshot_geometry_new`/`_free`/`_stats` for the
  render's state. Never per pixel.
- **Canvas ABI**: `#[repr(C)]` `{px, filtered, w, h, stride, geometry}`, 40
  bytes, asserted on both sides. The clip is Rust's alone, and the arc
  offsets and `Stamps` live behind the opaque `geometry` pointer, which may be
  NULL (nothing cached, same pixels).
- **Floats**: ported operation for operation in f32; no `mul_add`. floor,
  ceil and sqrt are exact, so std's are libm's. The arcs' sine and cosine are
  the one rounded function, and the C's compilers merged `sinf` and `cosf` of
  one angle into one sincos call. On macOS that merged form differs from
  `cosf` in the last bit for 2.5 million floats in the arcs' range
  (`bench/c-vs-rust/sincos.c`), and LLVM merged Rust's `f32::sin`/`cos` at
  some call sites only, so `sin_cos` calls `__sincosf_stret` (macOS) or
  `sincosf` (Linux) by name. Float-to-int casts are pixel coordinates and
  step counts bounded by the 2^27-pixel canvas, where C's `(int)` truncation
  and Rust's saturating `as` agree; C's out-of-range UB is never reached.
- **Memory**: the caches grow with `try_reserve_exact` under the same 4 MiB
  budget and the same TwoSum exactness check; a failed allocation stamps the
  stroke afresh, as in C. No geometry allocation failed a render before, and
  none does now.
- **Evidence**: `bench/c-vs-rust/run.sh geometry` (CI, all three hosts)
  compares 1,442,400 painted cells with draw.c as of `24d71fe`, byte for
  byte, then times both: Rust ÷ C 0.97-1.01 under Apple clang 21 and
  0.86-1.04 under GCC 16. End to end no geometry case is slower beyond noise
  on either host
  ([docs/performance.md](performance.md#box-drawing-in-rust-2026-10-04-629a4d4-12-step-2a)).

For 2b: the image layer (`paint_image_rows`, `backdrop_through`) still calls
`termshot_fill_rect` once per cell per backdrop row; moving it moves that
call inside Rust. `Canvas` is already shared, so 2b can take the same
pointer. Bounds checks cost up to 25% in short fills here, so the hot loops
check a span once and then write through a pointer; expect the same in
`blend` and the image sampler.

## Step 1 shipped: deflate is Rust (2026-10-04)

The owner decided #12: move `deflate.c`, then painting, and keep stb in C. Step 1
shipped: `src/deflate.c` is gone, and `src/deflate.rs`, started from this
comparison's `deflate.rs`, is the compressor that stb_image_write calls through
`STBIW_ZLIB_COMPRESS`. Two of the gaps found below were closed, and both open
items under "Against" were done:

- **x86-64 Adler-32:** an SSE2 form (`psadbw`, `pmaddwd`, one 16-byte load per
  chunk; SSE2 is baseline, so no detection) runs at 0.77 of GCC 16's C and
  0.70-0.72 of clang 22's, where the safe form was 1.14-1.21. arm64 keeps the
  safe form.
- **Matching:** scanning a bucket in two phases (before and after a first
  match) took the LLVM-side gap from 6-13% to 1-3%; against GCC 16 the
  whole compressor is now 0.87-0.96 on x86-64.
- **Allocation failure** returns NULL, as the C did, from libc
  `malloc`/`realloc` buffers that stb frees; the CLI exits 2. The
  `TERMSHOT_PROFILE` deflate timers are kept, with the same keys.
- `tests/deflate_diff.c` tests the Rust against stock stb, linked as a static
  library.

`bench/c-vs-rust/run.sh deflate` keeps the comparison running against
`deflate.c` as of `a8a95e0`, with the shipped compressor as a sixth variant,
so rustc upgrades stay gated. Numbers, and the end-to-end comparison with main:
[docs/performance.md](performance.md#png-compression-in-rust-2026-10-04-6396b8d-12-step-1).

## Deflate, current code (2026-10-03, `a8a95e0`)

Measured 2026-10-03 against `src/deflate.c` at `a8a95e0` (main after #20, #69).
Painting (`draw.c`) is **not** in scope for this round.

**Short answer:** a safe Rust port of the current `deflate.c` writes the same bytes and,
in the whole compressor, runs 1-9% slower than Apple clang's C on macOS arm64. Against GCC on Linux
x86-64 it runs from 2% faster to 3% slower on the image inputs, 11% slower on the one input that is
mostly checksum, and 6% slower on random bytes. The new 16-lane Adler-32 matches the C on arm64
in safe stable Rust, but only in some loop shapes: written in the C's own loop shape (one chunk
per step), it is 3.3× slower there at `opt-level=2`. On x86-64 the Rust Adler-32 is 14-21% slower
than the C. Two `unsafe` reads buy at most 4 points and close no gap.

### What was compared

`bench/c-vs-rust/run.sh deflate [rounds]` runs it; `CC=gcc` or `CC=clang` picks the C
compiler.

- **C:** `src/deflate.c` and `src/deflate_profile.h` at `a8a95e0`, taken with `git archive`
  as the 2026-10-01 POC takes `bd726a6`. `deflate_shim.c` includes it three times under three prefixes, all with
  `build.sh`'s `-O2`:
  - **C**: the default build. Under clang, this is the generic-vector Adler-32. Under GCC, it is
    the plain loop, which GCC vectorizes.
  - **C portable**: `-DTERMSHOT_PORTABLE_ADLER`, the plain loop on every compiler. Under GCC
    it is the same code as **C**.
  - **C zeroed table**: the default build with its 1 MiB hash table from `calloc` instead of
    `malloc`, because the safe Rust's `vec![0; n]` zeroes it too. This measures that cost
    rather than assuming it.
- **Rust:** `deflate.rs`, a port in safe Rust with the same algorithms. It has the 16-lane Adler-32 with
  weights applied once per 5552-byte block, the 256-byte reversed-byte table, `countm`
  comparing 16 bytes, then 8, then single bytes, the newest-first bucket scan with the same
  early exits, lazy matching, and the carried hash. The output buffer appends four bytes per token and keeps
  the complete ones, as the C does. It does not port the `TERMSHOT_PROFILE` timers, which are off in this
  bench, so the C pays only five thread-local `enabled` checks per call (four `profile_now()` calls and the final
  `if`). It is built with `build.sh`'s `-C opt-level=2`.
  - **Rust unchecked**: the 2026-10-01 POC's `--cfg unchecked` with the same two
    `unsafe` reads, the candidate-rejection byte and `countm`'s loads. Here it is a const
    generic, so both Rust variants run in the same process and rounds as the C builds.
- **Correctness gate:** every variant must write the C's bytes, and every Adler-32 must
  equal the C's, or the run aborts. The gate covers these inputs:
  - every timed input;
  - a port of `tests/deflate_diff.c`'s generator: its 3,000 cases (lengths 0-11, then mostly
    under 20,000, every 50th 70,000-190,000; noise, runs, copies and image-like rows;
    qualities 5-16) and its 60 all-0xff Adler-32 worst cases at four offsets;
  - 200 seeded random buffers of 2-256 symbols, up to 150,000 bytes, at offsets 0-15.

  That is 3,260 cases. All passed on every host and compiler below.
  `tests/deflate_diff.c` still checks the C against stock stb in `./test.sh`.
- **Inputs:** the deflate input of each image, which is the inflated IDAT of the current
  termshot's PNG. That is the filtered scanlines, rendered from the same logs by the current
  CLI with its defaults, cursor included, and inflated by the bench binary itself (`deflate.rs --inputs`;
  no Python). So `1-reply-px48` is exactly what
  a plain `reply-sent` run compresses. The three `bench.py` logs are rebuilt in `deflate.rs`,
  `color-grid` with a port of Python's `random.Random(13)`. The measurements below were taken
  with an earlier Python generator; the Rust one writes the same eight inputs, sha256 for
  sha256. There are also two seeded synthetic buffers. Quality 8, as termshot uses.

| input | bytes | source |
|---|---:|---|
| 1-reply-px48 | 9,505,440 | `tests::poc_workloads`, `reply-sent.pty` at 48 px (the README sample, `reply-sent`) |
| 2-reply-px128 | 66,819,840 | the same at 128 px (`reply-128px`, the #20 Adler-32 stress case) |
| 3-attrs-px24 | 456,336 | `poc_workloads`: 256 colours, every attribute |
| 4-boxes-px48 | 9,505,440 | `poc_workloads`: rounded boxes |
| 5-dense-200x60-px16 | 3,780,900 | `poc_workloads`: a different colour on every cell |
| 6-blank | 9,505,440 | `scripts/bench.py`'s `blank`, nearly all Adler-32 (from the #20 round) |
| 6-color-grid | 2,376,720 | `bench.py`'s `color-grid`: short matches |
| 6-large | 60,829,440 | `bench.py`'s `large`: 240×80 at 48 px |
| 7-random-uniform | 2,376,720 | seeded uniform random bytes: nearly every position a literal |
| 8-random-4sym-skewed | 2,376,720 | seeded, four symbols at 9/16, 4/16, 2/16, 1/16 |

- **Timing:** all five variants run in each round, in a seeded random order per round, so each
  variant follows every other about equally often. Each call is timed alone; copying and freeing
  the output are not timed. The tables give the median / p95 of 101 rounds per call, in ms. R/C
  is the Rust median ÷ the **C** median; below 1 means Rust is faster. The Adler-32 rows time the
  checksum alone on the two reply inputs, shuffled the same way.

| | macOS arm64 | Linux x86-64 |
|---|---|---|
| host | `lawrences-mac-studio`, Apple M2 Max, macOS 26.6.2 | `starship`, Ryzen 7 8745HS, kernel 7.2.5-3-omarchy, glibc 2.44 |
| C compilers | Apple clang 21.0.0 (clang-2100.3.34.2) | GCC 16.2.1 20260810, clang 22.1.8 |
| Rust | rustc 1.98.1 (Homebrew) | rustc 1.98.1 (Arch) |
| load average (1 min), start → end of each run | 3.8 → 3.6 and 3.6 → 3.3: a shared desktop, other sessions busy | GCC 0.6 → 1.2 and 2.5 → 1.4; clang 1.5 → 1.3 and 2.2 → 1.8 |

### Results

Each host and compiler ran twice. The tables show the first run; the line under each gives the
second run's ranges.

**macOS arm64, Apple clang 21 vs rustc 1.98.1** (both LLVM):

| input | C | C portable | C zeroed table | Rust safe | Rust unchecked | R/C safe | R/C unchecked |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1-reply-px48 | 3.082 / 3.241 | 4.129 / 4.336 | 3.103 / 3.267 | 3.260 / 3.430 | 3.200 / 3.383 | 1.06 | 1.04 |
| 2-reply-px128 | 13.329 / 13.648 | 20.820 / 21.265 | 13.329 / 13.708 | 13.754 / 14.140 | 13.658 / 14.052 | 1.03 | 1.02 |
| 3-attrs-px24 | 0.216 / 0.238 | 0.268 / 0.294 | 0.222 / 0.251 | 0.230 / 0.260 | 0.229 / 0.251 | 1.07 | 1.06 |
| 4-boxes-px48 | 1.726 / 1.876 | 2.790 / 2.935 | 1.744 / 1.865 | 1.764 / 1.885 | 1.768 / 1.894 | 1.02 | 1.02 |
| 5-dense-200x60-px16 | 24.570 / 24.973 | 25.150 / 25.630 | 24.695 / 25.272 | 26.626 / 27.211 | 26.171 / 26.715 | 1.08 | 1.07 |
| 6-blank | 1.167 / 1.271 | 2.235 / 2.369 | 1.174 / 1.267 | 1.190 / 1.288 | 1.195 / 1.272 | 1.02 | 1.02 |
| 6-color-grid | 11.541 / 11.971 | 11.844 / 12.214 | 11.600 / 12.065 | 12.370 / 12.752 | 12.285 / 12.724 | 1.07 | 1.06 |
| 6-large | 19.642 / 20.052 | 26.588 / 27.053 | 19.777 / 20.367 | 20.458 / 20.986 | 20.264 / 20.623 | 1.04 | 1.03 |
| 7-random-uniform | 59.484 / 60.175 | 59.750 / 60.597 | 59.527 / 60.412 | 63.334 / 63.998 | 63.110 / 64.050 | 1.06 | 1.06 |
| 8-random-4sym-skewed | 53.051 / 53.527 | 53.489 / 54.069 | 53.343 / 53.857 | 53.937 / 54.440 | 53.399 / 53.979 | 1.02 | 1.01 |
| Adler-32 alone, reply-px48 | 0.481 / 0.526 | 1.553 / 1.656 | — | 0.487 / 0.555 | — | 1.01 | — |
| Adler-32 alone, reply-px128 | 3.449 / 3.607 | 10.957 / 11.262 | — | 3.496 / 3.634 | — | 1.01 | — |

Second run: safe 1.01-1.09, unchecked 1.01-1.07, zeroed table / C 1.00-1.04, Adler-32 1.01-1.02.

**Linux x86-64, GCC 16.2.1 vs rustc 1.98.1** (GCC vs LLVM; Linux releases are built with
`musl-gcc`):

| input | C | C portable | C zeroed table | Rust safe | Rust unchecked | R/C safe | R/C unchecked |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1-reply-px48 | 3.117 / 3.145 | 3.123 / 3.165 | 3.122 / 3.151 | 3.072 / 3.122 | 3.019 / 3.053 | 0.99 | 0.97 |
| 2-reply-px128 | 13.462 / 13.561 | 13.479 / 13.582 | 13.415 / 13.478 | 13.754 / 13.854 | 13.623 / 13.718 | 1.02 | 1.01 |
| 3-attrs-px24 | 0.223 / 0.232 | 0.224 / 0.230 | 0.231 / 0.240 | 0.224 / 0.233 | 0.219 / 0.226 | 1.01 | 0.98 |
| 4-boxes-px48 | 1.649 / 1.679 | 1.664 / 1.699 | 1.663 / 1.683 | 1.691 / 1.717 | 1.699 / 1.728 | 1.03 | 1.03 |
| 5-dense-200x60-px16 | 23.421 / 23.940 | 23.476 / 23.743 | 23.440 / 23.633 | 23.327 / 23.643 | 22.480 / 22.788 | 1.00 | 0.96 |
| 6-blank | 1.123 / 1.143 | 1.124 / 1.151 | 1.134 / 1.151 | 1.248 / 1.276 | 1.258 / 1.278 | 1.11 | 1.12 |
| 6-color-grid | 11.007 / 11.109 | 11.015 / 11.110 | 11.043 / 11.132 | 11.014 / 11.124 | 10.679 / 10.768 | 1.00 | 0.97 |
| 6-large | 19.394 / 19.530 | 19.417 / 19.699 | 19.435 / 19.573 | 19.110 / 19.415 | 18.730 / 18.874 | 0.99 | 0.97 |
| 7-random-uniform | 52.849 / 53.507 | 52.838 / 53.285 | 52.865 / 53.626 | 56.158 / 56.779 | 54.243 / 54.913 | 1.06 | 1.03 |
| 8-random-4sym-skewed | 47.303 / 48.073 | 47.756 / 48.287 | 47.825 / 48.225 | 47.341 / 47.672 | 46.075 / 46.540 | 1.00 | 0.97 |
| Adler-32 alone, reply-px48 | 0.499 / 0.508 | 0.499 / 0.509 | — | 0.603 / 0.620 | — | 1.21 | — |
| Adler-32 alone, reply-px128 | 3.548 / 3.571 | 3.548 / 3.578 | — | 4.289 / 4.321 | — | 1.21 | — |

Second run: safe 0.98-1.11, unchecked 0.96-1.12, zeroed table / C 1.00-1.04, Adler-32 1.21.

**Linux x86-64, same machine, clang 22.1.8 vs rustc 1.98.1** (both LLVM):

| input | C | C portable | C zeroed table | Rust safe | Rust unchecked | R/C safe | R/C unchecked |
|---|---:|---:|---:|---:|---:|---:|---:|
| 1-reply-px48 | 2.973 / 3.027 | 3.664 / 3.717 | 2.950 / 3.057 | 3.073 / 3.130 | 3.028 / 3.074 | 1.03 | 1.02 |
| 2-reply-px128 | 13.431 / 13.647 | 18.354 / 18.556 | 13.182 / 13.327 | 13.733 / 14.045 | 13.675 / 13.905 | 1.02 | 1.02 |
| 3-attrs-px24 | 0.209 / 0.216 | 0.244 / 0.255 | 0.216 / 0.224 | 0.225 / 0.238 | 0.221 / 0.233 | 1.07 | 1.06 |
| 4-boxes-px48 | 1.668 / 1.698 | 2.386 / 2.428 | 1.666 / 1.690 | 1.704 / 1.739 | 1.714 / 1.754 | 1.02 | 1.03 |
| 5-dense-200x60-px16 | 21.944 / 22.061 | 22.132 / 22.299 | 21.843 / 22.000 | 23.786 / 23.889 | 23.082 / 23.249 | 1.08 | 1.05 |
| 6-blank | 1.206 / 1.220 | 1.920 / 1.933 | 1.206 / 1.219 | 1.258 / 1.305 | 1.274 / 1.307 | 1.04 | 1.06 |
| 6-color-grid | 10.340 / 10.413 | 10.406 / 10.492 | 10.250 / 10.325 | 11.041 / 11.117 | 10.751 / 10.809 | 1.07 | 1.04 |
| 6-large | 18.714 / 18.778 | 23.102 / 23.165 | 18.510 / 18.632 | 19.266 / 19.339 | 18.983 / 19.050 | 1.03 | 1.01 |
| 7-random-uniform | 52.957 / 53.292 | 53.161 / 53.426 | 52.823 / 53.160 | 59.606 / 60.130 | 57.731 / 58.182 | 1.13 | 1.09 |
| 8-random-4sym-skewed | 46.638 / 47.106 | 46.419 / 46.721 | 46.116 / 46.453 | 47.682 / 47.941 | 46.186 / 46.588 | 1.02 | 0.99 |
| Adler-32 alone, reply-px48 | 0.531 / 0.541 | 1.255 / 1.273 | — | 0.605 / 0.615 | — | 1.14 | — |
| Adler-32 alone, reply-px128 | 3.776 / 3.803 | 8.859 / 8.928 | — | 4.298 / 4.317 | — | 1.14 | — |

Second run: safe 1.01-1.13, unchecked 0.99-1.09, zeroed table / C 0.98-1.05, Adler-32 1.14.

**Linux aarch64, ratios only.** One run on a shared GitHub `ubuntu-24.04-arm` runner, with GCC
13.3.0 and Ubuntu clang 18.1.3 against rustc 1.98.1, 61 rounds
([run](https://github.com/momiji-rs/termshot/actions/runs/37174264965/job/111353491924),
from a scratch branch since deleted). Shared-runner timings are not kept as numbers, and the
ratios have that caveat too.

| | R/C safe | R/C unchecked | Adler-32 R/C | C portable / C | zeroed table / C |
|---|---|---|---|---|---|
| GCC 13.3 | 0.99-1.12 | 1.02-1.10 | 1.07 | 0.98-1.01 | 0.98-1.05 |
| clang 18.1 | 0.98-1.10 | 0.97-1.15 | 0.97 | 1.01-1.99 | 0.98-1.04 |

### Codegen: does safe Rust vectorize the 16-lane Adler-32?

Yes, on both targets, but the loop shape decides it. Every form below keeps the C's 16
lanes and gives each lane the same adds in the same order (`p[k] += a[k]`, then `a[k] += x[k]`, chunk
by chunk). They differ only in how many 16-byte chunks one step of the outer loop takes.
`bench/c-vs-rust/adler_forms.rs` checks each against the scalar definition, then times
them on 66,819,840 random bytes. The table gives ms, median of 31 rounds in a shuffled order. The C rows are
the bench's Adler-32 alone on `2-reply-px128`, which is the same length, and whose speed does not depend on
the data.

| form | M2 Max, `opt-level=2` | M2 Max, `opt-level=3` | Ryzen, `opt-level=2` | Ryzen, `opt-level=3` |
|---|---:|---:|---:|---:|
| one chunk per step (as the C) | 11.50 | 11.52 | 4.11 | 14.00 |
| literal: index a shrinking slice | 24.26 | 16.52 | 19.73 | 20.56 |
| 2 chunks per step | 3.39 | 9.43 | 4.26 | 3.58 |
| 4 chunks per step | 3.45 | 3.97 | 4.20 | 4.20 |
| **8 chunks per step (`deflate.rs`)** | **3.46** | 3.36 | **4.28** | 4.31 |
| C, Apple clang, generic vectors | 3.45 | | | |
| C, GCC 16, plain loop | | | 3.55 | |
| C, clang 22, generic vectors | | | 3.78 | |

On the aarch64 runner, at `opt-level=2`, one chunk per step took 2.5-2.7× as long as two, four or
eight.

- **The C's own loop shape (one chunk per step) stays scalar on arm64 at `opt-level=2`**: 11.50
  ms against 3.46. The POC's literal Adler-32 also lost its vectorization, but to bounds checks;
  this form has none in its inner loop. On x86-64 it vectorizes at 2 but not at 3.
  `-C no-vectorize-loops` makes every form scalar on both hosts at `opt-level=2` (10.6-31.5 ms
  on the M2, 11.0-27.9 on the Ryzen). So when the work is vectorized at 2, the loop vectorizer does it.
  `-C no-vectorize-slp` changes nothing at 2. Taking two or more chunks per step keeps the
  lane loop a loop that the loop vectorizer takes. Four and eight are vectorized at both
  opt-levels on both targets. Eight is about as fast as four at 2 and faster on arm64 at 3, which is
  why `deflate.rs` uses it.
- **The 2026-10-01 finding that `opt-level=3` was worse than 2 is a property of the loop shape,**
  not of the level. Here 3 is better for one form, worse for another, and equal for a third.
- **arm64, from `objdump -d` of the bench binary:** Apple clang's generic-vector C does
  per 16 bytes one `ldr q`, two `ushll` (u8 to u16), four `uaddw` (into `a`) and four
  `add.4s` (`p += a`). The Rust does exactly that mix, eight times per iteration with `ldp q`
  pairs. The time is equal (Adler-32 R/C 1.01).
- **x86-64, from `objdump -d`:** GCC's plain C and clang's generic vectors both load 16 bytes
  with `movdqu` and widen with `punpck{l,h}bw` / `punpck{l,h}wd`; clang unrolls twice. In the Rust, the
  loop vectorizer splits the 16 lanes into groups of four and loads each group's 4 bytes with
  `movd`. Per 16 bytes that is four loads and eight unpacks where the C has one load and six. That is the
  1.14× (clang) and 1.21× (GCC) Adler-32 gap. No safe form tried here gets the 16-byte
  load at `opt-level=2`; at 3, two chunks per step reaches 3.58 ms.
- **Inspect the linked binary, not `--emit asm`.** In one case (`adler_forms.rs`, one chunk per step,
  `opt-level=3`, arm64) the `.s` from `--emit asm` showed the vector loop while the linked
  binary's function was scalar.
- No `unsafe`, `std::simd` or `core::arch` is needed for the arm64 result. A `core::arch` x86-64
  form was not tried.

### Findings

1. **Safe Rust matches the new Adler-32 on arm64** (R/C 1.01-1.02 on the M2, 0.97 against
   clang 18 on aarch64 Linux, 1.07 against GCC 13's plain loop there), in a loop shape LLVM
   vectorizes. It does not match it on x86-64: 1.14× clang 22 and 1.21× GCC 16. That is about
   0.07-0.10 ms on `reply-sent`'s 9.5 MB, and 0.5-0.7 ms at 128 px.
2. **Safe Rust is slower in matching and emission against LLVM's C,** and about even with
   GCC's on image inputs. The compressor minus its checksum is 5-7% slower on `reply-sent`
   (M2: +0.12-0.17 ms of 2.6). On the match-heavy inputs (`dense`, `color-grid`,
   random bytes) it is 6-9% slower on the M2 and 7-13% with clang on x86-64, and 6% on random
   bytes with GCC. Against GCC the image inputs are 0.98-1.03, apart from `blank`, which is
   mostly Adler-32 and is 1.11. The cause of the LLVM-side matching gap was not found. Three
   tries showed no effect beyond noise in 15-round runs on the M2: one bounds check instead of
   three in the hash (kept), a fixed-size count array, and writing each token into a pre-sized
   scratch buffer as the C does.
3. **Zeroing the hash table costs little.** The C with a `calloc`'d table runs at 0.98-1.05 of
   the C, and the largest effect is on the smallest input, `3-attrs-px24` (0.46 MB, 1.03-1.05).
   That is up to about 4 points of that input's 7-8% R/C on the LLVM hosts, and about 1 point
   or less on the others. The allocation alone, `vec![0u32; 262144]` with no pages touched, takes 0.006-0.009 ms.
4. **`unchecked` buys little:** at most 4 points (`5-dense` with GCC 1.00 → 0.96, `7-random-uniform`
   with clang 1.13 → 1.09), and 0-3 points on most inputs. On aarch64 it was no better: with GCC it was slower than
   safe on 8 of 10 inputs. The safe `countm` here walks `chunks_exact(16)` over two slices cut to
   `limit`, so its 16-byte compares carry no per-load check. The POC's safe `countm`, which closed
   a 5% gap when unchecked, sliced `a[i..i + 8]` at every step.
5. **The compiler still matters as much as the language,** but it matters differently from 2026-10-01. #20's
   per-compiler C closed the GCC gap that made Rust 5-26% faster then. On clang hosts, the plain loop that clang
   does not vectorize costs the C up to 1.92× in the whole compressor on the M2 and up to 1.99× on
   the aarch64 runner with clang 18 (C portable / C, most where the checksum is a large share),
   and 2.35-3.23× in the Adler-32 alone.

### End-to-end context

Per `docs/performance.md`, `reply-sent` takes about 8.8 ms on the M2 Max and 7.6 ms on the
Ryzen, of which PNG encoding (deflate, with filtering and packing) is about 3.3 ms and 4.2
ms. On its deflate input, the measured difference of the whole compressor is:

| | C | Rust safe | difference | share of a `reply-sent` run |
|---|---:|---:|---:|---:|
| macOS, Apple clang (release compiler) | 3.08-3.09 | 3.22-3.26 | +0.13-0.18 ms | +1.5-2.0% |
| Linux x86-64, GCC (release compiler: `musl-gcc`) | 3.12-3.13 | 3.07-3.08 | -0.04-0.05 ms | -0.6% |
| Linux x86-64, clang | 2.97 | 3.07 | +0.10-0.11 ms | +1.4% |

At 128 px (`reply-128px`, about 29 ms on macOS) the difference is +0.39-0.43 ms, or about 1.5%. The x86-64 slice of the
macOS universal binary was not measured.

### What this means for #12

The evidence, without a decision:

- **For moving `deflate.c`:**
  - A safe-Rust port of the current code is byte-identical on 3,260 differential cases and ten
    workloads, on three platforms and five C compilers.
  - It needs no `unsafe`. The `unchecked` variant shows bounds checks are not where the remaining time
    goes, and the zeroed table explains only a point or so.
  - Against the Linux release compiler (GCC) it is even on image inputs on x86-64 (0.98-1.03;
    0.99-1.12 against GCC 13 on aarch64).
  - On macOS it costs about 1.5-2% of a typical run.
  - The 16-lane Adler-32 vectorizes in safe stable Rust on arm64 with no intrinsics.
  - The memory-safety argument in #12 is unchanged by this round.
- **Against, or not yet:**
  - It is not faster anywhere a release is built, and is 1-9% slower than Apple clang's C.
  - On x86-64 its Adler-32 is 14-21% slower and no safe form tried at `opt-level=2` closes that.
  - Whether LLVM vectorizes the checksum depends on the loop shape. It differs by target and
    opt-level, and the shape that ports the C's loop directly is 3.3× slower on arm64. A move
    would have to keep this benchmark, or a codegen check, as a gate across rustc upgrades.
  - The C is tuned per compiler (generic vectors for clang, the plain loop for GCC); the Rust has one
    shape for all.
  - The port leaves out the `TERMSHOT_PROFILE` deflate timers. It also does not return NULL on
    allocation failure as the C does (exit code 2 in `draw.c`), because `Vec` aborts instead. Both
    would need work.
  - `tests/deflate_diff.c` tests the C against stb directly. A Rust port would need the same
    differential test, which this bench's gate only approximates.

### Limits

- One machine per OS for the timed runs, two runs per host and compiler. macOS ran under a load of
  3.3-3.8 from other sessions. aarch64 is one shared-runner run, ratios only.
- The Rust side was tuned only as far as the Adler-32 loop shape and the three small matcher
  tries above; the matcher otherwise follows the C line for line.
- Only quality 8 is timed; the correctness gate covers qualities 5-16.

## Painting and deflate POC (2026-10-01, `bd726a6`), history

This section is history. It was measured on 2026-10-01 against `bd726a6`, before #20 rewrote
`deflate.c`'s Adler-32 and matcher, and before #61, #66 and #69 changed painting. Its painting
numbers do not describe the current `draw.c`. `bench/c-vs-rust/run.sh` without arguments still
reproduces it.

This POC answers one question: is termshot's own C (`src/draw.c`
painting and `src/deflate.c` compression) faster than the same code in Rust? The vendored stb
libraries are not part of it.

**Answer: no.** With the same compiler backend, the Rust and the C run at the same speed,
within a few percent. Swapping gcc for LLVM moves the numbers more than swapping C for Rust
does. So performance should not decide where code lives; memory safety should (see
[#12](https://github.com/momiji-rs/termshot/issues/12)). One caveat: the Rust port
has to be written so LLVM can optimize it, and a literal line-for-line port of one loop ran 3x
slower.

### What was compared

`bench/c-vs-rust/` holds both sides. Run it with `bench/c-vs-rust/run.sh`.

- **C:** `poc.c` includes `src/draw.c` as of `bd726a6` and copies `draw_png`'s painting section
  verbatim, without the timers. That section is the background, the glyph cache and blending,
  box-drawing geometry including arcs, and the underlines. Deflate is `src/deflate.c` at the
  same commit. `run.sh` builds this snapshot with `git archive`, so later changes to `draw.c`
  don't affect the comparison.
- **Rust:** `poc.rs` is a port of both, in safe Rust. It uses the same algorithms: the same
  bit-by-bit `bitrev` loop rather than `reverse_bits`, a 32-bit hash table like the C, and the
  same glyph cache. Glyph rasterization calls the same stb_truetype through FFI on both sides.
- **Checked first:** for every workload the two sides must produce byte-identical pixels and
  byte-identical compressed output, or the run aborts. All five match.
- **Timing:** both sides run in one process, alternating which goes first each round. The
  tables show the median of 61 rounds per call. C uses `build.sh`'s `-O2 -ffp-contract=off`;
  Rust uses `build.sh`'s `-C opt-level=2`.
- **Workloads** are parsed by termshot's own parser (`tests::poc_workloads`):

| workload | grid | image | what it stresses |
|---|---|---|---|
| 1-reply-px48 | 100×30 | 2200×1440 | the README sample |
| 2-reply-px128 | 100×30 | 5800×3840 | the same at a large size |
| 3-attrs-px24 | 96×6 | 2112×72 | 256 colours, every attribute |
| 4-boxes-px48 | 100×30 | 2200×1440 | rounded boxes: lines and arcs |
| 5-dense-200x60-px16 | 200×60 | 1400×900 | a different colour on every cell, so deflate barely matches |

### Results

R/C is Rust time ÷ C time; below 1 means Rust is faster.

**macOS, Apple M3, Apple clang 17 vs rustc 1.98.1** (both LLVM):

| workload | paint C | paint Rust | R/C | deflate C | deflate Rust | R/C |
|---|---:|---:|---:|---:|---:|---:|
| 1-reply-px48 | 1.110 ms | 1.073 ms | 0.97 | 4.075 ms | 4.282 ms | 1.05 |
| 2-reply-px128 | 3.563 ms | 3.971 ms | 1.11 | 19.129 ms | 20.174 ms | 1.05 |
| 3-attrs-px24 | 0.072 ms | 0.066 ms | 0.92 | 0.168 ms | 0.183 ms | 1.09 |
| 4-boxes-px48 | 1.641 ms | 1.696 ms | 1.03 | 2.429 ms | 2.633 ms | 1.08 |
| 5-dense-200x60-px16 | 1.870 ms | 1.879 ms | 1.00 | 29.512 ms | 31.194 ms | 1.06 |

Two more runs the same day gave paint 0.91–1.07 and deflate 1.04–1.07.

**Linux, Ryzen 7 8745HS, GCC 16.2.1 vs rustc 1.98.1** (gcc vs LLVM):

| workload | paint C | paint Rust | R/C | deflate C | deflate Rust | R/C |
|---|---:|---:|---:|---:|---:|---:|
| 1-reply-px48 | 0.669 ms | 0.945 ms | 1.41 | 5.143 ms | 4.065 ms | 0.79 |
| 2-reply-px128 | 5.759 ms | 6.014 ms | 1.04 | 27.061 ms | 19.975 ms | 0.74 |
| 3-attrs-px24 | 0.068 ms | 0.071 ms | 1.04 | 0.319 ms | 0.275 ms | 0.86 |
| 4-boxes-px48 | 1.013 ms | 1.352 ms | 1.34 | 3.421 ms | 2.574 ms | 0.75 |
| 5-dense-200x60-px16 | 1.785 ms | 2.008 ms | 1.12 | 27.030 ms | 25.588 ms | 0.95 |

**Linux, same machine, clang 22.1.8 vs rustc 1.98.1** (both LLVM):

| workload | paint C | paint Rust | R/C | deflate C | deflate Rust | R/C |
|---|---:|---:|---:|---:|---:|---:|
| 1-reply-px48 | 0.796 ms | 0.959 ms | 1.20 | 4.093 ms | 4.086 ms | 1.00 |
| 2-reply-px128 | 5.923 ms | 6.200 ms | 1.05 | 20.254 ms | 20.447 ms | 1.01 |
| 3-attrs-px24 | 0.066 ms | 0.070 ms | 1.05 | 0.274 ms | 0.285 ms | 1.04 |
| 4-boxes-px48 | 1.134 ms | 1.339 ms | 1.18 | 2.547 ms | 2.528 ms | 0.99 |
| 5-dense-200x60-px16 | 1.771 ms | 2.003 ms | 1.13 | 24.479 ms | 26.207 ms | 1.07 |

For scale: a whole run of workload 1 takes about 6 ms in-process. Deflate is about 4 ms of that,
painting about 1 ms, glyph rasterization (stb) 0.25 ms, the font check 0.2 ms, and parsing
0.05 ms (`TERMSHOT_PROFILE=1`).

### Findings

1. **Same backend, same speed.** On macOS (LLVM on both sides), painting is 0.91–1.11 and
   deflate 1.04–1.09. On Linux with clang, deflate is 0.99–1.07.
2. **The compiler matters more than the language.** Against gcc, Rust's deflate is 5–26%
   *faster* and its painting 4–41% slower. Switching the C to clang removes the deflate gap.
3. **Bounds checks cost deflate about 5%.** With `--cfg unchecked`, two `unsafe` reads in the
   match loop (the candidate prefix byte and the 8-byte compare), deflate on macOS is 0.98–1.01
   for workloads 1–4. Everything else stays safe.
4. **A literal port can be much slower.** The first port of Adler-32 indexed slices inside the
   32-byte block (`d[k]` on a shrinking `&d[32..]`). The bounds checks stopped vectorization:
   2.82 ms against 0.92 ms on workload 1, which made all of deflate 1.43–1.64x slower on
   workloads 1–2. Iterating `chunks_exact(32)` as `&[u8; 32]` fixed it in safe Rust; the C
   loop is written for vectorization in the same way. The literal version is kept in `poc.rs`
   as `adler32_literal`.
5. **`opt-level=3` was worse than 2** for this Adler-32 on M3: 4.5 ms against 0.92 ms, which made
   deflate 2.0–2.5x slower than the C. termshot builds with `opt-level=2`, so it isn't affected;
   the cause wasn't investigated.
6. **Not explained yet:** on x86_64, painting in Rust is 5–20% slower than clang's C, while on
   M3 it is even. The likely suspect is bounds checks in the blend loop. Painting is about a sixth
   of a run, so this is under 3% end to end.

### What this means for termshot

Moving `draw.c` or `deflate.c` to Rust would cost nothing in speed, if it is done the way the POC
does it: benchmarked against the C, and checked byte-identical. Every memory-safety bug found so
far was in C: the image-size overflow (#2), and the font out-of-bounds reads and the shared
canvas use-after-free (#8). That safety record, not speed, is the argument for moving.
[#12](https://github.com/momiji-rs/termshot/issues/12) weighs it.

### Limits

- One machine per OS. Linux was measured once per compiler; macOS three times.
- aarch64 Linux was not measured. The bench links, runs and matches byte for byte there
  (ubuntu-24.04-arm, gcc 13.3.0), checked by a 1-round CI step
  ([run](https://github.com/momiji-rs/termshot/actions/runs/37140972349/job/111255160837),
  2026-10-03). Shared-runner timings aren't kept.
- M3 timings move about ±10% with background load. The ratios come from alternating runs in one
  process, which is why they are steadier than the absolute times.
- stb_truetype and stb_image_write were not ported. They are third-party and stay C either way.
