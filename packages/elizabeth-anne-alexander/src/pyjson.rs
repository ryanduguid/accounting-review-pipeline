//! CPython's `json` module as the gateway uses it: `json.loads` with a
//! duplicate-key hook, `json.dumps(sort_keys=True)` in its canonical and
//! `indent=2` forms, and Python equality over the parsed values.
//!
//! Strings are code point sequences so a lone surrogate from a `\uD800`
//! escape survives parsing and re-serialisation, as it does in Python.

use std::cmp::Ordering;

use num_bigint::BigInt;
use num_traits::{FromPrimitive, ToPrimitive};

/// A Python `str`: Unicode code points, possibly including lone surrogates.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PyStr(pub Vec<u32>);

impl From<&str> for PyStr {
    fn from(text: &str) -> PyStr {
        PyStr(text.chars().map(|ch| ch as u32).collect())
    }
}

impl PyStr {
    /// A str from an OS string's text, with carried code points back as the
    /// lone surrogates Python held (see `pytext::carried`).
    pub fn from_os_text(text: &str) -> PyStr {
        PyStr(
            text.chars()
                .map(|ch| crate::pytext::surrogate_of(ch).unwrap_or(ch as u32))
                .collect(),
        )
    }

    /// The reverse of `from_os_text`: lone surrogates as carried code points.
    pub fn os_text(&self) -> String {
        self.0
            .iter()
            .map(|&cp| char::from_u32(cp).unwrap_or_else(|| crate::pytext::carried(cp)))
            .collect()
    }

    /// The text, or `None` when it holds a lone surrogate (not encodable as UTF-8).
    pub fn to_utf8(&self) -> Option<String> {
        self.0.iter().map(|&cp| char::from_u32(cp)).collect()
    }

    pub fn is(&self, text: &str) -> bool {
        self.0.iter().copied().eq(text.chars().map(|ch| ch as u32))
    }
}

#[derive(Clone, Debug)]
pub enum Json {
    Null,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    Str(PyStr),
    Array(Vec<Json>),
    Object(Vec<(PyStr, Json)>),
}

impl Json {
    pub fn str(text: &str) -> Json {
        Json::Str(PyStr::from(text))
    }

    pub fn int(value: i64) -> Json {
        Json::Int(BigInt::from(value))
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

    /// `obj[key]` for a key the caller has already proved present.
    pub fn at(&self, key: &str) -> &Json {
        self.get(key).unwrap_or_else(|| panic!("key {key} checked present"))
    }

    pub fn as_pystr(&self) -> Option<&PyStr> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn keys(&self) -> Option<Vec<&PyStr>> {
        match self {
            Json::Object(pairs) => Some(pairs.iter().map(|(k, _)| k).collect()),
            _ => None,
        }
    }

    /// `set(obj) == required` for a dict; false for anything else.
    pub fn has_exact_keys(&self, required: &[&str]) -> bool {
        match self.keys() {
            Some(keys) => keys.len() == required.len() && required.iter().all(|r| keys.iter().any(|k| k.is(r))),
            None => false,
        }
    }

    pub fn is_str(&self, text: &str) -> bool {
        matches!(self, Json::Str(s) if s.is(text))
    }

    /// `isinstance(value, int) and not isinstance(value, bool)`, as an i64 when it fits.
    pub fn as_plain_int(&self) -> Option<&BigInt> {
        match self {
            Json::Int(i) => Some(i),
            _ => None,
        }
    }
}

/// Python `==` between two parsed values. `nested` applies the identity
/// shortcut that container comparison uses: every NaN json.loads produces is
/// the same float object, so `[NaN] == [NaN]` while `NaN != NaN`.
pub fn py_eq(a: &Json, b: &Json, nested: bool) -> bool {
    use Json::*;
    fn number(value: &Json) -> Option<Result<BigInt, f64>> {
        match value {
            Bool(b) => Some(Ok(BigInt::from(*b as u8))),
            Int(i) => Some(Ok(i.clone())),
            Float(f) => Some(Err(*f)),
            _ => None,
        }
    }
    match (a, b) {
        (Null, Null) => true,
        (Str(x), Str(y)) => x == y,
        (Array(x), Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| py_eq(p, q, true)),
        (Object(x), Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.iter().any(|(k2, v2)| k == k2 && py_eq(v, v2, true)))
        }
        _ => match (number(a), number(b)) {
            (Some(Ok(x)), Some(Ok(y))) => x == y,
            (Some(Err(x)), Some(Err(y))) => x == y || (nested && x.is_nan() && y.is_nan()),
            (Some(Ok(i)), Some(Err(f))) | (Some(Err(f)), Some(Ok(i))) => {
                f.is_finite() && f.fract() == 0.0 && BigInt::from_f64(f).is_some_and(|g| g == i)
            }
            _ => false,
        },
    }
}

/// Why `json.loads` with the gateway's hook did not return a value.
#[derive(Debug, PartialEq)]
pub enum LoadError {
    /// `json.JSONDecodeError`.
    Invalid,
    /// The object_pairs_hook refused a key spelt twice.
    DuplicateKey(PyStr),
    /// An uncaught exception in CPython (integer digit limit, recursion), which exits 1.
    Crash(String),
}

const MAX_INT_DIGITS: usize = 4300;
const MAX_DEPTH: usize = 10_000;

type Step<'a> = fn(&mut Parser<'a>, usize) -> Result<(Json, usize), LoadError>;

struct Parser<'a> {
    text: &'a [u32],
    depth: usize,
}

fn is_ws(cp: u32) -> bool {
    matches!(cp, 0x20 | 0x09 | 0x0A | 0x0D)
}

/// `json.loads(text, object_pairs_hook=<refuse duplicates>)`.
pub fn loads(text: &str) -> Result<Json, LoadError> {
    let cps: Vec<u32> = text.chars().map(|ch| ch as u32).collect();
    if cps.first() == Some(&0xFEFF) {
        return Err(LoadError::Invalid);
    }
    let mut parser = Parser { text: &cps, depth: 0 };
    let start = parser.skip_ws(0);
    let (value, end) = parser.value(start)?;
    if parser.skip_ws(end) != cps.len() {
        return Err(LoadError::Invalid);
    }
    Ok(value)
}

impl<'a> Parser<'a> {
    fn at(&self, idx: usize) -> Option<u32> {
        self.text.get(idx).copied()
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
        let c = self.at(idx).ok_or(LoadError::Invalid)?;
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
            return Err(LoadError::Crash(
                "maximum recursion depth exceeded while decoding a JSON document".into(),
            ));
        }
        let result = parse(self, idx + 1);
        self.depth -= 1;
        result
    }

    fn number(&self, start: usize) -> Result<(Json, usize), LoadError> {
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
            _ => return Err(LoadError::Invalid),
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
        let numstr: String = self.text[start..idx]
            .iter()
            .map(|&c| char::from_u32(c).unwrap())
            .collect();
        if is_float {
            return Ok((Json::Float(numstr.parse().expect("JSON float grammar")), idx));
        }
        let digits = numstr.trim_start_matches('-').len();
        if digits > MAX_INT_DIGITS {
            return Err(LoadError::Crash(format!(
                "Exceeds the limit ({MAX_INT_DIGITS} digits) for integer string conversion: value has {digits} digits"
            )));
        }
        Ok((Json::Int(numstr.parse().expect("JSON integer grammar")), idx))
    }

    fn string(&self, begin: usize) -> Result<(PyStr, usize), LoadError> {
        let mut out = Vec::new();
        let mut idx = begin;
        let len = self.text.len();
        loop {
            let c = self.at(idx).ok_or(LoadError::Invalid)?;
            idx += 1;
            match c {
                0x22 => return Ok((PyStr(out), idx)),
                0x5C => {}
                c if c <= 0x1F => return Err(LoadError::Invalid),
                c => {
                    out.push(c);
                    continue;
                }
            }
            let escape = self.at(idx).ok_or(LoadError::Invalid)?;
            if escape != 'u' as u32 {
                out.push(match char::from_u32(escape) {
                    Some('"') => 0x22,
                    Some('\\') => 0x5C,
                    Some('/') => 0x2F,
                    Some('b') => 0x08,
                    Some('f') => 0x0C,
                    Some('n') => 0x0A,
                    Some('r') => 0x0D,
                    Some('t') => 0x09,
                    _ => return Err(LoadError::Invalid),
                });
                idx += 1;
                continue;
            }
            // CPython requires a character after the 4 hex digits.
            let end = idx + 5;
            if end >= len {
                return Err(LoadError::Invalid);
            }
            let mut unit = self.hex4(idx + 1)?;
            let mut next = end;
            if (0xD800..=0xDBFF).contains(&unit)
                && end + 6 < len
                && self.at(end) == Some(0x5C)
                && self.at(end + 1) == Some('u' as u32)
            {
                let low = self.hex4(end + 2)?;
                if (0xDC00..=0xDFFF).contains(&low) {
                    unit = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
                    next = end + 6;
                }
            }
            out.push(unit);
            idx = next;
        }
    }

    fn hex4(&self, idx: usize) -> Result<u32, LoadError> {
        let mut value = 0;
        for i in 0..4 {
            let digit = self.at(idx + i).and_then(char::from_u32).and_then(|c| c.to_digit(16));
            value = value * 16 + digit.ok_or(LoadError::Invalid)?;
        }
        Ok(value)
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
                Some(',') => idx = self.skip_ws(idx + 1),
                _ => return Err(LoadError::Invalid),
            }
            if self.at(idx) == Some(']' as u32) {
                return Err(LoadError::Invalid);
            }
        }
    }

    fn object(&mut self, idx: usize) -> Result<(Json, usize), LoadError> {
        let mut pairs: Vec<(PyStr, Json)> = Vec::new();
        let mut idx = self.skip_ws(idx);
        if self.at(idx) != Some('}' as u32) {
            loop {
                if self.at(idx) != Some('"' as u32) {
                    return Err(LoadError::Invalid);
                }
                let (key, next) = self.string(idx + 1)?;
                idx = self.skip_ws(next);
                if self.at(idx) != Some(':' as u32) {
                    return Err(LoadError::Invalid);
                }
                idx = self.skip_ws(idx + 1);
                let (value, next) = self.value(idx)?;
                pairs.push((key, value));
                idx = self.skip_ws(next);
                match self.at(idx).and_then(char::from_u32) {
                    Some('}') => break,
                    Some(',') => idx = self.skip_ws(idx + 1),
                    _ => return Err(LoadError::Invalid),
                }
                if self.at(idx) == Some('}' as u32) {
                    return Err(LoadError::Invalid);
                }
            }
        }
        // The hook runs once the object is complete, before parsing continues.
        for (i, (key, _)) in pairs.iter().enumerate() {
            if pairs[..i].iter().any(|(earlier, _)| earlier == key) {
                return Err(LoadError::DuplicateKey(key.clone()));
            }
        }
        Ok((Json::Object(pairs), idx + 1))
    }
}

/// `float.__repr__`, the shortest round-tripping form in Python's layout.
pub fn float_repr(value: f64) -> String {
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    let sci = format!("{value:e}");
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

fn sorted_pairs(pairs: &[(PyStr, Json)]) -> Vec<&(PyStr, Json)> {
    let mut sorted: Vec<_> = pairs.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted
}

fn write_scalar(out: &mut String, value: &Json) -> bool {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Int(i) => out.push_str(&i.to_string()),
        Json::Float(f) if f.is_nan() => out.push_str("NaN"),
        Json::Float(f) if f.is_infinite() => out.push_str(if *f > 0.0 { "Infinity" } else { "-Infinity" }),
        Json::Float(f) => out.push_str(&float_repr(*f)),
        Json::Str(s) => write_str(out, s),
        _ => return false,
    }
    true
}

/// `json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)`.
pub fn canonical(value: &Json) -> String {
    fn go(out: &mut String, value: &Json) {
        if write_scalar(out, value) {
            return;
        }
        match value {
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    go(out, item);
                }
                out.push(']');
            }
            Json::Object(pairs) => {
                out.push('{');
                for (i, (key, item)) in sorted_pairs(pairs).into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(out, key);
                    out.push(':');
                    go(out, item);
                }
                out.push('}');
            }
            _ => unreachable!("scalars are written above"),
        }
    }
    let mut out = String::new();
    go(&mut out, value);
    out
}

/// `json.dumps(value, indent=2, sort_keys=True)`.
pub fn pretty(value: &Json) -> String {
    fn go(out: &mut String, value: &Json, level: usize) {
        if write_scalar(out, value) {
            return;
        }
        let inner = format!("\n{}", "  ".repeat(level + 1));
        let outer = format!("\n{}", "  ".repeat(level));
        match value {
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
                for (i, (key, item)) in sorted_pairs(pairs).into_iter().enumerate() {
                    out.push_str(if i > 0 { "," } else { "" });
                    out.push_str(&inner);
                    write_str(out, key);
                    out.push_str(": ");
                    go(out, item, level + 1);
                }
                out.push_str(&outer);
                out.push('}');
            }
            _ => unreachable!("scalars are written above"),
        }
    }
    let mut out = String::new();
    go(&mut out, value, 0);
    out
}

/// Ordering for `max_results` style bounds on a parsed integer.
pub fn int_cmp(value: &BigInt, bound: i64) -> Ordering {
    value.to_i64().map_or_else(
        || {
            if value.sign() == num_bigint::Sign::Minus {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        },
        |v| v.cmp(&bound),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_repr_matches_python() {
        for (value, text) in [
            (1.0, "1.0"),
            (1e16, "1e+16"),
            (9999999999999998.0, "9999999999999998.0"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (1.5e-5, "1.5e-05"),
            (123456789012345678.0, "1.2345678901234568e+17"),
            (-0.0, "-0.0"),
            (0.1, "0.1"),
            (1e22, "1e+22"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
        ] {
            assert_eq!(float_repr(value), text);
        }
    }

    #[test]
    fn loads_follows_cpython() {
        assert!(loads("{\"a\": NaN}").is_ok());
        assert_eq!(loads("\u{feff}{}").unwrap_err(), LoadError::Invalid);
        assert_eq!(loads("{\"a\":1,}").unwrap_err(), LoadError::Invalid);
        assert_eq!(loads("[1,]").unwrap_err(), LoadError::Invalid);
        assert_eq!(loads("\"\u{1}\"").unwrap_err(), LoadError::Invalid);
        assert_eq!(
            loads("{\"a\":{\"b\":1,\"b\":2}, x}").unwrap_err(),
            LoadError::DuplicateKey(PyStr::from("b"))
        );
        assert_eq!(loads("{\"a\":1,\"a\":2, x}").unwrap_err(), LoadError::Invalid);
        let lone = loads("\"\\ud800x\"").unwrap();
        assert_eq!(lone.as_pystr().unwrap().0, vec![0xD800, 'x' as u32]);
        let pair = loads("\"\\ud83d\\ude00\"").unwrap();
        assert_eq!(pair.as_pystr().unwrap().0, vec![0x1F600]);
        assert!(matches!(
            loads(&format!("[{}]", "1".repeat(4301))),
            Err(LoadError::Crash(_))
        ));
        assert!(matches!(loads("1e5"), Ok(Json::Float(f)) if f == 100000.0));
        assert!(matches!(loads("1.5E+2"), Ok(Json::Float(_))));
        assert_eq!(loads("01").unwrap_err(), LoadError::Invalid);
        assert_eq!(loads("1.").unwrap_err(), LoadError::Invalid);
    }

    #[test]
    fn dumps_follows_cpython() {
        let value = loads("{\"b\":[1,2.5,\"\\u00e9\\ud800\\ud83d\\ude00\\u007f\"],\"a\":{},\"c\":[]}").unwrap();
        assert_eq!(
            canonical(&value),
            "{\"a\":{},\"b\":[1,2.5,\"\\u00e9\\ud800\\ud83d\\ude00\\u007f\"],\"c\":[]}"
        );
        assert_eq!(
            pretty(&value),
            "{\n  \"a\": {},\n  \"b\": [\n    1,\n    2.5,\n    \"\\u00e9\\ud800\\ud83d\\ude00\\u007f\"\n  ],\n  \"c\": []\n}"
        );
    }

    #[test]
    fn equality_follows_python() {
        let nan = loads("[NaN]").unwrap();
        assert!(py_eq(&nan, &nan.clone(), false));
        assert!(!py_eq(&Json::Float(f64::NAN), &Json::Float(f64::NAN), false));
        assert!(py_eq(&loads("[1, true]").unwrap(), &loads("[1.0, 1]").unwrap(), false));
        assert!(!py_eq(
            &loads("9007199254740993").unwrap(),
            &Json::Float(9007199254740992.0),
            false
        ));
        assert!(py_eq(
            &loads("{\"a\":1,\"b\":2}").unwrap(),
            &loads("{\"b\":2,\"a\":1}").unwrap(),
            false
        ));
    }
}
