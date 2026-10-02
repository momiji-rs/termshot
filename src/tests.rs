//! Parser unit tests. No crates, no FFI calls; ./test.sh builds and runs them.
//! A test for a known bug states the correct behaviour, is marked
//! #[ignore = "#N: ..."] with its issue, and should pass once it is fixed.
//! There are none open now. poc_workloads is ignored for another reason: it is
//! a benchmark helper, not a test.

use super::*;

const C: usize = 10;
const R: usize = 4;

fn grid(s: &[u8]) -> Vec<Cell> {
    parse(s, C, R)
}

fn at(cells: &[Cell], row: usize, col: usize) -> &Cell {
    &cells[row * C + col]
}

fn line(cells: &[Cell], row: usize) -> String {
    (0..C).map(|c| char::from_u32(at(cells, row, c).ch).unwrap_or('?')).collect()
}

/// A row as a terminal shows it: a wide character once, its tail skipped.
fn shown(row: &[Cell]) -> String {
    row.iter().filter(|c| c.attrs & TAIL == 0).map(|c| char::from_u32(c.ch).unwrap_or('?')).collect()
}

fn fg(c: &Cell) -> (u8, u8, u8) {
    (c.fr, c.fg, c.fb)
}

fn bg(c: &Cell) -> (u8, u8, u8) {
    (c.br, c.bg, c.bb)
}

#[test]
fn utf8_widths() {
    assert_eq!(utf8_at(b"A"), (0x41, 1));
    assert_eq!(utf8_at("·".as_bytes()), (0xb7, 2));
    assert_eq!(utf8_at("─".as_bytes()), (0x2500, 3));
    assert_eq!(utf8_at("😀".as_bytes()), (0x1f600, 4));
}

#[test]
fn prints_and_advances() {
    assert_eq!(line(&grid(b"ab"), 0), "ab        ");
}

#[test]
fn ascii_runs_without_wrap_preserve_cursor_and_attributes() {
    let g = grid(b"\x1b[?7l\x1b[1;38;2;1;2;3mabcdefghijklmnop\x1b[1D!\x1b[22m\r\nplain");
    assert_eq!(line(&g, 0), "abcdefgh!p");
    assert_eq!(line(&g, 1), "plain     ");
    assert_eq!(fg(at(&g, 0, 8)), (1, 2, 3));
    assert_eq!(at(&g, 0, 8).attrs & BOLD, BOLD);
    assert_eq!(at(&g, 1, 0).attrs & BOLD, 0);
}

#[test]
fn ascii_scroll_batches_match_individual_prints() {
    for cols in [1, 2, 10, 31] {
        for rows in [1, 2, 5] {
            for wrap in [false, true] {
                for alternate in [false, true] {
                    for len in [0, 1, cols - 1, cols, cols * rows, cols * rows * 3 + 1, 4097] {
                        let setup = || {
                            let mut s = Screen::new(cols, rows, Lf::Index);
                            s.use_alternate(alternate, false);
                            for i in 0..cols * rows {
                                s.print(if cols > 1 && i % 3 == 0 { '界' } else { '!' } as u32);
                            }
                            s.top = if rows > 2 { 1 } else { 0 };
                            s.bottom = if rows > 2 { rows - 2 } else { rows - 1 };
                            s.row = s.bottom;
                            s.col = cols - 1;
                            s.pending = true;
                            s.autowrap = wrap;
                            s.pen.fg = (1, 2, 3);
                            s.pen.bg = (4, 5, 6);
                            s.pen.attrs = BOLD | UNDERLINE | STRIKE;
                            s.pen.dim = true;
                            s.pen.reverse = true;
                            s
                        };
                        let bytes: Vec<u8> = (0..len).map(|i| b'!' + (i % 94) as u8).collect();
                        let (mut fast, mut reference) = (setup(), setup());
                        fast.print_ascii(&bytes);
                        for byte in bytes { reference.print(u32::from(byte)); }
                        assert_eq!((fast.row, fast.col, fast.pending, fast.last, fast.last_at),
                                   (reference.row, reference.col, reference.pending, reference.last, reference.last_at));
                        fast.combine(0x0301);
                        reference.combine(0x0301);
                        for (a, b) in fast.into_cells().iter().zip(reference.into_cells()) {
                            assert_eq!((a.ch, fg(a), bg(a), a.attrs), (b.ch, fg(&b), bg(&b), b.attrs));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn ascii_runs_stop_at_unicode_and_control_boundaries() {
    let g = grid("abéCD\x7fEF\rZ\nXY\x1b[2;8H!".as_bytes());
    assert_eq!(line(&g, 0), "ZbéCDEF   ");
    assert_eq!(line(&g, 1), " XY    !  ");
}

#[test]
fn ascii_runs_preserve_repeat_after_overwriting_last_column() {
    let g = grid(b"\x1b[?7labcdefghijk\r\x1b[2b");
    assert_eq!(line(&g, 0), "kkcdefghik");
}

#[test]
fn ascii_runs_preserve_mapped_repeat_across_charset_changes() {
    let g = grid(b"\x1b(0lqk\x1b(B\x1b[2bq\x1b)0\x0ex\x0f\x1b[b");
    assert_eq!(line(&g, 0), "┌─┐┐┐q││  ");
}

#[test]
fn csi_parameters_do_not_leak_across_sequences_or_aborts() {
    let g = grid(b"\x1b[38;2;1;2;3mA\x1b[mB\x1b[3;9Hc\x1b[123;456\x1b[Hd\x1b[;He");
    assert_eq!(line(&g, 0), "eB        ");
    assert_eq!(fg(at(&g, 0, 1)), DEFAULT_FG);
    assert_eq!(at(&g, 2, 8).ch, 'c' as u32);
}

#[test]
fn cr_and_lf() {
    let g = grid(b"ab\r\ncd\rX");
    assert_eq!(line(&g, 0), "ab        ");
    assert_eq!(line(&g, 1), "Xd        ");
}

fn lines(s: &[u8]) -> Vec<Cell> {
    parse_lf(s, C, R, Lf::Newline)
}

/// Every field, so two grids can be compared whole.
fn cell_key(c: &Cell) -> (u32, (u8, u8, u8), (u8, u8, u8), u8) {
    (c.ch, fg(c), bg(c), c.attrs)
}

#[test]
fn bare_lf_indexes_without_returning() {
    assert_eq!(line(&grid(b"ab\ncd"), 1), "  cd      ");
}

#[test]
fn a_log_without_cr_is_flagged() {
    assert!(lacks_cr(b"ab\ncd"));
    assert!(!lacks_cr(b"ab\r\ncd"));
    // One CR anywhere is enough to stay quiet: a PTY log with the odd bare
    // LF (a program that moves down without returning) is still a PTY log.
    assert!(!lacks_cr(b"ab\ncd\r"));
    // No line feed, nothing to hint about: a one-line log, or a TUI that
    // only addresses the cursor.
    assert!(!lacks_cr(b"ab"));
    assert!(!lacks_cr(b"\x1b[2;1Hab"));
    assert!(!lacks_cr(b""));
}

#[test]
fn lf_newline_returns_to_column_zero() {
    let g = lines(b"ab\ncd\r\nef");
    assert_eq!(line(&g, 0), "ab        ");
    assert_eq!(line(&g, 1), "cd        ");
    assert_eq!(line(&g, 2), "ef        ");
}

#[test]
fn lf_newline_cancels_a_pending_wrap_once() {
    // A full row leaves a wrap pending; the LF moves down one row, not two.
    let g = lines(b"0123456789\nab");
    assert_eq!(line(&g, 0), "0123456789");
    assert_eq!(line(&g, 1), "ab        ");
}

#[test]
fn lf_newline_scrolls_at_the_bottom() {
    let g = lines(b"1\n2\n3\n4\n5");
    assert_eq!((0..R).map(|r| line(&g, r)).collect::<Vec<_>>(), ["2         ", "3         ", "4         ", "5         "]);
}

#[test]
fn lf_newline_final_lf_ends_the_last_line() {
    // Four rows of text on four rows, as tmux capture-pane writes them.
    let g = lines(b"1\n2\n3\n4\n");
    assert_eq!((0..R).map(|r| line(&g, r)).collect::<Vec<_>>(), ["1         ", "2         ", "3         ", "4         "]);
    // Without the flag, the same LF scrolls, as on a terminal.
    assert_eq!(line(&grid(b"1\r\n2\r\n3\r\n4\r\n"), 0), "2         ");
}

#[test]
fn lf_newline_keeps_a_final_cr_lf() {
    // A CR LF is not a bare LF: a PTY log that ends in one scrolls either way.
    let log = b"1\r\n2\r\n3\r\n4\r\n";
    assert!(lines(log).iter().map(cell_key).eq(grid(log).iter().map(cell_key)));
    assert_eq!(line(&lines(log), 0), "2         ");
}

#[test]
fn lf_newline_keeps_an_empty_last_line() {
    // Only one LF is a terminator: the line before it is empty, and shown.
    let g = lines(b"1\n2\n3\n4\n\n");
    assert_eq!((0..R).map(|r| line(&g, r)).collect::<Vec<_>>(), ["2         ", "3         ", "4         ", "          "]);
}

#[test]
fn lf_newline_maps_only_lf() {
    // onlcr maps NL alone: VT and FF still only index.
    let g = lines(b"ab\x0bcd\x0cef");
    assert_eq!(line(&g, 1), "  cd      ");
    assert_eq!(line(&g, 2), "    ef    ");
}

#[test]
fn lf_newline_inside_a_csi() {
    // A C0 control inside a CSI executes in place, LF included.
    let g = lines(b"ab\x1b[\n1mcd");
    assert_eq!(line(&g, 1), "cd        ");
    assert_eq!(at(&g, 1, 0).attrs & BOLD, BOLD);
}

#[test]
fn lf_newline_survives_a_full_reset() {
    let g = lines(b"xx\x1bcab\ncd");
    assert_eq!(line(&g, 0), "ab        ");
    assert_eq!(line(&g, 1), "cd        ");
}

#[test]
fn bel_is_not_printed() {
    assert_eq!(line(&grid(b"a\x07b"), 0), "ab        ");
}

#[test]
fn cup_with_defaults() {
    let g = grid(b"\x1b[2;3Hx\x1b[Hy\x1b[3;fw\x1b[0;0fz");
    assert_eq!(at(&g, 1, 2).ch, 'x' as u32);
    assert_eq!(at(&g, 0, 0).ch, 'z' as u32);
    assert_eq!(at(&g, 2, 0).ch, 'w' as u32);
}

#[test]
fn cursor_moves_and_clamps() {
    let g = grid(b"\x1b[3;5H\x1b[2Aa\x1b[9Bb\x1b[3Dc\x1b[Cd\x1b[50Df");
    assert_eq!(at(&g, 0, 4).ch, 'a' as u32);
    assert_eq!(at(&g, 3, 3).ch, 'c' as u32);
    assert_eq!(at(&g, 3, 5).ch, 'd' as u32);
    assert_eq!(at(&g, 3, 0).ch, 'f' as u32);
    assert_eq!(at(&grid(b"\x1b[9Aa"), 0, 0).ch, 'a' as u32);
}

#[test]
fn save_and_restore_cursor() {
    let g = grid(b"\x1b[2;2H\x1b[s\x1b[4;4Hx\x1b[uy");
    assert_eq!(at(&g, 1, 1).ch, 'y' as u32);
    assert_eq!(at(&g, 3, 3).ch, 'x' as u32);
}

#[test]
fn private_and_unknown_csi_are_skipped() {
    let g = grid(b"\x1b[?25l\x1b[>4;1m\x1b[=1c\x1b[<0u\x1b[5ra\x1b[6n");
    assert_eq!(line(&g, 0), "a         ");
    assert_eq!(at(&g, 0, 0).attrs & BOLD, 0);
}

/// The grid with the cursor drawn where the log leaves it, if shown.
fn with_cursor(s: &[u8], cols: usize, rows: usize) -> Vec<Cell> {
    let Grid { mut cells, cursor } = replay(s, cols, rows, Lf::Index);
    if let Some((row, col)) = cursor {
        draw_cursor(&mut cells, cols, row, col);
    }
    cells
}

#[test]
fn the_cursor_is_a_block_in_reverse_video() {
    // On a blank cell: the default colours swapped.
    let g = with_cursor(b"ab", C, R);
    assert_eq!((fg(at(&g, 0, 2)), bg(at(&g, 0, 2))), (DEFAULT_BG, DEFAULT_FG));
    assert_eq!(bg(at(&g, 0, 1)), DEFAULT_BG);
    // On a character: its colours swapped, the character and attributes kept.
    let g = with_cursor(b"\x1b[1;4;31;42ma\x1b[m\x1b[H", C, R);
    let a = at(&g, 0, 0);
    assert_eq!((a.ch, a.attrs), ('a' as u32, BOLD | UNDERLINE));
    assert_eq!((fg(a), bg(a)), (palette(2).unwrap(), palette(1).unwrap()));
    // Hidden: nothing drawn.
    let g = with_cursor(b"ab\x1b[?25l", C, R);
    assert_eq!(bg(at(&g, 0, 2)), DEFAULT_BG);
}

#[test]
fn the_cursor_covers_both_halves_of_a_wide_character() {
    // On either half: both cells, and not the one after.
    for log in ["a中\x1b[1;2H", "a中\x1b[1;3H"] {
        let g = with_cursor(log.as_bytes(), C, R);
        assert_eq!((bg(at(&g, 0, 1)), bg(at(&g, 0, 2))), (DEFAULT_FG, DEFAULT_FG), "{log:?}");
        assert_eq!((bg(at(&g, 0, 0)), bg(at(&g, 0, 3))), (DEFAULT_BG, DEFAULT_BG), "{log:?}");
    }
    // On a one-column screen the wide character is narrow: one cell, no panic.
    let g = with_cursor("中".as_bytes(), 1, 1);
    assert_eq!(bg(&g[0]), DEFAULT_FG);
    // Wide at the end of a row: the block stays in the row.
    let g = with_cursor("\x1b[1;9H中\x1b[1;10H".as_bytes(), C, R);
    assert_eq!((bg(at(&g, 0, 8)), bg(at(&g, 0, 9)), bg(at(&g, 1, 0))), (DEFAULT_FG, DEFAULT_FG, DEFAULT_BG));
}

#[test]
fn the_cursor_keeps_concealed_text_hidden() {
    let g = with_cursor(b"\x1b[8mx\x1b[H", C, R);
    let x = at(&g, 0, 0);
    assert_eq!(fg(x), bg(x), "the text stays hidden");
    assert_ne!(bg(x), DEFAULT_BG, "the block still shows");
    // Concealed on a background the default foreground colour: still a block.
    let g = with_cursor(b"\x1b[8;48;2;219;231;247mx\x1b[H", C, R);
    let x = at(&g, 0, 0);
    assert_eq!((fg(x), bg(x)), (DEFAULT_BG, DEFAULT_BG));
}

#[test]
fn cursor_option_counts_from_0_as_tmux_does() {
    assert_eq!(parse_cursor("4,2", 10, 4), Ok(Some((2, 4))));
    assert_eq!(parse_cursor("0,0", 10, 4), Ok(Some((0, 0))));
    assert_eq!(parse_cursor("none", 10, 4), Ok(None));
    // tmux reports a pending wrap as one past the last column.
    assert_eq!(parse_cursor("10,3", 10, 4), Ok(Some((3, 9))));
    for bad in ["11,0", "0,4", "4", "4,", ",2", "-1,0", "4,2,1", "4;2", "", "None"] {
        assert!(parse_cursor(bad, 10, 4).is_err(), "{bad:?} accepted");
    }
    // The message gives the range that is accepted, pending wrap included.
    let message = parse_cursor("11,0", 10, 4).unwrap_err();
    assert!(message.contains("columns go from 0 to 10 (10 is a pending wrap"), "{message}");
    assert!(message.contains("rows from 0 to 3"), "{message}");
}

#[test]
fn ed2_clears_the_screen() {
    let g = grid(b"\x1b[3;3Hzz\x1b[2J");
    assert!((0..R).all(|r| line(&g, r) == "          "));
}

#[test]
fn el_erases_to_end_of_line_with_current_background() {
    let g = grid(b"abcdef\x1b[1;3H\x1b[48;2;1;2;3m\x1b[K");
    assert_eq!(line(&g, 0), "ab        ");
    assert_eq!(bg(at(&g, 0, 9)), (1, 2, 3));
    assert_eq!(bg(at(&g, 0, 1)), DEFAULT_BG);
}

#[test]
fn sgr_truecolor_bold_and_resets() {
    let g = grid(b"\x1b[1;38;2;10;20;30;48;2;40;50;60ma\x1b[22mb\x1b[39mc\x1b[49md\x1b[1me\x1b[mf");
    assert_eq!(fg(at(&g, 0, 0)), (10, 20, 30));
    assert_eq!(bg(at(&g, 0, 0)), (40, 50, 60));
    assert_eq!(at(&g, 0, 0).attrs & BOLD, BOLD);
    assert_eq!(at(&g, 0, 1).attrs & BOLD, 0);
    assert_eq!(fg(at(&g, 0, 2)), DEFAULT_FG);
    assert_eq!(bg(at(&g, 0, 3)), DEFAULT_BG);
    assert_eq!(at(&g, 0, 4).attrs & BOLD, BOLD);
    assert_eq!(at(&g, 0, 5).attrs & BOLD, 0);
}

#[test]
fn sgr_truncated_truecolor_is_ignored() {
    assert_eq!(fg(at(&grid(b"\x1b[38;2;1;2ma"), 0, 0)), DEFAULT_FG);
}

#[test]
fn osc_and_string_sequences_are_skipped() {
    let g = grid(b"\x1b]0;title\x07a\x1b]8;;http://x\x1b\\b\x1bPq#0\x1b\\c\x1b_G\x1b\\d");
    assert_eq!(line(&g, 0), "abcd      ");
}

#[test]
fn truncated_sequences_do_not_panic() {
    for s in [&b"\x1b"[..], b"\x1b[", b"\x1b[12;", b"\x1b]0;t", b"\x1bPq", b"\x1b]0;\x1b"] {
        assert_eq!(grid(s).len(), C * R);
    }
}

#[test]
fn pseudorandom_input_does_not_panic() {
    let pick = [
        0x1b, b'[', b']', b';', b':', b'0', b'2', b'5', b'9', b'3', b'8', b'H', b'A', b'B', b'C', b'D', b'K',
        b'J', b'm', b'?', b'(', b' ', 0x07, 0x18, b'\\', b'\n', b'\r', 0x7f, 0xc2, 0xe2, 0xed, 0xf4, 0x80,
        0x08, 0x09, 0x0e, 0x0f, b'7', b'@', b'P', b'X', b'b', b'g', b'I', b'Z', b'G', b'd', b'c', b'q',
        b'r', b'S', b'T', b'L', b'M', b'h', b'l', b'?', b'1', b'4', b'6',
    ];
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut step = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..20_000 {
        let len = step() % 128;
        let mut buf = Vec::new();
        for _ in 0..len {
            let r = step();
            buf.push(if r & 3 != 0 { pick[(r >> 8) as usize % pick.len()] } else { (r >> 16) as u8 });
        }
        let _ = parse(&buf, 7, 3);
    }
}

/// A WIDE cell has its TAIL to the right in the same row, and a TAIL has its
/// WIDE to the left; anything else is half a character.
fn wide_pairs_are_whole(cells: &[Cell], cols: usize) -> Result<(), String> {
    for (i, cell) in cells.iter().enumerate() {
        let col = i % cols;
        if cell.attrs & WIDE != 0 && (col + 1 == cols || cells[i + 1].attrs & TAIL == 0) {
            return Err(format!("WIDE without its TAIL at row {} col {col}", i / cols));
        }
        if cell.attrs & TAIL != 0 && (col == 0 || cells[i - 1].attrs & WIDE == 0) {
            return Err(format!("TAIL without its WIDE at row {} col {col}", i / cols));
        }
    }
    Ok(())
}

/// Random logs on random grid sizes, weighted toward the edges (one row or
/// column, and rarely the CLI's 500x200 limit), mixing wide and zero-width
/// characters with the sequences that move, wrap, insert, erase and scroll,
/// and with OSC and DCS strings. Checks that parse doesn't panic and
/// leaves no half of a wide character. Rounds and seed come from
/// TERMSHOT_FUZZ_ROUNDS and TERMSHOT_FUZZ_SEED; a failure prints both.
#[test]
fn fuzz_sizes_and_wide_characters() {
    let env = |name: &str, default: u64| std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
    let rounds = env("TERMSHOT_FUZZ_ROUNDS", 20_000);
    let seed = env("TERMSHOT_FUZZ_SEED", 0x2545_f491_4f6c_dd1d) | 1;
    let mut x = seed;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    // Wide (CJK, Hangul, fullwidth, emoji), zero-width (combining acute, Thai
    // vowel, ZWJ, VS16, skin tone), DEC graphics letters, and plain ASCII.
    let chars = ["界", "한", "Ｗ", "😀", "\u{301}", "\u{e31}", "\u{200d}", "\u{fe0f}", "🏽", "q", "x", " ", "é"];
    let finals = b"HfABCDEFGd`a@PXKJbrLMSTIZ";
    let modes = ["?7h", "?7l", "?6h", "?6l", "?1049h", "?1049l", "?47h", "?47l", "4h", "4l"];
    let escapes = ["\x1b7", "\x1b8", "\x1bD", "\x1bE", "\x1bM", "\x1bc", "\x1b#8", "\x1b(0", "\x1b(B", "\x0e", "\x0f"];
    // OSC and DCS ended by BEL or ST, with a wide character inside, and one
    // left open so what follows lands in the string.
    let strings = ["\x1b]0;界\x07", "\x1b]8;;http://x\x1b\\", "\x1bPq界\x1b\\", "\x1b]2;"];
    for round in 0..rounds {
        let (cols, rows) = match next() % 64 {
            // At or near the CLI's limits; rare, as each is 100,000 cells.
            0 => (500 - next() as usize % 2, 200 - next() as usize % 2),
            1 => (1 + next() as usize % 500, 1 + next() as usize % 200),
            r => match r % 4 {
                0 => (1, 1 + next() as usize % 4),
                1 => (1 + next() as usize % 4, 1),
                2 => (1 + next() as usize % 3, 1 + next() as usize % 3),
                _ => (1 + next() as usize % 40, 1 + next() as usize % 12),
            },
        };
        let mut log = Vec::new();
        for _ in 0..next() % 48 {
            let r = next();
            match r % 10 {
                0..=3 => log.extend_from_slice(chars[(r >> 8) as usize % chars.len()].as_bytes()),
                4 => log.push(b"\r\n\x08\t"[(r >> 8) as usize % 4]),
                5 if r & 0x100 == 0 => log.extend_from_slice(escapes[(r >> 9) as usize % escapes.len()].as_bytes()),
                5 => log.extend_from_slice(strings[(r >> 9) as usize % strings.len()].as_bytes()),
                6 => log.extend_from_slice(format!("\x1b[{}", modes[(r >> 8) as usize % modes.len()]).as_bytes()),
                7 => log.push([0x80, 0xff, 0xe7, 0xf0][(r >> 8) as usize % 4]),
                _ => {
                    // A CSI with up to two parameters near the grid's size, or huge.
                    let param = |v: u64| if v % 16 == 0 { 4_000_000_000 } else { v % (cols.max(rows) as u64 + 3) };
                    let final_byte = finals[(r >> 8) as usize % finals.len()] as char;
                    log.extend_from_slice(format!("\x1b[{};{}{final_byte}", param(r >> 16), param(r >> 40)).as_bytes());
                }
            }
        }
        let outcome = std::panic::catch_unwind(|| parse(&log, cols, rows));
        let mut failure = match &outcome {
            Err(_) => Some("panicked".to_string()),
            Ok(cells) if cells.len() != cols * rows => Some(format!("{} cells", cells.len())),
            Ok(cells) => wide_pairs_are_whole(cells, cols).err(),
        };
        // --lf-newline is exactly CR LF for every LF but a final bare one,
        // wherever the LF lands: at top level, inside a CSI, after an ESC,
        // or inside a string. The reference strips that LF by hand.
        if failure.is_none() {
            let text = if log.ends_with(b"\n") && !log.ends_with(b"\r\n") { &log[..log.len() - 1] } else { &log[..] };
            let crlf: Vec<u8> = text.iter().flat_map(|b| if *b == b'\n' { &b"\r\n"[..] } else { std::slice::from_ref(b) }).copied().collect();
            let outcome = std::panic::catch_unwind(|| (parse_lf(&log, cols, rows, Lf::Newline), parse(&crlf, cols, rows)));
            failure = match &outcome {
                Err(_) => Some("panicked with --lf-newline".to_string()),
                Ok((newline, crlf)) if !newline.iter().map(cell_key).eq(crlf.iter().map(cell_key)) => {
                    Some("--lf-newline differs from CR LF".to_string())
                }
                Ok(_) => None,
            };
        }
        if let Some(failure) = failure {
            panic!(
                "{failure} on round {round} (TERMSHOT_FUZZ_SEED={seed} TERMSHOT_FUZZ_ROUNDS={rounds}), \
                 {cols}x{rows}: b\"{}\"",
                log.escape_ascii()
            );
        }
    }
}

#[test]
fn charset_designation_is_not_printed() {
    assert_eq!(line(&grid(b"\x1b(B\x1b)0\x1b#8a"), 0), "a         ");
}

#[test]
fn el_modes() {
    assert_eq!(line(&grid(b"abcdef\x1b[1;4H\x1b[1K"), 0), "    ef    ");
    assert_eq!(line(&grid(b"abcdef\x1b[1;4H\x1b[2K"), 0), "          ");
}

#[test]
fn ed0_erases_below() {
    let g = grid(b"ab\r\ncd\x1b[1;2H\x1b[J");
    assert_eq!(line(&g, 0), "a         ");
    assert_eq!(line(&g, 1), "          ");
}

#[test]
fn ed2_keeps_the_cursor() {
    assert_eq!(line(&grid(b"abc\x1b[2Jx"), 0), "   x      ");
}

#[test]
fn cuf_stops_at_last_column() {
    assert_eq!(line(&grid(b"ab\x1b[99Cx"), 0), "ab       x");
}

#[test]
fn del_is_not_printed() {
    assert_eq!(line(&grid(b"a\x7fb"), 0), "ab        ");
}

#[test]
fn indexed_colour_does_not_set_bold() {
    assert_eq!(at(&grid(b"\x1b[38;5;1ma"), 0, 0).attrs & BOLD, 0);
}

#[test]
fn invalid_utf8_becomes_replacement_char() {
    assert_eq!(at(&grid(b"\xffA"), 0, 0).ch, 0xfffd);
    assert_eq!(at(&grid(b"\xe2A"), 0, 1).ch, 'A' as u32);
}

#[test]
fn utf8_rejects_overlongs_surrogates_and_truncation() {
    let replacements = |s: &[u8]| grid(s).iter().filter(|c| c.ch == 0xfffd).count();
    assert_eq!(replacements(b"\xc0\xaf"), 2);
    assert_eq!(replacements(b"\xed\xa0\x80"), 3);
    assert_eq!(replacements(b"\xf4\x90\x80\x80"), 4);
    assert_eq!(utf8_at(b"\xf0\x9f"), (0xfffd, 2));
}

#[test]
fn c1_controls_take_no_cell() {
    assert_eq!(line(&grid(b"a\xc2\x85b"), 0), "ab        ");
}

#[test]
fn lf_moves_down_without_carriage_return() {
    assert_eq!(line(&grid(b"ab\ncd"), 1), "  cd      ");
}

#[test]
fn cursor_counts_of_zero_mean_one() {
    assert_eq!(line(&grid(b"abc\x1b[0Dx"), 0), "abx       ");
}

#[test]
fn cup_clamps_to_the_grid() {
    assert_eq!(at(&grid(b"\x1b[99;99Hx"), R - 1, C - 1).ch, 'x' as u32);
}

#[test]
fn huge_parameters_saturate() {
    let g = grid(b"\x1b[4294967295;4294967295H\x1b[99999999999Ax\x1b[99999999999Cy\x1b[99999999999B\x1b[99999999999Dz");
    assert_eq!(at(&g, 0, C - 1).ch, 'y' as u32);
    assert_eq!(at(&g, R - 1, 0).ch, 'z' as u32);
}

#[test]
fn erase_uses_the_current_background() {
    let g = grid(b"\x1b[48;2;1;2;3m\x1b[2J");
    assert!(g.iter().all(|c| bg(c) == (1, 2, 3)));
}

#[test]
fn sgr_colon_truecolor() {
    let g = grid(b"\x1b[38:2::10:20:30ma\x1b[48:2:1:2:3mb");
    assert_eq!(fg(at(&g, 0, 0)), (10, 20, 30));
    assert_eq!(bg(at(&g, 0, 1)), (1, 2, 3));
}

#[test]
fn sgr_other_subparameters_are_skipped() {
    let g = grid(b"\x1b[1;4:3ma");
    assert_eq!(at(&g, 0, 0).attrs & BOLD, BOLD);
}

#[test]
fn sgr_component_over_255_is_ignored() {
    assert_eq!(fg(at(&grid(b"\x1b[38;2;300;0;0ma"), 0, 0)), DEFAULT_FG);
}

#[test]
fn esc_aborts_a_string_sequence() {
    let g = grid(b"\x1b]0;title\x1b[1ma\x1b]0;t\x18b");
    assert_eq!(line(&g, 0), "ab        ");
    assert_eq!(at(&g, 0, 0).attrs & BOLD, BOLD);
}

#[test]
fn esc_aborts_a_csi() {
    assert_eq!(at(&grid(b"\x1b[1\x1b[2;3Hx"), 1, 2).ch, 'x' as u32);
}

#[test]
fn esc_esc_starts_over() {
    assert_eq!(at(&grid(b"\x1b\x1b[1;2Hx"), 0, 1).ch, 'x' as u32);
}

#[test]
fn c0_inside_csi_executes() {
    assert_eq!(line(&grid(b"abc\x1b[\r2Cx"), 0), "abx       ");
}

#[test]
fn csi_with_intermediate_is_ignored() {
    assert_eq!(line(&grid(b"\x1b[2 qa"), 0), "a         ");
}

#[test]
fn tab_moves_to_the_next_stop_or_the_last_column() {
    assert_eq!(line(&grid(b"a\tb"), 0), "a       b ");
    assert_eq!(at(&grid(b"\x1b[1;10H\tx"), 0, 9).ch, 'x' as u32);
}

#[test]
fn backspace() {
    assert_eq!(line(&grid(b"abc\x08\x08x"), 0), "axc       ");
    assert_eq!(line(&grid(b"\x08x"), 0), "x         ");
    // After printing in the last column the cursor sits there, so BS lands one left.
    assert_eq!(line(&grid(b"\x1b[1;9Hab\x08c"), 0), "        cb");
}

#[test]
fn tab_stops_can_be_set_and_cleared() {
    assert_eq!(at(&grid(b"\x1b[3g\x1b[1;4H\x1bH\r\tx"), 0, 3).ch, 'x' as u32);
    assert_eq!(at(&grid(b"\x1b[1;9H\x1b[g\r\tx"), 0, 9).ch, 'x' as u32);
}

#[test]
fn tab_forward_and_back_by_count() {
    assert_eq!(at(&grid(b"\x1b[2Ix"), 0, 9).ch, 'x' as u32);
    assert_eq!(at(&grid(b"\x1b[1;10H\x1b[Zx"), 0, 8).ch, 'x' as u32);
}

#[test]
fn absolute_column_and_row() {
    let g = grid(b"\x1b[5Gx\x1b[3`y\x1b[3dz\x1b[99G!");
    assert_eq!(at(&g, 0, 4).ch, 'x' as u32);
    assert_eq!(at(&g, 0, 2).ch, 'y' as u32);
    assert_eq!(at(&g, 2, 3).ch, 'z' as u32);
    assert_eq!(at(&g, 2, 9).ch, '!' as u32);
}

#[test]
fn next_and_previous_line() {
    let g = grid(b"ab\x1b[2Ex\x1b[1Fy");
    assert_eq!(at(&g, 2, 0).ch, 'x' as u32);
    assert_eq!(at(&g, 1, 0).ch, 'y' as u32);
}

#[test]
fn relative_column_and_row() {
    let g = grid(b"\x1b[3ax\x1b[2ey");
    assert_eq!(at(&g, 0, 3).ch, 'x' as u32);
    assert_eq!(at(&g, 2, 4).ch, 'y' as u32);
}

#[test]
fn insert_chars_shift_right_in_the_current_background() {
    let g = grid(b"abcdef\x1b[1;3H\x1b[48;2;1;2;3m\x1b[2@");
    assert_eq!(line(&g, 0), "ab  cdef  ");
    assert_eq!(bg(at(&g, 0, 2)), (1, 2, 3));
    assert_eq!(line(&grid(b"abcdefghij\x1b[1;9H\x1b[99@"), 0), "abcdefgh  ");
}

#[test]
fn delete_chars_shift_left() {
    assert_eq!(line(&grid(b"abcdef\x1b[1;2H\x1b[2P"), 0), "adef      ");
    assert_eq!(line(&grid(b"abcdef\x1b[1;2H\x1b[99P"), 0), "a         ");
}

#[test]
fn erase_chars_keeps_the_cursor() {
    assert_eq!(line(&grid(b"abcdef\x1b[1;2H\x1b[2Xz"), 0), "az def    ");
}

#[test]
fn repeat_the_last_character() {
    assert_eq!(line(&grid(b"ab\x1b[3b"), 0), "abbbb     ");
    // A huge count is capped instead of looping billions of times.
    assert_eq!(line(&grid(b"x\x1b[4000000000b"), 0), "xxxxxxxxxx");
    assert_eq!(line(&grid(b"\x1b[3b"), 0), "          ");
}

#[test]
fn save_and_restore_keep_attributes() {
    for (save, restore) in [(&b"\x1b7"[..], &b"\x1b8"[..]), (b"\x1b[s", b"\x1b[u")] {
        let mut log = b"\x1b[38;2;1;2;3m\x1b[2;2H".to_vec();
        log.extend_from_slice(save);
        log.extend_from_slice(b"\x1b[0m\x1b[4;4H");
        log.extend_from_slice(restore);
        log.push(b'z');
        let g = grid(&log);
        assert_eq!(at(&g, 1, 1).ch, 'z' as u32);
        assert_eq!(fg(at(&g, 1, 1)), (1, 2, 3));
    }
}

#[test]
fn dec_special_graphics() {
    assert_eq!(line(&grid(b"\x1b(0lqk\x1b(Bq"), 0), "┌─┐q      ");
    // G1 through SO and SI.
    assert_eq!(line(&grid(b"\x1b)0\x0eq\x0fq"), 0), "─q        ");
    // DECSC saves the character sets.
    assert_eq!(line(&grid(b"\x1b(0\x1b7\x1b(B\x1b8q"), 0), "─         ");
    // Other sets (UK, ESC ( A) draw as ASCII; extra intermediates designate nothing.
    assert_eq!(line(&grid(b"\x1b(0\x1b(Aq\x1b(0\x1b($Bq"), 0), "q─        ");
}

#[test]
fn index_next_line_and_reverse_index() {
    assert_eq!(at(&grid(b"ab\x1bDc"), 1, 2).ch, 'c' as u32);
    assert_eq!(at(&grid(b"ab\x1bEc"), 1, 0).ch, 'c' as u32);
    assert_eq!(at(&grid(b"\x1b[3;1H\x1bMx"), 1, 0).ch, 'x' as u32);
}

#[test]
fn full_reset() {
    let g = grid(b"abc\x1b[2;2H\x1b[38;2;1;2;3m\x1b(0\x1bcxq");
    assert_eq!(line(&g, 0), "xq        ");
    assert_eq!(fg(at(&g, 0, 0)), DEFAULT_FG);
}

#[test]
fn vt_and_ff_move_down() {
    let g = grid(b"a\x0bb\x0cc");
    assert_eq!(at(&g, 1, 1).ch, 'b' as u32);
    assert_eq!(at(&g, 2, 2).ch, 'c' as u32);
}

#[test]
fn sgr_16_colours() {
    let g = grid(b"\x1b[31ma\x1b[42mb\x1b[91mc\x1b[103md\x1b[39;49me");
    assert_eq!(fg(at(&g, 0, 0)), (205, 0, 0));
    assert_eq!(bg(at(&g, 0, 1)), (0, 205, 0));
    assert_eq!(fg(at(&g, 0, 2)), (255, 0, 0));
    assert_eq!(bg(at(&g, 0, 3)), (255, 255, 0));
    assert_eq!((fg(at(&g, 0, 4)), bg(at(&g, 0, 4))), (DEFAULT_FG, DEFAULT_BG));
}

#[test]
fn sgr_256_colours() {
    let g = grid(b"\x1b[38;5;196ma\x1b[48;5;232mb\x1b[38;5;21mc\x1b[38:5:46md\x1b[38;5;300me");
    assert_eq!(fg(at(&g, 0, 0)), (255, 0, 0)); // cube 5,0,0
    assert_eq!(bg(at(&g, 0, 1)), (8, 8, 8)); // first grey
    assert_eq!(fg(at(&g, 0, 2)), (0, 0, 255)); // cube 0,0,5
    assert_eq!(fg(at(&g, 0, 3)), (0, 255, 0)); // colon form
    assert_eq!(fg(at(&g, 0, 4)), (0, 255, 0)); // 300 is not a colour: unchanged
}

#[test]
fn sgr_reverse_swaps_the_cell_colours() {
    let g = grid(b"\x1b[7ma\x1b[27mb\x1b[31;44;7mc");
    assert_eq!((fg(at(&g, 0, 0)), bg(at(&g, 0, 0))), (DEFAULT_BG, DEFAULT_FG));
    assert_eq!((fg(at(&g, 0, 1)), bg(at(&g, 0, 1))), (DEFAULT_FG, DEFAULT_BG));
    assert_eq!((fg(at(&g, 0, 2)), bg(at(&g, 0, 2))), ((0, 0, 238), (205, 0, 0)));
}

#[test]
fn sgr_dim_and_conceal() {
    let g = grid(b"\x1b[2ma\x1b[22mb\x1b[8mc\x1b[28md");
    // Two thirds of the way from the background to the text colour.
    assert_eq!(fg(at(&g, 0, 0)), (151, 162, 176));
    assert_eq!(fg(at(&g, 0, 1)), DEFAULT_FG);
    assert_eq!(fg(at(&g, 0, 2)), DEFAULT_BG);
    assert_eq!(fg(at(&g, 0, 3)), DEFAULT_FG);
}

#[test]
fn sgr_underline_and_strike() {
    let g = grid(b"\x1b[4ma\x1b[21mb\x1b[4:3mc\x1b[4:0md\x1b[9me\x1b[24;29mf\x1b[1;4;9mg\x1b[22mh");
    let attrs: Vec<u8> = (0..8).map(|c| at(&g, 0, c).attrs).collect();
    assert_eq!(
        attrs,
        [UNDERLINE, DOUBLE_UNDERLINE, UNDERLINE, 0, STRIKE, 0, BOLD | UNDERLINE | STRIKE, UNDERLINE | STRIKE]
    );
}

#[test]
fn erased_cells_take_colours_but_not_attributes() {
    let g = grid(b"\x1b[4;9;7;48;2;1;2;3m\x1b[2J");
    assert!(g.iter().all(|c| c.attrs == 0 && bg(c) == (1, 2, 3)));
}

/// Not a test: writes the cell grids bench/c-vs-rust/run.sh paints, parsed
/// by termshot's own parser, into $TERMSHOT_POC_DIR. See docs/c-vs-rust.md.
#[test]
#[ignore]
fn poc_workloads() {
    let Some(dir) = std::env::var_os("TERMSHOT_POC_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    let dump = |name: &str, log: &[u8], cols: usize, rows: usize, px: f64| {
        let cells = parse(log, cols, rows);
        let bytes: Vec<u8> = cells
            .iter()
            .flat_map(|c| {
                let mut b = c.ch.to_ne_bytes().to_vec();
                b.extend_from_slice(&[c.fr, c.fg, c.fb, c.br, c.bg, c.bb, c.attrs, 0]);
                b
            })
            .collect();
        fs::write(dir.join(format!("{name}.cells")), bytes).unwrap();
        fs::write(dir.join(format!("{name}.meta")), format!("{cols} {rows} {px}\n")).unwrap();
    };
    let reply = fs::read("examples/reply-sent.pty").unwrap();
    dump("1-reply-px48", &reply, 100, 30, 48.0);
    dump("2-reply-px128", &reply, 100, 30, 128.0);
    // 16 and 256 colours and every attribute.
    let mut attrs = Vec::new();
    for (i, n) in (0..16).enumerate() {
        attrs.extend_from_slice(format!("\x1b[38;5;{n}m{i:>3}").as_bytes());
    }
    attrs.extend_from_slice(b"\x1b[0m\r\n");
    for n in 16..112 {
        attrs.extend_from_slice(format!("\x1b[48;5;{n}m ").as_bytes());
    }
    attrs.extend_from_slice(b"\x1b[0m\r\nplain \x1b[1mbold\x1b[0m \x1b[2mdim\x1b[0m \x1b[4munderline\x1b[0m ");
    attrs.extend_from_slice(b"\x1b[21mdouble\x1b[0m \x1b[9mstrike\x1b[0m \x1b[7mreverse\x1b[0m \x1b[1;4;9;31mall\x1b[0m");
    dump("3-attrs-px24", &attrs, 96, 6, 24.0);
    // Rounded boxes and lines: geometry, arcs included.
    let ten = |left: &str, fill: &str, right: &str| format!("{left}{}{right}", fill.repeat(8)).repeat(10);
    let one_row = [ten("╭", "─", "╮"), ten("│", " ", "│"), ten("╰", "─", "╯")].join("\r\n");
    let boxes = vec![one_row; 10].join("\r\n");
    dump("4-boxes-px48", boxes.as_bytes(), 100, 30, 48.0);
    // Dense text in many colours.
    let mut dense = Vec::new();
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for r in 0..60 {
        dense.extend_from_slice(format!("\x1b[{};1H", r + 1).as_bytes());
        for _ in 0..200 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            dense.extend_from_slice(format!("\x1b[38;2;{};{};{}m", x & 255, (x >> 8) & 255, (x >> 16) & 255).as_bytes());
            dense.push(b'!' + (x >> 24) as u8 % 94);
        }
    }
    dump("5-dense-200x60-px16", &dense, 200, 60, 16.0);
}

/// Decode a printf(1) format string the way tests/vt/oracle.sh feeds it to
/// printf: backslash escapes, octal \NNN (1-3 digits) and %%.
fn printf_bytes(format: &str) -> Vec<u8> {
    let b = format.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1).copied()) {
            (b'\\', Some(c @ b'0'..=b'7')) => {
                let mut value = u32::from(c - b'0');
                let mut j = i + 2;
                while j < b.len() && j < i + 4 && (b'0'..=b'7').contains(&b[j]) {
                    value = value * 8 + u32::from(b[j] - b'0');
                    j += 1;
                }
                out.push(value as u8);
                i = j;
                continue;
            }
            (b'\\', Some(c)) => out.push(match c {
                b'a' => 7,
                b'b' => 8,
                b'f' => 12,
                b'n' => 10,
                b'r' => 13,
                b't' => 9,
                b'v' => 11,
                other => other,
            }),
            (b'%', Some(b'%')) => out.push(b'%'),
            (c, _) => {
                out.push(c);
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    out
}

/// A screen as tests/vt/ writes it: each row with trailing spaces trimmed,
/// then where the cursor is.
fn screen_lines(g: &Grid, cols: usize, rows: usize) -> Vec<String> {
    let mut lines: Vec<String> =
        (0..rows).map(|r| shown(&g.cells[r * cols..(r + 1) * cols]).trim_end().to_string()).collect();
    lines.push(match g.cursor {
        Some((row, col)) => format!("cursor {col},{row}"),
        None => "cursor hidden".into(),
    });
    lines
}

/// Every case in tests/vt/cases.txt renders the screen and cursor in
/// expected.txt, which tmux produced (tests/vt/oracle.sh) except for the
/// documented deviations.
#[test]
fn vt_cases_match_the_reference_screens() {
    let cases = fs::read_to_string("tests/vt/cases.txt").unwrap();
    let expected = fs::read_to_string("tests/vt/expected.txt").unwrap();
    let mut failures = Vec::new();
    let mut count = 0;
    for line in cases.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
        let mut fields = line.split_whitespace();
        let (name, size) = (fields.next().unwrap(), fields.next().unwrap());
        let input = line[line.find(size).unwrap() + size.len()..].trim_start();
        let (cols, rows) = size.split_once('x').unwrap();
        let (cols, rows): (usize, usize) = (cols.parse().unwrap(), rows.parse().unwrap());
        let got = screen_lines(&replay(&printf_bytes(input), cols, rows, Lf::Index), cols, rows);
        let header = format!("== {name}");
        let want: Vec<&str> = expected
            .lines()
            .skip_while(|l| *l != header)
            .skip(1)
            .take_while(|l| !l.starts_with("== "))
            .collect();
        count += 1;
        if want.len() != rows + 1 || got.iter().zip(&want).any(|(g, w)| g != w) {
            failures.push(format!("{name}:\n  got  {got:?}\n  want {want:?}"));
        }
    }
    assert!(count > 30, "only {count} cases read");
    assert!(failures.is_empty(), "{} of {count} cases differ:\n{}", failures.len(), failures.join("\n"));
}

/// Real sessions recorded from tmux (tests/vt/record.sh): termshot renders
/// each log to the screen tmux showed at the end, with the cursor there.
#[test]
fn real_sessions_match_tmux() {
    let (cols, rows) = (80, 24);
    for name in ["shell", "less", "vi"] {
        let log = fs::read(format!("tests/vt/real/{name}.log")).unwrap();
        let want = fs::read_to_string(format!("tests/vt/real/{name}.txt")).unwrap();
        let got: String =
            screen_lines(&replay(&log, cols, rows, Lf::Index), cols, rows).iter().map(|l| l.clone() + "\n").collect();
        assert!(got == want, "{name}: termshot shows\n{got}\ntmux showed\n{want}");
    }
}

#[test]
fn widths() {
    let cases = [
        ('a', 1), ('é', 1), ('\u{00AD}', 1), ('中', 2), ('한', 2), ('Ａ', 2), ('\u{3000}', 2),
        ('😀', 2), ('\u{1F1F9}', 2), ('\u{0301}', 0), ('\u{302A}', 0), ('\u{200B}', 0),
        ('\u{200D}', 0), ('\u{FE0F}', 0), ('\u{1160}', 0), ('─', 1), ('█', 1), ('\u{2028}', 1),
    ];
    for (ch, want) in cases {
        assert_eq!(unicode::width(ch as u32), want, "U+{:04X}", ch as u32);
    }
}

#[test]
fn wide_characters_take_two_cells() {
    let g = grid("a中b".as_bytes());
    assert_eq!(shown(&g[..C]), "a中b      ");
    assert_eq!(at(&g, 0, 1).attrs & WIDE, WIDE);
    assert_eq!((at(&g, 0, 2).ch, at(&g, 0, 2).attrs & TAIL), (0, TAIL));
    assert_eq!(at(&g, 0, 3).ch, 'b' as u32);
}

#[test]
fn wide_character_wraps_instead_of_splitting() {
    let g = grid("\x1b[1;10H中x".as_bytes());
    assert_eq!(line(&g, 0), "          ");
    assert_eq!(shown(&g[C..2 * C]), "中x       ");
    // Without autowrap there is nowhere to put it.
    assert_eq!(line(&grid("\x1b[?7l\x1b[1;10H中".as_bytes()), 0), "          ");
    // Filling the last two columns leaves a wrap pending, like any character.
    let g = grid("\x1b[1;9H中x".as_bytes());
    assert_eq!(shown(&g[..C]), "        中");
    assert_eq!(at(&g, 1, 0).ch, 'x' as u32);
}

#[test]
fn overwriting_half_a_wide_character_blanks_the_other_half() {
    assert_eq!(shown(&grid("中\x1b[1;2Hx".as_bytes())[..C]), " x        ");
    assert_eq!(shown(&grid("中\x1b[1;1Hx".as_bytes())[..C]), "x         ");
    assert_eq!(shown(&grid("中中\x1b[1;2H文".as_bytes())[..C]), " 文       ");
    // The ASCII fast path, too.
    assert_eq!(shown(&grid("中中\x1b[1;2Hab".as_bytes())[..C]), " ab       ");
    assert!(grid("中中\x1b[1;2Hab".as_bytes()).iter().all(|c| c.attrs & (WIDE | TAIL) == 0));
}

#[test]
fn edits_that_cut_a_wide_character_remove_all_of_it() {
    // EL from the tail, ECH on the lead, DCH of the lead, ICH pushing a tail off the line.
    assert_eq!(shown(&grid("中文\x1b[1;2H\x1b[K".as_bytes())[..C]), "          ");
    assert_eq!(shown(&grid("中x\x1b[1;1H\x1b[X".as_bytes())[..C]), "  x       ");
    assert_eq!(shown(&grid("中x\x1b[1;1H\x1b[P".as_bytes())[..C]), " x        ");
    assert_eq!(shown(&grid("aaaaaaaa中\x1b[1;1H\x1b[@".as_bytes())[..C]), " aaaaaaaa ");
    for log in ["中文\x1b[1;2H\x1b[K", "中x\x1b[1;1H\x1b[X", "中x\x1b[1;1H\x1b[P", "aaaaaaaa中\x1b[1;1H\x1b[@"] {
        let g = grid(log.as_bytes());
        for c in 0..C {
            let attrs = at(&g, 0, c).attrs;
            assert!(attrs & WIDE == 0 || (c + 1 < C && at(&g, 0, c + 1).attrs & TAIL != 0), "{log:?}: orphan lead");
            assert!(attrs & TAIL == 0 || (c > 0 && at(&g, 0, c - 1).attrs & WIDE != 0), "{log:?}: orphan tail");
        }
    }
}

#[test]
fn combining_marks_compose_or_are_dropped() {
    let g = grid("e\u{0301}a\u{0308}x\u{0301}y".as_bytes());
    assert_eq!(line(&g, 0), "éäxy      ");
    // Zero-width joiners and variation selectors take no cell.
    assert_eq!(line(&grid("a\u{200D}b\u{FE0F}c".as_bytes()), 0), "abc       ");
    // A mark with nothing before it is dropped.
    assert_eq!(line(&grid("\u{0301}z".as_bytes()), 0), "z         ");
}

#[test]
fn rep_and_attributes_cover_both_halves() {
    let g = grid("\x1b[4m中\x1b[2b".as_bytes());
    assert_eq!(shown(&g[..C]), "中中中    ");
    assert!((0..6).all(|c| at(&g, 0, c).attrs & UNDERLINE != 0));
}

#[test]
fn printf_decoding() {
    assert_eq!(printf_bytes(r"a\tb\033[1m\0337\\%%\n"), b"a\tb\x1b[1m\x1b7\\%\n");
}
