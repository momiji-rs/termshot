//! What a log's kitty place commands say about the cell they were written
//! for, read from the command headers alone. Image bytes are not decoded,
//! so a placement that never loads still counts: the warning is about the
//! log, which is where the cell size was.

use super::Command;

/// How many place commands the scan keeps. Past this it stops and says so,
/// rather than allocating with the log.
const MAX_SAMPLES: usize = 4096;

/// A cell size a kitty placement implies: the source rectangle's width over
/// the columns it asked for (`w/c`) and its height over the rows (`h/r`),
/// in lowest terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRatio {
    /// Width, source pixels over cells.
    pub width: (u32, u32),
    /// Height, source pixels over cells.
    pub height: (u32, u32),
}

impl CellRatio {
    /// The size in whole pixels, when both sides divide evenly.
    pub fn whole(&self) -> Option<(u32, u32)> {
        match (self.width, self.height) {
            ((w, 1), (h, 1)) => Some((w, h)),
            _ => None,
        }
    }
}

/// Source rows of one image that the next overlapping placement starts
/// past: those rows are never drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceGap {
    /// Kitty's image id, or None when the placement named none.
    pub image: Option<u32>,
    /// The first undrawn source row, in pixels.
    pub y: u32,
    /// How many source rows are skipped.
    pub rows: u32,
}

/// The cell sizes a log's kitty placements imply, and the source rows they
/// leave undrawn. From [`placement_hints`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacementHints {
    /// The distinct sizes, in the order first seen. Empty when no placement
    /// names `c`, `r`, `w` and `h` all nonzero. A placement that only names
    /// `c` and `r` scales its image into that many cells of whatever size
    /// the render uses, so it implies nothing.
    pub cells: Vec<CellRatio>,
    /// Gaps, in the order of the placements that leave them.
    pub gaps: Vec<SourceGap>,
    /// The log has more place commands than the scan keeps (4096). `cells`
    /// and `gaps` are from the ones kept, in order, and are not the whole log.
    pub truncated: bool,
    /// Some placement omits `c` or `r`, so the cells it covers are the
    /// source pixels divided by the render's cell. Also set when the scan
    /// stops early, since a later command may be one of these. False means
    /// every placement names both, and `--json` reports those counts at any
    /// cell size.
    pub follows_cell: bool,
}

/// One placement the scan kept.
struct Sample {
    /// The image's key: its id, or a private key when it has none.
    image: u64,
    id: Option<u32>,
    /// Its placement id, 0 when the command named none.
    placement: u32,
    x: u32,
    y: u32,
    /// 0 when the command omitted `w`, so the width is the rest of the
    /// image and the overlap test treats it as overlapping any x.
    w: u32,
    h: u32,
    /// The command's `c` and `r`. Both are nonzero when `implies` is set.
    cols: u32,
    rows: u32,
    z: i32,
    virtual_put: bool,
    /// `c`, `r` and `w` were set along with `h`, so `w/c` by `h/r` is a
    /// cell size. A command that names `c` and `r` but not `w` and `h`
    /// scales the image into that many cells of the render's size.
    implies: bool,
}

#[derive(Default)]
struct Scan {
    /// The next private image key. Ids are their own keys, and none reach
    /// here: an id is at most u32, and this starts one past it.
    next: u64,
    /// id -> key, and number -> key, of images a transmission has stored.
    by_id: Vec<(u32, u64)>,
    by_number: Vec<(u32, u64)>,
    samples: Vec<Sample>,
    truncated: bool,
    /// A placement omitted `c` or `r`. Sticky: a later delete does not clear
    /// it, so a `--json` run still reads a font rather than guess at 1x1.
    follows_cell: bool,
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn ratio(num: u32, den: u32) -> (u32, u32) {
    let d = gcd(num, den);
    (num / d, den / d)
}

impl Scan {
    fn alloc(&mut self) -> u64 {
        // The first private key is just past every id.
        if self.next == 0 {
            self.next = u64::from(u32::MAX);
        }
        self.next += 1;
        self.next
    }

    fn forget_id(&mut self, id: u32) {
        let Some(pos) = self.by_id.iter().position(|(stored, _)| *stored == id) else {
            return;
        };
        let key = self.by_id.swap_remove(pos).1;
        self.by_number.retain(|(_, stored)| *stored != key);
        self.samples.retain(|sample| sample.image != key);
    }

    /// The key of an image this transmission stores. Retransmitting an id
    /// drops the placements it had, as the replay does, before the new bytes
    /// are known to load.
    fn transmit(&mut self, cmd: &Command) -> u64 {
        if cmd.id != 0 {
            self.forget_id(cmd.id);
        }
        let key = if cmd.id != 0 { u64::from(cmd.id) } else { self.alloc() };
        if cmd.id != 0 {
            self.by_id.push((cmd.id, key));
        }
        if cmd.number != 0 {
            self.by_number.retain(|(number, _)| *number != cmd.number);
            self.by_number.push((cmd.number, key));
        }
        key
    }

    fn lookup(&self, cmd: &Command) -> Option<u64> {
        if cmd.id != 0 {
            self.by_id.iter().find(|(id, _)| *id == cmd.id).map(|(_, key)| *key)
        } else if cmd.number != 0 {
            self.by_number.iter().find(|(number, _)| *number == cmd.number).map(|(_, key)| *key)
        } else {
            None
        }
    }

    fn drop_where(&mut self, hit: impl Fn(&Sample) -> bool) {
        self.samples.retain(|sample| !hit(sample));
    }

    /// kitty's delete, for the selectors that name no cell. A delete by
    /// cursor, cell, column or row needs the replay's positions, which this
    /// scan does not have, so those selectors leave the samples.
    fn delete(&mut self, cmd: &Command) {
        let selector = cmd.delete.to_ascii_lowercase();
        let placement = cmd.placement_id;
        match selector {
            // Virtual placements have no cells for `d=a` to remove.
            b'a' => self.drop_where(|sample| !sample.virtual_put),
            b'i' if cmd.id != 0 => {
                let id = cmd.id;
                self.drop_where(|sample| sample.id == Some(id) && (placement == 0 || sample.placement == placement));
            }
            b'n' if cmd.number != 0 => {
                let Some(key) = self.lookup(cmd) else { return };
                self.drop_where(|sample| sample.image == key && (placement == 0 || sample.placement == placement));
            }
            b'r' => {
                let (from, to) = (cmd.x, cmd.y);
                self.drop_where(|sample| sample.id.is_some_and(|id| id != 0 && from <= id && id <= to));
            }
            b'z' => {
                let z = cmd.z;
                self.drop_where(|sample| !sample.virtual_put && sample.z == z);
            }
            _ => {}
        }
    }

    fn id_of(&self, key: u64) -> Option<u32> {
        self.by_id.iter().find(|(_, stored)| *stored == key).map(|(id, _)| *id)
    }

    /// `columns` and `rows` in `--json` are the command's `c` and `r` when
    /// both are set. Omitting either leaves that side to the pixels.
    fn placed(&mut self, cmd: &Command) {
        if cmd.cols == 0 || cmd.rows == 0 {
            self.follows_cell = true;
        }
    }

    fn record(&mut self, cmd: &Command, key: u64) {
        // `w` and `h` of 0 mean the rest of the image. Only a command that
        // names both, with `c` and `r`, says how many source pixels it
        // wanted per cell. A command that names `h` but not `w` can still
        // leave a gap in y.
        if cmd.h == 0 || self.truncated {
            return;
        }
        if self.samples.len() >= MAX_SAMPLES {
            self.truncated = true;
            return;
        }
        let id = if cmd.id != 0 { Some(cmd.id) } else { self.id_of(key) };
        self.samples.push(Sample {
            image: key,
            id,
            placement: cmd.placement_id,
            x: cmd.x,
            y: cmd.y,
            w: cmd.w,
            h: cmd.h,
            cols: cmd.cols,
            rows: cmd.rows,
            z: cmd.z,
            virtual_put: cmd.virtual_put,
            implies: cmd.cols != 0 && cmd.rows != 0 && cmd.w != 0,
        });
    }

    fn string(&mut self, s: &[u8]) {
        let Some(bytes) = s.strip_prefix(b"G").and_then(|s| s.strip_suffix(b"\x1b\\")) else {
            return;
        };
        let header = bytes.split(|&b| b == b';').next().unwrap_or_default();
        let Some(cmd) = Command::parse(header) else {
            return;
        };
        // A continuation chunk carries the upload's remaining bytes, not a
        // new placement. The command that opened the upload was recorded.
        if cmd.continuation {
            return;
        }
        if cmd.action == b'd' {
            self.delete(&cmd);
            return;
        }
        let key = match cmd.action {
            b't' => {
                // Stored, and placed only by a later `a=p`. An id replaces
                // the image and the placements it had.
                if cmd.id != 0 || cmd.number != 0 {
                    self.transmit(&cmd);
                }
                return;
            }
            b'T' => self.transmit(&cmd),
            b'p' => {
                let Some(key) = self.lookup(&cmd) else { return };
                key
            }
            _ => return,
        };
        self.placed(&cmd);
        self.record(&cmd, key);
    }

    fn finish(self) -> PlacementHints {
        let mut cells = Vec::new();
        for sample in &self.samples {
            if !sample.implies {
                continue;
            }
            let ratio = CellRatio {
                width: ratio(sample.w, sample.cols),
                height: ratio(sample.h, sample.rows),
            };
            if !cells.contains(&ratio) {
                cells.push(ratio);
            }
        }
        let mut gaps = Vec::new();
        for sample in &self.samples {
            let mut next_y = u32::MAX;
            for other in &self.samples {
                if other.image != sample.image || other.y <= sample.y || !overlaps(sample, other) {
                    continue;
                }
                next_y = next_y.min(other.y);
            }
            let end = sample.y.saturating_add(sample.h);
            if next_y != u32::MAX && next_y > end {
                gaps.push(SourceGap { image: sample.id, y: end, rows: next_y - end });
            }
        }
        PlacementHints {
            cells,
            gaps,
            truncated: self.truncated,
            follows_cell: self.follows_cell || self.truncated,
        }
    }
}

fn overlaps(a: &Sample, b: &Sample) -> bool {
    if a.w == 0 || b.w == 0 {
        return true;
    }
    let (a_end, b_end) = (a.x.saturating_add(a.w), b.x.saturating_add(b.w));
    a.x < b_end && b.x < a_end
}

/// The cell sizes `log`'s kitty placements imply, and the source rows they
/// leave undrawn. The scan reads each command's header and not its image
/// bytes. It follows the deletes that name no cell (`d=a`, `d=i`, `d=n`,
/// `d=r`, `d=z`), and a retransmission of an id. A delete by cursor, cell,
/// column or row is left in place, since that needs the replay's positions,
/// and an uppercase delete frees the image in the replay but here only
/// drops the placements it names. Sixel is not a kitty command and is not
/// included.
pub fn placement_hints(log: &[u8]) -> PlacementHints {
    let mut scan = Scan::default();
    crate::vt::any_string(log, |kind, s| {
        if kind == b'_' {
            scan.string(s);
        }
        false
    });
    scan.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apc(header: &str) -> Vec<u8> {
        format!("\x1b_G{header}\x1b\\").into_bytes()
    }

    fn hints(commands: &[&str]) -> PlacementHints {
        let mut log = Vec::new();
        for command in commands {
            log.extend(apc(command));
        }
        placement_hints(&log)
    }

    fn cell(w: u32, h: u32) -> CellRatio {
        CellRatio { width: (w, 1), height: (h, 1) }
    }

    #[test]
    fn explicit_source_rectangles_imply_one_cell_when_they_agree() {
        // 32 px of source across 2 columns and 32 down one row: 16x32 cells.
        let got = hints(&[
            "a=t,i=7,s=32,v=96",
            "a=p,i=7,p=1,x=0,y=0,w=32,h=32,c=2,r=1",
            "a=p,i=7,p=2,x=0,y=32,w=32,h=32,c=2,r=1",
            "a=p,i=7,p=3,x=0,y=64,w=32,h=32,c=2,r=1",
        ]);
        assert_eq!(got.cells, vec![cell(16, 32)]);
        assert!(got.gaps.is_empty(), "{:?}", got.gaps);
        // The same ratios reduce: 64/4 is 16 too.
        let mixed = hints(&["a=T,i=1,c=4,r=2,w=64,h=64", "a=p,i=1,c=2,r=1,w=32,h=32"]);
        assert_eq!(mixed.cells, vec![cell(16, 32)]);
    }

    #[test]
    fn a_source_row_past_the_previous_strip_is_a_gap() {
        // h is 31 and the next strip starts at 32, so y 31 is never drawn.
        let got = hints(&[
            "a=t,i=7,s=32,v=96",
            "a=p,i=7,p=1,y=0,w=32,h=31,c=2,r=1",
            "a=p,i=7,p=2,y=32,w=32,h=31,c=2,r=1",
        ]);
        assert_eq!(got.gaps, vec![SourceGap { image: Some(7), y: 31, rows: 1 }]);
        // Strips that share no columns are not one run of rows.
        let beside = hints(&[
            "a=t,i=1",
            "a=p,i=1,x=0,y=0,w=10,h=10,c=1,r=1",
            "a=p,i=1,x=10,y=0,w=10,h=10,c=1,r=1",
            "a=p,i=1,x=0,y=10,w=10,h=10,c=1,r=1",
        ]);
        assert!(beside.gaps.is_empty(), "{:?}", beside.gaps);
    }

    #[test]
    fn scaling_into_cells_without_a_source_rectangle_implies_nothing() {
        // c and r alone fit the image into the render's cells.
        let got = hints(&["a=T,i=1,s=4,v=2,c=4,r=2", "a=p,i=1,U=1,c=2,r=1"]);
        assert!(got.cells.is_empty(), "{:?}", got.cells);
        // A put of an image nothing transmitted is not a placement.
        assert!(hints(&["a=p,i=9,c=2,r=1,w=32,h=32"]).cells.is_empty());
    }

    #[test]
    fn deletes_and_retransmission_drop_the_placements_they_name() {
        let replaced = hints(&["a=T,i=1,c=2,r=1,w=32,h=32", "a=T,i=1,c=1,r=1,w=10,h=20"]);
        assert_eq!(replaced.cells, vec![CellRatio { width: (10, 1), height: (20, 1) }]);
        let cleared = hints(&["a=T,i=1,c=2,r=1,w=32,h=32", "a=d"]);
        assert!(cleared.cells.is_empty());
        let one = hints(&["a=T,i=1,c=2,r=1,w=32,h=32", "a=T,i=2,c=1,r=1,w=8,h=8", "a=d,d=i,i=1"]);
        assert_eq!(one.cells, vec![cell(8, 8)]);
        // A virtual placement survives d=a, as it does in the replay.
        let virtual_put = hints(&["a=T,i=1,U=1,c=2,r=1,w=32,h=32", "a=d"]);
        assert_eq!(virtual_put.cells, vec![cell(16, 32)]);
        // The command that opens a chunked upload is the placement; the
        // continuation is not a second one.
        let chunked = hints(&["a=T,i=1,c=2,r=1,w=32,h=32,m=1", "m=0"]);
        assert_eq!(chunked.cells, vec![cell(16, 32)]);
    }

    #[test]
    fn disagreement_keeps_each_ratio_and_the_cap_stops_the_scan() {
        let got = hints(&["a=T,i=1,c=2,r=1,w=32,h=32", "a=T,i=2,c=3,r=1,w=10,h=31"]);
        assert_eq!(
            got.cells,
            vec![cell(16, 32), CellRatio { width: (10, 3), height: (31, 1) }]
        );
        let mut commands = Vec::new();
        for i in 1..=MAX_SAMPLES + 1 {
            commands.extend(apc(&format!("a=T,i={i},c=1,r=1,w=8,h=8")));
        }
        let capped = placement_hints(&commands);
        assert!(capped.truncated);
        assert_eq!(capped.cells, vec![cell(8, 8)]);
    }

    #[test]
    fn the_strips_fixture_implies_16x32_cells_and_no_gap() {
        let log = std::fs::read("tests/fixtures/kitty-strips.pty").unwrap();
        let got = placement_hints(&log);
        assert_eq!(got.cells, vec![cell(16, 32)]);
        assert!(got.gaps.is_empty() && !got.truncated);
        assert!(!got.follows_cell);
    }

    #[test]
    fn a_placement_that_omits_a_cell_count_follows_the_render_cell() {
        // Both c and r fix the cells --json reports.
        assert!(!hints(&["a=T,i=1,c=2,r=1,w=32,h=32"]).follows_cell);
        // Native pixels, and a height with no row count: the cells covered
        // are the pixels divided by the render's cell.
        assert!(hints(&["a=T,i=1,s=40,v=20,C=1"]).follows_cell);
        assert!(hints(&["a=T,i=1,c=3,w=2,h=2"]).follows_cell);
    }
}
