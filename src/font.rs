//! Font loading. stb_truetype trusts the font completely: a bad offset or
//! count in the file becomes an out-of-bounds read, an assert, or unbounded
//! recursion. check() walks every structure stb will use and rejects the font
//! unless stb's reads stay inside it. The few reads still bounded only by
//! 16-bit values (cmap format 4 and 0 lookups, hmtx for a glyph id the cmap
//! made up) are covered by zero padding after the data. A CFF font's
//! outlines never reach stb: src/cff.rs runs its charstrings, and draw.c gets
//! the outlines through a Face.

use crate::cff;
use crate::metrics;
use crate::variations;
use std::ffi::{c_int, c_void};
use std::marker::PhantomData;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

/// Where loading a font goes, for TERMSHOT_PROFILE. The built-in font and a
/// file share four boundaries, which do not overlap:
/// - allocate: an empty buffer the size of the font;
/// - read: filling it; for a file also opening and sizing it, for the
///   built-in font copying it out of the binary;
/// - check: picking the face and checking it;
/// - padding: appending PADDING zero bytes, which may move the buffer.
#[derive(Default, Clone, Copy)]
pub struct LoadTimings {
    pub allocate_ms: f64,
    pub read_ms: f64,
    pub check_ms: f64,
    pub padding_ms: f64,
    /// The font's size, before padding.
    pub bytes: usize,
}

/// stb's reads past a checked table start reach at most about 460 KB
/// (format 4: 16-bit offset + 2 * (16-bit codepoint delta) past 6 * 16-bit
/// segment arrays); this leaves room.
const PADDING: usize = 1 << 20;

/// Composite glyphs nest; stb recurses once per level. Real fonts use 1-3.
const MAX_COMPOSITE_DEPTH: u32 = 16;

/// The most faces a hint or an error lists.
const MAX_LISTED: usize = 32;

/// The most characters of a face name that are read and printed.
const NAME_CHARS: usize = 100;

/// The most faces a collection may hold. Real ones hold tens; the bound keeps
/// a search by name, which reads each face's name table, short.
const MAX_FACES: usize = 1024;

/// A checked font, ready for draw_png.
pub struct Font {
    /// The file, followed by PADDING zero bytes.
    pub data: Vec<u8>,
    /// Where the face to draw with starts: 0 for a single font, else the
    /// offset a collection gives for it.
    pub start: usize,
    /// For a collection, the face drawn with and its name, for -v.
    pub face: Option<(usize, String)>,
    /// Set when a collection of several faces was given without picking one:
    /// what was used and how to pick another.
    pub hint: Option<String>,
    /// The instance of a CFF2 face drawn: normalized coordinates, as
    /// cff::Font::parse_cff2 takes them.
    pub coords: Vec<i32>,
    /// The axis settings that picked it, for -v.
    pub instance: Option<String>,
}

/// A --font value: the file, the face named after a `#`, if any, and the
/// axis settings after a last `#`, if any.
#[derive(Debug, PartialEq)]
pub struct Spec {
    pub path: String,
    pub face: Option<String>,
    pub axes: Option<String>,
}

impl Spec {
    /// A file named `a#1` is that file; otherwise `a.ttc#1` is face 1 of
    /// `a.ttc`. The path is the prefix before a `#` that is a file, so a face
    /// name may hold `#` too, as in `a.ttc#Foo #1`; with none, it ends at the
    /// first `#`. Two such prefixes, as when `a.ttc` and `a.ttc#Foo` both
    /// exist, are refused rather than guessed between. Decided once, before
    /// any output is created, so that creating one can't change what a value
    /// means.
    ///
    /// After the path, a last `#` part with a `=` in it is axis settings, as
    /// in `a.otf#wght=700` or `a.ttc#1#wght=700`, checked here as
    /// variations::parse reads them. So a face whose name has a `=` in it is
    /// picked by number, or with axis settings after it.
    pub fn parse(value: &str) -> Result<Spec, String> {
        if Path::new(value).exists() {
            return Ok(Spec { path: value.into(), face: None, axes: None });
        }
        let files: Vec<usize> =
            value.match_indices('#').map(|(at, _)| at).filter(|&at| Path::new(&value[..at]).is_file()).collect();
        let at = match files[..] {
            [] => value.find('#'),
            [at] => Some(at),
            [first, second, ..] => {
                let (first, second) = (&value[..first], &value[..second]);
                return Err(format!("{value:?} could name a face of {first} or of {second}; rename one of them"));
            }
        };
        let Some(at) = at else {
            return Ok(Spec { path: value.into(), face: None, axes: None });
        };
        let (path, rest) = (&value[..at], &value[at + 1..]);
        let (before, last) = rest.rsplit_once('#').map_or((None, rest), |(before, last)| (Some(before), last));
        let (face, axes) = if last.contains('=') { (before, Some(last)) } else { (Some(rest), None) };
        if let Some(axes) = axes {
            variations::parse(axes).map_err(|reason| format!("{value}: {reason}"))?;
        }
        Ok(Spec { path: path.into(), face: face.map(Into::into), axes: axes.map(Into::into) })
    }

    /// The value as given: the path, then the face and axis settings.
    pub fn name(&self) -> String {
        let mut name = self.path.clone();
        for part in [&self.face, &self.axes].into_iter().flatten() {
            name += &format!("#{part}");
        }
        name
    }
}

/// Read a font file and check the face `spec` names.
#[cfg(test)]
pub fn load(spec: &Spec) -> Result<Font, String> {
    load_timed(spec, &mut LoadTimings::default())
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// The same as load(), with the clocks LoadTimings describes.
pub fn load_timed(spec: &Spec, timings: &mut LoadTimings) -> Result<Font, String> {
    let path = &spec.path;
    let error = |error: std::io::Error| format!("{path}: {error}");
    let started = Instant::now();
    let mut file = fs::File::open(path).map_err(error)?;
    // Sized as fs::read sizes it: a hint, which read_to_end grows past.
    let len = file.metadata().map_or(0, |m| m.len() as usize);
    let opened = ms(started);
    let started = Instant::now();
    let mut data = Vec::with_capacity(len);
    timings.allocate_ms = ms(started);
    let started = Instant::now();
    file.read_to_end(&mut data).map_err(error)?;
    timings.read_ms = opened + ms(started);
    timings.bytes = data.len();
    let started = Instant::now();
    let mut font = choose(data, spec.face.as_deref(), path)?;
    if let Some(axes) = &spec.axes {
        vary(&mut font, axes, path)?;
    }
    timings.check_ms = ms(started);
    let started = Instant::now();
    let font = pad(font);
    timings.padding_ms = ms(started);
    Ok(font)
}

/// Check font bytes (the first face of a collection) and append the padding.
#[cfg(test)]
pub fn prepare(data: Vec<u8>) -> Result<Font, String> {
    Ok(pad(check_first(data)?))
}

/// prepare() for bytes it copies, the built-in font's, with the clocks
/// LoadTimings describes.
pub fn prepare_timed(bytes: &[u8], timings: &mut LoadTimings) -> Result<Font, String> {
    let started = Instant::now();
    let mut data = Vec::with_capacity(bytes.len());
    timings.allocate_ms = ms(started);
    let started = Instant::now();
    data.extend_from_slice(bytes);
    timings.read_ms = ms(started);
    timings.bytes = data.len();
    let started = Instant::now();
    let font = check_first(data)?;
    timings.check_ms = ms(started);
    let started = Instant::now();
    let font = pad(font);
    timings.padding_ms = ms(started);
    Ok(font)
}

fn check_first(data: Vec<u8>) -> Result<Font, String> {
    let start = faces(&data)?[0].ok_or("face #0 is not a font")?;
    check_at(&data, start)?;
    Ok(Font { data, start, face: None, hint: None, coords: Vec::new(), instance: None })
}

fn pad(mut font: Font) -> Font {
    font.data.resize(font.data.len() + PADDING, 0);
    font
}

/// draw.c's Face: a checked font, for a CFF font the outlines stb must not
/// read itself, and at an instance the metrics HVAR and MVAR vary.
#[repr(C)]
pub struct Face<'a> {
    ttf: *const u8,
    start: c_int,
    outline: Option<unsafe extern "C" fn(*const c_void, c_int, *mut cff::Vertex, c_int, *mut c_int) -> c_int>,
    cff: *const c_void,
    advance: Option<unsafe extern "C" fn(*const c_void, c_int, c_int) -> c_int>,
    advances: *const c_void,
    /// Set when ascent, descent and line_gap replace hhea's.
    varied: c_int,
    ascent: c_int,
    descent: c_int,
    line_gap: c_int,
    font: PhantomData<&'a Font>,
}

/// The metrics of a face at its instance, where it varies them; neither at
/// the default instance.
struct Varied<'a> {
    advances: Option<metrics::Advances<'a>>,
    vertical: Option<[i32; 3]>,
}

/// Read the HVAR and MVAR of the face at `start` for the instance at
/// `coords`, as cff::Font::parse_cff2 takes them.
fn varied<'a>(d: &'a [u8], start: usize, coords: &[i32]) -> Result<Varied<'a>, String> {
    if coords.is_empty() {
        return Ok(Varied { advances: None, vertical: None });
    }
    let advances = match table(d, start, b"HVAR")? {
        Some(hvar) => Some(metrics::Advances::parse(hvar.data, coords).map_err(|reason| format!("HVAR table: {reason}"))?),
        None => None,
    };
    let vertical = match table(d, start, b"MVAR")? {
        Some(mvar) => Some(
            metrics::vertical(required(d, start, b"hhea")?, mvar.data, coords)
                .map_err(|reason| format!("MVAR table: {reason}"))?,
        ),
        None => None,
    };
    // HarfBuzz varies advances only away from the default, and MVAR at any
    // instance chosen.
    let advances = advances.filter(|_| coords.iter().any(|&c| c != 0));
    Ok(Varied { advances, vertical })
}

impl Font {
    /// Call `f` with the face to draw with. The CFF table and the variation
    /// tables, checked at load, are parsed again here, as what they parse
    /// into borrows the data.
    pub fn with_face<R>(&self, f: impl FnOnce(&Face) -> R) -> Result<R, String> {
        let cff = cff_outlines(&self.data, self.start, &self.coords)?;
        let varied = varied(&self.data, self.start, &self.coords)?;
        Ok(f(&self.face(cff.as_ref(), &varied)))
    }

    /// Call `f` with the face's metrics and no outlines, for
    /// draw_face_cell_size, which reads no glyph; this skips parsing the
    /// CFF table.
    pub fn with_metrics<R>(&self, f: impl FnOnce(&Face) -> R) -> Result<R, String> {
        Ok(f(&self.face(None, &varied(&self.data, self.start, &self.coords)?)))
    }

    fn face<'a>(&'a self, cff: Option<&'a cff::Font>, varied: &'a Varied) -> Face<'a> {
        let [ascent, descent, line_gap] = varied.vertical.unwrap_or_default();
        Face {
            ttf: self.data.as_ptr(),
            start: self.start as c_int,
            outline: if cff.is_some() { Some(outline) } else { None },
            cff: cff.map_or(std::ptr::null(), |cff| cff as *const cff::Font as *const c_void),
            advance: if varied.advances.is_some() { Some(advance) } else { None },
            advances: varied.advances.as_ref().map_or(std::ptr::null(), |a| a as *const metrics::Advances as *const c_void),
            varied: varied.vertical.is_some() as c_int,
            ascent,
            descent,
            line_gap,
            font: PhantomData,
        }
    }
}

/// Face.advance: the advance of `glyph` at the instance, given its hmtx
/// advance.
unsafe extern "C" fn advance(advances: *const c_void, glyph: c_int, hmtx: c_int) -> c_int {
    // A panic must not unwind into C.
    std::panic::catch_unwind(|| (*(advances as *const metrics::Advances)).advance(glyph as u32, hmtx)).unwrap_or(hmtx)
}

/// Face.outline: the outline of `glyph` into out[..capacity] as
/// stbtt_GetGlyphShape gives it, and its box as stbtt_GetGlyphBox does.
/// Returns the vertex count, which when over `capacity` means call again
/// with room for that many; 0 for no outline or one that can't be drawn.
unsafe extern "C" fn outline(cff: *const c_void, glyph: c_int, out: *mut cff::Vertex, capacity: c_int, bbox: *mut c_int) -> c_int {
    // A panic must not unwind into C.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let font = &*(cff as *const cff::Font);
        let bbox = &mut *(bbox as *mut [i32; 4]);
        let mut vertices = Vec::new();
        let Ok(glyph) = usize::try_from(glyph) else { return 0 };
        if font.glyph(glyph, &mut vertices, bbox) != Ok(true) {
            *bbox = [0; 4];
            return 0;
        }
        let Ok(n) = c_int::try_from(vertices.len()) else { return 0 };
        if n <= capacity {
            std::ptr::copy_nonoverlapping(vertices.as_ptr(), out, vertices.len());
        }
        n
    }))
    .unwrap_or(0)
}

/// Pick the face `face` names (an index, or a name as list() shows it) and
/// check it. `path` is for messages.
fn choose(data: Vec<u8>, face: Option<&str>, path: &str) -> Result<Font, String> {
    let unusable = |reason: String| format!("{path}: not a usable font: {reason}");
    let starts = faces(&data).map_err(unusable)?;
    let collection = data.get(0..4) == Some(b"ttcf");
    let all = || list(&data, &starts, None);
    let index = match face {
        None => 0,
        Some("") => return Err(format!("{path}: nothing after #; give a face number or name")),
        Some(face) if face.bytes().all(|b| b.is_ascii_digit()) => match face.parse::<usize>() {
            Ok(index) if index < starts.len() => index,
            _ if !collection => return Err(format!("{path} is a single font, so its only face is #0, not #{face}")),
            _ => return Err(format!("{path} has no face #{face}; it holds:{}", all())),
        },
        Some(face) => {
            let wanted = face.to_lowercase();
            let named: Vec<usize> = (0..starts.len())
                .filter(|&i| starts[i].map_or(false, |start| names(&data, start).iter().any(|name| name.to_lowercase() == wanted)))
                .collect();
            match named[..] {
                [index] => index,
                [] => return Err(format!("{path} has no face named {face:?}; it holds:{}", all())),
                _ => {
                    let these = list(&data, &starts, Some(&named));
                    return Err(format!("{path} has {} faces named {face:?}; pick one by number:{these}", named.len()));
                }
            }
        }
    };
    // Only the chosen face has to be usable, as stb reads only its offset.
    let start = starts[index].ok_or_else(|| unusable(format!("face #{index} is not a font")))?;
    check_at(&data, start).map_err(unusable)?;
    let hint = (collection && face.is_none() && starts.len() > 1).then(|| {
        format!(
            "{path} holds {} faces and the first, {}, was used; pick one with {path}#N or {path}#NAME \
             (#0 picks the first and silences this):{}",
            starts.len(),
            display_name(&data, start),
            all()
        )
    });
    let face = collection.then(|| (index, display_name(&data, start)));
    Ok(Font { data, start, face, hint, coords: Vec::new(), instance: None })
}

/// Draw `font`, a checked face, at the instance the axis settings `axes`
/// pick. Only CFF2 outlines vary here; anything else is refused, as is an
/// axis the face does not have. `path` is for messages.
fn vary(font: &mut Font, axes: &str, path: &str) -> Result<(), String> {
    let settings = variations::parse(axes).map_err(|reason| format!("{path}#{axes}: {reason}"))?;
    let (d, start) = (&font.data, font.start);
    let refuse = |reason: &str| format!("{path}: cannot set {axes}: {reason}");
    if table(d, start, b"glyf")?.is_some() {
        return Err(refuse("termshot varies CFF2 outlines only, and this face has TrueType (glyf) ones"));
    }
    if table(d, start, b"CFF ")?.is_some() {
        return Err(refuse("termshot varies CFF2 outlines only, and this face has CFF ones, which do not vary"));
    }
    let Some(fvar) = table(d, start, b"fvar")? else {
        return Err(refuse("the face has no axes (no fvar table)"));
    };
    let unusable = |table: &str, reason: String| format!("{path}: not a usable font: {table}: {reason}");
    let all = variations::axes(fvar.data).map_err(|reason| unusable("fvar", reason))?;
    let maps = match table(d, start, b"avar")? {
        Some(avar) => variations::segment_maps(avar.data, all.len()).map_err(|reason| unusable("avar", reason))?,
        None => Vec::new(),
    };
    font.coords = variations::coords(&all, &maps, &settings).map_err(|reason| format!("{path}: {reason}"))?;
    // The CFF2 store is checked against fvar, and HVAR and MVAR read, only
    // at an instance, so check them now, at load, rather than when drawing.
    let broken = |reason: String| format!("{path}: not a usable font: {reason}");
    cff_outlines(&font.data, font.start, &font.coords).map_err(broken)?;
    varied(&font.data, font.start, &font.coords).map_err(broken)?;
    font.instance = Some(variations::instance(&all, &settings));
    Ok(())
}

/// Where each face starts: a single font is one face at 0. The same offsets
/// stbtt_GetFontOffsetForIndex returns; None for one that isn't a font.
fn faces(d: &[u8]) -> Result<Vec<Option<usize>>, String> {
    let tag = d.get(0..4).ok_or("file is shorter than a font header")?;
    if is_sfnt(tag) {
        return Ok(vec![Some(0)]);
    }
    if tag != b"ttcf" || !matches!(u32_at(d, 4)?, 0x0001_0000 | 0x0002_0000) {
        return Err("not a TrueType or OpenType file".into());
    }
    let count = u32_at(d, 8)? as i32;
    if count < 1 {
        return Err("font collection is empty".into());
    }
    if count as usize > MAX_FACES {
        return Err(format!("font collection holds {count} faces, more than {MAX_FACES}"));
    }
    if 12 + 4 * count as usize > d.len() {
        return Err("font collection header runs past the end of the file".into());
    }
    Ok((0..count as usize)
        .map(|i| {
            let start = u32_at(d, 12 + 4 * i).ok()? as usize;
            // stbtt_InitFont takes the offset as an int.
            let tag = d.get(start..start.saturating_add(4))?;
            (start <= i32::MAX as usize && is_sfnt(tag)).then_some(start)
        })
        .collect())
}

/// The faces as `#N name` lines, all of them or only those in `only`.
fn list(d: &[u8], starts: &[Option<usize>], only: Option<&[usize]>) -> String {
    let indices: Vec<usize> = only.map_or_else(|| (0..starts.len()).collect(), |only| only.to_vec());
    let mut lines: Vec<String> =
        indices.iter().take(MAX_LISTED).map(|&i| format!("\n  #{i}  {}", starts[i].map_or("(not a font)".into(), |start| display_name(d, start))))
        .collect();
    if indices.len() > MAX_LISTED {
        lines.push(format!("\n  and {} more", indices.len() - MAX_LISTED));
    }
    lines.concat()
}

fn display_name(d: &[u8], start: usize) -> String {
    names(d, start).into_iter().next().unwrap_or_else(|| "(no name)".into())
}

/// The face's full name, typographic family and family (name IDs 4, 16, 1),
/// those it has, deduplicated. A face can be picked by any of them.
fn names(d: &[u8], start: usize) -> Vec<String> {
    let mut names = Vec::new();
    for id in [4, 16, 1] {
        if let Some(name) = name(d, start, id) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// One name from the name table: Windows US English first, then any Windows
/// or Unicode record, then an ASCII Macintosh one. A damaged table is no name;
/// control characters are replaced, as the name is printed to a terminal.
/// Only a record that beats the best so far is decoded, and only its first
/// NAME_CHARS characters, so a hostile table costs a scan of its records.
fn name(d: &[u8], start: usize, id: u16) -> Option<String> {
    let table = table(d, start, b"name").ok()??.data;
    let count = u16_at(table, 2).ok()? as usize;
    let strings = u16_at(table, 4).ok()? as usize;
    let mut best: Option<(u8, String)> = None;
    for i in 0..count {
        let record = table.get(6 + 12 * i..18 + 12 * i)?;
        let field = |at: usize| u16::from_be_bytes([record[at], record[at + 1]]);
        let (platform, encoding, language) = (field(0), field(2), field(4));
        if field(6) != id {
            continue;
        }
        let rank = match (platform, encoding) {
            (3, 1 | 10) if language == 0x0409 => 0,
            (3, 1 | 10) => 1,
            (0, _) => 2,
            (1, 0) => 3,
            _ => continue,
        };
        if best.as_ref().map_or(false, |(best, _)| rank >= *best) {
            continue;
        }
        let offset = strings + field(10) as usize;
        let Some(bytes) = table.get(offset..offset + field(8) as usize) else { continue };
        let text = if rank == 3 {
            // Up to 4 bytes a character in UTF-16; 1 in ASCII.
            let bytes = &bytes[..bytes.len().min(NAME_CHARS)];
            if !bytes.is_ascii() {
                continue;
            }
            bytes.iter().map(|&b| b as char).collect()
        } else {
            let bytes = &bytes[..bytes.len().min(4 * NAME_CHARS)];
            let units: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_be_bytes([b[0], b[1]])).collect();
            String::from_utf16_lossy(&units)
        };
        best = Some((rank, text));
        if rank == 0 {
            break;
        }
    }
    let text: String =
        best?.1.chars().take(NAME_CHARS).map(|c| if c.is_control() { '\u{fffd}' } else { c }).collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
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

/// Check the font, or the first face of a collection.
#[cfg(test)]
pub fn check(d: &[u8]) -> Result<(), String> {
    check_at(d, faces(d)?[0].ok_or("face #0 is not a font")?)
}

/// Check the face whose table directory starts at `start`.
fn check_at(d: &[u8], start: usize) -> Result<(), String> {
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
    let cmap = required(d, start, b"cmap")?;
    let head = required(d, start, b"head")?;
    let hhea = required(d, start, b"hhea")?;
    let hmtx = required(d, start, b"hmtx")?;
    let maxp = required(d, start, b"maxp")?;

    let glyph_count = u16_at(maxp, 4)? as usize;
    if glyph_count == 0 {
        return Err("maxp says the font has no glyphs".into());
    }
    u16_at(head, 52)?;
    let long_metrics = u16_at(hhea, 34)? as usize;
    if long_metrics == 0 {
        return Err("hhea has no horizontal metrics".into());
    }
    let hmtx_needed = 4 * long_metrics + 2 * glyph_count.saturating_sub(long_metrics);
    if hmtx.len() < hmtx_needed {
        return Err(format!("hmtx is {} bytes, needs {hmtx_needed}", hmtx.len()));
    }
    // stb draws a face without glyf from its CFF table.
    if cff_outlines(d, start, &[])?.is_none() {
        check_glyf(d, start, head, glyph_count)?;
    }
    check_cmap(cmap, glyph_count)
}

/// The CFF outlines of a face without glyf, parsed and checked; None for a
/// TrueType face. A CFF2 face is drawn at the instance at `coords`, as
/// cff::Font::parse_cff2 takes them.
pub fn cff_outlines<'a>(d: &'a [u8], start: usize, coords: &[i32]) -> Result<Option<cff::Font<'a>>, String> {
    if table(d, start, b"glyf")?.is_some() {
        return Ok(None);
    }
    let glyphs = u16_at(required(d, start, b"maxp")?, 4)? as usize;
    // stb reads CFF and not CFF2, so CFF wins where a face has both.
    if let Some(cff) = table(d, start, b"CFF ")? {
        return cff::Font::parse(cff.data, glyphs).map(Some).map_err(|reason| format!("CFF table: {reason}"));
    }
    let Some(cff2) = table(d, start, b"CFF2")? else {
        return Err(no_outlines(d, start)?);
    };
    cff::Font::parse_cff2(cff2.data, glyphs, coords).map(Some).map_err(|reason| format!("CFF2 table: {reason}"))
}

/// Check the loca and glyf tables of a TrueType face.
fn check_glyf(d: &[u8], start: usize, head: &[u8], glyph_count: usize) -> Result<(), String> {
    let loca = required(d, start, b"loca")?;
    let glyf = required(d, start, b"glyf")?;
    let long_loca = match u16_at(head, 50)? {
        0 => false,
        1 => true,
        other => return Err(format!("unknown loca format {other}")),
    };
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
    check_composite_depth(&components)
}

/// Why a font with no glyf, CFF or CFF2 table can't be drawn, from the
/// tables it has instead.
fn no_outlines(d: &[u8], start: usize) -> Result<String, String> {
    if let Some(tag) = color_bitmap_at(d, start)? {
        return Ok(format!("a color bitmap font ({tag}) with no outlines; use a monochrome outline font, such as Noto Emoji"));
    }
    Ok("no glyf table, and no CFF or CFF2 table either".into())
}

fn color_bitmap_at(d: &[u8], start: usize) -> Result<Option<&'static str>, String> {
    for tag in ["CBDT", "CBLC", "sbix"] {
        if table(d, start, tag.as_bytes().try_into().unwrap())?.is_some() {
            return Ok(Some(tag));
        }
    }
    Ok(None)
}

/// The color bitmap table of a checked face, if it has one: such a face
/// draws its color glyphs from bitmaps stb_truetype can't read, and maps
/// those characters to empty outlines.
pub fn color_bitmap(font: &Font) -> Option<&'static str> {
    color_bitmap_at(&font.data, font.start).ok()?
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
        // Each flag run contributes fixed X/Y byte counts. Sum the runs
        // directly instead of allocating and rereading a flag per point.
        let mut remaining = points;
        let mut coordinates = 0usize;
        while remaining > 0 {
            let flag = *glyph.get(at).ok_or("flags run past the glyph")?;
            at += 1;
            let mut repeat = 1;
            if flag & 8 != 0 {
                repeat += *glyph.get(at).ok_or("flags run past the glyph")? as usize;
                at += 1;
            }
            let run = repeat.min(remaining);
            let x = if flag & 2 != 0 { 1 } else if flag & 16 != 0 { 0 } else { 2 };
            let y = if flag & 4 != 0 { 1 } else if flag & 32 != 0 { 0 } else { 2 };
            coordinates += run * (x + y);
            remaining -= run;
        }
        if at + coordinates > glyph.len() {
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

#[cfg(test)]
pub mod tests {
    use super::*;

    fn simple(flags: &[u8], coordinates: usize) -> Vec<u8> {
        let mut glyph = vec![0; 14];
        glyph[1] = 1; // one contour
        glyph[11] = 2; // three points; no instructions
        glyph.extend_from_slice(flags);
        glyph.resize(glyph.len() + coordinates, 0);
        glyph
    }

    #[test]
    fn repeated_flags_require_all_coordinate_bytes() {
        // Three repeated points: short X, long Y => nine coordinate bytes.
        assert!(check_glyph(&simple(&[0x0a, 2], 9), 1).is_ok());
        assert!(check_glyph(&simple(&[0x0a, 2], 8), 1).is_err());
        // A truncated repeat count is not an empty coordinate run.
        assert!(check_glyph(&simple(&[0x0a], 0), 1).is_err());
    }

    #[test]
    fn mixed_flags_and_zero_coordinate_runs_are_checked() {
        // Both coordinates unchanged, both short, both long: 0 + 2 + 4 bytes.
        assert!(check_glyph(&simple(&[0x30, 0x06, 0x00], 6), 1).is_ok());
        assert!(check_glyph(&simple(&[0x30, 0x06, 0x00], 5), 1).is_err());
        assert!(check_glyph(&simple(&[0x38, 2], 0), 1).is_ok());
    }

    const FONT: &str = "third_party/jetbrains-mono/JetBrainsMono-Regular.ttf";

    fn be16(v: u16) -> [u8; 2] {
        v.to_be_bytes()
    }

    /// A name table that gives `name` as the full name and the family, in
    /// Windows US English records.
    fn name_table(name: &str) -> Vec<u8> {
        let text: Vec<u8> = name.encode_utf16().flat_map(be16).collect();
        let mut table = [be16(0), be16(2), be16(6 + 2 * 12)].concat();
        for id in [1, 4] {
            table.extend([3, 1, 0x0409, id, text.len() as u16, 0].into_iter().flat_map(be16));
        }
        table.extend(text);
        table
    }

    /// The font with `edit` applied to its copy of table `tag`.
    pub fn edit_table(font: &[u8], tag: &[u8; 4], edit: impl Fn(&mut [u8])) -> Vec<u8> {
        let mut font = font.to_vec();
        let table = table(&font, 0, tag).unwrap().unwrap().data;
        let at = table.as_ptr() as usize - font.as_ptr() as usize;
        let len = table.len();
        edit(&mut font[at..at + len]);
        font
    }

    /// A collection of the fonts, each with a name table naming it as given.
    /// Each face gets its own copy of every table.
    pub fn collection(faces: &[(&[u8], &str)]) -> Vec<u8> {
        let faces: Vec<_> = faces.iter().map(|&(font, name)| (font, name_table(name))).collect();
        collection_of(&faces)
    }

    /// The same, with each face's name table as given.
    fn collection_of(faces: &[(&[u8], Vec<u8>)]) -> Vec<u8> {
        let mut out = [&b"ttcf"[..], &[0, 1, 0, 0], &(faces.len() as u32).to_be_bytes()].concat();
        out.resize(12 + 4 * faces.len(), 0);
        for (i, (font, names)) in faces.iter().enumerate() {
            let base = out.len();
            out[12 + 4 * i..16 + 4 * i].copy_from_slice(&(base as u32).to_be_bytes());
            out.extend_from_slice(font);
            out.resize((out.len() + 3) & !3, 0);
            let names_at = out.len();
            out.extend_from_slice(names);
            for t in 0..u16_at(font, 4).unwrap() as usize {
                let record = base + 12 + 16 * t;
                let (offset, len) = if &out[record..record + 4] == b"name" {
                    (names_at, names.len())
                } else {
                    (base + u32_at(&out, record + 8).unwrap() as usize, u32_at(&out, record + 12).unwrap() as usize)
                };
                out[record + 8..record + 12].copy_from_slice(&(offset as u32).to_be_bytes());
                out[record + 12..record + 16].copy_from_slice(&(len as u32).to_be_bytes());
            }
        }
        out
    }

    /// JetBrains Mono as "Face A", and as "Face B" with a taller ascent, so
    /// the two draw differently.
    pub fn two_faces() -> Vec<u8> {
        let font = fs::read(FONT).unwrap();
        let taller = edit_table(&font, b"hhea", |hhea| hhea[4..6].copy_from_slice(&be16(1300)));
        collection(&[(&font, "Face A"), (&taller, "Face B")])
    }

    #[test]
    fn faces_are_picked_by_number_or_name() {
        let ttc = two_faces();
        let pick = |face| choose(ttc.clone(), face, "c.ttc");
        let first = pick(None).unwrap();
        let second = pick(Some("1")).unwrap();
        assert!(first.start > 0 && second.start > first.start);
        assert_eq!(pick(Some("face b")).unwrap().start, second.start);
        assert_eq!(pick(Some("0")).unwrap().start, first.start);
        assert_eq!(first.face, Some((0, "Face A".into())));
        assert_eq!(second.face, Some((1, "Face B".into())));
        // Only a collection given without a face gets the hint; it lists them.
        let hint = first.hint.unwrap();
        assert!(hint.contains("#0  Face A") && hint.contains("#1  Face B"), "{hint}");
        assert!(pick(Some("0")).unwrap().hint.is_none());
        for (face, says) in [
            ("2", "c.ttc has no face #2; it holds:\n  #0  Face A\n  #1  Face B"),
            ("99999999999999999999999", "has no face #9"),
            ("Face", "has no face named \"Face\"; it holds:\n  #0  Face A"),
            ("", "nothing after #"),
        ] {
            let error = pick(Some(face)).err().unwrap();
            assert!(error.contains(says), "{error}");
        }
        let font = fs::read(FONT).unwrap();
        let twins = collection(&[(&font, "Twin"), (&font, "Twin")]);
        let error = choose(twins, Some("twin"), "t.ttc").err().unwrap();
        assert!(error.contains("2 faces named \"twin\"; pick one by number:\n  #0  Twin\n  #1  Twin"), "{error}");
    }

    /// A face of a collection, checked and padded for draw_png.
    pub fn choose_padded(ttc: Vec<u8>, face: &str) -> Font {
        pad(choose(ttc, Some(face), "t.ttc").ok().unwrap())
    }

    #[test]
    fn a_face_that_is_not_a_font_costs_only_itself() {
        let mut ttc = two_faces();
        let end = (ttc.len() as u32).to_be_bytes();
        ttc[16..20].copy_from_slice(&end);
        let first = choose(ttc.clone(), None, "c.ttc").unwrap();
        assert!(first.hint.unwrap().contains("#1  (not a font)"));
        let error = choose(ttc.clone(), Some("1"), "c.ttc").err().unwrap();
        assert!(error.contains("c.ttc: not a usable font: face #1 is not a font"), "{error}");
        assert!(choose(ttc, Some("face a"), "c.ttc").is_ok());
    }

    #[test]
    fn a_collection_holds_at_most_max_faces() {
        let font = fs::read(FONT).unwrap();
        // Every offset points at the one face, as in a file made to be slow.
        let ttc = |faces: usize| {
            let mut ttc = collection(&[(&font, "Only")]);
            let shift = 4 * (faces - 1);
            for t in 0..u16_at(&ttc, 16 + 4).unwrap() as usize {
                let field = 16 + 12 + 16 * t + 8;
                let offset = u32_at(&ttc, field).unwrap() + shift as u32;
                ttc[field..field + 4].copy_from_slice(&offset.to_be_bytes());
            }
            ttc[8..12].copy_from_slice(&(faces as u32).to_be_bytes());
            ttc[12..16].copy_from_slice(&(16 + shift as u32).to_be_bytes());
            let start = ttc[12..16].to_vec();
            ttc.splice(16..16, start.repeat(faces - 1));
            ttc
        };
        assert!(choose(ttc(MAX_FACES), Some("only"), "c.ttc").err().unwrap().contains("1024 faces named"));
        let error = choose(ttc(MAX_FACES + 1), Some("only"), "c.ttc").err().unwrap();
        assert!(error.contains("holds 1025 faces, more than 1024"), "{error}");
    }

    #[test]
    fn only_the_chosen_face_is_checked() {
        let font = fs::read(FONT).unwrap();
        let broken = edit_table(&font, b"head", |head| head[50] = 7);
        let ttc = collection(&[(&font, "Good"), (&broken, "Broken")]);
        assert!(choose(ttc.clone(), Some("0"), "c.ttc").is_ok());
        let error = choose(ttc, Some("Broken"), "c.ttc").err().unwrap();
        assert!(error.contains("c.ttc: not a usable font: unknown loca format"), "{error}");
    }

    #[test]
    fn a_single_font_is_face_zero() {
        let font = fs::read(FONT).unwrap();
        let pick = |face| choose(font.clone(), face, "f.ttf");
        let single = pick(None).unwrap();
        assert!(single.start == 0 && single.face.is_none() && single.hint.is_none());
        assert!(pick(Some("0")).is_ok() && pick(Some("JETBRAINS MONO")).is_ok());
        let error = pick(Some("1")).err().unwrap();
        assert!(error.contains("f.ttf is a single font, so its only face is #0, not #1"), "{error}");
    }

    #[test]
    fn names_are_printable() {
        let font = fs::read(FONT).unwrap();
        let ttc = collection(&[(&font, "Bad\x1b[2JName\n"), (&font, "")]);
        let hint = choose(ttc, None, "c.ttc").unwrap().hint.unwrap();
        assert!(hint.contains("#0  Bad\u{fffd}[2JName\u{fffd}\n  #1  (no name)"), "{hint}");
    }

    #[test]
    fn a_huge_name_table_costs_one_pass_over_its_records() {
        // The most records, each a 64 KiB name. The string offset is a u16,
        // so the strings overlap the records, as a hostile file may.
        let records = u16::MAX;
        let mut table = [be16(0), be16(records), be16(0)].concat();
        for _ in 0..records {
            table.extend([3, 1, 0x0411, 4, u16::MAX - 1, 0].into_iter().flat_map(be16));
        }
        let font = fs::read(FONT).unwrap();
        let faces: Vec<_> = (0..16).map(|_| (&font[..], table.clone())).collect();
        let ttc = collection_of(&faces);
        let started = Instant::now();
        let error = choose(ttc, Some("none"), "c.ttc").err().unwrap();
        let elapsed = started.elapsed();
        let listed = error.lines().nth(1).unwrap().trim_start().strip_prefix("#0  ").unwrap();
        assert!(listed.chars().count() <= NAME_CHARS, "{listed}");
        // Decoding every record would be 4 GiB a face.
        assert!(elapsed.as_secs() < 5, "took {elapsed:?}");
    }

    #[test]
    fn a_face_name_may_hold_a_hash() {
        let font = fs::read(FONT).unwrap();
        let path = "target/test/hash-names.ttc";
        fs::write(path, collection(&[(&font, "Foo"), (&font, "Foo #1")])).unwrap();
        let spec = Spec::parse(&format!("{path}#Foo #1")).unwrap();
        assert_eq!(spec, Spec { path: path.into(), face: Some("Foo #1".into()), axes: None });
        assert_eq!(load(&spec).unwrap().face, Some((1, "Foo #1".into())));
        // With no such file, the path ends at the first #.
        let spec = Spec::parse("target/test/absent.ttc#Foo #1").unwrap();
        assert_eq!(spec, Spec { path: "target/test/absent.ttc".into(), face: Some("Foo #1".into()), axes: None });
        // Two files that the value could start with: neither is guessed.
        let other = format!("{path}#Foo");
        fs::write(&other, collection(&[(&font, "Bar#1")])).unwrap();
        let error = Spec::parse(&format!("{path}#Foo#Bar#1")).err().unwrap();
        assert!(error.contains(&format!("could name a face of {path} or of {other}")), "{error}");
        fs::remove_file(&other).unwrap();
    }

    #[test]
    fn mutated_collection_headers_and_names_are_rejected_or_used() {
        let ttc = two_faces();
        let names_at = ttc.len() - name_table("Face B").len();
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as usize
        };
        for _ in 0..2000 {
            let mut mutant = ttc.clone();
            for _ in 0..1 + next() % 4 {
                // The collection header, face 0's table directory, or face 1's names.
                let at = if next() % 2 == 0 { next() % 300 } else { names_at + next() % (ttc.len() - names_at) };
                mutant[at] = next() as u8;
            }
            for face in [None, Some("1"), Some("face b")] {
                let _ = choose(mutant.clone(), face, "m.ttc");
            }
        }
    }
}
