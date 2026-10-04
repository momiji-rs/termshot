//! Replay a PTY log into a cell grid and paint it.
//! No crates. The rasterizer is draw.c (vendored stb, no window, no system font).

use std::env;
use std::fs;
use std::io::{IsTerminal, Read, Write};
use std::process::ExitCode;
use std::time::Instant;

mod cast;
mod cff;
mod font;
mod graphics;
#[rustfmt::skip]
mod rowcolumn_diacritics;
mod sixel;
mod unicode;
#[rustfmt::skip]
mod unicode_tables;
#[cfg(test)]
mod cast_tests;
#[cfg(test)]
mod cff_tests;
#[cfg(test)]
mod draw_tests;
#[cfg(test)]
mod prescan_tests;
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
    /// BOLD, UNDERLINE, DOUBLE_UNDERLINE, STRIKE, ITALIC, WIDE, TAIL and
    /// OPAQUE bits, as ATTR_* in draw.c.
    attrs: u8,
}

const _: () = assert!(std::mem::size_of::<Cell>() == 12);

/// The most combining marks a cell keeps after its character (#14). A cell
/// holds one code point, so a mark with no precomposed form goes in a side
/// table instead. Four is enough for Thai (a vowel and a tone mark), Hebrew
/// points, stacked Latin accents and the three diacritics of a kitty Unicode
/// placeholder; marks after the fourth are dropped.
const MAX_MARKS: usize = 4;

/// A cell's marks in the order they arrived; the unused slots are 0, which
/// no mark is.
type Marks = [u32; MAX_MARKS];

const NO_MARKS: Marks = [0; MAX_MARKS];

/// One cell's combining marks, for --text, --json and draw.c: the cell's
/// index in screen order (row * cols + col) and its marks. A list of them is
/// sorted by cell, one per cell. As CellMarks in src/draw.c.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellMarks {
    cell: u32,
    marks: Marks,
}

const _: () = assert!(std::mem::size_of::<CellMarks>() == 4 + 4 * MAX_MARKS);

impl CellMarks {
    /// The marks, without the unused slots.
    fn code_points(&self) -> &[u32] {
        &self.marks[..self.marks.iter().position(|&m| m == 0).unwrap_or(MAX_MARKS)]
    }
}

/// The marks of the cell at index (row * cols + col) in a sorted list, or none.
fn marks_of(marks: &[CellMarks], cell: usize) -> &[u32] {
    if marks.is_empty() {
        return &[];
    }
    match marks.binary_search_by_key(&cell, |m| m.cell as usize) {
        Ok(k) => marks[k].code_points(),
        Err(_) => &[],
    }
}

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
/// The background hides an image placed below the cell backgrounds
/// (z < -2^30). Reverse video and the block cursor set it as kitty treats
/// those cells, even in the default colour; before drawing,
/// `opaque_backgrounds` sets it on every other colour, so draw.c needs no
/// copy of DEFAULT_BG.
const OPAQUE: u8 = 128;

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
    /// The foreground and underline (SGR 58) colours as kitty numbers them
    /// for a Unicode placeholder (`Ids`). Neither changes what is drawn;
    /// the underline is drawn in the foreground colour.
    ids: Ids,
}

/// A kitty Unicode placeholder cell's image and placement ids: kitty's
/// `color_to_id` of its foreground and underline colours, 0 for the
/// default, n for palette colour n and 0xRRGGBB for a 24-bit colour. The
/// palette colour, not the RGB it stands for, is what names the image.
type Ids = [u32; 2];

impl Pen {
    const DEFAULT: Pen =
        Pen { fg: DEFAULT_FG, bg: DEFAULT_BG, attrs: 0, dim: false, reverse: false, conceal: false, ids: [0, 0] };

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
        let attrs = self.attrs | if self.reverse { OPAQUE } else { 0 };
        Cell { ch: ' ' as u32, fr: fg.0, fg: fg.1, fb: fg.2, br: bg.0, bg: bg.1, bb: bg.2, attrs }
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

/// The cells draw.c drew as a box because a font maps the character to an
/// empty glyph, as color bitmap fonts do: how many, and the first one, its
/// character and which fonts did. As EmptyGlyphs in src/draw.c.
#[repr(C)]
#[derive(Default)]
struct EmptyGlyphs {
    cp: u32,
    fonts: u32,
    col: i32,
    row: i32,
    cells: usize,
}

const EMPTY_IN_FONT: u32 = 1;
const EMPTY_IN_FALLBACK: u32 = 2;

extern "C" {
    fn draw_cell_size(font: *const u8, font_start: i32, px: f64, w: *mut i32, h: *mut i32) -> i32;
    // font is a face of a font that passed font::check; fallback is another,
    // for the characters the first lacks, or null. empty, if not null, is
    // filled in as EmptyGlyphs says.
    // marks lists the cells' combining marks, sorted by cell (CellMarks).
    fn draw_png_images(cells: *const Cell, marks: *const CellMarks, mark_count: usize, cols: i32, rows: i32,
        font: *const font::Face,
        fallback: *const font::Face, font_size: f64, out_path: *const std::ffi::c_char,
        verbose: i32, images: *const graphics::ImageView, count: usize,
        empty: *mut EmptyGlyphs) -> i32;
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

/// The cursor shapes DECSCUSR (CSI Ps SP q) picks. A still image can't
/// blink, so a blinking shape is drawn as the steady one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}

impl CursorShape {
    const ALL: [CursorShape; 3] = [CursorShape::Block, CursorShape::Underline, CursorShape::Bar];

    fn name(self) -> &'static str {
        match self {
            CursorShape::Block => "block",
            CursorShape::Underline => "underline",
            CursorShape::Bar => "bar",
        }
    }

    /// --cursor-shape's value.
    fn parse(value: &str) -> Result<CursorShape, String> {
        (CursorShape::ALL.into_iter().find(|shape| shape.name() == value))
            .ok_or_else(|| format!("cursor shape must be block, underline or bar, not {value:?}"))
    }
}

/// The grid and the terminal state that writes to it.
///
/// The cursor is always on the grid. After a character is printed in the
/// last column the cursor stays there with a wrap pending; the next printed
/// character wraps first, and most other controls cancel the wrap (xterm's
/// model).
struct Screen {
    graphics: graphics::Graphics,
    other_graphics: graphics::Graphics,
    cell_size: (i32, i32),
    /// Rows of cells in storage order; `map` gives the storage row of each
    /// screen row, so scrolling rotates `map` instead of moving cells.
    cells: Vec<Cell>,
    map: Vec<usize>,
    /// Combining marks with no precomposed form, a Marks per cell in the
    /// same storage order as cells, so scrolling moves them too; every edit
    /// that overwrites, erases or moves cells keeps it in step. Empty until
    /// the screen's first mark, as most logs have none.
    marks: Vec<Marks>,
    /// The Ids of each U+10EEEE cell, kept in step with cells as marks are:
    /// in storage order, empty until the screen's first placeholder with a
    /// colour, and 0 for every other cell.
    ids: Vec<Ids>,
    /// The other screen: the alternate one while on the main one, and back.
    other: Vec<Cell>,
    other_map: Vec<usize>,
    other_marks: Vec<Marks>,
    other_ids: Vec<Ids>,
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
    /// pen.cell(), kept up to date wherever pen changes (SGR, DECRC,
    /// RIS), so printing does not mix colours per character.
    pen_cell: Cell,
    /// Tab stops, one per column; every 8th column at start.
    tabs: Vec<bool>,
    /// G0 and G1. SO shifts to G1, SI back to G0.
    charsets: [Charset; 2],
    shifted: bool,
    /// The last printed character, which REP repeats with the marks its
    /// cell has (at last_at).
    last: Option<u32>,
    /// Where it went (a storage index), for combining marks that follow:
    /// a mark joins the last printed character wherever the cursor has gone
    /// since, as in xterm. None once that cell is erased, overwritten or
    /// moved, and then a mark is dropped.
    last_at: Option<usize>,
    /// DECTCEM (mode 25): whether the cursor is shown. One setting for both
    /// screens, and DECSC does not save it, as in xterm.
    cursor_shown: bool,
    /// DECSCUSR. One setting for both screens; DECSC does not save it and
    /// DECSTR keeps it, as in tmux. RIS resets it, as in xterm.
    cursor_shape: CursorShape,
    /// DECSDM (mode 80): Sixel images go to the top left corner and neither
    /// scroll nor move the cursor.
    sixel_display: bool,
    /// Not terminal state: RIS keeps it, as a reset keeps the tty's settings.
    lf: Lf,
}

impl Screen {
    fn new(cols: usize, rows: usize, lf: Lf) -> Self {
        Self {
            graphics: graphics::Graphics::default(),
            other_graphics: graphics::Graphics::default(),
            cell_size: (1, 1),
            cells: vec![Cell::blank(); cols * rows],
            map: (0..rows).collect(),
            marks: Vec::new(),
            ids: Vec::new(),
            other: vec![Cell::blank(); cols * rows],
            other_map: (0..rows).collect(),
            other_marks: Vec::new(),
            other_ids: Vec::new(),
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
            pen_cell: Pen::DEFAULT.cell(),
            tabs: (0..cols).map(|c| c % 8 == 0).collect(),
            charsets: [Charset::Ascii; 2],
            shifted: false,
            last: None,
            last_at: None,
            cursor_shown: true,
            cursor_shape: CursorShape::Block,
            sixel_display: false,
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
        self.pen_cell = s.pen.cell();
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

    /// The cells that have marks, in screen order, as CellMarks.
    fn screen_marks(&self) -> Vec<CellMarks> {
        if self.marks.is_empty() {
            return Vec::new();
        }
        let line = |r: usize| self.marks[self.line(r)].iter().enumerate();
        (0..self.rows)
            .flat_map(|r| line(r).filter(|(_, m)| m[0] != 0).map(move |(c, &marks)| (r * self.cols + c, marks)))
            .map(|(cell, marks)| CellMarks { cell: cell as u32, marks })
            .collect()
    }

    /// Forget the marks (and placeholder Ids) of storage cells [from, to),
    /// which are being overwritten or erased.
    #[inline]
    fn clear_marks(&mut self, from: usize, to: usize) {
        if !self.marks.is_empty() {
            self.marks[from..to].fill(NO_MARKS);
        }
        if !self.ids.is_empty() {
            self.ids[from..to].fill([0, 0]);
        }
    }

    /// The placeholder cells on the screen, in screen order.
    fn placeholders(&self) -> Vec<graphics::PlaceholderCell> {
        let mut found = Vec::new();
        for r in 0..self.rows {
            let line = self.line(r);
            for (c, cell) in self.cells[line.clone()].iter().enumerate() {
                if cell.ch != graphics::PLACEHOLDER {
                    continue;
                }
                let at = line.start + c;
                let [image, placement] = self.ids.get(at).copied().unwrap_or([0, 0]);
                let marks = self.marks.get(at).copied().unwrap_or(NO_MARKS);
                found.push(graphics::PlaceholderCell {
                    row: r,
                    col: c,
                    image,
                    placement,
                    marks: [marks[0], marks[1], marks[2]],
                });
            }
        }
        found
    }

    /// Before ICH or DCH moves storage cells [at, end) n to the right or
    /// the left, as cells.copy_within does: move their marks too. The cells
    /// it opens are erased after.
    fn move_marks(&mut self, at: usize, end: usize, n: usize, right: bool) {
        if matches!(self.last_at, Some(i) if (at..end).contains(&i)) {
            self.last_at = None;
        }
        if !self.marks.is_empty() {
            if right {
                self.marks.copy_within(at..end - n, at + n);
            } else {
                self.marks.copy_within(at + n..end, at);
            }
        }
        if !self.ids.is_empty() {
            if right {
                self.ids.copy_within(at..end - n, at + n);
            } else {
                self.ids.copy_within(at + n..end, at);
            }
        }
    }

    /// ICH: shift the rest of the line right by n, blanking the gap.
    fn insert_chars(&mut self, n: usize) {
        let line = self.line(self.row);
        let at = line.start + self.col;
        let n = n.min(line.end - at);
        self.move_marks(at, line.end, n, true);
        self.cells.copy_within(at..line.end - n, at + n);
        self.erase(at, at + n);
        self.mend_row(self.row);
    }

    /// DCH: shift the rest of the line left by n, blanking the end.
    fn delete_chars(&mut self, n: usize) {
        let line = self.line(self.row);
        let at = line.start + self.col;
        let n = n.min(line.end - at);
        self.move_marks(at, line.end, n, false);
        self.cells.copy_within(at + n..line.end, at);
        self.erase(line.end - n, line.end);
        self.mend_row(self.row);
    }

    /// Rotate row storage and move graphics together. Pixel clipping uses the
    /// logical distance, not the rotation modulo. Without margins, images
    /// below the screen move the whole distance, however far below they are.
    fn rotate_rows(&mut self, top: usize, bottom: usize, n: usize, up: bool) {
        let height = bottom + 1 - top;
        let distance = i64::try_from(n).unwrap_or(i64::MAX);
        self.graphics.scroll(top, bottom, if up { -distance } else { distance }, self.cell_size.1);
        if up {
            self.map[top..=bottom].rotate_left(n % height);
        } else {
            self.map[top..=bottom].rotate_right(n % height);
        }
    }

    /// Move rows top..=bottom up by n, blanking the n rows that open at the bottom.
    fn scroll_up(&mut self, top: usize, bottom: usize, n: usize) {
        let n = n.min(bottom + 1 - top);
        self.rotate_rows(top, bottom, n, true);
        self.erase_rows(bottom + 1 - n, bottom + 1);
    }

    /// Move rows top..=bottom down by n, blanking the n rows that open at the top.
    fn scroll_down(&mut self, top: usize, bottom: usize, n: usize) {
        let n = n.min(bottom + 1 - top);
        self.rotate_rows(top, bottom, n, false);
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
            80 => self.sixel_display = on,
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
                    self.graphics.clear();
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

    /// Place a Sixel image at the cursor, as xterm with Sixel scrolling on:
    /// the cursor goes to the last text row the image covers, in the same
    /// column, and the scrolling region scrolls up for an image that would
    /// pass its bottom margin. Rows scrolled above the top margin are cut off.
    fn sixel(&mut self, mut image: sixel::Image) {
        if self.sixel_display {
            self.graphics.sixel(&sixel::kitty_command(&image), 0, 0, self.cell_size, self.rows);
            return;
        }
        let (col, mut row) = (self.col, self.row);
        let ch = self.cell_size.1.max(1) as usize;
        let last = row + (image.height as usize + ch - 1) / ch - 1;
        if self.in_margins() && last > self.bottom {
            let scroll = last - self.bottom;
            self.scroll_up(self.top, self.bottom, scroll);
            if scroll > row - self.top {
                let cut = (scroll - (row - self.top)) * ch;
                image.rgba.drain(..cut * image.width as usize * 4);
                image.height -= cut as u32;
                row = self.top;
            } else {
                row -= scroll;
            }
            self.row = self.bottom;
        } else {
            self.row = last.min(self.rows - 1);
        }
        self.pending = false;
        self.graphics.sixel(&sixel::kitty_command(&image), col, row, self.cell_size, self.rows);
    }

    /// Clear the Sixel pixels over rows x cols cells from (row, col), as xterm's
    /// chararea_clear_displayed_graphics does: xterm keeps Sixel pixels with
    /// the cells, so a cell written later, or erased below or above the
    /// cursor, shows instead of them.
    fn clear_sixel(&mut self, row: usize, col: usize, rows: usize, cols: usize) {
        if rows == 0 || cols == 0 || self.graphics.placements.is_empty() {
            return;
        }
        let (cw, ch) = (i64::from(self.cell_size.0), i64::from(self.cell_size.1));
        let (x, y) = (col as i64 * cw, row as i64 * ch);
        self.graphics.erase_sixel(x, y, x + cols as i64 * cw, y + rows as i64 * ch);
    }

    /// Move the cursor past a kitty placement of cols x rows cells, as kitty
    /// does (handle_put_command, screen_handle_graphics_command): right by
    /// cols and down by rows - 1, to the start of the next row if that reaches
    /// the screen's right edge. Past the bottom margin, the region scrolls up
    /// by the overshoot, and the cursor stays on the screen.
    fn move_past_image(&mut self, cols: usize, rows: usize) {
        self.pending = false;
        let (mut col, mut row) = (self.col.saturating_add(cols), self.row.saturating_add(rows.saturating_sub(1)));
        if (col, row) == (self.col, self.row) {
            return;
        }
        // kitty keeps the cursor in the margins only in origin mode, and only
        // if the move left it inside them, before the wrap.
        let (top, bottom) = if self.origin && (self.top..=self.bottom).contains(&row) {
            (self.top, self.bottom)
        } else {
            (0, self.rows - 1)
        };
        if col >= self.cols {
            col = 0;
            row = row.saturating_add(1);
        }
        if row > self.bottom {
            self.scroll_up(self.top, self.bottom, (row - self.bottom).min(self.rows));
        }
        self.col = col.min(self.cols - 1);
        self.row = row.clamp(top, bottom);
    }

    /// Switch to the alternate screen or back. clear_first blanks the
    /// alternate screen before leaving it (mode 1047).
    fn use_alternate(&mut self, on: bool, clear_first: bool) {
        if on == self.on_alternate {
            return;
        }
        if clear_first {
            self.graphics.clear();
            self.erase_rows(0, self.rows);
        }
        self.graphics.abort();
        self.other_graphics.abort();
        std::mem::swap(&mut self.graphics, &mut self.other_graphics);
        std::mem::swap(&mut self.cells, &mut self.other);
        std::mem::swap(&mut self.map, &mut self.other_map);
        std::mem::swap(&mut self.marks, &mut self.other_marks);
        std::mem::swap(&mut self.ids, &mut self.other_ids);
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
            // and wraps; without autowrap the character is dropped, and so
            // are the marks that follow it.
            if !self.autowrap {
                self.last_at = None;
                return;
            }
            self.col = 0;
            self.index();
        }
        let line = self.line(self.row);
        let at = line.start + self.col;
        self.split_wide(&line, at);
        let mut cell = Cell { ch, ..self.pen_cell };
        if width == 2 {
            self.split_wide(&line, at + 1);
            cell.attrs |= WIDE;
            self.cells[at + 1] = Cell { ch: 0, attrs: cell.attrs & !WIDE | TAIL, ..cell };
        }
        self.cells[at] = cell;
        self.clear_marks(at, at + width);
        if ch == graphics::PLACEHOLDER && self.pen.ids != [0, 0] {
            if self.ids.is_empty() {
                self.ids = vec![[0, 0]; self.cells.len()];
            }
            self.ids[at] = self.pen.ids;
        }
        self.last_at = Some(at);
        self.clear_sixel(self.row, self.col, 1, width);
        if self.col + width <= self.last_col() {
            self.col += width;
        } else {
            self.col = self.last_col();
            self.pending = self.autowrap;
        }
    }

    /// A zero-width character joins the last printed character: composed
    /// with it when Unicode has a precomposed form (e + U+0301 is é) and the
    /// cell has no marks yet, otherwise kept in the cell's marks, up to
    /// MAX_MARKS. REP repeats the character as it then stands.
    fn combine(&mut self, mark: u32) {
        let Some(at) = self.last_at else {
            return;
        };
        if self.marks.get(at).map_or(true, |m| m[0] == 0) {
            if let Some(composed) = unicode::compose(self.cells[at].ch, mark) {
                self.cells[at].ch = composed;
                self.last = Some(composed);
                return;
            }
        }
        if self.marks.is_empty() {
            self.marks = vec![NO_MARKS; self.cells.len()];
        }
        if let Some(slot) = self.marks[at].iter_mut().find(|m| **m == 0) {
            *slot = mark;
        }
    }

    /// Before cell i of a row is overwritten: if it is half of a wide
    /// character, blank the other half.
    #[inline]
    fn split_wide(&mut self, line: &std::ops::Range<usize>, i: usize) {
        // Most cells are neither half: test that inline, split out of line.
        if self.cells[i].attrs & (WIDE | TAIL) != 0 {
            self.split_wide_cell(line, i);
        }
    }

    #[inline(never)]
    fn split_wide_cell(&mut self, line: &std::ops::Range<usize>, i: usize) {
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
        self.clear_marks(i, i + 1);
        if self.last_at == Some(i) {
            self.last_at = None;
        }
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
        let mut cell = self.pen_cell;
        while !text.is_empty() {
            if self.pending {
                if self.autowrap && self.row == self.bottom {
                    // Within this uninterrupted run, complete rows older than
                    // one scrolling region cannot survive. Rotate storage as
                    // if they were printed, then materialize the surviving rows.
                    let height = self.bottom + 1 - self.top;
                    let skip_rows = (text.len() / self.cols).saturating_sub(height);
                    if skip_rows > 0 {
                        self.rotate_rows(self.top, self.bottom, skip_rows, true);
                        text = &text[skip_rows * self.cols..];
                    }
                }
                self.col = 0;
                if self.row == self.bottom && text.len() >= self.cols {
                    // The entire incoming row is overwritten below; avoid
                    // clearing it just before assigning every cell again.
                    self.pending = false;
                    self.rotate_rows(self.top, self.bottom, 1, true);
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
            self.clear_marks(start, start + count);
            self.last_at = Some(start + count - 1);
            for (dest, byte) in self.cells[start..start + count].iter_mut().zip(text) {
                cell.ch = u32::from(*byte);
                *dest = cell;
            }
            self.clear_sixel(self.row, self.col, 1, count);
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
                        self.clear_marks(at, at + 1);
                        self.last_at = Some(at);
                        self.clear_sixel(self.row, self.col, 1, 1);
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
            fill_cells(&mut self.cells[from..to], blank);
            self.clear_marks(from, to);
            if matches!(self.last_at, Some(i) if (from..to).contains(&i)) {
                self.last_at = None;
            }
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
                    let marks = self.last_at.and_then(|at| self.marks.get(at).copied()).unwrap_or(NO_MARKS);
                    for _ in 0..count(0).min(self.cols * self.rows) {
                        self.print_mapped(ch);
                        for &mark in marks.iter().take_while(|&&m| m != 0) {
                            self.combine(mark);
                        }
                    }
                }
            }
            // xterm saves the same state for CSI s as for DECSC.
            b's' => self.save_cursor(),
            b'u' => self.restore_cursor(),
            b'm' => {
                self.sgr(p);
                self.pen_cell = self.pen.cell();
            }
            b'J' => {
                let (cursor, line) = (self.cursor_index(), self.line(self.row));
                match p.get(0, 0) {
                    // xterm erases Sixel pixels in the rows below or above the
                    // cursor's, but not in its own (ClearBelow, ClearAbove).
                    0 => {
                        self.erase(cursor, line.end);
                        self.erase_rows(self.row + 1, self.rows);
                        self.clear_sixel(self.row + 1, 0, self.rows - self.row - 1, self.cols);
                    }
                    1 => {
                        self.erase_rows(0, self.row);
                        self.erase(line.start, cursor + 1);
                        self.clear_sixel(0, 0, self.row, self.cols);
                    }
                    2 => {
                        self.erase_rows(0, self.rows);
                        self.graphics.clear();
                    },
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

    /// DECSCUSR: 0 (or none) to 2 a block, 3 and 4 an underline, 5 and 6 a
    /// bar. Other values change nothing, and only the first parameter counts,
    /// as in tmux.
    fn set_cursor_shape(&mut self, p: &Params) {
        self.cursor_shape = match p.get(0, 0) {
            0..=2 => CursorShape::Block,
            3 | 4 => CursorShape::Underline,
            5 | 6 => CursorShape::Bar,
            _ => return,
        };
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
                    (38 | 48 | 58, Some(2)) => {
                        let color = match subs.len() {
                            4 => rgb(&subs[1..4]),
                            n if n >= 5 => rgb(&subs[2..5]),
                            _ => None,
                        };
                        if let Some(color) = color {
                            self.set_color(v, color, rgb_id(color));
                        }
                    }
                    (38 | 48 | 58, Some(5)) => {
                        let n = subs.get(1).map_or(0, |n| n.value.unwrap_or(0));
                        if let Some(color) = palette(n) {
                            self.set_color(v, color, n);
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
                30..=37 => (pen.fg, pen.ids[0]) = (palette(v - 30).unwrap(), v - 30),
                40..=47 => pen.bg = palette(v - 40).unwrap(),
                90..=97 => (pen.fg, pen.ids[0]) = (palette(v - 90 + 8).unwrap(), v - 90 + 8),
                100..=107 => pen.bg = palette(v - 100 + 8).unwrap(),
                39 => (pen.fg, pen.ids[0]) = (DEFAULT_FG, 0),
                49 => pen.bg = DEFAULT_BG,
                59 => pen.ids[1] = 0,
                // Blink (5, 6, 25) and the rest are not drawn.
                38 | 48 | 58 => match p.get(k + 1, 0) {
                    2 if k + 4 < p.len => {
                        if let Some(color) = rgb(&p.list[k + 2..k + 5]) {
                            self.set_color(v, color, rgb_id(color));
                        }
                        k += 4;
                    }
                    5 if k + 2 < p.len => {
                        let n = p.get(k + 2, 0);
                        if let Some(color) = palette(n) {
                            self.set_color(v, color, n);
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

    /// SGR 38, 48 or 58 (`which`): the colour, and its kitty id (`Ids`).
    /// The underline colour only names a placeholder's placement.
    fn set_color(&mut self, which: u32, color: (u8, u8, u8), id: u32) {
        match which {
            38 => (self.pen.fg, self.pen.ids[0]) = (color, id),
            48 => self.pen.bg = color,
            _ => self.pen.ids[1] = id,
        }
    }
}

/// Set every cell to cell. A 12-byte Cell defeats the vectorized fill, so
/// a row is filled by copying what is already filled, doubling each time.
fn fill_cells(cells: &mut [Cell], cell: Cell) {
    if cells.len() <= 8 {
        cells.fill(cell);
        return;
    }
    cells[0] = cell;
    let mut done = 1;
    while done < cells.len() {
        let n = done.min(cells.len() - done);
        cells.copy_within(..n, done);
        done += n;
    }
}

/// A 24-bit colour's kitty id: 0xRRGGBB.
fn rgb_id((r, g, b): (u8, u8, u8)) -> u32 {
    u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
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

/// Where the run of printable ASCII (0x20..=0x7e) from i ends. Most runs
/// are short, so the first 16 bytes go one at a time; then eight at a time
/// while they all are printable (#21), and the rest one at a time again.
fn printable_end(data: &[u8], mut i: usize) -> usize {
    const ONES: u64 = u64::from_ne_bytes([0x01; 8]);
    const HIGH: u64 = u64::from_ne_bytes([0x80; 8]);
    let short = data.len().min(i + 16);
    while i < short {
        if !(0x20..0x7f).contains(&data[i]) {
            return i;
        }
        i += 1;
    }
    while let Some(chunk) = data.get(i..i + 8) {
        let w = u64::from_ne_bytes(chunk.try_into().unwrap());
        // A byte below 0x20, one with its high bit set, or 0x7f; the
        // first and third are the bit trick for "has a byte below n".
        let del = w ^ (ONES * 0x7f);
        if (w.wrapping_sub(ONES * 0x20) & !w | w | del.wrapping_sub(ONES) & !del) & HIGH != 0 {
            break;
        }
        i += 8;
    }
    while i < data.len() && (0x20..0x7f).contains(&data[i]) {
        i += 1;
    }
    i
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

/// The bytes of an eight-byte word equal to `byte`, as their high bits.
/// Exact for every byte: `(x & 0x7f) + 0x7f` cannot carry into the next one.
fn bytes_equal(word: u64, byte: u8) -> u64 {
    const LOW: u64 = u64::from_le_bytes([0x7f; 8]);
    let x = word ^ u64::from_le_bytes([byte; 8]);
    !((x & LOW).wrapping_add(LOW) | x | LOW)
}

/// Calls `found` with the introducer (`P` or `_`) and the bytes of each DCS
/// and APC string of a log, from after the introducer through where
/// `skip_string` ends it, in order, until `found` returns true. Like the
/// byte-at-a-time scans it replaced, it looks only at each ESC and the byte
/// after it, not at the parser's state. Those scans also skipped OSC, PM
/// and SOS strings, but a skip passes no ESC except the one of an ST, so
/// it never hid an ESC P or ESC _: every one of those is a string here.
/// Sixteen bytes at a time are tested for them (#21).
fn any_string(data: &[u8], mut found: impl FnMut(u8, &[u8]) -> bool) -> bool {
    let mut i = 0;
    while i + 1 < data.len() {
        // With no branch on where the ESCs are: in an escape-heavy log,
        // which word holds one is hard to predict.
        let at = match data.get(i..i + 17) {
            Some(block) => {
                let word = |at: usize| u64::from_le_bytes(block[at..at + 8].try_into().unwrap());
                let hits = |at: usize| {
                    bytes_equal(word(at), 0x1b) & (bytes_equal(word(at + 1), b'P') | bytes_equal(word(at + 1), b'_'))
                };
                let both = u128::from(hits(0)) | u128::from(hits(8)) << 64;
                if both == 0 {
                    i += 16;
                    continue;
                }
                i + both.trailing_zeros() as usize / 8
            }
            None if data[i] == 0x1b && matches!(data[i + 1], b'P' | b'_') => i,
            None => {
                i += 1;
                continue;
            }
        };
        let end = skip_string(data, at + 2);
        if found(data[at + 1], &data[at + 2..end]) {
            return true;
        }
        i = end;
    }
    false
}

/// Whether a run without a PNG must still read fonts: a kitty command or a
/// Sixel image whose placement can move the text cursor by cells of the
/// font's size. One pass over the log for both (#21).
fn needs_cell_metrics(data: &[u8]) -> bool {
    let mut kitty = graphics::CellMetricsScan::default();
    any_string(data, |kind, s| match kind {
        b'_' => kitty.string(s),
        b'P' => sixel::string_needs_cell_metrics(s),
        _ => false,
    })
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

/// The screen a log leaves: its cells in screen order, the cursor as
/// (row, col) unless the log hid it, and its shape.
struct Grid {
    images: Vec<graphics::Placement>,
    cells: Vec<Cell>,
    /// The cells' combining marks, sorted by cell.
    marks: Vec<CellMarks>,
    cursor: Option<(usize, usize)>,
    cursor_shape: CursorShape,
}

#[cfg(test)]
fn replay(data: &[u8], cols: usize, rows: usize, lf: Lf) -> Grid {
    replay_sized(data, cols, rows, lf, (1, 1))
}

fn replay_sized(data: &[u8], cols: usize, rows: usize, lf: Lf, cell_size: (i32, i32)) -> Grid {
    let data = match lf {
        Lf::Newline => strip_final_bare_lf(data),
        Lf::Index => data,
    };
    let mut screen = Screen::new(cols, rows, lf);
    screen.cell_size = cell_size;
    // Reuse the fixed parameter buffer across sequences; only len needs resetting.
    let mut params = Params { list: [Param::default(); MAX_PARAMS], len: 0 };
    // One budget for the whole log: a reset does not refill it.
    let mut sixel_budget = sixel::Budget::default();
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if (0x20..0x7f).contains(&b) {
            let start = i;
            i = printable_end(data, i + 1);
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
                b'_' if data.get(i) == Some(&b'G') => {
                    let end = skip_string(data, i);
                    // Only ST commits a graphics command; BEL/CAN/SUB, another
                    // escape, or EOF discard it and any incomplete upload.
                    if end >= i + 3 && data.get(end - 2..end) == Some(b"\x1b\\") {
                        if let Some((dc, dr)) = screen.graphics.command(
                            &data[i + 1..end - 2], screen.col, screen.row, cell_size, rows,
                        ) {
                            screen.move_past_image(dc, dr);
                        }
                    } else { screen.graphics.abort(); }
                    i = end;
                }
                b'P' => {
                    let end = skip_string(data, i);
                    // As for kitty graphics, only ST commits an image.
                    if end >= i + 2 && data.get(end - 2..end) == Some(b"\x1b\\") {
                        if let Some(image) = sixel::decode(&data[i..end - 2], &mut sixel_budget) {
                            screen.sixel(image);
                        }
                    }
                    i = end;
                }
                b']' | b'_' | b'^' | b'X' => i = skip_string(data, i),
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
                b'c' => {
                    // kitty's reset clears both screens' images but keeps
                    // their virtual placements (grman_clear).
                    let mut kept = [std::mem::take(&mut screen.graphics), std::mem::take(&mut screen.other_graphics)];
                    if screen.on_alternate {
                        kept.swap(0, 1);
                    }
                    for graphics in &mut kept {
                        graphics.abort();
                        graphics.clear();
                    }
                    screen = Screen::new(cols, rows, lf);
                    screen.cell_size = cell_size;
                    [screen.graphics, screen.other_graphics] = kept;
                },
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
    // Placeholder cells show nothing without a virtual placement to name.
    let placeholders = if screen.graphics.has_virtual() { screen.placeholders() } else { Vec::new() };
    let images = std::mem::take(&mut screen.graphics).finish(&placeholders, cell_size, rows);
    let cursor_shape = screen.cursor_shape;
    let marks = screen.screen_marks();
    Grid { cells: screen.into_cells(), marks, cursor, cursor_shape, images }
}

/// kitty draws a Unicode placeholder (U+10EEEE) as a blank cell, its
/// diacritics too: the image it shows comes from graphics::Graphics::finish.
/// Make each one a space, with its colours and attributes, and drop its
/// marks, for draw.c. --text and --json keep them.
fn blank_placeholders(cells: &mut [Cell], mut marks: Vec<CellMarks>) -> Vec<CellMarks> {
    marks.retain(|m| cells[m.cell as usize].ch != graphics::PLACEHOLDER);
    for cell in cells.iter_mut().filter(|cell| cell.ch == graphics::PLACEHOLDER) {
        cell.ch = ' ' as u32;
    }
    marks
}

/// Mark every background that is not the default colour OPAQUE, for draw.c.
/// kitty compares the colour's value, so a background set to the default
/// colour explicitly is a default one.
fn opaque_backgrounds(cells: &mut [Cell]) {
    for cell in cells {
        if (cell.br, cell.bg, cell.bb) != DEFAULT_BG {
            cell.attrs |= OPAQUE;
        }
    }
}

/// Draw the cursor as a block in reverse video over the cell at (row, col),
/// or over both cells of the wide character it is on.
fn draw_cursor(cells: &mut [Cell], cols: usize, row: usize, col: usize) {
    let under = cursor_cells(cells, cols, row, col);
    for cell in &mut cells[under] {
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
        cell.attrs |= OPAQUE;
    }
}

/// The cells the cursor at (row, col) covers: both halves of a wide
/// character, unless the line cuts it.
fn cursor_cells(cells: &[Cell], cols: usize, row: usize, col: usize) -> std::ops::Range<usize> {
    let line = row * cols..(row + 1) * cols;
    let mut start = line.start + col;
    if cells[start].attrs & TAIL != 0 && start > line.start {
        start -= 1;
    }
    let end = if cells[start].attrs & WIDE != 0 { start + 2 } else { start + 1 };
    start..end.min(line.end)
}

/// An underline or bar cursor at (row, col), in pixels for cells of
/// cell_w x cell_h: the rectangle (x, y, w, h) and its colour. The underline
/// runs along the bottom of the cells the cursor covers, the bar down the left
/// edge of the first. Both are an eighth of a cell wide, at least a pixel, in
/// the default foreground; on a cell whose background is that colour, in the
/// default background, so the cursor still shows.
fn cursor_mark(
    cells: &[Cell],
    cols: usize,
    (row, col): (usize, usize),
    shape: CursorShape,
    (cell_w, cell_h): (i32, i32),
) -> ((i64, i64, i64, i64), [u8; 4]) {
    let under = cursor_cells(cells, cols, row, col);
    let first = &cells[under.start];
    let (r, g, b) = if (first.br, first.bg, first.bb) == DEFAULT_FG { DEFAULT_BG } else { DEFAULT_FG };
    let (cell_w, cell_h) = (i64::from(cell_w), i64::from(cell_h));
    let thick = (cell_w / 8).max(1);
    let (x, y) = ((under.start - row * cols) as i64 * cell_w, row as i64 * cell_h);
    let rect = match shape {
        CursorShape::Underline => (x, y + cell_h - thick, under.len() as i64 * cell_w, thick),
        CursorShape::Bar | CursorShape::Block => (x, y, thick, cell_h),
    };
    (rect, [r, g, b, 255])
}

/// A code point as a char; none of the grid's are invalid.
fn to_char(cp: u32) -> char {
    char::from_u32(cp).unwrap_or('\u{fffd}')
}

/// The screen as text, the way tmux capture-pane -p prints it: a line per row
/// with its trailing spaces trimmed, a wide character once, and each
/// character followed by its combining marks.
fn grid_text(cells: &[Cell], marks: &[CellMarks], cols: usize) -> String {
    let mut text = String::with_capacity(cells.len() + cells.len() / cols);
    for (r, row) in cells.chunks(cols).enumerate() {
        if marks.is_empty() {
            text.extend(row.iter().filter(|c| c.attrs & TAIL == 0).map(|c| to_char(c.ch)));
        } else {
            for (c, cell) in row.iter().enumerate().filter(|(_, c)| c.attrs & TAIL == 0) {
                text.push(to_char(cell.ch));
                text.extend(marks_of(marks, r * cols + c).iter().map(|&m| to_char(m)));
            }
        }
        // The previous row's newline stops the trim.
        text.truncate(text.trim_end_matches(' ').len());
        text.push('\n');
    }
    text
}

/// The screen as JSON: the grid size; the cursor, or null when hidden; and a
/// line per row of the runs of cells alike in colour and attributes, each with
/// the column it starts at (a wide character takes two). A run's text has each
/// character followed by its combining marks, as --text does. Blank cells that
/// end a row are left out, as in --text, unless their background or a line shows.
fn grid_json(
    cells: &[Cell],
    marks: &[CellMarks],
    cols: usize,
    rows: usize,
    cursor: Option<(usize, usize)>,
    shape: CursorShape,
) -> String {
    use std::fmt::Write as _;
    const LINES: u8 = UNDERLINE | DOUBLE_UNDERLINE | STRIKE;
    const STYLE: u8 = BOLD | ITALIC | LINES;
    let style = |c: &Cell| ((c.fr, c.fg, c.fb), (c.br, c.bg, c.bb), c.attrs & STYLE);
    // Writing to a String cannot fail.
    let mut json = String::with_capacity(cells.len() * 2);
    let _ = write!(json, "{{\"cols\":{cols},\"rows\":{rows},\"cursor\":");
    let _ = match cursor {
        Some((row, col)) => write!(json, "{{\"col\":{col},\"row\":{row},\"shape\":\"{}\"}}", shape.name()),
        None => write!(json, "null"),
    };
    json.push_str(",\"lines\":[");
    for (r, row) in cells.chunks(cols).enumerate() {
        json.push_str(if r == 0 { "\n[" } else { ",\n[" });
        let marks_at = |c: usize| marks_of(marks, r * cols + c);
        let blank = |c: usize| {
            let cell = &row[c];
            cell.ch == ' ' as u32 && (cell.br, cell.bg, cell.bb) == DEFAULT_BG && cell.attrs & LINES == 0
                && marks_at(c).is_empty()
        };
        let end = (0..cols).rposition(|c| !blank(c)).map_or(0, |i| i + 1);
        let mut c = 0;
        while c < end {
            let start = c;
            let key = style(&row[c]);
            let _ = write!(json, "{}{{\"col\":{start},\"text\":\"", if start == 0 { "" } else { "," });
            // A wide character's tail is part of the run its first half is in.
            while c < end && (style(&row[c]) == key || row[c].attrs & TAIL != 0) {
                if row[c].attrs & TAIL == 0 {
                    for &cp in std::iter::once(&row[c].ch).chain(marks_at(c)) {
                        match to_char(cp) {
                            '"' => json.push_str("\\\""),
                            '\\' => json.push_str("\\\\"),
                            ch if ch < ' ' || ch == '\u{7f}' => {
                                let _ = write!(json, "\\u{:04x}", ch as u32);
                            }
                            ch => json.push(ch),
                        }
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
    // The intermediate byte; a second one makes a sequence nothing here uses.
    let mut intermediate = None;
    let mut malformed = false;
    let start = i;
    while i < data.len() {
        let c = data[i];
        // Digits, ':' and ';' first, with one test: they are most of every
        // sequence (#21). After an intermediate they are malformed, below.
        let offset = c.wrapping_sub(b'0');
        if offset <= b';' - b'0' && intermediate.is_none() {
            if offset < 10 {
                let value = u64::from(current.value.unwrap_or(0)) * 10 + u64::from(offset);
                current.value = Some(value.min(u64::from(u32::MAX)) as u32);
            } else {
                params.push(current);
                current = Param { value: None, sub: c == b':' };
            }
            any = true;
            i += 1;
            continue;
        }
        match c {
            // Parameter bytes come before intermediates, never after.
            0x30..=0x3f if intermediate.is_some() => malformed = true,
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
            0x20..=0x2f => {
                malformed |= intermediate.is_some();
                intermediate = Some(c);
            }
            0x40..=0x7e => {
                if any {
                    params.push(current);
                }
                if !malformed {
                    match (private, intermediate, c) {
                        (None, None, _) => screen.csi(c, params),
                        (Some(b'?'), None, _) => screen.private_csi(c, params),
                        (None, Some(b' '), b'q') => screen.set_cursor_shape(params),
                        _ => {}
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
      --text FILE   write the screen as text, a line per row with trailing
                    spaces trimmed, as tmux capture-pane -p prints it; the
                    PNG is then optional, and fonts are only read for it
      --json FILE   write the screen as JSON: the cursor and its shape, and for
                    each row the runs of cells alike in colour (#rrggbb)
                    and attributes, with the column each starts at
  -v, --verbose     print the cell and image size, and the face of each
                    collection, to stderr
  -h, --help        show this help
  -V, --version     show the version

The second form is the original one and still works.
For a font collection (.ttc), FILE#N picks face N, from 0, and FILE#NAME the
face with that full or family name; without either, the first is used.
An SGR reset uses foreground #dbe7f7 on background #111823.

A cast replays its output events in order, on the grid of the size it ends
at (its last resize event, or its header); input and markers are ignored.

exit status: 0 done; 1 a file could not be read or written, a cast is
malformed, or the font is unusable; 2 bad arguments, including an image over
134217728 pixels, or a cast's size beyond 500x200 without --size.
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
    let (mut lf, mut cast) = (Lf::Index, None);
    let (mut cursor, mut cursor_shape, mut text, mut json) = (None, None, None, None);
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
            "--cursor-shape" => cursor_shape = Some(CursorShape::parse(&option_value(&name, attached, &mut args)?)?),
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
        font: font.as_deref().map(font::Spec::parse).transpose()?,
        fallback_font: fallback_font.as_deref().map(font::Spec::parse).transpose()?,
        px: px.unwrap_or(48.0),
        cols,
        rows,
        lf,
        cast,
        cursor,
        cursor_shape,
        verbose,
    }))
}

/// The font and fallback font a render asks for, checked and padded.
/// Each font's load is timed on the same boundaries (font::LoadTimings),
/// whether it is built in or a file.
fn load_fonts(
    options: &Options,
    timings: &mut [font::LoadTimings; 2],
) -> Result<(font::Font, Option<font::Font>), String> {
    let [font_timings, fallback_timings] = timings;
    let font = match &options.font {
        Some(path) => font::load_timed(path, font_timings)?,
        None => font::prepare_timed(EMBEDDED_FONT, font_timings).map_err(|reason| format!("built-in font: {reason}"))?,
    };
    let fallback = options.fallback_font.as_ref().map(|path| font::load_timed(path, fallback_timings)).transpose()?;
    Ok((font, fallback))
}

/// The warning for characters drawn as boxes because a font maps them to
/// empty glyphs: which cell, which fonts, and what to pass instead.
fn empty_glyph_warning(
    empty: &EmptyGlyphs,
    options: &Options,
    font: &font::Font,
    fallback: Option<&font::Font>,
) -> Option<String> {
    if empty.cells == 0 {
        return None;
    }
    let mut color = false;
    let mut name = |flag: &str, spec: Option<&font::Spec>, font: &font::Font| {
        let mut name = spec.map_or("the built-in font".to_owned(), |spec| match &spec.face {
            Some(face) => format!("{flag} {}#{face}", spec.path),
            None => format!("{flag} {}", spec.path),
        });
        if let Some(tag) = font::color_bitmap(font) {
            color = true;
            name += &format!(" (a color bitmap font, {tag}, which termshot cannot draw)");
        }
        name
    };
    let mut blamed = Vec::new();
    if empty.fonts & EMPTY_IN_FONT != 0 {
        blamed.push(name("--font", options.font.as_ref(), font));
    }
    if let (true, Some(fallback)) = (empty.fonts & EMPTY_IN_FALLBACK != 0, fallback) {
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
    Some(format!(
        "warning: U+{:04X} at column {}, row {} (from 0) is drawn as a box{cells}: {blamed}; {instead}{}",
        empty.cp,
        empty.col,
        empty.row,
        if color { ", such as Noto Emoji" } else { "" }
    ))
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
    fn named<'a>(pairs: [(&'static str, Option<&'a String>); 3]) -> Vec<(&'static str, &'a String)> {
        pairs.into_iter().filter_map(|(name, path)| Some((name, path.filter(|p| *p != "-")?))).collect()
    }
    let outputs = named([("<out.png>", options.out.as_ref()), ("--text", options.text.as_ref()), ("--json", options.json.as_ref())]);
    fn font_file(spec: &Option<font::Spec>) -> Option<&String> {
        spec.as_ref().map(|spec| &spec.path)
    }
    let inputs = named([("<log>", Some(&options.log)), ("--font", font_file(&options.font)), ("--fallback-font", font_file(&options.fallback_font))]);
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
    let input_bytes = data.len();
    let name = if options.log == "-" { "stdin" } else { &options.log };
    // An asciinema recording replays its output events; its header gives
    // the grid size that --size and the original form's cols and rows don't.
    // Decoding it is part of reading the log, in the profile too.
    let (data, cast_size) = if options.cast.unwrap_or_else(|| cast::detect(&data)) {
        match cast::decode(data) {
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
    if options.lf == Lf::Index && lacks_cr(&data) {
        eprintln!(
            "termshot: hint: {name} has line feeds but no CR, so each line starts where the last ended; \
             if it was not captured through a PTY (a text file, cmd > out.log, tmux capture-pane), pass --lf-newline"
        );
    }

    let font_started = Instant::now();
    let mut font_timings = [font::LoadTimings::default(); 2];
    // Plain text/JSON logs need no fonts. Graphics also need cell metrics,
    // even without a PNG, because placement can move the text cursor.
    let needs_fonts = options.out.is_some() || needs_cell_metrics(&data);
    let fonts = match needs_fonts.then(|| load_fonts(&options, &mut font_timings)).transpose() {
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
    let (mut cell_w, mut cell_h) = (1, 1);
    if let Some((font, _)) = &fonts {
        if unsafe { draw_cell_size(font.data.as_ptr(), font.start as i32, options.px, &mut cell_w, &mut cell_h) } == 0 {
            return cleanup(1, "font metrics unusable".into());
        }
    }
    let Grid { mut cells, marks, cursor, cursor_shape, images } =
        replay_sized(&data, cols, rows, options.lf, (cell_w, cell_h));
    let mut image_views: Vec<_> = images.iter().flat_map(graphics::Placement::views).collect();
    // Rendering needs only the final grid. Release potentially large logs before
    // allocating the raster and compressor buffers.
    drop(data);
    let parse_ms = parse_started.elapsed().as_secs_f64() * 1000.0;
    let cursor = cursor_option.unwrap_or(cursor);
    let cursor_shape = options.cursor_shape.unwrap_or(cursor_shape);
    let write = |path: &String, output: String| {
        write_output(path, output.as_bytes())
            .map_err(|error| format!("{}: {error}", if path == "-" { "stdout" } else { path }))
    };
    let written = (options.text.as_ref())
        .map_or(Ok(()), |path| write(path, grid_text(&cells, &marks, cols)))
        .and_then(|()| {
            (options.json.as_ref())
                .map_or(Ok(()), |path| write(path, grid_json(&cells, &marks, cols, rows, cursor, cursor_shape)))
        });
    if let Err(message) = written {
        return cleanup(1, message);
    }
    let mut empty = EmptyGlyphs::default();
    // The underline or bar cursor's colour; its view borrows it.
    let mark_pixel;
    let mut face_ms = 0.0;
    let code = match (&options.out, &fonts) {
        (Some(out), Some((font, fallback))) => {
            let marks = blank_placeholders(&mut cells, marks);
            match (cursor, cursor_shape) {
                (None, _) => {}
                (Some((row, col)), CursorShape::Block) => draw_cursor(&mut cells, cols, row, col),
                // As kitty draws it, with the text: over the images under
                // the text, under those of z-index 0 and up, so it goes first
                // among the views drawn after the text.
                (Some(at), shape) => {
                    let ((x, y, w, h), pixel) = cursor_mark(&cells, cols, at, shape, (cell_w, cell_h));
                    mark_pixel = pixel;
                    image_views.insert(0, graphics::ImageView::solid(&mark_pixel, x, y, w, h));
                }
            }
            opaque_backgrounds(&mut cells);
            let out = if out == "-" { "/dev/stdout" } else { out };
            let Ok(out) = std::ffi::CString::new(out) else {
                return cleanup(2, "output path contains a nul byte".into());
            };
            // with_face parses a CFF table again; that is face_ms.
            let face_started = Instant::now();
            let mut draw = |font: &font::Face, fallback: Option<&font::Face>| unsafe {
                face_ms = face_started.elapsed().as_secs_f64() * 1000.0;
                draw_png_images(
                    cells.as_ptr(),
                    marks.as_ptr(),
                    marks.len(),
                    cols as i32,
                    rows as i32,
                    font,
                    fallback.map_or(std::ptr::null(), |f| f as *const font::Face),
                    options.px,
                    out.as_ptr(),
                    i32::from(options.verbose),
                    image_views.as_ptr(),
                    image_views.len(),
                    &mut empty,
                )
            };
            let drawn = font.with_face(|font| match &fallback {
                None => Ok(draw(font, None)),
                Some(fallback) => fallback.with_face(|fallback| draw(font, Some(fallback))),
            });
            match drawn.and_then(|code| code) {
                Ok(code) => code,
                Err(message) => return cleanup(1, message),
            }
        }
        _ => 0,
    };
    if let (0, Some((font, fallback))) = (code, &fonts) {
        if let Some(warning) = empty_glyph_warning(&empty, &options, font, fallback.as_ref()) {
            eprintln!("termshot: {warning}");
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
        eprintln!(
            "termshot-profile {{{fields},\"font_builtin\":{builtin},\"face_ms\":{face_ms:.6},\"total_ms\":{:.6},\"input_bytes\":{input_bytes}}}",
            started.elapsed().as_secs_f64() * 1000.0
        );
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
