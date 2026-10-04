//! geometry.rs tests: the exactness check and the budget of the stroke
//! cache, and a failure at each of its allocations in turn (which used to
//! be tests/stamps_alloc.c). Whether each character draws what its Unicode
//! name says, and whether reused strokes match fresh ones in every cell of
//! the largest screens, is tests/boxes.c's (test.sh links it with this
//! module as a static library).

use super::faults::{Faults, FAILED, FAULTS};
use super::*;

/// An RGB canvas of w x h pixels, rows 3 * w bytes apart, and the geometry
/// painted into it (or none).
struct Grid {
    px: Vec<u8>,
    w: i32,
    h: i32,
    geometry: *mut Geometry,
}

impl Grid {
    fn new(w: i32, h: i32, geometry: *mut Geometry) -> Grid {
        Grid { px: vec![0; (w * h) as usize * BPP], w, h, geometry }
    }

    fn canvas(&mut self) -> Canvas {
        let p = self.px.as_mut_ptr();
        Canvas { px: p, filtered: p, w: self.w, h: self.h, stride: self.w as usize * BPP, geometry: self.geometry }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint(&mut self, col: i32, row: i32, cell_w: i32, cell_h: i32, cp: u32, bold: bool, c: Rgb) -> c_int {
        let cv = self.canvas();
        unsafe { termshot_paint_geometry(&cv, col, row, cell_w, cell_h, cp, bold as c_int, c[0], c[1], c[2]) }
    }

    fn stats(&self) -> GeometryStats {
        let mut s = GeometryStats { hits: 9, ..Default::default() };
        unsafe { termshot_geometry_stats(self.geometry, &mut s) };
        s
    }

    fn stamps(&mut self) -> &mut Stamps {
        unsafe { &mut *self.geometry }.stamps.as_mut().expect("a stroke cache")
    }
}

impl Drop for Grid {
    fn drop(&mut self) {
        unsafe { termshot_geometry_free(self.geometry) };
    }
}

fn set_faults(fail_at: u32) {
    FAULTS.with(|f| f.set(Faults { calls: 0, fail_at }));
    FAILED.with(|f| f.set(None));
}

fn calls() -> u32 {
    FAULTS.with(|f| f.get().calls)
}

const STROKES: [u32; 7] = [0x256D, 0x256E, 0x256F, 0x2570, 0x2571, 0x2572, 0x2573];
const CELL_W: i32 = 22;
const CELL_H: i32 = 48;
const COLS: i32 = 14;
const ROWS: i32 = 6;

/// The seven strokes, plain and bold, over a grid, with the stroke cache
/// (and its budget, if given) or without it.
fn stroke_grid(reuse: bool, budget: Option<usize>) -> (Vec<u8>, GeometryStats) {
    let mut grid = Grid::new(COLS * CELL_W, ROWS * CELL_H, termshot_geometry_new(reuse as c_int));
    if let (Some(limit), false) = (budget, grid.geometry.is_null()) {
        grid.stamps().max_bytes = limit;
    }
    for row in 0..ROWS {
        for col in 0..COLS {
            let cp = STROKES[((row + col) % 7) as usize];
            assert_eq!(grid.paint(col, row, CELL_W, CELL_H, cp, (row / 3 + col) % 2 == 1, [255, 200, 100]), 1);
        }
    }
    let stats = grid.stats();
    (std::mem::take(&mut grid.px), stats)
}

#[test]
fn every_allocation_failure_paints_the_same() {
    set_faults(0);
    let (want, _) = stroke_grid(false, None);
    set_faults(0);
    let (got, stats) = stroke_grid(true, None);
    let allocations = calls();
    assert!(got == want && stats.hits > 0 && stats.uncached == 0 && stats.misses > 0, "{stats:?}");
    let mut sites = Vec::new();
    for fail in 1..=allocations {
        set_faults(fail);
        let (got, _) = stroke_grid(true, None);
        let site = FAILED.with(|f| f.get()).unwrap_or_else(|| panic!("allocation {fail} did not fail"));
        assert!(got == want, "allocation {fail} ({site:?}) failed and the pixels changed");
        if !sites.contains(&site) {
            sites.push(site);
        }
    }
    set_faults(0);
    for site in [Site::Geometry, Site::ArcOffsets, Site::Points, Site::Ids, Site::Sequences, Site::Masks, Site::Mask,
                 Site::Runs] {
        assert!(sites.contains(&site), "{site:?} never failed: {sites:?}");
    }
    assert!(allocations > 100, "only {allocations} allocations");
}

#[test]
fn the_budget_bounds_the_cache() {
    set_faults(0);
    let (want, _) = stroke_grid(false, None);
    let (mut limit, mut full, mut budgets) = (STAMP_MAX_BYTES, 0, 0);
    loop {
        let (got, stats) = stroke_grid(true, Some(limit));
        assert!(got == want, "budget {limit}: the pixels changed");
        // bytes is what was held when the render ended; peak the most at any time.
        assert!(stats.peak <= limit && stats.bytes <= stats.peak, "budget {limit}: {stats:?}");
        if full == 0 {
            full = stats.peak;
            assert!(stats.uncached == 0, "the default budget is too small: {stats:?}");
        }
        budgets += 1;
        if limit == 0 {
            assert!(stats.hits == 0 && stats.misses == 0 && stats.bytes == 0, "{stats:?}");
            break;
        }
        limit = limit * 3 / 4;
    }
    assert!(full > 0 && budgets > 40);
}

#[test]
fn budget_arithmetic() {
    let mut st = Stamps::new();
    st.max_bytes = 100;
    assert!(st.budget(100) && st.budget(0) && !st.budget(101) && !st.budget(usize::MAX));
    st.hold(60);
    assert!(st.budget(40) && !st.budget(41) && !st.budget(usize::MAX));
    st.bytes -= 50;
    assert!(st.bytes == 10 && st.peak == 60 && st.budget(90) && !st.budget(91));
    st.max_bytes = 5;
    // Holding more than the budget (it shrank) allows nothing, without wrapping.
    assert!(!st.budget(0) && !st.budget(1));
    assert_eq!(Stamps::new().max_bytes, 4 << 20);
}

/// xorshift, as tests/deflate_diff.c's.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

#[test]
fn exact_difference_is_exactness() {
    assert_eq!(exact_difference(5.25, 3.0), Some(2.25));
    assert_eq!(exact_difference(3.0, 3.0).map(f32::to_bits), Some(0.0f32.to_bits()));
    // Past 2^24 the integers are even: 16777218 - 1 rounds.
    assert_eq!(exact_difference(16_777_218.0, 1.0), None);
    assert_eq!(exact_difference(16_777_218.0, 2.0), Some(16_777_216.0));
    // 0.1 has bits below 1000's last one.
    assert_eq!(exact_difference(0.1, 1000.0), None);
    assert_eq!(exact_difference(1000.1, 1000.0), Some(1000.1 - 1000.0));
    // On a pixel grid's points and cell origins, TwoSum says exact exactly
    // when the f32 difference is the real one (f64 holds it exactly here).
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let (mut exact, mut inexact) = (0, 0);
    for _ in 0..200_000 {
        let origin = (rng.next() % 70_000) as f32;
        let a = (rng.next() % 70_000) as f32 + (rng.next() as f32 / 4_294_967_296.0).max(1.0 / 1024.0);
        let real = f64::from(a) - f64::from(origin);
        let got = exact_difference(a, origin);
        assert_eq!(got.is_some(), f64::from(a - origin) == real, "{a} - {origin}");
        if let Some(d) = got {
            assert_eq!(f64::from(d), real);
            exact += 1;
        } else {
            inexact += 1;
        }
    }
    assert!(exact > 1000 && inexact > 1000, "{exact} exact, {inexact} inexact");
}

/// A Stamps whose stroke has these x offsets from `origin`.
fn with_points(st: &mut Stamps, xs: &[f32]) {
    st.points = xs.iter().flat_map(|&x| [x, 0.0]).collect();
    st.capacity = xs.len() as i32;
}

#[test]
fn sequences_are_kept_only_when_exact() {
    let mut st = Stamps::new();
    let base = [0.25f32, 1.5, 3.75];
    let shifted = |by: f32| base.iter().map(|x| x + by).collect::<Vec<f32>>();
    with_points(&mut st, &shifted(10.0));
    assert_eq!(st.sequence(0, 0, 3, 10.0), 1);
    // The same offsets from another origin are the same sequence.
    with_points(&mut st, &shifted(5000.0));
    assert_eq!(st.sequence(0, 0, 3, 5000.0), 1);
    // Other offsets are another.
    with_points(&mut st, &[10.0, 11.0, 12.0]);
    assert_eq!(st.sequence(0, 0, 3, 10.0), 2);
    // Each axis and shape has its own.
    with_points(&mut st, &shifted(10.0));
    assert_eq!(st.sequence(1, 0, 3, 10.0), 1);
    assert_eq!(st.sequence(0, 3, 3, 10.0), 1);
    // An offset that rounds is not kept, and nothing is counted.
    with_points(&mut st, &[0.1, 1.0, 2.0]);
    assert_eq!(st.sequence(0, 0, 3, 1000.0), -1);
    assert_eq!(st.sequence_count[0][0], 2);
    // -0.0 and 0.0 differ bit for bit, as memcmp said.
    let mut st = Stamps::new();
    st.points = vec![0.0, 0.0];
    st.capacity = 1;
    assert_eq!(st.sequence(0, 0, 1, 0.0), 1);
    st.points = vec![-0.0, 0.0];
    assert_eq!(st.sequence(0, 0, 1, 0.0), 2);
}

#[test]
fn sequences_stop_at_their_limit() {
    let mut st = Stamps::new();
    for k in 0..STAMP_SEQUENCES {
        with_points(&mut st, &[k as f32, 0.5]);
        assert_eq!(st.sequence(0, 0, 2, 0.0), k + 1);
    }
    // Known ones are still found; a new one is not kept.
    with_points(&mut st, &[7.0, 0.5]);
    assert_eq!(st.sequence(0, 0, 2, 0.0), 8);
    with_points(&mut st, &[1000.0, 0.5]);
    assert_eq!(st.sequence(0, 0, 2, 0.0), -1);
    assert_eq!(st.sequence_room[0][0], STAMP_SEQUENCES + 1);
    let held = (STAMP_SEQUENCES + 1) as usize * 2 * std::mem::size_of::<f32>();
    assert_eq!((st.bytes, st.peak), (held, held));
}

#[test]
fn keys_and_slots() {
    let mut st = Stamps::new();
    assert_eq!(st.key(0, 0, 0, 1), 0, "nothing known yet");
    st.ids[0][2] = Some(vec![0, 3, -1]);
    st.ids[1][2] = Some(vec![5]);
    assert_eq!(st.key(2, 0, 0, 1), 0, "the column isn't known yet");
    assert_eq!(st.key(2, 2, 0, 1), -1, "the column is never reused");
    assert_eq!(st.key(2, 1, 0, 256), -1, "too thick to key");
    let seq = i64::from(STAMP_SEQUENCES) + 1;
    assert_eq!(st.key(2, 1, 0, 7), 1 + (((2 * 256 + 7) * seq + 3) * seq + 5));
    // The largest key fits the 32 bits a slot keeps.
    assert!(1 + (((5 * 256 + 255) * seq + seq - 1) * seq + seq - 1) < i64::from(u32::MAX));
    for key in [1u32, 2, 12345, u32::MAX] {
        assert!(stamp_slot(key) < STAMP_MASKS);
    }
    assert_eq!(stamp_slot(1), (2654435761u32 >> 22) as usize);
}

#[test]
fn cells_are_reused_only_on_their_grid() {
    let mut st = Stamps::new();
    let cell = |x0, y0, w, h| Rect { x0, y0, x1: x0 + w, y1: y0 + h };
    assert_eq!(st.cell(cell(20, 30, 10, 15), 100, 60, 0, 40), Some((2, 2)));
    assert_eq!((st.w, st.h, st.lines), (10, 15, [10, 4]));
    assert_eq!(st.cell(cell(25, 30, 10, 15), 100, 60, 0, 40), None, "off the grid");
    assert_eq!(st.cell(cell(20, 30, 11, 15), 100, 60, 0, 40), None, "another cell size");
    assert_eq!(st.cell(cell(20, 30, 10, 15), 100, 60, 0, 41), None, "another point count for the shape");
    assert_eq!(st.cell(cell(20, 30, 10, 15), 100, 60, 1, 41), Some((2, 2)));
    assert_eq!(st.cell(cell(100, 0, 10, 15), 100, 60, 0, 40), None, "past the last column");
    assert_eq!(st.cell(cell(0, 0, 10, 15), 100, 60, 0, STAMP_MAX_POINTS + 1), None, "too many points");
    assert_eq!(st.cell(cell(-10, 0, 10, 15), 100, 60, 0, 40), None);
}

#[test]
fn every_geometry_character_is_painted_and_nothing_else() {
    for cp in [0x20, 0x41, 0x24FF, 0x25A0, 0x2800, 0x1F600] {
        let mut grid = Grid::new(30, 40, ptr::null_mut());
        assert_eq!(grid.paint(0, 0, 10, 20, cp, false, [255; 3]), 0, "U+{cp:04X}");
        assert!(grid.px.iter().all(|&b| b == 0), "U+{cp:04X} painted");
    }
    for cp in 0x2500..=0x259F {
        for bold in [false, true] {
            // With and without a cache, the same pixels, inside the cell.
            let mut plain = Grid::new(30, 60, ptr::null_mut());
            let mut cached = Grid::new(30, 60, termshot_geometry_new(1));
            for grid in [&mut plain, &mut cached] {
                assert_eq!(grid.paint(1, 1, 10, 20, cp, bold, [200, 100, 50]), 1, "U+{cp:04X}");
            }
            assert!(plain.px == cached.px, "U+{cp:04X}");
            let lit = (0..60).flat_map(|y| (0..30).map(move |x| (x, y))).filter(|&(x, y)| plain.px[(y * 30 + x) * 3] != 0);
            let mut any = false;
            for (x, y) in lit {
                assert!((10..20).contains(&x) && (20..40).contains(&y), "U+{cp:04X} painted ({x}, {y})");
                any = true;
            }
            assert!(any, "U+{cp:04X} painted nothing");
        }
    }
}

#[test]
fn shades_blend_over_what_is_painted() {
    let mut grid = Grid::new(4, 4, ptr::null_mut());
    grid.px.fill(100);
    assert_eq!(grid.paint(0, 0, 4, 4, 0x2592, false, [200, 0, 255]), 1);
    // Half of each, rounded: (c * 2 + 100 * 2 + 2) / 4.
    assert!(grid.px.chunks(3).all(|p| p == [150, 50, 178]), "{:?}", &grid.px[..3]);
}

#[test]
fn shades_are_the_c_blend_at_every_width() {
    // Spans shorter and longer than a pattern block, over every byte value.
    let mut rng = Rng(77);
    for w in 1..=40 {
        for k in 1..=3 {
            let mut grid = Grid::new(w, 3, ptr::null_mut());
            grid.px.iter_mut().for_each(|b| *b = rng.next() as u8);
            let before = grid.px.clone();
            let c = [rng.next() as u8, rng.next() as u8, rng.next() as u8];
            assert_eq!(grid.paint(0, 0, w, 3, 0x2590 + k as u32, false, c), 1);
            for (i, (&got, &was)) in grid.px.iter().zip(&before).enumerate() {
                let want = (i32::from(c[i % 3]) * k + i32::from(was) * (4 - k) + 2) / 4;
                assert_eq!(i32::from(got), want, "width {w}, k {k}, byte {i}");
            }
        }
    }
}

#[test]
fn fill_rect_clips_to_the_canvas() {
    let mut grid = Grid::new(5, 4, ptr::null_mut());
    let cv = grid.canvas();
    unsafe { termshot_fill_rect(&cv, -3, 2, 2, 99, 9, 8, 7) };
    unsafe { termshot_fill_rect(&cv, 4, 0, 4, 4, 1, 1, 1) };
    unsafe { termshot_fill_rect(&cv, 3, 3, 1, 1, 1, 1, 1) };
    for y in 0..4 {
        for x in 0..5 {
            let want: &[u8] = if x < 2 && y >= 2 { &[9, 8, 7] } else { &[0, 0, 0] };
            assert_eq!(&grid.px[(y * 5 + x) * 3..][..3], want, "({x}, {y})");
        }
    }
    // A stride past the pixels (draw.c's has a filter byte) leaves the rest.
    let mut buffer = vec![0xeeu8; 3 * 7];
    let cv = Canvas { px: buffer[1..].as_mut_ptr(), filtered: buffer.as_mut_ptr(), w: 2, h: 3, stride: 7,
                      geometry: ptr::null_mut() };
    unsafe { termshot_fill_rect(&cv, 0, 0, 2, 3, 1, 2, 3) };
    for (i, &b) in buffer.iter().enumerate() {
        assert_eq!(b, if i % 7 == 0 { 0xee } else { [1, 2, 3][(i % 7 - 1) % 3] }, "byte {i}");
    }
}

#[test]
fn a_canvas_without_pixels_is_left_alone() {
    let g = termshot_geometry_new(1);
    for (w, h) in [(30, 40), (0, 40), (30, 0), (-5, 40)] {
        let cv = Canvas { px: ptr::null_mut(), filtered: ptr::null_mut(), w, h, stride: 90, geometry: g };
        for cp in [0x2500, 0x256D, 0x2573, 0x2588, 0x2592] {
            assert_eq!(unsafe { termshot_paint_geometry(&cv, 0, 0, 10, 20, cp, 0, 1, 2, 3) }, 1);
        }
        unsafe { termshot_fill_rect(&cv, 0, 0, 30, 40, 1, 2, 3) };
    }
    unsafe { termshot_geometry_free(g) };
}

#[test]
fn a_panic_is_caught_and_remembered_for_the_render() {
    unsafe { termshot_geometry_free(termshot_geometry_new(0)) };
    assert_eq!(termshot_paint_failed(), 0);
    assert_eq!(guarded(|| 7), Some(7));
    assert_eq!(termshot_paint_failed(), 0);
    assert_eq!(guarded(|| -> i32 { panic!("a painter bug") }), None);
    assert_eq!(termshot_paint_failed(), 1);
    // Still failed, until the next render begins.
    assert_eq!(guarded(|| 7), Some(7));
    assert_eq!(termshot_paint_failed(), 1);
    unsafe { termshot_geometry_free(termshot_geometry_new(1)) };
    assert_eq!(termshot_paint_failed(), 0);
}

#[test]
fn stats_without_a_cache_are_zero() {
    let mut s = GeometryStats { hits: 1, misses: 2, uncached: 3, bytes: 4, peak: 5 };
    unsafe { termshot_geometry_stats(ptr::null(), &mut s) };
    assert_eq!(s, GeometryStats::default());
    let g = termshot_geometry_new(0);
    unsafe { termshot_geometry_stats(g, &mut s) };
    assert_eq!(s, GeometryStats::default());
    unsafe { termshot_geometry_free(g) };
    unsafe { termshot_geometry_free(ptr::null_mut()) };
}
