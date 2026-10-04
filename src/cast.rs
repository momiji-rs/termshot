//! asciinema recordings (asciicast v2 and v3, `.cast`) as input. A cast is
//! newline-delimited JSON: a header object, then one event per line. termshot
//! concatenates the data of the output (`"o"`) events, in file order, and
//! replays those bytes as it would a raw PTY log. The formats:
//! <https://docs.asciinema.org/manual/asciicast/v2/> and
//! <https://docs.asciinema.org/manual/asciicast/v3/>.
//!
//! - v2: the header has `width` and `height`; an event is
//!   `[time, code, data]`, with time in seconds since the start.
//! - v3: the header has `term.cols` and `term.rows`; an event is
//!   `[interval, code, data]`, with the interval since the previous event,
//!   and a line starting with `#` is a comment.
//!
//! A resize (`"r"`, data `"COLSxROWS"`) does not resize the grid mid-replay:
//! the screen model has one size. The last resize gives the size the
//! recording ends at, which is the size used when no `--size` is given, and
//! every output event is replayed on that grid. Full-screen programs redraw
//! on SIGWINCH, so their final frame is right; text written before a resize
//! may wrap where the terminal of the time would not have. Input (`"i"`),
//! marker (`"m"`), exit (`"x"`) and unknown events are ignored.
//!
//! The JSON reader is strict (RFC 8259): UTF-8 only, every escape including
//! `\uXXXX` and surrogate pairs (a lone surrogate is refused, as it has no
//! UTF-8 form), no duplicate keys, no trailing commas, and nesting at most
//! MAX_DEPTH deep. Times must be finite and not negative, and sizes whole
//! numbers that fit in 64 bits. Anything else is refused with its line (and
//! for bad JSON or UTF-8, its column), never guessed at.

/// How deep arrays and objects may nest. A v3 header needs 3 (term.theme).
pub const MAX_DEPTH: usize = 16;

/// A JSON value. Numbers keep their text, checked against the grammar, so
/// a size is read as an integer and never through a float.
#[derive(Debug, PartialEq)]
pub enum Value<'a> {
    Null,
    Bool(bool),
    Number(&'a str),
    String(String),
    Array(Vec<Value<'a>>),
    Object(Vec<(String, Value<'a>)>),
}

impl<'a> Value<'a> {
    fn get(&self, key: &str) -> Option<&Value<'a>> {
        match self {
            Value::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "a boolean",
            Value::Number(_) => "a number",
            Value::String(_) => "a string",
            Value::Array(_) => "an array",
            Value::Object(_) => "an object",
        }
    }
}

/// Where a JSON text went wrong: a byte offset into it, and why.
#[derive(Debug, PartialEq)]
pub struct JsonError {
    pub at: usize,
    pub reason: String,
}

struct Parser<'a> {
    text: &'a str,
    i: usize,
}

impl<'a> Parser<'a> {
    fn bytes(&self) -> &'a [u8] {
        self.text.as_bytes()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes().get(self.i).copied()
    }

    fn error<T>(&self, reason: impl Into<String>) -> Result<T, JsonError> {
        Err(JsonError { at: self.i, reason: reason.into() })
    }

    fn skip_space(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.peek() {
            self.i += 1;
        }
    }

    fn expect(&mut self, byte: u8, what: &str) -> Result<(), JsonError> {
        if self.peek() == Some(byte) {
            self.i += 1;
            Ok(())
        } else {
            self.unexpected(what)
        }
    }

    fn unexpected<T>(&self, wanted: &str) -> Result<T, JsonError> {
        match self.text[self.i..].chars().next() {
            None => self.error(format!("the line ends where {wanted} was expected")),
            Some(c) => self.error(format!("expected {wanted}, found {:?}", c)),
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value<'a>, JsonError> {
        self.skip_space();
        match self.peek() {
            Some(b'{') => self.object(depth + 1),
            Some(b'[') => self.array(depth + 1),
            Some(b'"') => self.string().map(Value::String),
            Some(b'-' | b'0'..=b'9') => self.number().map(Value::Number),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            _ => self.unexpected("a JSON value"),
        }
    }

    fn literal(&mut self, word: &str, value: Value<'a>) -> Result<Value<'a>, JsonError> {
        if self.bytes()[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(value)
        } else {
            self.unexpected("a JSON value")
        }
    }

    fn nest(&self, depth: usize) -> Result<(), JsonError> {
        if depth > MAX_DEPTH {
            return self.error(format!("arrays and objects nest more than {MAX_DEPTH} deep"));
        }
        Ok(())
    }

    fn array(&mut self, depth: usize) -> Result<Value<'a>, JsonError> {
        self.nest(depth)?;
        self.i += 1;
        let mut items = Vec::new();
        self.skip_space();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value(depth)?);
            self.skip_space();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                _ => return self.unexpected("',' or ']'"),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value<'a>, JsonError> {
        self.nest(depth)?;
        self.i += 1;
        let mut members: Vec<(String, Value<'a>)> = Vec::new();
        // Where each key starts, to report a duplicate.
        let mut starts = Vec::new();
        self.skip_space();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.skip_space();
            if self.peek() != Some(b'"') {
                return self.unexpected("a string key");
            }
            starts.push(self.i);
            let key = self.string()?;
            self.skip_space();
            self.expect(b':', "':'")?;
            let value = self.value(depth)?;
            members.push((key, value));
            self.skip_space();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    break;
                }
                _ => return self.unexpected("',' or '}'"),
            }
        }
        // Sorted, so many keys cost n log n rather than n squared. The sort
        // is stable: of two equal keys, the second in the file is reported.
        let mut order: Vec<usize> = (0..members.len()).collect();
        order.sort_by(|&a, &b| members[a].0.cmp(&members[b].0));
        if let Some(pair) = order.windows(2).find(|pair| members[pair[0]].0 == members[pair[1]].0) {
            let reason = format!("the key {:?} appears twice", members[pair[1]].0);
            return Err(JsonError { at: starts[pair[1]], reason });
        }
        Ok(Value::Object(members))
    }

    /// Four hex digits after `\u`.
    fn hex4(&mut self) -> Result<u32, JsonError> {
        let digits = self.bytes().get(self.i..self.i + 4).filter(|d| d.iter().all(u8::is_ascii_hexdigit));
        let Some(digits) = digits else {
            return self.error("\\u needs four hex digits");
        };
        let value = digits.iter().fold(0, |n, &d| n * 16 + (d as char).to_digit(16).unwrap_or(0));
        self.i += 4;
        Ok(value)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.i += 1;
        let mut out = String::new();
        loop {
            // Copy the run up to the next quote, backslash or control.
            let start = self.i;
            while let Some(b) = self.peek() {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.i += 1;
            }
            // Those three are ASCII, so the run ends on a char boundary.
            out.push_str(&self.text[start..self.i]);
            match self.peek() {
                None => return self.error("the line ends inside a string"),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let escape = self.peek();
                    self.i += 1;
                    let c = match escape {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => self.unicode_escape()?,
                        _ => {
                            self.i -= 1;
                            return self.unexpected("an escape (\\\" \\\\ \\/ \\b \\f \\n \\r \\t \\uXXXX)");
                        }
                    };
                    out.push(c);
                }
                Some(_) => return self.error("a control character must be escaped in a string"),
            }
        }
    }

    /// The character of a `\uXXXX`, or of a surrogate pair of them.
    fn unicode_escape(&mut self) -> Result<char, JsonError> {
        let at = self.i - 2;
        let lone = |at| Err(JsonError { at, reason: "a lone UTF-16 surrogate has no UTF-8 form".into() });
        let first = self.hex4()?;
        let code = match first {
            0xD800..=0xDBFF => {
                if !self.bytes()[self.i..].starts_with(b"\\u") {
                    return lone(at);
                }
                self.i += 2;
                let second = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return lone(at);
                }
                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
            }
            0xDC00..=0xDFFF => return lone(at),
            _ => first,
        };
        char::from_u32(code).map_or_else(|| lone(at), Ok)
    }

    /// A number, checked against JSON's grammar: -?(0|[1-9]d*)(.d+)?([eE][+-]?d+)?
    fn number(&mut self) -> Result<&'a str, JsonError> {
        let start = self.i;
        let digits = |p: &mut Self| {
            let from = p.i;
            while let Some(b'0'..=b'9') = p.peek() {
                p.i += 1;
            }
            p.i - from
        };
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.i += 1;
                if let Some(b'0'..=b'9') = self.peek() {
                    return self.error("a number can't have a leading zero");
                }
            }
            Some(b'1'..=b'9') => {
                digits(self);
            }
            _ => return self.unexpected("a digit"),
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            if digits(self) == 0 {
                return self.unexpected("a digit after '.'");
            }
        }
        if let Some(b'e' | b'E') = self.peek() {
            self.i += 1;
            if let Some(b'+' | b'-') = self.peek() {
                self.i += 1;
            }
            if digits(self) == 0 {
                return self.unexpected("a digit in the exponent");
            }
        }
        Ok(&self.text[start..self.i])
    }
}

/// One JSON text, with nothing after it but whitespace.
pub fn parse(text: &str) -> Result<Value<'_>, JsonError> {
    let mut parser = Parser { text, i: 0 };
    let value = parser.value(0)?;
    parser.skip_space();
    if parser.i < text.len() {
        return parser.unexpected("the end of the line");
    }
    Ok(value)
}

/// What a cast holds for termshot.
#[derive(Debug, PartialEq)]
pub struct Cast {
    pub version: u8,
    /// The header's terminal size, as (cols, rows).
    pub size: (u64, u64),
    /// The size the recording ends at: the last resize event's, or the header's.
    pub final_size: (u64, u64),
    /// Whether final_size comes from a resize event.
    pub resized: bool,
    /// The output events' data, concatenated in file order.
    pub output: Vec<u8>,
}

/// A line as UTF-8, or why not. Columns count characters, as an editor
/// shows them.
fn line_text(n: usize, line: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(line).map_err(|e| {
        let valid = std::str::from_utf8(&line[..e.valid_up_to()]).unwrap_or_default();
        format!("line {n}, column {}: not UTF-8", valid.chars().count() + 1)
    })
}

fn json_error(n: usize, line: &str, error: JsonError) -> String {
    let column = line[..error.at.min(line.len())].chars().count() + 1;
    format!("line {n}, column {column}: {}", error.reason)
}

/// Whether the input is read as a cast: its first line, from its first
/// byte, is a JSON object with a "version" member. A raw PTY log starts
/// with terminal output, which is rarely such a line; --raw is for when it is.
///
/// This only finds the member: it scans the object's top level without
/// checking the rest, so a header that is otherwise malformed (too deep,
/// a duplicate key, cut short) is still taken for one, and decode says
/// what is wrong with it instead of it being drawn as text.
pub fn detect(data: &[u8]) -> bool {
    // The first byte before the line's end: a raw log can go megabytes
    // without a LF (a full-screen program's redraws), and finding it cost
    // more than a millisecond on the 4.7 MB ANSI replay (#21).
    if data.first() != Some(&b'{') {
        return false;
    }
    let first = data.split(|&b| b == b'\n').next().unwrap_or_default();
    // Iterative, so no nesting can overflow the stack.
    let (mut depth, mut i, mut key_next) = (0usize, 0, false);
    while i < first.len() {
        match first[i] {
            b'"' => {
                let start = i;
                i += 1;
                while i < first.len() && first[i] != b'"' {
                    i += if first[i] == b'\\' { 2 } else { 1 };
                }
                let Some(string) = first.get(start..=i) else { return false };
                // A key may spell itself with escapes; decode it to compare.
                let is_version = string == b"\"version\""
                    || std::str::from_utf8(string).ok().map(parse) == Some(Ok(Value::String("version".into())));
                if key_next && is_version {
                    return true;
                }
                key_next = false;
            }
            b'{' | b'[' => {
                depth += 1;
                key_next = depth == 1;
            }
            b'}' | b']' => {
                depth -= 1;
                // The header object closed without the member.
                if depth == 0 {
                    return false;
                }
            }
            b',' => key_next = depth == 1,
            _ => {}
        }
        i += 1;
    }
    false
}

/// A whole number that fits in 64 bits, from a JSON value.
fn whole(value: Option<&Value>, what: &str) -> Result<u64, String> {
    match value {
        None => Err(format!("the header has no {what}")),
        Some(Value::Number(n)) if n.bytes().all(|b| b.is_ascii_digit()) => {
            n.parse().map_err(|_| format!("{what} {n} is too large"))
        }
        Some(Value::Number(n)) => Err(format!("{what} must be a whole number, not {n}")),
        Some(other) => Err(format!("{what} must be a whole number, not {}", other.kind())),
    }
}

/// "COLSxROWS", as a resize event gives it.
fn resize(data: &Value) -> Result<(u64, u64), String> {
    let bad = || format!("a resize event's data must look like \"80x24\", not {data:?}");
    let Value::String(text) = data else { return Err(bad()) };
    let (cols, rows) = text.split_once('x').ok_or_else(bad)?;
    let number = |n: &str| {
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        n.parse::<u64>().map_err(|_| format!("the resize {text:?} is too large"))
    };
    Ok((number(cols)?, number(rows)?))
}

fn header(n: usize, text: &str) -> Result<(u8, (u64, u64)), String> {
    let at = |reason: String| format!("line {n}: {reason}");
    let header = parse(text).map_err(|e| format!("{} (the first line must be the asciicast header)", json_error(n, text, e)))?;
    if !matches!(header, Value::Object(_)) {
        return Err(at(format!("the asciicast header must be a JSON object, not {}", header.kind())));
    }
    let version = match header.get("version") {
        None => return Err(at("the asciicast header has no version".into())),
        Some(Value::Number("2")) => 2,
        Some(Value::Number("3")) => 3,
        Some(Value::Number(v)) => {
            return Err(at(format!("asciicast version {v} is not supported; termshot reads versions 2 and 3")))
        }
        Some(other) => return Err(at(format!("the version must be a number, not {}", other.kind()))),
    };
    let size = if version == 2 {
        (whole(header.get("width"), "width").map_err(at)?, whole(header.get("height"), "height").map_err(at)?)
    } else {
        let term = match header.get("term") {
            Some(term @ Value::Object(_)) => term,
            None => return Err(at("the header has no term".into())),
            Some(other) => return Err(at(format!("term must be an object, not {}", other.kind()))),
        };
        (whole(term.get("cols"), "term.cols").map_err(at)?, whole(term.get("rows"), "term.rows").map_err(at)?)
    };
    Ok((version, size))
}

/// What an event line does.
enum Event {
    Skip,
    Output(String),
    Resize((u64, u64)),
}

/// Whether a JSON number is below zero. Its text decides, not its value as
/// a float: -1e-9999 rounds to -0.0, which is not below 0.
fn negative(number: &str) -> bool {
    let significand = number.split(['e', 'E']).next().unwrap_or_default();
    number.starts_with('-') && significand.bytes().any(|b| matches!(b, b'1'..=b'9'))
}

fn event(n: usize, version: u8, text: &str) -> Result<Event, String> {
    // Only v3 has comments; a blank line is no event, and is refused below.
    if version == 3 && text.starts_with('#') {
        return Ok(Event::Skip);
    }
    let at = |reason: String| format!("line {n}: {reason}");
    let mut fields = match parse(text).map_err(|e| json_error(n, text, e))? {
        Value::Array(fields) => fields,
        other => return Err(at(format!("an event must be an array [time, code, data], not {}", other.kind()))),
    };
    if fields.len() != 3 {
        return Err(at(format!("an event must be [time, code, data], not {} items", fields.len())));
    }
    let data = fields.pop().unwrap_or(Value::Null);
    let time_ok = match &fields[0] {
        Value::Number(t) => !negative(t) && t.parse::<f64>().map_or(false, f64::is_finite),
        _ => false,
    };
    if !time_ok {
        let what = if version == 2 { "time" } else { "interval" };
        return Err(at(format!("an event's {what} must be a finite number of seconds, not below 0")));
    }
    match &fields[1] {
        Value::String(code) if code == "o" => match data {
            Value::String(output) => Ok(Event::Output(output)),
            other => Err(at(format!("an output event's data must be a string, not {}", other.kind()))),
        },
        Value::String(code) if code == "r" => resize(&data).map(Event::Resize).map_err(at),
        Value::String(_) => Ok(Event::Skip),
        other => Err(at(format!("an event's code must be a string, not {}", other.kind()))),
    }
}

/// Read a cast: check every line, and collect the output. The output is
/// written over the recording as it is read, so a long one needs no second
/// buffer. An event's data is never longer than the JSON it is decoded from,
/// so it lands before the end of the line it came from, which is read.
pub fn decode(mut data: Vec<u8>) -> Result<Cast, String> {
    let mut cast: Option<Cast> = None;
    let (mut start, mut n, mut written) = (0, 0, 0);
    // The lines, numbered from 1, without their LF. What follows the last LF
    // is a line too, unless the file ends in an LF: nothing follows it then.
    while start <= data.len() {
        if start == data.len() && start > 0 && data[start - 1] == b'\n' {
            break;
        }
        let end = data[start..].iter().position(|&b| b == b'\n').map_or(data.len(), |p| start + p);
        n += 1;
        let text = line_text(n, &data[start..end])?;
        let output = match &mut cast {
            None => {
                let (version, size) = header(n, text)?;
                cast = Some(Cast { version, size, final_size: size, resized: false, output: Vec::new() });
                None
            }
            Some(cast) => match event(n, cast.version, text)? {
                Event::Skip => None,
                Event::Output(output) => Some(output),
                Event::Resize(size) => {
                    cast.final_size = size;
                    cast.resized = true;
                    None
                }
            },
        };
        if let Some(output) = output {
            data[written..written + output.len()].copy_from_slice(output.as_bytes());
            written += output.len();
        }
        start = end + 1;
    }
    let mut cast = cast.ok_or("line 1: the asciicast header is missing")?;
    data.truncate(written);
    cast.output = data;
    Ok(cast)
}
