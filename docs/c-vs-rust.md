# C vs Rust for termshot's own C code

This file holds dated comparisons, newest first, for
[#12](https://github.com/momiji-rs/termshot/issues/12): should termshot's own C
(`src/draw.c` painting and `src/deflate.c` compression) move to Rust? The vendored stb
libraries are not part of it. Each comparison ports the C as of one commit to Rust,
checks that both write the same bytes, and times both in one process.

- [Deflate, current code (2026-10-03, `a8a95e0`)](#deflate-current-code-2026-10-03-a8a95e0):
  `deflate.c` after #20 (16-lane Adler-32, bit reversal by table, inlined matcher).
- [Painting and deflate POC (2026-10-01, `bd726a6`), history](#painting-and-deflate-poc-2026-10-01-bd726a6-history):
  the first port of both files. Its deflate C is two optimization rounds old and its
  painting C predates #61, #66 and #69, so do not compare its numbers with the current code.

## Deflate, current code (2026-10-03, `a8a95e0`)

Measured 2026-10-03 against `src/deflate.c` at `a8a95e0` (main after #20, #69).
Painting (`draw.c`) is **not** in scope for this round.

**Short answer:** a safe Rust port of the current `deflate.c` writes the same bytes and,
in the whole compressor, runs 2-8% slower than Apple clang's C on macOS arm64. Against GCC on Linux
x86-64 it runs from 2% faster to 3% slower on the image inputs, 12% slower on the one input that is
mostly checksum, and 7-8% slower on random bytes. The new 16-lane Adler-32 matches the C on arm64
in safe stable Rust, but only in some loop shapes: written in the C's own loop shape (one chunk
per step), it is 3.3× slower there at `opt-level=2`. On x86-64 the Rust Adler-32 is 14-21% slower
than the C. Two `unsafe` reads buy at most 4 points and close no gap.

### What was compared

`bench/c-vs-rust/run.sh deflate [rounds]` runs it; `CC=gcc` or `CC=clang` picks the C
compiler.

- **C:** `src/deflate.c` and `src/deflate_profile.h` at `a8a95e0`, taken with `git archive`
  as the 2026-10-01 POC takes `bd726a6`. `deflate_shim.c` includes it twice under two prefixes, both with
  `build.sh`'s `-O2`:
  - **C**: the default build. Under clang, this is the generic-vector Adler-32. Under GCC, it is
    the plain loop, which GCC vectorizes.
  - **C portable**: `-DTERMSHOT_PORTABLE_ADLER`, the plain loop on every compiler. Under GCC
    it is the same code as **C**.
- **Rust:** `deflate.rs`, a port in safe Rust with the same algorithms. It has the 16-lane Adler-32 with
  weights applied once per 5552-byte block, the 256-byte reversed-byte table, `countm`
  comparing 16 bytes, then 8, then single bytes, the newest-first bucket scan with the same
  early exits, lazy matching, and the carried hash. The output buffer appends four bytes per token and keeps
  the complete ones, as the C does. It does not port the `TERMSHOT_PROFILE` timers, which are off in this
  bench, so the C pays only four thread-local `enabled` checks per call. It is built with `build.sh`'s
  `-C opt-level=2`.
  - **Rust unchecked**: the 2026-10-01 POC's `--cfg unchecked` with the same two
    `unsafe` reads, the candidate-rejection byte and `countm`'s loads. Here it is a const
    generic, so both Rust variants run in the same process and rounds as the two C builds.
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
  termshot's PNG. That is the filtered scanlines, rendered by the current CLI from the same
  logs (`deflate_inputs.py`). There are also two seeded synthetic buffers. Quality 8, as termshot uses.

| input | bytes | source |
|---|---:|---|
| 1-reply-px48 | 9,505,440 | `tests::poc_workloads`, `reply-sent.pty` at 48 px (the README sample, `reply-sent`) |
| 2-reply-px128 | 66,819,840 | the same at 128 px: the size of `reply-128px`, the #20 Adler-32 stress case |
| 3-attrs-px24 | 456,336 | `poc_workloads`: 256 colours, every attribute |
| 4-boxes-px48 | 9,505,440 | `poc_workloads`: rounded boxes |
| 5-dense-200x60-px16 | 3,780,900 | `poc_workloads`: a different colour on every cell |
| 6-blank | 9,505,440 | `scripts/bench.py`'s `blank`, nearly all Adler-32 (from the #20 round) |
| 6-color-grid | 2,376,720 | `bench.py`'s `color-grid`: short matches |
| 6-large | 60,829,440 | `bench.py`'s `large`: 240×80 at 48 px |
| 7-random-uniform | 2,376,720 | seeded uniform random bytes: every position a literal |
| 8-random-4sym-skewed | 2,376,720 | seeded, four symbols at 9/16, 4/16, 2/16, 1/16 |

- **Timing:** all four variants run in each round, and the first of them rotates each round. Each
  call is timed alone; copying and freeing the output are not timed. The tables give the
  median / p95 of 101 rounds per call, in ms. R/C is the Rust median ÷ the **C** median;
  below 1 means Rust is faster. The Adler-32 rows time the checksum alone on the two reply inputs,
  with the same rotation.

| | macOS arm64 | Linux x86-64 |
|---|---|---|
| host | `lawrences-mac-studio`, Apple M2 Max, macOS 26.6.2 | `starship`, Ryzen 7 8745HS, kernel 7.2.5-3-omarchy, glibc 2.44 |
| C compilers | Apple clang 21.0.0 (clang-2100.3.34.2) | GCC 16.2.1 20260810, clang 22.1.8 |
| Rust | rustc 1.98.1 (Homebrew) | rustc 1.98.1 (Arch) |
| load average (1 min) | 3.3 → 4.4 and 2.6 → 4.3: a shared desktop, other sessions busy | GCC 1.1 → 1.3 and 3.3 → 1.9; clang 1.5 → 1.7 |

### Results

**macOS arm64, Apple clang 21 vs rustc 1.98.1** (both LLVM), first of two runs:

| input | C | C portable | Rust safe | Rust unchecked | R/C safe | R/C unchecked |
|---|---:|---:|---:|---:|---:|---:|
| 1-reply-px48 | 3.104 / 3.250 | 4.184 / 4.351 | 3.223 / 3.407 | 3.237 / 3.384 | 1.04 | 1.04 |
| 2-reply-px128 | 13.220 / 13.621 | 20.729 / 21.282 | 13.642 / 14.117 | 13.685 / 14.089 | 1.03 | 1.04 |
| 3-attrs-px24 | 0.205 / 0.229 | 0.258 / 0.286 | 0.221 / 0.248 | 0.218 / 0.243 | 1.08 | 1.06 |
| 4-boxes-px48 | 1.710 / 1.824 | 2.793 / 2.905 | 1.751 / 1.864 | 1.734 / 1.854 | 1.02 | 1.01 |
| 5-dense-200x60-px16 | 24.543 / 25.469 | 25.012 / 25.709 | 26.573 / 27.586 | 26.038 / 27.016 | 1.08 | 1.06 |
| 6-blank | 1.158 / 1.258 | 2.222 / 2.352 | 1.191 / 1.277 | 1.183 / 1.262 | 1.03 | 1.02 |
| 6-color-grid | 11.534 / 11.900 | 11.773 / 12.185 | 12.311 / 12.784 | 12.213 / 12.566 | 1.07 | 1.06 |
| 6-large | 18.685 / 19.720 | 25.408 / 27.256 | 19.432 / 20.774 | 19.245 / 20.013 | 1.04 | 1.03 |
| 7-random-uniform | 60.587 / 69.542 | 60.613 / 70.126 | 64.840 / 80.120 | 64.378 / 74.028 | 1.07 | 1.06 |
| 8-random-4sym-skewed | 53.629 / 59.817 | 54.231 / 59.693 | 54.484 / 59.872 | 54.078 / 60.641 | 1.02 | 1.01 |
| Adler-32 alone, reply-px48 | 0.484 / 0.533 | 1.555 / 1.652 | 0.488 / 0.540 | — | 1.01 | — |
| Adler-32 alone, reply-px128 | 3.455 / 3.642 | 11.009 / 11.304 | 3.505 / 3.637 | — | 1.01 | — |

The second run gave the same R/C to within 0.02: safe 1.02-1.08, unchecked 1.01-1.06,
Adler-32 1.01-1.02.

**Linux x86-64, GCC 16.2.1 vs rustc 1.98.1** (GCC vs LLVM; Linux releases are built with
`musl-gcc`), first of two runs:

| input | C | C portable | Rust safe | Rust unchecked | R/C safe | R/C unchecked |
|---|---:|---:|---:|---:|---:|---:|
| 1-reply-px48 | 3.110 / 3.159 | 3.099 / 3.146 | 3.073 / 3.121 | 3.032 / 3.081 | 0.99 | 0.98 |
| 2-reply-px128 | 13.538 / 13.912 | 13.445 / 13.606 | 13.813 / 14.008 | 13.784 / 13.933 | 1.02 | 1.02 |
| 3-attrs-px24 | 0.221 / 0.230 | 0.221 / 0.228 | 0.224 / 0.234 | 0.220 / 0.254 | 1.01 | 1.00 |
| 4-boxes-px48 | 1.658 / 1.705 | 1.668 / 1.707 | 1.697 / 1.743 | 1.715 / 1.770 | 1.02 | 1.03 |
| 5-dense-200x60-px16 | 23.263 / 23.482 | 23.264 / 23.460 | 23.287 / 23.428 | 22.599 / 22.863 | 1.00 | 0.97 |
| 6-blank | 1.124 / 1.140 | 1.130 / 1.147 | 1.253 / 1.274 | 1.264 / 1.282 | 1.11 | 1.12 |
| 6-color-grid | 11.005 / 11.288 | 10.989 / 11.265 | 11.099 / 11.288 | 10.737 / 10.919 | 1.01 | 0.98 |
| 6-large | 19.530 / 19.595 | 19.558 / 19.610 | 19.204 / 19.271 | 18.945 / 19.014 | 0.98 | 0.97 |
| 7-random-uniform | 55.734 / 56.312 | 55.836 / 56.426 | 59.913 / 60.221 | 57.286 / 57.967 | 1.07 | 1.03 |
| 8-random-4sym-skewed | 47.852 / 48.240 | 47.822 / 48.267 | 47.017 / 47.323 | 45.694 / 46.082 | 0.98 | 0.95 |
| Adler-32 alone, reply-px48 | 0.499 / 0.511 | 0.499 / 0.507 | 0.604 / 0.613 | — | 1.21 | — |
| Adler-32 alone, reply-px128 | 3.547 / 3.572 | 3.548 / 3.572 | 4.286 / 4.313 | — | 1.21 | — |

The second run gave the same R/C to within 0.01: safe 0.98-1.12, unchecked 0.95-1.12,
Adler-32 1.21.

**Linux x86-64, same machine, clang 22.1.8 vs rustc 1.98.1** (both LLVM):

| input | C | C portable | Rust safe | Rust unchecked | R/C safe | R/C unchecked |
|---|---:|---:|---:|---:|---:|---:|
| 1-reply-px48 | 2.937 / 3.023 | 3.696 / 3.764 | 3.081 / 3.133 | 3.037 / 3.102 | 1.05 | 1.03 |
| 2-reply-px128 | 13.361 / 13.599 | 18.426 / 18.819 | 13.777 / 14.249 | 13.854 / 14.127 | 1.03 | 1.04 |
| 3-attrs-px24 | 0.204 / 0.290 | 0.241 / 0.361 | 0.223 / 0.312 | 0.219 / 0.298 | 1.09 | 1.07 |
| 4-boxes-px48 | 1.621 / 1.651 | 2.364 / 2.395 | 1.689 / 1.712 | 1.707 / 1.739 | 1.04 | 1.05 |
| 5-dense-200x60-px16 | 21.105 / 21.648 | 21.815 / 22.163 | 23.271 / 23.815 | 22.499 / 22.837 | 1.10 | 1.07 |
| 6-blank | 1.177 / 1.204 | 1.919 / 1.971 | 1.253 / 1.283 | 1.272 / 1.288 | 1.06 | 1.08 |
| 6-color-grid | 10.285 / 10.620 | 10.609 / 10.767 | 11.166 / 11.453 | 10.869 / 11.205 | 1.09 | 1.06 |
| 6-large | 18.315 / 20.419 | 23.082 / 25.946 | 19.324 / 21.329 | 18.991 / 21.926 | 1.06 | 1.04 |
| 7-random-uniform | 52.836 / 56.054 | 53.985 / 56.984 | 60.957 / 65.170 | 58.666 / 62.108 | 1.15 | 1.11 |
| 8-random-4sym-skewed | 46.689 / 48.466 | 46.407 / 48.371 | 47.821 / 49.195 | 45.525 / 47.220 | 1.02 | 0.98 |
| Adler-32 alone, reply-px48 | 0.532 / 0.550 | 1.256 / 1.287 | 0.604 / 0.620 | — | 1.14 | — |
| Adler-32 alone, reply-px128 | 3.789 / 3.830 | 8.885 / 8.923 | 4.305 / 4.348 | — | 1.14 | — |

This is the third clang run. The first gave the same R/C to within 0.05, but the host's load rose to
7.7 while it ran and its `6-large` and `7-random-uniform` times were disturbed. In the second,
all four variants of `2-reply-px128` ran about 1.75× slower than in the other runs (23.4 ms for
the C), which points to contention from another job, so it is not used. Its R/C were 1.00-1.14
safe and 0.97-1.10 unchecked.

**Linux aarch64, ratios only.** One run on a shared GitHub `ubuntu-24.04-arm` runner, with GCC
13.3.0 and Ubuntu clang 18.1.3 against rustc 1.98.1, 61 rounds
([run](https://github.com/momiji-rs/termshot/actions/runs/37172877370/job/111349279959),
from a scratch branch since deleted). Shared-runner timings are not kept as numbers, and the
ratios have that caveat too.

| | R/C safe | R/C unchecked | Adler-32 R/C | C portable / C |
|---|---|---|---|---|
| GCC 13.3 | 0.98-1.10 | 1.03-1.11 | 1.05-1.07 | 0.99-1.02 |
| clang 18.1 | 0.99-1.09 | 0.98-1.15 | 0.97-0.99 | 1.01-2.05 |

### Codegen: does safe Rust vectorize the 16-lane Adler-32?

Yes, on both targets, but the loop shape decides it. Every form below keeps the C's 16
lanes and gives each lane the same adds in the same order (`p[k] += a[k]`, then `a[k] += x[k]`, chunk
by chunk). They differ only in how many 16-byte chunks one step of the outer loop takes.
`bench/c-vs-rust/adler_forms.rs` checks each against the scalar definition, then times
them on 66,819,840 random bytes. The table gives ms, median of 31 interleaved rounds. The C rows are
the bench's Adler-32 alone on `2-reply-px128`, which is the same length, and whose speed does not depend on
the data.

| form | M2 Max, `opt-level=2` | M2 Max, `opt-level=3` | Ryzen, `opt-level=2` | Ryzen, `opt-level=3` |
|---|---:|---:|---:|---:|
| one chunk per step (as the C) | 11.46 | 11.47 | 4.12 | 14.14 |
| literal: index a shrinking slice | 24.20 | 16.37 | 20.01 | 20.83 |
| 2 chunks per step | 3.37 | 9.38 | 4.27 | 3.63 |
| 4 chunks per step | 3.42 | 3.95 | 4.21 | 4.22 |
| **8 chunks per step (`deflate.rs`)** | **3.43** | 3.33 | **4.28** | 4.31 |
| C, Apple clang, generic vectors | 3.43-3.46 | | | |
| C, GCC 16, plain loop | | | 3.55 | |
| C, clang 22, generic vectors | | | 3.79 | |

- **The C's own loop shape (one chunk per step) stays scalar on arm64 at `opt-level=2`**: 11.46
  ms against 3.43. The POC's literal Adler-32 also lost its vectorization, but to bounds checks;
  this form has none in its inner loop. On x86-64 it vectorizes at 2 but not at 3.
  `-C no-vectorize-loops` makes every form scalar on both hosts at `opt-level=2` (10.7-31.5 ms
  on the M2, 11.0-28.3 on the Ryzen). So when the work is vectorized at 2, the loop vectorizer does it.
  `-C no-vectorize-slp` changes nothing at 2. Taking two or more chunks per step keeps the
  lane loop a loop that the loop vectorizer takes. Four and eight are vectorized at both
  opt-levels on both targets. Eight is as fast as four at 2 and faster on arm64 at 3, which is
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
  load at `opt-level=2`; at 3, two chunks per step reaches 3.63 ms.
- **Inspect the linked binary, not `--emit asm`.** In one case (`adler_forms.rs`, one chunk per step,
  `opt-level=3`, arm64) the `.s` from `--emit asm` showed the vector loop while the linked
  binary's function was scalar.
- No `unsafe`, `std::simd` or `core::arch` is needed for the arm64 result. A `core::arch` x86-64
  form was not tried.

### Findings

1. **Safe Rust matches the new Adler-32 on arm64** (R/C 1.01-1.02 on the M2, 0.97-0.99 against
   clang 18 on aarch64 Linux, 1.05-1.07 against GCC 13's plain loop there), in a loop shape LLVM
   vectorizes. It does not match it on x86-64: 1.14×
   clang 22 and 1.21× GCC 16. That is about 0.07-0.10 ms on `reply-sent`'s 9.5 MB, and 0.5-0.7 ms at
   128 px.
2. **Safe Rust is slightly slower in matching and emission against LLVM's C,** and even with
   GCC's on image inputs. The compressor minus its checksum is about 4% slower on `reply-sent`
   (M2: +0.11 ms of 2.6). On the match-heavy inputs (`dense`, `color-grid`,
   random bytes) it is 6-8% slower on the M2 and 9-15% with clang on x86-64, and 1.07-1.08 on random
   bytes with GCC. Against GCC the image inputs are 0.98-1.03, apart from `blank`, which is
   mostly Adler-32 and is 1.11-1.12. The cause of the LLVM-side matching gap was not found. Three
   tries showed no effect beyond noise in 15-round runs on the M2: one bounds check instead of
   three in the hash (kept), a fixed-size count array, and writing each token into a pre-sized
   scratch buffer as the C does.
3. **`unchecked` buys little:** at most 4 points (`7-random-uniform` with GCC, 1.07 → 1.03),
   0-2 points on most inputs, and on aarch64 it was slower than safe on most inputs. The safe
   `countm` here walks `chunks_exact(16)` over two slices cut to `limit`, so its 16-byte compares
   carry no per-load check. The POC's safe `countm`, which closed a 5% gap when unchecked, sliced
   `a[i..i + 8]` at every step.
4. **The compiler still matters as much as the language,** but it matters differently from 2026-10-01. #20's
   per-compiler C closed the GCC gap that made Rust 5-26% faster then. On clang hosts, the plain loop that clang
   does not vectorize costs the C up to 1.93× in the whole compressor (C portable / C, most where the
   checksum is a large share) and 2.35-3.24× in the Adler-32 alone.

### End-to-end context

Per `docs/performance.md`, `reply-sent` takes about 8.8 ms on the M2 Max and 7.6 ms on the
Ryzen, of which PNG encoding (deflate, with filtering and packing) is about 3.3 ms and 4.2
ms. On its deflate input, the measured difference of the whole compressor is:

| | C | Rust safe | difference | share of a `reply-sent` run |
|---|---:|---:|---:|---:|
| macOS, Apple clang (release compiler) | 3.10 | 3.22-3.24 | +0.12-0.13 ms | +1.4% |
| Linux x86-64, GCC (release compiler: `musl-gcc`) | 3.11-3.14 | 3.07-3.11 | -0.03-0.04 ms | -0.5% |
| Linux x86-64, clang | 2.94 | 3.08 | +0.14 ms | +1.9% |

At 128 px (`reply-128px`, about 29 ms on macOS) the difference is about +0.4 ms, or 1.5%. The x86-64 slice of the
macOS universal binary was not measured.

### What this means for #12

The evidence, without a decision:

- **For moving `deflate.c`:**
  - A safe-Rust port of the current code is byte-identical on 3,260 differential cases and ten
    workloads, on three platforms and five C compilers.
  - It needs no `unsafe`. The `unchecked` variant shows bounds checks are not where the remaining time
    goes.
  - Against the Linux release compiler (GCC) it is even on image inputs on x86-64 (0.98-1.03;
    0.98-1.10 against GCC 13 on aarch64).
  - On macOS it costs about 1.4% of a typical run.
  - The 16-lane Adler-32 vectorizes in safe stable Rust on arm64 with no intrinsics.
  - The memory-safety argument in #12 is unchanged by this round.
- **Against, or not yet:**
  - It is not faster anywhere a release is built, and is 2-8% slower than Apple clang's C.
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

- One machine per OS for the timed runs. macOS ran twice under a load of 2.6-4.4 from other sessions.
  Linux GCC ran twice and clang once cleanly. aarch64 is one shared-runner run, ratios only.
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
