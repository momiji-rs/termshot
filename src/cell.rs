//! A cell of the final screen, as draw.c, src/composite.rs and src/glyphs.rs
//! read it, the bits of its attributes, and the combining marks a cell keeps
//! beside it. Its own module so that the C harnesses' static library
//! (tests/rust_lib.rs) can share it with the painters.

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Cell {
    pub ch: u32,
    pub fr: u8,
    pub fg: u8,
    pub fb: u8,
    pub br: u8,
    pub bg: u8,
    pub bb: u8,
    /// BOLD, UNDERLINE, DOUBLE_UNDERLINE, STRIKE, ITALIC, WIDE, TAIL and
    /// OPAQUE bits, as ATTR_* in draw.c.
    pub attrs: u8,
}

const _: () = assert!(std::mem::size_of::<Cell>() == 12);

pub const BOLD: u8 = 1;
pub const UNDERLINE: u8 = 2;
pub const DOUBLE_UNDERLINE: u8 = 4;
pub const STRIKE: u8 = 8;
/// The first cell of a double-width character; its glyph spans both.
pub const WIDE: u8 = 16;
/// The second cell of a double-width character: ch is 0 and nothing is drawn.
pub const TAIL: u8 = 32;
/// The glyph is slanted; box drawing and blocks stay upright.
pub const ITALIC: u8 = 64;
/// The background hides an image placed below the cell backgrounds
/// (z < -2^30). Reverse video and the block cursor set it as kitty treats
/// those cells, even in the default colour; before drawing,
/// `opaque_backgrounds` sets it on every other colour, so painting needs no
/// copy of DEFAULT_BG.
pub const OPAQUE: u8 = 128;

/// The most combining marks a cell keeps after its character (#14). A cell
/// holds one code point, so a mark with no precomposed form goes in a side
/// table instead. Four is enough for Thai (a vowel and a tone mark), Hebrew
/// points, stacked Latin accents and the three diacritics of a kitty Unicode
/// placeholder; marks after the fourth are dropped.
pub const MAX_MARKS: usize = 4;

/// A cell's marks in the order they arrived; the unused slots are 0, which
/// no mark is.
pub type Marks = [u32; MAX_MARKS];

/// One cell's combining marks, for --text, --json and src/glyphs.rs: the
/// cell's index in screen order (row * cols + col) and its marks. A list of
/// them is sorted by cell, one per cell. As CellMarks in src/draw.c, which
/// passes the list on.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellMarks {
    pub cell: u32,
    pub marks: Marks,
}

const _: () = assert!(std::mem::size_of::<CellMarks>() == 4 + 4 * MAX_MARKS);

impl CellMarks {
    /// The marks, without the unused slots.
    pub fn code_points(&self) -> &[u32] {
        &self.marks[..self.marks.iter().position(|&m| m == 0).unwrap_or(MAX_MARKS)]
    }
}
