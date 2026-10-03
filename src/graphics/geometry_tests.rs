//! Placement geometry and layering: source crops (x, y, w, h), offsets in the
//! first cell (X, Y), the aspect ratio of a crop fitted to c and r, and the
//! signed z-index. Cells are 10x20 pixels, as in the other graphics tests.

use super::*;
use crate::{replay_sized, Lf};

/// A 4x2 RGB image: red, green, blue, yellow over cyan, magenta, white, black.
const IMAGE: &str = "f=24,s=4,v=2;/wAAAP8AAAD///8AAP///wD/////AAAA";

fn replay(s: &[u8]) -> crate::Grid {
    replay_sized(s, 20, 10, Lf::Index, (10, 20))
}

/// Transmit and place IMAGE at the cursor with these keys.
fn put(keys: &str) -> Vec<u8> {
    format!("\x1b_Ga=T,{keys},{IMAGE}\x1b\\").into_bytes()
}

/// (src, x, y, w, h) of the only placement.
fn shown(log: &[u8]) -> ([u32; 4], i64, i64, i64, i64) {
    let g = replay(log);
    assert_eq!(g.images.len(), 1, "{}", String::from_utf8_lossy(log));
    let p = &g.images[0];
    (p.src, p.x, p.slices[0].y, p.w, p.h)
}

fn layout_of(keys: &str) -> Option<Layout> {
    let cmd = Command::parse(keys.as_bytes()).unwrap();
    layout(&cmd, 4, 2, (10, 20))
}

#[test]
fn crops_are_the_intersection_with_the_image() {
    for (keys, src) in [
        ("C=1", [0, 0, 4, 2]),
        ("C=1,x=1", [1, 0, 3, 2]),
        ("C=1,x=1,w=2", [1, 0, 2, 2]),
        ("C=1,y=1,h=1", [0, 1, 4, 1]),
        ("C=1,x=1,y=1,w=1,h=1", [1, 1, 1, 1]),
        // Past the right and bottom edges, the crop stops at them.
        ("C=1,x=3,w=5", [3, 0, 1, 2]),
        ("C=1,y=1,h=4294967295", [0, 1, 4, 1]),
        ("C=1,w=4294967295,h=4294967295", [0, 0, 4, 2]),
    ] {
        let (got, x, y, w, h) = shown(&put(keys));
        // At native size the crop is drawn a pixel per source pixel.
        assert_eq!((got, x, y, w, h), (src, 0, 0, i64::from(src[2]), i64::from(src[3])), "{keys}");
    }
}

#[test]
fn an_empty_crop_draws_nothing_but_still_moves_the_cursor() {
    for keys in ["x=4", "y=2", "x=4294967295", "x=2,y=5,w=1"] {
        let g = replay(&put(keys));
        assert!(g.images.is_empty(), "{keys}");
        // No pixels, no cells: the cursor stays.
        assert_eq!(g.cursor, Some((0, 0)), "{keys}");
    }
    // The space c and r give is still passed.
    let g = replay(&put("x=4,c=3,r=2"));
    assert!(g.images.is_empty());
    assert_eq!(g.cursor, Some((2, 3)));
    // An offset is a cell the image would start in.
    let g = replay(&put("x=4,X=3"));
    assert_eq!(g.cursor, Some((0, 1)));
    // It replaces the placement it names, which leaves nothing.
    let mut log = put("i=1,p=1,C=1");
    log.extend_from_slice(b"\x1b_Ga=p,i=1,p=1,x=9,C=1\x1b\\");
    let g = replay(&log);
    assert!(g.images.is_empty());
    // The image is still stored: a later put shows it.
    log.extend_from_slice(b"\x1b_Ga=p,i=1,p=1,C=1\x1b\\");
    assert_eq!(replay(&log).images.len(), 1);
    // An anonymous image with only an empty placement is not kept.
    let mut g = Graphics::default();
    g.command(format!("a=T,x=4,{IMAGE}").as_bytes(), 0, 0, (10, 20), 10);
    assert!(g.images.is_empty() && g.placements.is_empty());
}

#[test]
fn offsets_start_the_image_inside_its_first_cell() {
    assert_eq!(shown(&put("X=3,Y=5")), ([0, 0, 4, 2], 3, 5, 4, 2));
    // It still fits in its cell, which the cursor passes.
    let g = replay(&put("X=3,Y=5"));
    assert_eq!(g.cursor, Some((1, 1)));
    // At most a pixel short of the cell's edge, as kitty clamps them.
    assert_eq!(shown(&put("X=9,Y=19")), ([0, 0, 4, 2], 9, 19, 4, 2));
    assert_eq!(shown(&put("X=10,Y=20")), ([0, 0, 4, 2], 9, 19, 4, 2));
    assert_eq!(shown(&put("X=4294967295,Y=4294967295")), ([0, 0, 4, 2], 9, 19, 4, 2));
    // The image runs into the next cells, which the cursor passes.
    let g = replay(&put("X=9,Y=19"));
    assert_eq!(g.cursor, Some((2, 2)));
    let p = &g.images[0];
    assert_eq!((p.col, p.cols, p.row, p.rows), (0, 2, 0, 2));
    // Placed at another cell, the offset is from that cell's corner.
    let mut log = b"\x1b[3;4H".to_vec();
    log.extend(put("X=2,Y=7,C=1"));
    assert_eq!(shown(&log), ([0, 0, 4, 2], 32, 47, 4, 2));
}

#[test]
fn offsets_shrink_the_cells_c_and_r_give() {
    // c=2 is 20 pixels from the cell's edge, so 16 after X=4: the 4x2 image
    // at 16x8, and c, not more, for the cursor.
    let l = layout_of("c=2,X=4").unwrap();
    assert_eq!((l.x, l.y, l.w, l.h, l.cols, l.rows), (4, 0, 16, 8, 2, 1));
    // r=1 is 20 pixels, 15 after Y=5: 30x15, over cells 0..=2.
    let l = layout_of("r=1,Y=5").unwrap();
    assert_eq!((l.x, l.y, l.w, l.h, l.cols, l.rows), (0, 5, 30, 15, 3, 1));
    // Both: a 16x15 box, the image fitted to its width and centered.
    let l = layout_of("c=2,r=1,X=4,Y=5").unwrap();
    assert_eq!((l.x, l.y, l.w, l.h, l.cols, l.rows), (4, 5 + 3, 16, 8, 2, 1));
    // r only, with X: the image's width follows its height, and the cells
    // covered run from the cell to the image's far edge.
    let l = layout_of("r=1,X=9").unwrap();
    assert_eq!((l.x, l.w, l.h, l.cols), (9, 40, 20, 5));
}

#[test]
fn a_crop_keeps_its_own_aspect_ratio() {
    // The left half, a square, in 4x1 cells (40x20): 20x20, centered.
    let l = layout_of("x=0,w=2,c=4,r=1").unwrap();
    assert_eq!((l.src, l.x, l.y, l.w, l.h), ([0, 0, 2, 2], 10, 0, 20, 20));
    // The whole image in the same cells fills them.
    let l = layout_of("c=4,r=1").unwrap();
    assert_eq!((l.x, l.y, l.w, l.h), (0, 0, 40, 20));
    // One column of the image, 1x2, given r=2: 20x40.
    let l = layout_of("x=1,w=1,r=2").unwrap();
    assert_eq!((l.src, l.w, l.h, l.cols, l.rows), ([1, 0, 1, 2], 20, 40, 2, 2));
    // One row, 4x1, given c=2: 20x5.
    let l = layout_of("y=1,h=1,c=2").unwrap();
    assert_eq!((l.src, l.w, l.h, l.cols, l.rows), ([0, 1, 4, 1], 20, 5, 2, 1));
    // A 1x1 crop given c=1,r=2 (10x40): 10x10, centered vertically.
    let l = layout_of("x=3,y=1,c=1,r=2").unwrap();
    assert_eq!((l.src, l.x, l.y, l.w, l.h), ([3, 1, 1, 1], 0, 15, 10, 10));
}

#[test]
fn the_views_draw_carries_the_crop_and_the_z_index() {
    let g = replay(&put("x=1,y=1,w=2,h=1,c=2,r=1,z=-7,C=1"));
    let views: Vec<_> = g.images[0].views().collect();
    assert_eq!(views.len(), 1);
    let v = &views[0];
    assert_eq!((v.src_x, v.src_y, v.src_w, v.src_h, v.z), (1, 1, 2, 1, -7));
    assert_eq!((v.width, v.height), (4, 2));
}

#[test]
fn z_is_a_signed_32_bit_integer() {
    for (z, want) in [
        ("0", 0),
        ("-0", 0),
        ("-1", -1),
        ("2147483647", i32::MAX),
        ("-2147483648", i32::MIN),
        ("-1073741824", -1073741824),
        ("-1073741825", -1073741825),
    ] {
        let g = replay(&put(&format!("z={z},C=1")));
        assert_eq!(g.images[0].z, want, "{z}");
    }
    for z in ["2147483648", "-2147483649", "+1", "--1", "-", "1-"] {
        assert!(replay(&put(&format!("z={z}"))).images.is_empty(), "{z}");
    }
}

#[test]
fn negative_z_draws_first_and_deletes_by_its_value() {
    let mut log = put("i=1,z=0,C=1");
    log.extend(put("i=2,z=-1,C=1"));
    log.extend(put("i=3,z=-1073741825,C=1"));
    log.extend(put("i=4,z=5,C=1"));
    let order = |log: &[u8]| replay(log).images.iter().map(|p| p.id).collect::<Vec<_>>();
    assert_eq!(order(&log), [3, 2, 1, 4]);
    let mut deleted = log.clone();
    deleted.extend_from_slice(b"\x1b_Ga=d,d=z,z=-1\x1b\\");
    assert_eq!(order(&deleted), [3, 1, 4]);
    // q: a cell and a z-index.
    log.extend_from_slice(b"\x1b_Ga=d,d=q,x=1,y=1,z=-1073741825\x1b\\");
    assert_eq!(order(&log), [2, 1, 4]);
}

#[test]
fn a_put_moves_its_placement_with_new_geometry() {
    let mut log = put("i=1,p=1,C=1");
    log.extend_from_slice(b"\x1b[2;3H\x1b_Ga=p,i=1,p=1,x=2,w=2,X=1,Y=2,c=1,C=1\x1b\\");
    let g = replay(&log);
    assert_eq!(g.images.len(), 1);
    let p = &g.images[0];
    // 2x2 source in a 9-pixel-wide space: 9x9 at (20 + 1, 20 + 2).
    assert_eq!((p.src, p.x, p.slices[0].y, p.w, p.h), ([2, 0, 2, 2], 21, 22, 9, 9));
}

#[test]
fn offset_images_scroll_and_clip_as_wholes() {
    // Y=5 on row 1: pixel rows 25..29 of a 4x4 (c=1,r=1 fits 9x15: 9x4 at
    // y 5 + 5). The region is rows 1..=3.
    let mut log = b"\x1b[2;1H".to_vec();
    log.extend(put("X=1,Y=5,c=1,r=1,C=1"));
    let g = replay(&log);
    let p = &g.images[0];
    assert_eq!((p.x, p.slices[0].y, p.w, p.h), (1, 20 + 5 + 5, 9, 4));
    log.extend_from_slice(b"\x1b[2;4r\x1b[S");
    // Scrolled up one row, it leaves the region: clipped at its top.
    let g = replay(&log);
    assert!(g.images.is_empty());
    // Scrolled down one row, it moves with its offset.
    let mut log = b"\x1b[2;1H".to_vec();
    log.extend(put("X=1,Y=5,c=1,r=1,C=1"));
    log.extend_from_slice(b"\x1b[2;4r\x1b[T");
    let g = replay(&log);
    let s = g.images[0].slices[0];
    assert_eq!((s.y, s.top, s.bottom), (50, 50, 54));
    // At the bottom of the screen, the screen clips it.
    let mut log = b"\x1b[10;1H".to_vec();
    log.extend(put("Y=19,C=1"));
    let g = replay(&log);
    let s = g.images[0].slices[0];
    assert_eq!((s.y, s.top, s.bottom), (199, 199, 200));
}

#[test]
fn crops_and_offsets_are_bounded_with_extreme_metrics() {
    let mut g = Graphics::default();
    let cell = (i32::MAX, i32::MAX);
    for keys in ["X=4294967295,Y=4294967295", "c=4294967295,X=4294967295", "r=4294967295,Y=1", "x=1,w=1,c=1"] {
        g.command(format!("a=T,{keys},C=1,{IMAGE}").as_bytes(), 0, 0, cell, 1);
    }
    // Native size fits; anything scaled to these cells is over the limit.
    assert_eq!(g.placements.len(), 1);
    assert_eq!((g.placements[0].x, g.placements[0].w), (i64::from(i32::MAX) - 1, 4));
}
