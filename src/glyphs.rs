//! The text: each cell's glyph from the font or else the fallback, cached,
//! slanted for italic and doubled for bold, the box of a character neither
//! font has, combining marks over their cells, and underlines and
//! strike-through, painted into the render's canvas over the backdrop
//! (src/composite.rs), which it paints a row of cells ahead of the text.
//! src/render.rs calls this once per render (termshot_paint_text); box
//! drawing and blocks go to src/geometry.rs from here.
//!
//! stb_truetype stays C (#12 step 3), so src/stb_glue.c hands this module the stb
//! functions it may call, in TextFonts: the cmap and metrics lookups and the
//! rasterizer for any face, and, for a TrueType face only, the four that
//! read glyph outlines (GlyphFace). stb_glue.c leaves those null on a CFF or
//! CFF2 face, whose charstrings stb must never run: its outlines come from
//! src/cff.rs through the Face's callback, as they did to the C, and go to
//! stbtt_Rasterize. Source::new refuses a face that has both or neither, and
//! only Outlines::TrueType holds the stb readers, so no path here can call
//! them on a CFF face.
//!
//! This was C in draw.c (find_glyph, slant_outline, paint_marks, blend and
//! the glyph and line passes of draw_png_images) until #12 step 2c, and it
//! paints the same pixels:
//!
//! - The float arithmetic is the C's, operation for operation, in f32 where
//!   the C used float and in the same order, with no mul_add (Rust never
//!   fuses; stb_glue.c is built with -ffp-contract=off, as draw.c was).
//! - The C's floorf, ceilf and lroundf are exact (lroundf only on values
//!   clamped to i16 first), so f32::floor, ceil and round give what they
//!   do, however a compiler emits them. No rounded libm function is called:
//!   the slant is the constant tan(12 degrees), as in the C.
//! - C's (int) of a float truncates, and is undefined out of range; `as`
//!   truncates and saturates. Every cast is a pixel coordinate or a vertex
//!   coordinate already clamped, so the two agree on every input that
//!   occurs.
//! - The blend is composite.rs's blend_pixel, exact integer arithmetic.
//!
//! Memory: the cache's slots, the CFF outline scratch and each glyph's
//! bitmap are allocated with try_reserve_exact; when one can't be, the
//! render fails with exit 2 ("glyph allocation failed"), as the C's malloc
//! and realloc did, and nothing aborts. A panic is caught and fails the
//! render too.

use std::ffi::{c_int, c_void};
use std::ptr;

use crate::cell::{Cell, CellMarks, BOLD, DOUBLE_UNDERLINE, ITALIC, MAX_MARKS, STRIKE, UNDERLINE, WIDE};
use crate::cff::{Vertex, CUBIC};
use crate::composite::{self, blend_pixel, Backdrop};
use crate::geometry::{self, guarded, Canvas, Rgb};

/// Bytes per pixel: the canvas is RGB, as BPP in src/render.rs.
const BPP: usize = 3;

/// stb_truetype's quadratic curve vertex (STBTT_vcurve); cff.rs makes none.
const CURVE: u8 = 3;

/// Slots of the glyph cache, per render. Bold and colours reuse the same
/// coverage bitmap. 1024 slots avoid thrashing on mixed Unicode screens (800
/// distinct code points in the benchmark), while keeping the slots to 64 KiB
/// on 64-bit builds.
pub const GLYPH_CACHE_SIZE: usize = 1024;

/// Italic is synthetic: the outline is slanted by 12 degrees before it is
/// rasterized, so its edges are as smooth as upright ones. tan(12 degrees).
const ITALIC_SLANT: f32 = 0.21256;

/// The cells drawn as a box because a font maps the character to an empty
/// glyph, as color bitmap fonts do: how many, and the first one, its
/// character and which fonts did (EMPTY_IN_*). main.rs says so; painting
/// stays quiet.
#[repr(C)]
#[derive(Default)]
pub struct EmptyGlyphs {
    pub cp: u32,
    pub fonts: u32,
    pub col: i32,
    pub row: i32,
    pub cells: usize,
}

pub const EMPTY_IN_FONT: u32 = 1;
pub const EMPTY_IN_FALLBACK: u32 = 2;

/// stbtt_fontinfo, which only C reads.
#[repr(C)]
pub struct FontInfo {
    _private: [u8; 0],
}

/// Face.outline: the outline of a glyph into out[..capacity] as
/// stbtt_GetGlyphShape gives it, and its box as stbtt_GetGlyphBox does; the
/// vertex count, over capacity when it needs more room (src/font.rs).
pub type OutlineFn = unsafe extern "C" fn(*const c_void, c_int, *mut Vertex, c_int, *mut c_int) -> c_int;

/// Face.advance: a glyph's advance at the instance from its hmtx advance
/// (src/font.rs).
pub type AdvanceFn = unsafe extern "C" fn(*const c_void, c_int, c_int) -> c_int;

/// stb_glue.c's Face, as src/font.rs makes it: a checked font, for a CFF face
/// the callback that gives its outlines, and at an instance of a variable
/// face the one that gives its advances (HVAR) and its vertical metrics
/// (MVAR). Read here only for the two callbacks; stb_glue.c scales each
/// face by its vertical metrics.
#[repr(C)]
pub struct Face {
    pub ttf: *const u8,
    pub start: c_int,
    pub outline: Option<OutlineFn>,
    pub cff: *const c_void,
    pub advance: Option<AdvanceFn>,
    pub advances: *const c_void,
    pub varied: c_int,
    pub ascent: c_int,
    pub descent: c_int,
    pub line_gap: c_int,
}

const _: () = assert!(std::mem::size_of::<Face>() == 64);

/// stbtt__bitmap, what stbtt_Rasterize paints.
#[repr(C)]
pub struct StbBitmap {
    pub w: c_int,
    pub h: c_int,
    pub stride: c_int,
    pub pixels: *mut u8,
}

pub type IsGlyphEmptyFn = unsafe extern "C" fn(*const FontInfo, c_int) -> c_int;
pub type GetGlyphShapeFn = unsafe extern "C" fn(*const FontInfo, c_int, *mut *mut Vertex) -> c_int;
pub type GetGlyphBitmapBoxFn =
    unsafe extern "C" fn(*const FontInfo, c_int, f32, f32, *mut c_int, *mut c_int, *mut c_int, *mut c_int);
pub type MakeGlyphBitmapFn = unsafe extern "C" fn(*const FontInfo, *mut u8, c_int, c_int, c_int, f32, f32, c_int);
pub type FindGlyphIndexFn = unsafe extern "C" fn(*const FontInfo, c_int) -> c_int;
pub type GetGlyphHMetricsFn = unsafe extern "C" fn(*const FontInfo, c_int, *mut c_int, *mut c_int);
pub type RasterizeFn = unsafe extern "C" fn(*mut StbBitmap, f32, *mut Vertex, c_int, f32, f32, f32, f32, c_int, c_int,
                                            c_int, *mut c_void);
pub type FreeShapeFn = unsafe extern "C" fn(*const FontInfo, *mut Vertex);

/// One face to draw from, as stb_glue.c fills it in (GlyphFace there): stb's
/// font, the Face it was made from, and its scale. The four stb functions
/// that read glyph outlines are set for a TrueType face and null for a CFF
/// or CFF2 one. `info` is null for no face (no fallback).
#[repr(C)]
pub struct GlyphFace {
    pub info: *const FontInfo,
    pub face: *const Face,
    pub scale: f32,
    pub is_glyph_empty: Option<IsGlyphEmptyFn>,
    pub get_glyph_shape: Option<GetGlyphShapeFn>,
    pub get_glyph_bitmap_box: Option<GetGlyphBitmapBoxFn>,
    pub make_glyph_bitmap: Option<MakeGlyphBitmapFn>,
}

const _: () = assert!(std::mem::size_of::<GlyphFace>() == 56);

/// The fonts of a render, as stb_glue.c's TextFonts: the font and the fallback
/// (whose `info` is null when there is none), the stb functions any face
/// may be asked, the italic pivot (the middle of the body, in pixels above
/// the baseline, for both fonts) and the baseline in the cell.
#[repr(C)]
pub struct TextFonts {
    pub font: GlyphFace,
    pub fallback: GlyphFace,
    pub find_glyph_index: Option<FindGlyphIndexFn>,
    pub get_glyph_h_metrics: Option<GetGlyphHMetricsFn>,
    pub rasterize: Option<RasterizeFn>,
    pub free_shape: Option<FreeShapeFn>,
    pub italic_pivot: f32,
    pub baseline: i32,
}

const _: () = assert!(std::mem::size_of::<TextFonts>() == 152);

/// What the text cost and did, for TERMSHOT_PROFILE (src/render.rs):
/// the time spent painting the backdrop, box drawing, finding glyphs and
/// blending them (with the boxes of missing ones), and the cache's counts.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct TextStats {
    pub backdrop_ms: f64,
    pub geometry_ms: f64,
    pub glyph_ms: f64,
    pub blend_ms: f64,
    /// Glyphs rasterized, and those from the fallback.
    pub glyphs: usize,
    pub cache_hits: usize,
    pub evictions: usize,
    /// Cells (not marks) whose character no font has.
    pub missing: usize,
    pub fallback_lookups: usize,
    pub fallback_glyphs: usize,
}

const _: () = assert!(std::mem::size_of::<TextStats>() == 80);

/// termshot_paint_text's results other than 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Failure {
    /// A glyph allocation failed.
    OutOfMemory = 1,
    /// The box painter panicked (a bug).
    BoxDrawing = 2,
    /// Painting the text panicked (a bug).
    Panicked = 3,
}

/// Paints the text of a render: the glyphs of `bd`'s cells (box drawing and
/// blocks through src/geometry.rs), their combining marks (`marks`,
/// `mark_count` long and sorted by cell, or null), the backdrop under them
/// as they reach each row, all of it by the end, then underlines and
/// strike-through. `empty`, if not null, counts the cells drawn as a box
/// for an empty glyph, as EmptyGlyphs says; the render has cleared it.
/// `profiling` times the stages into `stats`, which gets the counts too.
///
/// Returns 0; 1 when a glyph allocation failed, 2 if box drawing panicked
/// and 3 if anything else here did (Failure), with the render left
/// unfinished.
///
/// # Safety
/// `cv` and `bd` as for termshot_backdrop_through (src/composite.rs), and
/// nothing else may use them during the call; `marks` null or `mark_count`
/// entries; `fonts` filled in by stb_glue.c, its faces live and their stb
/// functions stb's; `stats` writable.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn termshot_paint_text(cv: *const Canvas, bd: *mut Backdrop, marks: *const CellMarks,
                                             mark_count: usize, fonts: *const TextFonts, profiling: c_int,
                                             empty: *mut EmptyGlyphs, stats: *mut TextStats) -> c_int {
    let (cv, bd, fonts) = (&*cv, &mut *bd, &*fonts);
    let marks = if marks.is_null() || mark_count == 0 { &[] } else { std::slice::from_raw_parts(marks, mark_count) };
    let mut out = TextStats::default();
    let result = guarded(|| paint_text(cv, bd, marks, fonts, profiling != 0, empty.as_mut(), &mut out));
    *stats = out;
    match result {
        Some(Ok(())) => 0,
        Some(Err(failure)) => failure as c_int,
        None => Failure::Panicked as c_int,
    }
}

/// The time in ms while profiling, from the clock stb_glue.c's now_ms reads
/// (CLOCK_MONOTONIC), so the stages it times here, the spans src/render.rs
/// subtracts them from and the PNG writer's marks agree; 0 when not, which reads no clock. A clock
/// read per stage per cell is the profile's own cost, and the same as the
/// C's: with std's Instant, the profiled foreground of a screen of cache
/// hits read up to 15% over the C's (cjk-dense 0.34 → 0.39 ms) for the
/// same wall time.
pub(crate) struct Clock(pub(crate) bool);

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn monotonic_ms() -> f64 {
    #[repr(C)]
    struct Timespec {
        tv_sec: i64,
        tv_nsec: std::ffi::c_long,
    }
    extern "C" {
        fn clock_gettime(clock: c_int, ts: *mut Timespec) -> c_int;
    }
    #[cfg(target_os = "macos")]
    const CLOCK_MONOTONIC: c_int = 6;
    #[cfg(target_os = "linux")]
    const CLOCK_MONOTONIC: c_int = 1;
    let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: ts is writable; the call can't fail for this clock.
    unsafe { clock_gettime(CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1_000_000.0
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn monotonic_ms() -> f64 {
    thread_local!(static START: std::time::Instant = std::time::Instant::now());
    START.with(|start| start.elapsed().as_secs_f64() * 1000.0)
}

impl Clock {
    pub(crate) fn now(&self) -> f64 {
        if self.0 {
            monotonic_ms()
        } else {
            0.0
        }
    }
}

/// The canvas and the backdrop under the text, which the text paints a row
/// of cells ahead of itself, timing it.
struct Ground<'a> {
    cv: &'a Canvas,
    bd: &'a mut Backdrop,
    clock: Clock,
    backdrop_ms: f64,
}

impl Ground<'_> {
    /// Paint the backdrop of every row of cells above pixel row y. A failure
    /// is reported by termshot_paint_failed, which the render asks before it
    /// writes the PNG.
    ///
    /// # Safety
    /// As termshot_paint_text says of the canvas and the backdrop.
    unsafe fn through(&mut self, y: i64) {
        if !composite::due(self.bd, y) {
            return;
        }
        let tick = self.clock.now();
        composite::termshot_backdrop_through(self.cv, self.bd, y);
        self.backdrop_ms += self.clock.now() - tick;
    }
}

/// termshot_paint_text's painting.
///
/// # Safety
/// As termshot_paint_text.
unsafe fn paint_text(cv: &Canvas, bd: &mut Backdrop, marks: &[CellMarks], fonts: &TextFonts, profiling: bool,
                     mut empty: Option<&mut EmptyGlyphs>, stats: &mut TextStats) -> Result<(), Failure> {
    faults::start();
    let (cols, rows, cell_w, cell_h) = (bd.cols, bd.rows, bd.cell_w, bd.cell_h);
    let cells: &[Cell] = if bd.cells.is_null() || cols <= 0 || rows <= 0 {
        &[]
    } else {
        std::slice::from_raw_parts(bd.cells, cols as usize * rows as usize)
    };
    let baseline = fonts.baseline;
    let mut ground = Ground { cv, bd, clock: Clock(profiling), backdrop_ms: 0.0 };
    let mut g = Glyphs::new(fonts)?;
    let (mut geometry_ms, mut glyph_ms, mut blend_ms) = (0.0, 0.0, 0.0);
    let mut next_mark = 0;
    let result = (|| {
        for r in 0..rows.max(0) {
            ground.through((r as i64 + 1) * cell_h as i64);
            for c in 0..cols {
                let index = r as usize * cols as usize + c as usize;
                let cell = &cells[index];
                let cp = cell.ch;
                // The cell's marks, if any; the list is sorted by cell.
                while next_mark < marks.len() && (marks[next_mark].cell as usize) < index {
                    next_mark += 1;
                }
                let cell_marks = marks.get(next_mark).filter(|m| m.cell as usize == index);
                if (cp == 0 || cp == ' ' as u32) && cell_marks.is_none() {
                    continue;
                }
                let wide = cell.attrs & WIDE != 0;
                let span = if wide { 2 * cell_w } else { cell_w };
                let fg = [cell.fr, cell.fg, cell.fb];
                let bold = cell.attrs & BOLD != 0;
                // Where the character starts and how far it advances, for
                // its marks: its cells, unless a glyph says otherwise.
                let mut base_x = c * cell_w;
                let mut base_advance = span as f32;
                // A Hangul filler draws nothing, though its marks still do.
                if cp != 0 && cp != ' ' as u32 && !is_ignorable(cp) {
                    let tick = ground.clock.now();
                    // Box drawing and blocks, painted as geometry.
                    let geometry = if (0x2500..=0x259f).contains(&cp) {
                        geometry::paint_cell(cv, c, r, cell_w, cell_h, cp, bold, fg)
                    } else {
                        Some(false)
                    };
                    geometry_ms += ground.clock.now() - tick;
                    let Some(geometry) = geometry else { return Err(Failure::BoxDrawing) };
                    if !geometry {
                        let tick = ground.clock.now();
                        let slot = g.find(cp, wide, cell.attrs & ITALIC != 0, false, span)?;
                        glyph_ms += ground.clock.now() - tick;
                        let tick = ground.clock.now();
                        let ahead = ground.backdrop_ms;
                        let entry = &g.cache[slot];
                        if entry.missing && !is_blank(cp) {
                            if let (true, Some(e)) = (entry.empty != 0, empty.as_deref_mut()) {
                                if e.cells == 0 {
                                    *e = EmptyGlyphs { cp, fonts: entry.empty, col: c, row: r, cells: 1 };
                                } else {
                                    e.cells += 1;
                                }
                            }
                            paint_tofu(cv, c * cell_w, r * cell_h, span, cell_w, cell_h, fg);
                        } else if !entry.bitmap.is_empty() {
                            let dx = c * cell_w + entry.shift + entry.ix0;
                            let dy = r * cell_h + baseline + entry.iy0;
                            ground.through(dy as i64 + entry.h as i64);
                            blend(cv, dx, dy, &entry.bitmap, entry.w, entry.h, fg);
                            if bold {
                                blend(cv, dx + 1, dy, &entry.bitmap, entry.w, entry.h, fg);
                            }
                        }
                        if !entry.missing {
                            base_x += entry.shift;
                            base_advance = entry.advance;
                        }
                        blend_ms += ground.clock.now() - tick - (ground.backdrop_ms - ahead);
                    }
                }
                if let Some(cell_marks) = cell_marks {
                    let y = r * cell_h + baseline;
                    paint_marks(&mut ground, &mut g, cell_marks, cell, base_x, base_advance, y, &mut glyph_ms,
                                &mut blend_ms)?;
                }
            }
        }
        Ok(())
    })();
    stats.geometry_ms = geometry_ms;
    stats.glyph_ms = glyph_ms;
    stats.blend_ms = blend_ms;
    (stats.glyphs, stats.cache_hits, stats.evictions) = (g.glyphs, g.cache_hits, g.evictions);
    (stats.missing, stats.fallback_lookups, stats.fallback_glyphs) = (g.missing, g.fallback_lookups, g.fallback_glyphs);
    result?;
    drop(g);

    ground.through(rows as i64 * cell_h as i64);
    stats.backdrop_ms = ground.backdrop_ms;
    paint_lines(cv, cells, cols, rows, cell_w, cell_h, baseline);
    Ok(())
}

/// Underlines and strike-through, over the glyphs, as thick as box-drawing
/// strokes and kept inside the cell.
///
/// # Safety
/// As termshot_paint_text says of the canvas; `cells` is cols x rows.
unsafe fn paint_lines(cv: &Canvas, cells: &[Cell], cols: i32, rows: i32, cell_w: i32, cell_h: i32, baseline: i32) {
    let line_t = if cell_w / 12 < 1 { 1 } else { cell_w / 12 };
    let mut under_y = baseline + (cell_h - baseline) / 3;
    if under_y > cell_h - line_t {
        under_y = cell_h - line_t;
    }
    let mut double_y = if under_y + 3 * line_t <= cell_h { under_y } else { cell_h - 3 * line_t };
    if double_y < 0 {
        double_y = 0;
    }
    let strike_y = baseline - baseline * 3 / 10;
    for r in 0..rows.max(0) {
        for c in 0..cols {
            let cell = &cells[r as usize * cols as usize + c as usize];
            if cell.attrs & (UNDERLINE | DOUBLE_UNDERLINE | STRIKE) == 0 {
                continue;
            }
            let fg = [cell.fr, cell.fg, cell.fb];
            let (x0, y) = (c * cell_w, r * cell_h);
            let x1 = x0 + cell_w;
            if cell.attrs & DOUBLE_UNDERLINE != 0 {
                geometry::fill_rect(cv, x0, y + double_y, x1, y + double_y + line_t, fg);
                geometry::fill_rect(cv, x0, y + double_y + 2 * line_t, x1, y + double_y + 3 * line_t, fg);
            } else if cell.attrs & UNDERLINE != 0 {
                geometry::fill_rect(cv, x0, y + under_y, x1, y + under_y + line_t, fg);
            }
            if cell.attrs & STRIKE != 0 {
                geometry::fill_rect(cv, x0, y + strike_y, x1, y + strike_y + line_t, fg);
            }
        }
    }
}

/// Draw a cell's combining marks over its character, in the cell's colours
/// and attributes, from the font or else the fallback; a mark neither has is
/// left out. There is no shaping (no GPOS anchors), so a mark goes where its
/// own outline puts it. Most fonts draw a mark to the left of its origin,
/// with no advance, to overlay the character before it: that mark is drawn
/// from where the character ends (base_x plus its advance). A mark drawn to
/// the right of its origin, as in right-to-left fonts, is centered over the
/// character instead. y is the baseline. Adds the time spent finding glyphs
/// and blending them to glyph_ms and blend_ms.
///
/// # Safety
/// As termshot_paint_text.
#[allow(clippy::too_many_arguments)]
unsafe fn paint_marks(ground: &mut Ground, g: &mut Glyphs, marks: &CellMarks, cell: &Cell, base_x: i32,
                      base_advance: f32, y: i32, glyph_ms: &mut f64, blend_ms: &mut f64) -> Result<(), Failure> {
    let italic = cell.attrs & ITALIC != 0;
    let fg = [cell.fr, cell.fg, cell.fb];
    for &cp in marks.marks.iter().take(MAX_MARKS).take_while(|&&m| m != 0) {
        if is_ignorable(cp) {
            continue;
        }
        let tick = ground.clock.now();
        let slot = g.find(cp, false, italic, true, 0);
        *glyph_ms += ground.clock.now() - tick;
        let mark = &g.cache[slot?];
        if mark.bitmap.is_empty() {
            continue;
        }
        ground.through(y as i64 + mark.iy0 as i64 + mark.h as i64);
        let tick = ground.clock.now();
        let dx = if 2 * mark.ix0 + mark.w < 0 {
            base_x + (base_advance + 0.5).floor() as i32 + mark.ix0
        } else {
            base_x + ((base_advance - mark.w as f32) / 2.0 + 0.5).floor() as i32
        };
        let dy = y + mark.iy0;
        blend(ground.cv, dx, dy, &mark.bitmap, mark.w, mark.h, fg);
        if cell.attrs & BOLD != 0 {
            blend(ground.cv, dx + 1, dy, &mark.bitmap, mark.w, mark.h, fg);
        }
        *blend_ms += ground.clock.now() - tick;
    }
    Ok(())
}

/// Characters that draw nothing by design, so blank even when no font has
/// them: Unicode's space separators (Zs), the line and paragraph separators,
/// and the blank Braille pattern that TUIs use as an empty dot graph.
pub fn is_blank(cp: u32) -> bool {
    matches!(cp, 0x20 | 0xa0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x2800 | 0x3000)
}

/// Default_Ignorable_Code_Point, past U+00AD: joiners, direction marks,
/// variation selectors, fillers and tags, which draw nothing even when a font
/// has a glyph for them. Most are zero width, and so marks; the Hangul
/// fillers U+115F, U+3164 and U+FFA0 take cells of their own.
pub fn is_ignorable(cp: u32) -> bool {
    matches!(cp, 0x034f | 0x061c | 0x115f..=0x1160 | 0x17b4..=0x17b5 | 0x180b..=0x180f | 0x200b..=0x200f
        | 0x202a..=0x202e | 0x2060..=0x206f | 0x3164 | 0xfe00..=0xfe0f | 0xfeff | 0xffa0 | 0xfff0..=0xfff8
        | 0x1bca0..=0x1bca3 | 0x1d173..=0x1d17a | 0xe0000..=0xe0fff)
}

/// An outlined box for a character neither font has, inset in its cells.
///
/// # Safety
/// As termshot_fill_rect.
unsafe fn paint_tofu(cv: &Canvas, x: i32, y: i32, span: i32, cell_w: i32, cell_h: i32, c: Rgb) {
    let t = if cell_w / 12 < 1 { 1 } else { cell_w / 12 };
    let (x0, x1) = (x + cell_w / 6, x + span - cell_w / 6);
    let (y0, y1) = (y + cell_h / 6, y + cell_h - cell_h / 6);
    if x1 - x0 <= 2 * t || y1 - y0 <= 2 * t {
        geometry::fill_rect(cv, x0, y0, x1, y1, c);
        return;
    }
    geometry::fill_rect(cv, x0, y0, x1, y0 + t, c);
    geometry::fill_rect(cv, x0, y1 - t, x1, y1, c);
    geometry::fill_rect(cv, x0, y0 + t, x0 + t, y1 - t, c);
    geometry::fill_rect(cv, x1 - t, y0 + t, x1, y1 - t, c);
}

/// Blend a glyph's coverage, gw x gh, in c with its top left at (dx, dy),
/// clipped to the canvas.
///
/// # Safety
/// As termshot_paint_text says of the canvas.
unsafe fn blend(cv: &Canvas, dx: i32, dy: i32, bm: &[u8], gw: i32, gh: i32, c: Rgb) {
    let x0 = if dx < 0 { -dx } else { 0 };
    let y0 = if dy < 0 { -dy } else { 0 };
    let x1 = gw.min(cv.w - dx);
    let y1 = gh.min(cv.h - dy);
    if x0 >= x1 || y0 >= y1 || cv.px.is_null() {
        return;
    }
    let n = (x1 - x0) as usize;
    for y in y0..y1 {
        // One check per row; the pixels then go through pointers, as the
        // C's did (a bounds check per pixel cost the geometry up to 25%).
        let from = (y * gw + x0) as usize;
        let row = &bm[from..from + n];
        // 0 <= dx + x0 and dx + x1 <= w, 0 <= dy + y < h: on the canvas.
        let mut d = cv.px.add((dy + y) as usize * cv.stride + (dx + x0) as usize * BPP);
        for &a in row {
            blend_pixel(d, c, a as u32);
            d = d.add(BPP);
        }
    }
}

/// A cached glyph. wide, italic and mark are part of the key: a wide glyph
/// is centered over two cells, an italic one is slanted, and a combining
/// mark is neither centered nor shrunk, as the font places it. shift moves
/// the glyph right within its cell (or cells), and advance is how far the
/// glyph, so drawn, moves the pen, in pixels; missing means neither font has
/// it, and empty which fonts (EMPTY_IN_*) map it to an empty glyph. The
/// bitmap is w x h coverage, empty when there is nothing to draw.
#[derive(Default)]
struct Glyph {
    cp: u32,
    valid: bool,
    wide: bool,
    italic: bool,
    mark: bool,
    missing: bool,
    empty: u32,
    shift: i32,
    ix0: i32,
    iy0: i32,
    w: i32,
    h: i32,
    advance: f32,
    bitmap: Vec<u8>,
}

/// Where a face's outlines come from.
#[derive(Clone, Copy)]
enum Outlines {
    /// stb reads them: a TrueType face, and the only one stb's outline
    /// readers are held for.
    TrueType {
        is_glyph_empty: IsGlyphEmptyFn,
        get_glyph_shape: GetGlyphShapeFn,
        get_glyph_bitmap_box: GetGlyphBitmapBoxFn,
        make_glyph_bitmap: MakeGlyphBitmapFn,
    },
    /// src/cff.rs runs a CFF or CFF2 face's charstrings, through its Face.
    Cff { outline: OutlineFn, cff: *const c_void },
}

/// A face to draw from.
struct Source {
    info: *const FontInfo,
    face: *const Face,
    scale: f32,
    outlines: Outlines,
}

impl Source {
    /// The face stb_glue.c filled in; None for none. Panics (failing the render)
    /// for a CFF face given stb's outline readers, or a TrueType one
    /// without them.
    ///
    /// # Safety
    /// `f.face` must be live when `f.info` is not null.
    unsafe fn new(f: &GlyphFace) -> Option<Source> {
        if f.info.is_null() {
            return None;
        }
        let face = &*f.face;
        let readers = (f.is_glyph_empty, f.get_glyph_shape, f.get_glyph_bitmap_box, f.make_glyph_bitmap);
        let outlines = match (face.outline, readers) {
            (Some(outline), (None, None, None, None)) => Outlines::Cff { outline, cff: face.cff },
            (None, (Some(is_glyph_empty), Some(get_glyph_shape), Some(get_glyph_bitmap_box), Some(make_glyph_bitmap))) => {
                Outlines::TrueType { is_glyph_empty, get_glyph_shape, get_glyph_bitmap_box, make_glyph_bitmap }
            }
            (Some(_), _) => panic!("a CFF face must not reach stb's outline readers"),
            (None, _) => panic!("a TrueType face needs stb's outline readers"),
        };
        Some(Source { info: f.info, face: f.face, scale: f.scale, outlines })
    }
}

/// The outline last fetched from a CFF face, so the check for an empty glyph
/// and the drawing share one run of its charstring. `v` is its capacity,
/// all of it handed to the callback; `n` vertices hold the outline.
struct Outline {
    v: Vec<Vertex>,
    n: usize,
    /// Whose glyph v holds, or null.
    face: *const Face,
    glyph: c_int,
    bbox: [c_int; 4],
}

impl Outline {
    /// Fetch `glyph` of a CFF face, unless held already. The vertex count,
    /// or None when memory runs out. A glyph with more vertices than there
    /// is room for runs its charstring twice, so the room starts at most
    /// glyphs' and doubles.
    ///
    /// # Safety
    /// `outline` and `cff` must be the live Face's.
    unsafe fn fetch(&mut self, outline: OutlineFn, cff: *const c_void, face: *const Face, glyph: c_int)
                    -> Option<usize> {
        if self.face == face && self.glyph == glyph {
            return Some(self.n);
        }
        self.face = ptr::null();
        let mut want = if self.v.is_empty() { 512 } else { 0 };
        loop {
            if want > self.v.len() && !grow(Site::Outline, &mut self.v, want) {
                return None;
            }
            let capacity = self.v.len();
            let n = outline(cff, glyph, self.v.as_mut_ptr(), capacity as c_int, self.bbox.as_mut_ptr());
            if n <= capacity as c_int {
                self.n = n.max(0) as usize;
                self.face = face;
                self.glyph = glyph;
                return Some(self.n);
            }
            let n = n as usize;
            want = if capacity > i32::MAX as usize / 2 || n > 2 * capacity { n } else { 2 * capacity };
        }
    }
}

/// What finding a glyph needs: both fonts, the cache and outline scratch,
/// and the profile's counters.
struct Glyphs {
    font: Source,
    fallback: Option<Source>,
    find_glyph_index: FindGlyphIndexFn,
    get_glyph_h_metrics: GetGlyphHMetricsFn,
    rasterize: RasterizeFn,
    free_shape: FreeShapeFn,
    italic_pivot: f32,
    cache: Vec<Glyph>,
    scratch: Outline,
    glyphs: usize,
    cache_hits: usize,
    evictions: usize,
    missing: usize,
    fallback_lookups: usize,
    fallback_glyphs: usize,
}

/// stb's outline of a glyph, freed with it.
struct Shape {
    v: *mut Vertex,
    info: *const FontInfo,
    free_shape: FreeShapeFn,
}

impl Drop for Shape {
    fn drop(&mut self) {
        if !self.v.is_null() {
            // SAFETY: v came from stbtt_GetGlyphShape of info.
            unsafe { (self.free_shape)(self.info, self.v) };
        }
    }
}

impl Glyphs {
    /// # Safety
    /// As termshot_paint_text says of `fonts`.
    unsafe fn new(fonts: &TextFonts) -> Result<Glyphs, Failure> {
        let font = Source::new(&fonts.font).expect("a render has a font");
        let fallback = Source::new(&fonts.fallback);
        let mut cache = Vec::new();
        if !allowed(Site::Cache) || cache.try_reserve_exact(GLYPH_CACHE_SIZE).is_err() {
            return Err(Failure::OutOfMemory);
        }
        cache.resize_with(GLYPH_CACHE_SIZE, Glyph::default);
        Ok(Glyphs {
            font,
            fallback,
            find_glyph_index: fonts.find_glyph_index.expect("stbtt_FindGlyphIndex"),
            get_glyph_h_metrics: fonts.get_glyph_h_metrics.expect("stbtt_GetGlyphHMetrics"),
            rasterize: fonts.rasterize.expect("stbtt_Rasterize"),
            free_shape: fonts.free_shape.expect("stbtt_FreeShape"),
            italic_pivot: fonts.italic_pivot,
            cache,
            scratch: Outline { v: Vec::new(), n: 0, face: ptr::null(), glyph: 0, bbox: [0; 4] },
            glyphs: 0,
            cache_hits: 0,
            evictions: 0,
            missing: 0,
            fallback_lookups: 0,
            fallback_glyphs: 0,
        })
    }

    /// stbtt_IsGlyphEmpty, asked of the face's own outlines for CFF: 1 for
    /// an empty glyph, 0 not, and -1 when memory runs out.
    ///
    /// # Safety
    /// As Glyphs::new.
    unsafe fn glyph_empty(source: &Source, scratch: &mut Outline, glyph: c_int) -> c_int {
        match source.outlines {
            Outlines::TrueType { is_glyph_empty, .. } => is_glyph_empty(source.info, glyph),
            Outlines::Cff { outline, cff } => match scratch.fetch(outline, cff, source.face, glyph) {
                None => -1,
                Some(n) => (n == 0) as c_int,
            },
        }
    }

    /// The slot of cp's cached glyph, rasterized on a miss, from the font or
    /// else the fallback; span is the width of its cells in pixels. A mark is
    /// placed as its font places it, never centered or shrunk. Err when
    /// memory runs out.
    ///
    /// # Safety
    /// As Glyphs::new.
    #[inline]
    unsafe fn find(&mut self, cp: u32, wide: bool, italic: bool, mark: bool, span: i32) -> Result<usize, Failure> {
        // An italic glyph has its own slot, so mixed text doesn't evict.
        let slot = ((cp ^ if italic { GLYPH_CACHE_SIZE as u32 / 2 } else { 0 }) % GLYPH_CACHE_SIZE as u32) as usize;
        let entry = &self.cache[slot];
        if entry.valid && entry.cp == cp && entry.wide == wide && entry.italic == italic && entry.mark == mark {
            self.cache_hits += 1;
            return Ok(slot);
        }
        self.load(slot, cp, wide, italic, mark, span)
    }

    /// find's miss: the glyph of cp into the slot, evicting what it held.
    ///
    /// # Safety
    /// As Glyphs::new.
    #[inline(never)]
    unsafe fn load(&mut self, slot: usize, cp: u32, wide: bool, italic: bool, mark: bool, span: i32)
                   -> Result<usize, Failure> {
        self.evictions += self.cache[slot].valid as usize;
        self.cache[slot] = Glyph { cp, valid: true, wide, italic, mark, ..Glyph::default() };
        let mut source = &self.font;
        let mut from_fallback = false;
        // A glyph with no outline counts as missing unless the character is
        // blank by design: color emoji fonts (sbix, CBDT) map characters to
        // empty glyphs and draw them from bitmaps, which stb_truetype cannot.
        let blank = is_blank(cp);
        let mut glyph = (self.find_glyph_index)(source.info, cp as c_int);
        let mut hollow = 0;
        // bare is 1 for an empty glyph, -1 when memory ran out.
        let mut bare = if glyph != 0 && !blank { Self::glyph_empty(source, &mut self.scratch, glyph) } else { 0 };
        if bare > 0 {
            glyph = 0;
            hollow = EMPTY_IN_FONT;
        }
        if let (0, Some(fallback), true) = (glyph, &self.fallback, bare >= 0) {
            self.fallback_lookups += 1;
            source = fallback;
            from_fallback = true;
            glyph = (self.find_glyph_index)(source.info, cp as c_int);
            bare = if glyph != 0 && !blank { Self::glyph_empty(source, &mut self.scratch, glyph) } else { 0 };
            if bare > 0 {
                glyph = 0;
                hollow |= EMPTY_IN_FALLBACK;
            }
        }
        if bare < 0 {
            return Err(Failure::OutOfMemory);
        }
        let mut s = source.scale;
        let entry = &mut self.cache[slot];
        entry.missing = glyph == 0;
        if !mark {
            self.missing += entry.missing as usize;
        }
        entry.empty = hollow;
        if glyph == 0 {
            return Ok(slot);
        }
        // The primary font's narrow glyphs sit where the font puts them.
        // Wide and fallback glyphs are centered, and a fallback glyph too
        // wide for its cells is shrunk. A mark is neither (paint_marks
        // places it).
        let (mut glyph_adv, mut glyph_lsb) = (0, 0);
        (self.get_glyph_h_metrics)(source.info, glyph, &mut glyph_adv, &mut glyph_lsb);
        // At an instance, as HVAR varies it.
        let face = &*source.face;
        if let Some(advance) = face.advance {
            glyph_adv = advance(face.advances, glyph, glyph_adv);
        }
        let mut advance = glyph_adv as f32 * s;
        if !mark && from_fallback && advance > span as f32 {
            s *= span as f32 / advance;
            advance = span as f32;
        }
        entry.advance = advance;
        if !mark && (wide || from_fallback) {
            entry.shift = ((span as f32 - advance) / 2.0 + 0.5).floor() as i32;
        }
        // stb's outline, freed on the way out; or the CFF face's, in scratch.
        let mut shape = Shape { v: ptr::null_mut(), info: source.info, free_shape: self.free_shape };
        let (mut outline, mut verts): (*mut Vertex, c_int) = (ptr::null_mut(), 0);
        match source.outlines {
            Outlines::Cff { outline: get, cff } => {
                let Some(n) = self.scratch.fetch(get, cff, source.face, glyph) else {
                    return Err(Failure::OutOfMemory);
                };
                verts = n as c_int;
                outline = self.scratch.v.as_mut_ptr();
            }
            Outlines::TrueType { get_glyph_shape, .. } if italic => {
                verts = get_glyph_shape(source.info, glyph, &mut shape.v);
                outline = shape.v;
            }
            Outlines::TrueType { .. } => {}
        }
        let (ix0, iy0, ix1, iy1);
        if italic {
            let v: &mut [Vertex] = if verts > 0 && !outline.is_null() {
                std::slice::from_raw_parts_mut(outline, verts as usize)
            } else {
                &mut []
            };
            [ix0, iy0, ix1, iy1] = slant_outline(v, self.italic_pivot / s, s);
            self.scratch.face = ptr::null(); // now slanted
        } else if let Outlines::TrueType { get_glyph_bitmap_box, .. } = source.outlines {
            let mut b = [0; 4];
            get_glyph_bitmap_box(source.info, glyph, s, s, &mut b[0], &mut b[1], &mut b[2], &mut b[3]);
            [ix0, iy0, ix1, iy1] = b;
        } else if verts == 0 {
            [ix0, iy0, ix1, iy1] = [0; 4];
        } else {
            // As stbtt_GetGlyphBitmapBox rounds the glyph's box.
            let b = self.scratch.bbox;
            ix0 = (b[0] as f32 * s).floor() as i32;
            iy0 = ((-b[3]) as f32 * s).floor() as i32;
            ix1 = (b[2] as f32 * s).ceil() as i32;
            iy1 = ((-b[1]) as f32 * s).ceil() as i32;
        }
        (entry.ix0, entry.iy0) = (ix0, iy0);
        entry.w = ix1 - ix0;
        entry.h = iy1 - iy0;
        if entry.w > 0 && entry.h > 0 {
            let len = entry.w as usize * entry.h as usize;
            if !grow(Site::Bitmap, &mut entry.bitmap, len) {
                return Err(Failure::OutOfMemory);
            }
            let pixels = entry.bitmap.as_mut_ptr();
            match source.outlines {
                Outlines::TrueType { make_glyph_bitmap, .. } if outline.is_null() => {
                    make_glyph_bitmap(source.info, pixels, entry.w, entry.h, entry.w, s, s, glyph);
                }
                _ => {
                    let mut out = StbBitmap { w: entry.w, h: entry.h, stride: entry.w, pixels };
                    // stb's userdata is for its allocator, which is malloc.
                    (self.rasterize)(&mut out, 0.35, outline, verts, s, s, 0.0, 0.0, ix0, iy0, 1, ptr::null_mut());
                }
            }
            self.glyphs += 1;
            self.fallback_glyphs += from_fallback as usize;
        }
        drop(shape);
        Ok(slot)
    }
}

/// Slants an outline in place by 12 degrees (ITALIC_SLANT), pivoting on
/// `pivot` font units above the baseline, the middle of the body, so a
/// glyph stays centered in its cell and leans as far into each neighbour.
/// Returns its pixel box at scale s, [ix0, iy0, ix1, iy1].
fn slant_outline(v: &mut [Vertex], pivot: f32, s: f32) -> [i32; 4] {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (0f32, 0f32, 0f32, 0f32);
    for (i, vx) in v.iter_mut().enumerate() {
        // Control points are inside the box of the curve's points, so the
        // box of all of them holds the outline. A line has none.
        let points = if vx.kind == CUBIC { 3 } else if vx.kind == CURVE { 2 } else { 1 };
        let ys = [vx.y, vx.cy, vx.cy1];
        for (k, &yk) in ys.iter().enumerate().take(points) {
            let xk = match k {
                0 => &mut vx.x,
                1 => &mut vx.cx,
                _ => &mut vx.cx1,
            };
            let mut x = *xk as f32 + ITALIC_SLANT * (yk as f32 - pivot);
            x = if x < -32768.0 {
                -32768.0
            } else if x > 32767.0 {
                32767.0
            } else {
                x
            };
            // lroundf: half away from zero, as f32::round.
            *xk = x.round() as i16;
            let (xf, yf) = (*xk as f32, yk as f32);
            let first = i == 0 && k == 0;
            if first || xf < min_x {
                min_x = xf;
            }
            if first || xf > max_x {
                max_x = xf;
            }
            if first || yf < min_y {
                min_y = yf;
            }
            if first || yf > max_y {
                max_y = yf;
            }
        }
    }
    if v.is_empty() {
        return [0; 4];
    }
    // As stbtt_GetGlyphBitmapBox rounds the glyph's own box; y grows down.
    [(min_x * s).floor() as i32, (-max_y * s).floor() as i32, (max_x * s).ceil() as i32, (-min_y * s).ceil() as i32]
}

/// Where glyph painting allocates. The fault tests fail each in turn.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Site {
    /// The cache's slots.
    Cache,
    /// The CFF outline scratch, at first and as it grows.
    Outline,
    /// A glyph's bitmap.
    Bitmap,
}

/// `v` resized to `len` (zeros or default vertices), or false, with `v` as
/// it was, when memory runs out. Each call is one allocation, as the C's
/// malloc or realloc was.
fn grow<T: Clone + Default>(site: Site, v: &mut Vec<T>, len: usize) -> bool {
    if !allowed(site) {
        return false;
    }
    if len > v.len() && v.try_reserve_exact(len - v.len()).is_err() {
        return false;
    }
    v.resize(len, T::default());
    true
}

/// Whether an allocation at `site` may go ahead (always, but for the fault
/// tests).
fn allowed(site: Site) -> bool {
    #[cfg(any(test, termshot_alloc_faults))]
    if faults::fail(site) {
        return false;
    }
    let _ = site;
    true
}

/// Allocation failure injection: the shipped binary has none. Unit tests
/// set it per thread; a build with `--cfg termshot_alloc_faults` reads
/// TERMSHOT_GLYPH_FAIL_AT=n and fails the nth glyph allocation of each
/// render (tests/run.sh checks that the CLI exits 2 and leaves no output).
#[cfg(any(test, termshot_alloc_faults))]
pub(crate) mod faults {
    use super::Site;
    use std::cell::Cell;

    #[derive(Clone, Copy, Default)]
    pub(crate) struct Faults {
        /// Allocations so far, and the one to fail (0 for none).
        pub(crate) calls: u32,
        pub(crate) fail_at: u32,
    }

    thread_local! {
        pub(crate) static FAULTS: Cell<Faults> = Cell::new(Faults::default());
        #[cfg(test)]
        pub(crate) static FAILED: Cell<Option<Site>> = Cell::new(None);
    }

    /// Starts a render: count from zero, and fail where asked.
    #[cfg(not(test))]
    pub(super) fn start() {
        let fail_at = std::env::var("TERMSHOT_GLYPH_FAIL_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        FAULTS.with(|f| f.set(Faults { calls: 0, fail_at }));
    }

    /// Starts a render: counts from zero; the test chose where to fail.
    #[cfg(test)]
    pub(super) fn start() {
        FAULTS.with(|f| f.set(Faults { calls: 0, ..f.get() }));
    }

    /// Whether this allocation fails.
    pub(super) fn fail(_site: Site) -> bool {
        FAULTS.with(|f| {
            let mut s = f.get();
            s.calls += 1;
            f.set(s);
            let fail = s.calls == s.fail_at;
            #[cfg(test)]
            if fail {
                FAILED.with(|failed| failed.set(Some(_site)));
            }
            // So tests/run.sh knows when it has failed them all.
            #[cfg(not(test))]
            if fail {
                eprintln!("termshot: glyph allocation {} ({:?}) fails", s.calls, _site);
            }
            fail
        })
    }
}

#[cfg(not(any(test, termshot_alloc_faults)))]
mod faults {
    pub(super) fn start() {}
}

#[cfg(test)]
#[path = "glyphs_tests.rs"]
mod glyphs_tests;
