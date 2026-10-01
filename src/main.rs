//! Replay a PTY log into a cell grid and paint it.
//! No crates. The rasterizer is draw.c (vendored stb, no window, no system font).

use std::env;
use std::fs;
use std::process::ExitCode;

mod font;
#[cfg(test)]
mod draw_tests;
#[cfg(test)]
mod tests;

const DEFAULT_COLS: usize = 100;
const DEFAULT_ROWS: usize = 30;
const DEFAULT_FG: (u8, u8, u8) = (219, 231, 247);
const DEFAULT_BG: (u8, u8, u8) = (17, 24, 35);

#[repr(C)]
#[derive(Clone, Copy)]
struct Cell {
    ch: u32,
    fr: u8,
    fg: u8,
    fb: u8,
    br: u8,
    bg: u8,
    bb: u8,
    bold: u8,
}

const _: () = assert!(std::mem::size_of::<Cell>() == 12);

impl Cell {
    fn blank() -> Self {
        Self {
            ch: ' ' as u32,
            fr: DEFAULT_FG.0,
            fg: DEFAULT_FG.1,
            fb: DEFAULT_FG.2,
            br: DEFAULT_BG.0,
            bg: DEFAULT_BG.1,
            bb: DEFAULT_BG.2,
            bold: 0,
        }
    }
}

extern "C" {
    fn draw_png(
        cells: *const Cell,
        cols: i32,
        rows: i32,
        // A font that passed font::check, followed by its zero padding.
        font: *const u8,
        font_size: f64,
        out_path: *const i8,
    ) -> i32;
}

/// The grid and the terminal state that writes to it.
struct Screen {
    cells: Vec<Cell>,
    cols: usize,
    rows: usize,
    row: i32,
    col: i32,
    saved: (i32, i32),
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    bold: bool,
}

impl Screen {
    fn new(cols: usize, rows: usize) -> Self {
        Self {
            cells: vec![Cell::blank(); cols * rows],
            cols,
            rows,
            row: 0,
            col: 0,
            saved: (0, 0),
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            bold: false,
        }
    }

    fn last_row(&self) -> i32 {
        self.rows as i32 - 1
    }

    fn last_col(&self) -> i32 {
        self.cols as i32 - 1
    }

    fn print(&mut self, ch: u32) {
        if (0..self.rows as i32).contains(&self.row) && (0..self.cols as i32).contains(&self.col) {
            let (fg, bg) = (self.fg, self.bg);
            let cell = &mut self.cells[self.row as usize * self.cols + self.col as usize];
            *cell = Cell {
                ch,
                fr: fg.0,
                fg: fg.1,
                fb: fg.2,
                br: bg.0,
                bg: bg.1,
                bb: bg.2,
                bold: u8::from(self.bold),
            };
        }
        self.col = self.col.saturating_add(1);
    }

    /// LF moves down only. A PTY with ONLCR (the default) already turned the
    /// program's newline into CR LF, so a bare LF in a log is a TUI moving down.
    fn line_feed(&mut self) {
        self.row = self.row.saturating_add(1);
    }

    /// Blank cells [from, to), row-major and clamped to the grid. Erased cells
    /// take the current background, as on terminals with back-colour erase.
    fn erase(&mut self, from: usize, to: usize) {
        let to = to.min(self.cells.len());
        if from < to {
            let mut blank = Cell::blank();
            (blank.fr, blank.fg, blank.fb) = self.fg;
            (blank.br, blank.bg, blank.bb) = self.bg;
            self.cells[from..to].fill(blank);
        }
    }

    /// The cursor as a cell index. A cursor past the last column (after
    /// printing there) counts as the last column, where a terminal keeps it.
    fn cursor_index(&self) -> usize {
        self.row.max(0) as usize * self.cols + self.col.clamp(0, self.last_col()) as usize
    }

    fn csi(&mut self, final_byte: u8, p: &Params) {
        // Cursor moves treat a missing or zero count as 1.
        let count = |index: usize| p.get(index, 1).max(1).min(i32::MAX as u32) as i32;
        match final_byte {
            b'H' | b'f' => {
                self.row = (count(0) - 1).min(self.last_row());
                self.col = (count(1) - 1).min(self.last_col());
            }
            b'A' => self.row = self.row.saturating_sub(count(0)).max(0),
            b'B' => self.row = self.row.saturating_add(count(0)).min(self.last_row()),
            b'C' => self.col = self.col.saturating_add(count(0)).min(self.last_col()),
            b'D' => self.col = self.col.min(self.last_col()).saturating_sub(count(0)).max(0),
            b's' => self.saved = (self.row, self.col),
            b'u' => (self.row, self.col) = self.saved,
            b'm' => self.sgr(p),
            b'J' => {
                let len = self.cells.len();
                match p.get(0, 0) {
                    0 => self.erase(self.cursor_index(), len),
                    1 => self.erase(0, self.cursor_index() + 1),
                    2 | 3 => self.erase(0, len),
                    _ => {}
                }
            }
            b'K' if (0..self.rows as i32).contains(&self.row) => {
                let start = self.row as usize * self.cols;
                let cursor = self.cursor_index();
                match p.get(0, 0) {
                    0 => self.erase(cursor, start + self.cols),
                    1 => self.erase(start, cursor + 1),
                    2 => self.erase(start, start + self.cols),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn sgr(&mut self, p: &Params) {
        if p.len == 0 {
            self.reset_attributes();
            return;
        }
        let mut k = 0;
        while k < p.len {
            let v = p.list[k].value.unwrap_or(0);
            let mut end = k + 1;
            while end < p.len && p.list[end].sub {
                end += 1;
            }
            if end > k + 1 {
                // Colon form: 38:2:[colour space]:r:g:b. Any other parameter
                // with subparameters (38:5:n, 4:3 curly underline) is skipped.
                if v == 38 || v == 48 {
                    let subs = &p.list[k + 1..end];
                    if subs[0].value == Some(2) {
                        let rgb = match subs.len() {
                            4 => rgb(&subs[1..4]),
                            n if n >= 5 => rgb(&subs[2..5]),
                            _ => None,
                        };
                        if let Some(color) = rgb {
                            self.set_color(v, color);
                        }
                    }
                }
                k = end;
                continue;
            }
            match v {
                0 => self.reset_attributes(),
                1 => self.bold = true,
                22 => self.bold = false,
                39 => self.fg = DEFAULT_FG,
                49 => self.bg = DEFAULT_BG,
                38 | 48 => match p.get(k + 1, 0) {
                    2 if k + 4 < p.len => {
                        if let Some(color) = rgb(&p.list[k + 2..k + 5]) {
                            self.set_color(v, color);
                        }
                        k += 4;
                    }
                    // 256-colour index: consumed so n is not read as an SGR.
                    // The palette itself is #6.
                    5 if k + 2 < p.len => k += 2,
                    _ => return,
                },
                _ => {}
            }
            k += 1;
        }
    }

    fn reset_attributes(&mut self) {
        self.fg = DEFAULT_FG;
        self.bg = DEFAULT_BG;
        self.bold = false;
    }

    fn set_color(&mut self, which: u32, color: (u8, u8, u8)) {
        if which == 38 {
            self.fg = color;
        } else {
            self.bg = color;
        }
    }
}

/// An RGB triple, or None when a component is over 255.
fn rgb(params: &[Param]) -> Option<(u8, u8, u8)> {
    let c = |i: usize| u8::try_from(params[i].value.unwrap_or(0)).ok();
    Some((c(0)?, c(1)?, c(2)?))
}

const MAX_PARAMS: usize = 32;

/// One CSI parameter. `sub` marks a ':' subparameter of the one before.
#[derive(Clone, Copy, Default)]
struct Param {
    value: Option<u32>,
    sub: bool,
}

/// CSI parameters, parsed in place without allocating. Extras past
/// MAX_PARAMS are dropped.
struct Params {
    list: [Param; MAX_PARAMS],
    len: usize,
}

impl Params {
    /// The parameter at index, or default when it is missing or empty.
    fn get(&self, index: usize, default: u32) -> u32 {
        if index < self.len {
            self.list[index].value.unwrap_or(default)
        } else {
            default
        }
    }

    fn push(&mut self, param: Param) {
        if self.len < MAX_PARAMS {
            self.list[self.len] = param;
            self.len += 1;
        }
    }
}

/// Skip a string sequence (OSC, DCS, APC, PM, SOS) starting at i, returning
/// where parsing resumes. It ends at BEL or ST (ESC \). Any other ESC aborts
/// it and starts the next sequence; CAN and SUB abort it.
fn skip_string(data: &[u8], mut i: usize) -> usize {
    while i < data.len() {
        match data[i] {
            0x07 | 0x18 | 0x1a => return i + 1,
            0x1b if data.get(i + 1) == Some(&b'\\') => return i + 2,
            0x1b => return i,
            _ => i += 1,
        }
    }
    i
}

fn parse(data: &[u8], cols: usize, rows: usize) -> Vec<Cell> {
    let mut screen = Screen::new(cols, rows);
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if b == 0x1b {
            let Some(&kind) = data.get(i + 1) else {
                break;
            };
            i += 2;
            match kind {
                b'[' => i = csi(&mut screen, data, i),
                b']' | b'P' | b'_' | b'^' | b'X' => i = skip_string(data, i),
                // ESC ( B, ESC ) 0, ESC # 8: intermediates, then one final byte.
                0x20..=0x2f => {
                    while i < data.len() && (0x20..=0x2f).contains(&data[i]) {
                        i += 1;
                    }
                    if i < data.len() && (0x30..=0x7e).contains(&data[i]) {
                        i += 1;
                    }
                }
                // CAN and SUB cancel the escape.
                0x18 | 0x1a => {}
                // Another ESC starts over; other C0 controls still execute.
                0x00..=0x1f => i -= 1,
                // Two-byte escapes (ESC 7, ESC =, ...) are not interpreted.
                _ => {}
            }
            continue;
        }
        match b {
            b'\n' => screen.line_feed(),
            b'\r' => screen.col = 0,
            0x00..=0x1f | 0x7f => {}
            _ => {
                let (cp, n) = utf8_at(&data[i..]);
                // C1 controls encoded as UTF-8 take no cell.
                if !(0x80..=0x9f).contains(&cp) {
                    screen.print(cp);
                }
                i += n;
                continue;
            }
        }
        i += 1;
    }
    screen.cells
}

/// Parse one CSI sequence whose parameters start at i, apply it, and return
/// where parsing resumes. C0 controls inside it execute in place; ESC aborts
/// it and starts the next sequence; CAN and SUB abort it.
fn csi(screen: &mut Screen, data: &[u8], mut i: usize) -> usize {
    let mut params = Params { list: [Param::default(); MAX_PARAMS], len: 0 };
    let mut current = Param::default();
    let mut any = false;
    let mut private = false;
    let mut intermediate = false;
    let mut malformed = false;
    let start = i;
    while i < data.len() {
        let c = data[i];
        match c {
            b'0'..=b'9' => {
                let digit = u32::from(c - b'0');
                current.value = Some(current.value.unwrap_or(0).saturating_mul(10).saturating_add(digit));
                any = true;
            }
            b';' | b':' => {
                params.push(current);
                current = Param { value: None, sub: c == b':' };
                any = true;
            }
            b'<'..=b'?' if i == start => private = true,
            b'<'..=b'?' => malformed = true,
            0x20..=0x2f => intermediate = true,
            0x40..=0x7e => {
                if any {
                    params.push(current);
                }
                if !(private || intermediate || malformed) {
                    screen.csi(c, &params);
                }
                return i + 1;
            }
            0x1b => return i,
            0x18 | 0x1a => return i + 1,
            b'\n' => screen.line_feed(),
            b'\r' => screen.col = 0,
            0x00..=0x1f | 0x7f => {}
            // A non-ASCII byte cannot be part of a CSI: abort and print it.
            _ => return i,
        }
        i += 1;
    }
    i
}

/// Decode one UTF-8 character. Invalid or truncated input gives U+FFFD and
/// consumes only the lead byte and the continuation bytes that were valid.
fn utf8_at(data: &[u8]) -> (u32, usize) {
    const REPLACEMENT: u32 = 0xfffd;
    let b0 = data[0];
    // Continuation count, and the allowed range of the first continuation
    // byte (it excludes overlongs, surrogates and code points past U+10FFFF).
    let (need, first) = match b0 {
        0x00..=0x7f => return (u32::from(b0), 1),
        0xc2..=0xdf => (1, 0x80..=0xbf),
        0xe0 => (2, 0xa0..=0xbf),
        0xe1..=0xec | 0xee..=0xef => (2, 0x80..=0xbf),
        0xed => (2, 0x80..=0x9f),
        0xf0 => (3, 0x90..=0xbf),
        0xf1..=0xf3 => (3, 0x80..=0xbf),
        0xf4 => (3, 0x80..=0x8f),
        _ => return (REPLACEMENT, 1),
    };
    let mut cp = u32::from(b0) & (0x7f >> (need + 1));
    for k in 1..=need {
        let valid = match data.get(k) {
            Some(&b) if k == 1 => first.contains(&b),
            Some(&b) => (0x80..=0xbf).contains(&b),
            None => false,
        };
        if !valid {
            return (REPLACEMENT, k);
        }
        cp = (cp << 6) | (u32::from(data[k]) & 0x3f);
    }
    (cp, need + 1)
}


const USAGE: &str = "\
usage: termshot <pty.log> <out.png> <font.ttf> [px] [cols] [rows]

px is the font pixel height (default 48). cols and rows are the capture
grid (default 100 30). SGR reset uses foreground #dbe7f7 on background #111823.
";

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(src) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if src == "-h" || src == "--help" {
        println!("{USAGE}");
        return ExitCode::from(0);
    }
    let Some(dest) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let Some(font_path) = args.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let px: f64 = match args.next() {
        Some(value) => match value.parse() {
            Ok(px) if px > 0.0 && px < 256.0 => px,
            _ => {
                eprintln!("px must be a number in (0, 256)");
                return ExitCode::from(2);
            }
        },
        None => 48.0,
    };
    let cols: usize = match args.next() {
        Some(value) => match value.parse() {
            Ok(cols) if (1..=500).contains(&cols) => cols,
            _ => {
                eprintln!("cols must be a number in 1..=500");
                return ExitCode::from(2);
            }
        },
        None => DEFAULT_COLS,
    };
    let rows: usize = match args.next() {
        Some(value) => match value.parse() {
            Ok(rows) if (1..=200).contains(&rows) => rows,
            _ => {
                eprintln!("rows must be a number in 1..=200");
                return ExitCode::from(2);
            }
        },
        None => DEFAULT_ROWS,
    };
    let data = match fs::read(&src) {
        Ok(data) => data,
        Err(error) => {
            eprintln!("{src}: {error}");
            return ExitCode::from(1);
        }
    };
    let font = match font::load(&font_path) {
        Ok(font) => font,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    let cells = parse(&data, cols, rows);
    let out = match std::ffi::CString::new(dest) {
        Ok(out) => out,
        Err(_) => {
            eprintln!("output path contains a nul");
            return ExitCode::from(2);
        }
    };
    let code = unsafe {
        draw_png(
            cells.as_ptr(),
            cols as i32,
            rows as i32,
            font.as_ptr(),
            px,
            out.as_ptr(),
        )
    };
    ExitCode::from(code as u8)
}
