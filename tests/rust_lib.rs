//! termshot's Rust that C calls, without the rest of termshot, as a static
//! library for the C harnesses: src/deflate.rs, the compressor
//! (tests/deflate_diff.c checks it against stock stb; tests/codec.c and
//! tests/image.c use it), src/geometry.rs, box drawing and blocks
//! (tests/boxes.c), src/composite.rs, the image layers and the backdrop
//! (tests/draw.c), and src/glyphs.rs, the text (tests/glyphs.c), with
//! src/cell.rs, the cell they read, and src/cff.rs for its vertex. The
//! harnesses that include src/draw.c need all of them. glyphs.rs calls stb
//! only through the functions draw.c hands it, so the library needs no
//! symbol of draw.c's and links into harnesses without it.
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
