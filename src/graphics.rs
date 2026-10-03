//! Replay the self-contained subset of kitty graphics: directly transmitted
//! images, stored (a=t) and placed (a=p, a=T), at the cursor, relative to
//! another placement, or in Unicode placeholder cells, and deleted as kitty
//! does. All coordinates are pixels computed from the same font metrics as
//! draw.c.

use std::rc::Rc;

const MAX_BYTES: usize = 16 * 1024 * 1024;
/// How much larger than its decoded size a compressed RGB or RGBA payload may
/// be, as kitty allows: room for the zlib framing of data that won't shrink.
const COMPRESSION_SLACK: usize = 1024;
const MAX_IMAGES: usize = 4096;
const MAX_PLACEMENTS: usize = 1024;
const MAX_EXTENT: i64 = 1 << 24;
/// The most parent links a relative placement may have to its root, kitty's
/// PARENT_DEPTH_LIMIT. The spec asks for at least 8.
const MAX_DEPTH: usize = 8;
/// Where a relative placement's cell may be, in cells either way: far past
/// any screen, and small enough that pixel positions cannot overflow.
const MAX_CELL: i64 = 1 << 24;

/// U+10EEEE, kitty's Unicode placeholder: a cell that shows part of the
/// image its foreground colour names, through a virtual placement.
pub const PLACEHOLDER: u32 = 0x10EEEE;

/// A placeholder cell on the final screen, as the screen gives it to
/// `Graphics::finish`.
pub struct PlaceholderCell {
    pub row: usize,
    pub col: usize,
    /// kitty's `color_to_id` of the cell's foreground and underline colours:
    /// 0 for the default, n for palette colour n (SGR 30-37 and 90-97 too),
    /// and 0xRRGGBB for a 24-bit colour.
    pub image: u32,
    pub placement: u32,
    /// The cell's first three combining marks, 0 where it has fewer.
    pub marks: [u32; 3],
}

/// The number a row or column diacritic stands for, plus one; 0 for any
/// other character, which leaves the value to be inherited (kitty's
/// `diacritic_to_num`).
fn diacritic(mark: u32) -> u32 {
    crate::rowcolumn_diacritics::DIACRITICS.binary_search(&mark).map_or(0, |i| i as u32 + 1)
}

extern "C" {
    fn image_png_size(data: *const u8, len: i32, w: *mut i32, h: *mut i32) -> i32;
    fn image_png_decode(data: *const u8, len: i32, out: *mut u8, w: i32, h: i32) -> i32;
    fn image_inflate(data: *const u8, len: i32, out: *mut u8, olen: i32) -> i32;
}

/// Stored images and their placements, kept in draw order. Every placement's
/// image is stored, and an image without an id or number lives only as long
/// as it has a placement, since nothing could place it again.
#[derive(Default)]
pub struct Graphics {
    images: Vec<Image>,
    pub placements: Vec<Placement>,
    pending: Option<(Command, Vec<u8>)>,
    /// Hands out image and placement keys, which are also kitty's creation
    /// order and its atime: both only need to increase.
    clock: u64,
    /// The cell size and screen rows of the last command, which relative
    /// placements are laid out with, and scrolling finds the screen's bottom by.
    cell: (i32, i32),
    screen_rows: usize,
}

struct Image {
    key: u64,
    id: u32,
    number: u32,
    pixels: Rc<Vec<u8>>,
    width: u32,
    height: u32,
    atime: u64,
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub struct Placement {
    pub pixels: Rc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// The source rectangle shown, x, y, w and h: the crop, within the image.
    src: [u32; 4],
    pub x: i64,
    pub w: i64,
    pub h: i64,
    slices: Vec<ImageSlice>,
    /// Where it is drawn across, in pixels: a placeholder run's cells. Other
    /// placements are not cut.
    clip_x: (i64, i64),
    /// The image's key, and a copy of its id, which never changes while the
    /// image has placements.
    image: u64,
    id: u32,
    key: u64,
    placement_id: u32,
    z: i32,
    /// The cells it covers, for the delete selectors; rows follow scrolling.
    /// A relative placement's are where it is drawn.
    col: i64,
    cols: i64,
    row: i64,
    rows: i64,
    /// A Sixel image, whose pixels later text and erasure clear.
    pub sixel: bool,
    /// kitty's start row: the row its children are placed from. It follows
    /// scrolling as `row` does, but goes above a full-screen region's top,
    /// as kitty's goes into the scrollback.
    anchor: i64,
    /// A relative placement's parent placement, by key, and its offset in
    /// cells (H, V) from the parent's top left cell.
    parent: Option<u64>,
    offset: (i64, i64),
    /// Where the parent was in `placements` when last laid out: a hint.
    parent_at: usize,
    /// Where the image starts in its first cell, in pixels.
    inner: (i64, i64),
    /// A virtual placement (U=1): never drawn itself, but shown by the
    /// placeholder cells that name it. It has no cells: `cols` and `rows`
    /// are the c and r it was put with, 0 for the image's own size.
    is_virtual: bool,
    /// A relative placement whose chain ends at a virtual placement: that
    /// placement's key and the offset in cells from it. It has no position
    /// until the final screen's placeholders give the virtual one theirs.
    virtual_root: Option<(u64, i64, i64)>,
}

/// Borrowed only for the duration of draw_png_images; pixels remain Rust-owned.
#[repr(C)]
pub struct ImageView {
    pixels: *const u8,
    width: u32,
    height: u32,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    clip_top: i64,
    clip_bottom: i64,
    clip_left: i64,
    clip_right: i64,
    /// The source rectangle sampled, inside width x height; never empty.
    src_x: u32,
    src_y: u32,
    src_w: u32,
    src_h: u32,
    /// Below INT32_MIN / 2 the image is drawn under non-default cell
    /// backgrounds, below 0 over every background but under the text, and
    /// from 0 over both.
    z: i32,
}

/// A visible vertical part of a placement. `y` is the translated origin of
/// the full source image, preserving sampling after a partial-region scroll.
/// Scrolling never splits a placement into independently moving parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ImageSlice {
    y: i64,
    top: i64,
    bottom: i64,
}

impl Image {
    /// The bytes it counts for the storage quota: its pixels, or their size
    /// once a Sixel image's placement holds them alone (`erase_sixel`). No
    /// image has none of its own otherwise.
    fn bytes(&self) -> usize {
        match self.pixels.len() {
            0 => self.width as usize * self.height as usize * 4,
            n => n,
        }
    }
}

impl Placement {
    pub fn views(&self) -> impl Iterator<Item = ImageView> + '_ {
        self.slices.iter().map(|slice| ImageView {
            pixels: self.pixels.as_ptr(),
            width: self.width,
            height: self.height,
            x: self.x,
            y: slice.y,
            w: self.w,
            h: self.h,
            clip_top: slice.top,
            clip_bottom: slice.bottom,
            clip_left: self.clip_x.0,
            clip_right: self.clip_x.1,
            src_x: self.src[0],
            src_y: self.src[1],
            src_w: self.src[2],
            src_h: self.src[3],
            z: self.z,
        })
    }
}

impl ImageView {
    /// A rectangle of one colour: the opaque pixel, stretched over it, in the
    /// layer over the text, where the views are drawn in their order. The
    /// pixel is borrowed; it must outlive the view.
    pub fn solid(pixel: &[u8; 4], x: i64, y: i64, w: i64, h: i64) -> ImageView {
        ImageView {
            pixels: pixel.as_ptr(),
            width: 1,
            height: 1,
            x,
            y,
            w,
            h,
            clip_top: y,
            clip_bottom: y + h,
            clip_left: x,
            clip_right: x + w,
            src_x: 0,
            src_y: 0,
            src_w: 1,
            src_h: 1,
            z: i32::MAX,
        }
    }
}

// Empty/clipped placements have no visible part.
fn append_slice(slices: &mut Vec<ImageSlice>, y: i64, top: i64, bottom: i64) {
    if top >= bottom {
        return;
    }
    slices.push(ImageSlice { y, top, bottom });
}

#[derive(Default)]
struct Command {
    action: u8,
    format: u32,
    width: u32,
    height: u32,
    cols: u32,
    rows: u32,
    id: u32,
    number: u32,
    placement_id: u32,
    /// x and y pick a cell for a delete; elsewhere x, y, w and h crop.
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    /// X and Y: where in its first cell the image starts, in pixels.
    offset_x: u32,
    offset_y: u32,
    z: i32,
    /// P and Q: the parent image and placement of a relative placement.
    parent_id: u32,
    parent_placement: u32,
    /// H and V: its offset in cells from the parent's top left cell.
    parent_x: i32,
    parent_y: i32,
    /// o=z: the payload is a zlib stream.
    compressed: bool,
    /// S: the size of the PNG data inside a compressed payload.
    size: u32,
    /// U=1 (any nonzero U, as kitty reads it): a virtual placement.
    virtual_put: bool,
    no_move: bool,
    more: bool,
    delete: u8,
    continuation: bool,
    /// The key of the image this transmission replaces, kept for its order.
    reuse: Option<u64>,
}

/// A 32-bit signed integer, as kitty reads the z-index.
fn signed(s: &[u8]) -> Option<i32> {
    match s.strip_prefix(b"-") {
        Some(digits) => i32::try_from(-i64::from(number(digits)?)).ok(),
        None => i32::try_from(number(s)?).ok(),
    }
}

fn number(s: &[u8]) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    s.iter().try_fold(0u32, |n, &c| {
        if !c.is_ascii_digit() {
            return None;
        }
        n.checked_mul(10)?.checked_add(u32::from(c - b'0'))
    })
}

impl Command {
    fn parse(header: &[u8]) -> Option<Self> {
        let mut cmd = Self {
            action: b't',
            format: 32,
            delete: b'a',
            continuation: true,
            ..Self::default()
        };
        let mut seen = [false; 128];
        for part in header.split(|&b| b == b',').filter(|p| !p.is_empty()) {
            if part.len() < 3 || part[1] != b'=' || part[0] >= 128 {
                return None;
            }
            let key = part[0];
            if seen[key as usize] {
                return None;
            }
            seen[key as usize] = true;
            let val = &part[2..];
            cmd.continuation &= matches!(key, b'm' | b'q');
            match key {
                b'a' if val.len() == 1 => cmd.action = val[0],
                b't' if val == b"d" => {}
                b'f' => cmd.format = number(val)?,
                b's' => cmd.width = number(val)?,
                b'v' => cmd.height = number(val)?,
                b'c' => cmd.cols = number(val)?,
                b'r' => cmd.rows = number(val)?,
                b'i' => {
                    cmd.id = number(val)?;
                    if cmd.id == 0 {
                        return None;
                    }
                }
                b'I' => {
                    cmd.number = number(val)?;
                    if cmd.number == 0 {
                        return None;
                    }
                }
                b'p' => cmd.placement_id = number(val)?,
                b'x' => cmd.x = number(val)?,
                b'y' => cmd.y = number(val)?,
                b'w' => cmd.w = number(val)?,
                b'h' => cmd.h = number(val)?,
                b'X' => cmd.offset_x = number(val)?,
                b'Y' => cmd.offset_y = number(val)?,
                b'z' => cmd.z = signed(val)?,
                b'P' => cmd.parent_id = number(val)?,
                b'Q' => cmd.parent_placement = number(val)?,
                b'H' => cmd.parent_x = signed(val)?,
                b'V' => cmd.parent_y = signed(val)?,
                b'o' if val == b"z" => cmd.compressed = true,
                b'S' => cmd.size = number(val)?,
                b'C' if val == b"0" || val == b"1" => cmd.no_move = val == b"1",
                b'm' if val == b"0" || val == b"1" => cmd.more = val == b"1",
                b'q' if number(val)? <= 2 => {}
                b'd' if val.len() == 1 => cmd.delete = val[0],
                b'U' => cmd.virtual_put = number(val)? != 0,
                // Reject features we cannot replay.
                _ => return None,
            }
        }
        (cmd.id == 0 || cmd.number == 0).then_some(cmd)
    }
}

/// Text/JSON output only needs font metrics for commands that can move its
/// cursor. Capability probes and other skipped strings must not require fonts,
/// nor does a put of an image no earlier transmission named, which replay
/// ignores. Any earlier transmission counts, so the answer stays conservative.
/// A numbered image takes the lowest free id, which the scan does not track,
/// but that id is at most the count of named transmissions so far, so a put
/// by any id up to that bound counts. Payloads are not decoded here: a failed
/// transmission counts too, which only loads fonts that were not needed.
pub fn needs_cell_metrics(data: &[u8]) -> bool {
    // (is a number, value) for every i or I a transmission named so far.
    let mut sent = std::collections::HashSet::new();
    let (mut named, mut free_ids) = (0u32, 0u32);
    let mut i = 0;
    while i + 1 < data.len() {
        if data[i] == 0x1b && matches!(data[i + 1], b']' | b'P' | b'_' | b'^' | b'X') {
            let start = i + 2;
            let end = crate::skip_string(data, start);
            if data[i + 1] == b'_'
                && data.get(start) == Some(&b'G')
                && end >= start + 3
                && data.get(end - 2..end) == Some(b"\x1b\\")
            {
                let bytes = &data[start + 1..end - 2];
                let header = bytes.split(|&b| b == b';').next().unwrap_or_default();
                if let Some(c) = Command::parse(header) {
                    let name = if c.id != 0 { (false, c.id) } else { (true, c.number) };
                    let moves = match c.action {
                        b't' | b'T' => {
                            if name.1 != 0 {
                                sent.insert(name);
                                named = named.saturating_add(1);
                            }
                            if c.number != 0 {
                                free_ids = named;
                            }
                            c.action == b'T'
                        }
                        b'p' => sent.contains(&name) || (c.id != 0 && c.id <= free_ids),
                        _ => false,
                    };
                    // Neither a relative nor a virtual placement moves the cursor.
                    if moves && !c.no_move && c.parent_id == 0 && !c.virtual_put {
                        return true;
                    }
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    false
}

/// The decoded size of an RGB or RGBA image, from its stated dimensions.
fn raw_size(cmd: &Command) -> Option<usize> {
    let bytes = match cmd.format {
        24 => 3,
        32 => 4,
        _ => return None,
    };
    (cmd.width as usize).checked_mul(cmd.height as usize)?.checked_mul(bytes)
}

/// The most payload bytes a transmission may carry, over all its chunks.
/// kitty sizes a compressed RGB or RGBA upload's buffer at its decoded size
/// plus 1 KiB and refuses more; anything else gets the decoded limit.
fn payload_limit(cmd: &Command) -> usize {
    match raw_size(cmd) {
        Some(size) if cmd.compressed => size.min(MAX_BYTES) + COMPRESSION_SLACK,
        _ => MAX_BYTES,
    }
}

fn base64(data: &[u8], limit: usize) -> Option<Vec<u8>> {
    if data.len() % 4 != 0 || data.len() > (limit + 2) / 3 * 4 {
        return None;
    }
    let value = |c| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    for (i, q) in data.chunks_exact(4).enumerate() {
        let a = value(q[0])?;
        let b = value(q[1])?;
        out.push(a << 2 | b >> 4);
        if q[2] == b'=' {
            if q[3] != b'=' || b & 15 != 0 || (i + 1) * 4 != data.len() {
                return None;
            }
        } else {
            let c = value(q[2])?;
            out.push(b << 4 | c >> 2);
            if q[3] == b'=' {
                if c & 3 != 0 || (i + 1) * 4 != data.len() {
                    return None;
                }
            } else {
                out.push(c << 6 | value(q[3])?);
            }
        }
    }
    (out.len() <= limit).then_some(out)
}

/// Inflates a zlib payload to exactly `size` bytes, as kitty's `inflate_zlib`
/// requires.
fn inflate(data: Vec<u8>, size: usize) -> Option<Vec<u8>> {
    let len = i32::try_from(data.len()).ok()?;
    let mut out = vec![0; size];
    let ok = unsafe { image_inflate(data.as_ptr(), len, out.as_mut_ptr(), size as i32) };
    (ok != 0).then_some(out)
}

/// How many bytes a compressed payload must inflate to, if the command could
/// load at all. It must be known before inflating: from the pixel size, or for
/// PNG from S, which kitty takes as 100 KiB when absent. Raw pixels must also
/// pass the decoded limits `decode` applies, so a stream for an image refused
/// anyway is never inflated.
fn inflated_size(cmd: &Command) -> Option<usize> {
    let size = match cmd.format {
        24 | 32 if cmd.width > 8192 || cmd.height > 8192 => return None,
        24 | 32 if cmd.width as usize * cmd.height as usize * 4 > MAX_BYTES => return None,
        24 | 32 => raw_size(cmd)?,
        100 if cmd.size == 0 => 100 * 1024,
        100 => cmd.size as usize,
        _ => return None,
    };
    (size != 0 && size <= MAX_BYTES).then_some(size)
}

fn decode(cmd: &Command, mut data: Vec<u8>) -> Option<(u32, u32, Vec<u8>)> {
    if cmd.compressed {
        data = inflate(data, inflated_size(cmd)?)?;
    }
    let (mut w, mut h) = (cmd.width, cmd.height);
    if cmd.format == 100 {
        let (mut x, mut y) = (0, 0);
        if unsafe { image_png_size(data.as_ptr(), data.len() as i32, &mut x, &mut y) } == 0 {
            return None;
        }
        w = x as u32;
        h = y as u32;
    }
    let size = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
    if w == 0 || h == 0 || w > 8192 || h > 8192 || size > MAX_BYTES {
        return None;
    }
    let rgba = match cmd.format {
        24 if data.len() == size / 4 * 3 => data
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        32 if data.len() == size => data,
        100 => {
            let mut rgba = vec![0; size];
            if unsafe {
                image_png_decode(
                    data.as_ptr(),
                    data.len() as i32,
                    rgba.as_mut_ptr(),
                    w as i32,
                    h as i32,
                )
            } == 0
            {
                return None;
            }
            rgba
        }
        _ => return None,
    };
    Some((w, h, rgba))
}

/// Where a put draws its image, relative to the top left of the cursor's
/// cell, and the cells it covers.
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
struct Layout {
    /// The source rectangle, x, y, w and h: the crop within the image.
    src: [u32; 4],
    x: i64,
    y: i64,
    w: i64,
    h: i64,
    cols: i64,
    rows: i64,
}

/// kitty's placement geometry, in integers. The crop is the intersection of
/// x, y, w, h (0 meaning the rest of the image) with the image. The image
/// starts X, Y pixels into the cell, at most a pixel short of its edge, as
/// kitty clamps them. c and r count whole cells from the cell's edge, so an
/// offset shrinks the space they give. The crop keeps its aspect ratio: with
/// one of c and r the other side follows it, and with both it is fitted
/// inside and centered. The cells covered, which the cursor moves past, run
/// from the cell to the far edge of that space.
fn layout(cmd: &Command, width: u32, height: u32, cell: (i32, i32)) -> Option<Layout> {
    let (cw, ch) = (i64::from(cell.0), i64::from(cell.1));
    let src_x = cmd.x.min(width);
    let src_y = cmd.y.min(height);
    let src_w = if cmd.w == 0 { width } else { cmd.w }.min(width - src_x);
    let src_h = if cmd.h == 0 { height } else { cmd.h }.min(height - src_y);
    let (sw, sh) = (i64::from(src_w), i64::from(src_h));
    let ox = i64::from(cmd.offset_x).min(cw - 1).max(0);
    let oy = i64::from(cmd.offset_y).min(ch - 1).max(0);
    // Bound target extents before multiplying by source dimensions. A
    // custom font can have much larger cell metrics than the built-in one.
    let (target_w, target_h) = (i64::from(cmd.cols) * cw - ox, i64::from(cmd.rows) * ch - oy);
    if target_w > MAX_EXTENT || target_h > MAX_EXTENT {
        return None;
    }
    let empty = sw == 0 || sh == 0;
    let (bw, bh) = match (cmd.cols, cmd.rows) {
        _ if empty => (target_w.max(0), target_h.max(0)),
        (0, 0) => (sw, sh),
        (_, 0) => (target_w, (target_w * sh / sw).max(1)),
        (0, _) => ((target_h * sw / sh).max(1), target_h),
        (_, _) => (target_w, target_h),
    };
    if bw > MAX_EXTENT || bh > MAX_EXTENT {
        return None;
    }
    // Preserve aspect ratio inside the requested rectangle, centered.
    let (w, h) = if empty {
        (0, 0)
    } else if bw * sh <= bh * sw {
        (bw, (bw * sh / sw).max(1))
    } else {
        ((bh * sw / sh).max(1), bh)
    };
    Some(Layout {
        src: [src_x, src_y, src_w, src_h],
        x: ox + (bw - w) / 2,
        y: oy + (bh - h) / 2,
        w,
        h,
        cols: (ox + bw + cw - 1) / cw,
        rows: (oy + bh + ch - 1) / ch,
    })
}

impl Graphics {
    pub fn abort(&mut self) {
        self.pending = None;
    }

    /// ED 2, RIS and the alternate screen, as kitty's grman_clear: remove
    /// every placement but the virtual ones, which have no place on the
    /// screen, and free every image left without one, stored images
    /// included. An upload continues.
    pub fn clear(&mut self) {
        self.placements.retain(|p| p.is_virtual);
        self.free_unplaced(|_| true);
    }

    /// Whether a virtual placement exists, for placeholder cells to show.
    pub fn has_virtual(&self) -> bool {
        self.placements.iter().any(|p| p.is_virtual)
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// Returns the cursor advance in cells, if the command placed an image.
    pub fn command(
        &mut self,
        bytes: &[u8],
        col: usize,
        row: usize,
        cell: (i32, i32),
        screen_rows: usize,
    ) -> Option<(usize, usize)> {
        self.cell = cell;
        self.screen_rows = screen_rows;
        let advance = self.execute(bytes, col, row, cell);
        self.relayout();
        advance
    }

    fn execute(&mut self, bytes: &[u8], col: usize, row: usize, cell: (i32, i32)) -> Option<(usize, usize)> {
        let split = bytes.iter().position(|&b| b == b';').unwrap_or(bytes.len());
        let Some(mut cmd) = Command::parse(&bytes[..split]) else {
            self.abort();
            return None;
        };
        let payload = bytes.get(split + 1..).unwrap_or_default();
        match cmd.action {
            b'd' => {
                self.abort();
                self.delete(&cmd, col, row);
                return None;
            }
            b'p' => {
                self.abort();
                let index = self.find(&cmd)?;
                return self.put(index, &cmd, col, row, cell);
            }
            b't' | b'T' => {}
            _ => {
                self.abort();
                return None;
            }
        }
        // Transmitting an id replaces its image and placements at once,
        // whether or not the new data turns out to load.
        if cmd.id != 0 {
            if let Some(index) = self.images.iter().position(|img| img.id == cmd.id) {
                cmd.reuse = Some(self.images[index].key);
                self.remove_image(index);
            }
        }
        // Continuation chunks are bounded by the command that began the upload.
        let limit = match &self.pending {
            Some((first, _)) if cmd.continuation => payload_limit(first),
            _ => payload_limit(&cmd),
        };
        let Some(chunk) = base64(payload, limit) else {
            self.abort();
            return None;
        };
        // A padded chunk can only finish a transmission.
        if cmd.more && payload.contains(&b'=') {
            self.abort();
            return None;
        }
        let mut data = chunk;
        if let Some((first, mut previous)) = self.pending.take() {
            if cmd.continuation {
                if previous.len() + data.len() > limit {
                    return None;
                }
                previous.extend_from_slice(&data);
                data = previous;
                let more = cmd.more;
                cmd = first;
                cmd.more = more;
            }
        }
        if cmd.more {
            self.pending = Some((cmd, data));
            return None;
        }
        // Nothing could ever place an image stored with neither id nor number.
        if cmd.action == b't' && cmd.id == 0 && cmd.number == 0 {
            return None;
        }
        let (width, height, pixels) = decode(&cmd, data)?;
        let key = match cmd.reuse {
            Some(key) => key,
            None => self.tick(),
        };
        let id = if cmd.id == 0 && cmd.number != 0 { self.free_id() } else { cmd.id };
        let atime = self.tick();
        let pixels = Rc::new(pixels);
        self.images.push(Image { key, id, number: cmd.number, pixels, width, height, atime });
        let mut advance = None;
        if cmd.action == b'T' {
            advance = self.put(self.images.len() - 1, &cmd, col, row, cell);
            if id == 0 && !self.placements.iter().any(|p| p.image == key) {
                self.images.pop();
            }
        }
        self.enforce_quota(key);
        advance
    }

    /// The image a put or delete names: by id, or the newest with a number.
    fn find(&self, cmd: &Command) -> Option<usize> {
        if cmd.id != 0 {
            self.images.iter().position(|img| img.id == cmd.id)
        } else if cmd.number != 0 {
            (0..self.images.len())
                .filter(|&i| self.images[i].number == cmd.number)
                .max_by_key(|&i| self.images[i].key)
        } else {
            None
        }
    }

    /// The lowest id no image has, as kitty gives one numbered without an id.
    fn free_id(&self) -> u32 {
        let mut ids: Vec<u32> = self.images.iter().map(|img| img.id).filter(|&id| id != 0).collect();
        ids.sort_unstable();
        let mut free = 1;
        for id in ids {
            if id > free {
                break;
            }
            free = id + 1;
        }
        free
    }

    /// Place a stored image at the cursor, or relative to a parent placement.
    /// The same nonzero placement id on the same image moves that placement
    /// instead of adding one.
    fn put(
        &mut self,
        index: usize,
        cmd: &Command,
        col: usize,
        row: usize,
        cell: (i32, i32),
    ) -> Option<(usize, usize)> {
        let image = &self.images[index];
        let (image_key, id, width, height) = (image.key, image.id, image.width, image.height);
        let pixels = Rc::clone(&image.pixels);
        // A virtual placement cannot be a relative one (kitty's EINVAL).
        if cmd.virtual_put && cmd.parent_id != 0 {
            return None;
        }
        // A parent that does not exist refuses the put (kitty's ENOPARENT).
        let parent = match cmd.parent_id {
            0 => None,
            _ => Some(self.parent(cmd)?),
        };
        let Layout { src, x, y, w, h, cols, rows } = layout(cmd, width, height, cell)?;
        // kitty ignores a placement id on an image without an id.
        let placement_id = if id == 0 { 0 } else { cmd.placement_id };
        let existing = match placement_id {
            0 => None,
            _ => self
                .placements
                .iter()
                .position(|p| p.image == image_key && p.placement_id == placement_id),
        };
        if let Some(parent) = parent {
            let me = existing.map(|i| self.placements[i].key);
            if !self.ancestry_fits(parent, me) {
                // kitty makes a new placement before it checks the chain,
                // and so has already marked the image used.
                if existing.is_none() {
                    self.images[index].atime = self.tick();
                }
                return None;
            }
        }
        // An empty crop shows nothing: its put still moves the cursor, and
        // replaces the placement it names, but leaves nothing to draw. A
        // virtual placement draws the whole image whatever its crop.
        let visible = cmd.virtual_put || (w > 0 && h > 0);
        if visible && existing.is_none() && self.placements.len() >= MAX_PLACEMENTS {
            return None;
        }
        let key = match existing {
            Some(i) => self.placements.remove(i).key,
            None => self.tick(),
        };
        self.images[index].atime = self.tick();
        if visible {
            // Not clipped at the screen's bottom: the screen may scroll the
            // rest into view, as kitty's does, and draw.c clips at the canvas.
            let (ch, y) = (i64::from(cell.1), row as i64 * i64::from(cell.1) + y);
            let mut slices = Vec::new();
            // A relative placement is laid out from its parent by relayout.
            if parent.is_none() && !cmd.virtual_put {
                append_slice(&mut slices, y, y, y + h);
            }
            let (cols, rows) = match cmd.virtual_put {
                true => (i64::from(cmd.cols), i64::from(cmd.rows)),
                false => (cols, rows),
            };
            self.placements.push(Placement {
                pixels,
                width,
                height,
                src,
                x: col as i64 * i64::from(cell.0) + x,
                w,
                h,
                slices,
                clip_x: (i64::MIN, i64::MAX),
                image: image_key,
                id,
                key,
                placement_id,
                z: cmd.z,
                col: col as i64,
                cols,
                row: row as i64,
                rows,
                sixel: false,
                anchor: row as i64,
                parent,
                offset: (i64::from(cmd.parent_x), i64::from(cmd.parent_y)),
                parent_at: usize::MAX,
                inner: (x, y - row as i64 * ch),
                is_virtual: cmd.virtual_put,
                virtual_root: None,
            });
            // kitty's draw order: z-index, then image and placement creation.
            self.placements.sort_by_key(|p| (p.z, p.image, p.key));
        }
        // Neither a relative nor a virtual placement moves the cursor,
        // whatever C says.
        if cmd.no_move || parent.is_some() || cmd.virtual_put {
            None
        } else {
            Some((cols as usize, rows as usize))
        }
    }

    /// The placement a relative put names: image P's placement Q, or with no
    /// Q the image's oldest placement. (kitty takes the first in its hash
    /// map, which is not an order a client can rely on.)
    fn parent(&self, cmd: &Command) -> Option<u64> {
        let image = self.images.iter().find(|img| img.id == cmd.parent_id)?.key;
        let q = cmd.parent_placement;
        self.placements
            .iter()
            .filter(|p| p.image == image && (q == 0 || p.placement_id == q))
            .map(|p| p.key)
            .min()
    }

    /// Whether a placement may be a child of `parent`: it must not be its own
    /// ancestor (kitty's EINVAL and ECYCLE), and the chain up to its root may
    /// have at most MAX_DEPTH links (ETOODEEP). `me` is the placement being
    /// moved, if the put names an existing one.
    fn ancestry_fits(&self, parent: u64, me: Option<u64>) -> bool {
        let mut key = parent;
        for _ in 0..MAX_DEPTH {
            if Some(key) == me {
                return false;
            }
            match self.placements.iter().find(|p| p.key == key) {
                Some(p) => match p.parent {
                    Some(next) => key = next,
                    None => return true,
                },
                None => return false,
            }
        }
        false
    }

    /// Lay relative placements out from their roots, and remove those whose
    /// chain no longer resolves, as kitty's grman_update_layers does: a
    /// parent that is gone, or a chain longer than MAX_DEPTH, which moving a
    /// placement with children can make. The children of a removed placement
    /// fail too, so one pass suffices. An image left without placements here
    /// is freed whatever its id, as the spec says of a relative placement's
    /// image. This runs after every command and scroll, so it is linear in
    /// the placements while no placement is added or removed: each child
    /// keeps the index of its parent, checked against the parent's key.
    fn relayout(&mut self) {
        if !self.placements.iter().any(|p| p.parent.is_some()) {
            return;
        }
        const NONE: usize = usize::MAX;
        let n = self.placements.len();
        // Each child's parent index: its hint while that is right, else found
        // by key in an index made once.
        let mut by_key: Option<Vec<(u64, usize)>> = None;
        let mut parents = vec![NONE; n];
        for (i, slot) in parents.iter_mut().enumerate() {
            let Some(key) = self.placements[i].parent else { continue };
            let hint = self.placements[i].parent_at;
            *slot = if self.placements.get(hint).map_or(false, |p| p.key == key) {
                hint
            } else {
                let index = by_key.get_or_insert_with(|| {
                    let mut v: Vec<_> = self.placements.iter().enumerate().map(|(i, p)| (p.key, i)).collect();
                    v.sort_unstable();
                    v
                });
                index.binary_search_by_key(&key, |&(k, _)| k).map_or(NONE, |j| index[j].1)
            };
        }
        #[derive(Clone, Copy)]
        enum Resolved {
            Unknown,
            /// On the walk being resolved: met again, it closes a cycle.
            Walking,
            Broken,
            /// The cell it is placed at, the links up to its root, and the
            /// root. Under a virtual root the cell is the offset from it.
            At(i64, i64, usize, usize),
        }
        let mut cells = vec![Resolved::Unknown; n];
        let mut path = Vec::new();
        for i in 0..n {
            // Walk up to a placement already resolved, or a root. Depth is
            // only known on the way back down: a placement met on the way up
            // may be fine even when the one the walk started from is too deep.
            let mut at = i;
            loop {
                match cells[at] {
                    Resolved::Unknown => {}
                    // A cycle, which the put checks prevent, resolves nowhere.
                    Resolved::Walking => {
                        cells[at] = Resolved::Broken;
                        break;
                    }
                    _ => break,
                }
                let p = &self.placements[at];
                if p.is_virtual {
                    cells[at] = Resolved::At(0, 0, 0, at);
                } else if p.parent.is_none() {
                    cells[at] = Resolved::At(p.col, p.anchor, 0, at);
                } else if parents[at] == NONE {
                    // Its parent is gone.
                    cells[at] = Resolved::Broken;
                } else {
                    cells[at] = Resolved::Walking;
                    path.push(at);
                    at = parents[at];
                }
            }
            while let Some(j) = path.pop() {
                let (dx, dy) = self.placements[j].offset;
                cells[j] = match cells[parents[j]] {
                    Resolved::At(col, row, links, root) if links < MAX_DEPTH => {
                        Resolved::At(col + dx, row + dy, links + 1, root)
                    }
                    _ => Resolved::Broken,
                };
            }
        }
        let (cw, ch) = (i64::from(self.cell.0), i64::from(self.cell.1));
        let bottom = self.screen_rows as i64 * ch;
        // The virtual placements' keys, by index: (index, key).
        let virtuals: Vec<(usize, u64)> =
            self.placements.iter().enumerate().filter(|(_, p)| p.is_virtual).map(|(i, p)| (i, p.key)).collect();
        let mut lost = Vec::new();
        let mut i = 0;
        self.placements.retain_mut(|p| {
            let (cell, parent_at) = (cells[i], parents[i]);
            i += 1;
            match cell {
                _ if p.parent.is_none() => true,
                Resolved::At(col, row, _, root) => {
                    let (col, row) = (col.clamp(-MAX_CELL, MAX_CELL), row.clamp(-MAX_CELL, MAX_CELL));
                    p.parent_at = parent_at;
                    // Placed from a virtual placement: laid out by finish.
                    if let Ok(k) = virtuals.binary_search_by_key(&root, |&(i, _)| i) {
                        p.virtual_root = Some((virtuals[k].1, col, row));
                        p.slices.clear();
                        return true;
                    }
                    p.virtual_root = None;
                    p.col = col;
                    p.row = row;
                    p.x = col * cw + p.inner.0;
                    let y = row * ch + p.inner.1;
                    p.slices.clear();
                    append_slice(&mut p.slices, y, y.max(0), (y + p.h).min(bottom));
                    true
                }
                _ => {
                    lost.push(p.image);
                    false
                }
            }
        });
        if !lost.is_empty() {
            lost.sort_unstable();
            let mut placed: Vec<u64> = self.placements.iter().map(|p| p.image).collect();
            placed.sort_unstable();
            self.images.retain(|img| {
                placed.binary_search(&img.key).is_ok() || lost.binary_search(&img.key).is_err()
            });
        }
    }

    fn remove_image(&mut self, index: usize) {
        let key = self.images.swap_remove(index).key;
        self.placements.retain(|p| p.image != key);
    }

    /// Free the images left without placements that `free` selects, and
    /// those without an id, which nothing could place again.
    fn free_unplaced(&mut self, free: impl Fn(&Image) -> bool) {
        let mut placed: Vec<u64> = self.placements.iter().map(|p| p.image).collect();
        placed.sort_unstable();
        self.images
            .retain(|img| (img.id != 0 && !free(img)) || placed.binary_search(&img.key).is_ok());
    }

    /// kitty's delete command. Lowercase removes placements; uppercase also
    /// frees the images it leaves without one. Unsupported selectors do nothing.
    fn delete(&mut self, cmd: &Command, col: usize, row: usize) {
        let upper = cmd.delete.is_ascii_uppercase();
        let selector = cmd.delete.to_ascii_lowercase();
        // Cells are 1-based in the command; the cursor's are not.
        let (x, y) = match selector {
            b'c' => (col as i64, row as i64),
            _ => (i64::from(cmd.x) - 1, i64::from(cmd.y) - 1),
        };
        // Virtual placements, and the relative ones placed from them, have
        // no cells until the final screen.
        let located = |p: &Placement| !p.is_virtual && p.virtual_root.is_none();
        let in_col = |p: &Placement| located(p) && p.col <= x && x < p.col + p.cols;
        let in_row = |p: &Placement| located(p) && p.row <= y && y < p.row + p.rows;
        let pid = cmd.placement_id;
        match selector {
            // Only i, n and r reach a virtual placement, as the spec says.
            b'a' => self.delete_where(upper, |p| !p.is_virtual, |_| false),
            b'i' | b'n' => {
                // A command cannot give both, so find uses the one selected.
                let image = match selector {
                    b'i' if cmd.id != 0 => self.find(cmd),
                    b'n' if cmd.number != 0 => self.find(cmd),
                    _ => None,
                };
                let Some(id) = image.map(|i| self.images[i].id) else {
                    return;
                };
                // Without a placement id, an image with no placements is freed too.
                self.delete_where(
                    upper,
                    |p| p.id == id && (pid == 0 || p.placement_id == pid),
                    |img| pid == 0 && img.id == id,
                );
            }
            b'r' => {
                let in_range = |id: u32| id != 0 && cmd.x <= id && id <= cmd.y;
                self.delete_where(upper, |p| in_range(p.id), |img| pid == 0 && in_range(img.id));
            }
            b'c' | b'p' => self.delete_where(upper, |p| in_col(p) && in_row(p), |_| false),
            b'q' => self.delete_where(upper, |p| in_col(p) && in_row(p) && p.z == cmd.z, |_| false),
            b'x' => self.delete_where(upper, in_col, |_| false),
            b'y' => self.delete_where(upper, in_row, |_| false),
            b'z' => self.delete_where(upper, |p| !p.is_virtual && p.z == cmd.z, |_| false),
            _ => {}
        }
    }

    /// Remove the placements `hit` selects. With `free`, then free the images
    /// left without one that lost a placement here, or that `also` names.
    fn delete_where(
        &mut self,
        free: bool,
        hit: impl Fn(&Placement) -> bool,
        also: impl Fn(&Image) -> bool,
    ) {
        let mut matched = Vec::new();
        self.placements.retain(|p| {
            let hit = hit(p);
            if hit {
                matched.push(p.image);
            }
            !hit
        });
        matched.sort_unstable();
        self.free_unplaced(|img| free && (also(img) || matched.binary_search(&img.key).is_ok()));
    }

    fn over_quota(&self) -> bool {
        self.images.len() > MAX_IMAGES || self.images.iter().map(Image::bytes).sum::<usize>() > MAX_BYTES
    }

    /// kitty's storage quota: first free every image without a placement, then
    /// the least recently used ones with their placements, until it fits.
    /// The image just added always fits on its own.
    fn enforce_quota(&mut self, added: u64) {
        if !self.over_quota() {
            return;
        }
        self.free_unplaced(|img| img.key != added);
        let mut bytes: usize = self.images.iter().map(Image::bytes).sum();
        let mut count = self.images.len();
        let mut oldest: Vec<_> = self
            .images
            .iter()
            .filter(|img| img.key != added)
            .map(|img| (img.atime, img.key, img.bytes()))
            .collect();
        oldest.sort_unstable();
        let mut evict = Vec::new();
        for (_, key, len) in oldest {
            if count <= MAX_IMAGES && bytes <= MAX_BYTES {
                break;
            }
            evict.push(key);
            count -= 1;
            bytes -= len;
        }
        evict.sort_unstable();
        self.placements.retain(|p| evict.binary_search(&p.image).is_err());
        self.images.retain(|img| evict.binary_search(&img.key).is_err());
    }

    /// Place a Sixel image, given as the command `sixel::kitty_command` makes.
    /// Its placement is marked as Sixel, for `erase_sixel`.
    pub fn sixel(&mut self, bytes: &[u8], col: usize, row: usize, cell: (i32, i32), screen_rows: usize) {
        let before = self.clock;
        self.command(bytes, col, row, cell, screen_rows);
        for p in self.placements.iter_mut().filter(|p| p.key > before) {
            p.sixel = true;
        }
    }

    /// Clear the Sixel pixels inside the screen rectangle from (x0, y0) to
    /// (x1, y1), as xterm's erase_graphic does for the cells text is written
    /// to and the rows ED erases: they become transparent. kitty images are a
    /// layer of their own, which text and erasure leave alone.
    pub fn erase_sixel(&mut self, x0: i64, y0: i64, x1: i64, y1: i64) {
        for i in 0..self.placements.len() {
            let p = &self.placements[i];
            // Sixel images are drawn whole, a pixel per screen pixel.
            let (w, h) = (i64::from(p.width), i64::from(p.height));
            if !p.sixel || p.w != w || p.h != h || p.src != [0, 0, p.width, p.height] {
                continue;
            }
            let (left, right) = (x0.max(p.x), x1.min(p.x + w));
            let rows: Vec<_> = p.slices.iter().map(|s| (y0.max(s.y), y1.min(s.y + h), s.y)).collect();
            if left >= right || rows.iter().all(|&(top, bottom, _)| top >= bottom) {
                continue;
            }
            // On the first erase the image store holds the other reference.
            // No command can place it again (a Sixel image has no id), so the
            // store lets it go, and keeps counting the image by its size: the
            // pixels are written in place, not copied, and later erases need
            // no search of the store.
            let (x, key) = (p.x, p.image);
            if Rc::strong_count(&p.pixels) > 1 {
                if let Some(img) = self.images.iter_mut().find(|img| img.key == key) {
                    img.pixels = Rc::default();
                }
            }
            let pixels = Rc::make_mut(&mut self.placements[i].pixels);
            for (top, bottom, origin) in rows {
                for y in top..bottom {
                    let start = ((y - origin) * w + left - x) as usize * 4;
                    pixels[start..start + (right - left) as usize * 4].fill(0);
                }
            }
        }
    }

    /// Scroll only placements wholly inside the region, as required by kitty.
    /// Use the surviving visible bounds: clipping is permanent, so clipped
    /// source pixels neither block later scrolling nor reappear on reversal.
    /// Without margins, kitty moves every placement and clips none, so the
    /// screen's bottom is no edge: what is below it stays, and can scroll up
    /// into view. Its top still clips.
    pub fn scroll(&mut self, top: usize, bottom: usize, delta: i64, cell_h: i32) {
        if self.placements.is_empty() {
            return;
        }
        let ch = i64::from(cell_h);
        let open = top == 0 && bottom + 1 >= self.screen_rows;
        let (first, last) = (top as i64, if open { i64::MAX / 2 / ch.max(1) } else { bottom as i64 + 1 });
        // Saturating: a distance can be as long as the log, and an image as
        // far below the screen as scrolling down has moved it.
        let (top, bottom, dy) = (first * ch, last * ch, delta.saturating_mul(ch));
        // kitty's start row stops at a margin only in a partial region.
        let partial = !open;
        for p in &mut self.placements {
            // Relative placements follow their roots, in relayout; virtual
            // ones are not on the screen.
            if p.is_virtual || p.parent.is_some() || p.slices.iter().any(|s| s.top < top || s.bottom > bottom) {
                continue;
            }
            for part in &mut p.slices {
                part.y = part.y.saturating_add(dy);
                part.top = part.top.saturating_add(dy).max(top);
                part.bottom = part.bottom.saturating_add(dy).min(bottom);
            }
            p.slices.retain(|s| s.top < s.bottom);
            // Its cells move too, clipped at the margins as kitty clips them.
            let row = p.row.saturating_add(delta);
            let (start, end) = (row.max(first), row.saturating_add(p.rows).min(last));
            p.row = start;
            p.rows = (end - start).max(0);
            p.anchor = p.anchor.saturating_add(delta);
            if partial {
                p.anchor = p.anchor.max(first);
            }
        }
        let before = self.placements.len();
        self.placements.retain(|p| p.is_virtual || p.parent.is_some() || !p.slices.is_empty());
        if self.placements.len() < before {
            self.free_unplaced(|_| false);
        }
        self.relayout();
    }
}

/// A run of placeholder cells in one row that show one stretch of one
/// virtual placement, as kitty's `screen_render_line_graphics` finds them.
struct Run {
    row: usize,
    start: usize,
    len: usize,
    image: u32,
    placement: u32,
    /// The row of the image, the column of the run's last cell in it, and
    /// the image id's high byte, each plus one, as kitty keeps them.
    img_row: u32,
    img_col: u32,
    high: u32,
}

/// Split the placeholder cells, in screen order, into runs. A cell continues
/// the run to its left when it is the next cell of the row, has the same
/// foreground and underline colours, and each diacritic it has agrees with
/// what it would inherit: the same row, the next column, the same high byte.
/// It inherits what it lacks. Any other cell starts a run, at row 0, column
/// 0 and high byte 0 where it has no diacritic.
fn runs(cells: &[PlaceholderCell]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for c in cells {
        let [row, col, high] = c.marks.map(diacritic);
        if let Some(run) = runs.last_mut() {
            if run.row == c.row
                && run.start + run.len == c.col
                && (run.image, run.placement) == (c.image, c.placement)
                && (row == 0 || row == run.img_row)
                && (col == 0 || col == run.img_col + 1)
                && (high == 0 || high == run.high)
            {
                run.len += 1;
                run.img_col += 1;
                continue;
            }
        }
        runs.push(Run {
            row: c.row,
            start: c.col,
            len: 1,
            image: c.image,
            placement: c.placement,
            img_row: row.max(1),
            img_col: col.max(1),
            high: high.max(1),
        });
    }
    runs
}

impl Graphics {
    /// What the final screen draws, in draw order: the placements, an image
    /// for each run of placeholder cells (`cells`, in screen order) that
    /// names a virtual placement, and the relative placements under a
    /// virtual one, laid out from where its placeholders are. Virtual
    /// placements themselves are not drawn. As kitty, which makes these
    /// images from the screen each time it draws (grman_put_cell_image):
    ///
    /// - The image id is the foreground's id, with the third diacritic, less
    ///   one, as its high byte. The underline colour's id names the virtual
    ///   placement; 0 takes any, here the oldest.
    /// - The whole image, whatever the placement's crop and offsets, is
    ///   fitted into a box of the placement's c x r cells (the image's size
    ///   in cells where 0), keeping its aspect ratio, and centered across
    ///   the side it does not fill. Each run shows the part of that box
    ///   under its cells, at z-index -1: over the backgrounds, under the
    ///   text and cursor.
    /// - A relative placement under a virtual one is placed from the top
    ///   row and the leftmost column, separately, of the cells that show
    ///   part of the image. With none on the screen it is not drawn.
    pub fn finish(mut self, cells: &[PlaceholderCell], cell: (i32, i32), screen_rows: usize) -> Vec<Placement> {
        let (cw, ch) = (i64::from(cell.0), i64::from(cell.1));
        let any_virtual = self.has_virtual();
        let mut out = std::mem::take(&mut self.placements);
        if cw <= 0 || ch <= 0 || !any_virtual {
            out.retain(|p| !p.is_virtual && p.virtual_root.is_none());
            return out;
        }
        // The virtual placement each (image id, placement id) names.
        let mut named: std::collections::HashMap<(u32, u32), Option<usize>> = Default::default();
        // Each virtual placement's top row and leftmost column shown.
        let mut shown: std::collections::HashMap<u64, (i64, i64)> = Default::default();
        let mut cell_images = Vec::new();
        for run in runs(cells) {
            let id = run.image | ((run.high - 1) & 0xff) << 24;
            // kitty's ids are never 0; it would find an image without one.
            if id == 0 {
                continue;
            }
            let found = *named.entry((id, run.placement)).or_insert_with(|| {
                (0..out.len())
                    .filter(|&i| out[i].is_virtual && out[i].id == id)
                    .filter(|&i| run.placement == 0 || out[i].placement_id == run.placement)
                    .min_by_key(|&i| out[i].key)
            });
            let Some(v) = found.map(|i| &out[i]) else { continue };
            let (iw, ih) = (i64::from(v.width), i64::from(v.height));
            let box_w = cw * if v.cols == 0 { (iw + cw - 1) / cw } else { v.cols };
            let box_h = ch * if v.rows == 0 { (ih + ch - 1) / ch } else { v.rows };
            // Fit to the width when the image is relatively wider than the
            // box, else to the height, as kitty compares them.
            let (w, h) = if iw * box_h > ih * box_w {
                (box_w, (box_w * ih / iw).max(1))
            } else {
                ((box_h * iw / ih).max(1), box_h)
            };
            let (row, start, len) = (run.row as i64, run.start as i64, run.len as i64);
            let x = (start - i64::from(run.img_col) + len) * cw + (box_w - w) / 2;
            let y = (row - i64::from(run.img_row) + 1) * ch + (box_h - h) / 2;
            let (left, right) = (x.max(start * cw), (x + w).min((start + len) * cw));
            let (top, bottom) = (y.max(row * ch), (y + h).min((row + 1) * ch));
            if left >= right || top >= bottom {
                continue;
            }
            let at = shown.entry(v.key).or_insert((row, i64::MAX));
            at.0 = at.0.min(row);
            at.1 = at.1.min(left / cw);
            self.clock += 1;
            cell_images.push(Placement {
                pixels: Rc::clone(&v.pixels),
                src: [0, 0, v.width, v.height],
                x,
                w,
                h,
                slices: vec![ImageSlice { y, top, bottom }],
                clip_x: (left, right),
                key: self.clock,
                placement_id: 0,
                z: -1,
                col: left / cw,
                cols: (right - 1) / cw + 1 - left / cw,
                row,
                rows: 1,
                anchor: row,
                parent: None,
                offset: (0, 0),
                parent_at: usize::MAX,
                inner: (0, 0),
                is_virtual: false,
                virtual_root: None,
                ..*v
            });
        }
        let bottom = screen_rows as i64 * ch;
        out.retain_mut(|p| {
            let Some((root, dx, dy)) = p.virtual_root else { return !p.is_virtual };
            let Some(&(row, col)) = shown.get(&root) else { return false };
            let (col, row) = ((col + dx).clamp(-MAX_CELL, MAX_CELL), (row + dy).clamp(-MAX_CELL, MAX_CELL));
            p.col = col;
            p.row = row;
            p.x = col * cw + p.inner.0;
            let y = row * ch + p.inner.1;
            append_slice(&mut p.slices, y, y.max(0), (y + p.h).min(bottom));
            true
        });
        out.append(&mut cell_images);
        out.sort_by_key(|p| (p.z, p.image, p.key));
        out
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod zlib_tests;
#[cfg(test)]
mod geometry_tests;
#[cfg(test)]
mod relative_tests;
#[cfg(test)]
mod placeholder_tests;
