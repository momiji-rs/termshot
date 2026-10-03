use super::*;
use crate::{replay_sized, Lf};

fn replay(s: &[u8]) -> crate::Grid {
    replay_sized(s, 20, 10, Lf::Index, (10, 20))
}
const RED: &[u8] = b"\x1b_Ga=T,f=24,s=1,v=1,c=2,r=1; /wAA\x1b\\";
fn red(extra: &str) -> Vec<u8> {
    format!("\x1b_Ga=T,f=24,s=1,v=1,c=2,r=1{extra};/wAA\x1b\\").into_bytes()
}

#[test]
fn rgb_placement_and_cursor() {
    let g = replay(&red(""));
    assert_eq!(g.cursor, Some((1, 2)));
    assert_eq!(g.images.len(), 1);
    let p = &g.images[0];
    assert_eq!(*p.pixels, [255, 0, 0, 255]);
    assert_eq!((p.x, p.slices[0].y, p.w, p.h), (0, 0, 20, 20));
}

#[test]
fn rgba_default_format_and_no_cursor_move() {
    let g = replay(b"\x1b[3;4H\x1b_Ga=T,s=1,v=1,C=1;/wAAgA==\x1b\\");
    assert_eq!(g.cursor, Some((2, 3)));
    assert_eq!(*g.images[0].pixels, [255, 0, 0, 128]);
    assert_eq!(
        (
            g.images[0].x,
            g.images[0].slices[0].y,
            g.images[0].w,
            g.images[0].h
        ),
        (30, 40, 1, 1)
    );
}

#[test]
fn chunks_commit_only_on_final_chunk_at_final_cursor() {
    let first = b"\x1b_Ga=T,f=24,s=2,v=1,c=4,r=1,m=1;/wAA\x1b\\";
    assert!(replay(first).images.is_empty());
    let mut log = first.to_vec();
    log.extend_from_slice(b"\x1b[3;4H\x1b_Gm=0,q=2;AP8A\x1b\\");
    let g = replay(&log);
    assert_eq!(g.images.len(), 1);
    assert_eq!(*g.images[0].pixels, [255, 0, 0, 255, 0, 255, 0, 255]);
    assert_eq!((g.images[0].x, g.images[0].slices[0].y), (30, 40));
    assert_eq!(g.cursor, Some((3, 7)));
}

#[test]
fn cancelled_or_incomplete_commands_do_not_place_or_leak_text() {
    for end in [b"".as_slice(), b"\x07", b"\x18", b"\x1a", b"\x1b[H"] {
        let mut log = b"\x1b_Ga=T,f=24,s=1,v=1;/wAA".to_vec();
        log.extend_from_slice(end);
        let g = replay(&log);
        assert!(g.images.is_empty());
        assert!(g.cells.iter().all(|c| c.ch == b' ' as u32));
    }
    assert!(replay(RED).images.is_empty()); // whitespace is not base64
}

#[test]
fn malformed_and_unsupported_commands_are_ignored() {
    for params in [
        "t=f",
        "t=t",
        "t=s",
        "o=z",
        "U=1",
        "x=-1",
        "w=1x",
        "X=-1",
        "Y=+1",
        "z=2147483648",
        "z=-2147483649",
        "z=--1",
        "z=-",
        "a=q",
        "a=f",
        "I=0",
        "i=1,I=1",
        "f=99",
        "s=0",
        "v=0",
        "s=4294967296",
        "c=4294967295",
        "m=2",
        "C=2",
    ] {
        // Override a key rather than duplicate it, to exercise each rejection.
        let key = params.split('=').next().unwrap();
        let mut fields: Vec<_> = ["a=T", "f=24", "s=1", "v=1"]
            .into_iter()
            .filter(|v| !v.starts_with(&format!("{key}=")))
            .collect();
        fields.push(params);
        let log = format!("\x1b_G{};/wAA\x1b\\OK", fields.join(","));
        let g = replay(log.as_bytes());
        assert!(g.images.is_empty(), "{params}");
        assert_eq!(g.cells[0].ch, b'O' as u32);
        assert_eq!(g.cells[1].ch, b'K' as u32);
    }
}

#[test]
fn base64_validation() {
    assert_eq!(base64(b"/wAA", MAX_BYTES), Some(vec![255, 0, 0]));
    assert_eq!(base64(b"/w==", MAX_BYTES), Some(vec![255]));
    assert_eq!(base64(b"/wA=", MAX_BYTES), Some(vec![255, 0]));
    for bad in [
        "/wA", "!!!!", "=AAA", "/x==", "/wB=", "/w==AAAA", "AA=A", "AA\nA",
    ] {
        assert!(base64(bad.as_bytes(), MAX_BYTES).is_none(), "{bad}");
    }
}

extern "C" {
    fn termshot_zlib_compress(data: *mut u8, len: i32, out_len: *mut i32, quality: i32) -> *mut u8;
    fn free(p: *mut std::ffi::c_void);
}

/// A zlib stream from the renderer's own compressor.
fn zlib(data: &[u8]) -> Vec<u8> {
    let mut data = data.to_vec();
    let mut len = 0;
    unsafe {
        let p = termshot_zlib_compress(data.as_mut_ptr(), data.len() as i32, &mut len, 8);
        assert!(!p.is_null());
        let z = std::slice::from_raw_parts(p, len as usize).to_vec();
        free(p.cast());
        z
    }
}

fn b64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let n = u32::from(c[0]) << 16 | u32::from(*c.get(1).unwrap_or(&0)) << 8 | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// A 2x2 RGBA PNG, and its pixels.
const PNG: &[u8] = &[
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 2, 8, 6, 0, 0, 0, 114, 182,
    13, 36, 0, 0, 0, 23, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 208, 192, 240, 31, 136, 25, 24, 254, 55,
    252, 7, 50, 0, 56, 232, 6, 252, 229, 30, 226, 71, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
];
const PNG_PIXELS: [u8; 16] = [255, 0, 0, 128, 0, 255, 0, 128, 0, 0, 255, 128, 255, 255, 0, 128];

/// The stored image's pixels after one transmission, if it loaded.
fn transmitted(cmd: &str) -> Option<Vec<u8>> {
    let mut g = Graphics::default();
    run(&mut g, (0, 0), cmd);
    g.images.first().map(|img| img.pixels.to_vec())
}

#[test]
fn compressed_payloads_inflate_to_exactly_the_size_kitty_expects() {
    let rgb = b64(&zlib(&[255, 0, 0, 0, 255, 0]));
    assert_eq!(transmitted(&format!("a=t,i=1,o=z,f=24,s=2,v=1;{rgb}")), Some(vec![255, 0, 0, 255, 0, 255, 0, 255]));
    let rgba = b64(&zlib(&[1, 2, 3, 4]));
    assert_eq!(transmitted(&format!("a=t,i=1,o=z,s=1,v=1;{rgba}")), Some(vec![1, 2, 3, 4]));
    // The stream must hold exactly the pixels: too few or too many fail.
    for size in ["s=3,v=1", "s=1,v=1"] {
        assert_eq!(transmitted(&format!("a=t,i=1,o=z,f=24,{size};{rgb}")), None, "{size}");
    }
    // A PNG inside needs its size in S; without S, kitty expects 100 KiB.
    let png = b64(&zlib(PNG));
    let n = PNG.len();
    assert_eq!(transmitted(&format!("a=t,i=1,o=z,f=100,S={n};{png}")), Some(PNG_PIXELS.to_vec()));
    for size in [format!("S={}", n - 1), format!("S={}", n + 1), "S=0".into(), "".into()] {
        assert_eq!(transmitted(&format!("a=t,i=1,o=z,f=100,{size};{png}")), None, "{size}");
    }
    let mut padded = PNG.to_vec();
    padded.resize(100 * 1024, 0);
    let padded = b64(&zlib(&padded));
    assert_eq!(transmitted(&format!("a=t,i=1,o=z,f=100;{padded}")), Some(PNG_PIXELS.to_vec()));
    assert_eq!(transmitted(&format!("a=t,i=1,o=z,f=100,S=102400;{padded}")), Some(PNG_PIXELS.to_vec()));
    // S changes nothing without compression.
    assert_eq!(transmitted("a=t,i=1,f=24,s=1,v=1,S=7;/wAA"), Some(vec![255, 0, 0, 255]));
    // Only zlib is defined; data that is not a stream fails.
    assert_eq!(transmitted(&format!("a=t,i=1,o=x,f=24,s=2,v=1;{rgb}")), None);
    assert_eq!(transmitted("a=t,i=1,o=z,f=24,s=1,v=1;/wAA"), None);
    // An inflated size over the limit fails before anything is allocated for it.
    assert_eq!(transmitted(&format!("a=t,i=1,o=z,f=24,s=8192,v=8192;{rgb}")), None);
}

#[test]
fn compressed_png_data_is_limited_to_16_mib() {
    let mut padded = PNG.to_vec();
    padded.resize(MAX_BYTES + 1, 0);
    let over = b64(&zlib(&padded));
    assert_eq!(transmitted(&format!("a=t,i=1,o=z,f=100,S={};{over}", MAX_BYTES + 1)), None);
    padded.pop();
    let limit = b64(&zlib(&padded));
    assert_eq!(transmitted(&format!("a=t,i=1,o=z,f=100,S={MAX_BYTES};{limit}")), Some(PNG_PIXELS.to_vec()));
}

#[test]
fn compressed_payloads_arrive_in_chunks() {
    let raw: Vec<u8> = (0..64 * 64 * 3).map(|i| (i % 251) as u8).collect();
    let z = b64(&zlib(&raw));
    let chunks: Vec<&str> = z.as_bytes().chunks(64).map(|c| std::str::from_utf8(c).unwrap()).collect();
    let mut g = Graphics::default();
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        let cmd = if i == 0 { format!("a=T,i=1,o=z,f=24,s=64,v=64,m={more};{chunk}") } else { format!("m={more};{chunk}") };
        run(&mut g, (0, 0), &cmd);
    }
    let want: Vec<u8> = raw.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
    assert_eq!(g.images.len(), 1);
    assert_eq!(*g.placements[0].pixels, want);
}

#[test]
fn incorrect_payload_lengths_and_dimensions() {
    for log in [
        "a=T,f=24,s=1,v=1;",
        "a=T,f=32,s=1,v=1;/wAA",
        "a=T,f=24,s=2,v=1;/wAA",
        "a=T,f=24,s=8193,v=1;/wAA",
        "a=T,f=24,s=8192,v=8192;/wAA",
        "a=T,f=24,s=4294967295,v=4294967295;/wAA",
        "a=T,f=100;bm90IHBuZw==",
    ] {
        assert!(
            replay(format!("\x1b_G{log}\x1b\\").as_bytes())
                .images
                .is_empty(),
            "{log}"
        );
    }
}

#[test]
fn deletion_by_id_and_placement_and_retransmission() {
    let mut log = red(",i=1,p=2,C=1");
    log.extend(red(",i=2,C=1"));
    log.extend_from_slice(b"\x1b_Ga=d,d=i,i=1,p=3\x1b\\");
    assert_eq!(replay(&log).images.len(), 2);
    log.extend_from_slice(b"\x1b_Ga=d,d=I,i=1,p=2\x1b\\");
    assert_eq!(replay(&log).images.len(), 1);
    log.extend(red(",i=2,C=1"));
    assert_eq!(replay(&log).images.len(), 1);
    log.extend_from_slice(b"\x1b_Ga=d\x1b\\");
    assert!(replay(&log).images.is_empty());
}

#[test]
fn deletion_cancels_partial_upload() {
    let g = replay(b"\x1b_Ga=T,f=24,s=2,v=1,m=1;/wAA\x1b\\\x1b_Ga=d\x1b\\\x1b_Gm=0;AP8A\x1b\\");
    assert!(g.images.is_empty());
}

#[test]
fn invalid_chunk_cancels_upload_and_new_command_recovers() {
    let mut log =
        b"\x1b_Ga=T,f=24,s=2,v=1,m=1;/wAA\x1b\\\x1b_Gm=1;!!!!\x1b\\\x1b_Gm=0;AP8A\x1b\\".to_vec();
    assert!(replay(&log).images.is_empty());
    log.extend(red(""));
    assert_eq!(replay(&log).images.len(), 1);
}

#[test]
fn placement_sizing_preserves_aspect_ratio() {
    for (extra, expected) in [
        (",c=4", (0, 0, 40, 40)),
        (",r=2", (0, 0, 40, 40)),
        (",c=4,r=1", (10, 0, 20, 20)),
        (",c=1,r=2", (0, 15, 10, 10)),
        ("", (0, 0, 1, 1)),
    ] {
        let g = replay(format!("\x1b_Ga=T,f=24,s=1,v=1{extra};/wAA\x1b\\").as_bytes());
        let p = &g.images[0];
        assert_eq!((p.x, p.slices[0].y, p.w, p.h), expected);
    }
}

#[test]
fn alternate_screens_and_reset() {
    let mut log = red(",C=1");
    log.extend_from_slice(b"\x1b[?1049h");
    assert!(replay(&log).images.is_empty());
    log.extend(red(",C=1"));
    log.extend_from_slice(b"\x1b[?1049l");
    assert_eq!(replay(&log).images.len(), 1);
    log.extend_from_slice(b"\x1b[?1049h");
    assert!(replay(&log).images.is_empty());
    for reset in [b"\x1bc".as_slice(), b"\x1b[2J"] {
        let mut log = red(",C=1");
        log.extend_from_slice(reset);
        assert!(replay(&log).images.is_empty());
        log.extend(red(",C=1"));
        assert_eq!(replay(&log).images[0].w, 20); // RIS preserves font metrics
    }
}

#[test]
fn scrolling_clips_images_and_reverse_scroll_cannot_restore_pixels() {
    let mut log = b"\x1b[2;1H".to_vec();
    log.extend_from_slice(b"\x1b_Ga=T,f=24,s=1,v=1,c=4,r=2,C=1;/wAA\x1b\\");
    log.extend_from_slice(b"\x1b[2S");
    let g = replay(&log);
    let p = &g.images[0];
    assert_eq!(
        (p.slices[0].y, p.slices[0].top, p.slices[0].bottom),
        (-20, 0, 20)
    );
    log.extend_from_slice(b"\x1b[T");
    let g = replay(&log);
    let p = &g.images[0];
    assert_eq!(
        (p.slices[0].y, p.slices[0].top, p.slices[0].bottom),
        (0, 20, 40)
    );
    log.extend_from_slice(b"\x1b[10S");
    assert!(replay(&log).images.is_empty());
}

#[test]
fn scroll_region_leaves_images_outside_it_alone() {
    let mut log = red(",C=1");
    log.extend_from_slice(b"\x1b[3;8r\x1b[5S");
    assert_eq!(replay(&log).images[0].slices[0].y, 0);
}

#[test]
fn z_order_then_creation_order() {
    // kitty draws by z-index, then the image's creation, then the placement's.
    let mut log = red(",C=1,i=3,z=2");
    log.extend(red(",C=1,i=2,z=0"));
    log.extend(red(",C=1,i=1,z=0"));
    log.extend_from_slice(b"\x1b_Ga=p,i=2,p=1,C=1\x1b\\");
    let order = |log: &[u8]| replay(log).images.iter().map(|p| (p.id, p.placement_id)).collect::<Vec<_>>();
    assert_eq!(order(&log), [(2, 0), (2, 1), (1, 0), (3, 0)]);
    // Moving a placement keeps its place; retransmitting keeps the image's.
    log.extend_from_slice(b"\x1b[5;5H\x1b_Ga=p,i=2,p=1,C=1\x1b\\");
    assert_eq!(order(&log), [(2, 0), (2, 1), (1, 0), (3, 0)]);
    log.extend(red(",C=1,i=2,z=0"));
    assert_eq!(order(&log), [(2, 0), (1, 0), (3, 0)]);
}

#[test]
fn placement_count_is_bounded() {
    let mut g = Graphics::default();
    for _ in 0..MAX_PLACEMENTS + 5 {
        g.command(b"a=T,f=24,s=1,v=1,C=1;/wAA", 0, 0, (1, 1), 10);
    }
    assert_eq!(g.placements.len(), MAX_PLACEMENTS);
    // An image without an id that could not be placed is not kept either.
    assert_eq!(g.images.len(), MAX_PLACEMENTS);
}

#[test]
fn randomized_graphics_do_not_panic_or_leak_escape_text() {
    let mut seed = 0x12345678u32;
    for _ in 0..2000 {
        let mut data = b"\x1b_G".to_vec();
        for _ in 0..64 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            data.push(b"a=T,f=24,s=12,v=12,m=1;AQID/+=,"[seed as usize % 30]);
        }
        data.extend_from_slice(b"\x1b\\OK");
        let g = replay(&data);
        assert!(g.images.is_empty());
        assert_eq!(g.cells[0].ch, b'O' as u32);
    }
}

#[test]
fn png_decoder_is_reentrant() {
    let jobs: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                for _ in 0..20 {
                    let g = replay(include_bytes!("../../tests/fixtures/kitty-png-alpha.pty"));
                    assert_eq!(
                        *g.images[0].pixels,
                        [255, 0, 0, 128, 0, 255, 0, 128, 0, 0, 255, 128, 255, 255, 0, 128]
                    );
                }
            })
        })
        .collect();
    for job in jobs {
        job.join().unwrap();
    }
}

#[test]
fn upload_and_retained_memory_are_bounded() {
    let mut g = Graphics::default();
    let cmd = Command::parse(b"a=T,f=24,s=1,v=1,m=1").unwrap();
    g.pending = Some((cmd, vec![0; MAX_BYTES]));
    g.command(b"m=0;/wAA", 0, 0, (1, 1), 10);
    assert!(g.pending.is_none());
    assert!(g.placements.is_empty());
    g.command(b"a=T,f=24,s=1,v=1,i=1;/wAA", 0, 0, (1, 1), 10);
    Rc::make_mut(&mut g.images[0].pixels).resize(MAX_BYTES, 0);
    // A new image over the quota evicts the old one, placement and all.
    g.command(b"a=T,f=24,s=1,v=1,i=2;/wAA", 0, 0, (1, 1), 10);
    assert_eq!(ids(&g), [2]);
    assert_eq!(g.placements.len(), 1);
    assert_eq!(g.placements[0].id, 2);
}

#[test]
fn alternate_1047_clears_images_on_exit() {
    let mut log = b"\x1b[?1047h".to_vec();
    log.extend(red(",C=1"));
    log.extend_from_slice(b"\x1b[?1047l\x1b[?47h");
    assert!(replay(&log).images.is_empty());
}

#[test]
fn padded_intermediate_chunk_is_rejected() {
    let g = replay(b"\x1b_Ga=T,f=32,s=1,v=1,m=1;/wAAgA==\x1b\\\x1b_Gm=0;\x1b\\");
    assert!(g.images.is_empty());
}

#[test]
fn extreme_font_metrics_and_requested_extents_do_not_overflow() {
    let mut g = Graphics::default();
    g.command(
        b"a=T,f=24,s=2,v=2,c=4294967295;/wAAAP8AAAD///8A",
        0,
        0,
        (i32::MAX, i32::MAX),
        1,
    );
    assert!(g.placements.is_empty());
    g.command(
        b"a=T,f=24,s=2,v=2,r=4294967295;/wAAAP8AAAD///8A",
        0,
        0,
        (i32::MAX, i32::MAX),
        1,
    );
    assert!(g.placements.is_empty());
}

#[test]
fn base64_accepts_exact_payload_limit_and_rejects_one_byte_more() {
    let mut encoded = vec![b'A'; (MAX_BYTES + 2) / 3 * 4];
    let n = encoded.len();
    encoded[n - 2..].copy_from_slice(b"==");
    assert_eq!(base64(&encoded, MAX_BYTES).unwrap().len(), MAX_BYTES);
    encoded[n - 2] = b'A';
    assert!(base64(&encoded, MAX_BYTES).is_none());
}

#[test]
fn crossing_scroll_margins_keeps_the_whole_placement_stationary() {
    // Cross upper, lower, or both margins, in both scroll directions.
    for (row, rows) in [(1, 3), (4, 3), (1, 6)] {
        for delta in [-4, -1, 1, 4] {
            let mut g = Graphics::default();
            g.command(
                format!("a=T,f=24,s=1,v=1,c={},r={rows},C=1,i=9,p=3;/wAA", rows * 2).as_bytes(),
                0,
                row,
                (10, 20),
                10,
            );
            let before = g.placements[0].slices.clone();
            let pixels = g.placements[0].pixels.as_ptr();
            g.scroll(2, 5, delta, 20);
            assert_eq!(
                g.placements[0].slices, before,
                "row={row} rows={rows} delta={delta}"
            );
            assert_eq!(g.placements[0].pixels.as_ptr(), pixels);
            g.command(b"a=d,d=i,i=9,p=3", 0, 0, (10, 20), 10);
            assert!(g.placements.is_empty());
        }
    }
}

#[test]
fn contained_placement_clips_at_both_margins_without_resurrection() {
    for delta in [-1, 1] {
        let mut g = Graphics::default();
        g.command(b"a=T,f=24,s=1,v=1,c=8,r=4,C=1;/wAA", 0, 2, (10, 20), 10);
        g.scroll(2, 5, delta, 20);
        let (y, top, bottom) = if delta < 0 {
            (20, 40, 100)
        } else {
            (60, 60, 120)
        };
        assert_eq!(g.placements[0].slices, [ImageSlice { y, top, bottom }]);
        g.scroll(2, 5, -delta, 20);
        let (top, bottom) = if delta < 0 { (60, 120) } else { (40, 100) };
        assert_eq!(g.placements[0].slices, [ImageSlice { y: 40, top, bottom }]);
        g.scroll(2, 5, delta * 4, 20);
        assert!(g.placements.is_empty());
    }
}

#[test]
fn explicit_image_id_must_be_nonzero_but_omission_is_valid() {
    for id in ["0", "000", "4294967296"] {
        let g = replay(&red(&format!(",i={id}")));
        assert!(g.images.is_empty(), "i={id}");
        assert_eq!(g.cursor, Some((0, 0)));
        assert!(g.cells.iter().all(|c| c.ch == b' ' as u32));
    }
    for extra in ["", ",i=1", ",i=4294967295"] {
        let g = replay(&red(extra));
        assert_eq!(g.images.len(), 1);
        assert_eq!(g.cursor, Some((1, 2)));
    }
}

#[test]
fn stationary_images_preserve_limits_and_replacement() {
    let mut g = Graphics::default();
    for id in 1..=MAX_PLACEMENTS {
        g.command(
            format!("a=T,f=24,s=1,v=1,c=20,r=10,C=1,i={id};/wAA").as_bytes(),
            0,
            0,
            (10, 20),
            10,
        );
    }
    g.scroll(2, 5, -1, 20);
    assert_eq!(g.placements.len(), MAX_PLACEMENTS);
    assert!(g
        .placements
        .iter()
        .all(|p| p.slices.len() == 1 && p.pixels.len() == 4));
    g.command(b"a=T,f=24,s=1,v=1,C=1,i=1;AP8A", 0, 0, (10, 20), 10);
    assert_eq!(g.placements.len(), MAX_PLACEMENTS);
    assert_eq!(*g.placements[0].pixels, [0, 255, 0, 255]);
    assert_eq!(g.placements[0].slices.len(), 1);
}

#[test]
fn repeated_partial_scrolls_match_independent_pixel_row_model() {
    let mut seed = 0x41cafeu32;
    for _ in 0..100 {
        let mut g = Graphics::default();
        g.command(b"a=T,f=24,s=1,v=1,c=20,r=10,C=1;/wAA", 0, 0, (10, 20), 10);
        let mut expected: Vec<_> = (0..200).map(Some).collect();
        for _ in 0..20 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let top = (seed as usize >> 8) % 10;
            let bottom = top + (seed as usize >> 16) % (10 - top);
            let n = 1 + (seed as usize >> 24) % (bottom - top + 1);
            let delta = if seed & 1 == 0 { n as i64 } else { -(n as i64) };
            let old = expected.clone();
            // A placement is eligible only if every surviving source row is
            // inside the margins. Otherwise its entire raster stays fixed.
            let contained = old.iter().enumerate().all(|(row, pixel)| {
                pixel.is_none() || (row >= top * 20 && row < (bottom + 1) * 20)
            });
            for row in top * 20..(bottom + 1) * 20 {
                if !contained {
                    continue;
                }
                let source = row as i64 - delta * 20;
                expected[row] =
                    if source >= (top * 20) as i64 && source < ((bottom + 1) * 20) as i64 {
                        old[source as usize]
                    } else {
                        None
                    };
            }
            g.scroll(top, bottom, delta, 20);
            let mut actual = vec![None; 200];
            for p in &g.placements {
                // A placement remains one contiguous visible rectangle.
                assert_eq!(p.slices.len(), 1);
                for pair in p.slices.windows(2) {
                    assert!(pair[0].bottom <= pair[1].top);
                    assert!(pair[0].bottom != pair[1].top || pair[0].y != pair[1].y);
                }
                for s in &p.slices {
                    assert!(s.top < s.bottom);
                    for row in s.top..s.bottom {
                        assert!(actual[row as usize].is_none());
                        actual[row as usize] = Some((row - s.y) as usize);
                    }
                }
            }
            assert_eq!(actual, expected, "top={top} bottom={bottom} delta={delta}");
        }
    }
}

#[test]
fn a_numbered_image_id_stays_within_the_preflight_bound() {
    // needs_cell_metrics assumes a numbered image's id is at most the count of
    // named transmissions so far, whatever deletes came between.
    let mut g = Graphics::default();
    let mut named = 0;
    let mut seed = 1u32;
    for _ in 0..300 {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        let pick = (seed >> 16) % 8 + 1;
        let cmd = match seed % 4 {
            0 => format!("a=t,i={pick},{PIXEL}"),
            1 => format!("a=t,I={pick},{PIXEL}"),
            2 => format!("a=d,d=I,i={pick}"),
            _ => format!("a=d,d=N,I={pick}"),
        };
        run(&mut g, (0, 0), &cmd);
        if seed % 4 < 2 {
            named += 1;
        }
        if seed % 4 == 1 {
            let newest = g.images.iter().filter(|img| img.number == pick).max_by_key(|img| img.key).unwrap();
            assert!(newest.id <= named, "{cmd}: id {} over {named}", newest.id);
        }
    }
}

#[test]
fn font_metrics_are_needed_only_for_potential_cursor_movement() {
    assert!(needs_cell_metrics(&red("")));
    assert!(!needs_cell_metrics(&red(",C=1")));
    assert!(needs_cell_metrics(b"\x1b_Ga=T,f=24,s=2,v=1,m=1;/wAA\x1b\\"));
    // Crops, offsets and a negative z-index are placed, so they move it too.
    for keys in [",x=1", ",y=1,h=1", ",X=3,Y=4", ",z=-1", ",z=-2147483648"] {
        assert!(needs_cell_metrics(&red(keys)), "{keys}");
        assert!(!needs_cell_metrics(&red(&format!("{keys},C=1"))), "{keys}");
    }
    let by_id = "\x1b_Ga=t,i=1,f=24,s=1,v=1;/wAA\x1b\\";
    let by_number = "\x1b_Ga=t,i=3,f=24,s=1,v=1;/wAA\x1b\\\x1b_Ga=t,I=7,f=24,s=1,v=1;/wAA\x1b\\";
    // A numbered image gets the lowest free id, which the scan does not know
    // but bounds by the named transmissions so far: here, at most 2.
    for (stored, put, needed) in [
        (by_id, "i=1", true),
        (by_id, "i=1,C=1", false),
        (by_id, "i=2", false),
        (by_id, "I=1", false),
        (by_number, "I=7", true),
        (by_number, "I=7,C=1", false),
        (by_number, "I=4", false),
        (by_number, "i=1", true),
        (by_number, "i=2", true),
        (by_number, "i=3", true),
        (by_number, "i=4", false),
        (by_number, "i=999", false),
        (by_number, "i=2,C=1", false),
    ] {
        assert!(!needs_cell_metrics(stored.as_bytes()));
        let put = format!("\x1b_Ga=p,{put}\x1b\\");
        // Replay ignores a put of an image never transmitted: no metrics.
        assert!(!needs_cell_metrics(put.as_bytes()), "{put:?} alone");
        assert_eq!(needs_cell_metrics(format!("{stored}{put}").as_bytes()), needed, "{stored:?}{put:?}");
        assert!(!needs_cell_metrics(format!("{put}{stored}").as_bytes()), "{put:?} first");
    }
    // A put with neither i nor I names no image, even after an anonymous one.
    assert!(!needs_cell_metrics(b"\x1b_Ga=T,C=1,f=24,s=1,v=1;/wAA\x1b\\\x1b_Ga=p\x1b\\"));
    // a=T with C=1 stores without moving; a later put may move.
    assert!(needs_cell_metrics(b"\x1b_Ga=T,i=3,C=1,f=24,s=1,v=1;/wAA\x1b\\\x1b_Ga=p,i=3\x1b\\"));
    for log in [
        b"text".as_slice(),
        b"\x1b_Ga=q,f=24,s=1,v=1;AAAA\x1b\\",
        b"\x1b_Ga=d\x1b\\",
        b"\x1b_Ga=T,t=f;AAAA\x1b\\",
        b"\x1b_Ga=T;AAAA",
        b"\x1b_Ga=T;AAAA\x18",
    ] {
        assert!(!needs_cell_metrics(log));
    }
}

const PIXEL: &str = "f=24,s=1,v=1;/wAA";
const GREEN: &str = "f=24,s=1,v=1;AP8A";

fn run(g: &mut Graphics, (col, row): (usize, usize), cmd: &str) -> Option<(usize, usize)> {
    g.command(cmd.as_bytes(), col, row, (10, 20), 10)
}

/// The ids of the stored images, sorted.
fn ids(g: &Graphics) -> Vec<u32> {
    let mut ids: Vec<_> = g.images.iter().map(|img| img.id).collect();
    ids.sort_unstable();
    ids
}

/// (image id, placement id) of each placement, in draw order.
fn placed(g: &Graphics) -> Vec<(u32, u32)> {
    g.placements.iter().map(|p| (p.id, p.placement_id)).collect()
}

#[test]
fn transmit_stores_and_put_places_at_the_cursor() {
    let mut log = format!("\x1b_Ga=t,i=5,{PIXEL}\x1b\\").into_bytes();
    let g = replay(&log);
    assert!(g.images.is_empty());
    assert_eq!(g.cursor, Some((0, 0)));
    log.extend_from_slice(b"\x1b[3;4H\x1b_Ga=p,i=5,c=2,r=1\x1b\\\x1b[6;1H\x1b_Ga=p,i=5,C=1\x1b\\");
    let g = replay(&log);
    assert_eq!(g.cursor, Some((5, 0)));
    let [a, b] = &g.images[..] else {
        panic!("{} placements", g.images.len())
    };
    assert_eq!((a.x, a.slices[0].y, a.w, a.h), (30, 40, 20, 20));
    assert_eq!((b.x, b.slices[0].y, b.w, b.h), (0, 100, 1, 1));
    // Placements share the stored pixels rather than copying them.
    assert!(Rc::ptr_eq(&a.pixels, &b.pixels));
    assert_eq!(*a.pixels, [255, 0, 0, 255]);
}

#[test]
fn placement_ids_move_a_placement_unless_the_image_has_no_id() {
    let mut g = Graphics::default();
    run(&mut g, (0, 0), &format!("a=t,i=1,{PIXEL}"));
    run(&mut g, (0, 0), "a=p,i=1,p=7,C=1");
    run(&mut g, (0, 2), "a=p,i=1,p=7,C=1");
    assert_eq!(placed(&g), [(1, 7)]);
    assert_eq!(g.placements[0].slices[0].y, 40);
    run(&mut g, (0, 0), "a=p,i=1,p=8,C=1");
    run(&mut g, (0, 0), "a=p,i=1,C=1");
    run(&mut g, (0, 0), "a=p,i=1,C=1");
    assert_eq!(placed(&g), [(1, 7), (1, 8), (1, 0), (1, 0)]);
    let mut g = Graphics::default();
    run(&mut g, (0, 0), &format!("a=T,p=7,C=1,{PIXEL}"));
    run(&mut g, (0, 0), &format!("a=T,p=7,C=1,{PIXEL}"));
    assert_eq!(placed(&g), [(0, 0), (0, 0)]);
}

#[test]
fn put_needs_a_stored_image_and_ignores_its_payload() {
    for put in ["a=p,i=9", "a=p,I=9", "a=p", "a=p,p=1", "a=p,i=1,I=1"] {
        let log = format!("\x1b_Ga=t,i=1,{PIXEL}\x1b\\\x1b_G{put}\x1b\\OK");
        let g = replay(log.as_bytes());
        assert!(g.images.is_empty(), "{put}");
        assert_eq!(g.cursor, Some((0, 2)), "{put}");
        assert_eq!(g.cells[0].ch, b'O' as u32);
    }
    let g = replay(format!("\x1b_Ga=t,i=1,{PIXEL}\x1b\\\x1b_Ga=p,i=1;!!\x1b\\").as_bytes());
    assert_eq!(g.images.len(), 1);
}

#[test]
fn retransmission_drops_old_placements_even_when_the_new_data_fails() {
    let mut g = Graphics::default();
    run(&mut g, (0, 0), &format!("a=T,i=1,C=1,{PIXEL}"));
    run(&mut g, (0, 0), "a=p,i=1,p=2,C=1");
    run(&mut g, (0, 0), &format!("a=t,i=1,{GREEN}"));
    assert!(g.placements.is_empty());
    run(&mut g, (0, 0), "a=p,i=1,C=1");
    assert_eq!(*g.placements[0].pixels, [0, 255, 0, 255]);
    run(&mut g, (0, 0), "a=t,i=1,f=24,s=2,v=1;/wAA");
    assert!(g.placements.is_empty());
    assert!(ids(&g).is_empty());
    assert_eq!(run(&mut g, (0, 0), "a=p,i=1"), None);
    assert!(g.placements.is_empty());
}

#[test]
fn image_numbers_name_the_newest_image_and_get_a_free_id() {
    let mut g = Graphics::default();
    run(&mut g, (0, 0), &format!("a=t,i=1,{PIXEL}"));
    run(&mut g, (0, 0), &format!("a=t,I=3,{PIXEL}"));
    run(&mut g, (0, 0), &format!("a=t,I=3,{GREEN}"));
    assert_eq!(ids(&g), [1, 2, 3]);
    run(&mut g, (0, 0), "a=p,I=3,C=1");
    assert_eq!(*g.placements[0].pixels, [0, 255, 0, 255]);
    assert_eq!(g.placements[0].id, 3);
    // The newest goes with its placement; the older one is the newest now.
    run(&mut g, (0, 0), "a=d,d=N,I=3");
    assert_eq!(ids(&g), [1, 2]);
    run(&mut g, (0, 0), "a=p,I=3,C=1");
    assert_eq!(*g.placements[0].pixels, [255, 0, 0, 255]);
    assert_eq!(g.placements[0].id, 2);
    run(&mut g, (0, 0), "a=d,d=n,I=3");
    assert!(g.placements.is_empty());
    assert_eq!(ids(&g), [1, 2]);
}

/// Image 1 covers cells (0..2, 0) at z 0, image 2 cells (5..7, 3..5) at z 1.
fn scene() -> Graphics {
    let mut g = Graphics::default();
    run(&mut g, (0, 0), &format!("a=T,i=1,c=2,r=1,C=1,{PIXEL}"));
    run(&mut g, (5, 3), &format!("a=T,i=2,c=2,r=2,z=1,C=1,{PIXEL}"));
    g
}

#[test]
fn delete_selectors_pick_placements_by_id_cell_and_z() {
    let both: &[u32] = &[1, 2];
    for (delete, cursor, left) in [
        ("d=x,x=1", (0, 0), &[2][..]),
        ("d=x,x=2", (0, 0), &[2]),
        ("d=x,x=3", (0, 0), both),
        ("d=x,x=6", (0, 0), &[1]),
        ("d=x,x=8", (0, 0), both),
        ("d=x", (0, 0), both), // 0 names no column
        ("d=y,y=1", (0, 0), &[2]),
        ("d=y,y=5", (0, 0), &[1]),
        ("d=y,y=6", (0, 0), both),
        ("d=p,x=7,y=4", (0, 0), &[1]),
        ("d=p,x=1,y=4", (0, 0), both),
        ("d=q,x=6,y=4,z=0", (0, 0), both),
        ("d=q,x=6,y=4,z=1", (0, 0), &[1]),
        ("d=z,z=1", (0, 0), &[1]),
        ("d=z", (0, 0), &[2]),
        ("d=c", (1, 0), &[2]),
        ("d=c", (4, 3), both),
        ("d=c", (6, 4), &[1]),
        ("d=c", (5, 3), &[1]),
        ("d=c", (2, 0), both),
        ("d=r,x=2,y=9", (0, 0), &[1]),
        ("d=r,x=1,y=2", (0, 0), &[]),
        ("d=r,x=2,y=1", (0, 0), both),
        ("d=i,i=2", (0, 0), &[1]),
        ("d=i,i=2,p=1", (0, 0), both),
        ("d=i,i=9", (0, 0), both),
        ("d=n,I=2", (0, 0), both),
        ("d=a", (0, 0), &[]),
        ("", (0, 0), &[]),
        ("d=f", (0, 0), both),
        ("d=w", (0, 0), both),
    ] {
        let mut g = scene();
        run(&mut g, cursor, &format!("a=d,{delete}"));
        let shown: Vec<_> = placed(&g).into_iter().map(|(id, _)| id).collect();
        assert_eq!(shown, left, "{delete}");
        assert_eq!(ids(&g), both, "{delete} keeps the images");
        // Uppercase removes the same placements and frees what it emptied.
        if let Some(rest) = delete.strip_prefix("d=") {
            let mut g = scene();
            let (selector, keys) = rest.split_at(1);
            run(&mut g, cursor, &format!("a=d,d={}{keys}", selector.to_ascii_uppercase()));
            let shown: Vec<_> = placed(&g).into_iter().map(|(id, _)| id).collect();
            assert_eq!(shown, left, "{delete} uppercase");
            assert_eq!(ids(&g), left, "{delete} uppercase frees");
        }
    }
}

#[test]
fn uppercase_deletes_free_images_left_unplaced() {
    // Only a selector naming the image frees one already without placements.
    for (delete, freed) in [
        ("d=I,i=3", true),
        ("d=i,i=3", false),
        ("d=I,i=3,p=1", false),
        ("d=R,x=3,y=3", true),
        ("d=r,x=1,y=5", false),
        ("d=A", false),
        ("d=X,x=1", false),
    ] {
        let mut g = scene();
        run(&mut g, (0, 0), &format!("a=t,i=3,{PIXEL}"));
        run(&mut g, (0, 0), &format!("a=d,{delete}"));
        assert_eq!(ids(&g).contains(&3), !freed, "{delete}");
    }
    // A lowercase delete frees an image without an id: nothing could place it again.
    let mut g = Graphics::default();
    run(&mut g, (0, 0), &format!("a=T,C=1,{PIXEL}"));
    run(&mut g, (0, 0), "a=d,d=a");
    assert!(g.images.is_empty());
    run(&mut g, (0, 0), &format!("a=t,{PIXEL}"));
    assert!(g.images.is_empty());
}

#[test]
fn clearing_a_screen_frees_its_stored_images() {
    let placed = |between: &[u8]| {
        let mut log = format!("\x1b_Ga=t,i=1,{PIXEL}\x1b\\").into_bytes();
        log.extend_from_slice(between);
        log.extend_from_slice(b"\x1b_Ga=p,i=1,C=1\x1b\\");
        replay(&log).images.len()
    };
    assert_eq!(placed(b""), 1);
    assert_eq!(placed(b"\x1b[2J"), 0);
    assert_eq!(placed(b"\x1bc"), 0);
    // Other erases leave graphics alone.
    assert_eq!(placed(b"\x1b[J\x1b[1J\x1b[3J\x1b[K\x1b[2K"), 1);
    // Each screen keeps its own images.
    assert_eq!(placed(b"\x1b[?1049h"), 0);
    assert_eq!(placed(b"\x1b[?1049h\x1b[2J\x1b[?1049l"), 1);
}

#[test]
fn quota_frees_unplaced_images_first_then_the_least_recently_used() {
    let grow = |g: &mut Graphics, id: u32, len: usize| {
        let img = g.images.iter_mut().find(|img| img.id == id).unwrap();
        Rc::make_mut(&mut img.pixels).resize(len, 0);
    };
    let mut g = Graphics::default();
    run(&mut g, (0, 0), &format!("a=t,i=1,{PIXEL}"));
    grow(&mut g, 1, MAX_BYTES / 2);
    for id in [2, 3] {
        run(&mut g, (0, 0), &format!("a=t,i={id},{PIXEL}"));
        grow(&mut g, id, MAX_BYTES / 4);
        run(&mut g, (0, 0), &format!("a=p,i={id},C=1"));
    }
    // Over by four bytes: every unplaced image but the new one goes.
    run(&mut g, (0, 0), &format!("a=t,i=4,{PIXEL}"));
    assert_eq!(ids(&g), [2, 3, 4]);
    grow(&mut g, 4, MAX_BYTES / 2);
    run(&mut g, (0, 0), "a=p,i=4,C=1");
    // Putting an image again makes it recently used.
    run(&mut g, (0, 0), "a=p,i=2,C=1");
    run(&mut g, (0, 0), &format!("a=t,i=5,{PIXEL}"));
    assert_eq!(ids(&g), [2, 4, 5]);
    assert_eq!(placed(&g), [(2, 0), (2, 0), (4, 0)]);

    let mut g = Graphics::default();
    for id in 1..=MAX_IMAGES {
        run(&mut g, (0, 0), &format!("a=t,i={id},{PIXEL}"));
    }
    run(&mut g, (0, 0), "a=p,i=1,C=1");
    assert_eq!(g.images.len(), MAX_IMAGES);
    run(&mut g, (0, 0), &format!("a=t,i={},{PIXEL}", MAX_IMAGES + 1));
    // As kitty does, all the unplaced images go at once, not just enough.
    assert_eq!(ids(&g), [1, MAX_IMAGES as u32 + 1]);
}

#[test]
fn delete_cells_follow_scrolling_and_scrolled_off_anonymous_images_go() {
    let mut g = Graphics::default();
    run(&mut g, (0, 2), &format!("a=T,i=1,c=1,r=2,C=1,{PIXEL}"));
    g.scroll(0, 9, -1, 20);
    assert_eq!((g.placements[0].row, g.placements[0].rows), (1, 2));
    // Clipped at a margin, it covers only the rows it still shows.
    g.scroll(1, 9, -1, 20);
    assert_eq!((g.placements[0].row, g.placements[0].rows), (1, 1));
    run(&mut g, (0, 0), "a=d,d=y,y=3");
    assert_eq!(g.placements.len(), 1);
    run(&mut g, (0, 0), "a=d,d=y,y=2");
    assert!(g.placements.is_empty());
    assert_eq!(ids(&g), [1]);

    let mut g = Graphics::default();
    run(&mut g, (0, 0), &format!("a=T,C=1,{PIXEL}"));
    g.scroll(0, 9, -1, 20);
    assert!(g.placements.is_empty());
    assert!(g.images.is_empty());
}
