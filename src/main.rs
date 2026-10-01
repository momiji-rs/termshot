//! Replay a PTY log into a cell grid and paint it.
//! No crates. The rasterizer is draw.c (vendored stb, no window, no system font).

use std::env;
use std::fs;
use std::process::ExitCode;

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
        font_path: *const i8,
        font_size: f64,
        out_path: *const i8,
    ) -> i32;
}

fn parse(data: &[u8], cols: usize, rows: usize) -> Vec<Cell> {
    let mut cells = vec![Cell::blank(); cols * rows];
    let mut row: i32 = 0;
    let mut col: i32 = 0;
    let mut saved = (0i32, 0i32);
    let mut fg = DEFAULT_FG;
    let mut bg = DEFAULT_BG;
    let mut bold = false;
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if b == 0x1b {
            i += 1;
            if i >= data.len() {
                break;
            }
            let kind = data[i];
            i += 1;
            if kind == b'[' {
                let start = i;
                while i < data.len() && !(0x40..=0x7e).contains(&data[i]) {
                    i += 1;
                }
                if i >= data.len() {
                    break;
                }
                let final_b = data[i];
                let body = &data[start..i];
                i += 1;
                if body.first().is_some_and(|c| b"?>=<".contains(c)) {
                    continue;
                }
                let parts: Vec<&[u8]> = if body.is_empty() {
                    Vec::new()
                } else {
                    body.split(|c| *c == b';').collect()
                };
                let num = |index: usize, default: i32| -> i32 {
                    parts
                        .get(index)
                        .and_then(|p| std::str::from_utf8(p).ok())
                        .and_then(|s| if s.is_empty() { None } else { s.parse().ok() })
                        .unwrap_or(default)
                };
                match final_b {
                    b'H' | b'f' => {
                        row = (num(0, 1) - 1).max(0);
                        col = (num(1, 1) - 1).max(0);
                    }
                    b'A' => row = (row - num(0, 1)).max(0),
                    b'B' => row = (row + num(0, 1)).min(rows as i32 - 1),
                    b'C' => col = (col + num(0, 1)).min(cols as i32),
                    b'D' => col = (col - num(0, 1)).max(0),
                    b's' => saved = (row, col),
                    b'u' => (row, col) = saved,
                    b'm' => {
                        let vals: Vec<i32> = if parts.is_empty() {
                            vec![0]
                        } else {
                            parts
                                .iter()
                                .map(|p| std::str::from_utf8(p).ok().and_then(|s| s.parse().ok()).unwrap_or(0))
                                .collect()
                        };
                        let mut k = 0;
                        while k < vals.len() {
                            let v = vals[k];
                            if v == 0 {
                                fg = DEFAULT_FG;
                                bg = DEFAULT_BG;
                                bold = false;
                            } else if v == 1 {
                                bold = true;
                            } else if v == 22 {
                                bold = false;
                            } else if v == 39 {
                                fg = DEFAULT_FG;
                            } else if v == 49 {
                                bg = DEFAULT_BG;
                            } else if (v == 38 || v == 48) && vals.get(k + 1) == Some(&2) && k + 4 < vals.len() {
                                let color = (vals[k + 2] as u8, vals[k + 3] as u8, vals[k + 4] as u8);
                                if v == 38 {
                                    fg = color;
                                } else {
                                    bg = color;
                                }
                                k += 4;
                            }
                            k += 1;
                        }
                    }
                    b'J' if num(0, 0) == 2 => {
                        cells.fill(Cell::blank());
                        row = 0;
                        col = 0;
                    }
                    b'K' if (0..rows as i32).contains(&row) => {
                        for c in (col.max(0) as usize)..cols {
                            let mut blank = Cell::blank();
                            blank.fr = fg.0;
                            blank.fg = fg.1;
                            blank.fb = fg.2;
                            blank.br = bg.0;
                            blank.bg = bg.1;
                            blank.bb = bg.2;
                            cells[row as usize * cols + c] = blank;
                        }
                    }
                    _ => {}
                }
            } else if kind == b']' {
                while i < data.len() && data[i] != 0x07 {
                    if data[i] == 0x1b && i + 1 < data.len() && data[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                if i < data.len() && data[i] == 0x07 {
                    i += 1;
                }
            } else if matches!(kind, b'P' | b'_' | b'^' | b'X') {
                while i + 1 < data.len() && !(data[i] == 0x1b && data[i + 1] == b'\\') {
                    i += 1;
                }
                i = (i + 2).min(data.len());
            }
            continue;
        }
        if b == b'\n' {
            row += 1;
            col = 0;
            i += 1;
            continue;
        }
        if b == b'\r' {
            col = 0;
            i += 1;
            continue;
        }
        if b < 32 {
            i += 1;
            continue;
        }
        let (cp, n) = utf8_at(&data[i..]);
        if (0..rows as i32).contains(&row) && (0..cols as i32).contains(&col) {
            let cell = &mut cells[row as usize * cols + col as usize];
            cell.ch = cp;
            cell.fr = fg.0;
            cell.fg = fg.1;
            cell.fb = fg.2;
            cell.br = bg.0;
            cell.bg = bg.1;
            cell.bb = bg.2;
            cell.bold = u8::from(bold);
        }
        col += 1;
        i += n;
    }
    cells
}

fn utf8_at(data: &[u8]) -> (u32, usize) {
    let b = data[0];
    if b < 0x80 {
        return (b as u32, 1);
    }
    if b & 0xe0 == 0xc0 && data.len() >= 2 {
        return ((((b as u32) & 0x1f) << 6) | ((data[1] as u32) & 0x3f), 2);
    }
    if b & 0xf0 == 0xe0 && data.len() >= 3 {
        return (
            (((b as u32) & 0x0f) << 12) | (((data[1] as u32) & 0x3f) << 6) | ((data[2] as u32) & 0x3f),
            3,
        );
    }
    if b & 0xf8 == 0xf0 && data.len() >= 4 {
        return (
            (((b as u32) & 0x07) << 18)
                | (((data[1] as u32) & 0x3f) << 12)
                | (((data[2] as u32) & 0x3f) << 6)
                | ((data[3] as u32) & 0x3f),
            4,
        );
    }
    (b as u32, 1)
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
    let cells = parse(&data, cols, rows);
    let font = match std::ffi::CString::new(font_path) {
        Ok(font) => font,
        Err(_) => {
            eprintln!("font path contains a nul");
            return ExitCode::from(2);
        }
    };
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
