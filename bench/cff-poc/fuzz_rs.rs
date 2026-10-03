//! cff.rs behind a C entry point, for fuzz.c. Built with overflow checks and
//! debug assertions on, so arithmetic slips panic instead of wrapping.
use std::os::raw::c_int;

#[allow(dead_code)]
mod cff;

/// FNV-1a, as fuzz.c's fnv.
fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h
}

/// Parse face `face` and run every glyph, hashing each outline and box into
/// `hash` as fuzz.c's glyph_hash does. 0: parsed (glyph errors are fine),
/// 1: rejected, 2: panicked.
#[no_mangle]
pub extern "C" fn rs_run(data: *const u8, len: usize, face: c_int, hash: *mut u64) -> c_int {
    let d = unsafe { std::slice::from_raw_parts(data, len) };
    let run = std::panic::catch_unwind(|| match cff::Font::parse(d, face as usize) {
        Ok(font) => {
            let mut h = unsafe { *hash };
            let mut out = Vec::new();
            let mut bounds = [0; 4];
            for g in 0..font.glyphs {
                let n = match font.glyph(g, &mut out, &mut bounds) {
                    Ok(_) => out.len() as i32,
                    Err(_) => -1,
                };
                h = fnv(h, &n.to_ne_bytes());
                for b in bounds {
                    h = fnv(h, &b.to_ne_bytes());
                }
                for v in out.iter().take(n.max(0) as usize) {
                    for c in [v.x, v.y, v.cx, v.cy, v.cx1, v.cy1] {
                        h = fnv(h, &c.to_ne_bytes());
                    }
                    h = fnv(h, &[v.kind]);
                }
            }
            unsafe { *hash = h };
            0
        }
        Err(_) => 1,
    });
    run.unwrap_or(2)
}
