//! CFF (Type 2 charstring) outlines, the Rust side of the #25 POC
//! (docs/cff-rust-vs-c.md). For a well-formed font it produces exactly the
//! vertices and glyph box stb_truetype 1.26 does; for anything else it returns
//! an error where stb would read out of bounds, assert, or run without end.
//! cff.c is the same design in C.

/// stb_truetype's vertex kinds.
pub const MOVE: u8 = 1;
pub const LINE: u8 = 2;
pub const CUBIC: u8 = 4;

/// stbtt_vertex, so the outline can go straight to stbtt_Rasterize.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vertex {
    pub x: i16,
    pub y: i16,
    pub cx: i16,
    pub cy: i16,
    pub cx1: i16,
    pub cy1: i16,
    pub kind: u8,
    pub padding: u8,
}

/// Charstring operators one glyph may execute, subroutines included. The
/// largest real glyph measured uses a few hundred (see the doc); this stops a
/// subroutine bomb, which stb would run for as long as it takes.
pub const MAX_OPS: u32 = 20_000;
/// The most operators any glyph has run, for the POC's report.
pub static PEAK_OPS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// stb's limits.
const MAX_STACK: usize = 48;
const MAX_SUBR_DEPTH: usize = 10;

fn u16_at(d: &[u8], at: usize) -> Result<usize, String> {
    match d.get(at..at.wrapping_add(2)) {
        Some(b) => Ok(u16::from_be_bytes([b[0], b[1]]) as usize),
        None => Err(format!("truncated at byte {at}")),
    }
}

fn u32_at(d: &[u8], at: usize) -> Result<usize, String> {
    match d.get(at..at.wrapping_add(4)) {
        Some(b) => Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize),
        None => Err(format!("truncated at byte {at}")),
    }
}

/// The offset of face `face`, as stbtt_GetFontOffsetForIndex returns it, and
/// the number of faces.
pub fn face_start(d: &[u8], face: usize) -> Result<(usize, usize), String> {
    let tag = d.get(0..4).ok_or("file is shorter than a font header")?;
    if matches!(tag, [b'1', 0, 0, 0] | b"typ1" | b"OTTO" | [0, 1, 0, 0] | b"true") {
        return if face == 0 { Ok((0, 1)) } else { Err(format!("face {face}: the file has one face")) };
    }
    if tag == b"ttcf" && matches!(u32_at(d, 4)?, 0x0001_0000 | 0x0002_0000) {
        let faces = u32_at(d, 8)?;
        if face >= faces {
            return Err(format!("face {face}: the collection has {faces}"));
        }
        return Ok((u32_at(d, 12 + 4 * face)?, faces));
    }
    Err("not a TrueType or OpenType file".into())
}

/// The first table with this tag, as stbtt__find_table picks it.
fn table<'a>(d: &'a [u8], start: usize, tag: &[u8; 4]) -> Result<Option<&'a [u8]>, String> {
    let count = u16_at(d, start + 4)?;
    for i in 0..count {
        let record = start + 12 + 16 * i;
        let entry = d.get(record..record + 16).ok_or("table directory runs past the end of the file")?;
        if &entry[..4] == tag {
            let offset = u32_at(entry, 8)?;
            if offset == 0 {
                return Ok(None);
            }
            let length = u32_at(entry, 12)?;
            return offset
                .checked_add(length)
                .and_then(|end| d.get(offset..end))
                .map(Some)
                .ok_or_else(|| format!("table {} runs past the end of the file", String::from_utf8_lossy(tag)));
        }
    }
    Ok(None)
}

/// A CFF INDEX. Its extent is checked to lie inside the table; each entry is
/// checked when it is fetched, so one bad offset costs one glyph, as in stb.
#[derive(Clone, Copy, Default)]
struct Index<'a> {
    cff: &'a [u8],
    count: usize,
    offsize: usize,
    /// Where the offset array starts.
    offsets: usize,
    /// The byte before the first object; CFF offsets are 1-based from it.
    base: usize,
    /// Just past the last object.
    end: usize,
}

impl<'a> Index<'a> {
    /// Read the INDEX at `at`; returns it and the offset just past it.
    fn read(cff: &'a [u8], at: usize) -> Result<(Index<'a>, usize), String> {
        let count = u16_at(cff, at)?;
        if count == 0 {
            return Ok((Index { cff, ..Index::default() }, at + 2));
        }
        let offsize = *cff.get(at + 2).ok_or("INDEX truncated")? as usize;
        if !(1..=4).contains(&offsize) {
            return Err(format!("INDEX at {at}: offset size {offsize}"));
        }
        let offsets = at + 3;
        let base = offsets + (count + 1) * offsize - 1;
        if base >= cff.len() {
            return Err(format!("INDEX at {at} runs past the table"));
        }
        let mut index = Index { cff, count, offsize, offsets, base, end: 0 };
        index.end = base + index.offset(count);
        if index.end > cff.len() {
            return Err(format!("INDEX at {at} runs past the table"));
        }
        Ok((index, index.end))
    }

    fn offset(&self, i: usize) -> usize {
        let at = self.offsets + i * self.offsize;
        self.cff[at..at + self.offsize].iter().fold(0, |v, &b| (v << 8) | b as usize)
    }

    /// Entry `i`; empty where its offsets are out of order or out of range,
    /// as stbtt__cff_index_get gives it.
    fn get(&self, i: usize) -> Option<&'a [u8]> {
        if i >= self.count {
            return None;
        }
        let (start, end) = (self.base + self.offset(i), self.base + self.offset(i + 1));
        Some(if start <= end && end <= self.end { &self.cff[start..end] } else { &[] })
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Operand {
    Int(i64),
    Real,
}

/// The operands of the first `key` entry in a DICT, read the way
/// stbtt__dict_get does. stb asserts on bytes 31 and 255, so they are errors.
fn dict_get(dict: &[u8], key: u16) -> Result<Vec<Operand>, String> {
    let mut at = 0;
    while at < dict.len() {
        let mut operands = Vec::new();
        loop {
            let b0 = dict.get(at).copied().unwrap_or(0);
            if b0 < 28 {
                break;
            }
            let (operand, len) = match b0 {
                28 => (u16_at(dict, at + 1).map_err(|_| "DICT truncated")? as i64, 3),
                29 => (u32_at(dict, at + 1).map_err(|_| "DICT truncated")? as i64, 5),
                30 => {
                    let mut end = at + 1;
                    while let Some(&v) = dict.get(end) {
                        end += 1;
                        if v & 0xf == 0xf || v >> 4 == 0xf {
                            break;
                        }
                    }
                    operands.push(Operand::Real);
                    at = end;
                    continue;
                }
                32..=246 => (b0 as i64 - 139, 1),
                247..=250 => ((b0 as i64 - 247) * 256 + *dict.get(at + 1).ok_or("DICT truncated")? as i64 + 108, 2),
                251..=254 => (-(b0 as i64 - 251) * 256 - *dict.get(at + 1).ok_or("DICT truncated")? as i64 - 108, 2),
                _ => return Err(format!("DICT operand byte {b0}")),
            };
            operands.push(Operand::Int(operand));
            at += len;
        }
        let mut op = dict.get(at).copied().unwrap_or(0) as u16;
        at += 1;
        if op == 12 {
            op = 0x100 | dict.get(at).copied().unwrap_or(0) as u16;
            at += 1;
        }
        if op == key {
            return Ok(operands);
        }
    }
    Ok(Vec::new())
}

/// The first `N` integer operands of `key`, zero where there are fewer;
/// stb asserts on a real number where it reads an integer.
fn dict_ints<const N: usize>(dict: &[u8], key: u16) -> Result<[usize; N], String> {
    let mut out = [0; N];
    for (slot, operand) in out.iter_mut().zip(dict_get(dict, key)?) {
        match operand {
            Operand::Int(v) if v >= 0 => *slot = v as usize,
            Operand::Int(v) => return Err(format!("DICT key {key:#x} is negative ({v})")),
            Operand::Real => return Err(format!("DICT key {key:#x} is a real number")),
        }
    }
    Ok(out)
}

/// The local Subrs of a Top DICT or Font DICT, as stbtt__get_subrs finds them.
fn read_subrs<'a>(cff: &'a [u8], dict: &[u8]) -> Result<Index<'a>, String> {
    let [size, offset] = dict_ints::<2>(dict, 18)?;
    if size == 0 || offset == 0 {
        return Ok(Index { cff, ..Index::default() });
    }
    let private = offset.checked_add(size).and_then(|end| cff.get(offset..end)).ok_or("Private DICT runs past the table")?;
    let [local] = dict_ints::<1>(private, 19)?;
    if local == 0 {
        return Ok(Index { cff, ..Index::default() });
    }
    Ok(Index::read(cff, offset + local)?.0)
}

enum FdSelect<'a> {
    /// Not a CID font: every glyph uses the Top DICT's Subrs.
    None,
    Format0(&'a [u8]),
    /// (first glyph, font dict) per range; the sentinel was checked at load.
    Format3(Vec<(usize, usize)>),
}

pub struct Font<'a> {
    pub glyphs: usize,
    charstrings: Index<'a>,
    gsubrs: Index<'a>,
    subrs: Index<'a>,
    fd_subrs: Vec<Index<'a>>,
    fdselect: FdSelect<'a>,
}

impl<'a> Font<'a> {
    /// Parse face `face` the way stbtt_InitFont does for a CFF font, checking
    /// every structure the charstrings will use.
    pub fn parse(d: &'a [u8], face: usize) -> Result<Font<'a>, String> {
        let (start, _) = face_start(d, face)?;
        if table(d, start, b"glyf")?.is_some() {
            return Err("TrueType outlines, not CFF".into());
        }
        if table(d, start, b"CFF2")?.is_some() {
            return Err("CFF2 (variable) outlines are not supported".into());
        }
        let cff = table(d, start, b"CFF ")?.ok_or("no CFF table")?;
        let glyphs = match table(d, start, b"maxp")? {
            Some(maxp) => u16_at(maxp, 4)?,
            None => 0xffff,
        };
        let header = *cff.get(2).ok_or("CFF header truncated")? as usize;
        let (_names, at) = Index::read(cff, header)?;
        let (top, at) = Index::read(cff, at)?;
        let top = top.get(0).ok_or("no Top DICT")?;
        let (_strings, at) = Index::read(cff, at)?;
        let (gsubrs, _) = Index::read(cff, at)?;
        let [charstrings] = dict_ints::<1>(top, 17)?;
        let cstype = match dict_get(top, 0x106)?.first() {
            None => 2,
            Some(Operand::Int(v)) => *v,
            Some(Operand::Real) => return Err("CharstringType is a real number".into()),
        };
        let [fdarray] = dict_ints::<1>(top, 0x124)?;
        let [fdselect_at] = dict_ints::<1>(top, 0x125)?;
        let subrs = read_subrs(cff, top)?;
        if cstype != 2 {
            return Err(format!("charstring type {cstype}"));
        }
        if charstrings == 0 {
            return Err("no CharStrings".into());
        }
        let (charstrings, _) = Index::read(cff, charstrings)?;
        if charstrings.count < glyphs {
            return Err(format!("{} charstrings for {glyphs} glyphs", charstrings.count));
        }
        let mut fd_subrs = Vec::new();
        let mut fdselect = FdSelect::None;
        if fdarray != 0 {
            if fdselect_at == 0 {
                return Err("FDArray without FDSelect".into());
            }
            let (dicts, _) = Index::read(cff, fdarray)?;
            for i in 0..dicts.count {
                fd_subrs.push(read_subrs(cff, dicts.get(i).unwrap_or(&[]))?);
            }
            let format = *cff.get(fdselect_at).ok_or("FDSelect is past the table")?;
            fdselect = match format {
                0 => {
                    let fds = cff.get(fdselect_at + 1..fdselect_at + 1 + glyphs).ok_or("FDSelect runs past the table")?;
                    if fds.iter().any(|&fd| fd as usize >= fd_subrs.len()) {
                        return Err("FDSelect names a missing font dict".into());
                    }
                    FdSelect::Format0(fds)
                }
                3 => {
                    let n = u16_at(cff, fdselect_at + 1)?;
                    let mut ranges = Vec::with_capacity(n);
                    for i in 0..n {
                        let at = fdselect_at + 3 + 3 * i;
                        let first = u16_at(cff, at)?;
                        let fd = *cff.get(at + 2).ok_or("FDSelect runs past the table")? as usize;
                        if fd >= fd_subrs.len() || ranges.last().map_or(first != 0, |&(last, _)| first <= last) {
                            return Err(format!("FDSelect range {i} is invalid"));
                        }
                        ranges.push((first, fd));
                    }
                    let sentinel = u16_at(cff, fdselect_at + 3 + 3 * n)?;
                    if ranges.last().map_or(true, |&(last, _)| sentinel <= last) || sentinel < glyphs {
                        return Err("FDSelect does not cover every glyph".into());
                    }
                    FdSelect::Format3(ranges)
                }
                other => return Err(format!("FDSelect format {other}")),
            };
        }
        Ok(Font { glyphs, charstrings, gsubrs, subrs, fd_subrs, fdselect })
    }

    fn local_subrs(&self, glyph: usize) -> Index<'a> {
        match &self.fdselect {
            FdSelect::None => self.subrs,
            FdSelect::Format0(fds) => self.fd_subrs[fds[glyph] as usize],
            FdSelect::Format3(ranges) => {
                let i = ranges.partition_point(|&(first, _)| first <= glyph) - 1;
                self.fd_subrs[ranges[i].1]
            }
        }
    }

    /// The outline of `glyph` and its box (x0, y0, x1, y1) in font units, as
    /// stbtt_GetGlyphShape and stbtt_GetGlyphBox give them. Ok(false) is a
    /// glyph stb draws as nothing (a malformed charstring, or no outline);
    /// `out` is then empty. Err is a glyph stb would assert or hang on.
    pub fn glyph(&self, glyph: usize, out: &mut Vec<Vertex>, bounds: &mut [i32; 4]) -> Result<bool, String> {
        out.clear();
        *bounds = [0; 4];
        if glyph >= self.glyphs {
            return Err(format!("glyph {glyph} is out of range"));
        }
        let mut pen = Pen {
            out,
            started: false,
            first_x: 0.0,
            first_y: 0.0,
            x: 0.0,
            y: 0.0,
            min_x: 0,
            max_x: 0,
            min_y: 0,
            max_y: 0,
        };
        let drawn = run(self, glyph, &mut pen)?;
        if !drawn || pen.out.is_empty() {
            pen.out.clear();
            return Ok(false);
        }
        *bounds = [pen.min_x, pen.min_y, pen.max_x, pen.max_y];
        Ok(true)
    }
}

/// stbtt__csctx, with both of stb's passes in one: the box from the full
/// 32-bit coordinates, the vertices truncated to 16 bits.
struct Pen<'v> {
    out: &'v mut Vec<Vertex>,
    started: bool,
    first_x: f32,
    first_y: f32,
    x: f32,
    y: f32,
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

impl Pen<'_> {
    fn track(&mut self, x: i32, y: i32) {
        if x > self.max_x || !self.started {
            self.max_x = x;
        }
        if y > self.max_y || !self.started {
            self.max_y = y;
        }
        if x < self.min_x || !self.started {
            self.min_x = x;
        }
        if y < self.min_y || !self.started {
            self.min_y = y;
        }
        self.started = true;
    }

    #[allow(clippy::too_many_arguments)]
    fn vertex(&mut self, kind: u8, x: i32, y: i32, cx: i32, cy: i32, cx1: i32, cy1: i32) {
        self.track(x, y);
        if kind == CUBIC {
            self.track(cx, cy);
            self.track(cx1, cy1);
        }
        self.out.push(Vertex {
            x: x as i16,
            y: y as i16,
            cx: cx as i16,
            cy: cy as i16,
            cx1: cx1 as i16,
            cy1: cy1 as i16,
            kind,
            padding: 0,
        });
    }

    fn close(&mut self) {
        if self.first_x != self.x || self.first_y != self.y {
            self.vertex(LINE, self.first_x as i32, self.first_y as i32, 0, 0, 0, 0);
        }
    }

    fn move_to(&mut self, dx: f32, dy: f32) {
        self.close();
        self.x += dx;
        self.y += dy;
        self.first_x = self.x;
        self.first_y = self.y;
        self.vertex(MOVE, self.x as i32, self.y as i32, 0, 0, 0, 0);
    }

    fn line_to(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
        self.vertex(LINE, self.x as i32, self.y as i32, 0, 0, 0, 0);
    }

    fn curve_to(&mut self, dx1: f32, dy1: f32, dx2: f32, dy2: f32, dx3: f32, dy3: f32) {
        let cx1 = self.x + dx1;
        let cy1 = self.y + dy1;
        let cx2 = cx1 + dx2;
        let cy2 = cy1 + dy2;
        self.x = cx2 + dx3;
        self.y = cy2 + dy3;
        self.vertex(CUBIC, self.x as i32, self.y as i32, cx1 as i32, cy1 as i32, cx2 as i32, cy2 as i32);
    }
}

/// stbtt__buf over one charstring: reads past the end give 0 and a seek past
/// the end stops at it, as in stb (where the seek also asserts).
#[derive(Clone, Copy)]
struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn get8(&mut self) -> u32 {
        match self.data.get(self.at) {
            Some(&b) => {
                self.at += 1;
                b as u32
            }
            None => 0,
        }
    }

    fn get(&mut self, n: usize) -> u32 {
        (0..n).fold(0, |v, _| (v << 8) | self.get8())
    }

    fn skip(&mut self, n: usize) {
        self.at = self.at.saturating_add(n).min(self.data.len());
    }
}

fn subr<'a>(index: &Index<'a>, n: i32) -> Option<&'a [u8]> {
    let bias = if index.count >= 33900 {
        32768
    } else if index.count >= 1240 {
        1131
    } else {
        107
    };
    let n = n.checked_add(bias)?;
    let body = index.get(usize::try_from(n).ok()?)?;
    if body.is_empty() {
        None
    } else {
        Some(body)
    }
}

/// stbtt__run_charstring. Ok(false) where stb returns 0.
fn run(font: &Font, glyph: usize, c: &mut Pen) -> Result<bool, String> {
    let mut in_header = true;
    let mut maskbits = 0usize;
    let mut stack = [0f32; MAX_STACK];
    let mut sp = 0usize;
    let mut calls: Vec<Cursor> = Vec::with_capacity(MAX_SUBR_DEPTH);
    let mut local: Option<Index> = None;
    let mut ops = 0u32;
    let mut b = Cursor { data: font.charstrings.get(glyph).unwrap_or(&[]), at: 0 };
    while b.at < b.data.len() {
        ops += 1;
        if ops > MAX_OPS {
            return Err(format!("glyph {glyph} runs more than {MAX_OPS} charstring operators"));
        }
        let mut i = 0;
        let mut clear = true;
        let s = &stack[..sp];
        let b0 = b.get8();
        match b0 {
            0x13 | 0x14 => {
                // hintmask, cntrmask; the first one implies a vstem
                if in_header {
                    maskbits += sp / 2;
                }
                in_header = false;
                b.skip((maskbits + 7) / 8);
            }
            0x01 | 0x03 | 0x12 | 0x17 => maskbits += sp / 2, // hstem, vstem, hstemhm, vstemhm
            0x15 => {
                in_header = false;
                if sp < 2 {
                    return Ok(false);
                }
                c.move_to(s[sp - 2], s[sp - 1]);
            }
            0x04 => {
                in_header = false;
                if sp < 1 {
                    return Ok(false);
                }
                c.move_to(0.0, s[sp - 1]);
            }
            0x16 => {
                in_header = false;
                if sp < 1 {
                    return Ok(false);
                }
                c.move_to(s[sp - 1], 0.0);
            }
            0x05 => {
                if sp < 2 {
                    return Ok(false);
                }
                while i + 1 < sp {
                    c.line_to(s[i], s[i + 1]);
                    i += 2;
                }
            }
            0x06 | 0x07 => {
                // hlineto, vlineto: alternate, starting horizontal or vertical
                if sp < 1 {
                    return Ok(false);
                }
                let mut horizontal = b0 == 0x06;
                while i < sp {
                    if horizontal {
                        c.line_to(s[i], 0.0);
                    } else {
                        c.line_to(0.0, s[i]);
                    }
                    horizontal = !horizontal;
                    i += 1;
                }
            }
            0x1e | 0x1f => {
                // vhcurveto, hvcurveto
                if sp < 4 {
                    return Ok(false);
                }
                let mut vertical = b0 == 0x1e;
                while i + 3 < sp {
                    let last = if sp - i == 5 { s[i + 4] } else { 0.0 };
                    if vertical {
                        c.curve_to(0.0, s[i], s[i + 1], s[i + 2], s[i + 3], last);
                    } else {
                        c.curve_to(s[i], 0.0, s[i + 1], s[i + 2], last, s[i + 3]);
                    }
                    vertical = !vertical;
                    i += 4;
                }
            }
            0x08 => {
                if sp < 6 {
                    return Ok(false);
                }
                while i + 5 < sp {
                    c.curve_to(s[i], s[i + 1], s[i + 2], s[i + 3], s[i + 4], s[i + 5]);
                    i += 6;
                }
            }
            0x18 => {
                // rcurveline
                if sp < 8 {
                    return Ok(false);
                }
                while i + 5 < sp - 2 {
                    c.curve_to(s[i], s[i + 1], s[i + 2], s[i + 3], s[i + 4], s[i + 5]);
                    i += 6;
                }
                if i + 1 >= sp {
                    return Ok(false);
                }
                c.line_to(s[i], s[i + 1]);
            }
            0x19 => {
                // rlinecurve
                if sp < 8 {
                    return Ok(false);
                }
                while i + 1 < sp - 6 {
                    c.line_to(s[i], s[i + 1]);
                    i += 2;
                }
                if i + 5 >= sp {
                    return Ok(false);
                }
                c.curve_to(s[i], s[i + 1], s[i + 2], s[i + 3], s[i + 4], s[i + 5]);
            }
            0x1a | 0x1b => {
                // vvcurveto, hhcurveto
                if sp < 4 {
                    return Ok(false);
                }
                let mut f = 0.0;
                if sp & 1 != 0 {
                    f = s[i];
                    i += 1;
                }
                while i + 3 < sp {
                    if b0 == 0x1b {
                        c.curve_to(s[i], f, s[i + 1], s[i + 2], s[i + 3], 0.0);
                    } else {
                        c.curve_to(f, s[i], s[i + 1], s[i + 2], 0.0, s[i + 3]);
                    }
                    f = 0.0;
                    i += 4;
                }
            }
            0x0a | 0x1d => {
                // callsubr, callgsubr
                if sp < 1 {
                    return Ok(false);
                }
                let index = if b0 == 0x0a { *local.get_or_insert_with(|| font.local_subrs(glyph)) } else { font.gsubrs };
                sp -= 1;
                let v = stack[sp] as i32;
                if calls.len() >= MAX_SUBR_DEPTH {
                    return Ok(false);
                }
                calls.push(b);
                match subr(&index, v) {
                    Some(body) => b = Cursor { data: body, at: 0 },
                    None => return Ok(false),
                }
                clear = false;
            }
            0x0b => {
                // return
                match calls.pop() {
                    Some(caller) => b = caller,
                    None => return Ok(false),
                }
                clear = false;
            }
            0x0e => {
                // endchar
                c.close();
                PEAK_OPS.fetch_max(ops, std::sync::atomic::Ordering::Relaxed);
                return Ok(true);
            }
            0x0c => {
                let b1 = b.get8();
                let at = |k: usize| s.get(k).copied().unwrap_or(0.0);
                match b1 {
                    0x22 => {
                        // hflex
                        if sp < 7 {
                            return Ok(false);
                        }
                        c.curve_to(at(0), 0.0, at(1), at(2), at(3), 0.0);
                        c.curve_to(at(4), 0.0, at(5), -at(2), at(6), 0.0);
                    }
                    0x23 => {
                        // flex
                        if sp < 13 {
                            return Ok(false);
                        }
                        c.curve_to(at(0), at(1), at(2), at(3), at(4), at(5));
                        c.curve_to(at(6), at(7), at(8), at(9), at(10), at(11));
                    }
                    0x24 => {
                        // hflex1
                        if sp < 9 {
                            return Ok(false);
                        }
                        c.curve_to(at(0), at(1), at(2), at(3), at(4), 0.0);
                        c.curve_to(at(5), 0.0, at(6), at(7), at(8), -(at(1) + at(3) + at(7)));
                    }
                    0x25 => {
                        // flex1
                        if sp < 11 {
                            return Ok(false);
                        }
                        let dx = at(0) + at(2) + at(4) + at(6) + at(8);
                        let dy = at(1) + at(3) + at(5) + at(7) + at(9);
                        let (mut dx6, mut dy6) = (at(10), at(10));
                        if dx.abs() > dy.abs() {
                            dy6 = -dy;
                        } else {
                            dx6 = -dx;
                        }
                        c.curve_to(at(0), at(1), at(2), at(3), at(4), at(5));
                        c.curve_to(at(6), at(7), at(8), at(9), dx6, dy6);
                    }
                    _ => return Ok(false),
                }
            }
            _ => {
                if b0 != 255 && b0 != 28 && b0 < 32 {
                    return Ok(false); // reserved operator
                }
                let f = match b0 {
                    255 => b.get(4) as i32 as f32 / 65536.0,
                    28 => b.get(2) as i16 as f32,
                    32..=246 => (b0 as i32 - 139) as i16 as f32,
                    247..=250 => ((b0 as i32 - 247) * 256 + b.get8() as i32 + 108) as i16 as f32,
                    _ => (-(b0 as i32 - 251) * 256 - b.get8() as i32 - 108) as i16 as f32,
                };
                if sp >= MAX_STACK {
                    return Ok(false);
                }
                stack[sp] = f;
                sp += 1;
                clear = false;
            }
        }
        if clear {
            sp = 0;
        }
    }
    PEAK_OPS.fetch_max(ops, std::sync::atomic::Ordering::Relaxed);
    Ok(false) // no endchar
}
