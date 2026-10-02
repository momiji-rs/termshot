//! Replay the self-contained subset of kitty graphics: direct a=T images.
//! All coordinates are pixels computed from the same font metrics as draw.c.

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_PLACEMENTS: usize = 1024;
const MAX_EXTENT: i64 = 1 << 24;

extern "C" {
    fn image_png_size(data: *const u8, len: i32, w: *mut i32, h: *mut i32) -> i32;
    fn image_png_decode(data: *const u8, len: i32, out: *mut u8, w: i32, h: i32) -> i32;
}

#[derive(Default)]
pub struct Graphics {
    pub placements: Vec<Placement>,
    pending: Option<(Command, Vec<u8>)>,
}

pub struct Placement {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub clip_top: i64,
    pub clip_bottom: i64,
    id: u32,
    placement_id: u32,
    z: u32,
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
}

impl Placement {
    pub fn view(&self) -> ImageView {
        ImageView {
            pixels: self.pixels.as_ptr(),
            width: self.width,
            height: self.height,
            x: self.x,
            y: self.y,
            w: self.w,
            h: self.h,
            clip_top: self.clip_top,
            clip_bottom: self.clip_bottom,
        }
    }
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
    placement_id: u32,
    z: u32,
    no_move: bool,
    more: bool,
    delete: u8,
    continuation: bool,
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
                b'i' => cmd.id = number(val)?,
                b'p' => cmd.placement_id = number(val)?,
                b'z' if number(val)? <= i32::MAX as u32 => cmd.z = number(val)?,
                b'C' if val == b"0" || val == b"1" => cmd.no_move = val == b"1",
                b'm' if val == b"0" || val == b"1" => cmd.more = val == b"1",
                b'q' if number(val)? <= 2 => {}
                b'd' if val.len() == 1 => cmd.delete = val[0],
                // These defaults are harmless. Reject features we cannot replay.
                b'U' | b'x' | b'y' | b'w' | b'h' | b'X' | b'Y' if val == b"0" => {}
                _ => return None,
            }
        }
        Some(cmd)
    }
}

fn base64(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() % 4 != 0 || data.len() > (MAX_BYTES + 2) / 3 * 4 {
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
    (out.len() <= MAX_BYTES).then_some(out)
}

fn decode(cmd: &Command, data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
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
        32 if data.len() == size => data.to_vec(),
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

impl Graphics {
    pub fn abort(&mut self) {
        self.pending = None;
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
        let split = bytes.iter().position(|&b| b == b';').unwrap_or(bytes.len());
        let Some(mut cmd) = Command::parse(&bytes[..split]) else {
            self.abort();
            return None;
        };
        let payload = bytes.get(split + 1..).unwrap_or_default();
        if cmd.action == b'd' {
            self.abort();
            match cmd.delete {
                b'a' | b'A' => self.placements.clear(),
                b'i' | b'I' if cmd.id != 0 => self.placements.retain(|p| {
                    p.id != cmd.id || (cmd.placement_id != 0 && p.placement_id != cmd.placement_id)
                }),
                _ => {}
            }
            return None;
        }
        let Some(chunk) = base64(payload) else {
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
                if previous.len() + data.len() > MAX_BYTES {
                    return None;
                }
                previous.extend_from_slice(&data);
                data = previous;
                let more = cmd.more;
                cmd = first;
                cmd.more = more;
            }
        }
        if cmd.action != b'T' {
            return None;
        }
        if cmd.more {
            self.pending = Some((cmd, data));
            return None;
        }
        let (width, height, pixels) = decode(&cmd, &data)?;
        let (cw, ch) = (i64::from(cell.0), i64::from(cell.1));
        let (sw, sh) = (i64::from(width), i64::from(height));
        // Bound target extents before multiplying by source dimensions. A
        // custom font can have much larger cell metrics than the built-in one.
        let (target_w, target_h) = (i64::from(cmd.cols) * cw, i64::from(cmd.rows) * ch);
        if target_w > MAX_EXTENT || target_h > MAX_EXTENT {
            return None;
        }
        let (bw, bh) = match (cmd.cols, cmd.rows) {
            (0, 0) => (sw, sh),
            (_, 0) => (target_w, (target_w * sh / sw).max(1)),
            (0, _) => ((target_h * sw / sh).max(1), target_h),
            (_, _) => (target_w, target_h),
        };
        if bw > MAX_EXTENT || bh > MAX_EXTENT {
            return None;
        }
        // Preserve aspect ratio inside the requested rectangle, centered.
        let (w, h) = if bw * sh <= bh * sw {
            (bw, (bw * sh / sw).max(1))
        } else {
            ((bh * sw / sh).max(1), bh)
        };
        let (count, retained) = self
            .placements
            .iter()
            .filter(|p| cmd.id == 0 || p.id != cmd.id)
            .fold((0, 0), |(count, size), p| {
                (count + 1, size + p.pixels.len())
            });
        if retained + pixels.len() > MAX_BYTES || count >= MAX_PLACEMENTS {
            return None;
        }
        if cmd.id != 0 {
            self.placements.retain(|p| p.id != cmd.id);
        }
        let y = row as i64 * ch + (bh - h) / 2;
        self.placements.push(Placement {
            pixels,
            width,
            height,
            x: col as i64 * cw + (bw - w) / 2,
            y,
            w,
            h,
            clip_top: 0,
            clip_bottom: screen_rows as i64 * ch,
            id: cmd.id,
            placement_id: cmd.placement_id,
            z: cmd.z,
        });
        self.placements.sort_by_key(|p| (p.z, p.id));
        if cmd.no_move {
            None
        } else {
            Some((((bw + cw - 1) / cw) as usize, ((bh + ch - 1) / ch) as usize))
        }
    }

    /// Images intersecting a scrolling region move with it and are clipped at
    /// its edges. Clipped pixels cannot reappear on a subsequent reverse scroll.
    pub fn scroll(&mut self, top: usize, bottom: usize, delta: i64, cell_h: i32) {
        let ch = i64::from(cell_h);
        let (top, bottom, dy) = (top as i64 * ch, (bottom + 1) as i64 * ch, delta * ch);
        for p in &mut self.placements {
            let lo = p.y.max(p.clip_top);
            let hi = (p.y + p.h).min(p.clip_bottom);
            if lo < bottom && hi > top {
                p.y += dy;
                p.clip_top = (lo.max(top) + dy).max(top);
                p.clip_bottom = (hi.min(bottom) + dy).min(bottom);
            }
        }
        self.placements
            .retain(|p| p.y.max(p.clip_top) < (p.y + p.h).min(p.clip_bottom));
    }
}

#[cfg(test)]
mod tests;
