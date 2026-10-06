//! The pixel goldens' cases, which tests/golden.rs draws with ./termshot and
//! tests/library.rs draws again through the library, to compare the PNGs
//! byte for byte. A module of both (`mod golden_cases;`).

pub const CJK: &str = "third_party/noto-sans-cjk/NotoSansCJKtc-Subset.otf";
pub const CJK_VF: &str = "third_party/noto-sans-cjk-vf/NotoSansCJKtc-VF-Subset.otf";
pub const MARKS: &str = "third_party/noto-sans-marks/NotoSans-Marks-Subset.ttf";
// (log, px, cols, rows, options). A log is examples/<log>.pty,
// tests/fixtures/<log>.pty, or an asciinema recording tests/fixtures/<log>.cast;
// with no options it is drawn with FONT, in the original form.
// px 46 is sensitive to FMA contraction (macOS vs Linux, #3); px 48 is the README size.
pub const CASES: [(&str, &str, u32, u32, &[&str]); 68] = [
    ("reply-sent", "46", 100, 30, &[]),
    ("reply-sent", "48", 100, 30, &[]),
    ("draft-ready", "46", 100, 30, &[]),
    ("draft-ready", "48", 100, 30, &[]),
    ("kitty-scroll", "24", 12, 8, &[]),
    ("kitty-rgb", "24", 6, 4, &[]),
    ("kitty-rgb", "47.5", 6, 4, &[]),
    ("kitty-rgba", "24", 6, 4, &[]),
    ("kitty-png", "24", 6, 4, &[]),
    ("kitty-png-alpha", "24", 6, 4, &[]),
    // Compressed (o=z): the same pixels as the four above; -chunks cuts the
    // zlib stream across three chunks.
    ("kitty-rgb-z", "24", 6, 4, &[]),
    ("kitty-rgb-z-chunks", "24", 6, 4, &[]),
    ("kitty-rgb-z-chunks", "47.5", 6, 4, &[]),
    ("kitty-rgba-z", "24", 6, 4, &[]),
    ("kitty-png-z", "24", 6, 4, &[]),
    ("kitty-png-alpha-z", "24", 6, 4, &[]),
    // Source crops and cell offsets (#44 stage 3).
    ("kitty-crop", "24", 6, 4, &[]),
    ("kitty-crop", "47.5", 6, 4, &[]),
    // The z-index layers: below backgrounds, under text, over text.
    ("kitty-layers", "24", 8, 4, &[]),
    ("kitty-layers", "47.5", 8, 4, &[]),
    // Relative placements: a chain that moves with its parent (#44).
    ("kitty-relative", "24", 12, 6, &[]),
    ("kitty-relative", "47.5", 12, 6, &[]),
    // Unicode placeholders: inherited diacritics, a placement named by the
    // underline colour, a 24-bit id with a high byte, a relative placement
    // under a virtual one, a missing image (#44); and kitten icat 0.49.2's
    // --unicode-placeholder output recorded through a PTY.
    ("kitty-placeholders", "24", 16, 8, &[]),
    ("kitty-placeholders", "47.5", 16, 8, &[]),
    ("kitty-icat-placeholder", "24", 20, 6, &[]),
    ("kitty-icat-placeholder", "47.5", 20, 6, &[]),
    // kitten icat 0.49.2 without --place: a 77-byte PNG in base64 without
    // its `=` padding, once with --unicode-placeholder and once direct.
    ("kitty-icat-unpadded", "24", 20, 6, &[]),
    ("kitty-icat-unpadded", "47.5", 20, 6, &[]),
    // Animation frames shown as a still (#44): a frame over its base, one
    // over a background colour, a=c onto the root, and a running animation
    // showing its root; and kitten icat 0.49.2 sending a 3-frame GIF.
    ("kitty-animation", "24", 16, 4, &[]),
    ("kitty-animation", "47.5", 16, 4, &[]),
    ("kitty-icat-animation", "24", 20, 6, &[]),
    ("kitty-icat-animation", "47.5", 20, 6, &[]),
    ("blank", "24", 20, 8, &[]),
    ("geometry", "1", 12, 2, &[]),
    ("geometry", "9", 12, 2, &[]),
    ("geometry", "24", 12, 2, &[]),
    ("geometry", "47.5", 12, 2, &[]),
    ("geometry", "128", 12, 2, &[]),
    ("geometry", "255", 12, 2, &[]),
    ("clipping", "48", 5, 2, &[]),
    ("cache-collisions", "16", 120, 4, &[]),
    ("cache-collisions-1024", "16", 120, 4, &[]),
    ("geometry-offsets", "47.5", 200, 40, &[]),
    ("missing-glyphs", "16", 100, 1, &[]),
    ("csi", "20", 20, 5, &[]),
    ("sgr", "24", 40, 2, &[]),
    ("italic", "24", 30, 5, &[]),
    ("italic", "46", 30, 5, &[]),
    ("control-strings", "24", 20, 4, &[]),
    ("random-colors", "20", 40, 12, &[]),
    // CFF outlines: as the fallback for CJK, and as the only font.
    ("cjk", "24", 40, 4, &["--fallback-font", CJK]),
    ("cjk", "46", 40, 4, &["--font", CJK]),
    // CFF2 outlines (a variable font, drawn at its default instance), the same ways.
    ("cff2", "24", 40, 4, &["--fallback-font", CJK_VF]),
    ("cff2", "46", 40, 4, &["--font", CJK_VF]),
    // Combining marks with no precomposed form (#14), drawn over their
    // characters: Latin from the built-in font, and Thai, Hebrew and
    // Devanagari (approximate: no shaping) from the fallback.
    ("marks", "24", 40, 6, &["--fallback-font", MARKS]),
    ("marks", "46", 40, 6, &["--fallback-font", MARKS]),
    // Glyphs and marks reaching into the rows above and below (the bracket
    // pieces, by a pixel), over backgrounds and images below and under the
    // text, with the backdrop painted at once (a small raster) and a row of
    // cells at a time (one over 16 MiB), which must draw the same (#22).
    ("row-overlap", "24", 100, 30, &["--fallback-font", MARKS]),
    ("row-overlap", "96", 100, 30, &["--fallback-font", MARKS]),
    ("cursor-underline", "24", 8, 2, &[]),
    ("cursor-bar", "24", 8, 2, &[]),
    // Sixel: hand-made images (HLS, P2 0 and 1, $ and -, the VT340 palette,
    // scrolling), and ImageMagick 7.1.2-31's output for two generated images
    // (`magick in.png -colors 16 sixel:-`, and 64 dithered colours).
    ("sixel-hand", "24", 20, 8, &[]),
    ("sixel-magick", "24", 40, 4, &[]),
    ("sixel-magick-dither", "24", 40, 5, &[]),
    // Text over a Sixel image, and ED 0 and 1, clear its pixels; a kitty
    // image beside it keeps them.
    ("sixel-text", "24", 20, 6, &[]),
    // asciicast v2 and v3: the output of tests/fixtures/asciicast.pty in
    // events, so the same pixels (test.sh compares them with the raw log).
    ("asciicast-v2", "24", 24, 6, &[]),
    ("asciicast-v3", "24", 24, 6, &[]),
    // A palette (#87): Solarized Dark in kitty's keys, over the 16 colours as
    // foregrounds, backgrounds and 38;5;n / 48;5;n, the cube and 24-bit
    // colours it leaves alone, attributes, box drawing and the bar cursor.
    ("palette", "24", 72, 5, &["--palette", "tests/fixtures/solarized-dark.conf"]),
    // Padding (#87): the kitty layers in a margin of the default background
    // (a size of their own, so the PNG is not kitty-layers-24's).
    ("kitty-layers", "33", 8, 4, &["--padding", "12,6"]),
];
