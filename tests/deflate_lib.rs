//! src/deflate.rs alone, as a static library for the C harnesses that link
//! the compressor without the rest of termshot: tests/deflate_diff.c (against
//! stock stb), tests/codec.c, tests/image.c, and the ones that include
//! src/draw.c. test.sh and tests/run.sh build it with build.sh's
//! -C opt-level=2 and link it as the binary links the module.

#[path = "../src/deflate.rs"]
mod deflate;
