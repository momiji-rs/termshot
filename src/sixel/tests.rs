use super::*;
use crate::{replay_sized, Lf};

/// The pixels of a decoded image, one [r, g, b, a] each.
fn pixels(body: &[u8]) -> (u32, u32, Vec<[u8; 4]>) {
    let image = decode(body, &mut Budget::default()).expect("an image");
    let px = image.rgba.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]]).collect();
    (image.width, image.height, px)
}

/// The colour of one opaque pixel.
fn only(body: &[u8]) -> [u8; 3] {
    let (w, h, px) = pixels(body);
    assert_eq!((w, h), (1, 1), "{}", String::from_utf8_lossy(body));
    assert_eq!(px[0][3], 255);
    [px[0][0], px[0][1], px[0][2]]
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const CLEAR: [u8; 4] = [0; 4];

#[test]
fn only_sixel_dcs_strings_are_sixel() {
    assert_eq!(split(b"q~"), Some((0, &b"~"[..])));
    assert_eq!(split(b"0;1;0q#0"), Some((1, &b"#0"[..])));
    assert_eq!(split(b";2q"), Some((2, &b""[..])));
    assert_eq!(split(b"9;99999999999999q").map(|s| s.0), Some(u32::MAX));
    // DECRQSS, XTGETTCAP, DECUDK, and others with intermediates or another final.
    for body in [&b"$q\"p"[..], b"+q4d73", b"1|", b"zz", b"data", b"?q", b"1:2q", b"", b"12"] {
        assert_eq!(split(body), None, "{}", String::from_utf8_lossy(body));
        assert!(decode(body, &mut Budget::default()).is_none());
    }
}

#[test]
fn default_palette_is_the_vt340s_and_drawing_starts_in_register_3() {
    // 20/80/80 % and 20/80/20 %, rounded half up to 8 bits.
    assert_eq!(only(b"q\"1;1;1;1#5@"), [51, 204, 204]);
    assert_eq!(only(b"q\"1;1;1;1@"), [51, 204, 51]);
    assert_eq!(only(b"q\"1;1;1;1#15@"), [204, 204, 204]);
    assert_eq!(only(b"q\"1;1;1;1#7@"), [135, 135, 135]);
    // Registers past the VT340's 16 start black; numbers wrap at 1,024.
    assert_eq!(only(b"q\"1;1;1;1#16@"), [0, 0, 0]);
    assert_eq!(only(b"q\"1;1;1;1#1029@"), [51, 204, 204]);
}

#[test]
fn rgb_colours_scale_from_percent() {
    assert_eq!(only(b"q#1;2;100;50;0@"), [255, 128, 0]);
    assert_eq!(only(b"q#1;2;1;99;33@"), [3, 252, 84]);
    // A register is shared by every register number that wraps to it.
    assert_eq!(only(b"q#1025;2;0;0;100#1@"), [0, 0, 255]);
}

#[test]
fn hls_colours_convert_as_xterm_does() {
    // DEC hue: 0 is blue, 120 red, 240 green.
    assert_eq!(only(b"q#1;1;0;50;100@"), [0, 0, 255]);
    assert_eq!(only(b"q#1;1;120;50;100@"), [255, 0, 0]);
    assert_eq!(only(b"q#1;1;240;50;100@"), [0, 255, 0]);
    assert_eq!(only(b"q#1;1;360;50;100@"), [0, 0, 255]);
    assert_eq!(only(b"q#1;1;60;50;100@"), [255, 0, 255]);
    assert_eq!(only(b"q#1;1;180;50;100@"), [255, 255, 0]);
    assert_eq!(only(b"q#1;1;300;50;100@"), [0, 255, 255]);
    // Saturation 0 is grey at the lightness; lightness 100 is white.
    assert_eq!(only(b"q#1;1;200;50;0@"), [128, 128, 128]);
    assert_eq!(only(b"q#1;1;200;100;100@"), [255, 255, 255]);
    // h 90, l 25, s 50: c = 0.25, x = 0.125, m = 0.125, so 38/13/25 %.
    assert_eq!(hls(90, 25, 50), [38, 13, 25]);
    assert_eq!(only(b"q#1;1;90;25;50@"), [97, 33, 64]);
}

#[test]
fn bad_colour_definitions_still_select_the_register() {
    // Out of range, an unknown colour space, too few or too many parameters,
    // or an empty one: register 1 keeps the VT340's 20/20/80 %.
    for def in ["1;2;101;0;0", "1;1;361;0;0", "1;1;0;101;0", "1;3;0;0;0", "1;2;100;0", "1;2;100;0;0;0", "1;2;;0;0", "1;2"] {
        let body = format!("q#{def}@");
        assert_eq!(only(body.as_bytes()), [51, 51, 204], "{def}");
    }
    // With no register number at all, the selection stays at register 3.
    assert_eq!(only(b"q#;2;100;0;0@"), [51, 204, 51]);
}

#[test]
fn the_palette_at_the_end_colours_every_pixel() {
    assert_eq!(only(b"q#1;2;100;0;0@#1;2;0;0;100"), [0, 0, 255]);
}

#[test]
fn sixels_are_six_pixels_tall_lowest_bit_on_top() {
    // 'A' is 0b000010: only the second row of the band.
    let (w, h, px) = pixels(b"q#1;2;100;0;0A");
    assert_eq!((w, h), (1, 2));
    assert_eq!(px, [CLEAR, RED]);
    let (w, h, px) = pixels(b"q#1;2;100;0;0~");
    assert_eq!((w, h), (1, 6));
    assert!(px.iter().all(|&p| p == RED));
}

#[test]
fn repeats() {
    let (w, h, px) = pixels(b"q#1;2;100;0;0!3@");
    assert_eq!((w, h), (3, 1));
    assert_eq!(px, [RED; 3]);
    // No count, or 0, draws once.
    assert_eq!(pixels(b"q!@").0, 1);
    assert_eq!(pixels(b"q!0@").0, 1);
    // A repeated empty sixel only moves; a repeat of anything else is
    // dropped together with what follows it.
    let (w, _, px) = pixels(b"q#1;2;100;0;0!2?@");
    assert_eq!((w, px), (3, vec![CLEAR, CLEAR, RED]));
    let (w, _, px) = pixels(b"q#1;2;100;0;0!5$@");
    assert_eq!((w, px), (1, vec![RED]));
    // Spaces and controls are ignored, even inside a count.
    assert_eq!(pixels(b"q!1 \r\n2@").0, 12);
}

#[test]
fn carriage_return_and_next_line() {
    // $ goes back to column 0 of the band to draw over it in another colour.
    let (w, h, px) = pixels(b"q#1;2;100;0;0!2B$#2;2;0;0;100@");
    assert_eq!((w, h), (2, 2));
    assert_eq!(px, [BLUE, RED, RED, RED]);
    // - starts the next band of six rows at column 0.
    let (w, h, px) = pixels(b"q#1;2;100;0;0?@-@");
    assert_eq!((w, h), (2, 7));
    assert_eq!(px[1], RED);
    assert_eq!(px[12], RED);
    assert_eq!(px.iter().filter(|&&p| p == RED).count(), 2);
    // Empty bands still count; trailing ones add nothing.
    assert_eq!(pixels(b"q@---@----").1, 19);
}

#[test]
fn raster_attributes_size_the_image_and_p2_its_background() {
    // P2 0 or 2 paints the declared area with register 0; P2 1 leaves it clear.
    for (p2, background) in [("", [0, 0, 0, 255]), ("0;0", [0, 0, 0, 255]), ("0;2", [0, 0, 0, 255]), ("0;1", CLEAR)] {
        let body = format!("{p2}q\"1;1;3;2#1;2;100;0;0@");
        let (w, h, px) = pixels(body.as_bytes());
        assert_eq!((w, h), (3, 2), "{p2}");
        assert_eq!(px, [RED, background, background, background, background, background], "{p2}");
    }
    // Register 0 is the background, at its final colour.
    let (_, _, px) = pixels(b"q\"1;1;2;1#1;2;100;0;0@#0;2;0;0;100");
    assert_eq!(px, [RED, BLUE]);
    // Pixels set past the declared area grow the image; what they leave unset
    // stays transparent, as does everything after a raster that comes late.
    let (w, h, px) = pixels(b"q\"1;1;1;1#1;2;100;0;0?A");
    assert_eq!((w, h), (2, 2));
    assert_eq!(px, [[0, 0, 0, 255], CLEAR, CLEAR, RED]);
    let (w, h, px) = pixels(b"q@\"1;1;2;1");
    assert_eq!((w, h), (2, 1));
    assert_eq!(px[1], CLEAR);
    // The aspect ratio and P1 change nothing: pixels stay square.
    assert_eq!(pixels(b"2q\"5;1;1;1").1, 1);
    assert_eq!(pixels(b"q\"1;3;2;2").0, 2);
    // A declared size alone is an image; nothing at all is not.
    assert_eq!(pixels(b"0;1q\"1;1;2;3").2, [CLEAR; 6]);
    assert!(decode(b"q", &mut Budget::default()).is_none());
    assert!(decode(b"q#0", &mut Budget::default()).is_none());
    assert!(decode(b"q#1;2;100;0;0!9?-?$", &mut Budget::default()).is_none());
    assert!(decode(b"q\"1;1", &mut Budget::default()).is_none());
}

#[test]
fn limits_refuse_the_whole_image() {
    assert_eq!(pixels(b"q!8192@").0, 8192);
    assert!(decode(b"q!8193@", &mut Budget::default()).is_none());
    assert!(decode(b"q!8192?@", &mut Budget::default()).is_none());
    assert!(decode(b"q!99999999999999999999@", &mut Budget::default()).is_none());
    assert_eq!(pixels(b"q\"1;1;1;8192").1, 8192);
    assert!(decode(b"q\"1;1;1;8193", &mut Budget::default()).is_none());
    assert!(decode(b"q\"1;1;8193;1", &mut Budget::default()).is_none());
    // 4,194,304 pixels at most, the 16 MiB of RGBA a kitty upload may have.
    assert_eq!(pixels(b"q\"1;1;4096;1024").1, 1024);
    assert!(decode(b"q\"1;1;4096;1025", &mut Budget::default()).is_none());
    assert!(decode(b"q\"1;1;4096;1024-!4097@", &mut Budget::default()).is_none());
    // Past 8,192 rows only a set pixel is refused; moving there is cheap.
    let mut body = b"q@".to_vec();
    body.extend(std::iter::repeat(b'-').take(100_000));
    assert_eq!(pixels(&body).1, 1);
    body.push(b'@');
    assert!(decode(&body, &mut Budget::default()).is_none());
    // A count past the limit costs nothing, drawn or not.
    assert!(decode(b"q!4294967295~", &mut Budget::default()).is_none());
    assert_eq!(pixels(b"q!4294967295?$@-@").1, 7);
}

#[test]
fn a_log_decodes_only_as_many_pixels_as_its_budget() {
    // Four of the largest images, then only what the data pays for.
    let mut budget = Budget::default();
    let large = b"q\"1;1;4096;1024";
    for _ in 0..4 {
        assert!(decode(large, &mut budget).is_some());
    }
    assert!(decode(large, &mut budget).is_none());
    assert!(decode(b"q!8192~", &mut budget).is_none());
    // Each byte of data pays for 256 pixels, its own image's first.
    let mut budget = Budget(0);
    assert_eq!(decode(b"q\"1;1;64;40", &mut budget).map(|i| i.height), Some(40));
    assert_eq!(budget.0, 10 * BUDGET_PER_BYTE - 64 * 40);
    assert!(decode(b"q\"1;1;64;64", &mut budget).is_none());
    // Refused images and strings that are not Sixel cost nothing.
    assert_eq!(budget.0, 20 * BUDGET_PER_BYTE - 64 * 40);
    assert!(decode(b"+q4d73", &mut budget).is_none());
    assert_eq!(budget.0, 20 * BUDGET_PER_BYTE - 64 * 40);
}

#[test]
fn drawing_over_the_same_pixels_again_is_paid_for() {
    // Each `!8192~$` writes 49,152 pixels and pays for 7 * 256.
    let over = |times: usize| [&b"q"[..], &b"!8192~$".repeat(times)].concat();
    let mut budget = Budget::default();
    assert_eq!(decode(&over(300), &mut budget).map(|i| i.width), Some(8192));
    let spent = 300 * 49_152 + 8192 * 6;
    assert_eq!(budget.0, BUDGET_BASE + 300 * 7 * BUDGET_PER_BYTE - spent);
    // Past the budget the image is refused, and what it wrote stays spent.
    assert!(decode(&over(300), &mut budget).is_none());
    assert!(budget.0 < 49_152);
    // Each pixel set once, as a real image sets it, costs twice its area.
    let mut budget = Budget(0);
    assert!(decode(b"q!40~", &mut budget).is_some());
    assert_eq!(budget.0, 4 * BUDGET_PER_BYTE - 2 * 240);
}

#[test]
fn needs_cell_metrics_only_for_committed_sixel() {
    assert!(needs_cell_metrics(b"\x1bPq~\x1b\\"));
    assert!(needs_cell_metrics(b"x\x1bP0;1;0q\"1;1;1;1\x1b\\"));
    for log in [&b"\x1bPq~"[..], b"\x1bPq~\x07", b"\x1bPq~\x18", b"\x1bP+q4d73\x1b\\", b"\x1bP$q\"p\x1b\\", b"\x1b_Gq~\x1b\\"] {
        assert!(!needs_cell_metrics(log), "{}", String::from_utf8_lossy(log));
    }
    // An OSC ends at its ESC; the DCS after it counts.
    assert!(needs_cell_metrics(b"\x1b]0;\x1bPq\x1b\\"));
}

#[test]
fn kitty_command_encodes_every_length() {
    // Rows of 1 to 7 pixels end the base64 with each padding.
    for len in 1..8u32 {
        let rgba: Vec<u8> = (0..len as u8 * 4).map(|b| b.wrapping_mul(37) | 1).collect();
        let image = Image { width: len, height: 1, rgba: rgba.clone() };
        let command = kitty_command(&image);
        let header = format!("a=T,f=32,s={len},v=1,C=1,q=2;");
        assert!(command.starts_with(header.as_bytes()));
        let mut log = b"\x1b_G".to_vec();
        log.extend_from_slice(&command);
        log.extend_from_slice(b"\x1b\\");
        assert_eq!(*replay(&log).images[0].pixels, rgba, "{len}");
    }
}

fn replay(log: &[u8]) -> crate::Grid {
    replay_sized(log, 20, 10, Lf::Index, (10, 20))
}

fn text(g: &crate::Grid, row: usize) -> String {
    g.cells[row * 20..(row + 1) * 20].iter().map(|c| char::from_u32(c.ch).unwrap()).collect::<String>().trim_end().to_string()
}

/// A red image `h` pixels tall and 2 wide.
fn red(h: usize) -> Vec<u8> {
    format!("\x1bP0;1q\"1;1;2;{h}#1;2;100;0;0").into_bytes().into_iter().chain(b"\x1b\\".iter().copied()).collect()
}

#[test]
fn placed_at_the_cursor_at_native_size() {
    let g = replay(&[b"\x1b[3;4H".as_slice(), &red(6)].concat());
    assert_eq!(g.images.len(), 1);
    let p = &g.images[0];
    assert_eq!((p.width, p.height, p.x, p.w, p.h), (2, 6, 30, 2, 6));
    assert_eq!(*p.pixels, vec![0; 2 * 6 * 4]);
    let g = replay(&[b"\x1b[3;4H".as_slice(), b"\x1bPq#1;2;100;0;0!2@\x1b\\"].concat());
    assert_eq!(*g.images[0].pixels, [RED, RED].concat());
}

#[test]
fn the_cursor_ends_on_the_last_row_the_image_covers() {
    for (h, row) in [(1, 2), (20, 2), (21, 3), (60, 4), (61, 5)] {
        let g = replay(&[b"\x1b[3;4H".as_slice(), &red(h)].concat());
        assert_eq!(g.cursor, Some((row, 3)), "{h} pixels");
    }
    // A pending wrap is cancelled, and the column stays.
    let mut log = vec![b'x'; 20];
    log.extend_from_slice(&red(20));
    log.push(b'y');
    let g = replay(&log);
    assert_eq!(text(&g, 0), "x".repeat(19) + "y");
}

#[test]
fn an_image_past_the_bottom_scrolls_the_screen() {
    // Three rows from row 9: two scroll, and the text with them.
    let g = replay(&[b"\x1b[3Htext\x1b[10;5H".as_slice(), &red(41)].concat());
    assert_eq!(text(&g, 0), "text");
    assert_eq!(g.cursor, Some((9, 4)));
    let p = &g.images[0];
    assert_eq!((p.x, p.height), (40, 41));
    // Taller than the screen: its top is cut off where it scrolled out.
    let g = replay(&[b"\x1b[10;5H".as_slice(), &red(250)].concat());
    assert_eq!(g.cursor, Some((9, 4)));
    assert_eq!(g.images[0].height, 190);
    assert_eq!(g.images[0].pixels.len(), 2 * 190 * 4);
    // Inside a scrolling region, only the region scrolls.
    let g = replay(&[b"\x1b[1Htop\x1b[5Hmid\x1b[2;6r\x1b[6;1H".as_slice(), &red(41)].concat());
    assert_eq!((text(&g, 0), text(&g, 2)), ("top".into(), "mid".into()));
    assert_eq!(g.cursor, Some((5, 0)));
    // Below the region, nothing scrolls and the image is clipped by the screen.
    let g = replay(&[b"\x1b[1Htop\x1b[2;6r\x1b[9;1H".as_slice(), &red(41)].concat());
    assert_eq!(text(&g, 0), "top");
    assert_eq!(g.cursor, Some((9, 0)));
}

#[test]
fn decsdm_draws_at_the_top_left_without_moving_the_cursor() {
    let g = replay(&[b"\x1b[?80h\x1b[10;5H".as_slice(), &red(250)].concat());
    assert_eq!(g.cursor, Some((9, 4)));
    assert_eq!((g.images[0].x, g.images[0].height), (0, 250));
    let g = replay(&[b"\x1b[?80h\x1b[?80l\x1b[3;4H".as_slice(), &red(30)].concat());
    assert_eq!((g.cursor, g.images[0].x), (Some((3, 3)), 30));
}

#[test]
fn unterminated_or_cancelled_images_are_not_drawn_and_never_print() {
    for end in [&b""[..], b"\x07", b"\x18", b"\x1a", b"\x1b[H"] {
        let mut log = b"\x1bPq#1;2;100;0;0~~".to_vec();
        log.extend_from_slice(end);
        let g = replay(&log);
        assert!(g.images.is_empty());
        assert!(g.cells.iter().all(|c| c.ch == u32::from(b' ')));
    }
}

#[test]
fn images_share_the_kitty_store() {
    // Erase in display removes them, as it does kitty's; so does a reset.
    assert!(replay(&[red(6).as_slice(), b"\x1b[2J"].concat()).images.is_empty());
    assert!(replay(&[red(6).as_slice(), b"\x1bc"].concat()).images.is_empty());
    // They scroll with the text like any placement inside the region.
    let g = replay(&[b"\x1b[5H".as_slice(), &red(20), b"\x1b[2S"].concat());
    assert_eq!(g.images.len(), 1);
    // Each image has its own palette: the second starts from the VT340's.
    let g = replay(b"\x1bPq#1;2;100;0;0@\x1b\\\x1bPq#1@\x1b\\");
    assert_eq!(*g.images[1].pixels, [51, 51, 204, 255]);
    // The alternate screen keeps its own.
    let g = replay(&[red(6).as_slice(), b"\x1b[?1049h"].concat());
    assert!(g.images.is_empty());
}
