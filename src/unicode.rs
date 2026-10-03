//! Character widths and canonical composition, looked up in the tables that
//! tools/unicode-tables.sh generates into src/unicode_tables.rs.

use crate::unicode_tables::{COMPOSE, COMPOSING_MARKS, DOUBLE_WIDTH, WIDTH_BLOCKS, WIDTH_LIMIT, WIDTH_ROWS, ZERO_WIDTH};
use std::cmp::Ordering;

fn in_ranges(table: &[(u32, u32)], cp: u32) -> bool {
    table
        .binary_search_by(|&(first, last)| {
            if last < cp {
                Ordering::Less
            } else if first > cp {
                Ordering::Greater
            } else {
                Ordering::Equal
            }
        })
        .is_ok()
}

/// The cells a printable character takes: 0 for combining marks and format
/// characters, 2 for wide (CJK, fullwidth, emoji), otherwise 1.
pub fn width(cp: u32) -> usize {
    // Nothing below U+0300 is zero width or wide (U+00AD counts as 1).
    if cp < 0x300 {
        1
    } else if cp < WIDTH_LIMIT {
        // The same widths as the ranges below, as a table (#21).
        let row = &WIDTH_ROWS[usize::from(WIDTH_BLOCKS[(cp >> 6) as usize])];
        usize::from(row[(cp as usize & 63) >> 2] >> ((cp & 3) * 2) & 3)
    } else {
        width_in_ranges(cp)
    }
}

/// width, from the range tables alone.
pub fn width_in_ranges(cp: u32) -> usize {
    if cp < 0x300 {
        1
    } else if in_ranges(&ZERO_WIDTH, cp) {
        0
    } else if in_ranges(&DOUBLE_WIDTH, cp) {
        2
    } else {
        1
    }
}

/// The canonical composition of base followed by mark, if Unicode has one
/// (e + U+0301 COMBINING ACUTE ACCENT is é).
pub fn compose(base: u32, mark: u32) -> Option<u32> {
    // Most marks compose with nothing (Thai, Hebrew): rule them out in
    // the short list of those that do before searching the pairs (#21).
    COMPOSING_MARKS.binary_search(&mark).ok()?;
    COMPOSE.binary_search_by(|&(b, m, _)| (b, m).cmp(&(base, mark))).ok().map(|i| COMPOSE[i].2)
}
