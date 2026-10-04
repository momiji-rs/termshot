//! The inputs and the timing of `bench/c-vs-rust/run.sh glyphs` (#12 step
//! 2c), which renders them with the CLI built from draw.c's glyph painting
//! (the C, before the step) and with the CLI built now (src/glyphs.rs), and
//! compares the PNGs and what each says on stderr byte for byte. No crates,
//! and no Python.
//!
//!     glyphs logs DIR [FONT...]
//!         Writes PTY logs to DIR, all for a 100x30 screen:
//!         - cover-FONT-NN-STYLE.pty: every code point each FONT's cmap
//!           covers, 1,000 to a screen, upright, italic, and bold italic
//!           underlined, so every glyph of the vendored fonts is drawn each
//!           way (combining marks among them land on the code point before);
//!         - random-NN.pty: random cells, a seeded xorshift's, a third of
//!           them from what the FONTs cover and the rest from: ASCII,
//!           Latin-1, Greek and Cyrillic, box drawing and blocks, CJK and
//!           Hangul (wide, some at the last column), fullwidth forms, emoji
//!           and private use (which no font here has), U+16910 (an empty
//!           glyph in the built-in font), the blank separators and Braille,
//!           default ignorables and the Hangul fillers, Hebrew and Arabic,
//!           and up to five combining marks a cell (Latin, Thai, Hebrew,
//!           enclosing), under random SGR: bold, italic, underline, double
//!           underline, strike-through, reverse, 256 and RGB colours, with
//!           cursor moves that overwrite cells and halves of wide ones;
//!         - cjk-NN.pty: every code point of CJK Unified Ideographs,
//!           U+4E00..U+9FFF, 1,500 to a screen, so each font's whole CJK
//!           coverage is drawn, upright, then italic and bold on the next;
//!         - cjk-overflow.pty: 1,500 distinct ideographs on one screen,
//!           more than the 1,024 slots of the glyph cache, so slots
//!           conflict and evict; cjk-overflow-italic.pty, every other one
//!           italic, code points 1,024 apart, upright and italic of each;
//!         - marks.pty: each combining mark over each kind of base (narrow,
//!           wide, blank, missing, a space, box drawing), italic and bold,
//!           four marks deep, and marks after ignorables and fillers.
//!
//!     glyphs time ROUNDS OLD NEW LOG [ARGS...]
//!         Renders LOG ROUNDS times with each CLI, alternating, under
//!         TERMSHOT_PROFILE, and prints the median glyph_ms, blend_ms and
//!         foreground_ms of each and their ratio.

use std::fs;
use std::path::Path;
use std::process::Command;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }

    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u32) as usize]
    }
}

/// Ranges of base characters, and how often each is picked.
const BASES: &[(u32, u32, u32)] = &[
    (0x21, 0x7e, 30),     // ASCII
    (0xa1, 0xff, 6),      // Latin-1
    (0x391, 0x3c9, 3),    // Greek
    (0x410, 0x44f, 3),    // Cyrillic
    (0x2500, 0x259f, 6),  // box drawing and blocks
    (0x4e00, 0x9fff, 10), // CJK, wide
    (0xac00, 0xd7a3, 3),  // Hangul syllables, wide
    (0xff01, 0xff5e, 2),  // fullwidth forms, wide
    (0x1f600, 0x1f64f, 2), // emoji, wide, in no font here
    (0xe000, 0xe0ff, 1),  // private use
    (0x5d0, 0x5ea, 2),    // Hebrew
    (0x627, 0x64a, 2),    // Arabic
    (0x16910, 0x16910, 2), // an empty glyph in the built-in font
    (0x2800, 0x28ff, 1),  // Braille, U+2800 blank
    (0x2000, 0x200a, 1),  // blank separators
    (0x3000, 0x3000, 1),  // ideographic space, wide and blank
    (0x3164, 0x3164, 1),  // Hangul filler, wide and ignorable
    (0xffa0, 0xffa0, 1),  // halfwidth Hangul filler
    (0x115f, 0x115f, 1),  // Hangul choseong filler
];

/// Combining marks and other zero-width characters a cell keeps.
const MARKS: &[(u32, u32)] = &[
    (0x300, 0x36f),  // Latin
    (0xe31, 0xe31),  // Thai
    (0xe34, 0xe3a),
    (0xe47, 0xe4e),
    (0x5b0, 0x5bd),  // Hebrew points
    (0x20dd, 0x20e0), // enclosing
    (0x200d, 0x200d), // joiner, ignorable
    (0xfe0f, 0xfe0f), // variation selector, ignorable
];

fn push(out: &mut Vec<u8>, cp: u32) {
    let mut buf = [0; 4];
    out.extend_from_slice(char::from_u32(cp).unwrap().encode_utf8(&mut buf).as_bytes());
}

fn pick_range(rng: &mut Rng, ranges: &[(u32, u32)]) -> u32 {
    let &(lo, hi) = rng.pick(ranges);
    lo + rng.below(hi - lo + 1)
}

fn base(rng: &mut Rng) -> u32 {
    let total: u32 = BASES.iter().map(|b| b.2).sum();
    let mut at = rng.below(total);
    for &(lo, hi, weight) in BASES {
        if at < weight {
            return lo + rng.below(hi - lo + 1);
        }
        at -= weight;
    }
    unreachable!()
}

fn sgr(rng: &mut Rng, out: &mut Vec<u8>) {
    let mut params: Vec<String> = Vec::new();
    for _ in 0..1 + rng.below(3) {
        params.push(match rng.below(14) {
            0 => "0".into(),
            1 | 2 => "1".into(),
            3 | 4 => "3".into(),
            5 => "4".into(),
            6 => "21".into(),
            7 => "4:2".into(),
            8 => "9".into(),
            9 => "7".into(),
            10 => format!("38;5;{}", rng.below(256)),
            11 => format!("48;5;{}", rng.below(256)),
            12 => format!("38;2;{};{};{}", rng.below(256), rng.below(256), rng.below(256)),
            _ => format!("48;2;{};{};{}", rng.below(256), rng.below(256), rng.below(256)),
        });
    }
    out.extend_from_slice(format!("\x1b[{}m", params.join(";")).as_bytes());
}

fn random_log(seed: u64, covered: &[u32]) -> Vec<u8> {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0x2545_f491_4f6c_dd1d));
    let mut out = Vec::new();
    for _ in 0..3600 {
        match rng.below(40) {
            0..=3 => sgr(&mut rng, &mut out),
            4 => out.extend_from_slice(b"\r\n"),
            // Overwrite somewhere, a half of a wide character included.
            5 => out.extend_from_slice(format!("\x1b[{};{}H", 1 + rng.below(30), 1 + rng.below(100)).as_bytes()),
            6 => out.push(b' '),
            _ => {
                // A third from what the fonts cover, so CJK is drawn, not boxed.
                let cp = if !covered.is_empty() && rng.below(3) == 0 { *rng.pick(covered) } else { base(&mut rng) };
                push(&mut out, cp);
                // Marks, past the four a cell keeps now and then.
                if rng.below(5) == 0 {
                    for _ in 0..1 + rng.below(5) {
                        push(&mut out, pick_range(&mut rng, MARKS));
                    }
                }
            }
        }
    }
    out
}

/// Ideographs from `from`, `count` of them, a screen of 100 columns: 50 to
/// a row.
fn ideographs(from: u32, count: u32, sgr: &str) -> Vec<u8> {
    let mut out = format!("\x1b[H{sgr}").into_bytes();
    for k in 0..count {
        push(&mut out, from + k);
        if k % 50 == 49 && k + 1 < count {
            out.extend_from_slice(b"\r\n");
        }
    }
    out.extend_from_slice(b"\x1b[0m");
    out
}

fn out_italic(out: &mut Vec<u8>, italic: bool) {
    out.extend_from_slice(if italic { b"\x1b[3m" } else { b"\x1b[23m" });
}

fn marks_log() -> Vec<u8> {
    let bases = ['a' as u32, 'M' as u32, 'i' as u32, 0x4e2d, 0x3000, 0x1f600, ' ' as u32, 0x2502, 0x5d0, 0x16910];
    let mut marks = Vec::new();
    for &(lo, hi) in MARKS {
        marks.extend(lo..=hi);
    }
    let mut out = Vec::new();
    for (k, &m) in marks.iter().enumerate() {
        let b = bases[k % bases.len()];
        let attrs = ["0", "1", "3", "1;3", "4", "9"][k % 6];
        out.extend_from_slice(format!("\x1b[{attrs}m").as_bytes());
        push(&mut out, b);
        push(&mut out, m);
        if k % 3 == 0 {
            // Four deep, and a fifth that is dropped.
            for d in 1..5 {
                push(&mut out, marks[(k + d * 7) % marks.len()]);
            }
        }
        out.extend_from_slice(b"\x1b[0m ");
    }
    // Marks after an ignorable and on the Hangul fillers.
    for &b in &[0x200d, 0x3164, 0x115f, 0xffa0] {
        push(&mut out, 'x' as u32);
        push(&mut out, b);
        push(&mut out, 0x301);
        push(&mut out, 0xe49);
        out.push(b' ');
    }
    out
}

fn be16(d: &[u8], at: usize) -> u32 {
    u16::from_be_bytes([d[at], d[at + 1]]) as u32
}

fn be32(d: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]])
}

/// The code points a font's Unicode cmap subtable covers: format 12's
/// groups, or format 4's segments (some code points of a segment may map to
/// no glyph, which draws a box, as it should).
fn coverage(font: &[u8]) -> Vec<u32> {
    let tables = be16(font, 4) as usize;
    let cmap = (0..tables)
        .map(|i| 12 + 16 * i)
        .find(|&r| &font[r..r + 4] == b"cmap")
        .map(|r| be32(font, r + 8) as usize)
        .expect("no cmap");
    let mut best: Option<(u32, usize)> = None;
    for i in 0..be16(font, cmap + 2) as usize {
        let record = cmap + 4 + 8 * i;
        let (platform, encoding) = (be16(font, record), be16(font, record + 2));
        if platform == 0 || (platform == 3 && (encoding == 1 || encoding == 10)) {
            let at = cmap + be32(font, record + 4) as usize;
            let format = be16(font, at);
            if (format == 12 || format == 4) && best.map_or(true, |(f, _)| format > f) {
                best = Some((format, at));
            }
        }
    }
    let (format, at) = best.expect("no Unicode cmap subtable");
    let mut cps = Vec::new();
    if format == 12 {
        for g in 0..be32(font, at + 12) as usize {
            let group = at + 16 + 12 * g;
            cps.extend(be32(font, group)..=be32(font, group + 4));
        }
    } else {
        let segments = be16(font, at + 6) as usize / 2;
        for s in 0..segments {
            let (end, start) = (be16(font, at + 14 + 2 * s), be16(font, at + 16 + 2 * segments + 2 * s));
            if start != 0xffff {
                cps.extend(start..=end);
            }
        }
    }
    cps.retain(|&c| c >= 0x20 && !(0x7f..0xa0).contains(&c) && char::from_u32(c).is_some());
    cps.sort_unstable();
    cps.dedup();
    cps
}

/// Every code point a font covers, 1,000 to a screen in rows of 40 (so
/// wide ones fit), upright on one screen, then italic, then bold italic
/// underlined.
fn cover_logs(dir: &Path, name: &str, font: &[u8]) {
    let cps = coverage(font);
    for (k, chunk) in cps.chunks(1000).enumerate() {
        for (style, sgr) in [("upright", ""), ("italic", "\x1b[3m"), ("bold", "\x1b[1;3;4m")] {
            let mut out = format!("\x1b[H{sgr}").into_bytes();
            for (i, &cp) in chunk.iter().enumerate() {
                push(&mut out, cp);
                if i % 40 == 39 {
                    out.extend_from_slice(b"\r\n");
                }
            }
            fs::write(dir.join(format!("cover-{name}-{k:02}-{style}.pty")), out).unwrap();
        }
    }
}

fn logs(dir: &Path, fonts: &[String]) {
    fs::create_dir_all(dir).unwrap();
    let mut covered = Vec::new();
    for font in fonts {
        let name = Path::new(font).file_stem().unwrap().to_string_lossy().to_lowercase();
        let data = fs::read(font).unwrap();
        cover_logs(dir, &name, &data);
        covered.extend(coverage(&data));
    }
    for seed in 0..24 {
        fs::write(dir.join(format!("random-{seed:02}.pty")), random_log(seed, &covered)).unwrap();
    }
    let (first, last) = (0x4e00u32, 0x9fffu32);
    let mut screen = 0;
    let mut from = first;
    while from <= last {
        let count = 1500.min(last + 1 - from);
        // Upright, then the same italic and bold on the next screen, every
        // fourth.
        let sgr = if screen % 4 == 3 { "\x1b[1;3m" } else { "" };
        fs::write(dir.join(format!("cjk-{screen:02}.pty")), ideographs(from, count, sgr)).unwrap();
        from += count;
        screen += 1;
    }
    fs::write(dir.join("cjk-overflow.pty"), ideographs(0x6000, 1500, "")).unwrap();
    // Every other one italic, whose slot is half the cache away, and the
    // pairs of them repeated, upright and italic, from slots shared.
    let mut mixed = b"\x1b[H".to_vec();
    for k in 0..1500u32 {
        out_italic(&mut mixed, k % 2 == 1);
        push(&mut mixed, 0x6000 + (k / 2) * 2 + (k % 4) / 2 * 1024);
        if k % 50 == 49 && k < 1499 {
            mixed.extend_from_slice(b"\r\n");
        }
    }
    fs::write(dir.join("cjk-overflow-italic.pty"), mixed).unwrap();
    fs::write(dir.join("marks.pty"), marks_log()).unwrap();
}

/// The value of `key` in a termshot-profile line.
fn field(stderr: &str, key: &str) -> Option<f64> {
    let line = stderr.lines().find(|l| l.starts_with("termshot-profile {") && l.contains("\"glyph_ms\""))?;
    let at = line.find(&format!("\"{key}\":"))? + key.len() + 3;
    let end = line[at..].find(|c| c == ',' || c == '}')?;
    line[at..at + end].parse().ok()
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn time(rounds: usize, old: &str, new: &str, log: &str, args: &[String]) {
    const KEYS: [&str; 3] = ["glyph_ms", "blend_ms", "foreground_ms"];
    let mut samples = [[Vec::new(), Vec::new(), Vec::new()], [Vec::new(), Vec::new(), Vec::new()]];
    let out = std::env::temp_dir().join(format!("termshot-glyphs-time-{}.png", std::process::id()));
    for _ in 0..rounds {
        for (side, binary) in [old, new].iter().enumerate() {
            let run = Command::new(binary).args(args).arg(log).arg(&out).env("TERMSHOT_PROFILE", "1").output().unwrap();
            assert!(run.status.success(), "{binary} {args:?} {log} failed");
            let stderr = String::from_utf8_lossy(&run.stderr);
            for (k, key) in KEYS.iter().enumerate() {
                samples[side][k].push(field(&stderr, key).unwrap_or_else(|| panic!("no {key} from {binary}")));
            }
        }
    }
    let _ = fs::remove_file(&out);
    let name = Path::new(log).file_name().unwrap().to_string_lossy();
    let mut line = format!("{:<22} {:<28}", name, args.join(" "));
    for k in 0..KEYS.len() {
        let (c, r) = (median(&mut samples[0][k]), median(&mut samples[1][k]));
        line += &format!("  {} C {:7.3} Rust {:7.3} ({:.3})", KEYS[k].trim_end_matches("_ms"), c, r, r / c);
    }
    println!("{line}");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("logs") if args.len() >= 3 => logs(Path::new(&args[2]), &args[3..]),
        Some("time") if args.len() >= 6 => time(args[2].parse().unwrap(), &args[3], &args[4], &args[5], &args[6..]),
        _ => {
            eprintln!("usage: glyphs logs DIR [FONT...] | glyphs time ROUNDS OLD NEW LOG [ARGS...]");
            std::process::exit(2);
        }
    }
}
