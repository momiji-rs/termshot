//! Unicode placeholders: virtual placements (U=1) shown by U+10EEEE cells,
//! the ids in the colours, rows and columns from diacritics and their
//! inheritance, fitting the image into the placeholder box, the lifecycle of
//! the cells and the placements, and relative placements under a virtual
//! one. Cells are 10x20 pixels on a 10x6 screen; semantics are kitty's
//! (screen.c screen_render_line_graphics, graphics.c grman_put_cell_image,
//! master), with the expected pixels worked out by hand from them.

use super::*;
use crate::{replay_sized, Lf};

/// A 1x1 red RGB image.
const PIXEL: &str = "f=24,s=1,v=1;/wAA";
/// A 2x1 RGB image: red, green.
const WIDE: &str = "f=24,s=2,v=1;/wAAAP8A";
/// A 1x2 RGB image: red over green.
const TALL: &str = "f=24,s=1,v=2;/wAAAP8A";
const P: &str = "\u{10EEEE}";
/// The diacritics for 0 to 4, from the spec and rowcolumn-diacritics.txt.
const D: [&str; 5] = ["\u{305}", "\u{30D}", "\u{30E}", "\u{310}", "\u{312}"];

fn replay(log: &str) -> crate::Grid {
    replay_sized(log.as_bytes(), 10, 6, Lf::Index, (10, 20))
}

fn apc(keys: &str) -> String {
    format!("\x1b_G{keys}\x1b\\")
}

/// What is drawn: (image id, z, x, y, w, h, [left, top, right, bottom]),
/// with the clip as draw.c applies it.
type Drawn = (u32, i32, i64, i64, i64, i64, [i64; 4]);

fn drawn(grid: &crate::Grid) -> Vec<Drawn> {
    let mut out = Vec::new();
    for p in &grid.images {
        for v in p.views() {
            out.push((p.id, v.z, v.x, v.y, v.w, v.h, [v.clip_left, v.clip_top, v.clip_right, v.clip_bottom]));
        }
    }
    out
}

/// The cell images alone, as (image id, x, y, w, h, clip).
fn cells(grid: &crate::Grid) -> Vec<(u32, i64, i64, i64, i64, [i64; 4])> {
    drawn(grid).into_iter().filter(|d| d.1 == -1).map(|(id, _, x, y, w, h, c)| (id, x, y, w, h, c)).collect()
}

/// The spec's 2x2 placeholder for image 42, at the top left.
fn spec_2x2() -> String {
    format!("\x1b[38;5;42m{P}{}{}{P}{}{}\r\n{P}{}{}{P}{}{}\x1b[39m", D[0], D[0], D[0], D[1], D[1], D[0], D[1], D[1])
}

#[test]
fn u_parses_as_kitty_reads_it() {
    for (keys, want) in [("U=1", Some(true)), ("U=0", Some(false)), ("U=2", Some(true)), ("U=00", Some(false))] {
        assert_eq!(Command::parse(keys.as_bytes()).map(|c| c.virtual_put), want, "{keys}");
    }
    for keys in ["U=", "U=-1", "U=x", "U=1,U=1", "U=4294967296"] {
        assert!(Command::parse(keys.as_bytes()).is_none(), "{keys}");
    }
}

#[test]
fn a_virtual_placement_is_not_drawn_and_moves_no_cursor() {
    for put in [apc(&format!("a=T,i=42,U=1,c=2,r=2,{PIXEL}")), apc(&format!("a=t,i=42,{PIXEL}")) + &apc("a=p,i=42,U=1,c=2,r=2")]
    {
        let mut g = Graphics::default();
        let mut advance = None;
        for cmd in put.split("\x1b\\").filter(|c| !c.is_empty()) {
            advance = g.command(&cmd.as_bytes()[3..], 3, 2, (10, 20), 6);
        }
        assert_eq!(advance, None);
        assert_eq!(g.placements.len(), 1);
        assert!(g.placements[0].is_virtual && g.placements[0].slices.is_empty());
        assert_eq!((g.placements[0].cols, g.placements[0].rows), (2, 2));
        let grid = replay(&format!("\x1b[3;4H{put}A"));
        assert_eq!(grid.cells[2 * 10 + 3].ch, u32::from(b'A'), "the cursor stays");
        assert!(grid.images.is_empty(), "nothing is drawn without placeholders");
        // Nor does the preflight load fonts for one.
        assert!(!needs_cell_metrics(put.as_bytes()));
    }
    // A crop that would be empty still makes a virtual placement.
    let grid = replay(&format!("{}{}", apc(&format!("a=T,i=1,U=1,x=5,{PIXEL}")), format!("\x1b[38;5;1m{P}")));
    assert_eq!(cells(&grid).len(), 1);
    // A virtual placement cannot also be a relative one (kitty's EINVAL).
    let mut g = Graphics::default();
    g.command(format!("a=T,i=1,C=1,{PIXEL}").as_bytes(), 0, 0, (10, 20), 6);
    assert_eq!(g.command(format!("a=T,i=2,U=1,P=1,{PIXEL}").as_bytes(), 0, 0, (10, 20), 6), None);
    assert!(g.command(b"a=p,i=1,p=2,U=1,P=1", 0, 0, (10, 20), 6).is_none());
    assert_eq!(g.placements.len(), 1);
    assert!(!g.placements[0].is_virtual);
}

#[test]
fn the_spec_2x2_placeholder_shows_the_image_in_its_four_cells() {
    // A 1x1 image in a 2x2 box of 20x40 pixels is fitted to the width,
    // 20x20, and centered down: 10 pixels from the box's top.
    let grid = replay(&format!("{}{}", apc(&format!("a=T,q=2,i=42,U=1,c=2,r=2,{PIXEL}")), spec_2x2()));
    assert_eq!(
        cells(&grid),
        [(42, 0, 10, 20, 20, [0, 10, 20, 20]), (42, 0, 10, 20, 20, [0, 20, 20, 30])],
        "one run a row, each cut to its row"
    );
    // The same image elsewhere: the box moves with the cells.
    let grid = replay(&format!("{}\x1b[3;5H{}", apc(&format!("a=T,i=42,U=1,c=2,r=2,{PIXEL}")), spec_2x2().replace("\r\n", "\r\n\x1b[4C")));
    assert_eq!(
        cells(&grid),
        [(42, 40, 50, 20, 20, [40, 50, 60, 60]), (42, 40, 50, 20, 20, [40, 60, 60, 70])]
    );
}

#[test]
fn the_ids_come_from_the_colours_and_the_third_diacritic() {
    // 33554474 = 42 + (2 << 24), as in the spec.
    let image = |id: u32| apc(&format!("a=T,i={id},U=1,c=1,r=1,{PIXEL}"));
    let one = (0, 5, 10, 10, [0, 5, 10, 15]);
    let at = |log: String| cells(&replay(&log)).into_iter().map(|c| c.0).collect::<Vec<_>>();
    // A palette colour is its index, not the RGB it stands for.
    assert_eq!(at(format!("{}\x1b[38;5;42m{P}", image(42))), [42]);
    assert_eq!(at(format!("{}\x1b[38:5:42m{P}", image(42))), [42]);
    assert_eq!(at(format!("{}\x1b[38;5;42m{P}", image(0x00d787))), []);
    // SGR 30-37 and 90-97 are palette colours 0-15.
    assert_eq!(at(format!("{}\x1b[31m{P}", image(1))), [1]);
    assert_eq!(at(format!("{}\x1b[95m{P}", image(13))), [13]);
    // 24-bit colours are 0xRRGGBB, in every form.
    for sgr in ["38;2;1;2;3", "38:2:1:2:3", "38:2::1:2:3", "38:2:0:1:2:3"] {
        assert_eq!(at(format!("{}\x1b[{sgr}m{P}", image(0x010203))), [0x010203], "{sgr}");
    }
    // The third diacritic, less one, is the high byte.
    let log = format!("{}\x1b[38;5;42m{P}{}{}{}", image(33554474), D[0], D[0], D[2]);
    assert_eq!(cells(&replay(&log)), [(33554474, one.0, one.1, one.2, one.3, one.4)]);
    assert_eq!(at(format!("{}\x1b[38;2;0;0;42m{P}{}{}{}", image(42 + (1 << 24)), D[0], D[0], D[1])), [42 + (1 << 24)]);
    // A U+0305 third diacritic is a high byte of 0.
    assert_eq!(at(format!("{}\x1b[38;5;42m{P}{}{}{}", image(42), D[0], D[0], D[0])), [42]);
    // The default foreground with no high byte is id 0, which names nothing,
    // even an image without an id that has a virtual placement.
    assert_eq!(at(format!("{}{P}", apc(&format!("a=T,U=1,{PIXEL}")))), []);
    assert_eq!(at(format!("{}{P}{}{}{}", image(1 << 24), D[0], D[0], D[1])), [1 << 24]);
    // Neither SGR 39 nor SGR 0 leaves an id; reverse video changes none.
    assert_eq!(at(format!("{}\x1b[38;5;42;39m{P}\x1b[38;5;42;0m{P}", image(42))), []);
    assert_eq!(at(format!("{}\x1b[38;5;42;7m{P}", image(42))), [42]);
    // A saved cursor keeps the ids.
    assert_eq!(at(format!("{}\x1b[38;5;42m\x1b7\x1b[0m\x1b8{P}", image(42))), [42]);
}

#[test]
fn the_underline_colour_names_the_virtual_placement() {
    // Image 7 has two virtual placements: p=1 is 1x1 and p=2 is 2x1, made
    // after it. A 2x1 box shows the 1x1 image 10x10, centered across.
    let log = |sgr: &str| {
        format!(
            "{}{}{}\x1b[38;5;7{sgr}m{P}{}{}",
            apc(&format!("a=t,i=7,{PIXEL}")),
            apc("a=p,i=7,p=1,U=1,c=1,r=1"),
            apc("a=p,i=7,p=2,U=1,c=2,r=1"),
            D[0],
            D[0]
        )
    };
    let one = (7, 0, 5, 10, 10, [0, 5, 10, 15]);
    // 1x1 in a 2x1 box (20x20) fills it: 20x20, cut to the one cell.
    let two = (7, 0, 0, 20, 20, [0, 0, 10, 20]);
    for (sgr, want) in [
        ("", one),
        (";58;5;1", one),
        (";58;5;2", two),
        (";58:5:2", two),
        (";58;2;0;0;2", two),
        (";58:2::0:0:2", two),
        (";58;5;2;59", one),
    ] {
        assert_eq!(cells(&replay(&log(sgr))), [want], "{sgr}");
    }
    // Putting p=1 again keeps its age: it is still the oldest.
    let moved = format!("{}{}", log("").split("\x1b[38").next().unwrap(), apc("a=p,i=7,p=1,U=1,c=2,r=1"));
    assert_eq!(cells(&replay(&format!("{moved}\x1b[38;5;7m{P}{}{}", D[0], D[0]))), [two]);
    // No virtual placement with that id: nothing.
    assert_eq!(cells(&replay(&log(";58;5;3"))), []);
    // An ordinary placement with that id is not a virtual one.
    let log = format!("{}{}\x1b[2;1H\x1b[38;5;7;58;5;3m{P}", apc(&format!("a=t,i=7,{PIXEL}")), apc("a=p,i=7,p=3,C=1"));
    assert_eq!(cells(&replay(&log)), []);
    // Nor is the underline colour drawn: it differs from the foreground
    // only in the ids.
    let look = |log: &str| {
        let c = replay(log).cells[0];
        (c.ch, c.fr, c.fg, c.fb, c.br, c.bg, c.bb, c.attrs)
    };
    assert_eq!(look("\x1b[4;38;5;7;58;5;3mA"), look("\x1b[4;38;5;7mA"));
    assert_eq!(look("\x1b[4;58:2::1:2:3mA"), look("\x1b[4mA"));
}

#[test]
fn missing_diacritics_are_inherited_from_the_left() {
    // A 1x1 image in a 3x2 box of 30x40 is 30x30, 5 pixels down.
    let image = apc(&format!("a=T,i=42,U=1,c=3,r=2,{PIXEL}"));
    let box_at = |left: i64, top: i64| (42, left, top + 5, 30, 30);
    // The spec's 2 rows by 3 columns with only the first column's rows.
    let log = format!("{image}\x1b[38;5;42m{P}{}{P}{P}\r\n{P}{}{P}{P}", D[0], D[1]);
    let (id, x, y, w, h) = box_at(0, 0);
    assert_eq!(cells(&replay(&log)), [(id, x, y, w, h, [0, 5, 30, 20]), (id, x, y, w, h, [0, 20, 30, 35])]);
    // No diacritics at all: row 0 from column 0, on each row alike.
    let log = format!("{image}\x1b[38;5;42m{P}{P}{P}\r\n{P}{P}{P}");
    assert_eq!(cells(&replay(&log)), [(id, x, y, w, h, [0, 5, 30, 20]), (id, x, 25, w, h, [0, 25, 30, 40])]);
    // Only the row: the column follows the cell to the left; a later column
    // diacritic must be the next column to continue the run.
    let log = format!("{image}\x1b[38;5;42m{P}{}{}{P}{}{P}{}{}", D[0], D[1], D[0], D[0], D[3]);
    // The run is box columns 1-3 on screen columns 0-2: the box starts a
    // cell left of the screen, and column 3 is past it.
    assert_eq!(cells(&replay(&log)), [(42, -10, 5, 30, 30, [0, 5, 20, 20])]);
    // A column that is not the next one breaks it: column 2 again.
    let log = format!("{image}\x1b[38;5;42m{P}{}{}{P}{}{P}{}{}", D[0], D[1], D[0], D[0], D[2]);
    assert_eq!(
        cells(&replay(&log)),
        [(42, -10, 5, 30, 30, [0, 5, 20, 20]), (42, 0, 5, 30, 30, [20, 5, 30, 20])]
    );
}

#[test]
fn a_cell_that_disagrees_with_the_left_starts_a_run() {
    let image = apc(&format!("a=T,i=42,U=1,c=3,r=2,{PIXEL}"))
        + &apc(&format!("a=T,i=43,U=1,c=3,r=2,{PIXEL}"));
    let starts = |cells_log: &str| {
        let grid = replay(&format!("{image}{cells_log}"));
        cells(&grid).into_iter().map(|(id, x, y, _, _, clip)| (id, x, y, clip[0], clip[2])).collect::<Vec<_>>()
    };
    // A box at screen column `col` showing box columns from `first` on row 0.
    let run = |id, col: i64, first: i64, len: i64| (id, (col - first) * 10, 5, col * 10, (col + len) * 10);
    // A column that is not the next one.
    assert_eq!(starts(&format!("\x1b[38;5;42m{P}{}{}{P}{}{}", D[0], D[0], D[0], D[2])), [run(42, 0, 0, 1), run(42, 1, 2, 1)]);
    // Another row: (1, 0) here, a row down in the box.
    let r = starts(&format!("\x1b[38;5;42m{P}{}{}{P}{}", D[0], D[0], D[1]));
    assert_eq!(r, [run(42, 0, 0, 1), (42, 10, -15, 10, 20)]);
    // Another foreground, or another underline colour.
    assert_eq!(starts(&format!("\x1b[38;5;42m{P}\x1b[38;5;43m{P}")), [run(42, 0, 0, 1), run(43, 1, 0, 1)]);
    assert_eq!(starts(&format!("\x1b[38;5;42m{P}\x1b[58;5;9m{P}")), [run(42, 0, 0, 1)]);
    // No high byte inherits it; another one, 1 here, names image
    // 42 + (1 << 24), which does not exist.
    assert_eq!(starts(&format!("\x1b[38;5;42m{P}{}{}{}{P}{}", D[0], D[0], D[0], D[0])), [run(42, 0, 0, 2)]);
    assert_eq!(starts(&format!("\x1b[38;5;42m{P}{}{}{}{P}{}{}{}", D[0], D[0], D[0], D[0], D[1], D[1])), [run(42, 0, 0, 1)]);
    // A gap: an ordinary character, a blank cell, or a cell from the next row.
    for gap in ["x", "\x1b[C"] {
        assert_eq!(
            starts(&format!("\x1b[38;5;42m{P}{gap}{P}")),
            [run(42, 0, 0, 1), run(42, 2, 0, 1)],
            "{gap:?}"
        );
    }
    let log = format!("\x1b[38;5;42m\x1b[1;10H{P}{P}");
    assert_eq!(starts(&log), [run(42, 9, 0, 1), (42, 0, 25, 0, 10)]);
    // An unrelated mark in a diacritic's place counts as none.
    assert_eq!(starts(&format!("\x1b[38;5;42m{P}{}{P}\u{301}", D[0])), [run(42, 0, 0, 2)]);
}

#[test]
fn the_image_is_fitted_into_the_box_as_kitty_letterboxes_it() {
    let shown = |image: &str, keys: &str, line: &str| {
        cells(&replay(&format!("{}\x1b[38;5;1m{line}", apc(&format!("a=T,i=1,U=1,{keys},{image}")))))
    };
    // 2x1 in a 2x2 box (20x40): fitted to the width, 20x10, 15 down.
    assert_eq!(shown(WIDE, "c=2,r=2", &format!("{P}{P}")), [(1, 0, 15, 20, 10, [0, 15, 20, 20])]);
    // 1x2 in a 2x1 box (20x20): fitted to the height, 10x20, 5 across.
    assert_eq!(shown(TALL, "c=2,r=1", &format!("{P}{P}")), [(1, 5, 0, 10, 20, [5, 0, 15, 20])]);
    // Exactly the box's shape: it fills it.
    assert_eq!(shown(TALL, "c=1,r=1", P), [(1, 0, 0, 10, 20, [0, 0, 10, 20])]);
    // The whole image, whatever the crop and offsets of the placement.
    assert_eq!(shown(WIDE, "c=2,r=2,x=1,w=1,X=3,Y=4", &format!("{P}{P}")), [(1, 0, 15, 20, 10, [0, 15, 20, 20])]);
    // Without c or r, the image's own size in cells: 1x1 for a 2x1 image.
    assert_eq!(shown(WIDE, "", P), [(1, 0, 7, 10, 5, [0, 7, 10, 12])]);
    // c alone keeps the image's rows: a 2x1 image, c=4: a 40x20 box.
    assert_eq!(shown(WIDE, "c=4", &format!("{P}{P}{P}{P}")), [(1, 0, 0, 40, 20, [0, 0, 40, 20])]);
    // r alone keeps its columns: a 1x2 image, r=3: a 10x60 box, 10x20
    // image, 20 down, in the box's middle row only.
    let line = format!("{P}\r\n{P}{}\r\n{P}{}", D[1], D[2]);
    assert_eq!(shown(TALL, "r=3", &line), [(1, 0, 20, 10, 20, [0, 20, 10, 40])]);
    // Cells of the box the image does not reach show nothing.
    assert_eq!(shown(WIDE, "c=2,r=3", &format!("{P}{P}")), [], "the top row of a 20x60 box over a 20x10 image");
    // Cells past the box show nothing either: one run, cut to the image.
    assert_eq!(shown(PIXEL, "c=1,r=1", &format!("{P}{P}")), [(1, 0, 5, 10, 10, [0, 5, 10, 15])]);
}

#[test]
fn placeholders_are_text_the_image_follows() {
    let image = apc(&format!("a=T,i=42,U=1,c=2,r=2,{PIXEL}"));
    let two = |top: i64| [(42, 0, top + 10, 20, 20, [0, top + 10, 20, top + 20]), (42, 0, top + 10, 20, 20, [0, top + 20, 20, top + 30])];
    // Scrolling up a row, and a row down.
    let grid = replay(&format!("{image}\x1b[2H{}\x1b[S", spec_2x2()));
    assert_eq!(cells(&grid), two(0));
    let grid = replay(&format!("{image}{}\x1b[T", spec_2x2()));
    assert_eq!(cells(&grid), two(20));
    // Scrolled off: the top row goes; the second row shows its own part.
    let grid = replay(&format!("{image}{}\x1b[S", spec_2x2()));
    assert_eq!(cells(&grid), [(42, 0, -10, 20, 20, [0, 0, 20, 10])]);
    // Erased (EL), overwritten, deleted (DCH) or inserted (ICH) over.
    let grid = replay(&format!("{image}{}\x1b[1;1H\x1b[K", spec_2x2()));
    assert_eq!(cells(&grid), [two(0)[1]]);
    let grid = replay(&format!("{image}{}\x1b[1;2Hx", spec_2x2()));
    assert_eq!(cells(&grid), [(42, 0, 10, 20, 20, [0, 10, 10, 20]), two(0)[1]]);
    let grid = replay(&format!("{image}{}\x1b[1;1H\x1b[P", spec_2x2()));
    assert_eq!(cells(&grid), [(42, -10, 10, 20, 20, [0, 10, 10, 20]), two(0)[1]], "box column 1 moved to screen column 0");
    let grid = replay(&format!("{image}{}\x1b[1;1H\x1b[3@", spec_2x2()));
    assert_eq!(cells(&grid), [(42, 30, 10, 20, 20, [30, 10, 50, 20]), two(0)[1]]);
    // ED 2 erases the cells but keeps the virtual placement: printing them
    // again shows the image.
    let grid = replay(&format!("{image}{}\x1b[2J", spec_2x2()));
    assert_eq!(cells(&grid), []);
    let grid = replay(&format!("{image}\x1b[2J\x1b[H{}", spec_2x2()));
    assert_eq!(cells(&grid), two(0));
    // RIS too, as kitty's reset clears both screens' images with grman_clear.
    let grid = replay(&format!("{image}\x1bc{}", spec_2x2()));
    assert_eq!(cells(&grid), two(0));
    let grid = replay(&format!("\x1b[?1049h{image}\x1bc\x1b[?1049h{}", spec_2x2()));
    assert_eq!(cells(&grid), two(0), "the alternate screen's after a reset");
    // The alternate screen: its own cells and images. The main screen's
    // placeholders are hidden while it shows and come back after.
    let main = format!("{image}{}", spec_2x2());
    assert_eq!(cells(&replay(&format!("{main}\x1b[?1049h"))), []);
    assert_eq!(cells(&replay(&format!("{main}\x1b[?1049h{}", spec_2x2()))), [], "image 42 is the main screen's");
    assert_eq!(cells(&replay(&format!("{main}\x1b[?1049h\x1b[?1049l"))), two(0));
    let alt = format!("\x1b[?1049h{image}\x1b[?1049l\x1b[?1049h{}", spec_2x2());
    assert_eq!(cells(&replay(&alt)), two(0), "entering again clears the alternate screen but keeps virtual placements");
    let alt = format!("\x1b[?1047h{image}\x1b[?1047l\x1b[?1047h{}", spec_2x2());
    assert_eq!(cells(&replay(&alt)), two(0), "so does leaving 1047");
    // REP repeats a placeholder with its marks and the pen's colour.
    // A 1x1 image fills a 2x1 box: one run of both cells.
    let grid = replay(&format!("{}\x1b[38;5;42m{P}{}\x1b[b", apc(&format!("a=T,i=42,U=1,c=2,r=1,{PIXEL}")), D[0]));
    assert_eq!(cells(&grid), [(42, 0, 0, 20, 20, [0, 0, 20, 20])]);
}

#[test]
fn only_i_n_and_r_delete_a_virtual_placement() {
    let base = format!("{}\x1b[2;1H", apc(&format!("a=T,i=42,U=1,c=2,r=2,{PIXEL}")));
    let shows = |delete: &str| !cells(&replay(&format!("{base}{}\x1b[H{}", apc(delete), spec_2x2()))).is_empty();
    for keys in [
        "a=d", "a=d,d=a", "a=d,d=A", "a=d,d=c", "a=d,d=C", "a=d,d=p,x=1,y=1", "a=d,d=P,x=1,y=1", "a=d,d=q,x=1,y=1",
        "a=d,d=x,x=1", "a=d,d=X,x=1", "a=d,d=y,y=1", "a=d,d=Y,y=1", "a=d,d=z", "a=d,d=Z", "a=d,d=i,i=41", "a=d,d=r,x=1,y=41",
    ] {
        assert!(shows(keys), "{keys} keeps it");
    }
    for keys in ["a=d,d=i,i=42", "a=d,d=I,i=42", "a=d,d=r,x=42,y=42", "a=d,d=R,x=1,y=99"] {
        assert!(!shows(keys), "{keys} deletes it");
    }
    // Lowercase keeps the image: a new virtual placement shows it again.
    let again = |delete: &str| {
        let log = format!("{base}{}{}\x1b[H{}", apc(delete), apc("a=p,i=42,U=1,c=2,r=2"), spec_2x2());
        !cells(&replay(&log)).is_empty()
    };
    assert!(again("a=d,d=i,i=42"));
    assert!(!again("a=d,d=I,i=42"));
    // By number, and by placement id.
    let numbered = format!("{}{}", apc(&format!("a=T,I=5,U=1,p=3,c=2,r=2,{PIXEL}")), spec_2x2().replace("42", "1"));
    assert_eq!(cells(&replay(&numbered)).len(), 2, "a numbered image gets id 1");
    assert_eq!(cells(&replay(&format!("{numbered}{}", apc("a=d,d=n,I=5")))).len(), 0);
    assert_eq!(cells(&replay(&format!("{numbered}{}", apc("a=d,d=i,i=1,p=4")))).len(), 2);
    assert_eq!(cells(&replay(&format!("{numbered}{}", apc("a=d,d=i,i=1,p=3")))).len(), 0);
    // Retransmitting the image replaces its placements, virtual ones too.
    assert_eq!(cells(&replay(&format!("{numbered}{}", apc(&format!("a=t,i=1,{PIXEL}"))))).len(), 0);
}

#[test]
fn a_virtual_placement_counts_as_a_placement() {
    // A stored image with only a virtual placement is not freed by a=d,d=A,
    // nor by the storage quota's first pass, which frees unplaced images.
    let mut g = Graphics::default();
    g.command(format!("a=T,i=1,U=1,{PIXEL}").as_bytes(), 0, 0, (10, 20), 6);
    g.command(b"a=d,d=A", 0, 0, (10, 20), 6);
    assert_eq!(g.images.len(), 1);
    g.free_unplaced(|_| true);
    assert_eq!(g.images.len(), 1);
    // An image without an id lives while its virtual placement does.
    let mut g = Graphics::default();
    g.command(format!("a=T,U=1,{PIXEL}").as_bytes(), 0, 0, (10, 20), 6);
    assert_eq!((g.images.len(), g.placements.len()), (1, 1));
    // The 1,024-placement limit counts it.
    let mut g = Graphics::default();
    g.command(format!("a=t,i=1,{PIXEL}").as_bytes(), 0, 0, (10, 20), 6);
    for p in 1..=MAX_PLACEMENTS as u32 {
        g.command(format!("a=p,i=1,p={p},U=1").as_bytes(), 0, 0, (10, 20), 6);
    }
    assert_eq!(g.placements.len(), MAX_PLACEMENTS);
    g.command(b"a=p,i=1,p=5000,U=1", 0, 0, (10, 20), 6);
    g.command(b"a=p,i=1,C=1", 0, 0, (10, 20), 6);
    assert_eq!(g.placements.len(), MAX_PLACEMENTS);
    // Moving one keeps the count, and can turn it into an ordinary one.
    g.command(b"a=p,i=1,p=7,C=1", 0, 0, (10, 20), 6);
    assert_eq!(g.placements.iter().filter(|p| !p.is_virtual).count(), 1);
    // Scrolling never touches virtual placements.
    g.scroll(0, 5, -6, 20);
    assert_eq!(g.placements.len(), MAX_PLACEMENTS - 1);
}

#[test]
fn placeholders_and_ordinary_placements_of_one_image_mix() {
    // Image 1 is put at z=-1 and z=0 too, before and after its virtual
    // placement. Draw order: z, then image, then placement creation, with
    // the placeholder images made last, when the screen is drawn.
    let log = format!(
        "{}\x1b[4;5H{}{}{}\x1b[H\x1b[38;5;1m{P}{}{}",
        apc(&format!("a=t,i=1,{PIXEL}")),
        apc("a=p,i=1,p=1,z=-1,C=1"),
        apc("a=p,i=1,p=2,U=1"),
        apc("a=p,i=1,p=3,C=1"),
        D[0],
        D[0]
    );
    let grid = replay(&log);
    // The placeholder's box is the image's size in cells, 1x1: 10x10, 5 down.
    assert_eq!(
        drawn(&grid),
        [
            (1, -1, 40, 60, 1, 1, [i64::MIN, 60, i64::MAX, 61]),
            (1, -1, 0, 5, 10, 10, [0, 5, 10, 15]),
            (1, 0, 40, 60, 1, 1, [i64::MIN, 60, i64::MAX, 61]),
        ]
    );
}

#[test]
fn relative_placements_under_a_virtual_one() {
    let image = apc(&format!("a=T,i=42,U=1,c=2,r=2,{PIXEL}"));
    // A child of the virtual placement, one cell right and down of it.
    let child = apc(&format!("a=T,i=2,P=42,H=1,V=1,c=1,r=1,{PIXEL}"));
    let at = |log: &str| {
        let grid = replay(log);
        drawn(&grid).into_iter().filter(|d| d.0 == 2).map(|d| (d.2, d.3)).collect::<Vec<_>>()
    };
    // Its parent is where the image shows: the top row and leftmost
    // column of the cells that show part of it. Box at (3, 2): the child's
    // 1x1 image is 10x10 in its cell (4, 3), 5 down.
    let shown = format!("\x1b[3;4H{}", spec_2x2().replace("\r\n", "\r\n\x1b[3C"));
    assert_eq!(at(&format!("{image}{child}{shown}")), [(40, 65)]);
    // The cursor stayed for the child; the placeholders went where it was.
    assert_eq!(at(&format!("{image}\x1b[3;4H{child}{}", spec_2x2().replace("\r\n", "\r\n\x1b[3C"))), [(40, 65)]);
    // Without placeholders it is not drawn, but stays: printed later, they
    // bring it.
    assert_eq!(at(&format!("{image}{child}")), []);
    // Only the cells that show the image count: a 2x1 image in a 2x2 box
    // is 20x10 in the middle, so the first row of a 2x3 box shows nothing.
    let tall_box = apc(&format!("a=T,i=42,U=1,c=2,r=3,{WIDE}"));
    let rows3 = format!(
        "\x1b[38;5;42m{P}{}{P}\r\n{P}{}{P}\r\n{P}{}{P}",
        D[0], D[1], D[2]
    );
    assert_eq!(at(&format!("{tall_box}{child}{rows3}")), [(10, 45)], "from row 1, not 0");
    // Columns and rows are found separately: the top row from one run, the
    // leftmost column from another.
    let split = format!("\x1b[38;5;42m\x1b[1;6H{P}{}{}\x1b[2;3H{P}{}{}", D[0], D[1], D[1], D[0]);
    assert_eq!(at(&format!("{image}{child}{split}")), [(30, 25)], "row 0 from (0, 5), column 2 from (1, 2)");
    // A chain: its child follows, with both offsets.
    let grandchild = apc(&format!("a=T,i=3,P=2,H=2,V=-1,c=1,r=1,{PIXEL}"));
    let grid = replay(&format!("{image}{child}{grandchild}{shown}"));
    let three: Vec<_> = drawn(&grid).into_iter().filter(|d| d.0 == 3).map(|d| (d.2, d.3)).collect();
    assert_eq!(three, [(60, 45)]);
    // Deleting the virtual placement deletes the children.
    let log = format!("{image}{child}{grandchild}{}{shown}", apc("a=d,d=i,i=42"));
    assert!(replay(&log).images.is_empty());
    // A cell selector does not reach a child of a virtual placement, which
    // has no cells while the log plays; d=a and d=z do, as in kitty.
    for (keys, kept) in [("a=d,d=c", true), ("a=d,d=p,x=5,y=4", true), ("a=d,d=x,x=1", true), ("a=d", false), ("a=d,d=z", false)] {
        let log = format!("{image}\x1b[H{child}{}{shown}", apc(keys));
        assert_eq!(!at(&log).is_empty(), kept, "{keys}");
    }
    // ED 2 removes the child, not its virtual parent.
    let log = format!("{image}{child}\x1b[2J{shown}");
    assert_eq!(at(&log), []);
    assert_eq!(cells(&replay(&log)).len(), 2);
    // Scrolling does not move or remove it; its parent's cells do.
    let log = format!("{image}{child}{shown}\x1b[2S");
    assert_eq!(at(&log), [(40, 25)]);
    let log = format!("{image}{child}\x1b[6S{shown}");
    assert_eq!(at(&log), [(40, 65)]);
    // A put of the virtual placement again keeps its children.
    let log = format!("{image}{child}{}{shown}", apc("a=p,i=42,U=1,c=2,r=2"));
    assert_eq!(at(&log), [(40, 65)]);
    // Made ordinary, it is an ordinary parent again: 1x1 at (5, 3), so the
    // child is at (6, 4).
    let named = apc(&format!("a=T,i=42,p=1,U=1,c=2,r=2,{PIXEL}"));
    let log = format!("{named}{child}\x1b[4;6H{}", apc("a=p,i=42,p=1,C=1"));
    assert_eq!(at(&log), [(60, 85)]);
    // An ordinary put without its placement id is another placement.
    let log = format!("{image}{child}\x1b[4;6H{}", apc("a=p,i=42,C=1"));
    assert_eq!(at(&log), []);
}

#[test]
fn text_and_json_keep_the_placeholders_as_code_points() {
    let grid = replay(&format!("{}{}", apc(&format!("a=T,i=42,U=1,c=2,r=2,{PIXEL}")), spec_2x2()));
    let text = crate::grid_text(&grid.cells, &grid.marks, 10);
    let want = format!("{P}{}{}{P}{}{}\n{P}{}{}{P}{}{}\n", D[0], D[0], D[0], D[1], D[1], D[0], D[1], D[1]);
    assert!(text.starts_with(&want), "{text:?}");
}

#[test]
fn the_diacritics_are_kittys_and_combine_as_marks() {
    use crate::rowcolumn_diacritics::DIACRITICS;
    assert_eq!(DIACRITICS.len(), 297);
    for (n, d) in D.iter().enumerate() {
        assert_eq!(diacritic(d.chars().next().unwrap() as u32), n as u32 + 1);
    }
    assert_eq!(DIACRITICS[296], 0x1D244);
    assert_eq!(diacritic(0x1D244), 297);
    assert_eq!(diacritic(0x301), 0);
    assert_eq!(diacritic(0), 0);
    assert!(DIACRITICS.windows(2).all(|w| w[0] < w[1]));
    // Every one takes no cell, composes with nothing after a placeholder,
    // and is kept as a mark, in order: 296, 0 and 2 here.
    assert_eq!(crate::unicode::width(PLACEHOLDER), 1);
    for &d in &DIACRITICS {
        assert_eq!(crate::unicode::width(d), 0, "{d:#x}");
        assert_eq!(crate::unicode::compose(PLACEHOLDER, d), None, "{d:#x}");
    }
    let mark = |d: u32| char::from_u32(d).unwrap();
    let grid = replay(&format!("{P}{}{}{}", mark(DIACRITICS[296]), mark(DIACRITICS[0]), mark(DIACRITICS[2])));
    assert_eq!(crate::marks_of(&grid.marks, 0), [DIACRITICS[296], DIACRITICS[0], DIACRITICS[2]]);
    // A high byte past 255 wraps, as kitty's 32-bit shift does: 297 - 1 = 296
    // is 40 in the top byte.
    let image = apc(&format!("a=T,i={},U=1,{PIXEL}", 1 + (40 << 24)));
    let log = format!("{image}\x1b[38;5;1m{P}{}{}{}", D[0], D[0], mark(DIACRITICS[296]));
    assert_eq!(cells(&replay(&log)).len(), 1);
}
