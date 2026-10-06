//! Relative placements (P, Q, H, V): position from the parent's top left cell,
//! movement with the parent, deletion with it, refused parents, chains,
//! cycles and scrolling. Cells are 10x20 pixels on a 10-row screen, as in the
//! other graphics tests; semantics are kitty's (graphics.c, master).

use super::*;
use crate::{screen::Lf, vt::replay_sized};

const PIXEL: &str = "f=24,s=1,v=1;/wAA";

fn run(g: &mut Graphics, (col, row): (usize, usize), cmd: &str) -> Option<(usize, usize)> {
    g.command(cmd.as_bytes(), col, row, (10, 20), 10)
}

/// (image id, placement id) -> (col, row, x, top of the first slice or None).
fn at(g: &Graphics, id: u32, pid: u32) -> Option<(i64, i64, i64, Option<i64>)> {
    let p = g.placements.iter().find(|p| p.id == id && p.placement_id == pid)?;
    Some((p.col, p.row, p.x, p.slices.first().map(|s| s.y)))
}

fn ids(g: &Graphics) -> Vec<u32> {
    let mut ids: Vec<_> = g.images.iter().map(|img| img.id).collect();
    ids.sort_unstable();
    ids
}

/// The (image id, placement id) of each placement, sorted.
fn placed(g: &Graphics) -> Vec<(u32, u32)> {
    let mut v: Vec<_> = g.placements.iter().map(|p| (p.id, p.placement_id)).collect();
    v.sort_unstable();
    v
}

/// Store image `id` and place it as placement 1 with these keys.
fn put(g: &mut Graphics, cursor: (usize, usize), id: u32, keys: &str) -> Option<(usize, usize)> {
    if !g.images.iter().any(|img| img.id == id) {
        run(g, (0, 0), &format!("a=t,i={id},{PIXEL}"));
    }
    run(g, cursor, &format!("a=p,i={id},p=1,{keys}"))
}

#[test]
fn parent_keys_parse_as_kitty_reads_them() {
    for (keys, want) in [
        ("P=1,Q=2,H=-3,V=4", Some((1, 2, -3, 4))),
        ("P=4294967295,H=2147483647,V=-2147483648", Some((u32::MAX, 0, i32::MAX, i32::MIN))),
        ("P=0", Some((0, 0, 0, 0))),
        ("P=-1", None),
        ("Q=x", None),
        ("H=2147483648", None),
        ("V=-2147483649", None),
        ("H=+1", None),
        ("P=1,P=1", None),
    ] {
        let got = Command::parse(keys.as_bytes())
            .map(|c| (c.parent_id, c.parent_placement, c.parent_x, c.parent_y));
        assert_eq!(got, want, "{keys}");
    }
}

#[test]
fn a_child_is_placed_from_its_parents_top_left_cell_and_the_cursor_stays() {
    let mut g = Graphics::default();
    assert_eq!(put(&mut g, (2, 3), 1, "c=2,r=2"), Some((2, 2)));
    // H and V count cells from the parent's top left cell; X, Y and the
    // child's own c and r still apply inside it. The cursor never moves,
    // without C=1 too.
    assert_eq!(put(&mut g, (9, 9), 2, "P=1,Q=1,H=3,V=-1,X=4,Y=5"), None);
    assert_eq!(at(&g, 2, 1), Some((5, 2, 54, Some(45))));
    assert_eq!(g.placements.iter().find(|p| p.id == 2).unwrap().cols, 1);
    // C=0 and C=1 alike.
    assert_eq!(put(&mut g, (9, 9), 3, "P=1,Q=1,C=0"), None);
    assert_eq!(at(&g, 3, 1), Some((2, 3, 20, Some(60))));
    // a=T too.
    assert_eq!(run(&mut g, (9, 9), &format!("a=T,i=4,P=1,H=1,{PIXEL}")), None);
    assert_eq!(at(&g, 4, 0), Some((3, 3, 30, Some(60))));

    // In a replay, text after a relative put lands where the cursor was.
    let log = format!(
        "\x1b[2;3H\x1b_Ga=T,i=1,p=1,C=1,{PIXEL}\x1b\\\x1b_Ga=T,i=2,P=1,V=4,{PIXEL}\x1b\\A"
    );
    let grid = replay_sized(log.as_bytes(), 20, 10, Lf::Index, (10, 20));
    assert_eq!(grid.cells[20 + 2].ch, u32::from(b'A'));
    // The preflight knows a relative put moves no cursor.
    assert!(!needs_cell_metrics(format!("\x1b_Ga=T,i=2,P=1,{PIXEL}\x1b\\").as_bytes()));
    assert!(!needs_cell_metrics(
        format!("\x1b_Ga=t,i=2,{PIXEL}\x1b\\\x1b_Ga=p,i=2,P=1\x1b\\").as_bytes()
    ));
    assert!(needs_cell_metrics(format!("\x1b_Ga=T,i=2,P=0,{PIXEL}\x1b\\").as_bytes()));
}

#[test]
fn q_names_a_parent_placement_or_the_oldest_one() {
    let mut g = Graphics::default();
    run(&mut g, (0, 0), &format!("a=t,i=1,{PIXEL}"));
    run(&mut g, (4, 4), "a=p,i=1,p=7,C=1");
    run(&mut g, (1, 1), "a=p,i=1,p=3,C=1");
    run(&mut g, (6, 2), "a=p,i=1,C=1");
    put(&mut g, (0, 0), 2, "P=1");
    assert_eq!(at(&g, 2, 1).unwrap().0, 4, "the oldest, not the lowest id");
    put(&mut g, (0, 0), 3, "P=1,Q=3");
    assert_eq!(at(&g, 3, 1).unwrap().0, 1);
    // Moving a placement keeps its age.
    run(&mut g, (8, 8), "a=p,i=1,p=7,C=1");
    put(&mut g, (0, 0), 4, "P=1");
    assert_eq!(at(&g, 4, 1).unwrap().0, 8);
    // P=0 is no parent: a put at the cursor.
    assert_eq!(put(&mut g, (5, 5), 5, "P=0,Q=3"), Some((1, 1)));
    assert_eq!(at(&g, 5, 1).unwrap().0, 5);
}

#[test]
fn a_missing_parent_refuses_the_put() {
    for keys in ["P=9", "P=2", "P=1,Q=2", "P=1,Q=4294967295"] {
        let mut g = Graphics::default();
        put(&mut g, (0, 0), 1, "C=1");
        // Image 2 is stored without placements.
        run(&mut g, (0, 0), &format!("a=t,i=2,{PIXEL}"));
        // An existing placement named by the refused put is left alone.
        put(&mut g, (6, 6), 3, "C=1");
        let before = at(&g, 3, 1);
        assert_eq!(put(&mut g, (2, 2), 3, keys), None, "{keys}");
        assert_eq!(at(&g, 3, 1), before, "{keys}");
        assert_eq!(put(&mut g, (2, 2), 4, keys), None, "{keys}");
        assert_eq!(at(&g, 4, 1), None, "{keys}");
        // An anonymous a=T whose put is refused stores nothing.
        let images = g.images.len();
        assert_eq!(run(&mut g, (2, 2), &format!("a=T,{keys},{PIXEL}")), None);
        assert_eq!(g.images.len(), images, "{keys}");
    }
    // A parent's image retransmitted by the same a=T has no placement yet.
    let mut g = Graphics::default();
    put(&mut g, (0, 0), 1, "C=1");
    run(&mut g, (0, 0), &format!("a=T,i=1,P=1,{PIXEL}"));
    assert!(g.placements.is_empty());
}

#[test]
fn children_move_with_their_parent_through_a_chain() {
    let mut g = Graphics::default();
    put(&mut g, (1, 1), 1, "C=1");
    put(&mut g, (0, 0), 2, "P=1,Q=1,H=2,V=1");
    put(&mut g, (0, 0), 3, "P=2,Q=1,H=-1,V=3");
    assert_eq!(at(&g, 2, 1).map(|a| (a.0, a.1)), Some((3, 2)));
    assert_eq!(at(&g, 3, 1).map(|a| (a.0, a.1)), Some((2, 5)));
    // Putting the parent again moves the whole group.
    put(&mut g, (5, 0), 1, "C=1");
    assert_eq!(at(&g, 2, 1).map(|a| (a.0, a.1)), Some((7, 1)));
    assert_eq!(at(&g, 3, 1).map(|a| (a.0, a.1)), Some((6, 4)));
    // Moving the middle one with new offsets moves its child; the root stays.
    put(&mut g, (0, 0), 2, "P=1,Q=1,H=0,V=2");
    assert_eq!(at(&g, 1, 1).map(|a| (a.0, a.1)), Some((5, 0)));
    assert_eq!(at(&g, 3, 1).map(|a| (a.0, a.1)), Some((4, 5)));
    // Put without P, the middle one becomes a root at the cursor and keeps
    // its child; the cursor moves again.
    assert_eq!(put(&mut g, (0, 8), 2, ""), Some((1, 1)));
    assert_eq!(at(&g, 2, 1).map(|a| (a.0, a.1)), Some((0, 8)));
    assert_eq!(at(&g, 3, 1).map(|a| (a.0, a.1)), Some((-1, 11)));
    assert_eq!(at(&g, 3, 1).unwrap().3, None, "below the screen");
    put(&mut g, (5, 5), 1, "C=1");
    assert_eq!(at(&g, 3, 1).map(|a| (a.0, a.1)), Some((-1, 11)), "no longer 1's");
    // It can be re-parented elsewhere, with its child.
    put(&mut g, (0, 0), 2, "P=1,Q=1,H=1");
    assert_eq!(at(&g, 3, 1).map(|a| (a.0, a.1)), Some((5, 8)));
}

#[test]
fn chains_are_limited_to_eight_links() {
    // Image 1 is the root; image k + 1 is the child of image k.
    let chain = |links: u32| {
        let mut g = Graphics::default();
        put(&mut g, (0, 0), 1, "C=1");
        for k in 1..=links {
            put(&mut g, (0, 0), k + 1, &format!("P={k},Q=1,H=1"));
        }
        g
    };
    let g = chain(MAX_DEPTH as u32);
    assert_eq!(g.placements.len(), MAX_DEPTH + 1);
    assert_eq!(at(&g, MAX_DEPTH as u32 + 1, 1).unwrap().0, MAX_DEPTH as i64);
    let mut g = chain(MAX_DEPTH as u32 + 1);
    assert_eq!(g.placements.len(), MAX_DEPTH + 1, "the ninth link is refused");
    assert_eq!(at(&g, MAX_DEPTH as u32 + 2, 1), None);
    // kitty made the placement before refusing it, so its image was used;
    // a missing parent refuses the put before that.
    let atime = |g: &Graphics, id| g.images.iter().find(|i| i.id == id).unwrap().atime;
    let used = atime(&g, 10);
    put(&mut g, (0, 0), 10, "P=99");
    assert_eq!(atime(&g, 10), used);
    put(&mut g, (0, 0), 10, "P=9,Q=1");
    assert!(atime(&g, 10) > used);

    // Re-parenting is checked the same way, and a refused one leaves the
    // placement where it was.
    put(&mut g, (4, 4), 20, "C=1");
    let before = at(&g, 20, 1);
    assert_eq!(put(&mut g, (0, 0), 20, "P=9,Q=1"), None);
    assert_eq!(at(&g, 20, 1), before);
    put(&mut g, (0, 0), 20, "P=8,Q=1");
    assert_eq!(at(&g, 20, 1).unwrap().0, 7);

    // A move can leave descendants too deep: they go, as kitty's
    // grman_update_layers drops a chain it cannot resolve.
    let mut g = chain(4);
    put(&mut g, (0, 0), 20, "C=1");
    for k in 21..=24 {
        put(&mut g, (0, 0), k, &format!("P={},Q=1", k - 1));
    }
    // 20 now has five links up, so 24 would have nine.
    put(&mut g, (0, 0), 20, "P=5,Q=1");
    assert_eq!(at(&g, 23, 1).map(|a| a.0), Some(4));
    assert_eq!(at(&g, 24, 1), None);
    assert!(!ids(&g).contains(&24), "its image went with its only placement");

    // The same when the descendants draw, and so are laid out, before their
    // ancestors, the deepest first: only those too deep go.
    let mut g = chain(5);
    put(&mut g, (0, 0), 20, "C=1");
    for k in 21..=24 {
        put(&mut g, (0, 0), k, &format!("P={},Q=1,z=-{}", k - 1, k - 20));
    }
    put(&mut g, (0, 0), 20, "P=6,Q=1");
    let left: Vec<_> = placed(&g).into_iter().map(|(id, _)| id).collect();
    assert_eq!(left, [1, 2, 3, 4, 5, 6, 20, 21, 22]);
    assert_eq!(ids(&g), [1, 2, 3, 4, 5, 6, 20, 21, 22]);
}

#[test]
fn cycles_are_refused() {
    let mut g = Graphics::default();
    put(&mut g, (1, 1), 1, "C=1");
    put(&mut g, (0, 0), 2, "P=1,Q=1");
    put(&mut g, (0, 0), 3, "P=2,Q=1");
    let before: Vec<_> = (1..=3).map(|id| at(&g, id, 1)).collect();
    // A is relative to B, B to C, C to A.
    assert_eq!(put(&mut g, (0, 0), 1, "P=3,Q=1"), None);
    assert_eq!(put(&mut g, (0, 0), 1, "P=2,Q=1"), None);
    // Its own parent, by Q and as its image's oldest placement.
    assert_eq!(put(&mut g, (0, 0), 1, "P=1,Q=1"), None);
    assert_eq!(put(&mut g, (0, 0), 1, "P=1"), None);
    assert_eq!(put(&mut g, (0, 0), 2, "P=2"), None);
    let after: Vec<_> = (1..=3).map(|id| at(&g, id, 1)).collect();
    assert_eq!(before, after);
    assert_eq!(g.placements.iter().find(|p| p.id == 1).unwrap().parent, None);
    // Another placement of its own image is a valid parent.
    run(&mut g, (7, 7), "a=p,i=1,p=2,C=1");
    put(&mut g, (0, 0), 1, "P=1,Q=2,H=1");
    assert_eq!(at(&g, 1, 1).map(|a| (a.0, a.1)), Some((8, 7)));
    assert_eq!(at(&g, 3, 1).map(|a| (a.0, a.1)), Some((8, 7)));
}

/// Image 1 at cell (2, 2), its child 2 at (5, 4) with z 1, and 2's child 3
/// at (6, 4) with z 2; each image has one placement, with id 1.
fn family() -> Graphics {
    let mut g = Graphics::default();
    put(&mut g, (2, 2), 1, "C=1");
    put(&mut g, (0, 0), 2, "P=1,Q=1,H=3,V=2,z=1");
    put(&mut g, (0, 0), 3, "P=2,Q=1,H=1,z=2");
    g
}

#[test]
fn deleting_a_placement_deletes_its_descendants() {
    // (delete, cursor, placements left, images left lowercase, uppercase).
    // A child removed because its parent went frees its image whatever its
    // id, as the spec says; the images the delete itself hit follow the
    // usual lowercase and uppercase rules.
    let all: &[u32] = &[1, 2, 3];
    for (delete, cursor, left, lower, upper) in [
        ("d=a", (0, 0), &[][..], all, &[][..]),
        ("d=i,i=1", (0, 0), &[], &[1], &[]),
        ("d=i,i=1,p=1", (0, 0), &[], &[1], &[]),
        ("d=i,i=2", (0, 0), &[1], &[1, 2], &[1]),
        ("d=i,i=3", (0, 0), &[1, 2], all, &[1, 2]),
        ("d=r,x=2,y=3", (0, 0), &[1], all, &[1]),
        // Cells are where each placement is drawn.
        ("d=x,x=3", (0, 0), &[], &[1], &[]),
        ("d=x,x=6", (0, 0), &[1], &[1, 2], &[1]),
        ("d=x,x=7", (0, 0), &[1, 2], all, &[1, 2]),
        ("d=x,x=8", (0, 0), all, all, all),
        ("d=y,y=5", (0, 0), &[1], &[1, 2, 3], &[1]),
        ("d=y,y=3", (0, 0), &[], &[1], &[]),
        ("d=p,x=7,y=5", (0, 0), &[1, 2], all, &[1, 2]),
        ("d=q,x=6,y=5,z=1", (0, 0), &[1], &[1, 2], &[1]),
        ("d=q,x=6,y=5,z=2", (0, 0), all, all, all),
        ("d=z,z=0", (0, 0), &[], &[1], &[]),
        ("d=z,z=2", (0, 0), &[1, 2], all, &[1, 2]),
        ("d=c", (5, 4), &[1], &[1, 2], &[1]),
        ("d=c", (2, 2), &[], &[1], &[]),
        ("d=c", (0, 0), all, all, all),
    ] {
        for (case, images) in [("lower", lower), ("upper", upper)] {
            let mut g = family();
            let cmd = match case {
                "upper" => {
                    let (selector, keys) = delete["d=".len()..].split_at(1);
                    format!("a=d,d={}{keys}", selector.to_ascii_uppercase())
                }
                _ => format!("a=d,{delete}"),
            };
            run(&mut g, cursor, &cmd);
            let shown: Vec<_> = placed(&g).into_iter().map(|(id, _)| id).collect();
            assert_eq!(shown, left, "{cmd}");
            // Lowercase d=y,y=5 keeps image 3: the delete hit it itself.
            assert_eq!(ids(&g), images, "{cmd} images");
        }
    }
    // By number: image 2 numbered too.
    let mut g = Graphics::default();
    put(&mut g, (2, 2), 1, "C=1");
    run(&mut g, (0, 0), &format!("a=t,I=5,{PIXEL}"));
    run(&mut g, (0, 0), "a=p,I=5,P=1,H=1");
    put(&mut g, (0, 0), 3, "P=2");
    run(&mut g, (0, 0), "a=d,d=n,I=5");
    assert_eq!(placed(&g), [(1, 1)]);
    assert_eq!(ids(&g), [1, 2]);
}

#[test]
fn a_parent_removed_any_other_way_takes_its_children() {
    // Retransmitting its image.
    let mut g = family();
    run(&mut g, (0, 0), &format!("a=t,i=1,{PIXEL}"));
    assert!(g.placements.is_empty());
    assert_eq!(ids(&g), [1]);
    // Replacing it with an empty crop, which termshot does not keep.
    let mut g = family();
    put(&mut g, (2, 2), 1, "x=5");
    assert!(g.placements.is_empty());
    // Evicted by the storage quota.
    let mut g = family();
    let first = g.images.iter_mut().find(|img| img.id == 1).unwrap();
    Arc::make_mut(&mut first.frames[0].data).resize(MAX_BYTES, 0);
    first.atime = 0;
    run(&mut g, (0, 0), &format!("a=T,i=9,C=1,{PIXEL}"));
    assert_eq!(placed(&g), [(9, 0)]);
    assert_eq!(ids(&g), [9]);
    // A full-screen erase.
    let mut g = family();
    g.clear();
    assert!(g.placements.is_empty() && g.images.is_empty());
}

#[test]
fn children_follow_a_scrolling_parent() {
    // A full-screen scroll moves the root and its children; the root's
    // start row goes above the screen, as kitty's goes into the scrollback.
    let mut g = Graphics::default();
    put(&mut g, (0, 1), 1, "C=1,r=3");
    put(&mut g, (0, 0), 2, "P=1,Q=1,V=2");
    put(&mut g, (0, 0), 3, "P=1,Q=1");
    g.scroll(0, 9, -2, 20);
    assert_eq!(at(&g, 1, 1).map(|a| (a.1, a.3)), Some((0, Some(-20))));
    assert_eq!(at(&g, 2, 1), Some((0, 1, 0, Some(20))));
    // Above the top, a child is cut off, but kept while its root is shown.
    assert_eq!(at(&g, 3, 1), Some((0, -1, 0, None)));
    g.scroll(0, 9, -1, 20);
    assert_eq!(at(&g, 1, 1).map(|a| (a.1, a.3)), Some((0, Some(-40))));
    assert_eq!(at(&g, 2, 1).map(|a| (a.1, a.3)), Some((0, Some(0))));
    assert_eq!(at(&g, 3, 1).map(|a| (a.1, a.3)), Some((-2, None)));
    // Once the root scrolls off, it goes, and its children with it, since
    // termshot keeps no scrollback. Their images go whatever their ids.
    g.scroll(0, 9, -1, 20);
    assert!(g.placements.is_empty());
    assert_eq!(ids(&g), [1]);

    // A child of a root left visible stays in view while the root is shown.
    let mut g = Graphics::default();
    put(&mut g, (0, 0), 1, "C=1,r=2");
    put(&mut g, (0, 0), 2, "P=1,Q=1,V=5");
    g.scroll(0, 9, -1, 20);
    assert_eq!(at(&g, 2, 1).map(|a| a.1), Some(4));
    g.scroll(0, 9, -1, 20);
    assert!(g.placements.is_empty());

    // Scrolling down moves them down.
    let mut g = Graphics::default();
    put(&mut g, (0, 1), 1, "C=1");
    put(&mut g, (0, 0), 2, "P=1,Q=1,V=2");
    g.scroll(0, 9, 3, 20);
    assert_eq!(at(&g, 2, 1).map(|a| a.1), Some(6));
}

#[test]
fn children_follow_their_root_in_a_scrolling_region() {
    // A root inside the region moves with it; its child moves with the root
    // even outside the region, as kitty draws it from the root's row.
    let mut g = Graphics::default();
    put(&mut g, (0, 4), 1, "C=1,r=2");
    put(&mut g, (0, 0), 2, "P=1,Q=1,V=-4");
    g.scroll(2, 7, -1, 20);
    assert_eq!(at(&g, 2, 1).map(|a| a.1), Some(-1));
    // Clipped at the top margin, the root's start row stops there, as
    // kitty's does, so the child stops too.
    g.scroll(2, 7, -2, 20);
    assert_eq!(at(&g, 1, 1).map(|a| (a.1, a.3)), Some((2, Some(20))));
    assert_eq!(at(&g, 2, 1).map(|a| a.1), Some(-2));
    // Scrolled out of the region, the root goes, with its child.
    g.scroll(2, 7, -1, 20);
    assert!(g.placements.is_empty());

    // A root crossing a margin stays, and so does its child, even inside.
    let mut g = Graphics::default();
    put(&mut g, (0, 1), 1, "C=1,r=2");
    put(&mut g, (0, 0), 2, "P=1,Q=1,V=3");
    g.scroll(2, 7, -1, 20);
    assert_eq!(at(&g, 2, 1).map(|a| a.1), Some(4));
    // A child never scrolls on its own.
    put(&mut g, (0, 0), 1, "C=1");
    assert_eq!(at(&g, 2, 1).map(|a| a.1), Some(3));
}

#[test]
fn offscreen_children_are_kept_and_clipped() {
    let mut g = Graphics::default();
    put(&mut g, (5, 5), 1, "C=1");
    put(&mut g, (0, 0), 2, "P=1,Q=1,H=-6,V=-6,c=3,r=3");
    let p = g.placements.iter().find(|p| p.id == 2).unwrap();
    assert_eq!((p.col, p.row, p.x), (-1, -1, -10));
    // Fitted to 3x3 cells, the square is 30 pixels, 15 down from the top.
    assert_eq!(p.slices, [ImageSlice { y: -5, top: 0, bottom: 25 }]);
    put(&mut g, (0, 0), 3, "P=1,Q=1,V=-2147483648,H=2147483647");
    let p = g.placements.iter().find(|p| p.id == 3).unwrap();
    assert!(p.slices.is_empty());
    assert_eq!((p.col, p.row), (MAX_CELL, -MAX_CELL));
    // Past the bottom of the screen, nothing is shown either.
    put(&mut g, (0, 0), 4, "P=1,Q=1,V=5");
    assert!(g.placements.iter().find(|p| p.id == 4).unwrap().slices.is_empty());
    // Moving the parent brings them back.
    put(&mut g, (5, 0), 1, "C=1");
    assert!(!g.placements.iter().find(|p| p.id == 4).unwrap().slices.is_empty());
    assert_eq!(g.placements.len(), 4);
}

#[test]
fn hostile_trees_stay_bounded() {
    // 1,024 placements, children included: a fan of chains eight deep.
    let mut g = Graphics::default();
    run(&mut g, (0, 0), &format!("a=t,i=1,{PIXEL}"));
    run(&mut g, (0, 0), "a=p,i=1,p=1,C=1");
    let mut made = 1;
    let mut pid = 1;
    while made < MAX_PLACEMENTS + 50 {
        pid += 1;
        let parent = if pid % 8 == 2 { 1 } else { pid - 1 };
        run(&mut g, (0, 0), &format!("a=p,i=1,p={pid},P=1,Q={parent},H=1"));
        made += 1;
    }
    assert_eq!(g.placements.len(), MAX_PLACEMENTS);
    // The links from each placement up to its root, which must be there.
    let links = |g: &Graphics| -> Vec<usize> {
        g.placements
            .iter()
            .map(|p| {
                let (mut p, mut n) = (p, 0);
                while let Some(key) = p.parent {
                    p = g.placements.iter().find(|q| q.key == key).unwrap();
                    n += 1;
                    assert!(n <= MAX_DEPTH);
                }
                n
            })
            .collect()
    };
    assert_eq!(links(&g).iter().max(), Some(&MAX_DEPTH));
    assert!(g.placements.iter().all(|p| (0..=8).contains(&p.col)));
    // Attempts at cycles are refused; a move under another chain at the
    // same depth is taken.
    for pid in 2..200 {
        run(&mut g, (0, 0), &format!("a=p,i=1,p=1,P=1,Q={pid}"));
        run(&mut g, (0, 0), &format!("a=p,i=1,p={pid},P=1,Q={}", pid + 7));
    }
    assert_eq!(g.placements.len(), MAX_PLACEMENTS);
    // Hang each chain's head under the fifth link of the chain before: the
    // links that would then be too deep go.
    for head in (10..400).step_by(8) {
        run(&mut g, (0, 0), &format!("a=p,i=1,p={head},P=1,Q={}", head - 4));
    }
    assert!(links(&g).iter().all(|&n| n <= MAX_DEPTH));
    assert!(g.placements.len() < MAX_PLACEMENTS);
    assert_eq!(g.placements.iter().filter(|p| p.parent.is_none()).count(), 1);
    // Scrolling them all is quick, and the root going takes all of them.
    for _ in 0..1000 {
        g.scroll(0, 9, 0, 20);
    }
    run(&mut g, (0, 0), "a=d,d=i,i=1,p=1");
    assert!(g.placements.is_empty());

    // A cycle cannot be made, but layout would drop one rather than loop.
    let mut g = Graphics::default();
    put(&mut g, (0, 0), 1, "C=1");
    put(&mut g, (0, 0), 2, "P=1,Q=1");
    put(&mut g, (0, 0), 3, "P=2,Q=1");
    put(&mut g, (0, 0), 4, "C=1");
    let key = |g: &Graphics, id| g.placements.iter().find(|p| p.id == id).unwrap().key;
    let (two, three) = (key(&g, 2), key(&g, 3));
    g.placements.iter_mut().find(|p| p.id == 2).unwrap().parent = Some(three);
    g.placements.iter_mut().find(|p| p.id == 3).unwrap().parent = Some(two);
    g.relayout();
    assert_eq!(placed(&g), [(1, 1), (4, 1)]);
}

#[test]
fn draw_order_is_by_z_for_children_too() {
    let mut g = Graphics::default();
    put(&mut g, (0, 0), 1, "C=1,z=5");
    put(&mut g, (0, 0), 2, "P=1,Q=1,z=-3");
    put(&mut g, (0, 0), 3, "P=1,Q=1,z=-1073741825");
    let order: Vec<_> = g.placements.iter().map(|p| (p.id, p.z)).collect();
    assert_eq!(order, [(3, -1073741825), (2, -3), (1, 5)]);
    let views: Vec<_> = g.placements.iter().flat_map(|p| p.views()).map(|v| v.z).collect();
    assert_eq!(views, [-1073741825, -3, 5]);
}
