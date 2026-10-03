//! CFF tests: cff.rs against stock stb_truetype on every glyph of a real CID
//! font, hand-made hostile fonts (each with one defect that made stb hang,
//! assert or read out of bounds; see docs/cff-rust-vs-c.md), and a mutation
//! fuzz through draw.c. Paths are relative to the repo root.

use super::*;
use crate::draw_tests::{mutate, render, render_with};
use std::ffi::c_int;

const CJK: &str = "third_party/noto-sans-cjk/NotoSansCJKtc-Subset.otf";
const CJK_TEXT: &str = "骨直角永東京台灣測試字型漢字中文繁體簡體龍鬱鑿齉 Ag";

/// stbtt_fontinfo, with room to spare (it is 168 bytes on 64-bit).
#[repr(C, align(8))]
struct FontInfo([u8; 512]);

extern "C" {
    fn stbtt_InitFont(info: *mut FontInfo, data: *const u8, offset: c_int) -> c_int;
    fn stbtt_GetGlyphShape(info: *const FontInfo, glyph: c_int, vertices: *mut *mut cff::Vertex) -> c_int;
    fn stbtt_GetGlyphBox(info: *const FontInfo, glyph: c_int, x0: *mut c_int, y0: *mut c_int, x1: *mut c_int, y1: *mut c_int) -> c_int;
    fn stbtt_FreeShape(info: *const FontInfo, vertices: *mut cff::Vertex);
}

/// Check that cff.rs gives every glyph of a well-formed font the vertices
/// and box stb does, and return how many glyphs have an outline.
fn same_as_stb(data: Vec<u8>) -> usize {
    let font = font::prepare(data).unwrap();
    let cff = font::cff_outlines(&font.data, font.start).unwrap().expect("a CFF font");
    let mut info = FontInfo([0; 512]);
    assert_ne!(unsafe { stbtt_InitFont(&mut info, font.data.as_ptr(), font.start as c_int) }, 0);
    let (mut ours, mut bounds, mut drawn) = (Vec::new(), [0; 4], 0);
    for glyph in 0..cff.glyphs {
        let outline = cff.glyph(glyph, &mut ours, &mut bounds).unwrap();
        let g = glyph as c_int;
        let mut theirs = Vec::new();
        let mut box_ = [0; 4];
        unsafe {
            let mut v = std::ptr::null_mut();
            let n = stbtt_GetGlyphShape(&info, g, &mut v);
            if n > 0 {
                theirs = std::slice::from_raw_parts(v, n as usize).to_vec();
            }
            stbtt_FreeShape(&info, v);
            let [x0, y0, x1, y1] = &mut box_;
            stbtt_GetGlyphBox(&info, g, x0, y0, x1, y1);
        }
        // stb leaves the padding byte unset.
        theirs.iter_mut().for_each(|v| v.padding = 0);
        assert!(ours == theirs, "glyph {glyph}: the vertices differ from stb's");
        assert_eq!(bounds, box_, "glyph {glyph}: the box differs from stb's");
        assert_eq!(outline, !theirs.is_empty(), "glyph {glyph}");
        drawn += usize::from(outline);
    }
    drawn
}

#[test]
fn every_glyph_of_a_cid_font_matches_stb() {
    let drawn = same_as_stb(fs::read(CJK).unwrap());
    assert!(drawn > 150, "only {drawn} glyphs have an outline");
}

#[test]
fn hand_made_fonts_match_stb() {
    for (name, font) in [
        ("control", craft::control()),
        ("local subrs", craft::local_subrs()),
        ("FDSelect format 0", craft::fdselect_format_0()),
    ] {
        assert_eq!(same_as_stb(font), 2, "{name}");
    }
}

#[test]
fn a_box_comes_out_as_drawn() {
    let font = font::prepare(craft::control()).unwrap();
    let cff = font::cff_outlines(&font.data, font.start).unwrap().unwrap();
    let (mut out, mut bounds) = (Vec::new(), [0; 4]);
    assert_eq!(cff.glyph(1, &mut out, &mut bounds), Ok(true));
    let points: Vec<(u8, i16, i16)> = out.iter().map(|v| (v.kind, v.x, v.y)).collect();
    let (m, l) = (cff::MOVE, cff::LINE);
    assert_eq!(points, [(m, 100, 100), (l, 600, 100), (l, 600, 600), (l, 100, 100)]);
    assert_eq!(bounds, [100, 100, 600, 600]);
}

/// Each defect of craft.py, and what termshot makes of it: the font refused
/// with this reason, or its glyph 1 refused with this one, or (None) drawn.
#[test]
fn hostile_fonts_are_refused_or_drawn() {
    let cells = parse(b"AAA", 3, 1);
    for (name, font, refused, glyph_refused) in [
        ("subroutine bomb", craft::subr_bomb(), None, Some("runs more than 20000 charstring operators")),
        ("CharStrings past the table", craft::index_past_table(), Some("runs past the table"), None),
        ("hintmask past its charstring", craft::hintmask_overflow(), None, None),
        ("Private DICT Subrs is a real", craft::private_real(), Some("is a real number"), None),
        ("Top DICT operand byte 31", craft::dict_byte_31(), Some("DICT operand byte 31"), None),
        ("fewer CharStrings than glyphs", craft::fewer_charstrings(), Some("2 charstrings for 6 glyphs"), None),
        ("FDSelect gap", craft::fdselect_gap(), Some("FDSelect range 0 is invalid"), None),
        ("7-byte INDEX offsets", craft::bad_offsize(), Some("offset size 7"), None),
        ("16-bit overflow", craft::huge_glyph(), None, Some("past 16-bit coordinates")),
        ("CFF2", craft::cff2(), Some("CFF2 (variable) outlines are not supported"), None),
    ] {
        if name == "CharStrings past the table" {
            // For test.sh's CLI checks, which run after these tests.
            fs::write("target/test/cff-past-table.otf", &font).unwrap();
        }
        let font = match (font::prepare(font), refused) {
            (Err(error), Some(reason)) => {
                assert!(error.contains(reason), "{name}: {error}");
                continue;
            }
            (Err(error), None) => panic!("{name}: refused: {error}"),
            (Ok(_), Some(_)) => panic!("{name}: accepted"),
            (Ok(font), None) => font,
        };
        let cff = font::cff_outlines(&font.data, font.start).unwrap().unwrap();
        let glyph = cff.glyph(1, &mut Vec::new(), &mut [0; 4]);
        match (glyph, glyph_refused) {
            (Err(error), Some(reason)) => assert!(error.contains(reason), "{name}: {error}"),
            (glyph, reason) => assert!(glyph.is_ok() && reason.is_none(), "{name}: {glyph:?}"),
        }
        assert_eq!(render(&cells, 3, 1, &font, 16.0, "target/test/cff-hostile.png"), 0, "{name}");
    }
}

#[test]
fn cff_fonts_render_as_primary_and_fallback() {
    let cjk = font::load(&font::Spec { path: CJK.into(), face: None }).unwrap();
    let mono = font::load(&font::Spec { path: draw_tests::FONT.into(), face: None }).unwrap();
    let text = format!("{CJK_TEXT} \x1b[3m{CJK_TEXT}\x1b[0m");
    let cells = parse(text.as_bytes(), 60, 2);
    let draw = |font: &font::Font, fallback: Option<&font::Font>, out: &str| {
        assert_eq!(render_with(&cells, 60, 2, font, fallback, 24.0, out), 0);
        fs::read(out).unwrap()
    };
    let alone = draw(&cjk, None, "target/test/cff-primary.png");
    let fallback = draw(&mono, Some(&cjk), "target/test/cff-fallback.png");
    let tofu = draw(&mono, None, "target/test/cff-tofu.png");
    assert!(alone != fallback && fallback != tofu);
}

#[test]
fn mutated_cff_fonts_are_refused_or_render() {
    let original = fs::read(CJK).unwrap();
    let cells = parse(format!("{CJK_TEXT} \x1b[3m{CJK_TEXT}").as_bytes(), 60, 1);
    let mut rendered = 0;
    for seed in 0..300 {
        let mut font = original.clone();
        mutate(&mut font, seed);
        if let Ok(font) = font::prepare(font) {
            assert_eq!(render(&cells, 60, 1, &font, 12.0, "target/test/fuzz-cff.png"), 0, "seed {seed}");
            rendered += 1;
        }
    }
    assert!(rendered > 100, "only {rendered} of 300 mutated fonts rendered");
}

/// craft.py's fonts: a minimal OpenType file stb_truetype accepts (cmap,
/// head, hhea, hmtx, maxp, CFF) whose cmap maps 'A' to glyph 1.
pub(crate) mod craft {
    const RMOVE: u8 = 0x15;
    const RLINE: u8 = 0x05;
    const ENDCHAR: u8 = 0x0e;
    const CALLSUBR: u8 = 0x0a;
    const CALLGSUBR: u8 = 0x1d;
    const RETURN: u8 = 0x0b;
    const BIAS: i32 = 107;

    /// A CFF INDEX with 4-byte offsets.
    fn index(items: &[Vec<u8>]) -> Vec<u8> {
        if items.is_empty() {
            return vec![0, 0];
        }
        let mut out = (items.len() as u16).to_be_bytes().to_vec();
        out.push(4);
        let mut offset = 1u32;
        out.extend(offset.to_be_bytes());
        for item in items {
            offset += item.len() as u32;
            out.extend(offset.to_be_bytes());
        }
        items.iter().for_each(|item| out.extend(item));
        out
    }

    /// A charstring integer.
    fn num(v: i32) -> Vec<u8> {
        if (-107..=107).contains(&v) {
            vec![(v + 139) as u8]
        } else {
            [&[28u8][..], &(v as i16).to_be_bytes()].concat()
        }
    }

    /// A DICT integer, always 5 bytes so offsets can be patched in place.
    fn dint(v: i32) -> Vec<u8> {
        [&[29u8][..], &v.to_be_bytes()].concat()
    }

    fn charstring(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    /// 'A': a square from (100, 100) to (600, 600).
    fn square() -> Vec<u8> {
        charstring(&[&num(100), &num(100), &[RMOVE], &num(500), &num(0), &[RLINE], &num(0), &num(500), &[RLINE], &[ENDCHAR]])
    }

    /// A font dict of a CID font: its Private DICT, or the one the Top
    /// DICT names.
    enum FontDict {
        Raw(Vec<u8>),
        Private,
    }

    #[derive(Default)]
    struct Cff {
        charstrings: Vec<Vec<u8>>,
        gsubrs: Vec<Vec<u8>>,
        private: Vec<u8>,
        top_extra: Vec<u8>,
        cid: Option<(Vec<FontDict>, Vec<u8>)>,
    }

    impl Cff {
        fn new(charstrings: Vec<Vec<u8>>) -> Cff {
            Cff { charstrings, ..Cff::default() }
        }

        /// Lay out the table.
        fn build(&self) -> Vec<u8> {
            let header = [1u8, 0, 4, 4];
            let names = index(&[b"X".to_vec()]);
            let strings = index(&[]);
            let gsubrs = index(&self.gsubrs);
            let private_ref = |at: usize| [dint(self.private.len() as i32), dint(at as i32), vec![18]].concat();
            let head = |charstrings: usize, private: usize, fdarray: usize, fdselect: usize| {
                let mut top = [dint(charstrings as i32), vec![17], self.top_extra.clone()].concat();
                if !self.private.is_empty() && self.cid.is_none() {
                    top.extend(private_ref(private));
                }
                if self.cid.is_some() {
                    top.extend([dint(fdarray as i32), vec![12, 36], dint(fdselect as i32), vec![12, 37]].concat());
                }
                [&header[..], &names, &index(&[top]), &strings, &gsubrs].concat()
            };
            // Two passes: the Top DICT's size does not depend on the offsets in it.
            let start = head(0, 0, 0, 0).len();
            let mut body = index(&self.charstrings);
            let at_private = start + body.len();
            body.extend(&self.private);
            let (mut at_fdarray, mut at_fdselect) = (0, 0);
            if let Some((dicts, fdselect)) = &self.cid {
                at_fdarray = start + body.len();
                let dicts: Vec<Vec<u8>> = dicts
                    .iter()
                    .map(|dict| match dict {
                        FontDict::Raw(dict) => dict.clone(),
                        FontDict::Private => private_ref(at_private),
                    })
                    .collect();
                body.extend(index(&dicts));
                at_fdselect = start + body.len();
                body.extend(fdselect);
            }
            [head(start, at_private, at_fdarray, at_fdselect), body].concat()
        }
    }

    /// The font file around a CFF table (or, with `tag`, another table).
    fn sfnt_tagged(cff: &[u8], glyphs: u16, tag: &[u8; 4]) -> Vec<u8> {
        let be = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
        let mut cmap = be(&[0, 1, 3, 1, 0, 12]);
        cmap.extend(be(&[4, 32, 0, 4, 4, 1, 0, 0x41, 0xffff, 0, 0x41, 0xffff, (1 - 0x41i16) as u16, 1, 0, 0]));
        let mut head = be(&[1, 0, 0, 0, 0, 0, 0x5F0F, 0x3CF5, 0, 1000]);
        head.extend([0; 16]);
        head.extend(be(&[0, 0, 1000, 1000, 0, 8, 2, 0, 0]));
        let hhea = be(&[1, 0, 800, (-200i16) as u16, 0, 1000, 0, 0, 1000, 1, 0, 0, 0, 0, 0, 0, 0, glyphs]);
        let hmtx = be(&[600, 0].repeat(glyphs as usize));
        let maxp = be(&[0x0000, 0x5000, glyphs]);
        let mut tables: Vec<(&[u8; 4], &[u8])> =
            vec![(tag, cff), (b"cmap", &cmap), (b"head", &head), (b"hhea", &hhea), (b"hmtx", &hmtx), (b"maxp", &maxp)];
        tables.sort();
        let mut out = [&b"OTTO"[..], &be(&[tables.len() as u16, 0, 0, 0])].concat();
        // The CFF table goes last and unpadded, so the file ends where it does.
        let mut placed = Vec::new();
        let mut data = Vec::new();
        let mut order: Vec<usize> = (0..tables.len()).collect();
        order.sort_by_key(|&i| tables[i].0 == tag);
        let mut at = vec![0; tables.len()];
        for i in order {
            at[i] = 12 + 16 * tables.len() + data.len();
            data.extend(tables[i].1);
            if tables[i].0 != tag {
                data.resize(data.len() + (4 - data.len() % 4) % 4, 0);
            }
        }
        for (i, (tag, table)) in tables.iter().enumerate() {
            placed.extend(tag.iter());
            placed.extend([0; 4]);
            placed.extend((at[i] as u32).to_be_bytes());
            placed.extend((table.len() as u32).to_be_bytes());
        }
        out.extend(placed);
        out.extend(data);
        out
    }

    fn sfnt(cff: &[u8], glyphs: u16) -> Vec<u8> {
        sfnt_tagged(cff, glyphs, b"CFF ")
    }

    /// A well-formed font that every reader draws.
    pub fn control() -> Vec<u8> {
        sfnt(&Cff::new(vec![square(), square()]).build(), 2)
    }

    /// The square drawn through a local subr, in a font that is not CID-keyed.
    pub fn local_subrs() -> Vec<u8> {
        let subr = charstring(&[&num(500), &num(0), &[RLINE], &num(0), &num(500), &[RLINE], &[RETURN]]);
        let glyph = charstring(&[&num(100), &num(100), &[RMOVE], &num(-BIAS), &[CALLSUBR], &[ENDCHAR]]);
        // Subrs (19) start right after the 6-byte Private DICT.
        let private = [dint(6), vec![19], index(&[subr])].concat();
        sfnt(&Cff { private, ..Cff::new(vec![square(), glyph]) }.build(), 2)
    }

    /// The same in a CID font with a format 0 FDSelect.
    pub fn fdselect_format_0() -> Vec<u8> {
        let subr = charstring(&[&num(500), &num(0), &[RLINE], &num(0), &num(500), &[RLINE], &[RETURN]]);
        let glyph = charstring(&[&num(100), &num(100), &[RMOVE], &num(-BIAS), &[CALLSUBR], &[ENDCHAR]]);
        let private = [dint(6), vec![19], index(&[subr])].concat();
        let cid = Some((vec![FontDict::Raw(Vec::new()), FontDict::Private], vec![0, 0, 1]));
        sfnt(&Cff { private, cid, ..Cff::new(vec![square(), glyph]) }.build(), 2)
    }

    /// Ten levels of global subrs, each calling the next eight times: 8^10
    /// calls. stb's depth limit is 10, but nothing limits fan-out.
    pub fn subr_bomb() -> Vec<u8> {
        let gsubrs = (0..10)
            .map(|k| {
                if k < 9 {
                    [charstring(&[&num(k + 1 - BIAS), &[CALLGSUBR]]).repeat(8), vec![RETURN]].concat()
                } else {
                    charstring(&[&num(1), &num(1), &[RLINE], &[RETURN]])
                }
            })
            .collect();
        let glyph = charstring(&[&num(0), &num(0), &[RMOVE], &num(-BIAS), &[CALLGSUBR], &[ENDCHAR]]);
        sfnt(&Cff { gsubrs, ..Cff::new(vec![square(), glyph]) }.build(), 2)
    }

    /// The CharStrings INDEX's last offset points 1 MB past the table, and
    /// the last charstring has no endchar, so a reader runs on past the file.
    pub fn index_past_table() -> Vec<u8> {
        let last = square()[..square().len() - 1].to_vec();
        let end = (1 + square().len() + last.len()) as u32;
        let mut t = Cff::new(vec![square(), last]).build();
        let at = t.windows(4).rposition(|w| w == end.to_be_bytes()).unwrap();
        t[at..at + 4].copy_from_slice(&(1u32 << 20).to_be_bytes());
        sfnt(&t, 2)
    }

    /// Forty hstems, then a hintmask whose mask bytes run off the charstring.
    pub fn hintmask_overflow() -> Vec<u8> {
        let stems = [(0..20).flat_map(|_| [num(10), num(10)].concat()).collect(), vec![1]].concat();
        let glyph = [charstring(&[&num(0), &num(0), &[RMOVE], &num(1), &num(1), &[RLINE]]), stems.repeat(2), vec![0x13]].concat();
        sfnt(&Cff::new(vec![square(), glyph]).build(), 2)
    }

    /// The Private DICT's Subrs offset is a real number.
    pub fn private_real() -> Vec<u8> {
        sfnt(&Cff { private: vec![0x1e, 0x1f, 19], ..Cff::new(vec![square(), square()]) }.build(), 2)
    }

    /// A Top DICT operand byte stb_truetype asserts on.
    pub fn dict_byte_31() -> Vec<u8> {
        sfnt(&Cff { top_extra: vec![0x1f, 0], ..Cff::new(vec![square(), square()]) }.build(), 2)
    }

    /// maxp says 6 glyphs; CharStrings has 2.
    pub fn fewer_charstrings() -> Vec<u8> {
        sfnt(&Cff::new(vec![square(), square()]).build(), 6)
    }

    /// A CID font whose FDSelect starts at glyph 1: glyph 0 has no font dict.
    pub fn fdselect_gap() -> Vec<u8> {
        let fd = [dint(0), dint(0), vec![18]].concat();
        let fdselect = [vec![3], 1u16.to_be_bytes().to_vec(), vec![0, 1, 0], 2u16.to_be_bytes().to_vec()].concat();
        let glyph = charstring(&[&num(0), &num(0), &[RMOVE], &num(0), &[CALLSUBR], &[ENDCHAR]]);
        sfnt(&Cff { cid: Some((vec![FontDict::Raw(fd)], fdselect)), ..Cff::new(vec![glyph, square()]) }.build(), 2)
    }

    /// The Global Subrs INDEX claims 7-byte offsets.
    pub fn bad_offsize() -> Vec<u8> {
        let gsubrs = vec![vec![RETURN]];
        let mut t = Cff { gsubrs: gsubrs.clone(), ..Cff::new(vec![square(), square()]) }.build();
        let wanted = index(&gsubrs);
        let at = t.windows(wanted.len()).position(|w| w == wanted).unwrap();
        t[at + 2] = 7;
        sfnt(&t, 2)
    }

    /// A glyph whose box reaches past 16 bits, which stb would size its
    /// bitmap from while it truncates the vertices.
    pub fn huge_glyph() -> Vec<u8> {
        let far = charstring(&[&num(30000), &num(30000), &[RLINE]]);
        let glyph = [charstring(&[&num(0), &num(0), &[RMOVE]]), far.repeat(3), vec![ENDCHAR]].concat();
        sfnt(&Cff::new(vec![square(), glyph]).build(), 2)
    }

    /// The control's table, tagged CFF2.
    pub fn cff2() -> Vec<u8> {
        sfnt_tagged(&Cff::new(vec![square(), square()]).build(), 2, b"CFF2")
    }
}
