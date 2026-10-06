//! The colour palette (#87): the default foreground and background and the
//! 16 named colours a log's SGR codes pick, and the --palette file that sets
//! them.
//!
//! A palette is applied as the log is replayed, not when it is drawn: a cell
//! keeps the colour its character was printed in, so --json reports the
//! colours under the palette, and every later comparison with "the default
//! background" (kitty's images below the backgrounds, the cursor's colour,
//! the blank cells --json leaves out) is with this palette's. Colours 16 to
//! 255 (the 6x6x6 cube and the greys) and 24-bit colours keep xterm's values,
//! as they do in the terminals whose palettes this copies.
//!
//! The file uses kitty's colour keys and syntax (kitty.conf, a theme's .conf):
//! a key and a #rrggbb colour on each line, `foreground`, `background` and
//! `color0` to `color15`; blank lines and lines starting with # are skipped.
//! Every other line is an error, with its line number, so that a typo is not
//! ignored: a kitty theme's other keys (cursor, selection_background,
//! color16 and up, ...) must be taken out first.

/// A colour as (red, green, blue).
pub type Rgb = (u8, u8, u8);

/// The colours a log's default and named colours stand for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// SGR 39's colour, and the cursor's.
    pub foreground: Rgb,
    /// SGR 49's colour, the screen's before anything is printed, and the
    /// padding's.
    pub background: Rgb,
    /// SGR 30-37 and 90-97 (and 40-47, 100-107; 38;5;n and 48;5;n for n below 16).
    pub named: [Rgb; 16],
}

/// The largest --palette file: 18 lines need under 400 bytes, and a theme's
/// comments some more.
pub const MAX_FILE_BYTES: usize = 64 << 10;

impl Palette {
    /// termshot's own default colours, with xterm's 16 named colours.
    pub const DEFAULT: Palette = Palette {
        foreground: (219, 231, 247),
        background: (17, 24, 35),
        named: [
            (0, 0, 0), (205, 0, 0), (0, 205, 0), (205, 205, 0),
            (0, 0, 238), (205, 0, 205), (0, 205, 205), (229, 229, 229),
            (127, 127, 127), (255, 0, 0), (0, 255, 0), (255, 255, 0),
            (92, 92, 255), (255, 0, 255), (0, 255, 255), (255, 255, 255),
        ],
    };

    /// Colour n of the 256: a named colour from this palette, else xterm's
    /// 6x6x6 cube and 24 greys. None past 255.
    pub fn color(&self, n: u32) -> Option<Rgb> {
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        match n {
            0..=15 => Some(self.named[n as usize]),
            16..=231 => {
                let i = (n - 16) as usize;
                Some((LEVELS[i / 36], LEVELS[i / 6 % 6], LEVELS[i % 6]))
            }
            232..=255 => {
                let v = (8 + 10 * (n - 232)) as u8;
                Some((v, v, v))
            }
            _ => None,
        }
    }

    /// This palette with the colours a --palette file sets (see the module
    /// documentation), or why the file is not one: the line, and what is
    /// wrong with it. A key given twice is an error too.
    pub fn with_file(mut self, file: &[u8]) -> Result<Palette, String> {
        if file.len() > MAX_FILE_BYTES {
            return Err(format!("over {MAX_FILE_BYTES} bytes, which no palette needs"));
        }
        let text = std::str::from_utf8(file).map_err(|e| {
            let at = e.valid_up_to();
            let line = 1 + file[..at].iter().filter(|&&b| b == b'\n').count();
            format!("line {line}: not UTF-8 text (at byte {at} of the file)")
        })?;
        // foreground, background, then color0 to color15.
        let mut seen = [0usize; 18];
        for (n, line) in text.split('\n').enumerate() {
            let n = n + 1;
            let line = line.strip_suffix('\r').unwrap_or(line).trim_matches(is_blank);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once(is_blank) else {
                return Err(format!("line {n}: {line:?} has no colour; write a key and #rrggbb, as in \"color1 #cd0000\""));
            };
            let value = value.trim_start_matches(is_blank);
            let slot = match key {
                "foreground" => 0,
                "background" => 1,
                _ => match key.strip_prefix("color").and_then(|i| i.parse::<u32>().ok()).filter(|i| key == format!("color{i}")) {
                    Some(i @ 0..=15) => 2 + i as usize,
                    Some(16..=255) => {
                        return Err(format!(
                            "line {n}: {key} is not one of the 16 named colours; colours 16 to 255 keep xterm's values"
                        ));
                    }
                    _ => {
                        return Err(format!(
                            "line {n}: unknown key {key:?}; a palette sets foreground, background and color0 to color15"
                        ));
                    }
                },
            };
            if seen[slot] != 0 {
                return Err(format!("line {n}: {key} is already set, on line {}", seen[slot]));
            }
            seen[slot] = n;
            let color = parse_color(value).map_err(|why| format!("line {n}: {key}: {why}"))?;
            match slot {
                0 => self.foreground = color,
                1 => self.background = color,
                i => self.named[i - 2] = color,
            }
        }
        Ok(self)
    }

    /// The palette as a --palette file sets it, every key once.
    #[cfg(test)]
    pub fn to_file(&self) -> String {
        use std::fmt::Write as _;
        let hex = |(r, g, b): Rgb| format!("#{r:02x}{g:02x}{b:02x}");
        let mut file = format!("foreground {}\nbackground {}\n", hex(self.foreground), hex(self.background));
        for (i, &color) in self.named.iter().enumerate() {
            let _ = writeln!(file, "color{i} {}", hex(color));
        }
        file
    }
}

/// What separates a file's keys from their colours, and is trimmed.
fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// A colour written #rrggbb (either case), as --fg, --bg and the file take it.
pub fn parse_color(value: &str) -> Result<Rgb, String> {
    let digits = value.strip_prefix('#').filter(|d| d.len() == 6 && d.bytes().all(|b| b.is_ascii_hexdigit()));
    let Some(d) = digits else {
        return Err(format!("a colour must look like #1e2a3b, not {value:?}"));
    };
    let byte = |i: usize| u8::from_str_radix(&d[i..i + 2], 16).unwrap_or(0);
    Ok((byte(0), byte(2), byte(4)))
}
