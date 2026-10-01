//! Parser unit tests. No crates, no FFI calls; ./test.sh builds and runs them.
//! Tests marked #[ignore] state the correct behaviour for a known bug (see the
//! issue in the reason) and should pass once it is fixed.

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
fn cr_and_lf() {
    let g = grid(b"ab\r\ncd\rX");
    assert_eq!(line(&g, 0), "ab        ");
    assert_eq!(line(&g, 1), "Xd        ");
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
    assert_eq!(at(&g, 0, 0).bold, 0);
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
    assert_eq!(at(&g, 0, 0).bold, 1);
    assert_eq!(at(&g, 0, 1).bold, 0);
    assert_eq!(fg(at(&g, 0, 2)), DEFAULT_FG);
    assert_eq!(bg(at(&g, 0, 3)), DEFAULT_BG);
    assert_eq!(at(&g, 0, 4).bold, 1);
    assert_eq!(at(&g, 0, 5).bold, 0);
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
    assert_eq!(at(&grid(b"\x1b[38;5;1ma"), 0, 0).bold, 0);
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
    assert_eq!(at(&g, 0, 0).bold, 1);
}

#[test]
fn sgr_component_over_255_is_ignored() {
    assert_eq!(fg(at(&grid(b"\x1b[38;2;300;0;0ma"), 0, 0)), DEFAULT_FG);
}

#[test]
fn esc_aborts_a_string_sequence() {
    let g = grid(b"\x1b]0;title\x1b[1ma\x1b]0;t\x18b");
    assert_eq!(line(&g, 0), "ab        ");
    assert_eq!(at(&g, 0, 0).bold, 1);
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
