//! What the CLI says about a font it can't draw with. The fonts are copies of
//! the vendored one, patched here, so no check depends on system fonts.
//!
//!     rustc --edition 2021 tests/font_cli.rs -o font_cli && ./font_cli [./termshot]
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

const FONT: &str = "third_party/jetbrains-mono/JetBrainsMono-Regular.ttf";

/// The vendored font with its glyf table renamed to tag, as if it had only
/// that table in its place.
fn retagged(tag: &[u8; 4]) -> Vec<u8> {
    let mut font = fs::read(FONT).unwrap();
    let tables = u16::from_be_bytes([font[4], font[5]]) as usize;
    let record = (0..tables).map(|i| 12 + 16 * i).find(|&r| &font[r..r + 4] == b"glyf").unwrap();
    font[record..record + 4].copy_from_slice(tag);
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
    let bin = env::args().nth(1).unwrap_or_else(|| "./termshot".into());
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
        fs::write(&font, retagged(tag)).unwrap();
        for flag in ["--font", "--fallback-font"] {
            let _ = fs::remove_file(&png);
            let r = run(&bin, &[Path::new(flag), &font, stdin, &png], b"a");
            let want = format!("{}: not a usable TrueType font: {reason}", font.display());
            expect(&format!("{flag} {name}.ttf is refused: {want}"), r.code == Some(1) && r.stderr.contains(&want), &r);
            expect(&format!("{flag} {name}.ttf leaves no PNG"), !png.exists(), &r);
        }
    }

    if failed > 0 {
        std::process::exit(1);
    }
    println!("ok");
}
