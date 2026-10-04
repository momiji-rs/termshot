//! deflate.rs tests: every Adler-32 form against the scalar definition, round
//! trips through an independent inflate, the FFI entry point, the profile
//! timings, and a failure at each allocation in turn. Byte-for-byte equality
//! with stock stb is tests/deflate_diff.c's (test.sh links it with this
//! module as a static library).

use super::faults::{Faults, FAILED, FAULTS};
use super::*;

struct Rng(u64);

impl Rng {
    /// tests/deflate_diff.c's xorshift.
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }

    fn bytes(&mut self, len: usize, bits: u32) -> Vec<u8> {
        (0..len).map(|_| (self.next() & ((1 << bits) - 1)) as u8).collect()
    }
}

/// The definition: both sums modulo 65521 after every byte.
fn adler32_scalar(data: &[u8]) -> u32 {
    let (mut s1, mut s2) = (1u32, 0u32);
    for &b in data {
        s1 = (s1 + u32::from(b)) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    (s2 << 16) | s1
}

/// Every form compiled for this target.
fn adler_forms() -> Vec<(&'static str, fn(&[u8]) -> u32)> {
    #[allow(unused_mut)]
    let mut forms: Vec<(&'static str, fn(&[u8]) -> u32)> = vec![("lanes", adler32_lanes), ("chosen", adler32)];
    #[cfg(target_arch = "x86_64")]
    forms.push(("sse2", adler32_sse2));
    forms
}

/// The checksum of every prefix of data, by the definition.
fn prefix_sums(data: &[u8]) -> Vec<u32> {
    let (mut s1, mut s2) = (1u32, 0u32);
    let mut all = vec![1];
    for &b in data {
        s1 = (s1 + u32::from(b)) % 65521;
        s2 = (s2 + s1) % 65521;
        all.push((s2 << 16) | s1);
    }
    all
}

#[test]
fn adler32_forms_agree_with_the_definition() {
    const MAX: usize = 70_000;
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    // Random bytes at every alignment of a 16-byte load, and all 0xff, the
    // worst case for the sums. Each has the definition at every prefix.
    let mut buffers: Vec<(String, Vec<u8>)> = (0..16).map(|offset| (format!("random, offset {offset}"), rng.bytes(MAX + offset, 8))).collect();
    buffers.push(("all 0xff".into(), vec![0xff; MAX + 3]));
    for (b, (name, buffer)) in buffers.iter().enumerate() {
        let offset = buffer.len() - MAX;
        let data = &buffer[offset..];
        let want = prefix_sums(data);
        // Every length through a few 128-byte steps, random lengths up to
        // MAX, and MAX. Unit tests are unoptimized, so only three buffers
        // also take every length within a chunk of each 5552-byte block's end.
        let mut lengths: Vec<usize> = (0..=600).collect();
        lengths.extend((0..40).map(|_| rng.next() as usize % (MAX + 1)));
        lengths.push(MAX);
        if [0, 7, buffers.len() - 1].contains(&b) {
            for block in 1..=MAX / ADLER_BLOCK {
                lengths.extend(block * ADLER_BLOCK - 17..=block * ADLER_BLOCK + 17);
            }
        }
        for len in lengths {
            for (form, f) in adler_forms() {
                assert_eq!(f(&data[..len]), want[len], "{form} Adler-32 of {len} bytes ({name})");
            }
        }
    }
    assert_eq!(adler32_scalar(b"Wikipedia"), 0x11e6_0398);
}

/// Inflates a zlib stream of stored and fixed-Huffman blocks, the only kinds
/// compress writes, and checks its Adler-32 by the definition.
fn inflate(z: &[u8]) -> Vec<u8> {
    const LBASE: [u32; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
    const LEXTRA: [u32; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
    const DBASE: [u32; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
    const DEXTRA: [u32; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
    struct Bits<'a> {
        d: &'a [u8],
        pos: usize,
    }
    impl Bits<'_> {
        fn bit(&mut self) -> u32 {
            let b = (self.d[self.pos >> 3] >> (self.pos & 7)) & 1;
            self.pos += 1;
            u32::from(b)
        }
        fn bits(&mut self, n: u32) -> u32 {
            (0..n).fold(0, |v, k| v | self.bit() << k)
        }
        /// n more bits of a Huffman code, sent most significant bit first.
        fn code(&mut self, n: u32, mut v: u32) -> u32 {
            for _ in 0..n {
                v = v << 1 | self.bit();
            }
            v
        }
    }
    assert!(z.len() >= 6 && z[0] == 0x78 && z[1] == 0x5e, "zlib header");
    let mut b = Bits { d: &z[2..], pos: 0 };
    let mut out = Vec::new();
    loop {
        let last = b.bit();
        match b.bits(2) {
            0 => {
                b.pos = (b.pos + 7) & !7;
                let len = b.bits(16) as usize;
                assert_eq!(b.bits(16) as usize, !len & 0xffff, "stored block length");
                let at = b.pos >> 3;
                out.extend_from_slice(&b.d[at..at + len]);
                b.pos += 8 * len;
            }
            1 => loop {
                let mut c = b.code(7, 0);
                let sym = if c <= 0x17 {
                    256 + c
                } else {
                    c = b.code(1, c);
                    if (0x30..=0xbf).contains(&c) {
                        c - 0x30
                    } else if (0xc0..=0xc7).contains(&c) {
                        280 + c - 0xc0
                    } else {
                        144 + b.code(1, c) - 0x190
                    }
                };
                if sym < 256 {
                    out.push(sym as u8);
                } else if sym == 256 {
                    break;
                } else {
                    let l = (sym - 257) as usize;
                    let len = (LBASE[l] + b.bits(LEXTRA[l])) as usize;
                    let d = b.code(5, 0) as usize;
                    let dist = (DBASE[d] + b.bits(DEXTRA[d])) as usize;
                    assert!(dist <= out.len() && dist <= WINDOW, "distance {dist} at {}", out.len());
                    let from = out.len() - dist;
                    for k in 0..len {
                        out.push(out[from + k]);
                    }
                }
            },
            t => panic!("block type {t}"),
        }
        if last == 1 {
            break;
        }
    }
    let at = (b.pos + 7) / 8 + 2;
    assert_eq!(at + 4, z.len(), "trailing bytes");
    assert_eq!(u32::from_be_bytes(z[at..at + 4].try_into().unwrap()), adler32_scalar(&out), "Adler-32 trailer");
    out
}

/// The stream compress writes, through the C entry point.
fn compressed(data: &[u8], quality: c_int) -> Vec<u8> {
    let mut copy = data.to_vec();
    let mut n = -1;
    let p = unsafe { termshot_zlib_compress(copy.as_mut_ptr(), data.len() as c_int, &mut n, quality) };
    assert!(!p.is_null() && n > 0, "compression of {} bytes failed", data.len());
    let out = unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec();
    unsafe { free(p as *mut c_void) };
    out
}

/// Inputs with every shape the compressor treats differently.
fn shapes() -> Vec<(String, Vec<u8>)> {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut v: Vec<(String, Vec<u8>)> = (0..12).map(|n| (format!("{n} random bytes"), rng.bytes(n, 8))).collect();
    v.push(("zeros past the window".into(), vec![0; 3 * WINDOW + 17]));
    v.push(("random bytes: stored blocks".into(), rng.bytes(100_000, 8)));
    v.push(("4 symbols".into(), rng.bytes(70_000, 2)));
    let row: Vec<u8> = rng.bytes(301, 8);
    v.push(("image-like rows".into(), row.iter().cycle().take(90_000).enumerate().map(|(i, &b)| if i % 97 == 0 { b ^ 1 } else { b }).collect()));
    v
}

#[test]
fn round_trips() {
    for (name, data) in shapes() {
        for quality in [0, 5, 8, 16] {
            assert!(inflate(&compressed(&data, quality)) == data, "{name}, quality {quality}");
        }
    }
    // Random bytes don't compress: stored blocks, as stb writes them.
    let random = Rng(7).bytes(100_000, 8);
    let z = compressed(&random, 8);
    assert_eq!(z.len(), 2 + 4 * 5 + random.len() + 4, "four stored blocks");
    assert_eq!(z[2] & 7, 0, "a stored block");
}

#[test]
fn negative_lengths_fail() {
    let mut n = 7;
    let p = unsafe { termshot_zlib_compress(ptr::null_mut(), -1, &mut n, 8) };
    assert!(p.is_null() && n == 7);
}

#[test]
fn empty_input_is_a_valid_stream() {
    let mut n = 0;
    let p = unsafe { termshot_zlib_compress(ptr::null_mut(), 0, &mut n, 8) };
    assert!(!p.is_null());
    let z = unsafe { std::slice::from_raw_parts(p, n as usize) }.to_vec();
    unsafe { free(p as *mut c_void) };
    // stb's own fallback drops the final block of an empty stream.
    assert_eq!(z, [0x78, 0x5e, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
    assert!(inflate(&z).is_empty());
}

#[test]
fn profile_timings() {
    let data = vec![1u8; 200_000];
    let timings = || {
        let mut t = DeflateTimings { allocate_ms: -1.0, ..Default::default() };
        unsafe { termshot_deflate_timings(&mut t) };
        [t.allocate_ms, t.match_emit_ms, t.finalize_ms, t.checksum_ms]
    };
    termshot_deflate_profiling(0);
    compressed(&data, 8);
    assert_eq!(timings(), [0.0; 4], "timed while off");
    termshot_deflate_profiling(1);
    compressed(&data, 8);
    let t = timings();
    assert!(t.iter().all(|&ms| ms >= 0.0) && t[1] > 0.0, "{t:?}");
    termshot_deflate_profiling(0);
    assert_eq!(timings(), [0.0; 4], "cleared");
}

fn set_faults(fail_at: u32) {
    FAULTS.with(|f| f.set(Faults { calls: 0, fail_at, live: 0 }));
    FAILED.with(|f| f.set(None));
}

/// Fail each allocation of each input in turn: the call returns NULL and
/// leaves nothing allocated, including an output buffer with pending bits.
/// Together the inputs fail every site.
#[test]
fn every_allocation_failure_returns_null() {
    let mut rng = Rng(42);
    let inputs = [
        ("empty", Vec::new()),
        ("1000 random bytes", rng.bytes(1000, 8)),
        ("200,000 random bytes", rng.bytes(200_000, 8)),
        ("300,000 bytes of 2 symbols", rng.bytes(300_000, 1)),
    ];
    let mut sites = Vec::new();
    for (name, data) in &inputs {
        set_faults(0);
        let out = compress(data, 8).expect("compresses with no failure");
        let allocations = FAULTS.with(|f| f.get().calls);
        drop(out);
        assert_eq!(FAULTS.with(|f| f.get().live), 0, "{name}: leak without failure");
        for fail_at in 1..=allocations {
            set_faults(fail_at);
            assert!(compress(data, 8).is_none(), "{name}: allocation {fail_at} of {allocations} failed, yet it returned a stream");
            let f = FAULTS.with(|f| f.get());
            assert_eq!(f.live, 0, "{name}: allocation {fail_at} failed and {} buffers leaked", f.live);
            sites.push(FAILED.with(|f| f.get()).expect("an allocation failed"));
            // Through the entry point too.
            set_faults(fail_at);
            let mut copy = data.clone();
            let mut n = -1;
            let p = unsafe { termshot_zlib_compress(copy.as_mut_ptr(), copy.len() as c_int, &mut n, 8) };
            assert!(p.is_null() && n == -1, "{name}: allocation {fail_at}: not NULL");
        }
    }
    set_faults(0);
    for site in [Site::Table, Site::Counts, Site::Output, Site::Grow] {
        assert!(sites.contains(&site), "no test failed the {site:?} allocation");
    }
    assert!(sites.len() >= 12, "{} failures", sites.len());
}
