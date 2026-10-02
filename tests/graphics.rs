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
    println!("ok, {checked} kitty RGB/RGBA/PNG pixel checks over 5 sizes; native clipping, text layering, transparency and deletion");
}
