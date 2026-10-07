//! Hand-made hostile CFF fonts for the #25 POC (docs/cff-rust-vs-c.md),
//! the ones src/cff_tests.rs refuses or draws (src/cff_craft.rs makes them).
//!
//!     rustc --edition 2021 bench/cff-poc/craft.rs -o target/cff-poc/craft-fonts
//!     target/cff-poc/craft-fonts <out-dir>
//!
//! Each font has one defect a real-world corrupt or malicious font could
//! have; control.otf is well formed and must draw in all three readers.
//! fuzz.c's `one` mode replays them through stb, cff.c and cff.rs.
#![allow(dead_code)]

#[path = "../../src/cff_craft.rs"]
mod craft;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: craft <out-dir>");
        std::process::exit(2);
    }
    let fonts: [(&str, fn() -> Vec<u8>); 9] = [
        ("subr_bomb", craft::subr_bomb),
        ("index_past_table", craft::index_past_table),
        ("hintmask_overflow", craft::hintmask_overflow),
        ("private_real", craft::private_real),
        ("dict_byte_31", craft::dict_byte_31),
        ("fewer_charstrings", craft::fewer_charstrings),
        ("fdselect_gap", craft::fdselect_gap),
        ("bad_offsize", craft::bad_offsize),
        ("control", craft::control),
    ];
    for (name, font) in fonts {
        let path = format!("{}/{name}.otf", args[1]);
        if let Err(e) = std::fs::write(&path, font()) {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    }
}
