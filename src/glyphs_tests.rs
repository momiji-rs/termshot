//! glyphs.rs tests, against fake stb functions: the cache's keys, slots and
//! conflicts, the font and fallback lookups, missing and empty glyphs and
//! their report, the CFF rule, the outline scratch, the fallback's scaling
//! and centering, mark placement, the slant, the lines, and a failure at
//! each allocation in turn. That the real fonts draw the same pixels as the
//! C did is bench/c-vs-rust/run.sh glyphs; tests/glyphs.c checks glyph
//! placement with them through draw.c, and src/draw_tests.rs renders them.

use super::faults::{Faults, FAILED, FAULTS};
use super::*;
use crate::cell::{TAIL, UNDERLINE};
use crate::composite::termshot_backdrop_init;
use std::cell::RefCell;

const CELL_W: i32 = 8;
const CELL_H: i32 = 16;
const BASELINE: i32 = 12;
/// Font units to pixels: 800 units of advance are a cell.
const SCALE: f32 = 0.01;

/// A font as the fake stb functions see it: the code points it has (glyph
/// id = code point), its empty glyphs, each glyph's advance and outline box
/// (x0, y0, x1, y1 in font units, y up), and for CFF each glyph's vertex
/// count.
struct Fake {
    has: fn(u32) -> bool,
    empty: fn(c_int) -> bool,
    advance: fn(c_int) -> c_int,
    ubox: fn(c_int) -> [c_int; 4],
    verts: fn(c_int) -> c_int,
}

fn every(_: u32) -> bool {
    true
}
fn none(_: c_int) -> bool {
    false
}
fn cell_advance(_: c_int) -> c_int {
    800
}
fn letter_box(_: c_int) -> [c_int; 4] {
    [0, -200, 600, 800]
}
fn four(_: c_int) -> c_int {
    4
}

const PLAIN: Fake = Fake { has: every, empty: none, advance: cell_advance, ubox: letter_box, verts: four };

thread_local! {
    /// What the fakes were asked, in order.
    static CALLS: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

fn note(call: String) {
    CALLS.with(|c| c.borrow_mut().push(call));
}

fn take_calls() -> Vec<String> {
    CALLS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

fn count(calls: &[String], prefix: &str) -> usize {
    calls.iter().filter(|c| c.starts_with(prefix)).count()
}

unsafe fn fake<'a>(info: *const FontInfo) -> &'a Fake {
    &*(info as *const Fake)
}

unsafe extern "C" fn find_glyph_index(info: *const FontInfo, cp: c_int) -> c_int {
    if (fake(info).has)(cp as u32) {
        cp
    } else {
        0
    }
}

unsafe extern "C" fn get_glyph_h_metrics(info: *const FontInfo, glyph: c_int, advance: *mut c_int, lsb: *mut c_int) {
    *advance = (fake(info).advance)(glyph);
    *lsb = 0;
}

unsafe extern "C" fn is_glyph_empty(info: *const FontInfo, glyph: c_int) -> c_int {
    note(format!("stb empty {glyph}"));
    (fake(info).empty)(glyph) as c_int
}

/// The outline box as a closed square's four vertices.
fn square(b: [c_int; 4]) -> [Vertex; 4] {
    let v = |kind, x: c_int, y: c_int| Vertex { x: x as i16, y: y as i16, kind, ..Vertex::default() };
    [v(crate::cff::MOVE, b[0], b[1]), v(crate::cff::LINE, b[2], b[1]), v(crate::cff::LINE, b[2], b[3]),
     v(crate::cff::LINE, b[0], b[3])]
}

unsafe extern "C" fn get_glyph_shape(info: *const FontInfo, glyph: c_int, out: *mut *mut Vertex) -> c_int {
    note(format!("stb shape {glyph}"));
    *out = Box::into_raw(Box::new(square((fake(info).ubox)(glyph)))) as *mut Vertex;
    4
}

unsafe extern "C" fn free_shape(_info: *const FontInfo, v: *mut Vertex) {
    if !v.is_null() {
        note("free".into());
        drop(Box::from_raw(v as *mut [Vertex; 4]));
    }
}

/// stbtt_GetGlyphBitmapBox's rounding of the box.
fn pixel_box(b: [c_int; 4], s: f32) -> [c_int; 4] {
    [(b[0] as f32 * s).floor() as c_int, (-b[3] as f32 * s).floor() as c_int, (b[2] as f32 * s).ceil() as c_int,
     (-b[1] as f32 * s).ceil() as c_int]
}

unsafe extern "C" fn get_glyph_bitmap_box(info: *const FontInfo, glyph: c_int, s: f32, _sy: f32, ix0: *mut c_int,
                                          iy0: *mut c_int, ix1: *mut c_int, iy1: *mut c_int) {
    note(format!("stb box {glyph} {s}"));
    let b = pixel_box((fake(info).ubox)(glyph), s);
    (*ix0, *iy0, *ix1, *iy1) = (b[0], b[1], b[2], b[3]);
}

unsafe extern "C" fn make_glyph_bitmap(_info: *const FontInfo, out: *mut u8, w: c_int, h: c_int, stride: c_int,
                                       s: f32, _sy: f32, glyph: c_int) {
    note(format!("stb bitmap {glyph} {s}"));
    // HOLLOW has a box but no points, as a composite of an empty glyph
    // has: stb's rasterizer then writes nothing.
    if glyph == HOLLOW {
        return;
    }
    for y in 0..h {
        std::ptr::write_bytes(out.add((y * stride) as usize), 200, w as usize);
    }
}

const HOLLOW: c_int = 0x391;

unsafe extern "C" fn rasterize(bm: *mut StbBitmap, _flatness: f32, v: *mut Vertex, n: c_int, s: f32, _sy: f32,
                               _shift_x: f32, _shift_y: f32, _x: c_int, _y: c_int, _invert: c_int,
                               _userdata: *mut c_void) {
    let first = std::slice::from_raw_parts(v, n as usize)[0];
    note(format!("rasterize {n} {s} {}", first.x));
    let bm = &*bm;
    for y in 0..bm.h {
        std::ptr::write_bytes(bm.pixels.add((y * bm.stride) as usize), 255, bm.w as usize);
    }
}

unsafe extern "C" fn cff_outline(cff: *const c_void, glyph: c_int, out: *mut Vertex, capacity: c_int,
                                 bbox: *mut c_int) -> c_int {
    note(format!("cff outline {glyph} {capacity}"));
    let f = &*(cff as *const Fake);
    let n = (f.verts)(glyph);
    let b = (f.ubox)(glyph);
    if n > 0 && n <= capacity {
        let sq = square(b);
        for k in 0..n as usize {
            *out.add(k) = sq[k.min(3)];
        }
    }
    let bbox = std::slice::from_raw_parts_mut(bbox, 4);
    bbox.copy_from_slice(&if n > 0 { b } else { [0; 4] });
    n
}

/// A TrueType face of `font`, at `scale`.
fn truetype(font: &Fake, face: &Face, scale: f32) -> GlyphFace {
    GlyphFace {
        info: font as *const Fake as *const FontInfo,
        face,
        scale,
        is_glyph_empty: Some(is_glyph_empty),
        get_glyph_shape: Some(get_glyph_shape),
        get_glyph_bitmap_box: Some(get_glyph_bitmap_box),
        make_glyph_bitmap: Some(make_glyph_bitmap),
    }
}

/// A CFF face of `font`, whose Face must be `cff_face(font)`.
fn cff(font: &Fake, face: &Face, scale: f32) -> GlyphFace {
    GlyphFace {
        info: font as *const Fake as *const FontInfo,
        face,
        scale,
        is_glyph_empty: None,
        get_glyph_shape: None,
        get_glyph_bitmap_box: None,
        make_glyph_bitmap: None,
    }
}

const TT_FACE: Face = Face {
    ttf: std::ptr::null(),
    start: 0,
    outline: None,
    cff: std::ptr::null(),
    advance: None,
    advances: std::ptr::null(),
    varied: 0,
    ascent: 0,
    descent: 0,
    line_gap: 0,
};

fn cff_face(font: &Fake) -> Face {
    Face { outline: Some(cff_outline), cff: font as *const Fake as *const c_void, ..TT_FACE }
}

fn no_face() -> GlyphFace {
    GlyphFace {
        info: std::ptr::null(),
        face: std::ptr::null(),
        scale: 0.0,
        is_glyph_empty: None,
        get_glyph_shape: None,
        get_glyph_bitmap_box: None,
        make_glyph_bitmap: None,
    }
}

fn cell(ch: u32, attrs: u8) -> Cell {
    Cell { ch, fr: 255, fg: 255, fb: 255, br: 0, bg: 0, bb: 0, attrs }
}

fn cells(text: &str) -> Vec<Cell> {
    text.chars().map(|c| cell(c as u32, 0)).collect()
}

/// What painting a row of cells gave: the result, the stats, the report of
/// empty glyphs, and the canvas, w x h RGB pixels.
struct Painted {
    code: c_int,
    stats: TextStats,
    empty: EmptyGlyphs,
    px: Vec<u8>,
    w: i32,
}

impl Painted {
    /// The columns of row y that are not the background.
    fn lit(&self, y: i32) -> Vec<i32> {
        (0..self.w).filter(|&x| self.at(x, y) != [0, 0, 0]).collect()
    }

    fn at(&self, x: i32, y: i32) -> [u8; 3] {
        let i = (y * self.w + x) as usize * 3;
        [self.px[i], self.px[i + 1], self.px[i + 2]]
    }
}

fn paint(cells: &[Cell], marks: &[CellMarks], font: GlyphFace, fallback: GlyphFace) -> Painted {
    let (cols, rows) = (cells.len() as i32, 1);
    let (w, h) = (cols * CELL_W, rows * CELL_H);
    let stride = w as usize * 3 + 1;
    let mut filtered = vec![0xeeu8; stride * h as usize];
    let p = filtered.as_mut_ptr();
    let cv = Canvas { px: unsafe { p.add(1) }, filtered: p, w, h, stride, geometry: std::ptr::null_mut() };
    let fonts = TextFonts {
        font,
        fallback,
        find_glyph_index: Some(find_glyph_index),
        get_glyph_h_metrics: Some(get_glyph_h_metrics),
        rasterize: Some(rasterize),
        free_shape: Some(free_shape),
        italic_pivot: 4.0,
        baseline: BASELINE,
    };
    let mut bd = std::mem::MaybeUninit::<Backdrop>::uninit();
    let mut empty = EmptyGlyphs::default();
    let mut stats = TextStats { glyphs: 99, ..Default::default() };
    let code = unsafe {
        termshot_backdrop_init(bd.as_mut_ptr(), &cv, cells.as_ptr(), cols, rows, CELL_W, CELL_H, std::ptr::null(), 0,
                               0);
        termshot_paint_text(&cv, bd.as_mut_ptr(), marks.as_ptr(), marks.len(), &fonts, 0, &mut empty, &mut stats)
    };
    let mut px = Vec::new();
    for y in 0..h as usize {
        px.extend_from_slice(&filtered[y * stride + 1..(y + 1) * stride]);
    }
    Painted { code, stats, empty, px, w }
}

fn plain(cells: &[Cell], marks: &[CellMarks]) -> Painted {
    paint(cells, marks, truetype(&PLAIN, &TT_FACE, SCALE), no_face())
}

fn marks(cell: u32, marks: &[u32]) -> CellMarks {
    let mut m = CellMarks { cell, marks: [0; MAX_MARKS] };
    m.marks[..marks.len()].copy_from_slice(marks);
    m
}

#[test]
fn the_cache_reuses_a_glyph_and_evicts_on_a_conflict() {
    let p = plain(&cells("aab"), &[]);
    assert_eq!((p.code, p.stats.glyphs, p.stats.cache_hits, p.stats.evictions), (0, 2, 1, 0));
    // Code points GLYPH_CACHE_SIZE apart share a slot, and take it in turn.
    let a = 'a' as u32;
    let row = [a, a + 1024, a, a + 1024].map(|cp| cell(cp, 0));
    let s = plain(&row, &[]).stats;
    assert_eq!((s.glyphs, s.cache_hits, s.evictions), (4, 0, 3));
    // An italic glyph's slot is half the cache away: cp italic and cp + 512
    // upright conflict, cp upright and italic don't.
    let s = plain(&[cell(a, ITALIC), cell(a + 512, 0)], &[]).stats;
    assert_eq!((s.glyphs, s.evictions), (2, 1));
    let s = plain(&[cell(a, ITALIC), cell(a, ITALIC), cell(a, 0), cell(a, 0)], &[]).stats;
    assert_eq!((s.glyphs, s.cache_hits, s.evictions), (2, 2, 0));
    // Wide and mark are part of the key, so the same code point narrow and
    // wide, or as a character and as a mark, conflict.
    let s = plain(&[cell(0x4e2d, WIDE), cell(0, TAIL), cell(0x4e2d, 0)], &[]).stats;
    assert_eq!((s.glyphs, s.evictions), (2, 1));
    let s = plain(&cells("ab"), &[marks(0, &[a])]).stats;
    assert_eq!((s.glyphs, s.cache_hits, s.evictions), (3, 0, 1));
    // Bold and colours share the glyph.
    let mut bold = cells("aa");
    bold[1].attrs = BOLD;
    bold[1].fg = 7;
    assert_eq!(plain(&bold, &[]).stats.cache_hits, 1);
}

fn no_z(cp: u32) -> bool {
    cp != 'z' as u32
}
fn empty_e(glyph: c_int) -> bool {
    glyph == 'e' as c_int
}
fn only_e(cp: u32) -> bool {
    cp == 'e' as u32
}

#[test]
fn missing_and_empty_glyphs_are_boxes_and_reported() {
    let font = Fake { has: no_z, empty: empty_e, ..PLAIN };
    let fallback = Fake { has: only_e, ..PLAIN };
    let empty_fallback = Fake { has: only_e, empty: empty_e, ..PLAIN };
    let paint_with = |text: &str, fallback: Option<&Fake>| {
        let fallback = fallback.map_or_else(no_face, |f| truetype(f, &TT_FACE, SCALE));
        paint(&cells(text), &[], truetype(&font, &TT_FACE, SCALE), fallback)
    };
    // In no font: a box, inset in its cell (1 pixel thick at 8 x 16), and
    // missing, but not reported: no font maps it to an empty glyph.
    let p = paint_with(" z", None);
    assert_eq!((p.stats.missing, p.empty.cells), (1, 0));
    assert_eq!(p.lit(2), (CELL_W + 1..2 * CELL_W - 1).collect::<Vec<_>>());
    assert_eq!(p.lit(8), vec![CELL_W + 1, 2 * CELL_W - 2]);
    // An empty glyph with no fallback: a box, reported.
    let p = paint_with("ae e", None);
    assert_eq!((p.stats.missing, p.empty.cp, p.empty.fonts, p.empty.col, p.empty.row, p.empty.cells),
               (1, 'e' as u32, EMPTY_IN_FONT, 1, 0, 2));
    assert_eq!(p.lit(2), (CELL_W + 1..2 * CELL_W - 1).chain(3 * CELL_W + 1..4 * CELL_W - 1).collect::<Vec<_>>());
    // The fallback draws it, so it is neither.
    let p = paint_with("e", Some(&fallback));
    let s = p.stats;
    assert_eq!((s.missing, s.fallback_lookups, s.fallback_glyphs, p.empty.cells), (0, 1, 1, 0));
    // Empty in both: reported for both.
    let p = paint_with("e", Some(&empty_fallback));
    assert_eq!((p.stats.missing, p.empty.fonts, p.empty.cells), (1, EMPTY_IN_FONT | EMPTY_IN_FALLBACK, 1));
    // A blank character no font has draws nothing, and an ignorable one
    // isn't looked up at all.
    let blank = Fake { has: |cp| cp == 'a' as u32, ..PLAIN };
    let p = paint(&cells("\u{3000}\u{2800}\u{200d}"), &[], truetype(&blank, &TT_FACE, SCALE), no_face());
    assert_eq!((p.code, p.stats.missing, p.empty.cells), (0, 2, 0));
    assert!((0..CELL_H).all(|y| p.lit(y).is_empty()));
    // A mark no font has is left out, and isn't counted.
    let p = paint(&cells("a"), &[marks(0, &[0x301])], truetype(&blank, &TT_FACE, SCALE), no_face());
    assert_eq!((p.stats.missing, p.stats.glyphs), (0, 1));
}

#[test]
fn stb_reads_only_truetype_outlines() {
    let face = cff_face(&PLAIN);
    // A CFF face's outlines come from its callback, one run for the check
    // for an empty glyph and the drawing; stb's readers aren't held.
    take_calls();
    let p = paint(&cells("ab"), &[], cff(&PLAIN, &face, SCALE), no_face());
    let calls = take_calls();
    assert_eq!(p.code, 0);
    assert_eq!(count(&calls, "cff outline"), 2, "{calls:?}");
    assert_eq!(count(&calls, "stb "), 0, "{calls:?}");
    assert_eq!(count(&calls, "rasterize 4"), 2, "{calls:?}");
    // A TrueType face: stb checks, boxes and draws an upright glyph, and
    // gives the outline of an italic one, freed after.
    let p = paint(&[cell('a' as u32, 0), cell('b' as u32, ITALIC)], &[], truetype(&PLAIN, &TT_FACE, SCALE), no_face());
    let calls = take_calls();
    assert_eq!(p.code, 0);
    for call in ["stb empty", "stb box", "stb bitmap", "stb shape", "free", "rasterize"] {
        assert_eq!(count(&calls, call), if call == "stb empty" { 2 } else { 1 }, "{call}: {calls:?}");
    }
    // A CFF face handed stb's readers, or a TrueType face without them, is
    // refused: the render fails.
    let both = GlyphFace { face: &face, ..truetype(&PLAIN, &TT_FACE, SCALE) };
    assert_eq!(paint(&cells("a"), &[], both, no_face()).code, Failure::Panicked as c_int);
    let neither = cff(&PLAIN, &TT_FACE, SCALE);
    assert_eq!(paint(&cells("a"), &[], neither, no_face()).code, Failure::Panicked as c_int);
    let both = GlyphFace { face: &face, ..truetype(&PLAIN, &TT_FACE, SCALE) };
    assert_eq!(paint(&cells("a"), &[], truetype(&PLAIN, &TT_FACE, SCALE), both).code, Failure::Panicked as c_int);
    assert_eq!(count(&take_calls(), "stb "), 0);
}

#[test]
fn a_slanted_outline_is_not_drawn_upright() {
    let face = cff_face(&PLAIN);
    take_calls();
    let row = [cell('a' as u32, ITALIC), cell('a' as u32, 0)];
    assert_eq!(paint(&row, &[], cff(&PLAIN, &face, SCALE), no_face()).code, 0);
    let calls = take_calls();
    // The italic glyph slants the scratch outline, so the upright one runs
    // the charstring again and is drawn from x = 0, not leaning.
    assert_eq!(count(&calls, "cff outline"), 2, "{calls:?}");
    let drawn: Vec<_> = calls.iter().filter(|c| c.starts_with("rasterize")).collect();
    // (-200 - 400) * 0.21256 = -127.5, rounded away from zero.
    assert_eq!(drawn, ["rasterize 4 0.01 -128", "rasterize 4 0.01 0"]);
}

fn grown(glyph: c_int) -> c_int {
    match glyph {
        0x61 => 600,
        0x62 => 5000,
        _ => 4,
    }
}

#[test]
fn the_outline_scratch_grows_to_fit() {
    let font = Fake { verts: grown, ..PLAIN };
    let face = cff_face(&font);
    take_calls();
    assert_eq!(paint(&cells("abc"), &[], cff(&font, &face, SCALE), no_face()).code, 0);
    let calls: Vec<_> = take_calls().into_iter().filter(|c| c.starts_with("cff")).collect();
    // Room for 512, then doubled, then as much as asked; then enough.
    assert_eq!(calls, ["cff outline 97 512", "cff outline 97 1024", "cff outline 98 1024", "cff outline 98 5000",
                       "cff outline 99 5000"]);
}

fn wide_advance(glyph: c_int) -> c_int {
    if glyph == 'w' as c_int {
        1600
    } else {
        400
    }
}
fn ink_box(glyph: c_int) -> [c_int; 4] {
    let advance = wide_advance(glyph);
    [0, 0, advance, 300]
}

#[test]
fn fallback_glyphs_are_centered_and_shrunk_to_fit() {
    let font = Fake { has: |cp| cp == 'x' as u32, ..PLAIN };
    let fallback = Fake { advance: wide_advance, ubox: ink_box, ..PLAIN };
    take_calls();
    let p = paint(&cells("nwx"), &[], truetype(&font, &TT_FACE, SCALE), truetype(&fallback, &TT_FACE, SCALE));
    let calls = take_calls();
    // 'n' is 4 pixels wide, centered in its 8: 2 in. 'w' is 16, shrunk to
    // 8 at half the scale.
    assert_eq!(p.lit(BASELINE - 1), (2..6).chain(CELL_W..2 * CELL_W).chain(2 * CELL_W..2 * CELL_W + 6).collect::<Vec<_>>());
    assert!(calls.contains(&"stb box 119 0.005".to_string()), "{calls:?}");
    // The primary font's narrow glyph sits where the font puts it; its
    // wide one is centered over both cells.
    let p = paint(&[cell('n' as u32, WIDE), cell(0, TAIL)], &[], truetype(&fallback, &TT_FACE, SCALE), no_face());
    assert_eq!(p.lit(BASELINE - 1), (6..10).collect::<Vec<_>>());
}

fn mark_box(glyph: c_int) -> [c_int; 4] {
    match glyph {
        0x301 => [-500, 900, -100, 1100],
        0x302 => [100, 900, 500, 1100],
        _ => [0, 0, 400, 300],
    }
}

#[test]
fn marks_overlay_the_character_before_them() {
    let font = Fake { advance: |_| 400, ubox: mark_box, ..PLAIN };
    let f = || truetype(&font, &TT_FACE, SCALE);
    // A mark left of its origin is drawn from where the character ends: its
    // advance is 4 pixels from 8, and the mark 5 to 1 left of that.
    let p = paint(&cells(" a "), &[marks(1, &[0x301])], f(), no_face());
    assert_eq!(p.lit(BASELINE - 10), (7..11).collect::<Vec<_>>());
    // A mark right of its origin is centered over the character, whatever
    // its own offset: 8 + (4 - 4) / 2.
    let p = paint(&cells(" a "), &[marks(1, &[0x302])], f(), no_face());
    assert_eq!(p.lit(BASELINE - 10), (8..12).collect::<Vec<_>>());
    // Over a space it is centered in the cell's 8 pixels, or drawn from its end.
    let p = paint(&cells("  "), &[marks(0, &[0x302])], f(), no_face());
    assert_eq!(p.lit(BASELINE - 10), (2..6).collect::<Vec<_>>());
    let p = paint(&cells("  "), &[marks(0, &[0x301])], f(), no_face());
    assert_eq!(p.lit(BASELINE - 10), (3..7).collect::<Vec<_>>());
    // Bold doubles it a pixel right; an ignorable mark draws nothing, and
    // the rest still do.
    let mut row = cells("  ");
    row[0].attrs = BOLD;
    let p = paint(&row, &[marks(0, &[0x200d, 0x302])], f(), no_face());
    assert_eq!(p.lit(BASELINE - 10), (2..7).collect::<Vec<_>>());
    // A Hangul filler draws nothing, but its marks do.
    let p = paint(&cells("\u{115f} "), &[marks(0, &[0x302])], f(), no_face());
    assert_eq!(p.lit(BASELINE - 10), (2..6).collect::<Vec<_>>());
    assert!(p.lit(BASELINE - 1).is_empty());
}

#[test]
fn the_slant_pivots_on_the_middle_of_the_body() {
    let v = |kind, x, y, cx, cy, cx1, cy1| Vertex { x, y, cx, cy, cx1, cy1, kind, padding: 0 };
    let mut outline = [
        v(crate::cff::MOVE, 10, 100, 0, 0, 0, 0),
        v(crate::cff::LINE, 10, 600, 7, 7, 7, 7),
        v(CURVE, 10, -400, 20, 1100, 0, 0),
        v(CUBIC, 0, 100, 0, 200, 0, -100),
    ];
    let b = slant_outline(&mut outline, 100.0, 0.1);
    // At the pivot nothing moves; 500 units above it, 106.28 right; a line's
    // control points stay as they were, a curve's one moves, a cubic's two.
    assert_eq!([outline[0].x, outline[1].x, outline[1].cx, outline[1].cx1], [10, 116, 7, 7]);
    assert_eq!([outline[2].x, outline[2].cx, outline[2].cx1], [-96, 233, 0]);
    assert_eq!([outline[3].x, outline[3].cx, outline[3].cx1], [0, 21, -43]);
    // Its box, of every point, control points included, at scale 0.1.
    assert_eq!(b, [(-96.0f32 * 0.1).floor() as i32, -110, 24, 40]);
    // Far from the pivot x is clamped to i16.
    let mut far = [v(crate::cff::MOVE, 32000, 32000, 0, 0, 0, 0), v(crate::cff::LINE, -32000, -32000, 0, 0, 0, 0)];
    slant_outline(&mut far, 0.0, 1.0);
    assert_eq!([far[0].x, far[1].x], [32767, -32768]);
    assert_eq!(slant_outline(&mut [], 0.0, 1.0), [0; 4]);
}

#[test]
fn lines_are_inside_the_cell() {
    // cell_w 8 makes them 1 pixel thick: underline at 12 + 4 / 3 = 13,
    // double at 13 and 15, strike-through at 12 - 3 = 9.
    let row = [cell(' ' as u32, UNDERLINE), cell(' ' as u32, DOUBLE_UNDERLINE), cell(' ' as u32, STRIKE),
               cell(' ' as u32, UNDERLINE | DOUBLE_UNDERLINE | STRIKE)];
    let p = plain(&row, &[]);
    let lit_rows = |c: i32| (0..CELL_H).filter(|&y| p.lit(y).contains(&(c * CELL_W))).collect::<Vec<_>>();
    assert_eq!([lit_rows(0), lit_rows(1), lit_rows(2), lit_rows(3)], [vec![13], vec![13, 15], vec![9], vec![9, 13, 15]]);
    assert!((0..CELL_H).all(|y| p.lit(y).len() % CELL_W as usize == 0));
}

#[test]
fn blank_and_ignorable_characters() {
    for cp in [0x20, 0xa0, 0x1680, 0x2000, 0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x2800, 0x3000] {
        assert!(is_blank(cp), "U+{cp:04X}");
    }
    for cp in [0x1f, 0x21, 0x1fff, 0x200b, 0x2801, 0x3001] {
        assert!(!is_blank(cp), "U+{cp:04X}");
    }
    for cp in [0x34f, 0x61c, 0x115f, 0x1160, 0x17b4, 0x180b, 0x180f, 0x200b, 0x200f, 0x202a, 0x2060, 0x206f, 0x3164,
               0xfe00, 0xfe0f, 0xfeff, 0xffa0, 0xfff0, 0xfff8, 0x1bca0, 0x1d173, 0x1d17a, 0xe0000, 0xe0fff] {
        assert!(is_ignorable(cp), "U+{cp:04X}");
    }
    for cp in [0xad, 0x34e, 0x350, 0x1161, 0x2010, 0x2070, 0x3165, 0xfe10, 0xfff9, 0x1d17b, 0xe1000] {
        assert!(!is_ignorable(cp), "U+{cp:04X}");
    }
}

fn set_faults(fail_at: u32) {
    FAULTS.with(|f| f.set(Faults { calls: 0, fail_at }));
    FAILED.with(|f| f.set(None));
}

/// A screen that allocates at every site: the cache, a CFF outline's
/// scratch and its growth, and bitmaps of each font and style. The result,
/// and how many allocations it made.
fn allocating(fail_at: u32) -> (Painted, u32) {
    set_faults(fail_at);
    let font = Fake { verts: grown, has: |cp| cp != 'x' as u32, ..PLAIN };
    let face = cff_face(&font);
    let row = [cell('a' as u32, 0), cell('b' as u32, ITALIC), cell('x' as u32, BOLD), cell('x' as u32, ITALIC)];
    let p = paint(&row, &[marks(0, &[0x301])], cff(&font, &face, SCALE), truetype(&PLAIN, &TT_FACE, SCALE));
    let calls = FAULTS.with(|f| f.get().calls);
    FAULTS.with(|f| f.set(Faults::default()));
    (p, calls)
}

#[test]
fn each_allocation_failure_fails_the_render() {
    take_calls();
    let (want, allocations) = allocating(0);
    assert_eq!(want.code, 0);
    let mut sites = Vec::new();
    for fail in 1..=allocations {
        let (got, _) = allocating(fail);
        let site = FAILED.with(|f| f.get()).unwrap_or_else(|| panic!("allocation {fail} did not fail"));
        assert_eq!(got.code, Failure::OutOfMemory as c_int, "allocation {fail} ({site:?})");
        if !sites.contains(&site) {
            sites.push(site);
        }
        // stb's outlines are freed however the render ends.
        let calls = take_calls();
        assert_eq!(count(&calls, "stb shape"), count(&calls, "free"), "allocation {fail}: {calls:?}");
    }
    for site in [Site::Cache, Site::Outline, Site::Bitmap] {
        assert!(sites.contains(&site), "{site:?} never failed: {sites:?}");
    }
    // The cache, the scratch at 512, 1024 and 5000 vertices, and five
    // bitmaps (two glyphs of each font and the mark).
    assert_eq!(allocations, 9);
}

/// A glyph with a box but no points (in the built-in font with 'A' emptied,
/// Α and А, composites of 'A') makes stb write nothing into its bitmap. The
/// C blended whatever malloc had left there, an evicted glyph's coverage on
/// glibc; the bitmap starts zeroed now, so it draws nothing, upright as
/// italic (whose box comes from the points).
#[test]
fn a_glyph_with_no_points_draws_nothing() {
    for _ in 0..3 {
        let row = [cell('a' as u32, 0), cell(HOLLOW as u32, 0), cell(HOLLOW as u32 + 1024, 0), cell(HOLLOW as u32, 0)];
        let p = plain(&row, &[]);
        assert_eq!((p.code, p.stats.glyphs, p.stats.missing, p.stats.evictions), (0, 4, 0, 2));
        for y in 0..CELL_H {
            assert!(p.lit(y).iter().all(|&x| x < CELL_W || (2 * CELL_W..3 * CELL_W).contains(&x)), "row {y}");
        }
    }
}
