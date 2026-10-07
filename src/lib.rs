//! termshot as a library: replay a raw PTY log (bytes with ANSI escapes)
//! into the grid of cells a terminal would show at its end, read that grid
//! cell by cell, as text or as JSON, and draw it as a PNG. The text, the
//! JSON and the PNG are byte for byte what the CLI's `--text`, `--json`
//! and `<out.png>` write.
//!
//! ```no_run
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let log = std::fs::read("session.pty")?;
//!     let grid = termshot::parse(&log, 100, 30, &termshot::ParseOptions::default())?;
//!     print!("{}", grid.to_text());
//!     let png = termshot::render(&grid, &termshot::RenderOptions::default())?;
//!     std::fs::write("session.png", &png.png)?;
//!     Ok(())
//! }
//! ```
//!
//! A log whose kitty or Sixel images move the cursor by pixels
//! ([`needs_cell_size`]) parses as the CLI parses it with the font's cell:
//!
//! ```no_run
//! # fn main() -> Result<(), termshot::Error> {
//! # let log = Vec::new();
//! let font = termshot::Font::embedded()?;
//! let options = termshot::RenderOptions { px: 24.0, font: Some(&font), ..Default::default() };
//! let cell = font.cell_size(options.px)?;
//! let grid = termshot::parse_with_cell_size(&log, 80, 24, &termshot::ParseOptions::default(), cell)?;
//! let png = termshot::render(&grid, &options)?;
//! # Ok(())
//! # }
//! ```
//!
//! Errors are values ([`Error`]): the library prints nothing, never exits,
//! and returns [`Error::OutOfMemory`] instead of aborting where an
//! allocation grows with the input: the parser's screens, rows, tab stops,
//! marks, placeholder ids and placeholder cells (up to [`MAX_CELLS`]
//! cells), the render's copies of them, its canvas, glyphs and PNG encoder
//! and the bytes it returns, and a font file's bytes and padding. What still aborts if memory runs out,
//! as any `Vec` does, is bounded otherwise: a kitty or Sixel image's
//! buffers and the placements' layout (by kitty's 16 MiB quota, its
//! limits on images and placements, and the Sixel budget), a font check's
//! tables (by the font's size and its 16-bit counts), an asciicast's
//! output (by the recording's size), and the strings of
//! [`Grid::to_text`] and [`Grid::to_json`] (by the grid's). A panic in a
//! render is caught and returned as [`Error::Internal`].
//!
//! The crate has no dependencies and builds with plain rustc (1.70 or
//! later), after `./build.sh`, which makes `libtermshot.rlib` with the C it
//! links (stb_truetype, the PNG writer, the PNG decoder kitty images use)
//! inside it:
//!
//! ```text
//! rustc --edition 2021 app.rs --extern termshot=path/to/libtermshot.rlib
//! ```
//!
//! The binary (src/main.rs) does not link this library: it compiles the same
//! modules itself, as one crate, which is as fast as main was
//! (docs/performance.md, "Two crates or one compilation unit").

mod api;
mod api_render;
mod cast;
mod cell;
mod cff;
mod composite;
mod deflate;
mod font;
mod geometry;
mod glyphs;
mod graphics;
mod grid;
mod metrics;
mod palette;
mod prepare;
mod render;
#[rustfmt::skip]
mod rowcolumn_diacritics;
mod screen;
mod sixel;
mod unicode;
#[rustfmt::skip]
mod unicode_tables;
mod variations;
mod vt;

pub use api::{
    decode_cast, is_cast, lacks_cr, needs_cell_size, parse, parse_color, parse_with_cell_size, Cast, CursorShape,
    Error, Grid, GridCell, Lf, Palette, ParseOptions, Rgb, MAX_CELLS, MAX_PALETTE_BYTES, MAX_PIXELS, MAX_SIDE,
};
pub use api_render::{
    render, render_rgba, Cursor, EmptyGlyph, FaceSelector, Font, FontSpec, Rendered, RenderOptions, RgbaImage,
    MAX_PADDING,
};
#[doc(hidden)]
pub use api_render::cli;
