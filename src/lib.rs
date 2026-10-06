//! termshot as a library: replay a raw PTY log (bytes with ANSI escapes)
//! into the grid of cells a terminal would show at its end, and read that
//! grid cell by cell, as text, or as JSON. The text and the JSON are byte for
//! byte what the CLI's `--text` and `--json` write.
//!
//! ```no_run
//! let log = std::fs::read("session.pty").unwrap();
//! let grid = termshot::parse(&log, 100, 30, &termshot::ParseOptions::default());
//! print!("{}", grid.to_text());
//! ```
//!
//! Drawing the grid as a PNG is the CLI's only, for now (#85).
//!
//! The crate has no dependencies and builds with plain rustc (1.70 or
//! later), after `./build.sh`, which makes `libtermshot.rlib` with the C it
//! links (the PNG decoder kitty images use) inside it:
//!
//! ```text
//! rustc --edition 2021 app.rs --extern termshot=path/to/libtermshot.rlib
//! ```
//!
//! The binary (src/main.rs) does not link this library: it compiles the same
//! modules itself, with the render, which is not part of the library yet.

// The modules the parser needs. A few of their items are only the binary's
// (the render's, and --cursor-shape's parsing): allowed dead here.
mod cast;
mod cell;
// The image layers: the parser keeps kitty images as composite::ImageView
// and composes animation frames with its blending.
#[allow(dead_code)]
mod composite;
mod geometry;
#[allow(dead_code)]
mod graphics;
mod grid;
mod palette;
#[rustfmt::skip]
mod rowcolumn_diacritics;
#[allow(dead_code)]
mod screen;
mod sixel;
mod unicode;
#[rustfmt::skip]
mod unicode_tables;
mod vt;

use cell::{Cell, BOLD, DOUBLE_UNDERLINE, ITALIC, STRIKE, TAIL, UNDERLINE, WIDE};

pub use cast::Cast;
pub use grid::Grid;
pub use palette::{Palette, Rgb};
pub use screen::{CursorShape, Lf};
pub use vt::ParseOptions;

/// Replay `log` on a terminal of `cols` x `rows` cells and return the screen
/// it leaves, as the CLI does without a font: cells 1 pixel square, which
/// only matters to an image placed by pixels (see [`needs_cell_size`]).
///
/// # Panics
///
/// If `cols` or `rows` is 0. The CLI allows up to 500 x 200.
pub fn parse(log: &[u8], cols: usize, rows: usize, options: &ParseOptions) -> Grid {
    vt::replay_with(log, cols, rows, options, (1, 1))
}

/// [`parse`] with cells of `cell_size` (width, height) pixels, the font's,
/// as the CLI does when it reads one. A kitty image sized in pixels, or a
/// Sixel image, moves the text cursor by the cells it covers, so the cursor
/// and the cells written after it depend on this. A size of 0 counts as 1.
///
/// # Panics
///
/// If `cols` or `rows` is 0.
pub fn parse_with_cell_size(
    log: &[u8],
    cols: usize,
    rows: usize,
    options: &ParseOptions,
    cell_size: (u16, u16),
) -> Grid {
    let (w, h) = cell_size;
    vt::replay_with(log, cols, rows, options, (i32::from(w.max(1)), i32::from(h.max(1))))
}

/// Whether the screen `log` leaves depends on the cell size: it has a kitty
/// command or a Sixel image that can move the text cursor by cells of the
/// font's size. The CLI reads a font for `--text` and `--json` only then.
pub fn needs_cell_size(log: &[u8]) -> bool {
    vt::needs_cell_metrics(log)
}

/// Whether `log` looks like it never went through a PTY: it has line feeds
/// but no CR at all, which a terminal's `onlcr` would have added. Such a log
/// (a text file, `tmux capture-pane`) wants `Lf::Newline`; the CLI hints at
/// `--lf-newline` then.
pub fn lacks_cr(log: &[u8]) -> bool {
    vt::lacks_cr(log)
}

/// Whether `log` reads as an asciinema recording (a v2 or v3 `.cast`): its
/// first line is a JSON object with a `"version"` member. This is how the
/// CLI decides without `--cast` or `--raw`.
pub fn is_cast(log: &[u8]) -> bool {
    cast::detect(log)
}

/// Decode an asciinema v2 or v3 recording: its output events, in order,
/// which [`parse`] replays, and the terminal size it ends at. The error says
/// which line is malformed, and how, as the CLI reports it.
pub fn decode_cast(log: Vec<u8>) -> Result<Cast, String> {
    cast::decode(log)
}

impl Grid {
    /// The number of columns.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// The number of rows.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// The cursor as (row, col), from 0, or None when the log hid it
    /// (DECTCEM). With a wrap pending it is on the last column.
    pub fn cursor(&self) -> Option<(usize, usize)> {
        self.cursor
    }

    /// The cursor's shape, as the log last set it (DECSCUSR).
    pub fn cursor_shape(&self) -> CursorShape {
        self.cursor_shape
    }

    /// The cell at (row, col), from 0, or None outside the grid.
    pub fn cell(&self, row: usize, col: usize) -> Option<GridCell<'_>> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        let at = row * self.cols + col;
        Some(GridCell { cell: &self.cells[at], marks: grid::marks_of(&self.marks, at) })
    }
}

/// One cell of a [`Grid`]: its character and combining marks, the colours it
/// was printed in (after reverse video, dim and conceal), and its attributes.
#[derive(Clone, Copy)]
pub struct GridCell<'a> {
    cell: &'a Cell,
    marks: &'a [u32],
}

impl<'a> GridCell<'a> {
    /// The character: a space in a blank cell, and in the right half of a
    /// wide character ([`GridCell::is_wide_tail`]). A kitty Unicode
    /// placeholder cell holds U+10EEEE, as `--text` shows it.
    pub fn ch(&self) -> char {
        grid::to_char(self.cell.ch)
    }

    /// The combining marks that follow the character and have no
    /// precomposed form with it, in order.
    pub fn marks(&self) -> impl Iterator<Item = char> + 'a {
        self.marks.iter().map(|&m| grid::to_char(m))
    }

    /// The text colour.
    pub fn fg(&self) -> Rgb {
        (self.cell.fr, self.cell.fg, self.cell.fb)
    }

    /// The background colour.
    pub fn bg(&self) -> Rgb {
        (self.cell.br, self.cell.bg, self.cell.bb)
    }

    /// SGR 1.
    pub fn is_bold(&self) -> bool {
        self.cell.attrs & BOLD != 0
    }

    /// SGR 3.
    pub fn is_italic(&self) -> bool {
        self.cell.attrs & ITALIC != 0
    }

    /// A single underline: SGR 4, or a 4:n style other than 4:0 and 4:2.
    pub fn is_underlined(&self) -> bool {
        self.cell.attrs & UNDERLINE != 0
    }

    /// SGR 21 or 4:2.
    pub fn is_double_underlined(&self) -> bool {
        self.cell.attrs & DOUBLE_UNDERLINE != 0
    }

    /// SGR 9.
    pub fn is_struck(&self) -> bool {
        self.cell.attrs & STRIKE != 0
    }

    /// The left half of a wide (two-column) character.
    pub fn is_wide(&self) -> bool {
        self.cell.attrs & WIDE != 0
    }

    /// The right half of a wide character, which the left half draws.
    pub fn is_wide_tail(&self) -> bool {
        self.cell.attrs & TAIL != 0
    }
}

impl std::fmt::Debug for GridCell<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GridCell")
            .field("ch", &self.ch())
            .field("marks", &self.marks().collect::<String>())
            .field("fg", &self.fg())
            .field("bg", &self.bg())
            .field("attrs", &self.cell.attrs)
            .finish()
    }
}
