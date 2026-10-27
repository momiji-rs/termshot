//! The screen a log leaves, and --text and --json, its text and JSON forms.

use crate::cell::{Cell, CellMarks, BOLD, DOUBLE_UNDERLINE, ITALIC, STRIKE, TAIL, UNDERLINE};
use crate::graphics;
use crate::palette::Rgb;
use crate::screen::CursorShape;

/// The marks of the cell at index (row * cols + col) in a sorted list, or none.
pub(crate) fn marks_of(marks: &[CellMarks], cell: usize) -> &[u32] {
    if marks.is_empty() {
        return &[];
    }
    match marks.binary_search_by_key(&cell, |m| m.cell as usize) {
        Ok(k) => marks[k].code_points(),
        Err(_) => &[],
    }
}

/// The screen a log leaves: its cells in screen order, the cursor as
/// (row, col) unless the log hid it, its shape, and the images placed on it.
pub struct Grid {
    /// The images placed, which the render draws.
    pub(crate) images: Vec<graphics::Placement>,
    pub(crate) cells: Vec<Cell>,
    /// The cells' combining marks, sorted by cell.
    pub(crate) marks: Vec<CellMarks>,
    pub(crate) cursor: Option<(usize, usize)>,
    pub(crate) cursor_shape: CursorShape,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    /// The palette's default colours: the JSON compares with the
    /// background, and the render draws the cursor and the padding in them.
    pub(crate) foreground: Rgb,
    pub(crate) background: Rgb,
}

impl Grid {
    /// The screen as text, byte for byte what `--text` writes: a line per
    /// row with its trailing spaces trimmed, the way `tmux capture-pane -p`
    /// prints it, each character followed by its combining marks.
    pub fn to_text(&self) -> String {
        grid_text(&self.cells, &self.marks, self.cols)
    }

        /// The screen as JSON, byte for byte what `--json` writes: the grid
    /// size, the cursor (or null when the log hid it), a line per row of
    /// the runs of cells alike in colour and attributes, and, when the
    /// screen has any, the images and where they are placed.
    pub fn to_json(&self) -> String {
        let images = screen_images(&self.images);
        grid_json(
            &self.cells,
            &self.marks,
            self.cols,
            self.rows,
            self.cursor,
            self.cursor_shape,
            self.background,
            &images,
        )
    }

    /// The images on the final screen, in draw order, each with the
    /// placements that show it. Empty when the screen has none. [`Grid::to_json`]
    /// writes this as `images` and leaves that key out when it is empty.
    pub fn images(&self) -> Vec<ScreenImage> {
        screen_images(&self.images)
    }
}

/// An image on the final screen, as `--json` describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenImage {
    /// Kitty's image id. None for a Sixel image, and for a kitty image
    /// transmitted without one.
    pub id: Option<u32>,
    /// The image's width in pixels.
    pub width: u32,
    /// The image's height in pixels.
    pub height: u32,
    /// Where it is drawn, in draw order.
    pub placements: Vec<ImagePlacement>,
}

/// One place an image is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImagePlacement {
    /// Kitty's placement id (`p`), when the command named one.
    pub id: Option<u32>,
    /// The source rectangle's left edge, in image pixels.
    pub x: u32,
    /// The source rectangle's top edge, in image pixels.
    pub y: u32,
    /// The source rectangle's width, in image pixels.
    pub w: u32,
    /// The source rectangle's height, in image pixels.
    pub h: u32,
    /// The column the placement covers, from 0. Negative when scrolling
    /// has carried it off the left of the screen.
    pub col: i64,
    /// The row the placement covers, from 0. Negative when scrolling has
    /// carried it above the screen.
    pub row: i64,
    /// How many columns it covers.
    pub columns: i64,
    /// How many rows it covers.
    pub rows: i64,
    /// Kitty's z-index: 0 and up draw over the text, below 0 under it.
    pub z: i32,
    /// Shown through Unicode placeholder cells.
    pub virtual_placement: bool,
    /// Placed relative to another placement.
    pub relative: bool,
    /// A Sixel image.
    pub sixel: bool,
}

/// The images `--json` lists, grouped by the image they show, in the
/// placements' draw order.
fn screen_images(placements: &[graphics::Placement]) -> Vec<ScreenImage> {
    let mut images: Vec<(u64, ScreenImage)> = Vec::new();
    for placement in placements {
        let drawn = ImagePlacement {
            id: (placement.placement_id != 0).then_some(placement.placement_id),
            x: placement.src[0],
            y: placement.src[1],
            w: placement.src[2],
            h: placement.src[3],
            col: placement.col,
            row: placement.row,
            columns: placement.cols,
            rows: placement.rows,
            z: placement.z,
            virtual_placement: placement.by_placeholder,
            relative: placement.parent.is_some(),
            sixel: placement.sixel,
        };
        if let Some((_, image)) = images.iter_mut().find(|(key, _)| *key == placement.image) {
            image.placements.push(drawn);
        } else {
            images.push((
                placement.image,
                ScreenImage {
                    id: (placement.id != 0).then_some(placement.id),
                    width: placement.width,
                    height: placement.height,
                    placements: vec![drawn],
                },
            ));
        }
    }
    images.into_iter().map(|(_, image)| image).collect()
}

/// A code point as a char; none of the grid's are invalid.
pub(crate) fn to_char(cp: u32) -> char {
    char::from_u32(cp).unwrap_or('\u{fffd}')
}

/// The screen as text, the way tmux capture-pane -p prints it: a line per row
/// with its trailing spaces trimmed, a wide character once, and each
/// character followed by its combining marks.
pub(crate) fn grid_text(cells: &[Cell], marks: &[CellMarks], cols: usize) -> String {
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
/// end a row are left out, as in --text, unless their background (other than
/// `background`, the palette's default) or a line shows. `images` follows
/// the rows when the screen has any, and is left out when it has none.
pub(crate) fn grid_json(
    cells: &[Cell],
    marks: &[CellMarks],
    cols: usize,
    rows: usize,
    cursor: Option<(usize, usize)>,
    shape: CursorShape,
    background: (u8, u8, u8),
    images: &[ScreenImage],
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
            cell.ch == ' ' as u32 && (cell.br, cell.bg, cell.bb) == background && cell.attrs & LINES == 0
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
    json.push_str("\n]");
    push_images(&mut json, images);
    json.push_str("}\n");
    json
}

/// `images`, after the rows. Absent when the screen has none, so a log
/// without images keeps the JSON it had.
fn push_images(json: &mut String, images: &[ScreenImage]) {
    use std::fmt::Write as _;
    if images.is_empty() {
        return;
    }
    json.push_str(",\"images\":[");
    for (i, image) in images.iter().enumerate() {
        json.push_str(if i == 0 { "\n{" } else { ",\n{" });
        if let Some(id) = image.id {
            let _ = write!(json, "\"id\":{id},");
        }
        let _ = write!(
            json,
            "\"width\":{},\"height\":{},\"format\":\"rgba\",\"placements\":[",
            image.width, image.height
        );
        for (j, placement) in image.placements.iter().enumerate() {
            json.push_str(if j == 0 { "\n{" } else { ",\n{" });
            if let Some(id) = placement.id {
                let _ = write!(json, "\"id\":{id},");
            }
            let _ = write!(
                json,
                "\"src\":{{\"x\":{},\"y\":{},\"w\":{},\"h\":{}}},\"col\":{},\"row\":{},\"columns\":{},\"rows\":{},\"z\":{}",
                placement.x, placement.y, placement.w, placement.h, placement.col, placement.row, placement.columns,
                placement.rows, placement.z
            );
            if placement.virtual_placement {
                json.push_str(",\"virtual\":true");
            }
            if placement.relative {
                json.push_str(",\"relative\":true");
            }
            if placement.sixel {
                json.push_str(",\"sixel\":true");
            }
            json.push('}');
        }
        json.push_str("\n]}");
    }
    json.push_str("\n]");
}
