//! Replay a PTY log into a cell grid and paint it.
//! No crates. The rasterizer is draw.c (vendored stb, no window, no system font).

use std::env;
use std::fs;
use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;
use std::time::Instant;

mod font;
mod unicode;
#[rustfmt::skip]
mod unicode_tables;
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
    /// BOLD, UNDERLINE, DOUBLE_UNDERLINE and STRIKE bits; draw.c draws them.
    attrs: u8,
}

const _: () = assert!(std::mem::size_of::<Cell>() == 12);

const BOLD: u8 = 1;
const UNDERLINE: u8 = 2;
const DOUBLE_UNDERLINE: u8 = 4;
const STRIKE: u8 = 8;
/// The first cell of a double-width character; draw.c spans its glyph over two.
const WIDE: u8 = 16;
/// The second cell of a double-width character: ch is 0 and nothing is drawn.
const TAIL: u8 = 32;
/// draw.c slants the glyph; box drawing and blocks stay upright.
const ITALIC: u8 = 64;

/// The colours and attributes SGR sets, applied to each printed character.
/// Reverse, dim and conceal change the cell's colours as it is printed.
#[derive(Clone, Copy)]
struct Pen {
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    attrs: u8,
    dim: bool,
    reverse: bool,
    conceal: bool,
}

impl Pen {
    const DEFAULT: Pen = Pen { fg: DEFAULT_FG, bg: DEFAULT_BG, attrs: 0, dim: false, reverse: false, conceal: false };

    /// A blank cell in this pen's colours, ready for a character.
    fn cell(&self) -> Cell {
        let (mut fg, bg) = if self.reverse { (self.bg, self.fg) } else { (self.fg, self.bg) };
        if self.dim {
            // Two thirds of the way from the background to the text colour.
            let mix = |f: u8, b: u8| ((2 * u16::from(f) + u16::from(b)) / 3) as u8;
            fg = (mix(fg.0, bg.0), mix(fg.1, bg.1), mix(fg.2, bg.2));
        }
        if self.conceal {
            fg = bg;
        }
        Cell { ch: ' ' as u32, fr: fg.0, fg: fg.1, fb: fg.2, br: bg.0, bg: bg.1, bb: bg.2, attrs: self.attrs }
    }
}

/// xterm's default 256-colour palette: 16 named colours, a 6x6x6 cube and
/// 24 greys.
fn palette(n: u32) -> Option<(u8, u8, u8)> {
    const NAMED: [(u8, u8, u8); 16] = [
        (0, 0, 0), (205, 0, 0), (0, 205, 0), (205, 205, 0),
        (0, 0, 238), (205, 0, 205), (0, 205, 205), (229, 229, 229),
        (127, 127, 127), (255, 0, 0), (0, 255, 0), (255, 255, 0),
        (92, 92, 255), (255, 0, 255), (0, 255, 255), (255, 255, 255),
    ];
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match n {
        0..=15 => Some(NAMED[n as usize]),
        16..=231 => {
            let i = (n - 16) as usize;
            Some((LEVELS[i / 36], LEVELS[i / 6 % 6], LEVELS[i % 6]))
        }
        232..=255 => {
            let v = (8 + 10 * (n - 232)) as u8;
            Some((v, v, v))
        }
        _ => None,
    }
}

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
            attrs: 0,
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
        // Where its face starts: font::Font::start.
        font_start: i32,
        // Another such font for the characters the first lacks, or null.
        fallback: *const u8,
        fallback_start: i32,
        font_size: f64,
        out_path: *const std::ffi::c_char,
        verbose: i32,
    ) -> i32;
}

/// What a bare LF does.
#[derive(Clone, Copy, PartialEq)]
enum Lf {
    /// Down a row, as on a terminal; PTY output carries its own CR.
    Index,
    /// Down a row and back to column 0, as on a terminal with `onlcr` output
    /// processing: for text files and other output not run under a PTY.
    Newline,
}

/// A character set that G0 or G1 can hold.
#[derive(Clone, Copy, PartialEq)]
enum Charset {
    Ascii,
    /// DEC Special Graphics: ncurses draws boxes with it (ESC ( 0, then "lqk").
    DecGraphics,
}

/// What DECSC (ESC 7) and CSI s save, and DECRC (ESC 8) and CSI u restore.
/// The main and alternate screens each keep one.
#[derive(Clone, Copy)]
struct Saved {
    row: usize,
    col: usize,
    pending: bool,
    origin: bool,
    pen: Pen,
    charsets: [Charset; 2],
    shifted: bool,
}

impl Saved {
    const HOME: Saved = Saved {
        row: 0,
        col: 0,
        pending: false,
        origin: false,
        pen: Pen::DEFAULT,
        charsets: [Charset::Ascii; 2],
        shifted: false,
    };
}

/// The grid and the terminal state that writes to it.
///
/// The cursor is always on the grid. After a character is printed in the
/// last column the cursor stays there with a wrap pending; the next printed
/// character wraps first, and most other controls cancel the wrap (xterm's
/// model).
struct Screen {
    /// Rows of cells in storage order; `map` gives the storage row of each
    /// screen row, so scrolling rotates `map` instead of moving cells.
    cells: Vec<Cell>,
    map: Vec<usize>,
    /// The other screen: the alternate one while on the main one, and back.
    other: Vec<Cell>,
    other_map: Vec<usize>,
    on_alternate: bool,
    cols: usize,
    rows: usize,
    row: usize,
    col: usize,
    pending: bool,
    autowrap: bool,
    /// DECOM: cursor rows count from the top margin and stay inside the margins.
    origin: bool,
    /// The scrolling region, inclusive rows.
    top: usize,
    bottom: usize,
    saved: [Saved; 2],
    pen: Pen,
    /// Tab stops, one per column; every 8th column at start.
    tabs: Vec<bool>,
    /// G0 and G1. SO shifts to G1, SI back to G0.
    charsets: [Charset; 2],
    shifted: bool,
    /// The last printed character, which REP repeats.
    last: Option<u32>,
    /// Where it went (a storage index), for combining marks that follow.
    last_at: Option<usize>,
    /// DECTCEM (mode 25): whether the cursor is shown. One setting for both
    /// screens, and DECSC does not save it, as in xterm.
    cursor_shown: bool,
    /// Not terminal state: RIS keeps it, as a reset keeps the tty's settings.
    lf: Lf,
}

impl Screen {
    fn new(cols: usize, rows: usize, lf: Lf) -> Self {
        Self {
            cells: vec![Cell::blank(); cols * rows],
            map: (0..rows).collect(),
            other: vec![Cell::blank(); cols * rows],
            other_map: (0..rows).collect(),
            on_alternate: false,
            cols,
            rows,
            row: 0,
            col: 0,
            pending: false,
            autowrap: true,
            origin: false,
            top: 0,
            bottom: rows - 1,
            saved: [Saved::HOME; 2],
            pen: Pen::DEFAULT,
            tabs: (0..cols).map(|c| c % 8 == 0).collect(),
            charsets: [Charset::Ascii; 2],
            shifted: false,
            last: None,
            last_at: None,
            cursor_shown: true,
            lf,
        }
    }

    fn save_cursor(&mut self) {
        self.saved[usize::from(self.on_alternate)] = Saved {
            row: self.row,
            col: self.col,
            pending: self.pending,
            origin: self.origin,
            pen: self.pen,
            charsets: self.charsets,
            shifted: self.shifted,
        };
    }

    fn restore_cursor(&mut self) {
        let s = self.saved[usize::from(self.on_alternate)];
        (self.row, self.col, self.pending, self.origin) = (s.row, s.col, s.pending, s.origin);
        self.pen = s.pen;
        (self.charsets, self.shifted) = (s.charsets, s.shifted);
    }

    /// C0 controls, at top level or inside a CSI.
    fn control(&mut self, c: u8) {
        match c {
            0x08 => {
                self.col = self.col.saturating_sub(1);
                self.pending = false;
            }
            0x09 => self.tab_forward(1),
            0x0a if self.lf == Lf::Newline => {
                self.col = 0;
                self.index();
            }
            // LF, VT and FF all index, as on a VT100.
            0x0a..=0x0c => self.index(),
            0x0d => {
                self.col = 0;
                self.pending = false;
            }
            0x0e => self.shifted = true,
            0x0f => self.shifted = false,
            _ => {}
        }
    }

    fn tab_forward(&mut self, n: usize) {
        self.pending = false;
        for _ in 0..n.min(self.cols) {
            self.col = (self.col + 1..self.cols).find(|&c| self.tabs[c]).unwrap_or(self.cols - 1);
        }
    }

    fn tab_back(&mut self, n: usize) {
        self.pending = false;
        for _ in 0..n.min(self.cols) {
            self.col = (0..self.col).rev().find(|&c| self.tabs[c]).unwrap_or(0);
        }
    }

    /// The cell index range of screen row r.
    fn line(&self, r: usize) -> std::ops::Range<usize> {
        let stored = self.map[r];
        stored * self.cols..(stored + 1) * self.cols
    }

    /// Blank screen rows [from, to).
    fn erase_rows(&mut self, from: usize, to: usize) {
        for r in from..to {
            let line = self.line(r);
            self.erase(line.start, line.end);
        }
    }

    /// The cells in screen order.
    fn into_cells(self) -> Vec<Cell> {
        if self.map.iter().enumerate().all(|(r, &stored)| r == stored) {
            return self.cells;
        }
        (0..self.rows).flat_map(|r| self.cells[self.line(r)].iter().copied()).collect()
    }

    /// ICH: shift the rest of the line right by n, blanking the gap.
    fn insert_chars(&mut self, n: usize) {
        let line = self.line(self.row);
        let at = line.start + self.col;
        let n = n.min(line.end - at);
        self.cells.copy_within(at..line.end - n, at + n);
        self.erase(at, at + n);
        self.mend_row(self.row);
    }

    /// DCH: shift the rest of the line left by n, blanking the end.
    fn delete_chars(&mut self, n: usize) {
        let line = self.line(self.row);
        let at = line.start + self.col;
        let n = n.min(line.end - at);
        self.cells.copy_within(at + n..line.end, at);
        self.erase(line.end - n, line.end);
        self.mend_row(self.row);
    }

    /// Move rows top..=bottom up by n, blanking the n rows that open at the bottom.
    fn scroll_up(&mut self, top: usize, bottom: usize, n: usize) {
        let n = n.min(bottom + 1 - top);
        self.map[top..=bottom].rotate_left(n);
        self.erase_rows(bottom + 1 - n, bottom + 1);
    }

    /// Move rows top..=bottom down by n, blanking the n rows that open at the top.
    fn scroll_down(&mut self, top: usize, bottom: usize, n: usize) {
        let n = n.min(bottom + 1 - top);
        self.map[top..=bottom].rotate_right(n);
        self.erase_rows(top, top + n);
    }

    /// LF, IND: down a row, scrolling the region at its bottom margin.
    fn index(&mut self) {
        self.pending = false;
        if self.row == self.bottom {
            self.scroll_up(self.top, self.bottom, 1);
        } else if self.row + 1 < self.rows {
            self.row += 1;
        }
    }

    /// RI: up a row, scrolling the region down at its top margin.
    fn reverse_index(&mut self) {
        self.pending = false;
        if self.row == self.top {
            self.scroll_down(self.top, self.bottom, 1);
        } else {
            self.row = self.row.saturating_sub(1);
        }
    }

    fn in_margins(&self) -> bool {
        (self.top..=self.bottom).contains(&self.row)
    }

    /// Rows a cursor-addressing sequence can reach: the margins in origin mode.
    fn addressable_rows(&self) -> (usize, usize) {
        if self.origin {
            (self.top, self.bottom)
        } else {
            (0, self.rows - 1)
        }
    }

    /// Move to a 1-based row as CUP and VPA give it, honouring origin mode.
    fn go_to_row(&mut self, n: usize) {
        let (first, last) = self.addressable_rows();
        self.row = (first + n - 1).min(last);
    }

    fn home(&mut self) {
        self.row = self.addressable_rows().0;
        self.col = 0;
    }

    fn set_mode(&mut self, mode: u32, on: bool) {
        match mode {
            6 => {
                self.origin = on;
                self.home();
            }
            7 => self.autowrap = on,
            25 => self.cursor_shown = on,
            47 | 1047 => self.use_alternate(on, mode == 1047 && !on),
            1048 => {
                if on {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            // Save the cursor, switch, and clear the alternate screen; back and
            // restore on the way out.
            1049 if on => {
                if !self.on_alternate {
                    self.save_cursor();
                    self.use_alternate(true, false);
                    self.erase_rows(0, self.rows);
                }
            }
            1049 => {
                if self.on_alternate {
                    self.use_alternate(false, false);
                    self.restore_cursor();
                }
            }
            _ => {}
        }
    }

    /// Switch to the alternate screen or back. clear_first blanks the
    /// alternate screen before leaving it (mode 1047).
    fn use_alternate(&mut self, on: bool, clear_first: bool) {
        if on == self.on_alternate {
            return;
        }
        if clear_first {
            self.erase_rows(0, self.rows);
        }
        std::mem::swap(&mut self.cells, &mut self.other);
        std::mem::swap(&mut self.map, &mut self.other_map);
        self.on_alternate = on;
        self.last_at = None;
        self.pending = false;
    }

    fn last_row(&self) -> usize {
        self.rows - 1
    }

    fn last_col(&self) -> usize {
        self.cols - 1
    }

    fn print(&mut self, ch: u32) {
        let charset = self.charsets[usize::from(self.shifted)];
        self.print_mapped(if charset == Charset::DecGraphics { dec_graphics(ch) } else { ch });
    }

    /// Print a character that has already been through the character set.
    /// Wide characters take two cells; zero-width ones combine with the
    /// character before them.
    fn print_mapped(&mut self, ch: u32) {
        let mut width = unicode::width(ch);
        if width == 0 {
            self.combine(ch);
            return;
        }
        // On a one-column screen no row can hold both halves, so a wide
        // character takes the one cell as a narrow one (#18). tmux keeps it
        // there too, but overwrites it in place instead of wrapping.
        if width == 2 && self.cols == 1 {
            width = 1;
        }
        self.last = Some(ch);
        if self.pending {
            self.col = 0;
            self.index();
        }
        if width == 2 && self.col == self.last_col() {
            // No room for both halves. xterm leaves the last column as it is
            // and wraps; without autowrap the character is dropped.
            if !self.autowrap {
                return;
            }
            self.col = 0;
            self.index();
        }
        let line = self.line(self.row);
        let at = line.start + self.col;
        self.split_wide(&line, at);
        let mut cell = Cell { ch, ..self.pen.cell() };
        if width == 2 {
            self.split_wide(&line, at + 1);
            cell.attrs |= WIDE;
            self.cells[at + 1] = Cell { ch: 0, attrs: cell.attrs & !WIDE | TAIL, ..cell };
        }
        self.cells[at] = cell;
        self.last_at = Some(at);
        if self.col + width <= self.last_col() {
            self.col += width;
        } else {
            self.col = self.last_col();
            self.pending = self.autowrap;
        }
    }

    /// A zero-width character: compose it with the last printed character if
    /// Unicode has a precomposed form (e + U+0301 is é); otherwise drop it.
    /// A cell holds one code point, so other combinations can't be kept.
    fn combine(&mut self, mark: u32) {
        if let Some(at) = self.last_at {
            if let Some(composed) = unicode::compose(self.cells[at].ch, mark) {
                self.cells[at].ch = composed;
            }
        }
    }

    /// Before cell i of a row is overwritten: if it is half of a wide
    /// character, blank the other half.
    fn split_wide(&mut self, line: &std::ops::Range<usize>, i: usize) {
        let attrs = self.cells[i].attrs;
        if attrs & TAIL != 0 && i > line.start {
            self.unwide(i - 1);
        }
        if attrs & WIDE != 0 && i + 1 < line.end {
            self.unwide(i + 1);
        }
    }

    fn unwide(&mut self, i: usize) {
        let cell = &mut self.cells[i];
        cell.ch = ' ' as u32;
        cell.attrs &= !(WIDE | TAIL);
    }

    /// After an edit that can cut a wide character in two (ICH, DCH, ECH,
    /// EL, ED), blank any half whose partner is gone.
    fn mend_row(&mut self, r: usize) {
        let line = self.line(r);
        for i in line.clone() {
            let attrs = self.cells[i].attrs;
            if attrs & WIDE != 0 && (i + 1 >= line.end || self.cells[i + 1].attrs & TAIL == 0) {
                self.unwide(i);
            } else if attrs & TAIL != 0 && (i == line.start || self.cells[i - 1].attrs & WIDE == 0) {
                self.unwide(i);
            }
        }
    }

    /// Batch a printable ASCII run within each physical row, preserving pending
    /// wraps, scroll regions, alternate-screen row maps, and REP's last glyph.
    fn print_ascii(&mut self, mut text: &[u8]) {
        if self.charsets[usize::from(self.shifted)] == Charset::DecGraphics {
            for byte in text {
                self.print(u32::from(*byte));
            }
            return;
        }
        if let Some(byte) = text.last() {
            self.last = Some(u32::from(*byte));
        }
        let mut cell = self.pen.cell();
        while !text.is_empty() {
            if self.pending {
                if self.autowrap && self.row == self.bottom {
                    // Within this uninterrupted run, complete rows older than
                    // one scrolling region cannot survive. Rotate storage as
                    // if they were printed, then materialize the surviving rows.
                    let height = self.bottom + 1 - self.top;
                    let skip_rows = (text.len() / self.cols).saturating_sub(height);
                    if skip_rows > 0 {
                        self.map[self.top..=self.bottom].rotate_left(skip_rows % height);
                        text = &text[skip_rows * self.cols..];
                    }
                }
                self.col = 0;
                if self.row == self.bottom && text.len() >= self.cols {
                    // The entire incoming row is overwritten below; avoid
                    // clearing it just before assigning every cell again.
                    self.pending = false;
                    self.map[self.top..=self.bottom].rotate_left(1);
                } else {
                    self.index();
                }
            }
            let count = text.len().min(self.cols - self.col);
            let start = self.cursor_index();
            // Only the run's ends can cut a wide character in two.
            let line = self.line(self.row);
            self.split_wide(&line, start);
            self.split_wide(&line, start + count - 1);
            self.last_at = Some(start + count - 1);
            for (dest, byte) in self.cells[start..start + count].iter_mut().zip(text) {
                cell.ch = u32::from(*byte);
                *dest = cell;
            }
            text = &text[count..];
            self.col += count;
            if self.col == self.cols {
                self.col -= 1;
                self.pending = self.autowrap;
                if !self.autowrap {
                    // With wrapping disabled, all remaining bytes overwrite
                    // the last column. Only the final byte remains visible.
                    if let Some(byte) = text.last() {
                        cell.ch = u32::from(*byte);
                        let at = self.cursor_index();
                        let line = self.line(self.row);
                        self.split_wide(&line, at);
                        self.cells[at] = cell;
                        self.last_at = Some(at);
                    }
                    break;
                }
            }
        }
    }

    /// Blank cells [from, to) of storage, which callers keep within one row.
    /// Erased cells take the current background, as on terminals with
    /// back-colour erase.
    fn erase(&mut self, from: usize, to: usize) {
        let to = to.min(self.cells.len());
        if from < to {
            let mut blank = Cell::blank();
            (blank.fr, blank.fg, blank.fb) = self.pen.fg;
            (blank.br, blank.bg, blank.bb) = self.pen.bg;
            self.cells[from..to].fill(blank);
        }
    }

    /// The cursor as a cell index in storage.
    fn cursor_index(&self) -> usize {
        self.line(self.row).start + self.col
    }

    fn csi(&mut self, final_byte: u8, p: &Params) {
        // Counts treat a missing or zero parameter as 1. The grid is at most
        // 500x200, so capping at u16 changes nothing and keeps sums small.
        let count = |index: usize| p.get(index, 1).clamp(1, u32::from(u16::MAX)) as usize;
        // Everything but SGR, REP and the cursor save/restore pair cancels a
        // pending wrap; EL and ED then erase from the last column.
        if !matches!(final_byte, b'm' | b'b' | b's' | b'u') {
            self.pending = false;
        }
        // CUU and CUD stop at a margin when the cursor starts on its side of it.
        let up_limit = if self.row >= self.top { self.top } else { 0 };
        let down_limit = if self.row <= self.bottom { self.bottom } else { self.last_row() };
        match final_byte {
            b'H' | b'f' => {
                self.go_to_row(count(0));
                self.col = (count(1) - 1).min(self.last_col());
            }
            b'A' => self.row = self.row.saturating_sub(count(0)).max(up_limit),
            b'B' | b'e' => self.row = (self.row + count(0)).min(down_limit),
            b'C' | b'a' => self.col = (self.col + count(0)).min(self.last_col()),
            b'D' => self.col = self.col.saturating_sub(count(0)),
            // CNL, CPL: down or up, to column 0.
            b'E' => (self.row, self.col) = ((self.row + count(0)).min(down_limit), 0),
            b'F' => (self.row, self.col) = (self.row.saturating_sub(count(0)).max(up_limit), 0),
            // CHA, HPA: column; VPA: row.
            b'G' | b'`' => self.col = (count(0) - 1).min(self.last_col()),
            b'd' => self.go_to_row(count(0)),
            b'I' => self.tab_forward(count(0)),
            b'Z' => self.tab_back(count(0)),
            b'g' => match p.get(0, 0) {
                0 => {
                    let col = self.col;
                    self.tabs[col] = false;
                }
                3 => self.tabs.fill(false),
                _ => {}
            },
            b'@' => self.insert_chars(count(0)),
            b'P' => self.delete_chars(count(0)),
            b'X' => {
                let at = self.cursor_index();
                let end = self.line(self.row).end;
                self.erase(at, (at + count(0)).min(end));
            }
            // IL, DL: only inside the margins; the column stays (as in tmux).
            b'L' if self.in_margins() => {
                let n = count(0);
                self.scroll_down(self.row, self.bottom, n);
            }
            b'M' if self.in_margins() => {
                let n = count(0);
                self.scroll_up(self.row, self.bottom, n);
            }
            // SU, SD scroll the region; CSI T with more parameters is mouse tracking.
            b'S' => self.scroll_up(self.top, self.bottom, count(0)),
            b'T' if p.len <= 1 => self.scroll_down(self.top, self.bottom, count(0)),
            // DECSTBM: set the margins, ignored unless top < bottom; homes the cursor.
            b'r' => {
                // 0 or missing means the default: the first and the last row.
                let top = p.get(0, 1).max(1) as usize - 1;
                let bottom = match p.get(1, 0) as usize {
                    0 => self.rows,
                    b => b.min(self.rows),
                } - 1;
                if top < bottom {
                    (self.top, self.bottom) = (top, bottom);
                    self.home();
                }
            }
            // REP: repeat the last printed character, wrapping like any other
            // printing. Capped at a screenful.
            b'b' => {
                if let Some(ch) = self.last {
                    for _ in 0..count(0).min(self.cols * self.rows) {
                        self.print_mapped(ch);
                    }
                }
            }
            // xterm saves the same state for CSI s as for DECSC.
            b's' => self.save_cursor(),
            b'u' => self.restore_cursor(),
            b'm' => self.sgr(p),
            b'J' => {
                let (cursor, line) = (self.cursor_index(), self.line(self.row));
                match p.get(0, 0) {
                    0 => {
                        self.erase(cursor, line.end);
                        self.erase_rows(self.row + 1, self.rows);
                    }
                    1 => {
                        self.erase_rows(0, self.row);
                        self.erase(line.start, cursor + 1);
                    }
                    2 => self.erase_rows(0, self.rows),
                    // 3 clears only the scrollback, which termshot does not keep.
                    _ => {}
                }
            }
            b'K' => {
                let line = self.line(self.row);
                let cursor = self.cursor_index();
                match p.get(0, 0) {
                    0 => self.erase(cursor, line.end),
                    1 => self.erase(line.start, cursor + 1),
                    2 => self.erase(line.start, line.end),
                    _ => {}
                }
            }
            _ => {}
        }
        // Erasing part of a wide character erases all of it.
        if matches!(final_byte, b'X' | b'J' | b'K') {
            self.mend_row(self.row);
        }
    }

    /// DECSET / DECRST: CSI ? Pm h and CSI ? Pm l.
    fn private_csi(&mut self, final_byte: u8, p: &Params) {
        if matches!(final_byte, b'h' | b'l') {
            for k in 0..p.len {
                if let Some(mode) = p.list[k].value {
                    self.set_mode(mode, final_byte == b'h');
                }
            }
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
                // Colon forms: 38:2:[colour space]:r:g:b, 38:5:n, and 4:n
                // underline styles (0 off, 2 double, any other single).
                // Other parameters with subparameters are skipped.
                let subs = &p.list[k + 1..end];
                match (v, subs[0].value) {
                    (38 | 48, Some(2)) => {
                        let color = match subs.len() {
                            4 => rgb(&subs[1..4]),
                            n if n >= 5 => rgb(&subs[2..5]),
                            _ => None,
                        };
                        if let Some(color) = color {
                            self.set_color(v, color);
                        }
                    }
                    (38 | 48, Some(5)) => {
                        if let Some(color) = subs.get(1).and_then(|n| palette(n.value.unwrap_or(0))) {
                            self.set_color(v, color);
                        }
                    }
                    (4, style) => {
                        self.pen.attrs &= !(UNDERLINE | DOUBLE_UNDERLINE);
                        match style.unwrap_or(0) {
                            0 => {}
                            2 => self.pen.attrs |= DOUBLE_UNDERLINE,
                            _ => self.pen.attrs |= UNDERLINE,
                        }
                    }
                    _ => {}
                }
                k = end;
                continue;
            }
            let pen = &mut self.pen;
            match v {
                0 => *pen = Pen::DEFAULT,
                1 => pen.attrs |= BOLD,
                2 => pen.dim = true,
                3 => pen.attrs |= ITALIC,
                4 => pen.attrs = pen.attrs & !DOUBLE_UNDERLINE | UNDERLINE,
                7 => pen.reverse = true,
                8 => pen.conceal = true,
                9 => pen.attrs |= STRIKE,
                21 => pen.attrs = pen.attrs & !UNDERLINE | DOUBLE_UNDERLINE,
                22 => {
                    pen.attrs &= !BOLD;
                    pen.dim = false;
                }
                23 => pen.attrs &= !ITALIC,
                24 => pen.attrs &= !(UNDERLINE | DOUBLE_UNDERLINE),
                27 => pen.reverse = false,
                28 => pen.conceal = false,
                29 => pen.attrs &= !STRIKE,
                30..=37 => pen.fg = palette(v - 30).unwrap(),
                40..=47 => pen.bg = palette(v - 40).unwrap(),
                90..=97 => pen.fg = palette(v - 90 + 8).unwrap(),
                100..=107 => pen.bg = palette(v - 100 + 8).unwrap(),
                39 => pen.fg = DEFAULT_FG,
                49 => pen.bg = DEFAULT_BG,
                // Blink (5, 6, 25) and the rest are not drawn.
                38 | 48 => match p.get(k + 1, 0) {
                    2 if k + 4 < p.len => {
                        if let Some(color) = rgb(&p.list[k + 2..k + 5]) {
                            self.set_color(v, color);
                        }
                        k += 4;
                    }
                    5 if k + 2 < p.len => {
                        if let Some(color) = palette(p.get(k + 2, 0)) {
                            self.set_color(v, color);
                        }
                        k += 2;
                    }
                    // Too few parameters to know what follows: stop here.
                    _ => return,
                },
                _ => {}
            }
            k += 1;
        }
    }

    fn reset_attributes(&mut self) {
        self.pen = Pen::DEFAULT;
    }

    fn set_color(&mut self, which: u32, color: (u8, u8, u8)) {
        if which == 38 {
            self.pen.fg = color;
        } else {
            self.pen.bg = color;
        }
    }
}

/// DEC Special Graphics: 0x5f..=0x7e become line drawing and symbols.
fn dec_graphics(ch: u32) -> u32 {
    const TABLE: [u32; 32] = [
        0x00a0, // _ blank
        0x25c6, 0x2592, 0x2409, 0x240c, 0x240d, 0x240a, 0x00b0, 0x00b1, // ` a b c d e f g
        0x2424, 0x240b, 0x2518, 0x2510, 0x250c, 0x2514, 0x253c, 0x23ba, // h i j k l m n o
        0x23bb, 0x2500, 0x23bc, 0x23bd, 0x251c, 0x2524, 0x2534, 0x252c, // p q r s t u v w
        0x2502, 0x2264, 0x2265, 0x03c0, 0x2260, 0x00a3, 0x00b7, // x y z { | } ~
    ];
    match ch {
        0x5f..=0x7e => TABLE[(ch - 0x5f) as usize],
        _ => ch,
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

/// Whether a log looks like it never went through a PTY: it has line feeds
/// but no CR at all, which `onlcr` would have added before each one.
/// Checks CR first, so a PTY log stops at its first line end.
fn lacks_cr(data: &[u8]) -> bool {
    !data.contains(&b'\r') && data.contains(&b'\n')
}

/// Text is lines that each end in LF, so the bare LF that ends the input
/// ends the last line rather than opening a new one. Otherwise a capture as
/// tall as the grid (tmux capture-pane ends every row with LF) would scroll
/// its top row away. A final CR LF is a PTY's and stays, so a PTY log
/// renders the same with --lf-newline as without.
fn strip_final_bare_lf(data: &[u8]) -> &[u8] {
    match data {
        [.., b'\r', b'\n'] => data,
        [rest @ .., b'\n'] => rest,
        _ => data,
    }
}

/// Replay a log as a terminal would, with bare LFs indexing.
#[cfg(test)]
fn parse(data: &[u8], cols: usize, rows: usize) -> Vec<Cell> {
    parse_lf(data, cols, rows, Lf::Index)
}

#[cfg(test)]
fn parse_lf(data: &[u8], cols: usize, rows: usize, lf: Lf) -> Vec<Cell> {
    replay(data, cols, rows, lf).cells
}

/// The screen a log leaves: its cells in screen order, and the cursor as
/// (row, col) unless the log hid it.
struct Grid {
    cells: Vec<Cell>,
    cursor: Option<(usize, usize)>,
}

fn replay(data: &[u8], cols: usize, rows: usize, lf: Lf) -> Grid {
    let data = match lf {
        Lf::Newline => strip_final_bare_lf(data),
        Lf::Index => data,
    };
    let mut screen = Screen::new(cols, rows, lf);
    // Reuse the fixed parameter buffer across sequences; only len needs resetting.
    let mut params = Params { list: [Param::default(); MAX_PARAMS], len: 0 };
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if (0x20..0x7f).contains(&b) {
            let start = i;
            while i < data.len() && (0x20..0x7f).contains(&data[i]) {
                i += 1;
            }
            screen.print_ascii(&data[start..i]);
            continue;
        }
        if b == 0x1b {
            let Some(&kind) = data.get(i + 1) else {
                break;
            };
            i += 2;
            match kind {
                b'[' => i = csi(&mut screen, &mut params, data, i),
                b']' | b'P' | b'_' | b'^' | b'X' => i = skip_string(data, i),
                // ESC ( B, ESC ) 0, ESC # 8: intermediates, then one final byte.
                0x20..=0x2f => {
                    let first = i;
                    while i < data.len() && (0x20..=0x2f).contains(&data[i]) {
                        i += 1;
                    }
                    if i < data.len() && (0x30..=0x7e).contains(&data[i]) {
                        // ESC ( x designates G0, ESC ) x G1. '0' is DEC Special
                        // Graphics; any other set is drawn as ASCII.
                        if i == first && matches!(kind, b'(' | b')') {
                            let set = if data[i] == b'0' { Charset::DecGraphics } else { Charset::Ascii };
                            screen.charsets[usize::from(kind == b')')] = set;
                        }
                        i += 1;
                    }
                }
                b'7' => screen.save_cursor(),
                b'8' => screen.restore_cursor(),
                // IND, NEL, RI.
                b'D' => screen.index(),
                b'E' => {
                    screen.col = 0;
                    screen.index();
                }
                b'M' => screen.reverse_index(),
                // HTS: set a tab stop at the cursor.
                b'H' => {
                    let col = screen.col;
                    screen.tabs[col] = true;
                }
                // RIS: full reset.
                b'c' => screen = Screen::new(cols, rows, lf),
                // CAN and SUB cancel the escape.
                0x18 | 0x1a => {}
                // Another ESC starts over; other C0 controls still execute.
                0x00..=0x1f => i -= 1,
                // Other two-byte escapes (ESC =, ESC >, ...) don't change the grid.
                _ => {}
            }
            continue;
        }
        match b {
            0x00..=0x1f => screen.control(b),
            0x7f => {}
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
    // With a wrap pending the cursor stays on the last column, where
    // terminals draw it.
    let cursor = screen.cursor_shown.then_some((screen.row, screen.col));
    Grid { cells: screen.into_cells(), cursor }
}

/// Draw the cursor as a block in reverse video over the cell at (row, col),
/// or over both cells of the wide character it is on.
fn draw_cursor(cells: &mut [Cell], cols: usize, row: usize, col: usize) {
    let line = row * cols..(row + 1) * cols;
    let mut start = line.start + col;
    if cells[start].attrs & TAIL != 0 && start > line.start {
        start -= 1;
    }
    let end = if cells[start].attrs & WIDE != 0 { start + 2 } else { start + 1 };
    for cell in &mut cells[start..end.min(line.end)] {
        let (fg, bg) = ((cell.fr, cell.fg, cell.fb), (cell.br, cell.bg, cell.bb));
        // Concealed text (the colours alike) stays hidden in a block that
        // still shows.
        let (fg, bg) = if fg != bg {
            (bg, fg)
        } else {
            let block = if bg == DEFAULT_FG { DEFAULT_BG } else { DEFAULT_FG };
            (block, block)
        };
        (cell.fr, cell.fg, cell.fb) = fg;
        (cell.br, cell.bg, cell.bb) = bg;
    }
}

/// The screen as text, the way tmux capture-pane -p prints it: a line per row
/// with its trailing spaces trimmed, and a wide character once.
fn grid_text(cells: &[Cell], cols: usize) -> String {
    let mut text = String::with_capacity(cells.len() + cells.len() / cols);
    for row in cells.chunks(cols) {
        text.extend(row.iter().filter(|c| c.attrs & TAIL == 0).map(|c| char::from_u32(c.ch).unwrap_or('\u{fffd}')));
        // The previous row's newline stops the trim.
        text.truncate(text.trim_end_matches(' ').len());
        text.push('\n');
    }
    text
}

/// The screen as JSON: the grid size; the cursor, or null when hidden; and a
/// line per row of the runs of cells alike in colour and attributes, each with
/// the column it starts at (a wide character takes two). Blank cells that end a
/// row are left out, as in --text, unless their background or a line shows.
fn grid_json(cells: &[Cell], cols: usize, rows: usize, cursor: Option<(usize, usize)>) -> String {
    use std::fmt::Write as _;
    const LINES: u8 = UNDERLINE | DOUBLE_UNDERLINE | STRIKE;
    const STYLE: u8 = BOLD | ITALIC | LINES;
    let style = |c: &Cell| ((c.fr, c.fg, c.fb), (c.br, c.bg, c.bb), c.attrs & STYLE);
    let blank = |c: &Cell| c.ch == ' ' as u32 && (c.br, c.bg, c.bb) == DEFAULT_BG && c.attrs & LINES == 0;
    // Writing to a String cannot fail.
    let mut json = String::with_capacity(cells.len() * 2);
    let _ = write!(json, "{{\"cols\":{cols},\"rows\":{rows},\"cursor\":");
    let _ = match cursor {
        Some((row, col)) => write!(json, "{{\"col\":{col},\"row\":{row}}}"),
        None => write!(json, "null"),
    };
    json.push_str(",\"lines\":[");
    for (r, row) in cells.chunks(cols).enumerate() {
        json.push_str(if r == 0 { "\n[" } else { ",\n[" });
        let end = row.iter().rposition(|c| !blank(c)).map_or(0, |i| i + 1);
        let mut c = 0;
        while c < end {
            let start = c;
            let key = style(&row[c]);
            let _ = write!(json, "{}{{\"col\":{start},\"text\":\"", if start == 0 { "" } else { "," });
            // A wide character's tail is part of the run its first half is in.
            while c < end && (style(&row[c]) == key || row[c].attrs & TAIL != 0) {
                if row[c].attrs & TAIL == 0 {
                    match char::from_u32(row[c].ch).unwrap_or('\u{fffd}') {
                        '"' => json.push_str("\\\""),
                        '\\' => json.push_str("\\\\"),
                        ch if ch < ' ' || ch == '\u{7f}' => {
                            let _ = write!(json, "\\u{:04x}", ch as u32);
                        }
                        ch => json.push(ch),
                    }
                }
                c += 1;
            }
            let ((fr, fg, fb), (br, bg, bb), attrs) = key;
            let _ = write!(json, "\",\"fg\":\"#{fr:02x}{fg:02x}{fb:02x}\",\"bg\":\"#{br:02x}{bg:02x}{bb:02x}\"");
            for (bit, name) in [(BOLD, "bold"), (ITALIC, "italic"), (UNDERLINE, "underline"), (DOUBLE_UNDERLINE, "double_underline"), (STRIKE, "strike")] {
                if attrs & bit != 0 {
                    let _ = write!(json, ",\"{name}\":true");
                }
            }
            json.push('}');
        }
        json.push(']');
    }
    json.push_str("\n]}\n");
    json
}

/// Parse one CSI sequence whose parameters start at i, apply it, and return
/// where parsing resumes. C0 controls inside it execute in place; ESC aborts
/// it and starts the next sequence; CAN and SUB abort it.
fn csi(screen: &mut Screen, params: &mut Params, data: &[u8], mut i: usize) -> usize {
    params.len = 0;
    let mut current = Param::default();
    let mut any = false;
    // The private marker (one of < = > ?) when the sequence starts with one.
    let mut private = None;
    let mut intermediate = false;
    let mut malformed = false;
    let start = i;
    while i < data.len() {
        let c = data[i];
        match c {
            b'0'..=b'9' => {
                // A u32 times ten plus a digit fits in u64; one clamp preserves
                // saturating arithmetic without two overflow checks per digit.
                let value = u64::from(current.value.unwrap_or(0)) * 10 + u64::from(c - b'0');
                current.value = Some(value.min(u64::from(u32::MAX)) as u32);
                any = true;
            }
            b';' | b':' => {
                params.push(current);
                current = Param { value: None, sub: c == b':' };
                any = true;
            }
            b'<'..=b'?' if i == start => private = Some(c),
            b'<'..=b'?' => malformed = true,
            0x20..=0x2f => intermediate = true,
            0x40..=0x7e => {
                if any {
                    params.push(current);
                }
                if !(intermediate || malformed) {
                    match private {
                        None => screen.csi(c, params),
                        Some(b'?') => screen.private_csi(c, params),
                        Some(_) => {}
                    }
                }
                return i + 1;
            }
            0x1b => return i,
            0x18 | 0x1a => return i + 1,
            0x00..=0x1f => screen.control(c),
            0x7f => {}
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

const VERSION: &str = "0.1.0";

/// The default font, built in so a lone binary works.
static EMBEDDED_FONT: &[u8] = include_bytes!("../third_party/jetbrains-mono/JetBrainsMono-Regular.ttf");

const USAGE: &str = "\
usage: termshot [options] <log> <out.png>
       termshot [options] --text FILE --json FILE <log> [<out.png>]
       termshot <log> <out.png> <font.ttf> [px] [cols] [rows]

Render the final screen of a terminal log (raw PTY output) as a PNG.
Use - as <log> to read stdin, and - as an output to write stdout.

options:
  -f, --font FILE   TrueType font (default: built-in JetBrains Mono)
      --fallback-font FILE
                    TrueType font for the characters the first lacks, such
                    as CJK or emoji; others are drawn as an empty box
  -p, --px N        font pixel height, above 0 and below 256 (default 48)
  -s, --size CxR    grid size in columns x rows, up to 500x200 (default 100x30)
      --lf-newline  treat each bare LF as CR LF, for logs not captured
                    through a PTY: text files, cmd > out.log, and
                    tmux capture-pane -e -p; a final bare LF ends the
                    last line instead of scrolling
      --cursor COL,ROW|none
                    draw the cursor there, counting from 0 as tmux's
                    #{cursor_x},#{cursor_y} do, or not at all (default:
                    where the log leaves it, unless it hides it)
      --text FILE   write the screen as text, a line per row with trailing
                    spaces trimmed, as tmux capture-pane -p prints it; the
                    PNG is then optional, and fonts are only read for it
      --json FILE   write the screen as JSON: the cursor, and for each row
                    the runs of cells alike in colour (#rrggbb) and
                    attributes, with the column each starts at
  -v, --verbose     print the cell and image size, and the face of each
                    collection, to stderr
  -h, --help        show this help
  -V, --version     show the version

The second form is the original one and still works.
For a font collection (.ttc), FILE#N picks face N, from 0, and FILE#NAME the
face with that full or family name; without either, the first is used.
An SGR reset uses foreground #dbe7f7 on background #111823.

exit status: 0 done; 1 a file could not be read or written, or the font is
unusable; 2 bad arguments, including an image over 134217728 pixels.
";

/// A rendering request from the command line.
struct Options {
    log: String,
    /// The PNG; None when only --text or --json is wanted.
    out: Option<String>,
    text: Option<String>,
    json: Option<String>,
    font: Option<font::Spec>,
    fallback_font: Option<font::Spec>,
    px: f64,
    cols: usize,
    rows: usize,
    lf: Lf,
    /// --cursor: Some(None) hides the cursor; None leaves it to the log.
    cursor: Option<Option<(usize, usize)>>,
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
    let mut lf = Lf::Index;
    let (mut cursor, mut text, mut json) = (None, None, None);
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
            "-h" | "--help" | "-V" | "--version" | "-v" | "--verbose" | "--lf-newline" if attached.is_some() => {
                return Err(format!("{name} takes no value"));
            }
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "-v" | "--verbose" => verbose = true,
            "--lf-newline" => lf = Lf::Newline,
            "-f" | "--font" => font = Some(option_value(&name, attached, &mut args)?),
            "--fallback-font" => fallback_font = Some(option_value(&name, attached, &mut args)?),
            "-p" | "--px" => px = Some(parse_px(&option_value(&name, attached, &mut args)?)?),
            "-s" | "--size" => size = Some(parse_size(&option_value(&name, attached, &mut args)?)?),
            // Checked once the grid size is known.
            "--cursor" => cursor = Some(option_value(&name, attached, &mut args)?),
            "--text" => text = Some(option_value(&name, attached, &mut args)?),
            "--json" => json = Some(option_value(&name, attached, &mut args)?),
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
    let (cols, rows) = size.unwrap_or((
        legacy_cols.unwrap_or(DEFAULT_COLS),
        legacy_rows.unwrap_or(DEFAULT_ROWS),
    ));
    let cursor = cursor.map(|value| parse_cursor(&value, cols, rows)).transpose()?;
    Ok(Command::Render(Options {
        log,
        out,
        text,
        json,
        font: font.as_deref().map(font::Spec::parse).transpose()?,
        fallback_font: fallback_font.as_deref().map(font::Spec::parse).transpose()?,
        px: px.unwrap_or(48.0),
        cols,
        rows,
        lf,
        cursor,
        verbose,
    }))
}

/// The font and fallback font a render asks for, checked and padded.
fn load_fonts(
    options: &Options,
    profile: bool,
    timings: &mut font::LoadTimings,
) -> Result<(font::Font, Option<font::Font>), String> {
    let font = match &options.font {
        Some(path) if profile => font::load_profiled(path, timings)?,
        Some(path) => font::load(path)?,
        // Built in: nothing to read; the check (and padding) is all the work.
        None => {
            let checked = Instant::now();
            let font = font::prepare(EMBEDDED_FONT.to_vec()).map_err(|reason| format!("built-in font: {reason}"));
            timings.check_ms = checked.elapsed().as_secs_f64() * 1000.0;
            font?
        }
    };
    let fallback = options.fallback_font.as_ref().map(font::load).transpose()?;
    Ok((font, fallback))
}

/// Write a text output to its file, or to stdout for -.
/// Why an output can't be written: it names the same file as another output,
/// which would overwrite it, or as an input, which would destroy it. One file
/// can be named many ways (`a`, `./a`, `d/../a`, a symlink), so paths are
/// compared canonical: the file if it exists, else its directory.
fn output_clash(options: &Options) -> Option<String> {
    let canonical = |path: &str| {
        let path = std::path::Path::new(path);
        let dir = path.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
        fs::canonicalize(path)
            .or_else(|_| fs::canonicalize(dir).map(|dir| dir.join(path.file_name().unwrap_or_default())))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    fn named<'a>(pairs: [(&'static str, Option<&'a String>); 3]) -> Vec<(&'static str, &'a String)> {
        pairs.into_iter().filter_map(|(name, path)| Some((name, path.filter(|p| *p != "-")?))).collect()
    }
    let outputs = named([("<out.png>", options.out.as_ref()), ("--text", options.text.as_ref()), ("--json", options.json.as_ref())]);
    fn font_file(spec: &Option<font::Spec>) -> Option<&String> {
        spec.as_ref().map(|spec| &spec.path)
    }
    let inputs = named([("<log>", Some(&options.log)), ("--font", font_file(&options.font)), ("--fallback-font", font_file(&options.fallback_font))]);
    let outputs: Vec<_> = outputs.into_iter().map(|(name, path)| (name, path, canonical(path))).collect();
    for (i, (name, path, file)) in outputs.iter().enumerate() {
        if let Some((other, ..)) = outputs[..i].iter().find(|(.., earlier)| earlier == file) {
            return Some(format!("{name} and {other} name the same file, {path}; give each output its own"));
        }
        if let Some((input, _)) = inputs.iter().find(|(_, input)| canonical(input) == *file) {
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
    let read_ms = read_started.elapsed().as_secs_f64() * 1000.0;
    // The image would still be made, with each line starting where the
    // last one ended; say why, and what fixes it.
    if options.lf == Lf::Index && lacks_cr(&data) {
        eprintln!(
            "termshot: hint: {} has line feeds but no CR, so each line starts where the last ended; \
             if it was not captured through a PTY (a text file, cmd > out.log, tmux capture-pane), pass --lf-newline",
            if options.log == "-" { "stdin" } else { &options.log }
        );
    }

    let font_started = Instant::now();
    let mut font_timings = font::LoadTimings::default();
    // Only the PNG needs fonts; text and JSON come from the cells alone.
    let fonts = match options.out.is_some().then(|| load_fonts(&options, profile, &mut font_timings)).transpose() {
        Ok(fonts) => fonts,
        Err(error) => return cleanup(1, error),
    };
    let font_load_ms = font_started.elapsed().as_secs_f64() * 1000.0;
    // A collection given without a face draws with its first, which may be
    // the wrong script (Noto CJK's is Japanese): say which, and what the others are.
    for (flag, font) in fonts.iter().flat_map(|(font, fallback)| [("--font", Some(font)), ("--fallback-font", fallback.as_ref())]) {
        let Some(font) = font else { continue };
        if let Some(hint) = &font.hint {
            eprintln!("termshot: hint: {flag} {hint}");
        }
        if let (true, Some((index, name))) = (options.verbose, &font.face) {
            eprintln!("{flag} face #{index} {name}");
        }
    }

    let parse_started = Instant::now();
    let input_bytes = data.len();
    let Grid { mut cells, cursor } = replay(&data, options.cols, options.rows, options.lf);
    // Rendering needs only the final grid. Release potentially large logs before
    // allocating the raster and compressor buffers.
    drop(data);
    let parse_ms = parse_started.elapsed().as_secs_f64() * 1000.0;
    let cursor = options.cursor.unwrap_or(cursor);
    let write = |path: &String, output: String| {
        write_output(path, output.as_bytes())
            .map_err(|error| format!("{}: {error}", if path == "-" { "stdout" } else { path }))
    };
    let written = (options.text.as_ref())
        .map_or(Ok(()), |path| write(path, grid_text(&cells, options.cols)))
        .and_then(|()| {
            (options.json.as_ref())
                .map_or(Ok(()), |path| write(path, grid_json(&cells, options.cols, options.rows, cursor)))
        });
    if let Err(message) = written {
        return cleanup(1, message);
    }
    let code = match (&options.out, fonts) {
        (Some(out), Some((font, fallback))) => {
            if let Some((row, col)) = cursor {
                draw_cursor(&mut cells, options.cols, row, col);
            }
            let out = if out == "-" { "/dev/stdout" } else { out };
            let Ok(out) = std::ffi::CString::new(out) else {
                return cleanup(2, "output path contains a nul byte".into());
            };
            unsafe {
                draw_png(
                    cells.as_ptr(),
                    options.cols as i32,
                    options.rows as i32,
                    font.data.as_ptr(),
                    font.start as i32,
                    fallback.as_ref().map_or(std::ptr::null(), |f| f.data.as_ptr()),
                    fallback.as_ref().map_or(0, |f| f.start as i32),
                    options.px,
                    out.as_ptr(),
                    i32::from(options.verbose),
                )
            }
        }
        _ => 0,
    };
    if profile {
        eprintln!("termshot-profile {{\"input_read_ms\":{read_ms:.6},\"parse_ms\":{parse_ms:.6},\"font_load_ms\":{font_load_ms:.6},\"font_read_ms\":{:.6},\"font_check_ms\":{:.6},\"font_padding_ms\":{:.6},\"total_ms\":{:.6},\"input_bytes\":{input_bytes}}}", font_timings.read_ms, font_timings.check_ms, font_timings.padding_ms, started.elapsed().as_secs_f64() * 1000.0);
    }
    // draw.c has already said what went wrong.
    match code {
        0 => ExitCode::SUCCESS,
        code => {
            remove_created(&created);
            ExitCode::from(if code == 2 { 2 } else { 1 })
        }
    }
}
