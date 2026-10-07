//! Reading a log: the byte-level scans, escape dispatch, CSI parameters and
//! UTF-8, the routing of APC and DCS strings to kitty graphics and Sixel, and
//! the replay that drives a Screen to the Grid a log leaves.


#[cfg(test)]
use crate::cell::Cell;
use crate::grid::Grid;
use crate::palette::Palette;
use crate::screen::{Charset, Lf, Screen};
use crate::{graphics, sixel};

/// An RGB triple, or None when a component is over 255.
pub(crate) fn rgb(params: &[Param]) -> Option<(u8, u8, u8)> {
    let c = |i: usize| u8::try_from(params[i].value.unwrap_or(0)).ok();
    Some((c(0)?, c(1)?, c(2)?))
}

pub(crate) const MAX_PARAMS: usize = 32;

/// One CSI parameter. `sub` marks a ':' subparameter of the one before.
#[derive(Clone, Copy, Default)]
pub(crate) struct Param {
    pub(crate) value: Option<u32>,
    pub(crate) sub: bool,
}

/// CSI parameters, parsed in place without allocating. Extras past
/// MAX_PARAMS are dropped.
pub(crate) struct Params {
    pub(crate) list: [Param; MAX_PARAMS],
    pub(crate) len: usize,
}

impl Params {
    /// The parameter at index, or default when it is missing or empty.
    pub(crate) fn get(&self, index: usize, default: u32) -> u32 {
        if index < self.len {
            self.list[index].value.unwrap_or(default)
        } else {
            default
        }
    }

    pub(crate) fn push(&mut self, param: Param) {
        if self.len < MAX_PARAMS {
            self.list[self.len] = param;
            self.len += 1;
        }
    }
}

/// Where the run of printable ASCII (0x20..=0x7e) from i ends. Most runs
/// are short, so the first 16 bytes go one at a time; then eight at a time
/// while they all are printable (#21), and the rest one at a time again.
pub(crate) fn printable_end(data: &[u8], mut i: usize) -> usize {
    const ONES: u64 = u64::from_ne_bytes([0x01; 8]);
    const HIGH: u64 = u64::from_ne_bytes([0x80; 8]);
    let short = data.len().min(i + 16);
    while i < short {
        if !(0x20..0x7f).contains(&data[i]) {
            return i;
        }
        i += 1;
    }
    while let Some(chunk) = data.get(i..i + 8) {
        let w = u64::from_ne_bytes(chunk.try_into().unwrap());
        // A byte below 0x20, one with its high bit set, or 0x7f; the
        // first and third are the bit trick for "has a byte below n".
        let del = w ^ (ONES * 0x7f);
        if (w.wrapping_sub(ONES * 0x20) & !w | w | del.wrapping_sub(ONES) & !del) & HIGH != 0 {
            break;
        }
        i += 8;
    }
    while i < data.len() && (0x20..0x7f).contains(&data[i]) {
        i += 1;
    }
    i
}

/// Skip a string sequence (OSC, DCS, APC, PM, SOS) starting at i, returning
/// where parsing resumes. It ends at BEL or ST (ESC \). Any other ESC aborts
/// it and starts the next sequence; CAN and SUB abort it.
pub(crate) fn skip_string(data: &[u8], mut i: usize) -> usize {
    while i < data.len() {
        match data[i] {
            0x07 | 0x18 | 0x1a => return i + 1,
            0x1b if data.get(i + 1) == Some(&b'\\') => return i + 2,
            0x1b => return i,
            _ => i += 1,
        }
    }
    i
}

/// The bytes of an eight-byte word equal to `byte`, as their high bits.
/// Exact for every byte: `(x & 0x7f) + 0x7f` cannot carry into the next one.
pub(crate) fn bytes_equal(word: u64, byte: u8) -> u64 {
    const LOW: u64 = u64::from_le_bytes([0x7f; 8]);
    let x = word ^ u64::from_le_bytes([byte; 8]);
    !((x & LOW).wrapping_add(LOW) | x | LOW)
}

/// Calls `found` with the introducer (`P` or `_`) and the bytes of each DCS
/// and APC string of a log, from after the introducer through where
/// `skip_string` ends it, in order, until `found` returns true. Like the
/// byte-at-a-time scans it replaced, it looks only at each ESC and the byte
/// after it, not at the parser's state. Those scans also skipped OSC, PM
/// and SOS strings, but a skip passes no ESC except the one of an ST, so
/// it never hid an ESC P or ESC _: every one of those is a string here.
/// Sixteen bytes at a time are tested for them (#21).
pub(crate) fn any_string(data: &[u8], mut found: impl FnMut(u8, &[u8]) -> bool) -> bool {
    let mut i = 0;
    while i + 1 < data.len() {
        // With no branch on where the ESCs are: in an escape-heavy log,
        // which word holds one is hard to predict.
        let at = match data.get(i..i + 17) {
            Some(block) => {
                let word = |at: usize| u64::from_le_bytes(block[at..at + 8].try_into().unwrap());
                let hits = |at: usize| {
                    bytes_equal(word(at), 0x1b) & (bytes_equal(word(at + 1), b'P') | bytes_equal(word(at + 1), b'_'))
                };
                let both = u128::from(hits(0)) | u128::from(hits(8)) << 64;
                if both == 0 {
                    i += 16;
                    continue;
                }
                i + both.trailing_zeros() as usize / 8
            }
            None if data[i] == 0x1b && matches!(data[i + 1], b'P' | b'_') => i,
            None => {
                i += 1;
                continue;
            }
        };
        let end = skip_string(data, at + 2);
        if found(data[at + 1], &data[at + 2..end]) {
            return true;
        }
        i = end;
    }
    false
}

/// Whether a run without a PNG must still read fonts: a kitty command or a
/// Sixel image whose placement can move the text cursor by cells of the
/// font's size. One pass over the log for both (#21).
pub(crate) fn needs_cell_metrics(data: &[u8]) -> bool {
    let mut kitty = graphics::CellMetricsScan::default();
    any_string(data, |kind, s| match kind {
        b'_' => kitty.string(s),
        b'P' => sixel::string_needs_cell_metrics(s),
        _ => false,
    })
}

/// Whether a log looks like it never went through a PTY: it has line feeds
/// but no CR at all, which `onlcr` would have added before each one.
/// Checks CR first, so a PTY log stops at its first line end.
pub(crate) fn lacks_cr(data: &[u8]) -> bool {
    !data.contains(&b'\r') && data.contains(&b'\n')
}

/// Text is lines that each end in LF, so the bare LF that ends the input
/// ends the last line rather than opening a new one. Otherwise a capture as
/// tall as the grid (tmux capture-pane ends every row with LF) would scroll
/// its top row away. A final CR LF is a PTY's and stays, so a PTY log
/// renders the same with --lf-newline as without.
pub(crate) fn strip_final_bare_lf(data: &[u8]) -> &[u8] {
    match data {
        [.., b'\r', b'\n'] => data,
        [rest @ .., b'\n'] => rest,
        _ => data,
    }
}

/// Replay a log as a terminal would, with bare LFs indexing.
#[cfg(test)]
pub(crate) fn parse(data: &[u8], cols: usize, rows: usize) -> Vec<Cell> {
    parse_lf(data, cols, rows, Lf::Index)
}

#[cfg(test)]
pub(crate) fn parse_lf(data: &[u8], cols: usize, rows: usize, lf: Lf) -> Vec<Cell> {
    replay(data, cols, rows, lf).cells
}

/// What decides the cells a log replays to: how a bare LF moves, and the
/// colours its SGR codes stand for. --text, --json and the PNG all see it;
/// the render's own options (render::RenderOptions) only change pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParseOptions {
    /// What a bare LF does: `Lf::Index` for PTY output, `Lf::Newline` for
    /// text that never went through a PTY (`--lf-newline`).
    pub lf: Lf,
    /// The colours the default and the 16 named colours stand for.
    pub palette: Palette,
}

impl Default for ParseOptions {
    /// What the CLI uses without `--lf-newline` or a palette option.
    fn default() -> ParseOptions {
        ParseOptions { lf: Lf::Index, palette: Palette::DEFAULT }
    }
}

#[cfg(test)]
pub(crate) fn replay(data: &[u8], cols: usize, rows: usize, lf: Lf) -> Grid {
    replay_sized(data, cols, rows, lf, (1, 1))
}

/// replay_with the default palette.
#[cfg(test)]
pub(crate) fn replay_sized(data: &[u8], cols: usize, rows: usize, lf: Lf, cell_size: (i32, i32)) -> Grid {
    replay_with(data, cols, rows, &ParseOptions { lf, palette: Palette::DEFAULT }, cell_size).unwrap()
}

/// Replay `data` on a `cols` x `rows` screen with cells of `cell_size`
/// pixels: the grid it leaves, or None when memory for the screen ran out
/// (the allocations src/screen.rs's `reserved` makes).
pub(crate) fn replay_with(data: &[u8], cols: usize, rows: usize, options: &ParseOptions, cell_size: (i32, i32))
    -> Option<Grid> {
    let lf = options.lf;
    let data = match lf {
        Lf::Newline => strip_final_bare_lf(data),
        Lf::Index => data,
    };
    crate::screen::faults::start();
    let mut screen = Screen::with(cols, rows, options)?;
    screen.cell_size = cell_size;
    // Reuse the fixed parameter buffer across sequences; only len needs resetting.
    let mut params = Params { list: [Param::default(); MAX_PARAMS], len: 0 };
    // One budget for the whole log: a reset does not refill it.
    let mut sixel_budget = sixel::Budget::default();
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        if (0x20..0x7f).contains(&b) {
            let start = i;
            i = printable_end(data, i + 1);
            screen.print_ascii(&data[start..i]);
            continue;
        }
        if b == 0x1b {
            let Some(&kind) = data.get(i + 1) else {
                break;
            };
            i += 2;
            match kind {
                b'[' => i = csi(&mut screen, &mut params, data, i),
                b'_' if data.get(i) == Some(&b'G') => {
                    let end = skip_string(data, i);
                    // Only ST commits a graphics command; BEL/CAN/SUB, another
                    // escape, or EOF discard it and any incomplete upload.
                    if end >= i + 3 && data.get(end - 2..end) == Some(b"\x1b\\") {
                        if let Some((dc, dr)) = screen.graphics.command(
                            &data[i + 1..end - 2], screen.col, screen.row, cell_size, rows,
                        ) {
                            screen.move_past_image(dc, dr);
                        }
                    } else { screen.graphics.abort(); }
                    i = end;
                }
                b'P' => {
                    let end = skip_string(data, i);
                    // As for kitty graphics, only ST commits an image.
                    if end >= i + 2 && data.get(end - 2..end) == Some(b"\x1b\\") {
                        if let Some(image) = sixel::decode(&data[i..end - 2], &mut sixel_budget) {
                            screen.sixel(image);
                        }
                    }
                    i = end;
                }
                b']' | b'_' | b'^' | b'X' => i = skip_string(data, i),
                // ESC ( B, ESC ) 0, ESC # 8: intermediates, then one final byte.
                0x20..=0x2f => {
                    let first = i;
                    while i < data.len() && (0x20..=0x2f).contains(&data[i]) {
                        i += 1;
                    }
                    if i < data.len() && (0x30..=0x7e).contains(&data[i]) {
                        // ESC ( x designates G0, ESC ) x G1. '0' is DEC Special
                        // Graphics; any other set is drawn as ASCII.
                        if i == first && matches!(kind, b'(' | b')') {
                            let set = if data[i] == b'0' { Charset::DecGraphics } else { Charset::Ascii };
                            screen.charsets[usize::from(kind == b')')] = set;
                        }
                        i += 1;
                    }
                }
                b'7' => screen.save_cursor(),
                b'8' => screen.restore_cursor(),
                // IND, NEL, RI.
                b'D' => screen.index(),
                b'E' => {
                    screen.col = 0;
                    screen.index();
                }
                b'M' => screen.reverse_index(),
                // HTS: set a tab stop at the cursor.
                b'H' => {
                    let col = screen.col;
                    screen.tabs[col] = true;
                }
                // RIS: full reset.
                b'c' => {
                    // kitty's reset clears both screens' images but keeps
                    // their virtual placements (grman_clear).
                    let mut kept = [std::mem::take(&mut screen.graphics), std::mem::take(&mut screen.other_graphics)];
                    if screen.on_alternate {
                        kept.swap(0, 1);
                    }
                    for graphics in &mut kept {
                        graphics.abort();
                        graphics.clear();
                    }
                    let out_of_memory = screen.out_of_memory;
                    screen = Screen::with(cols, rows, options)?;
                    screen.cell_size = cell_size;
                    screen.out_of_memory = out_of_memory;
                    [screen.graphics, screen.other_graphics] = kept;
                },
                // CAN and SUB cancel the escape.
                0x18 | 0x1a => {}
                // Another ESC starts over; other C0 controls still execute.
                0x00..=0x1f => i -= 1,
                // Other two-byte escapes (ESC =, ESC >, ...) don't change the grid.
                _ => {}
            }
            continue;
        }
        match b {
            0x00..=0x1f => screen.control(b),
            0x7f => {}
            _ => {
                let (cp, n) = utf8_at(&data[i..]);
                // C1 controls encoded as UTF-8 take no cell.
                if !(0x80..=0x9f).contains(&cp) {
                    screen.print(cp);
                }
                i += n;
                continue;
            }
        }
        i += 1;
    }
    // With a wrap pending the cursor stays on the last column, where
    // terminals draw it.
    let cursor = screen.cursor_shown.then_some((screen.row, screen.col));
    // Memory ran out for the marks or the ids: fail now, before the work
    // below allocates more.
    if screen.out_of_memory {
        return None;
    }
    // Placeholder cells show nothing without a virtual placement to name.
    let placeholders = if screen.graphics.has_virtual() { screen.placeholders()? } else { Vec::new() };
    let images = std::mem::take(&mut screen.graphics).finish(&placeholders, cell_size, rows)?;
    let cursor_shape = screen.cursor_shape;
    let marks = screen.screen_marks()?;
    let (foreground, background) = (options.palette.foreground, options.palette.background);
    Some(Grid { cells: screen.into_cells()?, marks, cursor, cursor_shape, images, cols, rows, foreground, background })
}

/// Parse one CSI sequence whose parameters start at i, apply it, and return
/// where parsing resumes. C0 controls inside it execute in place; ESC aborts
/// it and starts the next sequence; CAN and SUB abort it.
pub(crate) fn csi(screen: &mut Screen, params: &mut Params, data: &[u8], mut i: usize) -> usize {
    params.len = 0;
    let mut current = Param::default();
    let mut any = false;
    // The private marker (one of < = > ?) when the sequence starts with one.
    let mut private = None;
    // The intermediate byte; a second one makes a sequence nothing here uses.
    let mut intermediate = None;
    let mut malformed = false;
    let start = i;
    while i < data.len() {
        let c = data[i];
        // Digits, ':' and ';' first, with one test: they are most of every
        // sequence (#21). After an intermediate they are malformed, below.
        let offset = c.wrapping_sub(b'0');
        if offset <= b';' - b'0' && intermediate.is_none() {
            if offset < 10 {
                let value = u64::from(current.value.unwrap_or(0)) * 10 + u64::from(offset);
                current.value = Some(value.min(u64::from(u32::MAX)) as u32);
            } else {
                params.push(current);
                current = Param { value: None, sub: c == b':' };
            }
            any = true;
            i += 1;
            continue;
        }
        match c {
            // Parameter bytes come before intermediates, never after.
            0x30..=0x3f if intermediate.is_some() => malformed = true,
            b'0'..=b'9' => {
                // A u32 times ten plus a digit fits in u64; one clamp preserves
                // saturating arithmetic without two overflow checks per digit.
                let value = u64::from(current.value.unwrap_or(0)) * 10 + u64::from(c - b'0');
                current.value = Some(value.min(u64::from(u32::MAX)) as u32);
                any = true;
            }
            b';' | b':' => {
                params.push(current);
                current = Param { value: None, sub: c == b':' };
                any = true;
            }
            b'<'..=b'?' if i == start => private = Some(c),
            b'<'..=b'?' => malformed = true,
            0x20..=0x2f => {
                malformed |= intermediate.is_some();
                intermediate = Some(c);
            }
            0x40..=0x7e => {
                if any {
                    params.push(current);
                }
                if !malformed {
                    match (private, intermediate, c) {
                        (None, None, _) => screen.csi(c, params),
                        (Some(b'?'), None, _) => screen.private_csi(c, params),
                        (None, Some(b' '), b'q') => screen.set_cursor_shape(params),
                        _ => {}
                    }
                }
                return i + 1;
            }
            0x1b => return i,
            0x18 | 0x1a => return i + 1,
            0x00..=0x1f => screen.control(c),
            0x7f => {}
            // A non-ASCII byte cannot be part of a CSI: abort and print it.
            _ => return i,
        }
        i += 1;
    }
    i
}

/// Decode one UTF-8 character. Invalid or truncated input gives U+FFFD and
/// consumes only the lead byte and the continuation bytes that were valid.
pub(crate) fn utf8_at(data: &[u8]) -> (u32, usize) {
    const REPLACEMENT: u32 = 0xfffd;
    let b0 = data[0];
    // Continuation count, and the allowed range of the first continuation
    // byte (it excludes overlongs, surrogates and code points past U+10FFFF).
    let (need, first) = match b0 {
        0x00..=0x7f => return (u32::from(b0), 1),
        0xc2..=0xdf => (1, 0x80..=0xbf),
        0xe0 => (2, 0xa0..=0xbf),
        0xe1..=0xec | 0xee..=0xef => (2, 0x80..=0xbf),
        0xed => (2, 0x80..=0x9f),
        0xf0 => (3, 0x90..=0xbf),
        0xf1..=0xf3 => (3, 0x80..=0xbf),
        0xf4 => (3, 0x80..=0x8f),
        _ => return (REPLACEMENT, 1),
    };
    let mut cp = u32::from(b0) & (0x7f >> (need + 1));
    for k in 1..=need {
        let valid = match data.get(k) {
            Some(&b) if k == 1 => first.contains(&b),
            Some(&b) => (0x80..=0xbf).contains(&b),
            None => false,
        };
        if !valid {
            return (REPLACEMENT, k);
        }
        cp = (cp << 6) | (u32::from(data[k]) & 0x3f);
    }
    (cp, need + 1)
}
