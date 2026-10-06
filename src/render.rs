//! The render: a screen of cells, their combining marks and the image views
//! in, a PNG out. It sets up the fonts (stb, through src/stb_glue.c),
//! allocates the canvas, runs the passes in their order, encodes the PNG
//! (stb_image_write, with src/deflate.rs as its compressor) and writes it,
//! and prints the TERMSHOT_PROFILE record of the render's stages.
//!
//! The passes, each painting over the last:
//! 1. the backdrop (src/composite.rs): the cell backgrounds, the images
//!    below them (showing through the default ones only), and the images
//!    over every background but under the text, a row of cells ahead of
//!    the text;
//! 2. the text (src/glyphs.rs): box drawing and blocks (src/geometry.rs),
//!    the glyphs, the boxes of missing ones, combining marks, then
//!    underlines and strike-through;
//! 3. the images over the text, in their order: main.rs puts the underline
//!    or bar cursor first among them (the block cursor is reverse video in
//!    the cells);
//! 4. with padding (RenderOptions), the margin around the cells.
//!
//! Padding is a frame around the render it would make without: the passes
//! paint a canvas cut from the padded raster (the cells' pixels, a stride
//! as wide as the PNG's rows), so every glyph, image, clip and cursor mark
//! lands where it would, moved by the margin, and nothing reaches into the
//! margin, as nothing reaches past the cells without one. The cell metrics,
//! and so where native-pixel images move the cursor, don't change.
//!
//! This was C in draw.c (draw_png_images, draw_png) until #12 step 2d, and
//! it makes the same PNGs, messages and results. It does no float
//! arithmetic of its own: the cell metrics and the faces' scales come from
//! stb_glue.c, which keeps stb_truetype's float math under -ffp-contract=off.
//!
//! Memory: the canvas is the render's one allocation here, made with
//! alloc_zeroed, so running out of memory fails the render with exit 2
//! ("out of memory for a WxH image") instead of aborting; so does an
//! allocation the PNG encoder makes (its buffer, through termshot_png_alloc,
//! or the compressor's). The canvas, the glyph cache and the box-drawing
//! cache are the render's own, and the profile clocks and fault counters
//! are per thread, so concurrent renders stay independent.

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::ffi::{c_char, c_int, c_void};
use std::io::Write;
use std::os::unix::io::IntoRawFd;
use std::mem::MaybeUninit;

use crate::cell::{Cell, CellMarks};
use crate::composite::{self, Backdrop, ImageView, LAYER_OVER_TEXT};
use crate::deflate::{self, DeflateTimings};
use crate::geometry::{self, Canvas, GeometryStats};
use crate::glyphs::{self, Clock, EmptyGlyphs, Face, Failure, TextFonts, TextStats};

/// The canvas is RGB: alpha would always be 255, and an opaque RGBA PNG is
/// larger and blocks palette quantization in downstream optimizers.
const BPP: usize = 3;

/// 2^27 pixels keeps the 3-byte rows plus filter bytes near 384 MiB, well
/// under stb_image_write's int sizes (INT_MAX).
pub const MAX_PIXELS: i64 = 1 << 27;

/// Rasters at least this large paint their backdrop a row of cells at a
/// time, so that the text is painted while its rows are still in the cache
/// (src/composite.rs, termshot_backdrop_init). A smaller one stays in the
/// last-level cache anyway: rows gained nothing at 2200x1440 (9.5 MB) on
/// either host measured, and on macOS they cost 1-2% there, but were up to
/// 8% faster on Linux at 61-67 MB (docs/performance.md).
pub const BACKDROP_ROW_BYTES: usize = 16 << 20;

/// stbtt_fontinfo's storage, which only C reads: 160 bytes, 8-aligned, as
/// stb_glue.c asserts.
#[repr(C, align(8))]
struct FontInfoStorage([u8; 160]);

/// The cell metrics of the font, as CellMetrics in stb_glue.c: its advance
/// in font units, the cell, the ascent-to-descent body, the baseline (from
/// the cell's top), the scale and the italic pivot in pixels.
#[repr(C)]
#[derive(Clone, Copy)]
struct CellMetrics {
    adv: i32,
    cell_w: i32,
    cell_h: i32,
    body: i32,
    baseline: i32,
    scale: f32,
    italic_pivot: f32,
}

const _: () = assert!(std::mem::size_of::<CellMetrics>() == 28);

/// A render's fonts, filled in by termshot_font_setup, as FontSetup in
/// stb_glue.c: stb's font for each face, what the text may ask stb (whose
/// faces point into `font` and `fallback`, so this must not move once
/// filled in) and the cell metrics.
#[repr(C)]
struct FontSetup {
    font: FontInfoStorage,
    fallback: FontInfoStorage,
    text: TextFonts,
    metrics: CellMetrics,
}

const _: () = assert!(std::mem::size_of::<FontSetup>() == 504);

extern "C" {
    /// stb_glue.c: stb's fonts for the faces (fallback null for none), the
    /// cell metrics at `px` and TextFonts, in `setup`; 1, having said why on
    /// stderr, when a face or the font's metrics can't be used.
    fn termshot_font_setup(font: *const Face, fallback: *const Face, px: f64, setup: *mut FontSetup) -> c_int;
    /// stb_glue.c: the PNG of `h` filtered scanlines of `w` RGB pixels, a
    /// malloc'd buffer of `*len` bytes, or null when an allocation failed;
    /// with `profile`, the clock at its four stages in `marks`.
    fn termshot_png_encode(filtered: *mut u8, w: c_int, h: c_int, profile: c_int, marks: *mut [f64; 4],
                           len: *mut c_int) -> *mut u8;
    fn malloc(size: usize) -> *mut c_void;
    fn free(p: *mut c_void);
    fn close(fd: c_int) -> c_int;
}

/// STBIW_MALLOC in stb_glue.c: malloc, but where the fault tests fail it.
#[no_mangle]
pub extern "C" fn termshot_png_alloc(size: usize) -> *mut c_void {
    if !allowed(Site::Png) {
        return std::ptr::null_mut();
    }
    // SAFETY: malloc of any size; null when it fails, which stb checks.
    unsafe { malloc(size) }
}

/// The canvas's filtered scanlines (`filtered` in Canvas), zeroed, freed on
/// drop.
struct Raster {
    data: *mut u8,
    layout: Layout,
}

impl Raster {
    /// `len` zero bytes, or None when memory runs out. calloc gives a large
    /// raster as fresh pages, which it need not clear.
    fn new(len: usize) -> Option<Raster> {
        if !allowed(Site::Canvas) {
            return None;
        }
        let layout = Layout::array::<u8>(len.max(1)).ok()?;
        // SAFETY: the layout's size is nonzero.
        let data = unsafe { alloc_zeroed(layout) };
        // Never a Raster of null: its drop would deallocate it, which is
        // undefined, and LLVM then took the pointer for non-null and painted
        // through it (a segfault under ulimit -v, tests/run.sh).
        if data.is_null() {
            return None;
        }
        Some(Raster { data, layout })
    }
}

impl Drop for Raster {
    fn drop(&mut self) {
        // SAFETY: allocated with this layout in Raster::new.
        unsafe { dealloc(self.data, self.layout) }
    }
}

/// The largest padding on a side, in pixels.
pub const MAX_PADDING: u32 = 1024;

/// What the render adds that the cells don't say (#87): a margin of
/// `padding.0` pixels left and right of the cells and `padding.1` above and
/// below them, in `background` (the palette's default background).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    pub padding: (u32, u32),
    pub background: (u8, u8, u8),
}

impl RenderOptions {
    /// No margin: the PNG is the cells.
    pub const NONE: RenderOptions = RenderOptions { padding: (0, 0), background: (0, 0, 0) };
}

/// What a render ends with, as main.rs passes it on: 0 done; 1 a face or
/// its metrics can't be used; 2 the image is over MAX_PIXELS, memory ran
/// out, or a painter failed (a bug); 3 the PNG could not be written. Each
/// but 0 has been said on stderr.
pub type Code = i32;

/// Paint `cols` x `rows` cells with `font` and write the PNG to `out_path`
/// (main.rs names stdout /dev/stdout). `fallback` supplies the characters
/// `font` lacks; characters neither has are drawn as an outlined box.
/// `marks`, sorted by cell, gives the cells' combining marks, drawn over
/// them (src/glyphs.rs). `images` are the image views in their order, the
/// cursor mark among them. `verbose` prints the cell and image size to
/// stderr. `empty`, if given, is filled in as EmptyGlyphs says.
///
/// # Safety
/// `font` and `fallback` must be faces of fonts that passed font::check
/// (src/font.rs makes them), their data padded and live; each image view's
/// pixels must hold its `width` x `height` RGBA pixels.
#[allow(clippy::too_many_arguments)]
pub unsafe fn draw_png_images(cells: &[Cell], marks: &[CellMarks], cols: usize, rows: usize, font: &Face,
                              fallback: Option<&Face>, font_px: f64, out_path: &str, verbose: bool,
                              images: &[ImageView], empty: Option<&mut EmptyGlyphs>) -> Code {
    draw_png_with(cells, marks, cols, rows, font, fallback, font_px, out_path, verbose, images, empty,
                  &RenderOptions::NONE)
}

/// draw_png_images, with `options`' padding around the cells. Its margin
/// counts towards MAX_PIXELS; with `verbose`, the image size printed is the
/// padded one.
///
/// # Safety
/// As draw_png_images.
#[allow(clippy::too_many_arguments)]
pub unsafe fn draw_png_with(cells: &[Cell], marks: &[CellMarks], cols: usize, rows: usize, font: &Face,
                            fallback: Option<&Face>, font_px: f64, out_path: &str, verbose: bool,
                            images: &[ImageView], mut empty: Option<&mut EmptyGlyphs>,
                            options: &RenderOptions) -> Code {
    assert!(cells.len() >= cols * rows, "{} cells for {cols}x{rows}", cells.len());
    if let Some(empty) = empty.as_deref_mut() {
        *empty = EmptyGlyphs::default();
    }
    let profiling = std::env::var_os("TERMSHOT_PROFILE").is_some();
    deflate::termshot_deflate_profiling(c_int::from(profiling));
    faults::start();
    let clock = Clock(profiling);
    let started = clock.now();
    // SAFETY: all zeros is a valid FontSetup (null pointers, no functions);
    // it is filled in where it stays.
    let mut setup: FontSetup = std::mem::zeroed();
    let fallback_ptr = fallback.map_or(std::ptr::null(), |f| f as *const Face);
    if termshot_font_setup(font, fallback_ptr, font_px, &mut setup) != 0 {
        return 1;
    }
    let m = setup.metrics;
    let (cell_w, cell_h) = (m.cell_w, m.cell_h);
    // The cells' pixels, and the PNG's: the cells and the margin.
    let (grid_w, grid_h) = (cols as i64 * i64::from(cell_w), rows as i64 * i64::from(cell_h));
    let (pad_x, pad_y) = (i64::from(options.padding.0), i64::from(options.padding.1));
    let (width, height) = (grid_w + 2 * pad_x, grid_h + 2 * pad_y);
    if verbose {
        eprintln!(
            "advance {} units scale {:.5} cell {}x{} baseline {} image {}x{}",
            m.adv,
            f64::from(m.scale),
            cell_w,
            cell_h,
            m.baseline,
            width,
            height
        );
    }
    // stb_image_write sizes its buffers with int: (width*BPP+1)*height must not wrap.
    // Each side first, as a cell can be 2^28 pixels each way (stb_glue.c's
    // to_px) and their product overflow; a side past it is past it in pixels too.
    if width > MAX_PIXELS || height > MAX_PIXELS || width * height > MAX_PIXELS {
        let padding = if pad_x != 0 || pad_y != 0 { ", rows or padding" } else { " or rows" };
        eprintln!("termshot: image {width}x{height} is over {MAX_PIXELS} pixels; lower px, cols{padding}");
        return 2;
    }

    let font_setup = clock.now();
    // One leading zero per scanline is PNG's None filter. Paint directly into
    // the compressor input instead of copying a second full image later.
    let stride = width as usize * BPP + 1;
    let Some(raster) = Raster::new(stride * height as usize) else {
        eprintln!("termshot: out of memory for a {width}x{height} image");
        return 2;
    };
    // The cells' canvas, inside the margin: its `filtered` is the byte
    // before its first pixel, which is the scanline's filter byte only
    // without a left margin. The backdrop writes that byte as one; the margin
    // is painted last, over it.
    let inside = pad_y as usize * stride + pad_x as usize * BPP;
    let mut cv = Canvas {
        px: raster.data.add(inside + 1),
        filtered: raster.data.add(inside),
        w: grid_w as i32,
        h: grid_h as i32,
        stride,
        // Null only means nothing is cached; the pixels are the same. It
        // also begins the render for termshot_paint_failed.
        geometry: geometry::termshot_geometry_new(1),
    };
    let allocated = clock.now();
    // The backgrounds and the images under the text are painted a row of
    // cells ahead of the text over them (src/glyphs.rs); background_ms is
    // their time, and foreground_ms the rest.
    let mut backdrop = MaybeUninit::<Backdrop>::uninit();
    composite::termshot_backdrop_init(backdrop.as_mut_ptr(), &cv, cells.as_ptr(), cols as c_int, rows as c_int,
                                      cell_w, cell_h, images.as_ptr(), images.len(), BACKDROP_ROW_BYTES);
    let mut backdrop = backdrop.assume_init();
    let background = clock.now();
    let mut text = TextStats::default();
    let empty_ptr = empty.map_or(std::ptr::null_mut(), |e| e as *mut EmptyGlyphs);
    let painted = glyphs::termshot_paint_text(&cv, &mut backdrop, marks.as_ptr(), marks.len(), &setup.text,
                                              c_int::from(profiling), empty_ptr, &mut text);
    let failed = |cv: &mut Canvas, why: &str| {
        // What the render holds goes with it: the geometry here, the raster
        // when it is dropped.
        geometry::termshot_geometry_free(cv.geometry);
        cv.geometry = std::ptr::null_mut();
        eprintln!("termshot: {why}");
        2
    };
    if painted != 0 {
        let why = match painted {
            p if p == Failure::OutOfMemory as c_int => "glyph allocation failed",
            p if p == Failure::BoxDrawing as c_int => "box drawing failed",
            _ => "painting failed",
        };
        return failed(&mut cv, why);
    }
    composite::termshot_paint_images(&cv, images.as_ptr(), images.len(), LAYER_OVER_TEXT);
    // The fills and the backdrop have no result of their own: a fill or an
    // image that failed says so now, before an incomplete image is written.
    if geometry::termshot_paint_failed() != 0 {
        return failed(&mut cv, "painting failed");
    }
    let mut stamps = GeometryStats::default();
    geometry::termshot_geometry_stats(cv.geometry, &mut stamps);
    geometry::termshot_geometry_free(cv.geometry);
    if pad_x != 0 || pad_y != 0 {
        let filtered = std::slice::from_raw_parts_mut(raster.data, stride * height as usize);
        paint_margin(filtered, stride, (grid_w as usize, grid_h as usize), (pad_x as usize, pad_y as usize),
                     options.background);
    }
    let foreground = clock.now();
    let mut png_len: c_int = 0;
    let mut png_marks = [0.0f64; 4];
    let png = termshot_png_encode(raster.data, width as c_int, height as c_int, c_int::from(profiling),
                                  &mut png_marks, &mut png_len);
    let encoded = clock.now();
    let mut ok = false;
    // stb returns null only when an allocation failed, its own or the compressor's.
    let encode_failed = png.is_null();
    if encode_failed {
        eprintln!("termshot: out of memory encoding a {width}x{height} PNG");
    } else {
        let bytes = std::slice::from_raw_parts(png, png_len.max(0) as usize);
        if let Ok(mut out) = std::fs::File::create(out_path) {
            ok = out.write_all(bytes).is_ok();
            // File's drop ignores close's result, where a filesystem may
            // report a write it deferred; the C's fclose was checked too.
            ok &= close(out.into_raw_fd()) == 0;
        }
        free(png as *mut c_void);
    }
    let written = clock.now();
    drop(raster);
    if profiling {
        let mut deflate = DeflateTimings::default();
        deflate::termshot_deflate_timings(&mut deflate);
        let p = &png_marks;
        eprintln!(
            "termshot-profile {{\"deflate_allocate_ms\":{:.6},\"deflate_match_emit_ms\":{:.6},\"deflate_finalize_ms\":{:.6},\"deflate_checksum_ms\":{:.6},\"font_setup_ms\":{:.6},\"allocate_ms\":{:.6},\"background_ms\":{:.6},\"foreground_ms\":{:.6},\"geometry_ms\":{:.6},\"glyph_ms\":{:.6},\"blend_ms\":{:.6},\"png_filter_ms\":{:.6},\"png_deflate_ms\":{:.6},\"png_pack_ms\":{:.6},\"png_encode_ms\":{:.6},\"output_write_ms\":{:.6},\"cleanup_ms\":{:.6},\"geometry_cache_hits\":{},\"geometry_cache_misses\":{},\"geometry_cache_uncached\":{},\"geometry_cache_bytes\":{},\"glyph_rasterizations\":{},\"glyph_cache_hits\":{},\"glyph_cache_evictions\":{},\"glyph_missing\":{},\"fallback_lookups\":{},\"fallback_rasterizations\":{},\"png_bytes\":{},\"pixel_bytes\":{}}}",
            deflate.allocate_ms, deflate.match_emit_ms, deflate.finalize_ms, deflate.checksum_ms,
            font_setup - started, allocated - font_setup,
            background - allocated + text.backdrop_ms, foreground - background - text.backdrop_ms, text.geometry_ms,
            text.glyph_ms, text.blend_ms,
            p[1] - p[0], p[2] - p[1], p[3] - p[2],
            encoded - foreground, written - encoded, clock.now() - written, stamps.hits, stamps.misses, stamps.uncached,
            stamps.bytes, text.glyphs, text.cache_hits, text.evictions, text.missing, text.fallback_lookups,
            text.fallback_glyphs, png_len, width as usize * height as usize * BPP
        );
    }
    if encode_failed {
        return 2;
    }
    if !ok {
        eprintln!("termshot: png write failed: {out_path}");
        return 3;
    }
    0
}

/// Paints the margin of a padded raster, `filtered` (scanlines `stride`
/// bytes apart, each a filter byte then its pixels): `pad_y` whole rows
/// above and below the cells' `grid` pixels, and `pad_x` pixels left and
/// right of them, in `rgb`, with every filter byte 0 (PNG's None).
fn paint_margin(filtered: &mut [u8], stride: usize, (grid_w, grid_h): (usize, usize), (pad_x, pad_y): (usize, usize),
                (r, g, b): (u8, u8, u8)) {
    let rgb = [r, g, b];
    let fill = |pixels: &mut [u8]| pixels.chunks_exact_mut(BPP).for_each(|p| p.copy_from_slice(&rgb));
    for (y, scanline) in filtered.chunks_exact_mut(stride).enumerate() {
        scanline[0] = 0;
        let pixels = &mut scanline[1..];
        if y < pad_y || y >= pad_y + grid_h {
            fill(pixels);
        } else {
            fill(&mut pixels[..pad_x * BPP]);
            fill(&mut pixels[(pad_x + grid_w) * BPP..]);
        }
    }
}

/// The cell-only entry point for the C harnesses (tests/draw.c,
/// tests/glyphs.c), for TrueType faces: `cols` x `rows` cells drawn with
/// the font at `ttf` (its face at `ttf_start`) and the fallback at
/// `fallback_ttf`, or null, to the PNG `out_path`. Returns as
/// draw_png_images does, and 2 if it panicked (a bug).
///
/// # Safety
/// `cells` must hold cols * rows cells, the fonts be checked, padded
/// TrueType fonts (the harnesses' are vendored), and `out_path` a C string.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn draw_png(cells: *const Cell, cols: c_int, rows: c_int, ttf: *const u8, ttf_start: c_int,
                                  fallback_ttf: *const u8, fallback_start: c_int, font_px: f64,
                                  out_path: *const c_char, verbose: c_int) -> c_int {
    let (cols, rows) = (cols.max(0) as usize, rows.max(0) as usize);
    let cells = if cells.is_null() { &[][..] } else { std::slice::from_raw_parts(cells, cols * rows) };
    // TrueType faces at their default instance: no callbacks.
    let face = |ttf, start| Face {
        ttf,
        start,
        outline: None,
        cff: std::ptr::null(),
        advance: None,
        advances: std::ptr::null(),
        varied: 0,
        ascent: 0,
        descent: 0,
        line_gap: 0,
    };
    let font = face(ttf, ttf_start);
    let fallback = face(fallback_ttf, fallback_start);
    let fallback = (!fallback_ttf.is_null()).then_some(&fallback);
    let Ok(out_path) = std::ffi::CStr::from_ptr(out_path).to_str() else { return 3 };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        draw_png_images(cells, &[], cols, rows, &font, fallback, font_px, out_path, verbose != 0, &[], None)
    }))
    .unwrap_or(2)
}

/// Where the render allocates. The fault tests fail each in turn.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Site {
    /// The canvas.
    Canvas,
    /// The PNG's own buffer, which stb_image_write asks for.
    Png,
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
/// TERMSHOT_RENDER_FAIL_AT=n and fails the nth allocation of each render
/// here (tests/run.sh checks that the CLI exits 2 and leaves no output).
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
        let fail_at = std::env::var("TERMSHOT_RENDER_FAIL_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
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
                eprintln!("termshot: render allocation {} ({:?}) fails", s.calls, _site);
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
mod tests {
    use super::Raster;

    /// A raster the system can't give is None, never one that holds null
    /// (whose drop would be undefined). tests/run.sh checks the CLI under
    /// ulimit -v on Linux.
    #[test]
    fn a_raster_memory_cant_hold_is_none() {
        assert!(Raster::new(1 << 60).is_none());
        let raster = Raster::new(4096).unwrap();
        // SAFETY: 4096 bytes, zeroed.
        assert!(unsafe { std::slice::from_raw_parts(raster.data, 4096) }.iter().all(|&b| b == 0));
    }
}
