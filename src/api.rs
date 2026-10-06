//! The library's public API, which src/lib.rs re-exports: parse, the grid
//! it leaves and its cells, and the errors.

use crate::cell::{Cell, BOLD, DOUBLE_UNDERLINE, ITALIC, STRIKE, TAIL, UNDERLINE, WIDE};
use crate::{cast, grid, vt};

pub use crate::cast::Cast;
pub use crate::grid::Grid;
pub use crate::palette::{parse_color, Palette, Rgb};
pub use crate::screen::{CursorShape, Lf};
pub use crate::vt::ParseOptions;

/// The most cells a grid may have, `cols * rows`: 4,194,304, 2048 x 2048. The
/// CLI allows up to 500 x 200.
pub const MAX_CELLS: usize = 1 << 22;

/// The most columns, and the most rows, a grid may have: 65,535. The parser
/// caps a control's count there, as the CLI always has, so on a grid no wider
/// or taller a cursor move, an erase, an insert or a delete still reaches the
/// edge; REP repeats a character at most that many times.
pub const MAX_SIDE: usize = u16::MAX as usize;

/// The largest palette file [`Palette::with_file`] reads: 64 KiB.
pub const MAX_PALETTE_BYTES: usize = crate::palette::MAX_FILE_BYTES;

/// The most pixels an image may have, its padding included: 134,217,728
/// (2^27), as the CLI allows. A larger one is an [`Error::ImageTooLarge`].
pub const MAX_PIXELS: u64 = 1 << 27;

/// Why the library could not do what it was asked. Each message is the one
/// the CLI prints after `termshot: `; the CLI's exit status for each is in
/// the variant's docs.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A grid size with no cells, a side over [`MAX_SIDE`], or more than
    /// [`MAX_CELLS`] cells. The CLI's own limits are smaller (exit 2).
    GridSize {
        /// The columns asked for.
        cols: usize,
        /// The rows asked for.
        rows: usize,
    },
    /// A recording that is not a readable asciicast: which line is
    /// malformed, and how, as the CLI reports it (exit 1).
    Cast(String),
    /// A font that can't be read or used, or can't draw at the size asked:
    /// why, with the file's name, as the CLI reports it (exit 1).
    Font(String),
    /// An option out of its range: a pixel size, a padding or a cursor
    /// position (exit 2, as a bad argument is).
    Options(String),
    /// The image, its padding included, would be more than [`MAX_PIXELS`]
    /// pixels (exit 2).
    ImageTooLarge {
        /// Its width in pixels.
        width: u64,
        /// Its height in pixels.
        height: u64,
        /// Whether it has padding, which the message suggests lowering.
        padded: bool,
    },
    /// Memory ran out: for what (exit 2). The library returns this rather
    /// than aborting where an allocation's size comes from the input; the
    /// crate docs say what is left.
    OutOfMemory(String),
    /// A bug: a painter failed or panicked (exit 2).
    Internal(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::GridSize { cols, rows } => write!(
                f,
                "a {cols}x{rows} grid: it needs 1 to {MAX_SIDE} columns and rows, and at most {MAX_CELLS} cells"
            ),
            Error::ImageTooLarge { width, height, padded } => {
                let padding = if *padded { ", rows or padding" } else { " or rows" };
                write!(f, "image {width}x{height} is over {MAX_PIXELS} pixels; lower px, cols{padding}")
            }
            Error::Cast(reason)
            | Error::Font(reason)
            | Error::Options(reason)
            | Error::OutOfMemory(reason)
            | Error::Internal(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for Error {}

/// Replay `log` on a terminal of `cols` x `rows` cells and return the screen
/// it leaves, as the CLI does without a font: cells 1 pixel square, which
/// only matters to an image placed by pixels (see [`needs_cell_size`]).
///
/// A grid of 0 cells, wider or taller than [`MAX_SIDE`], or of more than
/// [`MAX_CELLS`] cells is an [`Error::GridSize`].
/// The two screens' cells are allocated up front, 24 bytes a cell; like any
/// `Vec`, a failed allocation aborts (errors for that come with the render,
/// #85).
pub fn parse(log: &[u8], cols: usize, rows: usize, options: &ParseOptions) -> Result<Grid, Error> {
    parse_with_cell_size(log, cols, rows, options, (1, 1))
}

/// [`parse`] with cells of `cell_size` (width, height) pixels, the font's,
/// as the CLI does when it reads one. A kitty image sized in pixels, or a
/// Sixel image, moves the text cursor by the cells it covers, so the cursor
/// and the cells written after it depend on this. A size of 0 counts as 1.
/// [`Font::cell_size`](crate::Font::cell_size) gives a font's.
pub fn parse_with_cell_size(
    log: &[u8],
    cols: usize,
    rows: usize,
    options: &ParseOptions,
    cell_size: (u32, u32),
) -> Result<Grid, Error> {
    let sides = (1..=MAX_SIDE).contains(&cols) && (1..=MAX_SIDE).contains(&rows);
    if !sides || cols.checked_mul(rows).map_or(true, |cells| cells > MAX_CELLS) {
        return Err(Error::GridSize { cols, rows });
    }
    let side = |n: u32| i32::try_from(n.max(1)).unwrap_or(i32::MAX);
    Ok(vt::replay_with(log, cols, rows, options, (side(cell_size.0), side(cell_size.1))))
}

/// Whether the screen `log` leaves may depend on the cell size: it has a
/// kitty command or a Sixel image that may move the text cursor by cells of
/// the font's size. False means the cell size cannot matter. The scan does
/// not decode images, so true does not promise a difference: a transmission
/// that fails, or a malformed image, still counts. The CLI reads a font for
/// `--text` and `--json` only when this is true.
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

/// Whether `log` should be read as an asciinema recording: its first line is
/// a JSON object with a `"version"` member. This is how the CLI decides
/// without `--cast` or `--raw`. It is not a check that the recording is
/// valid: an unsupported version or a malformed header still reads as a
/// cast, and [`decode_cast`] says what is wrong with it.
pub fn is_cast(log: &[u8]) -> bool {
    cast::detect(log)
}

/// Decode an asciinema v2 or v3 recording: its output events, in order,
/// which [`parse`] replays, and the terminal size it ends at. A malformed
/// recording is an [`Error::Cast`].
pub fn decode_cast(log: Vec<u8>) -> Result<Cast, Error> {
    cast::decode(log).map_err(Error::Cast)
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

    /// The cursor's shape, as the log last set it (DECSCUSR), or
    /// [`Grid::set_cursor_shape`] since.
    pub fn cursor_shape(&self) -> CursorShape {
        self.cursor_shape
    }

    /// Put the cursor at (row, col), from 0, or hide it with None, as the
    /// CLI's `--cursor` does: [`Grid::cursor`], [`Grid::to_json`] and the
    /// render then show it there. A position off the grid is an
    /// [`Error::Options`], and changes nothing.
    pub fn set_cursor(&mut self, cursor: Option<(usize, usize)>) -> Result<(), Error> {
        if let Some((row, col)) = cursor {
            check_cursor(row, col, self.cols, self.rows)?;
        }
        self.cursor = cursor;
        Ok(())
    }

    /// Set the cursor's shape, as the CLI's `--cursor-shape` does.
    pub fn set_cursor_shape(&mut self, shape: CursorShape) {
        self.cursor_shape = shape;
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

/// An [`Error::Options`] unless (row, col) is on a `cols` x `rows` grid.
pub(crate) fn check_cursor(row: usize, col: usize, cols: usize, rows: usize) -> Result<(), Error> {
    if row < rows && col < cols {
        return Ok(());
    }
    Err(Error::Options(format!("the cursor at row {row}, column {col} (from 0) is off the {cols}x{rows} grid")))
}

impl std::str::FromStr for CursorShape {
    type Err = Error;

    /// `block`, `underline` or `bar`, as `--cursor-shape` takes them; any
    /// other is an [`Error::Options`].
    fn from_str(value: &str) -> Result<CursorShape, Error> {
        CursorShape::parse(value).map_err(Error::Options)
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
        // The screen keeps a tail's character as 0.
        if self.is_wide_tail() {
            return ' ';
        }
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
