//! kitty animation, as a still: frame uploads (a=f), frame composition
//! (a=c), animation control (a=a) and frame deletion (d=f, d=F), following
//! kitty's graphics.c (handle_animation_frame_load_command,
//! handle_compose_command, handle_animation_control_command,
//! handle_delete_frame_command and get_coalesced_frame_data).
//!
//! termshot draws one picture from an offline log, with no timeline, so an
//! image shows its current frame: the one an explicit a=a with c last made
//! current, else the root frame. Gaps (z), the animation state (s) and the
//! loop count (v) are parsed and change nothing. A frame is kept as kitty
//! keeps it, as the pixels sent and a recipe, and composed on demand: a
//! frame with a base frame is drawn over that frame's composed pixels, one
//! without over a background colour, and editing or composing onto a frame
//! makes it whole. All of it is integer arithmetic, the same on every
//! platform.

use std::rc::Rc;

use super::{decode, Command, Graphics, Image, MAX_BYTES};

/// The most frames an image may have, its root included. kitty sets none;
/// this one keeps a frame's bookkeeping small next to its pixels.
pub(super) const MAX_FRAMES: usize = 1024;
/// The most frames past the root all images of a screen may have together.
pub(super) const MAX_EXTRA_FRAMES: usize = 16384;
/// How many pixels one screen's frames may be composed over, in all. A
/// frame can be built on others, so a few bytes of a command can ask for a
/// whole image to be composed; past this, commands that need composing are
/// refused. The final screen's frames are composed regardless: the storage
/// quota bounds that.
pub(super) const COMPOSE_BUDGET: u64 = 1 << 28;
/// The most frames a composition may go through, the frame and its bases:
/// kitty's get_coalesced_frame_data gives up past 32 bases.
const MAX_CHAIN: usize = 33;

/// One animation frame, as kitty's Frame and the data it caches for it.
#[derive(Clone)]
#[cfg_attr(test, derive(Debug))]
pub(super) struct Frame {
    pub id: u32,
    /// The pixels sent, RGBA, `w` by `h`. An opaque frame's alpha is 255.
    pub data: Rc<Vec<u8>>,
    /// Where `data` goes in the image, and its size.
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Sent as RGB (f=24): kitty keeps 3 bytes a pixel, so what is composed
    /// onto it stays opaque.
    pub opaque: bool,
    /// Blend `data` over what is under it, rather than replace it (X=0).
    pub blend: bool,
    /// The id of the frame it is drawn over, or 0 for none.
    pub base: u32,
    /// The background under `data` when there is no base, 0xRRGGBBAA.
    pub bg: u32,
}

impl Frame {
    /// An image's data as its root frame, frame id 1.
    pub fn root(data: Rc<Vec<u8>>, width: u32, height: u32, opaque: bool) -> Frame {
        Frame { id: 1, data, x: 0, y: 0, w: width, h: height, opaque, blend: false, base: 0, bg: 0 }
    }

    /// Whether its data is the whole frame: kitty's is_full_frame, without
    /// a base.
    fn whole(&self, width: u32, height: u32) -> bool {
        self.base == 0 && self.x == 0 && self.y == 0 && self.w == width && self.h == height
    }
}

impl Image {
    /// The bytes it stores for its frames, and the bytes of the pixels it
    /// shows when those are composed apart from them.
    pub(super) fn parts(&self) -> (usize, usize) {
        let stored = self.frames.iter().map(|f| f.data.len()).sum();
        (stored, self.shown_bytes(self.current))
    }

    /// The bytes it shows apart from its frames with frame `k` current.
    fn shown_bytes(&self, k: usize) -> usize {
        if self.frames[k].whole(self.width, self.height) {
            0
        } else {
            self.size()
        }
    }

    /// Its current frame's pixels, if they need no composing.
    pub(super) fn shown(&self) -> Option<Rc<Vec<u8>>> {
        let f = &self.frames[self.current];
        f.whole(self.width, self.height).then(|| Rc::clone(&f.data))
    }

    /// The frame with a 1-based number, kitty's frame_for_number.
    fn frame(&self, number: u32) -> Option<usize> {
        let index = (number as usize).checked_sub(1)?;
        (index < self.frames.len()).then_some(index)
    }
}

/// Spends `n` pixels of composing from `budget`, if there are that many.
fn spend(budget: &mut u64, n: u64) -> Option<()> {
    *budget = budget.checked_sub(n)?;
    Some(())
}

/// The pixels of frame `k`, the whole image, and whether they are opaque:
/// kitty's get_coalesced_frame_data. A frame without a base is its data, or
/// its data over its background colour (transparent black by default,
/// without alpha if the frame is opaque); one with a base is its data over
/// the base's pixels, which keep their opacity. A missing base, or more than
/// 32 of them, leaves the frame without pixels.
fn coalesce(img: &Image, k: usize, budget: &mut u64) -> Option<(Rc<Vec<u8>>, bool)> {
    let (width, height) = (img.width, img.height);
    let mut chain = vec![k];
    loop {
        let base = img.frames[chain[chain.len() - 1]].base;
        if base == 0 {
            break;
        }
        if chain.len() >= MAX_CHAIN {
            return None;
        }
        chain.push(img.frames.iter().position(|f| f.id == base)?);
    }
    let size = u64::from(width) * u64::from(height);
    let first = &img.frames[chain.pop()?];
    let (mut pixels, opaque) = if first.whole(width, height) {
        (Rc::clone(&first.data), first.opaque)
    } else {
        spend(budget, size)?;
        let mut bg = first.bg.to_be_bytes();
        if first.opaque {
            bg[3] = 255;
        }
        let mut canvas = bg.repeat(size as usize);
        draw(&mut canvas, width, height, first.opaque, first, budget)?;
        (Rc::new(canvas), first.opaque)
    };
    while let Some(i) = chain.pop() {
        if Rc::strong_count(&pixels) > 1 {
            spend(budget, size)?;
        }
        draw(Rc::make_mut(&mut pixels).as_mut_slice(), width, height, opaque, &img.frames[i], budget)?;
    }
    Some((pixels, opaque))
}

/// The pixels of frame `f` inside the image, which `draw` puts.
fn clipped(width: u32, height: u32, f: &Frame) -> u64 {
    let cols = u64::from(width).saturating_sub(u64::from(f.x)).min(u64::from(f.w));
    let rows = u64::from(height).saturating_sub(u64::from(f.y)).min(u64::from(f.h));
    cols * rows
}

/// Puts frame `f`'s data on `under`, the image's pixels, at its offset and
/// clipped to the image: kitty's compose.
fn draw(under: &mut [u8], width: u32, height: u32, opaque: bool, f: &Frame, budget: &mut u64) -> Option<()> {
    spend(budget, clipped(width, height, f))?;
    let cols = u64::from(width).saturating_sub(u64::from(f.x)).min(u64::from(f.w)) as usize;
    let rows = u64::from(height).saturating_sub(u64::from(f.y)).min(u64::from(f.h)) as usize;
    // Nothing inside the image: x may be past its right edge.
    if cols == 0 {
        return Some(());
    }
    let (width, x, y, fw) = (width as usize, f.x as usize, f.y as usize, f.w as usize);
    for row in 0..rows {
        let to = ((y + row) * width + x) * 4;
        let from = row * fw * 4;
        put_row(&mut under[to..to + cols * 4], &f.data[from..from + cols * 4], opaque, f.blend);
    }
    Some(())
}

/// One row of kitty's copy_or_blend_row, `over` onto `under`, both RGBA.
/// Blending over opaque pixels is the render's blend (composite.rs), and
/// leaves them opaque; over others it is the straight-alpha over operator,
/// rounded to nearest, which over an opaque pixel is the same. Copying takes
/// the colour, and the alpha unless `under` is opaque.
fn put_row(under: &mut [u8], over: &[u8], opaque: bool, blend: bool) {
    for (d, s) in under.chunks_exact_mut(4).zip(over.chunks_exact(4)) {
        if !blend {
            d[..3].copy_from_slice(&s[..3]);
            if !opaque {
                d[3] = s[3];
            }
        } else if opaque {
            // SAFETY: d is a pixel's 4 bytes.
            unsafe { crate::composite::blend_pixel(d.as_mut_ptr(), [s[0], s[1], s[2]], u32::from(s[3])) }
        } else {
            over_straight(d, s);
        }
    }
}

/// kitty's blend_pixel_over_straight in integers: the out alpha is
/// a + d(1 - a), each colour the alpha-weighted mean, rounded half up where
/// kitty rounds a float to nearest. Both transparent leaves `d` alone.
fn over_straight(d: &mut [u8], s: &[u8]) {
    let (a, da) = (u32::from(s[3]), u32::from(d[3]));
    let denom = a * 255 + da * (255 - a);
    if denom == 0 {
        return;
    }
    for c in 0..3 {
        let num = u32::from(s[c]) * a * 255 + u32::from(d[c]) * da * (255 - a);
        d[c] = ((2 * num + denom) / (2 * denom)) as u8;
    }
    d[3] = ((denom + 127) / 255) as u8;
}

/// kitty's reference_chain_too_large: whether a new frame based on frame
/// `k` should be composed now rather than drawn over it later, because `k`
/// is built on 4 or more frames, or on frames covering twice the image.
fn chain_too_large(img: &Image, k: usize) -> bool {
    let limit = u64::from(img.width) * u64::from(img.height) * 2;
    let mut f = &img.frames[k];
    let mut area = u64::from(f.w) * u64::from(f.h);
    let mut links = 1;
    while area < limit && links < 5 {
        match img.frames.iter().find(|b| f.base != 0 && b.id == f.base) {
            Some(base) => f = base,
            None => break,
        }
        area += u64::from(f.w) * u64::from(f.h);
        links += 1;
    }
    links >= 5 || area >= limit
}

/// What `compose_onto` puts on a frame: a frame's data, or for a=c a w by h
/// rectangle at sx, sy of another frame's pixels, put at dx, dy.
enum Over {
    Frame(Frame),
    Rect { source: Rc<Vec<u8>>, blend: bool, dx: u64, dy: u64, sx: u64, sy: u64, w: u64, h: u64 },
}

impl Graphics {
    fn budget(&self) -> u64 {
        COMPOSE_BUDGET.saturating_sub(self.composed)
    }

    /// `coalesce` with the budget left, charging what it used.
    fn coalesce(&mut self, index: usize, k: usize) -> Option<(Rc<Vec<u8>>, bool)> {
        let mut budget = self.budget();
        let out = coalesce(&self.images[index], k, &mut budget);
        self.composed = COMPOSE_BUDGET - budget;
        out
    }

    fn index_of(&self, key: u64) -> usize {
        self.images.iter().position(|img| img.key == key).expect("the image is stored")
    }

    /// Whether image `key` may go from `before` to `after` bytes: all the
    /// images must fit the storage quota, once the others without a
    /// placement are freed if they would not, as kitty makes room for a new
    /// frame. Unlike a new image, a frame never evicts placed images.
    fn make_room(&mut self, key: u64, before: usize, after: usize) -> bool {
        let fits = |g: &Self| {
            let total: usize = g.images.iter().map(Image::bytes).sum();
            total - before + after <= MAX_BYTES
        };
        if after <= before || fits(self) {
            return true;
        }
        self.free_unplaced(|img| img.key != key);
        fits(self)
    }

    /// Makes frame `k` of image `index` whole: `over` composed onto
    /// `pixels`, its pixels as `coalesce` gave them. Those are taken from
    /// the frame rather than copied when nothing else holds them. False if
    /// the budget is spent, leaving the frame as it was.
    fn compose_onto(&mut self, index: usize, k: usize, pixels: (Rc<Vec<u8>>, bool), over: Over) -> bool {
        let (mut pixels, opaque) = pixels;
        let img = &mut self.images[index];
        let (width, height) = (img.width, img.height);
        let mine = Rc::ptr_eq(&pixels, &img.frames[k].data);
        if mine {
            img.frames[k].data = Rc::default();
        }
        let copy = if Rc::strong_count(&pixels) > 1 { u64::from(width) * u64::from(height) } else { 0 };
        let work = match &over {
            Over::Frame(f) => clipped(width, height, f),
            Over::Rect { w, h, .. } => w * h,
        };
        let mut budget = self.budget();
        if spend(&mut budget, copy + work).is_none() {
            if mine {
                self.images[index].frames[k].data = pixels;
            }
            return false;
        }
        self.composed = COMPOSE_BUDGET - budget;
        let out = Rc::make_mut(&mut pixels);
        match over {
            Over::Frame(f) => {
                let mut unlimited = u64::MAX;
                let _ = draw(out, width, height, opaque, &f, &mut unlimited);
            }
            Over::Rect { source, blend, dx, dy, sx, sy, w, h } => {
                let (iw, n) = (u64::from(width), w as usize * 4);
                for row in 0..h {
                    let to = (((dy + row) * iw + dx) * 4) as usize;
                    let from = (((sy + row) * iw + sx) * 4) as usize;
                    put_row(&mut out[to..to + n], &source[from..from + n], opaque, blend);
                }
            }
        }
        let f = &mut self.images[index].frames[k];
        *f = Frame { id: f.id, data: pixels, opaque, ..Frame::root(Rc::default(), width, height, false) };
        true
    }

    /// a=f, once all its chunks are in: a new frame, or an edit of frame r.
    pub(super) fn load_frame(&mut self, cmd: &Command, data: Vec<u8>) {
        // The image may have gone while the chunks came in.
        let Some(index) = self.images.iter().position(|img| img.key == cmd.target) else {
            return;
        };
        let Some((w, h, rgba)) = decode(cmd, data) else { return };
        let img = &self.images[index];
        let (width, height, key) = (img.width, img.height, img.key);
        if w > width || h > height {
            return;
        }
        let opaque = cmd.format == 24;
        let mut frame = Frame {
            id: 0,
            data: Rc::new(rgba),
            x: cmd.x,
            y: cmd.y,
            w,
            h,
            opaque,
            blend: cmd.offset_x != 1 && !opaque,
            base: 0,
            bg: cmd.offset_y,
        };
        let count = img.frames.len();
        // r is the frame to edit; 0 or past the next one adds a frame.
        let number = match cmd.rows as usize {
            0 => count + 1,
            n => n.min(count + 1),
        };
        let (stored, shown) = img.parts();
        if number <= count {
            // Edit frame r: compose the data onto its pixels, which makes it
            // whole. Its base, background and offsets go; c is ignored.
            let k = number - 1;
            let after_shown = if k == img.current { 0 } else { shown };
            let after = stored - img.frames[k].data.len() + img.size() + after_shown;
            let Some(pixels) = self.coalesce(index, k) else { return };
            if self.make_room(key, stored + shown, after) {
                let index = self.index_of(key);
                self.compose_onto(index, k, pixels, Over::Frame(frame));
            }
            return;
        }
        let extra: usize = self.images.iter().map(|img| img.frames.len() - 1).sum();
        if count >= MAX_FRAMES || extra >= MAX_EXTRA_FRAMES {
            return;
        }
        if cmd.cols != 0 {
            // c: the frame it is drawn over, which must exist (kitty's EINVAL).
            let Some(other) = img.frame(cmd.cols) else { return };
            if img.frames[other].base != 0 && chain_too_large(img, other) {
                // Composed now, as kitty makes it a key frame.
                let Some((mut pixels, under_opaque)) = self.coalesce(index, other) else { return };
                let copy = if Rc::strong_count(&pixels) > 1 { u64::from(width) * u64::from(height) } else { 0 };
                let mut budget = self.budget();
                if spend(&mut budget, copy + clipped(width, height, &frame)).is_none() {
                    return;
                }
                self.composed = COMPOSE_BUDGET - budget;
                let mut unlimited = u64::MAX;
                let _ = draw(Rc::make_mut(&mut pixels).as_mut_slice(), width, height, under_opaque, &frame, &mut unlimited);
                frame = Frame { data: pixels, x: 0, y: 0, w: width, h: height, opaque: under_opaque, ..frame };
            } else {
                frame.base = img.frames[other].id;
            }
        }
        if !self.make_room(key, stored + shown, stored + shown + frame.data.len()) {
            return;
        }
        let index = self.index_of(key);
        let img = &mut self.images[index];
        img.frame_ids += 1;
        frame.id = img.frame_ids;
        img.frames.push(frame);
    }

    /// a=a: c makes a frame current, if it exists. The gaps (r, z), the
    /// state (s) and the loop count (v) do nothing to a still.
    pub(super) fn control(&mut self, cmd: &Command) {
        let Some(index) = self.find(cmd) else { return };
        let img = &self.images[index];
        let Some(k) = img.frame(cmd.cols) else { return };
        if k == img.current {
            return;
        }
        let (key, (stored, shown)) = (img.key, img.parts());
        let after = stored + img.shown_bytes(k);
        if self.make_room(key, stored + shown, after) {
            let index = self.index_of(key);
            self.images[index].current = k;
        }
    }

    /// a=c: copy or blend a w by h rectangle at X, Y in frame r onto x, y in
    /// frame c, which becomes whole; w and h default to the image's. Frames
    /// that do not exist, rectangles outside the image and overlapping ones
    /// in the same frame are refused, as kitty refuses them (ENOENT, EINVAL).
    pub(super) fn compose(&mut self, cmd: &Command) {
        let Some(index) = self.find(cmd) else { return };
        let img = &self.images[index];
        let (Some(src), Some(dest)) = (img.frame(cmd.rows), img.frame(cmd.cols)) else { return };
        let (iw, ih, key) = (u64::from(img.width), u64::from(img.height), img.key);
        let w = if cmd.w == 0 { iw } else { u64::from(cmd.w) };
        let h = if cmd.h == 0 { ih } else { u64::from(cmd.h) };
        let (dx, dy) = (u64::from(cmd.x), u64::from(cmd.y));
        let (sx, sy) = (u64::from(cmd.offset_x), u64::from(cmd.offset_y));
        if dx + w > iw || dy + h > ih || sx + w > iw || sy + h > ih {
            return;
        }
        if src == dest && sx.max(dx) < sx.min(dx) + w && sy.max(dy) < sy.min(dy) + h {
            return;
        }
        let (stored, shown) = img.parts();
        let after_shown = if dest == img.current { 0 } else { shown };
        let after = stored - img.frames[dest].data.len() + img.size() + after_shown;
        let Some((source, src_opaque)) = self.coalesce(index, src) else { return };
        let Some(pixels) = self.coalesce(index, dest) else { return };
        if !self.make_room(key, stored + shown, after) {
            return;
        }
        let index = self.index_of(key);
        let blend = cmd.compose == 0 && !src_opaque;
        self.compose_onto(index, dest, pixels, Over::Rect { source, blend, dx, dy, sx, sy, w, h });
    }

    /// d=f and d=F: delete frame r of the image i or I names, the last if r
    /// is past it, the root if r is 0. The frame after a deleted root
    /// becomes the root. The current frame stays current; if it is the one
    /// deleted, the frame before it is, or the new root. An image with only
    /// its root keeps it, but d=F then deletes the image and its placements.
    pub(super) fn delete_frame(&mut self, cmd: &Command, upper: bool) {
        if cmd.id == 0 && cmd.number == 0 {
            return;
        }
        let Some(index) = self.find(cmd) else { return };
        let count = self.images[index].frames.len();
        if count == 1 {
            if upper {
                self.remove_image(index);
            }
            return;
        }
        let img = &mut self.images[index];
        let k = (cmd.rows as usize).clamp(1, count) - 1;
        img.frames.remove(k);
        if k < img.current || (k == img.current && k > 0) {
            img.current -= 1;
        }
        // Its current frame may now need pixels of its own.
        let key = img.key;
        self.enforce_quota(key);
    }

    /// Give every placement its image's current frame, composed. A frame
    /// that cannot be composed (a base was deleted) shows nothing: its
    /// placements are not drawn. Sixel images never animate, and their
    /// placements may hold pixels erased since.
    pub(super) fn show_frames(&mut self) {
        let mut placed: Vec<u64> = self.placements.iter().filter(|p| !p.sixel).map(|p| p.image).collect();
        placed.sort_unstable();
        placed.dedup();
        let shown: Vec<Option<Rc<Vec<u8>>>> = placed
            .iter()
            .map(|&key| {
                let img = self.images.iter().find(|img| img.key == key)?;
                let mut unlimited = u64::MAX;
                img.shown().or_else(|| Some(coalesce(img, img.current, &mut unlimited)?.0))
            })
            .collect();
        self.placements.retain_mut(|p| {
            if p.sixel {
                return true;
            }
            let i = placed.binary_search(&p.image).expect("listed above");
            match &shown[i] {
                Some(pixels) => {
                    p.pixels = Rc::clone(pixels);
                    true
                }
                None => false,
            }
        });
    }
}

#[cfg(test)]
#[path = "animation_tests.rs"]
mod tests;
