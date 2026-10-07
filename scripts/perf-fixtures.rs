//! Regenerate the committed benchmark logs in tests/perf/ (docs/performance.md).
//!
//!     scripts/perf-fixtures.sh [--check]
//!
//! The output is deterministic: running this again must leave `git status`
//! clean. --check compares instead of writing (test.sh runs it).
#![allow(dead_code)]

#[path = "../src/unicode_tables.rs"]
mod unicode_tables;
#[path = "../src/unicode.rs"]
mod unicode;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::exit;

const OUT: &str = "tests/perf";
const JETBRAINS: &str = "third_party/jetbrains-mono/JetBrainsMono-Regular.ttf";
/// The 22 Han characters NotoSansCJKtc-Subset.otf maps (third_party/noto-sans-cjk/README.md).
const SUBSET_HAN: &str = "骨直角永東京台灣測試字型漢字中文繁體簡體龍鬱鑿齉";
const COLS: usize = 100;
const ROWS: usize = 30;
const CRLF: &str = "\r\n";

fn u16_at(data: &[u8], at: usize) -> usize {
    usize::from(u16::from_be_bytes([data[at], data[at + 1]]))
}

fn u32_at(data: &[u8], at: usize) -> usize {
    u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize
}

/// Every code point the font's Unicode cmap (format 4 or 12) maps to a glyph.
fn cmap(path: &str) -> BTreeSet<u32> {
    let data = fs::read(path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
    let count = u16_at(&data, 4);
    let base = (0..count)
        .find(|i| &data[12 + 16 * i..16 + 16 * i] == b"cmap")
        .map(|i| u32_at(&data, 20 + 16 * i))
        .unwrap_or_else(|| fail(&format!("{path}: no cmap")));
    let mut found = BTreeSet::new();
    for i in 0..u16_at(&data, base + 2) {
        let record = base + 4 + 8 * i;
        let (platform, encoding) = (u16_at(&data, record), u16_at(&data, record + 2));
        if !matches!((platform, encoding), (0, 3) | (0, 4) | (3, 1) | (3, 10)) {
            continue;
        }
        let sub = base + u32_at(&data, record + 4);
        match u16_at(&data, sub) {
            4 => {
                let segs = u16_at(&data, sub + 6) / 2;
                let range_at = sub + 16 + 6 * segs;
                for k in 0..segs {
                    let end = u16_at(&data, sub + 14 + 2 * k);
                    let start = u16_at(&data, sub + 16 + 2 * segs + 2 * k);
                    let delta = u16_at(&data, sub + 16 + 4 * segs + 2 * k);
                    let range = u16_at(&data, range_at + 2 * k);
                    for cp in start..=end {
                        if cp == 0xFFFF {
                            continue;
                        }
                        let glyph = if range == 0 {
                            (cp + delta) & 0xFFFF
                        } else {
                            match u16_at(&data, range_at + 2 * k + range + 2 * (cp - start)) {
                                0 => 0,
                                glyph => (glyph + delta) & 0xFFFF,
                            }
                        };
                        if glyph != 0 {
                            found.insert(cp as u32);
                        }
                    }
                }
            }
            12 => {
                for k in 0..u32_at(&data, sub + 12) {
                    let group = sub + 16 + 12 * k;
                    found.extend(u32_at(&data, group) as u32..=u32_at(&data, group + 4) as u32);
                }
            }
            _ => {}
        }
    }
    found
}

/// A letter, number, punctuation or symbol (general category L, N, P or S),
/// narrow and not combining, among the code points a font can map: what
/// termshot draws in one cell (unicode::width 1), less the space separators
/// (Zs), the soft hyphen (Cf) and private use (Co), which termshot also
/// draws in one cell. Python's unicodedata (16.0.0) picked the same 1116 of
/// JetBrains Mono's 1155 candidates when this was ported (2026-10-07).
fn drawn_narrow(cp: u32) -> bool {
    const SPACES: [u32; 6] = [0x20, 0xA0, 0x1680, 0x202F, 0x205F, 0x3000];
    unicode::width(cp) == 1
        && !SPACES.contains(&cp)
        && !(0x2000..=0x200A).contains(&cp)
        && cp != 0xAD
        && !(0xE000..=0xF8FF).contains(&cp)
}

/// More distinct drawn glyphs than draw.c's 1,024 cache slots, repeated.
///
/// Every code point is one JetBrains Mono maps, narrow, printable, and not
/// U+2500-U+259F (painted as geometry, so never cached), in the BMP (the few
/// beyond it map to empty glyphs). The screen holds the list in order, then
/// again from the start, so a second use finds its slot taken by a later
/// code point with the same key modulo 1,024.
fn glyph_overflow() -> (String, usize) {
    let cps: Vec<char> = cmap(JETBRAINS)
        .into_iter()
        .filter(|&cp| 0x20 < cp && cp <= 0xFFFF && !(0x2500..=0x259F).contains(&cp) && !(0x7F..0xA0).contains(&cp))
        .filter(|&cp| drawn_narrow(cp))
        .filter_map(char::from_u32)
        .collect();
    if cps.len() <= 1024 {
        fail(&format!("only {} usable codepoints, want more than 1,024", cps.len()));
    }
    let rows: Vec<String> =
        (0..ROWS).map(|r| (r * COLS..(r + 1) * COLS).map(|i| cps[i % cps.len()]).collect()).collect();
    (rows.join(CRLF), cps.len())
}

/// A full screen of the subset's Han characters with ASCII labels: every
/// Han cell is a fallback glyph when the subset (or full Noto CJK) is the
/// fallback, and a missing-glyph box without one.
fn cjk_dense() -> String {
    let han: Vec<char> = SUBSET_HAN.chars().collect();
    let rows: Vec<String> = (0..ROWS)
        .map(|r| {
            let label = format!("{r:02} ");
            let text: String = (0..(COLS - label.len()) / 2).map(|k| han[(r * 7 + k) % han.len()]).collect();
            format!("\x1b[3{}m{label}\x1b[0m{text}", 1 + r % 7)
        })
        .collect();
    rows.join(CRLF)
}

/// 1,500 distinct Han characters (U+4E00 onward), more than the cache's
/// 1,024 slots, for a full CJK fallback font; all are in Noto Sans CJK TC.
fn cjk_overflow() -> String {
    let per_row = COLS / 2;
    let rows: Vec<String> = (0..ROWS)
        .map(|r| (0..per_row).filter_map(|k| char::from_u32((0x4E00 + r * per_row + k) as u32)).collect())
        .collect();
    rows.join(CRLF)
}

/// What a CI log in several languages looks like: a prompt, ASCII output,
/// Latin, Greek and Cyrillic (in JetBrains Mono), Han (from the fallback),
/// Hiragana, Hangul and an emoji (in neither font: missing-glyph boxes), box
/// drawing, colours, bold and italic.
fn mixed_script() -> String {
    let lines = [
        "\x1b[1;32muser@host\x1b[0m:\x1b[1;34m~/src/termshot\x1b[0m$ make test LANG=zh_TW.UTF-8",
        "\x1b[2m[build]\x1b[0m cc -O2 -ffp-contract=off -c src/draw.c -o draw.o",
        "Résumé naïve façade — coöperate über Ærøskøbing; Øresund ﬁnal “quotes” ‘ok’",
        "Ελληνικά: Γειά σου κόσμε · Русский: Привет, мир · Українська: Добрий день",
        "\x1b[33m警告\x1b[0m：\x1b[3m測試字型\x1b[0m 東京 台灣 漢字 中文 繁體 簡體 — 骨直角永 龍鬱鑿齉",
        "日本語のテキスト ひらがな カタカナ · 한국어 텍스트 · emoji 🙂 🚀 ✅ (not in either font)",
        "╭──────────────┬──────────────────────────────╮",
        "│ \x1b[1mstage\x1b[0m        │ \x1b[1mresult\x1b[0m                       │",
        "├──────────────┼──────────────────────────────┤",
        "│ parse        │ \x1b[32m✓ ok\x1b[0m 中文 測試                 │",
        "│ render       │ \x1b[31m✗ fail\x1b[0m Ελληνικά Привет         │",
        "╰──────────────┴──────────────────────────────╯",
        "\x1b[38;2;255;128;0mtruecolor\x1b[0m \x1b[48;5;24m 256-colour \x1b[0m ░▒▓█ ▁▂▃▄▅▆▇█ ←↑→↓ ∀∂∈∑√∞≠≤≥",
    ];
    let rows: Vec<&str> = (0..ROWS).map(|r| lines[r % lines.len()]).collect();
    rows.join(CRLF)
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let check = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => false,
        ["--check"] => true,
        ["-h" | "--help"] => {
            println!("usage: scripts/perf-fixtures.sh [--check]\n\n--check  fail if a committed log differs");
            return;
        }
        _ => {
            eprintln!("usage: scripts/perf-fixtures.sh [--check]");
            exit(2);
        }
    };
    let (overflow, distinct) = glyph_overflow();
    let logs = [
        ("glyph-overflow.pty", overflow),
        ("cjk-dense.pty", cjk_dense()),
        ("cjk-overflow.pty", cjk_overflow()),
        ("mixed-script.pty", mixed_script()),
    ];
    fs::create_dir_all(OUT).unwrap_or_else(|e| fail(&format!("{OUT}: {e}")));
    let mut stale = Vec::new();
    for (name, text) in &logs {
        let path = Path::new(OUT).join(name);
        if check {
            if fs::read(&path).ok().as_deref() != Some(text.as_bytes()) {
                stale.push(*name);
            }
        } else {
            fs::write(&path, text).unwrap_or_else(|e| fail(&format!("{}: {e}", path.display())));
        }
    }
    if !stale.is_empty() {
        fail(&format!("stale: {}", stale.join(", ")));
    }
    println!("glyph-overflow: {distinct} distinct codepoints over {} cells", COLS * ROWS);
}
