//! POC: termshot's painting (draw.c) and deflate (deflate.c) ported line for
//! line to safe Rust, checked byte-identical against the C, then timed
//! against it in one process. Same algorithms; only the language differs.
//! Results and method: docs/c-vs-rust.md. Run it with bench/c-vs-rust/run.sh.
//!
//!   poc <workload dir> [rounds]   from the repo root; <name>.cells and
//!                                 <name>.meta come from the poc_workloads test
//!   ADLER=1                       also time the Rust Adler-32 alone
//!   --cfg unchecked (rustc)       drop bounds checks in deflate's two hot spots

use std::time::Instant;

#[repr(C)]
#[derive(Clone, Copy)]
struct Cell {
    ch: u32,
    fr: u8,
    fg: u8,
    fb: u8,
    br: u8,
    bg: u8,
    bb: u8,
    attrs: u8,
}

const BPP: usize = 3;
const ATTR_BOLD: u8 = 1;
const ATTR_UNDERLINE: u8 = 2;
const ATTR_DOUBLE_UNDERLINE: u8 = 4;
const ATTR_STRIKE: u8 = 8;
const GLYPH_CACHE_SIZE: usize = 256;

extern "C" {
    fn poc_fontinfo_size() -> usize;
    fn poc_metrics(ttf: *const u8, px: f64, scale: *mut f32, cw: *mut i32, ch: *mut i32, base: *mut i32);
    fn c_paint(cells: *const Cell, cols: i32, rows: i32, ttf: *const u8, scale: f32, cw: i32, ch: i32, base: i32, filtered: *mut u8);
    fn termshot_zlib_compress(data: *mut u8, len: i32, out_len: *mut i32, quality: i32) -> *mut u8;
    fn stbtt_GetFontOffsetForIndex(data: *const u8, index: i32) -> i32;
    fn stbtt_InitFont(info: *mut u8, data: *const u8, offset: i32) -> i32;
    fn stbtt_FindGlyphIndex(info: *const u8, cp: i32) -> i32;
    fn stbtt_GetGlyphBitmapBox(info: *const u8, glyph: i32, sx: f32, sy: f32, x0: *mut i32, y0: *mut i32, x1: *mut i32, y1: *mut i32);
    fn stbtt_MakeGlyphBitmap(info: *const u8, out: *mut u8, w: i32, h: i32, stride: i32, sx: f32, sy: f32, glyph: i32);
    fn free(p: *mut u8);
}

// ---------------------------------------------------------------- painting

struct Canvas<'a> {
    buf: &'a mut [u8],
    w: i32,
    h: i32,
    stride: usize,
    arc_offsets: [Option<Vec<f32>>; 4],
}

impl Canvas<'_> {
    fn put(&mut self, x: i32, y: i32, r: u8, g: u8, b: u8) {
        if (x as u32) >= self.w as u32 || (y as u32) >= self.h as u32 {
            return;
        }
        let i = 1 + y as usize * self.stride + x as usize * BPP;
        self.buf[i..i + 3].copy_from_slice(&[r, g, b]);
    }

    fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, r: u8, g: u8, b: u8) {
        let (x0, y0, x1, y1) = (x0.max(0), y0.max(0), x1.min(self.w), y1.min(self.h));
        if x0 >= x1 {
            return;
        }
        for y in y0..y1 {
            let row = 1 + y as usize * self.stride;
            for px in self.buf[row + x0 as usize * BPP..row + x1 as usize * BPP].chunks_exact_mut(BPP) {
                px.copy_from_slice(&[r, g, b]);
            }
        }
    }

    fn hbar(&mut self, x0: i32, x1: i32, mid: i32, thick: i32, r: u8, g: u8, b: u8) {
        self.fill_rect(x0, mid - thick / 2, x1, mid - thick / 2 + thick, r, g, b);
    }

    fn vbar(&mut self, mid: i32, y0: i32, y1: i32, thick: i32, r: u8, g: u8, b: u8) {
        self.fill_rect(mid - thick / 2, y0, mid - thick / 2 + thick, y1, r, g, b);
    }

    #[allow(clippy::too_many_arguments)]
    fn arc(&mut self, corner: usize, cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32, thick: f32, r: u8, g: u8, b: u8) {
        let mut steps = ((rx + ry) * 2.0) as i32;
        if steps < 12 {
            steps = 12;
        }
        let cached = self.arc_offsets[corner].is_some();
        if !cached && steps <= 2048 {
            self.arc_offsets[corner] = Some(vec![0.0; (steps as usize + 1) * 2]);
        }
        let rad = thick * 0.5;
        let rad2 = (rad + 0.6) * (rad + 0.6);
        for i in 0..=steps {
            let (ox, oy);
            if cached {
                let offsets = self.arc_offsets[corner].as_ref().unwrap();
                ox = offsets[2 * i as usize];
                oy = offsets[2 * i as usize + 1];
            } else {
                let a = a0 + (a1 - a0) * (i as f32 / steps as f32);
                ox = rx * a.cos();
                oy = ry * a.sin();
                if let Some(offsets) = self.arc_offsets[corner].as_mut() {
                    offsets[2 * i as usize] = ox;
                    offsets[2 * i as usize + 1] = oy;
                }
            }
            let px = cx + ox;
            let py = cy + oy;
            let x0 = (px - rad - 1.0).floor() as i32;
            let y0 = (py - rad - 1.0).floor() as i32;
            let x1 = (px + rad + 1.0).ceil() as i32;
            let y1 = (py + rad + 1.0).ceil() as i32;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    let dx = (x as f32 + 0.5) - px;
                    let dy = (y as f32 + 0.5) - py;
                    if dx * dx + dy * dy <= rad2 {
                        self.put(x, y, r, g, b);
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_geometry(&mut self, col: i32, row: i32, cell_w: i32, cell_h: i32, cp: u32, bold: bool, r: u8, g: u8, b: u8) -> bool {
        let x = col * cell_w;
        let y = row * cell_h;
        let right = x + cell_w;
        let bottom = y + cell_h;
        let cx = x + cell_w / 2;
        let cy = y + cell_h / 2;
        let mut t = cell_w / 12;
        if t < 1 {
            t = 1;
        }
        if bold {
            t += 1;
        }
        let jx = cx - t / 2;
        let jy = cy - t / 2;
        let pi: f32 = 3.14159265;
        let (wf, hf) = (cell_w as f32 * 0.5, cell_h as f32 * 0.5);
        match cp {
            0x2500 => self.hbar(x, right, cy, t, r, g, b),
            0x2502 => self.vbar(cx, y, bottom, t, r, g, b),
            0x250C => {
                self.hbar(jx, right, cy, t, r, g, b);
                self.vbar(cx, jy, bottom, t, r, g, b);
            }
            0x2510 => {
                self.hbar(x, jx + t, cy, t, r, g, b);
                self.vbar(cx, jy, bottom, t, r, g, b);
            }
            0x2514 => {
                self.hbar(jx, right, cy, t, r, g, b);
                self.vbar(cx, y, jy + t, t, r, g, b);
            }
            0x2518 => {
                self.hbar(x, jx + t, cy, t, r, g, b);
                self.vbar(cx, y, jy + t, t, r, g, b);
            }
            0x256D => self.arc(0, right as f32, bottom as f32, wf, hf, pi, pi * 1.5, t as f32, r, g, b),
            0x256E => self.arc(1, x as f32, bottom as f32, wf, hf, -pi * 0.5, 0.0, t as f32, r, g, b),
            0x256F => self.arc(2, x as f32, y as f32, wf, hf, 0.0, pi * 0.5, t as f32, r, g, b),
            0x2570 => self.arc(3, right as f32, y as f32, wf, hf, pi * 0.5, pi, t as f32, r, g, b),
            0x2588 => self.fill_rect(x, y, right, bottom, r, g, b),
            0x2580 => self.fill_rect(x, y, right, y + cell_h / 2, r, g, b),
            _ => return false,
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn blend(&mut self, dx: i32, dy: i32, bm: &[u8], gw: i32, gh: i32, r: u8, g: u8, b: u8) {
        let x0 = if dx < 0 { -dx } else { 0 };
        let y0 = if dy < 0 { -dy } else { 0 };
        let x1 = gw.min(self.w - dx);
        let y1 = gh.min(self.h - dy);
        let (r, g, b) = (u32::from(r), u32::from(g), u32::from(b));
        for y in y0..y1 {
            let iy = (dy + y) as usize;
            let src = &bm[(y * gw) as usize..(y * gw + gw) as usize];
            for x in x0..x1 {
                let a = u32::from(src[x as usize]);
                if a == 0 {
                    continue;
                }
                let i = 1 + iy * self.stride + (dx + x) as usize * BPP;
                let p = &mut self.buf[i..i + 3];
                if a == 255 {
                    p.copy_from_slice(&[r as u8, g as u8, b as u8]);
                } else {
                    p[0] = ((r * a + u32::from(p[0]) * (255 - a) + 127) / 255) as u8;
                    p[1] = ((g * a + u32::from(p[1]) * (255 - a) + 127) / 255) as u8;
                    p[2] = ((b * a + u32::from(p[2]) * (255 - a) + 127) / 255) as u8;
                }
            }
        }
    }
}

#[derive(Default, Clone)]
struct Glyph {
    cp: u32,
    valid: bool,
    ix0: i32,
    iy0: i32,
    w: i32,
    h: i32,
    bitmap: Option<Vec<u8>>,
}

#[allow(clippy::too_many_arguments)]
fn rust_paint(cells: &[Cell], cols: usize, rows: usize, ttf: &[u8], scale: f32, cell_w: i32, cell_h: i32, baseline: i32, filtered: &mut [u8]) {
    let mut info = vec![0u64; unsafe { poc_fontinfo_size() } / 8 + 1];
    let info = info.as_mut_ptr() as *mut u8;
    unsafe { stbtt_InitFont(info, ttf.as_ptr(), stbtt_GetFontOffsetForIndex(ttf.as_ptr(), 0)) };
    let width = cols as i32 * cell_w;
    let height = rows as i32 * cell_h;
    let stride = width as usize * BPP + 1;
    let mut cv = Canvas { buf: filtered, w: width, h: height, stride, arc_offsets: [None, None, None, None] };
    for r in 0..rows {
        let y = r * cell_h as usize;
        cv.buf[y * stride] = 0;
        for c in 0..cols {
            let cell = &cells[r * cols + c];
            cv.fill_rect(c as i32 * cell_w, y as i32, (c as i32 + 1) * cell_w, y as i32 + 1, cell.br, cell.bg, cell.bb);
        }
        for dy in 1..cell_h as usize {
            cv.buf.copy_within(y * stride..(y + 1) * stride, (y + dy) * stride);
        }
    }
    let mut cache = vec![Glyph::default(); GLYPH_CACHE_SIZE];
    for r in 0..rows {
        for c in 0..cols {
            let cell = cells[r * cols + c];
            let cp = cell.ch;
            if cp == 0 || cp == ' ' as u32 {
                continue;
            }
            if cv.paint_geometry(c as i32, r as i32, cell_w, cell_h, cp, cell.attrs & ATTR_BOLD != 0, cell.fr, cell.fg, cell.fb) {
                continue;
            }
            let entry = &mut cache[cp as usize % GLYPH_CACHE_SIZE];
            if !(entry.valid && entry.cp == cp) {
                *entry = Glyph { cp, valid: true, ..Glyph::default() };
                let glyph = unsafe { stbtt_FindGlyphIndex(info, cp as i32) };
                if glyph != 0 {
                    let (mut ix1, mut iy1) = (0, 0);
                    unsafe { stbtt_GetGlyphBitmapBox(info, glyph, scale, scale, &mut entry.ix0, &mut entry.iy0, &mut ix1, &mut iy1) };
                    entry.w = ix1 - entry.ix0;
                    entry.h = iy1 - entry.iy0;
                    if entry.w > 0 && entry.h > 0 {
                        let mut bitmap = vec![0u8; (entry.w * entry.h) as usize];
                        unsafe { stbtt_MakeGlyphBitmap(info, bitmap.as_mut_ptr(), entry.w, entry.h, entry.w, scale, scale, glyph) };
                        entry.bitmap = Some(bitmap);
                    }
                }
            }
            let Some(bm) = &entry.bitmap else { continue };
            let dx = c as i32 * cell_w + entry.ix0;
            let dy = r as i32 * cell_h + baseline + entry.iy0;
            cv.blend(dx, dy, bm, entry.w, entry.h, cell.fr, cell.fg, cell.fb);
            if cell.attrs & ATTR_BOLD != 0 {
                cv.blend(dx + 1, dy, bm, entry.w, entry.h, cell.fr, cell.fg, cell.fb);
            }
        }
    }
    let line_t = (cell_w / 12).max(1);
    let mut under_y = baseline + (cell_h - baseline) / 3;
    if under_y > cell_h - line_t {
        under_y = cell_h - line_t;
    }
    let mut double_y = if under_y + 3 * line_t <= cell_h { under_y } else { cell_h - 3 * line_t };
    if double_y < 0 {
        double_y = 0;
    }
    let strike_y = baseline - baseline * 3 / 10;
    for r in 0..rows {
        for c in 0..cols {
            let cell = cells[r * cols + c];
            if cell.attrs & (ATTR_UNDERLINE | ATTR_DOUBLE_UNDERLINE | ATTR_STRIKE) == 0 {
                continue;
            }
            let (x0, y) = (c as i32 * cell_w, r as i32 * cell_h);
            let x1 = x0 + cell_w;
            if cell.attrs & ATTR_DOUBLE_UNDERLINE != 0 {
                cv.fill_rect(x0, y + double_y, x1, y + double_y + line_t, cell.fr, cell.fg, cell.fb);
                cv.fill_rect(x0, y + double_y + 2 * line_t, x1, y + double_y + 3 * line_t, cell.fr, cell.fg, cell.fb);
            } else if cell.attrs & ATTR_UNDERLINE != 0 {
                cv.fill_rect(x0, y + under_y, x1, y + under_y + line_t, cell.fr, cell.fg, cell.fb);
            }
            if cell.attrs & ATTR_STRIKE != 0 {
                cv.fill_rect(x0, y + strike_y, x1, y + strike_y + line_t, cell.fr, cell.fg, cell.fb);
            }
        }
    }
}

// ---------------------------------------------------------------- deflate

const ZHASH: usize = 16384;
const WINDOW: usize = 32768;
const MAX_MATCH: usize = 258;

struct Out {
    p: Vec<u8>,
    bitbuf: u64,
    bitcount: u32,
}

impl Out {
    #[inline]
    fn add_bits(&mut self, code: u32, bits: u32) {
        self.bitbuf |= u64::from(code) << self.bitcount;
        self.bitcount += bits;
        while self.bitcount >= 8 {
            self.p.push(self.bitbuf as u8);
            self.bitbuf >>= 8;
            self.bitcount -= 8;
        }
    }

    #[inline]
    fn huff(&mut self, n: u32) {
        if n <= 143 {
            self.add_bits(bitrev(0x30 + n, 8), 8);
        } else if n <= 255 {
            self.add_bits(bitrev(0x190 + n - 144, 9), 9);
        } else if n <= 279 {
            self.add_bits(bitrev(n - 256, 7), 7);
        } else {
            self.add_bits(bitrev(0xc0 + n - 280, 8), 8);
        }
    }
}

/// The same bit-by-bit loop as deflate.c, not reverse_bits(), to compare languages.
#[inline]
fn bitrev(mut code: u32, mut bits: u32) -> u32 {
    let mut r = 0;
    while bits > 0 {
        r = (r << 1) | (code & 1);
        code >>= 1;
        bits -= 1;
    }
    r
}

#[inline]
fn zhash(d: &[u8]) -> usize {
    let mut h = u32::from(d[0]) + (u32::from(d[1]) << 8) + (u32::from(d[2]) << 16);
    h ^= h << 3;
    h = h.wrapping_add(h >> 5);
    h ^= h << 4;
    h = h.wrapping_add(h >> 17);
    h ^= h << 25;
    h = h.wrapping_add(h >> 6);
    h as usize & (ZHASH - 1)
}

#[cfg(unchecked)]
#[inline]
fn countm(a: &[u8], b: &[u8], limit: usize) -> usize {
    // Experiment: the same loop without bounds checks.
    let mut i = 0;
    unsafe {
        while i + 8 <= limit {
            let x = (a.as_ptr().add(i) as *const u64).read_unaligned();
            let y = (b.as_ptr().add(i) as *const u64).read_unaligned();
            if x != y {
                return i + ((x ^ y).trailing_zeros() as usize >> 3);
            }
            i += 8;
        }
        while i < limit && *a.get_unchecked(i) == *b.get_unchecked(i) {
            i += 1;
        }
    }
    i
}

#[cfg(not(unchecked))]
#[inline]
fn countm(a: &[u8], b: &[u8], limit: usize) -> usize {
    let mut i = 0;
    while i + 8 <= limit {
        let x = u64::from_le_bytes(a[i..i + 8].try_into().unwrap());
        let y = u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        if x != y {
            return i + ((x ^ y).trailing_zeros() as usize >> 3);
        }
        i += 8;
    }
    while i < limit && a[i] == b[i] {
        i += 1;
    }
    i
}

/// The literal port: slice indexing in the 32-byte loop. Kept for the record.
#[allow(dead_code)]
fn adler32_literal(mut d: &[u8]) -> u32 {
    let (mut s1, mut s2) = (1u32, 0u32);
    while !d.is_empty() {
        let mut block = d.len().min(5552);
        while block >= 32 {
            let (mut sum, mut weighted) = (0u32, 0u32);
            for k in 0..32 {
                sum += u32::from(d[k]);
                weighted += (32 - k as u32) * u32::from(d[k]);
            }
            s2 += 32 * s1 + weighted;
            s1 += sum;
            d = &d[32..];
            block -= 32;
        }
        for &byte in &d[..block] {
            s1 += u32::from(byte);
            s2 += s1;
        }
        d = &d[block..];
        s1 %= 65521;
        s2 %= 65521;
    }
    (s2 << 16) | s1
}

/// The same arithmetic over fixed 32-byte arrays, so there are no bounds
/// checks inside the block and LLVM can vectorize it as it does the C.
fn adler32(data: &[u8]) -> u32 {
    let (mut s1, mut s2) = (1u32, 0u32);
    for block in data.chunks(5552) {
        let mut chunks = block.chunks_exact(32);
        for chunk in &mut chunks {
            let chunk: &[u8; 32] = chunk.try_into().unwrap();
            let (mut sum, mut weighted) = (0u32, 0u32);
            for (k, &byte) in chunk.iter().enumerate() {
                sum += u32::from(byte);
                weighted += (32 - k as u32) * u32::from(byte);
            }
            s2 += 32 * s1 + weighted;
            s1 += sum;
        }
        for &byte in chunks.remainder() {
            s1 += u32::from(byte);
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
    }
    (s2 << 16) | s1
}

#[inline(always)]
fn byte(data: &[u8], i: usize) -> u8 {
    #[cfg(unchecked)]
    unsafe {
        return *data.get_unchecked(i);
    }
    #[cfg(not(unchecked))]
    data[i]
}

fn log2_floor(v: u32) -> u32 {
    31 - v.leading_zeros()
}

fn rust_compress(data: &[u8], quality: usize) -> Vec<u8> {
    const LENGTHC: [u32; 30] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258, 259];
    const LENGTHEB: [u32; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
    const DISTC: [u32; 31] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577, 32768];
    const DISTEB: [u32; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
    let quality = quality.max(5);
    let cap = 2 * quality;
    let mut tab = vec![0u32; ZHASH * cap];
    let mut cnt = vec![0usize; ZHASH];
    let mut o = Out { p: Vec::with_capacity(65536), bitbuf: 0, bitcount: 0 };
    o.p.push(0x78);
    o.p.push(0x5e);
    o.add_bits(1, 1);
    o.add_bits(1, 2);
    let len = data.len();
    let mut i = 0;
    while i + 3 < len {
        let h = zhash(&data[i..]);
        let base = h * cap;
        let n = cnt[h];
        let limit = (len - i).min(MAX_MATCH);
        let (mut best, mut bestpos) = (3usize, None::<usize>);
        for j in (0..n).rev() {
            let cand = tab[base + j] as usize;
            if cand + WINDOW <= i {
                break;
            }
            if bestpos.is_some() && byte(data, cand + best) != byte(data, i + best) {
                continue;
            }
            let d = countm(&data[cand..], &data[i..], limit);
            if if bestpos.is_none() { d >= best } else { d > best } {
                best = d;
                bestpos = Some(cand);
                if best == limit {
                    break;
                }
            }
        }
        let mut n = n;
        if n == cap {
            tab.copy_within(base + quality..base + cap, base);
            n = quality;
        }
        tab[base + n] = i as u32;
        cnt[h] = n + 1;

        if bestpos.is_some() {
            let limit1 = (len - i - 1).min(MAX_MATCH);
            if best < limit1 {
                let h1 = zhash(&data[i + 1..]);
                for j in (0..cnt[h1]).rev() {
                    let cand = tab[h1 * cap + j] as usize;
                    if cand + WINDOW - 1 <= i {
                        break;
                    }
                    if byte(data, cand + best) != byte(data, i + 1 + best) {
                        continue;
                    }
                    if countm(&data[cand..], &data[i + 1..], limit1) > best {
                        bestpos = None;
                        break;
                    }
                }
            }
        }

        if let Some(pos) = bestpos {
            let d = (i - pos) as u32;
            let best32 = best as u32;
            let mut j: usize = if best <= 10 {
                best - 3
            } else if best == MAX_MATCH {
                28
            } else {
                let magnitude = log2_floor(best32 - 3);
                (4 * (magnitude - 1) + ((best32 - 3) >> (magnitude - 2) & 3)) as usize
            };
            let mut bits = if j <= 22 { 7 } else { 8 };
            let mut code = if j <= 22 { bitrev(j as u32 + 1, 7) } else { bitrev(0xc0 + j as u32 - 23, 8) };
            code |= (best32 - LENGTHC[j]) << bits;
            bits += LENGTHEB[j];
            j = if d <= 4 {
                (d - 1) as usize
            } else {
                let magnitude = log2_floor(d - 1);
                (2 * magnitude + ((d - 1) >> (magnitude - 1) & 1)) as usize
            };
            code |= bitrev(j as u32, 5) << bits;
            bits += 5;
            code |= (d - DISTC[j]) << bits;
            bits += DISTEB[j];
            o.add_bits(code, bits);
            i += best;
        } else {
            o.huff(u32::from(data[i]));
            i += 1;
        }
    }
    while i < len {
        o.huff(u32::from(data[i]));
        i += 1;
    }
    o.huff(256);
    while o.bitcount != 0 {
        o.add_bits(0, 1);
    }
    if len > 0 && o.p.len() > len + 2 + (len + 32766) / 32767 * 5 {
        o.p.truncate(2);
        let mut j = 0;
        while j < len {
            let blocklen = (len - j).min(32767);
            o.p.push(u8::from(len - j == blocklen));
            o.p.extend_from_slice(&[blocklen as u8, (blocklen >> 8) as u8, !blocklen as u8, (!blocklen >> 8) as u8]);
            o.p.extend_from_slice(&data[j..j + blocklen]);
            j += blocklen;
        }
    }
    o.p.extend_from_slice(&adler32(data).to_be_bytes());
    o.p
}

fn c_compress(data: &mut [u8], quality: i32) -> Vec<u8> {
    let mut out_len = 0;
    unsafe {
        let p = termshot_zlib_compress(data.as_mut_ptr(), data.len() as i32, &mut out_len, quality);
        let v = std::slice::from_raw_parts(p, out_len as usize).to_vec();
        free(p);
        v
    }
}

// ---------------------------------------------------------------- bench

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let dir = std::env::args().nth(1).expect("workload dir");
    let rounds: usize = std::env::args().nth(2).map_or(41, |s| s.parse().unwrap());
    let ttf = {
        let mut f = std::fs::read("third_party/jetbrains-mono/JetBrainsMono-Regular.ttf").expect("run from the repo root");
        f.resize(f.len() + (1 << 20), 0);
        f
    };
    let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().map_or(false, |e| e == "meta")).collect();
    names.sort();
    println!("{:<22} {:>9} {:>10} {:>10} {:>7}   {:>10} {:>10} {:>7}", "workload", "pixels", "paint C", "paint Rust", "R/C", "deflate C", "defl Rust", "R/C");
    for meta in names {
        let text = std::fs::read_to_string(&meta).unwrap();
        let v: Vec<&str> = text.split_whitespace().collect();
        let (cols, rows, px): (usize, usize, f64) = (v[0].parse().unwrap(), v[1].parse().unwrap(), v[2].parse().unwrap());
        let raw = std::fs::read(meta.with_extension("cells")).unwrap();
        assert_eq!(raw.len(), cols * rows * 12);
        let cells: Vec<Cell> = raw
            .chunks_exact(12)
            .map(|b| Cell { ch: u32::from_ne_bytes([b[0], b[1], b[2], b[3]]), fr: b[4], fg: b[5], fb: b[6], br: b[7], bg: b[8], bb: b[9], attrs: b[10] })
            .collect();
        let (mut scale, mut cw, mut ch, mut base) = (0f32, 0, 0, 0);
        unsafe { poc_metrics(ttf.as_ptr(), px, &mut scale, &mut cw, &mut ch, &mut base) };
        let (w, h) = (cols * cw as usize, rows * ch as usize);
        let size = (w * BPP + 1) * h;
        let mut a = vec![0u8; size];
        let mut b = vec![0u8; size];
        unsafe { c_paint(cells.as_ptr(), cols as i32, rows as i32, ttf.as_ptr(), scale, cw, ch, base, a.as_mut_ptr()) };
        rust_paint(&cells, cols, rows, &ttf, scale, cw, ch, base, &mut b);
        assert!(a == b, "{}: painted pixels differ", meta.display());
        let zc = c_compress(&mut a.clone(), 8);
        let zr = rust_compress(&a, 8);
        assert!(zc == zr, "{}: compressed bytes differ ({} vs {})", meta.display(), zc.len(), zr.len());

        let (mut pc, mut pr, mut dc, mut dr) = (vec![], vec![], vec![], vec![]);
        for round in 0..rounds {
            let order = [round % 2 == 0, round % 2 != 0];
            for c_first in order {
                if c_first {
                    let t = Instant::now();
                    unsafe { c_paint(cells.as_ptr(), cols as i32, rows as i32, ttf.as_ptr(), scale, cw, ch, base, a.as_mut_ptr()) };
                    pc.push(t.elapsed().as_secs_f64() * 1e3);
                    let t = Instant::now();
                    std::hint::black_box(c_compress(&mut a, 8));
                    dc.push(t.elapsed().as_secs_f64() * 1e3);
                } else {
                    let t = Instant::now();
                    rust_paint(&cells, cols, rows, &ttf, scale, cw, ch, base, &mut b);
                    pr.push(t.elapsed().as_secs_f64() * 1e3);
                    let t = Instant::now();
                    std::hint::black_box(rust_compress(&b, 8));
                    dr.push(t.elapsed().as_secs_f64() * 1e3);
                }
            }
        }
        if std::env::var_os("ADLER").is_some() {
            let mut t = Vec::new();
            for _ in 0..21 {
                let s = Instant::now();
                std::hint::black_box(adler32(std::hint::black_box(&b)));
                t.push(s.elapsed().as_secs_f64() * 1e3);
            }
            println!("  rust adler32 alone: {:.3}ms", median(&mut t));
        }
        let (pc, pr, dc, dr) = (median(&mut pc), median(&mut pr), median(&mut dc), median(&mut dr));
        println!(
            "{:<22} {:>9} {:>8.3}ms {:>8.3}ms {:>7.2}   {:>8.3}ms {:>8.3}ms {:>7.2}",
            meta.file_stem().unwrap().to_string_lossy(), w * h, pc, pr, pr / pc, dc, dr, dr / dc
        );
    }
}
