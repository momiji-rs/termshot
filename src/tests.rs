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
    let pick = [0x1b, b'[', b';', b'0', b'9', b'H', b'K', b'J', b'm', b']', 0x07, b'\\', 0xe2, b'\n'];
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..20_000 {
        let mut buf = Vec::new();
        for _ in 0..(x % 96) {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            buf.push(if x & 1 == 0 { pick[(x >> 8) as usize % pick.len()] } else { (x >> 16) as u8 });
        }
        let _ = parse(&buf, 7, 3);
    }
}

#[test]
#[ignore = "#5: ESC with an intermediate byte prints its final byte"]
fn charset_designation_is_not_printed() {
    assert_eq!(line(&grid(b"\x1b(B\x1b)0\x1b#8a"), 0), "a         ");
}

#[test]
#[ignore = "#5: EL ignores its mode"]
fn el_modes() {
    assert_eq!(line(&grid(b"abcdef\x1b[1;4H\x1b[1K"), 0), "    ef    ");
    assert_eq!(line(&grid(b"abcdef\x1b[1;4H\x1b[2K"), 0), "          ");
}

#[test]
#[ignore = "#5: ED handles only mode 2"]
fn ed0_erases_below() {
    let g = grid(b"ab\r\ncd\x1b[1;2H\x1b[J");
    assert_eq!(line(&g, 0), "a         ");
    assert_eq!(line(&g, 1), "          ");
}

#[test]
#[ignore = "#5: ED 2 homes the cursor"]
fn ed2_keeps_the_cursor() {
    assert_eq!(line(&grid(b"abc\x1b[2Jx"), 0), "   x      ");
}

#[test]
#[ignore = "#5: CUF clamps to cols instead of the last column"]
fn cuf_stops_at_last_column() {
    assert_eq!(line(&grid(b"ab\x1b[99Cx"), 0), "ab       x");
}

#[test]
#[ignore = "#5: DEL is printed"]
fn del_is_not_printed() {
    assert_eq!(line(&grid(b"a\x7fb"), 0), "ab        ");
}

#[test]
#[ignore = "#5: 38;5;n reads n as its own SGR"]
fn indexed_colour_does_not_set_bold() {
    assert_eq!(at(&grid(b"\x1b[38;5;1ma"), 0, 0).bold, 0);
}

#[test]
#[ignore = "#5: invalid UTF-8 is not replaced"]
fn invalid_utf8_becomes_replacement_char() {
    assert_eq!(at(&grid(b"\xffA"), 0, 0).ch, 0xfffd);
    assert_eq!(at(&grid(b"\xe2A"), 0, 1).ch, 'A' as u32);
}
