//! termshot's Rust that C calls, without the rest of termshot, as a static
//! library for the C harnesses: src/deflate.rs, the compressor
//! (tests/deflate_diff.c checks it against stock stb; tests/codec.c and
//! tests/image.c use it), src/geometry.rs, box drawing and blocks
//! (tests/boxes.c), and src/composite.rs, the image layers and the backdrop
//! (tests/draw.c), with src/cell.rs, the cell they read. The harnesses that
//! include src/draw.c need all of them. test.sh and tests/run.sh build it
//! with build.sh's -C opt-level=2 and link it as the binary links the
//! modules.

#[path = "../src/cell.rs"]
#[allow(dead_code)]
mod cell;
#[path = "../src/composite.rs"]
#[allow(dead_code)]
mod composite;
#[path = "../src/deflate.rs"]
mod deflate;
#[path = "../src/geometry.rs"]
mod geometry;
