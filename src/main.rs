//! Replay a PTY log into a cell grid and paint it: the CLI. No crates.
//! It reads the arguments, the files and stdin, writes the outputs (or
//! stdout), removes what it created when a run fails, and words every
//! message and exit status; the work is the library's, libtermshot.rlib
//! (src/lib.rs), which this crate links (`--extern termshot`) and calls
//! through its public API alone: `termshot::parse_with_cell_size`,
//! `termshot::render`, and the `#[doc(hidden)]` `termshot::cli` hooks for
//! the profile record and -v.

use std::env;
use std::fs;
use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;
use std::time::Instant;

use termshot::{CursorShape, EmptyGlyph, Font, FontSpec, Lf, Palette, ParseOptions, RenderOptions};

#[cfg(test)]
mod cli_tests;

const DEFAULT_COLS: usize = 100;
const DEFAULT_ROWS: usize = 30;

extern "C" {
    fn close(fd: std::ffi::c_int) -> std::ffi::c_int;
}

const VERSION: &str = "0.2.0";

const USAGE: &str = "\
usage: termshot [options] <log> <out.png>
       termshot [options] --text FILE --json FILE <log> [<out.png>]
       termshot <log> <out.png> <font.ttf> [px] [cols] [rows]

Render the final screen of a terminal log (raw PTY output, or an
asciinema v2 or v3 .cast recording) as a PNG. Use - as <log> to read
stdin, and - as an output to write stdout.

options:
  -f, --font FILE   TrueType or OpenType (CFF, CFF2) font (default:
                    built-in JetBrains Mono)
      --fallback-font FILE
                    a font for the characters the first lacks, such
                    as CJK or emoji; others are drawn as an empty box
  -p, --px N        font pixel height, above 0 and below 256 (default 48)
  -s, --size CxR    grid size in columns x rows, up to 500x200 (default: a
                    cast's size, else 100x30)
      --cast        read <log> as an asciinema .cast; without it or --raw,
                    a log whose first line is a JSON object with a
                    \"version\" member is read as one
      --raw         read <log> as raw PTY output, even if it starts as a
                    cast does
      --lf-newline  treat each bare LF as CR LF, for logs not captured
                    through a PTY: text files, cmd > out.log, and
                    tmux capture-pane -e -p; a final bare LF ends the
                    last line instead of scrolling
      --cursor COL,ROW|none
                    draw the cursor there, counting from 0 as tmux's
                    #{cursor_x},#{cursor_y} do, or not at all (default:
                    where the log leaves it, unless it hides it)
      --cursor-shape block|underline|bar
                    draw the cursor as that shape (default: the one the
                    log sets with DECSCUSR, or a block)
      --palette FILE
                    the default colours and the 16 named ones, in kitty's
                    keys: lines of foreground, background or color0 to
                    color15, then #rrggbb; # starts a comment line
      --fg #RRGGBB  the default foreground, over the palette's
      --bg #RRGGBB  the default background, over the palette's
      --padding N|X,Y
                    a margin of N pixels around the cells, or X left and
                    right and Y above and below, in the default
                    background; 0 to 1024 (default 0)
      --text FILE   write the screen as text, a line per row with trailing
                    spaces trimmed, as tmux capture-pane -p prints it; the
                    PNG is then optional, and fonts are only read for it
      --json FILE   write the screen as JSON: the cursor and its shape, and for
                    each row the runs of cells alike in colour (#rrggbb)
                    and attributes, with the column each starts at
  -v, --verbose     print the cell and image size, the face of each
                    collection and the instance of each variable font,
                    to stderr
  -h, --help        show this help
  -V, --version     show the version

The second form is the original one and still works.
For a font collection (.ttc), FILE#N picks face N, from 0, and FILE#NAME the
face with that full or family name; without either, the first is used.
For a variable font with CFF2 outlines, a last #TAG=VALUE,... picks the
instance, as in FILE#wght=700 or FILE.ttc#1#wght=700,wdth=90.
An SGR reset uses foreground #dbe7f7 on background #111823, and the 16
named colours are xterm's, unless --palette, --fg or --bg say otherwise;
colours 16 to 255 and 24-bit colours are never changed. --text and --json
see the palette (--json reports the colours it resolves to), not the padding.

A cast replays its output events in order, on the grid of the size it ends
at (its last resize event, or its header); input and markers are ignored.

exit status: 0 done; 1 a file could not be read or written, a cast is
malformed, or the font is unusable; 2 bad arguments, including a malformed
--palette file, an image over 134217728 pixels (padding included), or a
cast's size beyond 500x200 without --size.
";

/// A rendering request from the command line.
struct Options {
    log: String,
    /// The PNG; None when only --text or --json is wanted.
    out: Option<String>,
    text: Option<String>,
    json: Option<String>,
    font: Option<FontSpec>,
    fallback_font: Option<FontSpec>,
    px: f64,
    /// The grid size given, by --size or the original form's cols and rows;
    /// what is missing comes from a cast's header, or the defaults.
    cols: Option<usize>,
    rows: Option<usize>,
    lf: Lf,
    /// --cast (Some(true)) or --raw (Some(false)); None reads the log as
    /// a cast if its first line is a cast's header.
    cast: Option<bool>,
    /// --cursor, checked once the grid size is known.
    cursor: Option<String>,
    /// --cursor-shape: None leaves it to the log.
    cursor_shape: Option<CursorShape>,
    /// --palette's file, read once the outputs are known to be writable.
    palette: Option<String>,
    /// --fg and --bg, over the palette's.
    fg: Option<termshot::Rgb>,
    bg: Option<termshot::Rgb>,
    /// --padding: left and right, top and bottom.
    padding: (u32, u32),
    verbose: bool,
}

enum Command {
    Render(Options),
    Help,
    Version,
}

fn parse_px(value: &str) -> Result<f64, String> {
    match value.parse::<f64>() {
        Ok(px) if px > 0.0 && px < 256.0 => Ok(px),
        _ => Err(format!("px must be a number above 0 and below 256, not {value:?}")),
    }
}

fn parse_count(value: &str, what: &str, max: usize) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(n) if (1..=max).contains(&n) => Ok(n),
        _ => Err(format!("{what} must be a whole number from 1 to {max}, not {value:?}")),
    }
}

fn parse_size(value: &str) -> Result<(usize, usize), String> {
    let (cols, rows) = value
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("size must look like 120x40, not {value:?}"))?;
    Ok((parse_count(cols, "cols", 500)?, parse_count(rows, "rows", 200)?))
}

/// --cursor's value, checked against the grid: Some((row, col)), or None for
/// none. COL may equal the column count, which tmux reports with a wrap
/// pending; it means the last column, where terminals draw the cursor then.
fn parse_cursor(value: &str, cols: usize, rows: usize) -> Result<Option<(usize, usize)>, String> {
    if value == "none" {
        return Ok(None);
    }
    let parsed = value
        .split_once(',')
        .and_then(|(col, row)| Some((col.parse::<usize>().ok()?, row.parse::<usize>().ok()?)));
    let Some((col, row)) = parsed else {
        return Err(format!("cursor must look like 4,2 (column and row from 0) or none, not {value:?}"));
    };
    if col > cols || row >= rows {
        return Err(format!(
            "cursor {value} is off the {cols}x{rows} grid: columns go from 0 to {cols} \
             ({cols} is a pending wrap, drawn on the last column), rows from 0 to {}",
            rows - 1
        ));
    }
    Ok(Some((row, col.min(cols - 1))))
}

/// --padding's value: N on every side, or X,Y (left and right, top and bottom).
fn parse_padding(value: &str) -> Result<(u32, u32), String> {
    let side = |v: &str| v.parse::<u32>().ok().filter(|&n| n <= termshot::MAX_PADDING);
    let parsed = match value.split_once(',') {
        Some((x, y)) => side(x).zip(side(y)),
        None => side(value).map(|n| (n, n)),
    };
    parsed.ok_or_else(|| {
        format!(
            "padding must be pixels from 0 to {}, as 16 or 32,16 (left and right, top and bottom), not {value:?}",
            termshot::MAX_PADDING
        )
    })
}

/// --fg's or --bg's value.
fn parse_color_option(name: &str, value: &str) -> Result<termshot::Rgb, String> {
    termshot::parse_color(value).map_err(|why| format!("{name}: {why}"))
}

/// The palette a render asks for: the default, then --palette's file, then
/// --fg and --bg. Exit 1 when the file can't be read, 2 when it is not a palette.
fn load_palette(options: &Options) -> Result<Palette, (u8, String)> {
    let mut palette = Palette::DEFAULT;
    if let Some(path) = &options.palette {
        let mut file = Vec::new();
        let limit = termshot::MAX_PALETTE_BYTES as u64 + 1;
        fs::File::open(path)
            .and_then(|f| f.take(limit).read_to_end(&mut file))
            .map_err(|error| (1, format!("--palette {path}: {error}")))?;
        palette = palette.with_file(&file).map_err(|why| (2, format!("--palette {path}: {why}")))?;
    }
    palette.foreground = options.fg.unwrap_or(palette.foreground);
    palette.background = options.bg.unwrap_or(palette.background);
    Ok(palette)
}

/// A --font or --fallback-font value: its file, face and instance.
fn font_spec(value: &str) -> Result<FontSpec, String> {
    FontSpec::parse(value).map_err(|error| error.to_string())
}

/// The value of an option: attached (--px=48, -p48) or the next argument.
fn option_value(
    name: &str,
    attached: Option<String>,
    rest: &mut impl Iterator<Item = String>,
) -> Result<String, String> {
    attached.or_else(|| rest.next()).ok_or_else(|| format!("{name} needs a value"))
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let mut positional = Vec::new();
    let (mut font, mut fallback_font, mut px, mut size, mut verbose) = (None, None, None, None, false);
    let (mut lf, mut cast) = (Lf::Index, None);
    let (mut cursor, mut cursor_shape, mut text, mut json) = (None, None, None, None);
    let (mut palette, mut fg, mut bg, mut padding) = (None, None, None, (0, 0));
    let mut options_done = false;
    while let Some(arg) = args.next() {
        if options_done || arg == "-" || !arg.starts_with('-') {
            positional.push(arg);
            continue;
        }
        if arg == "--" {
            options_done = true;
            continue;
        }
        // --name=value, -xVALUE, or a bare option.
        let (name, attached) = if let Some(long) = arg.strip_prefix("--") {
            match long.split_once('=') {
                Some((name, value)) => (format!("--{name}"), Some(value.to_string())),
                None => (arg.clone(), None),
            }
        } else if let Some((split, _)) = arg.char_indices().nth(2) {
            (arg[..split].to_string(), Some(arg[split..].to_string()))
        } else {
            (arg.clone(), None)
        };
        match name.as_str() {
            "-h" | "--help" | "-V" | "--version" | "-v" | "--verbose" | "--lf-newline" | "--cast" | "--raw" if attached.is_some() => {
                return Err(format!("{name} takes no value"));
            }
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "-v" | "--verbose" => verbose = true,
            "--lf-newline" => lf = Lf::Newline,
            "--cast" | "--raw" if cast == Some(name == "--raw") => {
                return Err("--cast and --raw contradict each other".into());
            }
            "--cast" => cast = Some(true),
            "--raw" => cast = Some(false),
            "-f" | "--font" => font = Some(option_value(&name, attached, &mut args)?),
            "--fallback-font" => fallback_font = Some(option_value(&name, attached, &mut args)?),
            "-p" | "--px" => px = Some(parse_px(&option_value(&name, attached, &mut args)?)?),
            "-s" | "--size" => size = Some(parse_size(&option_value(&name, attached, &mut args)?)?),
            // Checked once the grid size is known.
            "--cursor" => cursor = Some(option_value(&name, attached, &mut args)?),
            "--cursor-shape" => {
                let value = option_value(&name, attached, &mut args)?;
                cursor_shape = Some(value.parse::<CursorShape>().map_err(|error| error.to_string())?);
            }
            "--text" => text = Some(option_value(&name, attached, &mut args)?),
            "--json" => json = Some(option_value(&name, attached, &mut args)?),
            "--palette" => palette = Some(option_value(&name, attached, &mut args)?),
            "--fg" => fg = Some(parse_color_option(&name, &option_value(&name, attached, &mut args)?)?),
            "--bg" => bg = Some(parse_color_option(&name, &option_value(&name, attached, &mut args)?)?),
            "--padding" => padding = parse_padding(&option_value(&name, attached, &mut args)?)?,
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    let mut positional = positional.into_iter();
    let (Some(log), out) = (positional.next(), positional.next()) else {
        return Err("expected <log> and <out.png>".into());
    };
    if out.is_none() && text.is_none() && json.is_none() {
        return Err("expected <log> and <out.png>, or --text or --json FILE and <log>".into());
    }
    if [&out, &text, &json].iter().filter(|path| path.as_deref() == Some("-")).count() > 1 {
        return Err("only one output can be - (stdout)".into());
    }
    // The original form: <log> <out.png> <font.ttf> [px] [cols] [rows].
    if let Some(path) = positional.next() {
        if font.is_some() {
            return Err("the font is given twice".into());
        }
        font = Some(path);
    }
    if let Some(value) = positional.next() {
        if px.is_some() {
            return Err("px is given twice".into());
        }
        px = Some(parse_px(&value)?);
    }
    let legacy_cols = positional.next().map(|v| parse_count(&v, "cols", 500)).transpose()?;
    let legacy_rows = positional.next().map(|v| parse_count(&v, "rows", 200)).transpose()?;
    if let Some(extra) = positional.next() {
        return Err(format!("unexpected argument {extra:?}"));
    }
    if size.is_some() && legacy_cols.is_some() {
        return Err("the grid size is given twice".into());
    }
    let (mut cols, mut rows) = match size {
        Some((cols, rows)) => (Some(cols), Some(rows)),
        None => (legacy_cols, legacy_rows),
    };
    // Under --raw no cast can give the size, so the defaults fill it now.
    if cast == Some(false) {
        cols = cols.or(Some(DEFAULT_COLS));
        rows = rows.or(Some(DEFAULT_ROWS));
    }
    // Refuse a bad --cursor before reading the log, which may be stdin. Its
    // bounds wait for the size when a cast may give it; a size too large to
    // reach checks only the form.
    if let Some(value) = &cursor {
        parse_cursor(value, cols.unwrap_or(usize::MAX), rows.unwrap_or(usize::MAX))?;
    }
    Ok(Command::Render(Options {
        log,
        out,
        text,
        json,
        font: font.as_deref().map(font_spec).transpose()?,
        fallback_font: fallback_font.as_deref().map(font_spec).transpose()?,
        px: px.unwrap_or(48.0),
        cols,
        rows,
        lf,
        cast,
        cursor,
        cursor_shape,
        palette,
        fg,
        bg,
        padding,
        verbose,
    }))
}

/// The font and fallback font a render asks for, checked and padded.
/// Each font's load is timed on the same boundaries (LoadTimings), whether
/// it is built in or a file.
fn load_fonts(
    options: &Options,
    timings: &mut [termshot::cli::LoadTimings; 2],
) -> Result<(Font, Option<Font>), termshot::Error> {
    let [font_timings, fallback_timings] = timings;
    let font = match &options.font {
        Some(spec) => Font::open_timed(spec, font_timings)?,
        None => Font::embedded_timed(font_timings)?,
    };
    let fallback = options.fallback_font.as_ref().map(|spec| Font::open_timed(spec, fallback_timings)).transpose()?;
    Ok((font, fallback))
}

/// What the empty-glyph warning says: a render's EmptyGlyph (which only
/// the library can make), or a test's.
#[derive(Clone, Copy)]
struct Empty {
    ch: char,
    row: usize,
    col: usize,
    cells: usize,
    in_font: bool,
    in_fallback: bool,
}

impl From<&EmptyGlyph> for Empty {
    fn from(empty: &EmptyGlyph) -> Empty {
        let &EmptyGlyph { ch, row, col, cells, in_font, in_fallback, .. } = empty;
        Empty { ch, row, col, cells, in_font, in_fallback }
    }
}

/// The warning for characters drawn as boxes because a font maps them to
/// empty glyphs: which cell, which fonts, and what to pass instead.
fn empty_glyph_warning(empty: &Empty, options: &Options, font: &Font, fallback: Option<&Font>) -> String {
    let mut color = false;
    let mut name = |flag: &str, spec: Option<&FontSpec>, font: &Font| {
        let mut name = spec.map_or("the built-in font".to_owned(), |spec| format!("{flag} {}", spec.name()));
        if let Some(tag) = font.color_bitmap() {
            color = true;
            name += &format!(" (a color bitmap font, {tag}, which termshot cannot draw)");
        }
        name
    };
    let mut blamed = Vec::new();
    if empty.in_font {
        blamed.push(name("--font", options.font.as_ref(), font));
    }
    if let (true, Some(fallback)) = (empty.in_fallback, fallback) {
        blamed.push(name("--fallback-font", options.fallback_font.as_ref(), fallback));
    }
    let blamed = match blamed.len() {
        1 => format!("{} maps it to an empty glyph", blamed[0]),
        _ => format!("{} map it to empty glyphs", blamed.join(" and ")),
    };
    let cells = match empty.cells {
        1 => String::new(),
        n => format!(" (the first of {n} such cells)"),
    };
    let instead = match fallback {
        None => "pass --fallback-font with an outline font that has it",
        Some(_) => "pass a --fallback-font with an outline for it",
    };
    format!(
        "warning: U+{:04X} at column {}, row {} (from 0) is drawn as a box{cells}: {blamed}; {instead}{}",
        u32::from(empty.ch),
        empty.col,
        empty.row,
        if color { ", such as Noto Emoji" } else { "" }
    )
}

/// Resolve symlinks component by component, including a dangling final link.
/// canonicalize alone cannot name a target that an output has yet to create.
fn output_target(path: &str) -> std::path::PathBuf {
    use std::path::{Component, Path, PathBuf};
    if let Ok(path) = fs::canonicalize(path) { return path; }
    let original = env::current_dir().unwrap_or_default().join(path);
    let mut parts: std::collections::VecDeque<_> = original.components()
        .map(|part| part.as_os_str().to_os_string()).collect();
    let mut resolved = PathBuf::new();
    let mut links = 0;
    while let Some(part) = parts.pop_front() {
        match Path::new(&part).components().next() {
            Some(Component::CurDir) => continue,
            Some(Component::ParentDir) => { resolved.pop(); continue; }
            _ => resolved.push(&part),
        }
        if let Ok(target) = fs::read_link(&resolved) {
            links += 1;
            // A cyclic or excessive chain cannot be opened either. Leave the
            // usual preflight open to report the filesystem error, without a loop.
            if links > 40 { return original; }
            resolved.pop();
            if target.is_absolute() { resolved.clear(); }
            for part in target.components().rev() {
                parts.push_front(part.as_os_str().to_os_string());
            }
        }
    }
    resolved
}

/// Why an output can't be written: it names the same file as another output,
/// which would overwrite it, or as an input, which would destroy it. One file
/// can be named many ways (`a`, `./a`, `d/../a`, a symlink), so paths are
/// compared by resolved target and (for existing files) device/inode identity.
fn output_clash(options: &Options) -> Option<String> {
    let same_file = |a: &str, b: &str| {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(a), Ok(b)) = (fs::metadata(a), fs::metadata(b)) {
            if (a.dev(), a.ino()) == (b.dev(), b.ino()) { return true; }
        }
        output_target(a) == output_target(b)
    };
    fn named<'a, const N: usize>(pairs: [(&'static str, Option<&'a str>); N]) -> Vec<(&'static str, &'a str)> {
        pairs.into_iter().filter_map(|(name, path)| Some((name, path.filter(|p| *p != "-")?))).collect()
    }
    let outputs = named([("<out.png>", options.out.as_deref()), ("--text", options.text.as_deref()), ("--json", options.json.as_deref())]);
    fn font_file(spec: &Option<FontSpec>) -> Option<&str> {
        spec.as_ref().map(FontSpec::path)
    }
    // Only the log reads - as stdin; a font or palette named - is that file.
    let mut inputs = named([("<log>", Some(options.log.as_str()))]);
    let files = [("--font", font_file(&options.font)), ("--fallback-font", font_file(&options.fallback_font)),
                 ("--palette", options.palette.as_deref())];
    inputs.extend(files.into_iter().filter_map(|(name, path)| Some((name, path?))));
    for (i, (name, path)) in outputs.iter().enumerate() {
        if let Some((other, ..)) = outputs[..i].iter().find(|(_, earlier)| same_file(path, earlier)) {
            return Some(format!("{name} and {other} name the same file, {path}; give each output its own"));
        }
        if let Some((input, _)) = inputs.iter().find(|(_, input)| same_file(path, input)) {
            return Some(format!("{name} {path} is the {input} file; writing it would destroy the input"));
        }
    }
    None
}

fn write_output(path: &str, bytes: &[u8]) -> std::io::Result<()> {
    if path == "-" {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(bytes)?;
        stdout.flush()
    } else {
        fs::write(path, bytes)
    }
}

/// Write the PNG to `path` (stdout is /dev/stdout), and say whether it all
/// reached the file: File's drop ignores close's result, where a
/// filesystem may report a write it deferred, so close is checked too.
fn write_png(path: &str, png: &[u8]) -> bool {
    use std::os::unix::io::IntoRawFd;
    let Ok(mut file) = fs::File::create(path) else { return false };
    let written = file.write_all(png).is_ok();
    // SAFETY: the descriptor is the file's, closed once, here.
    written & (unsafe { close(file.into_raw_fd()) } == 0)
}

/// The exit status for a library error: 1 for a file that can't be read
/// or used (a cast, a font), 2 for the rest (a size or option out of
/// range, an image too large, memory run out, a bug).
fn status(error: &termshot::Error) -> u8 {
    match error {
        termshot::Error::Cast(_) | termshot::Error::Font(_) => 1,
        _ => 2,
    }
}

/// Print an error the way every failure path reports it, and pick the status.
fn fail(code: u8, message: impl std::fmt::Display) -> ExitCode {
    eprintln!("termshot: {message}");
    ExitCode::from(code)
}

fn main() -> ExitCode {
    let started = Instant::now();
    let profile = env::var_os("TERMSHOT_PROFILE").is_some();
    if env::args().len() == 1 {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    }
    let options = match parse_args(env::args().skip(1)) {
        Ok(Command::Render(options)) => options,
        Ok(Command::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("termshot {VERSION}");
            return ExitCode::SUCCESS;
        }
        Err(message) => return fail(2, format!("{message}\nRun termshot --help for usage.")),
    };

    // Refuse to pour a PNG into a terminal, and find an unwritable output
    // before doing any work. The files this run created are removed on failure.
    if options.out.as_deref() == Some("-") && std::io::stdout().is_terminal() {
        return fail(2, "refusing to write a PNG to a terminal; redirect stdout or name a file");
    }
    if let Some(message) = output_clash(&options) {
        return fail(2, message);
    }
    let mut created = Vec::new();
    let remove_created = |created: &[String]| {
        for path in created {
            let _ = fs::remove_file(path);
        }
    };
    for path in [&options.out, &options.text, &options.json].into_iter().flatten().filter(|path| *path != "-") {
        let existed = std::path::Path::new(path).exists();
        if let Err(error) = fs::OpenOptions::new().write(true).create(true).open(path) {
            remove_created(&created);
            return fail(1, format!("{path}: {error}"));
        }
        if !existed {
            created.push(path.clone());
        }
    }
    let cleanup = |code: u8, message: String| {
        remove_created(&created);
        fail(code, message)
    };

    let palette = match load_palette(&options) {
        Ok(palette) => palette,
        Err((code, message)) => return cleanup(code, message),
    };
    let parse_options = ParseOptions { lf: options.lf, palette };

    let read_started = Instant::now();
    let data = if options.log == "-" {
        let mut data = Vec::new();
        std::io::stdin().read_to_end(&mut data).map(|_| data)
    } else {
        fs::read(&options.log)
    };
    let data = match data {
        Ok(data) => data,
        Err(error) => return cleanup(1, format!("{}: {error}", options.log)),
    };
    let input_bytes = data.len();
    let name = if options.log == "-" { "stdin" } else { &options.log };
    // An asciinema recording replays its output events; its header gives
    // the grid size that --size and the original form's cols and rows don't.
    // Decoding it is part of reading the log, in the profile too.
    let (data, cast_size) = if options.cast.unwrap_or_else(|| termshot::is_cast(&data)) {
        match termshot::decode_cast(data) {
            Ok(cast) => (cast.output, Some((cast.final_size, cast.resized))),
            Err(reason) => {
                let raw = if options.cast.is_none() { "; if it is raw PTY output, pass --raw" } else { "" };
                return cleanup(1, format!("{name}: not a readable asciicast: {reason}{raw}"));
            }
        }
    } else {
        (data, None)
    };
    let read_ms = read_started.elapsed().as_secs_f64() * 1000.0;
    let source = match cast_size {
        Some((_, true)) => "its last resize event",
        _ => "its header",
    };
    let dimension = |given: Option<usize>, what: &str, max: usize, default: usize, from_cast: Option<u64>| {
        match (given, from_cast) {
            (Some(n), _) => Ok(n),
            (None, None) => Ok(default),
            (None, Some(n)) if (1..=max as u64).contains(&n) => Ok(n as usize),
            (None, Some(n)) => Err(format!(
                "{name}: the recording's terminal has {n} {what} ({source}), and termshot draws 1 to {max}; pass --size"
            )),
        }
    };
    let cols = dimension(options.cols, "columns", 500, DEFAULT_COLS, cast_size.map(|((c, _), _)| c));
    let rows = dimension(options.rows, "rows", 200, DEFAULT_ROWS, cast_size.map(|((_, r), _)| r));
    let (cols, rows) = match (cols, rows) {
        (Ok(cols), Ok(rows)) => (cols, rows),
        (Err(message), _) | (_, Err(message)) => return cleanup(2, message),
    };
    let cursor_option = match options.cursor.as_deref().map(|value| parse_cursor(value, cols, rows)).transpose() {
        Ok(cursor) => cursor,
        Err(message) => return cleanup(2, format!("{message}\nRun termshot --help for usage.")),
    };
    // The image would still be made, with each line starting where the
    // last one ended; say why, and what fixes it.
    if options.lf == Lf::Index && termshot::lacks_cr(&data) {
        eprintln!(
            "termshot: hint: {name} has line feeds but no CR, so each line starts where the last ended; \
             if it was not captured through a PTY (a text file, cmd > out.log, tmux capture-pane), pass --lf-newline"
        );
    }

    let font_started = Instant::now();
    let mut font_timings = [termshot::cli::LoadTimings::default(); 2];
    // Plain text/JSON logs need no fonts. Graphics also need cell metrics,
    // even without a PNG, because placement can move the text cursor.
    let needs_fonts = options.out.is_some() || termshot::needs_cell_size(&data);
    let fonts = match needs_fonts.then(|| load_fonts(&options, &mut font_timings)).transpose() {
        Ok(fonts) => fonts,
        Err(error) => return cleanup(status(&error), error.to_string()),
    };
    let font_load_ms = font_started.elapsed().as_secs_f64() * 1000.0;
    // A collection given without a face draws with its first, which may be
    // the wrong script (Noto CJK's is Japanese): say which, and what the others are.
    for (flag, font) in fonts.iter().flat_map(|(font, fallback)| [("--font", Some(font)), ("--fallback-font", fallback.as_ref())]) {
        let Some(font) = font else { continue };
        if let Some(hint) = font.hint() {
            eprintln!("termshot: hint: {flag} {hint}");
        }
        if let (true, Some((index, name))) = (options.verbose, font.face()) {
            eprintln!("{flag} face #{index} {name}");
        }
        if let (true, Some(instance)) = (options.verbose, font.instance()) {
            eprintln!("{flag} instance {instance}");
        }
    }

    let parse_started = Instant::now();
    let cell = match &fonts {
        Some((font, _)) => font.cell_size(options.px),
        None => Ok((1, 1)),
    };
    let grid = cell.and_then(|cell| termshot::parse_with_cell_size(&data, cols, rows, &parse_options, cell));
    let mut grid = match grid {
        Ok(grid) => grid,
        Err(error) => return cleanup(status(&error), error.to_string()),
    };
    // Rendering needs only the final grid. Release potentially large logs before
    // allocating the raster and compressor buffers.
    drop(data);
    let parse_ms = parse_started.elapsed().as_secs_f64() * 1000.0;
    // parse_cursor has put it on the grid.
    let cursor = cursor_option.map_or(Ok(()), |cursor| grid.set_cursor(cursor));
    if let Err(error) = cursor {
        return cleanup(status(&error), error.to_string());
    }
    if let Some(shape) = options.cursor_shape {
        grid.set_cursor_shape(shape);
    }
    let write = |path: &String, output: String| {
        write_output(path, output.as_bytes())
            .map_err(|error| format!("{}: {error}", if path == "-" { "stdout" } else { path }))
    };
    let written = (options.text.as_ref())
        .map_or(Ok(()), |path| write(path, grid.to_text()))
        .and_then(|()| (options.json.as_ref()).map_or(Ok(()), |path| write(path, grid.to_json())));
    if let Err(message) = written {
        return cleanup(1, message);
    }
    let mut rendered = false;
    if let (Some(out), Some((font, fallback))) = (&options.out, &fonts) {
        rendered = true;
        let out = if out == "-" { "/dev/stdout" } else { out };
        if out.contains('\0') {
            return cleanup(2, "output path contains a nul byte".into());
        }
        let render_options = RenderOptions {
            px: options.px,
            font: Some(font),
            fallback: fallback.as_ref(),
            padding: options.padding,
            profile,
            ..RenderOptions::default()
        };
        if options.verbose {
            match termshot::cli::verbose_line(font, fallback.as_ref(), options.px, cols, rows, options.padding) {
                Ok(line) => eprintln!("{line}"),
                Err(error) => return cleanup(status(&error), error.to_string()),
            }
        }
        let rendered = match termshot::render(&grid, &render_options) {
            Ok(rendered) => rendered,
            Err(error) => return cleanup(status(&error), error.to_string()),
        };
        let write_started = Instant::now();
        let written = write_png(out, &rendered.png);
        let write_ms = write_started.elapsed().as_secs_f64() * 1000.0;
        if let Some(fields) = &rendered.profile {
            eprintln!("termshot-profile {{{fields},\"output_write_ms\":{write_ms:.6}}}");
        }
        if !written {
            return cleanup(1, format!("png write failed: {out}"));
        }
        if let Some(empty) = &rendered.empty_glyph {
            eprintln!("termshot: {}", empty_glyph_warning(&empty.into(), &options, font, fallback.as_ref()));
        }
    }
    if profile {
        // font_* is the --font or built-in font, fallback_* the
        // --fallback-font; both are inside font_load_ms. docs/performance.md
        // lists every boundary.
        let mut fields = format!("\"input_read_ms\":{read_ms:.6},\"parse_ms\":{parse_ms:.6},\"font_load_ms\":{font_load_ms:.6}");
        for (name, t) in [("font", &font_timings[0]), ("fallback", &font_timings[1])] {
            fields += &format!(
                ",\"{name}_allocate_ms\":{:.6},\"{name}_read_ms\":{:.6},\"{name}_check_ms\":{:.6},\"{name}_padding_ms\":{:.6},\"{name}_bytes\":{}",
                t.allocate_ms, t.read_ms, t.check_ms, t.padding_ms, t.bytes
            );
        }
        let builtin = u8::from(needs_fonts && options.font.is_none());
        // A render's record has face_ms; a run without one reports 0, as
        // every run did when the CLI timed it.
        if !rendered {
            fields += ",\"face_ms\":0.000000";
        }
        eprintln!(
            "termshot-profile {{{fields},\"font_builtin\":{builtin},\"total_ms\":{:.6},\"input_bytes\":{input_bytes}}}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    ExitCode::SUCCESS
}
