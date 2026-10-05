//! The zlib compressor for stb_image_write's PNG writer. stb_glue.c defines
//! STBIW_ZLIB_COMPRESS as termshot_zlib_compress, so stb's writer, which
//! stays C, calls this through FFI. It is stb's own algorithm (public domain,
//! Sean Barrett) and writes the same bytes; tests/deflate_diff.c checks that
//! against stock stb. Search, emission and checksums are faster (both
//! implementations fix an invalid empty stream):
//!
//! - stb scans a hash bucket oldest first and keeps a match when d >= best,
//!   so among equally long matches the newest wins. Scanning newest first and
//!   taking a match only when it is strictly longer picks the same one, and
//!   lets the scan stop at the longest possible length or at the first entry
//!   outside the window (entries are stored in position order).
//! - Lazy matching asks only whether any match at i+1 is longer, so it is
//!   skipped when the current one cannot be beaten.
//! - Candidates that cannot beat the best length are rejected before
//!   comparing their prefixes.
//! - Matches are compared 16 bytes at a time, then 8, then bytes.
//! - Buckets are fixed arrays of positions instead of per-bucket stretchy
//!   buffers; stb caps a bucket at 2*quality entries anyway.
//! - Length/distance indexes are calculated directly and each token is
//!   emitted in one operation.
//! - Adler-32 keeps per-lane sums and applies the weights once per block.
//!
//! stb frees what this returns with free(), so the output buffer comes from
//! libc's malloc and realloc, not from Rust's allocator. Every allocation is
//! checked: when one fails, the call frees what it holds and returns NULL, as
//! stb's own hash-table failure does, and src/render.rs reports that as running out
//! of memory (exit 2). Nothing here aborts on allocation failure.
//!
//! The C version was `src/deflate.c` until #12 step 1; docs/c-vs-rust.md has
//! the comparison that preceded the move.

use std::cell::Cell;
use std::ffi::{c_int, c_uchar, c_void};
use std::ptr;
use std::time::Instant;

const ZHASH: usize = 16384;
const WINDOW: usize = 32768;
const MAX_MATCH: usize = 258;
/// The largest block whose Adler-32 sums cannot overflow 32 bits before the
/// modulo, as in zlib.
const ADLER_BLOCK: usize = 5552;
/// The output buffer's first size; it doubles from there.
const FIRST_OUTPUT: usize = 65536;
/// The longest stored block.
const STORED_MAX: usize = 32767;

extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn realloc(p: *mut c_void, size: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

/// The compressor for stb_image_write (STBIW_ZLIB_COMPRESS): `data_len`
/// bytes at `data` as a zlib stream at stb's `quality`. Returns a buffer for
/// the caller to free() with its length in `*out_len`, or NULL when memory
/// runs out (or `data_len` is negative).
///
/// # Safety
/// `data` must point to `data_len` readable bytes (or be anything when
/// `data_len` is 0), and `out_len` must be writable.
#[no_mangle]
pub unsafe extern "C" fn termshot_zlib_compress(data: *mut c_uchar, data_len: c_int, out_len: *mut c_int, quality: c_int) -> *mut c_uchar {
    let Ok(len) = usize::try_from(data_len) else { return ptr::null_mut() };
    let data = if len == 0 { &[][..] } else { std::slice::from_raw_parts(data as *const u8, len) };
    // A panic must not unwind into C. It has been reported by then; the
    // caller sees a failed compression.
    let compressed = std::panic::catch_unwind(|| compress(data, usize::try_from(quality).unwrap_or(0)));
    match compressed {
        Ok(Some(out)) => match c_int::try_from(out.n) {
            Ok(n) => {
                *out_len = n;
                out.into_raw()
            }
            Err(_) => ptr::null_mut(),
        },
        _ => ptr::null_mut(),
    }
}

/// Stage timings of a call, for TERMSHOT_PROFILE (src/render.rs prints them as the
/// deflate_* fields).
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DeflateTimings {
    pub allocate_ms: f64,
    pub match_emit_ms: f64,
    pub finalize_ms: f64,
    pub checksum_ms: f64,
}

thread_local! {
    /// Whether this thread's calls are timed, and the last successful one's
    /// timings. One compressor call per render, so per thread is enough.
    static PROFILE: Cell<(bool, DeflateTimings)> = Cell::new((false, DeflateTimings::default()));
}

/// Times this thread's calls from now on when `enabled` is nonzero (the
/// render sets it from TERMSHOT_PROFILE), and clears the last timings.
#[no_mangle]
pub extern "C" fn termshot_deflate_profiling(enabled: c_int) {
    PROFILE.with(|p| p.set((enabled != 0, DeflateTimings::default())));
}

/// The timings of this thread's last successful call while profiling was on;
/// zeros if there was none.
///
/// # Safety
/// `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn termshot_deflate_timings(out: *mut DeflateTimings) {
    *out = PROFILE.with(|p| p.get().1);
}

fn ms(from: Option<Instant>, to: Option<Instant>) -> f64 {
    match (from, to) {
        (Some(from), Some(to)) => (to - from).as_secs_f64() * 1000.0,
        _ => 0.0,
    }
}

/// Where the compressor allocates. The fault tests fail each in turn.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Site {
    Table,
    Counts,
    Output,
    Grow,
}

/// Allocation failure injection: the shipped binary has none. Unit tests
/// set it per thread; a build with `--cfg termshot_alloc_faults` reads
/// TERMSHOT_DEFLATE_FAIL_AT=n and fails the nth allocation of each call
/// (tests/run.sh checks that the CLI then exits 2).
#[cfg(any(test, termshot_alloc_faults))]
mod faults {
    use super::Site;
    use std::cell::Cell;

    #[derive(Clone, Copy, Default)]
    pub(super) struct Faults {
        /// Allocations so far, and the one to fail (0 for none).
        pub(super) calls: u32,
        pub(super) fail_at: u32,
        /// Buffers allocated and not yet freed.
        pub(super) live: i32,
    }

    thread_local! {
        pub(super) static FAULTS: Cell<Faults> = Cell::new(Faults::default());
        #[cfg(test)]
        pub(super) static FAILED: Cell<Option<Site>> = Cell::new(None);
    }

    /// Starts a call: count from zero, and fail where asked.
    #[cfg(not(test))]
    pub(super) fn start() {
        let fail_at = std::env::var("TERMSHOT_DEFLATE_FAIL_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        FAULTS.with(|f| f.set(Faults { calls: 0, fail_at, live: 0 }));
    }

    #[cfg(test)]
    pub(super) fn start() {}

    /// Whether this allocation fails.
    pub(super) fn fail(_site: Site) -> bool {
        FAULTS.with(|f| {
            let mut s = f.get();
            s.calls += 1;
            f.set(s);
            let fail = s.calls == s.fail_at;
            #[cfg(test)]
            if fail {
                FAILED.with(|failed| failed.set(Some(_site)));
            }
            fail
        })
    }

    pub(super) fn live(delta: i32) {
        FAULTS.with(|f| {
            let mut s = f.get();
            s.live += delta;
            f.set(s);
        });
    }
}

/// Bytes or zeroed words from libc's allocator: (calloc count size, or
/// malloc size), checked, through the fault hook when there is one.
fn allocate(site: Site, count: usize, size: usize, zeroed: bool) -> *mut c_void {
    #[cfg(any(test, termshot_alloc_faults))]
    if faults::fail(site) {
        return ptr::null_mut();
    }
    let _ = site;
    let p = match count.checked_mul(size) {
        // SAFETY: plain libc calls; the result is checked by the caller.
        Some(_) if zeroed => unsafe { calloc(count, size) },
        Some(bytes) => unsafe { malloc(bytes) },
        None => ptr::null_mut(),
    };
    #[cfg(any(test, termshot_alloc_faults))]
    if !p.is_null() {
        faults::live(1);
    }
    p
}

/// realloc, through the fault hook. On failure `p` is still allocated.
///
/// # Safety
/// `p` must come from `allocate` and not be freed.
unsafe fn reallocate(site: Site, p: *mut c_void, size: usize) -> *mut c_void {
    #[cfg(any(test, termshot_alloc_faults))]
    if faults::fail(site) {
        return ptr::null_mut();
    }
    let _ = site;
    realloc(p, size)
}

/// # Safety
/// `p` must come from `allocate` (or `reallocate`) and not be freed.
unsafe fn release(p: *mut c_void) {
    #[cfg(any(test, termshot_alloc_faults))]
    faults::live(-1);
    free(p);
}

/// `len` zeroed u32s from calloc, freed when dropped.
struct Words {
    p: *mut u32,
    len: usize,
}

impl Words {
    fn new(site: Site, len: usize) -> Option<Words> {
        let p = allocate(site, len, std::mem::size_of::<u32>(), true) as *mut u32;
        (!p.is_null()).then(|| Words { p, len })
    }

    fn as_mut_slice(&mut self) -> &mut [u32] {
        // SAFETY: p holds len u32s, zeroed by calloc, and only this borrows them.
        unsafe { std::slice::from_raw_parts_mut(self.p, self.len) }
    }
}

impl Drop for Words {
    fn drop(&mut self) {
        // SAFETY: p came from allocate and is freed only here.
        unsafe { release(self.p as *mut c_void) }
    }
}

/// The zlib stream being written, in a malloc'd buffer: n bytes of output
/// and cap - n of room. Bits are gathered in bitbuf, least significant first.
struct Out {
    p: *mut u8,
    n: usize,
    cap: usize,
    /// Set when the buffer could not grow. The stream is lost and the call
    /// returns NULL; until then it is written over the buffer's start, so the
    /// hot path needs no check of its own.
    failed: bool,
    bitbuf: u64,
    bitcount: u32,
}

impl Out {
    fn new() -> Option<Out> {
        let p = allocate(Site::Output, 1, FIRST_OUTPUT, false) as *mut u8;
        (!p.is_null()).then(|| Out { p, n: 0, cap: FIRST_OUTPUT, failed: false, bitbuf: 0, bitcount: 0 })
    }

    /// Makes room for `extra` bytes (at most FIRST_OUTPUT): afterwards
    /// cap - n >= extra, whether or not the buffer grew.
    #[inline(always)]
    fn reserve(&mut self, extra: usize) {
        if self.cap - self.n < extra {
            self.grow(extra);
        }
    }

    #[cold]
    #[inline(never)]
    fn grow(&mut self, extra: usize) {
        debug_assert!(extra <= FIRST_OUTPUT);
        if !self.failed {
            let mut cap = self.cap;
            while cap - self.n < extra {
                cap = cap.saturating_mul(2);
            }
            // SAFETY: p came from allocate; on failure it is kept, unchanged.
            let p = unsafe { reallocate(Site::Grow, self.p as *mut c_void, cap) } as *mut u8;
            if !p.is_null() {
                self.p = p;
                self.cap = cap;
                return;
            }
            self.failed = true;
        }
        // cap >= FIRST_OUTPUT >= extra.
        self.n = 0;
    }

    /// Appends bytes, which must fit in FIRST_OUTPUT.
    fn put(&mut self, bytes: &[u8]) {
        self.reserve(bytes.len());
        // SAFETY: reserve left room for bytes after n, and bytes is not in
        // the buffer (it is a caller's slice or the input).
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), self.p.add(self.n), bytes.len()) };
        self.n += bytes.len();
    }

    /// At most 31 new bits plus seven pending: reserve four bytes once, then
    /// store the accumulator's low word without a capacity check per byte,
    /// and keep the complete bytes. Bytes beyond n are scratch and are
    /// overwritten by the next token.
    #[inline(always)]
    fn add_bits(&mut self, code: u32, bits: u32) {
        self.bitbuf |= u64::from(code) << self.bitcount;
        self.bitcount += bits;
        self.reserve(4);
        let word = (self.bitbuf as u32).to_le_bytes();
        // SAFETY: reserve left four bytes after n.
        unsafe { (self.p.add(self.n) as *mut [u8; 4]).write_unaligned(word) };
        let bytes = self.bitcount >> 3;
        self.n += bytes as usize;
        self.bitbuf >>= bytes * 8;
        self.bitcount &= 7;
    }

    /// Fixed Huffman codes, as in stb.
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

    /// The buffer, for the caller to free().
    fn into_raw(self) -> *mut u8 {
        let p = self.p;
        std::mem::forget(self);
        #[cfg(any(test, termshot_alloc_faults))]
        faults::live(-1);
        p
    }
}

impl Drop for Out {
    fn drop(&mut self) {
        // SAFETY: p came from allocate or reallocate and is freed only here.
        unsafe { release(self.p as *mut c_void) }
    }
}

/// Every byte with its bits reversed.
const REVERSED_BYTE: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (i as u8).reverse_bits();
        i += 1;
    }
    t
};

/// code (below 2^bits, bits <= 9) with its bits reversed, Huffman codes
/// being sent most significant bit first. A bit-at-a-time loop here cost
/// GCC 3-10% of matching time when this was C.
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

/// Length of the common prefix of x and y, which are as long: 16 bytes a
/// step, then 8, then bytes.
#[inline(always)]
fn countm(x: &[u8], y: &[u8]) -> usize {
    let limit = x.len().min(y.len());
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

/// Callers pass nonzero values.
fn log2_floor(v: u32) -> u32 {
    31 - v.leading_zeros()
}

/// The zlib stream of data, or None when memory runs out.
fn compress(data: &[u8], quality: usize) -> Option<Out> {
    const LENGTHC: [u32; 30] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258, 259];
    const LENGTHEB: [u32; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
    const DISTC: [u32; 31] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577, 32768];
    const DISTEB: [u32; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
    #[cfg(any(test, termshot_alloc_faults))]
    faults::start();
    let profiling = PROFILE.with(|p| p.get().0);
    let now = || profiling.then(Instant::now);
    let started = now();
    let quality = quality.max(5);
    let cap = 2 * quality;
    // calloc'd, unlike the C's malloc'd table: safe code may only read
    // initialized memory. Fresh pages are zero anyway, so it costs little
    // (docs/c-vs-rust.md measured it).
    let mut tab_words = Words::new(Site::Table, ZHASH * cap)?;
    let mut cnt_words = Words::new(Site::Counts, ZHASH)?;
    let mut o = Out::new()?;
    let (tab, cnt) = (tab_words.as_mut_slice(), cnt_words.as_mut_slice());

    o.put(&[0x78, 0x5e]); // DEFLATE 32K window, FLEVEL = 1
    o.add_bits(1, 1); // BFINAL = 1
    o.add_bits(1, 2); // BTYPE = 1, fixed Huffman

    let allocated = now();
    let len = data.len();
    let mut i = 0;
    // The hash of position i, carried over from the previous step: lazy
    // matching has already hashed i + 1, and after a match it is computed
    // before the token is emitted.
    let mut h = if len > 3 { zhash(data, 0) } else { 0 };
    while i + 3 < len {
        let base = h * cap;
        let n = cnt[h] as usize;
        let limit = (len - i).min(MAX_MATCH);
        let cur = &data[i..i + limit];
        let mut cands = tab[base..base + n].iter().rev().map(|&c| c as usize).take_while(|&c| c + WINDOW > i);
        let (mut best, mut bestpos) = (3usize, None::<usize>);
        // The newest candidate of at least 3 bytes is the first match.
        for cand in &mut cands {
            let d = countm(&data[cand..cand + limit], cur);
            if d >= 3 {
                best = d;
                bestpos = Some(cand);
                break;
            }
        }
        if best < limit {
            for cand in cands {
                // A newer candidate already won ties. Only a longer match helps.
                if data[cand + best] != cur[best] {
                    continue;
                }
                let d = countm(&data[cand..cand + limit], cur);
                if d > best {
                    best = d;
                    bestpos = Some(cand);
                    if best == limit {
                        break;
                    }
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
                h = h1; // i + 1 + 3 < len, as limit1 > best >= 3
                next_hashed = true;
                let base1 = h1 * cap;
                for &cand in tab[base1..base1 + cnt[h1] as usize].iter().rev() {
                    let cand = cand as usize;
                    if cand + WINDOW - 1 <= i {
                        break;
                    }
                    if data[cand + best] != data[i + 1 + best] {
                        continue;
                    }
                    if countm(&data[cand..cand + limit1], &data[i + 1..i + 1 + limit1]) > best {
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
            // A complete length/distance token fits in 31 bits. The 64-bit
            // accumulator also holds the previous token's trailing bits.
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
    let matched = now();
    // Freed here, as the C did, so deflate_finalize_ms holds the frees.
    drop(tab_words);
    drop(cnt_words);

    // Store uncompressed instead if compression was worse, as stb does.
    // Empty input still needs the final fixed-Huffman block above.
    if !o.failed && len > 0 && o.n > len + 2 + (len + STORED_MAX - 1) / STORED_MAX * 5 {
        o.n = 2;
        let mut j = 0;
        for block in data.chunks(STORED_MAX) {
            j += block.len();
            let n = block.len();
            o.put(&[u8::from(j == len), n as u8, (n >> 8) as u8, !n as u8, (!n >> 8) as u8]);
            o.put(block);
        }
    }

    let finalized = now();
    o.put(&adler32(data).to_be_bytes());
    if o.failed {
        return None;
    }
    if profiling {
        let timings = DeflateTimings {
            allocate_ms: ms(started, allocated),
            match_emit_ms: ms(allocated, matched),
            finalize_ms: ms(matched, finalized),
            checksum_ms: ms(finalized, now()),
        };
        PROFILE.with(|p| p.set((true, timings)));
    }
    Some(o)
}

/// Adler-32 of data, by the fastest form for the target. Each gives the
/// value of the scalar definition (the unit tests check every form on
/// x86-64); the arithmetic is integer, so every form gives the same bytes.
pub(crate) fn adler32(data: &[u8]) -> u32 {
    #[cfg(all(target_arch = "x86_64", not(termshot_portable_adler)))]
    return adler32_sse2(data);
    #[cfg(not(all(target_arch = "x86_64", not(termshot_portable_adler))))]
    return adler32_lanes(data);
}

/// Adler-32 in 16 lanes. Within a block of n 16-byte chunks, byte k of chunk
/// c is added to s2 (16 * (n - 1 - c) + 16 - k) times, so it is enough to
/// keep, per lane k, the byte sum a[k] and the sum of earlier byte sums p[k]
/// (p += a before each a += chunk): s2 += 16n * s1 + 16 * sum(p) +
/// sum((16 - k) * a[k]). The inner loop is then only widening adds, with no
/// multiply.
///
/// LLVM's loop vectorizer takes this in safe Rust only in some loop shapes:
/// one chunk per step, as the C had it, stays scalar on arm64 at
/// opt-level=2 (3.3x slower). Eight chunks per step is vectorized at 2 and 3
/// on arm64 and x86-64, and matches Apple clang's NEON C; on x86-64 it loads
/// 4 bytes at a time, which adler32_sse2 avoids. docs/c-vs-rust.md has the
/// codegen. Check the linked binary, not --emit asm, when changing it.
#[cfg_attr(all(target_arch = "x86_64", not(termshot_portable_adler), not(test)), allow(dead_code))]
fn adler32_lanes(data: &[u8]) -> u32 {
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

/// adler32_lanes's sums with SSE2, which every x86-64 has, so there is no
/// runtime detection: one 16-byte load per chunk. psadbw adds a chunk's
/// bytes (the a[k] summed), and pmaddwd weights them by 16 - k (the
/// weighted sum, kept per chunk instead of once per block). p is the sum of
/// the byte sums before each chunk, as there. Integer math, so it gives
/// exactly adler32_lanes's value; the unit tests check it.
#[cfg(target_arch = "x86_64")]
#[cfg_attr(termshot_portable_adler, allow(dead_code))]
fn adler32_sse2(data: &[u8]) -> u32 {
    use std::arch::x86_64::*;
    let (mut s1, mut s2) = (1u32, 0u32);
    for block in data.chunks(ADLER_BLOCK) {
        let chunks = block.chunks_exact(16);
        let n = chunks.len() as u32;
        let tail = chunks.remainder();
        if n > 0 {
            // SAFETY: SSE2 is in x86-64's baseline, and each load reads one
            // 16-byte chunk of block.
            let (sum, prefix, weighted) = unsafe {
                let zero = _mm_setzero_si128();
                let high = _mm_setr_epi16(16, 15, 14, 13, 12, 11, 10, 9);
                let low = _mm_setr_epi16(8, 7, 6, 5, 4, 3, 2, 1);
                // a and p in the two 64-bit halves (psadbw's), the weighted
                // sums in four 32-bit lanes; none can pass 2^31 in a block.
                let (mut a, mut p, mut w) = (zero, zero, zero);
                for x in chunks {
                    let x = _mm_loadu_si128(x.as_ptr() as *const __m128i);
                    p = _mm_add_epi32(p, a);
                    a = _mm_add_epi32(a, _mm_sad_epu8(x, zero));
                    w = _mm_add_epi32(w, _mm_madd_epi16(_mm_unpacklo_epi8(x, zero), high));
                    w = _mm_add_epi32(w, _mm_madd_epi16(_mm_unpackhi_epi8(x, zero), low));
                }
                let lanes = |v: __m128i| {
                    let mut out = [0u32; 4];
                    _mm_storeu_si128(out.as_mut_ptr() as *mut __m128i, v);
                    out
                };
                let (a, p, w) = (lanes(a), lanes(p), lanes(w));
                (a[0] + a[2], p[0] + p[2], w[0] + w[1] + w[2] + w[3])
            };
            // No sum here exceeds the s2 the scalar loop would reach.
            s2 += n * 16 * s1 + 16 * prefix + weighted;
            s1 += sum;
        }
        for &b in tail {
            s1 += u32::from(b);
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
    }
    (s2 << 16) | s1
}

#[cfg(test)]
#[path = "deflate_tests.rs"]
mod deflate_tests;
