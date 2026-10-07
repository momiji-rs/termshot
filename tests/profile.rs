//! TERMSHOT_PROFILE records. Renders a fixture with and without profiling,
//! then runs the reentrancy unit test, which renders four screens on twelve
//! threads at once, and checks every record it prints.
//!
//! Built and run by tests/run.sh from the repo root:
//!   profile <unit-test-binary>

use std::collections::BTreeMap;
use std::fs;
use std::process::{Command, ExitCode};

const OUT: &str = "target/test";
const FONT: &str = "third_party/jetbrains-mono/JetBrainsMono-Regular.ttf";
const LOG: &str = "tests/fixtures/random-colors.pty";
const CJK_LOG: &str = "tests/fixtures/cjk.pty";
const CJK_FONT: &str = "third_party/noto-sans-cjk/NotoSansCJKtc-Subset.otf";

type Record = BTreeMap<String, f64>;

/// Parses the flat `{"key":number,...}` objects after `termshot-profile `.
fn records(stderr: &[u8]) -> Result<Vec<Record>, String> {
    let mut out = Vec::new();
    for line in String::from_utf8_lossy(stderr).lines() {
        let Some(json) = line.strip_prefix("termshot-profile ") else { continue };
        let body = json
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
            .ok_or_else(|| format!("not an object: {line}"))?;
        let mut record = Record::new();
        for field in body.split(',') {
            let (key, value) = field.split_once(':').ok_or_else(|| format!("bad field {field:?}"))?;
            let key = key
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .ok_or_else(|| format!("bad key {key:?}"))?;
            let value: f64 = value.parse().map_err(|_| format!("bad number {value:?} for {key}"))?;
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{key} is {value}"));
            }
            if record.insert(key.to_string(), value).is_some() {
                return Err(format!("{key} appears twice"));
            }
        }
        out.push(record);
    }
    Ok(out)
}

fn get(record: &Record, key: &str) -> Result<f64, String> {
    record.get(key).copied().ok_or_else(|| format!("no {key} in {record:?}"))
}

fn ensure(ok: bool, what: String) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(what)
    }
}

fn render(png: &str, profile: bool) -> Result<Vec<u8>, String> {
    let mut command = Command::new("./termshot");
    command.args([LOG, png, FONT, "20", "40", "12"]);
    if profile {
        command.env("TERMSHOT_PROFILE", "1");
    } else {
        command.env_remove("TERMSHOT_PROFILE");
    }
    let output = command.output().map_err(|error| format!("./termshot: {error}"))?;
    ensure(output.status.success(), format!("./termshot {LOG}: {}", output.status))?;
    Ok(output.stderr)
}

fn single_render() -> Result<(), String> {
    let plain = format!("{OUT}/profile-off.png");
    let profiled = format!("{OUT}/profile-on.png");
    ensure(records(&render(&plain, false)?)?.is_empty(), "records without TERMSHOT_PROFILE".into())?;
    let found = records(&render(&profiled, true)?)?;
    ensure(found.len() == 2, format!("want 2 records, got {}", found.len()))?;
    let profile: Record = found.into_iter().flatten().collect();

    font_parts(&profile)?;
    ensure(get(&profile, "font_builtin")? == 0.0, "an explicit font counted as built in".into())?;
    let font_len = fs::metadata(FONT).map_err(|error| format!("{FONT}: {error}"))?.len() as f64;
    ensure(get(&profile, "font_bytes")? == font_len, "font_bytes is not the font's size".into())?;
    ensure(get(&profile, "fallback_bytes")? == 0.0, "fallback_bytes without a fallback".into())?;

    let png = fs::read(&profiled).map_err(|error| format!("{profiled}: {error}"))?;
    ensure(png.len() >= 24, format!("{profiled}: too short"))?;
    let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]) as f64;
    let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]) as f64;
    let input = fs::metadata(LOG).map_err(|error| format!("{LOG}: {error}"))?.len() as f64;
    ensure(get(&profile, "png_bytes")? == png.len() as f64, "png_bytes is not the file size".into())?;
    ensure(get(&profile, "input_bytes")? == input, "input_bytes is not the log size".into())?;
    ensure(get(&profile, "pixel_bytes")? == width * height * 3.0, "pixel_bytes is not width * height * 3".into())?;

    let unprofiled = fs::read(&plain).map_err(|error| format!("{plain}: {error}"))?;
    ensure(png == unprofiled, "profiling changed the PNG".into())?;
    println!("profile records match a single render");
    Ok(())
}

/// Every font's allocate/read/check/padding timers sit inside font_load_ms
/// and do not overlap.
fn font_parts(profile: &Record) -> Result<(), String> {
    let font_load = get(profile, "font_load_ms")?;
    ensure(font_load > 0.0, "font_load_ms is 0".into())?;
    let mut parts = 0.0;
    for font in ["font", "fallback"] {
        for part in ["allocate", "read", "check", "padding"] {
            parts += get(profile, &format!("{font}_{part}_ms"))?;
        }
    }
    ensure(parts <= font_load + 0.00001, format!("font parts {parts} exceed font_load_ms {font_load}"))
}

/// A run without a PNG: one record, the CLI's, with every key it has with a
/// PNG, face_ms (which a render's record carries) 0.
fn text_only() -> Result<(), String> {
    let text = format!("{OUT}/profile-text.txt");
    let output = Command::new("./termshot")
        .args(["--text", &text, LOG])
        .env("TERMSHOT_PROFILE", "1")
        .output()
        .map_err(|error| format!("./termshot: {error}"))?;
    ensure(output.status.success(), format!("./termshot --text {LOG}: {}", output.status))?;
    let found = records(&output.stderr)?;
    ensure(found.len() == 1, format!("want 1 record without a PNG, got {}", found.len()))?;
    let record = &found[0];
    for key in ["input_read_ms", "parse_ms", "font_load_ms", "font_builtin", "total_ms", "input_bytes"] {
        get(record, key)?;
    }
    ensure(get(record, "face_ms")? == 0.0, "face_ms without a render".into())?;
    println!("a text-only run reports the CLI's record, face_ms 0");
    Ok(())
}

/// The built-in font with a CFF fallback: the fallback's own timers, and
/// counters showing its glyphs were drawn.
fn fallback_render() -> Result<(), String> {
    let png = format!("{OUT}/profile-fallback.png");
    let output = Command::new("./termshot")
        .args(["--fallback-font", CJK_FONT, "--px", "20", "--size", "40x6", CJK_LOG, &png])
        .env("TERMSHOT_PROFILE", "1")
        .output()
        .map_err(|error| format!("./termshot: {error}"))?;
    ensure(output.status.success(), format!("./termshot {CJK_LOG}: {}", output.status))?;
    let found = records(&output.stderr)?;
    ensure(found.len() == 2, format!("want 2 records, got {}", found.len()))?;
    let profile: Record = found.into_iter().flatten().collect();
    font_parts(&profile)?;
    ensure(get(&profile, "font_builtin")? == 1.0, "the built-in font not counted as built in".into())?;
    ensure(get(&profile, "font_read_ms")? > 0.0, "copying the built-in font took no time".into())?;
    let font_len = fs::metadata(FONT).map_err(|error| format!("{FONT}: {error}"))?.len() as f64;
    ensure(get(&profile, "font_bytes")? == font_len, "font_bytes is not the built-in font's size".into())?;
    let cjk_len = fs::metadata(CJK_FONT).map_err(|error| format!("{CJK_FONT}: {error}"))?.len() as f64;
    ensure(get(&profile, "fallback_bytes")? == cjk_len, "fallback_bytes is not the fallback's size".into())?;
    ensure(get(&profile, "fallback_check_ms")? > 0.0, "the fallback was not checked".into())?;
    let lookups = get(&profile, "fallback_lookups")?;
    let drawn = get(&profile, "fallback_rasterizations")?;
    ensure(drawn > 0.0 && drawn <= lookups, format!("fallback drew {drawn} of {lookups} lookups"))?;
    ensure(drawn <= get(&profile, "glyph_rasterizations")?, "more fallback glyphs than glyphs".into())?;
    // 日本語 is in neither font.
    ensure(get(&profile, "glyph_missing")? >= 3.0, "the characters neither font has were not missing".into())?;
    println!("a fallback render times both fonts and draws {drawn} fallback glyphs");
    Ok(())
}

/// The reentrancy test draws four screens alone, then each again on twelve
/// threads at once, three times over (src/draw_tests.rs). Every record of a
/// concurrent render must match its screen's alone, as a render's counters
/// are its own: the glyphs it rasterized and found cached, what it missed
/// and drew from the fallback, and the PNG's size.
fn concurrent_renders(unit: &str) -> Result<(), String> {
    let output = Command::new(unit)
        .args(["--exact", "draw_tests::draw_png_is_reentrant", "--nocapture"])
        .env("TERMSHOT_PROFILE", "1")
        .output()
        .map_err(|error| format!("{unit}: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    ensure(output.status.success() && stdout.contains("1 passed"), format!("{unit}: {}\n{stdout}", output.status))?;
    let found = records(&output.stderr)?;
    ensure(found.len() == 4 + 36, format!("want 40 records, got {}", found.len()))?;
    let (alone, together) = found.split_at(4);
    for (k, record) in alone.iter().enumerate() {
        let png = format!("{OUT}/thread-alone-{k}.png");
        let png_len = fs::metadata(&png).map_err(|error| format!("{png}: {error}"))?.len() as f64;
        ensure(get(record, "png_bytes")? == png_len, format!("png_bytes is not the size of {png}"))?;
        for other in &alone[..k] {
            ensure(get(other, "pixel_bytes")? != get(record, "pixel_bytes")?, "two screens of one size".into())?;
        }
    }
    for record in together {
        let size = get(record, "pixel_bytes")?;
        let screen = alone.iter().find(|a| a.get("pixel_bytes") == Some(&size)).ok_or("a render of no screen")?;
        let parts = get(record, "png_filter_ms")? + get(record, "png_deflate_ms")? + get(record, "png_pack_ms")?;
        let encode = get(record, "png_encode_ms")?;
        ensure(parts <= encode + 0.00001, format!("png parts {parts} exceed png_encode_ms {encode}"))?;
        for key in ["png_bytes", "glyph_rasterizations", "glyph_cache_hits", "glyph_cache_evictions", "glyph_missing",
                    "fallback_lookups", "fallback_rasterizations", "geometry_cache_hits"] {
            ensure(get(record, key)? == get(screen, key)?, format!("{key} differs from the screen drawn alone"))?;
        }
    }
    println!("36 concurrent renders of 4 screens match each drawn alone, pixels and profile records");
    Ok(())
}

fn main() -> ExitCode {
    let Some(unit) = std::env::args().nth(1) else {
        eprintln!("usage: profile <unit-test-binary>");
        return ExitCode::from(2);
    };
    if let Err(error) = fs::create_dir_all(OUT) {
        eprintln!("{OUT}: {error}");
        return ExitCode::FAILURE;
    }
    match single_render().and_then(|()| text_only()).and_then(|()| fallback_render()).and_then(|()| concurrent_renders(&unit)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("FAIL {error}");
            ExitCode::FAILURE
        }
    }
}
