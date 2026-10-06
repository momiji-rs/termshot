//! termshot as an embedder uses it: this program links libtermshot.rlib
//! (`rustc --extern termshot=...`) and sees only its public API. It parses
//! every log tests/grids/ has a grid for and compares `Grid::to_text` and
//! `Grid::to_json` with those files, which tests/golden.rs checks the CLI's
//! `--text` and `--json` against: so the library and the CLI agree byte for
//! byte. It then reads cells, the cursor and casts through the API.
//!
//! Built and run by test.sh from the repo root.

use std::fs;
use std::process::ExitCode;

use termshot::{CursorShape, Lf, Palette, ParseOptions};

const GRIDS: &str = "tests/grids";

/// The log a grid comes from, as tests/golden.rs finds it.
fn source(log: &str) -> Option<String> {
    let candidates = [format!("examples/{log}.pty"), format!("tests/fixtures/{log}.pty"), format!("tests/fixtures/{log}.cast")];
    candidates.into_iter().find(|path| fs::metadata(path).is_ok())
}

/// The grid size a JSON grid starts with: {"cols":C,"rows":R,...
fn size(json: &str) -> Option<(usize, usize)> {
    let number = |key: &str| {
        let rest = &json[json.find(key)? + key.len()..];
        rest[..rest.find(|c: char| !c.is_ascii_digit())?].parse().ok()
    };
    Some((number("{\"cols\":")?, number(",\"rows\":")?))
}

/// The options tests/golden.rs renders `log` with that change its grid.
fn options(log: &str) -> Result<ParseOptions, String> {
    let mut options = ParseOptions::default();
    if log == "palette" {
        let path = "tests/fixtures/solarized-dark.conf";
        let file = fs::read(path).map_err(|error| format!("{path}: {error}"))?;
        options.palette = Palette::DEFAULT.with_file(&file).map_err(|error| format!("{path}: {error}"))?;
    }
    Ok(options)
}

/// The built-in font's cell at 24 px, in pixels: `./termshot --px 24 --size
/// 1x1` draws an 11x24 PNG. The library cannot measure a font yet (that is
/// the render's, and the CLI's), so a log whose grid depends on the cell
/// size is parsed with this one. tests/golden.rs draws every such log at
/// 24 px, among others, and checks that its grids agree at every size.
const CELL_24PX: (u16, u16) = (11, 24);

/// One log against its grids. Ok(true) when the log needs a cell size.
fn check(log: &str) -> Result<bool, String> {
    let read = |path: String| fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"));
    let (text, json) = (read(format!("{GRIDS}/{log}.txt"))?, read(format!("{GRIDS}/{log}.json"))?);
    let (cols, rows) = size(&json).ok_or(format!("{GRIDS}/{log}.json: no grid size"))?;
    let path = source(log).ok_or(format!("{log}: no log"))?;
    let mut data = fs::read(&path).map_err(|error| format!("{path}: {error}"))?;
    if termshot::is_cast(&data) {
        let cast = termshot::decode_cast(data).map_err(|reason| format!("{path}: {reason}"))?;
        data = cast.output;
    }
    let sized = termshot::needs_cell_size(&data);
    let grid = match sized {
        true => termshot::parse_with_cell_size(&data, cols, rows, &options(log)?, CELL_24PX),
        false => termshot::parse(&data, cols, rows, &options(log)?),
    };
    let grid = grid.map_err(|error| format!("{log}: {error}"))?;
    if (grid.cols(), grid.rows()) != (cols, rows) {
        return Err(format!("{log}: a {}x{} grid, not {cols}x{rows}", grid.cols(), grid.rows()));
    }
    for (what, got, want) in [("text", grid.to_text(), text), ("JSON", grid.to_json(), json)] {
        if got != want {
            return Err(format!("{log}: the library's {what} differs from {GRIDS}/{log}"));
        }
    }
    Ok(sized)
}

/// The accessors, on a log whose every answer is known.
fn accessors() -> Result<(), String> {
    let fail = |what: &str| Err(format!("accessors: {what}"));
    // Bold red "a", e and a combining acute, a wide character, then the
    // cursor shown as a bar at row 1, column 2.
    let log = "\x1b[1;31ma\x1b[me\u{301}\u{4e00}\r\n\x1b[4:2mxy\x1b[m\x1b[5 q".as_bytes();
    let parse = |log: &[u8], cols, rows, options: &ParseOptions| {
        termshot::parse(log, cols, rows, options).map_err(|error| format!("accessors: {error}"))
    };
    let grid = parse(log, 6, 3, &ParseOptions::default())?;
    let Some(a) = grid.cell(0, 0) else { return fail("no cell (0, 0)") };
    let named = Palette::DEFAULT.named;
    if a.ch() != 'a' || !a.is_bold() || a.fg() != named[1] || a.bg() != Palette::DEFAULT.background {
        return fail("cell (0, 0) is not a bold red a");
    }
    let e = grid.cell(0, 1).ok_or("accessors: no cell (0, 1)")?;
    // é has a precomposed form, so it is one character with no marks.
    if e.ch() != '\u{e9}' || e.marks().count() != 0 || e.is_bold() {
        return fail("cell (0, 1) is not a plain é");
    }
    let (wide, tail) = (grid.cell(0, 2).ok_or("no (0, 2)")?, grid.cell(0, 3).ok_or("no (0, 3)")?);
    if wide.ch() != '\u{4e00}' || !wide.is_wide() || !tail.is_wide_tail() || tail.ch() != ' ' || tail.is_wide() {
        return fail("cells (0, 2) and (0, 3) are not a wide character");
    }
    let x = grid.cell(1, 0).ok_or("no (1, 0)")?;
    if !x.is_double_underlined() || x.is_underlined() {
        return fail("cell (1, 0) is not double underlined");
    }
    if grid.cell(3, 0).is_some() || grid.cell(0, 6).is_some() {
        return fail("a cell outside the grid");
    }
    if grid.cursor() != Some((1, 2)) || grid.cursor_shape() != CursorShape::Bar {
        return fail("the cursor is not a bar at (1, 2)");
    }
    // A combining mark with no precomposed form stays a mark.
    let grid = parse("q\u{301}\x1b[?25l".as_bytes(), 2, 1, &ParseOptions::default())?;
    let q = grid.cell(0, 0).ok_or("no (0, 0)")?;
    if q.ch() != 'q' || q.marks().collect::<String>() != "\u{301}" || grid.cursor().is_some() {
        return fail("q with a combining acute, cursor hidden");
    }
    // A bare LF: down a row, and back to column 0 only with Lf::Newline.
    let text = |lf| parse(b"ab\ncd\n", 4, 3, &ParseOptions { lf, ..ParseOptions::default() }).map(|grid| grid.to_text());
    if text(Lf::Index)? != "ab\n  cd\n\n" || text(Lf::Newline)? != "ab\ncd\n\n" {
        return fail("bare LF");
    }
    if !termshot::lacks_cr(b"ab\ncd\n") || termshot::lacks_cr(b"ab\r\ncd\r\n") {
        return fail("lacks_cr");
    }
    // A 30x40 RGB kitty image (all black: 3600 zero bytes in base64) leaves
    // the cursor after it, which in cells of 10x20 pixels is 3 columns on
    // and a row down.
    let image = format!("\x1b_Ga=T,f=24,s=30,v=40,q=2;{}\x1b\\", "A".repeat(4800));
    if !termshot::needs_cell_size(image.as_bytes()) || termshot::needs_cell_size(b"plain text") {
        return fail("needs_cell_size");
    }
    let options = ParseOptions::default();
    let sized = |cell| {
        termshot::parse_with_cell_size(image.as_bytes(), 10, 4, &options, cell).map_err(|error| format!("accessors: {error}"))
    };
    let cursor = sized((10, 20))?.cursor();
    if cursor != Some((1, 3)) {
        return fail(&format!("the cursor after an image in 10x20 cells is at {cursor:?}"));
    }
    // A cell size of 0 counts as 1, as parse's cells are.
    if sized((0, 0))?.cursor() != parse(image.as_bytes(), 10, 4, &options)?.cursor() {
        return fail("a cell size of 0 is not 1");
    }
    Ok(())
}

/// A grid, its images included, can be parsed on one thread and read on
/// another: this fails to compile otherwise.
fn threads() -> Result<(), String> {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<termshot::Grid>();
    send_sync::<termshot::Error>();
    let log = format!("\x1b_Ga=T,f=24,s=30,v=40,q=2;{}\x1b\\text", "A".repeat(4800));
    let grid = std::thread::spawn(move || termshot::parse(log.as_bytes(), 10, 4, &ParseOptions::default()))
        .join()
        .map_err(|_| "threads: the parse panicked".to_string())?
        .map_err(|error| format!("threads: {error}"))?;
    match grid.to_text() {
        text if text.contains("text") => Ok(()),
        text => Err(format!("threads: {text:?}")),
    }
}

/// The grid sizes parse refuses, as errors.
fn sizes() -> Result<(), String> {
    let options = ParseOptions::default();
    let max = termshot::MAX_CELLS;
    for (cols, rows) in [(0, 30), (100, 0), (0, 0), (max + 1, 1), (1, max + 1), (2048, 2049), (usize::MAX, 2)] {
        match termshot::parse(b"x", cols, rows, &options) {
            Err(error @ termshot::Error::GridSize { .. }) if error == termshot::Error::GridSize { cols, rows } => {
                if !error.to_string().contains(&format!("{cols}x{rows}")) {
                    return Err(format!("sizes: {cols}x{rows} reads {error}"));
                }
            }
            Err(error) => return Err(format!("sizes: {cols}x{rows} is {error:?}")),
            Ok(_) => return Err(format!("sizes: a {cols}x{rows} grid parses")),
        }
    }
    let one = termshot::parse(b"x", 1, 1, &options).map_err(|error| format!("sizes: 1x1: {error}"))?;
    if one.to_text() != "x\n" {
        return Err("sizes: 1x1".into());
    }
    Ok(())
}

/// A cast through the API: its size and output.
fn cast() -> Result<(), String> {
    let path = "tests/fixtures/asciicast-v3.cast";
    let data = fs::read(path).map_err(|error| format!("{path}: {error}"))?;
    if !termshot::is_cast(&data) || termshot::is_cast(b"\x1b[31mred") {
        return Err("is_cast".into());
    }
    let cast = termshot::decode_cast(data).map_err(|reason| format!("{path}: {reason}"))?;
    if cast.version != 3 || cast.output.is_empty() {
        return Err(format!("{path}: version {}, {} bytes of output", cast.version, cast.output.len()));
    }
    let Err(termshot::Error::Cast(reason)) = termshot::decode_cast(b"{\"version\": 2}\n".to_vec()) else {
        return Err("a header without a size is not an Error::Cast".into());
    };
    if reason.is_empty() {
        return Err("no reason for a bad header".into());
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut logs: Vec<String> = match fs::read_dir(GRIDS) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok()?.strip_suffix(".txt").map(str::to_owned))
            .collect(),
        Err(error) => {
            eprintln!("{GRIDS}: {error}");
            return ExitCode::FAILURE;
        }
    };
    logs.sort();
    let mut sized = Vec::new();
    for log in &logs {
        match check(log) {
            Ok(false) => {}
            Ok(true) => sized.push(log.as_str()),
            Err(error) => {
                println!("FAIL {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    for test in [accessors, sizes, threads, cast] {
        if let Err(error) = test() {
            println!("FAIL {error}");
            return ExitCode::FAILURE;
        }
    }
    println!("ok, {} grids as the CLI writes them, {} with a cell size ({})", logs.len(), sized.len(), sized.join(" "));
    ExitCode::SUCCESS
}
