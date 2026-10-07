//! Dependency-free, interleaved CLI benchmark. JSON is the durable result,
//! slim by default (--full-profile keeps every per-run profile record).
//!
//! Every round runs each binary twice, without and with TERMSHOT_PROFILE, in a
//! shuffled order, so wall/CPU samples, profile records and the profiling
//! overhead all come from the same rounds. docs/performance.md describes the
//! workloads, the timer boundaries and how to read the result.
//!
//! Run it through scripts/bench.sh, which builds it. It is the Rust port of
//! the Python bench.py every report in docs/ before 2026-10-07 came from, and
//! reproduces it exactly: the workload bytes, the random run order, the
//! statistics and the JSON (docs/performance.md, "The harness"). Python's
//! random.Random, statistics and json live in scripts/bench_common.rs.
//!
//! TERMSHOT_BENCH_FAKE_CLOCK=1 replaces the clock, the CPU times, the load
//! averages and the timestamp with fixed sequences, so a run against a fake
//! binary gives the same JSON every time (test.sh checks one).

#[path = "bench_common.rs"]
mod common;

use common::*;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

/// println!, flushed (bench.py printed with flush=True), quietly ending the
/// run when stdout is gone (a closed pipe) instead of panicking.
macro_rules! say {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let mut stdout = std::io::stdout().lock();
        if writeln!(stdout, $($arg)*).and_then(|_| stdout.flush()).is_err() {
            std::process::exit(1);
        }
    }};
}

const PROG: &str = "bench.sh";

const USAGE: &str = "usage: bench.sh [-h] [--binary BINARY] [--describe DESCRIBE] [--runs RUNS]
                [--warmups WARMUPS] --output OUTPUT [--full-profile]
                [--stage STAGE] [--slim REPORT] [--case CASE]
                [--suite {all,legacy,fonts,draw,parser,text}]
                [--cjk-font CJK_FONT] [--memory-runs MEMORY_RUNS]
                [--cold-runs COLD_RUNS] [--cold-case COLD_CASE]
                [--verify-identical] [--reference REFERENCE]
                [--unchecked UNCHECKED] [--seed SEED] [--list-workloads]
";

const HELP_TAIL: &str = "
Dependency-free, interleaved CLI benchmark. JSON is the durable result, slim
by default (--full-profile keeps every per-run profile record). Every round
runs each binary twice, without and with TERMSHOT_PROFILE, in a shuffled
order, so wall/CPU samples, profile records and the profiling overhead all
come from the same rounds. docs/performance.md describes the workloads, the
timer boundaries and how to read the result.

options:
  -h, --help            show this help message and exit
  --binary BINARY       label=/path/to/binary (repeatable)
  --describe DESCRIBE   label=text, e.g. the revision a binary is built from
  --runs RUNS
  --warmups WARMUPS
  --output OUTPUT
  --full-profile        keep every per-run TERMSHOT_PROFILE stage and counter
                        (the result is about 10x larger)
  --stage STAGE         a profile key to keep per run in the slim result, e.g.
                        parse_ms (repeatable)
  --slim REPORT         write the slim form of an existing full report to
                        --output, keeping --stage keys, and exit
  --case CASE
  --suite {all,legacy,fonts,draw,parser,text}
  --cjk-font CJK_FONT   full NotoSansCJK-Regular.ttc for the *-full cases
                        (default: /usr/share/fonts/noto-cjk/NotoSansCJK-
                        Regular.ttc if present)
  --memory-runs MEMORY_RUNS
                        separate peak-RSS runs using /usr/bin/time
  --cold-runs COLD_RUNS
                        Linux: runs after dropping the page cache (sudo -n)
  --cold-case COLD_CASE
                        cases for --cold-runs (default: all selected)
  --verify-identical    require byte-identical outputs (PNG or text) across
                        binaries
  --reference REFERENCE
                        binary label for paired speedup estimates
  --unchecked UNCHECKED
                        binary label exempt from the path checks, for one that
                        predates the counters (repeatable)
  --seed SEED           seed for interleaved execution order
  --list-workloads      print each selected case's input size and SHA-256, and
                        exit (no --output needed)
";

// Arch's noto-fonts-cjk 20240730-1; face 3 is Noto Sans CJK TC Regular.
const SYSTEM_CJK: &str = "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc";
const CJK_FACE: &str = "3";
const CRLF: &str = "\r\n";

/// The repository root and the files the cases name, as bench.py's
/// module-level paths.
struct Paths {
    root: String,
    font: String,
    cjk_subset: String,
    perf: String,
}

impl Paths {
    fn new() -> Paths {
        // target/scripts/bench in the checkout, as scripts/bench.sh builds it.
        let exe = std::fs::canonicalize(std::env::current_exe().expect("current_exe")).expect("canonicalize");
        let root = exe.parent().and_then(|p| p.parent()).and_then(|p| p.parent()).expect("repository root");
        let root = root.to_str().expect("a UTF-8 path").to_string();
        Paths {
            font: format!("{root}/third_party/jetbrains-mono/JetBrainsMono-Regular.ttf"),
            cjk_subset: format!("{root}/third_party/noto-sans-cjk/NotoSansCJKtc-Subset.otf"),
            perf: format!("{root}/tests/perf"),
            root,
        }
    }

    fn at(&self, rel: &str) -> String {
        format!("{}/{}", self.root, rel)
    }

    /// bench.py's relative(): the path from the root, or a generated file's name.
    fn relative(&self, path: &str) -> String {
        match path.strip_prefix(&self.root).and_then(|p| p.strip_prefix('/')) {
            Some(rel) if !rel.is_empty() => rel.to_string(),
            _ if path == self.root => ".".to_string(),
            _ => py_name(path),
        }
    }
}

type Check = (&'static str, &'static str, i128);

/// One workload. legacy cases use the positional CLI with an explicit font
/// (as every earlier report did); the others use options, so the built-in
/// font and --fallback-font can be measured.
struct Case {
    name: String,
    src: String,
    px: u32,
    cols: u32,
    rows: u32,
    fonts: Vec<String>,
    legacy: bool,
    group: &'static str,
    checks: Vec<Check>,
    text: bool,
}

impl Case {
    fn new(name: &str, src: &str, px: u32, cols: u32, rows: u32) -> Case {
        Case {
            name: name.to_string(),
            src: py_path(src),
            px,
            cols,
            rows,
            fonts: Vec::new(),
            legacy: false,
            group: "legacy",
            checks: Vec::new(),
            text: false,
        }
    }

    fn legacy(mut self) -> Case {
        self.legacy = true;
        self
    }

    fn group(mut self, group: &'static str) -> Case {
        self.group = group;
        self
    }

    fn fonts(mut self, fonts: &[String]) -> Case {
        self.fonts = fonts.to_vec();
        self
    }

    fn checks(mut self, checks: &[Check]) -> Case {
        self.checks = checks.to_vec();
        self
    }

    fn command(&self, paths: &Paths, binary: &str, out: &str) -> Vec<String> {
        let size = format!("{}x{}", self.cols, self.rows);
        if self.text {
            return vec![binary.into(), "--size".into(), size, "--text".into(), out.into(), self.src.clone()];
        }
        if self.legacy {
            return vec![
                binary.into(),
                self.src.clone(),
                out.into(),
                paths.font.clone(),
                self.px.to_string(),
                self.cols.to_string(),
                self.rows.to_string(),
            ];
        }
        let mut c = vec![binary.to_string()];
        c.extend(self.fonts.iter().cloned());
        c.extend(["--px".to_string(), self.px.to_string(), "--size".to_string(), size, self.src.clone(), out.to_string()]);
        c
    }

    fn font_files(&self, paths: &Paths) -> Json {
        let mut files = Map::new();
        if self.legacy {
            files.set("font", Json::str(&paths.font));
            return Json::Obj(files);
        }
        files.set("font", Json::str("built-in JetBrains Mono"));
        for pair in self.fonts.chunks(2) {
            if pair.len() == 2 {
                files.set(&pair[0].trim_start_matches('-').replace("-font", ""), Json::str(&pair[1]));
            }
        }
        Json::Obj(files)
    }

    fn checks_json(&self) -> Json {
        Json::Arr(self.checks.iter().map(|(k, op, v)| Json::Str(format!("{k} {op} {v}"))).collect())
    }
}

fn write(dir: &str, name: &str, data: &[u8]) -> String {
    let path = format!("{dir}/{name}.pty");
    std::fs::write(&path, data).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
    path
}

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| fail(&format!("{path}: {e}")))
}

fn rep(s: &str, n: usize) -> String {
    s.repeat(n)
}

/// str.ljust(width).
fn ljust(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n >= width {
        s.to_string()
    } else {
        format!("{}{}", s, " ".repeat(width - n))
    }
}

fn chr(cp: i64) -> char {
    char::from_u32(cp as u32).unwrap()
}

fn legacy_workloads(paths: &Paths, directory: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    for name in ["reply-sent", "draft-ready"] {
        cases.push(Case::new(name, &paths.at(&format!("examples/{name}.pty")), 48, 100, 30).legacy());
    }
    for px in [24, 128] {
        cases.push(Case::new(&format!("reply-{px}px"), &paths.at("examples/reply-sent.pty"), px, 100, 30).legacy());
    }
    for name in ["shell", "less", "vi"] {
        cases.push(Case::new(&format!("real-{name}"), &paths.at(&format!("tests/vt/real/{name}.log")), 48, 80, 24).legacy());
    }
    let mut rng = PyRandom::new(13);
    let mut colors = String::new();
    for row in 0..30 {
        for col in 0..100 {
            let v: Vec<i64> = (0..6).map(|_| rng.below(256)).collect();
            let ch = chr(rng.randrange(33, 127));
            colors.push_str(&format!(
                "\x1b[{};{}H\x1b[38;2;{};{};{};48;2;{};{};{}m{}",
                row + 1,
                col + 1,
                v[0],
                v[1],
                v[2],
                v[3],
                v[4],
                v[5],
                ch
            ));
        }
    }
    let unicode: Vec<String> =
        (0..30).map(|r| (0..100).map(|c| chr(0x100 + (r * 100 + c) % 800)).collect::<String>()).collect();
    let reply = read(&paths.at("examples/reply-sent.pty"));
    let generated: Vec<(&str, Vec<u8>, u32, u32, u32)> = vec![
        ("blank", Vec::new(), 48, 100, 30),
        ("color-grid", colors.into_bytes(), 24, 100, 30),
        ("ascii-overflow", vec![b'x'; 4_000_000], 24, 100, 30),
        ("rounded-boxes", vec![rep("╭╮╰╯", 25); 30].join(CRLF).into_bytes(), 48, 100, 30),
        ("dense", vec![rep("The quick brown fox 0123456789! @#$% ", 3); 30].join(CRLF).into_bytes(), 48, 100, 30),
        ("ansi-replay", reply.repeat(250), 48, 100, 30),
        ("large", vec![rep("Terminal benchmark 0123456789 ", 9); 80].join(CRLF).into_bytes(), 48, 240, 80),
        ("unicode", unicode.join(CRLF).into_bytes(), 24, 100, 30),
    ];
    for (name, data, px, cols, rows) in generated {
        let path = write(directory, name, &data);
        cases.push(Case::new(name, &path, px, cols, rows).legacy());
    }
    cases
}

/// A kitty direct transmission (a=T, f=32) of a width x height RGBA gradient
/// with some transparency, scaled to cols x rows cells at z, sent in the
/// 4096-byte chunks the protocol allows.
fn kitty_image(width: u32, height: u32, cols: u32, rows: u32, z: i64, seed: i128) -> Vec<u8> {
    let mut rng = PyRandom::new(seed);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.push((x * 255 / width) as u8);
            pixels.push((y * 255 / height) as u8);
            pixels.push(rng.below(256) as u8);
            pixels.push(if (x / 16 + y / 16) % 3 != 0 { 255 } else { 128 });
        }
    }
    let data = base64(&pixels);
    let chunks: Vec<&[u8]> = data.chunks(4096).collect();
    let mut out = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = (i + 1 < chunks.len()) as u8;
        let keys = if i == 0 {
            format!("a=T,f=32,s={width},v={height},c={cols},r={rows},z={z},q=2,m={more}")
        } else {
            format!("m={more}")
        };
        out.extend_from_slice(b"\x1b_G");
        out.extend_from_slice(keys.as_bytes());
        out.push(b';');
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    out
}

/// Painting and geometry (#22): box, block and rounded grids, the same at
/// other sizes, every geometry character at once, large sparse and colored
/// screens, and an image below, under and over text.
fn draw_workloads(directory: &str) -> Vec<Case> {
    // Every row fits its screen exactly, so none wraps.
    let mut table = Vec::new();
    for row in 0..30 {
        let kind = row % 3;
        if row % 6 == 0 {
            table.push(format!("┌{}──────┐━━━┳━━━┓", rep("──────┬", 12)));
        } else if row % 6 == 5 {
            table.push(format!("└{}──────┘━━━┻━━━┛", rep("──────┴", 12)));
        } else if kind == 1 {
            table.push(format!("│{}═══╬═══╣", rep(" cell │", 13)));
        } else {
            table.push(format!("├{}──────┤║  ╠═══╣", rep("──────┼", 12)));
        }
    }
    assert!(table.iter().all(|line| line.chars().count() == 100));
    let blocks: Vec<char> = "█▀▄▌▐░▒▓▖▗▘▝▚▞▙▟▁▂▃▅▆▇▏▎▍▋▊▉▔▕".chars().collect();
    let mut rng = PyRandom::new(22);
    let block_grid: Vec<String> = (0..30)
        .map(|r| {
            (0..100)
                .map(|c| format!("\x1b[38;5;{}m{}", rng.below(256), blocks[(r * 7 + c) % blocks.len()]))
                .collect::<String>()
        })
        .collect();
    let every: Vec<char> = (0x2500..0x25a0).map(chr).collect();
    let geometry_all: Vec<String> = (0..80)
        .map(|r| {
            (0..240)
                .map(|c| format!("{}{}", if (r + c) % 2 != 0 { "\x1b[1m" } else { "\x1b[22m" }, every[(r * 240 + c) % every.len()]))
                .collect::<String>()
        })
        .collect();
    let mut panes = Vec::new();
    for r in 0..30 {
        let band = r % 10;
        if band == 0 {
            panes.push(format!("╭{}╮{}", rep("─", 23), rep(&format!("╭{}╮", rep("─", 23)), 3)));
        } else if band == 9 {
            panes.push(format!("╰{}╯{}", rep("─", 23), rep(&format!("╰{}╯", rep("─", 23)), 3)));
        } else {
            let item = ljust(&format!("item {:02} value {:4}", r, r * 37 % 1000), 21);
            panes.push(rep(&format!("│ {item} │"), 4));
        }
    }
    assert!(panes.iter().all(|line| line.chars().count() == 100));
    let rounded = vec![rep("╭╮╰╯", 25); 30].join(CRLF);
    let mut sparse_lines = vec![
        "$ termshot --px 48 --size 240x80 large.log out.png".to_string(),
        "done in 42 ms".to_string(),
        "$ ".to_string(),
    ];
    sparse_lines.extend(std::iter::repeat(String::new()).take(77));
    let sparse = sparse_lines.join(CRLF);
    let colored: Vec<String> = (0..80)
        .map(|r| {
            (0..240)
                .step_by(10)
                .map(|c| format!("\x1b[48;5;{}mBenchmark ", (r * 3 + c / 10) % 216 + 16))
                .collect::<String>()
        })
        .collect();
    // Every third row on a background of its own, which hides an image below
    // the backgrounds.
    let quick: String = rep("The quick brown fox 0123456789! @#$% ", 3).chars().take(100).collect();
    let text: Vec<String> = (0..30)
        .map(|r| format!("{}{}\x1b[0m", if r % 3 == 0 { format!("\x1b[48;5;{}m", 17 + r) } else { String::new() }, quick))
        .collect();
    let text = text.join(CRLF);
    let image = |z: i64| {
        let mut v = b"\x1b[H".to_vec();
        v.extend(kitty_image(128, 128, 60, 20, z, 22));
        v.extend_from_slice(b"\x1b[H");
        v.extend_from_slice(text.as_bytes());
        v
    };
    let generated: Vec<(&str, Vec<u8>, u32, u32, u32)> = vec![
        ("box-grid", table.join(CRLF).into_bytes(), 48, 100, 30),
        ("block-grid", block_grid.join(CRLF).into_bytes(), 48, 100, 30),
        ("rounded-panes", panes.join(CRLF).into_bytes(), 48, 100, 30),
        ("rounded-24px", rounded.clone().into_bytes(), 24, 100, 30),
        ("rounded-128px", rounded.into_bytes(), 128, 100, 30),
        ("geometry-all", geometry_all.join(CRLF).into_bytes(), 48, 240, 80),
        ("large-sparse", sparse.into_bytes(), 48, 240, 80),
        ("large-color", colored.join(CRLF).into_bytes(), 48, 240, 80),
        ("image-below", image(-1073741825), 48, 100, 30),
        ("image-under", image(-1), 48, 100, 30),
        ("image-over", image(1), 48, 100, 30),
    ];
    let mut cases = Vec::new();
    for (name, data, px, cols, rows) in generated {
        let path = write(directory, name, &data);
        cases.push(Case::new(name, &path, px, cols, rows).group("draw"));
    }
    cases
}

const PARSER_CASES: [&str; 5] = ["dense-sgr", "cursor-moves", "scrolling", "mixed-unicode", "thai-combining"];

/// Long logs for the parser (#21), each about 4-5 MB so parsing is a large
/// part of the run. Deterministic: the same bytes on every host. The logs
/// share one random sequence, so they are made together, in order.
fn parser_logs() -> &'static [(&'static str, Vec<u8>)] {
    // The parser and text suites both use them; bench.py made them twice.
    static LOGS: std::sync::OnceLock<Vec<(&'static str, Vec<u8>)>> = std::sync::OnceLock::new();
    LOGS.get_or_init(make_parser_logs)
}

fn make_parser_logs() -> Vec<(&'static str, Vec<u8>)> {
    let mut rng = PyRandom::new(21);

    // Every character in its own SGR: palette, 256-colour, truecolour in
    // both separators, attributes on and off, resets.
    let mut lines = Vec::with_capacity(3_000);
    for _ in 0..3_000 {
        let mut line = String::new();
        for _ in 0..100 {
            let form = rng.randbelow(11);
            let sgr = match form {
                0 => {
                    let a = 30 + rng.below(8);
                    format!("{};{}", a, 40 + rng.below(8))
                }
                1 => {
                    let a = 90 + rng.below(8);
                    format!("{};{}", a, 100 + rng.below(8))
                }
                2 => {
                    let a = rng.below(256);
                    format!("38;5;{};48;5;{}", a, rng.below(256))
                }
                3 | 4 | 5 => {
                    let (a, b) = (rng.below(256), rng.below(256));
                    let c = rng.below(256);
                    match form {
                        3 => format!("38;2;{a};{b};{c}"),
                        4 => format!("48;2;{a};{b};{c}"),
                        _ => format!("38:2::{a}:{b}:{c}"),
                    }
                }
                6 => format!("1;3;4;{}", 30 + rng.below(8)),
                7 => "22;23;24;39;49".to_string(),
                8 => format!("4:{};7", rng.below(4)),
                9 => "0".to_string(),
                _ => String::new(),
            };
            line.push_str(&format!("\x1b[{}m{}", sgr, chr(rng.randrange(33, 127))));
        }
        lines.push(line);
    }
    let dense_sgr = lines.join(CRLF).into_bytes();

    // Absolute and relative moves, each followed by a character, with the
    // C0 moves (CR, BS, HT) in between. bench.py built the list of every
    // move before indexing it, so each relative move draws all six numbers.
    let mut moves = String::new();
    for _ in 0..700_000 {
        let k = rng.below(13);
        if k < 4 {
            let row = rng.randrange(1, 31);
            moves.push_str(&format!("\x1b[{};{}H", row, rng.randrange(1, 101)));
        } else {
            let all = [
                format!("\x1b[{}A", rng.randrange(1, 9)),
                format!("\x1b[{}B", rng.randrange(1, 9)),
                format!("\x1b[{}C", rng.randrange(1, 30)),
                format!("\x1b[{}D", rng.randrange(1, 30)),
                format!("\x1b[{}G", rng.randrange(1, 101)),
                format!("\x1b[{}d", rng.randrange(1, 31)),
                "\r".to_string(),
                "\x08\x08".to_string(),
                "\t".to_string(),
            ];
            moves.push_str(&all[(k - 4) as usize]);
        }
        moves.push(chr(rng.randrange(33, 127)));
    }
    let cursor_moves = moves.into_bytes();

    // Lines that scroll the whole screen, then a scroll region with IND, RI,
    // IL, DL, SU and SD, then the margins reset.
    let mut parts = String::new();
    let tail = rep("abcdefghij", 4);
    for block in 0..2_000 {
        for n in 0..30 {
            parts.push_str(&format!("line {block}.{n} {tail}\r\n"));
        }
        let top = rng.randrange(2, 10);
        let bottom = rng.randrange(15, 30);
        parts.push_str(&format!("\x1b[{};{}r\x1b[{};1H", top, bottom, rng.randrange(10, 15)));
        for p in ["scrolled text\x1bD", "\x1bM", "\x1b[2L", "\x1b[3M", "\x1b[2S", "\x1b[T"] {
            parts.push_str(p);
        }
        parts.push_str(&rep("region\n", 8));
        parts.push_str("\x1b[r");
    }
    let scrolling = parts.into_bytes();

    // ASCII words between Latin-1, Greek, Cyrillic, box drawing, CJK and a
    // rare emoji, so UTF-8 decoding and the width lookup take turns.
    let words = [
        "terminal", "output", "na\u{ef}ve", "caf\u{e9}", "Ελληνικά", "Кириллица", "│", "├──", "漢字", "日本語",
        "한국어", "→", "✓", "🙂", "42", "ß", "µs",
    ];
    let lines: Vec<String> = (0..40_000)
        .map(|_| (0..14).map(|_| *rng.choice(&words)).collect::<Vec<&str>>().join(" "))
        .collect();
    let mixed_unicode = lines.join(CRLF).into_bytes();

    // Thai syllables with above and below vowels and tone marks (kept as
    // marks: no precomposed form), Latin with a composing acute, and Hebrew
    // with points.
    let consonants: Vec<char> = (0x0e01..0x0e2f).map(chr).collect();
    let above = ['\u{e31}', '\u{e34}', '\u{e35}', '\u{e36}', '\u{e37}', '\u{e47}'];
    let tones = ['\u{e48}', '\u{e49}', '\u{e4a}', '\u{e4b}'];
    let below = ['\u{e38}', '\u{e39}'];
    let mut lines = Vec::with_capacity(16_000);
    for _ in 0..16_000 {
        let mut words = Vec::with_capacity(12);
        for _ in 0..10 {
            let count = rng.randrange(2, 6);
            let mut word = String::new();
            for _ in 0..count {
                word.push(*rng.choice(&consonants));
                match rng.below(4) {
                    0 => {
                        word.push(*rng.choice(&above));
                        word.push(*rng.choice(&tones));
                    }
                    1 => {
                        word.push(*rng.choice(&below));
                        word.push(*rng.choice(&tones));
                    }
                    2 => word.push(*rng.choice(&above)),
                    _ => {}
                }
            }
            words.push(word);
        }
        words.push("cafe\u{301}".to_string());
        words.push("\u{5e9}\u{5b8}\u{5c1}\u{5dc}\u{5d5}\u{5b9}\u{5dd}".to_string());
        lines.push(words.join(" "));
    }
    let thai = lines.join(CRLF).into_bytes();

    vec![
        ("dense-sgr", dense_sgr),
        ("cursor-moves", cursor_moves),
        ("scrolling", scrolling),
        ("mixed-unicode", mixed_unicode),
        ("thai-combining", thai),
    ]
}

fn names_any(wanted: &[String], names: &[&str]) -> bool {
    wanted.iter().any(|w| names.contains(&w.as_str()))
}

/// The parser matrix (#21), at 24 px. With ansi-replay and ascii-overflow
/// from the legacy suite it covers each kind of input the parser handles.
/// Making the logs takes a while, so when --case names none of them
/// (wanted), they are not made.
fn parser_workloads(directory: &str, wanted: &[String]) -> Vec<Case> {
    if !wanted.is_empty() && !names_any(wanted, &PARSER_CASES) {
        return Vec::new();
    }
    let mut cases = Vec::new();
    for (name, data) in parser_logs() {
        let path = write(directory, name, data);
        cases.push(Case::new(name, &path, 24, 100, 30).legacy().group("parser"));
    }
    cases
}

const TEXT_CASES: [&str; 7] = [
    "text-ansi-replay",
    "text-ascii-overflow",
    "text-dense-sgr",
    "text-mixed-unicode",
    "text-reply-sent",
    "text-kitty",
    "text-sixel",
];

/// Text-only runs (--text, no PNG). Fonts are read only when the log has an
/// image that needs cell metrics, so most of these read none; text-kitty (an
/// a=T upload) and text-sixel do. font_builtin says which happened. The two
/// parser logs are made only when --case names one of them (or none).
fn text_workloads(paths: &Paths, directory: &str, wanted: &[String]) -> Vec<Case> {
    const NO_FONT: &[Check] = &[("font_builtin", "==", 0), ("font_bytes", "==", 0)];
    const FONT: &[Check] = &[("font_builtin", "==", 1)];
    let reply = read(&paths.at("examples/reply-sent.pty"));
    let mut kitty = b"\x1b[H".to_vec();
    kitty.extend(kitty_image(128, 128, 60, 20, 1, 22));
    kitty.extend_from_slice(b"\x1b[H");
    kitty.extend_from_slice(rep("text\r\n", 20).as_bytes());
    let mut logs: Vec<(&str, Vec<u8>, &[Check])> = vec![
        ("text-ansi-replay", reply.repeat(250), NO_FONT),
        ("text-ascii-overflow", vec![b'x'; 4_000_000], NO_FONT),
        ("text-reply-sent", reply, NO_FONT),
        ("text-kitty", kitty, FONT),
        ("text-sixel", read(&paths.at("tests/fixtures/sixel-magick.pty")), FONT),
    ];
    if wanted.is_empty() || names_any(wanted, &["text-dense-sgr", "text-mixed-unicode"]) {
        let parser = parser_logs();
        let mixed = parser[3].1.clone();
        let dense = parser[0].1.clone();
        logs.push(("text-dense-sgr", dense, NO_FONT));
        logs.push(("text-mixed-unicode", mixed, NO_FONT));
    }
    let mut cases = Vec::new();
    for name in TEXT_CASES {
        let Some(i) = logs.iter().position(|(n, _, _)| *n == name) else { continue };
        let (_, data, checks) = std::mem::replace(&mut logs[i], ("", Vec::new(), &[]));
        let path = write(directory, name, &data);
        let mut case = Case::new(name, &path, 48, 100, 30).group("text").checks(checks);
        case.text = true;
        cases.push(case);
    }
    cases
}

/// The font-path matrix (#19). checks are (counter, op, value) on the
/// profile record, verified on every profiled run of a binary that has it.
fn font_workloads(paths: &Paths, cjk: Option<&str>) -> Vec<Case> {
    let sub = vec!["--fallback-font".to_string(), paths.cjk_subset.clone()];
    let full = cjk.map(|c| vec!["--fallback-font".to_string(), format!("{c}#{CJK_FACE}")]);
    let reply = paths.at("examples/reply-sent.pty");
    let dense = format!("{}/cjk-dense.pty", paths.perf);
    let mixed = format!("{}/mixed-script.pty", paths.perf);
    let used: [Check; 2] = [("fallback_rasterizations", ">", 0), ("fallback_lookups", ">", 0)];
    let with = |extra: &[Check]| -> Vec<Check> { used.iter().chain(extra.iter()).cloned().collect() };
    let mut cases = vec![
        Case::new("font-builtin", &reply, 48, 100, 30).group("font").checks(&[("font_builtin", "==", 1)]),
        Case::new("font-file", &reply, 48, 100, 30)
            .fonts(&["--font".to_string(), paths.font.clone()])
            .group("font")
            .checks(&[("font_builtin", "==", 0)]),
        Case::new("cjk-none", &dense, 24, 100, 30)
            .group("fallback")
            .checks(&[("fallback_lookups", "==", 0), ("glyph_missing", ">", 0)]),
        Case::new("cjk-subset", &dense, 24, 100, 30).fonts(&sub).group("fallback").checks(&with(&[("glyph_missing", "==", 0)])),
        Case::new("cjk-cff-primary", &dense, 24, 100, 30)
            .fonts(&["--font".to_string(), paths.cjk_subset.clone()])
            .group("fallback")
            .checks(&[("fallback_lookups", "==", 0), ("glyph_missing", "==", 0)]),
        Case::new("mixed-subset", &mixed, 24, 100, 30).fonts(&sub).group("mixed").checks(&used),
        Case::new("glyph-overflow", &format!("{}/glyph-overflow.pty", paths.perf), 24, 100, 30)
            .group("cache")
            .checks(&[("glyph_cache_evictions", ">", 0), ("glyph_missing", "==", 0)]),
    ];
    if let Some(full) = full {
        cases.push(Case::new("cjk-full", &dense, 24, 100, 30).fonts(&full).group("fallback").checks(&with(&[("glyph_missing", "==", 0)])));
        cases.push(Case::new("mixed-full", &mixed, 24, 100, 30).fonts(&full).group("mixed").checks(&used));
        cases.push(
            Case::new("cjk-overflow-full", &format!("{}/cjk-overflow.pty", paths.perf), 24, 100, 30)
                .fonts(&full)
                .group("cache")
                .checks(&with(&[("glyph_cache_evictions", ">", 0), ("glyph_missing", "==", 0)])),
        );
    }
    cases
}

// ---------------------------------------------------------------- statistics

fn summary(values: &[Num]) -> Json {
    let mut v = values.to_vec();
    sort_nums(&mut v);
    let p95 = (v.len() as f64 * 0.95).ceil() as usize - 1;
    Json::obj(vec![
        ("mean", Json::num(mean(&v))),
        ("median", Json::num(median(&v))),
        ("p95", Json::num(v[p95])),
        ("min", Json::num(v[0])),
        ("max", Json::num(v[v.len() - 1])),
    ])
}

/// Ratio for each interleaved round; bootstrap whole pairs, without trimming.
fn paired_ratio(numerator: &[Num], denominator: &[Num]) -> Json {
    let ratios: Vec<Num> = numerator
        .iter()
        .zip(denominator)
        .map(|(a, b)| a.div(*b).unwrap_or_else(|e| fail(&e)))
        .collect();
    let mut rng = PyRandom::new(42);
    let mut estimates: Vec<Num> = (0..2000).map(|_| median(&rng.choices(&ratios, ratios.len()))).collect();
    sort_nums(&mut estimates);
    Json::obj(vec![
        ("median", Json::num(median(&ratios))),
        ("bootstrap_95pct", Json::Arr(vec![Json::num(estimates[49]), Json::num(estimates[1949])])),
    ])
}

/// The slim form of a report, in place: every per-run wall, CPU and RSS
/// sample, the outputs' hashes and sizes and all metadata stay; the per-run
/// TERMSHOT_PROFILE records (profile and profile_samples), which are most of
/// a full report's bytes, keep only the stages named. A slim report records
/// what it dropped under 'slim'.
fn slim(report: &mut Json, stages: &[String]) {
    let report_map = report.as_obj_mut().unwrap_or_else(|| fail("the report is not an object"));
    if let Some(Json::Obj(cases)) = report_map.get_mut("cases") {
        for (_, case) in cases.0.iter_mut() {
            let Some(case) = case.as_obj_mut() else { continue };
            for (label, entry) in case.0.iter_mut() {
                let Some(entry) = entry.as_obj_mut() else { continue };
                if label == "workload" || !entry.contains("profile_samples") {
                    continue;
                }
                let keep = |m: Option<&Json>| {
                    let mut out = Map::new();
                    if let Some(Json::Obj(m)) = m {
                        for key in stages {
                            if let Some(v) = m.get(key) {
                                out.set(key, v.clone());
                            }
                        }
                    }
                    Json::Obj(out)
                };
                let samples = keep(entry.get("profile_samples"));
                let prof = keep(entry.get("profile"));
                entry.set("profile_samples", samples);
                entry.set("profile", prof);
            }
        }
    } else {
        fail("KeyError: 'cases'");
    }
    report_map.set(
        "slim",
        Json::obj(vec![
            ("profile_stages_kept", Json::Arr(stages.iter().map(|s| Json::str(s)).collect())),
            ("dropped", Json::str("every other key of profile and profile_samples (bench.sh --full-profile keeps them)")),
        ]),
    );
}

// ---------------------------------------------------------------- the host

mod sys {
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Timeval {
        pub sec: i64,
        #[cfg(target_os = "macos")]
        pub usec: i32,
        #[cfg(target_os = "macos")]
        _pad: i32,
        #[cfg(not(target_os = "macos"))]
        pub usec: i64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Rusage {
        pub utime: Timeval,
        pub stime: Timeval,
        rest: [i64; 14],
    }

    #[cfg(target_os = "macos")]
    #[repr(C)]
    pub struct Utsname {
        pub fields: [[u8; 256]; 5],
    }

    #[cfg(not(target_os = "macos"))]
    #[repr(C)]
    pub struct Utsname {
        pub fields: [[u8; 65]; 6],
    }

    extern "C" {
        pub fn getrusage(who: i32, usage: *mut Rusage) -> i32;
        pub fn getloadavg(loads: *mut f64, n: i32) -> i32;
        pub fn gethostname(name: *mut u8, len: usize) -> i32;
        pub fn uname(buf: *mut Utsname) -> i32;
        pub fn sysconf(name: i32) -> i64;
        pub fn mkdtemp(template: *mut u8) -> *mut u8;
        #[cfg(not(target_os = "macos"))]
        pub fn confstr(name: i32, buf: *mut u8, len: usize) -> usize;
    }

    pub const RUSAGE_CHILDREN: i32 = -1;
    #[cfg(target_os = "macos")]
    pub const SC_NPROCESSORS_ONLN: i32 = 58;
    #[cfg(not(target_os = "macos"))]
    pub const SC_NPROCESSORS_ONLN: i32 = 84;
    #[cfg(not(target_os = "macos"))]
    pub const SC_PAGE_SIZE: i32 = 30;
    #[cfg(not(target_os = "macos"))]
    pub const SC_PHYS_PAGES: i32 = 85;
    #[cfg(not(target_os = "macos"))]
    pub const CS_GNU_LIBC_VERSION: i32 = 2;

    pub fn cstr(b: &[u8]) -> String {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        String::from_utf8_lossy(&b[..end]).into_owned()
    }
}

/// The clock, the children's CPU times, the load averages and the time of
/// day, real or (TERMSHOT_BENCH_FAKE_CLOCK=1) fixed sequences.
struct Clock {
    fake: bool,
    start: Instant,
    ticks: u64,
    now_ns: u64,
    usage_calls: u64,
}

impl Clock {
    fn new() -> Clock {
        Clock {
            fake: std::env::var_os("TERMSHOT_BENCH_FAKE_CLOCK").map_or(false, |v| v == "1"),
            start: Instant::now(),
            ticks: 0,
            now_ns: 1_000_000_000,
            usage_calls: 0,
        }
    }

    /// time.perf_counter_ns().
    fn perf_counter_ns(&mut self) -> u64 {
        if self.fake {
            self.ticks += 1;
            self.now_ns += 1_000_000 + (self.ticks * 7_919_731) % 9_000_000;
            return self.now_ns;
        }
        self.start.elapsed().as_nanos() as u64
    }

    /// resource.getrusage(RUSAGE_CHILDREN): user and system seconds as
    /// Python's doubletime makes them, tv_sec + tv_usec * 0.000001.
    fn children_times(&mut self) -> (f64, f64) {
        let (u, s) = if self.fake {
            self.usage_calls += 1;
            let c = self.usage_calls;
            let u = 1_000_000 * c + (c * 77_777) % 1_000_000;
            let s = 500_000 * c + (c * 33_331) % 1_000_000;
            ((u / 1_000_000, u % 1_000_000), (s / 1_000_000, s % 1_000_000))
        } else {
            let mut r = sys::Rusage::default();
            // SAFETY: r is a properly sized rusage for this platform.
            if unsafe { sys::getrusage(sys::RUSAGE_CHILDREN, &mut r) } != 0 {
                fail("getrusage failed");
            }
            ((r.utime.sec as u64, r.utime.usec as u64), (r.stime.sec as u64, r.stime.usec as u64))
        };
        let double = |(sec, usec): (u64, u64)| sec as f64 + usec as f64 * 0.000001;
        (double(u), double(s))
    }

    /// os.getloadavg().
    fn loadavg(&self) -> Json {
        let mut l = [0f64; 3];
        if self.fake {
            l = [1.5, 0.25, 0.125];
        } else if unsafe { sys::getloadavg(l.as_mut_ptr(), 3) } != 3 {
            fail("OSError: Load averages are unobtainable");
        }
        Json::Arr(l.iter().map(|&v| Json::Float(v)).collect())
    }

    /// datetime.now(timezone.utc).isoformat().
    fn timestamp(&self) -> String {
        let (secs, micros) = if self.fake {
            (1_791_331_200i64, 123_456u32) // 2026-10-07T00:00:00.123456
        } else {
            let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock");
            // datetime rounds the fraction to microseconds, half to even.
            let mut secs = d.as_secs() as i64;
            let nanos = d.subsec_nanos();
            let mut micros = nanos / 1000;
            let rest = nanos % 1000;
            if rest > 500 || (rest == 500 && micros % 2 == 1) {
                micros += 1;
            }
            if micros == 1_000_000 {
                secs += 1;
                micros = 0;
            }
            (secs, micros)
        };
        let days = secs.div_euclid(86_400);
        let t = secs.rem_euclid(86_400);
        // Howard Hinnant's civil_from_days.
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + (month <= 2) as i64;
        let mut out = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", year, month, day, t / 3600, t / 60 % 60, t % 60);
        if micros != 0 {
            out.push_str(&format!(".{:06}", micros));
        }
        out.push_str("+00:00");
        out
    }
}

fn run_text(command: &[&str]) -> Option<String> {
    let out = Command::new(command[0]).args(&command[1..]).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// bench.py's first_line(): the first line of a tool's output (stdout and
/// stderr together), or None.
fn first_line(command: &[&str]) -> Option<String> {
    let out = Command::new(command[0]).args(&command[1..]).stdin(Stdio::inherit()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    splitlines(&text).first().map(|s| s.to_string())
}

fn uname() -> Vec<String> {
    // SAFETY: Utsname matches the platform's struct utsname.
    let mut u: sys::Utsname = unsafe { std::mem::zeroed() };
    if unsafe { sys::uname(&mut u) } != 0 {
        fail("uname failed");
    }
    u.fields.iter().map(|f| sys::cstr(f)).collect()
}

/// platform's _platform(): the parts joined with '-', made safe for a file
/// name, 'unknown' dropped.
fn platform_join(parts: &[&str]) -> String {
    let mut p = parts.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    p = p.replace(' ', "_");
    for c in ['/', '\\', ':', ';', '"', '(', ')'] {
        p = p.replace(c, "-");
    }
    p = p.replace("unknown", "");
    loop {
        let cleaned = p.replace("--", "-");
        if cleaned == p {
            break;
        }
        p = cleaned;
    }
    while p.ends_with('-') {
        p.pop();
    }
    p
}

/// platform.processor(): `uname -p`, blank when unknown.
fn processor() -> String {
    let p = run_text(&["uname", "-p"]).map(|s| s.trim().to_string()).unwrap_or_default();
    if p == "unknown" {
        String::new()
    } else {
        p
    }
}

#[cfg(not(target_os = "macos"))]
fn libc_ver() -> (String, String) {
    let mut buf = [0u8; 256];
    // SAFETY: buf is writable for its length.
    let n = unsafe { sys::confstr(sys::CS_GNU_LIBC_VERSION, buf.as_mut_ptr(), buf.len()) };
    if n == 0 || n > buf.len() {
        return (String::new(), String::new());
    }
    let s = sys::cstr(&buf);
    match s.split_once(char::is_whitespace) {
        Some((a, b)) => (a.to_string(), b.trim_start().to_string()),
        None => (String::new(), String::new()),
    }
}

fn hostname() -> String {
    let mut buf = [0u8; 1024];
    // SAFETY: buf is writable for its length.
    unsafe { sys::gethostname(buf.as_mut_ptr(), buf.len()) };
    sys::cstr(&buf)
}

fn machine() -> Json {
    let u = uname(); // sysname, nodename, release, version, machine
    let (system, release, mach) = (u[0].clone(), u[2].clone(), u[4].clone());
    let mut proc = processor();
    if proc == mach {
        proc.clear();
    }
    let mut info = Map::new();
    #[cfg(target_os = "macos")]
    {
        let os = first_line(&["sw_vers", "-productVersion"]);
        let mac = run_text(&["sw_vers", "-productVersion"]).map(|s| s.trim().to_string()).unwrap_or_default();
        let (sys_name, rel) = if system == "Darwin" && !mac.is_empty() { ("macOS".to_string(), mac) } else { (system.clone(), release) };
        info.set("platform", Json::Str(platform_join(&[&sys_name, &rel, &mach, &proc, "64bit", "Mach-O"])));
        info.set("machine", Json::Str(mach.clone()));
        info.set("hostname", Json::Str(hostname()));
        let cpu = run_text(&["sysctl", "-n", "machdep.cpu.brand_string"]).unwrap_or_else(|| fail("sysctl failed"));
        info.set("cpu", Json::Str(cpu.trim().to_string()));
        let ram = run_text(&["sysctl", "-n", "hw.memsize"]).unwrap_or_else(|| fail("sysctl failed"));
        info.set("ram_bytes", Json::Int(py_int(&ram).unwrap_or_else(|| fail("hw.memsize"))));
        info.set("os", Json::opt_str(os));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let (libc, version) = libc_ver();
        let with = format!("{libc}{version}");
        info.set("platform", Json::Str(platform_join(&[&system, &release, &mach, &proc, "with", &with])));
        info.set("machine", Json::Str(mach.clone()));
        info.set("hostname", Json::Str(hostname()));
        let text = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_else(|e| fail(&format!("/proc/cpuinfo: {e}")));
        info.set("cpu", Json::opt_str(model_name(&text)));
        // SAFETY: plain sysconf queries.
        let ram = unsafe { sys::sysconf(sys::SC_PAGE_SIZE) as i128 * sys::sysconf(sys::SC_PHYS_PAGES) as i128 };
        info.set("ram_bytes", Json::Int(ram));
        let governor = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor";
        info.set(
            "cpufreq_governor",
            if Path::new(governor).exists() {
                Json::Str(std::fs::read_to_string(governor).unwrap_or_default().trim().to_string())
            } else {
                Json::Null
            },
        );
        info.set("libc", Json::Str(format!("{libc} {version}")));
    }
    let _ = (&system, &proc);
    // SAFETY: a plain sysconf query.
    let cpus = unsafe { sys::sysconf(sys::SC_NPROCESSORS_ONLN) };
    info.set("logical_cpus", if cpus < 1 { Json::Null } else { Json::Int(cpus as i128) });
    Json::Obj(info)
}

/// re.search(r'model name\s*:\s*(.+)', text)[1].strip().
#[cfg(not(target_os = "macos"))]
fn model_name(text: &str) -> Option<String> {
    let mut from = 0;
    while let Some(at) = text[from..].find("model name") {
        let start = from + at;
        let rest = text[start + "model name".len()..].trim_start_matches(|c: char| c.is_whitespace());
        if let Some(rest) = rest.strip_prefix(':') {
            let rest = rest.trim_start_matches(|c: char| c.is_whitespace());
            let line = rest.split('\n').next().unwrap_or("");
            if !line.is_empty() {
                return Some(line.trim().to_string());
            }
        }
        from = start + 1;
    }
    None
}

/// Every file under dir, recursively, without following links to directories
/// (Path.rglob('*')), or only its entries (Path.glob('*')).
fn walk(dir: &Path, recurse: bool, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        out.push(path.clone());
        if recurse && std::fs::symlink_metadata(&path).map_or(false, |m| m.is_dir()) {
            walk(&path, true, out);
        }
    }
}

fn source(paths: &Paths) -> Json {
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git").args(args).current_dir(&paths.root).stderr(Stdio::null()).output().ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    // The hash of the files the build reads, so a tarball without .git is identified too.
    let root = Path::new(&paths.root);
    let mut files = vec![root.join("build.sh")];
    walk(&root.join("src"), false, &mut files);
    walk(&root.join("third_party"), true, &mut files);
    // Path ordering: part by part, as pathlib compares.
    let mut rel: Vec<(Vec<String>, std::path::PathBuf)> = files
        .into_iter()
        .map(|p| {
            let parts = p.strip_prefix(root).unwrap().iter().map(|s| s.to_string_lossy().into_owned()).collect();
            (parts, p)
        })
        .collect();
    rel.sort();
    let mut digest = Sha256::new();
    for (parts, path) in &rel {
        if path.is_file() {
            digest.update(parts.join("/").as_bytes());
            digest.update(b"\0");
            digest.update(&read(path.to_str().unwrap()));
        }
    }
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).map_or(false, |s| !s.is_empty());
    Json::obj(vec![
        ("git_head", Json::opt_str(git(&["rev-parse", "HEAD"]))),
        ("git_dirty", Json::Bool(dirty)),
        ("build_inputs_sha256", Json::Str(digest.hex())),
        ("build_sh_sha256", Json::Str(sha256_hex(&read(&paths.at("build.sh"))))),
    ])
}

/// The termshot-profile records in a run's stderr, merged.
fn profile_records(stderr: &[u8]) -> Map {
    let mut profile = Map::new();
    let text = String::from_utf8_lossy(stderr);
    for line in splitlines(&text) {
        if let Some(record) = line.strip_prefix("termshot-profile ") {
            match parse_json(record) {
                Ok(Json::Obj(m)) => {
                    for (k, v) in m.0 {
                        profile.set(&k, v);
                    }
                }
                Ok(_) => fail("AttributeError: a termshot-profile record is not an object"),
                Err(e) => fail(&format!("json.decoder.JSONDecodeError: {e}")),
            }
        }
    }
    profile
}

/// str() of a JSON value as bench.py's messages print one.
fn py_str(v: &Json) -> String {
    match v {
        Json::Null => "None".to_string(),
        Json::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        Json::Int(i) => i.to_string(),
        Json::Float(f) => py_float_repr(*f),
        Json::Str(s) => s.clone(),
        other => dumps(other),
    }
}

/// The counters that show a case exercised the path it is named for. A
/// missing counter fails: only --unchecked exempts a binary.
fn check(case: &Case, profile: &Map) -> Vec<String> {
    if profile.0.is_empty() {
        return vec!["no termshot-profile records".to_string()];
    }
    let mut failed = Vec::new();
    for (key, op, value) in &case.checks {
        let Some(have) = profile.get(key) else {
            failed.push(format!("no {key} in the profile"));
            continue;
        };
        let n = have.as_num().unwrap_or_else(|e| fail(&e));
        let ok = match *op {
            ">" => n.cmp(Num::Int(*value)) == std::cmp::Ordering::Greater,
            _ => n.cmp(Num::Int(*value)) == std::cmp::Ordering::Equal,
        };
        if !ok {
            failed.push(format!("{key} {op} {value} (got {})", py_str(have)));
        }
    }
    failed
}

fn py_list(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|s| py_repr_str(s)).collect::<Vec<_>>().join(", "))
}

/// Linux only: write back and drop the page cache (needs passwordless sudo).
fn drop_caches() {
    for command in [&["sync"][..], &["sudo", "-n", "sh", "-c", "echo 3 > /proc/sys/vm/drop_caches"][..]] {
        let ok = Command::new(command[0]).args(&command[1..]).status().map_or(false, |s| s.success());
        if !ok {
            fail(&format!("subprocess.CalledProcessError: Command {} failed", py_list(&command.iter().map(|s| s.to_string()).collect::<Vec<_>>())));
        }
    }
}

static TMP: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Stop with an error, as an uncaught Python exception does: a message on
/// stderr, status 1, the temporary directory removed.
fn fail(message: &str) -> ! {
    if let Some(dir) = TMP.lock().unwrap().as_ref() {
        let _ = std::fs::remove_dir_all(dir);
    }
    eprintln!("{message}");
    std::process::exit(1);
}

fn make_tmp() -> String {
    let base = std::env::var("TMPDIR").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/tmp".to_string());
    let mut template = format!("{}/termshot-bench-XXXXXX\0", base.trim_end_matches('/')).into_bytes();
    // SAFETY: template is a writable NUL-terminated buffer.
    if unsafe { sys::mkdtemp(template.as_mut_ptr()) }.is_null() {
        fail(&format!("mkdtemp in {base} failed"));
    }
    template.pop();
    String::from_utf8(template).unwrap()
}

fn run(command: &[String], env: &[(OsString, OsString)], stderr: Stdio) -> std::process::Output {
    let out = Command::new(&command[0])
        .args(&command[1..])
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(stderr)
        .output()
        .unwrap_or_else(|e| fail(&format!("{}: {e}", command[0])));
    if !out.status.success() {
        let code = out.status.code().map_or("a signal".to_string(), |c| c.to_string());
        fail(&format!("subprocess.CalledProcessError: Command {} returned non-zero exit status {code}.", py_list(command)));
    }
    out
}

fn file_size(path: &str) -> i128 {
    std::fs::metadata(path).unwrap_or_else(|e| fail(&format!("{path}: {e}"))).len() as i128
}

/// The digits before "maximum resident set size" (macOS /usr/bin/time -l),
/// or after "Maximum resident set size (kbytes):" (GNU time -v).
fn peak_rss(stderr: &str, darwin: bool) -> Option<i128> {
    if darwin {
        let mut from = 0;
        while let Some(at) = stderr[from..].find("maximum resident set size") {
            let at = from + at;
            let before = stderr[..at].trim_end_matches(|c: char| c.is_whitespace());
            if before.len() < at {
                let digits = before.len() - before.bytes().rev().take_while(|b| b.is_ascii_digit()).count();
                if digits < before.len() {
                    return before[digits..].parse().ok();
                }
            }
            from = at + 1;
        }
        None
    } else {
        let key = "Maximum resident set size (kbytes):";
        let at = stderr.find(key)?;
        let rest = stderr[at + key.len()..].trim_start_matches(|c: char| c.is_whitespace());
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    }
}

fn main() {
    const SUITES: &[&str] = &["all", "legacy", "fonts", "draw", "parser", "text"];
    let opt = |name, metavar, int| Opt { name, metavar, choices: None, int };
    let opts = [
        opt("--binary", Some("BINARY"), false),
        opt("--describe", Some("DESCRIBE"), false),
        opt("--runs", Some("RUNS"), true),
        opt("--warmups", Some("WARMUPS"), true),
        opt("--output", Some("OUTPUT"), false),
        opt("--full-profile", None, false),
        opt("--stage", Some("STAGE"), false),
        opt("--slim", Some("REPORT"), false),
        opt("--case", Some("CASE"), false),
        Opt { name: "--suite", metavar: Some("SUITE"), choices: Some(SUITES), int: false },
        opt("--cjk-font", Some("CJK_FONT"), false),
        opt("--memory-runs", Some("MEMORY_RUNS"), true),
        opt("--cold-runs", Some("COLD_RUNS"), true),
        opt("--cold-case", Some("COLD_CASE"), false),
        opt("--verify-identical", None, false),
        opt("--reference", Some("REFERENCE"), false),
        opt("--unchecked", Some("UNCHECKED"), false),
        opt("--seed", Some("SEED"), true),
        opt("--list-workloads", None, false),
        // Verification only: write every selected case's input and metadata to a directory.
        opt("--dump-workloads", Some("DIR"), false),
    ];
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let help = format!("{USAGE}{HELP_TAIL}");
    let listing = argv.iter().any(|a| a == "--list-workloads" || a.starts_with("--dump-workloads"));
    let required: &[&str] = if listing { &[] } else { &["--output"] };
    let args = parse_args(&argv, &opts, USAGE, &help, PROG, required, None);
    let error = |message: &str| -> ! { arg_error(USAGE, PROG, message) };

    if let Some(report_path) = args.last("--slim") {
        let text = std::fs::read_to_string(py_path(report_path)).unwrap_or_else(|e| fail(&format!("{report_path}: {e}")));
        let mut report = parse_json(&text).unwrap_or_else(|e| fail(&e));
        slim(&mut report, &args.all("--stage"));
        let out = py_path(args.last("--output").unwrap());
        std::fs::write(&out, dumps(&report) + "\n").unwrap_or_else(|e| fail(&format!("{out}: {e}")));
        return;
    }
    let paths = Paths::new();
    let runs = args.int("--runs", 10);
    let warmups = args.int("--warmups", 2);
    let memory_runs = args.int("--memory-runs", 0);
    let cold_runs = args.int("--cold-runs", 0);
    let seed = args.int("--seed", 0);
    let suite = args.last("--suite").unwrap_or("all").to_string();
    let wanted = args.all("--case");
    let cold_cases = args.all("--cold-case");
    let unchecked = args.all("--unchecked");
    let stages = args.all("--stage");
    let cjk_font: Option<String> = match args.last("--cjk-font") {
        Some(p) => Some(py_path(p)),
        None if Path::new(SYSTEM_CJK).exists() => Some(SYSTEM_CJK.to_string()),
        None => None,
    };
    if let Some(dump) = args.last("--dump-workloads").map(py_path) {
        list_workloads(&paths, &suite, &wanted, cjk_font.as_deref(), Some(&dump));
        return;
    }
    if args.flag("--list-workloads") {
        list_workloads(&paths, &suite, &wanted, cjk_font.as_deref(), None);
        return;
    }
    let output = py_path(args.last("--output").unwrap());
    let binary_args = args.all("--binary");
    if binary_args.is_empty() {
        error("--binary is required");
    }
    if runs < 1 || warmups < 0 || memory_runs < 0 || cold_runs < 0 {
        error("runs must be positive; warmups, memory-runs and cold-runs must be nonnegative");
    }
    let mut binaries = Map::new();
    for item in &binary_args {
        let Some((label, path)) = item.split_once('=') else {
            fail("ValueError: not enough values to unpack (expected 2, got 1)");
        };
        let resolved = std::fs::canonicalize(path).unwrap_or_else(|e| fail(&format!("{path}: {e}")));
        binaries.set(label, Json::Str(resolved.to_str().unwrap().to_string()));
    }
    let labels = binaries.keys();
    let binary = |label: &str| binaries.get(label).unwrap().as_str().unwrap().to_string();
    let reference = args.last("--reference").map(|s| s.to_string());
    if let Some(r) = &reference {
        if !r.is_empty() && !labels.contains(r) {
            error("reference must name one of the binary labels");
        }
    }
    if unchecked.iter().any(|u| !labels.contains(u)) {
        error("--unchecked must name binary labels");
    }
    if !cold_cases.is_empty() && cold_runs == 0 {
        error("--cold-case needs --cold-runs");
    }
    let linux = cfg!(target_os = "linux");
    let darwin = cfg!(target_os = "macos");
    if cold_runs != 0 && !linux {
        error("--cold-runs drops the Linux page cache; it is not supported here");
    }
    if let Some(cjk) = &cjk_font {
        if !Path::new(cjk).exists() {
            error(&format!("{cjk}: no such file"));
        }
    }
    let reference = reference.filter(|r| !r.is_empty());
    let mut clock = Clock::new();

    let mut describe = Map::new();
    for item in args.all("--describe") {
        match item.split_once('=') {
            Some((k, v)) => describe.set(k, Json::str(v)),
            None => fail("ValueError: dictionary update sequence element has length 1; 2 is required"),
        }
    }
    let mut binary_sha = Map::new();
    for label in &labels {
        binary_sha.set(label, Json::Str(sha256_hex(&read(&binary(label)))));
    }
    let mut toolchain = Map::new();
    for tool in ["rustc", "cc"] {
        toolchain.set(tool, Json::opt_str(first_line(&[tool, "--version"])));
    }
    let machine_info = machine();
    let source_info = source(&paths);
    let mut report = Map::new();
    report.set("machine", machine_info);
    report.set("source", source_info);
    report.set("timestamp_utc", Json::Str(clock.timestamp()));
    report.set("binaries", Json::Obj(binaries.clone()));
    report.set("binary_sha256", Json::Obj(binary_sha));
    report.set("describe", Json::Obj(describe));
    report.set("toolchain", Json::Obj(toolchain));
    report.set(
        "build_flags",
        Json::obj(vec![
            ("c", Json::str("-O2 -ffp-contract=off (stb_glue.c); -O2 (image.c)")),
            ("rust", Json::str("--edition 2021 -C opt-level=2")),
        ]),
    );
    report.set("runs", Json::Int(runs));
    report.set("warmups", Json::Int(warmups));
    report.set("memory_runs", Json::Int(memory_runs));
    report.set("cold_runs", Json::Int(cold_runs));
    report.set("verify_identical", Json::Bool(args.flag("--verify-identical")));
    report.set("seed", Json::Int(seed));
    report.set("reference", Json::opt_str(args.last("--reference").map(|s| s.to_string())));
    report.set("unchecked", Json::Arr(unchecked.iter().map(|s| Json::str(s)).collect()));
    report.set("cache_condition", Json::str("warm: page cache populated by warmups; output files rewritten, no fsync"));
    let mut fonts = Map::new();
    let mut font_paths = vec![paths.font.clone(), paths.cjk_subset.clone()];
    font_paths.extend(cjk_font.iter().cloned());
    for path in &font_paths {
        fonts.set(
            path,
            Json::obj(vec![("bytes", Json::Int(file_size(path))), ("sha256", Json::Str(sha256_hex(&read(path))))]),
        );
    }
    report.set("fonts", Json::Obj(fonts));
    report.set("load_average_start", clock.loadavg());
    report.set("cases", Json::Obj(Map::new()));
    report.set("cold", Json::Obj(Map::new()));

    let mut env: Vec<(OsString, OsString)> = std::env::vars_os().filter(|(k, _)| k != "TERMSHOT_PROFILE").collect();
    let mut profiled_env = env.clone();
    profiled_env.push(("TERMSHOT_PROFILE".into(), "1".into()));
    let mut rng = PyRandom::new(seed);
    let directory = make_tmp();
    *TMP.lock().unwrap() = Some(directory.clone());
    let cases = select(&paths, &directory, &suite, &wanted, cjk_font.as_deref());
    if !cold_cases.is_empty() {
        let names: BTreeSet<&str> = cases.iter().map(|c| c.name.as_str()).collect();
        let unknown: BTreeSet<&str> = cold_cases.iter().map(|s| s.as_str()).filter(|n| !names.contains(n)).collect();
        if !unknown.is_empty() {
            let _ = std::fs::remove_dir_all(&directory);
            error(&format!("--cold-case names a case not selected: {}", unknown.into_iter().collect::<Vec<_>>().join(", ")));
        }
    }
    let ext = |case: &Case| if case.text { "txt" } else { "png" };
    let verify_identical = args.flag("--verify-identical");
    let mut report_cases = Map::new();
    for case in &cases {
        let out: Vec<String> = labels.iter().map(|l| format!("{directory}/{l}.{}", ext(case))).collect();
        let n = labels.len();
        let mut plain: Vec<Vec<Num>> = vec![Vec::new(); n];
        let mut cpu: Vec<Vec<Num>> = vec![Vec::new(); n];
        let mut profiled_wall: Vec<Vec<Num>> = vec![Vec::new(); n];
        let mut profiles: Vec<Vec<Map>> = vec![Vec::new(); n];
        let mut failures: Vec<String> = Vec::new();
        let mut seen: Vec<BTreeSet<String>> = vec![BTreeSet::new(); n];
        for round in -warmups..runs {
            let mut order: Vec<(usize, bool)> = (0..n).flat_map(|l| [(l, false), (l, true)]).collect();
            rng.shuffle(&mut order);
            for (l, profiled) in order {
                let command = case.command(&paths, &binary(&labels[l]), &out[l]);
                let before = clock.children_times();
                let start = clock.perf_counter_ns();
                // Plain and profiled runs are spawned alike; parsing the
                // profile happens after the clock stops.
                let result = run(&command, if profiled { &profiled_env } else { &env }, Stdio::piped());
                let elapsed = py_round((clock.perf_counter_ns() - start) as f64 / 1e6, 4);
                let after = clock.children_times();
                let profile = if profiled { Some(profile_records(&result.stderr)) } else { None };
                // Every run's output, outside the timed interval.
                seen[l].insert(sha256_hex(&read(&out[l])));
                if round < 0 {
                    continue;
                }
                if let Some(profile) = profile {
                    profiled_wall[l].push(Num::Float(elapsed));
                    if !unchecked.contains(&labels[l]) {
                        failures.extend(check(case, &profile).into_iter().map(|f| format!("{}: {f}", labels[l])));
                    }
                    profiles[l].push(profile);
                } else {
                    plain[l].push(Num::Float(elapsed));
                    let used = after.0 + after.1 - before.0 - before.1;
                    cpu[l].push(Num::Float(py_round(1000.0 * used, 4)));
                }
            }
        }
        if !failures.is_empty() {
            let unique: BTreeSet<String> = failures.into_iter().collect();
            fail(&format!(
                "RuntimeError: {} did not exercise its path: {}",
                case.name,
                py_list(&unique.into_iter().collect::<Vec<_>>())
            ));
        }
        let mut rss: Vec<Vec<Num>> = vec![Vec::new(); n];
        for _ in 0..memory_runs {
            let mut order: Vec<usize> = (0..n).collect();
            rng.shuffle(&mut order);
            for l in order {
                let mut command = vec!["/usr/bin/time".to_string(), if darwin { "-l" } else { "-v" }.to_string()];
                command.extend(case.command(&paths, &binary(&labels[l]), &out[l]));
                let mut c_env = env.clone();
                c_env.retain(|(k, _)| k != "LC_ALL");
                c_env.push(("LC_ALL".into(), "C".into()));
                let result = run(&command, &c_env, Stdio::piped());
                let stderr = String::from_utf8_lossy(&result.stderr).into_owned();
                let Some(bytes) = peak_rss(&stderr, darwin) else {
                    fail(&format!("RuntimeError: Unable to read peak RSS: {stderr}"));
                };
                rss[l].push(Num::Int(bytes * if darwin { 1 } else { 1024 }));
                seen[l].insert(sha256_hex(&read(&out[l])));
            }
        }
        let varied: Vec<String> = (0..n)
            .filter(|&l| seen[l].len() != 1)
            .map(|l| format!("{}: {}", py_repr_str(&labels[l]), py_list(&seen[l].iter().cloned().collect::<Vec<_>>())))
            .collect();
        if !varied.is_empty() {
            fail(&format!("RuntimeError: Output bytes vary between runs in {}: {{{}}}", case.name, varied.join(", ")));
        }
        let hashes: Vec<String> = seen.iter().map(|s| s.iter().next().unwrap().clone()).collect();
        if verify_identical && hashes.iter().collect::<BTreeSet<_>>().len() != 1 {
            let shown: Vec<String> = (0..n).map(|l| format!("{}: {}", py_repr_str(&labels[l]), py_repr_str(&hashes[l]))).collect();
            fail(&format!("RuntimeError: Output bytes differ in {}: {{{}}}", case.name, shown.join(", ")));
        }
        let mut entry = Map::new();
        entry.set(
            "workload",
            Json::obj(vec![
                ("group", Json::str(case.group)),
                ("px", Json::Int(case.px as i128)),
                ("cols", Json::Int(case.cols as i128)),
                ("rows", Json::Int(case.rows as i128)),
                ("cli", Json::str(if case.legacy { "positional" } else { "options" })),
                ("fonts", case.font_files(&paths)),
                ("output", Json::str(if case.text { "text" } else { "png" })),
                ("input", Json::Str(paths.relative(&case.src))),
                ("input_bytes", Json::Int(file_size(&case.src))),
                ("input_sha256", Json::Str(sha256_hex(&read(&case.src)))),
                ("checks", case.checks_json()),
            ]),
        );
        let reference_index = reference.as_ref().and_then(|r| labels.iter().position(|l| l == r));
        for l in 0..n {
            let keys = profiles[l][0].keys();
            let mut profile = Map::new();
            let mut samples = Map::new();
            for key in &keys {
                let values: Vec<Num> = profiles[l]
                    .iter()
                    .map(|s| {
                        s.get(key)
                            .unwrap_or_else(|| fail(&format!("KeyError: {}", py_repr_str(key))))
                            .as_num()
                            .unwrap_or_else(|e| fail(&e))
                    })
                    .collect();
                profile.set(key, summary(&values));
                samples.set(key, Json::nums(&values));
            }
            let size = file_size(&out[l]);
            let mut e = Map::new();
            e.set("wall_ms", summary(&plain[l]));
            e.set("wall_samples_ms", Json::nums(&plain[l]));
            e.set("child_cpu_ms", summary(&cpu[l]));
            e.set("child_cpu_samples_ms", Json::nums(&cpu[l]));
            e.set("profiled_wall_ms", summary(&profiled_wall[l]));
            e.set("profiled_wall_samples_ms", Json::nums(&profiled_wall[l]));
            e.set("profile_overhead", paired_ratio(&profiled_wall[l], &plain[l]));
            e.set("profile", Json::Obj(profile));
            e.set("profile_samples", Json::Obj(samples));
            // output_* for every case; png_* too for a PNG, as in earlier
            // reports (a text case's output is no PNG).
            e.set("output_bytes", Json::Int(size));
            e.set("output_sha256", Json::Str(hashes[l].clone()));
            if !case.text {
                e.set("png_bytes", Json::Int(size));
                e.set("png_sha256", Json::Str(hashes[l].clone()));
            }
            e.set("peak_rss_bytes", if rss[l].is_empty() { Json::Null } else { summary(&rss[l]) });
            e.set("peak_rss_samples_bytes", Json::nums(&rss[l]));
            if let Some(r) = reference_index {
                if r != l {
                    e.set("paired_wall_speedup", paired_ratio(&plain[r], &plain[l]));
                    e.set("paired_cpu_speedup", paired_ratio(&cpu[r], &cpu[l]));
                }
            }
            let overhead = e.get("profile_overhead").unwrap().at("median").unwrap().as_num().unwrap().f();
            let mut sorted = plain[l].clone();
            sort_nums(&mut sorted);
            let p95 = sorted[(sorted.len() as f64 * 0.95).ceil() as usize - 1].f();
            say!(
                "{:18} {:10} median={:8.2} ms p95={:8.2} ms profiled x{:.3}",
                case.name,
                labels[l],
                median(&plain[l]).f(),
                p95,
                overhead
            );
            entry.set(&labels[l], Json::Obj(e));
        }
        report_cases.set(&case.name, Json::Obj(entry));
        report.set("cases", Json::Obj(report_cases.clone()));
    }
    // Cold: each run starts with an empty page cache, so the binary, the
    // input and any font file come from storage.
    let mut cold_report = Map::new();
    for case in &cases {
        if cold_runs == 0 || (!cold_cases.is_empty() && !cold_cases.contains(&case.name)) {
            continue;
        }
        let n = labels.len();
        let mut cold: Vec<Vec<Num>> = vec![Vec::new(); n];
        for _ in 0..cold_runs {
            let mut order: Vec<usize> = (0..n).collect();
            rng.shuffle(&mut order);
            for l in order {
                drop_caches();
                let cold_out = format!("{directory}/{}-cold.{}", labels[l], ext(case));
                let command = case.command(&paths, &binary(&labels[l]), &cold_out);
                let start = clock.perf_counter_ns();
                run(&command, &env, Stdio::piped());
                cold[l].push(Num::Float(py_round((clock.perf_counter_ns() - start) as f64 / 1e6, 4)));
                let want = report_cases.get(&case.name).unwrap().at(&labels[l]).unwrap().at("output_sha256").unwrap().as_str().unwrap();
                if sha256_hex(&read(&cold_out)) != want {
                    fail(&format!("RuntimeError: {}: a cold run of {} wrote a different output", case.name, labels[l]));
                }
            }
        }
        let mut per = Map::new();
        for l in 0..n {
            per.set(&labels[l], Json::obj(vec![("wall_ms", summary(&cold[l])), ("wall_samples_ms", Json::nums(&cold[l]))]));
        }
        cold_report.set(&case.name, Json::Obj(per));
        report.set("cold", Json::Obj(cold_report.clone()));
        for l in 0..n {
            say!("{:18} {:10} cold median={:8.2} ms", case.name, labels[l], median(&cold[l]).f());
        }
    }
    let _ = std::fs::remove_dir_all(&directory);
    *TMP.lock().unwrap() = None;
    env.clear();
    report.set("load_average_end", clock.loadavg());
    let mut report = Json::Obj(report);
    if !args.flag("--full-profile") {
        slim(&mut report, &stages);
    }
    std::fs::write(&output, dumps(&report) + "\n").unwrap_or_else(|e| fail(&format!("{output}: {e}")));
}

/// The cases a run measures: the suites in bench.py's order, then --case's
/// selection, or the usage error for a name no suite has.
fn select(paths: &Paths, directory: &str, suite: &str, wanted: &[String], cjk: Option<&str>) -> Vec<Case> {
    let mut cases = Vec::new();
    if suite == "all" || suite == "fonts" {
        cases.extend(font_workloads(paths, cjk));
    }
    if suite == "all" || suite == "legacy" {
        cases.extend(legacy_workloads(paths, directory));
    }
    if suite == "all" || suite == "draw" {
        cases.extend(draw_workloads(directory));
    }
    if suite == "all" || suite == "parser" {
        cases.extend(parser_workloads(directory, wanted));
    }
    if suite == "all" || suite == "text" {
        cases.extend(text_workloads(paths, directory, wanted));
    }
    if !wanted.is_empty() {
        let names: BTreeSet<&str> = cases.iter().map(|c| c.name.as_str()).collect();
        let unknown: BTreeSet<&str> = wanted.iter().map(|s| s.as_str()).filter(|n| !names.contains(n)).collect();
        if !unknown.is_empty() {
            let _ = std::fs::remove_dir_all(directory);
            arg_error(USAGE, PROG, &format!("unknown case: {}", unknown.into_iter().collect::<Vec<_>>().join(", ")));
        }
        cases.retain(|c| wanted.contains(&c.name));
    }
    cases
}

/// --list-workloads: one line per selected case, its shape and its input's
/// size and SHA-256, so a change to a generator shows. --dump-workloads DIR
/// (for checking a port against bench.py): each case's metadata as JSON in
/// DIR/cases.json and its input in DIR/inputs/.
fn list_workloads(paths: &Paths, suite: &str, wanted: &[String], cjk: Option<&str>, dump: Option<&str>) {
    let directory = make_tmp();
    *TMP.lock().unwrap() = Some(directory.clone());
    let cases = select(paths, &directory, suite, wanted, cjk);
    let mut listed = Vec::new();
    if let Some(dump) = dump {
        std::fs::create_dir_all(format!("{dump}/inputs")).unwrap_or_else(|e| fail(&format!("{dump}: {e}")));
    }
    for case in &cases {
        let data = read(&case.src);
        let sha = sha256_hex(&data);
        let input = paths.relative(&case.src);
        if let Some(dump) = dump {
            std::fs::write(format!("{dump}/inputs/{}.pty", case.name), &data).unwrap_or_else(|e| fail(&e.to_string()));
            let command: Vec<Json> = case
                .command(paths, "BIN", "OUT")
                .iter()
                .map(|a| Json::Str(a.replace(&directory, "TMP")))
                .collect();
            listed.push(Json::obj(vec![
                ("name", Json::str(&case.name)),
                ("group", Json::str(case.group)),
                ("px", Json::Int(case.px as i128)),
                ("cols", Json::Int(case.cols as i128)),
                ("rows", Json::Int(case.rows as i128)),
                ("cli", Json::str(if case.legacy { "positional" } else { "options" })),
                ("fonts", case.font_files(paths)),
                ("output", Json::str(if case.text { "text" } else { "png" })),
                ("input", Json::Str(input)),
                ("input_bytes", Json::Int(data.len() as i128)),
                ("input_sha256", Json::Str(sha)),
                ("checks", case.checks_json()),
                ("command", Json::Arr(command)),
            ]));
        } else {
            let checks: Vec<String> = case.checks.iter().map(|(k, op, v)| format!("{k}{op}{v}")).collect();
            let fonts: Vec<String> = case.fonts.iter().map(|f| f.replace(&paths.root, ".")).collect();
            say!(
                "{} {} {}px {}x{} {} {} {} {} {} [{}] [{}]",
                case.name,
                case.group,
                case.px,
                case.cols,
                case.rows,
                if case.legacy { "positional" } else { "options" },
                if case.text { "text" } else { "png" },
                input,
                data.len(),
                sha,
                fonts.join(" "),
                checks.join(" ")
            );
        }
    }
    if let Some(dump) = dump {
        std::fs::write(format!("{dump}/cases.json"), dumps(&Json::Arr(listed)) + "\n").unwrap_or_else(|e| fail(&e.to_string()));
    }
    let _ = std::fs::remove_dir_all(&directory);
}
