//! The screen model: the grid of cells, the cursor, the pen SGR sets, the
//! scrolling region, the alternate screen, tab stops and character sets, and
//! the edits every control and escape makes to them. vt.rs reads the log and
//! calls into it.

use crate::cell::{Cell, CellMarks, Marks, MAX_MARKS, BOLD, DOUBLE_UNDERLINE, ITALIC, OPAQUE, STRIKE, TAIL, UNDERLINE, WIDE};
use crate::palette::Palette;
use crate::vt::{rgb, Params, ParseOptions};
use crate::{graphics, sixel, unicode};

pub(crate) const NO_MARKS: Marks = [0; MAX_MARKS];

/// The colours and attributes SGR sets, applied to each printed character.
/// Reverse, dim and conceal change the cell's colours as it is printed.
#[derive(Clone, Copy)]
pub(crate) struct Pen {
    pub(crate) fg: (u8, u8, u8),
    pub(crate) bg: (u8, u8, u8),
    pub(crate) attrs: u8,
    pub(crate) dim: bool,
    pub(crate) reverse: bool,
    pub(crate) conceal: bool,
    /// The foreground and underline (SGR 58) colours as kitty numbers them
    /// for a Unicode placeholder (`Ids`). Neither changes what is drawn;
    /// the underline is drawn in the foreground colour.
    pub(crate) ids: Ids,
}

/// A kitty Unicode placeholder cell's image and placement ids: kitty's
/// `color_to_id` of its foreground and underline colours, 0 for the
/// default, n for palette colour n and 0xRRGGBB for a 24-bit colour. The
/// palette colour, not the RGB it stands for, is what names the image.
pub(crate) type Ids = [u32; 2];

impl Pen {
    /// SGR 0's pen: the palette's default colours, no attributes.
    pub(crate) fn reset(palette: &Palette) -> Pen {
        let (fg, bg) = (palette.foreground, palette.background);
        Pen { fg, bg, attrs: 0, dim: false, reverse: false, conceal: false, ids: [0, 0] }
    }

    /// A blank cell in this pen's colours, ready for a character.
    pub(crate) fn cell(&self) -> Cell {
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

/// A blank cell in the palette's default colours.
pub(crate) fn blank_cell(palette: &Palette) -> Cell {
    Pen::reset(palette).cell()
}

/// What a bare LF does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lf {
    /// Down a row, as on a terminal; PTY output carries its own CR.
    Index,
    /// Down a row and back to column 0, as on a terminal with `onlcr` output
    /// processing: for text files and other output not run under a PTY.
    Newline,
}

/// A character set that G0 or G1 can hold.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Charset {
    Ascii,
    /// DEC Special Graphics: ncurses draws boxes with it (ESC ( 0, then "lqk").
    DecGraphics,
}

/// What DECSC (ESC 7) and CSI s save, and DECRC (ESC 8) and CSI u restore.
/// The main and alternate screens each keep one.
#[derive(Clone, Copy)]
pub(crate) struct Saved {
    pub(crate) row: usize,
    pub(crate) col: usize,
    pub(crate) pending: bool,
    pub(crate) origin: bool,
    pub(crate) pen: Pen,
    pub(crate) charsets: [Charset; 2],
    pub(crate) shifted: bool,
}

impl Saved {
    /// What DECRC restores with nothing saved.
    pub(crate) fn home(palette: &Palette) -> Saved {
        Saved {
            row: 0,
            col: 0,
            pending: false,
            origin: false,
            pen: Pen::reset(palette),
            charsets: [Charset::Ascii; 2],
            shifted: false,
        }
    }
}

/// The cursor shapes DECSCUSR (CSI Ps SP q) picks. A still image can't
/// blink, so a blinking shape is drawn as the steady one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    /// The whole cell, in reverse video (DECSCUSR 0, 1 and 2).
    #[default]
    Block,
    /// A line along the bottom of the cell (DECSCUSR 3 and 4).
    Underline,
    /// A line down the left edge of the cell (DECSCUSR 5 and 6).
    Bar,
}

impl CursorShape {
    pub(crate) const ALL: [CursorShape; 3] = [CursorShape::Block, CursorShape::Underline, CursorShape::Bar];

    pub(crate) fn name(self) -> &'static str {
        match self {
            CursorShape::Block => "block",
            CursorShape::Underline => "underline",
            CursorShape::Bar => "bar",
        }
    }

    /// --cursor-shape's value.
    pub(crate) fn parse(value: &str) -> Result<CursorShape, String> {
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
pub(crate) struct Screen {
    pub(crate) graphics: graphics::Graphics,
    pub(crate) other_graphics: graphics::Graphics,
    pub(crate) cell_size: (i32, i32),
    /// Rows of cells in storage order; `map` gives the storage row of each
    /// screen row, so scrolling rotates `map` instead of moving cells.
    pub(crate) cells: Vec<Cell>,
    pub(crate) map: Vec<usize>,
    /// Combining marks with no precomposed form, a Marks per cell in the
    /// same storage order as cells, so scrolling moves them too; every edit
    /// that overwrites, erases or moves cells keeps it in step. Empty until
    /// the screen's first mark, as most logs have none.
    pub(crate) marks: Vec<Marks>,
    /// The Ids of each U+10EEEE cell, kept in step with cells as marks are:
    /// in storage order, empty until the screen's first placeholder with a
    /// colour, and 0 for every other cell.
    pub(crate) ids: Vec<Ids>,
    /// The other screen: the alternate one while on the main one, and back.
    pub(crate) other: Vec<Cell>,
    pub(crate) other_map: Vec<usize>,
    pub(crate) other_marks: Vec<Marks>,
    pub(crate) other_ids: Vec<Ids>,
    pub(crate) on_alternate: bool,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    pub(crate) row: usize,
    pub(crate) col: usize,
    pub(crate) pending: bool,
    pub(crate) autowrap: bool,
    /// DECOM: cursor rows count from the top margin and stay inside the margins.
    pub(crate) origin: bool,
    /// The scrolling region, inclusive rows.
    pub(crate) top: usize,
    pub(crate) bottom: usize,
    pub(crate) saved: [Saved; 2],
    pub(crate) pen: Pen,
    /// pen.cell(), kept up to date wherever pen changes (SGR, DECRC,
    /// RIS), so printing does not mix colours per character.
    pub(crate) pen_cell: Cell,
    /// Tab stops, one per column; every 8th column at start.
    pub(crate) tabs: Vec<bool>,
    /// G0 and G1. SO shifts to G1, SI back to G0.
    pub(crate) charsets: [Charset; 2],
    pub(crate) shifted: bool,
    /// The last printed character, which REP repeats with the marks its
    /// cell has (at last_at).
    pub(crate) last: Option<u32>,
    /// Where it went (a storage index), for combining marks that follow:
    /// a mark joins the last printed character wherever the cursor has gone
    /// since, as in xterm. None once that cell is erased, overwritten or
    /// moved, and then a mark is dropped.
    pub(crate) last_at: Option<usize>,
    /// DECTCEM (mode 25): whether the cursor is shown. One setting for both
    /// screens, and DECSC does not save it, as in xterm.
    pub(crate) cursor_shown: bool,
    /// DECSCUSR. One setting for both screens; DECSC does not save it and
    /// DECSTR keeps it, as in tmux. RIS resets it, as in xterm.
    pub(crate) cursor_shape: CursorShape,
    /// DECSDM (mode 80): Sixel images go to the top left corner and neither
    /// scroll nor move the cursor.
    pub(crate) sixel_display: bool,
    /// Not terminal state: RIS keeps it, as a reset keeps the tty's settings.
    pub(crate) lf: Lf,
    /// The colours SGR's default and named colours stand for. RIS keeps
    /// it, as a terminal's reset keeps its configured colours.
    pub(crate) palette: Palette,
}

impl Screen {
    #[cfg(test)]
    pub(crate) fn new(cols: usize, rows: usize, lf: Lf) -> Self {
        Self::with(cols, rows, &ParseOptions { lf, palette: Palette::DEFAULT })
    }

    pub(crate) fn with(cols: usize, rows: usize, options: &ParseOptions) -> Self {
        let (lf, palette) = (options.lf, options.palette);
        Self {
            graphics: graphics::Graphics::default(),
            other_graphics: graphics::Graphics::default(),
            cell_size: (1, 1),
            cells: vec![blank_cell(&palette); cols * rows],
            map: (0..rows).collect(),
            marks: Vec::new(),
            ids: Vec::new(),
            other: vec![blank_cell(&palette); cols * rows],
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
            saved: [Saved::home(&palette); 2],
            pen: Pen::reset(&palette),
            pen_cell: Pen::reset(&palette).cell(),
            tabs: (0..cols).map(|c| c % 8 == 0).collect(),
            charsets: [Charset::Ascii; 2],
            shifted: false,
            last: None,
            last_at: None,
            cursor_shown: true,
            cursor_shape: CursorShape::Block,
            sixel_display: false,
            lf,
            palette,
        }
    }

    pub(crate) fn save_cursor(&mut self) {
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

    pub(crate) fn restore_cursor(&mut self) {
        let s = self.saved[usize::from(self.on_alternate)];
        (self.row, self.col, self.pending, self.origin) = (s.row, s.col, s.pending, s.origin);
        self.pen = s.pen;
        self.pen_cell = s.pen.cell();
        (self.charsets, self.shifted) = (s.charsets, s.shifted);
    }

    /// C0 controls, at top level or inside a CSI.
    pub(crate) fn control(&mut self, c: u8) {
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

    pub(crate) fn tab_forward(&mut self, n: usize) {
        self.pending = false;
        for _ in 0..n.min(self.cols) {
            self.col = (self.col + 1..self.cols).find(|&c| self.tabs[c]).unwrap_or(self.cols - 1);
        }
    }

    pub(crate) fn tab_back(&mut self, n: usize) {
        self.pending = false;
        for _ in 0..n.min(self.cols) {
            self.col = (0..self.col).rev().find(|&c| self.tabs[c]).unwrap_or(0);
        }
    }

    /// The cell index range of screen row r.
    pub(crate) fn line(&self, r: usize) -> std::ops::Range<usize> {
        let stored = self.map[r];
        stored * self.cols..(stored + 1) * self.cols
    }

    /// Blank screen rows [from, to).
    pub(crate) fn erase_rows(&mut self, from: usize, to: usize) {
        for r in from..to {
            let line = self.line(r);
            self.erase(line.start, line.end);
        }
    }

    /// The cells in screen order.
    pub(crate) fn into_cells(self) -> Vec<Cell> {
        if self.map.iter().enumerate().all(|(r, &stored)| r == stored) {
            return self.cells;
        }
        (0..self.rows).flat_map(|r| self.cells[self.line(r)].iter().copied()).collect()
    }

    /// The cells that have marks, in screen order, as CellMarks.
    pub(crate) fn screen_marks(&self) -> Vec<CellMarks> {
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
    pub(crate) fn clear_marks(&mut self, from: usize, to: usize) {
        if !self.marks.is_empty() {
            self.marks[from..to].fill(NO_MARKS);
        }
        if !self.ids.is_empty() {
            self.ids[from..to].fill([0, 0]);
        }
    }

    /// The placeholder cells on the screen, in screen order.
    pub(crate) fn placeholders(&self) -> Vec<graphics::PlaceholderCell> {
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
    pub(crate) fn move_marks(&mut self, at: usize, end: usize, n: usize, right: bool) {
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
    pub(crate) fn insert_chars(&mut self, n: usize) {
        let line = self.line(self.row);
        let at = line.start + self.col;
        let n = n.min(line.end - at);
        self.move_marks(at, line.end, n, true);
        self.cells.copy_within(at..line.end - n, at + n);
        self.erase(at, at + n);
        self.mend_row(self.row);
    }

    /// DCH: shift the rest of the line left by n, blanking the end.
    pub(crate) fn delete_chars(&mut self, n: usize) {
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
    pub(crate) fn rotate_rows(&mut self, top: usize, bottom: usize, n: usize, up: bool) {
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
    pub(crate) fn scroll_up(&mut self, top: usize, bottom: usize, n: usize) {
        let n = n.min(bottom + 1 - top);
        self.rotate_rows(top, bottom, n, true);
        self.erase_rows(bottom + 1 - n, bottom + 1);
    }

    /// Move rows top..=bottom down by n, blanking the n rows that open at the top.
    pub(crate) fn scroll_down(&mut self, top: usize, bottom: usize, n: usize) {
        let n = n.min(bottom + 1 - top);
        self.rotate_rows(top, bottom, n, false);
        self.erase_rows(top, top + n);
    }

    /// LF, IND: down a row, scrolling the region at its bottom margin.
    pub(crate) fn index(&mut self) {
        self.pending = false;
        if self.row == self.bottom {
            self.scroll_up(self.top, self.bottom, 1);
        } else if self.row + 1 < self.rows {
            self.row += 1;
        }
    }

    /// RI: up a row, scrolling the region down at its top margin.
    pub(crate) fn reverse_index(&mut self) {
        self.pending = false;
        if self.row == self.top {
            self.scroll_down(self.top, self.bottom, 1);
        } else {
            self.row = self.row.saturating_sub(1);
        }
    }

    pub(crate) fn in_margins(&self) -> bool {
        (self.top..=self.bottom).contains(&self.row)
    }

    /// Rows a cursor-addressing sequence can reach: the margins in origin mode.
    pub(crate) fn addressable_rows(&self) -> (usize, usize) {
        if self.origin {
            (self.top, self.bottom)
        } else {
            (0, self.rows - 1)
        }
    }

    /// Move to a 1-based row as CUP and VPA give it, honouring origin mode.
    pub(crate) fn go_to_row(&mut self, n: usize) {
        let (first, last) = self.addressable_rows();
        self.row = (first + n - 1).min(last);
    }

    pub(crate) fn home(&mut self) {
        self.row = self.addressable_rows().0;
        self.col = 0;
    }

    pub(crate) fn set_mode(&mut self, mode: u32, on: bool) {
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
    pub(crate) fn sixel(&mut self, mut image: sixel::Image) {
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
    pub(crate) fn clear_sixel(&mut self, row: usize, col: usize, rows: usize, cols: usize) {
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
    pub(crate) fn move_past_image(&mut self, cols: usize, rows: usize) {
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
    pub(crate) fn use_alternate(&mut self, on: bool, clear_first: bool) {
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

    pub(crate) fn last_row(&self) -> usize {
        self.rows - 1
    }

    pub(crate) fn last_col(&self) -> usize {
        self.cols - 1
    }

    pub(crate) fn print(&mut self, ch: u32) {
        let charset = self.charsets[usize::from(self.shifted)];
        self.print_mapped(if charset == Charset::DecGraphics { dec_graphics(ch) } else { ch });
    }

    /// Print a character that has already been through the character set.
    /// Wide characters take two cells; zero-width ones combine with the
    /// character before them.
    pub(crate) fn print_mapped(&mut self, ch: u32) {
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
    pub(crate) fn combine(&mut self, mark: u32) {
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
    pub(crate) fn split_wide(&mut self, line: &std::ops::Range<usize>, i: usize) {
        // Most cells are neither half: test that inline, split out of line.
        if self.cells[i].attrs & (WIDE | TAIL) != 0 {
            self.split_wide_cell(line, i);
        }
    }

    #[inline(never)]
    pub(crate) fn split_wide_cell(&mut self, line: &std::ops::Range<usize>, i: usize) {
        let attrs = self.cells[i].attrs;
        if attrs & TAIL != 0 && i > line.start {
            self.unwide(i - 1);
        }
        if attrs & WIDE != 0 && i + 1 < line.end {
            self.unwide(i + 1);
        }
    }

    pub(crate) fn unwide(&mut self, i: usize) {
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
    pub(crate) fn mend_row(&mut self, r: usize) {
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
    pub(crate) fn print_ascii(&mut self, mut text: &[u8]) {
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
    pub(crate) fn erase(&mut self, from: usize, to: usize) {
        let to = to.min(self.cells.len());
        if from < to {
            let mut blank = blank_cell(&self.palette);
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
    pub(crate) fn cursor_index(&self) -> usize {
        self.line(self.row).start + self.col
    }

    pub(crate) fn csi(&mut self, final_byte: u8, p: &Params) {
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
    pub(crate) fn set_cursor_shape(&mut self, p: &Params) {
        self.cursor_shape = match p.get(0, 0) {
            0..=2 => CursorShape::Block,
            3 | 4 => CursorShape::Underline,
            5 | 6 => CursorShape::Bar,
            _ => return,
        };
    }

    /// DECSET / DECRST: CSI ? Pm h and CSI ? Pm l.
    pub(crate) fn private_csi(&mut self, final_byte: u8, p: &Params) {
        if matches!(final_byte, b'h' | b'l') {
            for k in 0..p.len {
                if let Some(mode) = p.list[k].value {
                    self.set_mode(mode, final_byte == b'h');
                }
            }
        }
    }

    pub(crate) fn sgr(&mut self, p: &Params) {
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
                        if let Some(color) = self.palette.color(n) {
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
            let (pen, palette) = (&mut self.pen, &self.palette);
            let named = |n: u32| palette.named[n as usize];
            match v {
                0 => *pen = Pen::reset(palette),
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
                30..=37 => (pen.fg, pen.ids[0]) = (named(v - 30), v - 30),
                40..=47 => pen.bg = named(v - 40),
                90..=97 => (pen.fg, pen.ids[0]) = (named(v - 90 + 8), v - 90 + 8),
                100..=107 => pen.bg = named(v - 100 + 8),
                39 => (pen.fg, pen.ids[0]) = (palette.foreground, 0),
                49 => pen.bg = palette.background,
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
                        if let Some(color) = self.palette.color(n) {
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

    pub(crate) fn reset_attributes(&mut self) {
        self.pen = Pen::reset(&self.palette);
    }

    /// SGR 38, 48 or 58 (`which`): the colour, and its kitty id (`Ids`).
    /// The underline colour only names a placeholder's placement.
    pub(crate) fn set_color(&mut self, which: u32, color: (u8, u8, u8), id: u32) {
        match which {
            38 => (self.pen.fg, self.pen.ids[0]) = (color, id),
            48 => self.pen.bg = color,
            _ => self.pen.ids[1] = id,
        }
    }
}

/// Set every cell to cell. A 12-byte Cell defeats the vectorized fill, so
/// a row is filled by copying what is already filled, doubling each time.
pub(crate) fn fill_cells(cells: &mut [Cell], cell: Cell) {
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
pub(crate) fn rgb_id((r, g, b): (u8, u8, u8)) -> u32 {
    u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
}

/// DEC Special Graphics: 0x5f..=0x7e become line drawing and symbols.
pub(crate) fn dec_graphics(ch: u32) -> u32 {
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
