//! Record the metrics HarfBuzz gives a variable font at an instance, which
//! src/metrics_tests.rs checks termshot's HVAR and MVAR against: the
//! horizontal advance of every glyph, and the ascender, descender and line gap.
//!
//!     tools/cff2-metrics.sh [--variations=LIST] FONT OUT
//!
//! --variations takes hb-view's list (wght=700,wdth=100); without it the
//! default instance is recorded. It calls libharfbuzz directly (the wrapper
//! links it), so it needs only the library, and asks for each glyph by id,
//! as no shaping is involved. The font is read at its units per em, so
//! every value is in font units, as HarfBuzz rounds it.
//!
//! The extents are hhea's, as termshot reads them (stb_truetype does), varied
//! by HarfBuzz's MVAR deltas (hb_ot_metrics_get_variation) and rounded as
//! hb_font_get_h_extents rounds them. hb_font_get_h_extents itself starts from
//! OS/2's typo metrics instead when fsSelection sets USE_TYPO_METRICS, so it
//! is only a cross-check: where it starts from hhea, the two must agree.
//!
//! The output starts with comment lines naming the HarfBuzz version, the
//! variations and the extents ("# extents: ASCENDER DESCENDER LINE_GAP"),
//! then a glyph id and its advance per line.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::process::exit;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Variation {
    tag: u32,
    value: f32,
}

/// hb_font_extents_t: three metrics, then nine reserved fields.
#[repr(C)]
#[derive(Default)]
struct Extents {
    ascender: i32,
    descender: i32,
    line_gap: i32,
    reserved: [i32; 9],
}

extern "C" {
    fn hb_version_string() -> *const c_char;
    fn hb_blob_create_from_file_or_fail(path: *const c_char) -> *mut c_void;
    fn hb_face_create(blob: *mut c_void, index: c_uint) -> *mut c_void;
    fn hb_face_get_glyph_count(face: *mut c_void) -> c_uint;
    fn hb_face_reference_table(face: *mut c_void, tag: u32) -> *mut c_void;
    fn hb_blob_get_data(blob: *mut c_void, length: *mut c_uint) -> *const c_char;
    fn hb_font_create(face: *mut c_void) -> *mut c_void;
    fn hb_variation_from_string(text: *const c_char, len: c_int, variation: *mut Variation) -> c_int;
    fn hb_font_set_variations(font: *mut c_void, variations: *const Variation, count: c_uint);
    fn hb_font_get_glyph_h_advance(font: *mut c_void, glyph: u32) -> i32;
    fn hb_font_get_h_extents(font: *mut c_void, extents: *mut Extents) -> c_int;
    fn hb_ot_metrics_get_variation(font: *mut c_void, tag: u32) -> f32;
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    exit(1);
}

fn tag(name: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*name)
}

/// A table of the face, empty when it has none.
fn table(face: *mut c_void, name: &[u8; 4]) -> Vec<u8> {
    let mut length: c_uint = 0;
    // SAFETY: HarfBuzz returns the blob's data and its length, or an empty
    // blob for a missing table; the bytes are copied before anything else.
    unsafe {
        let data = hb_blob_get_data(hb_face_reference_table(face, tag(name)), &mut length);
        if length == 0 || data.is_null() {
            return Vec::new();
        }
        std::slice::from_raw_parts(data as *const u8, length as usize).to_vec()
    }
}

/// HarfBuzz's roundf, floorf(v + 0.5f), in f32.
fn roundf(v: f64) -> i64 {
    ((v + 0.5) as f32 as f64).floor() as i64
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut variations = String::new();
    if let Some(list) = args.first().and_then(|a| a.strip_prefix("--variations=")) {
        variations = list.to_string();
        args.remove(0);
    }
    if args.len() != 2 {
        fail("    tools/cff2-metrics.sh [--variations=LIST] FONT OUT");
    }
    let (font_path, out_path) = (&args[0], &args[1]);
    let path = CString::new(font_path.as_str()).unwrap_or_else(|_| fail(&format!("{font_path}: cannot read it")));
    // SAFETY: plain HarfBuzz calls on objects it made; none is freed, as
    // the process ends after one font.
    unsafe {
        let blob = hb_blob_create_from_file_or_fail(path.as_ptr());
        if blob.is_null() {
            fail(&format!("{font_path}: cannot read it"));
        }
        let face = hb_face_create(blob, 0);
        let font = hb_font_create(face);
        if !variations.is_empty() {
            let mut settings = Vec::new();
            for setting in variations.split(',') {
                let mut variation = Variation::default();
                let text = CString::new(setting).unwrap_or_else(|_| fail(&format!("'{setting}' is not a variation setting")));
                if hb_variation_from_string(text.as_ptr(), -1, &mut variation) == 0 {
                    fail(&format!("'{setting}' is not a variation setting"));
                }
                settings.push(variation);
            }
            hb_font_set_variations(font, settings.as_ptr(), settings.len() as c_uint);
        }
        // The base value varied by MVAR, as f32.
        let varied = |name: &[u8; 4], base: i16| (f64::from(base) + f64::from(hb_ot_metrics_get_variation(font, tag(name)))) as f32;
        let hhea = table(face, b"hhea");
        if hhea.len() < 10 {
            fail(&format!("{font_path}: no hhea table"));
        }
        let field = |at: usize| i16::from_be_bytes([hhea[at], hhea[at + 1]]);
        let ascender = roundf(f64::from(varied(b"hasc", field(4)).abs()));
        let descender = roundf(f64::from(-varied(b"hdsc", field(6)).abs()));
        let line_gap = roundf(f64::from(varied(b"hlgp", field(8))));
        let os2 = table(face, b"OS/2");
        if !(os2.len() >= 64 && u16::from_be_bytes([os2[62], os2[63]]) & 0x80 != 0) {
            let mut extents = Extents::default();
            hb_font_get_h_extents(font, &mut extents);
            let harfbuzz = (i64::from(extents.ascender), i64::from(extents.descender), i64::from(extents.line_gap));
            if harfbuzz != (ascender, descender, line_gap) {
                fail(&format!(
                    "{font_path}: hb_font_get_h_extents gives {harfbuzz:?}, hhea and MVAR {:?}",
                    (ascender, descender, line_gap)
                ));
            }
        }
        let version = CStr::from_ptr(hb_version_string()).to_string_lossy();
        let mut out = format!("# HarfBuzz {version}, from tools/cff2-metrics.rs\n");
        if !variations.is_empty() {
            out.push_str(&format!("# variations: {variations}\n"));
        }
        out.push_str(&format!("# extents: {ascender} {descender} {line_gap}\n"));
        for gid in 0..hb_face_get_glyph_count(face) {
            out.push_str(&format!("{gid} {}\n", hb_font_get_glyph_h_advance(font, gid)));
        }
        if let Err(e) = std::fs::write(out_path, out) {
            fail(&format!("{out_path}: {e}"));
        }
    }
}
