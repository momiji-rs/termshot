//! A cell of the final screen, as draw.c and src/composite.rs read it, and
//! the bits of its attributes. Its own module so that the C harnesses'
//! static library (tests/rust_lib.rs) can share it with the image compositor.

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
/// The first cell of a double-width character; draw.c spans its glyph over two.
pub const WIDE: u8 = 16;
/// The second cell of a double-width character: ch is 0 and nothing is drawn.
pub const TAIL: u8 = 32;
/// draw.c slants the glyph; box drawing and blocks stay upright.
pub const ITALIC: u8 = 64;
/// The background hides an image placed below the cell backgrounds
/// (z < -2^30). Reverse video and the block cursor set it as kitty treats
/// those cells, even in the default colour; before drawing,
/// `opaque_backgrounds` sets it on every other colour, so draw.c needs no
/// copy of DEFAULT_BG.
pub const OPAQUE: u8 = 128;
