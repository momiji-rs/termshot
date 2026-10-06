//! Compressed payloads (o=z): chunking, interrupted uploads, the payload cap
//! and the checks made before inflating. `tests.rs` covers the exact sizes,
//! S and the 16 MiB limit. Streams come from two encoders that share no code
//! with the inflater: stored blocks built here, with an Adler-32 computed
//! here, and the renderer's own DEFLATE compressor (src/deflate.rs).
use super::*;
use crate::{screen::Lf, vt::replay_sized};

extern "C" {
    fn termshot_zlib_compress(data: *mut u8, len: i32, out_len: *mut i32, quality: i32) -> *mut u8;
    fn free(p: *mut std::ffi::c_void);
}

fn replay(s: &[u8]) -> crate::grid::Grid {
    replay_sized(s, 20, 10, Lf::Index, (10, 20))
}

fn adler32(data: &[u8]) -> [u8; 4] {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16 | a).to_be_bytes()
}

/// A zlib stream of stored blocks, `empty` empty ones first.
fn stored(data: &[u8], empty: usize) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    for _ in 0..empty {
        out.extend_from_slice(&[0, 0, 0, 0xff, 0xff]);
    }
    let mut blocks = data.chunks(65535).peekable();
    while let Some(block) = blocks.next() {
        let len = block.len() as u16;
        out.push(u8::from(blocks.peek().is_none()));
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data));
    out
}

/// The renderer's compressor, which uses Huffman codes and back-references.
fn deflate(data: &[u8]) -> Vec<u8> {
    let mut input = data.to_vec();
    let mut n = 0;
    unsafe {
        let p = termshot_zlib_compress(input.as_mut_ptr(), input.len() as i32, &mut n, 8);
        assert!(!p.is_null());
        let out = std::slice::from_raw_parts(p, n as usize).to_vec();
        free(p.cast());
        out
    }
}

fn encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for q in data.chunks(3) {
        let v = u32::from(q[0]) << 16 | u32::from(*q.get(1).unwrap_or(&0)) << 8 | u32::from(*q.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= q.len() { T[(v >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// One a=T,o=z transmission of `stream` with the given keys, then "OK".
fn send(keys: &str, stream: &[u8]) -> crate::grid::Grid {
    replay(format!("\x1b_Ga=T,o=z,C=1,{keys};{}\x1b\\OK", encode(stream)).as_bytes())
}

/// The pixels the single placement shows, or None, after checking that the
/// payload never reaches the text.
fn shown(g: &crate::grid::Grid) -> Option<Vec<u8>> {
    assert_eq!((g.cells[0].ch, g.cells[1].ch), (b'O' as u32, b'K' as u32));
    assert!(g.images.len() <= 1);
    g.images.first().map(|p| p.pixels.to_vec())
}

/// Deterministic pixels with runs, so the compressor finds matches.
fn pixels(n: usize) -> Vec<u8> {
    (0..n).map(|i| ((i / 7) as u8).wrapping_mul(37) ^ (i % 5) as u8).collect()
}

fn rgba_of_rgb(rgb: &[u8]) -> Vec<u8> {
    rgb.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect()
}

#[test]
fn the_chunked_fixture_is_the_renderers_compression_of_kitty_rgb() {
    let log = include_bytes!("../../tests/fixtures/kitty-rgb-z-chunks.pty");
    let text = std::str::from_utf8(log).unwrap();
    let parts: Vec<&str> = text.split('\x1b').filter_map(|s| s.split_once(';')).map(|(_, b)| b).collect();
    assert_eq!(parts.len(), 3);
    let stream = base64(parts.concat().as_bytes(), MAX_BYTES).unwrap();
    let rgb = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0];
    assert_eq!(stream, deflate(&rgb));
    // The first cut falls inside the DEFLATE data, not at a block boundary.
    assert!(base64(parts[0].as_bytes(), MAX_BYTES).unwrap().len() < stream.len() - 4);
    assert_eq!(*replay(log).images[0].pixels, rgba_of_rgb(&rgb));
}

#[test]
fn a_stream_split_at_every_chunk_size_decodes_once_at_the_final_chunk() {
    let rgb = pixels(31 * 7 * 3);
    for stream in [stored(&rgb, 3), deflate(&rgb)] {
        let encoded = encode(&stream);
        // Every chunk size, so the cuts fall inside the header, the blocks,
        // their codes and the trailer.
        for size in (4..=encoded.len()).step_by(4) {
            let parts: Vec<&str> = encoded.as_bytes().chunks(size).map(|c| std::str::from_utf8(c).unwrap()).collect();
            let mut log = Vec::new();
            for (i, part) in parts.iter().enumerate() {
                let more = u8::from(i + 1 < parts.len());
                let keys = if i == 0 { "a=T,o=z,f=24,s=31,v=7," } else { "" };
                if i + 1 == parts.len() {
                    log.extend_from_slice(b"\x1b[3;4H");
                }
                log.extend_from_slice(format!("\x1b_G{keys}m={more};{part}\x1b\\").as_bytes());
            }
            let g = replay(&log);
            assert_eq!(g.images.len(), 1, "chunks of {size}");
            assert_eq!(*g.images[0].pixels, rgba_of_rgb(&rgb), "chunks of {size}");
            assert_eq!((g.images[0].x, g.images[0].slices[0].y), (30, 40));
            // Cut short before the final chunk, nothing is shown.
            if parts.len() > 1 {
                let cut = log.len() - format!("\x1b_Gm=0;{}\x1b\\", parts.last().unwrap()).len();
                assert!(replay(&log[..cut]).images.is_empty());
            }
        }
    }
}

#[test]
fn an_interrupted_compressed_upload_is_dropped() {
    let rgb = pixels(4 * 4 * 3);
    let encoded = encode(&deflate(&rgb));
    let (head, tail) = encoded.split_at(8);
    let first = format!("\x1b_Ga=T,o=z,f=24,s=4,v=4,m=1;{head}\x1b\\");
    let red = "\x1b_Ga=T,f=24,s=1,v=1,C=1;/wAA\x1b\\";
    // A new command mid-upload replaces it; the rest of the stream, arriving
    // after, starts nothing, as a chunk without its first command is RGBA.
    for between in [red, "\x1b_Ga=d\x1b\\", "\x1b_Ga=p,i=7\x1b\\", "\x1b_Ga=T,f=24,s=1,v=1;!!!!\x1b\\"] {
        let log = format!("{first}{between}\x1b_Gm=0;{tail}\x1b\\");
        let g = replay(log.as_bytes());
        let reds = if between == red { 1 } else { 0 };
        assert_eq!(g.images.len(), reds, "{between:?}");
        assert!(g.images.iter().all(|p| *p.pixels == [255, 0, 0, 255]));
    }
    // Malformed base64 in a later chunk drops the upload too.
    let log = format!("{first}\x1b_Gm=0;{}*\x1b\\", &tail[..tail.len() - 1]);
    assert!(replay(log.as_bytes()).images.is_empty());
    // A later complete upload is unaffected by an abandoned one.
    let log = format!("{first}\x1b_Ga=T,o=z,f=24,s=4,v=4;{encoded}\x1b\\");
    assert_eq!(*replay(log.as_bytes()).images[0].pixels, rgba_of_rgb(&rgb));
}

#[test]
fn compressed_payloads_are_capped_at_their_decoded_size_plus_1_kib() {
    // Empty stored blocks pad a stream without changing what it inflates to.
    // An image of 3 bytes may have 3 + 1,024 bytes of payload: 14 bytes of
    // framing and data, plus 5 for each empty block.
    let rgb = [1, 2, 3];
    for (empty, ok) in [(202, true), (203, false)] {
        let stream = stored(&rgb, empty);
        assert_eq!(stream.len(), if ok { 1024 } else { 1029 });
        assert_eq!(shown(&send("f=24,s=1,v=1", &stream)).is_some(), ok, "{empty} empty blocks");
        // The same over chunks: the cap applies to the whole upload, and to
        // continuation chunks by the first command's size.
        let encoded = encode(&stream);
        let (head, tail) = encoded.split_at(encoded.len() / 8 * 4);
        let log = format!("\x1b_Ga=T,o=z,f=24,s=1,v=1,m=1;{head}\x1b\\\x1b_Gm=0;{tail}\x1b\\");
        assert_eq!(replay(log.as_bytes()).images.len(), usize::from(ok));
    }
    // The exact edge: three bytes of trailing padding after the stream reach
    // 1,027, which the inflater ignores; one more byte is too many.
    let mut edge = stored(&rgb, 202);
    edge.extend_from_slice(&[0; 3]);
    assert_eq!(edge.len(), 3 + COMPRESSION_SLACK);
    assert!(shown(&send("f=24,s=1,v=1", &edge)).is_some());
    edge.push(0);
    assert_eq!(shown(&send("f=24,s=1,v=1", &edge)), None);
    let cap = |keys: &str| payload_limit(&Command::parse(keys.as_bytes()).unwrap());
    assert_eq!(cap("o=z,f=24,s=1,v=1"), 3 + COMPRESSION_SLACK);
    assert_eq!(cap("o=z,s=2,v=3"), 24 + COMPRESSION_SLACK);
    assert_eq!(cap("o=z,f=32,s=8192,v=8192"), MAX_BYTES + COMPRESSION_SLACK);
    assert_eq!(cap("o=z,f=32,s=4294967295,v=4294967295"), MAX_BYTES);
    // A compressed PNG keeps the payload limit; uncompressed RGB gets 10 bytes.
    assert_eq!(cap("o=z,f=100,S=80"), MAX_BYTES);
    assert_eq!(cap("f=24,s=1,v=1"), 3 + RAW_SLACK);
}

#[test]
fn bytes_after_the_trailer_are_ignored_as_zlib_ignores_them() {
    let rgb = pixels(5 * 3 * 3);
    for stream in [stored(&rgb, 1), deflate(&rgb)] {
        let trailer = stream[stream.len() - 4..].to_vec();
        for extra in [&[0u8][..], &[0xff; 4], &trailer, &[0x03, 0x00]] {
            let mut long = stream.clone();
            long.extend_from_slice(extra);
            assert_eq!(shown(&send("f=24,s=5,v=3", &long)), Some(rgba_of_rgb(&rgb)), "trailing {extra:?}");
        }
    }
}

#[test]
fn dimensions_are_checked_before_inflating() {
    let size = |keys: &str| inflated_size(&Command::parse(keys.as_bytes()).unwrap());
    assert_eq!(size("o=z,f=24,s=2,v=3"), Some(18));
    assert_eq!(size("o=z,f=32,s=2048,v=2048"), Some(MAX_BYTES));
    // RGB that fits in 16 MiB but whose RGBA would not, or an axis over
    // 8,192 pixels: decode refuses both, so neither is inflated.
    for keys in ["o=z,f=24,s=2049,v=2048", "o=z,f=32,s=2049,v=2048", "o=z,f=24,s=8193,v=1", "o=z,f=32,s=1,v=8193"] {
        assert_eq!(size(keys), None, "{keys}");
    }
    for keys in ["o=z,f=24,s=0,v=1", "o=z,f=24,v=1", "o=z,f=99,s=1,v=1", "o=z,f=100,S=16777217"] {
        assert_eq!(size(keys), None, "{keys}");
    }
    assert_eq!(size("o=z,f=100"), Some(100 * 1024));
    assert_eq!(size("o=z,f=100,S=16777216"), Some(MAX_BYTES));
    // End to end, a valid stream of 2049x2048 RGB is refused.
    let rgb = vec![0; 2049 * 2048 * 3];
    assert_eq!(shown(&send("f=24,s=2049,v=2048", &deflate(&rgb))), None);
}

#[test]
fn a_stream_that_would_inflate_further_stops_at_the_bound() {
    // 100,000 zeros in under 1 KiB, sent for images smaller, near and exact.
    let small = deflate(&[0; 100_000]);
    assert!(small.len() <= 3 + COMPRESSION_SLACK);
    assert_eq!(shown(&send("f=24,s=1,v=1", &small)), None);
    assert_eq!(shown(&send("f=24,s=100,v=333", &small)), None);
    assert_eq!(shown(&send("f=32,s=250,v=100", &small)), Some(vec![0; 100_000]));
    // 16 MiB of zeros in a stream about 1% of that.
    let bomb = deflate(&vec![0; MAX_BYTES]);
    assert!(bomb.len() < MAX_BYTES / 64, "{}", bomb.len());
    for keys in ["f=32,s=2048,v=2047", "f=24,s=2048,v=2048", "f=100,S=16777215"] {
        assert_eq!(shown(&send(keys, &bomb)), None, "{keys}");
    }
    assert_eq!(shown(&send("f=32,s=2048,v=2048", &bomb)).map(|p| p.len()), Some(MAX_BYTES));
}

#[test]
fn stored_compressed_images_place_and_replace_like_any_other() {
    let green = stored(&[0, 255, 0], 0);
    let mut log = format!("\x1b_Ga=t,i=3,o=z,f=24,s=1,v=1;{}\x1b\\", encode(&green)).into_bytes();
    log.extend_from_slice(b"\x1b_Ga=p,i=3,C=1\x1b\\\x1b[2;1H\x1b_Ga=p,i=3,C=1\x1b\\");
    let g = replay(&log);
    assert_eq!(g.images.len(), 2);
    assert_eq!(*g.images[0].pixels, [0, 255, 0, 255]);
    // A compressed retransmission that fails still removes the old image.
    log.extend_from_slice(format!("\x1b_Ga=t,i=3,o=z,f=24,s=1,v=1;{}\x1b\\", encode(&green[..green.len() - 1])).as_bytes());
    assert!(replay(&log).images.is_empty());
    // The preflight counts compressed transmissions as any other.
    assert!(needs_cell_metrics(b"\x1b_Ga=T,o=z,f=24,s=1,v=1;AAAA\x1b\\"));
    assert!(!needs_cell_metrics(b"\x1b_Ga=T,o=x,f=24,s=1,v=1;AAAA\x1b\\"));
}
