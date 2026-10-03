//! CFF (Type 2 charstring) outlines. stb_truetype runs CFF charstrings
//! without bounds: a bad offset reads out of bounds, a bad operand asserts,
//! and a subroutine bomb runs for as long as it takes. So termshot runs them
//! here instead, and stb only rasterizes the outline. For a well-formed font
//! this gives exactly the vertices and glyph box stb_truetype 1.26 does; for
//! anything else it gives an error. docs/cff-rust-vs-c.md has the reasoning
//! and the measurements.
//!
//! It also reads CFF2 tables (variable fonts), which stb can't, and draws
//! their default instance: `blend` keeps its default values and drops the
//! deltas. A CFF2 table is checked as strictly. Of its variation store only
//! the region counts are used, because they say how many operands each
//! `blend` takes.

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
/// stb's limits.
const MAX_STACK: usize = 48;
const MAX_SUBR_DEPTH: usize = 10;
/// CFF2's argument stack, in charstrings and DICTs alike. Its subroutine
/// nesting limit is stb's 10.
const MAX_STACK_CFF2: usize = 513;
/// FDSelect format 4 names a font dict in 16 bits, so a CFF2 FDArray can use
/// no more than this many; the cap also bounds what checking them costs.
const MAX_FONT_DICTS: usize = 65536;

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
        Index::read_counted(cff, at, 2)
    }

    /// A CFF2 INDEX, whose count is 32 bits.
    fn read2(cff: &'a [u8], at: usize) -> Result<(Index<'a>, usize), String> {
        Index::read_counted(cff, at, 4)
    }

    fn read_counted(cff: &'a [u8], at: usize, count_size: usize) -> Result<(Index<'a>, usize), String> {
        let count = if count_size == 2 { u16_at(cff, at)? } else { u32_at(cff, at)? };
        if count == 0 {
            return Ok((Index { cff, ..Index::default() }, at + count_size));
        }
        let offsize = *cff.get(at + count_size).ok_or("INDEX truncated")? as usize;
        if !(1..=4).contains(&offsize) {
            return Err(format!("INDEX at {at}: offset size {offsize}"));
        }
        let offsets = at + count_size + 1;
        let past = || format!("INDEX at {at} runs past the table");
        // Checked, as a 32-bit count of 4-byte offsets overflows a 32-bit usize.
        let offsets_end = count.checked_add(1).and_then(|n| n.checked_mul(offsize));
        let base = offsets_end.and_then(|n| n.checked_add(offsets - 1)).ok_or_else(past)?;
        if base >= cff.len() {
            return Err(past());
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
        while dict.get(at).map_or(false, |&b0| b0 >= 28) {
            let (operand, next) = dict_operand(dict, at)?;
            operands.push(operand);
            at = next;
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

/// The DICT operand at `at`, whose first byte is 28 or more, and the offset
/// just past it.
fn dict_operand(dict: &[u8], at: usize) -> Result<(Operand, usize), String> {
    let b0 = dict[at];
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
            return Ok((Operand::Real, end));
        }
        32..=246 => (b0 as i64 - 139, 1),
        247..=250 => ((b0 as i64 - 247) * 256 + *dict.get(at + 1).ok_or("DICT truncated")? as i64 + 108, 2),
        251..=254 => (-(b0 as i64 - 251) * 256 - *dict.get(at + 1).ok_or("DICT truncated")? as i64 - 108, 2),
        _ => return Err(format!("DICT operand byte {b0}")),
    };
    Ok((Operand::Int(operand), at + len))
}

/// The first `N` integer operands of `key`, zero where there are fewer;
/// stb asserts on a real number where it reads an integer.
fn dict_ints<const N: usize>(dict: &[u8], key: u16) -> Result<[usize; N], String> {
    ints(&dict_get(dict, key)?, key)
}

fn ints<const N: usize>(operands: &[Operand], key: u16) -> Result<[usize; N], String> {
    let mut out = [0; N];
    for (slot, &operand) in out.iter_mut().zip(operands) {
        match operand {
            Operand::Int(v) if v >= 0 => *slot = v as usize,
            Operand::Int(v) => return Err(format!("DICT key {key:#x} is negative ({v})")),
            Operand::Real => return Err(format!("DICT key {key:#x} is a real number")),
        }
    }
    Ok(out)
}

/// A CFF2 DICT: each operator and its operands, in order. Unlike a CFF DICT
/// it is read whole and strictly, as nothing else reads it. In a Private
/// DICT, `regions` gives the region count of each ItemVariationData, and
/// `blend` leaves its default values on the stack; elsewhere `blend` is an
/// error. Its `vsindex` is checked against the store.
fn dict2(dict: &[u8], regions: Option<&[usize]>) -> Result<Vec<(u16, Vec<Operand>)>, String> {
    let mut entries = Vec::new();
    let mut operands = Vec::new();
    let mut vsindex = 0;
    let mut at = 0;
    while at < dict.len() {
        if dict[at] >= 28 {
            let (operand, next) = dict_operand(dict, at)?;
            if operands.len() == MAX_STACK_CFF2 {
                return Err(format!("a DICT has more than {MAX_STACK_CFF2} operands"));
            }
            operands.push(operand);
            at = next;
            continue;
        }
        let mut op = dict[at] as u16;
        at += 1;
        if op == 12 {
            op = 0x100 | *dict.get(at).ok_or("DICT truncated")? as u16;
            at += 1;
        }
        match (op, regions) {
            (22, Some(regions)) => {
                let [index] = ints::<1>(&operands, op)?;
                if operands.len() != 1 || index >= regions.len() {
                    return Err(format!("Private DICT vsindex {index}, with {} ItemVariationData", regions.len()));
                }
                vsindex = index;
            }
            (23, Some(regions)) => {
                let depth = blend(&operands, regions.get(vsindex).copied(), vsindex)?;
                operands.truncate(depth);
                continue;
            }
            (23, None) => return Err("blend outside a Private DICT".into()),
            _ => {}
        }
        entries.push((op, std::mem::take(&mut operands)));
    }
    if !operands.is_empty() {
        return Err("DICT ends in operands with no operator".into());
    }
    Ok(entries)
}

/// The operands of the first `key` entry of a CFF2 DICT, none if it has none.
fn find(entries: &[(u16, Vec<Operand>)], key: u16) -> &[Operand] {
    entries.iter().find(|(op, _)| *op == key).map_or(&[], |(_, operands)| operands)
}

/// `blend` on a stack of DICT or charstring operands: the count n on top,
/// under it n default values and then n * regions deltas. Returns the stack
/// depth that leaves only the defaults. `regions` is the region count of the
/// ItemVariationData `vsindex` names, None where the store has no such data.
fn blend<T: Copy + Into<Count>>(stack: &[T], regions: Option<usize>, vsindex: usize) -> Result<usize, String> {
    let regions = regions.ok_or_else(|| format!("blend with vsindex {vsindex}, which names no ItemVariationData"))?;
    let Some(Count(Some(n))) = stack.last().map(|&v| v.into()) else {
        return Err("blend without a count of values".into());
    };
    let needed = n.checked_mul(regions + 1).and_then(|v| v.checked_add(1));
    match needed {
        Some(needed) if needed <= stack.len() => Ok(stack.len() - 1 - n * regions),
        _ => Err(format!("blend of {n} values over {regions} regions has only {} operands", stack.len())),
    }
}

/// A blend count: a whole number, or None.
struct Count(Option<usize>);

impl From<Operand> for Count {
    fn from(v: Operand) -> Count {
        match v {
            Operand::Int(n) => Count(usize::try_from(n).ok()),
            Operand::Real => Count(None),
        }
    }
}

impl From<f32> for Count {
    fn from(v: f32) -> Count {
        Count(if v >= 0.0 && v.fract() == 0.0 { Some(v as usize) } else { None })
    }
}

/// The region count of each ItemVariationData of the CFF2 variation store
/// at `at`: how many deltas `blend` takes per value. The deltas are dropped
/// for the default instance, so the rest of the store is only checked to
/// lie inside it.
fn read_store(cff: &[u8], at: usize) -> Result<Vec<usize>, String> {
    let length = u16_at(cff, at)?;
    let store = cff.get(at + 2..at + 2 + length).ok_or("runs past the table")?;
    let format = u16_at(store, 0)?;
    if format != 1 {
        return Err(format!("format {format}"));
    }
    let list = u32_at(store, 2)?;
    let count = u16_at(store, 6)?;
    let (axes, regions) = (u16_at(store, list)?, u16_at(store, list + 2)?);
    if list + 4 + regions * axes * 6 > store.len() {
        return Err("the region list runs past the store".into());
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let data = u32_at(store, 8 + 4 * i)?;
        let (items, words, n) = (u16_at(store, data)?, u16_at(store, data + 2)?, u16_at(store, data + 4)?);
        // The high bit of the word count makes words 32 bits and the rest 16.
        let (long, words) = (words & 0x8000 != 0, words & 0x7fff);
        if words > n {
            return Err(format!("ItemVariationData {i} has {words} word deltas of {n}"));
        }
        for j in 0..n {
            let region = u16_at(store, data + 6 + 2 * j)?;
            if region >= regions {
                return Err(format!("ItemVariationData {i} names region {region} of {regions}"));
            }
        }
        let row = if long { 4 * words + 2 * (n - words) } else { 2 * words + (n - words) };
        if data + 6 + 2 * n + items * row > store.len() {
            return Err(format!("ItemVariationData {i} runs past the store"));
        }
        out.push(n);
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

/// The local Subrs and the default vsindex of a CFF2 Font DICT, from its
/// Private DICT. `regions` is the variation store's region counts.
fn read_private2<'a>(cff: &'a [u8], dict: &[u8], regions: &[usize]) -> Result<(Index<'a>, usize), String> {
    let [size, offset] = ints::<2>(find(&dict2(dict, None)?, 18), 18)?;
    if size == 0 || offset == 0 {
        return Ok((Index { cff, ..Index::default() }, 0));
    }
    let private = offset.checked_add(size).and_then(|end| cff.get(offset..end)).ok_or("Private DICT runs past the table")?;
    let private = dict2(private, Some(regions))?;
    let [vsindex] = ints::<1>(find(&private, 22), 22)?;
    let [local] = ints::<1>(find(&private, 19), 19)?;
    if local == 0 {
        return Ok((Index { cff, ..Index::default() }, vsindex));
    }
    Ok((Index::read2(cff, offset + local)?.0, vsindex))
}

enum FdSelect<'a> {
    /// Not a CID font: every glyph uses the Top DICT's Subrs. In CFF2, a
    /// font with one font dict and no FDSelect: every glyph uses that.
    None,
    Format0(&'a [u8]),
    /// (first glyph, font dict) per range, from format 3 or CFF2's format 4;
    /// the sentinel was checked at load.
    Ranges(Vec<(usize, usize)>),
}

/// The FDSelect at `at` for `dicts` font dicts. Format 4, with 32-bit glyph
/// ids and 16-bit font dicts, is CFF2's.
fn read_fdselect(cff: &[u8], at: usize, glyphs: usize, dicts: usize, cff2: bool) -> Result<FdSelect<'_>, String> {
    let format = *cff.get(at).ok_or("FDSelect is past the table")?;
    let (count_size, glyph_size, fd_size) = match format {
        0 => {
            let fds = cff.get(at + 1..at + 1 + glyphs).ok_or("FDSelect runs past the table")?;
            if fds.iter().any(|&fd| fd as usize >= dicts) {
                return Err("FDSelect names a missing font dict".into());
            }
            return Ok(FdSelect::Format0(fds));
        }
        3 => (2, 2, 1),
        4 if cff2 => (4, 4, 2),
        other => return Err(format!("FDSelect format {other}")),
    };
    let uint = |at: usize, size: usize| {
        let bytes = cff.get(at..at.saturating_add(size)).ok_or("FDSelect runs past the table")?;
        Ok::<usize, String>(bytes.iter().fold(0, |v, &b| (v << 8) | b as usize))
    };
    let n = uint(at + 1, count_size)?;
    let first_range = at + 1 + count_size;
    let stride = glyph_size + fd_size;
    // Each range is read and checked, so a huge count stops at the table's end.
    let mut ranges = Vec::with_capacity(n.min(MAX_FONT_DICTS));
    for i in 0..n {
        let at = first_range + stride * i;
        let first = uint(at, glyph_size)?;
        let fd = uint(at + glyph_size, fd_size)?;
        if fd >= dicts || ranges.last().map_or(first != 0, |&(last, _)| first <= last) {
            return Err(format!("FDSelect range {i} is invalid"));
        }
        ranges.push((first, fd));
    }
    let sentinel = uint(first_range + stride * n, glyph_size)?;
    if ranges.last().map_or(true, |&(last, _)| sentinel <= last) || sentinel < glyphs {
        return Err("FDSelect does not cover every glyph".into());
    }
    Ok(FdSelect::Ranges(ranges))
}

pub struct Font<'a> {
    pub glyphs: usize,
    charstrings: Index<'a>,
    gsubrs: Index<'a>,
    subrs: Index<'a>,
    fd_subrs: Vec<Index<'a>>,
    fdselect: FdSelect<'a>,
    /// CFF2: charstrings without endchar or width, with blend and vsindex.
    cff2: bool,
    /// CFF2: the region count of each ItemVariationData in the store.
    regions: Vec<usize>,
    /// CFF2: each font dict's default vsindex, and for FdSelect::None the
    /// one font dict's.
    fd_vsindex: Vec<usize>,
    vsindex: usize,
}

impl<'a> Font<'a> {
    /// Parse a `CFF ` table the way stbtt_InitFont does, checking every
    /// structure the charstrings will use. `glyphs` is maxp's glyph count.
    pub fn parse(cff: &'a [u8], glyphs: usize) -> Result<Font<'a>, String> {
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
            fdselect = read_fdselect(cff, fdselect_at, glyphs, fd_subrs.len(), false)?;
        }
        let (cff2, regions, fd_vsindex, vsindex) = (false, Vec::new(), Vec::new(), 0);
        Ok(Font { glyphs, charstrings, gsubrs, subrs, fd_subrs, fdselect, cff2, regions, fd_vsindex, vsindex })
    }

    /// Parse a `CFF2` table, checking every structure the charstrings will
    /// use, as for CFF.
    pub fn parse_cff2(cff: &'a [u8], glyphs: usize) -> Result<Font<'a>, String> {
        let major = *cff.first().ok_or("header truncated")?;
        if major != 2 {
            return Err(format!("header says version {major}"));
        }
        let header = *cff.get(2).ok_or("header truncated")? as usize;
        let top_length = u16_at(cff, 3).map_err(|_| "header truncated")?;
        if header < 5 {
            return Err(format!("header size {header}"));
        }
        let top = cff.get(header..header + top_length).ok_or("Top DICT runs past the table")?;
        let (gsubrs, _) = Index::read2(cff, header + top_length)?;
        let top = dict2(top, None)?;
        let [charstrings] = ints::<1>(find(&top, 17), 17)?;
        let [fdarray] = ints::<1>(find(&top, 0x124), 0x124)?;
        let [fdselect_at] = ints::<1>(find(&top, 0x125), 0x125)?;
        let [vstore] = ints::<1>(find(&top, 24), 24)?;
        let regions = match vstore {
            0 => Vec::new(),
            at => read_store(cff, at).map_err(|reason| format!("vstore: {reason}"))?,
        };
        if charstrings == 0 {
            return Err("no CharStrings".into());
        }
        let (charstrings, _) = Index::read2(cff, charstrings)?;
        if charstrings.count < glyphs {
            return Err(format!("{} charstrings for {glyphs} glyphs", charstrings.count));
        }
        if fdarray == 0 {
            return Err("no FDArray".into());
        }
        let (dicts, _) = Index::read2(cff, fdarray)?;
        if dicts.count == 0 || dicts.count > MAX_FONT_DICTS {
            return Err(format!("FDArray has {} font dicts", dicts.count));
        }
        let (mut fd_subrs, mut fd_vsindex) = (Vec::with_capacity(dicts.count), Vec::with_capacity(dicts.count));
        for i in 0..dicts.count {
            let (subrs, vsindex) = read_private2(cff, dicts.get(i).unwrap_or(&[]), &regions)?;
            fd_subrs.push(subrs);
            fd_vsindex.push(vsindex);
        }
        let fdselect = match fdselect_at {
            0 if dicts.count > 1 => return Err(format!("FDArray has {} font dicts and no FDSelect", dicts.count)),
            0 => FdSelect::None,
            at => read_fdselect(cff, at, glyphs, dicts.count, true)?,
        };
        let (subrs, vsindex) = (fd_subrs[0], fd_vsindex[0]);
        Ok(Font { glyphs, charstrings, gsubrs, subrs, fd_subrs, fdselect, cff2: true, regions, fd_vsindex, vsindex })
    }

    /// The font dict of `glyph`, None for FdSelect::None.
    fn font_dict(&self, glyph: usize) -> Option<usize> {
        match &self.fdselect {
            FdSelect::None => None,
            FdSelect::Format0(fds) => Some(fds[glyph] as usize),
            FdSelect::Ranges(ranges) => Some(ranges[ranges.partition_point(|&(first, _)| first <= glyph) - 1].1),
        }
    }

    fn local_subrs(&self, glyph: usize) -> Index<'a> {
        self.font_dict(glyph).map_or(self.subrs, |fd| self.fd_subrs[fd])
    }

    /// The vsindex a CFF2 glyph starts with: its Private DICT's.
    fn default_vsindex(&self, glyph: usize) -> usize {
        self.font_dict(glyph).map_or(self.vsindex, |fd| self.fd_vsindex[fd])
    }

    /// The outline of `glyph` and its box (x0, y0, x1, y1) in font units, as
    /// stbtt_GetGlyphShape and stbtt_GetGlyphBox give them. Ok(false) is a
    /// glyph stb draws as nothing (a malformed charstring, or no outline);
    /// `out` is then empty. Err is a glyph stb would assert or hang on, or one
    /// past 16-bit coordinates, whose vertices stb would truncate while it
    /// sizes the bitmap from the full box. A CFF2 glyph is read as stb would
    /// read a CFF one where the two formats agree; Err is also a CFF2
    /// charstring that breaks a rule of its own (blend, vsindex, the
    /// 513-deep stack, an operator CFF2 dropped).
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
        if bounds.iter().any(|&v| i16::try_from(v).is_err()) {
            *bounds = [0; 4];
            return Err(format!("glyph {glyph} reaches past 16-bit coordinates"));
        }
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

/// stbtt__run_charstring. Ok(false) where stb returns 0. For CFF2, the same
/// with its changes: a charstring and a subroutine end where their data
/// does, as there is no endchar or return, and blend and vsindex.
fn run(font: &Font, glyph: usize, c: &mut Pen) -> Result<bool, String> {
    let mut in_header = true;
    let mut maskbits = 0usize;
    let mut stack = [0f32; MAX_STACK_CFF2];
    let max_stack = if font.cff2 { MAX_STACK_CFF2 } else { MAX_STACK };
    let mut sp = 0usize;
    let mut calls: Vec<Cursor> = Vec::with_capacity(MAX_SUBR_DEPTH);
    let mut local: Option<Index> = None;
    let mut vsindex: Option<usize> = None;
    let mut ops = 0u32;
    let mut b = Cursor { data: font.charstrings.get(glyph).unwrap_or(&[]), at: 0 };
    loop {
        if b.at >= b.data.len() {
            if !font.cff2 {
                return Ok(false); // no endchar
            }
            match calls.pop() {
                Some(caller) => {
                    b = caller;
                    continue;
                }
                None => {
                    c.close();
                    return Ok(true);
                }
            }
        }
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
            0x0b | 0x0e if font.cff2 => {
                let name = if b0 == 0x0b { "return" } else { "endchar" };
                return Err(format!("glyph {glyph}: {name} is not a CFF2 operator"));
            }
            0x0f if font.cff2 => {
                // vsindex
                let Some(&v) = s.last() else {
                    return Err(format!("glyph {glyph}: vsindex with no operand"));
                };
                let index = match Count::from(v) {
                    Count(Some(index)) if index < font.regions.len() => index,
                    _ => return Err(format!("glyph {glyph}: vsindex {v}, with {} ItemVariationData", font.regions.len())),
                };
                vsindex = Some(index);
            }
            0x10 if font.cff2 => {
                // blend
                let index = *vsindex.get_or_insert_with(|| font.default_vsindex(glyph));
                let regions = font.regions.get(index).copied();
                sp = blend(s, regions, index).map_err(|reason| format!("glyph {glyph}: {reason}"))?;
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
                if sp >= max_stack {
                    if font.cff2 {
                        return Err(format!("glyph {glyph} has more than {MAX_STACK_CFF2} operands on the stack"));
                    }
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
}
