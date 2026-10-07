//! CFF tests: cff.rs against stock stb_truetype on every glyph of a real CID
//! font, hand-made hostile fonts (each with one defect that made stb hang,
//! assert or read out of bounds; see docs/cff-rust-vs-c.md), and a mutation
//! fuzz through the render. CFF2, which stb can't read, has hand-made fonts of
//! its own, and a real variable font checked against HarfBuzz's outlines and
//! fuzzed the same way. Paths are relative to the repo root.

use super::*;
use crate::draw_tests::{mutate, render, render_with};
use crate::vt::parse;
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

/// Each defect of the POC's fonts (bench/cff-poc/craft.rs writes them), and
/// what termshot makes of it: the font refused with this reason, or its
/// glyph 1 refused with this one, or (None) drawn.
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

/// The hand-made fonts (src/cff_craft.rs).
#[path = "cff_craft.rs"]
pub(crate) mod craft;
