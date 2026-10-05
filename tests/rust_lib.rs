//! termshot's Rust that C calls, without the rest of termshot, as a static
//! library for the C harnesses: src/deflate.rs, the compressor
//! (tests/deflate_diff.c checks it against stock stb; tests/codec.c and
//! tests/image.c use it), src/geometry.rs, box drawing and blocks
//! (tests/boxes.c), src/composite.rs, the image layers and the backdrop,
//! and src/glyphs.rs, the text, with src/cell.rs, the cell they read, and
//! src/cff.rs for its vertex. bench/c-vs-rust's harnesses link these with
//! the old draw.c they compare them to. glyphs.rs calls stb only through
//! the functions it is handed, so this library needs no C symbol.
//!
//! With --cfg termshot_render it has src/render.rs too, the render
//! (draw_png, for tests/draw.c and tests/glyphs.c), which calls
//! src/stb_glue.c: those harnesses include it. Without the cfg the library
//! defines no draw_png, which the old draw.c defines.
//!
//! test.sh and tests/run.sh build it with build.sh's -C opt-level=2 and link
//! it as the binary links the modules.

#[path = "../src/cell.rs"]
#[allow(dead_code)]
mod cell;
#[path = "../src/cff.rs"]
#[allow(dead_code)]
mod cff;
#[path = "../src/composite.rs"]
#[allow(dead_code)]
mod composite;
#[path = "../src/deflate.rs"]
mod deflate;
#[path = "../src/geometry.rs"]
mod geometry;
#[path = "../src/glyphs.rs"]
#[allow(dead_code)]
mod glyphs;
#[path = "../src/render.rs"]
#[cfg(termshot_render)]
#[allow(dead_code)]
mod render;
