//! Parser unit tests. No crates, no FFI calls; ./test.sh builds and runs them.
//! A test for a known bug states the correct behaviour, is marked
//! #[ignore = "#N: ..."] with its issue, and should pass once it is fixed.
//! There are none open now. poc_workloads is ignored for another reason: it is
//! a benchmark helper, not a test.

use super::*;
use crate::cell::{BOLD, DOUBLE_UNDERLINE, ITALIC, STRIKE, UNDERLINE};
use crate::grid::{grid_json, grid_text, marks_of};
use crate::screen::Screen;
use crate::vt::{parse, parse_lf, printable_end, replay, replay_sized, utf8_at};

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
                            // As SGR would: printing takes the pen's cell.
                            s.pen_cell = s.pen.cell();
                            // Keep an image across both scroll margins so the
                            // optimized text path must preserve its outside parts.
                            s.graphics.command(format!("a=T,f=24,s=1,v=1,c={rows},r={rows},C=1;/wAA").as_bytes(),
                                               0, 0, s.cell_size, rows);
                            s
                        };
                        let bytes: Vec<u8> = (0..len).map(|i| b'!' + (i % 94) as u8).collect();
                        let (mut fast, mut reference) = (setup(), setup());
                        fast.print_ascii(&bytes);
                        for byte in bytes { reference.print(u32::from(byte)); }
                        assert_eq!((fast.row, fast.col, fast.pending, fast.last, fast.last_at),
                                   (reference.row, reference.col, reference.pending, reference.last, reference.last_at));
                        assert_eq!(fast.graphics.placements, reference.graphics.placements,
                                   "cols={cols} rows={rows} wrap={wrap} alternate={alternate} len={len}");
                        // The run is printed in the pen's style, not the default one.
                        let want = fast.pen.cell();
                        let styled = |s: &Screen| s.cells.iter().any(|c| {
                            c.ch != ' ' as u32 && (fg(c), bg(c), c.attrs & !(WIDE | TAIL)) == (fg(&want), bg(&want), want.attrs)
                        });
                        assert_eq!(styled(&fast), len > 0, "cols={cols} rows={rows} wrap={wrap} len={len}");
                        assert_eq!(styled(&reference), len > 0);
                        fast.combine(0x0301);
                        reference.combine(0x0301);
                        assert_eq!(fast.screen_marks(), reference.screen_marks());
                        for (a, b) in fast.into_cells().unwrap().iter().zip(reference.into_cells().unwrap()) {
                            assert_eq!((a.ch, fg(a), bg(a), a.attrs), (b.ch, fg(&b), bg(&b), b.attrs));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn row_rotation_uses_logical_scroll_distance_for_images() {
    for up in [false, true] {
        for n in [0, 1, 2, 3, 6, 7, usize::MAX] {
            let setup = || {
                let mut s = Screen::new(5, 5, Lf::Index);
                s.graphics.command(b"a=T,f=24,s=1,v=1,c=3,r=3,C=1;/wAA", 0, 1, s.cell_size, 5);
                s
            };
            let (mut fast, mut reference) = (setup(), setup());
            fast.rotate_rows(1, 3, n, up);
            // Exhausting the region removes the contained placement.
            // Rotating by a multiple of 3 must still clip.
            for _ in 0..n.min(3) {
                if up { reference.scroll_up(1, 3, 1); }
                else { reference.scroll_down(1, 3, 1); }
            }
            assert_eq!(fast.graphics.placements, reference.graphics.placements, "n={n} up={up}");
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
    let Grid { mut cells, cursor, .. } = replay(s, cols, rows, Lf::Index);
    if let Some((row, col)) = cursor {
        draw_cursor(&mut cells, cols, row, col, &Palette::DEFAULT);
    }
    cells
}

#[test]
fn the_cursor_is_a_block_in_reverse_video() {
    // On a blank cell: the default colours swapped.
    let g = with_cursor(b"ab", C, R);
    assert_eq!((fg(at(&g, 0, 2)), bg(at(&g, 0, 2))), (DEFAULT_BG, DEFAULT_FG));
    assert_eq!(bg(at(&g, 0, 1)), DEFAULT_BG);
    // On a character: its colours swapped, the character and attributes kept,
    // and its background opaque, as kitty draws the block over images below
    // the cell backgrounds.
    let g = with_cursor(b"\x1b[1;4;31;42ma\x1b[m\x1b[H", C, R);
    let a = at(&g, 0, 0);
    assert_eq!((a.ch, a.attrs), ('a' as u32, BOLD | UNDERLINE | OPAQUE));
    assert_eq!(at(&g, 0, 1).attrs & OPAQUE, 0);
    assert_eq!((fg(a), bg(a)), (Palette::DEFAULT.named[2], Palette::DEFAULT.named[1]));
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
fn decscusr_sets_the_cursor_shape() {
    let shape = |log: &str| replay(log.as_bytes(), C, R, Lf::Index).cursor_shape;
    assert_eq!(shape("ab"), CursorShape::Block);
    for (ps, want) in [
        ("", CursorShape::Block),
        ("0", CursorShape::Block),
        ("1", CursorShape::Block),
        ("2", CursorShape::Block),
        ("3", CursorShape::Underline),
        ("4", CursorShape::Underline),
        ("5", CursorShape::Bar),
        ("6", CursorShape::Bar),
    ] {
        // Over another shape, so each value is seen to set one.
        let before = if want == CursorShape::Bar { "\x1b[3 q" } else { "\x1b[5 q" };
        assert_eq!(shape(&format!("{before}\x1b[{ps} q")), want, "Ps {ps:?}");
    }
    // Unknown values and other sequences change nothing.
    for log in ["\x1b[5 q\x1b[7 q", "\x1b[5 q\x1b[99999 q", "\x1b[5 q\x1b[?2 q", "\x1b[5 q\x1b[2 !q", "\x1b[5 q\x1b[2 p"] {
        assert_eq!(shape(log), CursorShape::Bar, "{log:?}");
    }
    // Parameter bytes after the intermediate make the sequence malformed.
    for log in ["\x1b[5 q\x1b[0 2q", "\x1b[5 q\x1b[ ;2q", "\x1b[5 q\x1b[ ?2q", "\x1b[5 q\x1b[ 2q"] {
        assert_eq!(shape(log), CursorShape::Bar, "{log:?}");
    }
    // Only the first parameter counts; RIS resets it, DECSTR keeps it.
    assert_eq!(shape("\x1b[5;2 q"), CursorShape::Bar);
    assert_eq!(shape("\x1b[5 q\x1bc"), CursorShape::Block);
    assert_eq!(shape("\x1b[5 q\x1b[!p"), CursorShape::Bar);
    // A sequence with an intermediate does nothing else: no SGR, no cursor move.
    let g = replay(b"\x1b[1 mab\x1b[1 H", C, R, Lf::Index);
    assert_eq!((g.cells[0].attrs & BOLD, g.cursor), (0, Some((0, 2))));
}

#[test]
fn cursor_shape_option() {
    for shape in CursorShape::ALL {
        assert_eq!(CursorShape::parse(shape.name()), Ok(shape));
    }
    for bad in ["", "Block", "beam", "bar ", "5"] {
        let message = CursorShape::parse(bad).unwrap_err();
        assert!(message.contains("block, underline or bar"), "{message}");
    }
}

/// The underline or bar cursor's rectangle and colour, for 16x32 cells.
fn mark(log: &str, shape: CursorShape) -> ((i64, i64, i64, i64), [u8; 4]) {
    let g = replay(log.as_bytes(), C, R, Lf::Index);
    cursor_mark(&g.cells, C, g.cursor.unwrap(), shape, (16, 32), &Palette::DEFAULT)
}

#[test]
fn underline_and_bar_cursors_cover_the_cell() {
    let (r, g, b) = DEFAULT_FG;
    let fg = [r, g, b, 255];
    // At column 2, row 1: an eighth of the cell's width thick.
    assert_eq!(mark("\r\nab", CursorShape::Underline), ((32, 62, 16, 2), fg));
    assert_eq!(mark("\r\nab", CursorShape::Bar), ((32, 32, 2, 32), fg));
    // Under a wide character, from either half: both cells; the bar on the first.
    for log in ["a中\x1b[1;2H", "a中\x1b[1;3H"] {
        assert_eq!(mark(log, CursorShape::Underline).0, (16, 30, 32, 2), "{log:?}");
        assert_eq!(mark(log, CursorShape::Bar).0, (16, 0, 2, 32), "{log:?}");
    }
    // A wide character the last column cuts: one cell.
    let g = replay("\x1b[1;10H中".as_bytes(), 10, 1, Lf::Index);
    let cut = cursor_mark(&g.cells, 10, (0, 9), CursorShape::Underline, (16, 32), &Palette::DEFAULT);
    assert_eq!(cut.0, (144, 30, 16, 2));
    // Never thinner than a pixel.
    let g = replay(b"a", C, R, Lf::Index);
    assert_eq!(cursor_mark(&g.cells, C, (0, 1), CursorShape::Bar, (7, 14), &Palette::DEFAULT).0, (7, 0, 1, 14));
    // On a background the default foreground colour, the default background.
    let (r, g, b) = DEFAULT_BG;
    assert_eq!(mark("\x1b[48;2;219;231;247mx\x1b[H", CursorShape::Bar).1, [r, g, b, 255]);
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
fn json_has_runs_of_alike_cells_and_the_cursor() {
    let log = "\x1b[1;31mab\x1b[m c\x1b[4m \x1b[m\r\n中\x1b[32mx\x1b[44m  \x1b[m\r\n\"\\";
    let g = replay(log.as_bytes(), 8, 4, Lf::Index);
    let want = r##"{"cols":8,"rows":4,"cursor":{"col":2,"row":2,"shape":"block"},"lines":[
[{"col":0,"text":"ab","fg":"#cd0000","bg":"#111823","bold":true},{"col":2,"text":" c","fg":"#dbe7f7","bg":"#111823"},{"col":4,"text":" ","fg":"#dbe7f7","bg":"#111823","underline":true}],
[{"col":0,"text":"中","fg":"#dbe7f7","bg":"#111823"},{"col":2,"text":"x","fg":"#00cd00","bg":"#111823"},{"col":3,"text":"  ","fg":"#00cd00","bg":"#0000ee"}],
[{"col":0,"text":"\"\\","fg":"#dbe7f7","bg":"#111823"}],
[]
]}
"##;
    assert_eq!(grid_json(&g.cells, &g.marks, 8, 4, g.cursor, g.cursor_shape, DEFAULT_BG), want);
    let bar = grid_json(&g.cells, &g.marks, 8, 4, g.cursor, CursorShape::Bar, DEFAULT_BG);
    assert!(bar.starts_with(r#"{"cols":8,"rows":4,"cursor":{"col":2,"row":2,"shape":"bar"},"#), "{bar}");
}

#[test]
fn json_escapes_controls_and_reports_a_hidden_cursor() {
    let mut cells = parse(b"\x1b[9;21;3mab", 3, 1);
    cells[1].ch = 0x1b;
    let want = "{\"cols\":3,\"rows\":1,\"cursor\":null,\"lines\":[\n\
        [{\"col\":0,\"text\":\"a\\u001b\",\"fg\":\"#dbe7f7\",\"bg\":\"#111823\",\"italic\":true,\"double_underline\":true,\"strike\":true}]\n]}\n";
    assert_eq!(grid_json(&cells, &[], 3, 1, None, CursorShape::Bar, DEFAULT_BG), want);
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

/// A Sixel image is drawn now, but its data never reaches the text: a
/// pixel-tall image keeps the cursor on its row and column, so `c` lands
/// where the image is (and clears its pixel, as in xterm), and DCS strings
/// that are not Sixel are skipped.
#[test]
fn sixel_and_other_dcs_strings_leave_no_text() {
    let log = b"ab\x1bP0;1q\"1;1;1;1#1;2;100;0;0@\x1b\\c\x1bP+q4d73\x1b\\\x1bP$q\"p\x1b\\d";
    let g = replay(log, C, R, Lf::Index);
    assert_eq!(line(&g.cells, 0), "abcd      ");
    assert_eq!(g.images.len(), 1);
    assert_eq!(*g.images[0].pixels, [0, 0, 0, 0]);
    let g = replay(b"ab\x1bP0;1q\"1;1;1;1#1;2;100;0;0@\x1b\\\r\n", C, R, Lf::Index);
    assert_eq!(*g.images[0].pixels, [255, 0, 0, 255]);
}

/// Which 10x6 cells of a red 30x18 Sixel image at the top left still have
/// pixels, after `then`, a row of three per text row. Cells are 10x6 pixels,
/// so the image covers 3x3 cells.
fn sixel_cells_left(then: &[u8]) -> [[bool; 3]; 3] {
    let mut log = b"\x1bP0;1q#1;2;100;0;0!30~-!30~-!30~\x1b\\\x1b[H".to_vec();
    log.extend_from_slice(then);
    let g = replay_sized(&log, 6, 4, Lf::Index, (10, 6));
    let sixel: Vec<_> = g.images.iter().filter(|p| p.sixel).collect();
    assert_eq!(sixel.len(), 1, "{}", String::from_utf8_lossy(then));
    let pixels = &sixel[0].pixels;
    let mut left = [[false; 3]; 3];
    for (r, row) in left.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            let alpha = |x: usize, y: usize| pixels[(y * 30 + x) * 4 + 3];
            let set: Vec<bool> = (0..6).flat_map(|y| (0..10).map(move |x| (x, y))).map(|(x, y)| alpha(c * 10 + x, r * 6 + y) != 0).collect();
            // A cell is cleared whole or not at all.
            assert!(set.iter().all(|&s| s == set[0]), "cell {r},{c} after {}", String::from_utf8_lossy(then));
            *cell = set[0];
        }
    }
    left
}

/// xterm keeps Sixel pixels with the cells (graphics.c, erase_graphic): a
/// cell written later clears the pixels under it, and so do ED 0 and 1 for
/// the rows below and above the cursor's, not for the cursor's own row.
#[test]
fn text_and_erasure_clear_sixel_pixels() {
    const ALL: [bool; 3] = [true; 3];
    assert_eq!(sixel_cells_left(b""), [ALL; 3]);
    assert_eq!(sixel_cells_left(b"\x1b[1;2HX"), [[true, false, true], ALL, ALL]);
    // A wide character clears both its cells; REP and print runs too.
    assert_eq!(sixel_cells_left("\x1b[2;2H\u{6771}".as_bytes()), [ALL, [true, false, false], ALL]);
    assert_eq!(sixel_cells_left(b"\x1b[3;1Ha\x1b[b"), [ALL, ALL, [false, false, true]]);
    assert_eq!(sixel_cells_left(b"abc"), [[false; 3], ALL, ALL]);
    // A space is written like any character.
    assert_eq!(sixel_cells_left(b"\x1b[2;3H "), [ALL, [true, true, false], ALL]);
    // ED 0 and 1 clear the other rows whole, but not the cursor's.
    assert_eq!(sixel_cells_left(b"\x1b[2;2H\x1b[J"), [ALL, ALL, [false; 3]]);
    assert_eq!(sixel_cells_left(b"\x1b[2;2H\x1b[1J"), [[false; 3], ALL, ALL]);
    // EL, ECH, DCH and ICH leave them, as in xterm.
    for edit in ["\x1b[K", "\x1b[1K", "\x1b[2K", "\x1b[2X", "\x1b[P", "\x1b[@"] {
        assert_eq!(sixel_cells_left(format!("\x1b[2;2H{edit}").as_bytes()), [ALL; 3], "{edit:?}");
    }
    // Cursor movement and controls write nothing.
    assert_eq!(sixel_cells_left(b"\x1b[3;3H\r\n\t\x08"), [ALL; 3]);
}

/// kitty images are a layer of their own: text and erasure leave them.
#[test]
fn text_and_erasure_leave_kitty_images() {
    let log = b"\x1b_Ga=T,f=24,s=1,v=1,c=3,r=3,C=1;/wAA\x1b\\abc\x1b[2;1H\x1b[J\x1b[1J";
    let g = replay_sized(log, 6, 4, Lf::Index, (10, 6));
    assert_eq!(g.images.len(), 1);
    assert!(!g.images[0].sixel);
    assert_eq!(*g.images[0].pixels, [255, 0, 0, 255]);
}

/// Repeated writes over many overlapping Sixel images clear each once and
/// then write in place: 1,024 one-pixel images in one cell, and 4,097 writes
/// there (no autowrap; REP repeats at most a screenful, so one at a time),
/// with a Sixel image stored for each.
#[test]
fn repeated_writes_over_overlapping_sixel_images() {
    let mut log = Vec::new();
    for _ in 0..1024 {
        log.extend_from_slice(b"\x1b[1;6H\x1bP0;1q#1;2;100;0;0@\x1b\\");
    }
    log.extend_from_slice(b"\x1b[?7l\x1b[1;6Hx");
    for _ in 0..4096 {
        log.extend_from_slice(b"\x1b[b");
    }
    let g = replay_sized(&log, 6, 4, Lf::Index, (1, 1));
    assert_eq!(g.images.len(), 1024);
    assert!(g.images.iter().all(|p| p.sixel && *p.pixels == [0, 0, 0, 0]));
}

#[test]
fn truncated_sequences_do_not_panic() {
    for s in [&b"\x1b"[..], b"\x1b[", b"\x1b[12;", b"\x1b]0;t", b"\x1bPq", b"\x1b]0;\x1b"] {
        assert_eq!(grid(s).len(), C * R);
    }
}

#[test]
fn printable_runs_end_at_the_first_other_byte() {
    // Every byte value at every offset of a run long enough for the
    // eight-byte steps, from every start.
    let naive = |data: &[u8], mut i: usize| {
        while i < data.len() && (0x20..0x7f).contains(&data[i]) {
            i += 1;
        }
        i
    };
    for at in 0..40 {
        for byte in 0..=255u8 {
            let mut data = vec![b'x'; 40];
            data[at] = byte;
            for start in 0..data.len() {
                assert_eq!(printable_end(&data, start), naive(&data, start), "byte {byte:#x} at {at}, from {start}");
            }
        }
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

/// Combining marks that are consistent with their cells: sorted by cell, one
/// entry per cell, on the grid, each one to MAX_MARKS zero-width code points
/// packed at the front, and never on the tail of a wide character, nor on a
/// space where the log prints none.
fn marks_are_consistent(g: &Grid, cols: usize) -> Result<(), String> {
    for (k, m) in g.marks.iter().enumerate() {
        let i = m.cell as usize;
        let at = format!("row {} col {}", i / cols, i % cols);
        if k > 0 && g.marks[k - 1].cell >= m.cell {
            return Err(format!("marks out of order at {at}"));
        }
        if i >= g.cells.len() {
            return Err(format!("marks past the grid at {at}"));
        }
        let points = m.code_points();
        if points.is_empty() || m.marks[points.len()..].iter().any(|&c| c != 0) {
            return Err(format!("marks {:x?} badly packed at {at}", m.marks));
        }
        if let Some(c) = points.iter().find(|&&c| unicode::width(c) != 0) {
            return Err(format!("U+{c:04X} kept as a mark at {at}"));
        }
        if g.cells[i].attrs & TAIL != 0 || g.cells[i].ch == 0 {
            return Err(format!("marks on the tail of a wide character at {at}"));
        }
        if g.cells[i].ch == ' ' as u32 {
            return Err(format!("marks on an erased cell at {at}"));
        }
    }
    Ok(())
}

/// Random logs on random grid sizes, weighted toward the edges (one row or
/// column, and rarely the CLI's 500x200 limit), mixing wide and zero-width
/// characters with the sequences that move, wrap, insert, erase and scroll,
/// and with OSC and DCS strings. Checks that parse doesn't panic, leaves no
/// half of a wide character, and keeps combining marks consistent; and,
/// as none of the marks composes with any character here, that the cells
/// are those of the same log without them. Rounds and seed come from
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
    // Wide (CJK, Hangul, fullwidth, emoji, skin tone), DEC graphics
    // letters, plain ASCII and a no-break space (only erased cells are
    // spaces, so no space may keep a mark); then zero-width marks (combining acute, Thai
    // vowel and tone, Hebrew dagesh, Devanagari virama, ZWJ, VS16, and more
    // marks than a cell keeps).
    let chars = ["界", "한", "Ｗ", "😀", "🏽", "q", "x", "\u{a0}", "é"];
    let zero_width = ["\u{301}", "\u{e31}", "\u{e48}", "\u{5bc}", "\u{94d}", "\u{200d}", "\u{fe0f}",
                      "\u{301}\u{302}\u{303}\u{304}\u{305}"];
    let finals = b"HfABCDEFGd`a@PXKJbrLMSTIZ";
    let modes = ["?7h", "?7l", "?6h", "?6l", "?1049h", "?1049l", "?47h", "?47l", "4h", "4l", "?80h", "?80l"];
    let escapes = ["\x1b7", "\x1b8", "\x1bD", "\x1bE", "\x1bM", "\x1bc", "\x1b#8", "\x1b(0", "\x1b(B", "\x0e", "\x0f"];
    // OSC and DCS ended by BEL or ST, with a wide character inside, and one
    // left open so what follows lands in the string. The Sixel image is 7
    // rows tall in parse's 1x1 cells, so it scrolls, and DECSDM (?80) moves it.
    let strings = [
        "\x1b]0;界\x07",
        "\x1b]8;;http://x\x1b\\",
        "\x1bPq界\x1b\\",
        "\x1b]2;",
        "\x1bPq#1;2;100;0;0!3~-~\x1b\\",
    ];
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
        // plain is log without most of its marks; only a mark after a lone
        // UTF-8 lead byte stays, as leaving it out would join the lead to
        // what follows.
        let (mut log, mut plain) = (Vec::new(), Vec::new());
        for _ in 0..next() % 48 {
            let r = next();
            let start = log.len();
            match r % 12 {
                10 | 11 => {
                    log.extend_from_slice(zero_width[(r >> 8) as usize % zero_width.len()].as_bytes());
                    if !matches!(plain.last(), Some(0xe7 | 0xf0)) {
                        continue;
                    }
                }
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
            plain.extend_from_slice(&log[start..]);
        }
        let outcome = std::panic::catch_unwind(|| (replay(&log, cols, rows, Lf::Index), parse(&plain, cols, rows)));
        let mut failure = match &outcome {
            Err(_) => Some("panicked".to_string()),
            Ok((g, _)) if g.cells.len() != cols * rows => Some(format!("{} cells", g.cells.len())),
            Ok((g, plain)) => wide_pairs_are_whole(&g.cells, cols).and_then(|()| marks_are_consistent(g, cols)).and_then(|()| {
                match g.cells.iter().map(cell_key).eq(plain.iter().map(cell_key)) {
                    true => Ok(()),
                    false => Err("the marks changed the cells".into()),
                }
            }).err(),
        };
        // --lf-newline is exactly CR LF for every LF but a final bare one,
        // wherever the LF lands: at top level, inside a CSI, after an ESC,
        // or inside a string. The reference strips that LF by hand.
        if failure.is_none() {
            let text = if log.ends_with(b"\n") && !log.ends_with(b"\r\n") { &log[..log.len() - 1] } else { &log[..] };
            let crlf: Vec<u8> = text.iter().flat_map(|b| if *b == b'\n' { &b"\r\n"[..] } else { std::slice::from_ref(b) }).copied().collect();
            let outcome = std::panic::catch_unwind(|| (replay(&log, cols, rows, Lf::Newline), replay(&crlf, cols, rows, Lf::Index)));
            failure = match &outcome {
                Err(_) => Some("panicked with --lf-newline".to_string()),
                Ok((newline, crlf))
                    if !newline.cells.iter().map(cell_key).eq(crlf.cells.iter().map(cell_key)) || newline.marks != crlf.marks =>
                {
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
    // Reverse video is opaque over an image below the backgrounds, as in kitty.
    let opaque: Vec<_> = (0..3).map(|c| at(&g, 0, c).attrs & OPAQUE != 0).collect();
    assert_eq!(opaque, [true, false, true]);
}

#[test]
fn only_default_backgrounds_stay_clear_for_draw_c() {
    // Default, red, the default colour set explicitly, reverse video, and
    // reverse video whose background is the default colour by value.
    let mut g = grid(b"a\x1b[41mb\x1b[48;2;17;24;35mc\x1b[0;7md\x1b[0;7;38;2;17;24;35me");
    opaque_backgrounds(&mut g, DEFAULT_BG);
    let opaque: Vec<_> = (0..6).map(|c| at(&g, 0, c).attrs & OPAQUE != 0).collect();
    assert_eq!(opaque, [false, true, false, true, true, false]);
    // Other attributes are kept.
    let mut g = grid(b"\x1b[1;3;41mx");
    opaque_backgrounds(&mut g, DEFAULT_BG);
    assert_eq!(at(&g, 0, 0).attrs, BOLD | ITALIC | OPAQUE);
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
fn sgr_italic() {
    let g = grid(b"\x1b[3ma\x1b[23mb\x1b[1;3mc\x1b[22md\x1b[me");
    let attrs: Vec<u8> = (0..5).map(|c| at(&g, 0, c).attrs).collect();
    assert_eq!(attrs, [ITALIC, 0, BOLD | ITALIC, ITALIC, 0]);
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
        // The log too, so bench/c-vs-rust/deflate.rs --inputs can render it with the CLI.
        fs::write(dir.join(format!("{name}.pty")), log).unwrap();
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
/// then where the cursor is. The rows are --text's output, so the tmux
/// references check --text too.
fn screen_lines(g: &Grid, cols: usize, rows: usize) -> Vec<String> {
    let mut lines: Vec<String> = grid_text(&g.cells, &g.marks, cols).lines().map(String::from).collect();
    assert_eq!(lines.len(), rows);
    lines.push(match (g.cursor, g.cursor_shape) {
        (Some((row, col)), CursorShape::Block) => format!("cursor {col},{row}"),
        (Some((row, col)), shape) => format!("cursor {col},{row} {}", shape.name()),
        (None, _) => "cursor hidden".into(),
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
fn the_width_table_matches_the_width_ranges() {
    // Every code point, and past the end, as utf8_at never decodes there.
    for cp in 0..0x11_0100 {
        assert_eq!(unicode::width(cp), unicode::width_in_ranges(cp), "U+{cp:04X}");
    }
}

#[test]
fn every_composition_passes_the_mark_filter() {
    for &(base, mark, composed) in crate::unicode_tables::COMPOSE.iter() {
        assert_eq!(unicode::compose(base, mark), Some(composed), "U+{base:04X} U+{mark:04X}");
    }
    assert_eq!(unicode::compose('ก' as u32, 0x0E48), None);
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

/// The marks a log leaves on a C x R grid, as (row, col, marks) per cell.
fn marks(log: &str) -> Vec<(usize, usize, Vec<u32>)> {
    marks_on(log, C, R)
}

fn marks_on(log: &str, cols: usize, rows: usize) -> Vec<(usize, usize, Vec<u32>)> {
    let g = replay(log.as_bytes(), cols, rows, Lf::Index);
    g.marks.iter().map(|m| (m.cell as usize / cols, m.cell as usize % cols, m.code_points().to_vec())).collect()
}

#[test]
fn combining_marks_compose_or_join_the_cell() {
    // é and ä compose; x + U+0301 has no precomposed form, so the acute is
    // kept with the x.
    let log = "e\u{0301}a\u{0308}x\u{0301}y";
    assert_eq!(line(&grid(log.as_bytes()), 0), "éäxy      ");
    assert_eq!(marks(log), [(0, 2, vec![0x301])]);
    // Zero-width joiners and variation selectors take no cell either.
    let log = "a\u{200D}b\u{FE0F}c";
    assert_eq!(line(&grid(log.as_bytes()), 0), "abc       ");
    assert_eq!(marks(log), [(0, 0, vec![0x200d]), (0, 1, vec![0xfe0f])]);
    // A mark with nothing before it is dropped.
    assert_eq!(line(&grid("\u{0301}z".as_bytes()), 0), "z         ");
    assert_eq!(marks("\u{0301}z"), []);
    // Once a cell has a mark, later ones are kept in order rather than
    // composed: q + U+0301 + U+0304 + U+0301, and é + U+0302 + U+0301.
    assert_eq!(marks("q\u{301}\u{304}\u{301}"), [(0, 0, vec![0x301, 0x304, 0x301])]);
    assert_eq!(marks("e\u{301}\u{302}\u{301}"), [(0, 0, vec![0x302, 0x301])]);
    // Thai: a vowel above and a tone mark, neither precomposed.
    assert_eq!(marks("\u{e01}\u{e31}\u{e48}"), [(0, 0, vec![0xe31, 0xe48])]);
    // At most MAX_MARKS; the rest are dropped.
    assert_eq!(marks("q\u{301}\u{302}\u{303}\u{304}\u{305}x"), [(0, 0, vec![0x301, 0x302, 0x303, 0x304])]);
}

#[test]
fn marks_join_the_last_printed_character() {
    // On a wide character: its first cell.
    assert_eq!(marks("界\u{301}x"), [(0, 0, vec![0x301])]);
    // With a wrap pending: the last column, before anything wraps.
    assert_eq!(marks("abcdefghij\u{301}k"), [(0, 9, vec![0x301])]);
    // Wherever the cursor has gone since, as in xterm, and across CR LF.
    assert_eq!(marks("ab\x1b[3;6H\u{301}"), [(0, 1, vec![0x301])]);
    assert_eq!(marks("ab\r\n\u{301}x"), [(0, 1, vec![0x301])]);
    // Not to a character dropped for want of room (no autowrap, wide in
    // the last column): its marks go with it.
    assert_eq!(marks("a\x1b[?7l\x1b[1;10H界\u{301}"), []);
    assert_eq!(line(&grid("a\x1b[?7l\x1b[1;10H界\u{301}".as_bytes()), 0), "a         ");
    // Not to a cell erased, overwritten or moved since.
    for log in ["ab\x1b[1;2H\x1b[K\u{301}", "ab\x1b[1;2H\x1b[X\u{301}", "ab\x1b[2J\u{301}", "ab\x1b[1;1H\x1b[@\u{301}",
                "ab\x1b[1;1H\x1b[P\u{301}", "中\x1b[1;2Hx\x1b[1;1H\x1b[2X\u{301}"] {
        assert_eq!(marks(log), [], "{:?}", log);
    }
    // Scrolling keeps the character, so the mark still finds it.
    assert_eq!(marks("\x1b[4;1Hab\n\u{301}"), [(2, 1, vec![0x301])]);
    // The alternate screen has its own last character; leaving it forgets it.
    assert_eq!(marks("ab\x1b[?1049h\u{301}"), []);
    assert_eq!(marks("ab\x1b[?1049hx\x1b[?1049l\u{301}"), []);
}

#[test]
fn marks_go_with_the_cell_they_are_on() {
    let q = "q\u{301}";
    // ICH and DCH move them along the line.
    assert_eq!(marks(&format!("a{q}x\x1b[1;1H\x1b[2@")), [(0, 3, vec![0x301])]);
    assert_eq!(marks(&format!("ab{q}x\x1b[1;1H\x1b[2P")), [(0, 0, vec![0x301])]);
    // IL, DL, SU, SD, IND and RI move them with their row, inside the margins.
    assert_eq!(marks(&format!("\x1b[2;1H{q}\x1b[1;1H\x1b[L")), [(2, 0, vec![0x301])]);
    assert_eq!(marks(&format!("\x1b[3;1H{q}\x1b[1;1H\x1b[M")), [(1, 0, vec![0x301])]);
    assert_eq!(marks(&format!("\x1b[3;1H{q}\x1b[2S")), [(0, 0, vec![0x301])]);
    assert_eq!(marks(&format!("\x1b[2;1H{q}\x1b[T")), [(2, 0, vec![0x301])]);
    assert_eq!(marks(&format!("\x1b[2;3r\x1b[3;1H{q}\n")), [(1, 0, vec![0x301])]);
    assert_eq!(marks(&format!("\x1b[2;3r\x1b[2;1H{q}\x1bM")), [(2, 0, vec![0x301])]);
    // Outside the margins nothing moves.
    assert_eq!(marks(&format!("\x1b[4;1H{q}\x1b[1;3r\x1b[3;1H\n")), [(3, 0, vec![0x301])]);
    // The main screen keeps its marks while the alternate one is shown.
    assert_eq!(marks(&format!("{q}\x1b[?1049hx\x1b[?1049l")), [(0, 0, vec![0x301])]);
    assert_eq!(marks(&format!("{q}\x1b[?1049h")), []);
    assert_eq!(marks(&format!("\x1b[?1049h{q}")), [(0, 0, vec![0x301])]);
    assert_eq!(marks(&format!("\x1b[?1049h{q}\x1b[?1049l\x1b[?1049h")), []);
    // Mode 47 does not clear the alternate screen, so its marks come back.
    assert_eq!(marks(&format!("\x1b[?47h{q}\x1b[?47l\x1b[?47h")), [(0, 0, vec![0x301])]);
    assert_eq!(marks(&format!("\x1b[?1047h{q}\x1b[?1047l\x1b[?1047h")), []);
}

#[test]
fn marks_go_when_their_cell_does() {
    let q = "q\u{301}";
    for (log, why) in [
        (format!("{q}\x1b[1;1Hx"), "overwritten"),
        (format!("{q}\x1b[1;1Hxyz"), "overwritten by an ASCII run"),
        (format!("{q}\x1b[1;1H界"), "overwritten by a wide character"),
        (format!("界\u{301}\x1b[1;2Hx"), "its wide character's tail overwritten"),
        (format!("\x1b[?7l\x1b[1;10H{q}xyz"), "overwritten in the last column, without autowrap"),
        (format!("{q}\x1b[1;1H\x1b[X"), "ECH"),
        (format!("{q}\x1b[K\x1b[1;1H\x1b[K"), "EL"),
        (format!("{q}\x1b[1;1H\x1b[1K"), "EL 1"),
        (format!("{q}\x1b[2K"), "EL 2"),
        (format!("{q}\x1b[J\x1b[1;1H\x1b[J"), "ED"),
        (format!("{q}\x1b[2;1H\x1b[1J"), "ED 1"),
        (format!("{q}\x1b[2J"), "ED 2"),
        (format!("{q}\x1b[1;1H\x1b[P"), "DCH"),
        (format!("\x1b[1;10H{q}\x1b[1;1H\x1b[@"), "ICH pushing it off the line"),
        (format!("{q}\x1b[1;1H\x1b[M"), "DL"),
        (format!("{q}\x1b[4;1H\n"), "scrolled off the top"),
        (format!("{q}\x1b[S"), "SU"),
        (format!("{q}\x1bc"), "RIS"),
    ] {
        assert_eq!(marks(&log), [], "{why}: {log:?}");
    }
    // A wide character cut in two by an edit loses all of itself: ECH of
    // its tail, ICH pushing its tail off the line.
    assert_eq!(marks("界\u{301}\x1b[1;2H\x1b[X"), []);
    assert_eq!(marks("\x1b[1;9H界\u{301}\x1b[1;1H\x1b[@"), []);
    // Moved whole, it keeps them.
    assert_eq!(marks("x界\u{301}\x1b[1;1H\x1b[P"), [(0, 0, vec![0x301])]);
}

#[test]
fn rep_repeats_a_character_with_its_marks() {
    let g = replay("q\u{301}\x1b[2b".as_bytes(), C, R, Lf::Index);
    assert_eq!(grid_text(&g.cells, &g.marks, C).lines().next(), Some("q\u{301}q\u{301}q\u{301}"));
    // A precomposed character repeats as itself.
    assert_eq!(line(&grid("e\u{301}\x1b[2b".as_bytes()), 0), "ééé       ");
    // Once its cell is erased, the character repeats without them.
    assert_eq!(marks("\u{e01}\u{e31}\u{e48}\x1b[1;1H\x1b[K\x1b[b"), []);
    assert_eq!(line(&grid("\u{e01}\u{e31}\x1b[1;1H\x1b[K\x1b[b".as_bytes()), 0), "ก         ");
    // A character printed since has none.
    assert_eq!(marks("q\u{301}x\x1b[b"), [(0, 0, vec![0x301])]);
}

#[test]
fn text_and_json_put_marks_after_their_character() {
    let log = "q\u{301}\x1b[1mx\x1b[m \u{302}\r\n界\u{e31}\u{e48}";
    let g = replay(log.as_bytes(), 6, 2, Lf::Index);
    assert_eq!(grid_text(&g.cells, &g.marks, 6), "q\u{301}x \u{302}\n界\u{e31}\u{e48}\n");
    // A space with a mark ends no run early: it is not blank.
    let want = "{\"cols\":6,\"rows\":2,\"cursor\":{\"col\":2,\"row\":1,\"shape\":\"block\"},\"lines\":[\n\
        [{\"col\":0,\"text\":\"q\u{301}\",\"fg\":\"#dbe7f7\",\"bg\":\"#111823\"},\
        {\"col\":1,\"text\":\"x\",\"fg\":\"#dbe7f7\",\"bg\":\"#111823\",\"bold\":true},\
        {\"col\":2,\"text\":\" \u{302}\",\"fg\":\"#dbe7f7\",\"bg\":\"#111823\"}],\n\
        [{\"col\":0,\"text\":\"界\u{e31}\u{e48}\",\"fg\":\"#dbe7f7\",\"bg\":\"#111823\"}]\n]}\n";
    assert_eq!(grid_json(&g.cells, &g.marks, 6, 2, g.cursor, g.cursor_shape, DEFAULT_BG), want);
    assert_eq!(marks_of(&g.marks, 0), [0x301]);
    assert_eq!(marks_of(&g.marks, 1), []);
    assert_eq!(marks_of(&g.marks, 6), [0xe31, 0xe48]);
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

/// Each of a parse's allocations that grow with the grid (both screens'
/// cells and rows, the tab stops, the marks, the placeholder ids, the
/// cells and the marks in screen order, and a reset's fresh screens) fails
/// it in turn: replay_with
/// returns None, which the library makes Error::OutOfMemory and the CLI
/// exit 2, and with nothing left to fail it gives the grid it gives
/// without faults. tests/run.sh and tests/library.rs fail them through the
/// CLI and the library too.
#[test]
fn a_failed_parse_allocation_fails_the_parse() {
    use crate::screen::faults::FAULTS;
    // A placeholder with a colour (ids), a combining mark (marks), a reset
    // (two screens again), another mark, and a scroll (cells in screen order).
    let log = "\x1b[38;5;1m\u{10EEEE}e\u{301}q\u{302}\x1bcq\u{301}\r\n\r\nx\u{303}".as_bytes();
    let options = ParseOptions::default();
    let want = replay_with(log, 4, 2, &options, (1, 1)).unwrap();
    let mut fail = 1;
    loop {
        FAULTS.with(|f| f.set((0, fail)));
        let grid = replay_with(log, 4, 2, &options, (1, 1));
        let calls = FAULTS.with(|f| f.get().0);
        FAULTS.with(|f| f.set((0, 0)));
        match grid {
            None => fail += 1,
            Some(grid) => {
                assert!(fail > calls, "allocation {fail} of {calls} failed, and the parse went on");
                assert_eq!((grid.to_text(), grid.to_json()), (want.to_text(), want.to_json()));
                break;
            }
        }
    }
    // Both screens' cells and rows and the tab stops, the ids, the marks,
    // all five again after the reset, the marks again, then the marks and
    // the cells in screen order.
    assert_eq!(fail - 1, 15, "{} allocations failed", fail - 1);
}
