//! Decode Sixel images (DCS P1;P2;P3 q ... ST) as xterm does, into RGBA that
//! goes to the kitty image store in graphics.rs like any other image.
//!
//! The semantics follow xterm's graphics_sixel.c (patch 412):
//! - Pixels are 1:1 device pixels. P1, P3 and the raster aspect Pan;Pad are
//!   read and ignored, as xterm forces its pixel aspect to 1:1.
//! - Each image has its own 1,024 colour registers, starting from the VT340's
//!   16 colours (the rest black), as xterm's default private colour registers.
//!   Drawing selects register 3 until a `#` says otherwise, as xterm does.
//! - Pixels keep their register, so redefining a register later recolours
//!   what was drawn with it: the palette at the end of the image applies.
//! - P2 1 leaves pixels no sixel set transparent. P2 0 or 2 paints the area
//!   the raster attributes declared before the first sixel, the largest of
//!   them, with register 0; pixels outside it that nothing set stay
//!   transparent.
//! - The image is as wide and tall as the raster attributes declared or its
//!   set pixels reach, whichever is larger.

#[cfg(test)]
mod tests;

const REGISTERS: usize = 1024;
/// A pixel no sixel set.
const HOLE: u16 = u16::MAX;
/// The kitty upload limits in graphics.rs: 8,192 pixels per axis and 16 MiB
/// of RGBA.
const MAX_AXIS: usize = 8192;
const MAX_PIXELS: usize = 16 * 1024 * 1024 / 4;
/// A few bytes can declare a large image or draw over the same pixels again
/// and again, so a log may only write so many pixels: four of the largest
/// images, plus 256 for each byte of Sixel data, about what a byte of a
/// zlib-compressed kitty upload can give.
const BUDGET_BASE: usize = 4 * MAX_PIXELS;
const BUDGET_PER_BYTE: usize = 256;

/// The pixel writes the rest of a log may still make: every pixel a sixel
/// sets, each time it sets it, and every pixel of each finished image.
pub struct Budget(usize);

impl Default for Budget {
    fn default() -> Self {
        Self(BUDGET_BASE)
    }
}

/// The VT340's colour registers 0 to 15 in percent, as xterm sets them.
const VT340: [[u8; 3]; 16] = [
    [0, 0, 0],
    [20, 20, 80],
    [80, 13, 13],
    [20, 80, 20],
    [80, 20, 80],
    [20, 80, 80],
    [80, 80, 20],
    [53, 53, 53],
    [26, 26, 26],
    [33, 33, 60],
    [60, 26, 26],
    [33, 60, 33],
    [60, 33, 60],
    [33, 60, 60],
    [60, 60, 33],
    [80, 80, 80],
];

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A percentage as an 8-bit channel, rounded half up as libsixel does.
fn channel(percent: u32) -> u8 {
    ((percent * 255 + 50) / 100) as u8
}

fn rgb(percent: [u32; 3]) -> [u8; 3] {
    percent.map(channel)
}

/// xterm's hls2rgb: DEC hue (0 is blue, 120 red, 240 green), lightness and
/// saturation in percent, to RGB in percent. f64 arithmetic is the same on
/// every platform, since rustc never fuses multiply-adds.
fn hls(h: u32, l: u32, s: u32) -> [u32; 3] {
    if s == 0 {
        return [l; 3];
    }
    let h = (h + 360 - 120) % 360;
    let hs = ((h % 120) as f64 - 60.0).abs();
    let (lv, sv) = (f64::from(l) / 100.0, f64::from(s) / 100.0);
    let c = (1.0 - (2.0 * lv - 1.0).abs()) * sv;
    let x = (60.0 - hs) / 60.0 * c;
    let m = lv - 0.5 * c;
    let (r, g, b) = match h / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r, g, b].map(|v| ((v + m) * 100.0 + 0.5).clamp(0.0, 100.0) as u32)
}

/// The data of a Sixel DCS and its P2, from the bytes between ESC P and ST:
/// parameters (digits and `;`), no intermediates, and the final `q`. C0
/// controls and DEL in the header are ignored, as a DEC parser ignores them
/// there. Other DCS strings (DECRQSS `$q`, XTGETTCAP `+q`, ...) are not Sixel.
fn split(body: &[u8]) -> Option<(u32, &[u8])> {
    let ignored = |b: u8| b < 0x20 || b == 0x7f;
    let q = body.iter().position(|&b| !ignored(b) && !matches!(b, b'0'..=b'9' | b';'))?;
    if body[q] != b'q' {
        return None;
    }
    let mut params = body[..q].iter().copied().filter(|&b| !ignored(b));
    let p2 = params.by_ref().skip_while(|&b| b != b';').skip(1).take_while(|&b| b != b';');
    let p2 = p2.fold(0u32, |n, d| n.saturating_mul(10).saturating_add(u32::from(d - b'0')));
    Some((p2, &body[q + 1..]))
}

/// Whether this DCS string (its bytes after ESC P, through the terminator
/// `crate::vt::skip_string` stopped after) is a Sixel image that ST commits, so
/// its placement may move the text cursor by rows of the font's cell height.
/// `crate::vt::needs_cell_metrics` hands it every DCS string of a log.
pub fn string_needs_cell_metrics(s: &[u8]) -> bool {
    s.strip_suffix(b"\x1b\\").is_some_and(|body| split(body).is_some())
}

/// The Sixel half of `crate::vt::needs_cell_metrics`, for tests.
#[cfg(test)]
pub fn needs_cell_metrics(data: &[u8]) -> bool {
    crate::vt::any_string(data, |kind, s| kind == b'P' && string_needs_cell_metrics(s))
}

/// The byte-at-a-time scan `string_needs_cell_metrics` replaced (main
/// `a8a95e0`), kept as the reference the differential test compares it with.
#[cfg(test)]
pub fn needs_cell_metrics_reference(data: &[u8]) -> bool {
    let mut i = 0;
    while i + 1 < data.len() {
        if data[i] == 0x1b && matches!(data[i + 1], b']' | b'P' | b'_' | b'^' | b'X') {
            let start = i + 2;
            let end = crate::vt::skip_string(data, start);
            if data[i + 1] == b'P'
                && end >= start + 2
                && data.get(end - 2..end) == Some(b"\x1b\\")
                && split(&data[start..end - 2]).is_some()
            {
                return true;
            }
            i = end;
        } else {
            i += 1;
        }
    }
    false
}

struct Decoder {
    /// One register per pixel, row by row; rows are only as long as their
    /// rightmost set pixel.
    rows: Vec<Vec<u16>>,
    width: usize,
    height: usize,
    /// The area painted with register 0, fixed at the first sixel.
    fill: Option<(usize, usize)>,
    opaque: bool,
    started: bool,
    x: usize,
    /// The top of the current band.
    y: usize,
    color: u16,
    palette: Vec<[u8; 3]>,
    /// What is left of the log's budget.
    work: usize,
}

impl Decoder {
    fn new(opaque: bool, work: usize) -> Self {
        let mut palette = vec![[0; 3]; REGISTERS];
        for (reg, percent) in palette.iter_mut().zip(VT340) {
            *reg = rgb(percent.map(u32::from));
        }
        Self {
            rows: Vec::new(),
            width: 0,
            height: 0,
            fill: None,
            opaque,
            started: false,
            x: 0,
            y: 0,
            color: 3,
            palette,
            work,
        }
    }

    /// Grow the image to at least w x h, or refuse it past the limits or
    /// once the budget could not pay for finishing it.
    fn extend(&mut self, w: usize, h: usize) -> Option<()> {
        let (w, h) = (w.max(self.width), h.max(self.height));
        if w > MAX_AXIS || h > MAX_AXIS || w * h > MAX_PIXELS || w * h > self.work {
            return None;
        }
        (self.width, self.height) = (w, h);
        Some(())
    }

    /// `count` columns of one sixel (6 bits, the lowest on top) at the cursor.
    /// The limits and the budget bound the columns before any are written.
    fn sixel(&mut self, bits: u8, count: usize) -> Option<()> {
        if !self.started {
            self.started = true;
            if self.opaque {
                // Only raster attributes can have sized the image so far.
                self.fill = Some((self.width, self.height));
            }
        }
        if bits == 0 {
            self.x = self.x.saturating_add(count);
            return Some(());
        }
        let end = self.x.checked_add(count)?;
        let top_bit = 7 - bits.leading_zeros() as usize;
        self.extend(end, self.y.checked_add(top_bit + 1)?)?;
        self.work = self.work.checked_sub(count * bits.count_ones() as usize)?;
        if self.rows.len() < self.height {
            self.rows.resize_with(self.height, Vec::new);
        }
        for bit in 0..=top_bit {
            if bits & 1 << bit != 0 {
                let row = &mut self.rows[self.y + bit];
                if row.len() < end {
                    row.resize(end, HOLE);
                }
                row[self.x..end].fill(self.color);
            }
        }
        self.x = end;
        Some(())
    }

    /// `#Pc` selects a register; `#Pc;Pu;Px;Py;Pz` also defines it, in HLS
    /// (Pu 1) or RGB (Pu 2). A definition out of range, of another colour
    /// space or with the wrong number of parameters is ignored; the register
    /// is still selected, as xterm selects it before reading the rest.
    fn color(&mut self, params: &[Option<u32>]) {
        let Some(Some(reg)) = params.first() else {
            return;
        };
        self.color = (reg % REGISTERS as u32) as u16;
        let [_, Some(space), Some(a), Some(b), Some(c)] = *params else {
            return;
        };
        let percent = match space {
            1 if a <= 360 && b <= 100 && c <= 100 => hls(a, b, c),
            2 if a <= 100 && b <= 100 && c <= 100 => [a, b, c],
            _ => return,
        };
        self.palette[usize::from(self.color)] = rgb(percent);
    }

    /// `"Pan;Pad;Ph;Pv`: the aspect is ignored; the size grows the image.
    /// As in xterm's GetExtent, an extent that is given, even as 0 or empty,
    /// is at least 1; one left out declares nothing.
    fn raster(&mut self, params: &[Option<u32>]) -> Option<()> {
        let get = |i: usize| params.get(i).map_or(0, |v| v.unwrap_or(0).max(1) as usize);
        self.extend(get(2), get(3))
    }

    fn run(&mut self, data: &[u8]) -> Option<()> {
        let mut i = 0;
        while let Some(&c) = data.get(i) {
            i += 1;
            match c {
                0x3f..=0x7e => self.sixel(c - 0x3f, 1)?,
                // A repeat with no count or 0 draws once. Followed by anything
                // but a sixel, xterm drops both.
                b'!' => {
                    let count = number(data, &mut i).unwrap_or(1).max(1);
                    if let Some(&s @ 0x3f..=0x7e) = data.get(i) {
                        self.sixel(s - 0x3f, count as usize)?;
                    }
                    i += 1;
                }
                b'#' => self.color(&params(data, &mut i)),
                b'"' => self.raster(&params(data, &mut i))?,
                b'$' => self.x = 0,
                b'-' => {
                    self.x = 0;
                    self.y = self.y.saturating_add(6);
                }
                _ => {}
            }
        }
        Some(())
    }

    fn finish(&mut self) -> Option<Image> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        self.work = self.work.checked_sub(self.width * self.height)?;
        let (fw, fh) = self.fill.unwrap_or((0, 0));
        let mut rgba = Vec::with_capacity(self.width * self.height * 4);
        let empty = Vec::new();
        for y in 0..self.height {
            let row = self.rows.get(y).unwrap_or(&empty);
            for x in 0..self.width {
                let reg = row.get(x).copied().unwrap_or(HOLE);
                let pixel = if reg != HOLE {
                    Some(self.palette[usize::from(reg)])
                } else if x < fw && y < fh {
                    Some(self.palette[0])
                } else {
                    None
                };
                match pixel {
                    Some([r, g, b]) => rgba.extend_from_slice(&[r, g, b, 255]),
                    None => rgba.extend_from_slice(&[0; 4]),
                }
            }
        }
        Some(Image { width: self.width as u32, height: self.height as u32, rgba })
    }
}

/// A number made of the digits at the front of `data`, saturating, if any.
fn number(data: &[u8], i: &mut usize) -> Option<u32> {
    let start = *i;
    let mut n = 0u32;
    while let Some(d) = data.get(*i).filter(|d| d.is_ascii_digit()) {
        n = n.saturating_mul(10).saturating_add(u32::from(d - b'0'));
        *i += 1;
    }
    (*i > start).then_some(n)
}

/// `;`-separated numbers, each possibly empty, up to 8 kept.
fn params(data: &[u8], i: &mut usize) -> Vec<Option<u32>> {
    let mut list = vec![number(data, i)];
    while data.get(*i) == Some(&b';') {
        *i += 1;
        let n = number(data, i);
        if list.len() < 8 {
            list.push(n);
        }
    }
    list
}

/// Decode the bytes between ESC P and ST, if they are a Sixel image with at
/// least one pixel inside the limits and the budget. An image past them is
/// refused whole, before the pixels past them are allocated or written; the
/// writes it made before that stay paid for.
pub fn decode(body: &[u8], budget: &mut Budget) -> Option<Image> {
    let (p2, data) = split(body)?;
    budget.0 = budget.0.saturating_add(data.len().saturating_mul(BUDGET_PER_BYTE));
    // Spaces, controls and bytes past ASCII are ignored, even inside numbers.
    let data: Vec<u8> = data.iter().copied().filter(|b| (0x21..0x7f).contains(b)).collect();
    let mut decoder = Decoder::new(p2 != 1, budget.0);
    let image = decoder.run(&data).and_then(|()| decoder.finish());
    budget.0 = decoder.work;
    image
}

/// A kitty command that transmits and places the image at the cursor at its
/// native size, leaving the cursor where it is: the existing store's API.
pub fn kitty_command(image: &Image) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = format!("a=T,f=32,s={},v={},C=1,q=2;", image.width, image.height).into_bytes();
    out.reserve(image.rgba.len() / 3 * 4 + 4);
    for chunk in image.rgba.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (k, &b)| n | u32::from(b) << (16 - 8 * k));
        for k in 0..4 {
            out.push(if k <= chunk.len() { ALPHABET[(n >> (18 - 6 * k) & 63) as usize] } else { b'=' });
        }
    }
    out
}
