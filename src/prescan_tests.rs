//! The one-pass font decision (`needs_cell_metrics`) against the two
//! byte-at-a-time scans it replaced, kept as `needs_cell_metrics_reference`
//! in graphics.rs and sixel.rs.

use super::*;
use crate::vt::bytes_equal;
use std::fs;

/// Every way of deciding, new and old, agrees on `log`; returns the old
/// (kitty, Sixel) answers.
fn agree(log: &[u8], what: &str) -> (bool, bool) {
    let kitty = graphics::needs_cell_metrics_reference(log);
    let sixel = sixel::needs_cell_metrics_reference(log);
    let show = || format!("{what}: b\"{}\"", log.escape_ascii());
    assert_eq!(graphics::needs_cell_metrics(log), kitty, "kitty, {}", show());
    assert_eq!(sixel::needs_cell_metrics(log), sixel, "Sixel, {}", show());
    assert_eq!(needs_cell_metrics(log), kitty || sixel, "both, {}", show());
    (kitty, sixel)
}

/// Every log the repository has: the fixtures, the recorded sessions, the
/// examples and the performance logs.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut logs = Vec::new();
    for dir in ["tests/fixtures", "tests/vt/real", "examples", "tests/perf"] {
        let mut paths: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        paths.sort();
        for path in paths.into_iter().filter(|p| p.is_file()) {
            logs.push((path.display().to_string(), fs::read(&path).unwrap()));
        }
    }
    logs
}

#[test]
fn every_repository_log_decides_as_before() {
    let logs = corpus();
    let fixtures = logs.iter().filter(|(name, _)| name.starts_with("tests/fixtures/")).count();
    assert!(fixtures >= 40, "{fixtures} fixtures");
    let (mut kitty, mut sixel, mut moved) = (0, 0, 0);
    for (name, log) in &logs {
        let (k, s) = agree(log, name);
        kitty += k as usize;
        sixel += s as usize;
        // Every prefix too: a log cut inside a string or before its ST.
        for end in (0..log.len()).step_by(log.len() / 64 + 1) {
            agree(&log[..end], &format!("{name} cut at {end}"));
        }
        // Most kitty fixtures keep the cursor still (C=1); let them move it.
        let mut moving = log.clone();
        for k in 0..moving.len().saturating_sub(2) {
            if &moving[k..k + 3] == b"C=1" {
                moving[k + 2] = b'0';
            }
        }
        if moving != *log {
            moved += agree(&moving, &format!("{name} with C=0")).0 as usize;
        }
    }
    // Both kinds of image that need metrics are there.
    assert!(kitty >= 2 && sixel >= 4 && moved >= 10, "kitty {kitty}, Sixel {sixel}, C=0 {moved}");
}

/// A random kitty command, Sixel string, other string, escape or text, from
/// the pieces the decision looks at, each with a random terminator.
fn fragment(next: &mut impl FnMut() -> u64) -> Vec<u8> {
    let keys = [
        "a=t", "a=T", "a=p", "a=d", "a=q", "a=x", "a=TT", "i=1", "i=2", "i=3", "i=0", "i=007", "i=4294967296",
        "I=1", "I=2", "I=0", "p=1", "p=2", "C=0", "C=1", "C=2", "P=1", "P=0", "Q=1", "H=-1", "V=2", "U=0", "U=1",
        "o=z", "o=x", "f=24", "f=32", "f=100", "f=7", "s=1", "v=1", "m=0", "m=1", "q=2", "q=3", "z=-1", "x=1",
        "X=1", "d=a", "d=i", "S=4", "w=1", "t=d", "t=f", "k=1", "i", "=1", "", "a=T",
    ];
    let ends: [&[u8]; 9] = [b"\x1b\\", b"\x1b\\", b"\x1b\\", b"\x07", b"\x18", b"\x1a", b"\x1b[", b"\x1b\x1b\\", b""];
    let r = next();
    let mut out = Vec::new();
    match r % 8 {
        0..=2 => {
            out.extend_from_slice(if r & 0x100 == 0 { b"\x1b_G" } else { b"\x1b_" });
            let mut parts = Vec::new();
            if r & 0x200 == 0 {
                // Mostly a well-formed command: an action, an image named by
                // a small id or number (or both, or neither), a few more keys.
                // These reach the transmitted, numbered and put-by-id cases.
                parts.push(["a=t", "a=T", "a=T", "a=p", "a=p", "a=p", "a=d", "a=q", ""][(next() % 9) as usize].to_string());
                let id = 1 + next() % 4;
                match next() % 6 {
                    0 | 1 => parts.push(format!("i={id}")),
                    2 | 3 => parts.push(format!("I={id}")),
                    4 => parts.push(format!("i={id},I={id}")),
                    _ => {}
                }
                for _ in 0..next() % 3 {
                    parts.push(["C=1", "P=1", "U=1", "p=1", "q=2", "f=24", "s=1", "v=1", "m=1", "o=z", "z=-1"][(next() % 11) as usize].to_string());
                }
            } else {
                for _ in 0..(r >> 10) % 6 {
                    parts.push(keys[(next() % keys.len() as u64) as usize].to_string());
                }
            }
            if r & 0x8000 != 0 {
                out.push(b',');
            }
            out.extend_from_slice(parts.join(",").as_bytes());
            if r & 0x10000 != 0 {
                out.extend_from_slice(b";/wAA");
            }
        }
        3 | 4 => {
            out.extend_from_slice(b"\x1bP");
            let header: [&[u8]; 11] = [b"0", b"1", b"2", b";", b";", b"q", b"$", b"+", b"\x05", b"\x7f", b"9999999999"];
            for _ in 0..(r >> 8) % 5 {
                out.extend_from_slice(header[(next() % header.len() as u64) as usize]);
            }
            if r & 0x4000 != 0 {
                out.push(b'q');
            }
            out.extend_from_slice(b"#1~-~");
        }
        5 => {
            // A string holding another string's introducer, or ending in one.
            let open: [&[u8]; 5] = [b"\x1b]0;", b"\x1b^", b"\x1bX", b"\x1b_", b"\x1bP"];
            out.extend_from_slice(open[(r >> 8) as usize % open.len()]);
            let inner: [&[u8]; 4] = [b"\x1bPq~", b"\x1b_Ga=T", b"title", b"\x1b"];
            out.extend_from_slice(inner[(r >> 12) as usize % inner.len()]);
        }
        6 => {
            let escapes: [&[u8]; 8] = [b"\x1b", b"\x1b\x1b", b"\x1b[1;2H", b"\x1b[38;5;9m", b"\x1b7", b"\x1b\\", b"\x1bc", b"\x07"];
            return escapes[(r >> 8) as usize % escapes.len()].to_vec();
        }
        _ => {
            for _ in 0..(r >> 8) % 12 {
                let v = next();
                let text = b"x _GPq\\;\r\n\xe7";
                out.push(if v & 1 == 0 { text[(v >> 8) as usize % text.len()] } else { (v >> 16) as u8 });
            }
            return out;
        }
    }
    out.extend_from_slice(ends[(next() % ends.len() as u64) as usize]);
    out
}

/// Generated logs, and the repository's logs cut, spliced, and with
/// fragments and bytes the decision looks at put in. Rounds and seed come
/// from TERMSHOT_FUZZ_ROUNDS and TERMSHOT_FUZZ_SEED, as in the grid fuzz.
#[test]
fn generated_and_mutated_logs_decide_as_before() {
    let env = |name: &str, default: u64| std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
    let rounds = env("TERMSHOT_FUZZ_ROUNDS", 20_000);
    let seed = env("TERMSHOT_FUZZ_SEED", 0x2545_f491_4f6c_dd1d) | 1;
    let mut x = seed;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let logs = corpus();
    let special = b"\x1b\x1b\x1b\\\x07\x18\x1aG_P]^Xq;,=1";
    let mut counts = [[0usize; 2]; 2];
    for round in 0..rounds {
        let mut log = Vec::new();
        if next() % 4 == 0 {
            // A repository log, mutated.
            let (_, base) = &logs[(next() % logs.len() as u64) as usize];
            log.extend_from_slice(base);
            for _ in 0..1 + next() % 4 {
                let at = (next() % (log.len() as u64 + 1)) as usize;
                match next() % 5 {
                    0 => log.truncate(at),
                    1 => {
                        let (_, other) = &logs[(next() % logs.len() as u64) as usize];
                        let from = (next() % (other.len() as u64 + 1)) as usize;
                        let to = from + (next() % 4096) as usize;
                        log.splice(at..at, other[from..to.min(other.len())].iter().copied());
                    }
                    2 if at < log.len() => log[at] = special[(next() % special.len() as u64) as usize],
                    _ => {
                        let f = fragment(&mut next);
                        log.splice(at..at, f);
                    }
                }
            }
        } else {
            for _ in 0..next() % 16 {
                log.extend(fragment(&mut next));
            }
        }
        let (kitty, sixel) = agree(&log, &format!("round {round} (TERMSHOT_FUZZ_SEED={seed})"));
        counts[0][kitty as usize] += 1;
        counts[1][sixel as usize] += 1;
    }
    // Both answers come up often for both kinds, so the agreement means
    // something.
    if rounds >= 20_000 {
        for (kind, [no, yes]) in ["kitty", "Sixel"].iter().zip(counts) {
            assert!(no >= rounds as usize / 20 && yes >= rounds as usize / 20, "{kind}: {no} no, {yes} yes");
        }
    }
}

/// The byte mask against a byte compare, for the three bytes the scan looks
/// for and each byte value in each lane, beside them and their neighbours.
#[test]
fn bytes_equal_is_exact() {
    for byte in [0x1b, b'P', b'_'] {
        for value in 0..=255u8 {
            for lane in 0..8 {
                for fill in [0x00, byte - 1, byte, byte + 1, 0x7f, 0x80, byte | 0x80, 0xff] {
                    let mut word = [fill; 8];
                    word[lane] = value;
                    let want = word.iter().enumerate().fold(0u64, |m, (k, &b)| m | (u64::from(b == byte) << (8 * k + 7)));
                    assert_eq!(bytes_equal(u64::from_le_bytes(word), byte), want, "{byte:#x} in {word:x?}");
                }
            }
        }
    }
}
