//! The screen a log leaves, and --text and --json, its text and JSON forms.

use crate::cell::{Cell, CellMarks, BOLD, DOUBLE_UNDERLINE, ITALIC, STRIKE, TAIL, UNDERLINE};
use crate::graphics;
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
/// (row, col) unless the log hid it, and its shape.
pub(crate) struct Grid {
    pub(crate) images: Vec<graphics::Placement>,
    pub(crate) cells: Vec<Cell>,
    /// The cells' combining marks, sorted by cell.
    pub(crate) marks: Vec<CellMarks>,
    pub(crate) cursor: Option<(usize, usize)>,
    pub(crate) cursor_shape: CursorShape,
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
/// `background`, the palette's default) or a line shows.
pub(crate) fn grid_json(
    cells: &[Cell],
    marks: &[CellMarks],
    cols: usize,
    rows: usize,
    cursor: Option<(usize, usize)>,
    shape: CursorShape,
    background: (u8, u8, u8),
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
    json.push_str("\n]}\n");
    json
}
