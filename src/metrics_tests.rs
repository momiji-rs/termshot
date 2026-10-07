//! Tests for src/metrics.rs: HVAR's advances and MVAR's vertical metrics,
//! as stb_glue.c gets them, against HarfBuzz's.

use crate::cff_tests::craft;
use crate::draw_tests::{self, render_with};
use crate::font;
use crate::font::tests::edit_table;
use std::ffi::c_int;
use std::fs;

const CJK_VF: &str = "third_party/noto-sans-cjk-vf/NotoSansCJKtc-VF-Subset.otf";

extern "C" {
    fn draw_face_metrics(face: *const font::Face, count: c_int, advances: *mut c_int, v: *mut c_int) -> c_int;
    fn draw_face_cell_size(face: *const font::Face, px: f64, w: *mut c_int, h: *mut c_int) -> c_int;
}

/// The advances of glyphs 0..count of `font` and its ascent, descent and
/// line gap, as draw_png_images uses them.
fn drawn(font: &font::Font, count: usize) -> (Vec<i32>, [i32; 3]) {
    let (mut advances, mut v) = (vec![0; count], [0; 3]);
    let ok = font.with_face(|face| unsafe { draw_face_metrics(face, count as c_int, advances.as_mut_ptr(), v.as_mut_ptr()) });
    assert_eq!(ok, Ok(1));
    (advances, v)
}

/// Check the font at `path` against `reference`, which
/// tools/cff2-metrics.sh wrote, at the instance it names. Returns the
/// glyphs checked and how many of them the instance moved.
fn matches_harfbuzz(path: &str, reference: &str) -> (usize, usize) {
    let axes = reference.lines().find_map(|line| line.strip_prefix("# variations: ")).map(String::from);
    let extents = reference.lines().find_map(|line| line.strip_prefix("# extents: ")).expect("an extents line");
    let extents: Vec<i32> = extents.split(' ').map(|v| v.parse().unwrap()).collect();
    let expected: Vec<(usize, i32)> = reference
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| match line.split_once(' ') {
            Some((glyph, advance)) => (glyph.parse().unwrap(), advance.parse().unwrap()),
            None => panic!("{line}"),
        })
        .collect();
    let load = |axes| font::load(&font::Spec { path: path.into(), face: None, axes }).unwrap();
    let (advances, v) = drawn(&load(axes), expected.len());
    let (default, _) = drawn(&load(None), expected.len());
    assert_eq!(v[..], extents[..], "ascent, descent and line gap");
    for &(glyph, advance) in &expected {
        assert_eq!(advances[glyph], advance, "the advance of glyph {glyph}");
    }
    (expected.len(), advances.iter().zip(&default).filter(|(a, b)| a != b).count())
}

/// Every glyph of the subset has HarfBuzz's advance, at the default
/// instance in cff2-metrics.txt and at the one each other
/// cff2-metrics-*.txt names.
#[test]
fn every_advance_of_a_cff2_font_matches_harfbuzz() {
    let mut fixtures: Vec<_> = fs::read_dir("tests/fixtures")
        .unwrap()
        .map(|entry| entry.unwrap().path().to_str().unwrap().to_string())
        .filter(|path| path.starts_with("tests/fixtures/cff2-metrics"))
        .collect();
    fixtures.sort();
    assert!(fixtures.len() > 3, "{fixtures:?}");
    for fixture in fixtures {
        let (checked, moved) = matches_harfbuzz(CJK_VF, &fs::read_to_string(&fixture).unwrap());
        let default = fixture == "tests/fixtures/cff2-metrics.txt";
        assert!(checked > 150 && (default || moved > 50), "{fixture}: {checked} checked, {moved} moved");
    }
}

/// Any variable CFF2 font against HarfBuzz, at any instance: record its
/// metrics, then check them, with no change here.
///
///     tools/cff2-metrics.sh --variations=wght=700 FONT OUT
///     TERMSHOT_CFF2_FONT=FONT TERMSHOT_CFF2_METRICS=OUT ./target/test/unit --ignored any_cff2_font_s_metrics
///
/// Without the variables it checks nothing.
#[test]
#[ignore]
fn any_cff2_font_s_metrics_match_harfbuzz() {
    let (Ok(font), Ok(metrics)) = (std::env::var("TERMSHOT_CFF2_FONT"), std::env::var("TERMSHOT_CFF2_METRICS")) else {
        return;
    };
    let (checked, moved) = matches_harfbuzz(&font, &fs::read_to_string(&metrics).unwrap());
    println!("{font}: {checked} advances match {metrics}; the instance moved {moved}");
}

const ONE: i16 = 0x4000;

fn be16(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|v| v.to_be_bytes()).collect()
}

/// An ItemVariationData: the regions it names, its word count (with 0x8000
/// for 32-bit words), and a row of deltas per item.
struct Data {
    regions: Vec<u16>,
    words: u16,
    rows: Vec<Vec<i32>>,
}

/// An ItemVariationStore over `axes` axes, of `regions` and `data`, where
/// None is a null offset.
fn store(axes: u16, regions: &[Vec<[i16; 3]>], data: &[Option<Data>]) -> Vec<u8> {
    let list_at = 8 + 4 * data.len();
    let mut list = be16(&[axes, regions.len() as u16]);
    for triple in regions.iter().flatten() {
        list.extend(be16(&triple.map(|v| v as u16)));
    }
    let (mut offsets, mut sets) = (Vec::new(), Vec::new());
    for data in data {
        let Some(data) = data else {
            offsets.extend(0u32.to_be_bytes());
            continue;
        };
        offsets.extend(((list_at + list.len() + sets.len()) as u32).to_be_bytes());
        sets.extend(be16(&[data.rows.len() as u16, data.words, data.regions.len() as u16]));
        sets.extend(be16(&data.regions));
        let (long, words) = (data.words & 0x8000 != 0, (data.words & 0x7fff) as usize);
        for row in &data.rows {
            for (i, &v) in row.iter().enumerate() {
                match (long, i < words) {
                    (true, true) => sets.extend(v.to_be_bytes()),
                    (false, false) => sets.push(v as i8 as u8),
                    _ => sets.extend((v as i16).to_be_bytes()),
                }
            }
        }
    }
    [be16(&[1]), (list_at as u32).to_be_bytes().to_vec(), be16(&[data.len() as u16]), offsets, list, sets].concat()
}

/// A DeltaSetIndexMap of `format`, `width` bytes an entry with `inner`
/// bits of inner index, of (outer, inner) entries.
fn index_map(format: u8, width: u8, inner: u8, entries: &[(u32, u32)]) -> Vec<u8> {
    let mut out = vec![format, (width - 1) << 4 | (inner - 1)];
    match format {
        0 => out.extend(be16(&[entries.len() as u16])),
        _ => out.extend((entries.len() as u32).to_be_bytes()),
    }
    for &(outer, i) in entries {
        out.extend(&(outer << inner | i).to_be_bytes()[4 - width as usize..]);
    }
    out
}

/// An HVAR of `store` and, unless it is empty, the advance map `map`.
fn hvar(store: &[u8], map: &[u8]) -> Vec<u8> {
    let map_at = if map.is_empty() { 0 } else { 20 + store.len() as u32 };
    [be16(&[1, 0]), 20u32.to_be_bytes().to_vec(), map_at.to_be_bytes().to_vec(), vec![0; 8], store.to_vec(), map.to_vec()]
        .concat()
}

/// An MVAR of `records` (tag, delta index), each `size` bytes, and `store`.
fn mvar(size: u16, records: &[(&[u8; 4], u32)], store: &[u8]) -> Vec<u8> {
    let count = records.len() as u16;
    let mut out = be16(&[1, 0, 0, size, count, 12 + size * count]);
    for (tag, index) in records {
        out.extend([&tag[..], &index.to_be_bytes(), &vec![0; size as usize - 8]].concat());
    }
    [out, store.to_vec()].concat()
}

/// Regions over two axes, ax0 and ax1: ax0 up, ax0 down, both up, and a
/// tent on ax1 peaking at 0.5.
fn regions() -> Vec<Vec<[i16; 3]>> {
    let (up, down, none) = ([0, ONE, ONE], [-ONE, -ONE, 0], [0, 0, 0]);
    vec![vec![up, none], vec![down, none], vec![up, up], vec![none, [0, ONE / 2, ONE]]]
}

fn data(regions: &[u16], words: u16, rows: &[&[i32]]) -> Option<Data> {
    Some(Data { regions: regions.to_vec(), words, rows: rows.iter().map(|row| row.to_vec()).collect() })
}

/// A font of 14 glyphs over two axes whose HVAR has deltas of each width
/// (8, 16 and 32 bits) and an advance map of format 1 that names a null
/// ItemVariationData, an item and an ItemVariationData past the store's,
/// and is two entries short; and whose MVAR moves hasc, hdsc and hlgp, the
/// descender far enough to cross 0.
fn crafted() -> Vec<u8> {
    let (hvar, mvar) = (crafted_hvar(), crafted_mvar());
    two_axes(&[(b"HVAR", &hvar), (b"MVAR", &mvar)])
}

/// A font of 14 squares over ax0 and ax1, with the `extra` tables.
fn two_axes(extra: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let fvar = craft::fvar(&[(b"ax0 ", -1.0, 0.0, 1.0), (b"ax1 ", -1.0, 0.0, 1.0)]);
    craft::cff2_squares_with(14, &[&[(b"fvar", &fvar[..])], extra].concat())
}

fn crafted_hvar() -> Vec<u8> {
    let advances = store(
        2,
        &regions(),
        &[
            data(&[0, 1, 2], 0, &[&[10, -20, 30], &[127, -128, 1], &[-100, 100, -100]]),
            data(&[0, 3], 1, &[&[1000, 5], &[-30000, -7], &[1, 1]]),
            data(&[2, 3, 0], 0x8001, &[&[70000, -300, 2], &[-1_000_000, 32767, -32768]]),
            None,
        ],
    );
    let entries = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (2, 0), (2, 1), (3, 0), (7, 0), (0, 9), (2, 0)];
    hvar(&advances, &index_map(1, 3, 4, &entries))
}

fn crafted_mvar() -> Vec<u8> {
    let vertical = store(2, &regions(), &[data(&[0, 3], 1, &[&[100, 20], &[300, -50], &[55, 7], &[999, 9]])]);
    mvar(10, &[(b"hasc", 0), (b"hdsc", 1), (b"hlgp", 2), (b"xhgt", 3)], &vertical)
}

/// The font with `cp` mapped to `glyph`, in place of A to glyph 1.
fn one_maps_to(font: &[u8], cp: u16, glyph: u16) -> Vec<u8> {
    edit_table(font, b"cmap", |cmap| {
        // The format 4 subtable's first segment: its end, start and delta.
        for (at, v) in [(26, cp), (32, cp), (36, glyph.wrapping_sub(cp))] {
            cmap[at..at + 2].copy_from_slice(&u16::to_be_bytes(v));
        }
    })
}

/// A fallback glyph is centered in its cells, or shrunk to fit them, by
/// its advance at the instance: at ax0=1 the crafted HVAR advances glyph 1
/// 727 units (centered) and glyph 3 1600 (shrunk), so each draws as a
/// font whose hmtx says so draws at the default.
#[test]
fn a_fallback_is_placed_by_its_advance_at_the_instance() {
    let mono = font::load(&font::Spec { path: draw_tests::FONT.into(), face: None, axes: None }).unwrap();
    let cells = crate::vt::parse("\u{4e00}".as_bytes(), 4, 1);
    let hvar = crafted_hvar();
    for (glyph, advance) in [(1usize, 727u16), (3, 1600)] {
        let varied = one_maps_to(&two_axes(&[(b"HVAR", &hvar)]), 0x4e00, glyph as u16);
        let fixed = edit_table(&varied, b"hmtx", |hmtx| hmtx[4 * glyph..][..2].copy_from_slice(&advance.to_be_bytes()));
        let draw = |data: &[u8], axes: Option<&str>, name: &str| {
            let path = format!("target/test/metrics-fallback-{glyph}-{name}.otf");
            fs::write(&path, data).unwrap();
            let fallback = font::load(&font::Spec { path: path.clone(), face: None, axes: axes.map(String::from) }).unwrap();
            let out = path.replace(".otf", ".png");
            assert_eq!(render_with(&cells, 4, 1, &mono, Some(&fallback), 24.0, &out), 0);
            fs::read(out).unwrap()
        };
        let at_instance = draw(&varied, Some("ax0=1"), "instance");
        assert!(at_instance == draw(&fixed, None, "hmtx"), "glyph {glyph} at ax0=1 is not drawn as advancing {advance}");
        assert!(at_instance != draw(&varied, None, "default"), "glyph {glyph} draws the same at ax0=1");
    }
}

/// A damaged HVAR or MVAR is refused with a reason at an instance, at load,
/// and ignored at the default instance, chosen or not, where neither is read.
#[test]
fn damaged_metrics_tables_are_refused_at_an_instance() {
    let one = || store(2, &regions(), &[data(&[0], 0, &[&[1]])]);
    let patched = |mut table: Vec<u8>, at: usize, v: u8| {
        table[at] = v;
        table
    };
    // A store of one ItemVariationData has its region list at 12.
    let hvars = [
        (patched(hvar(&one(), &[]), 1, 2), "version 2"),
        (hvar(&store(1, &[vec![[0, ONE, ONE]]], &[data(&[0], 0, &[&[1]])]), &[]), "the region list's axis count is 1 and fvar's 2"),
        (hvar(&patched(one(), 14, 0x80), &[]), "the region count 32772 sets the reserved top bit"),
        (hvar(&one()[..30], &[]), "the region list runs past the store"),
        (hvar(&store(2, &regions(), &[data(&[0], 2, &[&[1]])]), &[]), "ItemVariationData 0 has 2 word deltas of 1"),
        (hvar(&store(2, &regions(), &[data(&[9], 0, &[&[1]])]), &[]), "ItemVariationData 0 names region 9 of 4"),
        (hvar(&one(), &index_map(2, 1, 1, &[(0, 0)])), "a DeltaSetIndexMap of format 2"),
        (hvar(&one(), &index_map(0, 2, 1, &[(0, 0)])[..5]), "a DeltaSetIndexMap runs past the table"),
    ];
    let records = |tags: &[&'static [u8; 4]]| -> Vec<(&'static [u8; 4], u32)> { tags.iter().map(|&tag| (tag, 0)).collect() };
    let mvars = [
        (patched(mvar(8, &records(&[b"hasc"]), &one()), 1, 2), "version 2"),
        (patched(mvar(8, &records(&[b"hasc"]), &one()), 7, 6), "value records of 6 bytes, under 8"),
        (mvar(8, &records(&[b"hdsc", b"hasc"]), &one()), "its value records are not in tag order"),
        (mvar(8, &records(&[b"hasc", b"hasc"]), &one()), "its value records are not in tag order"),
        (patched(mvar(8, &records(&[b"hasc"]), &one()), 8, 9), "its value records run past the table"),
        (mvar(8, &records(&[b"hasc"]), &store(1, &[vec![[0, ONE, ONE]]], &[])), "the region list's axis count is 1 and fvar's 2"),
        (
            mvar(8, &[(b"hasc", 0), (b"hdsc", 1)], &store(2, &regions(), &[data(&[0], 1, &[&[-800], &[200]])])),
            "at this instance the ascender 0 is not above the descender 0",
        ),
    ];
    let cases = hvars.into_iter().map(|(t, e)| (*b"HVAR", t, e)).chain(mvars.into_iter().map(|(t, e)| (*b"MVAR", t, e)));
    fs::create_dir_all("target/test").unwrap();
    for (i, (tag, table, error)) in cases.enumerate() {
        let path = format!("target/test/metrics-damaged-{i}.otf");
        fs::write(&path, two_axes(&[(&tag, &table)])).unwrap();
        let load = |axes: Option<&str>| font::load(&font::Spec { path: path.clone(), face: None, axes: axes.map(String::from) });
        let tag = std::str::from_utf8(&tag).unwrap();
        let got = load(Some("ax0=1")).err().unwrap_or_else(|| panic!("{path} ({tag}: {error}) loaded"));
        assert_eq!(got, format!("{path}: not a usable font: {tag} table: {error}"));
        assert!(load(None).is_ok(), "{path} at its default instance");
        assert!(load(Some("ax0=0")).is_ok(), "{path} with the default chosen");
    }
}

/// HVAR's and MVAR's 32-bit deltas can take an advance or an extent past
/// the 16 bits hmtx and hhea hold, and the cell they size past what an
/// int holds. A wide advance sizes the cell as any does: the crafted
/// font's glyph 13, mapped to M, advances 70002 units more at ax0=1,ax1=1.
/// A cell past the largest image is saturated, not overflowed, and the
/// render refuses it (exit 2): at ax0=1, MVAR takes the height to 1 unit,
/// so the scale to 24, and the line gap or every advance far past 2^28
/// pixels, or both, on a grid whose pixels overflow 64 bits. A line gap
/// as far below 0 adds nothing, as any below 0.
#[test]
fn metrics_past_16_bits_size_the_cell_without_overflow() {
    fs::create_dir_all("target/test").unwrap();
    let mono = font::load(&font::Spec { path: draw_tests::FONT.into(), face: None, axes: None }).unwrap();
    let size_on = |name: &str, data: Vec<u8>, axes: &str, cols: usize, rows: usize| {
        let cells = crate::vt::parse(b"MM", cols, rows);
        let path = format!("target/test/metrics-cell-{name}.otf");
        fs::write(&path, data).unwrap();
        let font = font::load(&font::Spec { path: path.clone(), face: None, axes: Some(axes.into()) }).unwrap();
        let (mut w, mut h) = (0, 0);
        let sized = font.with_metrics(|face| unsafe { draw_face_cell_size(face, 24.0, &mut w, &mut h) });
        assert_eq!(sized, Ok(1), "{name}");
        let code = render_with(&cells, cols, rows, &font, Some(&mono), 24.0, &path.replace(".otf", ".png"));
        (w, h, code)
    };
    let size = |name: &str, data: Vec<u8>, axes: &str| size_on(name, data, axes, 2, 1);
    let wide = one_maps_to(&crafted(), b'M' as u16, 13);
    let (w, _, code) = size("wide-m", wide.clone(), "ax0=1,ax1=1");
    let font = font::load(&font::Spec { path: "target/test/metrics-cell-wide-m.otf".into(), face: None, axes: Some("ax0=1,ax1=1".into()) });
    assert!(drawn(&font.unwrap(), 14).0[13] > 65535);
    assert!(w > 1 && code == 0, "a wide M: cell {w} wide, exit {code}");
    // At ax0=1 (region 0): hhea's 800 and -200 to 1 and 0, and its line gap 0 moved by `gap`.
    let short = |gap: i32| {
        let rows: [&[i32]; 3] = [&[-799], &[200], &[gap]];
        mvar(8, &[(b"hasc", 0), (b"hdsc", 1), (b"hlgp", 2)], &store(2, &regions(), &[data(&[0], 0x8001, &rows)]))
    };
    let (w0, h0, code) = size("short", two_axes(&[(b"MVAR", &short(0))]), "ax0=1");
    assert!(code == 0 && w0 < 1 << 20 && h0 < 1 << 20, "a short face: cell {w0}x{h0}, exit {code}");
    let (w, h, code) = size("gap-up", two_axes(&[(b"MVAR", &short(2_000_000_000))]), "ax0=1");
    assert_eq!((w, h, code), (w0, h0 + (1 << 28), 2), "a line gap past 2^28 pixels");
    let (w, h, code) = size("gap-down", two_axes(&[(b"MVAR", &short(-2_000_000_000))]), "ax0=1");
    assert_eq!((w, h, code), (w0, h0, 0), "a line gap far below 0");
    let all = hvar(&store(2, &regions(), &[data(&[0], 0x8001, &[&[100_000_000]])]), &index_map(0, 1, 1, &[(0, 0)]));
    let (w, h, code) = size("advance-up", two_axes(&[(b"HVAR", &all), (b"MVAR", &short(0))]), "ax0=1");
    assert_eq!((w, code), (1 << 28, 2), "an advance past 2^28 pixels, cell {w}x{h}");
    // Both ways, on a grid whose pixels (2^64 and more) overflow 64 bits.
    let both = two_axes(&[(b"HVAR", &all), (b"MVAR", &short(2_000_000_000))]);
    let (w, h, code) = size_on("both-up", both, "ax0=1", 16, 16);
    assert!(w == 1 << 28 && h > 1 << 28 && code == 2, "a cell past 2^28 pixels each way: {w}x{h}, exit {code}");
}

/// The crafted font has HarfBuzz's metrics at each instance that
/// metrics-crafted-*.txt names. The font is written to
/// target/test/metrics-crafted.otf; after changing it, record them again:
///
///     tools/cff2-metrics.sh --variations=ax0=1,ax1=1 target/test/metrics-crafted.otf \
///         tests/fixtures/metrics-crafted-ax0=1,ax1=1.txt
#[test]
fn crafted_metrics_match_harfbuzz() {
    fs::create_dir_all("target/test").unwrap();
    fs::write("target/test/metrics-crafted.otf", crafted()).unwrap();
    let mut fixtures: Vec<_> = fs::read_dir("tests/fixtures")
        .unwrap()
        .map(|entry| entry.unwrap().path().to_str().unwrap().to_string())
        .filter(|path| path.starts_with("tests/fixtures/metrics-crafted"))
        .collect();
    fixtures.sort();
    assert!(fixtures.len() > 3, "{fixtures:?}");
    for fixture in fixtures {
        let (checked, moved) = matches_harfbuzz("target/test/metrics-crafted.otf", &fs::read_to_string(&fixture).unwrap());
        let default = fixture == "tests/fixtures/metrics-crafted.txt";
        assert!(checked == 14 && (default || moved > 5), "{fixture}: {checked} checked, {moved} moved");
    }
}
