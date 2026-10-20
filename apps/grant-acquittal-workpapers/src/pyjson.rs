//! CPython's `json` as the workpaper script used it: `json.loads(bytes)` with a
//! duplicate-field hook, including the decoder's error messages and positions,
//! and `json.dumps(value, indent=2)` with keys in insertion order.

use crate::pytext;

/// A Python `str`: Unicode code points, possibly including lone surrogates.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PyStr(pub Vec<u32>);

impl From<&str> for PyStr {
    fn from(text: &str) -> PyStr {
        PyStr(text.chars().map(|ch| ch as u32).collect())
    }
}

impl PyStr {
    /// The text, or `None` when it holds a lone surrogate.
    pub fn to_utf8(&self) -> Option<String> {
        self.0.iter().map(|&cp| char::from_u32(cp)).collect()
    }

    pub fn is(&self, text: &str) -> bool {
        self.0.iter().copied().eq(text.chars().map(|ch| ch as u32))
    }

    /// Text for output: lone surrogates written as `\udXXX`.
    /// The text with lone surrogates carried as escape pairs and a genuine
    /// escape doubled, for printing through the stdout and stderr writers.
    pub fn for_output(&self) -> String {
        let mut out = String::new();
        self.0
            .iter()
            .for_each(|&cp| crate::pytext::push_code_point(&mut out, cp));
        out
    }

    pub fn lossy(&self) -> String {
        self.0
            .iter()
            .map(|&cp| char::from_u32(cp).map_or_else(|| format!("\\u{cp:04x}"), String::from))
            .collect()
    }
}

#[derive(Clone, Debug)]
pub enum Json {
    Null,
    Bool(bool),
    /// An integer's decimal digits (with sign), as Python keeps any size.
    Int(String),
    Float(f64),
    Str(PyStr),
    Array(Vec<Json>),
    Object(Vec<(PyStr, Json)>),
}

impl Json {
    pub fn str(text: &str) -> Json {
        Json::Str(PyStr::from(text))
    }

    pub fn object<const N: usize>(pairs: [(&str, Json); N]) -> Json {
        Json::Object(
            pairs
                .into_iter()
                .map(|(key, value)| (PyStr::from(key), value))
                .collect(),
        )
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(pairs) => pairs.iter().find(|(k, _)| k.is(key)).map(|(_, v)| v),
            _ => None,
        }
    }

    /// `obj[key]` for a key the caller has proved present.
    pub fn at(&self, key: &str) -> &Json {
        self.get(key).unwrap_or_else(|| panic!("key {key} checked present"))
    }

    pub fn is_str(&self, text: &str) -> bool {
        matches!(self, Json::Str(s) if s.is(text))
    }

    /// `set(obj) == required` for a dict; false for anything else.
    pub fn has_exact_keys(&self, required: &[&str]) -> bool {
        match self {
            Json::Object(pairs) => {
                pairs.len() == required.len() && required.iter().all(|r| pairs.iter().any(|(k, _)| k.is(r)))
            }
            _ => false,
        }
    }
}

/// Why `json.loads` did not return a value.
#[derive(Debug, PartialEq, Eq)]
pub enum LoadError {
    /// A `ValueError` (JSONDecodeError, UnicodeDecodeError, the duplicate-field hook, the integer digit limit).
    Value(String),
    /// `RecursionError` from nesting deeper than the decoder allows.
    Recursion,
}

const MAX_INT_DIGITS: usize = 4300;
const MAX_DEPTH: usize = 10_000;

/// `bytes.decode(json.detect_encoding(b), "surrogatepass")`.
pub fn decode_json_bytes(bytes: &[u8]) -> Result<Vec<u32>, String> {
    let starts = |prefix: &[u8]| bytes.starts_with(prefix);
    let encoding = if starts(b"\x00\x00\xfe\xff") || starts(b"\xff\xfe\x00\x00") {
        "utf-32"
    } else if starts(b"\xfe\xff") || starts(b"\xff\xfe") {
        "utf-16"
    } else if starts(b"\xef\xbb\xbf") {
        "utf-8-sig"
    } else if bytes.len() >= 4 {
        match (bytes[0], bytes[1], bytes[2], bytes[3]) {
            (0, 0, _, _) => "utf-32-be",
            (0, _, _, _) => "utf-16-be",
            (_, 0, a, b) if a != 0 || b != 0 => "utf-16-le",
            (_, 0, _, _) => "utf-32-le",
            _ => "utf-8",
        }
    } else if bytes.len() == 2 && bytes[0] == 0 {
        "utf-16-be"
    } else if bytes.len() == 2 && bytes[1] == 0 {
        "utf-16-le"
    } else {
        "utf-8"
    };
    match encoding {
        "utf-8" => decode_utf8(bytes, true),
        "utf-8-sig" => decode_utf8(&bytes[3..], true),
        "utf-16" | "utf-16-le" | "utf-16-be" => decode_utf16(bytes, encoding),
        _ => decode_utf32(bytes, encoding),
    }
}

/// CPython's UTF-8 decoder: strict, or with `surrogatepass` admitting encoded surrogates.
pub fn decode_utf8(bytes: &[u8], surrogatepass: bool) -> Result<Vec<u32>, String> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let (len, low, high) = match b {
            0x00..=0x7F => {
                out.push(b as u32);
                i += 1;
                continue;
            }
            0xC2..=0xDF => (2, 0x80, 0xBF),
            0xE0 => (3, 0xA0, 0xBF),
            0xED if surrogatepass => (3, 0x80, 0xBF),
            0xED => (3, 0x80, 0x9F),
            0xE1..=0xEF => (3, 0x80, 0xBF),
            0xF0 => (4, 0x90, 0xBF),
            0xF1..=0xF3 => (4, 0x80, 0xBF),
            0xF4 => (4, 0x80, 0x8F),
            _ => return Err(utf8_error(b, i, i, "invalid start byte")),
        };
        let mut cp = (b as u32) & (0x7F >> len);
        for k in 1..len {
            let Some(&c) = bytes.get(i + k) else {
                return Err(utf8_error(b, i, bytes.len() - 1, "unexpected end of data"));
            };
            let (lo, hi) = if k == 1 { (low, high) } else { (0x80, 0xBF) };
            if !(lo..=hi).contains(&c) {
                return Err(utf8_error(b, i, i + k - 1, "invalid continuation byte"));
            }
            cp = (cp << 6) | (c as u32 & 0x3F);
        }
        out.push(cp);
        i += len;
    }
    Ok(out)
}

fn utf8_error(first: u8, start: usize, end: usize, reason: &str) -> String {
    if start == end {
        format!("'utf-8' codec can't decode byte 0x{first:02x} in position {start}: {reason}")
    } else {
        format!("'utf-8' codec can't decode bytes in position {start}-{end}: {reason}")
    }
}

fn decode_utf16(bytes: &[u8], encoding: &str) -> Result<Vec<u32>, String> {
    let (big, body) = match encoding {
        "utf-16-be" => (true, bytes),
        "utf-16-le" => (false, bytes),
        _ if bytes.starts_with(b"\xfe\xff") => (true, &bytes[2..]),
        _ => (false, &bytes[2..]),
    };
    let offset = bytes.len() - body.len();
    if body.len() % 2 == 1 {
        let at = offset + body.len() - 1;
        return Err(format!(
            "'{encoding}' codec can't decode byte 0x{:02x} in position {at}: truncated data",
            body[body.len() - 1]
        ));
    }
    let units: Vec<u32> = body
        .chunks(2)
        .map(|c| if big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) } as u32)
        .collect();
    let mut out = Vec::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        let unit = units[i];
        if (0xD800..0xDC00).contains(&unit) && units.get(i + 1).is_some_and(|n| (0xDC00..0xE000).contains(n)) {
            out.push(0x10000 + ((unit - 0xD800) << 10) + (units[i + 1] - 0xDC00));
            i += 2;
        } else {
            // surrogatepass keeps an unpaired surrogate as a code point.
            out.push(unit);
            i += 1;
        }
    }
    Ok(out)
}

fn decode_utf32(bytes: &[u8], encoding: &str) -> Result<Vec<u32>, String> {
    let (big, body) = match encoding {
        "utf-32-be" => (true, bytes),
        "utf-32-le" => (false, bytes),
        _ if bytes.starts_with(b"\x00\x00\xfe\xff") => (true, &bytes[4..]),
        _ => (false, &bytes[4..]),
    };
    let offset = bytes.len() - body.len();
    if body.len() % 4 != 0 {
        let start = offset + body.len() / 4 * 4;
        return Err(format!(
            "'{encoding}' codec can't decode bytes in position {start}-{}: truncated data",
            bytes.len() - 1
        ));
    }
    let mut out = Vec::with_capacity(body.len() / 4);
    for (index, chunk) in body.chunks(4).enumerate() {
        let array = [chunk[0], chunk[1], chunk[2], chunk[3]];
        let cp = if big {
            u32::from_be_bytes(array)
        } else {
            u32::from_le_bytes(array)
        };
        if cp > 0x10FFFF {
            let start = offset + index * 4;
            return Err(format!(
                "'{encoding}' codec can't decode bytes in position {start}-{}: code point not in range(0x110000)",
                start + 3
            ));
        }
        out.push(cp);
    }
    Ok(out)
}

/// `json.loads(content, object_pairs_hook=unique_fields)` on bytes.
pub fn loads_bytes(bytes: &[u8]) -> Result<Json, LoadError> {
    let text = decode_json_bytes(bytes).map_err(LoadError::Value)?;
    let mut parser = Parser { text: &text, depth: 0 };
    let start = parser.skip_ws(0);
    let (value, end) = parser.value(start)?;
    let end = parser.skip_ws(end);
    if end != text.len() {
        return Err(parser.error("Extra data", end));
    }
    Ok(value)
}

type Step<'a> = fn(&mut Parser<'a>, usize) -> Result<(Json, usize), LoadError>;

struct Parser<'a> {
    text: &'a [u32],
    depth: usize,
}

fn is_ws(cp: u32) -> bool {
    matches!(cp, 0x20 | 0x09 | 0x0A | 0x0D)
}

impl<'a> Parser<'a> {
    fn at(&self, idx: usize) -> Option<u32> {
        self.text.get(idx).copied()
    }

    /// `JSONDecodeError(msg, doc, pos)`: "msg: line L column C (char P)".
    fn error(&self, message: &str, pos: usize) -> LoadError {
        let before = &self.text[..pos.min(self.text.len())];
        let line = before.iter().filter(|&&c| c == 0x0A).count() + 1;
        let column = match before.iter().rposition(|&c| c == 0x0A) {
            Some(newline) => pos - newline,
            None => pos + 1,
        };
        LoadError::Value(format!("{message}: line {line} column {column} (char {pos})"))
    }

    fn skip_ws(&self, mut idx: usize) -> usize {
        while self.at(idx).is_some_and(is_ws) {
            idx += 1;
        }
        idx
    }

    fn literal(&self, idx: usize, word: &str) -> bool {
        word.chars()
            .enumerate()
            .all(|(i, ch)| self.at(idx + i) == Some(ch as u32))
    }

    fn value(&mut self, idx: usize) -> Result<(Json, usize), LoadError> {
        let Some(c) = self.at(idx) else {
            return Err(self.error("Expecting value", idx));
        };
        match char::from_u32(c) {
            Some('"') => self.string(idx + 1).map(|(s, end)| (Json::Str(s), end)),
            Some('{') => self.nested(idx, Self::object),
            Some('[') => self.nested(idx, Self::array),
            Some('n') if self.literal(idx, "null") => Ok((Json::Null, idx + 4)),
            Some('t') if self.literal(idx, "true") => Ok((Json::Bool(true), idx + 4)),
            Some('f') if self.literal(idx, "false") => Ok((Json::Bool(false), idx + 5)),
            Some('N') if self.literal(idx, "NaN") => Ok((Json::Float(f64::NAN), idx + 3)),
            Some('I') if self.literal(idx, "Infinity") => Ok((Json::Float(f64::INFINITY), idx + 8)),
            Some('-') if self.literal(idx, "-Infinity") => Ok((Json::Float(f64::NEG_INFINITY), idx + 9)),
            _ => self.number(idx),
        }
    }

    fn nested(&mut self, idx: usize, parse: Step<'a>) -> Result<(Json, usize), LoadError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(LoadError::Recursion);
        }
        let result = parse(self, idx + 1);
        self.depth -= 1;
        result
    }

    fn number(&self, start: usize) -> Result<(Json, usize), LoadError> {
        let (idx, is_float) = self.scan_number(start)?;
        let numstr: String = self.text[start..idx]
            .iter()
            .map(|&c| char::from_u32(c).unwrap())
            .collect();
        if is_float {
            return Ok((Json::Float(numstr.parse().expect("JSON float grammar")), idx));
        }
        let digits = numstr.trim_start_matches('-').len();
        if digits > MAX_INT_DIGITS {
            return Err(LoadError::Value(format!(
                "Exceeds the limit ({MAX_INT_DIGITS} digits) for integer string conversion: value has {digits} digits; use sys.set_int_max_str_digits() to increase the limit"
            )));
        }
        let normalised = match numstr.strip_prefix('-') {
            Some("0") => "0".to_string(),
            _ => numstr,
        };
        Ok((Json::Int(normalised), idx))
    }

    /// The scanner's NUMBER_RE: where the number ends and whether it is a float.
    fn scan_number(&self, start: usize) -> Result<(usize, bool), LoadError> {
        let digit = |idx: usize| self.at(idx).is_some_and(|c| (0x30..=0x39).contains(&c));
        let mut idx = start;
        if self.at(idx) == Some('-' as u32) {
            idx += 1;
        }
        match self.at(idx) {
            Some(c) if (0x31..=0x39).contains(&c) => {
                idx += 1;
                while digit(idx) {
                    idx += 1;
                }
            }
            Some(0x30) => idx += 1,
            _ => return Err(self.error("Expecting value", start)),
        }
        let mut is_float = false;
        if self.at(idx) == Some('.' as u32) && digit(idx + 1) {
            is_float = true;
            idx += 2;
            while digit(idx) {
                idx += 1;
            }
        }
        if matches!(self.at(idx), Some(0x65 | 0x45)) && idx + 1 < self.text.len() {
            let e_start = idx;
            idx += 1;
            if matches!(self.at(idx), Some(0x2B | 0x2D)) && idx + 1 < self.text.len() {
                idx += 1;
            }
            while digit(idx) {
                idx += 1;
            }
            if digit(idx - 1) {
                is_float = true;
            } else {
                idx = e_start;
            }
        }
        Ok((idx, is_float))
    }

    fn string(&self, end: usize) -> Result<(PyStr, usize), LoadError> {
        let begin = end - 1;
        let mut out = Vec::new();
        let mut next = end;
        loop {
            let c = loop {
                let Some(c) = self.at(next) else {
                    return Err(self.error("Unterminated string starting at", begin));
                };
                if c == 0x22 || c == 0x5C {
                    break c;
                }
                if c <= 0x1F {
                    return Err(self.error("Invalid control character at", next));
                }
                out.push(c);
                next += 1;
            };
            next += 1;
            if c == 0x22 {
                return Ok((PyStr(out), next));
            }
            let Some(escape) = self.at(next) else {
                return Err(self.error("Unterminated string starting at", begin));
            };
            let (unit, after) = if escape == 'u' as u32 {
                self.unicode_escape(next + 1)?
            } else {
                (self.simple_escape(escape, next)?, next + 1)
            };
            out.push(unit);
            next = after;
        }
    }

    /// A backslash escape other than `\u`, at `idx` (just after the backslash).
    fn simple_escape(&self, escape: u32, idx: usize) -> Result<u32, LoadError> {
        match char::from_u32(escape) {
            Some('"') => Ok(0x22),
            Some('\\') => Ok(0x5C),
            Some('/') => Ok(0x2F),
            Some('b') => Ok(0x08),
            Some('f') => Ok(0x0C),
            Some('n') => Ok(0x0A),
            Some('r') => Ok(0x0D),
            Some('t') => Ok(0x09),
            _ => Err(self.error("Invalid \\escape", idx - 1)),
        }
    }

    /// `\uXXXX` with its hex digits at `next`, joining a following low surrogate escape.
    fn unicode_escape(&self, next: usize) -> Result<(u32, usize), LoadError> {
        let len = self.text.len();
        let mut end = next + 4;
        if end >= len {
            return Err(self.error("Invalid \\uXXXX escape", next - 1));
        }
        let mut unit = self
            .hex4(next)
            .ok_or_else(|| self.error("Invalid \\uXXXX escape", end - 5))?;
        if (0xD800..=0xDBFF).contains(&unit)
            && end + 6 < len
            && self.at(end) == Some(0x5C)
            && self.at(end + 1) == Some('u' as u32)
        {
            let low = self
                .hex4(end + 2)
                .ok_or_else(|| self.error("Invalid \\uXXXX escape", end + 1))?;
            if (0xDC00..=0xDFFF).contains(&low) {
                unit = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                end += 6;
            }
        }
        Ok((unit, end))
    }

    fn hex4(&self, idx: usize) -> Option<u32> {
        let mut value = 0;
        for i in 0..4 {
            value = value * 16 + self.at(idx + i).and_then(char::from_u32).and_then(|c| c.to_digit(16))?;
        }
        Some(value)
    }

    fn array(&mut self, idx: usize) -> Result<(Json, usize), LoadError> {
        let mut items = Vec::new();
        let mut idx = self.skip_ws(idx);
        if self.at(idx) == Some(']' as u32) {
            return Ok((Json::Array(items), idx + 1));
        }
        loop {
            let (item, next) = self.value(idx)?;
            items.push(item);
            idx = self.skip_ws(next);
            match self.at(idx).and_then(char::from_u32) {
                Some(']') => return Ok((Json::Array(items), idx + 1)),
                Some(',') => {}
                _ => return Err(self.error("Expecting ',' delimiter", idx)),
            }
            let comma = idx;
            idx = self.skip_ws(idx + 1);
            if self.at(idx) == Some(']' as u32) {
                return Err(self.error("Illegal trailing comma before end of array", comma));
            }
        }
    }

    fn object(&mut self, idx: usize) -> Result<(Json, usize), LoadError> {
        let mut pairs: Vec<(PyStr, Json)> = Vec::new();
        let mut idx = self.skip_ws(idx);
        if self.at(idx) != Some('}' as u32) {
            loop {
                if self.at(idx) != Some('"' as u32) {
                    return Err(self.error("Expecting property name enclosed in double quotes", idx));
                }
                let (key, next) = self.string(idx + 1)?;
                idx = self.skip_ws(next);
                if self.at(idx) != Some(':' as u32) {
                    return Err(self.error("Expecting ':' delimiter", idx));
                }
                idx = self.skip_ws(idx + 1);
                let (value, next) = self.value(idx)?;
                pairs.push((key, value));
                idx = self.skip_ws(next);
                match self.at(idx).and_then(char::from_u32) {
                    Some('}') => break,
                    Some(',') => {}
                    _ => return Err(self.error("Expecting ',' delimiter", idx)),
                }
                let comma = idx;
                idx = self.skip_ws(idx + 1);
                if self.at(idx) == Some('}' as u32) {
                    return Err(self.error("Illegal trailing comma before end of object", comma));
                }
            }
        }
        // unique_fields runs once the object is complete, before parsing continues.
        for (i, (key, _)) in pairs.iter().enumerate() {
            if pairs[..i].iter().any(|(earlier, _)| earlier == key) {
                return Err(LoadError::Value(format!("Duplicate JSON field: {}", key.for_output())));
            }
        }
        Ok((Json::Object(pairs), idx + 1))
    }
}

/// `format!("{value:e}")`, but where two spellings are equally short, the one
/// CPython's `repr` prints: the nearer to `value`, an exact tie going to the even
/// digit. Rust's shortest formatting can break that tie upwards: `1 + 2**-17` is
/// `1.0000076293945312` in Python, not `…313`. `{:.Ne}` rounds exactly with ties
/// to even, so its spelling is used whenever it still reads back as `value`.
fn shortest_exp(value: f64) -> String {
    let shortest = format!("{value:e}");
    let precision = shortest
        .bytes()
        .take_while(|&b| b != b'e')
        .filter(u8::is_ascii_digit)
        .count()
        - 1;
    let nearest = format!("{value:.precision$e}");
    if nearest.parse::<f64>() == Ok(value) {
        nearest
    } else {
        shortest
    }
}

/// `float.__repr__`.
pub fn float_repr(value: f64) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    let sci = shortest_exp(value);
    let (mantissa, exponent) = sci.split_once('e').expect("Rust {:e} has an exponent");
    let exponent: i32 = exponent.parse().expect("integer exponent");
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
    let decpt = exponent + 1;
    let sign = if negative { "-" } else { "" };
    if !(-4 < decpt && decpt <= 16) {
        let (first, rest) = digits.split_at(1);
        let rest = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let exp_sign = if exponent < 0 { '-' } else { '+' };
        return format!("{sign}{first}{rest}e{exp_sign}{:02}", exponent.abs());
    }
    let n = digits.len() as i32;
    if decpt <= 0 {
        format!("{sign}0.{}{digits}", "0".repeat((-decpt) as usize))
    } else if decpt >= n {
        format!("{sign}{digits}{}.0", "0".repeat((decpt - n) as usize))
    } else {
        let (int, frac) = digits.split_at(decpt as usize);
        format!("{sign}{int}.{frac}")
    }
}

/// `repr()` of a parsed value, as the script's error messages print it.
pub fn repr(value: &Json) -> String {
    match value {
        Json::Null => "None".into(),
        Json::Bool(true) => "True".into(),
        Json::Bool(false) => "False".into(),
        Json::Int(digits) => digits.clone(),
        Json::Float(f) => float_repr(*f),
        Json::Str(s) => pytext::repr_code_points(&s.0),
        Json::Array(items) => format!("[{}]", items.iter().map(repr).collect::<Vec<_>>().join(", ")),
        Json::Object(pairs) => format!(
            "{{{}}}",
            pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", pytext::repr_code_points(&k.0), repr(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn write_str(out: &mut String, s: &PyStr) {
    out.push('"');
    for &cp in &s.0 {
        match cp {
            0x22 => out.push_str("\\\""),
            0x5C => out.push_str("\\\\"),
            0x0A => out.push_str("\\n"),
            0x0D => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            0x08 => out.push_str("\\b"),
            0x0C => out.push_str("\\f"),
            0x20..=0x7E => out.push(char::from_u32(cp).unwrap()),
            0x10000.. => {
                let v = cp - 0x10000;
                out.push_str(&format!("\\u{:04x}\\u{:04x}", 0xD800 + (v >> 10), 0xDC00 + (v & 0x3FF)));
            }
            _ => out.push_str(&format!("\\u{cp:04x}")),
        }
    }
    out.push('"');
}

/// `json.dumps(value, indent=2)`: keys in insertion order, ensure_ascii.
pub fn dumps_indent2(value: &Json) -> String {
    fn go(out: &mut String, value: &Json, level: usize) {
        let inner = format!("\n{}", "  ".repeat(level + 1));
        let outer = format!("\n{}", "  ".repeat(level));
        match value {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Int(digits) => out.push_str(digits),
            Json::Float(f) if f.is_nan() => out.push_str("NaN"),
            Json::Float(f) if f.is_infinite() => out.push_str(if *f > 0.0 { "Infinity" } else { "-Infinity" }),
            Json::Float(f) => out.push_str(&float_repr(*f)),
            Json::Str(s) => write_str(out, s),
            Json::Array(items) if items.is_empty() => out.push_str("[]"),
            Json::Object(pairs) if pairs.is_empty() => out.push_str("{}"),
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    out.push_str(if i > 0 { "," } else { "" });
                    out.push_str(&inner);
                    go(out, item, level + 1);
                }
                out.push_str(&outer);
                out.push(']');
            }
            Json::Object(pairs) => {
                out.push('{');
                for (i, (key, item)) in pairs.iter().enumerate() {
                    out.push_str(if i > 0 { "," } else { "" });
                    out.push_str(&inner);
                    write_str(out, key);
                    out.push_str(": ");
                    go(out, item, level + 1);
                }
                out.push_str(&outer);
                out.push('}');
            }
        }
    }
    let mut out = String::new();
    go(&mut out, value, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(text: &[u8]) -> String {
        match loads_bytes(text).unwrap_err() {
            LoadError::Value(message) => message,
            LoadError::Recursion => "recursion".into(),
        }
    }

    #[test]
    fn decoder_messages_match_cpython() {
        assert_eq!(err(b""), "Expecting value: line 1 column 1 (char 0)");
        assert_eq!(err(b"{\"a\":1} x"), "Extra data: line 1 column 9 (char 8)");
        assert_eq!(err(b"{\"a\" 1}"), "Expecting ':' delimiter: line 1 column 6 (char 5)");
        assert_eq!(
            err(b"{\"a\":1 \"b\"}"),
            "Expecting ',' delimiter: line 1 column 8 (char 7)"
        );
        assert_eq!(
            err(b"{\"a\":1,}"),
            "Illegal trailing comma before end of object: line 1 column 7 (char 6)"
        );
        assert_eq!(
            err(b"[1,]"),
            "Illegal trailing comma before end of array: line 1 column 3 (char 2)"
        );
        assert_eq!(
            err(b"{1}"),
            "Expecting property name enclosed in double quotes: line 1 column 2 (char 1)"
        );
        assert_eq!(
            err(b"\n  \"abc"),
            "Unterminated string starting at: line 2 column 3 (char 3)"
        );
        assert_eq!(err(b"\"a\\x\""), "Invalid \\escape: line 1 column 3 (char 2)");
        assert_eq!(err(b"\"\\u12g4\""), "Invalid \\uXXXX escape: line 1 column 3 (char 2)");
        assert_eq!(
            err(b"\"a\x01\""),
            "Invalid control character at: line 1 column 3 (char 2)"
        );
        assert_eq!(err(b"{\"a\":1,\"a\":2}"), "Duplicate JSON field: a");
        // A key holding the carrier's own characters prints as itself after the writer decodes it.
        let key = "\u{10F7FF}\u{10F7FF}\u{10F7FF}\u{10F800}";
        let message = err(format!("{{\"{key}\":1,\"{key}\":2}}").as_bytes());
        let printed: String = crate::pytext::code_points(&message)
            .map(|cp| char::from_u32(cp).unwrap())
            .collect();
        assert_eq!(printed, format!("Duplicate JSON field: {key}"));
        assert_eq!(
            err(b"\xff{}"),
            "'utf-8' codec can't decode byte 0xff in position 0: invalid start byte"
        );
        assert!(loads_bytes(b"\xef\xbb\xbf{}").is_ok());
        assert!(matches!(loads_bytes(b"\"\xed\xa0\x80\""), Ok(Json::Str(s)) if s.0 == vec![0xD800]));
    }

    #[test]
    fn dumps_and_repr_match_cpython() {
        let value = loads_bytes(b"{\"b\": [1, 2.5, \"\\u00e9\"], \"a\": {}}").unwrap();
        assert_eq!(
            dumps_indent2(&value),
            "{\n  \"b\": [\n    1,\n    2.5,\n    \"\\u00e9\"\n  ],\n  \"a\": {}\n}"
        );
        assert_eq!(repr(&value), "{'b': [1, 2.5, '\u{e9}'], 'a': {}}");
        assert_eq!(
            repr(&loads_bytes(b"[null, true, 1e400, -0]").unwrap()),
            "[None, True, inf, 0]"
        );
    }

    #[test]
    fn float_repr_matches_cpython_corpus() {
        // CPython 3.15's repr for doubles chosen to exercise ties between two
        // equally short spellings: bits in hex, then the repr.
        assert_eq!(float_repr(1.0 + 2f64.powi(-17)), "1.0000076293945312");
        let wrong: Vec<String> = include_str!("../tests/data/float_repr.txt")
            .lines()
            .filter_map(|line| {
                let (bits, expected) = line.split_once(' ').expect("bits and repr");
                let actual = float_repr(f64::from_bits(u64::from_str_radix(bits, 16).unwrap()));
                (actual != expected).then(|| format!("{bits}: python {expected}, rust {actual}"))
            })
            .collect();
        assert!(
            wrong.is_empty(),
            "{} wrong: {:?}",
            wrong.len(),
            &wrong[..wrong.len().min(10)]
        );
    }
}
