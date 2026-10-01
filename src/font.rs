//! Font loading. stb_truetype trusts the font completely: a bad offset or
//! count in the file becomes an out-of-bounds read, an assert, or unbounded
//! recursion. check() walks every structure stb will use and rejects the font
//! unless stb's reads stay inside it. The few reads still bounded only by
//! 16-bit values (cmap format 4 and 0 lookups, hmtx for a glyph id the cmap
//! made up) are covered by zero padding after the data.

use std::fs;

/// stb's reads past a checked table start reach at most about 460 KB
/// (format 4: 16-bit offset + 2 * (16-bit codepoint delta) past 6 * 16-bit
/// segment arrays); this leaves room.
const PADDING: usize = 1 << 20;

/// Composite glyphs nest; stb recurses once per level. Real fonts use 1-3.
const MAX_COMPOSITE_DEPTH: u32 = 16;

/// Read and check a TrueType font, returning its bytes followed by PADDING
/// zero bytes, ready for draw_png.
pub fn load(path: &str) -> Result<Vec<u8>, String> {
    let data = fs::read(path).map_err(|error| format!("{path}: {error}"))?;
    prepare(data).map_err(|reason| format!("{path}: not a usable TrueType font: {reason}"))
}

/// Check font bytes and append the padding.
pub fn prepare(mut data: Vec<u8>) -> Result<Vec<u8>, String> {
    check(&data)?;
    data.resize(data.len() + PADDING, 0);
    Ok(data)
}

fn u16_at(d: &[u8], at: usize) -> Result<u16, String> {
    d.get(at..at.wrapping_add(2))
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .ok_or_else(|| format!("truncated at byte {at}"))
}

fn u32_at(d: &[u8], at: usize) -> Result<u32, String> {
    d.get(at..at.wrapping_add(4))
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| format!("truncated at byte {at}"))
}

fn is_sfnt(tag: &[u8]) -> bool {
    matches!(tag, [b'1', 0, 0, 0] | b"typ1" | b"OTTO" | [0, 1, 0, 0] | b"true")
}

/// The same offset stbtt_GetFontOffsetForIndex(data, 0) returns.
fn font_start(d: &[u8]) -> Result<usize, String> {
    let tag = d.get(0..4).ok_or("file is shorter than a font header")?;
    if is_sfnt(tag) {
        return Ok(0);
    }
    if tag == b"ttcf" && matches!(u32_at(d, 4)?, 0x0001_0000 | 0x0002_0000) {
        if (u32_at(d, 8)? as i32) < 1 {
            return Err("font collection is empty".into());
        }
        return Ok(u32_at(d, 12)? as usize);
    }
    Err("not a TrueType or OpenType file".into())
}

struct Table<'a> {
    data: &'a [u8],
}

/// The first table with this tag, as stbtt__find_table picks it.
fn table<'a>(d: &'a [u8], start: usize, tag: &[u8; 4]) -> Result<Option<Table<'a>>, String> {
    let count = u16_at(d, start + 4)? as usize;
    for i in 0..count {
        let record = start + 12 + 16 * i;
        if d.get(record..record + 4) == Some(&tag[..]) {
            let offset = u32_at(d, record + 8)? as usize;
            // stb treats an offset of 0 as a missing table.
            if offset == 0 {
                return Ok(None);
            }
            let length = u32_at(d, record + 12)? as usize;
            let data = offset
                .checked_add(length)
                .and_then(|end| d.get(offset..end))
                .ok_or_else(|| format!("table {} runs past the end of the file", String::from_utf8_lossy(tag)))?;
            return Ok(Some(Table { data }));
        }
    }
    Ok(None)
}

fn required<'a>(d: &'a [u8], start: usize, tag: &[u8; 4]) -> Result<&'a [u8], String> {
    table(d, start, tag)?
        .map(|t| t.data)
        .ok_or_else(|| format!("missing the {} table", String::from_utf8_lossy(tag)))
}

pub fn check(d: &[u8]) -> Result<(), String> {
    let start = font_start(d)?;
    let tables = u16_at(d, start + 4)? as usize;
    if start + 12 + 16 * tables > d.len() {
        return Err("table directory runs past the end of the file".into());
    }
    // Every table, used or not, must fit: a file that doesn't is damaged.
    for i in 0..tables {
        let record = start + 12 + 16 * i;
        let end = u32_at(d, record + 8)? as u64 + u32_at(d, record + 12)? as u64;
        if end > d.len() as u64 {
            return Err(format!("table {} runs past the end of the file", String::from_utf8_lossy(&d[record..record + 4])));
        }
    }
    if table(d, start, b"glyf")?.is_none() {
        return Err("no glyf table; CFF (PostScript) outlines are not supported".into());
    }
    let cmap = required(d, start, b"cmap")?;
    let head = required(d, start, b"head")?;
    let hhea = required(d, start, b"hhea")?;
    let hmtx = required(d, start, b"hmtx")?;
    let loca = required(d, start, b"loca")?;
    let glyf = required(d, start, b"glyf")?;
    let maxp = required(d, start, b"maxp")?;

    let glyph_count = u16_at(maxp, 4)? as usize;
    if glyph_count == 0 {
        return Err("maxp says the font has no glyphs".into());
    }
    u16_at(head, 52)?;
    let long_loca = match u16_at(head, 50)? {
        0 => false,
        1 => true,
        other => return Err(format!("unknown loca format {other}")),
    };
    let long_metrics = u16_at(hhea, 34)? as usize;
    if long_metrics == 0 {
        return Err("hhea has no horizontal metrics".into());
    }
    let hmtx_needed = 4 * long_metrics + 2 * glyph_count.saturating_sub(long_metrics);
    if hmtx.len() < hmtx_needed {
        return Err(format!("hmtx is {} bytes, needs {hmtx_needed}", hmtx.len()));
    }

    let mut glyphs = Vec::with_capacity(glyph_count);
    let mut previous = 0usize;
    for g in 0..=glyph_count {
        let offset = if long_loca { u32_at(loca, g * 4)? as usize } else { u16_at(loca, g * 2)? as usize * 2 };
        if offset > glyf.len() || offset < previous {
            return Err(format!("loca entry {g} points outside glyf or backwards"));
        }
        if g > 0 {
            glyphs.push(&glyf[previous..offset]);
        }
        previous = offset;
    }
    let mut components = vec![Vec::new(); glyph_count];
    for (g, glyph) in glyphs.iter().enumerate() {
        components[g] = check_glyph(glyph, glyph_count).map_err(|reason| format!("glyph {g}: {reason}"))?;
    }
    check_composite_depth(&components)?;
    check_cmap(cmap, glyph_count)
}

/// Check one glyph's outline the way stbtt__GetGlyphShapeTT reads it, and
/// return the glyphs it is composed of.
fn check_glyph(glyph: &[u8], glyph_count: usize) -> Result<Vec<usize>, String> {
    if glyph.is_empty() {
        return Ok(Vec::new());
    }
    let contours = u16_at(glyph, 0)? as i16;
    u16_at(glyph, 8)?;
    if contours > 0 {
        let contours = contours as usize;
        let mut last_end = None;
        for c in 0..contours {
            let end = u16_at(glyph, 10 + 2 * c)?;
            // stb sizes its vertex buffer for exactly this many contours.
            if last_end.is_some_and(|last| end <= last) {
                return Err("contour end points are not increasing".into());
            }
            last_end = Some(end);
        }
        let points = last_end.map_or(0, |end| end as usize + 1);
        let instructions = u16_at(glyph, 10 + 2 * contours)? as usize;
        let mut at = 12 + 2 * contours + instructions;
        let mut flags = Vec::with_capacity(points);
        while flags.len() < points {
            let flag = *glyph.get(at).ok_or("flags run past the glyph")?;
            at += 1;
            let mut repeat = 1;
            if flag & 8 != 0 {
                repeat += *glyph.get(at).ok_or("flags run past the glyph")? as usize;
                at += 1;
            }
            for _ in 0..repeat.min(points - flags.len()) {
                flags.push(flag);
            }
        }
        let xs: usize = flags.iter().map(|f| if f & 2 != 0 { 1 } else if f & 16 != 0 { 0 } else { 2 }).sum();
        let ys: usize = flags.iter().map(|f| if f & 4 != 0 { 1 } else if f & 32 != 0 { 0 } else { 2 }).sum();
        if at + xs + ys > glyph.len() {
            return Err("coordinates run past the glyph".into());
        }
        Ok(Vec::new())
    } else if contours < 0 {
        let mut parts = Vec::new();
        let mut at = 10;
        loop {
            let flags = u16_at(glyph, at)?;
            let part = u16_at(glyph, at + 2)? as usize;
            if part >= glyph_count {
                return Err(format!("component glyph {part} is out of range"));
            }
            // stb asserts on point-matched components (ARGS_ARE_XY_VALUES unset).
            if flags & 2 == 0 {
                return Err("component positioned by point matching".into());
            }
            at += 4 + if flags & 1 != 0 { 4 } else { 2 };
            at += if flags & 8 != 0 {
                2
            } else if flags & 0x40 != 0 {
                4
            } else if flags & 0x80 != 0 {
                8
            } else {
                0
            };
            if at > glyph.len() {
                return Err("component list runs past the glyph".into());
            }
            parts.push(part);
            if flags & 0x20 == 0 {
                return Ok(parts);
            }
        }
    } else {
        Ok(Vec::new())
    }
}

/// Reject composite cycles (stb would recurse forever) and deep nesting.
fn check_composite_depth(components: &[Vec<usize>]) -> Result<(), String> {
    // 0 = unvisited, 1 = on the current path, 2 = done.
    let mut state = vec![0u8; components.len()];
    fn visit(g: usize, depth: u32, components: &[Vec<usize>], state: &mut [u8]) -> Result<(), String> {
        if depth > MAX_COMPOSITE_DEPTH {
            return Err(format!("composite glyphs nest deeper than {MAX_COMPOSITE_DEPTH}"));
        }
        match state[g] {
            1 => return Err(format!("composite glyph {g} contains itself")),
            2 => return Ok(()),
            _ => {}
        }
        state[g] = 1;
        for &part in &components[g] {
            visit(part, depth + 1, components, state)?;
        }
        state[g] = 2;
        Ok(())
    }
    for g in 0..components.len() {
        visit(g, 0, components, &mut state)?;
    }
    Ok(())
}

/// Check the cmap subtable stbtt_InitFont chooses: the last Unicode one.
fn check_cmap(cmap: &[u8], glyph_count: usize) -> Result<(), String> {
    let records = u16_at(cmap, 2)? as usize;
    let mut chosen = None;
    for i in 0..records {
        let record = 4 + 8 * i;
        let platform = u16_at(cmap, record)?;
        let encoding = u16_at(cmap, record + 2)?;
        if platform == 0 || (platform == 3 && matches!(encoding, 1 | 10)) {
            chosen = Some(u32_at(cmap, record + 4)? as usize);
        }
    }
    let offset = chosen.ok_or("no Unicode cmap")?;
    let sub = cmap.get(offset..).ok_or("cmap subtable starts past the cmap table")?;
    match u16_at(sub, 0)? {
        0 => {
            let length = u16_at(sub, 2)? as usize;
            if length < 6 || length > sub.len() {
                return Err("cmap format 0 length is wrong".into());
            }
        }
        4 => {
            let segments = (u16_at(sub, 6)? >> 1) as usize;
            u16_at(sub, 12)?;
            if 16 + 8 * segments > sub.len() {
                return Err("cmap format 4 segments run past the table".into());
            }
        }
        6 => {
            let count = u16_at(sub, 8)? as usize;
            if 10 + 2 * count > sub.len() {
                return Err("cmap format 6 runs past the table".into());
            }
        }
        format @ (12 | 13) => {
            let groups = u32_at(sub, 12)? as usize;
            if groups.checked_mul(12).and_then(|n| n.checked_add(16)).map_or(true, |end| end > sub.len()) {
                return Err(format!("cmap format {format} groups run past the table"));
            }
            for k in 0..groups {
                let first = u32_at(sub, 16 + 12 * k)? as u64;
                let last = u32_at(sub, 20 + 12 * k)? as u64;
                let glyph = u32_at(sub, 24 + 12 * k)? as u64;
                let highest = if format == 12 { glyph + last.saturating_sub(first) } else { glyph };
                if first > last || highest >= glyph_count as u64 {
                    return Err(format!("cmap format {format} group {k} maps outside the font"));
                }
            }
        }
        format => return Err(format!("cmap format {format} is not supported")),
    }
    Ok(())
}
