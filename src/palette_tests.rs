//! Palette tests (#87): the --palette file's format, and how a palette
//! changes the colours a log resolves to and everything that compares
//! with the default colours afterwards.

use super::*;
use crate::palette::{parse_color, MAX_FILE_BYTES};

type Rgb = (u8, u8, u8);

/// A palette where every colour differs from the default palette's and from
/// each other: foreground, background and color0..15.
fn custom() -> Palette {
    let mut named = [(0, 0, 0); 16];
    for (i, color) in named.iter_mut().enumerate() {
        *color = (10 + i as u8, 100 + i as u8, 200 - i as u8);
    }
    Palette { foreground: (250, 240, 230), background: (1, 2, 3), named }
}

fn replay_in(log: &[u8], cols: usize, rows: usize, palette: Palette) -> Grid {
    replay_with(log, cols, rows, &ParseOptions { lf: Lf::Index, palette }, (1, 1))
}

fn fg(c: &Cell) -> Rgb {
    (c.fr, c.fg, c.fb)
}

fn bg(c: &Cell) -> Rgb {
    (c.br, c.bg, c.bb)
}

#[test]
fn a_file_sets_each_key_it_names() {
    let p = custom();
    assert_eq!(Palette::DEFAULT.with_file(p.to_file().as_bytes()), Ok(p));
    // Comments, blank lines, CRLF, tabs and spaces around; upper-case hex.
    let file = "# a theme\r\n\r\n  foreground\t#A0B0C0  \r\n#color1 #000000\n\tcolor15   #0f0F0f\ncolor0 #010203";
    let got = Palette::DEFAULT.with_file(file.as_bytes()).unwrap();
    let mut want = Palette::DEFAULT;
    want.foreground = (0xa0, 0xb0, 0xc0);
    want.named[15] = (15, 15, 15);
    want.named[0] = (1, 2, 3);
    assert_eq!(got, want);
    // An empty file, or only comments, changes nothing.
    assert_eq!(Palette::DEFAULT.with_file(b""), Ok(Palette::DEFAULT));
    assert_eq!(Palette::DEFAULT.with_file(b"# nothing\n\n"), Ok(Palette::DEFAULT));
    // It applies over the palette it is given.
    assert_eq!(p.with_file(b"color3 #ffffff\n").unwrap().named[2], p.named[2]);
}

#[test]
fn a_malformed_file_says_which_line_and_why() {
    let error = |file: &[u8]| Palette::DEFAULT.with_file(file).unwrap_err();
    for (file, want) in [
        (&b"foreground #000000\ncursor #ffffff\n"[..], "line 2: unknown key \"cursor\"; a palette sets foreground, background and color0 to color15"),
        (b"color16 #ffffff", "line 1: color16 is not one of the 16 named colours; colours 16 to 255 keep xterm's values"),
        (b"color255 #ffffff", "line 1: color255 is not one of the 16 named colours"),
        (b"color256 #ffffff", "line 1: unknown key \"color256\""),
        (b"color01 #ffffff", "line 1: unknown key \"color01\""),
        (b"color+1 #ffffff", "line 1: unknown key \"color+1\""),
        (b"color #ffffff", "line 1: unknown key \"color\""),
        (b"Foreground #ffffff", "line 1: unknown key \"Foreground\""),
        (b"\n\nbackground", "line 3: \"background\" has no colour"),
        (b"background #fff", "line 1: background: a colour must look like #1e2a3b, not \"#fff\""),
        (b"background ffffff", "line 1: background: a colour must look like #1e2a3b, not \"ffffff\""),
        (b"background #ffffff #000000", "not \"#ffffff #000000\""),
        (b"background #gggggg", "not \"#gggggg\""),
        (b"background red", "not \"red\""),
        (b"color2 #000000\ncolor2 #ffffff", "line 2: color2 is already set, on line 1"),
        (b"background #000000\xff", "not UTF-8 text (at byte 18)"),
        // Only spaces and tabs separate or trim; a no-break space is a key.
        (b"color1 #000000\n\xc2\xa0", "line 2: \"\\u{a0}\" has no colour"),
    ] {
        let got = error(file);
        assert!(got.contains(want), "{:?}: {got}", String::from_utf8_lossy(file));
    }
    let big = vec![b'#'; MAX_FILE_BYTES + 1];
    assert!(error(&big).contains("over 65536 bytes"));
    let mut fits = vec![b'#'; MAX_FILE_BYTES];
    fits[MAX_FILE_BYTES - 1] = b'\n';
    assert_eq!(Palette::DEFAULT.with_file(&fits), Ok(Palette::DEFAULT));
}

#[test]
fn colours_are_six_hex_digits() {
    assert_eq!(parse_color("#000000"), Ok((0, 0, 0)));
    assert_eq!(parse_color("#fFa01b"), Ok((255, 160, 27)));
    for bad in ["", "#", "000000", "#00000", "#0000000", "#00000g", "# 00000", "#+12345", "#-12345", "rgb:00/00/00", "#ffffff\n"] {
        assert!(parse_color(bad).is_err(), "{bad:?}");
    }
}

/// The 256 colours under a palette: 0 to 15 are its own, the rest xterm's.
#[test]
fn a_palette_names_only_the_16_colours() {
    let p = custom();
    for n in 0..256 {
        let want = if n < 16 { p.named[n as usize] } else { Palette::DEFAULT.color(n).unwrap() };
        assert_eq!(p.color(n), Some(want), "colour {n}");
    }
    assert_eq!(p.color(256), None);
    // xterm's cube and greys.
    assert_eq!(p.color(16), Some((0, 0, 0)));
    assert_eq!(p.color(196), Some((255, 0, 0)));
    assert_eq!(p.color(231), Some((255, 255, 255)));
    assert_eq!(p.color(232), Some((8, 8, 8)));
    assert_eq!(p.color(255), Some((238, 238, 238)));
}

#[test]
fn sgr_resolves_through_the_palette() {
    let p = custom();
    // Each code on its own cell, then a reset.
    let codes: [(&str, Rgb, Rgb); 20] = [
        ("31", p.named[1], p.background),
        ("37", p.named[7], p.background),
        ("41", p.foreground, p.named[1]),
        ("47", p.foreground, p.named[7]),
        ("90", p.named[8], p.background),
        ("97", p.named[15], p.background),
        ("100", p.foreground, p.named[8]),
        ("107", p.foreground, p.named[15]),
        ("38;5;3", p.named[3], p.background),
        ("38;5;15", p.named[15], p.background),
        ("48;5;12", p.foreground, p.named[12]),
        ("38:5:4", p.named[4], p.background),
        ("48:5:9", p.foreground, p.named[9]),
        // The cube, the greys and 24-bit colours keep their values.
        ("38;5;16", (0, 0, 0), p.background),
        ("48;5;196", p.foreground, (255, 0, 0)),
        ("38;5;244", (128, 128, 128), p.background),
        ("38;2;1;2;3", (1, 2, 3), p.background),
        ("48:2::9:8:7", p.foreground, (9, 8, 7)),
        // 39 and 49 are the palette's defaults.
        ("31;41;39;49", p.foreground, p.background),
        ("0", p.foreground, p.background),
    ];
    let mut log = String::new();
    for (code, _, _) in codes {
        log += &format!("\x1b[{code}mx\x1b[m");
    }
    let g = replay_in(log.as_bytes(), codes.len(), 2, p);
    for (i, (code, want_fg, want_bg)) in codes.iter().enumerate() {
        assert_eq!((fg(&g.cells[i]), bg(&g.cells[i])), (*want_fg, *want_bg), "SGR {code}");
    }
    // The cells nothing was printed in are in the palette's background.
    assert!(g.cells[codes.len()..].iter().all(|c| (fg(c), bg(c)) == (p.foreground, p.background)));
    // The same log with the default palette is what it always was.
    let d = replay_sized(log.as_bytes(), codes.len(), 2, Lf::Index, (1, 1));
    assert_eq!(fg(&d.cells[0]), (205, 0, 0));
    assert_eq!(bg(&d.cells[2]), (205, 0, 0));
    assert_eq!(bg(&d.cells[codes.len()]), DEFAULT_BG);
}

/// Reverse, dim and conceal work on the palette's colours, as they do on
/// any others; bold does not brighten a named colour (termshot draws bold
/// as bold, as kitty does by default).
#[test]
fn attributes_mix_the_palette_s_colours() {
    let p = custom();
    let g = replay_in(b"\x1b[7ma\x1b[31;42mb\x1b[m\x1b[2mc\x1b[m\x1b[8md\x1b[m\x1b[1;31me", 10, 1, p);
    assert_eq!((fg(&g.cells[0]), bg(&g.cells[0])), (p.background, p.foreground));
    assert_eq!((fg(&g.cells[1]), bg(&g.cells[1])), (p.named[2], p.named[1]));
    let mix = |f: u8, b: u8| ((2 * u16::from(f) + u16::from(b)) / 3) as u8;
    let (f, b) = (p.foreground, p.background);
    assert_eq!(fg(&g.cells[2]), (mix(f.0, b.0), mix(f.1, b.1), mix(f.2, b.2)));
    assert_eq!(fg(&g.cells[3]), p.background);
    assert_eq!(fg(&g.cells[4]), p.named[1]);
    assert_eq!(g.cells[4].attrs & BOLD, BOLD);
}

/// Erasing with the default pen, a reset (RIS), DECRC with nothing saved,
/// scrolling and the alternate screen all bring in the palette's colours.
#[test]
fn blank_cells_are_the_palette_s() {
    let p = custom();
    let plain = (p.foreground, p.background);
    for log in [
        &b"\x1b[41mxxxx\x1b[m\x1b[2J"[..],
        b"\x1b[41mxxxx\x1b[m\r\x1b[K",
        b"\x1b[41mxxxx\x1bc",
        b"\x1b[41m\x1b8xxxx",
        b"\x1b[41mxxxx\x1b[m\r\n\n\n",
        b"\x1b[41mxxxx\x1b[m\x1b[?1049h",
        b"\x1b[41mxxxx\x1b[m\x1b[S",
        b"\x1b[41mxxxx\x1b[m\r\x1b[4@",
    ] {
        let g = replay_in(log, 4, 2, p);
        let row = &g.cells[..4];
        let want = if log.ends_with(b"xxxx") { (p.foreground, p.background) } else { plain };
        assert!(row.iter().all(|c| (fg(c), bg(c)) == want), "{:?}", String::from_utf8_lossy(log));
        assert!(g.cells[4..].iter().all(|c| (fg(c), bg(c)) == plain), "{:?}", String::from_utf8_lossy(log));
    }
}

/// A Unicode placeholder's ids are the colour numbers, whatever colour the
/// palette gives them: a theme doesn't change which image a cell shows.
#[test]
fn placeholder_ids_ignore_the_palette() {
    let log = "\x1b_Ga=T,f=24,s=1,v=1,i=5,U=1,c=1,r=1;/wAA\x1b\\\x1b[38;5;5m\u{10EEEE}\u{305}\u{305}";
    let default = replay_sized(log.as_bytes(), 4, 2, Lf::Index, (10, 20));
    assert_eq!(default.images.len(), 1, "the placeholder shows image 5");
    let themed = replay_with(log.as_bytes(), 4, 2, &ParseOptions { lf: Lf::Index, palette: custom() }, (10, 20));
    assert_eq!(themed.images.len(), 1);
    let views = |g: &Grid| g.images.iter().flat_map(graphics::Placement::views).map(|v| (v.x, v.y, v.w, v.h)).collect::<Vec<_>>();
    assert_eq!(views(&themed), views(&default));
}

/// What is compared with the default background afterwards uses the
/// palette's: the cells kitty's lowest layer shows through, the cursor's
/// colour, and the blank cells --json leaves out.
#[test]
fn the_default_background_is_the_palette_s() {
    let p = custom();
    let (br, bgr, bb) = p.background;
    let old = DEFAULT_BG;
    let log = format!(
        "a\x1b[48;2;{br};{bgr};{bb}mb\x1b[48;2;{};{};{}mc\x1b[41md\x1b[m",
        old.0, old.1, old.2
    );
    let mut cells = replay_in(log.as_bytes(), 6, 1, p).cells;
    opaque_backgrounds(&mut cells, p.background);
    let opaque: Vec<bool> = cells.iter().map(|c| c.attrs & OPAQUE != 0).collect();
    // The palette's background, given or set explicitly, is clear; the old
    // default is now a colour like any other.
    assert_eq!(opaque, [false, false, true, true, false, false]);

    // The bar cursor: the palette's foreground, or its background on a cell
    // whose background is the palette's foreground.
    let (fr, fg_, fb) = p.foreground;
    let g = replay_in(format!("x\x1b[48;2;{fr};{fg_};{fb}my\x1b[m\x1b[H").as_bytes(), 4, 1, p);
    let colour = |col| cursor_mark(&g.cells, 4, (0, col), CursorShape::Bar, (8, 16), &p).1;
    assert_eq!(colour(0), [fr, fg_, fb, 255]);
    assert_eq!(colour(1), [br, bgr, bb, 255]);
    // The block cursor over concealed text: a block of the palette's foreground.
    let mut cells = replay_in(b"\x1b[8mx\x1b[H", 4, 1, p).cells;
    draw_cursor(&mut cells, 4, 0, 0, &p);
    assert_eq!((fg(&cells[0]), bg(&cells[0])), (p.foreground, p.foreground));
    let mut cells = replay_in(format!("\x1b[8;48;2;{fr};{fg_};{fb}mx\x1b[H").as_bytes(), 4, 1, p).cells;
    draw_cursor(&mut cells, 4, 0, 0, &p);
    assert_eq!((fg(&cells[0]), bg(&cells[0])), (p.background, p.background));

    // --json: colours as the palette resolves them, and trailing blanks in
    // its background left out, while the old default background shows.
    let g = replay_in(format!("\x1b[31ma\x1b[m \x1b[48;2;{};{};{}m \x1b[m  ", old.0, old.1, old.2).as_bytes(), 6, 2, p);
    let json = grid_json(&g.cells, &g.marks, 6, 2, None, CursorShape::Block, p.background);
    let want = "{\"cols\":6,\"rows\":2,\"cursor\":null,\"lines\":[\n\
        [{\"col\":0,\"text\":\"a\",\"fg\":\"#0b65c7\",\"bg\":\"#010203\"},\
        {\"col\":1,\"text\":\" \",\"fg\":\"#faf0e6\",\"bg\":\"#010203\"},\
        {\"col\":2,\"text\":\" \",\"fg\":\"#faf0e6\",\"bg\":\"#111823\"}],\n\
        []\n]}\n";
    assert_eq!(json, want);
}
