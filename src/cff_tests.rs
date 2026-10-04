//! CFF tests: cff.rs against stock stb_truetype on every glyph of a real CID
//! font, hand-made hostile fonts (each with one defect that made stb hang,
//! assert or read out of bounds; see docs/cff-rust-vs-c.md), and a mutation
//! fuzz through draw.c. CFF2, which stb can't read, has hand-made fonts of
//! its own, and a real variable font checked against HarfBuzz's outlines and
//! fuzzed the same way. Paths are relative to the repo root.

use super::*;
use crate::draw_tests::{mutate, render, render_with};
use std::ffi::c_int;

const CJK: &str = "third_party/noto-sans-cjk/NotoSansCJKtc-Subset.otf";
const CJK_VF: &str = "third_party/noto-sans-cjk-vf/NotoSansCJKtc-VF-Subset.otf";
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
    let cff = font::cff_outlines(&font.data, font.start, &[]).unwrap().expect("a CFF font");
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
        ("steps a float rounds away", craft::small_steps_cff()),
    ] {
        assert_eq!(same_as_stb(font), 2, "{name}");
    }
}

#[test]
fn a_box_comes_out_as_drawn() {
    let font = font::prepare(craft::control()).unwrap();
    let cff = font::cff_outlines(&font.data, font.start, &[]).unwrap().unwrap();
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
        ("a CFF table tagged CFF2", craft::cff2(), Some("CFF2 table: header says version 1"), None),
        // CFF2: the same guarantees, and the rules CFF2 adds.
        ("CFF2 subroutine bomb", craft::cff2_subr_bomb(), None, Some("runs more than 20000 charstring operators")),
        ("CFF2 subroutine recursion", craft::cff2_recursion(), None, None),
        ("CFF2 CharStrings count of 2^32 - 1", craft::cff2_huge_index(), Some("runs past the table"), None),
        ("CFF2 table cut in its CharStrings", craft::cff2_truncated(), Some("runs past the table"), None),
        ("CFF2 Top DICT past the table", craft::cff2_top_past_table(), Some("Top DICT runs past the table"), None),
        ("CFF2 blend in the Top DICT", craft::cff2_top_blend(), Some("blend outside a Private DICT"), None),
        ("CFF2 Top DICT of 514 operands", craft::cff2_deep_dict(), Some("more than 513 operands"), None),
        ("CFF2 without FDArray", craft::cff2_no_fdarray(), Some("no FDArray"), None),
        ("CFF2 two font dicts, no FDSelect", craft::cff2_no_fdselect(), Some("2 font dicts and no FDSelect"), None),
        ("CFF2 FDSelect of 2^32 - 1 ranges", craft::cff2_huge_fdselect(), Some("FDSelect runs past the table"), None),
        ("CFF2 FDSelect names font dict 2 of 2", craft::cff2_fdselect_missing_dict(), Some("FDSelect range 1 is invalid"), None),
        ("CFF2 vstore past the table", craft::cff2_vstore_past_table(), Some("vstore: runs past the table"), None),
        ("CFF2 vstore format 2", craft::cff2_vstore_format(), Some("vstore: format 2"), None),
        ("CFF2 vstore names a missing region", craft::cff2_vstore_bad_region(), Some("names region 1 of 1"), None),
        ("CFF2 Private DICT vsindex 1 of 1", craft::cff2_private_vsindex(), Some("Private DICT vsindex 1, with 1 ItemVariationData"), None),
        ("CFF2 Private DICT blend short", craft::cff2_private_blend_short(), Some("has only 4 operands"), None),
        ("CFF2 vsindex 1 of 1", craft::cff2_vsindex(), None, Some("vsindex 1, with 1 ItemVariationData")),
        ("CFF2 blend short of operands", craft::cff2_blend_short(), None, Some("blend of 2 values over 2 regions has only 6 operands")),
        ("CFF2 blend of 30000 values", craft::cff2_blend_huge(), None, Some("blend of 30000 values")),
        ("CFF2 blend without vstore", craft::cff2_blend_no_vstore(), None, Some("names no ItemVariationData")),
        ("CFF2 blend of -1 values", craft::cff2_blend_negative(), None, Some("blend without a count of values")),
        ("CFF2 514 operands", craft::cff2_stack_overflow(), None, Some("more than 513 operands on the stack")),
        ("CFF2 endchar", craft::cff2_endchar(), None, Some("endchar is not a CFF2 operator")),
        ("CFF2 vsindex after blend", craft::cff2_vsindex_after_blend(), None, Some("vsindex after a blend")),
        ("CFF2 two vsindex", craft::cff2_vsindex_twice(), None, Some("vsindex after a blend or another vsindex")),
    ] {
        // For test.sh's CLI checks, which run after these tests.
        if name == "CharStrings past the table" {
            fs::write("target/test/cff-past-table.otf", &font).unwrap();
        }
        if name == "CFF2 CharStrings count of 2^32 - 1" {
            fs::write("target/test/cff2-past-table.otf", &font).unwrap();
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
        let cff = font::cff_outlines(&font.data, font.start, &[]).unwrap().unwrap();
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
    let cjk = font::load(&font::Spec { path: CJK.into(), face: None, axes: None }).unwrap();
    let mono = font::load(&font::Spec { path: draw_tests::FONT.into(), face: None, axes: None }).unwrap();
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

/// POSIX cksum: the CRC and the length, as tools/cff2-outlines.sh records
/// HarfBuzz's outlines.
fn cksum(data: &[u8]) -> (u32, usize) {
    let step = |crc: u32, byte: u8| {
        (0..8).fold(crc ^ ((byte as u32) << 24), |c, _| if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04c1_1db7 } else { c << 1 })
    };
    let mut crc = data.iter().fold(0, |crc, &b| step(crc, b));
    let mut n = data.len();
    while n > 0 {
        crc = step(crc, n as u8);
        n >>= 8;
    }
    (!crc, data.len())
}

/// An outline as hb-vector writes it as an SVG path: a contour starts when
/// it draws, and each is closed.
fn svg_path(outline: &[cff::Vertex]) -> String {
    let mut path = String::new();
    for (i, v) in outline.iter().enumerate() {
        match v.kind {
            cff::MOVE if outline.get(i + 1).map_or(true, |next| next.kind == cff::MOVE) => {}
            cff::MOVE => path += &format!("{}M{},{}", if path.is_empty() { "" } else { "Z" }, v.x, v.y),
            cff::LINE => path += &format!("L{},{}", v.x, v.y),
            _ => path += &format!("C{},{} {},{} {},{}", v.cx, v.cy, v.cx1, v.cy1, v.x, v.y),
        }
    }
    if !path.is_empty() {
        path.push('Z');
    }
    path
}

/// Checks the font at `path` against `reference`, as tools/cff2-outlines.sh
/// writes it: every glyph draws without an error, and each it lists has
/// HarfBuzz's outline, at the instance its "# variations:" line names (the
/// default without one). Returns how many it lists and how many draw.
fn matches_harfbuzz(path: &str, reference: &str) -> (usize, usize) {
    let axes = reference.lines().find_map(|line| line.strip_prefix("# variations: ")).map(String::from);
    let font = font::load(&font::Spec { path: path.into(), face: None, axes }).unwrap();
    let cff = font::cff_outlines(&font.data, font.start, &font.coords).unwrap().expect("a CFF2 font");
    let (mut out, mut bounds, mut drawn) = (Vec::new(), [0; 4], 0);
    for glyph in 0..cff.glyphs {
        drawn += usize::from(cff.glyph(glyph, &mut out, &mut bounds).unwrap_or_else(|e| panic!("glyph {glyph}: {e}")));
    }
    let mut checked = 0;
    for line in reference.lines().filter(|line| !line.starts_with('#')) {
        let fields: Vec<usize> = line.split(' ').map(|field| field.parse().unwrap()).collect();
        let [glyph, crc, length] = fields[..] else { panic!("{line}") };
        assert_eq!(cff.glyph(glyph, &mut out, &mut bounds), Ok(true), "glyph {glyph}");
        let path = svg_path(&out);
        assert_eq!(cksum(path.as_bytes()), (crc as u32, length), "glyph {glyph} differs from HarfBuzz's: {path}");
        checked += 1;
    }
    (checked, drawn)
}

/// stb has no CFF2 reader, so HarfBuzz is the reference: every character of
/// the subset has the outline hb-vector draws, for the default instance in
/// cff2-outlines.txt and for the one each other cff2-outlines-*.txt names.
#[test]
fn every_character_of_a_cff2_font_matches_harfbuzz() {
    assert_eq!(cksum(b"abc"), (1219131554, 3));
    let mut fixtures: Vec<_> = fs::read_dir("tests/fixtures")
        .unwrap()
        .map(|entry| entry.unwrap().path().to_str().unwrap().to_string())
        .filter(|path| path.starts_with("tests/fixtures/cff2-outlines"))
        .collect();
    fixtures.sort();
    assert!(fixtures.len() > 3, "{fixtures:?}");
    for fixture in fixtures {
        let (checked, drawn) = matches_harfbuzz(CJK_VF, &fs::read_to_string(&fixture).unwrap());
        assert!(checked > 100 && drawn > 150, "{fixture}: {checked} checked, {drawn} drawn");
    }
}

/// Any CFF2 font against HarfBuzz, at any instance: record its outlines,
/// then check them, with no change here.
///
///     tools/cff2-outlines.sh --variations=wght=700 FONT OUT
///     TERMSHOT_CFF2_FONT=FONT TERMSHOT_CFF2_OUTLINES=OUT ./target/test/unit --ignored any_cff2_font
///
/// Without the variables it checks nothing.
#[test]
#[ignore]
fn any_cff2_font_matches_harfbuzz() {
    let (Ok(font), Ok(outlines)) = (std::env::var("TERMSHOT_CFF2_FONT"), std::env::var("TERMSHOT_CFF2_OUTLINES")) else {
        return;
    };
    let (checked, drawn) = matches_harfbuzz(&font, &fs::read_to_string(&outlines).unwrap());
    println!("{font}: {drawn} glyphs drawn; {checked} match {outlines}");
}

#[test]
fn hand_made_cff2_fonts_draw_the_square() {
    for (name, font) in [
        ("control", craft::cff2_control()),
        ("deltas dropped", craft::cff2_blended()),
        ("blend in the Private DICT", craft::cff2_private_blend()),
        ("vsindex in the Private DICT", craft::cff2_private_vsindex_1()),
        ("two vsindex in the Private DICT", craft::cff2_private_vsindex_twice()),
        ("8000 offsets to one ItemVariationData", craft::cff2_shared_store_data()),
        ("vsindex in the charstring", craft::cff2_charstring_vsindex()),
        ("local and global subrs", craft::cff2_subrs()),
        ("FDSelect format 0", craft::cff2_fdselect(0)),
        ("FDSelect format 3", craft::cff2_fdselect(3)),
        ("FDSelect format 4", craft::cff2_fdselect(4)),
        ("more operands than CFF allows", craft::cff2_deep_stack()),
        ("hintmask", craft::cff2_hintmask()),
    ] {
        let font = font::prepare(font).unwrap_or_else(|error| panic!("{name}: {error}"));
        let cff = font::cff_outlines(&font.data, font.start, &[]).unwrap().unwrap();
        let (mut out, mut bounds) = (Vec::new(), [0; 4]);
        assert_eq!(cff.glyph(1, &mut out, &mut bounds), Ok(true), "{name}");
        let points: Vec<(u8, i16, i16)> = out.iter().map(|v| (v.kind, v.x, v.y)).collect();
        let (m, l) = (cff::MOVE, cff::LINE);
        assert_eq!(points, [(m, 100, 100), (l, 600, 100), (l, 600, 600), (l, 100, 100)], "{name}");
        assert_eq!(bounds, [100, 100, 600, 600], "{name}");
        assert_eq!(render(&parse(b"AAA", 3, 1), 3, 1, &font, 16.0, "target/test/cff2-square.png"), 0, "{name}");
    }
}

/// The first point of glyph 1 of `font` at `coords`.
fn first_point(font: Vec<u8>, coords: &[i32]) -> (i16, i16) {
    let font = font::prepare(font).unwrap();
    let cff = font::cff_outlines(&font.data, font.start, coords).unwrap().unwrap();
    let (mut out, mut bounds) = (Vec::new(), [0; 4]);
    assert_eq!(cff.glyph(1, &mut out, &mut bounds), Ok(true));
    (out[0].x, out[0].y)
}

/// Each case of a region's scalar, as HarfBuzz's VarRegionAxis::evaluate
/// works it out: glyph 1 moves to 16384 times the scalars' sum.
#[test]
fn region_scalars_are_harfbuzz_s() {
    const ONE: i16 = 0x4000;
    let up = [0, ONE, ONE];
    fs::create_dir_all("target/test").unwrap();
    let mut manifest = Vec::new();
    for (case, axes, regions, coords, x) in [
        ("the default instance", 1, vec![vec![up]], vec![], 0),
        ("at the peak", 1, vec![vec![up]], vec![16384], 16384),
        ("at 0", 1, vec![vec![up]], vec![0], 0),
        ("halfway up", 1, vec![vec![up]], vec![8192], 8192),
        ("a quarter up", 1, vec![vec![up]], vec![4096], 4096),
        ("past the peak, down to the end", 1, vec![vec![[0, 8192, ONE]]], vec![12288], 8192),
        ("a third, as an f32", 1, vec![vec![[0, 12288, ONE]]], vec![4096], 5461),
        ("at the start", 1, vec![vec![[8192, 12288, ONE]]], vec![8192], 0),
        ("at the end", 1, vec![vec![[0, 8192, 12288]]], vec![12288], 0),
        ("below the start", 1, vec![vec![up]], vec![-8192], 0),
        ("peak 0: the axis is ignored", 1, vec![vec![[0, 0, ONE]]], vec![5000], 16384),
        ("peak 0, but every coordinate 0: the default", 1, vec![vec![[0, 0, ONE]]], vec![0], 0),
        ("peak 0, and another coordinate not 0", 2, vec![vec![[0, 0, ONE], [0, 0, ONE]]], vec![0, 4096], 16384),
        ("start past the peak: ignored", 1, vec![vec![[12000, 8000, ONE]]], vec![4000], 16384),
        ("peak past the end: ignored", 1, vec![vec![[0, 12000, 8000]]], vec![4000], 16384),
        ("straddling 0: ignored", 1, vec![vec![[-8192, 8192, ONE]]], vec![4000], 16384),
        ("malformed, but at 0", 1, vec![vec![[-8192, 8192, ONE]]], vec![0], 0),
        ("the negative direction", 1, vec![vec![[-ONE, -ONE, 0]]], vec![-8192], 8192),
        ("two axes multiply", 2, vec![vec![up, up]], vec![8192, 4096], 2048),
        ("one axis at 0", 2, vec![vec![up, up]], vec![8192, 0], 0),
        ("two regions add", 1, vec![vec![up], vec![[0, 8192, ONE]]], vec![12288], 20480),
    ] {
        let font = craft::cff2_scalars(axes, &regions);
        assert_eq!(first_point(font.clone(), &coords), (x, 0), "{case}");
        // Each font has an axis per coordinate, so hb-vector can draw it:
        // hb-vector --font-size=1000 --precision=9 --variations=SETTINGS --glyphs FONT gid1
        // moves to x (rounded here to an i16). All agree with 14.4.0.
        let settings: Vec<String> = coords.iter().enumerate().map(|(i, c)| format!("ax{i}={}", *c as f64 / 16384.0)).collect();
        let file = format!("target/test/cff2-scalars-{}.otf", manifest.len());
        fs::write(&file, font).unwrap();
        manifest.push(format!("{file} {} {x} {case}", settings.join(",")));
    }
    fs::write("target/test/cff2-scalars.txt", manifest.join("\n") + "\n").unwrap();
}

/// The deltas of each blended value, at the instance: the square's corner
/// and first side move by `d` each way, so its other corners by 2 * `d`.
#[test]
fn blend_applies_its_deltas_at_an_instance() {
    let points = |font: &[u8], coords: &[i32]| -> Vec<(u8, i32, i32)> {
        let font = font::prepare(font.to_vec()).unwrap();
        let cff = font::cff_outlines(&font.data, font.start, coords).unwrap().unwrap();
        let (mut out, mut bounds) = (Vec::new(), [0; 4]);
        assert_eq!(cff.glyph(1, &mut out, &mut bounds), Ok(true));
        out.iter().map(|v| (v.kind, v.x as i32, v.y as i32)).collect()
    };
    for (name, font, coords, d) in [
        // 2 regions peaking at +1, each delta 7.
        ("halfway", craft::cff2_blended(), vec![8192], 7),
        ("at the end", craft::cff2_blended(), vec![16384], 14),
        ("past the end", craft::cff2_blended(), vec![20000], 0),
        ("against the regions", craft::cff2_blended(), vec![-16384], 0),
        // 3 regions, picked by the Private DICT or by the charstring.
        ("Private DICT vsindex", craft::cff2_private_vsindex_1(), vec![16384], 21),
        ("charstring vsindex", craft::cff2_charstring_vsindex(), vec![16384], 21),
        ("Private DICT blend", craft::cff2_private_blend(), vec![16384], 14),
    ] {
        let default = points(&font, &[]);
        let expected: Vec<_> = default.iter().enumerate().map(|(i, &(kind, x, y))| {
            let k = if i == 0 || i == default.len() - 1 { d } else { 2 * d };
            (kind, x + k, y + k)
        }).collect();
        assert_eq!(points(&font, &coords), expected, "{name}");
    }
    // Each value's deltas are its own.
    assert_eq!(first_point(craft::cff2_blend_two_values(), &[16384]), (1100, 2200));
    // Four offsets to two ItemVariationData: vsindex 3 is the first's.
    assert_eq!(first_point(craft::cff2_shared_data_order(), &[16384]), (16384, 0));
    assert_eq!(first_point(craft::cff2_shared_data_order(), &[-16384]), (0, 0));
    // 8000 offsets to one ItemVariationData of 8000 regions: evaluated once.
    assert_eq!(first_point(craft::cff2_shared_store_data(), &[8192]), (100, 100));
}

/// HarfBuzz runs CFF2 charstrings in doubles, so cff.rs does; stb, the
/// reference for CFF, runs them in floats, where this glyph would stay at
/// x = 8192. hb-vector draws it as M8192,0 L8192,0 L8191.999511719,0
/// L8191.999023438,0 L8191.999023438,0 L8191.999023438,500
/// L8191.999023438,0 L8192,0 Z: the first step and the last point round to
/// 8192 as floats, so no line closes the contour.
#[test]
fn cff2_charstrings_add_in_doubles_as_harfbuzz_does() {
    let font = craft::cff2_small_steps();
    fs::write("target/test/cff2-small-steps.otf", &font).unwrap();
    let font = font::prepare(font).unwrap();
    let cff = font::cff_outlines(&font.data, font.start, &[]).unwrap().unwrap();
    let (mut out, mut bounds) = (Vec::new(), [0; 4]);
    assert_eq!(cff.glyph(1, &mut out, &mut bounds), Ok(true));
    let points: Vec<(u8, i16, i16)> = out.iter().map(|v| (v.kind, v.x, v.y)).collect();
    let (m, l) = (cff::MOVE, cff::LINE);
    let steps = [(l, 8192, 0), (l, 8191, 0), (l, 8191, 0), (l, 8191, 0)];
    assert_eq!(points, [&[(m, 8192, 0)][..], &steps, &[(l, 8191, 500), (l, 8191, 0), (l, 8192, 0)]].concat());
    assert_eq!(bounds, [8191, 0, 8192, 500]);
}

#[test]
fn cff2_fonts_render_as_primary_and_fallback() {
    let vf = font::load(&font::Spec { path: CJK_VF.into(), face: None, axes: None }).unwrap();
    let mono = font::load(&font::Spec { path: draw_tests::FONT.into(), face: None, axes: None }).unwrap();
    let cells = parse(CJK_TEXT.as_bytes(), 60, 1);
    let draw = |font: &font::Font, fallback: Option<&font::Font>, out: &str| {
        assert_eq!(render_with(&cells, 60, 1, font, fallback, 24.0, out), 0);
        fs::read(out).unwrap()
    };
    let alone = draw(&vf, None, "target/test/cff2-primary.png");
    let fallback = draw(&mono, Some(&vf), "target/test/cff2-fallback.png");
    let tofu = draw(&mono, None, "target/test/cff2-tofu.png");
    assert!(alone != fallback && fallback != tofu);
}

/// The mutation fuzz, on the CFF2 table alone: the rest of the file is
/// font.rs's, which the TrueType fuzz covers.
#[test]
fn mutated_cff2_fonts_are_refused_or_render() {
    let original = fs::read(CJK_VF).unwrap();
    let tables = u16::from_be_bytes([original[4], original[5]]) as usize;
    let record = (0..tables).map(|i| 12 + 16 * i).find(|&r| &original[r..r + 4] == b"CFF2").unwrap();
    let field = |at: usize| u32::from_be_bytes(original[at..at + 4].try_into().unwrap()) as usize;
    let (at, length) = (field(record + 8), field(record + 12));
    let cells = parse(format!("{CJK_TEXT} \x1b[3m{CJK_TEXT}").as_bytes(), 60, 1);
    let (mut rendered, mut out, mut bounds) = (0, Vec::new(), [0; 4]);
    for seed in 0..300 {
        let mut font = original.clone();
        mutate(&mut font[at..at + length], seed);
        if let Ok(font) = font::prepare(font) {
            let cff = font::cff_outlines(&font.data, font.start, &[]).unwrap().unwrap();
            for glyph in 0..cff.glyphs {
                let _ = cff.glyph(glyph, &mut out, &mut bounds);
            }
            assert_eq!(render(&cells, 60, 1, &font, 12.0, "target/test/fuzz-cff2.png"), 0, "seed {seed}");
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
        sfnt_with(cff, glyphs, tag, &[])
    }

    /// The same with the `extra` tables too.
    fn sfnt_with(cff: &[u8], glyphs: u16, tag: &[u8; 4], extra: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
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
        tables.extend(extra);
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

    // CFF2 fonts. Their glyph 1 draws the square, unless it has the defect.

    const VSINDEX: u8 = 0x0f;
    const BLEND: u8 = 0x10;
    const HSTEMHM: u8 = 0x12;
    const HINTMASK: u8 = 0x13;

    /// A CFF2 INDEX: a 32-bit count, then 4-byte offsets.
    fn index2(items: &[Vec<u8>]) -> Vec<u8> {
        let count = (items.len() as u32).to_be_bytes().to_vec();
        if items.is_empty() {
            return count;
        }
        [count, index(items)[2..].to_vec()].concat()
    }

    /// The square without endchar, which CFF2 doesn't have.
    fn square2() -> Vec<u8> {
        let square = square();
        square[..square.len() - 1].to_vec()
    }

    /// `values` blended over `regions` regions, each delta 7; the default
    /// instance leaves `values` on the stack.
    fn blended(values: &[i32], regions: usize) -> Vec<u8> {
        let defaults: Vec<u8> = values.iter().flat_map(|&v| num(v)).collect();
        let deltas = num(7).repeat(values.len() * regions);
        [defaults, deltas, num(values.len() as i32), vec![BLEND]].concat()
    }

    /// The square, its coordinates blended.
    fn blended_square(regions: usize) -> Vec<u8> {
        let (corner, side) = (blended(&[100, 100], regions), blended(&[500, 0], regions));
        charstring(&[&corner, &[RMOVE], &side, &[RLINE], &num(0), &num(500), &[RLINE]])
    }

    /// The square's sides, in a subroutine that ends without return.
    fn sides() -> Vec<u8> {
        charstring(&[&num(500), &num(0), &[RLINE], &num(0), &num(500), &[RLINE]])
    }

    /// The square drawn by a local subr.
    fn square_by_subr() -> Vec<u8> {
        charstring(&[&num(100), &num(100), &[RMOVE], &num(-BIAS), &[CALLSUBR]])
    }

    /// A region's (start, peak, end) on each axis, in F2Dot14 units.
    type Region = Vec<[i16; 3]>;

    /// A variation store of `regions`, each over `axes` axes, and an
    /// ItemVariationData per entry of `data`, naming those regions. Its
    /// offsets name them in the order `order` gives.
    fn store_full(axes: u16, regions: &[Region], data: &[Vec<u16>], order: &[usize]) -> Vec<u8> {
        let be16 = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
        let list_at = 8 + 4 * order.len();
        let mut list = be16(&[axes, regions.len() as u16]);
        for triple in regions.iter().flatten() {
            list.extend(be16(&triple.map(|v| v as u16)));
        }
        let (mut at, mut tables) = (Vec::new(), Vec::new());
        for indexes in data {
            at.push(list_at + list.len() + tables.len());
            tables.extend([be16(&[0, 0, indexes.len() as u16]), be16(indexes)].concat());
        }
        let offsets: Vec<u8> = order.iter().flat_map(|&i| (at[i] as u32).to_be_bytes()).collect();
        let store = [be16(&[1]), (list_at as u32).to_be_bytes().to_vec(), be16(&[order.len() as u16]), offsets, list, tables].concat();
        [be16(&[store.len() as u16]), store].concat()
    }

    /// A variation store with one axis and `regions` regions, each peaking
    /// at its maximum, and an ItemVariationData per entry of `data`, naming
    /// those regions.
    fn store_with(regions: u16, data: &[Vec<u16>]) -> Vec<u8> {
        let order: Vec<usize> = (0..data.len()).collect();
        store_full(1, &vec![vec![[0, 0x4000, 0x4000]]; regions as usize], data, &order)
    }

    /// The same, each ItemVariationData naming that many regions.
    fn store(regions: u16, data: &[u16]) -> Vec<u8> {
        store_with(regions, &data.iter().map(|&n| (0..n).map(|i| i % regions).collect()).collect::<Vec<_>>())
    }

    /// A CFF2 font dict: its Private DICT, and the local Subrs it is given.
    #[derive(Clone, Default)]
    struct FontDict2 {
        private: Vec<u8>,
        subrs: Vec<Vec<u8>>,
    }

    #[derive(Default)]
    struct Cff2 {
        charstrings: Vec<Vec<u8>>,
        gsubrs: Vec<Vec<u8>>,
        dicts: Vec<FontDict2>,
        no_fdarray: bool,
        fdselect: Vec<u8>,
        vstore: Vec<u8>,
        top_extra: Vec<u8>,
    }

    impl Cff2 {
        fn new(charstrings: Vec<Vec<u8>>) -> Cff2 {
            Cff2 { charstrings, dicts: vec![FontDict2::default()], ..Cff2::default() }
        }

        /// Lay out the table: header, Top DICT, Global Subrs, then the rest.
        fn build(&self) -> Vec<u8> {
            let top = |charstrings: usize, fdarray: usize, fdselect: usize, vstore: usize| {
                let mut top = [dint(charstrings as i32), vec![17]].concat();
                if !self.no_fdarray {
                    top.extend([dint(fdarray as i32), vec![12, 36]].concat());
                }
                if !self.fdselect.is_empty() {
                    top.extend([dint(fdselect as i32), vec![12, 37]].concat());
                }
                if !self.vstore.is_empty() {
                    top.extend([dint(vstore as i32), vec![24]].concat());
                }
                [top, self.top_extra.clone()].concat()
            };
            let head = |top: Vec<u8>| [vec![2, 0, 5], (top.len() as u16).to_be_bytes().to_vec(), top, index2(&self.gsubrs)].concat();
            // Two passes, as for CFF.
            let start = head(top(0, 0, 0, 0)).len();
            let mut body = index2(&self.charstrings);
            let mut dicts = Vec::new();
            for dict in &self.dicts {
                // Each Private DICT, with its Subrs just past it.
                let mut private = dict.private.clone();
                if !dict.subrs.is_empty() {
                    private.extend([dint(private.len() as i32 + 6), vec![19]].concat());
                }
                dicts.push([dint(private.len() as i32), dint((start + body.len()) as i32), vec![18]].concat());
                body.extend(private);
                if !dict.subrs.is_empty() {
                    body.extend(index2(&dict.subrs));
                }
            }
            let at_fdarray = start + body.len();
            body.extend(index2(&dicts));
            let at_fdselect = start + body.len();
            body.extend(&self.fdselect);
            let at_vstore = start + body.len();
            body.extend(&self.vstore);
            [head(top(start, at_fdarray, at_fdselect, at_vstore)), body].concat()
        }
    }

    fn sfnt2(cff2: &Cff2) -> Vec<u8> {
        sfnt_tagged(&cff2.build(), 2, b"CFF2")
    }

    /// The control's table, and where its CharStrings INDEX starts.
    fn control_table() -> (Vec<u8>, usize) {
        let t = Cff2::new(vec![square2(), square2()]).build();
        let charstrings = index2(&[square2(), square2()]);
        let at = t.windows(charstrings.len()).position(|w| w == charstrings).unwrap();
        (t, at)
    }

    pub fn cff2_control() -> Vec<u8> {
        sfnt2(&Cff2::new(vec![square2(), square2()]))
    }

    /// Its coordinates blended over two regions, whose deltas are dropped.
    pub fn cff2_blended() -> Vec<u8> {
        sfnt2(&Cff2 { vstore: store(2, &[2]), ..Cff2::new(vec![square2(), blended_square(2)]) })
    }

    /// BlueValues blended in the Private DICT.
    pub fn cff2_private_blend() -> Vec<u8> {
        let private = [dint(-250), dint(0), dint(7), dint(7), dint(7), dint(7), dint(2), vec![23, 6]].concat();
        let dicts = vec![FontDict2 { private, subrs: Vec::new() }];
        sfnt2(&Cff2 { dicts, vstore: store(2, &[2]), ..Cff2::new(vec![square2(), blended_square(2)]) })
    }

    /// The Private DICT's vsindex picks the second ItemVariationData, of 3
    /// regions, so each blend takes 3 deltas per value.
    pub fn cff2_private_vsindex_1() -> Vec<u8> {
        let dicts = vec![FontDict2 { private: [dint(1), vec![22]].concat(), subrs: Vec::new() }];
        sfnt2(&Cff2 { dicts, vstore: store(3, &[1, 3]), ..Cff2::new(vec![square2(), blended_square(3)]) })
    }

    /// vsindex 0, then 1: the last is the one the charstrings start with.
    pub fn cff2_private_vsindex_twice() -> Vec<u8> {
        let dicts = vec![FontDict2 { private: [dint(0), vec![22], dint(1), vec![22]].concat(), subrs: Vec::new() }];
        sfnt2(&Cff2 { dicts, vstore: store(3, &[1, 3]), ..Cff2::new(vec![square2(), blended_square(3)]) })
    }

    /// 8000 ItemVariationData offsets, all to one of 8000 region indexes:
    /// checked once, not 8000 times (64 million region checks).
    pub fn cff2_shared_store_data() -> Vec<u8> {
        let be16 = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
        let n = 8000usize;
        let list_at = 8 + 4 * n;
        let list = be16(&[1, 1, 0, 0x4000, 0x4000]);
        let data = (list_at + list.len()) as u32;
        let ivd = [be16(&[0, 0, n as u16]), vec![0; 2 * n]].concat();
        let offsets = data.to_be_bytes().repeat(n);
        let body = [be16(&[1]), (list_at as u32).to_be_bytes().to_vec(), be16(&[n as u16]), offsets, list, ivd].concat();
        let vstore = [be16(&[body.len() as u16]), body].concat();
        sfnt2(&Cff2 { vstore, ..Cff2::new(vec![square2(), square2()]) })
    }

    /// The same chosen by the charstring.
    pub fn cff2_charstring_vsindex() -> Vec<u8> {
        let glyph = [num(1), vec![VSINDEX], blended_square(3)].concat();
        sfnt2(&Cff2 { vstore: store(3, &[1, 3]), ..Cff2::new(vec![square2(), glyph]) })
    }

    /// Two sides in a local subr and two in a global one, neither with return.
    pub fn cff2_subrs() -> Vec<u8> {
        let local = charstring(&[&num(500), &num(0), &[RLINE]]);
        let global = charstring(&[&num(0), &num(500), &[RLINE]]);
        let glyph = [square_by_subr(), num(-BIAS), vec![CALLGSUBR]].concat();
        let dicts = vec![FontDict2 { private: Vec::new(), subrs: vec![local] }];
        sfnt2(&Cff2 { dicts, gsubrs: vec![global], ..Cff2::new(vec![square2(), glyph]) })
    }

    /// Glyph 1 in font dict 1, whose local subr draws the sides.
    pub fn cff2_fdselect(format: u8) -> Vec<u8> {
        let fdselect = match format {
            0 => vec![0, 0, 1],
            3 => vec![3, 0, 2, 0, 0, 0, 0, 1, 1, 0, 2],
            _ => vec![4, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 2],
        };
        let dicts = vec![FontDict2::default(), FontDict2 { private: Vec::new(), subrs: vec![sides()] }];
        sfnt2(&Cff2 { dicts, fdselect, ..Cff2::new(vec![square2(), square_by_subr()]) })
    }

    /// 60 operands under rmoveto, which CFF's limit of 48 would refuse.
    pub fn cff2_deep_stack() -> Vec<u8> {
        let glyph = [num(0).repeat(58), square2()].concat();
        sfnt2(&Cff2::new(vec![square2(), glyph]))
    }

    /// Two hstems and a hintmask of one byte before the square.
    pub fn cff2_hintmask() -> Vec<u8> {
        let glyph = [charstring(&[&num(0), &num(10), &num(20), &num(10), &[HSTEMHM], &[HINTMASK, 0xc0]]), square2()].concat();
        sfnt2(&Cff2::new(vec![square2(), glyph]))
    }

    /// The subroutine bomb, its subrs ending without return.
    pub fn cff2_subr_bomb() -> Vec<u8> {
        let gsubrs = (0..10)
            .map(|k| {
                if k < 9 {
                    charstring(&[&num(k + 1 - BIAS), &[CALLGSUBR]]).repeat(8)
                } else {
                    charstring(&[&num(1), &num(1), &[RLINE]])
                }
            })
            .collect();
        let glyph = charstring(&[&num(0), &num(0), &[RMOVE], &num(-BIAS), &[CALLGSUBR]]);
        sfnt2(&Cff2 { gsubrs, ..Cff2::new(vec![square2(), glyph]) })
    }

    /// A local subr that calls itself: past the depth limit, nothing is drawn.
    pub fn cff2_recursion() -> Vec<u8> {
        let dicts = vec![FontDict2 { private: Vec::new(), subrs: vec![charstring(&[&num(-BIAS), &[CALLSUBR]])] }];
        sfnt2(&Cff2 { dicts, ..Cff2::new(vec![square2(), square_by_subr()]) })
    }

    /// CharStrings claims 2^32 - 1 glyphs.
    pub fn cff2_huge_index() -> Vec<u8> {
        let (mut t, at) = control_table();
        t[at..at + 4].copy_from_slice(&[0xff; 4]);
        sfnt_tagged(&t, 2, b"CFF2")
    }

    /// The table ends inside its CharStrings INDEX.
    pub fn cff2_truncated() -> Vec<u8> {
        let (t, at) = control_table();
        sfnt_tagged(&t[..at + 10], 2, b"CFF2")
    }

    /// The header gives the Top DICT 65535 bytes.
    pub fn cff2_top_past_table() -> Vec<u8> {
        let (mut t, _) = control_table();
        t[3..5].copy_from_slice(&[0xff, 0xff]);
        sfnt_tagged(&t, 2, b"CFF2")
    }

    pub fn cff2_top_blend() -> Vec<u8> {
        let top_extra = [dint(1), dint(1), dint(1), vec![23]].concat();
        sfnt2(&Cff2 { top_extra, vstore: store(1, &[1]), ..Cff2::new(vec![square2(), square2()]) })
    }

    /// 514 operands for FontMatrix.
    pub fn cff2_deep_dict() -> Vec<u8> {
        sfnt2(&Cff2 { top_extra: [vec![139; 514], vec![12, 7]].concat(), ..Cff2::new(vec![square2(), square2()]) })
    }

    pub fn cff2_no_fdarray() -> Vec<u8> {
        sfnt2(&Cff2 { no_fdarray: true, ..Cff2::new(vec![square2(), square2()]) })
    }

    pub fn cff2_no_fdselect() -> Vec<u8> {
        sfnt2(&Cff2 { dicts: vec![FontDict2::default(); 2], ..Cff2::new(vec![square2(), square2()]) })
    }

    /// A format 4 FDSelect claims 2^32 - 1 ranges, and has one.
    pub fn cff2_huge_fdselect() -> Vec<u8> {
        let fdselect = vec![4, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0, 0];
        sfnt2(&Cff2 { fdselect, ..Cff2::new(vec![square2(), square2()]) })
    }

    pub fn cff2_fdselect_missing_dict() -> Vec<u8> {
        let fdselect = vec![3, 0, 2, 0, 0, 0, 0, 1, 2, 0, 2];
        sfnt2(&Cff2 { dicts: vec![FontDict2::default(); 2], fdselect, ..Cff2::new(vec![square2(), square2()]) })
    }

    /// The store says it is 65535 bytes.
    pub fn cff2_vstore_past_table() -> Vec<u8> {
        let mut vstore = store(1, &[1]);
        vstore[..2].copy_from_slice(&[0xff, 0xff]);
        sfnt2(&Cff2 { vstore, ..Cff2::new(vec![square2(), square2()]) })
    }

    pub fn cff2_vstore_format() -> Vec<u8> {
        let mut vstore = store(1, &[1]);
        vstore[3] = 2;
        sfnt2(&Cff2 { vstore, ..Cff2::new(vec![square2(), square2()]) })
    }

    pub fn cff2_vstore_bad_region() -> Vec<u8> {
        sfnt2(&Cff2 { vstore: store_with(1, &[vec![1]]), ..Cff2::new(vec![square2(), square2()]) })
    }

    pub fn cff2_private_vsindex() -> Vec<u8> {
        let dicts = vec![FontDict2 { private: [dint(1), vec![22]].concat(), subrs: Vec::new() }];
        sfnt2(&Cff2 { dicts, vstore: store(1, &[1]), ..Cff2::new(vec![square2(), square2()]) })
    }

    /// BlueValues blends 2 values over 2 regions from 3 operands and a count.
    pub fn cff2_private_blend_short() -> Vec<u8> {
        let private = [dint(0), dint(0), dint(7), dint(2), vec![23, 6]].concat();
        let dicts = vec![FontDict2 { private, subrs: Vec::new() }];
        sfnt2(&Cff2 { dicts, vstore: store(2, &[2]), ..Cff2::new(vec![square2(), square2()]) })
    }

    /// Glyph 1 opens with this, in a font of one ItemVariationData of
    /// `regions` regions.
    fn cff2_glyph_after(prefix: &[u8], regions: u16) -> Vec<u8> {
        let glyph = [prefix, &square2()].concat();
        sfnt2(&Cff2 { vstore: store(regions, &[regions]), ..Cff2::new(vec![square2(), glyph]) })
    }

    pub fn cff2_vsindex() -> Vec<u8> {
        cff2_glyph_after(&[num(1), vec![VSINDEX]].concat(), 1)
    }

    pub fn cff2_blend_short() -> Vec<u8> {
        cff2_glyph_after(&[num(100), num(100), num(7), num(7), num(7), num(2), vec![BLEND]].concat(), 2)
    }

    pub fn cff2_blend_huge() -> Vec<u8> {
        cff2_glyph_after(&[num(100), num(100), num(30000), vec![BLEND]].concat(), 1)
    }

    pub fn cff2_blend_no_vstore() -> Vec<u8> {
        sfnt2(&Cff2::new(vec![square2(), blended_square(1)]))
    }

    pub fn cff2_blend_negative() -> Vec<u8> {
        cff2_glyph_after(&[num(100), num(100), num(-1), vec![BLEND]].concat(), 1)
    }

    pub fn cff2_stack_overflow() -> Vec<u8> {
        cff2_glyph_after(&num(0).repeat(514), 1)
    }

    pub fn cff2_endchar() -> Vec<u8> {
        sfnt2(&Cff2::new(vec![square2(), square()]))
    }

    /// Glyph 1 moves to x = 16384 times the sum of the scalars of
    /// `regions`, then draws the square's sides. Its store has one
    /// ItemVariationData, naming every region.
    /// It has an fvar of `axes` axes, `ax0 `, `ax1 ` and so on, each from
    /// -1 to 1 by default 0, so that a setting is its normalized coordinate
    /// and hb-vector can draw it at any.
    pub fn cff2_scalars(axes: u16, regions: &[Region]) -> Vec<u8> {
        let tags: Vec<[u8; 4]> = (0..axes).map(|i| [b'a', b'x', b'0' + i as u8, b' ']).collect();
        cff2_scalars_with(&fvar(&tags.iter().map(|tag| (tag, -1.0, 0.0, 1.0)).collect::<Vec<_>>()), axes, regions)
    }

    /// cff2_scalars, with the given fvar, whose axes need not be the store's.
    pub fn cff2_scalars_with(fvar: &[u8], axes: u16, regions: &[Region]) -> Vec<u8> {
        let deltas = num(16384).repeat(regions.len());
        let glyph = [num(0), deltas, num(1), vec![BLEND], num(0), vec![RMOVE], sides()].concat();
        let all = (0..regions.len() as u16).collect();
        let vstore = store_full(axes, regions, &[all], &[0]);
        sfnt_with(&Cff2 { vstore, ..Cff2::new(vec![square2(), glyph]) }.build(), 2, b"CFF2", &[(b"fvar", fvar)])
    }

    /// An fvar of `axes`, each a tag and its minimum, default and maximum.
    pub fn fvar(axes: &[(&[u8; 4], f64, f64, f64)]) -> Vec<u8> {
        let be16 = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
        let fixed = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
        let count = axes.len() as u16;
        let mut out = be16(&[1, 0, 16, 2, count, 20, 0, 4 * count + 4]);
        for &(tag, min, default, max) in axes {
            out.extend([&tag[..], &fixed(min), &fixed(default), &fixed(max), &be16(&[0, 256])].concat());
        }
        out
    }

    /// An avar of a segment map per axis, each (from, to) in F2Dot14 units.
    pub fn avar(maps: &[Vec<(i16, i16)>]) -> Vec<u8> {
        let be16 = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
        let mut out = be16(&[1, 0, 0, maps.len() as u16]);
        for map in maps {
            out.extend(be16(&[map.len() as u16]));
            out.extend(map.iter().flat_map(|&(from, to)| [from as u16, to as u16]).flat_map(u16::to_be_bytes));
        }
        out
    }

    /// A font whose glyph k + 1, for character 'A' + k, moves to x = the
    /// normalized coordinate of axis k, in F2Dot14 units, for one in -1..1:
    /// a region peaking at +1 on that axis and one at -1, with deltas of
    /// 16384 and -16384. So hb-vector draws what HarfBuzz normalizes axis
    /// settings to.
    pub fn cff2_coordinates(fvar: &[u8], avar: Option<&[u8]>, axes: u16) -> Vec<u8> {
        let one = 0x4000;
        let mut regions = Vec::new();
        for k in 0..axes as usize {
            for peak in [[0, one, one], [-one, -one, 0]] {
                let mut region = vec![[0, 0, 0]; axes as usize];
                region[k] = peak;
                regions.push(region);
            }
        }
        let data: Vec<Vec<u16>> = (0..axes).map(|k| vec![2 * k, 2 * k + 1]).collect();
        let order: Vec<usize> = (0..axes as usize).collect();
        let vstore = store_full(axes, &regions, &data, &order);
        let mut glyphs = vec![square2()];
        for k in 0..axes as i32 {
            let blend = [num(0), num(16384), num(-16384), num(1), vec![BLEND]].concat();
            glyphs.push([num(k), vec![VSINDEX], blend, num(0), vec![RMOVE], sides()].concat());
        }
        let mut extra: Vec<(&[u8; 4], &[u8])> = vec![(b"fvar", fvar)];
        extra.extend(avar.map(|avar| (b"avar", avar)));
        sfnt_with(&Cff2 { vstore, ..Cff2::new(glyphs) }.build(), axes + 1, b"CFF2", &extra)
    }

    /// Four ItemVariationData offsets naming two: the first, of a region
    /// peaking at +1, then the second, of one peaking at -1, twice, then
    /// the first again. Glyph 1 picks the last with vsindex 3 and moves by
    /// 16384 times its scalar, so at +1 it moves and at -1 it does not.
    pub fn cff2_shared_data_order() -> Vec<u8> {
        let regions = [vec![[0, 0x4000, 0x4000]], vec![[-0x4000, -0x4000, 0]]];
        let vstore = store_full(1, &regions, &[vec![0], vec![1]], &[0, 1, 1, 0]);
        let glyph = [num(3), vec![VSINDEX], num(0), num(16384), num(1), vec![BLEND], num(0), vec![RMOVE], sides()].concat();
        sfnt2(&Cff2 { vstore, ..Cff2::new(vec![square2(), glyph]) })
    }

    /// Two values blended over two regions, each with deltas of its own:
    /// at +1, glyph 1 moves to (1100, 2200).
    pub fn cff2_blend_two_values() -> Vec<u8> {
        let deltas = [num(100), num(1000), num(200), num(2000)].concat();
        let glyph = [num(0), num(0), deltas, num(2), vec![BLEND], vec![RMOVE], sides()].concat();
        sfnt2(&Cff2 { vstore: store(2, &[2]), ..Cff2::new(vec![square2(), glyph]) })
    }

    /// vsindex after a blend.
    pub fn cff2_vsindex_after_blend() -> Vec<u8> {
        cff2_glyph_after(&[blended(&[0], 1), vec![VSINDEX]].concat(), 1)
    }

    /// Two vsindex operators.
    pub fn cff2_vsindex_twice() -> Vec<u8> {
        cff2_glyph_after(&[num(0), vec![VSINDEX], num(0), vec![VSINDEX]].concat(), 1)
    }

    /// A charstring 16.16 fixed-point number.
    fn fixed(v: i32) -> Vec<u8> {
        [vec![255], v.to_be_bytes().to_vec()].concat()
    }

    /// From x = 8192, four steps of -1/4096, up and back down, then 3/4096
    /// right, without endchar. In f32 each step rounds back to 8192; in f64
    /// the line ends at 8191.999, and the contour 1/4096 short of its start,
    /// which is the start once drawn as a float.
    fn small_steps() -> Vec<u8> {
        let step = [fixed(-16), num(0)].concat().repeat(4);
        let back = [num(0), num(-500), fixed(48), num(0), vec![RLINE]].concat();
        [num(8192), num(0), vec![RMOVE], step, vec![RLINE], num(0), num(500), vec![RLINE], back].concat()
    }

    pub fn cff2_small_steps() -> Vec<u8> {
        sfnt2(&Cff2::new(vec![square2(), small_steps()]))
    }

    pub fn small_steps_cff() -> Vec<u8> {
        sfnt(&Cff::new(vec![square(), [small_steps(), vec![ENDCHAR]].concat()]).build(), 2)
    }
}
