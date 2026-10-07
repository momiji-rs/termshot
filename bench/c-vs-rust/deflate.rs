//! The current src/deflate.c (16-lane Adler-32, bit reversal by table, the
//! inlined matcher with the carried hash) ported to safe Rust, checked
//! byte-identical against the C, then timed against it in one process. Same
//! algorithms; only the language differs. Results and method:
//! docs/c-vs-rust.md. Run it with `bench/c-vs-rust/run.sh deflate`.
//!
//!   deflate <input dir> [rounds]   <name>.raw are deflate inputs
//!   deflate --inputs <termshot> <poc dir> <input dir>
//!                                  write them: render each workload with the
//!                                  CLI and inflate its PNG's IDAT (run.sh
//!                                  does this first; see make_inputs)
//!
//! Six variants run in every round, in a seeded random order per round, so
//! each one follows every other about equally often:
//! the C as built by the default compiler, the C with TERMSHOT_PORTABLE_ADLER,
//! the safe Rust, the Rust with two unchecked reads in the match loop
//! (`unchecked`, the 2026-10-01 POC's `--cfg unchecked`, here a const
//! generic so both Rust variants share the process and the rounds), the
//! default C with its hash table zeroed as the safe Rust's is, and the
//! compressor termshot ships since #12 step 1 (src/deflate.rs, through its C
//! entry point, as stb calls it; on x86-64 with its SSE2 Adler-32).

use std::time::Instant;

/// What termshot ships.
#[path = "../../src/deflate.rs"]
#[allow(dead_code)]
mod shipped;

extern "C" {
    fn cdef_zlib_compress(data: *mut u8, len: i32, out_len: *mut i32, quality: i32) -> *mut u8;
    fn cport_zlib_compress(data: *mut u8, len: i32, out_len: *mut i32, quality: i32) -> *mut u8;
    fn czero_zlib_compress(data: *mut u8, len: i32, out_len: *mut i32, quality: i32) -> *mut u8;
    fn cdef_adler32(d: *const u8, len: usize) -> u32;
    fn cport_adler32(d: *const u8, len: usize) -> u32;
    fn free(p: *mut std::ffi::c_void);
}

const ZHASH: usize = 16384;
const WINDOW: usize = 32768;
const MAX_MATCH: usize = 258;
const ADLER_BLOCK: usize = 5552;
/// termshot's PNG quality (stb_image_write's default compression level).
const QUALITY: usize = 8;

struct Out {
    p: Vec<u8>,
    bitbuf: u64,
    bitcount: u32,
}

impl Out {
    /// As the C: at most 31 new bits plus 7 pending, so append the low four
    /// bytes of the accumulator and keep the complete ones. The capacity
    /// check is extend_from_slice's.
    #[inline(always)]
    fn add_bits(&mut self, code: u32, bits: u32) {
        self.bitbuf |= u64::from(code) << self.bitcount;
        self.bitcount += bits;
        let bytes = (self.bitcount >> 3) as usize;
        let n = self.p.len();
        self.p.extend_from_slice(&(self.bitbuf as u32).to_le_bytes());
        self.p.truncate(n + bytes);
        self.bitbuf >>= bytes * 8;
        self.bitcount &= 7;
    }

    #[inline(always)]
    fn huff(&mut self, n: u32) {
        if n <= 143 {
            self.add_bits(bitrev(0x30 + n, 8), 8);
        } else if n <= 255 {
            self.add_bits(bitrev(0x190 + n - 144, 9), 9);
        } else if n <= 279 {
            self.add_bits(bitrev(n - 256, 7), 7);
        } else {
            self.add_bits(bitrev(0xc0 + n - 280, 8), 8);
        }
    }
}

/// Every byte with its bits reversed: the C's macro-built table.
const REVERSED_BYTE: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (i as u8).reverse_bits();
        i += 1;
    }
    t
};

/// code (below 2^bits, bits <= 9) with its bits reversed, by the table.
#[inline(always)]
fn bitrev(code: u32, bits: u32) -> u32 {
    let r9 = (u32::from(REVERSED_BYTE[(code & 0xff) as usize]) << 1) | ((code >> 8) & 1);
    r9 >> (9 - bits)
}

#[inline(always)]
fn zhash(d: &[u8], i: usize) -> usize {
    let d: &[u8; 3] = d[i..i + 3].try_into().unwrap();
    let mut h = u32::from(d[0]) + (u32::from(d[1]) << 8) + (u32::from(d[2]) << 16);
    h ^= h << 3;
    h = h.wrapping_add(h >> 5);
    h ^= h << 4;
    h = h.wrapping_add(h >> 17);
    h ^= h << 25;
    h = h.wrapping_add(h >> 6);
    h as usize & (ZHASH - 1)
}

#[inline(always)]
fn word(s: &[u8]) -> u64 {
    u64::from_le_bytes(s[..8].try_into().unwrap())
}

/// Length of the common prefix of data[a..] and data[b..], at most limit
/// (a < b, b + limit <= data.len()): 16 bytes a step, then 8, then bytes.
#[inline(always)]
fn countm<const UNCHECKED: bool>(data: &[u8], a: usize, b: usize, limit: usize) -> usize {
    if UNCHECKED {
        return countm_unchecked(data, a, b, limit);
    }
    let (x, y) = (&data[a..a + limit], &data[b..b + limit]);
    let mut i = 0;
    for (cx, cy) in x.chunks_exact(16).zip(y.chunks_exact(16)) {
        let d0 = word(cx) ^ word(cy);
        let d1 = word(&cx[8..]) ^ word(&cy[8..]);
        if d0 | d1 != 0 {
            let (offset, diff) = if d0 != 0 { (0, d0) } else { (8, d1) };
            return i + offset + (diff.trailing_zeros() as usize >> 3);
        }
        i += 16;
    }
    if i + 8 <= limit {
        let d = word(&x[i..]) ^ word(&y[i..]);
        if d != 0 {
            return i + (d.trailing_zeros() as usize >> 3);
        }
        i += 8;
    }
    while i < limit && x[i] == y[i] {
        i += 1;
    }
    i
}

/// The same loop without bounds checks: one of the two unsafe reads.
#[inline(always)]
fn countm_unchecked(data: &[u8], a: usize, b: usize, limit: usize) -> usize {
    debug_assert!(a < b && b + limit <= data.len());
    let p = data.as_ptr();
    // SAFETY: callers pass a < b and b + limit <= data.len(), so every read
    // is in bounds.
    unsafe {
        let load = |at: usize| (p.add(at) as *const u64).read_unaligned().to_le();
        let mut i = 0;
        while i + 16 <= limit {
            let d0 = load(a + i) ^ load(b + i);
            let d1 = load(a + i + 8) ^ load(b + i + 8);
            if d0 | d1 != 0 {
                let (offset, diff) = if d0 != 0 { (0, d0) } else { (8, d1) };
                return i + offset + (diff.trailing_zeros() as usize >> 3);
            }
            i += 16;
        }
        if i + 8 <= limit {
            let d = load(a + i) ^ load(b + i);
            if d != 0 {
                return i + (d.trailing_zeros() as usize >> 3);
            }
            i += 8;
        }
        while i < limit && *p.add(a + i) == *p.add(b + i) {
            i += 1;
        }
        i
    }
}

/// The candidate-rejection byte test's read: the other unsafe read.
#[inline(always)]
fn byte<const UNCHECKED: bool>(data: &[u8], i: usize) -> u8 {
    if UNCHECKED {
        debug_assert!(i < data.len());
        // SAFETY: callers pass i < data.len() (pos + best with best < limit).
        unsafe { *data.get_unchecked(i) }
    } else {
        data[i]
    }
}

/// Adler-32 in 16 lanes, as the C: per lane k, the byte sum a[k] and the sum
/// of earlier byte sums p[k], weights applied once per 5552-byte block. The
/// lanes see the same adds in the same order as the C's; only the loop shape
/// differs. One chunk per step, as the C, stays scalar at -C opt-level=2 on
/// arm64 and at 3 on x86-64. Eight chunks per step is vectorized by LLVM's
/// loop vectorizer at opt-level 2 and 3 on both (adler_forms.rs measures the
/// forms; docs/c-vs-rust.md has the codegen). No unsafe, no std::simd, no
/// core::arch.
#[inline(never)]
fn adler32(data: &[u8]) -> u32 {
    const STEP: usize = 8;
    let (mut s1, mut s2) = (1u32, 0u32);
    for block in data.chunks(ADLER_BLOCK) {
        let chunks = block.chunks_exact(16);
        let n = chunks.len() as u32;
        if n > 0 {
            let (mut a, mut p) = ([0u32; 16], [0u32; 16]);
            let mut steps = block[..16 * n as usize].chunks_exact(16 * STEP);
            for g in &mut steps {
                for k in 0..16 {
                    for c in 0..STEP {
                        p[k] += a[k];
                        a[k] += u32::from(g[16 * c + k]);
                    }
                }
            }
            for x in steps.remainder().chunks_exact(16) {
                let x: &[u8; 16] = x.try_into().unwrap();
                for k in 0..16 {
                    p[k] += a[k];
                    a[k] += u32::from(x[k]);
                }
            }
            let (mut sum, mut prefix, mut weighted) = (0u32, 0u32, 0u32);
            for k in 0..16 {
                sum += a[k];
                prefix += p[k];
                weighted += (16 - k as u32) * a[k];
            }
            // No sum here exceeds the s2 the scalar loop would reach.
            s2 += n * 16 * s1 + 16 * prefix + weighted;
            s1 += sum;
        }
        for &b in chunks.remainder() {
            s1 += u32::from(b);
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
    }
    (s2 << 16) | s1
}

fn log2_floor(v: u32) -> u32 {
    31 - v.leading_zeros()
}

fn rust_compress<const U: bool>(data: &[u8], quality: usize) -> Vec<u8> {
    const LENGTHC: [u32; 30] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258, 259];
    const LENGTHEB: [u32; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
    const DISTC: [u32; 31] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577, 32768];
    const DISTEB: [u32; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
    let quality = quality.max(5);
    let cap = 2 * quality;
    let mut tab = vec![0u32; ZHASH * cap];
    let mut cnt = vec![0u32; ZHASH];
    let mut o = Out { p: Vec::with_capacity(65536), bitbuf: 0, bitcount: 0 };
    o.p.push(0x78); // DEFLATE 32K window
    o.p.push(0x5e); // FLEVEL = 1
    o.add_bits(1, 1); // BFINAL = 1
    o.add_bits(1, 2); // BTYPE = 1, fixed Huffman
    let len = data.len();
    let mut i = 0;
    // The hash of position i, carried over from the previous step.
    let mut h = if len > 3 { zhash(data, 0) } else { 0 };
    while i + 3 < len {
        let base = h * cap;
        let n = cnt[h] as usize;
        let limit = (len - i).min(MAX_MATCH);
        let (mut best, mut bestpos) = (3usize, None::<usize>);
        for &cand in tab[base..base + n].iter().rev() {
            let cand = cand as usize;
            if cand + WINDOW <= i {
                break;
            }
            // A newer candidate already won ties. Only a longer match helps.
            if bestpos.is_some() && byte::<U>(data, cand + best) != byte::<U>(data, i + best) {
                continue;
            }
            let d = countm::<U>(data, cand, i, limit);
            if if bestpos.is_none() { d >= best } else { d > best } {
                best = d;
                bestpos = Some(cand);
                if best == limit {
                    break;
                }
            }
        }
        let mut n = n;
        if n == cap {
            tab.copy_within(base + quality..base + cap, base);
            n = quality;
        }
        tab[base + n] = i as u32;
        cnt[h] = n as u32 + 1;

        let mut next_hashed = false;
        if bestpos.is_some() {
            // Lazy matching: if the match at i+1 is longer, emit i as a literal.
            let limit1 = (len - i - 1).min(MAX_MATCH);
            if best < limit1 {
                let h1 = zhash(data, i + 1);
                h = h1;
                next_hashed = true;
                let base1 = h1 * cap;
                for &cand in tab[base1..base1 + cnt[h1] as usize].iter().rev() {
                    let cand = cand as usize;
                    if cand + WINDOW - 1 <= i {
                        break;
                    }
                    if byte::<U>(data, cand + best) != byte::<U>(data, i + 1 + best) {
                        continue;
                    }
                    if countm::<U>(data, cand, i + 1, limit1) > best {
                        bestpos = None;
                        break;
                    }
                }
            }
        }

        if let Some(pos) = bestpos {
            let d = (i - pos) as u32;
            let best32 = best as u32;
            let mut j: usize = if best <= 10 {
                best - 3
            } else if best == MAX_MATCH {
                28
            } else {
                let magnitude = log2_floor(best32 - 3);
                (4 * (magnitude - 1) + ((best32 - 3) >> (magnitude - 2) & 3)) as usize
            };
            let mut bits = if j <= 22 { 7 } else { 8 };
            let mut code = if j <= 22 { bitrev(j as u32 + 1, 7) } else { bitrev(0xc0 + j as u32 - 23, 8) };
            code |= (best32 - LENGTHC[j]) << bits;
            bits += LENGTHEB[j];
            j = if d <= 4 {
                (d - 1) as usize
            } else {
                let magnitude = log2_floor(d - 1);
                (2 * magnitude + ((d - 1) >> (magnitude - 1) & 1)) as usize
            };
            code |= bitrev(j as u32, 5) << bits;
            bits += 5;
            code |= (d - DISTC[j]) << bits;
            bits += DISTEB[j];
            i += best;
            if i + 3 < len {
                h = zhash(data, i);
            }
            o.add_bits(code, bits);
        } else {
            o.huff(u32::from(data[i]));
            i += 1;
            if !next_hashed && i + 3 < len {
                h = zhash(data, i);
            }
        }
    }
    while i < len {
        o.huff(u32::from(data[i]));
        i += 1;
    }
    o.huff(256); // end of block
    while o.bitcount != 0 {
        o.add_bits(0, 1);
    }
    // Store uncompressed instead if compression was worse, as stb does.
    if len > 0 && o.p.len() > len + 2 + (len + 32766) / 32767 * 5 {
        o.p.truncate(2);
        let mut j = 0;
        while j < len {
            let blocklen = (len - j).min(32767);
            o.p.push(u8::from(len - j == blocklen));
            o.p.extend_from_slice(&[blocklen as u8, (blocklen >> 8) as u8, !blocklen as u8, (!blocklen >> 8) as u8]);
            o.p.extend_from_slice(&data[j..j + blocklen]);
            j += blocklen;
        }
    }
    o.p.extend_from_slice(&adler32(data).to_be_bytes());
    o.p
}

// ---------------------------------------------------------------- harness

type CCompress = unsafe extern "C" fn(*mut u8, i32, *mut i32, i32) -> *mut u8;

/// Calls a C compressor; returns the output and the time of the call alone.
fn c_compress(f: CCompress, data: &mut [u8], quality: usize) -> (Vec<u8>, f64) {
    let mut out_len = 0;
    let t = Instant::now();
    let p = unsafe { f(data.as_mut_ptr(), data.len() as i32, &mut out_len, quality as i32) };
    let ms = t.elapsed().as_secs_f64() * 1e3;
    assert!(!p.is_null(), "C compressor failed");
    let v = unsafe { std::slice::from_raw_parts(p, out_len as usize).to_vec() };
    unsafe { free(p as *mut std::ffi::c_void) };
    (v, ms)
}

const NAMES: [&str; 6] = ["C", "C portable", "Rust safe", "Rust unchecked", "C zeroed table", "Rust shipped"];

/// Variant v on data: the output and the time of the compression alone.
fn variant(v: usize, data: &mut [u8], quality: usize) -> (Vec<u8>, f64) {
    match v {
        0 => c_compress(cdef_zlib_compress, data, quality),
        1 => c_compress(cport_zlib_compress, data, quality),
        4 => c_compress(czero_zlib_compress, data, quality),
        5 => c_compress(shipped::termshot_zlib_compress, data, quality),
        _ => {
            let t = Instant::now();
            let out = if v == 2 { rust_compress::<false>(data, quality) } else { rust_compress::<true>(data, quality) };
            let ms = t.elapsed().as_secs_f64() * 1e3;
            (out, ms)
        }
    }
}

fn adler_variant(v: usize, data: &[u8]) -> u32 {
    match v {
        0 => unsafe { cdef_adler32(data.as_ptr(), data.len()) },
        1 => unsafe { cport_adler32(data.as_ptr(), data.len()) },
        2 => adler32(data),
        _ => shipped::adler32(data),
    }
}

/// Every variant must write the same bytes, or the run stops.
fn check(what: &str, data: &[u8], quality: usize) {
    let mut copy = data.to_vec();
    let (want, _) = variant(0, &mut copy, quality);
    for v in 1..NAMES.len() {
        let (got, _) = variant(v, &mut copy, quality);
        assert!(got == want, "{what}: {} writes {} bytes that differ from the C's {}", NAMES[v], got.len(), want.len());
    }
    let a = adler_variant(0, data);
    for v in 1..4 {
        assert_eq!(adler_variant(v, data), a, "{what}: {} Adler-32 differs", ["C", "C portable", "Rust", "Rust shipped"][v]);
    }
}

struct Rng(u64);

impl Rng {
    /// tests/deflate_diff.c's xorshift.
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

/// tests/deflate_diff.c's fill(): noise, runs, copies, image-like rows.
fn fill(rng: &mut Rng, buf: &mut [u8], shape: u32) {
    let len = buf.len();
    let alphabet = 1 + rng.next() % 255;
    let mut i = 0;
    while i < len {
        match shape {
            0 => {
                buf[i] = (rng.next() % alphabet) as u8;
                i += 1;
            }
            1 => {
                let mut run = 1 + rng.next() % 600;
                let b = (rng.next() % alphabet) as u8;
                while run > 0 && i < len {
                    buf[i] = b;
                    i += 1;
                    run -= 1;
                }
            }
            2 => {
                if i < 4 {
                    buf[i] = rng.next() as u8;
                    i += 1;
                    continue;
                }
                let dist = 1 + rng.next() as usize % i.min(40000);
                let mut run = 3 + rng.next() % 300;
                while run > 0 && i < len {
                    buf[i] = buf[i - dist];
                    i += 1;
                    run -= 1;
                }
                if i < len && rng.next() % 3 == 0 {
                    buf[i] = rng.next() as u8;
                    i += 1;
                }
            }
            _ => {
                let stride = 4 * (1 + rng.next() as usize % 64);
                if i < stride {
                    buf[i] = (rng.next() % alphabet) as u8;
                } else {
                    buf[i] = if rng.next() % 50 == 0 { rng.next() as u8 } else { buf[i - stride] };
                }
                i += 1;
            }
        }
    }
}

/// tests/deflate_diff.c's cases (lengths 0-11, then mostly under 20,000 and
/// every 50th past the window, random shapes and qualities 5-16), its
/// all-0xff Adler-32 worst cases, and seeded random-entropy buffers at odd
/// lengths and alignments. Returns the number of cases.
fn check_corpus(cases: usize) -> usize {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut buf = vec![0u8; 200_000];
    for c in 0..cases {
        let len = if c < 12 {
            c
        } else if c % 50 == 0 {
            70000 + rng.next() as usize % 120000
        } else {
            rng.next() as usize % 20000
        };
        let shape = rng.next() % 4;
        let quality = 5 + rng.next() as usize % 12;
        fill(&mut rng, &mut buf[..len], shape);
        check(&format!("deflate_diff case {c} (len {len} shape {shape} quality {quality})"), &buf[..len], quality);
    }
    let mut n = cases;
    for len in [15, 16, 17, 31, 32, 33, 5551, 5552, 5553, 5568, 11103, 11104, 11105, 65536, 199000] {
        for offset in 0..4 {
            let ones = vec![0xffu8; 200_000];
            check(&format!("all-0xff len {} offset {offset}", len - offset), &ones[offset..len], QUALITY);
            n += 1;
        }
    }
    let mut seeded = Rng(0x2545_f491_4f6c_dd1d);
    for k in 0..200 {
        let len = seeded.next() as usize % 150_000;
        let offset = seeded.next() as usize % 16;
        let bits = 1 + k % 8; // 2 to 256 symbols
        let v: Vec<u8> = (0..len + offset).map(|_| (seeded.next() & ((1 << bits) - 1)) as u8).collect();
        check(&format!("random {bits}-bit len {len} offset {offset}"), &v[offset..], QUALITY);
        n += 1;
    }
    n
}

/// Seeded synthetic inputs of reply-24px's size (2,376,720 bytes): uniform
/// random bytes (nearly all literals) and a skewed four-symbol alphabet.
fn synthetic() -> Vec<(String, Vec<u8>)> {
    let len = 2_376_720;
    let mut rng = Rng(0x0123_4567_89ab_cdef);
    let uniform: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
    let skewed: Vec<u8> = (0..len)
        .map(|_| match rng.next() % 16 {
            0..=8 => 0,
            9..=12 => 1,
            13..=14 => 2,
            _ => 3,
        })
        .collect();
    vec![("7-random-uniform".into(), uniform), ("8-random-4sym-skewed".into(), skewed)]
}

/// A seeded shuffle of the six variants for each round (Fisher-Yates).
struct Order(Rng);

impl Order {
    fn next(&mut self) -> [usize; 6] {
        let mut o = [0, 1, 2, 3, 4, 5];
        for i in (1..o.len()).rev() {
            o.swap(i, self.0.next() as usize % (i + 1));
        }
        o
    }
}

fn stats(v: &mut [f64]) -> (f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95 = ((v.len() as f64 * 0.95).ceil() as usize).max(1) - 1;
    (v[v.len() / 2], v[p95])
}

// ---------------------------------------------------------------- inputs

/// Python's random.Random(seed) for a seed below 2^32 (MT19937, seeded by
/// init_by_array([seed])), with randrange as Python 3 computes it. Only for
/// rebuilding scripts/bench.rs's color-grid log byte for byte.
struct PyRandom {
    mt: [u32; 624],
    i: usize,
}

impl PyRandom {
    fn new(seed: u32) -> PyRandom {
        let mut mt = [0u32; 624];
        mt[0] = 19650218;
        for i in 1..624 {
            mt[i] = 1812433253u32.wrapping_mul(mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_add(i as u32);
        }
        // init_by_array with a one-word key: j is always 0.
        let mut i = 1usize;
        for _ in 0..624 {
            mt[i] = (mt[i] ^ (mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_mul(1664525)).wrapping_add(seed);
            i += 1;
            if i >= 624 {
                mt[0] = mt[623];
                i = 1;
            }
        }
        for _ in 0..623 {
            mt[i] = (mt[i] ^ (mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_mul(1566083941)).wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                mt[0] = mt[623];
                i = 1;
            }
        }
        mt[0] = 0x8000_0000;
        PyRandom { mt, i: 624 }
    }

    fn next_u32(&mut self) -> u32 {
        if self.i >= 624 {
            for k in 0..624 {
                let y = (self.mt[k] & 0x8000_0000) | (self.mt[(k + 1) % 624] & 0x7fff_ffff);
                self.mt[k] = self.mt[(k + 397) % 624] ^ (y >> 1) ^ if y & 1 != 0 { 0x9908_b0df } else { 0 };
            }
            self.i = 0;
        }
        let mut y = self.mt[self.i];
        self.i += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// randrange(start, stop): getrandbits(n.bit_length()) until below n.
    fn randrange(&mut self, start: u32, stop: u32) -> u32 {
        let n = stop - start;
        let k = 32 - n.leading_zeros();
        loop {
            let r = self.next_u32() >> (32 - k);
            if r < n {
                return start + r;
            }
        }
    }
}

/// The logs of the #20 round's cases from scripts/bench.rs's
/// legacy_workloads: (name, log, px, cols, rows).
fn bench_logs() -> Vec<(&'static str, Vec<u8>, u32, u32, u32)> {
    let mut rng = PyRandom::new(13);
    let mut colors = String::new();
    for row in 0..30 {
        for col in 0..100 {
            let v: Vec<u32> = (0..6).map(|_| rng.randrange(0, 256)).collect();
            let ch = char::from_u32(rng.randrange(33, 127)).unwrap();
            colors.push_str(&format!(
                "\x1b[{};{}H\x1b[38;2;{};{};{};48;2;{};{};{}m{ch}",
                row + 1, col + 1, v[0], v[1], v[2], v[3], v[4], v[5]
            ));
        }
    }
    let large = vec!["Terminal benchmark 0123456789 ".repeat(9); 80].join("\r\n");
    vec![("blank", Vec::new(), 48, 100, 30), ("color-grid", colors.into_bytes(), 24, 100, 30), ("large", large.into_bytes(), 48, 240, 80)]
}

/// LSB-first bit reader for inflate.
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
    /// n more bits of a Huffman code, which is sent most significant bit first.
    fn code(&mut self, n: u32, mut v: u32) -> u32 {
        for _ in 0..n {
            v = v << 1 | self.bit();
        }
        v
    }
}

/// Inflates a zlib stream of stored and fixed-Huffman blocks, the only kinds
/// deflate.c writes, and checks its Adler-32.
fn inflate(z: &[u8]) -> Vec<u8> {
    const LBASE: [u32; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
    const LEXTRA: [u32; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
    const DBASE: [u32; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
    const DEXTRA: [u32; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
    assert!(z.len() > 6 && z[0] & 0x0f == 8, "not a zlib stream");
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
                    let from = out.len() - dist;
                    for k in 0..len {
                        out.push(out[from + k]);
                    }
                }
            },
            t => panic!("block type {t}: deflate.c writes only stored and fixed blocks"),
        }
        if last == 1 {
            break;
        }
    }
    let at = (b.pos + 7) / 8 + 2;
    assert_eq!(u32::from_be_bytes(z[at..at + 4].try_into().unwrap()), adler32(&out), "Adler-32 of the inflated stream");
    out
}

/// The deflate input of a PNG: its IDAT stream, inflated.
fn idat(png: &[u8]) -> Vec<u8> {
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "not a PNG");
    let (mut pos, mut z) = (8, Vec::new());
    while pos + 8 <= png.len() {
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        if &png[pos + 4..pos + 8] == b"IDAT" {
            z.extend_from_slice(&png[pos + 8..pos + 8 + len]);
        }
        pos += 12 + len;
    }
    inflate(&z)
}

/// Renders every workload with the current CLI and writes the bytes its
/// compressor was given, <name>.raw, to out. The poc_workloads logs
/// (<name>.pty and <name>.meta "cols rows px") use the CLI's defaults, cursor
/// included, so 1-reply-px48 is what a plain reply-sent run compresses. The
/// #20 round's blank, color-grid and large use bench.rs's positional form.
fn make_inputs(termshot: &str, poc: &str, out: &str) {
    let out = std::path::Path::new(out);
    std::fs::create_dir_all(out).unwrap();
    let png = out.join("render.png");
    let render = |args: Vec<String>, name: &str| {
        let status = std::process::Command::new(termshot).args(&args).status().expect("run termshot");
        assert!(status.success(), "termshot {args:?} failed");
        std::fs::write(out.join(format!("{name}.raw")), idat(&std::fs::read(&png).unwrap())).unwrap();
    };
    let mut metas: Vec<_> = std::fs::read_dir(poc).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().map_or(false, |e| e == "meta")).collect();
    metas.sort();
    for meta in metas {
        let text = std::fs::read_to_string(&meta).unwrap();
        let v: Vec<&str> = text.split_whitespace().collect();
        let px = v[2].parse::<f64>().unwrap() as u32;
        let log = meta.with_extension("pty");
        let args = vec!["--raw".into(), "--px".into(), px.to_string(), "--size".into(), format!("{}x{}", v[0], v[1]), log.display().to_string(), png.display().to_string()];
        render(args, &meta.file_stem().unwrap().to_string_lossy());
    }
    let font = "third_party/jetbrains-mono/JetBrainsMono-Regular.ttf";
    for (name, log, px, cols, rows) in bench_logs() {
        let path = out.join(format!("{name}.pty"));
        std::fs::write(&path, log).unwrap();
        let args = vec![path.display().to_string(), png.display().to_string(), font.into(), px.to_string(), cols.to_string(), rows.to_string()];
        render(args, &format!("6-{name}"));
        std::fs::remove_file(&path).unwrap();
    }
    std::fs::remove_file(&png).unwrap();
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--inputs") {
        let a: Vec<String> = std::env::args().collect();
        make_inputs(&a[2], &a[3], &a[4]);
        return;
    }
    let dir = std::env::args().nth(1).expect("input dir");
    let rounds: usize = std::env::args().nth(2).map_or(61, |s| s.parse().unwrap());
    let mut order = Order(Rng(0x5851_f42d_4c95_7f2d));
    let cases = check_corpus(3000);
    println!("checked: {cases} deflate_diff-style and random cases byte-identical across all six variants");

    // Safe Rust zeroes its hash table (vec! is calloc); the C mallocs it
    // and writes each entry before reading it. Time that part alone.
    let mut z = Vec::new();
    for _ in 0..rounds.max(101) {
        let t = Instant::now();
        let tab = vec![0u32; ZHASH * 2 * QUALITY];
        std::hint::black_box(&tab);
        drop(tab);
        z.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let (m, p) = stats(&mut z);
    println!("Rust's zeroed hash table alone ({} KiB, allocate and free): {m:.3} / {p:.3} ms", ZHASH * 2 * QUALITY * 4 / 1024);

    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().map_or(false, |e| e == "raw")).collect();
    paths.sort();
    let mut inputs: Vec<(String, Option<std::path::PathBuf>)> = paths.into_iter().map(|p| (p.file_stem().unwrap().to_string_lossy().into_owned(), Some(p))).collect();
    let mut synth = synthetic().into_iter();
    inputs.push(("7-random-uniform".into(), None));
    inputs.push(("8-random-4sym-skewed".into(), None));

    println!("deflate, time per call in ms, median / p95 of {rounds} rounds; ratios are medians over the C's");
    println!(
        "{:<22} {:>10} {:>15} {:>15} {:>15} {:>15} {:>15} {:>15} {:>6} {:>6} {:>6} {:>6} {:>6}",
        "input", "bytes", "C", "C portable", "Rust safe", "Rust unchecked", "C zeroed table", "Rust shipped", "Cp/C", "safe", "unchk", "Cz/C", "ship"
    );
    let mut adler_inputs = Vec::new();
    for (name, path) in inputs {
        let mut data = match path {
            Some(p) => std::fs::read(p).unwrap(),
            None => synth.next().unwrap().1,
        };
        check(&name, &data, QUALITY);
        let mut t: [Vec<f64>; 6] = Default::default();
        for _ in 0..rounds {
            for v in order.next() {
                let (out, ms) = variant(v, &mut data, QUALITY);
                std::hint::black_box(out);
                t[v].push(ms);
            }
        }
        let s: Vec<(f64, f64)> = t.iter_mut().map(|v| stats(v)).collect();
        let cell = |(m, p): (f64, f64)| format!("{m:.3} / {p:.3}");
        println!(
            "{:<22} {:>10} {:>15} {:>15} {:>15} {:>15} {:>15} {:>15} {:>6.2} {:>6.2} {:>6.2} {:>6.2} {:>6.2}",
            name, data.len(), cell(s[0]), cell(s[1]), cell(s[2]), cell(s[3]), cell(s[4]), cell(s[5]),
            s[1].0 / s[0].0, s[2].0 / s[0].0, s[3].0 / s[0].0, s[4].0 / s[0].0, s[5].0 / s[0].0
        );
        if name.starts_with("1-") || name.starts_with("2-") {
            adler_inputs.push((name, data));
        }
    }

    println!("Adler-32 alone, ms, median / p95 of {rounds} rounds");
    println!("{:<22} {:>10} {:>15} {:>15} {:>15} {:>15} {:>6} {:>6} {:>6}", "input", "bytes", "C", "C portable", "Rust", "Rust shipped", "Cp/C", "R/C", "ship");
    for (name, data) in adler_inputs {
        let mut t: [Vec<f64>; 4] = Default::default();
        for _ in 0..rounds {
            for v in order.next().into_iter().filter(|&v| v < 4) {
                let s = Instant::now();
                std::hint::black_box(adler_variant(v, std::hint::black_box(&data)));
                t[v].push(s.elapsed().as_secs_f64() * 1e3);
            }
        }
        let s: Vec<(f64, f64)> = t.iter_mut().map(|v| stats(v)).collect();
        let cell = |(m, p): (f64, f64)| format!("{m:.3} / {p:.3}");
        println!(
            "{:<22} {:>10} {:>15} {:>15} {:>15} {:>15} {:>6.2} {:>6.2} {:>6.2}",
            name, data.len(), cell(s[0]), cell(s[1]), cell(s[2]), cell(s[3]), s[1].0 / s[0].0, s[2].0 / s[0].0, s[3].0 / s[0].0
        );
    }
}
