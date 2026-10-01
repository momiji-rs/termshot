//! Font checking and draw.c tests. These call draw_png through FFI; run them
//! with SANITIZE=1 ./test.sh to have ASan watch stb_truetype. Paths are
//! relative to the repo root, where test.sh runs.

use super::*;
use std::ffi::CString;

const FONT: &str = "third_party/jetbrains-mono/JetBrainsMono-Regular.ttf";

fn render(cells: &[Cell], cols: usize, rows: usize, font: &[u8], px: f64, out: &str) -> i32 {
    let out = CString::new(out).unwrap();
    unsafe { draw_png(cells.as_ptr(), cols as i32, rows as i32, font.as_ptr(), px, out.as_ptr(), 0) }
}

/// The mutation tests/fontfuzz used to find stb_truetype crashes, ported
/// exactly so its crashing seeds reproduce here.
fn mutate(font: &mut [u8], seed: u64) {
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

#[test]
fn draw_png_is_reentrant() {
    let font = font::load(FONT).unwrap();
    let cells = parse("\x1b[1mbold\x1b[0m ─╭╮ plain".as_bytes(), 20, 2);
    let outs: Vec<String> = (0..8).map(|i| format!("target/test/thread-{i}.png")).collect();
    std::thread::scope(|scope| {
        for out in &outs {
            let (font, cells) = (&font, &cells);
            scope.spawn(move || assert_eq!(render(cells, 20, 2, font, 48.0, out), 0));
        }
    });
    let first = fs::read(&outs[0]).unwrap();
    for out in &outs[1..] {
        assert!(fs::read(out).unwrap() == first, "{out} differs from {}", outs[0]);
    }
}
