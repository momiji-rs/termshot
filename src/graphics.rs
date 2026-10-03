//! Replay the self-contained subset of kitty graphics: directly transmitted
//! images, stored (a=t) and placed (a=p, a=T), at the cursor or relative to
//! another placement, and deleted as kitty does.
//! All coordinates are pixels computed from the same font metrics as draw.c.

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
    /// placements are laid out with.
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
            src_x: self.src[0],
            src_y: self.src[1],
            src_w: self.src[2],
            src_h: self.src[3],
            z: self.z,
        })
    }
}

impl ImageView {
    /// A rectangle of one colour: the opaque pixel, stretched over it, above
    /// everything else. The pixel is borrowed; it must outlive the view.
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
                // These defaults are harmless. Reject features we cannot replay.
                b'U' if val == b"0" => {}
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
                    // A relative placement never moves the cursor.
                    if moves && !c.no_move && c.parent_id == 0 {
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
/// requires. `data` gets the 8 bytes of zero padding `image_inflate` needs.
fn inflate(mut data: Vec<u8>, size: usize) -> Option<Vec<u8>> {
    let len = i32::try_from(data.len()).ok()?;
    data.extend_from_slice(&[0; 8]);
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

    /// ED 2, as kitty's grman_clear: remove every placement and free every
    /// image left without one, stored images included. An upload continues.
    pub fn clear(&mut self) {
        self.placements.clear();
        self.images.clear();
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
        let advance = self.execute(bytes, col, row, cell, screen_rows);
        self.relayout();
        advance
    }

    fn execute(
        &mut self,
        bytes: &[u8],
        col: usize,
        row: usize,
        cell: (i32, i32),
        screen_rows: usize,
    ) -> Option<(usize, usize)> {
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
                return self.put(index, &cmd, col, row, cell, screen_rows);
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
            advance = self.put(self.images.len() - 1, &cmd, col, row, cell, screen_rows);
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
        screen_rows: usize,
    ) -> Option<(usize, usize)> {
        let image = &self.images[index];
        let (image_key, id, width, height) = (image.key, image.id, image.width, image.height);
        let pixels = Rc::clone(&image.pixels);
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
        // replaces the placement it names, but leaves nothing to draw.
        let visible = w > 0 && h > 0;
        if visible && existing.is_none() && self.placements.len() >= MAX_PLACEMENTS {
            return None;
        }
        let key = match existing {
            Some(i) => self.placements.remove(i).key,
            None => self.tick(),
        };
        self.images[index].atime = self.tick();
        if visible {
            let (ch, y) = (i64::from(cell.1), row as i64 * i64::from(cell.1) + y);
            let mut slices = Vec::new();
            // A relative placement is laid out from its parent by relayout.
            if parent.is_none() {
                append_slice(&mut slices, y, y, (y + h).min(screen_rows as i64 * ch));
            }
            self.placements.push(Placement {
                pixels,
                width,
                height,
                src,
                x: col as i64 * i64::from(cell.0) + x,
                w,
                h,
                slices,
                image: image_key,
                id,
                key,
                placement_id,
                z: cmd.z,
                col: col as i64,
                cols,
                row: row as i64,
                rows,
                anchor: row as i64,
                parent,
                offset: (i64::from(cmd.parent_x), i64::from(cmd.parent_y)),
                parent_at: usize::MAX,
                inner: (x, y - row as i64 * ch),
            });
            // kitty's draw order: z-index, then image and placement creation.
            self.placements.sort_by_key(|p| (p.z, p.image, p.key));
        }
        // A relative placement never moves the cursor, whatever C says.
        if cmd.no_move || parent.is_some() {
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
            /// The cell it is placed at, and the links up to its root.
            At(i64, i64, usize),
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
                if p.parent.is_none() {
                    cells[at] = Resolved::At(p.col, p.anchor, 0);
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
                    Resolved::At(col, row, links) if links < MAX_DEPTH => {
                        Resolved::At(col + dx, row + dy, links + 1)
                    }
                    _ => Resolved::Broken,
                };
            }
        }
        let (cw, ch) = (i64::from(self.cell.0), i64::from(self.cell.1));
        let bottom = self.screen_rows as i64 * ch;
        let mut lost = Vec::new();
        let mut i = 0;
        self.placements.retain_mut(|p| {
            let (cell, parent_at) = (cells[i], parents[i]);
            i += 1;
            match cell {
                _ if p.parent.is_none() => true,
                Resolved::At(col, row, _) => {
                    let (col, row) = (col.clamp(-MAX_CELL, MAX_CELL), row.clamp(-MAX_CELL, MAX_CELL));
                    p.parent_at = parent_at;
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
        let in_col = |p: &Placement| p.col <= x && x < p.col + p.cols;
        let in_row = |p: &Placement| p.row <= y && y < p.row + p.rows;
        let pid = cmd.placement_id;
        match selector {
            b'a' => self.delete_where(upper, |_| true, |_| false),
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
            b'z' => self.delete_where(upper, |p| p.z == cmd.z, |_| false),
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
        self.images.len() > MAX_IMAGES
            || self.images.iter().map(|img| img.pixels.len()).sum::<usize>() > MAX_BYTES
    }

    /// kitty's storage quota: first free every image without a placement, then
    /// the least recently used ones with their placements, until it fits.
    /// The image just added always fits on its own.
    fn enforce_quota(&mut self, added: u64) {
        if !self.over_quota() {
            return;
        }
        self.free_unplaced(|img| img.key != added);
        let mut bytes: usize = self.images.iter().map(|img| img.pixels.len()).sum();
        let mut count = self.images.len();
        let mut oldest: Vec<_> = self
            .images
            .iter()
            .filter(|img| img.key != added)
            .map(|img| (img.atime, img.key, img.pixels.len()))
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

    /// Scroll only placements wholly inside the region, as required by kitty.
    /// Use the surviving visible bounds: clipping is permanent, so clipped
    /// source pixels neither block later scrolling nor reappear on reversal.
    pub fn scroll(&mut self, top: usize, bottom: usize, delta: i64, cell_h: i32) {
        if self.placements.is_empty() {
            return;
        }
        let ch = i64::from(cell_h);
        let (first, last) = (top as i64, bottom as i64 + 1);
        let (top, bottom, dy) = (first * ch, last * ch, delta * ch);
        // kitty's start row stops at a margin only in a partial region.
        let partial = first != 0 || last != self.screen_rows as i64;
        for p in &mut self.placements {
            // Relative placements follow their roots, in relayout.
            if p.parent.is_some() || p.slices.iter().any(|s| s.top < top || s.bottom > bottom) {
                continue;
            }
            for part in &mut p.slices {
                part.y += dy;
                part.top = (part.top + dy).max(top);
                part.bottom = (part.bottom + dy).min(bottom);
            }
            p.slices.retain(|s| s.top < s.bottom);
            // Its cells move too, clipped at the margins as kitty clips them.
            let (start, end) = ((p.row + delta).max(first), (p.row + delta + p.rows).min(last));
            p.row = start;
            p.rows = (end - start).max(0);
            p.anchor += delta;
            if partial {
                p.anchor = p.anchor.max(first);
            }
        }
        let before = self.placements.len();
        self.placements.retain(|p| p.parent.is_some() || !p.slices.is_empty());
        if self.placements.len() < before {
            self.free_unplaced(|_| false);
        }
        self.relayout();
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
