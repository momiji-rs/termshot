//! Pixel goldens. Renders the samples and tests/fixtures/ with ./termshot,
//! decodes each PNG with tests/png_read.c (stb_image), and compares the sha256
//! of the RGBA pixels with tests/goldens.txt. Hashing pixels rather than file bytes keeps the
//! goldens valid when the encoder changes but the image does not. The same runs
//! write --text and --json, which are compared byte for byte with
//! tests/grids/<log>.txt and <log>.json.
//!
//! Built and run by test.sh from the repo root:
//!   golden             check
//!   golden --update    rewrite tests/goldens.txt and tests/grids/

use std::ffi::{c_char, CStr, CString};
use std::fs;
use std::process::{Command, ExitCode};

const GOLDENS: &str = "tests/goldens.txt";
const GRIDS: &str = "tests/grids";
const OUT: &str = "target/test";
const FONT: &str = "third_party/jetbrains-mono/JetBrainsMono-Regular.ttf";
const HEADER: &str = "# sha256 of decoded RGBA pixels, size, log, px. Rewrite with ./test.sh --update-goldens";
// (log, px, cols, rows). A log is examples/<log>.pty or tests/fixtures/<log>.pty.
// px 46 is sensitive to FMA contraction (macOS vs Linux, #3); px 48 is the README size.
const CASES: [(&str, &str, u32, u32); 28] = [
    ("reply-sent", "46", 100, 30),
    ("reply-sent", "48", 100, 30),
    ("draft-ready", "46", 100, 30),
    ("draft-ready", "48", 100, 30),
    ("kitty-scroll", "24", 12, 8),
    ("kitty-rgb", "24", 6, 4),
    ("kitty-rgb", "47.5", 6, 4),
    ("kitty-rgba", "24", 6, 4),
    ("kitty-png", "24", 6, 4),
    ("kitty-png-alpha", "24", 6, 4),
    ("blank", "24", 20, 8),
    ("geometry", "1", 12, 2),
    ("geometry", "9", 12, 2),
    ("geometry", "24", 12, 2),
    ("geometry", "47.5", 12, 2),
    ("geometry", "128", 12, 2),
    ("geometry", "255", 12, 2),
    ("clipping", "48", 5, 2),
    ("cache-collisions", "16", 120, 4),
    ("cache-collisions-1024", "16", 120, 4),
    ("geometry-offsets", "47.5", 200, 40),
    ("missing-glyphs", "16", 100, 1),
    ("csi", "20", 20, 5),
    ("sgr", "24", 40, 2),
    ("italic", "24", 30, 5),
    ("italic", "46", 30, 5),
    ("control-strings", "24", 20, 4),
    ("random-colors", "20", 40, 12),
];

extern "C" {
    fn png_read_rgba(path: *const c_char, width: *mut i32, height: *mut i32) -> *mut u8;
    fn png_read_error() -> *const c_char;
    fn png_read_free(pixels: *mut u8);
}

fn decode(path: &str) -> Result<(i32, i32, Vec<u8>), String> {
    let c_path = CString::new(path).map_err(|_| format!("{path}: nul in path"))?;
    let (mut width, mut height) = (0, 0);
    unsafe {
        let pixels = png_read_rgba(c_path.as_ptr(), &mut width, &mut height);
        if pixels.is_null() {
            let reason = CStr::from_ptr(png_read_error()).to_string_lossy();
            return Err(format!("{path}: {reason}"));
        }
        let rgba = std::slice::from_raw_parts(pixels, width as usize * height as usize * 4).to_vec();
        png_read_free(pixels);
        Ok((width, height, rgba))
    }
}

fn sha256(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (state, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *state = state.wrapping_add(value);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

/// The grid outputs: their option, and the extension of their files.
const GRID_FORMATS: [(&str, &str); 2] = [("--text", "txt"), ("--json", "json")];

/// A case's golden line, and its grid outputs in GRID_FORMATS order.
fn render(log: &str, px: &str, cols: u32, rows: u32) -> Result<(String, Vec<String>), String> {
    let png = format!("{OUT}/{log}-{px}.png");
    let grids = GRID_FORMATS.map(|(_, ext)| format!("{OUT}/{log}-{px}.{ext}"));
    let mut src = format!("examples/{log}.pty");
    if fs::metadata(&src).is_err() {
        src = format!("tests/fixtures/{log}.pty");
    }
    let output = Command::new("./termshot")
        .args(["--text", &grids[0], "--json", &grids[1]])
        .args([&src, &png, FONT, px, &cols.to_string(), &rows.to_string()])
        .env_remove("TERMSHOT_PROFILE")
        .output()
        .map_err(|error| format!("./termshot: {error}"))?;
    if !output.status.success() {
        return Err(format!("./termshot {log} {px}: {}", output.status));
    }
    if String::from_utf8_lossy(&output.stderr).contains("termshot-profile ") {
        return Err(format!("./termshot {log} {px}: profile records without TERMSHOT_PROFILE"));
    }
    let (width, height, rgba) = decode(&png)?;
    let grids = grids.iter().map(|path| fs::read_to_string(path).map_err(|error| format!("{path}: {error}")));
    Ok((format!("{} {width}x{height} {log} {px}", sha256(&rgba)), grids.collect::<Result<_, _>>()?))
}

/// Check each log's grid outputs against tests/grids/, or rewrite them. A
/// log's cases share a grid size, so they must agree on them.
fn check_grids(grids: &[(&str, Vec<String>)], update: bool) -> bool {
    let mut ok = true;
    for (i, (log, outputs)) in grids.iter().enumerate() {
        if let Some((_, first)) = grids[..i].iter().find(|(other, _)| other == log) {
            if first != outputs {
                println!("FAIL {log}: the grid outputs differ between its cases");
                ok = false;
            }
            continue;
        }
        for ((option, ext), output) in GRID_FORMATS.iter().zip(outputs) {
            let path = format!("{GRIDS}/{log}.{ext}");
            if update {
                if let Err(error) = fs::create_dir_all(GRIDS).and_then(|()| fs::write(&path, output)) {
                    eprintln!("{path}: {error}");
                    ok = false;
                }
            } else if fs::read_to_string(&path).ok().as_ref() != Some(output) {
                println!("FAIL {path} differs from {option}; see {OUT}/{log}-*.{ext}");
                ok = false;
            }
        }
    }
    ok
}

fn main() -> ExitCode {
    assert_eq!(sha256(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let update = std::env::args().nth(1).as_deref() == Some("--update");
    let mut fresh = vec![HEADER.to_string()];
    let mut grids = Vec::new();
    for (log, px, cols, rows) in CASES {
        match render(log, px, cols, rows) {
            Ok((line, outputs)) => {
                fresh.push(line);
                grids.push((log, outputs));
            }
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        }
    }
    let fresh = fresh.join("\n") + "\n";
    let grids_ok = check_grids(&grids, update);
    if update {
        if let Err(error) = fs::write(GOLDENS, &fresh) {
            eprintln!("{GOLDENS}: {error}");
            return ExitCode::FAILURE;
        }
        if !grids_ok {
            return ExitCode::FAILURE;
        }
        println!("updated {GOLDENS} and {GRIDS}/");
        return ExitCode::SUCCESS;
    }
    let want = fs::read_to_string(GOLDENS).unwrap_or_default();
    if want == fresh {
        if !grids_ok {
            return ExitCode::FAILURE;
        }
        println!("ok");
        return ExitCode::SUCCESS;
    }
    let want_lines: Vec<&str> = want.lines().collect();
    for line in fresh.lines() {
        if !want_lines.contains(&line) {
            println!("got  {line}");
        }
    }
    for line in &want_lines {
        if !fresh.lines().any(|l| l == *line) {
            println!("want {line}");
        }
    }
    println!("FAIL pixels changed; renders are in {OUT}/");
    ExitCode::FAILURE
}
