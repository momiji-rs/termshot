//! End-to-end checks of --palette, --fg, --bg and --padding (#87) against
//! expected pixels worked out here, independently of the parser and the
//! painters: the margin, the cells, kitty's three layers over the configured
//! default background, Sixel, the cursor shapes, and the frame around what
//! the render draws without padding. Run by test.sh. PNG decoding is
//! test-only stb (tests/png_read.c).
use std::ffi::{c_char, CString};
use std::fs;
use std::process::Command;

extern "C" {
    fn png_read_rgba(path: *const c_char, width: *mut i32, height: *mut i32) -> *mut u8;
    fn png_read_free(pixels: *mut u8);
}

type Rgb = [u8; 3];

/// RGB pixels, a row at a time.
struct Image {
    w: usize,
    h: usize,
    rgb: Vec<Rgb>,
}

impl Image {
    fn at(&self, x: usize, y: usize) -> Rgb {
        self.rgb[y * self.w + x]
    }
}

fn decode(path: &str) -> Image {
    let name = CString::new(path).unwrap();
    let (mut w, mut h) = (0, 0);
    unsafe {
        let p = png_read_rgba(name.as_ptr(), &mut w, &mut h);
        assert!(!p.is_null(), "decode {path}");
        let rgba = std::slice::from_raw_parts(p, w as usize * h as usize * 4);
        let rgb = rgba.chunks_exact(4).map(|p| [p[0], p[1], p[2]]).collect();
        png_read_free(p);
        Image { w: w as usize, h: h as usize, rgb }
    }
}

const OUT: &str = "target/test";

fn run(bin: &str, name: &str, log: &[u8], args: &[&str]) -> Image {
    let (input, out) = (format!("{OUT}/pp-{name}.pty"), format!("{OUT}/pp-{name}.png"));
    fs::write(&input, log).unwrap();
    let output = Command::new(bin).args(args).args([&input, &out]).output().unwrap();
    assert!(output.status.success(), "{name}: {}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stderr.is_empty(), "{name} is not quiet: {}", String::from_utf8_lossy(&output.stderr));
    decode(&out)
}

/// The palette the checks use, every colour unlike the default's, as a file.
const FG: Rgb = [250, 200, 60];
const BG: Rgb = [30, 60, 90];
const RED: Rgb = [10, 160, 120];
const OLD_BG: Rgb = [17, 24, 35];
const OLD_FG: Rgb = [219, 231, 247];

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn named(n: usize) -> Rgb {
    match n {
        1 => RED,
        _ => [n as u8 * 13, 255 - n as u8 * 7, 100 + n as u8],
    }
}

/// The palette file: a theme's background and foreground, which --fg and
/// --bg replace, and the 16 named colours.
fn palette_file() -> String {
    let path = format!("{OUT}/pp-palette.conf");
    let mut file = "# a test theme\nforeground #000000\nbackground #ffffff\n".to_string();
    for n in 0..16 {
        file += &format!("color{n} {}\n", hex(named(n)));
    }
    fs::write(&path, file).unwrap();
    path
}

fn over(a: Rgb, alpha: u32, b: Rgb) -> Rgb {
    let mut out = [0; 3];
    for c in 0..3 {
        out[c] = ((u32::from(a[c]) * alpha + u32::from(b[c]) * (255 - alpha) + 127) / 255) as u8;
    }
    out
}

/// Where the cells are in a padded image of `cols` x `rows` cells: their
/// size, and whether (x, y) is in the margin, else its place in the cells.
struct Frame {
    pad: (usize, usize),
    cw: usize,
    ch: usize,
    cols: usize,
    rows: usize,
}

impl Frame {
    fn of(image: &Image, cols: usize, rows: usize, pad: (usize, usize)) -> Frame {
        let (gw, gh) = (image.w - 2 * pad.0, image.h - 2 * pad.1);
        assert_eq!((gw % cols, gh % rows), (0, 0), "{}x{} with padding {pad:?}", image.w, image.h);
        Frame { pad, cw: gw / cols, ch: gh / rows, cols, rows }
    }

    fn inside(&self, x: usize, y: usize) -> Option<(usize, usize)> {
        let (x, y) = (x.checked_sub(self.pad.0)?, y.checked_sub(self.pad.1)?);
        (x < self.cols * self.cw && y < self.rows * self.ch).then_some((x, y))
    }
}

const PADS: [(usize, usize); 4] = [(0, 0), (3, 5), (1, 0), (0, 2)];

fn pad_args(pad: (usize, usize)) -> String {
    format!("{},{}", pad.0, pad.1)
}

/// kitty's layers over a palette: an image below the backgrounds shows
/// through the cells whose background is the configured default (given or
/// set explicitly), not the old default nor a named colour; text, reverse
/// video and the cursors use the configured colours; the image is cut at the
/// cells' edge, never drawn into the margin.
fn layers(bin: &str, palette: &str) -> usize {
    const GREEN: Rgb = [0, 255, 0];
    let b = BG;
    let text = format!(
        "\x1b[H\u{2580}\x1b[41m \x1b[48;2;17;24;35m \x1b[48;2;{};{};{}m \x1b[0;7m \x1b[0m\u{2591} ",
        b[0], b[1], b[2]
    );
    let mut checked = 0;
    for (alpha, data) in [(255, "AP8A/w=="), (128, "AP8AgA==")] {
        for z in ["-1073741825", "-1", "0"] {
            for cursor in ["none", "block", "bar", "underline"] {
                let at = if cursor == "none" { "" } else { "\x1b[1;1H" };
                let shape = match cursor {
                    "bar" => "\x1b[6 q",
                    "underline" => "\x1b[4 q",
                    _ => "",
                };
                let log = format!("{shape}\x1b_Ga=T,s=1,v=1,c=7,z={z},C=1;{data}\x1b\\{text}{at}");
                for px in ["9", "24", "47.5"] {
                    for pad in PADS {
                        let name = format!("layers-{alpha}-{z}-{cursor}-{px}-{}x{}", pad.0, pad.1);
                        let pads = pad_args(pad);
                        let mut args = vec!["--size", "7x1", "--px", px, "--palette", palette, "--fg", "#fac83c",
                                            "--bg", "#1e3c5a", "--padding", &pads];
                        if cursor == "none" {
                            args.extend(["--cursor", "none"]);
                        }
                        let image = run(bin, &name, log.as_bytes(), &args);
                        let frame = Frame::of(&image, 7, 1, pad);
                        let (cw, ch) = (frame.cw, frame.ch);
                        let z: i64 = z.parse().unwrap();
                        let thick = (cw / 8).max(1);
                        for y in 0..image.h {
                            for x in 0..image.w {
                                let Some((x, y)) = frame.inside(x, y) else {
                                    assert_eq!(image.at(x, y), BG, "{name}: margin at ({x},{y})");
                                    checked += 1;
                                    continue;
                                };
                                let cell = x / cw;
                                // Cell 2 is the old default background, now a colour like any other.
                                let (mut fg, mut bg, mut opaque) = match cell {
                                    1 => (FG, RED, true),
                                    2 => (FG, OLD_BG, true),
                                    4 => (BG, FG, true),
                                    _ => (FG, BG, false),
                                };
                                if cursor == "block" && cell == 0 {
                                    (fg, bg, opaque) = (bg, fg, true);
                                }
                                let coverage = match cell {
                                    0 if y < ch / 2 => 4,
                                    5 => 1,
                                    _ => 0,
                                };
                                let mut want = bg;
                                if z < -1073741824 && !opaque || (-1073741824..0).contains(&z) {
                                    want = over(GREEN, alpha, want);
                                }
                                for c in 0..3 {
                                    want[c] = ((u32::from(fg[c]) * coverage + u32::from(want[c]) * (4 - coverage) + 2)
                                        / 4) as u8;
                                }
                                let mark = match cursor {
                                    "bar" => x < thick,
                                    "underline" => cell == 0 && y >= ch - thick,
                                    _ => false,
                                };
                                if mark {
                                    want = FG;
                                }
                                if z >= 0 {
                                    want = over(GREEN, alpha, want);
                                }
                                assert_eq!(image.at(x + pad.0, y + pad.1), want, "{name} at ({x},{y}) in the cells");
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    checked
}

/// Sixel over the configured background, moved by the margin: an image
/// larger than the screen, cut at the cells' right and bottom edges, never
/// drawn into the margin; and a small one at the top left with the bar
/// cursor beside it, in the configured foreground.
fn sixel(bin: &str, palette: &str) -> usize {
    let mut checked = 0;
    // 200 pixels wide and 120 high, red over green, with DECSDM, which puts
    // it at the top left corner and keeps it from scrolling the screen.
    let rows = |register: u8| vec![format!("#{register}!200~"); 10].join("-");
    let large = format!("\x1b[?80h\x1bPq#1;2;100;0;0#2;2;0;100;0{}-{}\x1b\\", rows(1), rows(2));
    let small = "\x1b[H\x1bPq#1;2;0;0;100!5~\x1b\\\x1b[6 q\x1b[1;3H".to_string();
    for (case, log, extent) in [("large", large, (200, 120)), ("small", small, (5, 6))] {
        for px in ["9", "24", "47.5"] {
            for pad in PADS {
                let name = format!("sixel-{case}-{px}-{}x{}", pad.0, pad.1);
                let pads = pad_args(pad);
                let mut args = vec!["--size", "3x2", "--px", px, "--palette", palette, "--bg", "#1e3c5a", "--fg",
                                    "#fac83c", "--padding", &pads];
                if case == "large" {
                    args.extend(["--cursor", "none"]);
                }
                let image = run(bin, &name, log.as_bytes(), &args);
                let frame = Frame::of(&image, 3, 2, pad);
                assert!(case == "small" || 3 * frame.cw < 200 && 2 * frame.ch < 120, "{name} is not cut");
                let thick = (frame.cw / 8).max(1);
                for y in 0..image.h {
                    for x in 0..image.w {
                        let Some((x, y)) = frame.inside(x, y) else {
                            assert_eq!(image.at(x, y), BG, "{name}: margin at ({x},{y})");
                            continue;
                        };
                        let mut want = BG;
                        if x < extent.0 && y < extent.1 {
                            want = match (case, y < 60) {
                                ("large", true) => [255, 0, 0],
                                ("large", false) => [0, 255, 0],
                                _ => [0, 0, 255],
                            };
                        }
                        if case == "small" && (2 * frame.cw..2 * frame.cw + thick).contains(&x) && y < frame.ch {
                            want = FG;
                        }
                        assert_eq!(image.at(x + pad.0, y + pad.1), want, "{name} at ({x},{y}) in the cells");
                        checked += 1;
                    }
                }
            }
        }
    }
    checked
}

/// Every named colour, as SGR 40-47 and 100-107 and as 48;5;n, then the cube,
/// a grey and a 24-bit colour, which the palette leaves alone; spaces, so
/// each cell is its background only.
fn colours(bin: &str, palette: &str) -> usize {
    let mut log = String::new();
    let mut want = Vec::new();
    for n in 0..16 {
        let sgr = if n < 8 { 40 + n } else { 100 + n - 8 };
        log += &format!("\x1b[{sgr}m \x1b[48;5;{n}m ");
        want.extend([named(n), named(n)]);
    }
    log += "\x1b[48;5;196m \x1b[48;5;244m \x1b[48;2;1;2;3m \x1b[49m \x1b[7m \x1b[m";
    want.extend([[255, 0, 0], [128, 128, 128], [1, 2, 3], BG, FG]);
    let cols = want.len() + 1;
    want.push(BG);
    let mut checked = 0;
    for px in ["9", "24"] {
        for pad in PADS {
            let name = format!("colours-{px}-{}x{}", pad.0, pad.1);
            let (size, pads) = (format!("{cols}x1"), pad_args(pad));
            let image = run(bin, &name, log.as_bytes(), &["--size", &size, "--px", px, "--cursor", "none", "--palette",
                                                          palette, "--fg", "#fac83c", "--bg", "#1e3c5a", "--padding",
                                                          &pads]);
            let frame = Frame::of(&image, cols, 1, pad);
            for y in 0..image.h {
                for x in 0..image.w {
                    let expected = frame.inside(x, y).map_or(BG, |(x, _)| want[x / frame.cw]);
                    assert_eq!(image.at(x, y), expected, "{name} at ({x},{y})");
                    checked += 1;
                }
            }
        }
    }
    // The file's own background and foreground, without --fg and --bg.
    let image = run(bin, "file-colours", b"\x1b[7m \x1b[m ", &["--size", "3x1", "--px", "9", "--cursor", "none",
                                                               "--palette", palette, "--padding", "2"]);
    let frame = Frame::of(&image, 3, 1, (2, 2));
    for y in 0..image.h {
        for x in 0..image.w {
            let expected = match frame.inside(x, y) {
                Some((x, _)) if x < frame.cw => [0, 0, 0],
                _ => [255, 255, 255],
            };
            assert_eq!(image.at(x, y), expected, "file-colours at ({x},{y})");
            checked += 1;
        }
    }
    checked
}

/// Padding is a frame around the very image the render makes without it:
/// glyphs that reach past their cells (italic, bold, a wide character
/// split by the edge or a block cursor on one), combining marks and box
/// drawing are cut where they were, and nothing else moves.
fn frames(bin: &str, palette: &str) -> usize {
    let logs: [(&str, &str, usize, usize); 4] = [
        ("wide-edges", "中ab\x1b[1;9H界\x1b[2;1H\x1b[41m国\x1b[m\x1b[2;9H\x1b[3;1m世\x1b[1;9H", 10, 2),
        ("italic-edges", "\x1b[3;1mWjfy\x1b[1;7Hgqf/\x1b[2;1H\x1b[3m_ \u{301}\x1b[2;8H\u{2571}\u{2572}\u{256D}", 10, 2),
        ("boxes", "\u{250C}\u{2500}\u{2510}\u{2588}\r\n\u{2514}\u{2500}\u{2518}\u{2591}\x1b[5 q", 4, 2),
        ("kitty", "\x1b[2;3H\x1b_Ga=T,f=24,s=2,v=2,c=5,r=3,C=1;/wAAAP8AAAD///8A\x1b\\ok", 6, 3),
    ];
    let mut checked = 0;
    for (name, log, cols, rows) in logs {
        for px in ["9", "24", "47.5"] {
            let size = format!("{cols}x{rows}");
            let base = ["--size", &size, "--px", px, "--palette", palette];
            let plain = run(bin, &format!("{name}-{px}"), log.as_bytes(), &base);
            for pad in [(3, 5), (1, 0), (0, 2), (17, 17)] {
                let pads = pad_args(pad);
                let mut args = base.to_vec();
                args.extend(["--padding", &pads]);
                let padded = run(bin, &format!("{name}-{px}-{}x{}", pad.0, pad.1), log.as_bytes(), &args);
                assert_eq!((padded.w, padded.h), (plain.w + 2 * pad.0, plain.h + 2 * pad.1), "{name} {px} {pad:?}");
                // The file's background, as no --bg is given.
                for y in 0..padded.h {
                    for x in 0..padded.w {
                        let inside = (pad.0..pad.0 + plain.w).contains(&x) && (pad.1..pad.1 + plain.h).contains(&y);
                        let want = if inside { plain.at(x - pad.0, y - pad.1) } else { [255, 255, 255] };
                        assert_eq!(padded.at(x, y), want, "{name} px={px} padding {pad:?} at ({x},{y})");
                        checked += 1;
                    }
                }
            }
        }
    }
    checked
}

/// Without the new options nothing changes: the default palette written
/// out, --fg and --bg of the default colours, and --padding 0 all draw the
/// PNG drawn without them.
fn defaults(bin: &str) {
    let path = format!("{OUT}/pp-default.conf");
    let mut file = format!("foreground {}\nbackground {}\n", hex(OLD_FG), hex(OLD_BG));
    let xterm = ["#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd", "#00cdcd", "#e5e5e5",
                 "#7f7f7f", "#ff0000", "#00ff00", "#ffff00", "#5c5cff", "#ff00ff", "#00ffff", "#ffffff"];
    for (n, c) in xterm.iter().enumerate() {
        file += &format!("color{n} {c}\n");
    }
    fs::write(&path, file).unwrap();
    let log = fs::read("tests/fixtures/random-colors.pty").unwrap();
    let base = ["--size", "40x12", "--px", "9"];
    let plain = run(bin, "defaults", &log, &base);
    for extra in [vec!["--palette", path.as_str()], vec!["--fg", "#dbe7f7", "--bg", "#111823"], vec!["--padding", "0"],
                  vec!["--padding", "0,0"]] {
        let mut args = base.to_vec();
        args.extend(&extra);
        let image = run(bin, "defaults-given", &log, &args);
        assert!(image.w == plain.w && image.h == plain.h && image.rgb == plain.rgb, "{extra:?} changes the PNG");
    }
}

fn main() {
    let bin = std::env::args().nth(1).unwrap_or_else(|| "./termshot".into());
    let palette = palette_file();
    defaults(&bin);
    let counts = [layers(&bin, &palette), sixel(&bin, &palette), colours(&bin, &palette), frames(&bin, &palette)];
    println!(
        "ok, palette and padding: {} kitty layer, {} Sixel and cursor, {} colour and {} frame pixel checks at 3 sizes \
         and 4 paddings; the defaults draw as before",
        counts[0], counts[1], counts[2], counts[3]
    );
}
