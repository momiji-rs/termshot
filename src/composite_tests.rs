//! composite.rs tests: what tests/draw.c checked of the C it replaced
//! (layers, crops, clips, the mask of default backgrounds, the backdrop a
//! row at a time against all at once), the sampler and the blend against a
//! per-pixel transcription of the C on random views, and a panic failing the
//! render. bench/c-vs-rust/run.sh images compares it with the C itself.

use super::*;
use crate::geometry::termshot_paint_failed;
use std::ptr;

/// A canvas of w x h RGB pixels with draw.c's layout: each scanline a filter
/// byte then the pixels, 3 * w + 1 bytes apart.
struct Raster {
    buf: Vec<u8>,
    w: i32,
    h: i32,
}

impl Raster {
    fn new(w: i32, h: i32, fill: u8) -> Raster {
        Raster { buf: vec![fill; (3 * w as usize + 1) * h as usize], w, h }
    }

    fn stride(&self) -> usize {
        3 * self.w as usize + 1
    }

    fn canvas(&mut self) -> Canvas {
        let p = self.buf.as_mut_ptr();
        Canvas { px: unsafe { p.add(1) }, filtered: p, w: self.w, h: self.h, stride: self.stride(),
                 geometry: ptr::null_mut() }
    }

    fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let at = y * self.stride() + 1 + 3 * x;
        [self.buf[at], self.buf[at + 1], self.buf[at + 2]]
    }
}

fn view(pixels: &[u8], width: u32, height: u32, x: i64, y: i64, w: i64, h: i64) -> ImageView {
    ImageView { pixels: pixels.as_ptr(), width, height, x, y, w, h, clip_top: i64::MIN, clip_bottom: i64::MAX,
                clip_left: i64::MIN, clip_right: i64::MAX, src_x: 0, src_y: 0, src_w: width, src_h: height, z: 0 }
}

fn paint(r: &mut Raster, images: &[ImageView], layer: c_int) -> c_int {
    let cv = r.canvas();
    unsafe { termshot_paint_images(&cv, images.as_ptr(), images.len(), layer) }
}

fn cell(br: u8, bg: u8, bb: u8, attrs: u8) -> Cell {
    Cell { ch: ' ' as u32, fr: 0, fg: 0, fb: 0, br, bg, bb, attrs }
}

/// Negative origins, vertical clipping and partial alpha, with the filter
/// bytes and the columns past the image left alone. Source pixels expand to
/// 2x2.
#[test]
fn images_are_clipped_sampled_and_blended() {
    let pixels = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 255, 255, 0, 255];
    let mut r = Raster::new(4, 4, 0);
    let mut im = view(&pixels, 2, 2, -1, -1, 4, 4);
    im.clip_top = 0;
    im.clip_bottom = 2;
    assert_eq!(paint(&mut r, &[im], LAYER_OVER_TEXT), 0);
    let b = &r.buf;
    assert_eq!(b[1], 255); // red at (0,0)
    assert_eq!(b[5], 128); // half-alpha green at (1,0)
    assert_eq!(b[14], 0); // transparent blue at (0,1)
    assert!(b[17] == 255 && b[18] == 255); // yellow at (1,1)
    for y in 0..4 {
        assert_eq!(b[y * 13], 0); // the filter byte
        for c in 0..3 {
            assert_eq!(b[y * 13 + 10 + c], 0);
        }
    }
    assert!(b[26..].iter().all(|&v| v == 0));
}

#[test]
fn z_picks_the_layer() {
    for z in -3..=3 {
        assert_eq!(image_layer(z), if z < 0 { LAYER_UNDER_TEXT } else { LAYER_OVER_TEXT });
    }
    assert_eq!(image_layer(i32::MIN / 2), LAYER_UNDER_TEXT);
    assert_eq!(image_layer(i32::MIN / 2 - 1), LAYER_BELOW);
    assert_eq!(image_layer(-1_073_741_825), LAYER_BELOW);
    assert_eq!(image_layer(i32::MIN), LAYER_BELOW);
    assert_eq!(image_layer(i32::MAX), LAYER_OVER_TEXT);
}

/// Crops, layers and the mask of default backgrounds, on a 4x4 canvas of two
/// 2x4 cells: the first in the default background, the second red.
#[test]
fn layers_crops_and_the_mask() {
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255];
    let mut cells = [cell(17, 24, 35, 0), cell(205, 0, 0, OPAQUE)];
    // The bottom right source pixel, yellow, stretched over the canvas.
    let mut im = view(&pixels, 2, 2, 0, 0, 4, 4);
    (im.clip_top, im.clip_bottom, im.src_x, im.src_y, im.src_w, im.src_h) = (0, 4, 1, 1, 1, 1);
    for layer in [LAYER_BELOW, LAYER_UNDER_TEXT, LAYER_OVER_TEXT] {
        im.z = [i32::MIN, -1, 0][layer as usize];
        for masked in [false, true] {
            let mut r = Raster::new(4, 4, 0);
            let cv = r.canvas();
            let mask = masked.then_some(Mask { cells: &cells, cell_w: 2, cell_h: 4 });
            // Another layer paints nothing.
            unsafe { paint_images(&cv, &[im], (layer + 1) % 3, mask, 0, 4) };
            assert!(r.buf.iter().all(|&v| v == 0));
            unsafe { paint_images(&cv, &[im], layer, mask, 0, 4) };
            for y in 0..4 {
                for x in 0..4 {
                    let shown = !masked || x < 2;
                    let v = if shown { 255 } else { 0 };
                    assert_eq!(r.pixel(x, y), [v, v, 0], "layer {layer}, masked {masked}, ({x}, {y})");
                }
            }
        }
    }
    // Only OPAQUE decides, whatever the colour or other attributes.
    cells[0].attrs = OPAQUE;
    assert!(!clear_background(&cells[0]) && !clear_background(&cells[1]));
    cells[1].attrs = crate::cell::BOLD | crate::cell::ITALIC;
    assert!(clear_background(&cells[1]));
    // A crop of the top row, sampled across: red then green.
    let mut im = view(&pixels, 2, 2, 0, 0, 4, 1);
    (im.clip_top, im.clip_bottom, im.src_h) = (0, 4, 1);
    let mut r = Raster::new(4, 4, 0);
    paint(&mut r, &[im], LAYER_OVER_TEXT);
    let b = &r.buf;
    assert!(b[1] == 255 && b[4] == 255 && b[8] == 255 && b[11] == 255);
    assert!(b[2] == 0 && b[7] == 0 && b[10] == 0);
    assert!(b[13..].iter().all(|&v| v == 0));
}

/// A Unicode placeholder run shows the columns of its cells only: the image
/// is sampled as a whole and cut at clip_left and clip_right, so runs side
/// by side join without a seam.
#[test]
fn clipped_runs_join_without_a_seam() {
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255];
    let mut im = view(&pixels, 2, 2, -1, 0, 5, 4);
    (im.clip_top, im.clip_bottom) = (0, 4);
    let mut whole = Raster::new(4, 4, 0);
    paint(&mut whole, &[im], LAYER_OVER_TEXT);
    let mut r = Raster::new(4, 4, 0);
    for left in [0, 2] {
        (im.clip_left, im.clip_right) = (left, left + 2);
        paint(&mut r, &[im], LAYER_OVER_TEXT);
    }
    assert_eq!(r.buf, whole.buf);
    // One run alone leaves the other columns untouched.
    let mut r = Raster::new(4, 4, 0);
    (im.clip_left, im.clip_right) = (1, 3);
    paint(&mut r, &[im], LAYER_OVER_TEXT);
    for y in 0..4 {
        for x in 0..4 {
            assert_eq!(r.pixel(x, y), if (1..3).contains(&x) { whole.pixel(x, y) } else { [0; 3] });
        }
    }
    // An empty or reversed clip draws nothing.
    let mut r = Raster::new(4, 4, 0);
    for (left, right) in [(3, 3), (4, 1)] {
        (im.clip_left, im.clip_right) = (left, right);
        paint(&mut r, &[im], LAYER_OVER_TEXT);
    }
    assert!(r.buf.iter().all(|&v| v == 0));
}

/// The cells and views of backdrop tests: COLS x ROWS cells of CW x CH,
/// backgrounds of both kinds, and images below them and under the text that
/// overlap, cross rows of cells, hang off the canvas and blend.
const COLS: i32 = 5;
const ROWS: i32 = 4;
const CW: i32 = 3;
const CH: i32 = 5;

fn backdrop_scene(pixels: &[u8]) -> (Vec<Cell>, Vec<ImageView>) {
    let cells = (0..COLS * ROWS)
        .map(|i| cell((i * 11) as u8, 40, (255 - i) as u8, if i % 3 != 0 { 0 } else { OPAQUE }))
        .collect();
    let views = (0..3)
        .map(|k| {
            let mut v = view(pixels, 3, 2, -2 + 4 * k, 3 + 2 * k, 9, 8 + k);
            (v.clip_top, v.clip_bottom) = (0, (ROWS * CH) as i64);
            v.z = if k == 1 { -1 } else { i32::MIN };
            v
        })
        .collect();
    (cells, views)
}

/// The backdrop a row of cells at a time and all at once is the same.
#[test]
fn backdrop_rows_paint_what_the_whole_does() {
    let pixels: Vec<u8> = (0..24).map(|i: u32| (i * 37 + if i % 4 == 3 { 90 } else { 0 }) as u8).collect();
    let (cells, views) = backdrop_scene(&pixels);
    let mut out = Vec::new();
    for whole in [0, 1] {
        let mut r = Raster::new(COLS * CW, ROWS * CH, 0xee);
        let cv = r.canvas();
        let mut bd = std::mem::MaybeUninit::<Backdrop>::uninit();
        unsafe {
            termshot_backdrop_init(bd.as_mut_ptr(), &cv, cells.as_ptr(), COLS, ROWS, CW, CH, views.as_ptr(),
                                   views.len(), 16 << 20)
        };
        let mut bd = unsafe { bd.assume_init() };
        // A 15x20 raster is under 16 MiB.
        assert_eq!(bd.whole, 1);
        bd.whole = whole;
        let mut y = 1;
        while y <= ROWS * CH {
            let painted = unsafe { termshot_backdrop_through(&cv, &mut bd, y as i64) };
            assert!(painted >= 0);
            y += 3;
        }
        unsafe { termshot_backdrop_through(&cv, &mut bd, (ROWS * CH) as i64) };
        assert_eq!(bd.done, ROWS);
        // Nothing is left to paint.
        assert_eq!(unsafe { termshot_backdrop_through(&cv, &mut bd, (ROWS * CH) as i64) }, 0);
        out.push(r.buf);
    }
    assert_eq!(out[0], out[1]);
    // Every scanline's filter byte is 0, and a cell with no image over it
    // has its background.
    for y in 0..(ROWS * CH) as usize {
        assert_eq!(out[0][y * (3 * (COLS * CW) as usize + 1)], 0);
    }
    let r = Raster { buf: out.swap_remove(0), w: COLS * CW, h: ROWS * CH };
    assert_eq!(r.pixel(14, 0), [44, 40, 251]);
}

#[test]
fn backdrop_paints_rows_only_when_due() {
    let pixels = [9u8; 24];
    let (cells, _) = backdrop_scene(&pixels);
    let mut r = Raster::new(COLS * CW, ROWS * CH, 0xee);
    let cv = r.canvas();
    let mut bd = std::mem::MaybeUninit::<Backdrop>::uninit();
    unsafe { termshot_backdrop_init(bd.as_mut_ptr(), &cv, cells.as_ptr(), COLS, ROWS, CW, CH, ptr::null(), 0, 0) };
    let mut bd = unsafe { bd.assume_init() };
    assert_eq!(bd.whole, 0);
    let through = |bd: &mut Backdrop, y: i64| unsafe { termshot_backdrop_through(&cv, bd, y) };
    assert_eq!(through(&mut bd, 0), 0);
    assert_eq!(through(&mut bd, 1), 1);
    assert_eq!(bd.done, 1);
    assert_eq!(through(&mut bd, CH as i64), 0);
    assert_eq!(through(&mut bd, CH as i64 + 1), 1);
    assert_eq!(bd.done, 2);
    // Past the canvas, the last row.
    assert_eq!(through(&mut bd, 1 << 40), 1);
    assert_eq!(bd.done, ROWS);
    let stride = r.stride();
    assert!((0..(ROWS * CH) as usize).all(|y| r.buf[y * stride] == 0));
    assert_eq!(r.pixel(14, 19), [(19 * 11) as u8, 40, 255 - 19]);
}

#[test]
fn many_images_under_the_text_paint_the_backdrop_at_once() {
    let pixels = [9u8; 4];
    let mut r = Raster::new(30, 40, 0);
    let cv = r.canvas();
    let init = |views: &[ImageView]| {
        let mut bd = std::mem::MaybeUninit::<Backdrop>::uninit();
        unsafe {
            termshot_backdrop_init(bd.as_mut_ptr(), &cv, ptr::null(), 3, 2, 10, 20, views.as_ptr(), views.len(), 0);
            bd.assume_init().whole
        }
    };
    // As many under the text (below the backgrounds, or over them) as rows
    // still take.
    let mut views = vec![view(&pixels, 1, 1, 0, 0, 1, 1); BACKDROP_ROW_IMAGES];
    for (i, v) in views.iter_mut().enumerate() {
        v.z = if i % 2 == 0 { -1 } else { i32::MIN };
    }
    assert_eq!(init(&views), 0);
    // Images over the text don't count.
    views.push(view(&pixels, 1, 1, 0, 0, 1, 1));
    assert_eq!(init(&views), 0);
    views.last_mut().unwrap().z = i32::MIN;
    assert_eq!(init(&views), 1);
    // A raster under row_bytes is painted at once too.
    let raster = cv.stride * 40;
    let mut bd = std::mem::MaybeUninit::<Backdrop>::uninit();
    for (bytes, whole) in [(raster, 0), (raster + 1, 1)] {
        unsafe {
            termshot_backdrop_init(bd.as_mut_ptr(), &cv, ptr::null(), 3, 2, 10, 20, ptr::null(), 0, bytes);
            assert_eq!(bd.assume_init_ref().whole, whole);
        }
    }
}

/// A canvas with no pixels, and a null list of no images, paint nothing.
#[test]
fn nothing_to_paint_on_or_with() {
    let pixels = [9u8; 4];
    let im = view(&pixels, 1, 1, 0, 0, 30, 40);
    for (w, h) in [(30, 40), (0, 40), (30, 0)] {
        let cv = Canvas { px: ptr::null_mut(), filtered: ptr::null_mut(), w, h, stride: 91,
                          geometry: ptr::null_mut() };
        assert_eq!(unsafe { termshot_paint_images(&cv, &im, 1, LAYER_OVER_TEXT) }, 0);
    }
    let mut r = Raster::new(3, 2, 7);
    let cv = r.canvas();
    assert_eq!(unsafe { termshot_paint_images(&cv, ptr::null(), 0, LAYER_OVER_TEXT) }, 0);
    assert!(r.buf.iter().all(|&v| v == 7));
}

/// A crop outside its image, which the C would have read past, panics; the
/// panic is caught, and remembered to fail the render.
#[test]
fn a_crop_outside_the_image_fails_the_render() {
    unsafe { crate::geometry::termshot_geometry_free(crate::geometry::termshot_geometry_new(0)) };
    assert_eq!(termshot_paint_failed(), 0);
    let pixels = [9u8; 16];
    let mut im = view(&pixels, 2, 2, 0, 0, 4, 4);
    let mut r = Raster::new(4, 4, 0);
    assert_eq!(paint(&mut r, &[im], LAYER_OVER_TEXT), 0);
    assert_eq!(termshot_paint_failed(), 0);
    im.src_y = 2;
    assert_eq!(paint(&mut r, &[im], LAYER_OVER_TEXT), -1);
    assert_eq!(termshot_paint_failed(), 1);
    unsafe { crate::geometry::termshot_geometry_free(crate::geometry::termshot_geometry_new(0)) };
    im.src_y = 0;
    im.src_w = 3;
    assert_eq!(paint(&mut r, &[im], LAYER_OVER_TEXT), -1);
    assert_eq!(termshot_paint_failed(), 1);
    unsafe { crate::geometry::termshot_geometry_free(crate::geometry::termshot_geometry_new(0)) };
}

/// The C's paint_image_rows, transcribed per pixel: the source column is
/// divided out for each pixel instead of stepped, and the mask is looked up
/// for each.
fn reference(r: &mut Raster, im: &ImageView, mask: Option<(&[Cell], i64, i64)>, top: i64, bottom: i64) {
    let x0 = im.x.max(im.clip_left).max(0);
    let y0 = im.y.max(im.clip_top).max(top);
    let x1 = (im.x + im.w).min(im.clip_right).min(r.w as i64);
    let y1 = (im.y + im.h).min(im.clip_bottom).min(bottom);
    let pixels = unsafe { std::slice::from_raw_parts(im.pixels, (im.width * im.height * 4) as usize) };
    let stride = r.stride();
    for y in y0..y1 {
        for x in x0..x1 {
            if let Some((cells, cw, ch)) = mask {
                let cell = &cells[(y / ch) as usize * (r.w as i64 / cw) as usize + (x / cw) as usize];
                if cell.attrs & OPAQUE != 0 {
                    continue;
                }
            }
            let sx = im.src_x as usize + ((x - im.x) * im.src_w as i64 / im.w) as usize;
            let sy = im.src_y as usize + ((y - im.y) * im.src_h as i64 / im.h) as usize;
            let s = &pixels[(sy * im.width as usize + sx) * 4..][..4];
            let d = &mut r.buf[y as usize * stride + 1 + x as usize * 3..][..3];
            let a = s[3] as u32;
            for c in 0..3 {
                d[c] = ((s[c] as u32 * a + d[c] as u32 * (255 - a) + 127) / 255) as u8;
            }
        }
    }
}

/// Random views, crops, clips and masks against the transcription.
#[test]
fn sampling_and_blending_match_the_c_per_pixel() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move |n: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % n
    };
    for round in 0..3000 {
        let (cw, ch) = (1 + next(6) as i32, 1 + next(7) as i32);
        let (cols, rows) = (1 + next(9) as i32, 1 + next(6) as i32);
        let (width, height) = (1 + next(12) as u32, 1 + next(12) as u32);
        let pixels: Vec<u8> = (0..width * height * 4)
            .map(|i| match (i % 4, next(4)) {
                (3, 0) => 0,
                (3, 1) => 255,
                _ => next(256) as u8,
            })
            .collect();
        let (src_x, src_y) = (next(width as u64) as u32, next(height as u64) as u32);
        let mut im = view(&pixels, width, height, next(40) as i64 - 15, next(40) as i64 - 15, 1 + next(50) as i64,
                          1 + next(50) as i64);
        (im.src_x, im.src_y) = (src_x, src_y);
        im.src_w = 1 + next((width - src_x) as u64) as u32;
        im.src_h = 1 + next((height - src_y) as u64) as u32;
        if next(2) == 0 {
            im.clip_top = next(30) as i64 - 5;
            im.clip_bottom = next(40) as i64 - 5;
            im.clip_left = next(30) as i64 - 5;
            im.clip_right = next(50) as i64 - 5;
        }
        let cells: Vec<Cell> = (0..cols * rows).map(|_| cell(1, 2, 3, if next(3) == 0 { OPAQUE } else { 0 })).collect();
        let masked = next(2) == 0;
        let (top, bottom) = if next(2) == 0 { (0, (rows * ch) as i64) } else {
            let t = next((rows * ch) as u64) as i64;
            (t, t + next(10) as i64)
        };
        let mut want = Raster::new(cols * cw, rows * ch, 0);
        for (i, b) in want.buf.iter_mut().enumerate() {
            *b = (i * 31 + round) as u8;
        }
        let mut got = Raster { buf: want.buf.clone(), w: want.w, h: want.h };
        let on_canvas = bottom.min(want.h as i64);
        reference(&mut want, &im, masked.then_some((&cells[..], cw as i64, ch as i64)), top, on_canvas);
        let cv = got.canvas();
        let mask = masked.then_some(Mask { cells: &cells, cell_w: cw as i64, cell_h: ch as i64 });
        unsafe { paint_images(&cv, &[im], LAYER_OVER_TEXT, mask, top, bottom) };
        assert!(got.buf == want.buf, "round {round}");
    }
}
