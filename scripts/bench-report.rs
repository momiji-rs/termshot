//! Markdown tables from scripts/bench.sh JSON results, for docs/performance.md.
//!
//!     scripts/bench-report.sh A.json [B.json ...] --label branch
//!
//! Prints, per case: median/p95 wall, child CPU and peak RSS for each binary,
//! the paired speedup of every batch, profiling overhead, the stages that
//! dominate, and the counters that show which path ran. Nothing is measured
//! here.
//!
//! The Rust port of bench-report.py: on every report in docs/ it prints what
//! the Python printed, byte for byte. A report missing a key it needs stops
//! it with status 1 after the lines already printed, as the Python's
//! KeyError did.

#[path = "bench_common.rs"]
mod common;

use common::*;
use std::io::Write;

const PROG: &str = "bench-report.sh";
const USAGE: &str = "usage: bench-report.sh [-h] [--label LABEL] results [results ...]\n";
const HELP_TAIL: &str = "
Markdown tables from scripts/bench.sh JSON results, for docs/performance.md.

    scripts/bench-report.sh A.json [B.json ...] --label branch

Prints, per case: median/p95 wall, child CPU and peak RSS for each binary,
the paired speedup of every batch, profiling overhead, the stages that
dominate, and the counters that show which path ran. Nothing is measured here.

positional arguments:
  results

options:
  -h, --help     show this help message and exit
  --label LABEL  binary whose profile is ranked
";

/// Stages that do not overlap (docs/performance.md, "Stage timings"), so
/// their medians can be ranked side by side. foreground_other is foreground
/// minus its three child timers; font_load holds every font_* and fallback_*
/// timer.
const LEAVES: [&str; 16] = [
    "input_read_ms",
    "font_load_ms",
    "parse_ms",
    "face_ms",
    "font_setup_ms",
    "allocate_ms",
    "background_ms",
    "geometry_ms",
    "glyph_ms",
    "blend_ms",
    "foreground_other_ms",
    "deflate_match_emit_ms",
    "deflate_checksum_ms",
    "png_deflate_other_ms",
    "png_pack_ms",
    "output_write_ms",
];

type R<T> = Result<T, String>;

fn out(line: &str) {
    let mut stdout = std::io::stdout().lock();
    if writeln!(stdout, "{line}").is_err() {
        std::process::exit(1);
    }
}

/// A binary's output size or hash: output_* since the text suite, png_* in
/// earlier reports.
fn output<'a>(entry: &'a Json, field: &str) -> R<&'a Json> {
    let map = entry.as_obj()?;
    Ok(map
        .get(&format!("output_{field}"))
        .or_else(|| map.get(&format!("png_{field}")))
        .unwrap_or(&Json::Null))
}

/// f'{x:.{digits}f}' of a report's number.
fn fmt(x: &Json, digits: usize) -> R<String> {
    Ok(fixed(x.as_num()?.f(), digits))
}

/// str() of a value in an f-string.
fn py_str(v: &Json) -> String {
    match v {
        Json::Null => "None".to_string(),
        Json::Bool(b) => if *b { "True" } else { "False" }.to_string(),
        Json::Int(i) => i.to_string(),
        Json::Float(f) => py_float_repr(*f),
        Json::Str(s) => s.clone(),
        other => dumps(other),
    }
}

fn ratio_cell(s: &Json) -> R<String> {
    let ci = s.at("bootstrap_95pct")?;
    Ok(format!("{} [{}, {}]", fmt(s.at("median")?, 3)?, fmt(ci.idx(0)?, 3)?, fmt(ci.idx(1)?, 3)?))
}

/// Per-run leaf stages from a columnar profile_samples record.
fn leaves(samples: &Json) -> R<Vec<(&'static str, Num)>> {
    let map = samples.as_obj()?;
    let total = match samples.at("total_ms")? {
        Json::Arr(v) => v.len(),
        _ => return Err("TypeError: total_ms is not a list".to_string()),
    };
    let mut columns: Vec<Vec<Num>> = vec![Vec::new(); LEAVES.len()];
    for i in 0..total {
        let get = |key: &str| -> R<Num> {
            match map.get(key) {
                Some(column) => column.idx(i)?.as_num(),
                None => Ok(Num::Float(0.0)),
            }
        };
        let mut row: Vec<(&str, Num)> = Vec::new();
        let set = |row: &mut Vec<(&str, Num)>, key: &'static str, v: Num| match row.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = v,
            None => row.push((key, v)),
        };
        for key in LEAVES {
            if map.contains(key) {
                set(&mut row, key, get(key)?);
            }
        }
        set(&mut row, "face_ms", get("face_ms")?);
        let fg = get("foreground_ms")?.sub(get("geometry_ms")?).sub(get("glyph_ms")?).sub(get("blend_ms")?);
        set(&mut row, "foreground_other_ms", fg);
        let deflate = get("png_deflate_ms")?
            .sub(get("deflate_match_emit_ms")?)
            .sub(get("deflate_checksum_ms")?)
            .sub(get("deflate_allocate_ms")?)
            .sub(get("deflate_finalize_ms")?);
        set(&mut row, "png_deflate_other_ms", deflate);
        for (n, key) in LEAVES.iter().enumerate() {
            let v = row.iter().find(|(k, _)| k == key).map_or(Num::Float(0.0), |(_, v)| *v);
            columns[n].push(v);
        }
    }
    if total == 0 {
        return Err("statistics.StatisticsError: no median for empty data".to_string());
    }
    Ok(LEAVES.iter().zip(columns).map(|(k, v)| (*k, median(&v))).collect())
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let help = format!("{USAGE}{HELP_TAIL}");
    let opts = [Opt { name: "--label", metavar: Some("LABEL"), choices: None, int: false }];
    let args = parse_args(&argv, &opts, USAGE, &help, PROG, &[], Some("results"));
    let label = args.last("--label").unwrap_or("branch").to_string();
    let results: Vec<String> = args.positionals.iter().map(|p| py_path(p)).collect();
    let mut batches = Vec::new();
    for path in &results {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        });
        batches.push(parse_json(&text).unwrap_or_else(|e| {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }));
    }
    if let Err(e) = report(&batches, &results, &label) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn report(batches: &[Json], results: &[String], label: &str) -> R<()> {
    let first = &batches[0];
    let labels = first.at("binaries")?.as_obj()?.keys();
    let reference = first.at("reference")?;
    let machine = first.at("machine")?;
    out(&format!(
        "host {}: {}, {}",
        py_str(machine.at("hostname")?),
        py_str(machine.at("cpu")?),
        py_str(machine.at("platform")?)
    ));
    for (b, path) in batches.iter().zip(results) {
        let mut binaries = Vec::new();
        for (k, v) in &b.at("binary_sha256")?.as_obj()?.0 {
            binaries.push(format!("{k}={}", v.as_str()?.chars().take(12).collect::<String>()));
        }
        out(&format!(
            "- {}: seed {}, {} runs, {}, load {} -> {}, binaries {}",
            py_name(path),
            py_str(b.at("seed")?),
            py_str(b.at("runs")?),
            py_str(b.at("timestamp_utc")?),
            fmt(b.at("load_average_start")?.idx(0)?, 2)?,
            fmt(b.at("load_average_end")?.idx(0)?, 2)?,
            binaries.join(", ")
        ));
    }
    out("");
    let join = |f: &dyn Fn(&str) -> String| labels.iter().map(|l| f(l)).collect::<Vec<_>>().join(" | ");
    out(&format!(
        "| case | {} | {} | {} | output bytes |",
        join(&|l| format!("{l} wall med / p95")),
        join(&|l| format!("{l} CPU")),
        join(&|l| format!("{l} RSS MiB"))
    ));
    out(&format!("| --- |{}", " ---: |".repeat(3 * labels.len() + 1)));
    let cases = first.at("cases")?.as_obj()?;
    for (name, case) in &cases.0 {
        let mut walls = Vec::new();
        let mut cpus = Vec::new();
        let mut rss = Vec::new();
        for l in &labels {
            let e = case.at(l)?;
            walls.push(format!("{} / {}", fmt(e.at("wall_ms")?.at("median")?, 2)?, fmt(e.at("wall_ms")?.at("p95")?, 2)?));
        }
        for l in &labels {
            cpus.push(fmt(case.at(l)?.at("child_cpu_ms")?.at("median")?, 2)?);
        }
        for l in &labels {
            let peak = case.at(l)?.at("peak_rss_bytes")?;
            rss.push(if peak.truthy() {
                fixed(peak.at("median")?.as_num()?.div(Num::Int(1 << 20))?.f(), 2)
            } else {
                "-".to_string()
            });
        }
        let bytes = match output(case.at(&labels[0])?, "bytes")? {
            Json::Int(i) => thousands(*i),
            Json::Bool(b) => thousands(*b as i128),
            Json::Float(f) => format!("{f}"),
            other => return Err(format!("TypeError: unsupported format string passed to {}.__format__", py_str(other))),
        };
        out(&format!("| {name} | {} | {} | {} | {bytes} |", walls.join(" | "), cpus.join(" | "), rss.join(" | ")));
    }
    out("");
    let others: Vec<&String> = labels.iter().filter(|l| Json::Str(l.to_string()) != *reference).collect();
    if reference.truthy() && !others.is_empty() {
        let r = py_str(reference);
        let mut head = Vec::new();
        for l in &others {
            for i in 0..batches.len() {
                head.push(format!("{r}/{l} batch {} [95%]", i + 1));
            }
        }
        out(&format!("| case | {} | {} |", head.join(" | "), join(&|l| format!("{l} profiled/plain [95%]"))));
        out(&format!("| --- |{}", " ---: |".repeat(others.len() * batches.len() + labels.len())));
        for (name, _) in &cases.0 {
            let mut cells = Vec::new();
            for l in &others {
                for b in batches {
                    cells.push(ratio_cell(b.at("cases")?.at(name)?.at(l)?.at("paired_wall_speedup")?)?);
                }
            }
            for l in &labels {
                cells.push(ratio_cell(first.at("cases")?.at(name)?.at(l)?.at("profile_overhead")?)?);
            }
            out(&format!("| {name} | {} |", cells.join(" | ")));
        }
        out("");
    }
    if first.as_obj()?.contains("slim") {
        // A slim report keeps only the stages named with --stage; the stage
        // table and counters would read the dropped ones as zero.
        let kept: Vec<String> = match first.at("slim")?.at("profile_stages_kept")? {
            Json::Arr(v) => v.iter().map(py_str).collect(),
            _ => return Err("TypeError: profile_stages_kept is not a list".to_string()),
        };
        out("Top stages and counters: not in this report (slim; bench.sh --full-profile keeps them)");
        if !kept.is_empty() {
            out("");
            let head: Vec<String> = kept.iter().map(|k| format!("{k} ({label}, batch 1 median)")).collect();
            out(&format!("| case | {} |", head.join(" | ")));
            out(&format!("| --- |{}", " ---: |".repeat(kept.len())));
            for (name, case) in &cases.0 {
                let prof = case.at(label)?.at("profile")?.as_obj()?;
                let mut cells = Vec::new();
                for k in &kept {
                    cells.push(match prof.get(k) {
                        Some(p) => fmt(p.at("median")?, 3)?,
                        None => "-".to_string(),
                    });
                }
                out(&format!("| {name} | {} |", cells.join(" | ")));
            }
        }
        out("");
    } else {
        stages(first, label)?;
    }
    outputs(batches, results, &labels)
}

fn stages(first: &Json, label: &str) -> R<()> {
    out(&format!("Top stages ({label}, batch 1 medians, ms) and counters:"));
    out("");
    out("| case | profiled wall | total_ms | top stages | font_load parts | glyphs: raster / hits / evict / missing / fallback lookups / fallback raster |");
    out("| --- | ---: | ---: | --- | --- | --- |");
    for (name, case) in &first.at("cases")?.as_obj()?.0 {
        let entry = case.at(label)?;
        let mut med = leaves(entry.at("profile_samples")?)?;
        // sorted(key=-value): highest first, ties in LEAVES order.
        med.sort_by(|a, b| b.1.cmp(a.1));
        let prof = entry.at("profile")?.as_obj()?;
        let m = |key: &str| -> R<Num> {
            match prof.get(key) {
                Some(p) => p.at("median")?.as_num(),
                None => Ok(Num::Float(0.0)),
            }
        };
        let mut parts = Vec::new();
        for f in ["font", "fallback"] {
            if m(&format!("{f}_bytes"))?.f() != 0.0 {
                let p = |s: &str| -> R<String> { Ok(fixed(m(&format!("{f}_{s}_ms"))?.f(), 3)) };
                parts.push(format!("{f}: {}/{}/{}/{}", p("allocate")?, p("read")?, p("check")?, p("padding")?));
            }
        }
        let mut counters = Vec::new();
        for k in [
            "glyph_rasterizations",
            "glyph_cache_hits",
            "glyph_cache_evictions",
            "glyph_missing",
            "fallback_lookups",
            "fallback_rasterizations",
        ] {
            counters.push(match m(k)? {
                Num::Int(i) => i.to_string(),
                Num::Float(f) => (f.trunc() as i128).to_string(),
            });
        }
        let top: Vec<String> = med
            .iter()
            .take(3)
            .map(|(k, v)| format!("{} {}", &k[..k.len() - 3], fixed(v.f(), 2)))
            .collect();
        out(&format!(
            "| {name} | {} | {} | {} | {} | {} |",
            fmt(entry.at("profiled_wall_ms")?.at("median")?, 2)?,
            fixed(m("total_ms")?.f(), 2),
            top.join(", "),
            parts.join(", "),
            counters.join(" / ")
        ));
    }
    out("");
    Ok(())
}

fn outputs(batches: &[Json], results: &[String], labels: &[String]) -> R<()> {
    let first = &batches[0];
    let cases = first.at("cases")?.as_obj()?;
    let mut hashes: Vec<(Json, Vec<String>)> = Vec::new();
    for (name, case) in &cases.0 {
        let h = output(case.at(&labels[0])?, "sha256")?.clone();
        match hashes.iter_mut().find(|(k, _)| *k == h) {
            Some((_, names)) => names.push(name.clone()),
            None => hashes.push((h, vec![name.clone()])),
        }
    }
    let same: Vec<String> = hashes.iter().filter(|(_, n)| n.len() > 1).map(|(_, n)| n.join(" = ")).collect();
    out(&format!("Identical outputs across cases: {}", if same.is_empty() { "none".to_string() } else { same.join("; ") }));
    let mut agree = true;
    'all: for b in batches {
        for (name, _) in &cases.0 {
            for l in labels {
                let want = output(cases.get(name).unwrap().at(&labels[0])?, "sha256")?;
                if output(b.at("cases")?.at(name)?.at(l)?, "sha256")? != want {
                    agree = false;
                    break 'all;
                }
            }
        }
    }
    out(&format!("All binaries and batches give the same output per case: {}", if agree { "True" } else { "False" }));
    for (b, path) in batches.iter().zip(results) {
        let Some(cold) = b.as_obj()?.get("cold") else { continue };
        for (name, cold) in &cold.as_obj()?.0 {
            let warm = b.at("cases")?.at(name)?;
            let mut cells = Vec::new();
            for l in labels {
                let wall = cold.at(l)?.at("wall_ms")?;
                cells.push(format!(
                    "{l} {} / p95 {} (warm {})",
                    fmt(wall.at("median")?, 2)?,
                    fmt(wall.at("p95")?, 2)?,
                    fmt(warm.at(l)?.at("wall_ms")?.at("median")?, 2)?
                ));
            }
            out(&format!("cold {} {name}: {}", py_name(path), cells.join(", ")));
        }
    }
    Ok(())
}
