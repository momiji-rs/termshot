//! termshot as a library: replay a raw PTY log (bytes with ANSI escapes)
//! into the grid of cells a terminal would show at its end, and read that
//! grid cell by cell, as text, or as JSON. The text and the JSON are byte for
//! byte what the CLI's `--text` and `--json` write.
//!
//! ```no_run
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let log = std::fs::read("session.pty")?;
//!     let grid = termshot::parse(&log, 100, 30, &termshot::ParseOptions::default())?;
//!     print!("{}", grid.to_text());
//!     Ok(())
//! }
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
mod api;

pub use api::{
    decode_cast, is_cast, lacks_cr, needs_cell_size, parse, parse_with_cell_size, Cast, CursorShape, Error, Grid, GridCell,
    Lf, Palette, ParseOptions, Rgb, MAX_CELLS, MAX_SIDE,
};
