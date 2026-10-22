use super::Result;
use num_bigint::BigInt;
use serde_json::Value;

pub fn parse(data: &[u8], label: &str) -> Result<Value> {
    let data = data.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(data);
    let parse = || -> Result<Value> {
        let text = std::str::from_utf8(data).map_err(|error| error.to_string())?;
        let mut scanner = Scanner { text, offset: 0 };
        scanner.value(0)?;
        let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
        finite(&value)?;
        Ok(value)
    };
    parse().map_err(|error| format!("{label}: {error}"))
}

fn finite(value: &Value) -> Result<()> {
    match value {
        Value::Number(number) if number.to_string().contains(['.', 'e', 'E']) => {
            if !number.as_f64().is_some_and(f64::is_finite) {
                return Err("JSON number is outside the finite float range.".into());
            }
        }
        Value::Number(number) if number.to_string().trim_start_matches('-').len() > 4300 => {
            return Err("JSON integer exceeds 4300 digits.".into());
        }
        Value::Array(values) => values.iter().try_for_each(finite)?,
        Value::Object(values) => values.values().try_for_each(finite)?,
        _ => {}
    }
    Ok(())
}

struct Scanner<'a> {
    text: &'a str,
    offset: usize,
}

impl Scanner<'_> {
    fn whitespace(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.offset)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.offset += 1;
        }
    }

    fn take(&mut self, expected: u8) -> Result<()> {
        self.whitespace();
        if self.text.as_bytes().get(self.offset) != Some(&expected) {
            return Err("Invalid JSON structure.".into());
        }
        self.offset += 1;
        Ok(())
    }

    fn string(&mut self) -> Result<String> {
        self.whitespace();
        let start = self.offset;
        self.take(b'"')?;
        let mut escaped = false;
        while let Some(&byte) = self.text.as_bytes().get(self.offset) {
            self.offset += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                return serde_json::from_str(&self.text[start..self.offset]).map_err(|error| error.to_string());
            }
        }
        Err("Unterminated JSON string.".into())
    }

    fn value(&mut self, depth: usize) -> Result<()> {
        self.whitespace();
        match self.text.as_bytes().get(self.offset).copied() {
            Some(b'{') => self.object(depth + 1),
            Some(b'[') => self.array(depth + 1),
            Some(b'"') => self.string().map(|_| ()),
            Some(_) => {
                let start = self.offset;
                while self
                    .text
                    .as_bytes()
                    .get(self.offset)
                    .is_some_and(|byte| !b",]} \t\r\n".contains(byte))
                {
                    self.offset += 1;
                }
                if self.offset == start {
                    return Err("Missing JSON value.".into());
                }
                Ok(())
            }
            None => Err("Missing JSON value.".into()),
        }
    }

    fn object(&mut self, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err("JSON nesting exceeds 64 levels.".into());
        }
        self.take(b'{')?;
        let mut keys = std::collections::BTreeSet::new();
        self.whitespace();
        if self.text.as_bytes().get(self.offset) != Some(&b'}') {
            loop {
                if !keys.insert(self.string()?) {
                    return Err("Duplicate object member.".into());
                }
                self.take(b':')?;
                self.value(depth)?;
                self.whitespace();
                if self.text.as_bytes().get(self.offset) != Some(&b',') {
                    break;
                }
                self.offset += 1;
            }
        }
        self.take(b'}')
    }

    fn array(&mut self, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err("JSON nesting exceeds 64 levels.".into());
        }
        self.take(b'[')?;
        self.whitespace();
        if self.text.as_bytes().get(self.offset) != Some(&b']') {
            loop {
                self.value(depth)?;
                self.whitespace();
                if self.text.as_bytes().get(self.offset) != Some(&b',') {
                    break;
                }
                self.offset += 1;
            }
        }
        self.take(b']')
    }
}

pub fn bytes(value: &Value) -> Vec<u8> {
    let mut out = String::new();
    write(&mut out, value, Some(0), true);
    out.push('\n');
    out.into_bytes()
}

pub fn inline(value: &Value) -> String {
    let mut out = String::new();
    write(&mut out, value, None, false);
    out
}

pub fn pretty(value: &Value) -> String {
    let mut out = String::new();
    write(&mut out, value, Some(0), false);
    out
}

pub fn sort_key(value: &Value) -> String {
    let mut out = String::new();
    write(&mut out, value, None, true);
    out.chars()
        .map(|character| {
            if character.is_ascii() {
                character.to_string()
            } else {
                character
                    .encode_utf16(&mut [0; 2])
                    .iter()
                    .map(|unit| format!("\\u{unit:04x}"))
                    .collect()
            }
        })
        .collect()
}

pub fn equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).is_some_and(|other| equal(value, other)))
        }
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b)),
        (Value::Number(_) | Value::Bool(_), Value::Number(_) | Value::Bool(_)) => numeric_equal(left, right),
        _ => left == right,
    }
}

fn numeric_equal(left: &Value, right: &Value) -> bool {
    use num_traits::FromPrimitive;
    let integer = |value: &Value| match value {
        Value::Bool(value) => Some(BigInt::from(u8::from(*value))),
        Value::Number(number) if !number.to_string().contains(['.', 'e', 'E']) => {
            number.to_string().parse::<BigInt>().ok()
        }
        Value::Number(number) if number.as_f64().is_some_and(|value| value.fract() == 0.0) => {
            BigInt::from_f64(number.as_f64().unwrap())
        }
        _ => None,
    };
    match (integer(left), integer(right)) {
        (Some(a), Some(b)) => a == b,
        (None, None) => left.as_f64() == right.as_f64(),
        _ => false,
    }
}

fn write(out: &mut String, value: &Value, depth: Option<usize>, sorted: bool) {
    match value {
        Value::Object(values) => {
            out.push('{');
            let mut values: Vec<_> = values.iter().collect();
            if sorted {
                values.sort_by_key(|(key, _)| *key);
            }
            for (index, (key, value)) in values.iter().enumerate() {
                separator(out, index, depth);
                out.push_str(&serde_json::to_string(key).expect("JSON string"));
                out.push_str(": ");
                write(out, value, depth.map(|depth| depth + 1), sorted);
            }
            close(out, '}', values.is_empty(), depth);
        }
        Value::Array(values) => {
            out.push('[');
            for (index, value) in values.iter().enumerate() {
                separator(out, index, depth);
                write(out, value, depth.map(|depth| depth + 1), sorted);
            }
            close(out, ']', values.is_empty(), depth);
        }
        Value::Number(number) => {
            let raw = number.to_string();
            if raw.contains(['.', 'e', 'E']) {
                out.push_str(&float_repr(number.as_f64().expect("validated finite number")));
            } else {
                out.push_str(&raw.parse::<BigInt>().expect("JSON integer").to_string());
            }
        }
        _ => out.push_str(&serde_json::to_string(value).expect("JSON scalar")),
    }
}

fn separator(out: &mut String, index: usize, depth: Option<usize>) {
    if index != 0 {
        out.push(',');
    }
    if let Some(depth) = depth {
        out.push('\n');
        out.push_str(&"  ".repeat(depth + 1));
    } else if index != 0 {
        out.push(' ');
    }
}

fn close(out: &mut String, closing: char, empty: bool, depth: Option<usize>) {
    if !empty && let Some(depth) = depth {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    }
    out.push(closing);
}

fn shortest_half_even(value: f64, digits: String, scale: i32) -> String {
    if (digits.as_bytes().last().unwrap() - b'0').is_multiple_of(2) {
        return digits;
    }
    // Resolve shortest-decimal ties with Python's even choice.
    let bits = value.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let (significand, binary_scale) = if exponent == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1u64 << 52), exponent - 1023 - 52)
    };
    let mut exact = BigInt::from(significand) * 2;
    let mut unit = BigInt::from(1);
    if binary_scale >= 0 {
        exact <<= binary_scale as usize;
    } else {
        unit <<= (-binary_scale) as usize;
    }
    if scale >= 0 {
        unit *= BigInt::from(10).pow(scale as u32);
    } else {
        exact *= BigInt::from(10).pow((-scale) as u32);
    }
    let coefficient = BigInt::parse_bytes(digits.as_bytes(), 10).unwrap();
    let centre = &coefficient * 2 * &unit;
    let candidate = if exact == &centre - &unit {
        (coefficient - 1u32).to_string()
    } else if exact == centre + unit {
        (coefficient + 1u32).to_string()
    } else {
        return digits;
    };
    if format!("{candidate}e{scale}").parse::<f64>().ok() == Some(value.abs()) {
        candidate
    } else {
        digits
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
    let scale = exponent - digits.len() as i32 + 1;
    let digits = shortest_half_even(value, digits, scale);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_json_refuses_ambiguous_and_unbounded_values() {
        for invalid in [
            "{\"a\":1,\"a\":2}",
            "[NaN]",
            "[Infinity]",
            "[1e999]",
            "[1,]",
            "{\"a\":1} null",
        ] {
            assert!(parse(invalid.as_bytes(), "test").is_err(), "{invalid}");
        }
        let deep = format!("{}0{}", "[".repeat(65), "]".repeat(65));
        assert!(parse(deep.as_bytes(), "test").is_err());
        let shallow = format!("{}0{}", "[".repeat(64), "]".repeat(64));
        assert!(parse(shallow.as_bytes(), "test").is_ok());
    }

    #[test]
    fn canonical_bytes_match_python_layout_and_number_types() {
        let value = parse(br#"{"z":[],"text":"\u00e9","b":1.0,"a":-2.9802322387695312e-8,"integer":10000000000000000000000000000000000000000}"#, "test").unwrap();
        assert_eq!(
            String::from_utf8(bytes(&value)).unwrap(),
            "{\n  \"a\": -2.9802322387695312e-08,\n  \"b\": 1.0,\n  \"integer\": 10000000000000000000000000000000000000000,\n  \"text\": \"é\",\n  \"z\": []\n}\n"
        );
        assert_eq!(inline(&parse(b"[1, true, {}]", "test").unwrap()), "[1, true, {}]");
        assert_eq!(
            pretty(&parse(br#"{"z":0,"a":[1]}"#, "test").unwrap()),
            "{\n  \"z\": 0,\n  \"a\": [\n    1\n  ]\n}"
        );
        assert_eq!(
            sort_key(&parse(br#"{"z":"\ud83d\ude00","a":1}"#, "test").unwrap()),
            r#"{"a": 1, "z": "\ud83d\ude00"}"#
        );
        assert!(equal(&Value::Bool(true), &serde_json::json!(1.0)));
        assert!(!equal(
            &parse(b"9007199254740993", "test").unwrap(),
            &serde_json::json!(9007199254740992.0)
        ));
    }
}
