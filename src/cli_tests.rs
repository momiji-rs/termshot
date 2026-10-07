//! The CLI's unit tests: its argument checks and the wording of its
//! warning, which only src/main.rs has. The library's are its own test
//! build's (`rustc --test src/lib.rs`).

use super::*;

const FONT: &str = "third_party/jetbrains-mono/JetBrainsMono-Regular.ttf";

#[test]
fn a_bad_cursor_is_refused_before_the_log_is_read() {
    let args = |list: &[&str]| parse_args(list.iter().map(|s| s.to_string()));
    assert!(matches!(args(&["--cursor", "nonsense", "-", "out.png"]), Err(e) if e.contains("must look like 4,2")));
    // With the size given, its bounds are checked at once too.
    assert!(matches!(args(&["--size", "10x4", "--cursor", "10,4", "-", "out.png"]), Err(e) if e.contains("off the 10x4 grid")));
    // Without it, a cast may give the size: the bounds wait for it.
    assert!(matches!(args(&["--cursor", "300,100", "-", "out.png"]), Ok(Command::Render(_))));
    // Under --raw no cast can, so the default size bounds it at once.
    assert!(matches!(args(&["--raw", "--cursor", "300,100", "-", "out.png"]), Err(e) if e.contains("off the 100x30 grid")));
    assert!(matches!(args(&["--raw", "--cursor", "99,29", "-", "out.png"]), Ok(Command::Render(_))));
    let legacy = args(&["--raw", "--cursor", "41,29", "-", "out.png", "font.ttf", "48", "40"]);
    assert!(matches!(legacy, Err(e) if e.contains("off the 40x30 grid")));
}

#[test]
fn cursor_option_counts_from_0_as_tmux_does() {
    assert_eq!(parse_cursor("4,2", 10, 4), Ok(Some((2, 4))));
    assert_eq!(parse_cursor("0,0", 10, 4), Ok(Some((0, 0))));
    assert_eq!(parse_cursor("none", 10, 4), Ok(None));
    // tmux reports a pending wrap as one past the last column.
    assert_eq!(parse_cursor("10,3", 10, 4), Ok(Some((3, 9))));
    for bad in ["11,0", "0,4", "4", "4,", ",2", "-1,0", "4,2,1", "4;2", "", "None"] {
        assert!(parse_cursor(bad, 10, 4).is_err(), "{bad:?} accepted");
    }
    // The message gives the range that is accepted, pending wrap included.
    let message = parse_cursor("11,0", 10, 4).unwrap_err();
    assert!(message.contains("columns go from 0 to 10 (10 is a pending wrap"), "{message}");
    assert!(message.contains("rows from 0 to 3"), "{message}");
}

/// The CLI's warning for what a render reports in `empty`, None when it
/// reports nothing.
fn warning(args: &[&str], empty: Option<Empty>, font: &[u8], fallback: Option<&[u8]>) -> Option<String> {
    let Ok(Command::Render(options)) = parse_args(args.iter().map(|a| a.to_string())) else { panic!("{args:?}") };
    let load = |bytes: &[u8]| Font::from_bytes(bytes.to_vec(), &termshot::FaceSelector::default()).unwrap();
    let (font, fallback) = (load(font), fallback.map(load));
    Some(empty_glyph_warning(&empty?, &options, &font, fallback.as_ref()))
}

#[test]
fn the_empty_glyph_warning_names_the_cell_the_fonts_and_the_fix() {
    let plain = fs::read(FONT).unwrap();
    // A font with a color bitmap table beside its outlines, as Apple Color Emoji has.
    let mut color = plain.clone();
    let tables = u16::from_be_bytes([color[4], color[5]]) as usize;
    let record = (0..tables).map(|i| 12 + 16 * i).find(|&r| &color[r..r + 4] == b"name").unwrap();
    color[record..record + 4].copy_from_slice(b"sbix");
    let face = termshot::FaceSelector::default();
    assert_eq!(Font::from_bytes(color.clone(), &face).unwrap().color_bitmap(), Some("sbix"));

    let one = Empty { ch: '\u{1F600}', row: 1, col: 3, cells: 1, in_font: true, in_fallback: false };
    assert_eq!(warning(&["log", "out.png"], None, &plain, None), None);
    assert_eq!(
        warning(&["log", "out.png"], Some(one), &plain, None).unwrap(),
        "warning: U+1F600 at column 3, row 1 (from 0) is drawn as a box: the built-in font maps it to an \
         empty glyph; pass --fallback-font with an outline font that has it"
    );
    let many = Empty { cells: 4, ..one };
    assert_eq!(
        warning(&["--font", "c.ttf", "log", "out.png"], Some(many), &color, None).unwrap(),
        "warning: U+1F600 at column 3, row 1 (from 0) is drawn as a box (the first of 4 such cells): \
         --font c.ttf (a color bitmap font, sbix, which termshot cannot draw) maps it to an empty glyph; \
         pass --fallback-font with an outline font that has it, such as Noto Emoji"
    );
    // Only the fallback: the font lacks the character outright.
    let fallback = Empty { in_font: false, in_fallback: true, ..one };
    assert_eq!(
        warning(&["--fallback-font", "c.ttf", "log", "out.png"], Some(fallback), &plain, Some(&color)).unwrap(),
        "warning: U+1F600 at column 3, row 1 (from 0) is drawn as a box: --fallback-font c.ttf (a color \
         bitmap font, sbix, which termshot cannot draw) maps it to an empty glyph; pass a --fallback-font \
         with an outline for it, such as Noto Emoji"
    );
    // A face of a collection is named as it was given.
    assert!(warning(&["--font", "a.ttc#Color", "log", "out.png"], Some(one), &plain, None)
        .unwrap()
        .contains(": --font a.ttc#Color maps it to an empty glyph;"));
    let both = Empty { in_fallback: true, ..one };
    assert_eq!(
        warning(&["--font", "a.ttf", "--fallback-font", "b.ttf", "log", "out.png"], Some(both), &plain, Some(&plain)).unwrap(),
        "warning: U+1F600 at column 3, row 1 (from 0) is drawn as a box: --font a.ttf and --fallback-font \
         b.ttf map it to empty glyphs; pass a --fallback-font with an outline for it"
    );
}
