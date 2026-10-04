//! Instances of a variable font. Axis settings such as `wght=700`, in the
//! design units fvar gives, become the normalized coordinates (F2Dot14) that
//! cff.rs blends at. HarfBuzz is the reference, so they are worked out as
//! hb_font_set_variations works them out, in f32: each axis's setting
//! clamped to its range and scaled to -1..1 on its side of the default,
//! rounded to 16.16, mapped through avar's segment maps, then rounded to
//! 2.14. Only avar version 1 is read.

/// An axis setting: the axis tag, padded with spaces, and its value.
pub type Setting = ([u8; 4], f32);

/// An fvar axis: its tag and its range, in design units.
#[derive(Debug, PartialEq)]
pub struct Axis {
    pub tag: [u8; 4],
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

impl Axis {
    /// The range HarfBuzz clamps to, which always holds the default.
    fn range(&self) -> (f32, f32) {
        (self.default.min(self.min), self.default.max(self.max))
    }

    /// `v` clamped to the range and scaled to -1..1, 0 at the default.
    fn normalize(&self, v: f32) -> f32 {
        let (min, max) = self.range();
        let v = v.max(min).min(max);
        if v == self.default {
            0.0
        } else if v < self.default {
            (v - self.default) / (self.default - min)
        } else {
            (v - self.default) / (max - self.default)
        }
    }
}

/// An axis tag as it is written: trailing spaces dropped.
pub fn tag_name(tag: &[u8; 4]) -> String {
    String::from_utf8_lossy(tag).trim_end_matches(' ').to_string()
}

/// Parse axis settings, `TAG=VALUE` separated by commas, such as
/// `wght=700,wdth=100`. A tag is 1 to 4 ASCII letters, digits or
/// punctuation; a value is a finite number. A tag given twice takes the
/// last value, as in HarfBuzz.
pub fn parse(settings: &str) -> Result<Vec<Setting>, String> {
    let example = "give them as TAG=VALUE separated by commas, such as wght=700,wdth=100";
    settings
        .split(',')
        .map(|setting| {
            let Some((tag, value)) = setting.split_once('=') else {
                return Err(format!("{setting:?} is not an axis setting; {example}"));
            };
            if tag.is_empty() || tag.len() > 4 || !tag.bytes().all(|b| b.is_ascii_graphic()) {
                return Err(format!("{tag:?} is not an axis tag: tags are 1 to 4 printable ASCII characters other than a space, such as wght"));
            }
            let mut padded = *b"    ";
            padded[..tag.len()].copy_from_slice(tag.as_bytes());
            // HarfBuzz reads a double and keeps it as a float.
            match value.parse::<f64>() {
                Ok(v) if v.is_finite() => Ok((padded, v as f32)),
                _ => Err(format!("{setting:?}: the value of an axis is a number, such as {tag}=700")),
            }
        })
        .collect()
}

fn u16_at(d: &[u8], at: usize) -> Result<u16, String> {
    d.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]])).ok_or_else(|| format!("truncated at byte {at}"))
}

fn fixed_at(d: &[u8], at: usize) -> Result<f32, String> {
    let b = d.get(at..at + 4).ok_or_else(|| format!("truncated at byte {at}"))?;
    Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f32 / 65536.0)
}

/// The axes of an fvar table, checked as HarfBuzz checks them: version 1,
/// 20-byte axis records, and the axis and instance arrays inside the table.
/// HarfBuzz treats a table that fails as no axes at all.
pub fn axes(fvar: &[u8]) -> Result<Vec<Axis>, String> {
    let major = u16_at(fvar, 0)?;
    if major != 1 {
        return Err(format!("version {major}"));
    }
    let (at, count, axis_size) = (u16_at(fvar, 4)? as usize, u16_at(fvar, 8)? as usize, u16_at(fvar, 10)?);
    let (instances, instance_size) = (u16_at(fvar, 12)? as usize, u16_at(fvar, 14)? as usize);
    if axis_size != 20 {
        return Err(format!("axis records of {axis_size} bytes, not 20"));
    }
    if instance_size < 4 * count + 4 {
        return Err(format!("instance records of {instance_size} bytes, for {count} axes"));
    }
    if at + 20 * count + instances * instance_size > fvar.len() {
        return Err("its axes and instances run past the table".into());
    }
    (0..count)
        .map(|i| {
            let record = &fvar[at + 20 * i..][..20];
            let tag = [record[0], record[1], record[2], record[3]];
            Ok(Axis { tag, min: fixed_at(record, 4)?, default: fixed_at(record, 8)?, max: fixed_at(record, 12)? })
        })
        .collect()
}

/// An avar segment map: (from, to) pairs, in normalized coordinates.
pub type SegmentMap = Vec<(f32, f32)>;

/// The segment maps of an avar table, one per axis of `axes`.
pub fn segment_maps(avar: &[u8], axes: usize) -> Result<Vec<SegmentMap>, String> {
    let major = u16_at(avar, 0)?;
    if major != 1 {
        return Err(format!("version {major}; only version 1 is supported"));
    }
    let count = u16_at(avar, 6)? as usize;
    if count != axes {
        return Err(format!("it maps {count} axes, and fvar has {axes}"));
    }
    let f2dot14 = |at: usize| Ok::<f32, String>(u16_at(avar, at)? as i16 as f32 / 16384.0);
    let mut at = 8;
    let mut maps = Vec::with_capacity(count);
    for _ in 0..count {
        let pairs = u16_at(avar, at)? as usize;
        let map = (0..pairs).map(|k| Ok((f2dot14(at + 2 + 4 * k)?, f2dot14(at + 4 + 4 * k)?))).collect::<Result<_, String>>()?;
        maps.push(map);
        at += 2 + 4 * pairs;
    }
    Ok(maps)
}

/// `value` through an avar segment map, as HarfBuzz's SegmentMaps::map_float
/// maps it, which also gives an answer for maps OpenType calls malformed:
/// none or one pair, repeated or unsorted `from` values.
fn map(map: &[(f32, f32)], value: f32) -> f32 {
    match map {
        [] => return value,
        [(from, to)] => return value - from + to,
        _ => {}
    }
    let (mut start, mut end) = (0, map.len());
    if map[0] == (-1.0, -1.0) && map[1].0 == -1.0 {
        start += 1;
    }
    if map[end - 1] == (1.0, 1.0) && map[end - 2].0 == 1.0 {
        end -= 1;
    }
    let map = &map[start..end];
    if let Some(i) = map.iter().position(|m| m.0 == value) {
        let j = i + map[i + 1..].iter().take_while(|m| m.0 == value).count();
        // One match maps there and three map to the middle one. Otherwise
        // the last below 0, the first above it, and at 0 the one nearer 0.
        return if i == j {
            map[i].1
        } else if i + 2 == j {
            map[i + 1].1
        } else if value < 0.0 {
            map[j].1
        } else if value > 0.0 || map[i].1.abs() < map[j].1.abs() {
            map[i].1
        } else {
            map[j].1
        };
    }
    let i = map.iter().position(|m| value < m.0).unwrap_or(map.len());
    if i == 0 {
        return value - map[0].0 + map[0].1;
    }
    let before = map[i - 1];
    let Some(&after) = map.get(i) else {
        return value - before.0 + before.1;
    };
    before.1 + ((after.1 - before.1) * (value - before.0)) / (after.0 - before.0)
}

/// HarfBuzz's roundf, which is floorf(v + 0.5f): halves round up, so -2.5
/// to -2, not -3.
pub fn round(v: f32) -> f32 {
    (v + 0.5).floor()
}

/// The normalized coordinates, in F2Dot14 units, of each axis of `axes` at
/// `settings`: an axis not set is at its default. `maps` is avar's segment
/// maps, or none. An unknown tag is an error that lists the axes.
pub fn coords(axes: &[Axis], maps: &[SegmentMap], settings: &[Setting]) -> Result<Vec<i32>, String> {
    let mut design: Vec<f32> = axes.iter().map(|axis| axis.default).collect();
    for &(tag, value) in settings {
        let mut found = false;
        for (i, axis) in axes.iter().enumerate() {
            if axis.tag == tag {
                design[i] = value;
                found = true;
            }
        }
        if !found {
            return Err(format!("no axis {:?}; {}", tag_name(&tag), describe(axes)));
        }
    }
    Ok(axes
        .iter()
        .zip(design)
        .enumerate()
        .map(|(i, (axis, v))| {
            let mut c = round(axis.normalize(v) * 65536.0) as i32;
            if let Some(m) = maps.get(i) {
                c = round(map(m, c as f32 / 65536.0) * 65536.0) as i32;
            }
            (c + 2) >> 2
        })
        .collect())
}

/// The axes and their ranges, for messages.
pub fn describe(axes: &[Axis]) -> String {
    let list: Vec<String> = axes
        .iter()
        .map(|axis| {
            let (min, max) = axis.range();
            format!("{} {min} to {max}, default {}", tag_name(&axis.tag), axis.default)
        })
        .collect();
    match &list[..] {
        [] => "it has no axes".into(),
        [one] => format!("its axis is {one}"),
        _ => format!("its axes are {}", list.join("; ")),
    }
}

/// The instance `settings` pick, as `-v` shows it: each axis set, at the
/// value clamped to its range.
pub fn instance(axes: &[Axis], settings: &[Setting]) -> String {
    let parts: Vec<String> = axes
        .iter()
        .filter_map(|axis| {
            let &(_, v) = settings.iter().rev().find(|(tag, _)| *tag == axis.tag)?;
            let (min, max) = axis.range();
            let clamped = v.max(min).min(max);
            let note = if clamped == v { String::new() } else { format!(" ({v} clamped)") };
            Some(format!("{}={clamped}{note}", tag_name(&axis.tag)))
        })
        .collect();
    parts.join(",")
}
