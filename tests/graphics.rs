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
    println!("ok, {checked} kitty RGB/RGBA/PNG pixel checks over 5 sizes; native clipping, text layering, transparency and deletion");
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
