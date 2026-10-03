//! #25 CFF POC driver (docs/cff-rust-vs-c.md). Links cff.c and stock
//! stb_truetype (stb_ref.c) and checks cff.rs and cff.c against stb, then
//! times all three in one process.
//!
//!   poc check <font>...                every glyph of every face
//!   poc time <rounds> <font> <face> <first codepoint> <count>
use std::ffi::{c_char, c_int, c_void, CStr};
use std::hint::black_box;
use std::time::Instant;

#[allow(dead_code)]
mod cff;
use cff::{Font, Vertex};

#[repr(C)]
struct Outline {
    v: *mut Vertex,
    len: usize,
    cap: usize,
}

extern "C" {
    fn cff_parse(data: *const u8, len: usize, face: c_int, error: *mut *const c_char) -> *mut c_void;
    fn cff_glyph_count(font: *const c_void) -> c_int;
    fn cff_glyph(font: *const c_void, glyph: c_int, out: *mut Outline, bounds: *mut c_int, error: *mut *const c_char) -> c_int;
    fn cff_free(font: *mut c_void);
    fn free(p: *mut c_void);

    fn stb_faces(data: *const u8) -> c_int;
    fn stb_open(data: *const u8, face: c_int) -> *mut c_void;
    fn stb_glyph_count(font: *const c_void) -> c_int;
    fn stb_glyph_index(font: *const c_void, codepoint: c_int) -> c_int;
    fn stb_shape(font: *const c_void, glyph: c_int, out: *mut *mut Vertex) -> c_int;
    fn stb_free_shape(font: *const c_void, v: *mut Vertex);
    fn stb_box(font: *const c_void, glyph: c_int, bounds: *mut c_int);
    fn stb_draw(font: *const c_void, glyph: c_int, scale: f32, buf: *mut u8, cap: c_int) -> c_int;
    fn stb_raster(v: *const Vertex, n: c_int, bounds: *const c_int, scale: f32, buf: *mut u8, cap: c_int) -> c_int;
}

/// cff.c, behind the same shape of API as cff.rs.
struct CFont(*mut c_void);

impl CFont {
    fn parse(d: &[u8], face: usize) -> Result<CFont, String> {
        let mut error = std::ptr::null();
        let f = unsafe { cff_parse(d.as_ptr(), d.len(), face as c_int, &mut error) };
        if f.is_null() {
            return Err(unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned());
        }
        Ok(CFont(f))
    }

    fn glyphs(&self) -> usize {
        unsafe { cff_glyph_count(self.0) as usize }
    }

    fn glyph(&self, glyph: usize, out: &mut Outline, bounds: &mut [i32; 4]) -> Result<bool, String> {
        let mut error = std::ptr::null();
        match unsafe { cff_glyph(self.0, glyph as c_int, out, bounds.as_mut_ptr(), &mut error) } {
            1 => Ok(true),
            0 => Ok(false),
            _ => Err(unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned()),
        }
    }
}

impl Drop for CFont {
    fn drop(&mut self) {
        unsafe { cff_free(self.0) }
    }
}

impl Outline {
    fn new() -> Outline {
        Outline { v: std::ptr::null_mut(), len: 0, cap: 0 }
    }
    fn as_slice(&self) -> &[Vertex] {
        if self.len == 0 {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(self.v, self.len) }
        }
    }
}

impl Drop for Outline {
    fn drop(&mut self) {
        unsafe { free(self.v as *mut c_void) }
    }
}

/// Stock stb_truetype.
struct Stb(*mut c_void);

impl Stb {
    fn open(d: &[u8], face: usize) -> Option<Stb> {
        let f = unsafe { stb_open(d.as_ptr(), face as c_int) };
        if f.is_null() {
            None
        } else {
            Some(Stb(f))
        }
    }

    fn glyphs(&self) -> usize {
        unsafe { stb_glyph_count(self.0) as usize }
    }

    /// The outline and box, padding bytes zeroed (stb leaves them unset).
    fn glyph(&self, glyph: usize, out: &mut Vec<Vertex>, bounds: &mut [i32; 4]) {
        let mut v = std::ptr::null_mut();
        let n = unsafe { stb_shape(self.0, glyph as c_int, &mut v) };
        out.clear();
        if n > 0 {
            out.extend(unsafe { std::slice::from_raw_parts(v, n as usize) }.iter().map(|&v| Vertex { padding: 0, ..v }));
        }
        unsafe {
            stb_free_shape(self.0, v);
            stb_box(self.0, glyph as c_int, bounds.as_mut_ptr());
        }
    }
}

impl Drop for Stb {
    fn drop(&mut self) {
        unsafe { free(self.0) }
    }
}

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

const CAP: usize = 1 << 20;

fn check(paths: &[String]) -> bool {
    let mut ok = true;
    let (mut faces, mut glyphs, mut drawn, mut rasters) = (0, 0, 0, 0);
    let (mut rv, mut sv) = (Vec::new(), Vec::new());
    let mut cv = Outline::new();
    let (mut a, mut b) = (vec![0u8; CAP], vec![0u8; CAP]);
    let start = Instant::now();
    for path in paths {
        let d = read(path);
        let n = unsafe { stb_faces(d.as_ptr()) }.max(1) as usize;
        for face in 0..n {
            let stb = Stb::open(&d, face).unwrap_or_else(|| panic!("{path} #{face}: stb cannot open it"));
            let rust = Font::parse(&d, face).unwrap_or_else(|e| panic!("{path} #{face}: cff.rs: {e}"));
            let c = CFont::parse(&d, face).unwrap_or_else(|e| panic!("{path} #{face}: cff.c: {e}"));
            assert_eq!((rust.glyphs, c.glyphs()), (stb.glyphs(), stb.glyphs()), "{path} #{face}: glyph count");
            faces += 1;
            for g in 0..stb.glyphs() {
                let (mut sb, mut rb, mut cb) = ([0; 4], [0; 4], [0; 4]);
                stb.glyph(g, &mut sv, &mut sb);
                let r = rust.glyph(g, &mut rv, &mut rb);
                let k = c.glyph(g, &mut cv, &mut cb);
                if r.is_err() || k.is_err() || rv != sv || rb != sb || cv.as_slice() != &sv[..] || cb != sb {
                    if ok {
                        eprintln!("{path} #{face} glyph {g}: stb {} vertices {sb:?}, cff.rs {r:?} {} {rb:?}, cff.c {k:?} {} {cb:?}",
                            sv.len(), rv.len(), cv.len);
                    }
                    ok = false;
                }
                glyphs += 1;
                drawn += !sv.is_empty() as usize;
                // Option C: our outline through stbtt_Rasterize, pixel for
                // pixel what MakeGlyphBitmap draws. A sample, at a whole and
                // a fractional scale.
                if face == 0 && g % 16 == 0 && !rv.is_empty() {
                    for px in [16.0f32, 33.5] {
                        let scale = px / 1000.0;
                        let n = unsafe { stb_draw(stb.0, g as c_int, scale, a.as_mut_ptr(), CAP as c_int) } as usize;
                        let m = unsafe { stb_raster(rv.as_ptr(), rv.len() as c_int, rb.as_ptr(), scale, b.as_mut_ptr(), CAP as c_int) } as usize;
                        if n != m || a[..n] != b[..m] {
                            eprintln!("{path} glyph {g} at {px} px: bitmaps differ ({n} vs {m} bytes)");
                            ok = false;
                        }
                        rasters += 1;
                    }
                }
            }
        }
    }
    println!(
        "{} files, {faces} faces, {glyphs} glyphs ({drawn} with outlines): cff.rs and cff.c {} stb; {rasters} rasters {}; {:.1} s",
        paths.len(),
        if ok { "match" } else { "DIFFER from" },
        if ok { "identical" } else { "checked, some differ" },
        start.elapsed().as_secs_f64()
    );
    println!("most charstring operators in one glyph: {} (limit {})", cff::PEAK_OPS.load(std::sync::atomic::Ordering::Relaxed), cff::MAX_OPS);
    ok
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn time(rounds: usize, path: &str, face: usize, first: u32, count: usize) {
    let d = read(path);
    let stb = Stb::open(&d, face).expect("stb cannot open it");
    let rust = Font::parse(&d, face).unwrap();
    let c = CFont::parse(&d, face).unwrap();
    let glyphs = stb.glyphs();
    // The codepoints a screenshot would draw: the first `count` the font has
    // from `first` on.
    let mut used = Vec::new();
    let mut cp = first;
    while used.len() < count && cp < 0x11_0000 {
        let g = unsafe { stb_glyph_index(stb.0, cp as c_int) };
        if g > 0 {
            used.push(g as usize);
        }
        cp += 1;
    }
    let scale = 16.0 / 1000.0;
    let (mut rv, mut sv) = (Vec::new(), Vec::new());
    let mut cv = Outline::new();
    let mut buf = vec![0u8; CAP];
    let names = ["parse", "all glyphs", "draw used"];
    let impls = ["cff.rs", "cff.c", "stb"];
    let mut t = vec![vec![Vec::new(); 3]; 3];
    for _ in 0..rounds {
        for (k, out) in t.iter_mut().enumerate() {
            // parse
            let s = Instant::now();
            match k {
                0 => drop(black_box(Font::parse(black_box(&d), face).unwrap())),
                1 => drop(black_box(CFont::parse(black_box(&d), face).unwrap())),
                _ => drop(black_box(Stb::open(black_box(&d), face).unwrap())),
            }
            out[0].push(s.elapsed().as_secs_f64());
            // every glyph's outline and box
            let s = Instant::now();
            let mut bounds = [0; 4];
            for g in 0..glyphs {
                match k {
                    0 => drop(black_box(rust.glyph(g, &mut rv, &mut bounds))),
                    1 => drop(black_box(c.glyph(g, &mut cv, &mut bounds))),
                    _ => stb.glyph(g, &mut sv, &mut bounds),
                }
            }
            out[1].push(s.elapsed().as_secs_f64());
            // the used glyphs, rasterised: stb as draw.c calls it today, or
            // our outline handed to stbtt_Rasterize (option C)
            let s = Instant::now();
            for &g in &used {
                let n = match k {
                    0 => {
                        let _ = rust.glyph(g, &mut rv, &mut bounds);
                        unsafe { stb_raster(rv.as_ptr(), rv.len() as c_int, bounds.as_ptr(), scale, buf.as_mut_ptr(), CAP as c_int) }
                    }
                    1 => {
                        let _ = c.glyph(g, &mut cv, &mut bounds);
                        unsafe { stb_raster(cv.v, cv.len as c_int, bounds.as_ptr(), scale, buf.as_mut_ptr(), CAP as c_int) }
                    }
                    _ => unsafe { stb_draw(stb.0, g as c_int, scale, buf.as_mut_ptr(), CAP as c_int) },
                };
                black_box(n);
            }
            out[2].push(s.elapsed().as_secs_f64());
        }
    }
    println!("{path} #{face}: {glyphs} glyphs, {} used (from U+{first:04X}), 16 px; median of {rounds} rounds", used.len());
    for (w, name) in names.iter().enumerate() {
        let per = [1.0, glyphs as f64, used.len() as f64][w];
        let line: Vec<String> = (0..3)
            .map(|k| {
                let m = median(t[k][w].clone());
                if w == 0 {
                    format!("{} {:8.1} µs", impls[k], m * 1e6)
                } else {
                    format!("{} {:7.2} ms ({:5.2} µs/glyph)", impls[k], m * 1e3, m * 1e6 / per)
                }
            })
            .collect();
        println!("  {name:10} {}", line.join("   "));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("check") => std::process::exit(if check(&args[2..]) { 0 } else { 1 }),
        Some("time") => {
            let n = |i: usize| args[i].parse::<usize>().unwrap();
            let first = u32::from_str_radix(args[5].trim_start_matches("U+"), 16).unwrap();
            time(n(2), &args[3], n(4), first, n(6));
        }
        _ => {
            eprintln!("usage: poc check <font>... | poc time <rounds> <font> <face> <first codepoint> <count>");
            std::process::exit(2);
        }
    }
}
