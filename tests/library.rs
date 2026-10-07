//! termshot as an embedder uses it: this program links libtermshot.rlib
//! (`rustc --extern termshot=...`) and sees only its public API.
//!
//! - It parses every log tests/grids/ has a grid for and compares
//!   `Grid::to_text` and `Grid::to_json` with those files, which
//!   tests/golden.rs checks the CLI's `--text` and `--json` against.
//! - It draws every pixel golden's case (tests/golden_cases.rs) with
//!   `termshot::render` and compares the PNG, byte for byte, with the one
//!   the CLI wrote to target/test/ for tests/golden.rs.
//! - It reads cells, the cursor and casts through the API, returns every
//!   error the API has, renders on twelve threads at once, and renders
//!   hostile logs and options, which must return Ok or an error, never
//!   panic or abort.
//!
//! With `--faults`, linked to a library built with `--cfg
//! termshot_alloc_faults` (tests/run.sh), it fails each of a parse's and a
//! render's allocations in turn instead: each must return
//! `Error::OutOfMemory`.
//!
//! Built and run by test.sh, after the goldens, from the repo root.

use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::ExitCode;

use termshot::{
    Cursor, CursorShape, Error, FaceSelector, Font, FontSpec, Grid, Lf, Palette, ParseOptions, RenderOptions,
};

mod golden_cases;

const GRIDS: &str = "tests/grids";
const OUT: &str = "target/test";
const FONT: &str = "third_party/jetbrains-mono/JetBrainsMono-Regular.ttf";

/// The log a grid comes from, as tests/golden.rs finds it.
fn source(log: &str) -> Option<String> {
    let candidates = [format!("examples/{log}.pty"), format!("tests/fixtures/{log}.pty"), format!("tests/fixtures/{log}.cast")];
    candidates.into_iter().find(|path| fs::metadata(path).is_ok())
}

/// The log's bytes, a cast's output events for a cast.
fn read_log(log: &str) -> Result<Vec<u8>, String> {
    let path = source(log).ok_or(format!("{log}: no log"))?;
    let data = fs::read(&path).map_err(|error| format!("{path}: {error}"))?;
    if !termshot::is_cast(&data) {
        return Ok(data);
    }
    Ok(termshot::decode_cast(data).map_err(|reason| format!("{path}: {reason}"))?.output)
}

/// The grid size a JSON grid starts with: {"cols":C,"rows":R,...
fn size(json: &str) -> Option<(usize, usize)> {
    let number = |key: &str| {
        let rest = &json[json.find(key)? + key.len()..];
        rest[..rest.find(|c: char| !c.is_ascii_digit())?].parse().ok()
    };
    Some((number("{\"cols\":")?, number(",\"rows\":")?))
}

/// The palette a --palette file gives.
fn palette(path: &str) -> Result<Palette, String> {
    let file = fs::read(path).map_err(|error| format!("{path}: {error}"))?;
    Palette::DEFAULT.with_file(&file).map_err(|error| format!("{path}: {error}"))
}

/// The options tests/golden.rs renders `log` with that change its grid.
fn options(log: &str) -> Result<ParseOptions, String> {
    let mut options = ParseOptions::default();
    if log == "palette" {
        options.palette = palette("tests/fixtures/solarized-dark.conf")?;
    }
    Ok(options)
}

/// The built-in font's cell at 24 px, in pixels: a log whose grid depends
/// on the cell size is parsed with it. tests/golden.rs draws every such log
/// at 24 px, among others, and checks that its grids agree at every size.
fn cell_24px() -> Result<(u32, u32), String> {
    let font = Font::embedded().map_err(|error| format!("the built-in font: {error}"))?;
    let cell = font.cell_size(24.0).map_err(|error| format!("cell_size: {error}"))?;
    // `./termshot --px 24 --size 1x1` draws an 11x24 PNG.
    if cell != (11, 24) {
        return Err(format!("the built-in font's cell at 24 px is {cell:?}, not 11x24"));
    }
    Ok(cell)
}

/// One log against its grids. Ok(true) when the log needs a cell size.
fn check(log: &str, cell: (u32, u32)) -> Result<bool, String> {
    let read = |path: String| fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"));
    let (text, json) = (read(format!("{GRIDS}/{log}.txt"))?, read(format!("{GRIDS}/{log}.json"))?);
    let (cols, rows) = size(&json).ok_or(format!("{GRIDS}/{log}.json: no grid size"))?;
    let data = read_log(log)?;
    let sized = termshot::needs_cell_size(&data);
    let grid = match sized {
        true => termshot::parse_with_cell_size(&data, cols, rows, &options(log)?, cell),
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

/// A font as the CLI opens a --font value.
fn open(value: &str) -> Result<Font, String> {
    let spec = FontSpec::parse(value).map_err(|error| format!("{value}: {error}"))?;
    Font::open(&spec).map_err(|error| error.to_string())
}

/// One pixel golden's case, drawn through the library as the CLI draws it:
/// the PNG must be the one tests/golden.rs had the CLI write.
fn case_png(log: &str, px: &str, cols: u32, rows: u32, args: &[&str]) -> Result<(), String> {
    let name = format!("{log} {px}");
    let data = read_log(log)?;
    let mut parse_options = ParseOptions::default();
    let (mut font, mut fallback, mut padding) = (None, None, (0, 0));
    // With no options, the original form, which names the font file.
    if args.is_empty() {
        font = Some(open(FONT)?);
    }
    for pair in args.chunks(2) {
        match pair {
            ["--font", value] => font = Some(open(value)?),
            ["--fallback-font", value] => fallback = Some(open(value)?),
            ["--palette", path] => parse_options.palette = palette(path)?,
            ["--padding", value] => {
                let (x, y) = value.split_once(',').ok_or(format!("{name}: padding {value}"))?;
                padding = (x.parse().map_err(|_| format!("{name}: {x}"))?, y.parse().map_err(|_| format!("{name}: {y}"))?);
            }
            _ => return Err(format!("{name}: an option this test does not know, {pair:?}")),
        }
    }
    let font = match font {
        Some(font) => font,
        None => Font::embedded().map_err(|error| error.to_string())?,
    };
    let size: f64 = px.parse().map_err(|_| format!("{name}: px"))?;
    let options = RenderOptions { px: size, font: Some(&font), fallback: fallback.as_ref(), padding, ..RenderOptions::default() };
    let cell = font.cell_size(size).map_err(|error| format!("{name}: {error}"))?;
    let grid = termshot::parse_with_cell_size(&data, cols as usize, rows as usize, &parse_options, cell)
        .map_err(|error| format!("{name}: {error}"))?;
    let rendered = termshot::render(&grid, &options).map_err(|error| format!("{name}: {error}"))?;
    let path = format!("{OUT}/{log}-{px}.png");
    let cli = fs::read(&path).map_err(|error| format!("{path}: {error}"))?;
    if rendered.png != cli {
        return Err(format!("{name}: the library's PNG ({} bytes) differs from {path} ({} bytes)", rendered.png.len(), cli.len()));
    }
    let ihdr = |at: usize| u32::from_be_bytes([cli[at], cli[at + 1], cli[at + 2], cli[at + 3]]);
    if (rendered.width, rendered.height) != (ihdr(16), ihdr(20)) {
        return Err(format!("{name}: Rendered says {}x{}", rendered.width, rendered.height));
    }
    Ok(())
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
    // A cell size of 0 counts as 1, as parse's cells are; one past i32
    // counts as i32::MAX, as wide as a cell can be.
    if sized((0, 0))?.cursor() != parse(image.as_bytes(), 10, 4, &options)?.cursor() {
        return fail("a cell size of 0 is not 1");
    }
    if sized((u32::MAX, u32::MAX))?.cursor() != Some((0, 1)) {
        return fail("a cell of u32::MAX pixels");
    }
    // The cursor can be moved, hidden, and reshaped, as --cursor and
    // --cursor-shape do, and --json shows it.
    let mut grid = parse(b"ab", 4, 2, &ParseOptions::default())?;
    grid.set_cursor(Some((1, 3))).map_err(|error| format!("accessors: {error}"))?;
    grid.set_cursor_shape(CursorShape::Underline);
    if grid.cursor() != Some((1, 3)) || grid.cursor_shape() != CursorShape::Underline
        || !grid.to_json().contains("\"cursor\":{\"col\":3,\"row\":1,\"shape\":\"underline\"}")
    {
        return fail(&format!("set_cursor: {}", grid.to_json()));
    }
    grid.set_cursor(None).map_err(|error| format!("accessors: {error}"))?;
    if grid.cursor().is_some() || !grid.to_json().contains("\"cursor\":null") {
        return fail("set_cursor(None)");
    }
    if "bar".parse::<CursorShape>() != Ok(CursorShape::Bar) {
        return fail("CursorShape::from_str");
    }
    Ok(())
}

/// A font through the API: from bytes, a face of a collection by number and
/// by name, an instance, and what it says about itself.
fn fonts() -> Result<(), String> {
    let bytes = fs::read(FONT).map_err(|error| format!("{FONT}: {error}"))?;
    let font = Font::from_bytes(bytes, &FaceSelector::default()).map_err(|error| format!("fonts: {error}"))?;
    let embedded = Font::embedded().map_err(|error| format!("fonts: {error}"))?;
    if font.cell_size(48.0) != embedded.cell_size(48.0) || font.face().is_some() || font.hint().is_some() {
        return Err("fonts: the font file is not the built-in font".into());
    }
    if font.color_bitmap().is_some() || font.instance().is_some() {
        return Err("fonts: JetBrains Mono as a color bitmap font or an instance".into());
    }
    let selector = FaceSelector::parse("1#wght=700").map_err(|error| format!("fonts: {error}"))?;
    if selector != (FaceSelector { face: Some("1".into()), axes: Some("wght=700".into()) }) {
        return Err(format!("fonts: FaceSelector::parse gives {selector:?}"));
    }
    if FaceSelector::parse("Noto Sans").map(|s| s.face) != Ok(Some("Noto Sans".into())) {
        return Err("fonts: FaceSelector::parse of a name".into());
    }
    // The variable CJK subset at an instance: the CLI's -v names it.
    let vf = "third_party/noto-sans-cjk-vf/NotoSansCJKtc-VF-Subset.otf";
    let spec = FontSpec::parse(&format!("{vf}#wght=700")).map_err(|error| format!("fonts: {error}"))?;
    if spec.path() != vf || spec.selector().axes.as_deref() != Some("wght=700") || spec.name() != format!("{vf}#wght=700") {
        return Err(format!("fonts: {spec:?}"));
    }
    let bold = Font::open(&spec).map_err(|error| format!("fonts: {error}"))?;
    if bold.instance().is_none() {
        return Err("fonts: an instance with no settings".into());
    }
    let vf_bytes = fs::read(vf).map_err(|error| format!("{vf}: {error}"))?;
    let same = Font::from_bytes(vf_bytes, &spec.selector()).map_err(|error| format!("fonts: {error}"))?;
    if same.instance() != bold.instance() {
        return Err("fonts: from_bytes and open pick another instance".into());
    }
    Ok(())
}

/// Every error the API returns, with the CLI's message.
fn errors() -> Result<(), String> {
    let fail = |what: String| Err(format!("errors: {what}"));
    let grid = termshot::parse(b"x", 4, 2, &ParseOptions::default()).map_err(|error| format!("errors: {error}"))?;
    let render = |options: RenderOptions| termshot::render(&grid, &options).map(|_| ());
    // Options out of range.
    for px in [0.0, -1.0, 256.0, f64::NAN, f64::INFINITY] {
        match render(RenderOptions { px, ..RenderOptions::default() }) {
            Err(Error::Options(message)) if message.contains("px must be a number above 0 and below 256") => {}
            other => return fail(format!("px {px}: {other:?}")),
        }
    }
    let padding = termshot::MAX_PADDING + 1;
    match render(RenderOptions { padding: (0, padding), ..RenderOptions::default() }) {
        Err(Error::Options(message)) if message.contains("padding") => {}
        other => return fail(format!("padding {padding}: {other:?}")),
    }
    for (row, col) in [(2, 0), (0, 4), (usize::MAX, usize::MAX)] {
        match render(RenderOptions { cursor: Cursor::At { row, col }, ..RenderOptions::default() }) {
            Err(Error::Options(message)) if message.contains("off the 4x2 grid") => {}
            other => return fail(format!("the cursor at ({row}, {col}): {other:?}")),
        }
    }
    let mut moved = termshot::parse(b"x", 4, 2, &ParseOptions::default()).map_err(|error| format!("errors: {error}"))?;
    if !matches!(moved.set_cursor(Some((2, 0))), Err(Error::Options(_))) || moved.cursor() != Some((0, 1)) {
        return fail("set_cursor off the grid".into());
    }
    match "beam".parse::<CursorShape>() {
        Err(Error::Options(message)) if message == "cursor shape must be block, underline or bar, not \"beam\"" => {}
        other => return fail(format!("CursorShape \"beam\": {other:?}")),
    }
    if !matches!(Font::embedded().map(|font| font.cell_size(0.0)), Ok(Err(Error::Options(_)))) {
        return fail("cell_size(0)".into());
    }
    // Fonts that can't be used.
    let font_error = |result: Result<Font, Error>, says: &str| match result {
        Err(Error::Font(message)) if message.contains(says) => Ok(()),
        other => Err(format!("errors: a font that should say {says:?}: {:?}", other.map(|_| ()))),
    };
    font_error(Font::from_bytes(b"#!/bin/sh\n".to_vec(), &FaceSelector::default()), "font: not a usable font: ")?;
    let bytes = fs::read(FONT).map_err(|error| format!("{FONT}: {error}"))?;
    let second = FaceSelector { face: Some("2".into()), axes: None };
    font_error(Font::from_bytes(bytes.clone(), &second), "font is a single font, so its only face is #0, not #2")?;
    let axes = FaceSelector { face: None, axes: Some("wght=700".into()) };
    font_error(Font::from_bytes(bytes, &axes), "font: cannot set wght=700: termshot varies CFF2 outlines only")?;
    let missing = FontSpec::parse("target/test/no-such-font.ttf").map_err(|error| format!("errors: {error}"))?;
    font_error(Font::open(&missing), "target/test/no-such-font.ttf: ")?;
    match FaceSelector::parse("wght=bold") {
        Err(Error::Font(message)) if message.starts_with("#wght=bold: ") => {}
        other => return fail(format!("FaceSelector::parse(\"wght=bold\"): {other:?}")),
    }
    // An image over 2^27 pixels, with the CLI's message.
    let big = termshot::parse(b"x", 500, 200, &ParseOptions::default()).map_err(|error| format!("errors: {error}"))?;
    for (padding, says) in [((0, 0), " or rows"), ((1, 0), ", rows or padding")] {
        match termshot::render(&big, &RenderOptions { px: 255.0, padding, ..RenderOptions::default() }) {
            Err(error @ Error::ImageTooLarge { .. }) => {
                let message = error.to_string();
                if !message.starts_with("image ") || !message.ends_with(&format!(" is over 134217728 pixels; lower px, cols{says}")) {
                    return fail(format!("too large: {message}"));
                }
            }
            other => return fail(format!("500x200 at 255 px: {:?}", other.map(|_| ()))),
        }
    }
    // A grid parse refuses, and a cast it can't read, are checked by
    // sizes() and cast(). Running out of memory is checked with --faults
    // (tests/run.sh); a bug, Internal, has no input that makes it, so only
    // its message is checked here.
    if Error::Internal("painting failed".into()).to_string() != "painting failed"
        || Error::OutOfMemory("out of memory".into()).to_string() != "out of memory"
    {
        return fail("the messages of Internal and OutOfMemory".into());
    }
    Ok(())
}

/// What a render finds, as data: a character drawn as a box for an empty
/// glyph, and the RGBA pixels it encodes.
fn findings() -> Result<(), String> {
    // The built-in font maps U+16910 (Bamum) to an empty glyph.
    let grid = termshot::parse("ab\r\nc\u{16910}d\u{16910}".as_bytes(), 4, 2, &ParseOptions::default())
        .map_err(|error| format!("findings: {error}"))?;
    let options = RenderOptions { px: 16.0, ..RenderOptions::default() };
    let rendered = termshot::render(&grid, &options).map_err(|error| format!("findings: {error}"))?;
    let Some(empty) = rendered.empty_glyph else { return Err("findings: no empty glyph".into()) };
    if (empty.ch, empty.row, empty.col, empty.cells, empty.in_font, empty.in_fallback) != ('\u{16910}', 1, 1, 2, true, false) {
        return Err(format!("findings: {empty:?}"));
    }
    let rgba = termshot::render_rgba(&grid, &options).map_err(|error| format!("findings: {error}"))?;
    let pixels = rgba.width as usize * rgba.height as usize;
    if (rgba.width, rgba.height) != (rendered.width, rendered.height) || rgba.rgba.len() != 4 * pixels
        || rgba.rgba.chunks_exact(4).any(|p| p[3] != 255) || rgba.empty_glyph != rendered.empty_glyph
    {
        return Err("findings: the RGBA pixels".into());
    }
    let plain = termshot::parse(b"ab", 4, 2, &ParseOptions::default()).map_err(|error| format!("findings: {error}"))?;
    if termshot::render(&plain, &options).map_err(|error| format!("findings: {error}"))?.empty_glyph.is_some() {
        return Err("findings: an empty glyph in plain text".into());
    }
    Ok(())
}

/// The cursor as render options draws as the cursor set on the grid does.
fn cursor_options() -> Result<(), String> {
    let log = b"ab\x1b[?25l";
    let parse = || termshot::parse(log, 5, 2, &ParseOptions::default()).map_err(|error| format!("cursor: {error}"));
    let png = |grid: &Grid, options: RenderOptions| {
        termshot::render(grid, &options).map(|rendered| rendered.png).map_err(|error| format!("cursor: {error}"))
    };
    let base = RenderOptions { px: 16.0, ..RenderOptions::default() };
    for shape in [CursorShape::Block, CursorShape::Underline, CursorShape::Bar] {
        let mut set = parse()?;
        set.set_cursor(Some((1, 2))).map_err(|error| format!("cursor: {error}"))?;
        set.set_cursor_shape(shape);
        let at = RenderOptions { cursor: Cursor::At { row: 1, col: 2 }, cursor_shape: Some(shape), ..base };
        if png(&set, base)? != png(&parse()?, at)? {
            return Err(format!("cursor: a {shape:?} cursor from the options differs from one on the grid"));
        }
    }
    let shown = parse()?;
    let mut hidden = parse()?;
    hidden.set_cursor(None).map_err(|error| format!("cursor: {error}"))?;
    if png(&shown, RenderOptions { cursor: Cursor::Hidden, ..base })? != png(&hidden, base)? {
        return Err("cursor: Cursor::Hidden".into());
    }
    Ok(())
}

/// Grids, fonts and renders cross threads; renders on twelve threads at
/// once, of four screens with their own fonts, sizes, palettes and
/// padding, are each the PNG of that screen drawn alone (#80).
fn threads() -> Result<(), String> {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<termshot::Grid>();
    send_sync::<termshot::Error>();
    send_sync::<termshot::Font>();
    send_sync::<termshot::Rendered>();
    send_sync::<termshot::RenderOptions>();
    let log = format!("\x1b_Ga=T,f=24,s=30,v=40,q=2;{}\x1b\\text", "A".repeat(4800));
    let grid = std::thread::spawn(move || termshot::parse(log.as_bytes(), 10, 4, &ParseOptions::default()))
        .join()
        .map_err(|_| "threads: the parse panicked".to_string())?
        .map_err(|error| format!("threads: {error}"))?;
    if !grid.to_text().contains("text") {
        return Err(format!("threads: {:?}", grid.to_text()));
    }
    let err = |error: Error| format!("threads: {error}");
    let mono = Font::embedded().map_err(err)?;
    let cjk = open(golden_cases::CJK)?;
    let vf = open(golden_cases::CJK_VF)?;
    let solarized = ParseOptions { palette: palette("tests/fixtures/solarized-dark.conf")?, ..ParseOptions::default() };
    let screens: [(&str, usize, usize, ParseOptions, RenderOptions); 4] = [
        ("reply-sent", 100, 30, ParseOptions::default(), RenderOptions { px: 20.0, font: Some(&mono), ..RenderOptions::default() }),
        ("cjk", 40, 4, ParseOptions::default(), RenderOptions { px: 24.0, font: Some(&mono), fallback: Some(&cjk), ..RenderOptions::default() }),
        ("cff2", 40, 4, solarized, RenderOptions { px: 30.0, font: Some(&vf), padding: (7, 3), ..RenderOptions::default() }),
        ("kitty-layers", 8, 4, solarized, RenderOptions { px: 33.0, font: Some(&mono), padding: (12, 6), cursor_shape: Some(CursorShape::Bar), ..RenderOptions::default() }),
    ];
    let mut grids = Vec::new();
    for (log, cols, rows, parse_options, options) in &screens {
        let cell = options.font.map_or(Ok((1, 1)), |font| font.cell_size(options.px)).map_err(err)?;
        grids.push(termshot::parse_with_cell_size(&read_log(log)?, *cols, *rows, parse_options, cell).map_err(err)?);
    }
    let draw = |k: usize| termshot::render(&grids[k], &screens[k].4).map(|rendered| rendered.png);
    let alone: Vec<Vec<u8>> = (0..4).map(draw).collect::<Result<_, _>>().map_err(err)?;
    for k in 0..4 {
        if alone[..k].contains(&alone[k]) {
            return Err(format!("threads: screen {k} is another's"));
        }
    }
    let differ = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for t in 0..12 {
            let (draw, alone, differ) = (&draw, &alone, &differ);
            scope.spawn(move || {
                for round in 0..3 {
                    let k = (t + round) % 4;
                    if draw(k).ok().as_ref() != Some(&alone[k]) {
                        differ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
        }
    });
    match differ.into_inner() {
        0 => Ok(()),
        n => Err(format!("threads: {n} of 36 concurrent renders differ from their screen drawn alone")),
    }
}

/// The grid sizes parse refuses, as errors.
fn sizes() -> Result<(), String> {
    let options = ParseOptions::default();
    let (max, side) = (termshot::MAX_CELLS, termshot::MAX_SIDE);
    let refused = [(0, 30), (100, 0), (0, 0), (side + 1, 1), (1, side + 1), (2048, 2049), (max + 1, 1), (usize::MAX, 2)];
    for (cols, rows) in refused {
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
    // The widest grid: a count over 65,535 still moves to its last column.
    let wide = termshot::parse(b"\x1b[100000Cx", side, 1, &options).map_err(|error| format!("sizes: {side}x1: {error}"))?;
    if wide.cursor() != Some((0, side - 1)) || wide.cell(0, side - 1).map(|cell| cell.ch()) != Some('x') {
        return Err(format!("sizes: CUF 100000 on a {side}x1 grid leaves the cursor at {:?}", wide.cursor()));
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

/// A xorshift generator: the same hostile inputs on every run.
struct Random(u64);

impl Random {
    fn next(&mut self) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as usize
    }

    fn pick<'a, T>(&mut self, from: &'a [T]) -> &'a T {
        &from[self.next() % from.len()]
    }
}

/// Logs and options no test means to be valid: pieces of every kind of
/// sequence, mutated and cut, on grids from 1x1 to the widest, drawn at
/// sizes from the smallest to the largest, with padding, cursors and
/// shapes of every kind. Each render returns an image or an error that is
/// not Internal (a bug); none panics, and none aborts, or this program
/// would not finish.
fn hostile() -> Result<(), String> {
    let image = format!("\x1b_Ga=T,f=24,s=3,v=2,q=2;{}\x1b\\", "/wAA".repeat(6));
    let placeholder = "\x1b_Ga=T,U=1,i=7,f=24,s=1,v=1,q=2;/wAA\x1b\\\x1b[38;5;7m\u{10EEEE}\u{305}\u{30D}\u{10EEEE}";
    let pieces: Vec<Vec<u8>> = [
        "plain text ", "\r\n", "\n", "\x1b[2J", "\x1b[H", "\x1b[999;999H", "\x1b[31;1;4;9m", "\x1b[38;2;1;2;3m",
        "\x1b[48;5;200m", "\x1b[7m", "\x1b[m", "\u{4e00}\u{754c}", "e\u{301}\u{302}\u{303}\u{304}\u{305}", "\u{16910}",
        "\u{2500}\u{256d}\u{2588}\u{2591}", "\x1b[?25l", "\x1b[?25h", "\x1b[3 q", "\x1b[5 q", "\x1b[?1049h", "\x1b[?1049l",
        "\x1b[2;3r", "\x1bM", "\x1b[5S", "\x1b[3T", "\x1b[10@", "\x1b[10P", "\x1b[100000b", "\x1b(0lqk\x1b(B",
        "\x1bPq#0;2;100;0;0#0!20~-!20~\x1b\\", &image, placeholder, "\x1b_Ga=p,i=7,c=3,r=2,z=-1,q=2\x1b\\",
        "\x1b_Ga=d,d=A\x1b\\", "\x1b]0;title\x07", "\x1b[", "\x1b_G", "\u{10EEEE}",
    ]
    .iter()
    .map(|piece| piece.as_bytes().to_vec())
    .collect();
    let mono = Font::embedded().map_err(|error| format!("hostile: {error}"))?;
    let cjk = open(golden_cases::CJK)?;
    let sizes = [(1, 1), (2, 1), (1, 3), (7, 3), (20, 6), (termshot::MAX_SIDE, 1), (1, 200)];
    let pxs = [f64::MIN_POSITIVE, 0.001, 1.0, 1.5, 7.0, 16.0, 47.5, 128.0, 255.999];
    let shapes = [None, Some(CursorShape::Block), Some(CursorShape::Underline), Some(CursorShape::Bar)];
    let mut random = Random(0x9e37_79b9_7f4a_7c15);
    let (mut drawn, mut refused) = (0, 0);
    for round in 0..240 {
        let mut log = Vec::new();
        for _ in 0..random.next() % 12 {
            let piece = random.pick(&pieces);
            log.extend_from_slice(piece);
        }
        // Mutate a few bytes, and sometimes cut the log short.
        for _ in 0..random.next() % 4 {
            if !log.is_empty() {
                let at = random.next() % log.len();
                log[at] = random.next() as u8;
            }
        }
        if random.next() % 4 == 0 {
            log.truncate(random.next() % (log.len() + 1));
        }
        let (cols, rows) = *random.pick(&sizes);
        // Large images are slow to draw: the widest and tallest grids only
        // at the smallest sizes, and the largest padding only at those.
        let small = cols * rows > 200 || random.next() % 4 != 0;
        let px = if small { *random.pick(&pxs[..5]) } else { *random.pick(&pxs) };
        let paddings = [(0, 0), (1, 0), (3, 2), (0, termshot::MAX_PADDING), (termshot::MAX_PADDING, 5)];
        let padding = if px < 2.0 { *random.pick(&paddings) } else { *random.pick(&paddings[..3]) };
        let cursor = match random.next() % 4 {
            0 => Cursor::FromGrid,
            1 => Cursor::Hidden,
            // Off the grid now and then, which is an Options error.
            _ => Cursor::At { row: random.next() % (rows + 1), col: random.next() % (cols + 1) },
        };
        let fallback = (random.next() % 3 == 0).then_some(&cjk);
        let options = RenderOptions { px, font: Some(&mono), fallback, padding, cursor, cursor_shape: *random.pick(&shapes),
                                      ..RenderOptions::default() };
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let cell = mono.cell_size(px)?;
            let grid = termshot::parse_with_cell_size(&log, cols, rows, &ParseOptions::default(), cell)?;
            let rendered = termshot::render(&grid, &options)?;
            Ok::<_, Error>((rendered.png.len(), rendered.width, rendered.height))
        }));
        match outcome {
            Err(_) => return Err(format!("hostile: round {round} panicked: {cols}x{rows} at {px} px, {log:?}")),
            Ok(Err(error @ Error::Internal(_))) => return Err(format!("hostile: round {round}: {error}")),
            Ok(Err(_)) => refused += 1,
            Ok(Ok((len, width, height))) if len > 0 && width > 0 && height > 0 => drawn += 1,
            Ok(Ok(_)) => return Err(format!("hostile: round {round}: an empty image")),
        }
    }
    // Most draw; the cursors off the grid and the largest images don't.
    if drawn < 100 || refused == 0 {
        return Err(format!("hostile: {drawn} drawn and {refused} refused of 240"));
    }
    println!("ok, {drawn} hostile renders drawn and {refused} refused, none panicked");
    Ok(())
}

/// With a library built with --cfg termshot_alloc_faults: fail each
/// allocation of a parse, then of a render, in turn, by site (the
/// TERMSHOT_*_FAIL_AT the fault build reads), and check that each returns
/// OutOfMemory, until one with nothing left to fail gives the same grid,
/// or draws the same PNG, as without.
fn faults() -> Result<(), String> {
    let log = "ab \x1b[3mcd\x1b[0m \u{4e2d}q\u{301} \x1b[1m\u{2500}x\x1b[0m\r\n".as_bytes();
    let cjk = open(golden_cases::CJK)?;
    let mono = Font::embedded().map_err(|error| format!("faults: {error}"))?;
    let options = RenderOptions { px: 24.0, font: Some(&cjk), fallback: Some(&mono), ..RenderOptions::default() };
    let grid = termshot::parse(log, 20, 2, &ParseOptions::default()).map_err(|error| format!("faults: {error}"))?;
    let sites = ["TERMSHOT_RENDER_FAIL_AT", "TERMSHOT_DEFLATE_FAIL_AT", "TERMSHOT_GLYPH_FAIL_AT"];
    for site in sites {
        std::env::remove_var(site);
    }
    let want = termshot::render(&grid, &options).map_err(|error| format!("faults: {error}"))?.png;
    let mut report = Vec::new();
    // The parse's: both screens' cells and rows, the tab stops, the marks,
    // and the marks in screen order.
    let mut n = 1;
    loop {
        std::env::set_var("TERMSHOT_PARSE_FAIL_AT", n.to_string());
        let result = catch_unwind(|| termshot::parse(log, 20, 2, &ParseOptions::default()));
        std::env::remove_var("TERMSHOT_PARSE_FAIL_AT");
        match result {
            Err(_) => return Err(format!("faults: TERMSHOT_PARSE_FAIL_AT={n} panicked")),
            Ok(Err(Error::OutOfMemory(message))) if message == "out of memory replaying the log on a 20x2 grid" => n += 1,
            Ok(Err(error)) => return Err(format!("faults: TERMSHOT_PARSE_FAIL_AT={n}: {error:?}")),
            Ok(Ok(parsed)) if parsed.to_json() == grid.to_json() => break,
            Ok(Ok(_)) => return Err(format!("faults: TERMSHOT_PARSE_FAIL_AT={n} parsed another grid")),
        }
    }
    if n - 1 < 7 {
        return Err(format!("faults: only {} parse allocations failed", n - 1));
    }
    report.push(format!("{} at TERMSHOT_PARSE_FAIL_AT", n - 1));
    for site in sites {
        let mut n = 1;
        loop {
            std::env::set_var(site, n.to_string());
            let result = catch_unwind(AssertUnwindSafe(|| termshot::render(&grid, &options)));
            std::env::remove_var(site);
            match result {
                Err(_) => return Err(format!("faults: {site}={n} panicked")),
                Ok(Err(Error::OutOfMemory(_))) => n += 1,
                Ok(Err(error)) => return Err(format!("faults: {site}={n}: {error:?}, not OutOfMemory")),
                Ok(Ok(rendered)) if rendered.png == want => break,
                Ok(Ok(_)) => return Err(format!("faults: {site}={n} drew another PNG")),
            }
        }
        // The copies of the cells, the marks and the image views, the
        // canvas and the PNG's buffer; the compressor's hash table, its
        // counts and its output; the glyph cache, the scratch and the
        // bitmaps.
        let least = match site {
            "TERMSHOT_RENDER_FAIL_AT" => 5,
            "TERMSHOT_DEFLATE_FAIL_AT" => 3,
            _ => 8,
        };
        if n - 1 < least {
            return Err(format!("faults: only {} allocations failed at {site} (built without --cfg termshot_alloc_faults?)", n - 1));
        }
        report.push(format!("{} at {site}", n - 1));
    }
    println!("ok, each of {} allocation failures returns Error::OutOfMemory", report.join(", "));
    Ok(())
}

fn main() -> ExitCode {
    let run = |tests: &[fn() -> Result<(), String>]| {
        for test in tests {
            if let Err(error) = test() {
                println!("FAIL {error}");
                return false;
            }
        }
        true
    };
    if std::env::args().nth(1).as_deref() == Some("--faults") {
        return if run(&[faults]) { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }
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
    let cell = match cell_24px() {
        Ok(cell) => cell,
        Err(error) => {
            println!("FAIL {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut sized = Vec::new();
    for log in &logs {
        match check(log, cell) {
            Ok(false) => {}
            Ok(true) => sized.push(log.as_str()),
            Err(error) => {
                println!("FAIL {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    println!("ok, {} grids as the CLI writes them, {} with a cell size ({})", logs.len(), sized.len(), sized.join(" "));
    for (log, px, cols, rows, args) in golden_cases::CASES {
        if let Err(error) = case_png(log, px, cols, rows, args) {
            println!("FAIL {error}");
            return ExitCode::FAILURE;
        }
    }
    println!("ok, {} PNGs byte for byte as the CLI writes them", golden_cases::CASES.len());
    if !run(&[accessors, fonts, errors, findings, cursor_options, sizes, threads, cast, hostile]) {
        return ExitCode::FAILURE;
    }
    println!("ok, the API: accessors, fonts, every error, findings, cursors, sizes, 36 renders on 12 threads, casts");
    ExitCode::SUCCESS
}
