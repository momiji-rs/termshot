# C vs Rust for termshot's own C code

Measured 2026-10-01. This POC answers one question: is termshot's own C (`src/draw.c`
painting and `src/deflate.c` compression) faster than the same code in Rust? The vendored stb
libraries are not part of it.

**Answer: no.** With the same compiler backend, the Rust and the C run at the same speed,
within a few percent. Swapping gcc for LLVM moves the numbers more than swapping C for Rust
does. So performance should not decide where code lives; memory safety should (see
[#12](https://github.com/momiji-rs/termshot/issues/12)). One caveat: the Rust port
has to be written so LLVM can optimize it, and a literal line-for-line port of one loop ran 3x
slower.

## What was compared

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

## Results

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

## Findings

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

## What this means for termshot

Moving `draw.c` or `deflate.c` to Rust would cost nothing in speed, if it is done the way the POC
does it: benchmarked against the C, and checked byte-identical. Every memory-safety bug found so
far was in C: the image-size overflow (#2), and the font out-of-bounds reads and the shared
canvas use-after-free (#8). That safety record, not speed, is the argument for moving.
[#12](https://github.com/momiji-rs/termshot/issues/12) weighs it.

## Limits

- One machine per OS. Linux was measured once per compiler; macOS three times.
- M3 timings move about ±10% with background load. The ratios come from alternating runs in one
  process, which is why they are steadier than the absolute times.
- stb_truetype and stb_image_write were not ported. They are third-party and stay C either way.
