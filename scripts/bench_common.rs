//! What scripts/bench.rs and scripts/bench-report.rs share: JSON as Python's
//! json module reads and writes it, numbers and statistics as Python's
//! statistics module computes them, Python's random.Random, SHA-256 and
//! base64, and an argument parser that answers as argparse does.
//!
//! The two programs replaced Python scripts whose results (the JSON in
//! docs/) are compared across rounds, so each piece reproduces the Python
//! exactly: the same workload bytes, the same shuffled order, the same
//! bootstrap, the same digits.
#![allow(dead_code)]

use std::fmt::Write as _;

// ---------------------------------------------------------------- JSON

/// A JSON value as Python's json.loads makes it: integers stay integers
/// (Python's int; i128 is far more than any report holds), numbers with a
/// fraction or an exponent are floats, and objects keep their keys in
/// insertion order (a repeated key keeps its first place and its last value).
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Map),
}

/// An insertion-ordered object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Map(pub Vec<(String, Json)>);

impl Map {
    pub fn new() -> Map {
        Map(Vec::new())
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// dict[key] = value: a new key goes last, an existing one keeps its place.
    pub fn set(&mut self, key: &str, value: Json) {
        match self.get_mut(key) {
            Some(slot) => *slot = value,
            None => self.0.push((key.to_string(), value)),
        }
    }

    pub fn keys(&self) -> Vec<String> {
        self.0.iter().map(|(k, _)| k.clone()).collect()
    }
}

impl Json {
    pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
        let mut map = Map::new();
        for (k, v) in pairs {
            map.set(k, v);
        }
        Json::Obj(map)
    }

    pub fn str(s: &str) -> Json {
        Json::Str(s.to_string())
    }

    pub fn opt_str(s: Option<String>) -> Json {
        match s {
            Some(s) => Json::Str(s),
            None => Json::Null,
        }
    }

    pub fn num(n: Num) -> Json {
        match n {
            Num::Int(i) => Json::Int(i),
            Num::Float(f) => Json::Float(f),
        }
    }

    pub fn nums(values: &[Num]) -> Json {
        Json::Arr(values.iter().map(|&n| Json::num(n)).collect())
    }

    /// value[key], or a KeyError as Python words it.
    pub fn at(&self, key: &str) -> Result<&Json, String> {
        match self {
            Json::Obj(map) => map.get(key).ok_or_else(|| format!("KeyError: {}", py_repr_str(key))),
            Json::Null => Err("TypeError: 'NoneType' object is not subscriptable".to_string()),
            _ => Err(format!("TypeError: {} indexed with {}", self.type_name(), py_repr_str(key))),
        }
    }

    pub fn idx(&self, i: usize) -> Result<&Json, String> {
        match self {
            Json::Arr(v) => v.get(i).ok_or_else(|| "IndexError: list index out of range".to_string()),
            _ => Err(format!("TypeError: {} indexed with {}", self.type_name(), i)),
        }
    }

    pub fn as_obj(&self) -> Result<&Map, String> {
        match self {
            Json::Obj(m) => Ok(m),
            _ => Err(format!("TypeError: {} is not an object", self.type_name())),
        }
    }

    pub fn as_obj_mut(&mut self) -> Option<&mut Map> {
        match self {
            Json::Obj(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Result<&str, String> {
        match self {
            Json::Str(s) => Ok(s),
            _ => Err(format!("TypeError: {} is not a string", self.type_name())),
        }
    }

    pub fn as_num(&self) -> Result<Num, String> {
        match *self {
            Json::Int(i) => Ok(Num::Int(i)),
            Json::Float(f) => Ok(Num::Float(f)),
            Json::Bool(b) => Ok(Num::Int(b as i128)),
            _ => Err(format!("TypeError: {} is not a number", self.type_name())),
        }
    }

    /// Python's truth value.
    pub fn truthy(&self) -> bool {
        match self {
            Json::Null => false,
            Json::Bool(b) => *b,
            Json::Int(i) => *i != 0,
            Json::Float(f) => *f != 0.0,
            Json::Str(s) => !s.is_empty(),
            Json::Arr(v) => !v.is_empty(),
            Json::Obj(m) => !m.0.is_empty(),
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            Json::Null => "NoneType",
            Json::Bool(_) => "bool",
            Json::Int(_) => "int",
            Json::Float(_) => "float",
            Json::Str(_) => "str",
            Json::Arr(_) => "list",
            Json::Obj(_) => "dict",
        }
    }
}

/// json.loads.
pub fn parse_json(text: &str) -> Result<Json, String> {
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("json: extra data at char {}", p.i));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn err<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("json: {} at byte {}", what, self.i))
    }

    fn lit(&mut self, word: &str) -> bool {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > 900 {
            return self.err("nesting too deep");
        }
        let Some(&c) = self.s.get(self.i) else { return self.err("expecting value") };
        match c {
            b'{' => {
                self.i += 1;
                let mut map = Map::new();
                self.ws();
                if self.s.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(map));
                }
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return self.err("expecting property name");
                    }
                    let key = self.string()?;
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return self.err("expecting ':'");
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value(depth + 1)?;
                    map.set(&key, v);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(map));
                        }
                        _ => return self.err("expecting ',' delimiter"),
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.s.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(v));
                }
                loop {
                    self.ws();
                    v.push(self.value(depth + 1)?);
                    self.ws();
                    match self.s.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(v));
                        }
                        _ => return self.err("expecting ',' delimiter"),
                    }
                }
            }
            b'"' => Ok(Json::Str(self.string()?)),
            b'n' if self.lit("null") => Ok(Json::Null),
            b't' if self.lit("true") => Ok(Json::Bool(true)),
            b'f' if self.lit("false") => Ok(Json::Bool(false)),
            b'N' if self.lit("NaN") => Ok(Json::Float(f64::NAN)),
            b'I' if self.lit("Infinity") => Ok(Json::Float(f64::INFINITY)),
            b'-' if self.lit("-Infinity") => Ok(Json::Float(f64::NEG_INFINITY)),
            b'-' | b'0'..=b'9' => self.number(),
            _ => self.err("expecting value"),
        }
    }

    /// Python's NUMBER_RE, (-?(?:0|[1-9]\d*))(\.\d+)?([eE][-+]?\d+)?: an
    /// integer unless a fraction or an exponent follows.
    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        let digits = |p: &mut Parser| {
            let s = p.i;
            while p.i < p.s.len() && p.s[p.i].is_ascii_digit() {
                p.i += 1;
            }
            p.i - s
        };
        if self.s[self.i] == b'-' {
            self.i += 1;
        }
        match self.s.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                digits(self);
            }
            _ => return self.err("expecting value"),
        }
        let mut float = false;
        if self.s.get(self.i) == Some(&b'.') {
            let save = self.i;
            self.i += 1;
            if digits(self) == 0 {
                self.i = save;
            } else {
                float = true;
            }
        }
        if matches!(self.s.get(self.i), Some(b'e') | Some(b'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.s.get(self.i), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            if digits(self) == 0 {
                self.i = save;
            } else {
                float = true;
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).unwrap();
        if float {
            Ok(Json::Float(text.parse::<f64>().map_err(|e| e.to_string())?))
        } else {
            text.parse::<i128>().map(Json::Int).map_err(|_| format!("json: integer {text} is too large"))
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.s.get(self.i..self.i + 4).and_then(|h| std::str::from_utf8(h).ok());
        match h.and_then(|h| u32::from_str_radix(h, 16).ok()) {
            Some(v) if h.unwrap().bytes().all(|b| b.is_ascii_hexdigit()) => {
                self.i += 4;
                Ok(v)
            }
            _ => self.err("invalid \\uXXXX escape"),
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while self.i < self.s.len() && self.s[self.i] != b'"' && self.s[self.i] != b'\\' && self.s[self.i] >= 0x20 {
                self.i += 1;
            }
            out.push_str(std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?);
            match self.s.get(self.i) {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let Some(&e) = self.s.get(self.i) else { return self.err("unterminated string") };
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xd800..0xdc00).contains(&cp) && self.s[self.i..].starts_with(b"\\u") {
                                let save = self.i;
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xdc00..0xe000).contains(&lo) {
                                    cp = 0x10000 + ((cp - 0xd800) << 10) + (lo - 0xdc00);
                                } else {
                                    self.i = save;
                                }
                            }
                            match char::from_u32(cp) {
                                Some(c) => out.push(c),
                                // Python keeps a lone surrogate in its str; a
                                // Rust String cannot, and no report has one.
                                None => return self.err("lone surrogate"),
                            }
                        }
                        _ => return self.err("invalid escape"),
                    }
                }
                None => return self.err("unterminated string"),
                _ => return self.err("invalid control character"),
            }
        }
    }
}

/// json.dumps(value, separators=(',', ':')): ensure_ascii, floats as repr.
pub fn dumps(v: &Json) -> String {
    let mut out = String::new();
    dump(v, &mut out);
    out
}

fn dump(v: &Json, out: &mut String) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Json::Float(f) => out.push_str(&json_float(*f)),
        Json::Str(s) => dump_str(s, out),
        Json::Arr(items) => {
            out.push('[');
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                dump(item, out);
            }
            out.push(']');
        }
        Json::Obj(map) => {
            out.push('{');
            for (n, (k, item)) in map.0.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                dump_str(k, out);
                out.push(':');
                dump(item, out);
            }
            out.push('}');
        }
    }
}

fn dump_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{:04x}", u);
                }
            }
        }
    }
    out.push('"');
}

fn json_float(f: f64) -> String {
    if f.is_nan() {
        "NaN".to_string()
    } else if f.is_infinite() {
        if f > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
    } else {
        py_float_repr(f)
    }
}

/// repr(float): the shortest digits that read back as f, in fixed notation
/// for a decimal exponent from -4 to 15 and in scientific notation (two
/// exponent digits at least) otherwise.
pub fn py_float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".to_string();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    let sci = format!("{:e}", f); // shortest round-trip digits: "-1.2345e-5"
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => ("-", m),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let mut out = String::from(sign);
    if (-4..16).contains(&exp) {
        if exp < 0 {
            out.push_str("0.");
            for _ in 0..(-exp - 1) {
                out.push('0');
            }
            out.push_str(&digits);
        } else {
            let point = exp as usize + 1;
            if digits.len() <= point {
                out.push_str(&digits);
                for _ in digits.len()..point {
                    out.push('0');
                }
                out.push_str(".0");
            } else {
                out.push_str(&digits[..point]);
                out.push('.');
                out.push_str(&digits[point..]);
            }
        }
    } else {
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let _ = write!(out, "e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    out
}

/// repr(str), as Python prints a string inside a list or dict.
pub fn py_repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::new();
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// str() of a number, as an f-string's {x} prints it.
pub fn py_str_num(n: Num) -> String {
    match n {
        Num::Int(i) => i.to_string(),
        Num::Float(f) => py_float_repr(f),
    }
}

/// f'{x:.{digits}f}': correctly rounded, ties to even on the exact value,
/// as Rust's own formatting does.
pub fn fixed(x: f64, digits: usize) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    format!("{:.*}", digits, x)
}

/// round(x, digits) for a float.
pub fn py_round(x: f64, digits: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{:.*}", digits, x).parse().unwrap()
}

/// f'{n:,}' for an integer.
pub fn thousands(n: i128) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    if n < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ---------------------------------------------------------------- numbers

/// A Python number: an int or a float. Arithmetic follows Python's: int with
/// int stays exact, true division rounds the exact quotient once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    Int(i128),
    Float(f64),
}

impl Num {
    pub fn f(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Float(f) => f,
        }
    }

    pub fn add(self, other: Num) -> Num {
        match (self, other) {
            (Num::Int(a), Num::Int(b)) => Num::Int(a + b),
            (a, b) => Num::Float(a.f() + b.f()),
        }
    }

    pub fn sub(self, other: Num) -> Num {
        match (self, other) {
            (Num::Int(a), Num::Int(b)) => Num::Int(a - b),
            (a, b) => Num::Float(a.f() - b.f()),
        }
    }

    /// a / b, or Python's ZeroDivisionError.
    pub fn div(self, other: Num) -> Result<Num, String> {
        match (self, other) {
            (_, Num::Int(0)) => Err("ZeroDivisionError: division by zero".to_string()),
            (_, Num::Float(b)) if b == 0.0 => Err("ZeroDivisionError: float division by zero".to_string()),
            (Num::Int(a), Num::Int(b)) => Ok(Num::Float(int_ratio(a, b))),
            (a, b) => Ok(Num::Float(a.f() / b.f())),
        }
    }

    /// Python's comparison of two numbers (exact between int and float for
    /// the magnitudes a report holds).
    pub fn cmp(self, other: Num) -> std::cmp::Ordering {
        match (self, other) {
            (Num::Int(a), Num::Int(b)) => a.cmp(&b),
            (a, b) => a.f().partial_cmp(&b.f()).unwrap_or(std::cmp::Ordering::Equal),
        }
    }
}

/// int / int, correctly rounded as Python's true division is.
fn int_ratio(a: i128, b: i128) -> f64 {
    let neg = (a < 0) != (b < 0);
    let (ua, ub) = (a.unsigned_abs(), b.unsigned_abs());
    if ua < (1u128 << 53) && ub < (1u128 << 53) {
        return a as f64 / b as f64;
    }
    match u32::try_from(ub) {
        Ok(d) => ratio_to_f64(neg, Big::from_u128(ua), d, 0),
        // Not met in a report: two huge integers.
        Err(_) => a as f64 / b as f64,
    }
}

/// statistics.mean: the exact mean, rounded once. An int when every value is
/// an int and the mean is whole, as Python's _convert gives.
pub fn mean(values: &[Num]) -> Num {
    let n = values.len() as u32;
    assert!(n > 0, "mean requires at least one data point");
    if values.iter().all(|v| matches!(v, Num::Int(_))) {
        let sum: i128 = values.iter().map(|v| if let Num::Int(i) = v { *i } else { 0 }).sum();
        if sum % n as i128 == 0 {
            return Num::Int(sum / n as i128);
        }
        return Num::Float(ratio_to_f64(sum < 0, Big::from_u128(sum.unsigned_abs()), n, 0));
    }
    if values.iter().any(|v| matches!(v, Num::Float(f) if !f.is_finite())) {
        let sum: f64 = values.iter().map(|v| v.f()).sum();
        return Num::Float(sum / n as f64);
    }
    // Every value as an integer multiple of 2^-1074, summed exactly.
    let mut pos = Big::zero();
    let mut neg = Big::zero();
    for v in values {
        let (negative, big) = match *v {
            Num::Int(i) => (i < 0, Big::from_u128(i.unsigned_abs()).shl(1074)),
            Num::Float(f) => {
                let bits = f.to_bits();
                let exp = ((bits >> 52) & 0x7ff) as i64;
                let frac = bits & ((1u64 << 52) - 1);
                let (m, e) = if exp == 0 { (frac, -1074) } else { (frac | (1u64 << 52), exp - 1075) };
                (bits >> 63 == 1, Big::from_u128(m as u128).shl((e + 1074) as usize))
            }
        };
        if negative {
            neg = neg.add(&big);
        } else {
            pos = pos.add(&big);
        }
    }
    let (negative, total) = if pos.cmp(&neg) == std::cmp::Ordering::Less {
        (true, neg.sub(&pos))
    } else {
        (false, pos.sub(&neg))
    };
    Num::Float(ratio_to_f64(negative, total, n, -1074))
}

/// statistics.median: the middle value, or the mean of the two middle ones
/// as Python adds and halves them.
pub fn median(values: &[Num]) -> Num {
    assert!(!values.is_empty(), "no median for empty data");
    let mut v = values.to_vec();
    sort_nums(&mut v);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        v[n / 2 - 1].add(v[n / 2]).div(Num::Int(2)).unwrap()
    }
}

/// sorted(): stable, Python's ordering of numbers.
pub fn sort_nums(v: &mut [Num]) {
    v.sort_by(|a, b| a.cmp(*b));
}

/// (-1)^neg * a / d * 2^exp2, rounded once to the nearest double, ties to
/// even, as int.__truediv__ (and so float(Fraction)) rounds.
fn ratio_to_f64(neg: bool, a: Big, d: u32, exp2: i64) -> f64 {
    if a.is_zero() {
        return if neg { -0.0 } else { 0.0 };
    }
    let la = a.bits() as i64;
    let ld = (32 - d.leading_zeros()) as i64;
    // q = floor(a * 2^k / d) has 55 or 56 bits; sticky says the rest is not 0.
    let k = 55 - (la - ld);
    let (q, sticky) = if k >= 0 {
        let (q, r) = a.shl(k as usize).divmod(d);
        (q, r != 0)
    } else {
        let lost = a.low_bits_nonzero((-k) as usize);
        let (q, r) = a.shr((-k) as usize).divmod(d);
        (q, r != 0 || lost)
    };
    let q = q.to_u64();
    let qb = 64 - q.leading_zeros() as i64;
    // The value is q * 2^(exp2 - k); its lowest kept bit must be at 2^-1074 or above.
    let low = exp2 - k;
    let drop = (qb - 53).max(-1074 - low);
    let (mut mant, exp) = if drop <= 0 {
        (q << (-drop), low + drop)
    } else if drop >= 64 {
        let up = drop == 64 && (q > 1u64 << 63 || (q == 1u64 << 63 && sticky));
        (up as u64, low + drop)
    } else {
        let rest = q & ((1u64 << drop) - 1);
        let half = 1u64 << (drop - 1);
        let mut m = q >> drop;
        if rest > half || (rest == half && (sticky || m & 1 == 1)) {
            m += 1;
        }
        (m, low + drop)
    };
    if mant == 0 {
        return if neg { -0.0 } else { 0.0 };
    }
    let mut exp = exp;
    if mant == 1u64 << 53 {
        mant >>= 1;
        exp += 1;
    }
    let value = mant as f64 * pow2(exp);
    if neg {
        -value
    } else {
        value
    }
}

fn pow2(p: i64) -> f64 {
    if p > 1023 {
        f64::INFINITY
    } else if p >= -1022 {
        f64::from_bits(((p + 1023) as u64) << 52)
    } else if p >= -1074 {
        f64::from_bits(1u64 << (p + 1074))
    } else {
        0.0
    }
}

/// An unsigned big integer, 32-bit limbs, little-endian: only what the exact
/// mean needs.
#[derive(Clone, Debug)]
struct Big(Vec<u32>);

impl Big {
    fn zero() -> Big {
        Big(Vec::new())
    }

    fn from_u128(mut v: u128) -> Big {
        let mut limbs = Vec::new();
        while v != 0 {
            limbs.push(v as u32);
            v >>= 32;
        }
        Big(limbs)
    }

    fn trim(mut self) -> Big {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
        self
    }

    fn is_zero(&self) -> bool {
        self.0.iter().all(|&l| l == 0)
    }

    fn bits(&self) -> usize {
        let t = self.clone().trim();
        match t.0.last() {
            None => 0,
            Some(&top) => 32 * (t.0.len() - 1) + (32 - top.leading_zeros() as usize),
        }
    }

    fn shl(&self, n: usize) -> Big {
        let (words, bits) = (n / 32, n % 32);
        let mut out = vec![0u32; words];
        let mut carry = 0u32;
        for &l in &self.0 {
            if bits == 0 {
                out.push(l);
            } else {
                out.push((l << bits) | carry);
                carry = l >> (32 - bits);
            }
        }
        if carry != 0 {
            out.push(carry);
        }
        Big(out).trim()
    }

    fn shr(&self, n: usize) -> Big {
        let (words, bits) = (n / 32, n % 32);
        if words >= self.0.len() {
            return Big::zero();
        }
        let src = &self.0[words..];
        let mut out = Vec::with_capacity(src.len());
        for i in 0..src.len() {
            let hi = if i + 1 < src.len() { src[i + 1] } else { 0 };
            out.push(if bits == 0 { src[i] } else { (src[i] >> bits) | (hi << (32 - bits)) });
        }
        Big(out).trim()
    }

    fn low_bits_nonzero(&self, n: usize) -> bool {
        let (words, bits) = (n / 32, n % 32);
        for (i, &l) in self.0.iter().enumerate() {
            if i < words && l != 0 {
                return true;
            }
            if i == words {
                return bits > 0 && l & ((1u32 << bits) - 1) != 0;
            }
        }
        false
    }

    fn add(&self, other: &Big) -> Big {
        let n = self.0.len().max(other.0.len());
        let mut out = Vec::with_capacity(n + 1);
        let mut carry = 0u64;
        for i in 0..n {
            let s = *self.0.get(i).unwrap_or(&0) as u64 + *other.0.get(i).unwrap_or(&0) as u64 + carry;
            out.push(s as u32);
            carry = s >> 32;
        }
        if carry != 0 {
            out.push(carry as u32);
        }
        Big(out).trim()
    }

    /// self - other, for self >= other.
    fn sub(&self, other: &Big) -> Big {
        let mut out = Vec::with_capacity(self.0.len());
        let mut borrow = 0i64;
        for i in 0..self.0.len() {
            let mut d = self.0[i] as i64 - *other.0.get(i).unwrap_or(&0) as i64 - borrow;
            borrow = 0;
            if d < 0 {
                d += 1 << 32;
                borrow = 1;
            }
            out.push(d as u32);
        }
        Big(out).trim()
    }

    fn cmp(&self, other: &Big) -> std::cmp::Ordering {
        let (a, b) = (self.clone().trim(), other.clone().trim());
        a.0.len().cmp(&b.0.len()).then_with(|| {
            for i in (0..a.0.len()).rev() {
                let c = a.0[i].cmp(&b.0[i]);
                if c != std::cmp::Ordering::Equal {
                    return c;
                }
            }
            std::cmp::Ordering::Equal
        })
    }

    fn divmod(&self, d: u32) -> (Big, u32) {
        let mut out = vec![0u32; self.0.len()];
        let mut rem = 0u64;
        for i in (0..self.0.len()).rev() {
            let cur = (rem << 32) | self.0[i] as u64;
            out[i] = (cur / d as u64) as u32;
            rem = cur % d as u64;
        }
        (Big(out).trim(), rem as u32)
    }

    fn to_u64(&self) -> u64 {
        let t = self.clone().trim();
        assert!(t.0.len() <= 2);
        t.0.iter().rev().fold(0u64, |acc, &l| (acc << 32) | l as u64)
    }
}

// ---------------------------------------------------------------- random

/// Python's random.Random: MT19937 seeded by init_by_array with the 32-bit
/// words of abs(seed), and the methods built on it as CPython defines them.
pub struct PyRandom {
    mt: [u32; 624],
    i: usize,
}

impl PyRandom {
    pub fn new(seed: i128) -> PyRandom {
        let mut n = seed.unsigned_abs();
        let mut key = Vec::new();
        while n != 0 {
            key.push(n as u32);
            n >>= 32;
        }
        if key.is_empty() {
            key.push(0);
        }
        let mut mt = [0u32; 624];
        mt[0] = 19650218;
        for i in 1..624 {
            mt[i] = 1812433253u32.wrapping_mul(mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_add(i as u32);
        }
        let (mut i, mut j) = (1usize, 0usize);
        for _ in 0..624.max(key.len()) {
            mt[i] = (mt[i] ^ (mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_mul(1664525))
                .wrapping_add(key[j])
                .wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                mt[0] = mt[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
        }
        for _ in 0..623 {
            mt[i] = (mt[i] ^ (mt[i - 1] ^ (mt[i - 1] >> 30)).wrapping_mul(1566083941)).wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                mt[0] = mt[623];
                i = 1;
            }
        }
        mt[0] = 0x8000_0000;
        PyRandom { mt, i: 624 }
    }

    fn next_u32(&mut self) -> u32 {
        if self.i >= 624 {
            for k in 0..624 {
                let y = (self.mt[k] & 0x8000_0000) | (self.mt[(k + 1) % 624] & 0x7fff_ffff);
                self.mt[k] = self.mt[(k + 397) % 624] ^ (y >> 1) ^ if y & 1 != 0 { 0x9908_b0df } else { 0 };
            }
            self.i = 0;
        }
        let mut y = self.mt[self.i];
        self.i += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    /// getrandbits(k) for k <= 32.
    fn getrandbits(&mut self, k: u32) -> u32 {
        if k == 0 {
            0
        } else {
            self.next_u32() >> (32 - k)
        }
    }

    /// _randbelow(n): getrandbits(n.bit_length()) until it is below n.
    pub fn randbelow(&mut self, n: usize) -> usize {
        assert!(n > 0 && n <= u32::MAX as usize);
        let k = usize::BITS - n.leading_zeros();
        loop {
            let r = self.getrandbits(k) as usize;
            if r < n {
                return r;
            }
        }
    }

    /// randrange(start, stop).
    pub fn randrange(&mut self, start: i64, stop: i64) -> i64 {
        assert!(stop > start, "empty range for randrange");
        start + self.randbelow((stop - start) as usize) as i64
    }

    /// randrange(stop).
    pub fn below(&mut self, stop: i64) -> i64 {
        self.randrange(0, stop)
    }

    /// random(): 53 random bits in [0, 1).
    pub fn random(&mut self) -> f64 {
        let a = (self.next_u32() >> 5) as f64;
        let b = (self.next_u32() >> 6) as f64;
        (a * 67108864.0 + b) * (1.0 / 9007199254740992.0)
    }

    pub fn choice<'a, T>(&mut self, seq: &'a [T]) -> &'a T {
        &seq[self.randbelow(seq.len())]
    }

    /// choices(population, k=k), without weights.
    pub fn choices<T: Copy>(&mut self, population: &[T], k: usize) -> Vec<T> {
        let n = population.len() as f64;
        (0..k).map(|_| population[(self.random() * n).floor() as usize]).collect()
    }

    pub fn shuffle<T>(&mut self, x: &mut [T]) {
        for i in (1..x.len()).rev() {
            let j = self.randbelow(i + 1);
            x.swap(i, j);
        }
    }
}

// ---------------------------------------------------------------- hashes

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    h.hex()
}

pub struct Sha256 {
    state: [u32; 8],
    buf: Vec<u8>,
    len: u64,
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
    0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
    0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

impl Sha256 {
    pub fn new() -> Sha256 {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            buf: Vec::with_capacity(64),
            len: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.len += data.len() as u64;
        if !self.buf.is_empty() {
            let take = (64 - self.buf.len()).min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buf.len() == 64 {
                let block: [u8; 64] = self.buf[..].try_into().unwrap();
                self.block(&block);
                self.buf.clear();
            }
        }
        while data.len() >= 64 {
            let block: [u8; 64] = data[..64].try_into().unwrap();
            self.block(&block);
            data = &data[64..];
        }
        self.buf.extend_from_slice(data);
    }

    fn block(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut s = self.state;
        for i in 0..64 {
            let s1 = s[4].rotate_right(6) ^ s[4].rotate_right(11) ^ s[4].rotate_right(25);
            let ch = (s[4] & s[5]) ^ (!s[4] & s[6]);
            let t1 = s[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = s[0].rotate_right(2) ^ s[0].rotate_right(13) ^ s[0].rotate_right(22);
            let maj = (s[0] & s[1]) ^ (s[0] & s[2]) ^ (s[1] & s[2]);
            let t2 = s0.wrapping_add(maj);
            s = [t1.wrapping_add(t2), s[0], s[1], s[2], s[3].wrapping_add(t1), s[4], s[5], s[6]];
        }
        for i in 0..8 {
            self.state[i] = self.state[i].wrapping_add(s[i]);
        }
    }

    pub fn hex(mut self) -> String {
        let bits = self.len.wrapping_mul(8);
        let mut pad = vec![0x80u8];
        while (self.len as usize + pad.len()) % 64 != 56 {
            pad.push(0);
        }
        pad.extend_from_slice(&bits.to_be_bytes());
        let len = self.len;
        self.update(&pad);
        self.len = len;
        self.state.iter().map(|w| format!("{:08x}", w)).collect()
    }
}

pub fn base64(data: &[u8]) -> Vec<u8> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63]);
        out.push(T[(n >> 12) as usize & 63]);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] } else { b'=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] } else { b'=' });
    }
    out
}

// ---------------------------------------------------------------- paths

/// str(Path(s)): repeated slashes and '.' parts dropped, no trailing slash.
pub fn py_path(s: &str) -> String {
    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
    let lead = if s.starts_with("//") && !s.starts_with("///") {
        "//"
    } else if s.starts_with('/') {
        "/"
    } else {
        ""
    };
    let joined = parts.join("/");
    if lead.is_empty() && joined.is_empty() {
        ".".to_string()
    } else {
        format!("{lead}{joined}")
    }
}

/// Path(s).name.
pub fn py_name(s: &str) -> String {
    let p = py_path(s);
    match p.rsplit_once('/') {
        Some((_, name)) => name.to_string(),
        None if p == "." => String::new(),
        None => p,
    }
}

// ---------------------------------------------------------------- arguments

/// One option of an argparse-like parser.
pub struct Opt {
    pub name: &'static str,
    /// The metavar, or None for a flag (store_true).
    pub metavar: Option<&'static str>,
    pub choices: Option<&'static [&'static str]>,
    pub int: bool,
}

pub struct Args {
    /// Every value an option was given, in order (a flag gets one empty value).
    pub values: Vec<(&'static str, String)>,
    pub positionals: Vec<String>,
}

impl Args {
    pub fn last(&self, name: &str) -> Option<&str> {
        self.values.iter().rev().find(|(n, _)| *n == name).map(|(_, v)| v.as_str())
    }

    pub fn all(&self, name: &str) -> Vec<String> {
        self.values.iter().filter(|(n, _)| *n == name).map(|(_, v)| v.clone()).collect()
    }

    pub fn flag(&self, name: &str) -> bool {
        self.last(name).is_some()
    }

    pub fn int(&self, name: &str, default: i128) -> i128 {
        self.last(name).map(|v| py_int(v).unwrap()).unwrap_or(default)
    }
}

/// argparse's usage and error lines, then exit status 2.
pub fn arg_error(usage: &str, prog: &str, message: &str) -> ! {
    eprint!("{usage}");
    eprintln!("{prog}: error: {message}");
    std::process::exit(2);
}

/// int(s) as argparse's type=int calls it: surrounding whitespace, a sign,
/// and single underscores between digits.
pub fn py_int(s: &str) -> Option<i128> {
    let t = s.trim();
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if digits.is_empty() || digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    let clean: String = digits.chars().filter(|&c| c != '_').collect();
    if !clean.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let v: i128 = clean.parse().ok()?;
    Some(if neg { -v } else { v })
}

fn negative_number(s: &str) -> bool {
    // argparse's _negative_number_matcher: '^-\d+$|^-\d*\.\d+$'
    let Some(rest) = s.strip_prefix('-') else { return false };
    if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    match rest.split_once('.') {
        Some((a, b)) => a.bytes().all(|c| c.is_ascii_digit()) && !b.is_empty() && b.bytes().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// Parse argv as argparse would for long options only: --name VALUE,
/// --name=VALUE, unique prefixes, -h/--help, '--', and its error messages.
/// `required` options are checked after the rest; `min_positionals` names
/// the positional argument that needs one value at least.
pub fn parse_args(
    argv: &[String],
    opts: &[Opt],
    usage: &str,
    help: &str,
    prog: &str,
    required: &[&str],
    positional: Option<&str>,
) -> Args {
    let mut values = Vec::new();
    let mut positionals = Vec::new();
    let mut extras = Vec::new();
    let mut i = 0;
    let mut only_positional = false;
    while i < argv.len() {
        let arg = &argv[i];
        i += 1;
        if only_positional || !arg.starts_with('-') || arg == "-" || negative_number(arg) {
            if positional.is_some() {
                positionals.push(arg.clone());
            } else {
                extras.push(arg.clone());
            }
            continue;
        }
        if arg == "--" {
            only_positional = true;
            continue;
        }
        if arg == "-h" || arg == "--help" {
            print!("{help}");
            std::process::exit(0);
        }
        if !arg.starts_with("--") {
            extras.push(arg.clone());
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (arg.as_str(), None),
        };
        let opt = match opts.iter().find(|o| o.name == name) {
            Some(o) => o,
            None => {
                let mut matches: Vec<&Opt> = opts.iter().filter(|o| o.name.starts_with(name)).collect();
                let help_matches = "--help".starts_with(name);
                if matches.len() + help_matches as usize > 1 {
                    let mut names: Vec<&str> = Vec::new();
                    if help_matches {
                        names.push("--help");
                    }
                    names.extend(matches.iter().map(|o| o.name));
                    arg_error(usage, prog, &format!("ambiguous option: {} could match {}", name, names.join(", ")));
                }
                match matches.pop() {
                    Some(o) => o,
                    None if help_matches && name.len() > 2 => {
                        print!("{help}");
                        std::process::exit(0);
                    }
                    None => {
                        extras.push(arg.clone());
                        continue;
                    }
                }
            }
        };
        let Some(metavar) = opt.metavar else {
            if inline.is_some() {
                arg_error(usage, prog, &format!("argument {}: ignored explicit argument {}", opt.name, py_repr_str(inline.as_deref().unwrap())));
            }
            values.push((opt.name, String::new()));
            continue;
        };
        let _ = metavar;
        let value = match inline {
            Some(v) => v,
            None => match argv.get(i) {
                Some(v) if !(v.starts_with('-') && v != "-" && !negative_number(v)) => {
                    i += 1;
                    v.clone()
                }
                _ => arg_error(usage, prog, &format!("argument {}: expected one argument", opt.name)),
            },
        };
        if opt.int && py_int(&value).is_none() {
            arg_error(usage, prog, &format!("argument {}: invalid int value: {}", opt.name, py_repr_str(&value)));
        }
        if let Some(choices) = opt.choices {
            if !choices.contains(&value.as_str()) {
                let list: Vec<String> = choices.iter().map(|c| py_repr_str(c)).collect();
                arg_error(usage, prog, &format!("argument {}: invalid choice: {} (choose from {})", opt.name, py_repr_str(&value), list.join(", ")));
            }
        }
        values.push((opt.name, value));
    }
    let mut missing: Vec<&str> = required.iter().copied().filter(|r| !values.iter().any(|(n, _)| n == r)).collect();
    if let Some(p) = positional {
        if positionals.is_empty() {
            missing.push(p);
        }
    }
    if !missing.is_empty() {
        arg_error(usage, prog, &format!("the following arguments are required: {}", missing.join(", ")));
    }
    if !extras.is_empty() {
        arg_error(usage, prog, &format!("unrecognized arguments: {}", extras.join(" ")));
    }
    Args { values, positionals }
}

/// str.splitlines().
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if matches!(c, '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}') {
            out.push(&s[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' {
                if let Some(&(_, '\n')) = it.peek() {
                    it.next();
                    end += 1;
                }
            }
            start = end;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}
