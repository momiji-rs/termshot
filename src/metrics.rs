//! The metrics of a variable face at an instance, as HarfBuzz varies them:
//! each glyph's advance by HVAR, and hhea's ascender, descender and line
//! gap by MVAR's hasc, hdsc and hlgp. They are read only when an instance
//! is chosen, so the default instance draws as it always has.
//!
//! HarfBuzz ignores a table that fails its checks; these refuse it with a
//! reason, as font.rs refuses any table it can't trust. Out of range
//! indexes that HarfBuzz reads as no delta, read as no delta here too, as
//! fonts rely on it (an identity advance map past the store's items).

use crate::cff::region_scalar;
use crate::variations::round;

fn u16_at(d: &[u8], at: usize) -> Result<usize, String> {
    match d.get(at..at.wrapping_add(2)) {
        Some(b) => Ok(u16::from_be_bytes([b[0], b[1]]) as usize),
        None => Err(format!("truncated at byte {at}")),
    }
}

fn u32_at(d: &[u8], at: usize) -> Result<usize, String> {
    match d.get(at..at.wrapping_add(4)) {
        Some(b) => Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize),
        None => Err(format!("truncated at byte {at}")),
    }
}

/// One ItemVariationData, located and checked.
struct Data {
    items: usize,
    /// Where its rows of deltas start, and how many bytes each takes.
    rows: usize,
    row: usize,
    /// The first `words` deltas of a row are 32-bit if `long`, else 16;
    /// the rest are 16-bit if `long`, else 8.
    words: usize,
    long: bool,
    /// The scalar of each region it names, at the instance.
    scalars: Vec<f32>,
}

/// An ItemVariationStore at an instance: the deltas of each item.
pub struct Store<'a> {
    store: &'a [u8],
    /// Which of `sets` each ItemVariationData offset names: one copy of
    /// each however many offsets name it.
    data: Vec<usize>,
    sets: Vec<Data>,
}

impl<'a> Store<'a> {
    /// Read the store that starts `store`, for the instance at `coords`
    /// (normalized, F2Dot14, one per fvar axis), whose axis count the
    /// region list must have. Each region is evaluated once and each
    /// ItemVariationData checked once, so this costs at most the store's
    /// size; sizes are worked out in u64, which the fields can't overflow.
    pub fn parse(store: &'a [u8], coords: &[i32]) -> Result<Store<'a>, String> {
        let fits = |end: u64| end <= store.len() as u64;
        let format = u16_at(store, 0)?;
        if format != 1 {
            return Err(format!("format {format}"));
        }
        let (list, count) = (u32_at(store, 2)?, u16_at(store, 6)?);
        let (axes, regions) = (u16_at(store, list)?, u16_at(store, list + 2)?);
        // HarfBuzz reads an axis the coordinates lack as 0 and drops one
        // past the region list's, so a mismatch would draw something else
        // quietly.
        if axes != coords.len() {
            return Err(format!("the region list's axis count is {axes} and fvar's {}", coords.len()));
        }
        // The top bit is reserved, and HarfBuzz drops it.
        if regions & 0x8000 != 0 {
            return Err(format!("the region count {regions} sets the reserved top bit"));
        }
        if !fits(list as u64 + 4 + regions as u64 * axes as u64 * 6) {
            return Err("the region list runs past the store".into());
        }
        let region_scalars: Vec<f32> =
            (0..regions).map(|r| region_scalar(&store[list + 4 + 6 * axes * r..][..6 * axes], coords)).collect();
        let mut out = Store { store, data: Vec::with_capacity(count), sets: Vec::new() };
        let mut checked = std::collections::HashMap::new();
        for i in 0..count {
            let at = u32_at(store, 8 + 4 * i)?;
            if let Some(&slot) = checked.get(&at) {
                out.data.push(slot);
                continue;
            }
            // A null offset is an ItemVariationData with no items.
            if at == 0 {
                checked.insert(at, out.sets.len());
                out.data.push(out.sets.len());
                out.sets.push(Data { items: 0, rows: 0, row: 0, words: 0, long: false, scalars: Vec::new() });
                continue;
            }
            let (items, words, n) = (u16_at(store, at)?, u16_at(store, at + 2)?, u16_at(store, at + 4)?);
            // The high bit of the word count makes words 32 bits and the rest 16.
            let (long, words) = (words & 0x8000 != 0, words & 0x7fff);
            if words > n {
                return Err(format!("ItemVariationData {i} has {words} word deltas of {n}"));
            }
            let rows = at as u64 + 6 + 2 * n as u64;
            let row = (words + n) as u64 * if long { 2 } else { 1 };
            if !fits(rows + items as u64 * row) {
                return Err(format!("ItemVariationData {i} runs past the store"));
            }
            let scalars = (0..n)
                .map(|j| match u16_at(store, at + 6 + 2 * j)? {
                    region if region < regions => Ok(region_scalars[region]),
                    region => Err(format!("ItemVariationData {i} names region {region} of {regions}")),
                })
                .collect::<Result<_, String>>()?;
            checked.insert(at, out.sets.len());
            out.data.push(out.sets.len());
            out.sets.push(Data { items, rows: rows as usize, row: row as usize, words, long, scalars });
        }
        Ok(out)
    }

    /// The delta of the item at `index` (outer << 16 | inner), summed in
    /// f32 in the row's order, as HarfBuzz sums it; 0 for an item the store
    /// lacks.
    pub fn delta(&self, index: u32) -> f32 {
        let (outer, inner) = ((index >> 16) as usize, (index & 0xffff) as usize);
        let Some(&slot) = self.data.get(outer) else { return 0.0 };
        let data = &self.sets[slot];
        if inner >= data.items {
            return 0.0;
        }
        let row = &self.store[data.rows + inner * data.row..][..data.row];
        // The wide deltas, then the narrow ones.
        let split = if data.long { 4 * data.words } else { 2 * data.words };
        let mut delta = 0.0f32;
        for (i, &scalar) in data.scalars.iter().enumerate() {
            if scalar == 0.0 {
                continue;
            }
            let value = match (data.long, i < data.words) {
                (true, true) => i32::from_be_bytes([row[4 * i], row[4 * i + 1], row[4 * i + 2], row[4 * i + 3]]),
                (false, true) => i16::from_be_bytes([row[2 * i], row[2 * i + 1]]) as i32,
                (true, false) => {
                    let at = split + 2 * (i - data.words);
                    i16::from_be_bytes([row[at], row[at + 1]]) as i32
                }
                (false, false) => row[split + i - data.words] as i8 as i32,
            };
            delta += scalar * value as f32;
        }
        delta
    }
}

/// A DeltaSetIndexMap: from a glyph to the index of its delta.
struct Map<'a> {
    entries: &'a [u8],
    count: usize,
    width: usize,
    inner_bits: u32,
}

impl<'a> Map<'a> {
    /// Read the map at `at` in `table`, checked as HarfBuzz checks it.
    fn parse(table: &'a [u8], at: usize) -> Result<Map<'a>, String> {
        let header = table.get(at..at.wrapping_add(2)).ok_or_else(|| format!("truncated at byte {at}"))?;
        let (format, entry) = (header[0], header[1] as usize);
        let (count, start) = match format {
            0 => (u16_at(table, at + 2)?, at + 4),
            1 => (u32_at(table, at + 2)?, at + 6),
            _ => return Err(format!("a DeltaSetIndexMap of format {format}")),
        };
        let width = ((entry >> 4) & 3) + 1;
        let entries = table
            .get(start..)
            .and_then(|rest| rest.get(..(count as u64 * width as u64).try_into().ok()?))
            .ok_or("a DeltaSetIndexMap runs past the table")?;
        Ok(Map { entries, count, width, inner_bits: (entry & 0xf) as u32 + 1 })
    }

    /// The delta index of `v`: itself for an empty map, the last entry's
    /// past the end.
    fn map(&self, v: u32) -> u32 {
        if self.count == 0 {
            return v;
        }
        let at = self.width * (v as usize).min(self.count - 1);
        let u = self.entries[at..at + self.width].iter().fold(0u32, |u, &b| u << 8 | b as u32);
        (u >> self.inner_bits) << 16 | (u & ((1 << self.inner_bits) - 1))
    }
}

/// HVAR at an instance: the advance of each glyph.
pub struct Advances<'a> {
    store: Option<Store<'a>>,
    /// The advance map; None maps each glyph to itself.
    map: Option<Map<'a>>,
}

impl<'a> Advances<'a> {
    /// Read HVAR for the instance at `coords`. Its side-bearing maps are
    /// unused but checked, as HarfBuzz ignores the table if they fail.
    pub fn parse(hvar: &'a [u8], coords: &[i32]) -> Result<Advances<'a>, String> {
        let major = u16_at(hvar, 0)?;
        if major != 1 {
            return Err(format!("version {major}"));
        }
        let store = match u32_at(hvar, 4)? {
            0 => None,
            at => Some(Store::parse(hvar.get(at..).ok_or("the store starts past the table")?, coords)?),
        };
        let mut maps = [8, 12, 16].iter().map(|&field| match u32_at(hvar, field)? {
            0 => Ok(None),
            at => Map::parse(hvar, at).map(Some),
        });
        let map = maps.next().unwrap()?;
        maps.try_for_each(|side| side.map(drop))?;
        Ok(Advances { store, map })
    }

    /// The advance of `glyph`, whose hmtx advance is `advance`: that plus
    /// its delta rounded, and at least 0, as HarfBuzz works it out.
    pub fn advance(&self, glyph: u32, advance: i32) -> i32 {
        let Some(store) = &self.store else { return advance };
        let index = self.map.as_ref().map_or(glyph, |map| map.map(glyph));
        (advance as f32 + round(store.delta(index))).max(0.0) as i32
    }
}

/// hhea's ascender, descender and line gap at the instance at `coords`,
/// varied by MVAR's hasc, hdsc and hlgp. As HarfBuzz does, the ascender
/// is made positive and the descender negative, then each is rounded to
/// whole units, each saturated at i32's bounds (draw.c's cell arithmetic
/// takes any). Refused if of no height.
pub fn vertical(hhea: &[u8], mvar: &[u8], coords: &[i32]) -> Result<[i32; 3], String> {
    let major = u16_at(mvar, 0)?;
    if major != 1 {
        return Err(format!("version {major}"));
    }
    let (size, count, at) = (u16_at(mvar, 6)?, u16_at(mvar, 8)?, u16_at(mvar, 10)?);
    if size < 8 {
        return Err(format!("value records of {size} bytes, under 8"));
    }
    let store = match at {
        0 => None,
        at => Some(Store::parse(mvar.get(at..).ok_or("the store starts past the table")?, coords)?),
    };
    let records = mvar.get(12..12 + size * count).ok_or("its value records run past the table")?;
    let records: Vec<&[u8]> = records.chunks_exact(size).collect();
    // HarfBuzz finds a tag by binary search, which would miss one out of order.
    if records.windows(2).any(|pair| pair[0][..4] >= pair[1][..4]) {
        return Err("its value records are not in tag order".into());
    }
    let delta = |tag: &[u8; 4]| match (&store, records.iter().find(|record| &record[..4] == tag)) {
        (Some(store), Some(record)) => store.delta(u32::from_be_bytes([record[4], record[5], record[6], record[7]])),
        _ => 0.0,
    };
    let hhea_at = |at| u16_at(hhea, at).map(|v| v as i16 as f32);
    let ascender = round((hhea_at(4)? + delta(b"hasc")).abs()) as i32;
    let descender = round(-(hhea_at(6)? + delta(b"hdsc")).abs()) as i32;
    let line_gap = round(hhea_at(8)? + delta(b"hlgp")) as i32;
    // As font::check refuses it in hhea: draw.c scales a face by its height.
    if ascender <= descender {
        return Err(format!("at this instance the ascender {ascender} is not above the descender {descender}"));
    }
    Ok([ascender, descender, line_gap])
}
