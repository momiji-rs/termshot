//! The image layers: kitty and Sixel images and the underline and bar
//! cursors (solid ImageViews), composited into draw.c's canvas, and the
//! backdrop under the text, which is the cell backgrounds and the images
//! under the text, painted a row of cells ahead of the glyphs over them.
//! draw.c calls this through FFI, per backdrop row or per layer, never per
//! pixel: termshot_backdrop_init and termshot_backdrop_through paint the
//! backdrop, and termshot_paint_images a layer of images (the one over the
//! text).
//!
//! This was C in draw.c (`paint_image_rows`, `backdrop_through`) until #12
//! step 2b, and it paints the same pixels:
//!
//! - All of it is integer arithmetic, in the C's types: i64 for positions
//!   and the sampler's quotient and remainder, usize where the C indexed
//!   with size_t, and the blend in u32 as the C's unsigned. The blend
//!   `(s * a + d * (255 - a) + 127) / 255` is exact integer division in both
//!   languages, so there is no rounding to match.
//! - C's signed overflow is undefined; Rust's wraps in release builds and
//!   panics with overflow checks (SANITIZE=1). graphics.rs keeps positions
//!   within 2^24 cells and sizes within 2^24 pixels and sources within 8192
//!   pixels a side, so no product here comes near i64's range on the views
//!   termshot makes, and the two agree on all of them.
//! - Where the C read past an image (a crop outside it) or the cells, which
//!   termshot never asks for, the Rust panics instead; the panic is caught,
//!   and the render fails with exit 2 (termshot_paint_failed).
//!
//! Memory: nothing here allocates. The backdrop lives in draw.c's frame.

use std::ffi::c_int;

use crate::cell::{Cell, OPAQUE};
use crate::geometry::{guarded, Canvas};

/// Bytes per pixel: the canvas is RGB, as BPP in draw.c.
const BPP: usize = 3;

/// A placement's visible part, or a solid rectangle, for draw.c and this
/// module; borrowed only for the duration of draw_png_images, the pixels
/// remaining Rust-owned. As ImageView in src/draw.c, which asserts the
/// same size.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ImageView {
    /// `width` x `height` RGBA pixels, row by row.
    pub(crate) pixels: *const u8,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// Where the whole source rectangle lands on the canvas, in pixels.
    pub(crate) x: i64,
    pub(crate) y: i64,
    pub(crate) w: i64,
    pub(crate) h: i64,
    /// The rows [clip_top, clip_bottom) and columns [clip_left,
    /// clip_right) of the canvas it may paint.
    pub(crate) clip_top: i64,
    pub(crate) clip_bottom: i64,
    pub(crate) clip_left: i64,
    pub(crate) clip_right: i64,
    /// The source rectangle sampled, inside width x height; never empty.
    pub(crate) src_x: u32,
    pub(crate) src_y: u32,
    pub(crate) src_w: u32,
    pub(crate) src_h: u32,
    /// Below INT32_MIN / 2 the image is drawn under non-default cell
    /// backgrounds, below 0 over every background but under the text, and
    /// from 0 over both.
    pub(crate) z: i32,
}

// 104 bytes on the 64-bit targets termshot supports: a pointer, two u32, eight
// i64, five 32-bit fields and 4 bytes of tail padding. Checked in src/draw.c too.
const _: () = assert!(std::mem::size_of::<ImageView>() == 104);

impl ImageView {
    /// A rectangle of one colour: the opaque pixel, stretched over it, in the
    /// layer over the text, where the views are drawn in their order. The
    /// pixel is borrowed; it must outlive the view.
    pub fn solid(pixel: &[u8; 4], x: i64, y: i64, w: i64, h: i64) -> ImageView {
        ImageView {
            pixels: pixel.as_ptr(),
            width: 1,
            height: 1,
            x,
            y,
            w,
            h,
            clip_top: y,
            clip_bottom: y + h,
            clip_left: x,
            clip_right: x + w,
            src_x: 0,
            src_y: 0,
            src_w: 1,
            src_h: 1,
            z: i32::MAX,
        }
    }
}

/// kitty's three image layers, by z-index, as in draw.c: under the cell
/// backgrounds that are not the default (z below INT32_MIN / 2), over every
/// background but under the text (other negative z), and over the text.
pub const LAYER_BELOW: c_int = 0;
pub const LAYER_UNDER_TEXT: c_int = 1;
pub const LAYER_OVER_TEXT: c_int = 2;

pub fn image_layer(z: i32) -> c_int {
    // i32::MIN / 2 truncates toward zero, as C's INT32_MIN / 2 does.
    if z < i32::MIN / 2 {
        LAYER_BELOW
    } else if z < 0 {
        LAYER_UNDER_TEXT
    } else {
        LAYER_OVER_TEXT
    }
}

/// Whether the cell's background is the default one, which shows an image
/// of LAYER_BELOW through it: only OPAQUE decides.
pub fn clear_background(cell: &Cell) -> bool {
    cell.attrs & OPAQUE == 0
}

/// More images under the text than this and the backdrop is painted at
/// once, since each row looks at every image (Unicode placeholders make one
/// view a run).
pub const BACKDROP_ROW_IMAGES: usize = 64;

/// What is painted under the text, a row of cells at a time, so that the
/// text is painted while its rows are still in the cache: the cell
/// backgrounds, then the images below them, which show only through the
/// default ones, and those over every background. A row is painted before
/// anything over it (the row's own cells, or a glyph or mark reaching down
/// into it), so each pixel is painted in the same order as if every row were
/// painted first. With `whole`, every row is painted at the first call, then
/// each layer of images over the whole canvas, as before rows.
///
/// Shared with draw.c (its Backdrop), which keeps it in its frame:
/// termshot_backdrop_init fills it in, and draw.c reads `done`, `rows` and
/// `cell_h` to skip a call with nothing to paint. `ms` is draw.c's, the
/// time spent painting it, for TERMSHOT_PROFILE; not read here.
#[repr(C)]
pub struct Backdrop {
    /// cols x rows cells, row by row.
    pub cells: *const Cell,
    pub images: *const ImageView,
    pub image_count: usize,
    pub cols: i32,
    pub rows: i32,
    pub cell_w: i32,
    pub cell_h: i32,
    /// Rows of cells painted.
    pub done: i32,
    /// Nonzero: paint every row at the first call.
    pub whole: i32,
    pub ms: f64,
}

/// As draw.c asserts of its Backdrop.
const _: () = assert!(std::mem::size_of::<Backdrop>() == 56);

/// The images, or none for a null pointer.
///
/// # Safety
/// `images` must be null or point to `count` views.
unsafe fn views<'a>(images: *const ImageView, count: usize) -> &'a [ImageView] {
    if images.is_null() || count == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(images, count)
    }
}

/// Fills in the backdrop of a render: cols x rows cells of cell_w x cell_h
/// pixels, which is the canvas, and the images. It paints every row at the
/// first call (`whole`) for a raster under `row_bytes`, which stays in the
/// last-level cache anyway (draw.c's BACKDROP_ROW_BYTES: rows gained
/// nothing at 2200x1440, 9.5 MB, on either host measured, and on macOS they
/// cost 1-2% there, but were up to 8% faster on Linux at 61-67 MB;
/// docs/performance.md), and with more than BACKDROP_ROW_IMAGES images
/// under the text.
///
/// # Safety
/// `bd` must be writable and `cv` readable. `cells` and `images` are kept,
/// not read, except for each image's z: `images` must be null or point to
/// `image_count` views.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn termshot_backdrop_init(bd: *mut Backdrop, cv: *const Canvas, cells: *const Cell,
                                                cols: c_int, rows: c_int, cell_w: c_int, cell_h: c_int,
                                                images: *const ImageView, image_count: usize, row_bytes: usize) {
    let cv = &*cv;
    let under = views(images, image_count).iter().filter(|im| image_layer(im.z) != LAYER_OVER_TEXT).count();
    let raster = cv.stride.wrapping_mul(cv.h.max(0) as usize);
    let whole = under > BACKDROP_ROW_IMAGES || raster < row_bytes;
    bd.write(Backdrop { cells, images, image_count, cols, rows, cell_w, cell_h, done: 0, whole: whole as i32,
                        ms: 0.0 });
}

/// Paints the backdrop of every row of cells above pixel row `y` that is not
/// painted yet. Returns 1 when it painted, 0 when there was nothing to paint,
/// and -1 if painting panicked (a bug), which termshot_paint_failed then
/// reports, failing the render.
///
/// # Safety
/// `cv` must point to a Canvas whose `filtered` holds its `h` scanlines,
/// `stride` bytes apart (stride >= 3 * w + 1), with `px` at `filtered + 1`;
/// `bd` must come from termshot_backdrop_init for that canvas, with its
/// cells and images live, `cols * cell_w == w` and `rows * cell_h == h`; and
/// each image's pixels must hold its `width` x `height` RGBA pixels. Nothing
/// else may use any of them during the call.
#[no_mangle]
pub unsafe extern "C" fn termshot_backdrop_through(cv: *const Canvas, bd: *mut Backdrop, y: i64) -> c_int {
    let (cv, bd) = (&*cv, &mut *bd);
    if bd.done >= bd.rows || y <= bd.done as i64 * bd.cell_h as i64 {
        return 0;
    }
    match guarded(|| backdrop_through(cv, bd, y)) {
        Some(()) => 1,
        None => -1,
    }
}

/// Paints the images of `layer` (LAYER_*), in their order, over the whole
/// canvas: draw.c's call for the images over the text. 0, or -1 if painting
/// panicked (a bug), which termshot_paint_failed then reports.
///
/// # Safety
/// `cv` as for termshot_paint_geometry (src/geometry.rs); `images` null or
/// `count` views, each with its pixels.
#[no_mangle]
pub unsafe extern "C" fn termshot_paint_images(cv: *const Canvas, images: *const ImageView, count: usize,
                                               layer: c_int) -> c_int {
    let cv = &*cv;
    let images = views(images, count);
    match guarded(|| paint_images(cv, images, layer, None, 0, cv.h as i64)) {
        Some(()) => 0,
        None => -1,
    }
}

/// The cells a layer of images shows through (LAYER_BELOW): `cells` row by
/// row, as many to a row as the canvas's width over `cell_w`, each `cell_w`
/// x `cell_h` pixels.
#[derive(Clone, Copy)]
pub struct Mask<'a> {
    pub cells: &'a [Cell],
    pub cell_w: i64,
    pub cell_h: i64,
}

/// termshot_backdrop_through's painting, for rows `bd.done` up to the one
/// that holds pixel row y - 1.
///
/// # Safety
/// As termshot_backdrop_through.
unsafe fn backdrop_through(cv: &Canvas, bd: &mut Backdrop, y: i64) {
    let (cols, rows, cell_w, cell_h) = (bd.cols, bd.rows, bd.cell_w, bd.cell_h);
    let last = if bd.whole != 0 { rows as i64 } else { (y + cell_h as i64 - 1) / cell_h as i64 }.min(rows as i64);
    let images = views(bd.images, bd.image_count);
    let cells: &[Cell] = if bd.cells.is_null() || cols <= 0 || rows <= 0 {
        &[]
    } else {
        std::slice::from_raw_parts(bd.cells, cols as usize * rows as usize)
    };
    let mask = Mask { cells, cell_w: cell_w as i64, cell_h: cell_h as i64 };
    for r in bd.done..last as i32 {
        let top = r * cell_h;
        backgrounds(cv, &cells[r as usize * cols as usize..][..cols as usize], top, cell_w, cell_h);
        if bd.whole != 0 {
            continue;
        }
        let (top, bottom) = (top as i64, top as i64 + cell_h as i64);
        paint_images(cv, images, LAYER_BELOW, Some(mask), top, bottom);
        paint_images(cv, images, LAYER_UNDER_TEXT, None, top, bottom);
    }
    if bd.whole != 0 {
        paint_images(cv, images, LAYER_BELOW, Some(mask), 0, cv.h as i64);
        paint_images(cv, images, LAYER_UNDER_TEXT, None, 0, cv.h as i64);
    }
    bd.done = last as i32;
}

/// Paints the backgrounds of a row of cells, whose pixels start at row
/// `top`: the first scanline a cell at a time, clipped to the canvas, then
/// that scanline, its filter byte (0, PNG's None) included, copied to the
/// others. Through `filtered`, which holds whole scanlines.
///
/// # Safety
/// As termshot_backdrop_through says of `cv`.
unsafe fn backgrounds(cv: &Canvas, row: &[Cell], top: i32, cell_w: i32, cell_h: i32) {
    if cv.filtered.is_null() || cv.w <= 0 || cv.h <= 0 || top < 0 || top >= cv.h {
        return;
    }
    let stride = cv.stride;
    let filtered = std::slice::from_raw_parts_mut(cv.filtered, stride * cv.h as usize);
    let first = top as usize * stride;
    let scanline = &mut filtered[first..first + stride];
    scanline[0] = 0;
    for (c, cell) in row.iter().enumerate() {
        // As termshot_fill_rect clips [c * cell_w, (c + 1) * cell_w).
        let x0 = (c as i32 * cell_w).max(0);
        let x1 = ((c as i32 + 1) * cell_w).min(cv.w);
        if x0 >= x1 {
            continue;
        }
        let rgb = [cell.br, cell.bg, cell.bb];
        for p in scanline[1 + x0 as usize * BPP..1 + x1 as usize * BPP].chunks_exact_mut(BPP) {
            p.copy_from_slice(&rgb);
        }
    }
    let end = (top as usize + cell_h.max(0) as usize).min(cv.h as usize);
    for line in top as usize + 1..end {
        filtered.copy_within(first..first + stride, line * stride);
    }
}

/// Paints the images of one layer, in their order, over the pixel rows
/// [top, bottom); with a mask, only over the clear backgrounds. Clips
/// before looping, and samples nearest-neighbour in integers, so
/// screenshots are reproducible.
///
/// # Safety
/// `cv` as for termshot_paint_geometry; each image's pixels must hold its
/// `width` x `height` RGBA pixels.
pub unsafe fn paint_images(cv: &Canvas, images: &[ImageView], layer: c_int, mask: Option<Mask>, top: i64,
                           bottom: i64) {
    if cv.px.is_null() || cv.w <= 0 || cv.h <= 0 {
        // Nothing is on such a canvas, so nothing is painted.
        return;
    }
    // The last row ends with its last pixel: filtered has no byte past it.
    let px = std::slice::from_raw_parts_mut(cv.px, (cv.h as usize - 1) * cv.stride + cv.w as usize * BPP);
    for im in images {
        if image_layer(im.z) == layer {
            paint_image(px, cv.w as i64, cv.h as i64, cv.stride, im, mask, top, bottom);
        }
    }
}

/// One image of paint_images, into `px`, the canvas's w x h pixels, `stride`
/// bytes apart.
///
/// # Safety
/// The image's pixels must hold its `width` x `height` RGBA pixels.
#[allow(clippy::too_many_arguments)]
unsafe fn paint_image(px: &mut [u8], cw: i64, ch: i64, stride: usize, im: &ImageView, mask: Option<Mask>, top: i64,
                      bottom: i64) {
    let x0 = im.x.max(im.clip_left).max(0);
    let y0 = im.y.max(im.clip_top).max(top);
    let x1 = (im.x + im.w).min(im.clip_right).min(cw);
    // The canvas's bottom too: the C's callers passed a bottom within it.
    let y1 = (im.y + im.h).min(im.clip_bottom).min(bottom).min(ch);
    if x0 >= x1 {
        return;
    }
    // x0 >= im.x and x1 <= im.x + w, so every column is inside the image's
    // w (and so w > 0): the source column of x, src_x + (x - im.x) * src_w
    // / w, is inside the crop. Its quotient and remainder are stepped from
    // x0 a column at a time.
    let (w, src_w) = (im.w, im.src_w as i64);
    let (step, carry) = (src_w / w, src_w % w);
    let pixels = std::slice::from_raw_parts(im.pixels, im.width as usize * im.height as usize * 4);
    let span = (x1 - x0) as usize;
    for y in y0..y1 {
        // y >= im.y and y < im.y + h likewise (so h > 0).
        let sy = im.src_y as usize + ((y - im.y) * im.src_h as i64 / im.h) as usize;
        // The crop's row: a crop outside the image panics here.
        let from = (sy * im.width as usize + im.src_x as usize) * 4;
        let line = &pixels[from..from + im.src_w as usize * 4];
        let at = y as usize * stride + x0 as usize * BPP;
        let dst = &mut px[at..at + span * BPP];
        match mask {
            None => sample_run(dst, line, x0 - im.x, src_w, w, step, carry),
            Some(m) => {
                let per_row = (cw / m.cell_w) as usize;
                let row = &m.cells[(y / m.cell_h) as usize * per_row..][..per_row];
                // A cell's columns at a time: those of an opaque background
                // are skipped.
                let mut x = x0;
                while x < x1 {
                    let col = x / m.cell_w;
                    let end = ((col + 1) * m.cell_w).min(x1);
                    if clear_background(&row[col as usize]) {
                        let run = &mut dst[(x - x0) as usize * BPP..(end - x0) as usize * BPP];
                        sample_run(run, line, x - im.x, src_w, w, step, carry);
                    }
                    x = end;
                }
            }
        }
    }
}

/// Blends `dst.len() / 3` pixels from the source row `line` (src_w RGBA
/// pixels), starting at the column `from` pixels into the image's width
/// `w`, whose source pixel is from * src_w / w. Each next column is `step`
/// source pixels on, and one more when the remainder, gaining `carry`,
/// reaches w.
fn sample_run(dst: &mut [u8], line: &[u8], from: i64, src_w: i64, w: i64, step: i64, carry: i64) {
    let n = dst.len() / BPP;
    if n == 0 {
        return;
    }
    let first = from * src_w;
    let (mut sx, mut rem) = (first / w, first % w);
    // The last column's source pixel, and so every one before it, since sx
    // only grows, is inside the line: checked once, so the loop reads and
    // writes through pointers (a bounds check per pixel cost the geometry up
    // to 25%, docs/performance.md).
    let last = (from + n as i64 - 1) * src_w / w;
    assert!(sx >= 0 && last < (line.len() / 4) as i64, "image column outside its crop");
    let (mut d, s) = (dst.as_mut_ptr(), line.as_ptr());
    for _ in 0..n {
        debug_assert!(sx <= last);
        // SAFETY: 0 <= sx <= last < line.len() / 4, and d is one of dst's
        // n pixels.
        unsafe {
            let src = s.add(sx as usize * 4);
            let a = *src.add(3) as u32;
            // At 255 and 0 the blend below is the source and the destination
            // exactly.
            if a == 255 {
                *d = *src;
                *d.add(1) = *src.add(1);
                *d.add(2) = *src.add(2);
            } else if a != 0 {
                for c in 0..3 {
                    let (sv, dv) = (*src.add(c) as u32, *d.add(c) as u32);
                    *d.add(c) = ((sv * a + dv * (255 - a) + 127) / 255) as u8;
                }
            }
            d = d.add(BPP);
        }
        sx += step;
        rem += carry;
        if rem >= w {
            sx += 1;
            rem -= w;
        }
    }
}

#[cfg(test)]
#[path = "composite_tests.rs"]
mod composite_tests;
