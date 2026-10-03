//! Generate src/unicode_tables.rs from the Unicode Character Database.
//! Run tools/unicode-tables.sh, which downloads the files and runs this.
//!
//!   unicode-tables <ucd dir> <version> > src/unicode_tables.rs
//!
//! Widths follow wcwidth as terminals use it:
//! - 0: nonspacing and enclosing marks (Mn, Me), format characters (Cf)
//!   except U+00AD SOFT HYPHEN, and Hangul Jamo medial vowels and finals
//!   (U+1160..U+11FF, U+D7B0..U+D7FF);
//! - 2: East Asian Wide or Fullwidth (W, F), or Emoji_Presentation;
//! - 1: everything else.
//!
//! Canonical compositions are the two-code-point canonical decompositions
//! in UnicodeData.txt, minus Full_Composition_Exclusion.

use std::collections::BTreeSet;
use std::fs;

fn hex(s: &str) -> u32 {
    u32::from_str_radix(s.trim(), 16).unwrap_or_else(|_| panic!("bad code point {s:?}"))
}

/// "0041" or "0041..005A" -> (first, last).
fn range(s: &str) -> (u32, u32) {
    match s.trim().split_once("..") {
        Some((a, b)) => (hex(a), hex(b)),
        None => (hex(s), hex(s)),
    }
}

/// Lines of a UCD property file as (range, value), comments dropped.
fn property_lines(text: &str) -> impl Iterator<Item = ((u32, u32), String)> + '_ {
    text.lines().filter_map(|line| {
        let line = line.split('#').next().unwrap().trim();
        let (cps, value) = line.split_once(';')?;
        Some((range(cps), value.trim().to_string()))
    })
}

/// Merge a set of code points into sorted, non-overlapping ranges.
fn ranges(set: &BTreeSet<u32>) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for &cp in set {
        match out.last_mut() {
            Some((_, last)) if *last + 1 == cp => *last = cp,
            _ => out.push((cp, cp)),
        }
    }
    out
}

fn print_ranges(name: &str, doc: &str, set: &BTreeSet<u32>) {
    let rs = ranges(set);
    println!("/// {doc}");
    println!("pub static {name}: [(u32, u32); {}] = [", rs.len());
    for chunk in rs.chunks(6) {
        let row: Vec<String> = chunk.iter().map(|(a, b)| format!("(0x{a:04X}, 0x{b:04X})")).collect();
        println!("    {},", row.join(", "));
    }
    println!("];");
    println!();
}

/// Widths below WIDTH_LIMIT as a two-level table, so a lookup is two
/// loads instead of two binary searches: WIDTH_BLOCKS has a byte per
/// 64-code-point block naming its row in WIDTH_ROWS, and a row holds 2 bits
/// per code point (the width), 4 to a byte, lowest bits first. Blocks with
/// the same widths share a row.
fn print_width_table(zero: &BTreeSet<u32>, wide: &BTreeSet<u32>) {
    const LIMIT: u32 = 0x40000;
    let mut rows: Vec<[u8; 16]> = Vec::new();
    let mut blocks = Vec::new();
    for block in 0..LIMIT / 64 {
        let mut row = [0u8; 16];
        for k in 0..64 {
            let cp = block * 64 + k;
            let width = if zero.contains(&cp) { 0 } else if wide.contains(&cp) { 2 } else { 1 };
            row[k as usize / 4] |= width << (k % 4 * 2);
        }
        let index = rows.iter().position(|r| *r == row).unwrap_or_else(|| {
            rows.push(row);
            rows.len() - 1
        });
        blocks.push(u8::try_from(index).expect("more than 256 distinct width rows"));
    }
    println!("/// Code points below this have their width in WIDTH_BLOCKS and WIDTH_ROWS.");
    println!("pub const WIDTH_LIMIT: u32 = 0x{LIMIT:X};");
    println!();
    println!("/// The WIDTH_ROWS row of each 64-code-point block below WIDTH_LIMIT.");
    println!("pub static WIDTH_BLOCKS: [u8; {}] = [", blocks.len());
    for chunk in blocks.chunks(24) {
        let row: Vec<String> = chunk.iter().map(|b| b.to_string()).collect();
        println!("    {},", row.join(", "));
    }
    println!("];");
    println!();
    println!("/// The widths of a block's 64 code points, 2 bits each, lowest bits first.");
    println!("pub static WIDTH_ROWS: [[u8; 16]; {}] = [", rows.len());
    for row in &rows {
        let bytes: Vec<String> = row.iter().map(|b| format!("0x{b:02X}")).collect();
        println!("    [{}],", bytes.join(", "));
    }
    println!("];");
    println!();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (dir, version) = (&args[1], &args[2]);
    let read = |name: &str| fs::read_to_string(format!("{dir}/{name}")).unwrap_or_else(|e| panic!("{name}: {e}"));

    let mut zero = BTreeSet::new();
    let mut decompositions = Vec::new();
    for line in read("UnicodeData.txt").lines() {
        let fields: Vec<&str> = line.split(';').collect();
        let cp = hex(fields[0]);
        if matches!(fields[2], "Mn" | "Me" | "Cf") && cp != 0x00AD {
            zero.insert(cp);
        }
        let parts: Vec<&str> = fields[5].split_whitespace().collect();
        if parts.len() == 2 && !parts[0].starts_with('<') {
            decompositions.push((hex(parts[0]), hex(parts[1]), cp));
        }
    }
    zero.extend(0x1160..=0x11FF);
    zero.extend(0xD7B0..=0xD7FF);

    let mut wide = BTreeSet::new();
    for ((a, b), value) in property_lines(&read("EastAsianWidth.txt")) {
        if value == "W" || value == "F" {
            wide.extend(a..=b);
        }
    }
    for ((a, b), value) in property_lines(&read("emoji-data.txt")) {
        if value == "Emoji_Presentation" {
            wide.extend(a..=b);
        }
    }
    // A combining mark in a wide block (U+302A..U+302D) is still zero width.
    wide.retain(|cp| !zero.contains(cp));

    let mut excluded = BTreeSet::new();
    for ((a, b), value) in property_lines(&read("DerivedNormalizationProps.txt")) {
        if value == "Full_Composition_Exclusion" {
            excluded.extend(a..=b);
        }
    }
    let mut compose: Vec<(u32, u32, u32)> = decompositions.into_iter().filter(|(_, _, c)| !excluded.contains(c)).collect();
    compose.sort();

    println!("// Generated by tools/unicode-tables.sh from Unicode {version}. Do not edit.");
    println!();
    print_ranges("ZERO_WIDTH", "Code points that take no cell.", &zero);
    print_ranges("DOUBLE_WIDTH", "Code points that take two cells.", &wide);
    print_width_table(&zero, &wide);
    println!("/// Canonical compositions (base, mark, composed), sorted by base then mark.");
    println!("pub static COMPOSE: [(u32, u32, u32); {}] = [", compose.len());
    for chunk in compose.chunks(4) {
        let row: Vec<String> = chunk.iter().map(|(a, b, c)| format!("(0x{a:04X}, 0x{b:04X}, 0x{c:04X})")).collect();
        println!("    {},", row.join(", "));
    }
    println!("];");
    println!();
    let mut marks: Vec<u32> = compose.iter().map(|&(_, mark, _)| mark).collect();
    marks.sort();
    marks.dedup();
    println!("/// The second code points of COMPOSE, sorted: a mark not here composes with nothing.");
    println!("pub static COMPOSING_MARKS: [u32; {}] = [", marks.len());
    for chunk in marks.chunks(8) {
        let row: Vec<String> = chunk.iter().map(|m| format!("0x{m:04X}")).collect();
        println!("    {},", row.join(", "));
    }
    println!("];");
}
