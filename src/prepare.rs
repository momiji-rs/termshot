//! The render's preparation of a grid's cells: what the PNG shows that the
//! cells, as --text and --json see them, do not. Unicode placeholders become
//! blanks, the cursor is drawn in, and non-default backgrounds are marked
//! opaque.

use crate::cell::{Cell, CellMarks, OPAQUE, TAIL, WIDE};
use crate::graphics;
use crate::palette::Palette;
use crate::screen::CursorShape;

/// kitty draws a Unicode placeholder (U+10EEEE) as a blank cell, its
/// diacritics too: the image it shows comes from graphics::Graphics::finish.
/// Make each one a space, with its colours and attributes, and drop its
/// marks, for the render. --text and --json keep them.
pub(crate) fn blank_placeholders(cells: &mut [Cell], mut marks: Vec<CellMarks>) -> Vec<CellMarks> {
    marks.retain(|m| cells[m.cell as usize].ch != graphics::PLACEHOLDER);
    for cell in cells.iter_mut().filter(|cell| cell.ch == graphics::PLACEHOLDER) {
        cell.ch = ' ' as u32;
    }
    marks
}

/// Mark every background that is not the default colour, `background`
/// (the palette's), OPAQUE, for the render. kitty compares the colour's
/// value, so a background set to the default colour explicitly is a default
/// one.
pub(crate) fn opaque_backgrounds(cells: &mut [Cell], background: (u8, u8, u8)) {
    for cell in cells {
        if (cell.br, cell.bg, cell.bb) != background {
            cell.attrs |= OPAQUE;
        }
    }
}

/// Draw the cursor as a block in reverse video over the cell at (row, col),
/// or over both cells of the wide character it is on. Over concealed text it
/// is a block of the palette's default foreground, or of its background on
/// a cell already that colour.
pub(crate) fn draw_cursor(cells: &mut [Cell], cols: usize, row: usize, col: usize, palette: &Palette) {
    let under = cursor_cells(cells, cols, row, col);
    for cell in &mut cells[under] {
        let (fg, bg) = ((cell.fr, cell.fg, cell.fb), (cell.br, cell.bg, cell.bb));
        // Concealed text (the colours alike) stays hidden in a block that
        // still shows.
        let (fg, bg) = if fg != bg {
            (bg, fg)
        } else {
            let block = if bg == palette.foreground { palette.background } else { palette.foreground };
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
/// the palette's default foreground; on a cell whose background is that
/// colour, in its default background, so the cursor still shows.
pub(crate) fn cursor_mark(
    cells: &[Cell],
    cols: usize,
    (row, col): (usize, usize),
    shape: CursorShape,
    (cell_w, cell_h): (i32, i32),
    palette: &Palette,
) -> ((i64, i64, i64, i64), [u8; 4]) {
    let under = cursor_cells(cells, cols, row, col);
    let first = &cells[under.start];
    let (fg, bg) = (palette.foreground, palette.background);
    let (r, g, b) = if (first.br, first.bg, first.bb) == fg { bg } else { fg };
    let (cell_w, cell_h) = (i64::from(cell_w), i64::from(cell_h));
    let thick = (cell_w / 8).max(1);
    let (x, y) = ((under.start - row * cols) as i64 * cell_w, row as i64 * cell_h);
    let rect = match shape {
        CursorShape::Underline => (x, y + cell_h - thick, under.len() as i64 * cell_w, thick),
        CursorShape::Bar | CursorShape::Block => (x, y, thick, cell_h),
    };
    (rect, [r, g, b, 255])
}
