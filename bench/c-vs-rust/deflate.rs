//! The current src/deflate.c (16-lane Adler-32, bit reversal by table, the
//! inlined matcher with the carried hash) ported to safe Rust, checked
//! byte-identical against the C, then timed against it in one process. Same
//! algorithms; only the language differs. Results and method:
//! docs/c-vs-rust.md. Run it with `bench/c-vs-rust/run.sh deflate`.
//!
//!   deflate <input dir> [rounds]   <name>.raw are deflate inputs (run.sh
//!                                  writes them with deflate_inputs.py)
//!
//! Five variants run in every round, in a seeded random order per round, so
//! each one follows every other about equally often:
//! the C as built by the default compiler, the C with TERMSHOT_PORTABLE_ADLER,
//! the safe Rust, the Rust with two unchecked reads in the match loop
//! (`unchecked`, the 2026-10-01 POC's `--cfg unchecked`, here a const
//! generic so both Rust variants share the process and the rounds), and the
//! default C with its hash table zeroed as the safe Rust's is.

use std::time::Instant;

extern "C" {
    fn cdef_zlib_compress(data: *mut u8, len: i32, out_len: *mut i32, quality: i32) -> *mut u8;
    fn cport_zlib_compress(data: *mut u8, len: i32, out_len: *mut i32, quality: i32) -> *mut u8;
    fn czero_zlib_compress(data: *mut u8, len: i32, out_len: *mut i32, quality: i32) -> *mut u8;
    fn cdef_adler32(d: *const u8, len: usize) -> u32;
    fn cport_adler32(d: *const u8, len: usize) -> u32;
    fn free(p: *mut u8);
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
    unsafe { free(p) };
    (v, ms)
}

const NAMES: [&str; 5] = ["C", "C portable", "Rust safe", "Rust unchecked", "C zeroed table"];

/// Variant v on data: the output and the time of the compression alone.
fn variant(v: usize, data: &mut [u8], quality: usize) -> (Vec<u8>, f64) {
    match v {
        0 => c_compress(cdef_zlib_compress, data, quality),
        1 => c_compress(cport_zlib_compress, data, quality),
        4 => c_compress(czero_zlib_compress, data, quality),
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
        _ => adler32(data),
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
    for v in 1..3 {
        assert_eq!(adler_variant(v, data), a, "{what}: {} Adler-32 differs", NAMES[v]);
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
/// random bytes (every position a literal) and a skewed four-symbol alphabet.
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

/// A seeded shuffle of the five variants for each round (Fisher-Yates).
struct Order(Rng);

impl Order {
    fn next(&mut self) -> [usize; 5] {
        let mut o = [0, 1, 2, 3, 4];
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

fn main() {
    let dir = std::env::args().nth(1).expect("input dir");
    let rounds: usize = std::env::args().nth(2).map_or(61, |s| s.parse().unwrap());
    let mut order = Order(Rng(0x5851_f42d_4c95_7f2d));
    let cases = check_corpus(3000);
    println!("checked: {cases} deflate_diff-style and random cases byte-identical across all five variants");

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
        "{:<22} {:>10} {:>15} {:>15} {:>15} {:>15} {:>15} {:>6} {:>6} {:>6} {:>6}",
        "input", "bytes", "C", "C portable", "Rust safe", "Rust unchecked", "C zeroed table", "Cp/C", "safe", "unchk", "Cz/C"
    );
    let mut adler_inputs = Vec::new();
    for (name, path) in inputs {
        let mut data = match path {
            Some(p) => std::fs::read(p).unwrap(),
            None => synth.next().unwrap().1,
        };
        check(&name, &data, QUALITY);
        let mut t: [Vec<f64>; 5] = Default::default();
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
            "{:<22} {:>10} {:>15} {:>15} {:>15} {:>15} {:>15} {:>6.2} {:>6.2} {:>6.2} {:>6.2}",
            name, data.len(), cell(s[0]), cell(s[1]), cell(s[2]), cell(s[3]), cell(s[4]),
            s[1].0 / s[0].0, s[2].0 / s[0].0, s[3].0 / s[0].0, s[4].0 / s[0].0
        );
        if name.starts_with("1-") || name.starts_with("2-") {
            adler_inputs.push((name, data));
        }
    }

    println!("Adler-32 alone, ms, median / p95 of {rounds} rounds");
    println!("{:<22} {:>10} {:>15} {:>15} {:>15} {:>6} {:>6}", "input", "bytes", "C", "C portable", "Rust", "Cp/C", "R/C");
    for (name, data) in adler_inputs {
        let mut t: [Vec<f64>; 3] = Default::default();
        for _ in 0..rounds {
            for v in order.next().into_iter().filter(|&v| v < 3) {
                let s = Instant::now();
                std::hint::black_box(adler_variant(v, std::hint::black_box(&data)));
                t[v].push(s.elapsed().as_secs_f64() * 1e3);
            }
        }
        let s: Vec<(f64, f64)> = t.iter_mut().map(|v| stats(v)).collect();
        let cell = |(m, p): (f64, f64)| format!("{m:.3} / {p:.3}");
        println!(
            "{:<22} {:>10} {:>15} {:>15} {:>15} {:>6.2} {:>6.2}",
            name, data.len(), cell(s[0]), cell(s[1]), cell(s[2]), s[1].0 / s[0].0, s[2].0 / s[0].0
        );
    }
}
