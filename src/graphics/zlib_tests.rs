//! Compressed payloads (o=z). Streams come from two encoders that share no
//! code with the inflater: stored blocks built here, with an Adler-32 computed
//! here, and the renderer's own DEFLATE compressor (src/deflate.c).
use super::*;
use crate::{replay_sized, Lf};

extern "C" {
    fn termshot_zlib_compress(data: *mut u8, len: i32, out_len: *mut i32, quality: i32) -> *mut u8;
    fn free(p: *mut std::ffi::c_void);
}

fn replay(s: &[u8]) -> crate::Grid {
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
    if data.is_empty() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
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
fn send(keys: &str, stream: &[u8]) -> crate::Grid {
    replay(format!("\x1b_Ga=T,o=z,C=1,{keys};{}\x1b\\OK", encode(stream)).as_bytes())
}

/// The pixels the single placement shows, or None, after checking that the
/// payload never reaches the text.
fn shown(g: &crate::Grid) -> Option<Vec<u8>> {
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
fn compressed_rgb_and_rgba_decode_to_their_pixels() {
    for (w, h) in [(1, 1), (2, 2), (17, 9), (300, 200)] {
        let rgb = pixels(w * h * 3);
        let rgba = pixels(w * h * 4);
        for stream in [stored(&rgb, 0), deflate(&rgb)] {
            let keys = format!("f=24,s={w},v={h}");
            assert_eq!(shown(&send(&keys, &stream)), Some(rgba_of_rgb(&rgb)), "rgb {w}x{h}");
        }
        for stream in [stored(&rgba, 0), deflate(&rgba)] {
            let keys = format!("f=32,s={w},v={h}");
            assert_eq!(shown(&send(&keys, &stream)), Some(rgba.clone()), "rgba {w}x{h}");
        }
    }
    // More than one stored block, and Huffman coding that shrinks the data.
    let big = pixels(200 * 120 * 4);
    assert_eq!(shown(&send("f=32,s=200,v=120", &stored(&big, 0))), Some(big.clone()));
    assert!(deflate(&big).len() < big.len() / 2);
}

#[test]
fn the_fixtures_are_the_renderers_own_compression_of_their_pixels() {
    let fixture = |log: &[u8]| {
        let text = std::str::from_utf8(log).unwrap();
        let chunks: String = text.split('\x1b').filter_map(|s| s.split_once(';')).map(|(_, b)| b).collect();
        base64(chunks.as_bytes(), MAX_BYTES).unwrap()
    };
    let rgb = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0];
    let rgba = [255, 0, 0, 128, 0, 255, 0, 128, 0, 0, 255, 128, 255, 255, 0, 128];
    let png = include_bytes!("../../tests/fixtures/kitty-png-alpha.pty");
    let png = fixture(&png[..png.len() - 2]);
    assert_eq!(fixture(include_bytes!("../../tests/fixtures/kitty-zlib-rgb.pty")), deflate(&rgb));
    assert_eq!(fixture(include_bytes!("../../tests/fixtures/kitty-zlib-rgba.pty")), deflate(&rgba));
    assert_eq!(fixture(include_bytes!("../../tests/fixtures/kitty-zlib-png.pty")), deflate(&png));
    let g = replay(include_bytes!("../../tests/fixtures/kitty-zlib-rgb.pty"));
    assert_eq!(*g.images[0].pixels, rgba_of_rgb(&rgb));
    let g = replay(include_bytes!("../../tests/fixtures/kitty-zlib-rgba.pty"));
    assert_eq!(*g.images[0].pixels, rgba);
    let g = replay(include_bytes!("../../tests/fixtures/kitty-zlib-png.pty"));
    assert_eq!(*g.images[0].pixels, rgba);
}

#[test]
fn a_stream_split_across_chunks_decodes_once_at_the_final_chunk() {
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
    // A later complete upload is unaffected by an abandoned one.
    let log = format!("{first}\x1b_Ga=T,o=z,f=24,s=4,v=4;{encoded}\x1b\\");
    assert_eq!(*replay(log.as_bytes()).images[0].pixels, rgba_of_rgb(&rgb));
}

#[test]
fn wrong_sizes_trailers_and_trailing_bytes_are_refused() {
    let rgb = pixels(5 * 3 * 3);
    let keys = "f=24,s=5,v=3";
    for stream in [stored(&rgb, 1), deflate(&rgb)] {
        assert!(shown(&send(keys, &stream)).is_some());
        // Every truncation, and any byte after the trailer.
        for n in 0..stream.len() {
            assert_eq!(shown(&send(keys, &stream[..n])), None, "cut at {n}");
        }
        for extra in [&[0u8][..], &[0, 0, 0, 0], &stream[stream.len() - 4..], &[0x03, 0x00]] {
            let mut long = stream.clone();
            long.extend_from_slice(extra);
            assert_eq!(shown(&send(keys, &long)), None, "trailing {extra:?}");
        }
        // Each bit of the Adler-32.
        for bit in 0..32 {
            let mut bad = stream.clone();
            let n = bad.len();
            bad[n - 4 + bit / 8] ^= 1 << (bit % 8);
            assert_eq!(shown(&send(keys, &bad)), None, "adler bit {bit}");
        }
        // The dimensions must match the inflated size exactly.
        for other in ["f=24,s=5,v=2", "f=24,s=4,v=3", "f=24,s=6,v=3", "f=32,s=5,v=3", "f=24,s=15,v=1"] {
            let expect_ok = other == "f=24,s=15,v=1";
            assert_eq!(shown(&send(other, &stream)).is_some(), expect_ok, "{other}");
        }
    }
    // One byte short of or past the stated size, with a correct trailer.
    assert_eq!(shown(&send(keys, &stored(&rgb[1..], 0))), None);
    let mut more = rgb.clone();
    more.push(0);
    assert_eq!(shown(&send(keys, &stored(&more, 0))), None);
    assert_eq!(shown(&send(keys, &deflate(&more))), None);
}

#[test]
fn malformed_headers_and_streams_are_refused() {
    let rgb = pixels(3 * 3);
    let good = stored(&rgb, 0);
    let keys = "f=24,s=3,v=1";
    assert!(shown(&send(keys, &good)).is_some());
    let header = |cmf: u8, flg: u8| {
        let mut s = good.clone();
        s[0] = cmf;
        s[1] = flg;
        s
    };
    // Check bits, a method other than DEFLATE, a window over 32 KiB, a preset
    // dictionary: each with valid check bits unless the check bits are the point.
    let fcheck = |cmf: u8, flg: u8| flg + (31 - (u16::from(cmf) * 256 + u16::from(flg)) % 31) as u8 % 31;
    for (cmf, flg) in [(0x78, 0x02), (0x77, fcheck(0x77, 0)), (0x88, fcheck(0x88, 0)), (0x78, fcheck(0x78, 0x20))] {
        assert_eq!(shown(&send(keys, &header(cmf, flg))), None, "header {cmf:#x} {flg:#x}");
    }
    // Every window size up to 32 KiB is accepted.
    for cinfo in 0..=7u8 {
        let cmf = cinfo << 4 | 8;
        assert!(shown(&send(keys, &header(cmf, fcheck(cmf, 0)))).is_some(), "cinfo {cinfo}");
    }
    // A reserved block type, a stored length that disagrees with its
    // complement, and one that runs past the data.
    let mut s = good.clone();
    s[2] = 0x07;
    assert_eq!(shown(&send(keys, &s)), None);
    let mut s = good.clone();
    s[5] ^= 1;
    assert_eq!(shown(&send(keys, &s)), None);
    let mut s = good.clone();
    s[3] = 0xff;
    s[5] = 0x00;
    assert_eq!(shown(&send(keys, &s)), None);
    // Deterministic mutations of a Huffman-coded stream never panic, leak
    // text or show pixels other than the image's.
    let rgb = pixels(16 * 16 * 3);
    let stream = deflate(&rgb);
    let mut seed = 0x2545_f491u32;
    let mut refused = 0;
    for _ in 0..3000 {
        let mut bad = stream.clone();
        for _ in 0..1 + seed % 3 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let n = bad.len();
            bad[seed as usize % n] ^= 1 << (seed >> 24) % 8;
        }
        match shown(&send("f=24,s=16,v=16", &bad)) {
            None => refused += 1,
            Some(p) => assert_eq!(p, rgba_of_rgb(&rgb)),
        }
    }
    assert!(refused > 2900, "{refused}");
    // Malformed base64 in a compressed upload, alone or in a later chunk.
    let encoded = encode(&stream);
    assert!(replay(format!("\x1b_Ga=T,o=z,f=24,s=16,v=16;{}!\x1b\\", &encoded[..encoded.len() - 1]).as_bytes()).images.is_empty());
    let log = format!("\x1b_Ga=T,o=z,f=24,s=16,v=16,m=1;{}\x1b\\\x1b_Gm=0;{}*\x1b\\", &encoded[..8], &encoded[8..encoded.len() - 1]);
    assert!(replay(log.as_bytes()).images.is_empty());
}

#[test]
fn compressed_payloads_are_bounded_by_their_decoded_size() {
    // Empty stored blocks pad a stream without changing what it inflates to.
    // An image of 3 bytes may have 3 + 1,024 bytes of payload: 14 bytes of
    // framing and data, plus 5 for each empty block.
    let rgb = [1, 2, 3];
    for (empty, ok) in [(202, true), (203, false)] {
        let stream = stored(&rgb, empty);
        assert_eq!(stream.len() <= 3 + COMPRESSION_SLACK, ok);
        assert_eq!(shown(&send("f=24,s=1,v=1", &stream)).is_some(), ok, "{empty} empty blocks");
        // The same over chunks: the bound applies to the whole upload.
        let encoded = encode(&stream);
        let (head, tail) = encoded.split_at(encoded.len() / 8 * 4);
        let log = format!("\x1b_Ga=T,o=z,f=24,s=1,v=1,m=1;{head}\x1b\\\x1b_Gm=0;{tail}\x1b\\");
        assert_eq!(replay(log.as_bytes()).images.len(), usize::from(ok));
    }
    assert_eq!(payload_limit(&Command::parse(b"o=z,f=24,s=1,v=1").unwrap()), 3 + COMPRESSION_SLACK);
    assert_eq!(payload_limit(&Command::parse(b"o=z,f=32,s=8192,v=8192").unwrap()), MAX_BYTES + COMPRESSION_SLACK);
    assert_eq!(payload_limit(&Command::parse(b"o=z,f=100").unwrap()), MAX_BYTES);
    assert_eq!(payload_limit(&Command::parse(b"f=24,s=1,v=1").unwrap()), MAX_BYTES);
    // A small stream that would inflate far past its image stops at the
    // bound: 100,000 zeros in under 1 KiB for a one-pixel image, and 16 MiB
    // of zeros for images a little smaller than that.
    let small = deflate(&[0; 100_000]);
    assert!(small.len() <= 3 + COMPRESSION_SLACK);
    assert_eq!(shown(&send("f=24,s=1,v=1", &small)), None);
    assert_eq!(shown(&send("f=24,s=100,v=333", &small)), None);
    assert!(shown(&send("f=32,s=250,v=100", &small)).is_some());
    let bomb = deflate(&vec![0; MAX_BYTES]);
    assert!(bomb.len() < 256 * 1024, "{}", bomb.len());
    for keys in ["f=24,s=1,v=1", "f=32,s=2048,v=2047", "f=100,S=16777215"] {
        assert_eq!(shown(&send(keys, &bomb)), None, "{keys}");
    }
    // Exactly 16 MiB is the decoded limit, and is kept; past it, refused before inflating.
    assert_eq!(shown(&send("f=32,s=2048,v=2048", &bomb)).map(|p| p.len()), Some(MAX_BYTES));
    assert_eq!(shown(&send("f=32,s=2049,v=2048", &bomb)), None);
    assert_eq!(shown(&send("f=24,s=2049,v=2048", &bomb)), None);
    assert_eq!(shown(&send("f=24,s=8193,v=1", &stored(&[0; 8193 * 3], 0))), None);
    assert_eq!(shown(&send("f=100,S=16777217", &bomb)), None);
    assert!(inflate(&bomb, MAX_BYTES + 1).is_none());
    assert!(inflate(&vec![0; MAX_BYTES + COMPRESSION_SLACK + 1], 3).is_none());
}

#[test]
fn compressed_png_inflates_to_its_stated_size() {
    let log = include_bytes!("../../tests/fixtures/kitty-png-alpha.pty");
    let text = std::str::from_utf8(&log[..log.len() - 2]).unwrap();
    let png = base64(text.split_once(';').unwrap().1.as_bytes(), MAX_BYTES).unwrap();
    let want = [255, 0, 0, 128, 0, 255, 0, 128, 0, 0, 255, 128, 255, 255, 0, 128];
    let n = png.len();
    for stream in [stored(&png, 0), deflate(&png)] {
        assert_eq!(shown(&send(&format!("f=100,S={n}"), &stream)).as_deref(), Some(&want[..]));
        // S is the size of the PNG, not of its pixels; kitty needs it exact.
        for wrong in [n - 1, n + 1, 16] {
            assert_eq!(shown(&send(&format!("f=100,S={wrong}"), &stream)), None, "S={wrong}");
        }
        // Without S, kitty expects 100 KiB, which this PNG is not.
        assert_eq!(shown(&send("f=100", &stream)), None);
    }
    // A PNG of exactly 100 KiB, with bytes after its end, needs no S.
    let mut padded = png.clone();
    padded.resize(DEFAULT_PNG_SIZE, 0);
    assert_eq!(shown(&send("f=100", &deflate(&padded))).as_deref(), Some(&want[..]));
    assert_eq!(shown(&send("f=100,S=0", &deflate(&padded))).as_deref(), Some(&want[..]));
    // A zlib stream of something that is not a PNG.
    assert_eq!(shown(&send("f=100,S=8", &stored(b"not png!", 0))), None);
    // S is not needed elsewhere: ignored on raw pixels and uncompressed PNG.
    let rgb = pixels(3);
    assert!(shown(&send("f=24,s=1,v=1,S=99", &stored(&rgb, 0))).is_some());
    let png_log = format!("\x1b_Ga=T,f=100,S=1,C=1;{}\x1b\\OK", encode(&png));
    assert_eq!(shown(&replay(png_log.as_bytes())).as_deref(), Some(&want[..]));
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
