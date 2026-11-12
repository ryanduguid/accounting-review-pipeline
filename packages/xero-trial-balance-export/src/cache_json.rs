// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Python token-cache JSON preserves object insertion order and Unicode text.
use crate::{
    Failure, Result,
    pyjson::{self, Json, PyStr},
};

pub fn dumps(value: &Json, pretty: bool, sorted: bool, ascii: bool) -> Result<Vec<u8>> {
    let mut out = String::new();
    write(&mut out, value, 0, pretty, sorted, ascii)?;
    if pretty {
        out.push('\n');
    }
    Ok(out.into_bytes())
}

fn string(out: &mut String, value: &PyStr, ascii: bool) -> Result<()> {
    out.push('"');
    for &cp in &value.0 {
        match cp {
            0x22 => out.push_str("\\\""),
            0x5c => out.push_str("\\\\"),
            8 => out.push_str("\\b"),
            9 => out.push_str("\\t"),
            10 => out.push_str("\\n"),
            12 => out.push_str("\\f"),
            13 => out.push_str("\\r"),
            0..=31 => out.push_str(&format!("\\u{cp:04x}")),
            _ if ascii && cp > 127 => {
                if cp <= 0xffff {
                    out.push_str(&format!("\\u{cp:04x}"));
                } else {
                    let n = cp - 0x10000;
                    out.push_str(&format!(
                        "\\u{:04x}\\u{:04x}",
                        0xd800 + (n >> 10),
                        0xdc00 + (n & 0x3ff)
                    ));
                }
            }
            _ => out.push(char::from_u32(cp).ok_or_else(|| {
                Failure::message("error: token cache text is not valid UTF-8; no token was saved.")
            })?),
        }
    }
    out.push('"');
    Ok(())
}

fn write(
    out: &mut String,
    value: &Json,
    level: usize,
    pretty: bool,
    sorted: bool,
    ascii: bool,
) -> Result<()> {
    let separator = if pretty { ": " } else { ":" };
    match value {
        Json::Str(s) => string(out, s, ascii)?,
        Json::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                item_prefix(out, i, level, pretty);
                write(out, item, level + 1, pretty, sorted, ascii)?;
            }
            close(out, ']', items.is_empty(), level, pretty);
        }
        Json::Object(pairs) => {
            let mut pairs: Vec<_> = pairs.iter().collect();
            if sorted {
                pairs.sort_by(|a, b| a.0.cmp(&b.0));
            }
            out.push('{');
            for (i, (key, item)) in pairs.iter().enumerate() {
                item_prefix(out, i, level, pretty);
                string(out, key, ascii)?;
                out.push_str(separator);
                write(out, item, level + 1, pretty, sorted, ascii)?;
            }
            close(out, '}', pairs.is_empty(), level, pretty);
        }
        _ => out.push_str(&pyjson::canonical(value)),
    }
    Ok(())
}

fn item_prefix(out: &mut String, index: usize, level: usize, pretty: bool) {
    if index > 0 {
        out.push(',');
    }
    if pretty {
        out.push('\n');
        out.push_str(&"  ".repeat(level + 1));
    }
}

fn close(out: &mut String, delimiter: char, empty: bool, level: usize, pretty: bool) {
    if pretty && !empty {
        out.push('\n');
        out.push_str(&"  ".repeat(level));
    }
    out.push(delimiter);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_json_uses_insertion_order_and_native_unicode() {
        let value = Json::object([("z", Json::str("é")), ("a", Json::int(2))]);
        assert_eq!(
            dumps(&value, true, false, false).unwrap(),
            "{\n  \"z\": \"é\",\n  \"a\": 2\n}\n".as_bytes()
        );
        assert_eq!(
            dumps(&value, false, true, false).unwrap(),
            "{\"a\":2,\"z\":\"é\"}".as_bytes()
        );
    }

    #[test]
    fn cache_text_escapes_controls_and_optional_ascii_in_keys_and_values() {
        let text = "\"\\\u{8}\t\n\u{c}\r\0\u{1f}é😀";
        let value = Json::object([(text, Json::str(text))]);
        for (ascii, escaped) in [
            (false, r#"\"\\\b\t\n\f\r\u0000\u001fé😀"#),
            (true, r#"\"\\\b\t\n\f\r\u0000\u001f\u00e9\ud83d\ude00"#),
        ] {
            assert_eq!(
                dumps(&value, false, false, ascii).unwrap(),
                format!("{{\"{escaped}\":\"{escaped}\"}}").as_bytes()
            );
        }
    }

    #[test]
    fn sorted_nested_cache_json_keeps_empty_collections_and_indentation() {
        let value = Json::object([
            (
                "z",
                Json::Array(vec![
                    Json::object([("β", Json::str("😀")), ("a", Json::Null)]),
                    Json::Array(vec![]),
                ]),
            ),
            ("a", Json::object([])),
        ]);
        assert_eq!(
            dumps(&value, false, true, true).unwrap(),
            br#"{"a":{},"z":[{"a":null,"\u03b2":"\ud83d\ude00"},[]]}"#
        );
        assert_eq!(
            dumps(&value, true, true, true).unwrap(),
            concat!(
                "{\n  \"a\": {},\n  \"z\": [\n    {\n",
                "      \"a\": null,\n      \"\\u03b2\": \"\\ud83d\\ude00\"\n",
                "    },\n    []\n  ]\n}\n"
            )
            .as_bytes()
        );
    }

    #[test]
    fn lone_surrogates_need_ascii_escaping_before_a_cache_can_be_saved() {
        let surrogate = PyStr(vec![0xd800]);
        let cases = [
            (Json::Str(surrogate.clone()), br#""\ud800""#.as_slice()),
            (
                Json::Object(vec![(surrogate, Json::str("x"))]),
                br#"{"\ud800":"x"}"#.as_slice(),
            ),
        ];
        for (value, escaped) in cases {
            assert_eq!(dumps(&value, false, false, true).unwrap(), escaped);
            assert_eq!(
                dumps(&value, false, false, false).unwrap_err(),
                Failure::message("error: token cache text is not valid UTF-8; no token was saved.")
            );
        }
    }
}
