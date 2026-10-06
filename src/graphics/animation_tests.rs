//! Animation frames (#44): a=f, a=c, a=a and d=f/F, checked against pixels
//! worked out here from the protocol's rules, and the limits on frames.
use super::*;
use crate::{replay_sized, Lf};

const R: [u8; 4] = [255, 0, 0, 255];
const G: [u8; 4] = [0, 255, 0, 255];
const B: [u8; 4] = [0, 0, 255, 255];
const W: [u8; 4] = [255, 255, 255, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

fn b64(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

fn px(pixels: &[[u8; 4]]) -> String {
    b64(&pixels.concat())
}

fn rgb(pixels: &[[u8; 4]]) -> String {
    b64(&pixels.iter().flat_map(|p| [p[0], p[1], p[2]]).collect::<Vec<_>>())
}

fn run(g: &mut Graphics, cmd: &str) {
    g.command(cmd.as_bytes(), 0, 0, (10, 20), 10);
}

/// A stored image `id`, w by h RGBA, with a placement.
fn image(id: u32, w: u32, h: u32, pixels: &[[u8; 4]]) -> Graphics {
    let mut g = Graphics::default();
    run(&mut g, &format!("a=T,i={id},s={w},v={h};{}", px(pixels)));
    g
}

fn img(g: &Graphics, id: u32) -> &Image {
    g.images.iter().find(|img| img.id == id).unwrap()
}

/// Frame `n` (1-based) of image `id`, composed.
fn frame(g: &Graphics, id: u32, n: usize) -> Option<Vec<u8>> {
    coalesce(img(g, id), n - 1, &mut u64::MAX.clone()).map(|(p, _)| p.to_vec())
}

/// What image `id` shows: its current frame.
fn shown(g: &Graphics, id: u32) -> Option<Vec<u8>> {
    frame(g, id, img(g, id).current + 1)
}

fn frames(g: &Graphics, id: u32) -> usize {
    img(g, id).frames.len()
}

fn current(g: &Graphics, id: u32) -> usize {
    img(g, id).current + 1
}

/// The placements' pixels after `finish`, as the render gets them.
fn drawn(g: Graphics) -> Vec<Vec<u8>> {
    g.finish(&[], (10, 20), 10).iter().map(|p| p.pixels.to_vec()).collect()
}

/// a over b as the render blends, for an opaque b.
fn over(a: [u8; 4], b: [u8; 4]) -> [u8; 4] {
    let alpha = u32::from(a[3]);
    let c = |i: usize| ((u32::from(a[i]) * alpha + u32::from(b[i]) * (255 - alpha) + 127) / 255) as u8;
    [c(0), c(1), c(2), 255]
}

/// The straight-alpha over operator, in floating point, rounded half up.
fn over_float(s: [u8; 4], d: [u8; 4]) -> [u8; 4] {
    let (a, da) = (f64::from(s[3]) / 255.0, f64::from(d[3]) / 255.0);
    let out = a + da * (1.0 - a);
    if out == 0.0 {
        return d;
    }
    let c = |i: usize| ((f64::from(s[i]) * a + f64::from(d[i]) * da * (1.0 - a)) / out + 0.5).floor() as u8;
    [c(0), c(1), c(2), (out * 255.0 + 0.5).floor() as u8]
}

#[test]
fn animation_keys_parse_as_kitty_reads_them() {
    let cmd = Command::parse(b"a=f,i=1,r=2,c=3,x=4,y=5,X=1,Y=4278190335,z=-1,s=2,v=1").unwrap();
    assert_eq!((cmd.action, cmd.rows, cmd.cols, cmd.x, cmd.y), (b'f', 2, 3, 4, 5));
    assert_eq!((cmd.offset_x, cmd.offset_y, cmd.z), (1, 0xff0000ff, -1));
    let cmd = Command::parse(b"a=a,I=7,s=3,v=5,r=1,z=40,c=2").unwrap();
    assert_eq!((cmd.width, cmd.height, cmd.rows, cmd.z, cmd.cols), (3, 5, 1, 40, 2));
    // C is any number as a composition mode, and only 0 or 1 otherwise.
    assert_eq!(Command::parse(b"a=c,i=1,C=2").unwrap().compose, 2);
    assert!(!Command::parse(b"a=c,i=1,C=1").unwrap().no_move);
    assert!(Command::parse(b"a=p,i=1,C=2").is_none());
    assert!(Command::parse(b"a=f,i=1,C=01").is_none());
    assert!(Command::parse(b"a=p,i=1,C=1").unwrap().no_move);
    // A continuation chunk may name a=f, and nothing else.
    assert!(Command::parse(b"a=f,m=1,q=2").unwrap().continuation);
    assert!(!Command::parse(b"a=t,m=1").unwrap().continuation);
    assert!(!Command::parse(b"a=f,i=1,m=1").unwrap().continuation);
    // The gap is a 32-bit signed integer.
    assert!(Command::parse(b"a=f,i=1,z=-2147483649").is_none());
}

#[test]
fn a_new_frame_is_added_and_the_root_stays_current() {
    let mut g = image(1, 2, 1, &[R, G]);
    run(&mut g, &format!("a=f,i=1,s=2,v=1;{}", px(&[B, W])));
    assert_eq!(frames(&g, 1), 2);
    assert_eq!(frame(&g, 1, 2).unwrap(), [B, W].concat());
    // Shown as a still: the root, until a=a makes another frame current.
    assert_eq!(current(&g, 1), 1);
    assert_eq!(drawn(g), [[R, G].concat()]);
}

#[test]
fn a_a_makes_a_frame_current_and_nothing_else_changes_it() {
    let mut g = image(1, 1, 1, &[R]);
    for color in [G, B] {
        run(&mut g, &format!("a=f,i=1,s=1,v=1;{}", px(&[color])));
    }
    run(&mut g, "a=a,i=1,c=3");
    assert_eq!(current(&g, 1), 3);
    // Gaps, the state and loops are parsed, and change nothing.
    for keys in ["s=3", "s=2,v=1", "s=1", "v=4", "r=1,z=100", "r=2,z=-1", "s=7", "c=0", "c=4", "r=9,z=5"] {
        run(&mut g, &format!("a=a,i=1,{keys}"));
        assert_eq!(current(&g, 1), 3, "{keys}");
    }
    run(&mut g, "a=a,i=1,c=2,s=3,v=1");
    assert_eq!(current(&g, 1), 2);
    // An image that does not exist, and a=a without i or I, do nothing.
    run(&mut g, "a=a,i=2,c=1");
    run(&mut g, "a=a,c=1");
    assert_eq!(current(&g, 1), 2);
    assert_eq!(drawn(g), [G.to_vec()]);
}

#[test]
fn a_partial_frame_is_drawn_over_its_background_colour() {
    let half = [255, 0, 0, 128];
    // Blended (X=0) over Y, a translucent green, outside it Y alone.
    let mut g = image(1, 3, 1, &[W, W, W]);
    run(&mut g, &format!("a=f,i=1,x=1,s=1,v=1,Y={};{}", 0x00ff0088u32, px(&[half])));
    let bg = [0, 255, 0, 0x88];
    assert_eq!(frame(&g, 1, 2).unwrap(), [bg, over_float(half, bg), bg].concat());
    // Overwritten (X=1), and by default over transparent black.
    run(&mut g, &format!("a=f,i=1,x=1,s=1,v=1,X=1;{}", px(&[half])));
    assert_eq!(frame(&g, 1, 3).unwrap(), [CLEAR, half, CLEAR].concat());
    // Any X but 1 blends, as kitty reads it: over transparent black, the
    // pixel itself.
    run(&mut g, &format!("a=f,i=1,s=1,v=1,X=2;{}", px(&[half])));
    assert_eq!(frame(&g, 1, 4).unwrap(), [half, CLEAR, CLEAR].concat());
    // An RGB frame is opaque: its background is too, black by default, and
    // Y's alpha is dropped.
    run(&mut g, &format!("a=f,i=1,f=24,x=2,s=1,v=1;{}", rgb(&[B])));
    assert_eq!(frame(&g, 1, 5).unwrap(), [[0, 0, 0, 255], [0, 0, 0, 255], B].concat());
    run(&mut g, &format!("a=f,i=1,f=24,s=1,v=1,Y={};{}", 0x11223300u32, rgb(&[B])));
    assert_eq!(frame(&g, 1, 6).unwrap(), [B, [0x11, 0x22, 0x33, 255], [0x11, 0x22, 0x33, 255]].concat());
    // A frame the image's size has no background: it is its data.
    run(&mut g, &format!("a=f,i=1,s=3,v=1,Y=4278190335;{}", px(&[half, CLEAR, G])));
    assert_eq!(frame(&g, 1, 7).unwrap(), [half, CLEAR, G].concat());
}

#[test]
fn a_frame_with_a_base_is_drawn_over_the_base_frame() {
    let half = [0, 0, 255, 128];
    let mut g = image(1, 2, 2, &[R, G, B, W]);
    // c=1: over the root, blended.
    run(&mut g, &format!("a=f,i=1,c=1,x=1,y=1,s=1,v=1;{}", px(&[half])));
    assert_eq!(frame(&g, 1, 2).unwrap(), [R, G, B, over(half, W)].concat());
    assert_eq!(img(&g, 1).frames[1].base, 1);
    // c=2: over frame 2, overwriting, alpha and all.
    run(&mut g, &format!("a=f,i=1,c=2,s=1,v=1,X=1;{}", px(&[half])));
    assert_eq!(frame(&g, 1, 3).unwrap(), [half, G, B, over(half, W)].concat());
    // The base is drawn when it is needed: editing it changes this frame.
    run(&mut g, &format!("a=f,i=1,r=2,x=1,s=1,v=1,X=1;{}", px(&[CLEAR])));
    assert_eq!(frame(&g, 1, 3).unwrap(), [half, CLEAR, B, over(half, W)].concat());
    // A base that does not exist refuses the frame (kitty's EINVAL).
    run(&mut g, &format!("a=f,i=1,c=4,s=1,v=1;{}", px(&[R])));
    run(&mut g, &format!("a=f,i=1,c=9,s=1,v=1;{}", px(&[R])));
    assert_eq!(frames(&g, 1), 3);
}

#[test]
fn an_opaque_base_stays_opaque() {
    // RGB is kept opaque: what is put on it keeps its alpha.
    let mut g = Graphics::default();
    run(&mut g, &format!("a=t,i=1,f=24,s=2,v=1;{}", rgb(&[R, G])));
    let half = [0, 0, 255, 128];
    run(&mut g, &format!("a=f,i=1,c=1,s=2,v=1,X=1;{}", px(&[half, CLEAR])));
    assert_eq!(frame(&g, 1, 2).unwrap(), [[0, 0, 255, 255], [0, 0, 0, 255]].concat());
    run(&mut g, &format!("a=f,i=1,c=1,s=2,v=1;{}", px(&[half, CLEAR])));
    assert_eq!(frame(&g, 1, 3).unwrap(), [over(half, R), G].concat());
}

#[test]
fn editing_a_frame_composes_onto_it_and_makes_it_whole() {
    let half = [0, 255, 0, 128];
    let mut g = image(1, 2, 1, &[R, B]);
    // r=1 edits the root: it is current, so the image shows the edit.
    run(&mut g, &format!("a=f,i=1,r=1,x=1,s=1,v=1;{}", px(&[half])));
    assert_eq!((frames(&g, 1), shown(&g, 1).unwrap()), (1, [R, over(half, B)].concat()));
    // r=2 edits frame 2, which becomes whole: its base, offsets and
    // background go, and c is ignored.
    run(&mut g, &format!("a=f,i=1,c=1,x=1,s=1,v=1,Y=4278190335;{}", px(&[W])));
    run(&mut g, &format!("a=f,i=1,r=2,c=9,s=1,v=1,X=1;{}", px(&[CLEAR])));
    let f = &img(&g, 1).frames[1];
    assert_eq!((f.base, f.x, f.w, f.bg, f.data.len()), (0, 0, 2, 0, 8));
    assert_eq!(frame(&g, 1, 2).unwrap(), [CLEAR, W].concat());
    // r one past the last frame, or further, adds one.
    run(&mut g, &format!("a=f,i=1,r=3,s=1,v=1;{}", px(&[G])));
    run(&mut g, &format!("a=f,i=1,r=99,s=1,v=1;{}", px(&[B])));
    assert_eq!(frames(&g, 1), 4);
    // A frame larger than the image is refused.
    run(&mut g, &format!("a=f,i=1,r=1,s=3,v=1;{}", px(&[G, G, G])));
    run(&mut g, &format!("a=f,i=1,s=1,v=2;{}", px(&[G, G])));
    assert_eq!(frames(&g, 1), 4);
    assert_eq!(shown(&g, 1).unwrap(), [R, over(half, B)].concat());
    // Data past the image is clipped.
    run(&mut g, &format!("a=f,i=1,r=1,x=1,s=2,v=1,X=1;{}", px(&[W, W])));
    assert_eq!(shown(&g, 1).unwrap(), [R, W].concat());
    run(&mut g, &format!("a=f,i=1,r=1,x=4294967295,y=4294967295,s=1,v=1;{}", px(&[G])));
    assert_eq!(shown(&g, 1).unwrap(), [R, W].concat());
    // Wholly outside, past the right edge on a row inside: an edit, and a
    // new frame over a base or a background, drawn nothing.
    run(&mut g, &format!("a=f,i=1,r=1,x=5,s=1,v=1;{}", px(&[G])));
    assert_eq!(shown(&g, 1).unwrap(), [R, W].concat());
    run(&mut g, &format!("a=f,i=1,c=1,x=2,s=1,v=1;{}", px(&[G])));
    run(&mut g, &format!("a=f,i=1,x=9,s=1,v=1,Y=4278190335;{}", px(&[G])));
    assert_eq!(frame(&g, 1, 5).unwrap(), [R, W].concat());
    assert_eq!(frame(&g, 1, 6).unwrap(), [[255, 0, 0, 255]; 2].concat());
    run(&mut g, "a=a,i=1,c=6");
    assert_eq!(drawn(g), [[[255, 0, 0, 255]; 2].concat()]);
}

#[test]
fn frames_take_every_format_chunked_and_compressed() {
    // The 2x2 PNG of tests/fixtures/kitty-png.pty: red, green, blue, yellow.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAFElEQVR4nGP4z8DAAMIM////ZwAAHu8E/KPItPcAAAAASUVORK5CYII=";
    let yellow = [255, 255, 0, 255];
    let mut g = image(1, 2, 2, &[W, W, W, W]);
    run(&mut g, &format!("a=f,i=1,f=100;{PNG}"));
    assert_eq!(frame(&g, 1, 2).unwrap(), [R, G, B, yellow].concat());
    // A zlib stream of one stored block, with its Adler-32.
    let data = [G, B].concat();
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    let mut z = vec![0x78, 0x01, 1, 8, 0, !8, !0];
    z.extend_from_slice(&data);
    z.extend_from_slice(&(b << 16 | a).to_be_bytes());
    run(&mut g, &format!("a=f,i=1,o=z,y=1,s=2,v=1;{}", b64(&z)));
    assert_eq!(frame(&g, 1, 3).unwrap(), [CLEAR, CLEAR, G, B].concat());
    // Chunks: later ones have only m and q, and may name a=f. The cursor
    // and the image are the first chunk's.
    let all = px(&[B, B, B, B]);
    run(&mut g, &format!("a=f,i=1,s=2,v=2,m=1;{}", &all[..8]));
    run(&mut g, &format!("a=f,m=1,q=2;{}", &all[8..12]));
    run(&mut g, &format!("m=0;{}", &all[12..]));
    assert_eq!(frame(&g, 1, 4).unwrap(), [B, B, B, B].concat());
    // A chunk with other keys starts again: here a frame with no image.
    run(&mut g, &format!("a=f,i=1,s=2,v=2,m=1;{}", &all[..8]));
    run(&mut g, &format!("a=f,r=1,m=0;{}", &all[8..]));
    // a=f does not continue an a=t upload.
    run(&mut g, &format!("a=t,i=2,s=2,v=2,m=1;{}", &all[..8]));
    run(&mut g, &format!("a=f,m=0;{}", &all[8..]));
    assert_eq!((frames(&g, 1), g.images.len()), (4, 1));
    // A missing image refuses the upload before its data.
    run(&mut g, &format!("a=f,i=5,s=2,v=2,m=1;{}", &all[..8]));
    assert!(g.pending.is_none());
}

#[test]
fn a_frame_on_a_long_chain_is_composed_at_once() {
    // kitty composes a new frame now when its base is drawn over 4 others
    // or over frames covering twice the image; otherwise later.
    let mut g = image(1, 4, 4, &[W; 16]);
    for n in 1..=4 {
        run(&mut g, &format!("a=f,i=1,c={n},x={},s=1,v=1;{}", n - 1, px(&[R])));
        assert_eq!(img(&g, 1).frames[n].base, n as u32, "frame {}", n + 1);
    }
    // Frame 5 is on frames 4, 3, 2 and the root: its successor is whole.
    run(&mut g, &format!("a=f,i=1,c=5,y=1,s=1,v=1;{}", px(&[G])));
    let f = &img(&g, 1).frames[5];
    assert!(f.whole(4, 4));
    let mut want = vec![R; 4];
    want.extend([G, W, W, W]);
    want.extend([W; 8]);
    assert_eq!(frame(&g, 1, 6).unwrap(), want.concat());
    // A base of more than half the image, then another: twice the area.
    let mut g = image(1, 2, 1, &[W, W]);
    run(&mut g, &format!("a=f,i=1,c=1,s=2,v=1;{}", px(&[R, R])));
    run(&mut g, &format!("a=f,i=1,c=2,s=1,v=1;{}", px(&[G])));
    assert!(img(&g, 1).frames[2].whole(2, 1));
}

#[test]
fn composing_a_chain_stops_past_32_bases() {
    let mut g = image(1, 1, 1, &[R]);
    // Built by hand: the protocol keeps chains shorter (above).
    let root = img(&g, 1).frames[0].clone();
    let images = &mut g.images;
    for id in 2..=34 {
        let f = Frame { id, base: id - 1, blend: false, data: Rc::new(G.to_vec()), ..root.clone() };
        images[0].frames.push(f);
    }
    assert_eq!(frame(&g, 1, 33).unwrap(), G);
    assert_eq!(frame(&g, 1, 34), None);
}

#[test]
fn a_c_copies_or_blends_a_rectangle_between_frames() {
    let half = [255, 0, 0, 128];
    let mut g = image(1, 2, 2, &[W, W, W, W]);
    run(&mut g, &format!("a=f,i=1,s=2,v=2;{}", px(&[half, G, B, half])));
    // Blend (C=0) the top left pixel of frame 2 onto the root's bottom right.
    run(&mut g, "a=c,i=1,r=2,c=1,w=1,h=1,x=1,y=1");
    assert_eq!(shown(&g, 1).unwrap(), [W, W, W, over(half, W)].concat());
    // Overwrite (any nonzero C) the bottom row of frame 2 onto the top.
    run(&mut g, "a=c,i=1,r=2,c=1,h=1,Y=1,C=2");
    assert_eq!(shown(&g, 1).unwrap(), [B, half, W, over(half, W)].concat());
    // w and h default to the image's: all of the root onto frame 2.
    run(&mut g, "a=c,i=1,r=1,c=2,C=1");
    assert_eq!(frame(&g, 1, 2).unwrap(), [B, half, W, over(half, W)].concat());
    // Within one frame, rectangles that do not overlap.
    run(&mut g, "a=c,i=1,r=2,c=2,w=1,h=1,X=0,Y=1,x=1,y=0,C=1");
    assert_eq!(frame(&g, 1, 2).unwrap(), [B, W, W, over(half, W)].concat());
    // Refused: missing frames, rectangles out of the image, overlapping ones.
    for keys in ["r=3,c=1", "r=2,c=0", "r=0,c=1", "r=2,c=1,w=2,x=1", "r=2,c=1,h=3", "r=2,c=1,X=1,w=2", "r=2,c=1,Y=2,h=1", "r=1,c=1,w=2,h=1,y=0,Y=0", "r=1,c=1,w=2,h=2,x=1,y=1"] {
        let before = (frame(&g, 1, 1), frame(&g, 1, 2));
        run(&mut g, &format!("a=c,i=1,{keys},C=1"));
        assert_eq!((frame(&g, 1, 1), frame(&g, 1, 2)), before, "{keys}");
    }
    // An image that does not exist.
    run(&mut g, "a=c,i=4,r=1,c=2");
    run(&mut g, "a=c,r=1,c=2");
}

#[test]
fn a_c_onto_an_opaque_frame_keeps_it_opaque() {
    let mut g = Graphics::default();
    run(&mut g, &format!("a=t,i=1,f=24,s=2,v=1;{}", rgb(&[R, G])));
    run(&mut g, &format!("a=f,i=1,s=2,v=1;{}", px(&[[0, 0, 255, 0], [0, 0, 255, 128]])));
    run(&mut g, "a=c,i=1,r=2,c=1,C=1");
    assert_eq!(frame(&g, 1, 1).unwrap(), [[0, 0, 255, 255], [0, 0, 255, 255]].concat());
    // An opaque source overwrites alpha and all, even when blending.
    let mut g = image(1, 1, 1, &[CLEAR]);
    run(&mut g, &format!("a=f,i=1,f=24,s=1,v=1;{}", rgb(&[G])));
    run(&mut g, "a=c,i=1,r=2,c=1");
    assert_eq!(shown(&g, 1).unwrap(), G);
}

#[test]
fn deleting_frames_keeps_the_current_one_or_the_one_before() {
    let colors = [R, G, B, W];
    let setup = |current: usize| {
        let mut g = image(1, 1, 1, &[R]);
        for c in &colors[1..] {
            run(&mut g, &format!("a=f,i=1,s=1,v=1;{}", px(&[*c])));
        }
        run(&mut g, &format!("a=a,i=1,c={current}"));
        g
    };
    // (current, r, current after, what shows).
    for (cur, r, after, want) in [
        (3, 3, 2, G), // the current frame: the one before it
        (3, 2, 2, B), // one before it: the same frame
        (3, 4, 3, B), // one after it
        (4, 4, 3, B), // the last, current
        (4, 9, 3, B), // r past the last deletes it
        (1, 1, 1, G), // the root, current: the next frame is the root
        (1, 0, 1, G), // r=0 is the root
        (2, 1, 1, G),
    ] {
        let mut g = setup(cur);
        run(&mut g, &format!("a=d,d=f,i=1,r={r}"));
        assert_eq!((frames(&g, 1), current(&g, 1)), (3, after), "current {cur}, r={r}");
        assert_eq!(shown(&g, 1).unwrap(), want, "current {cur}, r={r}");
        assert_eq!(drawn(g), [want.to_vec()]);
    }
    // d=F is d=f while the image has frames past the root.
    let mut g = setup(1);
    run(&mut g, "a=d,d=F,i=1,r=2");
    assert_eq!(frames(&g, 1), 3);
    // With only the root, d=f does nothing and d=F deletes the image and
    // its placements.
    let mut g = image(1, 1, 1, &[R]);
    run(&mut g, "a=d,d=f,i=1");
    assert_eq!((g.images.len(), g.placements.len()), (1, 1));
    run(&mut g, "a=d,d=F,i=1");
    assert_eq!((g.images.len(), g.placements.len()), (0, 0));
    // Without i or I, or for an image that does not exist, nothing.
    let mut g = setup(1);
    run(&mut g, "a=d,d=f,r=2");
    run(&mut g, "a=d,d=f,i=2,r=2");
    assert_eq!(frames(&g, 1), 4);
}

#[test]
fn a_frame_whose_base_is_deleted_shows_nothing() {
    let mut g = image(1, 2, 1, &[R, G]);
    run(&mut g, &format!("a=f,i=1,c=1,s=1,v=1;{}", px(&[B])));
    run(&mut g, "a=a,i=1,c=2");
    assert_eq!(shown(&g, 1).unwrap(), [B, G].concat());
    // Deleting the root makes frame 2 the root, drawn over a frame gone.
    run(&mut g, "a=d,d=f,i=1,r=1");
    assert_eq!((frames(&g, 1), current(&g, 1), shown(&g, 1)), (1, 1, None));
    assert!(drawn(g).is_empty());
}

#[test]
fn relative_placements_on_a_placement_not_drawn_are_not_drawn() {
    let mut g = image(1, 2, 1, &[R, G]);
    run(&mut g, "a=p,i=1,p=1,C=1");
    run(&mut g, &format!("a=T,i=2,P=1,Q=1,H=1,s=1,v=1;{}", px(&[B])));
    run(&mut g, &format!("a=f,i=1,c=1,s=1,v=1;{}", px(&[W])));
    run(&mut g, "a=a,i=1,c=2");
    assert_eq!(g.placements.len(), 3);
    // Its current frame stranded, image 1 is not drawn, nor its child.
    run(&mut g, "a=d,d=f,i=1,r=1");
    assert!(drawn(g).is_empty());
}

#[test]
fn placements_let_go_of_pixels_a_frame_no_longer_has() {
    let mut g = image(1, 2, 1, &[R, G]);
    run(&mut g, "a=p,i=1,p=2,C=1");
    let root = Rc::clone(&img(&g, 1).frames[0].data);
    assert_eq!(Rc::strong_count(&root), 4);
    // After an edit, the placements no longer hold the old pixels, which
    // the quota no longer counts.
    run(&mut g, &format!("a=f,i=1,r=1,s=1,v=1;{}", px(&[B])));
    assert_eq!(Rc::strong_count(&root), 1);
    assert!(g.placements.iter().all(|p| p.pixels.is_empty()));
    run(&mut g, &format!("a=f,i=1,s=2,v=1;{}", px(&[W, W])));
    run(&mut g, "a=a,i=1,c=2");
    run(&mut g, "a=p,i=1,p=3,C=1");
    run(&mut g, "a=c,i=1,r=2,c=2,w=1,h=1,x=1,C=1");
    run(&mut g, "a=d,d=f,i=1,r=2");
    assert!(g.placements.iter().all(|p| p.pixels.is_empty()));
    // The end gives them the current frame's: the edited root.
    assert_eq!(drawn(g), vec![[B, G].concat(); 3]);
}

#[test]
fn retransmitting_an_image_resets_its_frames() {
    let mut g = image(1, 1, 1, &[R]);
    run(&mut g, &format!("a=f,i=1,s=1,v=1;{}", px(&[G])));
    run(&mut g, "a=a,i=1,c=2");
    run(&mut g, &format!("a=T,i=1,s=1,v=1;{}", px(&[B])));
    assert_eq!((frames(&g, 1), current(&g, 1)), (1, 1));
    assert_eq!(drawn(g), [B.to_vec()]);
}

#[test]
fn image_numbers_name_the_newest_image() {
    let mut g = Graphics::default();
    for color in [R, G] {
        run(&mut g, &format!("a=T,I=7,s=1,v=1;{}", px(&[color])));
    }
    run(&mut g, &format!("a=f,I=7,s=1,v=1;{}", px(&[B])));
    run(&mut g, "a=a,I=7,c=2");
    let newest = g.images.iter().max_by_key(|img| img.key).unwrap();
    assert_eq!((newest.frames.len(), newest.current), (2, 1));
    let mut shown = drawn(g);
    shown.sort();
    assert_eq!(shown, [B.to_vec(), R.to_vec()]);
}

#[test]
fn every_placement_shows_the_current_frame() {
    // Put before and after the frame changes, moved, relative, virtual
    // through a placeholder, with crops, offsets and z-indexes.
    const P: &str = "\u{10EEEE}";
    let mut log = format!("\x1b_Ga=T,i=1,p=1,s=2,v=1,C=1;{}\x1b\\", px(&[R, G]));
    log += &format!("\x1b_Ga=f,i=1,s=2,v=1;{}\x1b\\", px(&[B, W]));
    log += "\x1b[2;1H\x1b_Ga=p,i=1,p=2,x=1,w=1,z=-1,C=1\x1b\\";
    log += "\x1b_Ga=a,i=1,c=2\x1b\\";
    log += "\x1b[3;1H\x1b_Ga=p,i=1,p=3,X=3,Y=2,z=5,C=1\x1b\\";
    log += "\x1b_Ga=p,i=1,p=4,P=1,Q=1,H=3,V=4\x1b\\";
    log += "\x1b_Ga=p,i=1,p=5,U=1,c=2,r=1\x1b\\";
    log += &format!("\x1b[6;1H\x1b[38;5;1m\x1b[58;5;5m{P}{P}");
    let g = replay_sized(log.as_bytes(), 20, 10, Lf::Index, (10, 20));
    assert_eq!(g.images.len(), 5);
    for p in &g.images {
        assert_eq!(*p.pixels, [B, W].concat());
    }
}

#[test]
fn sixel_images_are_never_animated() {
    let mut g = Graphics::default();
    let image = crate::sixel::Image { width: 1, height: 1, rgba: R.to_vec() };
    g.sixel(&crate::sixel::kitty_command(&image), 0, 0, (1, 1), 10);
    g.erase_sixel(0, 0, 1, 1);
    // A Sixel image has no id or number, so no animation command names it.
    run(&mut g, &format!("a=f,s=1,v=1;{}", px(&[G])));
    assert_eq!(drawn(g), [CLEAR.to_vec()]);
}

#[test]
fn screens_keep_their_own_frames_and_resets_free_them() {
    let image = |c: [u8; 4]| format!("\x1b_Ga=T,i=1,s=1,v=1,C=1;{}\x1b\\", px(&[c]));
    let frame = |c: [u8; 4]| format!("\x1b_Ga=f,i=1,s=1,v=1;{}\x1b\\\x1b_Ga=a,i=1,c=2\x1b\\", px(&[c]));
    let shown = |log: String| {
        let g = replay_sized(log.as_bytes(), 20, 10, Lf::Index, (10, 20));
        g.images.iter().map(|p| p.pixels.to_vec()).collect::<Vec<_>>()
    };
    // The alternate screen's image 1 is another image: its frames and
    // control leave the main screen's alone.
    let log = image(R) + "\x1b[?1049h" + &image(G) + &frame(B) + "\x1b[?1049l";
    assert_eq!(shown(log), [R.to_vec()]);
    let log = image(R) + &frame(G) + "\x1b[?1049h" + &image(B) + "\x1b_Ga=a,i=1,c=1\x1b\\\x1b[?1049l";
    assert_eq!(shown(log), [G.to_vec()]);
    // A full reset and a full-screen erase free the images and their frames.
    for reset in ["\x1bc", "\x1b[2J"] {
        let log = image(R) + &frame(G) + reset + &frame(B);
        assert!(shown(log).is_empty(), "{reset:?}");
    }
}

#[test]
fn frames_count_against_the_storage_quota() {
    // A frame over the quota frees images without a placement, then is
    // refused; it never evicts a placed image.
    let big = |w: u32| format!("a=f,i=1,s={w},v=1;{}", px(&vec![G; w as usize]));
    let mut g = image(1, 4096, 1, &[R; 4096]);
    run(&mut g, &format!("a=t,i=2,s=1,v=1;{}", px(&[B])));
    run(&mut g, &format!("a=T,i=3,s=1,v=1;{}", px(&[B])));
    let total = |g: &Graphics| g.images.iter().map(Image::bytes).sum::<usize>();
    let frame_bytes = 4096 * 4;
    let room = (MAX_BYTES - total(&g)) / frame_bytes;
    for _ in 0..room {
        run(&mut g, &big(4096));
    }
    assert_eq!(frames(&g, 1), 1 + room);
    assert_eq!(g.images.len(), 3);
    run(&mut g, &big(4096));
    assert_eq!(frames(&g, 1), 1 + room);
    // Image 2 had no placement: it went to make room, and still not enough.
    let mut ids: Vec<_> = g.images.iter().map(|img| img.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, [1, 3]);
    assert!(total(&g) <= MAX_BYTES);
    // A smaller frame still fits.
    run(&mut g, &big(1));
    assert_eq!(frames(&g, 1), 2 + room);
    // A new image over the quota evicts by least recent use, counting every
    // frame: image 1 goes, frames and all.
    run(&mut g, &format!("a=T,i=4,s=1024,v=1024;{}", px(&vec![W; 1024 * 1024])));
    let mut ids: Vec<_> = g.images.iter().map(|img| img.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, [3, 4]);
}

#[test]
fn a_shown_frame_composed_apart_counts_too() {
    let mut g = image(1, 2, 1, &[R, G]);
    run(&mut g, &format!("a=f,i=1,c=1,s=1,v=1;{}", px(&[B])));
    assert_eq!(img(&g, 1).bytes(), 8 + 4);
    // Showing frame 2 needs pixels of its own, the image's size.
    run(&mut g, "a=a,i=1,c=2");
    assert_eq!(img(&g, 1).bytes(), 8 + 4 + 8);
    // Editing it makes it whole: its pixels are what is shown.
    run(&mut g, &format!("a=f,i=1,r=2,s=1,v=1;{}", px(&[W])));
    assert_eq!(img(&g, 1).bytes(), 8 + 8);
    // Refused when the quota cannot fit them.
    let mut g = image(1, 1024, 1024, &vec![R; 1024 * 1024]);
    for _ in 0..2 {
        run(&mut g, &format!("a=f,i=1,c=1,s=1,v=1;{}", px(&[B])));
    }
    // A placed image fills the quota but for 4 MiB less a byte.
    run(&mut g, &format!("a=T,i=9,s=1,v=1;{}", px(&[R])));
    let filler = g.images.iter_mut().find(|img| img.id == 9).unwrap();
    Rc::make_mut(&mut filler.frames[0].data).resize(MAX_BYTES - 2 * 4 * 1024 * 1024 - 8 + 1, 0);
    run(&mut g, "a=a,i=1,c=2");
    assert_eq!(current(&g, 1), 1);
    // A byte less, and it fits.
    let filler = g.images.iter_mut().find(|img| img.id == 9).unwrap();
    Rc::make_mut(&mut filler.frames[0].data).pop();
    run(&mut g, "a=a,i=1,c=2");
    assert_eq!(current(&g, 1), 2);
    assert_eq!(g.images.iter().map(Image::bytes).sum::<usize>(), MAX_BYTES);
}

#[test]
fn frame_counts_are_limited() {
    let mut g = image(1, 1, 1, &[R]);
    for _ in 0..MAX_FRAMES + 5 {
        run(&mut g, &format!("a=f,i=1,s=1,v=1;{}", px(&[G])));
    }
    assert_eq!(frames(&g, 1), MAX_FRAMES);
    // Over all images, the frames past their roots.
    let mut g = Graphics::default();
    let per = MAX_FRAMES - 1;
    let images = MAX_EXTRA_FRAMES / per + 1;
    for id in 1..=images as u32 {
        run(&mut g, &format!("a=T,i={id},s=1,v=1;{}", px(&[R])));
        for _ in 0..per {
            run(&mut g, &format!("a=f,i={id},s=1,v=1;{}", px(&[G])));
        }
    }
    let extra: usize = g.images.iter().map(|img| img.frames.len() - 1).sum();
    assert_eq!(extra, MAX_EXTRA_FRAMES);
}

#[test]
fn composing_is_bounded() {
    let mut g = image(1, 2, 1, &[R, G]);
    run(&mut g, &format!("a=f,i=1,c=1,s=1,v=1;{}", px(&[B])));
    // A composition needs the budget for its pixels, and for a copy when
    // the frame's pixels are shared; past it, it is refused.
    g.composed = COMPOSE_BUDGET - 1;
    run(&mut g, "a=c,i=1,r=2,c=1,w=1,h=1,C=1");
    run(&mut g, &format!("a=f,i=1,r=2,s=1,v=1;{}", px(&[W])));
    assert_eq!((frame(&g, 1, 1).unwrap(), frame(&g, 1, 2).unwrap()), ([R, G].concat(), [B, G].concat()));
    assert!(!img(&g, 1).frames[1].whole(2, 1));
    // The final screen is composed regardless.
    run(&mut g, "a=a,i=1,c=2");
    assert_eq!(drawn(g), [[B, G].concat()]);
    // Plenty left: the same commands work.
    let mut g = image(1, 2, 1, &[R, G]);
    run(&mut g, &format!("a=f,i=1,c=1,s=1,v=1;{}", px(&[B])));
    run(&mut g, "a=c,i=1,r=2,c=1,w=1,h=1,x=1,C=1");
    assert_eq!(frame(&g, 1, 1).unwrap(), [R, B].concat());
    assert!(g.composed > 0 && g.composed < 16);
}

#[test]
fn integer_blending_matches_the_over_operator() {
    // Over an opaque pixel, the render's blend; over others the over
    // operator in floating point, rounded half up.
    for &(s, d) in &[
        ([255, 0, 0, 128], [0, 0, 255, 255]),
        ([10, 200, 30, 1], [250, 5, 99, 255]),
        ([255, 0, 0, 128], [0, 255, 0, 136]),
        ([1, 2, 3, 77], [200, 100, 50, 33]),
        ([0, 0, 0, 0], [9, 9, 9, 0]),
        ([90, 80, 70, 0], [9, 9, 9, 200]),
        ([90, 80, 70, 255], [9, 9, 9, 7]),
    ] {
        let mut out = d;
        over_straight(&mut out, &s);
        assert_eq!(out, over_float(s, d), "{s:?} over {d:?}");
        if d[3] == 255 {
            assert_eq!(out, over(s, d));
        }
    }
    for a in 0..=255u8 {
        for v in [0u8, 1, 127, 128, 254, 255] {
            let (s, d) = ([v, 255 - v, a, a], [255 - v, v, 3, 255]);
            let mut out = d;
            over_straight(&mut out, &s);
            assert_eq!(out, over(s, d));
            let mut row = d;
            put_row(&mut row, &s, true, true);
            assert_eq!(row, over(s, d));
        }
    }
}
