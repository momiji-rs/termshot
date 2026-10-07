//! Font checking and render tests. These call render::draw_png_images, which
//! calls stb through src/stb_glue.c; run them with SANITIZE=1 ./test.sh to
//! have ASan watch stb_truetype.
//! Paths are relative to the repo root, where test.sh runs.

use super::*;
use crate::vt::{parse, replay};

pub(crate) const FONT: &str = "third_party/jetbrains-mono/JetBrainsMono-Regular.ttf";

pub(crate) fn render(cells: &[Cell], cols: usize, rows: usize, font: &font::Font, px: f64, out: &str) -> i32 {
    render_with(cells, cols, rows, font, None, px, out)
}

pub(crate) fn render_with(
    cells: &[Cell],
    cols: usize,
    rows: usize,
    font: &font::Font,
    fallback: Option<&font::Font>,
    px: f64,
    out: &str,
) -> i32 {
    render_marked(cells, &[], cols, rows, font, fallback, px, out)
}

/// render_with, with the cells' combining marks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_marked(
    cells: &[Cell],
    marks: &[CellMarks],
    cols: usize,
    rows: usize,
    font: &font::Font,
    fallback: Option<&font::Font>,
    px: f64,
    out: &str,
) -> i32 {
    render_framed(cells, marks, cols, rows, (font, fallback), px, out, &render::RenderOptions::NONE)
}

/// render_marked, with padding.
#[allow(clippy::too_many_arguments)]
fn render_framed(
    cells: &[Cell],
    marks: &[CellMarks],
    cols: usize,
    rows: usize,
    (font, fallback): (&font::Font, Option<&font::Font>),
    px: f64,
    out: &str,
    options: &render::RenderOptions,
) -> i32 {
    let draw = |font: &font::Face, fallback: Option<&font::Face>| unsafe {
        render::draw_png_with(cells, marks, cols, rows, font.ffi(), fallback.map(font::Face::ffi), px, out, false,
            &[], None, options)
    };
    font.with_face(|font| match fallback {
        None => draw(font, None),
        Some(fallback) => fallback.with_face(|fallback| draw(font, Some(fallback))).unwrap(),
    })
    .unwrap()
}

fn load(value: &str) -> font::Font {
    font::load(&font::Spec::parse(value).unwrap()).unwrap()
}

/// The mutation tests/fontfuzz used to find stb_truetype crashes, ported
/// exactly so its crashing seeds reproduce here.
pub(crate) fn mutate(font: &mut [u8], seed: u64) {
    let mut x = 0x9e37_79b9_7f4a_7c15u64 ^ seed.wrapping_mul(0x2545_f491_4f6c_dd1d);
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 16) as u32
    };
    let len = font.len();
    for _ in 0..1 + next() % 8 {
        let at = next() as usize % len;
        let width = 1 + next() % 4;
        for b in 0..width as usize {
            if at + b >= len {
                break;
            }
            font[at + b] = if next() % 3 == 0 { 0xff } else { next() as u8 };
        }
    }
}

#[test]
fn vendored_font_passes_the_check() {
    font::check(&fs::read(FONT).unwrap()).unwrap();
}

#[test]
fn truncated_and_foreign_files_are_rejected() {
    let font = fs::read(FONT).unwrap();
    for len in [0, 3, 11, 100, 1000, font.len() / 2, font.len() - 1] {
        assert!(font::check(&font[..len]).is_err(), "accepted the first {len} bytes");
    }
    assert!(font::check(b"#!/bin/sh\necho not a font\n").is_err());
}

/// The vendored font with its glyf table renamed to tag.
fn retagged(tag: &[u8; 4]) -> Vec<u8> {
    let mut font = fs::read(FONT).unwrap();
    let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
    let record = (0..tables).map(|i| 12 + 16 * i).find(|&r| &font[r..r + 4] == b"glyf").unwrap();
    font[record..record + 4].copy_from_slice(tag);
    font
}

#[test]
fn a_font_without_glyf_is_refused_for_what_it_has_instead() {
    // glyf's bytes, read as a CFF or CFF2 table.
    let error = font::check(&retagged(b"CFF ")).unwrap_err();
    assert!(error.starts_with("CFF table: "), "{error}");
    let error = font::check(&retagged(b"CFF2")).unwrap_err();
    assert!(error.starts_with("CFF2 table: "), "{error}");
    for (tag, name) in [(b"CBDT", "CBDT"), (b"CBLC", "CBLC"), (b"sbix", "sbix")] {
        assert_eq!(
            font::check(&retagged(tag)).unwrap_err(),
            format!("a color bitmap font ({name}) with no outlines; use a monochrome outline font, such as Noto Emoji")
        );
    }
    assert_eq!(font::check(&retagged(b"xxxx")).unwrap_err(), "no glyf table, and no CFF or CFF2 table either");
}

/// Render with the vendored font, as both fonts when fallback is set, and
/// return what the render reports about empty glyphs.
fn empty_glyphs(text: &str, cols: usize, fallback: bool) -> EmptyGlyphs {
    let font = font::prepare(fs::read(FONT).unwrap()).unwrap();
    let cells = parse(text.as_bytes(), cols, 2);
    // Not zeroed: the render must clear it.
    let mut empty = EmptyGlyphs { cp: 1, fonts: 9, col: 9, row: 9, cells: 9 };
    let code = font
        .with_face(|face| {
            let fallback = fallback.then_some(face.ffi());
            unsafe {
                render::draw_png_images(&cells, &[], cols, 2, face.ffi(), fallback, 16.0,
                    "target/test/empty-glyphs.png", false, &[], Some(&mut empty))
            }
        })
        .unwrap();
    assert_eq!(code, 0);
    empty
}

/// A bug in the image layers (here a crop outside its image, which
/// termshot never makes) fails the render, in every layer: the render returns 2,
/// which main makes exit 2, and writes no PNG.
#[test]
fn a_failed_image_layer_fails_the_render() {
    let font = font::prepare(fs::read(FONT).unwrap()).unwrap();
    let cells = parse(b"ab\r\ncd", 4, 2);
    let path = "target/test/failed-image-layer.png";
    let pixels = [255u8; 16];
    for z in [i32::MIN, -1, 0] {
        for src_y in [0, 2] {
            let mut view = composite::ImageView::solid(&[0; 4], 1, 1, 9, 9);
            (view.pixels, view.width, view.height, view.src_w, view.src_h, view.src_y, view.z) =
                (pixels.as_ptr(), 2, 2, 2, 2, src_y, z);
            let _ = fs::remove_file(path);
            let code = font
                .with_face(|face| unsafe {
                    render::draw_png_images(&cells, &[], 4, 2, face.ffi(), None, 16.0, path, false,
                        std::slice::from_ref(&view), None)
                })
                .unwrap();
            assert_eq!(code, if src_y == 0 { 0 } else { 2 }, "z {z}, src_y {src_y}");
            assert_eq!(fs::metadata(path).is_ok(), src_y == 0);
        }
    }
}

#[test]
fn draw_png_reports_cells_drawn_as_boxes_for_an_empty_glyph() {
    // The vendored font maps U+16910 (Bamum) to an empty glyph.
    let e = empty_glyphs("ab\r\nc\u{16910}d\u{16910}", 4, false);
    assert_eq!((e.cp, e.fonts, e.col, e.row, e.cells), (0x16910, EMPTY_IN_FONT, 1, 1, 2));
    let e = empty_glyphs("\u{16910}", 4, true);
    assert_eq!((e.cp, e.fonts, e.cells), (0x16910, EMPTY_IN_FONT | EMPTY_IN_FALLBACK, 1));
}

#[test]
fn draw_png_reports_nothing_for_glyphs_it_draws_or_lacks() {
    // Drawn; blank by design; not in the font at all (plain tofu).
    for text in ["Ag─█", "\u{3000}\u{2800}", "中\u{10FFFD}"] {
        let e = empty_glyphs(text, 6, false);
        assert_eq!((e.cp, e.fonts, e.col, e.row, e.cells), (0, 0, 0, 0, 0), "{text:?}");
    }
}

/// A face's color bitmap table is the face's own: a font with one beside
/// its outlines, as Apple Color Emoji has, and in a collection only the
/// face drawn with (the CLI's empty-glyph warning names it,
/// src/cli_tests.rs).
#[test]
fn a_color_bitmap_table_is_the_face_s_own() {
    let plain = fs::read(FONT).unwrap();
    let mut color = plain.clone();
    let tables = u16::from_be_bytes([color[4], color[5]]) as usize;
    let record = (0..tables).map(|i| 12 + 16 * i).find(|&r| &color[r..r + 4] == b"name").unwrap();
    color[record..record + 4].copy_from_slice(b"sbix");
    assert_eq!(font::color_bitmap(&font::prepare(plain.clone()).unwrap()), None);
    assert_eq!(font::color_bitmap(&font::prepare(color).unwrap()), Some("sbix"));

    // In a collection, only the face drawn with counts. ("name" is the
    // collection's own, so another table becomes sbix.)
    let mut color_face = plain.clone();
    let record = (0..tables).map(|i| 12 + 16 * i).find(|&r| &color_face[r..r + 4] == b"post").unwrap();
    color_face[record..record + 4].copy_from_slice(b"sbix");
    let ttc = font::tests::collection(&[(&plain, "Plain"), (&color_face, "Color")]);
    assert_eq!(font::color_bitmap(&font::tests::choose_padded(ttc.clone(), "0")), None);
    assert_eq!(font::color_bitmap(&font::tests::choose_padded(ttc, "1")), Some("sbix"));
}

/// Seeds where stock stb_truetype read out of bounds, hit an assert, or
/// overflowed (cmap offsets and groups, loca, glyph ids from the cmap).
const CRASHING_SEEDS: [u64; 8] = [504, 1614, 2189, 2813, 3335, 3414, 3607, 4545];

#[test]
fn fonts_that_crashed_stb_are_rejected() {
    let original = fs::read(FONT).unwrap();
    for seed in CRASHING_SEEDS {
        let mut font = original.clone();
        mutate(&mut font, seed);
        assert!(font::check(&font).is_err(), "seed {seed} passed the check");
    }
}

#[test]
fn mutated_fonts_are_rejected_or_render() {
    let original = fs::read(FONT).unwrap();
    let cells = parse("Ag@M0│·●—W╭─╮█".as_bytes(), 14, 1);
    let mut rendered = 0;
    for seed in 0..400 {
        let mut font = original.clone();
        mutate(&mut font, seed);
        if let Ok(font) = font::prepare(font) {
            render(&cells, 14, 1, &font, 16.0, "target/test/fuzz-font.png");
            rendered += 1;
        }
    }
    // Most single-byte edits land in glyph data or hinting and stay valid.
    assert!(rendered > 100, "only {rendered} of 400 mutated fonts rendered");
}

const CJK_FONT: &str = "third_party/noto-sans-cjk/NotoSansCJKtc-Subset.otf";
const CJK_VF_FONT: &str = "third_party/noto-sans-cjk-vf/NotoSansCJKtc-VF-Subset.otf";

/// Renders on several threads at once are independent (#8 was a glyph cache
/// shared between them): each thread draws one of four different screens,
/// with different fonts (TrueType, a CFF fallback, CFF2) and sizes, three
/// times over, and each PNG is the one drawn alone. Two of them have a
/// palette and padding of their own (#87). The glyph cache, the outline
/// scratch, the box-drawing cache, the backdrop, the palette, the padding
/// and the fault counters are all the render's or the thread's own.
#[test]
fn draw_png_is_reentrant() {
    let font = load(FONT);
    let (cjk, vf, marks_font) = (load(CJK_FONT), load(CJK_VF_FONT), load(MARKS_FONT));
    let logs = ["\x1b[1mbold\x1b[0m ─╭╮ plain \x1b[4munder\x1b[0m",
                "\x1b[3mitalic\x1b[23m q\u{301}x \u{e31}\u{e49} \x1b[9m\u{1F600}\x1b[0m",
                "中文 \x1b[3m界\x1b[1m面\x1b[0m ab\u{16910}",
                "\x1b[1;3m中文 Ag\x1b[0m ╰─╯ 界"];
    let mut themed = Palette::DEFAULT;
    (themed.foreground, themed.background, themed.named[1]) = ((10, 20, 30), (240, 230, 220), (0, 128, 255));
    let palettes = [Palette::DEFAULT, themed, Palette::DEFAULT, Palette { background: (0, 64, 0), ..themed }];
    let paddings = [(0, 0), (7, 3), (0, 0), (1, 12)];
    let screens: Vec<(Vec<Cell>, Vec<CellMarks>)> = logs.iter().zip(palettes).map(|(log, palette)| {
        let log = format!("\x1b[41m \x1b[49m{log}");
        let g = replay_with(log.as_bytes(), 20, 2, &ParseOptions { lf: Lf::Index, palette }, (1, 1)).unwrap();
        (g.cells, g.marks)
    }).collect();
    let draw = |k: usize, out: &str| {
        let (fonts, px): ((&font::Font, Option<&font::Font>), f64) = match k {
            0 => ((&font, None), 48.0),
            1 => ((&font, Some(&marks_font)), 46.0),
            2 => ((&font, Some(&cjk)), 24.0),
            _ => ((&vf, Some(&font)), 47.5),
        };
        let (cells, marks) = &screens[k];
        let options = render::RenderOptions { padding: paddings[k], background: palettes[k].background };
        assert_eq!(render_framed(cells, marks, 20, 2, fonts, px, out, &options), 0);
        fs::read(out).unwrap()
    };
    let alone: Vec<Vec<u8>> = (0..4).map(|k| draw(k, &format!("target/test/thread-alone-{k}.png"))).collect();
    for k in 1..4 {
        assert!(alone[k] != alone[0], "screen {k} draws as screen 0 does");
    }
    std::thread::scope(|scope| {
        for t in 0..12 {
            let (draw, alone) = (&draw, &alone);
            scope.spawn(move || {
                for round in 0..3 {
                    let k = (t + round) % 4;
                    let out = format!("target/test/thread-{t}.png");
                    assert!(draw(k, &out) == alone[k], "thread {t} round {round}: screen {k} differs");
                }
            });
        }
    });
}

/// A glyph allocation that fails (the cache's slots, a CFF outline's
/// scratch, or a glyph's bitmap; src/glyphs.rs fails each in turn with fake
/// fonts) fails the render with the vendored fonts too: the render returns 2,
/// which main makes exit 2, and writes no PNG.
#[test]
fn a_failed_glyph_allocation_fails_the_render() {
    use glyphs::faults::{Faults, FAULTS};
    let (font, cjk) = (load(FONT), load(CJK_FONT));
    let g = replay("ab\x1b[3mc\x1b[0m 中q\u{301}".as_bytes(), 8, 1, Lf::Index);
    let path = "target/test/failed-glyph-allocation.png";
    let mut fail = 0;
    loop {
        FAULTS.with(|f| f.set(Faults { calls: 0, fail_at: fail }));
        let _ = fs::remove_file(path);
        let code = render_marked(&g.cells, &g.marks, 8, 1, &cjk, Some(&font), 16.0, path);
        let calls = FAULTS.with(|f| f.get().calls);
        FAULTS.with(|f| f.set(Faults::default()));
        if fail > calls {
            break;
        }
        assert_eq!(code, if fail == 0 { 0 } else { 2 }, "allocation {fail} of {calls}");
        assert_eq!(fs::metadata(path).is_ok(), fail == 0, "allocation {fail}");
        fail += 1;
    }
    // The cache, the scratch, and the bitmaps of a, b, c (italic), 中, q
    // and the mark.
    assert!(fail > 6, "only {} allocations", fail - 1);
}

/// The render's own allocations, the canvas and the PNG's buffer (which
/// stb_image_write asks for through termshot_png_alloc, and the render
/// returns), fail it each in turn: the render returns 2, which main makes
/// exit 2, and writes no PNG. tests/run.sh fails them through the CLI too.
#[test]
fn a_failed_render_allocation_fails_the_render() {
    use render::faults::{Faults, FAILED, FAULTS};
    use render::Site;
    let font = load(FONT);
    let cells = parse("ab ─╮".as_bytes(), 6, 1);
    let path = "target/test/failed-render-allocation.png";
    let mut sites = Vec::new();
    let mut fail = 0;
    loop {
        FAULTS.with(|f| f.set(Faults { calls: 0, fail_at: fail }));
        FAILED.with(|f| f.set(None));
        let _ = fs::remove_file(path);
        let code = render(&cells, 6, 1, &font, 16.0, path);
        let calls = FAULTS.with(|f| f.get().calls);
        FAULTS.with(|f| f.set(Faults::default()));
        if fail > calls {
            break;
        }
        assert_eq!(code, if fail == 0 { 0 } else { 2 }, "allocation {fail} of {calls}");
        assert_eq!(fs::metadata(path).is_ok(), fail == 0, "allocation {fail}");
        if fail > 0 {
            sites.push(FAILED.with(|f| f.get()).unwrap());
        }
        fail += 1;
    }
    assert_eq!(sites, [Site::Canvas, Site::Png]);
}

/// Written for test.sh's CLI checks, which run after these tests.
const COLLECTION: &str = "target/test/collection.ttc";

#[test]
fn draw_png_draws_the_chosen_face() {
    fs::write(COLLECTION, font::tests::two_faces()).unwrap();
    let cells = parse(b"Ag", 2, 1);
    let draw = |spec: &str, out: &str| {
        assert_eq!(render(&cells, 2, 1, &load(spec), 16.0, out), 0);
        fs::read(out).unwrap()
    };
    let plain = draw(FONT, "target/test/face-plain.png");
    assert!(draw(&format!("{COLLECTION}#0"), "target/test/face-0.png") == plain);
    assert!(draw(&format!("{COLLECTION}#Face B"), "target/test/face-1.png") != plain);
}

#[test]
fn draw_png_draws_the_chosen_fallback_face() {
    // A first font with no outlines at all, so the fallback draws every character.
    let ttc = font::tests::two_faces();
    let empty = font::tests::edit_table(&fs::read(FONT).unwrap(), b"loca", |loca| loca.fill(0));
    let empty = font::prepare(empty).unwrap();
    let cells = parse(b"Ag", 2, 1);
    let draw = |fallback: font::Font, out: &str| {
        assert_eq!(render_with(&cells, 2, 1, &empty, Some(&fallback), 16.0, out), 0);
        fs::read(out).unwrap()
    };
    let plain = draw(load(FONT), "target/test/fallback-plain.png");
    let face = |face: &str| font::tests::choose_padded(ttc.clone(), face);
    assert!(draw(face("0"), "target/test/fallback-0.png") == plain);
    assert!(draw(face("1"), "target/test/fallback-1.png") != plain);
}

#[test]
fn a_file_named_with_a_hash_is_that_file() {
    let odd = "target/test/odd#1.ttf";
    fs::copy(FONT, odd).unwrap();
    let font = load(odd);
    assert!(font.start == 0 && font.face.is_none());
    assert_eq!(font::Spec::parse(odd), Ok(font::Spec { path: odd.into(), face: None, axes: None }));
    assert_eq!(
        font::Spec::parse("target/test/absent.ttc#Noto Sans"),
        Ok(font::Spec { path: "target/test/absent.ttc".into(), face: Some("Noto Sans".into()), axes: None })
    );
}

const MARKS_FONT: &str = "third_party/noto-sans-marks/NotoSans-Marks-Subset.ttf";

/// Combining marks are drawn over their cell, from the font or else the
/// fallback; a mark neither has and a joiner draw nothing, not a box.
#[test]
fn draw_png_draws_marks_over_their_cells() {
    let font = load(FONT);
    let fallback = load(MARKS_FONT);
    let draw = |log: &str, fallback: Option<&font::Font>, name: &str| {
        let g = replay(log.as_bytes(), 3, 1, Lf::Index);
        let out = format!("target/test/draw-marks-{name}.png");
        assert_eq!(render_marked(&g.cells, &g.marks, 3, 1, &font, fallback, 24.0, &out), 0);
        fs::read(out).unwrap()
    };
    let plain = draw("q x", None, "plain");
    assert!(draw("q\u{301} x", None, "acute") != plain);
    assert!(draw("q \u{302}x", None, "on-a-space") != plain);
    for (log, name) in [("q\u{200d} x", "joiner"), ("q\u{20dd} x", "in-no-font"), ("q\u{e31} x", "no-fallback")] {
        assert!(draw(log, None, name) == plain, "{name}");
    }
    assert!(draw("q\u{e31} x", Some(&fallback), "fallback") != draw("q x", Some(&fallback), "fallback-plain"));
}

/// The Hangul fillers are default ignorable, as joiners are, and draw
/// nothing either, though they take cells of their own (U+3164 two).
#[test]
fn draw_png_draws_nothing_for_a_hangul_filler() {
    let font = load(FONT);
    let draw = |log: &str, name: &str| {
        let cells = parse(log.as_bytes(), 5, 1);
        let out = format!("target/test/draw-filler-{name}.png");
        assert_eq!(render(&cells, 5, 1, &font, 24.0, &out), 0);
        fs::read(out).unwrap()
    };
    let plain = draw("q   x", "plain");
    for (log, name) in [("q\u{3164} x", "3164"), ("q\u{115f} x", "115f"), ("q\u{ffa0}  x", "ffa0")] {
        assert!(draw(log, name) == plain, "U+{name}");
    }
}
