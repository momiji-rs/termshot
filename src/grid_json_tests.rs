//! --json's shape, checked against --text: every committed grid in
//! tests/grids/ (test.sh runs this again after --update-goldens rewrites
//! them), the grids of random logs, and grids broken on purpose, each of
//! which the check must refuse.

use super::*;
use crate::cast::{parse, Value};
use crate::grid::grid_text;

/// The grid the CLI replays log into, or with --lf-newline.
fn grid_of(log: &[u8], cols: usize, rows: usize, lf: Lf) -> Grid {
    replay_with(log, cols, rows, &ParseOptions { lf, ..ParseOptions::default() }, (1, 1)).unwrap()
}

/// The members of a JSON object, or why it isn't one.
fn members<'v, 'a>(value: &'v Value<'a>, what: &str) -> Result<&'v [(String, Value<'a>)], String> {
    match value {
        Value::Object(members) => Ok(members),
        _ => Err(format!("{what} is not an object")),
    }
}

/// The value of key, which must be there.
fn field<'v, 'a>(members: &'v [(String, Value<'a>)], key: &str, what: &str) -> Result<&'v Value<'a>, String> {
    members.iter().find(|(k, _)| k == key).map(|(_, v)| v).ok_or_else(|| format!("{what} has no \"{key}\""))
}

/// Only these keys, so a misspelled one is caught.
fn only(members: &[(String, Value)], keys: &[&str], what: &str) -> Result<(), String> {
    match members.iter().find(|(k, _)| !keys.contains(&k.as_str())) {
        Some((k, _)) => Err(format!("{what} has an unknown key \"{k}\"")),
        None => Ok(()),
    }
}

fn whole(value: &Value, what: &str) -> Result<usize, String> {
    match value {
        Value::Number(n) if n.bytes().all(|b| b.is_ascii_digit()) && (n.len() == 1 || !n.starts_with('0')) => {
            n.parse().map_err(|_| format!("{what} {n} is too large"))
        }
        _ => Err(format!("{what} is not a whole number")),
    }
}

fn string<'v>(value: &'v Value, what: &str) -> Result<&'v str, String> {
    match value {
        Value::String(s) => Ok(s),
        _ => Err(format!("{what} is not a string")),
    }
}

fn array<'v, 'a>(value: &'v Value<'a>, what: &str) -> Result<&'v [Value<'a>], String> {
    match value {
        Value::Array(items) => Ok(items),
        _ => Err(format!("{what} is not an array")),
    }
}

/// A JSON number, with a sign. No leading zero, no fraction.
fn integer(value: &Value, what: &str) -> Result<i64, String> {
    let Value::Number(n) = value else {
        return Err(format!("{what} is not an integer"));
    };
    let digits = n.strip_prefix('-').unwrap_or(n);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || (digits.len() > 1 && digits.starts_with('0')) {
        return Err(format!("{what} {n} is not an integer"));
    }
    n.parse().map_err(|_| format!("{what} {n} is too large"))
}

/// `images`, when --json wrote it: each image's pixels and the placements
/// that show it. Absent when the screen has none.
fn check_images(value: &Value) -> Result<(), String> {
    let images = array(value, "images")?;
    if images.is_empty() {
        return Err("images is empty; --json leaves the key out then".into());
    }
    for (i, image) in images.iter().enumerate() {
        let image = members(image, &format!("image {i}"))?;
        only(image, &["id", "width", "height", "format", "placements"], &format!("image {i}"))?;
        if image.iter().any(|(key, _)| key == "id") {
            whole(field(image, "id", &format!("image {i}"))?, &format!("image {i}: id"))?;
        }
        whole(field(image, "width", &format!("image {i}"))?, &format!("image {i}: width"))?;
        whole(field(image, "height", &format!("image {i}"))?, &format!("image {i}: height"))?;
        let format = string(field(image, "format", &format!("image {i}"))?, &format!("image {i}: format"))?;
        if format != "rgba" {
            return Err(format!("image {i}: format {format}"));
        }
        let placements = array(field(image, "placements", &format!("image {i}"))?, &format!("image {i}: placements"))?;
        if placements.is_empty() {
            return Err(format!("image {i} has no placement"));
        }
        for (j, placement) in placements.iter().enumerate() {
            let what = format!("image {i}: placement {j}");
            let placement = members(placement, &what)?;
            only(
                placement,
                &["id", "src", "col", "row", "columns", "rows", "z", "virtual", "relative", "sixel"],
                &what,
            )?;
            if placement.iter().any(|(key, _)| key == "id") {
                whole(field(placement, "id", &what)?, &format!("{what}: id"))?;
            }
            let src = members(field(placement, "src", &what)?, &format!("{what}: src"))?;
            only(src, &["x", "y", "w", "h"], &format!("{what}: src"))?;
            for key in ["x", "y", "w", "h"] {
                whole(field(src, key, &format!("{what}: src"))?, &format!("{what}: src {key}"))?;
            }
            for key in ["col", "row", "z"] {
                integer(field(placement, key, &what)?, &format!("{what}: {key}"))?;
            }
            for key in ["columns", "rows"] {
                whole(field(placement, key, &what)?, &format!("{what}: {key}"))?;
            }
            for flag in ["virtual", "relative", "sixel"] {
                if let Some(value) = placement.iter().find(|(key, _)| key == flag).map(|(_, value)| value) {
                    if *value != Value::Bool(true) {
                        return Err(format!("{what}: {flag} is not true"));
                    }
                }
            }
        }
    }
    Ok(())
}

/// That json is what --json promises and spells the rows of text, the
/// --text output of the same screen: the size, the cursor on the grid, and
/// per row runs that cover it from column 0, each starting where the one
/// before ends (a wide character takes two columns, but one on a one-column
/// screen, and a combining mark none),
/// with #rrggbb colours and only true style flags, whose text joined and
/// trimmed of trailing spaces is the row. `images`, when present, is the
/// screen's images and their placements.
fn check_grid(json: &str, text: &str) -> Result<(), String> {
    let grid = parse(json).map_err(|e| format!("bad JSON at byte {}: {}", e.at, e.reason))?;
    let grid = members(&grid, "the grid")?;
    only(grid, &["cols", "rows", "cursor", "lines", "images"], "the grid")?;
    if let Some(images) = grid.iter().find(|(key, _)| key == "images").map(|(_, value)| value) {
        check_images(images)?;
    }
    let cols = whole(field(grid, "cols", "the grid")?, "cols")?;
    let rows = whole(field(grid, "rows", "the grid")?, "rows")?;
    let lines = array(field(grid, "lines", "the grid")?, "lines")?;
    let Some(text_rows) = text.strip_suffix('\n') else {
        return Err("the text does not end with a newline".into());
    };
    let text_rows: Vec<&str> = text_rows.split('\n').collect();
    if lines.len() != rows || text_rows.len() != rows {
        return Err(format!("row count: {} lines, {rows} rows, {} text rows", lines.len(), text_rows.len()));
    }
    match field(grid, "cursor", "the grid")? {
        Value::Null => {}
        cursor => {
            let cursor = members(cursor, "the cursor")?;
            only(cursor, &["col", "row", "shape"], "the cursor")?;
            let col = whole(field(cursor, "col", "the cursor")?, "the cursor's col")?;
            let row = whole(field(cursor, "row", "the cursor")?, "the cursor's row")?;
            if col >= cols || row >= rows {
                return Err(format!("cursor at {col},{row} is off the {cols}x{rows} grid"));
            }
            let shape = string(field(cursor, "shape", "the cursor")?, "the cursor's shape")?;
            if !["block", "underline", "bar"].contains(&shape) {
                return Err(format!("cursor shape {shape}"));
            }
        }
    }
    const FLAGS: [&str; 5] = ["bold", "italic", "underline", "double_underline", "strike"];
    for (r, (runs, row)) in lines.iter().zip(&text_rows).enumerate() {
        let runs = array(runs, &format!("row {r}"))?;
        let mut end = 0;
        let mut joined = String::new();
        for run in runs {
            let run = members(run, &format!("row {r}: a run"))?;
            let what = format!("row {r}: the run at col {end}");
            only(run, &["col", "text", "fg", "bg", "bold", "italic", "underline", "double_underline", "strike"], &what)?;
            let col = whole(field(run, "col", &what)?, &format!("{what}: col"))?;
            if col != end {
                return Err(format!("row {r}: run at col {col}, want {end}"));
            }
            let run_text = string(field(run, "text", &what)?, &format!("{what}: text"))?;
            if run_text.is_empty() {
                return Err(format!("row {r}: empty run at col {col}"));
            }
            for key in ["fg", "bg"] {
                let color = string(field(run, key, &what)?, &format!("{what}: {key}"))?;
                let hex = color.strip_prefix('#').unwrap_or("");
                if hex.len() != 6 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                    return Err(format!("row {r}: {key} {color}"));
                }
            }
            for flag in FLAGS {
                if let Some(value) = run.iter().find(|(k, _)| k == flag).map(|(_, v)| v) {
                    if *value != Value::Bool(true) {
                        return Err(format!("{what}: {flag} is not true"));
                    }
                }
            }
            // On a one-column screen a wide character is narrow (#18).
            let width = |ch: char| unicode::width(ch as u32).min(if cols == 1 { 1 } else { 2 });
            end = col + run_text.chars().map(width).sum::<usize>();
            joined.push_str(run_text);
        }
        if end > cols {
            return Err(format!("row {r}: runs end at col {end}, past the last column"));
        }
        // --text trims every trailing space; --json keeps those that show.
        if joined.trim_end_matches(' ') != *row {
            return Err(format!("row {r}: text {joined:?} differs from --text {row:?}"));
        }
    }
    Ok(())
}

/// The grid directory: tests/grids, or $TERMSHOT_GRIDS.
fn grids_dir() -> String {
    std::env::var("TERMSHOT_GRIDS").unwrap_or_else(|_| "tests/grids".into())
}

#[test]
fn golden_grid_json_agrees_with_text() {
    let dir = grids_dir();
    let mut names: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().map_or(false, |e| e == "json"))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no grids in {dir}");
    let failures: Vec<String> = names
        .iter()
        .filter_map(|json_path| {
            let json = fs::read_to_string(json_path);
            let text = fs::read_to_string(json_path.with_extension("txt"));
            let result = match (json, text) {
                (Ok(json), Ok(text)) => check_grid(&json, &text),
                (Err(e), _) | (_, Err(e)) => Err(e.to_string()),
            };
            result.err().map(|e| format!("FAIL {}: {e}", json_path.display()))
        })
        .collect();
    assert!(failures.is_empty(), "{} of {} grids:\n{}", failures.len(), names.len(), failures.join("\n"));
}

/// A small deterministic generator for the random logs.
struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Logs of everything that shapes a row's runs: ASCII, wide characters
/// (also at the last column), combining marks, default ignorables, colours,
/// each style flag, erases, cursor moves, tabs and a hidden cursor.
fn random_log(rng: &mut Xorshift) -> Vec<u8> {
    const PIECES: &[&str] = &[
        "a", "Z", " ", "  ", "\"", "\\", "\x7f", "\t", "\r\n", "\r", "\n", "\x08", "東", "漢字", "😀", "e\u{301}",
        "\u{301}", "a\u{300}\u{301}\u{302}\u{303}\u{304}", "\u{200b}", "\u{ad}", "\u{1160}", "\u{3099}", "\u{1f1e6}",
        "\x1b[1m", "\x1b[3m", "\x1b[4m", "\x1b[4:2m", "\x1b[9m", "\x1b[0m", "\x1b[m", "\x1b[31m", "\x1b[44m",
        "\x1b[38;2;1;2;3m", "\x1b[48;5;200m", "\x1b[7m", "\x1b[K", "\x1b[1K", "\x1b[2J", "\x1b[H", "\x1b[2;3H",
        "\x1b[5C", "\x1b[3D", "\x1b[?25l", "\x1b[?25h", "\x1b[4 q", "\x1b[6 q", "\x1b(0qx\x1b(B", "\x1b[2@", "\x1b[P",
    ];
    let mut log = Vec::new();
    for _ in 0..rng.below(120) {
        log.extend_from_slice(PIECES[rng.below(PIECES.len())].as_bytes());
    }
    log
}

#[test]
fn random_grids_json_agrees_with_text() {
    let mut rng = Xorshift(0x9e37_79b9_7f4a_7c15);
    for case in 0..2000 {
        let log = random_log(&mut rng);
        let (cols, rows) = (1 + rng.below(12), 1 + rng.below(5));
        let lf = if rng.below(2) == 0 { Lf::Index } else { Lf::Newline };
        let grid = grid_of(&log, cols, rows, lf);
        let (json, text) = (grid.to_json(), grid.to_text());
        if let Err(e) = check_grid(&json, &text) {
            panic!("case {case}, {cols}x{rows}, log {:?}: {e}\n{json}", String::from_utf8_lossy(&log));
        }
    }
}

/// A grid with a wide character, a mark, a style and a cursor, then edits
/// of its JSON or text that each break one promise.
#[test]
fn grid_json_check_refuses_what_json_does_not_promise() {
    let grid = grid_of("ab東e\u{301}\x1b[1mX\x1b[m\r\n\x1b[44m \x1b[m".as_bytes(), 8, 2, Lf::Index);
    let (json, text) = (grid.to_json(), grid.to_text());
    check_grid(&json, &text).unwrap();
    assert!(json.contains("\"bold\":true") && json.contains("東"), "{json}");
    let cases: &[(&str, &str, &str)] = &[
        // (what is broken, the JSON fragment, its replacement)
        ("bad JSON", "]}\n", "],}\n"),
        ("an unknown key", "\"rows\":2", "\"rows\":2,\"extra\":1"),
        ("a missing key", "\"cols\":8,", ""),
        ("too few rows", "\"rows\":2", "\"rows\":3"),
        ("a cursor off the grid", "\"col\":1,\"row\":1", "\"col\":8,\"row\":1"),
        ("a cursor shape", "\"block\"", "\"beam\""),
        ("a gap between runs", "{\"col\":5,", "{\"col\":6,"),
        ("an overlap", "{\"col\":5,", "{\"col\":4,"),
        ("a first run not at 0", "[{\"col\":0,\"text\":\"ab", "[{\"col\":1,\"text\":\"ab"),
        ("an empty run", "\"text\":\"X\"", "\"text\":\"\""),
        ("a colour", "\"bg\":\"#", "\"bg\":\"x"),
        ("an upper-case colour", "#111823", "#11182F"),
        ("a false flag", "\"bold\":true", "\"bold\":false"),
        ("text that differs", "\"text\":\"X\"", "\"text\":\"Y\""),
        ("a wide character counted once", "東", "x"),
        ("runs past the last column", "\"cols\":8", "\"cols\":5"),
        ("a negative column", "{\"col\":5,", "{\"col\":-6,"),
    ];
    for &(what, from, to) in cases {
        assert!(json.contains(from), "{what}: {from:?} not in {json}");
        let broken = json.replacen(from, to, 1);
        assert!(check_grid(&broken, &text).is_err(), "{what} passed:\n{broken}");
    }
    let rows: Vec<&str> = text.split('\n').collect();
    for (what, broken) in [
        ("a text row that differs", text.replacen('a', "A", 1)),
        ("a missing final newline", text.trim_end_matches('\n').to_string()),
        ("an extra text row", format!("{text}x\n")),
        ("a trailing space kept", format!("{} \n{}\n", rows[0], rows[1])),
    ] {
        assert!(check_grid(&json, &broken).is_err(), "{what} passed:\n{broken:?}");
    }
    // The text is grid_text's, as --text writes it.
    assert_eq!(text, grid_text(&grid.cells, &grid.marks, grid.cols));
}

#[test]
fn json_omits_images_when_the_screen_has_none() {
    let grid = grid_of(b"hi\r\n", 4, 2, Lf::Index);
    assert!(!grid.to_json().contains("images"), "{}", grid.to_json());
    assert!(grid.images().is_empty());
}

#[test]
fn json_lists_each_strip_of_an_image() {
    let log = fs::read("tests/fixtures/kitty-strips.pty").unwrap();
    let grid = replay_with(&log, 10, 5, &ParseOptions::default(), (13, 29)).unwrap();
    let images = grid.images();
    assert_eq!(images.len(), 1, "{images:?}");
    let image = &images[0];
    assert_eq!(image.id, Some(7));
    assert_eq!((image.width, image.height), (32, 96));
    assert_eq!(image.placements.len(), 3);
    for (i, placement) in image.placements.iter().enumerate() {
        let i = i as u32;
        assert_eq!(placement.id, Some(i + 1));
        assert_eq!((placement.x, placement.y, placement.w, placement.h), (0, i * 32, 32, 32));
        assert_eq!((placement.col, placement.row, placement.columns, placement.rows), (1, 1 + i64::from(i), 2, 1));
        assert_eq!(placement.z, 0);
        assert!(!placement.virtual_placement && !placement.relative && !placement.sixel);
    }
    let json = grid.to_json();
    check_grid(&json, &grid.to_text()).unwrap();
    assert!(json.contains("\"format\":\"rgba\"") && json.contains("\"y\":64"), "{json}");
}

#[test]
fn json_marks_sixel_placeholder_and_relative_placements() {
    let sixel = replay_with(b"\x1bPq\"1;1;1;1@\x1b\\", 10, 4, &ParseOptions::default(), (8, 16)).unwrap();
    let images = sixel.images();
    assert_eq!(images.len(), 1, "{images:?}");
    assert_eq!(images[0].id, None);
    assert!(images[0].placements[0].sixel && !images[0].placements[0].relative);
    assert!(sixel.to_json().contains("\"sixel\":true"));
    check_grid(&sixel.to_json(), &sixel.to_text()).unwrap();

    let relative = fs::read("tests/fixtures/kitty-relative.pty").unwrap();
    let grid = replay_with(&relative, 40, 12, &ParseOptions::default(), (8, 16)).unwrap();
    assert!(grid.images().iter().any(|image| image.placements.iter().any(|placement| placement.relative)));
    check_grid(&grid.to_json(), &grid.to_text()).unwrap();

    let placeholders = fs::read("tests/fixtures/kitty-placeholders.pty").unwrap();
    let grid = replay_with(&placeholders, 40, 12, &ParseOptions::default(), (8, 16)).unwrap();
    assert!(grid.images().iter().any(|image| image.placements.iter().any(|placement| placement.virtual_placement)));
    check_grid(&grid.to_json(), &grid.to_text()).unwrap();
}

#[test]
fn json_cell_span_follows_the_cell_when_the_command_leaves_it() {
    // kitten icat places this 40x20 image in native pixels, with no c and r.
    // 11x24 (px 24) covers 4 columns; 22x48 (px 47.5) covers 2. The text does
    // not move, which is why the two goldens share one grid file.
    let log = fs::read("tests/fixtures/kitty-icat-animation.pty").unwrap();
    let at = |cell| replay_with(&log, 20, 6, &ParseOptions::default(), cell).unwrap();
    let narrow = at((11, 24));
    let wide = at((22, 48));
    assert_eq!(narrow.to_text(), wide.to_text());
    let span = |grid: &Grid| {
        let image = &grid.images()[0];
        let placement = &image.placements[0];
        (
            image.id,
            image.width,
            image.height,
            placement.x,
            placement.y,
            placement.w,
            placement.h,
            placement.col,
            placement.row,
            placement.columns,
            placement.rows,
        )
    };
    assert_eq!(span(&narrow), (Some(1), 40, 20, 0, 0, 40, 20, 1, 1, 4, 1));
    assert_eq!(span(&wide), (Some(1), 40, 20, 0, 0, 40, 20, 1, 1, 2, 1));
}
