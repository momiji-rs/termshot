//! What the CLI says about a font it can't draw with. The fonts are copies of
//! the vendored one, patched here or by tests/glyphs.c, so no check depends
//! on system fonts. hollow-A.ttf is the font tests/glyphs.c writes, with an
//! empty glyph for 'A'.
//!
//!     rustc --edition 2021 tests/font_cli.rs -o font_cli && ./font_cli ./termshot hollow-A.ttf
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

const FONT: &str = "third_party/jetbrains-mono/JetBrainsMono-Regular.ttf";

/// The font with its table from renamed to to.
fn retagged(mut font: Vec<u8>, from: &[u8; 4], to: &[u8; 4]) -> Vec<u8> {
    let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
    let record = (0..tables).map(|i| 12 + 16 * i).find(|&r| &font[r..r + 4] == from).unwrap();
    font[record..record + 4].copy_from_slice(to);
    font
}

struct Run {
    code: Option<i32>,
    stderr: String,
}

fn run(bin: &str, args: &[&Path], input: &[u8]) -> Run {
    use std::io::Write;
    let mut child = Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    Run { code: out.status.code(), stderr: String::from_utf8_lossy(&out.stderr).into_owned() }
}

fn main() {
    let mut args = env::args().skip(1);
    let (Some(bin), Some(hollow)) = (args.next(), args.next()) else {
        eprintln!("usage: font_cli <termshot> <hollow-A.ttf>");
        std::process::exit(2);
    };
    let hollow = PathBuf::from(hollow);
    let dir = env::temp_dir().join(format!("termshot-font-cli-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let png = dir.join("out.png");
    let stdin = Path::new("-");
    let mut failed = 0;
    let mut expect = |what: &str, ok: bool, run: &Run| {
        if !ok {
            println!("FAIL {what}: exit {:?}, stderr:\n{}", run.code, run.stderr);
            failed += 1;
        }
    };

    // A font without glyf is refused, for what it has instead, as either font.
    for (tag, reason) in [
        (b"CBDT", "a color bitmap font (CBDT) with no outlines; use a monochrome outline font, such as Noto Emoji"),
        (b"sbix", "a color bitmap font (sbix) with no outlines; use a monochrome outline font, such as Noto Emoji"),
        (b"CFF ", "no glyf table; CFF (PostScript) outlines are not supported"),
        (b"xxxx", "no glyf table"),
    ] {
        let name = String::from_utf8_lossy(tag).trim_end().to_owned();
        let font = dir.join(format!("{name}.ttf"));
        // As if the font had only that table in glyf's place.
        fs::write(&font, retagged(fs::read(FONT).unwrap(), b"glyf", tag)).unwrap();
        for flag in ["--font", "--fallback-font"] {
            let _ = fs::remove_file(&png);
            let r = run(&bin, &[Path::new(flag), &font, stdin, &png], b"a");
            let want = format!("{}: not a usable TrueType font: {reason}", font.display());
            expect(&format!("{flag} {name}.ttf is refused: {want}"), r.code == Some(1) && r.stderr.contains(&want), &r);
            expect(&format!("{flag} {name}.ttf leaves no PNG"), !png.exists(), &r);
        }
    }

    // A character a font maps to an empty glyph is drawn as a box, with a
    // warning naming it, the font and the fix. The image is still made.
    let plain = Path::new(FONT);
    let font = Path::new("--font");
    let fallback = Path::new("--fallback-font");
    let bamum = "\u{16910}"; // the vendored font maps it to an empty glyph
    let warned = |r: &Run, want: &str| r.code == Some(0) && r.stderr.starts_with("termshot: warning: ") && r.stderr.contains(want);
    let _ = fs::remove_file(&png);
    let r = run(&bin, &[stdin, &png], format!("a{bamum}b{bamum}").as_bytes());
    expect(
        "the built-in font's empty glyph is named, with its cell and count",
        warned(&r, "U+16910 at column 1, row 0 (from 0) is drawn as a box (the first of 2 such cells): \
            the built-in font maps it to an empty glyph; pass --fallback-font with an outline font that has it\n"),
        &r,
    );
    expect("the warning keeps the PNG", png.exists(), &r);
    let r = run(&bin, &[font, &hollow, stdin, &png], b"A");
    let named = format!("U+0041 at column 0, row 0 (from 0) is drawn as a box: --font {} maps it to an empty glyph", hollow.display());
    expect("--font's empty glyph is named", warned(&r, &named), &r);
    let r = run(&bin, &[font, &hollow, fallback, &hollow, stdin, &png], b"A");
    let both = format!(
        "--font {0} and --fallback-font {0} map it to empty glyphs; pass a --fallback-font with an outline for it\n",
        hollow.display()
    );
    expect("both fonts' empty glyphs are named", warned(&r, &both), &r);
    let color = dir.join("color.ttf");
    fs::write(&color, retagged(fs::read(&hollow).unwrap(), b"name", b"sbix")).unwrap();
    let r = run(&bin, &[font, &color, stdin, &png], b"A");
    expect(
        "a color bitmap font is named as one",
        warned(&r, "(a color bitmap font, sbix, which termshot cannot draw) maps it to an empty glyph; \
            pass --fallback-font with an outline font that has it, such as Noto Emoji\n"),
        &r,
    );
    // Silent when every character is drawn, a fallback drew it, the font
    // simply lacks it (plain tofu), or there is no PNG to draw.
    let text = dir.join("out.txt");
    for (what, args, input) in [
        ("a normal font", vec![stdin, &png], "ab─█".as_bytes()),
        ("--fallback-font drew it", vec![font, &hollow, fallback, plain, stdin, &png], b"A"),
        ("a character no font has", vec![stdin, &png], "中".as_bytes()),
        ("no PNG", vec![Path::new("--text"), &text, stdin], bamum.as_bytes()),
    ] {
        let r = run(&bin, &args, input);
        expect(&format!("silent for {what}"), r.code == Some(0) && r.stderr.is_empty(), &r);
    }

    if failed > 0 {
        std::process::exit(1);
    }
    println!("ok");
}
