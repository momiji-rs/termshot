//! End-to-end kitty checks against explicit expected pixels, independent of
//! the production parser and sampler. Run by test.sh; accepts a baseline CLI
//! path to demonstrate the pre-fix failure. PNG decoding is test-only stb.
use std::ffi::{c_char, CString};
use std::fs;
use std::process::Command;

extern "C" {
    fn png_read_rgba(path: *const c_char, width: *mut i32, height: *mut i32) -> *mut u8;
    fn png_read_free(pixels: *mut u8);
}
fn decode(path: &str) -> (usize, usize, Vec<u8>) {
    let name = CString::new(path).unwrap();
    let (mut w, mut h) = (0, 0);
    unsafe {
        let p = png_read_rgba(name.as_ptr(), &mut w, &mut h);
        assert!(!p.is_null(), "decode {path}");
        let data = std::slice::from_raw_parts(p, w as usize * h as usize * 4).to_vec();
        png_read_free(p);
        (w as usize, h as usize, data)
    }
}
fn render(
    bin: &str,
    name: &str,
    log: &[u8],
    px: &str,
    cols: usize,
    rows: usize,
) -> (usize, usize, Vec<u8>) {
    let input = format!("target/test/graphics-{name}.pty");
    let out = format!("target/test/graphics-{name}.png");
    fs::write(&input, log).unwrap();
    assert!(Command::new(bin)
        .args([
            "--cursor",
            "none",
            "--size",
            &format!("{cols}x{rows}"),
            "--px",
            px,
            &input,
            &out
        ])
        .status()
        .unwrap()
        .success());
    decode(&out)
}
const BG: [u8; 3] = [17, 24, 35];
const COLORS: [[u8; 3]; 4] = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
fn main() {
    let bin = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "./termshot".into());
    let mut checked = 0;
    for px in ["1", "9", "24", "47.5", "128"] {
        for (kind, alpha) in [
            ("rgb", 255u32),
            ("rgba", 128),
            ("png", 255),
            ("png-alpha", 128),
            // The same images compressed (o=z) by Python's zlib.
            ("rgb-z", 255),
            ("rgba-z", 128),
            ("png-z", 255),
            ("png-alpha-z", 128),
            // kitty-rgb compressed by src/deflate.c, cut across three chunks.
            ("rgb-z-chunks", 255),
        ] {
            let log = fs::read(format!("tests/fixtures/kitty-{kind}.pty")).unwrap();
            let (w, h, pixels) = render(&bin, &format!("{kind}-{px}"), &log, px, 6, 4);
            let (cw, ch) = (w / 6, h / 4);
            let side = (4 * cw).min(2 * ch);
            let (left, top) = ((4 * cw - side) / 2, (2 * ch - side) / 2);
            for y in 0..h {
                for x in 0..w {
                    let mut want = BG;
                    if x >= left && x < left + side && y >= top && y < top + side {
                        let color = COLORS[((y - top) * 2 / side) * 2 + (x - left) * 2 / side];
                        for c in 0..3 {
                            want[c] = ((u32::from(color[c]) * alpha
                                + u32::from(BG[c]) * (255 - alpha)
                                + 127)
                                / 255) as u8;
                        }
                    }
                    assert_eq!(
                        &pixels[(y * w + x) * 4..][..3],
                        &want,
                        "{kind} px={px} at ({x},{y})"
                    );
                    assert_eq!(pixels[(y * w + x) * 4 + 3], 255);
                    checked += 1;
                }
            }
        }
    }
    // Native pixel dimensions, cursor placement and right/bottom clipping.
    let log = b"\x1b[4;6H\x1b_Ga=T,f=24,s=2,v=2,C=1;/wAAAP8AAAD///8A\x1b\\";
    let (w, h, pixels) = render(&bin, "native", log, "1", 6, 4);
    let at = ((3 * (h / 4)) * w + 5 * (w / 6)) * 4;
    assert_eq!(&pixels[at..at + 3], &COLORS[0]);
    assert_eq!(&pixels[..3], &BG);
    // Images composite over text, and deletion restores exactly the text raster.
    let text = b"\x1b[31mAB\x1b[H";
    let mut log = text.to_vec();
    log.extend_from_slice(b"\x1b_Ga=T,f=24,s=1,v=1,c=2,r=1,C=1;/wAA\x1b\\");
    let (_, _, image) = render(&bin, "over-text", &log, "24", 2, 1);
    let (_, _, plain) = render(&bin, "text", text, "24", 2, 1);
    assert_ne!(image, plain);
    log.extend_from_slice(b"\x1b_Ga=d\x1b\\");
    let (_, _, deleted) = render(&bin, "deleted", &log, "24", 2, 1);
    assert_eq!(deleted, plain);
    // Transparent pixels leave text untouched, including glyph coverage.
    let mut transparent = text.to_vec();
    transparent.extend_from_slice(b"\x1b_Ga=T,s=1,v=1,c=2,r=1,C=1;/wAAAA==\x1b\\");
    let (_, _, actual) = render(&bin, "transparent", &transparent, "24", 2, 1);
    assert_eq!(actual, plain);
    scroll_regions(&bin);
    ascii_scroll(&bin);
    grid_outputs(&bin);
    cursor_shapes(&bin);
    stored_placements(&bin);
    sixel(&bin);
    println!("ok, {checked} kitty RGB/RGBA/PNG pixel checks over 5 sizes, plain and zlib-compressed; native clipping, text layering, transparency and deletion");
}

// Images stored once (a=t) and put (a=p) in several cells: each placement is
// the image's square centred in its cell, drawn by z-index, then by the order
// the images were made. A moved placement and a delete by column are checked
// over the whole raster.
fn stored_placements(bin: &str) {
    let mut log = b"\x1b_Ga=t,i=1,f=24,s=1,v=1;/wAA\x1b\\\x1b_Ga=t,i=2,f=24,s=1,v=1;AP8A\x1b\\".to_vec();
    for (at, put) in [
        ("1;1", "i=1"),
        ("2;4", "i=1"),
        ("1;6", "i=2,p=5"),
        ("2;6", "i=2,p=5"),
        ("1;1", "i=2"),
        ("2;1", "i=2"),
        ("2;1", "i=1,z=1"),
    ] {
        log.extend_from_slice(format!("\x1b[{at}H\x1b_Ga=p,{put},c=1,r=1,C=1\x1b\\").as_bytes());
    }
    log.extend_from_slice(b"\x1b_Ga=d,d=x,x=4\x1b\\");
    let (w, h, pixels) = render(bin, "stored", &log, "24", 6, 2);
    let (cw, ch) = (w / 6, h / 2);
    let side = cw.min(ch);
    let (left, top) = ((cw - side) / 2, (ch - side) / 2);
    // The colour on top in each (row, col): the move emptied (0, 5), d=x (1, 3).
    let shown = [((0, 0), COLORS[1]), ((1, 5), COLORS[1]), ((1, 0), COLORS[0])];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x % cw, y % ch);
            let inside = dx >= left && dx < left + side && dy >= top && dy < top + side;
            let want = shown
                .iter()
                .find(|(cell, _)| inside && *cell == (y / ch, x / cw))
                .map_or(BG, |&(_, color)| color);
            assert_eq!(&pixels[(y * w + x) * 4..][..3], &want, "stored placements at ({x},{y})");
        }
    }
    println!("ok, stored images put in several cells: draw order, a moved placement, delete by column");
}

// Underline and bar cursors (DECSCUSR), pixel by pixel: a solid rectangle an
// eighth of a cell wide in the default foreground, over images too, and
// nothing else changed.
fn cursor_shapes(bin: &str) {
    const FG: [u8; 3] = [219, 231, 247];
    const RED: [u8; 3] = [255, 0, 0];
    // A red image fills cell 1, which the cursor is on.
    let image = "\x1b[1;2H\x1b_Ga=T,f=24,s=1,v=1,c=1,r=1,C=1;/wAA\x1b\\";
    for px in ["9", "24", "47.5"] {
        for (shape, ps) in [("underline", 4), ("bar", 6)] {
            for with_image in [false, true] {
                let name = format!("graphics-cursor-{shape}-{px}-{with_image}");
                let (input, out) = (format!("target/test/{name}.pty"), format!("target/test/{name}.png"));
                let log = format!("\x1b[{ps} q{}\x1b[1;2H", if with_image { image } else { "" });
                fs::write(&input, log).unwrap();
                let ok = Command::new(bin).args(["--size", "3x1", "--px", px, &input, &out]).status().unwrap().success();
                assert!(ok, "{name}");
                let (w, h, pixels) = decode(&out);
                let cw = w / 3;
                let thick = (cw / 8).max(1);
                // The square image, centred in the cell as the kitty checks above have it.
                let side = cw.min(h);
                let (left, top) = (cw + (cw - side) / 2, (h - side) / 2);
                for y in 0..h {
                    for x in 0..w {
                        let in_cell = (cw..2 * cw).contains(&x);
                        let mark = in_cell && if shape == "bar" { x < cw + thick } else { y >= h - thick };
                        let red = with_image && (left..left + side).contains(&x) && (top..top + side).contains(&y);
                        let want = if mark { FG } else if red { RED } else { BG };
                        assert_eq!(&pixels[(y * w + x) * 4..][..3], &want, "{name} at ({x},{y})");
                    }
                }
            }
        }
    }
    println!("ok, underline and bar cursors, over the background and over an image");
}

// Compare the final PNG with an independent raster-row scroll oracle. The
// initial image crosses both margins; this catches damage outside the region.
fn scroll_regions(bin: &str) {
    for (case, initial) in [
        (
            "crossing",
            b"\x1b[2;2H\x1b_Ga=T,f=24,s=2,v=2,c=10,r=6,C=1;/wAAAP8AAAD///8A\x1b\\".as_slice(),
        ),
        (
            "contained",
            b"\x1b[4;2H\x1b_Ga=T,f=24,s=2,v=2,c=4,r=2,C=1;/wAAAP8AAAD///8A\x1b\\".as_slice(),
        ),
    ] {
        let (w, h, before) = render(bin, "scroll-source", initial, "24", 12, 8);
        let ch = h / 8;
        for (name, control, top, bottom, delta) in [
            ("up", "\x1b[S", 2, 6, -1isize),
            ("down", "\x1b[T", 2, 6, 1),
            ("clear-region", "\x1b[99S", 2, 6, -4),
            ("delete-line", "\x1b[4;1H\x1b[M", 3, 6, -1),
            ("insert-line", "\x1b[4;1H\x1b[L", 3, 6, 1),
            ("index", "\x1b[6;1H\x1bD", 2, 6, -1),
            ("reverse-index", "\x1b[3;1H\x1bM", 2, 6, 1),
        ] {
            let mut log = initial.to_vec();
            log.extend_from_slice(b"\x1b[3;6r");
            log.extend_from_slice(control.as_bytes());
            let (_, _, actual) = render(bin, &format!("scroll-{case}-{name}"), &log, "24", 12, 8);
            let mut expected = before.clone();
            for y in top * ch..bottom * ch {
                if case == "crossing" {
                    continue;
                }
                let source = y as isize - delta * ch as isize;
                for x in 0..w {
                    let at = (y * w + x) * 4;
                    if source >= (top * ch) as isize && source < (bottom * ch) as isize {
                        let src = (source as usize * w + x) * 4;
                        expected[at..at + 4].copy_from_slice(&before[src..src + 4]);
                    } else {
                        expected[at..at + 4].copy_from_slice(&[BG[0], BG[1], BG[2], 255]);
                    }
                }
            }
            for (i, (got, want)) in actual
                .chunks_exact(4)
                .zip(expected.chunks_exact(4))
                .enumerate()
            {
                assert_eq!(got, want, "scroll {name} at ({}, {})", i % w, i / w);
            }
        }
    }
    println!(
        "ok, 14 scroll-region pixel oracles (crossing/contained, both directions, IL/DL, IND/RI)"
    );
}

fn ascii_scroll(bin: &str) {
    // Spaces have no foreground pixels, so an explicit SU command
    // is an independent rendering oracle for the equivalent autowrap scroll.
    let image = b"\x1b[4;2H\x1b_Ga=T,f=24,s=2,v=2,c=4,r=2,C=1;/wAAAP8AAAD///8A\x1b\\";
    for (name, count) in [
        ("one-row", 12),
        ("skip-whole-regions", 12 * 16),
        ("skip-and-tail", 12 * 17 + 1),
    ] {
        let mut setup = image.to_vec();
        setup.extend_from_slice(b"\x1b[3;6r\x1b[6;12H "); // wrap pending at bottom margin
        let mut actual_log = setup.clone();
        actual_log.extend(std::iter::repeat(b' ').take(count));
        let mut expected_log = setup;
        expected_log.extend_from_slice(format!("\x1b[{}S", (count + 11) / 12).as_bytes());
        let (_, _, actual) = render(bin, &format!("ascii-{name}"), &actual_log, "24", 12, 8);
        let (_, _, expected) = render(
            bin,
            &format!("ascii-{name}-oracle"),
            &expected_log,
            "24",
            12,
            8,
        );
        for (i, (got, want)) in actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .enumerate()
        {
            assert_eq!(got, want, "ASCII {name} pixel {i}");
        }
    }
    println!("ok, 3 ASCII autowrap pixel comparisons (single-row, skipped regions, partial tail)");
}

fn grid_outputs(bin: &str) {
    let input = "target/test/graphics-grid.pty";
    // Native pixels advance by cell metrics, even when no PNG is requested.
    fs::write(input, b"\x1b_Ga=T,f=24,s=2,v=2;/wAAAP8AAAD///8A\x1b\\X").unwrap();
    for combined in [false, true] {
        let stem = if combined { "combined" } else { "grid-only" };
        let text = format!("target/test/graphics-{stem}.txt");
        let json = format!("target/test/graphics-{stem}.json");
        let png = format!("target/test/graphics-{stem}.png");
        let mut cmd = Command::new(bin);
        cmd.args([
            "--size", "6x4", "--px", "24", "--text", &text, "--json", &json, input,
        ]);
        if combined {
            cmd.arg(png);
        }
        assert!(cmd.status().unwrap().success());
        assert_eq!(fs::read_to_string(text).unwrap(), "\n X\n\n\n");
        let data = fs::read_to_string(json).unwrap();
        assert!(data.contains("\"cursor\":{\"col\":2,\"row\":1,\"shape\":\"block\"}"), "{data}");
    }
    for extension in ["txt", "json"] {
        assert_eq!(
            fs::read(format!("target/test/graphics-grid-only.{extension}")).unwrap(),
            fs::read(format!("target/test/graphics-combined.{extension}")).unwrap()
        );
    }
    println!("ok, graphics text/JSON outputs match with and without PNG, including cursor metrics");
}

// Sixel images (DCS q), pixel by pixel. Every expectation comes from the
// sixel text itself: its colours in percent, rounded half up to 8 bits, and
// where its bands put them.
fn sixel(bin: &str) {
    let mut checked = 0;
    // Four 4x6 quadrants, RGB, as an 8x12 image at row 2, column 3.
    let quadrants = "\x1b[2;3H\x1bPq\"1;1;8;12#1;2;100;0;0#2;2;0;100;0#3;2;0;0;100#4;2;100;100;0\
                     #1!4~#2!4~-#3!4~#4!4~\x1b\\";
    for px in ["9", "24", "47.5", "128"] {
        let (w, h, pixels) = render(bin, &format!("sixel-quadrants-{px}"), quadrants.as_bytes(), px, 6, 4);
        let (cw, ch) = (w / 6, h / 4);
        let (left, top) = (2 * cw, ch);
        for y in 0..h {
            for x in 0..w {
                let inside = (left..left + 8).contains(&x) && (top..top + 12).contains(&y);
                let want = if inside { COLORS[(y - top) / 6 * 2 + (x - left) / 4] } else { BG };
                assert_eq!(&pixels[(y * w + x) * 4..][..3], &want, "sixel px={px} at ({x},{y})");
                checked += 1;
            }
        }
    }
    // HLS (DEC hues: 0 blue, 120 red, 240 green) and the VT340's default
    // registers 11 and 7 (33/60/33 % and 53 %); P2 0 paints the rest of the
    // declared area with register 0, black.
    let hls = b"\x1bPq\"1;1;6;1#1;1;0;50;100@#2;1;120;50;100@#3;1;240;50;100@#11@#7@\x1b\\";
    let (w, _, pixels) = render(bin, "sixel-hls", hls, "24", 1, 1);
    let want: [[u8; 3]; 6] = [[0, 0, 255], [255, 0, 0], [0, 255, 0], [84, 153, 84], [135, 135, 135], [0, 0, 0]];
    for (x, want) in want.iter().enumerate() {
        assert_eq!(&pixels[x * 4..][..3], want, "sixel colour {x}");
    }
    assert_eq!(&pixels[6 * 4..][..3], &BG);
    assert_eq!(&pixels[w * 4..][..3], &BG);
    // A transparent (P2 1) image with nothing set leaves the text exactly.
    let text = b"\x1b[31mAB\x1b[H";
    let mut log = text.to_vec();
    log.extend_from_slice(b"\x1bP0;1q\"1;1;40;40\x1b\\");
    let (_, _, actual) = render(bin, "sixel-transparent", &log, "24", 2, 2);
    let (_, _, plain) = render(bin, "sixel-text", text, "24", 2, 2);
    assert_eq!(actual, plain);
    // An image passing the bottom scrolls first, like SU before the same
    // image two rows higher; the cursor ends on its last row either way.
    let image = format!("\x1bP0;1q#1;2;100;0;0{}\x1b\\", ["!30~"; 12].join("-"));
    let setup = "line 1\r\nline 2\r\nline 3\r\nline 4";
    let scrolled = format!("{setup}\x1b[4;3H{image}X");
    let oracle = format!("{setup}\x1b[2S\x1b[2;3H{image}X");
    let (_, _, actual) = render(bin, "sixel-scroll", scrolled.as_bytes(), "24", 8, 4);
    let (_, _, expected) = render(bin, "sixel-scroll-oracle", oracle.as_bytes(), "24", 8, 4);
    assert!(actual == expected, "a Sixel image past the bottom scrolls as SU does");
    // ImageMagick's encoding of an image whose top 11 rows are four solid
    // bars, 10 pixels each, in the registers it defines first.
    let log = fs::read("tests/fixtures/sixel-magick.pty").unwrap();
    let (w, h, pixels) = render(bin, "sixel-magick", &log, "24", 40, 4);
    let top = h / 4;
    let bars = [[252, 0, 3], [0, 199, 0], [3, 3, 252], [252, 252, 3]];
    for y in top..top + 11 {
        for x in 0..40 {
            assert_eq!(&pixels[(y * w + x) * 4..][..3], &bars[x / 10], "magick bars at ({x},{y})");
            checked += 1;
        }
    }
    // Text and JSON need the cell height too: 30 pixels at px 24 are two rows.
    let input = "target/test/graphics-sixel-grid.pty";
    fs::write(input, b"\x1bPq!2~-!2~-!2~-!2~-!2~\x1b\\X").unwrap();
    let json = "target/test/graphics-sixel-grid.json";
    let ok = Command::new(bin).args(["--size", "6x4", "--px", "24", "--json", json, input]).status().unwrap();
    assert!(ok.success());
    let data = fs::read_to_string(json).unwrap();
    assert!(data.contains("\"cursor\":{\"col\":1,\"row\":1,\"shape\":\"block\"}"), "{data}");
    println!("ok, {checked} Sixel pixel checks: RGB over 4 sizes, HLS and VT340 colours, transparency, scrolling, ImageMagick output, grid metrics");
}
