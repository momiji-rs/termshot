//! Box drawing (U+2500..U+257F) and block elements (U+2580..U+259F), painted
//! as geometry so the joints meet at any integer cell size, and Stamps, the
//! cache that reuses rounded corners and diagonals. src/glyphs.rs calls this
//! once per cell or coarser, never per pixel: paint_cell paints one cell's
//! character, and fill_rect is the rectangle fill of the underlines and
//! missing-glyph boxes. The C harnesses call the same through FFI
//! (termshot_paint_geometry, termshot_fill_rect).
//!
//! This was C in draw.c until #12 step 2a, and it paints the same pixels:
//!
//! - The float arithmetic is the C's, operation for operation, in f32 where
//!   the C used float and in the same order. Rust never fuses a multiply and
//!   an add unless asked to with mul_add, which is not used here; draw.c was
//!   built with -ffp-contract=off for the same reason.
//! - floor, ceil and sqrt are exact IEEE operations, so f32::floor, ceil
//!   and sqrt give what floorf, ceilf and sqrtf do. sin and cos are the
//!   only rounded functions, and the C's compilers merged its sinf and cosf
//!   into one sincos call; sin_cos makes that call by name.
//! - C's (int) of a float truncates, and is undefined out of int's range;
//!   `as i32` truncates and saturates. Every float cast here is a pixel
//!   coordinate, a stroke width or a step count, bounded by the canvas
//!   (2^27 pixels at most, MAX_PIXELS in src/render.rs), so the two agree on every
//!   input that occurs. Integer arithmetic is i32 like the C's int, and the
//!   same bounds keep it from overflowing.
//!
//! Memory: the caches grow with `Vec::try_reserve_exact`, so running out of
//! memory never aborts. A stroke whose cache can't grow is stamped afresh,
//! with the same pixels, as the C did; nothing here fails a render.

use std::alloc::{alloc, dealloc, Layout};
use std::ffi::c_int;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

/// Bytes per pixel: the canvas is RGB, as BPP in src/render.rs.
const BPP: usize = 3;

/// The image being painted, shared with the C harnesses (tests/termshot.h).
/// src/render.rs owns the pixels: `filtered` is the PNG's filtered scanlines, a filter byte then
/// `w` RGB pixels each, `stride` bytes apart, and `px` its first pixel, so
/// the pixel (x, y) is at `px + y * stride + 3 * x`. Painting here writes
/// through `px` only. `geometry` is this module's state for the render, from
/// termshot_geometry_new, or null to paint without caching anything.
#[repr(C)]
pub struct Canvas {
    pub px: *mut u8,
    /// Not read here; the backdrop (src/composite.rs) copies whole
    /// scanlines through it.
    pub filtered: *mut u8,
    pub w: i32,
    pub h: i32,
    pub stride: usize,
    pub geometry: *mut Geometry,
}

/// As tests/termshot.h asserts of its Canvas.
const _: () = assert!(std::mem::size_of::<Canvas>() == 40);

/// What geometry keeps over a render: the offsets of each arc's points from
/// its centre (one per corner), and the strokes to reuse, if any.
pub struct Geometry {
    arc_offsets: [Option<Vec<f32>>; 4],
    stamps: Option<Stamps>,
}

/// The cache's counters, for TERMSHOT_PROFILE; as GeometryStats in
/// tests/termshot.h.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct GeometryStats {
    pub hits: usize,
    pub misses: usize,
    pub uncached: usize,
    /// Held when asked, and the most held at any time.
    pub bytes: usize,
    pub peak: usize,
}

const _: () = assert!(std::mem::size_of::<GeometryStats>() == 5 * std::mem::size_of::<usize>());

/// The state of a render's geometry, which reuses rounded corners and
/// diagonals (Stamps) when `reuse_strokes` is nonzero; only the arcs' offsets
/// otherwise. Free it with termshot_geometry_free. Null when memory runs out,
/// which only means nothing is cached: termshot_paint_geometry takes a
/// canvas whose geometry is null.
#[no_mangle]
pub extern "C" fn termshot_geometry_new(reuse_strokes: c_int) -> *mut Geometry {
    PANICKED.with(|p| p.set(false));
    catch_unwind(|| {
        faults::start();
        if !allowed(Site::Geometry) {
            return ptr::null_mut();
        }
        // Box::new would abort when memory runs out.
        let layout = Layout::new::<Geometry>();
        // SAFETY: Geometry has a nonzero size. The pointer is checked, and
        // written before it is returned.
        unsafe {
            let p = alloc(layout) as *mut Geometry;
            if !p.is_null() {
                p.write(Geometry { arc_offsets: Default::default(), stamps: (reuse_strokes != 0).then(Stamps::new) });
            }
            p
        }
    })
    .unwrap_or(ptr::null_mut())
}

/// Frees what termshot_geometry_new returned (null is ignored).
///
/// # Safety
/// `geometry` must be null or come from termshot_geometry_new, and not be
/// freed already or used again.
#[no_mangle]
pub unsafe extern "C" fn termshot_geometry_free(geometry: *mut Geometry) {
    if geometry.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        ptr::drop_in_place(geometry);
        dealloc(geometry as *mut u8, Layout::new::<Geometry>());
    }));
}

/// The cache's counters so far: zeros when there is no cache.
///
/// # Safety
/// `geometry` must be null or live (termshot_geometry_new), and `out`
/// writable.
#[no_mangle]
pub unsafe extern "C" fn termshot_geometry_stats(geometry: *const Geometry, out: *mut GeometryStats) {
    *out = match geometry.as_ref().and_then(|g| g.stamps.as_ref()) {
        Some(st) => st.stats(),
        None => GeometryStats::default(),
    };
}

/// Paints the box-drawing or block character `cp` of the cell at `col`,
/// `row` (of `cell_w` x `cell_h` pixels), in (r, g, b), inside the cell.
/// Bold strokes are a pixel thicker. Returns 1 when painted; 0 for other
/// characters, which are drawn from the font; -1 if painting panicked (a
/// bug), which fails the render.
///
/// # Safety
/// `cv` must point to a Canvas whose `px` holds its `h` rows of `w` pixels,
/// `stride` bytes apart (stride >= 3 * w), and whose geometry is null or
/// live; nothing else may use either during the call.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn termshot_paint_geometry(cv: *const Canvas, col: c_int, row: c_int, cell_w: c_int,
                                                 cell_h: c_int, cp: u32, bold: c_int, r: u8, g: u8, b: u8)
                                                 -> c_int {
    match paint_cell(&*cv, col, row, cell_w, cell_h, cp, bold != 0, [r, g, b]) {
        Some(true) => 1,
        Some(false) => 0,
        None => -1,
    }
}

/// termshot_paint_geometry, for src/glyphs.rs: Some(true) when painted,
/// Some(false) for other characters, and None if painting panicked (a bug),
/// which termshot_paint_failed then reports too.
///
/// # Safety
/// As termshot_paint_geometry.
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn paint_cell(cv: &Canvas, col: i32, row: i32, cell_w: i32, cell_h: i32, cp: u32, bold: bool,
                                c: Rgb) -> Option<bool> {
    if !(0x2500..=0x259F).contains(&cp) {
        return Some(false);
    }
    let geometry = cv.geometry.as_mut();
    guarded(|| {
        let px = Pixels::of(cv);
        let (arcs, stamps) = match geometry {
            Some(g) => (Some(&mut g.arc_offsets), g.stamps.as_mut()),
            None => (None, None),
        };
        let mut p = Painter { px, arcs, stamps };
        p.paint_geometry(col, row, cell_w, cell_h, cp, bold, c)
    })
}

/// Fills [x0, x1) x [y0, y1), clipped to the canvas, with (r, g, b).
///
/// # Safety
/// As termshot_paint_geometry; the geometry is not used.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn termshot_fill_rect(cv: *const Canvas, x0: c_int, y0: c_int, x1: c_int, y1: c_int, r: u8,
                                            g: u8, b: u8) {
    fill_rect(&*cv, x0, y0, x1, y1, [r, g, b]);
}

/// termshot_fill_rect, for src/glyphs.rs (underlines and the box of a
/// missing glyph).
///
/// # Safety
/// As termshot_fill_rect.
pub(crate) unsafe fn fill_rect(cv: &Canvas, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb) {
    // It writes only inside the rectangle it clipped, so it shouldn't panic;
    // if it does, termshot_paint_failed says so.
    guarded(|| Pixels::of(cv).fill_rect(x0, y0, x1, y1, c));
}

thread_local! {
    /// Whether a call here panicked on this thread since the render began
    /// (termshot_geometry_new).
    static PANICKED: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

/// f's result, or None if it panicked, which is remembered for
/// termshot_paint_failed. A panic must not unwind into C.
pub(crate) fn guarded<T>(f: impl FnOnce() -> T) -> Option<T> {
    let result = catch_unwind(AssertUnwindSafe(f)).ok();
    if result.is_none() {
        PANICKED.with(|p| p.set(true));
    }
    result
}

/// Nonzero if painting panicked (a bug) on this thread since
/// termshot_geometry_new began the render: termshot_fill_rect has no result
/// of its own, so the render asks once, before it writes the PNG, and fails the
/// render (exit 2) rather than write an incomplete image.
#[no_mangle]
pub extern "C" fn termshot_paint_failed() -> c_int {
    PANICKED.with(|p| p.get()) as c_int
}

pub(crate) type Rgb = [u8; 3];

/// Bytes in 16 pixels: shades blend that many at a time, against the
/// colour repeated as often, so the blend vectorizes on x86-64 too (a pixel
/// at a time, shaded blocks were 28% slower than GCC's C there).
const PATTERN: usize = 16 * BPP;

fn pixels_of(c: Rgb) -> [u8; PATTERN] {
    let mut pattern = [0; PATTERN];
    for p in pattern.chunks_exact_mut(BPP) {
        p.copy_from_slice(&c);
    }
    pattern
}

#[derive(Clone, Copy)]
struct Rect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

/// The canvas's pixels, borrowed for one call.
struct Pixels<'a> {
    px: &'a mut [u8],
    w: i32,
    h: i32,
    stride: usize,
    /// While set, put and fill_rect stay inside it: geometry never paints
    /// into a neighbouring cell.
    clip: Option<Rect>,
}

impl<'a> Pixels<'a> {
    /// # Safety
    /// As termshot_paint_geometry says of `cv`.
    unsafe fn of(cv: &'a Canvas) -> Pixels<'a> {
        if cv.px.is_null() || cv.w <= 0 || cv.h <= 0 {
            // Nothing is on such a canvas, so nothing is painted.
            return Pixels { px: &mut [], w: 0, h: 0, stride: 0, clip: None };
        }
        // The last row ends with its last pixel: filtered has no byte past it.
        let px = std::slice::from_raw_parts_mut(cv.px, (cv.h as usize - 1) * cv.stride + cv.w as usize * BPP);
        Pixels { px, w: cv.w, h: cv.h, stride: cv.stride, clip: None }
    }

    /// Whether [x0, x1) of row y is on the canvas.
    fn on_canvas(&self, x0: i32, x1: i32, y: i32) -> bool {
        0 <= x0 && x0 <= x1 && x1 <= self.w && 0 <= y && y < self.h
    }

    /// The pixels [x0, x1) of row y, which must be on the canvas.
    fn span(&mut self, x0: i32, x1: i32, y: i32) -> &mut [u8] {
        assert!(self.on_canvas(x0, x1, y), "span off the canvas");
        // SAFETY: just checked.
        unsafe { self.span_unchecked(x0, x1, y) }
    }

    /// # Safety
    /// `on_canvas(x0, x1, y)`.
    unsafe fn span_unchecked(&mut self, x0: i32, x1: i32, y: i32) -> &mut [u8] {
        debug_assert!(self.on_canvas(x0, x1, y));
        let from = y as usize * self.stride + x0 as usize * BPP;
        let len = (x1 - x0) as usize * BPP;
        // from + len <= (h - 1) * stride + w * BPP, the length of px
        // (Pixels::of), since y < h and x1 <= w.
        std::slice::from_raw_parts_mut(self.px.as_mut_ptr().add(from), len)
    }

    /// Paint [x0, x1) of row y in c. The spans of rectangles and kept
    /// strokes are known to be on the canvas, so a checked span per row cost
    /// box drawing up to 25% next to the C's plain loop.
    ///
    /// # Safety
    /// `on_canvas(x0, x1, y)`.
    unsafe fn fill_span(&mut self, x0: i32, x1: i32, y: i32, c: Rgb) {
        let span = self.span_unchecked(x0, x1, y);
        let mut p = span.as_mut_ptr();
        for _ in 0..span.len() / BPP {
            *p = c[0];
            *p.add(1) = c[1];
            *p.add(2) = c[2];
            p = p.add(BPP);
        }
    }

    fn put(&mut self, x: i32, y: i32, c: Rgb) {
        if (x as u32) >= (self.w as u32) || (y as u32) >= (self.h as u32) {
            return;
        }
        if let Some(cl) = self.clip {
            if x < cl.x0 || x >= cl.x1 || y < cl.y0 || y >= cl.y1 {
                return;
            }
        }
        // SAFETY: 0 <= x < w and 0 <= y < h.
        unsafe { self.fill_span(x, x + 1, y, c) };
    }

    /// Clip [x0, x1) x [y0, y1) to the canvas and, while clipped, to the
    /// clip; None when nothing is left.
    fn clip_rect(&self, mut x0: i32, mut y0: i32, mut x1: i32, mut y1: i32) -> Option<Rect> {
        x0 = x0.max(0);
        y0 = y0.max(0);
        x1 = x1.min(self.w);
        y1 = y1.min(self.h);
        if let Some(cl) = self.clip {
            x0 = x0.max(cl.x0);
            y0 = y0.max(cl.y0);
            x1 = x1.min(cl.x1);
            y1 = y1.min(cl.y1);
        }
        (x0 < x1 && y0 < y1).then_some(Rect { x0, y0, x1, y1 })
    }

    /// A shade: the colour at k quarters over what is painted, which is the
    /// cell's background unless an image under the text shows there.
    fn shade_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, k: i32, c: Rgb) {
        let Some(r) = self.clip_rect(x0, y0, x1, y1) else { return };
        // (s * k + d * (4 - k) + 2) / 4, as the C's int arithmetic, which is
        // never negative here, so u16 and a shift give the same.
        let (k, j) = (k as u16, (4 - k) as u16);
        let blend = |d: &mut u8, s: u8| *d = ((u16::from(s) * k + u16::from(*d) * j + 2) >> 2) as u8;
        let pattern = pixels_of(c);
        for y in r.y0..r.y1 {
            let mut blocks = self.span(r.x0, r.x1, y).chunks_exact_mut(PATTERN);
            for block in &mut blocks {
                for (d, &s) in block.iter_mut().zip(&pattern) {
                    blend(d, s);
                }
            }
            for p in blocks.into_remainder().chunks_exact_mut(BPP) {
                for (d, &s) in p.iter_mut().zip(&c) {
                    blend(d, s);
                }
            }
        }
    }

    fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgb) {
        let Some(r) = self.clip_rect(x0, y0, x1, y1) else { return };
        for y in r.y0..r.y1 {
            // SAFETY: clip_rect keeps the rectangle on the canvas.
            unsafe { self.fill_span(r.x0, r.x1, y, c) };
        }
    }

    fn hbar(&mut self, x0: i32, x1: i32, mid: i32, thick: i32, c: Rgb) {
        self.fill_rect(x0, mid - thick / 2, x1, mid - thick / 2 + thick, c);
    }

    fn vbar(&mut self, mid: i32, y0: i32, y1: i32, thick: i32, c: Rgb) {
        self.fill_rect(mid - thick / 2, y0, mid - thick / 2 + thick, y1, c);
    }

    /// Paint a kept stroke's runs into the clip, the cell of w x h pixels
    /// it was kept for, whose runs (stamp_points) are inside it.
    fn paint_runs(&mut self, runs: &[u16], w: i32, h: i32, c: Rgb) {
        let Some(cl) = self.clip else { return };
        assert!(cl.x1 - cl.x0 == w && cl.y1 - cl.y0 == h && self.on_canvas(cl.x0, cl.x1, cl.y0) && cl.y1 <= self.h);
        for run in runs.chunks_exact(3) {
            let (y, x0, x1) = (i32::from(run[0]), i32::from(run[1]), i32::from(run[2]));
            debug_assert!(y < h && x0 <= x1 && x1 <= w);
            // SAFETY: the run is inside the cell, which is on the canvas.
            unsafe { self.fill_span(cl.x0 + x0, cl.x0 + x1, cl.y0 + y, c) };
        }
    }
}

/// Where geometry allocates. The fault tests fail each in turn.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Site {
    /// The Geometry itself.
    Geometry,
    ArcOffsets,
    /// The rest are the Stamps' fields of the same names.
    Points,
    Ids,
    Sequences,
    Masks,
    Mask,
    Runs,
}

/// Allocation failure injection: the shipped binary has none. Unit tests
/// set it per thread; a build with `--cfg termshot_alloc_faults` reads
/// TERMSHOT_GEOMETRY_FAIL_AT=n and fails the nth allocation of each render
/// (tests/run.sh checks that the CLI still draws the same PNG).
#[cfg(any(test, termshot_alloc_faults))]
pub(super) mod faults {
    use super::Site;
    use std::cell::Cell;

    #[derive(Clone, Copy, Default)]
    pub(super) struct Faults {
        /// Allocations so far, and the one to fail (0 for none).
        pub(super) calls: u32,
        pub(super) fail_at: u32,
    }

    thread_local! {
        pub(super) static FAULTS: Cell<Faults> = Cell::new(Faults::default());
        #[cfg(test)]
        pub(super) static FAILED: Cell<Option<Site>> = Cell::new(None);
    }

    /// Starts a render: count from zero, and fail where asked.
    #[cfg(not(test))]
    pub(super) fn start() {
        let fail_at = std::env::var("TERMSHOT_GEOMETRY_FAIL_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        FAULTS.with(|f| f.set(Faults { calls: 0, fail_at }));
    }

    #[cfg(test)]
    pub(super) fn start() {}

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
                eprintln!("termshot: geometry allocation {} ({:?}) fails", s.calls, _site);
            }
            fail
        })
    }
}

#[cfg(not(any(test, termshot_alloc_faults)))]
mod faults {
    pub(super) fn start() {}
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

/// `v` resized to `len`, new elements `fill`; false, with `v` as it was,
/// when memory runs out. Each call is one allocation, as the C's malloc,
/// calloc or realloc was.
fn grow<T: Clone>(site: Site, v: &mut Vec<T>, len: usize, fill: T) -> bool {
    if !allowed(site) {
        return false;
    }
    if len > v.len() && v.try_reserve_exact(len - v.len()).is_err() {
        return false;
    }
    v.resize(len, fill);
    true
}

/// A new vector of `len` elements `fill`, or None when memory runs out.
fn filled<T: Clone>(site: Site, len: usize, fill: T) -> Option<Vec<T>> {
    let mut v = Vec::new();
    grow(site, &mut v, len, fill).then_some(v)
}

/* Rounded corners and diagonals are stamped: a disc at each of a stroke's
   points. Stamps keeps which pixels of its cell a stroke covered, to paint
   the next one like it as runs; stamp_points says when two are alike. A
   render of the largest screen finds well under STAMP_SEQUENCES sequences
   per shape and axis (tests/boxes.c checks), and every allocation of the
   cache stays within STAMP_MAX_BYTES in all (the unit tests check); past
   either, strokes are stamped afresh. */

/// The four corners, then the two diagonals.
const STAMP_SHAPES: usize = 6;
const STAMP_SEQUENCES: i32 = 64;
const STAMP_MASKS: usize = 1024;
const STAMP_MAX_POINTS: i32 = 4096;
/// The cache's budget, across all its allocations.
const STAMP_MAX_BYTES: usize = 4 << 20;

/// A kept stroke: its key (0 for an empty slot) and the runs it covers, 3 per
/// run: row, first column and past the last, in the cell.
#[derive(Clone, Default)]
struct StampMask {
    key: u32,
    runs: Vec<u16>,
}

struct Stamps {
    /// The cell size, set by the first stroke.
    w: i32,
    h: i32,
    /// Each shape's point count.
    n: [i32; STAMP_SHAPES],
    /// Columns and rows.
    lines: [i32; 2],
    /// Per axis (x, y), shape and column or row: the sequence of offsets the
    /// stroke there has, as 1 + its index; 0 not known yet, -1 none.
    ids: [[Option<Vec<i16>>; STAMP_SHAPES]; 2],
    /// Per axis and shape: the distinct sequences, n floats each, with room
    /// for sequence_room of them.
    sequences: [[Vec<f32>; STAMP_SHAPES]; 2],
    sequence_count: [[i32; STAMP_SHAPES]; 2],
    sequence_room: [[i32; STAMP_SHAPES]; 2],
    /// STAMP_MASKS slots once a stroke is kept; empty before.
    masks: Vec<StampMask>,
    /// The stroke being stamped: x, y; room for `capacity` points.
    points: Vec<f32>,
    capacity: i32,
    /// A cell's coverage, on a miss; empty before the first.
    mask: Vec<u8>,
    hits: usize,
    misses: usize,
    uncached: usize,
    /// Bytes held now, and at most.
    bytes: usize,
    peak: usize,
    /// STAMP_MAX_BYTES, but in the tests that shrink it.
    max_bytes: usize,
}

impl Stamps {
    fn new() -> Stamps {
        Stamps {
            w: 0,
            h: 0,
            n: [0; STAMP_SHAPES],
            lines: [0; 2],
            ids: Default::default(),
            sequences: Default::default(),
            sequence_count: [[0; STAMP_SHAPES]; 2],
            sequence_room: [[0; STAMP_SHAPES]; 2],
            masks: Vec::new(),
            points: Vec::new(),
            capacity: 0,
            mask: Vec::new(),
            hits: 0,
            misses: 0,
            uncached: 0,
            bytes: 0,
            peak: 0,
            max_bytes: STAMP_MAX_BYTES,
        }
    }

    fn stats(&self) -> GeometryStats {
        GeometryStats { hits: self.hits, misses: self.misses, uncached: self.uncached, bytes: self.bytes, peak: self.peak }
    }

    fn hold(&mut self, bytes: usize) {
        self.bytes += bytes;
        if self.bytes > self.peak {
            self.peak = self.bytes;
        }
    }

    /// Whether the cache may allocate bytes more.
    fn budget(&self, bytes: usize) -> bool {
        bytes <= self.max_bytes && self.bytes <= self.max_bytes - bytes
    }

    /// The cell being stamped, `clip`: its column and row in a grid of cells
    /// of the first stroke's size on a canvas of `canvas_w` x `canvas_h`.
    /// None when strokes there are not reused.
    fn cell(&mut self, clip: Rect, canvas_w: i32, canvas_h: i32, shape: usize, n: i32) -> Option<(i32, i32)> {
        let (w, h) = (clip.x1 - clip.x0, clip.y1 - clip.y0);
        if n > STAMP_MAX_POINTS || w <= 0 || h <= 0 || w > 65535 || h > 65535 {
            return None;
        }
        if self.w == 0 {
            self.w = w;
            self.h = h;
            self.lines = [canvas_w / w, canvas_h / h];
        }
        if w != self.w || h != self.h || (self.n[shape] != 0 && self.n[shape] != n) {
            return None;
        }
        self.n[shape] = n;
        if clip.x0 < 0 || clip.y0 < 0 || clip.x0 % w != 0 || clip.y0 % h != 0 {
            return None;
        }
        let (col, row) = (clip.x0 / w, clip.y0 / h);
        (col < self.lines[0] && row < self.lines[1]).then_some((col, row))
    }

    /// The key of shape's stroke at col and row, at thickness thick: 0 when
    /// it isn't known yet, -1 when it is not reused.
    fn key(&self, shape: usize, col: i32, row: i32, thick: i32) -> i64 {
        let (Some(xs), Some(ys)) = (&self.ids[0][shape], &self.ids[1][shape]) else { return 0 };
        let (x, y) = (i64::from(xs[col as usize]), i64::from(ys[row as usize]));
        if x < 0 || y < 0 || thick >= 256 {
            return -1;
        }
        if x == 0 || y == 0 {
            return 0;
        }
        let seq = i64::from(STAMP_SEQUENCES) + 1;
        1 + (((shape as i64 * 256 + i64::from(thick)) * seq + x) * seq + y)
    }

    /// The index + 1 of the sequence of offsets of the n points (stride 2,
    /// from `axis`) from origin, kept for shape and axis; -1 when an offset
    /// is inexact or there is no room.
    fn sequence(&mut self, axis: usize, shape: usize, n: i32, origin: f32) -> i32 {
        let (count, room) = (self.sequence_count[axis][shape], self.sequence_room[axis][shape]);
        let n = n as usize;
        if count + 1 > room {
            // Room for one more row than is kept, for the one being compared.
            let more = if room != 0 { 2 * room } else { 4 }.min(STAMP_SEQUENCES + 1);
            let bytes = (more - room) as usize * n * std::mem::size_of::<f32>();
            if !self.budget(bytes) || !grow(Site::Sequences, &mut self.sequences[axis][shape], more as usize * n, 0.0) {
                return -1;
            }
            self.sequence_room[axis][shape] = more;
            self.hold(bytes);
        }
        let seen = &mut self.sequences[axis][shape];
        let (kept, rest) = seen.split_at_mut(count as usize * n);
        let d = &mut rest[..n];
        for (i, out) in d.iter_mut().enumerate() {
            match exact_difference(self.points[2 * i + axis], origin) {
                Some(diff) => *out = diff,
                None => return -1,
            }
        }
        // Bit for bit, as memcmp did.
        let same = |k: usize| kept[k * n..(k + 1) * n].iter().zip(d.iter()).all(|(a, b)| a.to_bits() == b.to_bits());
        if let Some(k) = (0..count as usize).find(|&k| same(k)) {
            return k as i32 + 1;
        }
        if count == STAMP_SEQUENCES {
            return -1;
        }
        self.sequence_count[axis][shape] = count + 1;
        count + 1
    }
}

/// a - b, if that is exact: Knuth's TwoSum error is zero.
fn exact_difference(a: f32, b: f32) -> Option<f32> {
    let s = a - b;
    let bv = s - a;
    let av = s - bv;
    let err = (a - av) + (-b - bv);
    (err == 0.0).then_some(s)
}

/// Of the slot a key goes in.
fn stamp_slot(key: u32) -> usize {
    (key.wrapping_mul(2654435761) >> 22) as usize & (STAMP_MASKS - 1)
}

/// What stamp_find says to do.
#[derive(PartialEq)]
enum Found {
    /// Painted from a stroke like it.
    Painted,
    /// Put the points in Stamps::points (there is room for n) and call
    /// stamp_points.
    Points,
    /// Stamp them without the cache.
    Uncached,
}

struct Painter<'a> {
    px: Pixels<'a>,
    arcs: Option<&'a mut [Option<Vec<f32>>; 4]>,
    stamps: Option<&'a mut Stamps>,
}

impl Painter<'_> {
    /// Paint shape's stroke in this cell from a stroke like it, if one was
    /// kept.
    fn stamp_find(&mut self, shape: usize, n: i32, thick: i32, c: Rgb) -> Found {
        let (Some(st), Some(clip)) = (self.stamps.as_deref_mut(), self.px.clip) else { return Found::Uncached };
        let Some((col, row)) = st.cell(clip, self.px.w, self.px.h, shape, n) else {
            st.uncached += 1;
            return Found::Uncached;
        };
        let key = st.key(shape, col, row, thick);
        if key < 0 {
            st.uncached += 1;
            return Found::Uncached;
        }
        if key != 0 && !st.masks.is_empty() {
            let m = &st.masks[stamp_slot(key as u32)];
            if m.key == key as u32 {
                st.hits += 1;
                self.px.paint_runs(&m.runs, st.w, st.h, c);
                return Found::Painted;
            }
        }
        if n > st.capacity {
            let bytes = (n - st.capacity) as usize * 2 * std::mem::size_of::<f32>();
            if !st.budget(bytes) || !grow(Site::Points, &mut st.points, n as usize * 2, 0.0) {
                st.uncached += 1;
                return Found::Uncached;
            }
            st.hold(bytes);
            st.capacity = n;
        }
        Found::Points
    }

    /// Stamp the n points in Stamps::points with discs of squared radius
    /// rad2 (radius rad) in the clip, which stamp_find found to be a cell,
    /// as put would, and keep what they cover. True when painted, false when
    /// the caller must.
    ///
    /// Which pixels a disc covers depends only on (x + 0.5f) - px and
    /// (y + 0.5f) - py for the pixel (x, y) and the point (px, py). Moved by
    /// whole pixels, x + 0.5f stays exact, so if each of a stroke's points is
    /// exactly as far from its cell's origin as the same point of a stroke
    /// kept before, each difference is the same real number, rounds the
    /// same, and the stroke covers the same pixels of its cell. That is
    /// checked, not assumed: how a point rounds depends on where its cell is.
    /// A stroke's x offsets depend only on its column, and its y offsets on
    /// its row, so each is found once.
    #[allow(clippy::too_many_arguments)]
    fn stamp_points(&mut self, shape: usize, n: i32, thick: i32, rad: f32, rad2: f32, c: Rgb) -> bool {
        let (Some(st), Some(clip)) = (self.stamps.as_deref_mut(), self.px.clip) else { return false };
        let (w, h) = (st.w, st.h);
        let (col, row) = (clip.x0 / w, clip.y0 / h);
        for axis in 0..2 {
            if st.ids[axis][shape].is_none() {
                let lines = st.lines[axis] as usize;
                let bytes = lines * std::mem::size_of::<i16>();
                let ids = if st.budget(bytes) { filled(Site::Ids, lines, 0i16) } else { None };
                if ids.is_none() {
                    st.uncached += 1;
                    return false;
                }
                st.ids[axis][shape] = ids;
                st.hold(bytes);
            }
            let line = if axis == 1 { row } else { col } as usize;
            if st.ids[axis][shape].as_ref().map_or(0, |ids| ids[line]) == 0 {
                let origin = (if axis == 1 { clip.y0 } else { clip.x0 }) as f32;
                let id = st.sequence(axis, shape, n, origin) as i16;
                if let Some(ids) = st.ids[axis][shape].as_mut() {
                    ids[line] = id;
                }
            }
        }
        let key = st.key(shape, col, row, thick);
        if key <= 0 {
            st.uncached += 1;
            return false;
        }
        if st.masks.is_empty() {
            let bytes = STAMP_MASKS * std::mem::size_of::<StampMask>();
            if !st.budget(bytes) || !grow(Site::Masks, &mut st.masks, STAMP_MASKS, StampMask::default()) {
                st.uncached += 1;
                return false;
            }
            st.hold(bytes);
        }
        let slot = stamp_slot(key as u32);
        if st.masks[slot].key == key as u32 {
            st.hits += 1;
            self.px.paint_runs(&st.masks[slot].runs, w, h, c);
            return true;
        }
        let size = w as usize * h as usize;
        if st.mask.is_empty() {
            if !st.budget(size) || !grow(Site::Mask, &mut st.mask, size, 0u8) {
                st.uncached += 1;
                return false;
            }
            st.hold(size);
        }
        // Stamp into a mask of the cell what the caller would into the canvas.
        let (cx, cy) = (clip.x0, clip.y0);
        let mask = &mut st.mask[..size];
        mask.fill(0);
        for p in st.points[..2 * n as usize].chunks_exact(2) {
            let (px, py) = (p[0], p[1]);
            let x0 = ((px - rad - 1.0).floor() as i32).max(cx);
            let y0 = ((py - rad - 1.0).floor() as i32).max(cy);
            let x1 = ((px + rad + 1.0).ceil() as i32).min(cx + w - 1);
            let y1 = ((py + rad + 1.0).ceil() as i32).min(cy + h - 1);
            if x0 > x1 {
                continue;
            }
            for y in y0..=y1 {
                let line = &mut mask[(y - cy) as usize * w as usize..][..w as usize];
                for (m, x) in line[(x0 - cx) as usize..=(x1 - cx) as usize].iter_mut().zip(x0..) {
                    let dx = (x as f32 + 0.5) - px;
                    let dy = (y as f32 + 0.5) - py;
                    if dx * dx + dy * dy <= rad2 {
                        *m = 1;
                    }
                }
            }
        }
        let mask = &st.mask[..size];
        let lines = || mask.chunks_exact(w as usize);
        let count: usize =
            lines().map(|line| (0..line.len()).filter(|&x| line[x] != 0 && (x == 0 || line[x - 1] == 0)).count()).sum();
        let bytes = count.max(1) * 3 * std::mem::size_of::<u16>();
        let mut runs = Vec::new();
        if !(st.budget(bytes) && allowed(Site::Runs) && runs.try_reserve_exact(3 * count).is_ok()) {
            // Not kept: paint from the mask all the same.
            st.uncached += 1;
            for (y, line) in lines().enumerate() {
                for (p, &on) in self.px.span(cx, cx + w, cy + y as i32).chunks_exact_mut(BPP).zip(line) {
                    if on != 0 {
                        p.copy_from_slice(&c);
                    }
                }
            }
            return true;
        }
        for (y, line) in lines().enumerate() {
            let mut x = 0;
            while x < line.len() {
                if line[x] == 0 {
                    x += 1;
                    continue;
                }
                let mut end = x;
                while end < line.len() && line[end] != 0 {
                    end += 1;
                }
                runs.extend_from_slice(&[y as u16, x as u16, end as u16]);
                x = end;
            }
        }
        let old = &st.masks[slot];
        if old.key != 0 {
            st.bytes -= (old.runs.len() / 3).max(1) * 3 * std::mem::size_of::<u16>();
        }
        st.masks[slot] = StampMask { key: key as u32, runs };
        st.hold(bytes);
        st.misses += 1;
        self.px.paint_runs(&st.masks[slot].runs, w, h, c);
        true
    }
}

/* Box drawing, U+2500..U+257F: two bits per arm, (left, up, right, down),
   each 0 none, 1 light (or "single"), 2 heavy, 3 double. Dashes, arcs and
   diagonals are 0 here and drawn by their own code. tests/boxes.c checks this
   table against the Unicode character names. */
const NO: u8 = 0;
const LT: u8 = 1;
const HV: u8 = 2;
const DB: u8 = 3;

const fn arms(l: u8, u: u8, r: u8, d: u8) -> u8 {
    l | u << 2 | r << 4 | d << 6
}

#[rustfmt::skip]
static BOX_ARMS: [u8; 128] = [
    arms(LT, NO, LT, NO), arms(HV, NO, HV, NO), arms(NO, LT, NO, LT), arms(NO, HV, NO, HV), // 2500 ─━│┃
    0, 0, 0, 0, 0, 0, 0, 0,                                                               // 2504 dashes
    arms(NO, NO, LT, LT), arms(NO, NO, HV, LT), arms(NO, NO, LT, HV), arms(NO, NO, HV, HV), // 250C ┌┍┎┏
    arms(LT, NO, NO, LT), arms(HV, NO, NO, LT), arms(LT, NO, NO, HV), arms(HV, NO, NO, HV), // 2510 ┐┑┒┓
    arms(NO, LT, LT, NO), arms(NO, LT, HV, NO), arms(NO, HV, LT, NO), arms(NO, HV, HV, NO), // 2514 └┕┖┗
    arms(LT, LT, NO, NO), arms(HV, LT, NO, NO), arms(LT, HV, NO, NO), arms(HV, HV, NO, NO), // 2518 ┘┙┚┛
    arms(NO, LT, LT, LT), arms(NO, LT, HV, LT), arms(NO, HV, LT, LT), arms(NO, LT, LT, HV), // 251C ├┝┞┟
    arms(NO, HV, LT, HV), arms(NO, HV, HV, LT), arms(NO, LT, HV, HV), arms(NO, HV, HV, HV), // 2520 ┠┡┢┣
    arms(LT, LT, NO, LT), arms(HV, LT, NO, LT), arms(LT, HV, NO, LT), arms(LT, LT, NO, HV), // 2524 ┤┥┦┧
    arms(LT, HV, NO, HV), arms(HV, HV, NO, LT), arms(HV, LT, NO, HV), arms(HV, HV, NO, HV), // 2528 ┨┩┪┫
    arms(LT, NO, LT, LT), arms(HV, NO, LT, LT), arms(LT, NO, HV, LT), arms(HV, NO, HV, LT), // 252C ┬┭┮┯
    arms(LT, NO, LT, HV), arms(HV, NO, LT, HV), arms(LT, NO, HV, HV), arms(HV, NO, HV, HV), // 2530 ┰┱┲┳
    arms(LT, LT, LT, NO), arms(HV, LT, LT, NO), arms(LT, LT, HV, NO), arms(HV, LT, HV, NO), // 2534 ┴┵┶┷
    arms(LT, HV, LT, NO), arms(HV, HV, LT, NO), arms(LT, HV, HV, NO), arms(HV, HV, HV, NO), // 2538 ┸┹┺┻
    arms(LT, LT, LT, LT), arms(HV, LT, LT, LT), arms(LT, LT, HV, LT), arms(HV, LT, HV, LT), // 253C ┼┽┾┿
    arms(LT, HV, LT, LT), arms(LT, LT, LT, HV), arms(LT, HV, LT, HV), arms(HV, HV, LT, LT), // 2540 ╀╁╂╃
    arms(LT, HV, HV, LT), arms(HV, LT, LT, HV), arms(LT, LT, HV, HV), arms(HV, HV, HV, LT), // 2544 ╄╅╆╇
    arms(HV, LT, HV, HV), arms(HV, HV, LT, HV), arms(LT, HV, HV, HV), arms(HV, HV, HV, HV), // 2548 ╈╉╊╋
    0, 0, 0, 0,                                                                           // 254C dashes
    arms(DB, NO, DB, NO), arms(NO, DB, NO, DB), arms(NO, NO, DB, LT), arms(NO, NO, LT, DB), // 2550 ═║╒╓
    arms(NO, NO, DB, DB), arms(DB, NO, NO, LT), arms(LT, NO, NO, DB), arms(DB, NO, NO, DB), // 2554 ╔╕╖╗
    arms(NO, LT, DB, NO), arms(NO, DB, LT, NO), arms(NO, DB, DB, NO), arms(DB, LT, NO, NO), // 2558 ╘╙╚╛
    arms(LT, DB, NO, NO), arms(DB, DB, NO, NO), arms(NO, LT, DB, LT), arms(NO, DB, LT, DB), // 255C ╜╝╞╟
    arms(NO, DB, DB, DB), arms(DB, LT, NO, LT), arms(LT, DB, NO, DB), arms(DB, DB, NO, DB), // 2560 ╠╡╢╣
    arms(DB, NO, DB, LT), arms(LT, NO, LT, DB), arms(DB, NO, DB, DB), arms(DB, LT, DB, NO), // 2564 ╤╥╦╧
    arms(LT, DB, LT, NO), arms(DB, DB, DB, NO), arms(DB, LT, DB, LT), arms(LT, DB, LT, DB), // 2568 ╨╩╪╫
    arms(DB, DB, DB, DB), 0, 0, 0,                                                        // 256C ╬, arcs
    0, 0, 0, 0,                                                                           // 2570 arc, diagonals
    arms(LT, NO, NO, NO), arms(NO, LT, NO, NO), arms(NO, NO, LT, NO), arms(NO, NO, NO, LT), // 2574 ╴╵╶╷
    arms(HV, NO, NO, NO), arms(NO, HV, NO, NO), arms(NO, NO, HV, NO), arms(NO, NO, NO, HV), // 2578 ╸╹╺╻
    arms(LT, NO, HV, NO), arms(NO, LT, NO, HV), arms(HV, NO, LT, NO), arms(NO, HV, NO, LT), // 257C ╼╽╾╿
];

fn arm_width(weight: i32, t: i32) -> i32 {
    if weight == i32::from(NO) {
        0
    } else if weight == i32::from(HV) {
        2 * t
    } else {
        t
    }
}

/// Stamp a disc of squared radius rad2 (radius rad) at (px, py) as put
/// would, inside `bound` ([x0, x1] x [y0, y1]) when there is one.
fn stamp_disc(pixels: &mut Pixels, px: f32, py: f32, rad: f32, rad2: f32, bound: Option<Rect>, c: Rgb) {
    let mut x0 = (px - rad - 1.0).floor() as i32;
    let mut y0 = (py - rad - 1.0).floor() as i32;
    let mut x1 = (px + rad + 1.0).ceil() as i32;
    let mut y1 = (py + rad + 1.0).ceil() as i32;
    if let Some(b) = bound {
        x0 = x0.max(b.x0);
        y0 = y0.max(b.y0);
        x1 = x1.min(b.x1);
        y1 = y1.min(b.y1);
    }
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = (x as f32 + 0.5) - px;
            let dy = (y as f32 + 0.5) - py;
            if dx * dx + dy * dy <= rad2 {
                pixels.put(x, y, c);
            }
        }
    }
}

impl Painter<'_> {
    /// Quarter ellipse. Angles are standard math angles with y growing
    /// downward.
    #[allow(clippy::too_many_arguments)]
    fn arc(&mut self, corner: usize, cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32, thick: f32, c: Rgb) {
        let steps = (((rx + ry) * 2.0) as i32).max(12);
        // Cache only the translation-independent products. Add cx/cy at the
        // original position, preserving the original float rounding of
        // pixels. Cap storage for unusual font metrics; allocation failure
        // uses the old path. The offsets leave the cache while they are
        // used, and go back after.
        let mut offsets = self.arcs.as_deref_mut().and_then(|arcs| arcs[corner].take());
        if offsets.is_none() && steps <= 2048 && self.arcs.is_some() {
            offsets = filled(Site::ArcOffsets, (steps as usize + 1) * 2, 0.0f32).map(|mut o| {
                for i in 0..=steps {
                    let (s, k) = arc_point(a0, a1, i, steps);
                    o[2 * i as usize] = rx * k;
                    o[2 * i as usize + 1] = ry * s;
                }
                o
            });
        }
        self.stroke_arc(corner, cx, cy, rx, ry, a0, a1, thick, steps, offsets.as_deref(), c);
        if let (Some(arcs), Some(o)) = (self.arcs.as_deref_mut(), offsets) {
            arcs[corner] = Some(o);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stroke_arc(&mut self, corner: usize, cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32, thick: f32,
                  steps: i32, offsets: Option<&[f32]>, c: Rgb) {
        let rad = thick * 0.5;
        let rad2 = (rad + 0.6) * (rad + 0.6);
        let n = steps as usize + 1;
        let found = if offsets.is_some() { self.stamp_find(corner, steps + 1, thick as i32, c) } else { Found::Uncached };
        if found == Found::Painted {
            return;
        }
        if let (Found::Points, Some(o)) = (&found, offsets) {
            if let Some(st) = self.stamps.as_deref_mut() {
                for (p, o) in st.points[..2 * n].chunks_exact_mut(2).zip(o.chunks_exact(2)) {
                    p[0] = cx + o[0];
                    p[1] = cy + o[1];
                }
            }
            if self.stamp_points(corner, steps + 1, thick as i32, rad, rad2, c) {
                return;
            }
        }
        for i in 0..=steps {
            let (ox, oy) = match offsets {
                Some(o) => (o[2 * i as usize], o[2 * i as usize + 1]),
                None => {
                    let (s, k) = arc_point(a0, a1, i, steps);
                    (rx * k, ry * s)
                }
            };
            let px = cx + ox;
            let py = cy + oy;
            stamp_disc(&mut self.px, px, py, rad, rad2, None, c);
        }
    }

    /// Light and heavy lines meeting at the centre. Each arm runs from the
    /// cell edge across the centre to the far side of the widest crossing
    /// stroke, so joins are solid and lines continue straight into the next
    /// cell.
    #[allow(clippy::too_many_arguments)]
    fn paint_lines(&mut self, x: i32, y: i32, w: i32, h: i32, arms: i32, t: i32, c: Rgb) {
        let (left, up, right, down) = (arms & 3, arms >> 2 & 3, arms >> 4 & 3, arms >> 6 & 3);
        let (cx, cy) = (x + w / 2, y + h / 2);
        let (wl, wu, wr, wd) = (arm_width(left, t), arm_width(up, t), arm_width(right, t), arm_width(down, t));
        let (vmax, hmax) = (wu.max(wd), wl.max(wr));
        let p = &mut self.px;
        if left != 0 {
            p.hbar(x, if vmax != 0 { cx - vmax / 2 + vmax } else { cx - wl / 2 + wl }, cy, wl, c);
        }
        if right != 0 {
            p.hbar(if vmax != 0 { cx - vmax / 2 } else { cx - wr / 2 }, x + w, cy, wr, c);
        }
        if up != 0 {
            p.vbar(cx, y, if hmax != 0 { cy - hmax / 2 + hmax } else { cy - wu / 2 + wu }, wu, c);
        }
        if down != 0 {
            p.vbar(cx, if hmax != 0 { cy - hmax / 2 } else { cy - wd / 2 }, y + h, wd, c);
        }
    }

    /// Double lines, alone or meeting single (light) ones. A double arm is
    /// two strokes of width t with a gap of t, narrowed when needed so the
    /// pair keeps a clear pixel on each side of the cell. Where double arms
    /// meet they form inner and outer corners; a single arm meeting a double
    /// line stops at the near stroke, or crosses both when it turns a corner
    /// or crosses straight over.
    #[allow(clippy::too_many_arguments)]
    fn paint_double(&mut self, x: i32, y: i32, w: i32, h: i32, arms: i32, t: i32, c: Rgb) {
        let (left, up, right, down) = (arms & 3, arms >> 2 & 3, arms >> 4 & 3, arms >> 6 & 3);
        let (lt, db) = (i32::from(LT), i32::from(DB));
        let (cx, cy) = (x + w / 2, y + h / 2);
        let room = w.min(h) - 2;
        let (mut dt, mut gap) = (t, t);
        while 2 * dt + gap > room && gap > 1 {
            gap -= 1;
        }
        while 2 * dt + gap > room && dt > 1 {
            dt -= 1;
        }
        let span = 2 * dt + gap;
        let (bx, by) = (cx - span / 2, cy - span / 2); // first stroke of a double line
        let (bx2, by2) = (bx + dt + gap, by + dt + gap); // second stroke
        let (st, sb) = (cx - t / 2, cy - t / 2); // a single line
        let v2 = up == db || down == db;
        let h2 = left == db || right == db;
        let v1 = up == lt || down == lt;
        let p = &mut self.px;
        if left == db || right == db {
            let (mut stop, mut start) = ([0; 2], [0; 2]); // per stroke: [0] upper, [1] lower
            for s in 0..2 {
                let toward = if s == 0 { up } else { down }; // the vertical arm on this stroke's side
                if v2 {
                    stop[s] = if toward == db { bx + dt } else { bx + span };
                    start[s] = if toward == db { bx2 } else { bx };
                } else if left == db && right == db {
                    stop[s] = bx + span;
                    start[s] = bx;
                } else if v1 {
                    stop[s] = st + t;
                    start[s] = st;
                } else {
                    stop[s] = bx + span;
                    start[s] = bx;
                }
            }
            if left == db {
                p.fill_rect(x, by, stop[0], by + dt, c);
                p.fill_rect(x, by2, stop[1], by2 + dt, c);
            }
            if right == db {
                p.fill_rect(start[0], by, x + w, by + dt, c);
                p.fill_rect(start[1], by2, x + w, by2 + dt, c);
            }
        }
        if up == db || down == db {
            let (mut stop, mut start) = ([0; 2], [0; 2]); // per stroke: [0] left, [1] right
            for s in 0..2 {
                let toward = if s == 0 { left } else { right };
                if h2 {
                    stop[s] = if toward == db { by + dt } else { by + span };
                    start[s] = if toward == db { by2 } else { by };
                } else if up == db && down == db {
                    stop[s] = by + span;
                    start[s] = by;
                } else if left == lt || right == lt {
                    stop[s] = sb + t;
                    start[s] = sb;
                } else {
                    stop[s] = by + span;
                    start[s] = by;
                }
            }
            if up == db {
                p.fill_rect(bx, y, bx + dt, stop[0], c);
                p.fill_rect(bx2, y, bx2 + dt, stop[1], c);
            }
            if down == db {
                p.fill_rect(bx, start[0], bx + dt, y + h, c);
                p.fill_rect(bx2, start[1], bx2 + dt, y + h, c);
            }
        }
        // Single arms.
        let (cross_h, cross_v) = (left == lt && right == lt, up == lt && down == lt);
        let (along_v, along_h) = (up == db && down == db, left == db && right == db);
        if left == lt {
            p.hbar(x, if cross_h || !along_v { bx + span } else { bx + dt }, cy, t, c);
        }
        if right == lt {
            p.hbar(if cross_h || !along_v { bx } else { bx2 }, x + w, cy, t, c);
        }
        if up == lt {
            p.vbar(cx, y, if cross_v || !along_h { by + span } else { by + dt }, t, c);
        }
        if down == lt {
            p.vbar(cx, if cross_v || !along_h { by } else { by2 }, y + h, t, c);
        }
    }

    /// Dashed lines: n segments, evenly spaced, with half a gap at each end
    /// so that a row of dashed cells keeps one rhythm.
    #[allow(clippy::too_many_arguments)]
    fn paint_dashes(&mut self, x: i32, y: i32, w: i32, h: i32, vertical: bool, n: i32, thick: i32, c: Rgb) {
        let len = if vertical { h } else { w };
        for i in 0..n {
            let (a, e) = (len * i / n, len * (i + 1) / n);
            let slot = e - a;
            // At least 2, so each end of the cell keeps a clear pixel.
            let gap = if slot / 3 >= 2 {
                slot / 3
            } else if slot >= 3 {
                2
            } else {
                slot - 1
            };
            let (s0, s1) = (a + gap / 2, e - (gap - gap / 2));
            if vertical {
                self.px.vbar(x + w / 2, y + s0, y + s1, thick, c);
            } else {
                self.px.hbar(x + s0, x + s1, y + h / 2, thick, c);
            }
        }
    }

    /// A straight stroke from (ax, ay) to (bx, by), stamped like the arcs
    /// but clipped to the cell so it never paints a neighbour.
    #[allow(clippy::too_many_arguments)]
    fn paint_segment(&mut self, x: i32, y: i32, w: i32, h: i32, ax: f32, ay: f32, bx: f32, by: f32, thick: f32,
                     c: Rgb) {
        let len = ((bx - ax) * (bx - ax) + (by - ay) * (by - ay)).sqrt();
        let steps = (len * 2.0) as i32 + 1;
        let rad = thick * 0.5;
        let rad2 = (rad + 0.6) * (rad + 0.6);
        // Shapes 4 and 5: from the top right (as ╱) or the top left.
        let shape = if ax > x as f32 { 4 } else { 5 };
        let point = |i: i32| (ax + (bx - ax) * (i as f32 / steps as f32), ay + (by - ay) * (i as f32 / steps as f32));
        let found = self.stamp_find(shape, steps + 1, thick as i32, c);
        if found == Found::Painted {
            return;
        }
        if found == Found::Points {
            if let Some(st) = self.stamps.as_deref_mut() {
                for (i, p) in st.points[..2 * (steps as usize + 1)].chunks_exact_mut(2).enumerate() {
                    let (px, py) = point(i as i32);
                    p[0] = px;
                    p[1] = py;
                }
            }
            if self.stamp_points(shape, steps + 1, thick as i32, rad, rad2, c) {
                return;
            }
        }
        let cell = Rect { x0: x, y0: y, x1: x + w - 1, y1: y + h - 1 };
        for i in 0..=steps {
            let (px, py) = point(i);
            stamp_disc(&mut self.px, px, py, rad, rad2, Some(cell), c);
        }
    }

    /// Block elements, U+2580..U+259F. Eighths round down from the top or
    /// left edge, so a block and its complement (upper and lower half, left
    /// and right half) fill the cell exactly. Shades blend the colour over
    /// what is painted instead of dithering.
    #[allow(clippy::too_many_arguments)]
    fn paint_block(&mut self, x: i32, y: i32, w: i32, h: i32, cp: u32, c: Rgb) -> bool {
        let (right, bottom, mx, my) = (x + w, y + h, x + w / 2, y + h / 2);
        let p = &mut self.px;
        match cp {
            0x2580 => p.fill_rect(x, y, right, my, c),
            0x2581..=0x2588 => {
                // lower n/8, n = 1..8
                let n = (cp - 0x2580) as i32;
                p.fill_rect(x, y + h * (8 - n) / 8, right, bottom, c);
            }
            0x2589..=0x258F => {
                // left n/8, n = 7..1
                let n = 8 - (cp - 0x2588) as i32;
                p.fill_rect(x, y, x + w * n / 8, bottom, c);
            }
            0x2590 => p.fill_rect(mx, y, right, bottom, c),
            // light, medium, dark shade
            0x2591..=0x2593 => p.shade_rect(x, y, right, bottom, (cp - 0x2590) as i32, c),
            0x2594 => p.fill_rect(x, y, right, y + h / 8, c),
            0x2595 => p.fill_rect(x + w * 7 / 8, y, right, bottom, c),
            0x2596..=0x259F => {
                // Quadrant bits: 1 upper left, 2 upper right, 4 lower left, 8 lower right.
                const QUADS: [u8; 10] = [4, 8, 1, 1 | 4 | 8, 1 | 8, 1 | 2 | 4, 1 | 2 | 8, 2, 2 | 4, 2 | 4 | 8];
                let q = QUADS[(cp - 0x2596) as usize];
                if q & 1 != 0 {
                    p.fill_rect(x, y, mx, my, c);
                }
                if q & 2 != 0 {
                    p.fill_rect(mx, y, right, my, c);
                }
                if q & 4 != 0 {
                    p.fill_rect(x, my, mx, bottom, c);
                }
                if q & 8 != 0 {
                    p.fill_rect(mx, my, right, bottom, c);
                }
            }
            _ => return false,
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_cell_geometry(&mut self, col: i32, row: i32, cell_w: i32, cell_h: i32, cp: u32, bold: bool, c: Rgb)
                           -> bool {
        let x = col * cell_w;
        let y = row * cell_h;
        if (0x2580..=0x259F).contains(&cp) {
            return self.paint_block(x, y, cell_w, cell_h, cp, c);
        }
        if !(0x2500..=0x257F).contains(&cp) {
            return false;
        }
        let right = x + cell_w;
        let bottom = y + cell_h;
        let mut t = (cell_w / 12).max(1);
        if bold {
            t += 1;
        }
        // As the C's float literal 3.14159265f.
        #[allow(clippy::approx_constant)]
        let pi = 3.14159265f32;
        let (rx, ry, tf) = (cell_w as f32 * 0.5, cell_h as f32 * 0.5, t as f32);
        let (xf, yf, rightf, bottomf) = (x as f32, y as f32, right as f32, bottom as f32);
        match cp {
            0x256D => self.arc(0, rightf, bottomf, rx, ry, pi, pi * 1.5, tf, c), // ╭ arc down and right
            0x256E => self.arc(1, xf, bottomf, rx, ry, -pi * 0.5, 0.0, tf, c),   // ╮
            0x256F => self.arc(2, xf, yf, rx, ry, 0.0, pi * 0.5, tf, c),         // ╯
            0x2570 => self.arc(3, rightf, yf, rx, ry, pi * 0.5, pi, tf, c),      // ╰
            0x2571 => self.paint_segment(x, y, cell_w, cell_h, rightf, yf, xf, bottomf, tf, c), // ╱
            0x2572 => self.paint_segment(x, y, cell_w, cell_h, xf, yf, rightf, bottomf, tf, c), // ╲
            0x2573 => {
                // ╳
                self.paint_segment(x, y, cell_w, cell_h, rightf, yf, xf, bottomf, tf, c);
                self.paint_segment(x, y, cell_w, cell_h, xf, yf, rightf, bottomf, tf, c);
            }
            // Dashes: 2504..250B triple and quadruple, 254C..254F double;
            // light then heavy, horizontal then vertical.
            0x2504..=0x250B | 0x254C..=0x254F => {
                let k = if cp >= 0x254C { (cp - 0x254C) as i32 } else { (cp - 0x2504) as i32 % 4 };
                let n = if cp >= 0x254C {
                    2
                } else if cp <= 0x2507 {
                    3
                } else {
                    4
                };
                self.paint_dashes(x, y, cell_w, cell_h, k >= 2, n, if k % 2 != 0 { 2 * t } else { t }, c);
            }
            _ => {
                let arms = i32::from(BOX_ARMS[(cp - 0x2500) as usize]);
                if arms == 0 {
                    return false;
                }
                let db = i32::from(DB);
                let doubled = arms & 3 == db || arms >> 2 & 3 == db || arms >> 4 & 3 == db || arms >> 6 & 3 == db;
                if doubled {
                    self.paint_double(x, y, cell_w, cell_h, arms, t, c);
                } else {
                    self.paint_lines(x, y, cell_w, cell_h, arms, t, c);
                }
            }
        }
        true
    }

    /// Box drawing and blocks, clipped to the cell. False for other
    /// characters.
    #[allow(clippy::too_many_arguments)]
    fn paint_geometry(&mut self, col: i32, row: i32, cell_w: i32, cell_h: i32, cp: u32, bold: bool, c: Rgb) -> bool {
        if !(0x2500..=0x259F).contains(&cp) {
            return false;
        }
        let (x0, y0) = (col * cell_w, row * cell_h);
        self.px.clip = Some(Rect { x0, y0, x1: x0 + cell_w, y1: y0 + cell_h });
        let painted = self.paint_cell_geometry(col, row, cell_w, cell_h, cp, bold, c);
        self.px.clip = None;
        painted
    }
}

/// The sine and cosine of the angle of an arc's point i of steps, from a0
/// to a1, as the C had them: a0 + (a1 - a0) * (i / steps), then sinf and
/// cosf of that one angle.
fn arc_point(a0: f32, a1: f32, i: i32, steps: i32) -> (f32, f32) {
    sin_cos(a0 + (a1 - a0) * (i as f32 / steps as f32))
}

/// sinf and cosf of x, as draw.c's compilers called them: merged into one
/// sincos call, which clang does on macOS (__sincosf_stret) and GCC on Linux
/// (sincosf). LLVM merges f32::sin and f32::cos the same way, but not at
/// every call site, and on macOS the merged form differs from cosf alone in
/// the last bit for 2.5 million floats an arc's angle can be
/// (bench/c-vs-rust/sincos.c), so this names the call itself.
#[cfg(target_os = "macos")]
fn sin_cos(x: f32) -> (f32, f32) {
    #[repr(C)]
    struct Float2 {
        sin: f32,
        cos: f32,
    }
    extern "C" {
        fn __sincosf_stret(x: f32) -> Float2;
    }
    // SAFETY: a libm function of a float, with no side effects.
    let r = unsafe { __sincosf_stret(x) };
    (r.sin, r.cos)
}

#[cfg(target_os = "linux")]
fn sin_cos(x: f32) -> (f32, f32) {
    extern "C" {
        fn sincosf(x: f32, sin: *mut f32, cos: *mut f32);
    }
    let (mut s, mut c) = (0.0, 0.0);
    // SAFETY: it writes the two floats it is given.
    unsafe { sincosf(x, &mut s, &mut c) };
    (s, c)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn sin_cos(x: f32) -> (f32, f32) {
    (x.sin(), x.cos())
}

#[cfg(test)]
#[path = "geometry_tests.rs"]
mod geometry_tests;
