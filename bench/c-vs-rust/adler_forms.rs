//! Which safe-Rust forms of deflate.c's 16-lane Adler-32 LLVM vectorizes
//! (docs/c-vs-rust.md, 2026-10-03). Every form keeps the same 16 lanes and
//! the same per-lane order of adds; they differ only in loop shape.
//!
//!   rustc --edition 2021 -C opt-level=2 bench/c-vs-rust/adler_forms.rs -o target/adler_forms
//!   target/adler_forms [rounds]       67 MB of seeded random bytes, ms per call
//!
//! Add `--emit asm` to see the loops. `-C opt-level=3`, `-C no-vectorize-loops`
//! and `-C no-vectorize-slp` show which vectorizer does the work.

use std::time::Instant;

const BLOCK: usize = 5552;

#[inline(always)]
fn finish(n: u32, a: &[u32; 16], p: &[u32; 16], s1: &mut u32, s2: &mut u32) {
    let (mut sum, mut prefix, mut weighted) = (0u32, 0u32, 0u32);
    for k in 0..16 {
        sum += a[k];
        prefix += p[k];
        weighted += (16 - k as u32) * a[k];
    }
    *s2 += n * 16 * *s1 + 16 * prefix + weighted;
    *s1 += sum;
}

#[inline(always)]
fn chunk(a: &mut [u32; 16], p: &mut [u32; 16], x: &[u8; 16]) {
    for k in 0..16 {
        p[k] += a[k];
        a[k] += u32::from(x[k]);
    }
}

/// S chunks of 16 bytes per step of the outer loop, then single chunks.
#[inline(always)]
fn steps<const S: usize>(data: &[u8]) -> u32 {
    let (mut s1, mut s2) = (1u32, 0u32);
    for block in data.chunks(BLOCK) {
        let chunks = block.chunks_exact(16);
        let n = chunks.len() as u32;
        if n > 0 {
            let (mut a, mut p) = ([0u32; 16], [0u32; 16]);
            let mut groups = block[..16 * n as usize].chunks_exact(16 * S);
            for g in &mut groups {
                for k in 0..16 {
                    for c in 0..S {
                        p[k] += a[k];
                        a[k] += u32::from(g[16 * c + k]);
                    }
                }
            }
            for x in groups.remainder().chunks_exact(16) {
                chunk(&mut a, &mut p, x.try_into().unwrap());
            }
            finish(n, &a, &p, &mut s1, &mut s2);
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

/// One chunk per step, as the C's plain loop.
#[inline(never)]
fn one_chunk(data: &[u8]) -> u32 {
    let (mut s1, mut s2) = (1u32, 0u32);
    for block in data.chunks(BLOCK) {
        let mut chunks = block.chunks_exact(16);
        let n = chunks.len() as u32;
        if n > 0 {
            let (mut a, mut p) = ([0u32; 16], [0u32; 16]);
            for x in &mut chunks {
                let x: &[u8; 16] = x.try_into().unwrap();
                chunk(&mut a, &mut p, x);
            }
            finish(n, &a, &p, &mut s1, &mut s2);
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

/// The literal port: index a shrinking slice, as the C walks its pointer.
#[inline(never)]
fn literal(mut d: &[u8]) -> u32 {
    let (mut s1, mut s2) = (1u32, 0u32);
    while !d.is_empty() {
        let mut block = d.len().min(BLOCK);
        let chunks = block / 16;
        if chunks > 0 {
            let (mut a, mut p) = ([0u32; 16], [0u32; 16]);
            for _ in 0..chunks {
                for k in 0..16 {
                    p[k] += a[k];
                    a[k] += u32::from(d[k]);
                }
                d = &d[16..];
            }
            finish(chunks as u32, &a, &p, &mut s1, &mut s2);
            block -= chunks * 16;
        }
        for &b in &d[..block] {
            s1 += u32::from(b);
            s2 += s1;
        }
        d = &d[block..];
        s1 %= 65521;
        s2 %= 65521;
    }
    (s2 << 16) | s1
}

#[inline(never)]
fn two_chunks(d: &[u8]) -> u32 {
    steps::<2>(d)
}
#[inline(never)]
fn four_chunks(d: &[u8]) -> u32 {
    steps::<4>(d)
}
#[inline(never)]
fn eight_chunks(d: &[u8]) -> u32 {
    steps::<8>(d)
}

/// The scalar definition, for checking.
fn reference(d: &[u8]) -> u32 {
    let (mut s1, mut s2) = (1u32, 0u32);
    for &b in d {
        s1 = (s1 + u32::from(b)) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    (s2 << 16) | s1
}

fn main() {
    let rounds: usize = std::env::args().nth(1).map_or(31, |s| s.parse().unwrap());
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let data: Vec<u8> = (0..66_819_840).map(|_| next() as u8).collect();
    let forms: [(&str, fn(&[u8]) -> u32); 5] =
        [("one chunk per step (as the C)", one_chunk), ("literal (shrinking slice)", literal), ("2 chunks per step", two_chunks), ("4 chunks per step", four_chunks), ("8 chunks per step", eight_chunks)];
    // Every length and alignment class of the lanes and blocks, all-0xff
    // (the largest sums) and random.
    let ones = vec![0xffu8; 3 * BLOCK + 48];
    for (name, f) in forms {
        for len in (0..80).chain(BLOCK - 40..BLOCK + 40).chain(3 * BLOCK..3 * BLOCK + 40) {
            for off in 0..4 {
                let (o, r) = (&ones[off..off + len], &data[off..off + len]);
                assert_eq!(f(o), reference(o), "{name}: all-0xff len {len}");
                assert_eq!(f(r), reference(r), "{name}: random len {len}");
            }
        }
        assert_eq!(f(&data), reference(&data), "{name}: 67 MB");
    }
    let mut t: Vec<Vec<f64>> = vec![Vec::new(); forms.len()];
    for round in 0..rounds {
        for k in 0..forms.len() {
            let v = (round + k) % forms.len();
            let s = Instant::now();
            std::hint::black_box(forms[v].1(std::hint::black_box(&data)));
            t[v].push(s.elapsed().as_secs_f64() * 1e3);
        }
    }
    println!("Adler-32 forms, 66,819,840 random bytes, ms per call, median / p95 of {rounds} rounds");
    for (k, (name, _)) in forms.iter().enumerate() {
        let v = &mut t[k];
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95 = ((v.len() as f64 * 0.95).ceil() as usize).max(1) - 1;
        println!("{name:<32} {:>8.2} / {:>6.2}", v[v.len() / 2], v[p95]);
    }
}
