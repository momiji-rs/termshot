//! The library's render API, which src/lib.rs re-exports: fonts, the
//! options of a render, and render itself, which draws a Grid as the CLI
//! draws it. What the CLI alone needs (its profile and -v lines, the load
//! timings) is in `cli`, hidden from the docs.

use std::ffi::c_int;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

use crate::api::{check_cursor, Error};
use crate::cell::CellMarks;
use crate::composite::ImageView;
use crate::font;
use crate::glyphs::{EmptyGlyphs, EMPTY_IN_FALLBACK, EMPTY_IN_FONT};
use crate::grid::Grid;
use crate::palette::Palette;
use crate::prepare;
use crate::render::{self, Encoding, Failure, Site};
use crate::screen::CursorShape;

/// The built-in font, JetBrains Mono Regular, so a lone binary works.
static EMBEDDED_FONT: &[u8] = include_bytes!("../third_party/jetbrains-mono/JetBrainsMono-Regular.ttf");

/// The most padding on a side, in pixels: 1,024, as `--padding` allows.
pub const MAX_PADDING: u32 = render::MAX_PADDING;

extern "C" {
    /// stb_glue.c: the cell of `font` at `px`, as the render sizes it; 0
    /// when its metrics are unusable.
    fn draw_face_cell_size(font: *const font::Face, px: f64, w: *mut c_int, h: *mut c_int) -> c_int;
}

/// A font, checked and ready to draw with: TrueType, or OpenType with CFF
/// or CFF2 outlines, a single font or a face of a collection, and for a
/// variable CFF2 face an instance of it. Every structure the rasterizer
/// reads is checked when the font is made, so a font that is damaged or
/// hostile is an [`Error::Font`] then, not a crash later.
///
/// A font holds its file and 1 MiB of zero padding after it. It is `Send`
/// and `Sync`: one font can draw on many threads at once.
pub struct Font {
    font: font::Font,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Font")
            .field("face", &self.face())
            .field("instance", &self.instance())
            .field("bytes", &self.font.data.len())
            .finish()
    }
}

/// Which face of a font file to draw with, and at which instance: what
/// follows a `#` in the CLI's `--font FILE#...`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FaceSelector {
    /// A face of a collection: its index from 0 (`"1"`), or its full or
    /// family name, in any case (`"Noto Sans CJK TC"`). None is the first,
    /// and the only face of a single font.
    pub face: Option<String>,
    /// The instance of a variable CFF2 face, as axis settings
    /// (`"wght=700,wdth=90"`). None is the default instance.
    pub axes: Option<String>,
}

impl FaceSelector {
    /// The selector the CLI reads after a font's `#`: a face (`1`, `Name`),
    /// axis settings (`wght=700`), or both (`1#wght=700`); a last `#` part
    /// with a `=` in it is the axis settings. Malformed settings are an
    /// [`Error::Font`].
    pub fn parse(value: &str) -> Result<FaceSelector, Error> {
        let (face, axes) = font::selector(value).map_err(|reason| Error::Font(format!("#{value}: {reason}")))?;
        Ok(FaceSelector { face: face.map(Into::into), axes: axes.map(Into::into) })
    }
}

/// A font file as the CLI's `--font` names it: a path, then optionally `#`
/// and a [`FaceSelector`], as in `fonts.ttc#1`, `NotoSansCJK.ttc#Noto Sans
/// CJK TC` or `VF.otf#wght=700`.
#[derive(Debug, PartialEq, Eq)]
pub struct FontSpec {
    spec: font::Spec,
}

impl FontSpec {
    /// Read a `--font` value. A file named with a `#` in it is that file;
    /// otherwise the path is the part before a `#` that names a file, so
    /// this looks at the filesystem, once. A value that could name a face
    /// of two files, or malformed axis settings, is an [`Error::Font`].
    pub fn parse(value: &str) -> Result<FontSpec, Error> {
        font::Spec::parse(value).map(|spec| FontSpec { spec }).map_err(Error::Font)
    }

    /// The file's path.
    pub fn path(&self) -> &str {
        &self.spec.path
    }

    /// The face and the instance it picks.
    pub fn selector(&self) -> FaceSelector {
        FaceSelector { face: self.spec.face.clone(), axes: self.spec.axes.clone() }
    }

    /// The value as given: the path, then the face and the axis settings.
    pub fn name(&self) -> String {
        self.spec.name()
    }
}

/// An [`Error`] for a font that could not be loaded.
fn load_error(error: font::LoadError) -> Error {
    match error {
        font::LoadError::Unusable(reason) => Error::Font(reason),
        font::LoadError::OutOfMemory(reason) => Error::OutOfMemory(reason),
    }
}

impl Font {
    /// The built-in font, JetBrains Mono Regular, which a render uses when
    /// [`RenderOptions::font`] is None. Each call copies it (274 KB) and
    /// checks it; keep the font to draw with it again.
    pub fn embedded() -> Result<Font, Error> {
        Font::embedded_timed(&mut cli::LoadTimings::default())
    }

    /// The CLI's built-in font, with the clocks of its load.
    #[doc(hidden)]
    pub fn embedded_timed(timings: &mut cli::LoadTimings) -> Result<Font, Error> {
        let font = font::prepare_timed(EMBEDDED_FONT, "built-in font", timings).map_err(load_error)?;
        Ok(Font { font })
    }

    /// A font from the bytes of a font file, and the face of it `face`
    /// selects. An unusable font, or a face it does not have, is an
    /// [`Error::Font`] with the reason the CLI gives, naming the file
    /// "font".
    pub fn from_bytes(bytes: Vec<u8>, face: &FaceSelector) -> Result<Font, Error> {
        let font = font::from_bytes(bytes, face.face.as_deref(), face.axes.as_deref(), "font").map_err(load_error)?;
        Ok(Font { font })
    }

    /// Read the font file `spec` names and check the face it selects, as
    /// the CLI's `--font` does, with its messages.
    pub fn open(spec: &FontSpec) -> Result<Font, Error> {
        Font::open_timed(spec, &mut cli::LoadTimings::default())
    }

    /// [`Font::open`], with the clocks of the load.
    #[doc(hidden)]
    pub fn open_timed(spec: &FontSpec, timings: &mut cli::LoadTimings) -> Result<Font, Error> {
        font::load_timed(&spec.spec, timings).map(|font| Font { font }).map_err(load_error)
    }

    /// The cell this font draws at `px` pixels, (width, height) in pixels,
    /// as the render sizes it: [`parse_with_cell_size`](crate::parse_with_cell_size)
    /// takes it, for logs whose images move the cursor by pixels. `px` must
    /// be above 0 and below 256 ([`Error::Options`]); a font whose metrics
    /// can't make a cell is an [`Error::Font`].
    pub fn cell_size(&self, px: f64) -> Result<(u32, u32), Error> {
        check_px(px)?;
        let (w, h) = self.cell(px)?;
        Ok((w as u32, h as u32))
    }

    /// The cell at a checked `px`, as the render's metrics: each side at
    /// least 1.
    fn cell(&self, px: f64) -> Result<(i32, i32), Error> {
        let (mut w, mut h) = (1, 1);
        // SAFETY: the face is a checked font's, padded; the C reads no glyph.
        let sized = self.font.with_metrics(|face| unsafe { draw_face_cell_size(face, px, &mut w, &mut h) });
        match sized.map_err(Error::Font)? {
            1 => Ok((w.max(1), h.max(1))),
            _ => Err(failure(Failure::FontMetrics)),
        }
    }

    /// For a face of a collection: its index and its name.
    pub fn face(&self) -> Option<(usize, &str)> {
        self.font.face.as_ref().map(|(index, name)| (*index, name.as_str()))
    }

    /// For an instance of a variable face: its axis settings, as the CLI's
    /// `-v` prints them.
    pub fn instance(&self) -> Option<&str> {
        self.font.instance.as_deref()
    }

    /// Set when a collection of several faces was opened without picking
    /// one: which face is used, and how to pick another. The CLI prints it
    /// as a hint.
    pub fn hint(&self) -> Option<&str> {
        self.font.hint.as_deref()
    }

    /// The color bitmap table of the face (`CBDT`, `CBLC` or `sbix`), if it
    /// has one: such a face's colour glyphs are bitmaps termshot can't
    /// draw, and come out as empty boxes ([`EmptyGlyph`]).
    pub fn color_bitmap(&self) -> Option<&'static str> {
        font::color_bitmap(&self.font)
    }
}

/// Where a render draws the cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    /// Where the grid has it ([`Grid::cursor`]), or nowhere if the log hid
    /// it.
    #[default]
    FromGrid,
    /// Nowhere, as `--cursor none`.
    Hidden,
    /// On the cell at (row, col), from 0, as `--cursor COL,ROW`.
    At {
        /// The row, from 0.
        row: usize,
        /// The column, from 0.
        col: usize,
    },
}

/// How to draw a grid. [`RenderOptions::default`] draws as the CLI does with
/// no options: the built-in font at 48 px, no fallback, no padding, and the
/// cursor where the grid has it. Name the fields you change and fill in the
/// rest with `..RenderOptions::default()`; fields may be added.
#[derive(Clone, Copy, Debug)]
pub struct RenderOptions<'a> {
    /// The font's pixel height, above 0 and below 256 (`--px`).
    pub px: f64,
    /// The font to draw with (`--font`); None is [`Font::embedded`].
    pub font: Option<&'a Font>,
    /// A font for the characters the first lacks (`--fallback-font`), such
    /// as CJK; characters neither has are drawn as an outlined box.
    pub fallback: Option<&'a Font>,
    /// A margin around the cells, left and right then above and below, in
    /// pixels up to [`MAX_PADDING`], in the default background of the
    /// palette the grid was parsed with (`--padding`).
    pub padding: (u32, u32),
    /// Where to draw the cursor.
    pub cursor: Cursor,
    /// The cursor's shape; None is the grid's ([`Grid::cursor_shape`]).
    pub cursor_shape: Option<CursorShape>,
    /// TERMSHOT_PROFILE: put the render's stage timings in
    /// [`Rendered::profile`].
    #[doc(hidden)]
    pub profile: bool,
}

impl Default for RenderOptions<'_> {
    fn default() -> Self {
        RenderOptions {
            px: 48.0,
            font: None,
            fallback: None,
            padding: (0, 0),
            cursor: Cursor::FromGrid,
            cursor_shape: None,
            profile: false,
        }
    }
}

/// A character drawn as an outlined box because a font maps it to an empty
/// glyph, as a color bitmap font does for its colour glyphs: the first such
/// cell, and how many there are. The CLI warns about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EmptyGlyph {
    /// The character.
    pub ch: char,
    /// The first cell it is in: its row, from 0.
    pub row: usize,
    /// And its column, from 0.
    pub col: usize,
    /// How many cells were drawn as a box this way.
    pub cells: usize,
    /// Whether [`RenderOptions::font`] maps it to an empty glyph.
    pub in_font: bool,
    /// Whether the fallback font does too.
    pub in_fallback: bool,
}

impl EmptyGlyph {
    /// What the render reports in `empty`, if anything.
    pub(crate) fn of(empty: &EmptyGlyphs) -> Option<EmptyGlyph> {
        (empty.cells != 0).then(|| EmptyGlyph {
            ch: char::from_u32(empty.cp).unwrap_or(char::REPLACEMENT_CHARACTER),
            row: empty.row.max(0) as usize,
            col: empty.col.max(0) as usize,
            cells: empty.cells,
            in_font: empty.fonts & EMPTY_IN_FONT != 0,
            in_fallback: empty.fonts & EMPTY_IN_FALLBACK != 0,
        })
    }
}

/// A rendered grid: the PNG, byte for byte what the CLI writes, and what the
/// render found.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Rendered {
    /// The PNG file's bytes: 8-bit RGB, no alpha.
    pub png: Vec<u8>,
    /// The image's width in pixels, padding included.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
    /// A character drawn as a box for an empty glyph, if any.
    pub empty_glyph: Option<EmptyGlyph>,
    /// With [`RenderOptions::profile`], the fields of the render's
    /// TERMSHOT_PROFILE record but output_write_ms.
    #[doc(hidden)]
    pub profile: Option<String>,
}

/// A rendered grid as pixels: what [`render`] encodes as its PNG.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RgbaImage {
    /// `width * height` pixels, 4 bytes each (red, green, blue, alpha), row
    /// by row from the top. Alpha is always 255.
    pub rgba: Vec<u8>,
    /// The image's width in pixels, padding included.
    pub width: u32,
    /// Its height in pixels.
    pub height: u32,
    /// A character drawn as a box for an empty glyph, if any.
    pub empty_glyph: Option<EmptyGlyph>,
}

/// Draw `grid` as a PNG, as the CLI draws it with the same options: the
/// same bytes. The render prints nothing and writes no file.
///
/// The options are checked first ([`Error::Options`]). A font that can't
/// draw is an [`Error::Font`], an image over [`MAX_PIXELS`](crate::MAX_PIXELS)
/// an [`Error::ImageTooLarge`], memory that runs out an
/// [`Error::OutOfMemory`], and a bug an [`Error::Internal`]: a render never
/// panics or aborts on its input. Renders on many threads at once are
/// independent.
pub fn render(grid: &Grid, options: &RenderOptions) -> Result<Rendered, Error> {
    let (drawn, empty_glyph) = draw(grid, options, Encoding::Png)?;
    Ok(Rendered { png: drawn.bytes, width: drawn.width, height: drawn.height, empty_glyph, profile: drawn.profile })
}

/// [`render`], but the pixels it would encode, as RGBA.
pub fn render_rgba(grid: &Grid, options: &RenderOptions) -> Result<RgbaImage, Error> {
    let (drawn, empty_glyph) = draw(grid, options, Encoding::Rgba)?;
    Ok(RgbaImage { rgba: drawn.bytes, width: drawn.width, height: drawn.height, empty_glyph })
}

/// An [`Error::Options`] unless `px` is above 0 and below 256.
fn check_px(px: f64) -> Result<(), Error> {
    if px > 0.0 && px < 256.0 {
        return Ok(());
    }
    Err(Error::Options(format!("px must be a number above 0 and below 256, not {px}")))
}

/// Check the options, then draw, with any panic an Error::Internal.
fn draw(grid: &Grid, options: &RenderOptions, encoding: Encoding) -> Result<(render::Drawn, Option<EmptyGlyph>), Error> {
    check_px(options.px)?;
    let (x, y) = options.padding;
    if x > MAX_PADDING || y > MAX_PADDING {
        return Err(Error::Options(format!("padding must be pixels from 0 to {MAX_PADDING} on each side, not {x},{y}")));
    }
    if let Cursor::At { row, col } = options.cursor {
        check_cursor(row, col, grid.cols, grid.rows)?;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let embedded;
        let font = match options.font {
            Some(font) => font,
            None => {
                embedded = Font::embedded()?;
                &embedded
            }
        };
        draw_checked(grid, font, options, encoding)
    }))
    .unwrap_or_else(|_| Err(Error::Internal("the render panicked".into())))
}

/// The render of `grid` with `font`, its cells prepared as the CLI prepares
/// them (src/prepare.rs): placeholders blank, the cursor drawn, non-default
/// backgrounds opaque.
fn draw_checked(grid: &Grid, font: &Font, options: &RenderOptions, encoding: Encoding)
    -> Result<(render::Drawn, Option<EmptyGlyph>), Error> {
    let (cols, rows) = (grid.cols, grid.rows);
    let cell = font.cell(options.px)?;
    let palette = Palette { foreground: grid.foreground, background: grid.background, ..Palette::DEFAULT };
    render::start_faults();
    // The copies the cells are prepared in, made with try_reserve_exact.
    fn reserve<T>(v: &mut Vec<T>, len: usize, what: &str, (cols, rows): (usize, usize)) -> Result<(), Error> {
        if render::allowed(Site::Prepare) && v.try_reserve_exact(len).is_ok() {
            return Ok(());
        }
        Err(Error::OutOfMemory(format!("out of memory for the {what} of a {cols}x{rows} grid")))
    }
    let mut cells = Vec::new();
    reserve(&mut cells, grid.cells.len(), "cells", (cols, rows))?;
    cells.extend_from_slice(&grid.cells);
    let mut marks: Vec<CellMarks> = Vec::new();
    reserve(&mut marks, grid.marks.len(), "combining marks", (cols, rows))?;
    marks.extend_from_slice(&grid.marks);
    let marks = prepare::blank_placeholders(&mut cells, marks);
    let views = grid.images.iter().map(|placement| placement.views().count()).sum::<usize>();
    let mut images: Vec<ImageView> = Vec::new();
    reserve(&mut images, views + 1, "image views", (cols, rows))?;
    let cursor = match options.cursor {
        Cursor::FromGrid => grid.cursor,
        Cursor::Hidden => None,
        Cursor::At { row, col } => Some((row, col)),
    };
    // The underline or bar cursor's colour; its view borrows it.
    let mark_pixel;
    match (cursor, options.cursor_shape.unwrap_or(grid.cursor_shape)) {
        (None, _) => {}
        (Some((row, col)), CursorShape::Block) => prepare::draw_cursor(&mut cells, cols, row, col, &palette),
        // As kitty draws it, with the text: over the images under the text,
        // under those of z-index 0 and up, so it goes first among the views
        // drawn after the text.
        (Some(at), shape) => {
            let ((x, y, w, h), pixel) = prepare::cursor_mark(&cells, cols, at, shape, cell, &palette);
            mark_pixel = pixel;
            images.push(ImageView::solid(&mark_pixel, x, y, w, h));
        }
    }
    images.extend(grid.images.iter().flat_map(|placement| placement.views()));
    prepare::opaque_backgrounds(&mut cells, grid.background);
    let render_options = render::RenderOptions { padding: options.padding, background: grid.background };
    let mut empty = EmptyGlyphs::default();
    // with_face parses a CFF table again; that is face_ms.
    let face_started = Instant::now();
    let mut face_ms = 0.0;
    let mut draw = |face: &font::Face, fallback: Option<&font::Face>| {
        face_ms = face_started.elapsed().as_secs_f64() * 1000.0;
        // SAFETY: the faces are checked fonts', padded and borrowed for the
        // call; each view's pixels are a placement's, which the grid holds,
        // or the cursor's pixel.
        unsafe {
            render::draw(&cells, &marks, cols, rows, face.ffi(), fallback.map(font::Face::ffi), options.px, &images,
                         Some(&mut empty), &render_options, encoding, options.profile)
        }
    };
    let drawn = font.font.with_face(|face| match options.fallback {
        None => Ok(draw(face, None)),
        Some(fallback) => fallback.font.with_face(|fallback| draw(face, Some(fallback))),
    });
    let mut drawn = drawn.and_then(|drawn| drawn).map_err(Error::Font)?.map_err(failure)?;
    if let Some(fields) = &mut drawn.profile {
        *fields += &format!(",\"face_ms\":{face_ms:.6}");
    }
    Ok((drawn, EmptyGlyph::of(&empty)))
}

/// The Error for a render's Failure, with the CLI's message.
fn failure(failure: Failure) -> Error {
    match failure {
        Failure::FontInit | Failure::FontMetrics => Error::Font(failure.message()),
        Failure::TooLarge { width, height, padded } => {
            Error::ImageTooLarge { width: width as u64, height: height as u64, padded }
        }
        Failure::Canvas { .. } | Failure::Glyphs | Failure::Encode { .. } | Failure::Rgba { .. } => {
            Error::OutOfMemory(failure.message())
        }
        Failure::BoxDrawing | Failure::Painting => Error::Internal(failure.message()),
    }
}

/// What only the CLI needs: the timings of a font's load for its profile
/// record, and its -v line. Not part of the library's API.
#[doc(hidden)]
pub mod cli {
    use super::*;

    pub use crate::font::LoadTimings;

    /// The line -v prints before a render of a `cols` x `rows` grid with
    /// these fonts: the font's advance, scale, cell and baseline, and the
    /// image's size, padding included. The fonts are set up as the render
    /// sets them up, so a face that can't be is the same Error.
    pub fn verbose_line(font: &Font, fallback: Option<&Font>, px: f64, cols: usize, rows: usize,
                        padding: (u32, u32)) -> Result<String, Error> {
        check_px(px)?;
        let metrics = |face: &font::Face, fallback: Option<&font::Face>| {
            // SAFETY: the faces are checked fonts', padded and borrowed.
            unsafe { render::cell_metrics(face.ffi(), fallback.map(font::Face::ffi), px) }
        };
        let metrics = font.font.with_face(|face| match fallback {
            None => Ok(metrics(face, None)),
            Some(fallback) => fallback.font.with_face(|fallback| metrics(face, Some(fallback))),
        });
        let m = metrics.and_then(|m| m).map_err(Error::Font)?.map_err(failure)?;
        Ok(render::verbose_line(&m, render::image_size(&m, cols, rows, padding).1))
    }
}
