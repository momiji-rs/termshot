# CFF outlines for #25: Rust vs C

Measured 2026-10-02 on starship (Ryzen 7 8745HS, Arch, GCC 16.2.1, rustc 1.98.1). Issue
[#25](https://github.com/momiji-rs/termshot/issues/25) asks for CFF fonts (`.otf`, Noto
Sans CJK `.ttc`) and for picking a face in a collection. stb_truetype 1.26 already draws CFF
outlines, so the question is not whether termshot *can* draw them but whether it can do so
**safely**. `src/font.rs` exists because stb trusts its input, and stb's CFF reader trusts it
more than its TrueType reader does. This POC writes a CFF reader twice, in Rust and in C, with
the same design. It checks both against stb and compares speed, robustness and cost.

**Answer: build it in Rust.** It is now `src/cff.rs`; `bench/cff-poc/` keeps both POC versions.
Both versions draw every glyph of 115 faces exactly as stb does,
and neither crashed once. stb crashed on 1.8% of fuzzed CJK fonts and 0.6% of fuzzed Latin
ones, and crashed or hung on all 8 hostile fonts. The two run at the same speed, about 1.5×
faster per drawn glyph than stb because stb runs each charstring four times. They are also
the same size. The difference is what keeps them safe. Rust's safety comes from the compiler.
The C's safety rests on every hand-written length check (43 error returns) staying correct,
and termshot has decided ([#12](https://github.com/momiji-rs/termshot/issues/12), [#8](https://github.com/momiji-rs/termshot/issues/8))
that parsing untrusted bytes belongs in Rust.

## What stb does with a CFF font

Read from `stb_truetype.h` 1.26 and confirmed with the hostile fonts below:

- `info->cff` is given a fixed length of **512 MB** (`stbtt__new_buf(data+cff, 512*1024*1024)`,
  marked `@TODO`). Nothing inside the CFF table is bounded by the table, or by the file.
- Asserts are on in termshot's build, so a bad byte **aborts the process**. The triggers include
  an INDEX offset size outside 1–4, DICT operand byte 31 or 255, a real number where stb reads
  an integer, a glyph id at or past the CharStrings count, a glyph that FDSelect doesn't cover
  (`if (fdselector == -1) stbtt__new_buf(NULL, 0);` is missing its `return`), and a hintmask
  that runs past the charstring.
- Subroutine calls are limited to depth 10 but **not in number**: ten levels that each call
  the next eight times are 8⁹ calls per pass, and stb makes four passes per glyph.
- Each drawn glyph runs its charstring **four times**: `GetGlyphBitmapBox` once, then
  `MakeGlyphBitmap` once for the box and twice for the shape (a counting pass and a writing
  pass).

Verified separately: stb draws Noto Sans CJK correctly once `font.rs` lets it through. The
`.ttc` has 10 faces (JP, KR, SC, TC, HK, then the Mono five). All of them share one 15 MB CFF
table with 65535 glyphs and differ only in their `cmap`. 骨 and 直 are drawn differently in TC,
SC and JP, so zh-TW users want face 3.

## What was compared

`bench/cff-poc/` holds both sides. Run `bench/cff-poc/run.sh`.

- **`cff.rs` and `cff.c`** have the same design. Both find the face, check that the CFF
  structures lie inside the table (not 512 MB), and run Type 2 charstrings as
  `stbtt__run_charstring` does, with the same float accumulation, casts, subroutine bias and
  stack rules. They count and bound with full 32-bit coordinates and store 16-bit vertices,
  all in one pass rather than stb's two. Where stb would assert, read out of bounds or run
  without end, they return an error. A per-glyph budget of 20,000 operators stops subroutine
  bombs; the most any real glyph used was 2,419. A bad entry in an INDEX costs only that
  glyph, which is drawn empty, as stb draws it.
- **Option C** is the integration both are built for: our outline and box go to stb's public
  `stbtt_Rasterize`, with the same arguments `stbtt_MakeGlyphBitmapSubpixel` uses. stb keeps
  the rasteriser, which is trustworthy because it only sees vertices we produced. The pixels
  don't move.
- **Checked:** `poc check` compares the vertices and box of every glyph of every face against
  stock stb. It also compares rasterised bitmaps byte for byte for every 16th glyph of each
  file, at 16 px and 33.5 px.
- **Timing:** all three run in one process, in turns, for 21 or 61 rounds; the tables give the
  median. Every implementation uses `build.sh`'s flags: `-O2 -ffp-contract=off` for the C,
  `-C opt-level=2` for the Rust.
- **Fuzzing:** `fuzz.c` corrupts 1–8 bytes of the CFF table: bit flips, random bytes, and
  bytes that are charstring and DICT operators. It runs every glyph through each
  implementation in its own child process, under ASan and UBSan, with a 3 s limit. cff.rs runs
  there with overflow checks and debug assertions, and a panic counts as a failure. For a
  mutant on which all three run to the end, a hash of every outline and box shows whether
  they drew the same thing. `craft.py` writes eight hostile fonts, one defect each, plus a
  well-formed control.

## Results

### Correctness

| fonts | faces | glyphs | cff.rs = stb | cff.c = stb | rasters identical |
|---|---:|---:|---|---|---:|
| gsfonts, 35 `.otf` (plain CFF) | 35 | 28,609 | all | all | 3,536 / 3,536 |
| Noto Sans + Serif CJK, 14 `.ttc` (CID-keyed) | 80 | 5,242,800 | all | all | 114,506 / 114,506 |

### Speed

Per glyph, median. "All glyphs" is the outline and box of every glyph; stb's figure is
`GetGlyphShape` + `GetGlyphBox` (three charstring runs, ours one). "Draw" is what termshot
pays per glyph it renders at 16 px: stb as `draw.c` calls it today, against our outline plus
`stbtt_Rasterize`.

| font | | cff.rs | cff.c | stb |
|---|---|---:|---:|---:|
| Noto Sans CJK TC, 65535 glyphs | parse | 9.5 µs | 5.1 µs | 2.5 µs |
| | all glyphs | 1.23 µs | 1.26 µs | 3.78 µs |
| | draw 1500 CJK glyphs | 5.78 µs | 5.81 µs | 8.63 µs |
| Nimbus Sans, 855 glyphs | parse | 1.7 µs | 0.7 µs | 1.0 µs |
| | all glyphs | 0.44 µs | 0.45 µs | 1.14 µs |
| | draw 95 ASCII glyphs | 2.36 µs | 2.38 µs | 3.24 µs |

A screenshot full of CJK draws a few hundred distinct glyphs, so the whole CFF cost is about
2 ms either way, and 1500 glyphs save about 4 ms against stb. Parsing costs a few µs because
INDEX entries are checked when fetched, not up front. The first version checked every offset
at load: 137 µs on the CJK font, and it refused a whole font over one bad offset.

### Robustness

Hand-made fonts (`craft.py`):

| defect | stb | cff.c | cff.rs |
|---|---|---|---|
| subroutine bomb (8 calls × 10 levels) | **hang** | glyph refused | glyph refused |
| CharStrings INDEX runs 1 MB past the table | **heap overflow** (ASan) | font refused | font refused |
| hintmask runs past its charstring | **assert** | drawn | drawn |
| Private DICT Subrs offset is a real | **assert** | font refused | font refused |
| Top DICT operand byte 31 | **assert** | font refused | font refused |
| maxp has more glyphs than CharStrings | **assert** | font refused | font refused |
| FDSelect doesn't cover glyph 0 | **assert** | font refused | font refused |
| an INDEX with 7-byte offsets | **assert** | font refused | font refused |
| control | drawn | drawn | drawn |

Mutation fuzz, 20,000 mutants per font:

| font | | ran | refused | assert | memory / UB | hang | panic |
|---|---|---:|---:|---:|---:|---:|---:|
| CJK subset (CID, 176 glyphs, 25 KB CFF) | stb | 19,634 | 11 | 284 | 71 | 0 | n/a |
| | cff.c | 19,572 | 428 | 0 | 0 | 0 | n/a |
| | cff.rs | 19,572 | 428 | 0 | 0 | 0 | 0 |
| Nimbus Sans (plain CFF, 855 glyphs) | stb | 19,875 | 2 | 118 | 5 | 0 | n/a |
| | cff.c | 19,941 | 59 | 0 | 0 | 0 | n/a |
| | cff.rs | 19,941 | 59 | 0 | 0 | 0 | 0 |

The CJK row was rerun on 2026-10-02. The first subset held only its ASCII glyphs, because
`hb-subset --unicodes=` replaces the set `--text` gives rather than adding to it; `run.sh` now
uses `--unicodes+=`. That run (129 glyphs, all Latin, in a CID-keyed font) had stb crash on 3.2%
and the two readers agree with stb on all 19,020 mutants all three ran.

Where all three ran to the end, cff.rs and cff.c drew **every glyph identically to stb**: in
19,489 of 19,489 CJK mutants and 19,869 of 19,869 Latin ones. cff.rs and cff.c agreed with each
other on every mutant both ran (19,572 and 19,941). Matching stb on garbage was not a
goal, so this is the strongest sign that the two readers follow stb's semantics, and not just
on well-formed fonts. They refuse more fonts than stb, because a font whose headers are broken
is refused with a reason where stb reads on. Most of those are refused for what makes stb
assert: an INDEX offset size, a DICT operand byte, an FDSelect that doesn't cover every glyph.

### Size and checking

| | cff.rs | cff.c (+ cff.h) |
|---|---:|---:|
| non-blank, non-comment lines | 663 (154 just a brace) | 566 + 18 |
| non-space characters | 13,305 | 13,811 + 446 |
| `unsafe` | 0 | n/a |
| what a missed length check becomes | a panic, caught as an error | an out-of-bounds read |
| static analysis | clippy: 2 hints, both need Rust > 1.70 | `gcc -fanalyzer`, `clang --analyze`: clean after 2 fixes |

cff.rs compiles with rustc 1.70 and cff.c with `-std=c99 -pedantic`. Neither needs anything
beyond libc and libm.

## Findings

1. **Speed doesn't separate them.** cff.rs and cff.c run within 3% of each other on every
   measure, as in [docs/c-vs-rust.md](c-vs-rust.md). The 1.5× gain over stb comes from the
   design (one pass instead of four) and would come with either language.
2. **The C needed help that the Rust didn't.** The C passes ASan, UBSan and both static
   analysers, but only because of discipline the compiler doesn't check. Every read goes
   through `u16_at`/`u32_at`/`sub`, so no pointer leaves the table. Float-to-int casts go
   through `to_int`, because C's cast is undefined behaviour out of range and a fuzzed
   coordinate gets there. Pointers are clamped so no out-of-range one is formed. The analysers
   found two places where that discipline relied on reasoning they couldn't follow: an ignored
   return value that was in fact safe, and a pointer that was NULL only in a path that couldn't
   happen. In the Rust each of these is a slice index, `as`, or `Option`, checked by the
   compiler, and a slip is a panic (caught and turned into an error), not memory corruption.
3. **The Rust is not shorter.** The same logic is the same size. The C spells out 43 error
   returns; in the Rust most of them are a `?`, `ok_or` or `.get(` folded into an expression
   (54 lines), but rustfmt's layout gives back the lines.
4. **Integration is where they differ.** cff.c would sit next to draw.c and be called lazily,
   with no FFI change. cff.rs needs outlines to cross from Rust to C, in one of two ways:
   - *a callback*: draw.c calls an exported Rust function for each new glyph. This is small,
     but the C-only harness `tests/draw.c` then needs a stub or the Rust objects.
   - *precomputed*: Rust works out which glyphs the grid uses (cmap lookup, primary or
     fallback) and hands draw.c a table of outlines. This is bigger, but it moves font logic
     out of C, which is where #8 points, and it would let the TrueType path follow later.
5. **Validation alone isn't enough, in either language.** Checking at load can't catch a
   subroutine bomb or a per-glyph defect without running every charstring (248 ms for the CJK
   font in stb). Only an interpreter of our own, with a budget, closes that, and once we have
   one, keeping stb's would mean running two.

## What this means for termshot

- Implement #25 as **cff.rs in `src/`, behind option C**: Rust parses CFF and runs
  charstrings, and stb rasterises. `font.rs` accepts `CFF ` in place of `glyf`/`loca`
  (refusing `CFF2`) and runs `cff::Font::parse`. A glyph that errors is drawn as tofu.
- Start with the **callback** (one new extern function, `draw.c` swaps `MakeGlyphBitmap` for
  `stbtt_Rasterize` on CFF faces). Leave precomputing for the #8 refactor.
- Keep `bench/cff-poc/fuzz.c` and the `craft.py` fonts as tests, and the stb differential as a
  harness in the style of `tests/deflate_diff.c`. stb is the oracle on well-formed fonts.
- Face selection is independent and can go first. All Noto CJK faces share their outlines, so
  it only changes which `cmap` is read.

## Limits

- One machine, one compiler pair. docs/c-vs-rust.md found that gcc vs LLVM moves numbers more
  than the language does; that should hold here.
- Fonts: the system's gsfonts and Noto CJK, plus one `hb-subset` cut. No CFF2 (variable
  fonts), which neither stb nor this POC reads, and no Type 1-derived fonts with `seac`
  accents (stb doesn't draw them either).
- The fuzzer is a blind mutator, 40,000 mutants, not coverage-guided. It mutates only the CFF
  table; `font.rs` covers the rest of the sfnt.
- The operator budget (20,000) is about 8× the busiest glyph seen. A font that needs more is
  refused glyph by glyph, not misdrawn.

## CFF2: the default instance (#51)

Added 2026-10-03. stb_truetype has no CFF2 reader (`stbtt_InitFont` refuses a face with
neither `glyf` nor `CFF `), so `cff.rs` reads CFF2 too, with the same design: every structure
checked at load, the charstrings run with the 20,000-operator budget, and stb only
rasterizes. For a CFF2 face, `draw.c` does the part of `stbtt_InitFont` that doesn't touch
outlines itself (`init_cff2`: metrics tables and the cmap subtable stb would pick).

- **What differs from CFF**: a header with the Top DICT's length and no Name or String INDEX;
  32-bit INDEX counts; a required FDArray, an optional FDSelect (and its format 4); no width,
  `endchar` or `return`, as a charstring or a subroutine ends where its data does; `vsindex`
  and `blend`, also in Private DICTs; a 513-deep argument stack.
- **The default instance**: `blend` keeps its n default values and drops the n × k deltas
  under them. k is the region count of the ItemVariationData that `vsindex` names, so the
  variation store is read for its region counts, and its region list and data are checked to
  lie inside it.
- **Other instances**: given normalized coordinates (F2Dot14), `blend` adds to each value the
  sum of its k deltas times their region scalars, as HarfBuzz does: the sum in f64 from 0, each
  scalar the f32 product of its axes', in `VarRegionAxis::evaluate`'s order of cases (a
  malformed axis, or one peaking at 0, counts as 1). Each region is evaluated once per font,
  when the store is read, so a `blend` costs its k multiply-adds. As in HarfBuzz, `vsindex`
  after a `blend` or a second `vsindex` is an error, since the scalars are fixed by then.
- **Arithmetic as HarfBuzz's**: CFF charstrings run in f32, as stb runs them; CFF2 ones run
  in f64, as HarfBuzz runs them, and each point is drawn as the f32 HarfBuzz hands on, then
  truncated. A contour is closed with a line where its ends differ as f32s. Steps of 1/4096
  from x = 8192 show the difference: f32 rounds each one away (`cff_tests.rs`).
- **Strict where CFF follows stb**: with no stb to match, a CFF2 charstring that breaks a CFF2
  rule (a `vsindex` past the store, a `blend` short of operands, a stack past 513, `endchar`)
  is an error for that glyph, which is drawn as the missing-glyph box. Everything else in the
  charstring language is read as for CFF.
- **Checked against HarfBuzz 14.4.0** (`hb-vector`, the default instance), outline for outline,
  on starship: every mapped character of the full `NotoSansCJKtc-VF.otf` (65,535 glyphs, 44,798
  compared), Source Serif 4 Variable Roman (6 font dicts, 8 regions; 906 compared) and Adobe's
  variable font prototype (548 local subrs, 5 regions; 247 compared). None differed, and no glyph
  of the three was refused. The repository keeps a 176-glyph subset of the Noto font and
  `tools/cff2-outlines.sh`, which records HarfBuzz's outlines for `cff_tests.rs`.
- **Hostile fonts and fuzz**: `cff_tests.rs` builds CFF2 fonts with one defect each (32-bit
  counts past the table, a cut table, a Top DICT past it, `blend` outside a Private DICT or
  short of operands, bad `vsindex`, a damaged store, FDSelect gaps and huge range counts,
  subroutine bombs and recursion without `return`) and mutates the subset's CFF2 table.

## CFF2: other instances (#51)

Added 2026-10-04. `--font FILE#wght=700` (or `FILE.ttc#1#wght=700,wdth=90`) picks an instance.
`src/variations.rs` turns the settings into normalized coordinates and `cff.rs` blends at them.
HarfBuzz is the reference throughout, so each step is done as `hb_font_set_variations` does it:

- **Normalizing**: in f32, each setting clamped to its axis's range (widened to hold the default,
  as HarfBuzz widens it), scaled to -1..1 on its side of the default, and rounded to 16.16 with HarfBuzz's `roundf`,
  which is `floorf(v + 0.5f)`: a half rounds up, so -2.5 to -2 where `f32::round` gives -3.
  Then avar's segment map for the axis is applied, ported from `SegmentMaps::map_float`, with
  its answers for maps that OpenType calls malformed: none or one pair, repeated or unsorted
  `from` values, and the skipped (-1, -1) and (1, 1) ends. Last, it is rounded to 2.14 with
  `(c + 2) >> 2`. `variations_tests.rs` has an axis for each case, 29 in one font: hb-vector,
  `hb_font_get_var_coords_normalized` and termshot give the same 29 coordinates. Four cases,
  two found by search and two at a half, are there because their f32 details are each
  1/16384 off if missed. A
  setting sets every axis with its tag, as `hb_font_set_variations` does, should fvar repeat
  one (hb-vector agrees).
- **Coordinates of 0 are the default**: HarfBuzz draws a font whose coordinates are all 0
  as it draws one with none, blending nothing, even where a region that peaks at 0 would
  count 1 there. So does `parse_cff2`, and a setting at the default draws the default's
  pixels (`test.sh` checks `wght=100`).
- **What is refused**: fvar is checked as HarfBuzz checks it (version 1, 20-byte axis records,
  instance records of at least 4 × axes + 4 bytes, arrays inside the table). Where HarfBuzz
  would quietly read a bad fvar as no axes, termshot refuses the font, as it does an avar of
  another version (avar 2 is not read) or of a different axis count, and a variation store
  whose region list has another axis count than fvar, at an instance (HarfBuzz reads a
  missing axis as 0 and drops an extra one). An axis the font lacks
  is refused with the axes it has, where HarfBuzz ignores the setting, and a TrueType or CFF
  face with that reason: `glyf` variations (gvar) are not read.
- **Metrics** ([#77](https://github.com/momiji-rs/termshot/issues/77)): `src/metrics.rs`
  varies each glyph's advance by HVAR, as `hb_font_get_glyph_h_advance` does: hmtx's advance
  plus the delta, the delta summed in f32 in row order and rounded with `roundf`, and the sum
  at least 0, only when some coordinate is not 0. It varies hhea's ascender, descender and
  line gap by MVAR's `hasc`, `hdsc` and `hlgp` at any instance chosen, as
  `hb_font_get_h_extents` does: the ascender made positive and the descender negative. So the
  cell size, the centering of wide and fallback glyphs, and the baseline follow the instance:
  at wght=900 Noto Sans CJK VF advances `M` 877 units, not 770, and the cell widens with it
  (`test.sh` checks it). An ideograph advances 1000 at every weight. VVAR is not read, as
  termshot lays text out horizontally only. Out of range indexes read as no delta, as in
  HarfBuzz (fonts rely on it: an identity advance map past the store's items); everything else
  HarfBuzz would ignore a table for, from a format to unsorted MVAR records, refuses the font
  at an instance with a reason. draw.c gets the advances through a callback in the Face.
  Checked against HarfBuzz 14.4.0 (`tools/cff2-metrics.py`, through libharfbuzz) on starship:
  every advance and the extents of the subset at four instances and of a crafted font at
  seven (`metrics_tests.rs`; deltas of each width, a null ItemVariationData, an advance map
  with entries past the store), and of the full Noto font, Source Serif 4 and Adobe's
  prototype (both with an advance map) at two instances each, through the ignored
  `any_cff2_font_s_metrics_match_harfbuzz`. None of those three varies `hasc`, `hdsc` or
  `hlgp`, so only the crafted font tests MVAR.
- **Cost**: the coordinates are worked out once, when the font loads, and the region scalars
  once per font, when the store is read. A `blend` at an instance costs its k multiply-adds
  per value; at the default, where every scalar is 0, it costs nothing extra.
- **Checked against HarfBuzz 14.4.0** (`hb-vector`), outline for outline, on starship. The
  repository keeps the subset at wght=350.5, 700 and 900 (`tests/fixtures/cff2-outlines-*.txt`).
  The ignored `any_cff2_font_matches_harfbuzz` checks any font at any instance; with it, the
  full `NotoSansCJKtc-VF.otf` at wght=700 (44,801 compared), Source Serif 4 Variable Roman at
  wght=650,opsz=12 (two axes and an avar; 906 compared) and Adobe's prototype at
  wght=700,CNTR=60 (248 compared) matched. `tools/cff2-outlines.sh` draws by glyph id, since
  shaping a character can substitute another glyph at an instance (the prototype's dollar sign
  does, through GSUB FeatureVariations) or move a combining mark (Noto's U+302E).
