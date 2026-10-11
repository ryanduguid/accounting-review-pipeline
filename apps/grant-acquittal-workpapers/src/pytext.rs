//! CPython text rules the workpaper depends on: `str.isspace`, decimal digit
//! values, `unicodedata.category` and `repr()` of a string.

use crate::tables::{CATEGORY, DECIMAL_ZEROS, PY_NOT_PRINTABLE, PY_SPACE};

fn in_ranges(table: &[(u32, u32)], cp: u32) -> bool {
    table
        .binary_search_by(|&(low, high)| {
            if high < cp {
                std::cmp::Ordering::Less
            } else if low > cp {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Rust strings cannot hold the lone surrogates a Python str can (command-line
/// bytes decoded with surrogateescape, or unpaired UTF-16 on Windows). They
/// are carried as the last 2,048 code points of plane 16's private use area,
/// U+10F800 + (surrogate - U+D800).
/// shortcut: an argument that already holds one of those private-use code
/// points prints as a surrogate, where Python would print it as `\U0010f...`.
const CARRY_OFFSET: u32 = 0x10_F800 - 0xD800;

pub fn carried(surrogate: u32) -> char {
    char::from_u32(surrogate + CARRY_OFFSET).expect("U+10F800..U+10FFFF are scalar values")
}

/// The surrogate a carried code point stands for.
pub fn surrogate_of(ch: char) -> Option<u32> {
    (0x10_F800..=0x10_FFFF)
        .contains(&(ch as u32))
        .then(|| ch as u32 - CARRY_OFFSET)
}

pub fn is_space(cp: u32) -> bool {
    in_ranges(PY_SPACE, cp)
}

/// `str.strip()` with no argument.
pub fn strip(text: &str) -> &str {
    text.trim_matches(|ch: char| is_space(ch as u32))
}

/// `str.lstrip()` with no argument.
pub fn strip_start(text: &str) -> &str {
    text.trim_start_matches(|ch: char| is_space(ch as u32))
}

/// The digit value of a Unicode decimal digit (category Nd), as `unicodedata.decimal`.
pub fn decimal_value(cp: u32) -> Option<u32> {
    let index = DECIMAL_ZEROS.partition_point(|&zero| zero <= cp);
    let zero = *DECIMAL_ZEROS.get(index.checked_sub(1)?)?;
    (cp - zero < 10).then_some(cp - zero)
}

/// `unicodedata.category(ch)` reduced to its first letter, except `Zl` and `Zp`.
pub fn category(cp: u32) -> &'static str {
    let index = CATEGORY.partition_point(|&(low, _, _)| low <= cp);
    CATEGORY[index - 1].2
}

/// `repr()` of a Python string given as code points (lone surrogates allowed).
pub fn repr_code_points(cps: &[u32]) -> String {
    let has_single = cps.contains(&('\'' as u32));
    let has_double = cps.contains(&('"' as u32));
    let quote = if has_single && !has_double { '"' } else { '\'' };
    let mut out = String::with_capacity(cps.len() + 2);
    out.push(quote);
    for &cp in cps {
        match cp {
            0x5C => out.push_str("\\\\"),
            0x09 => out.push_str("\\t"),
            0x0A => out.push_str("\\n"),
            0x0D => out.push_str("\\r"),
            _ if cp == quote as u32 => {
                out.push('\\');
                out.push(quote);
            }
            _ if in_ranges(PY_NOT_PRINTABLE, cp) => {
                if cp < 0x100 {
                    out.push_str(&format!("\\x{cp:02x}"));
                } else if cp < 0x10000 {
                    out.push_str(&format!("\\u{cp:04x}"));
                } else {
                    out.push_str(&format!("\\U{cp:08x}"));
                }
            }
            _ => out.push(char::from_u32(cp).expect("printable code points are scalar values")),
        }
    }
    out.push(quote);
    out
}

pub fn repr(text: &str) -> String {
    let cps: Vec<u32> = text.chars().map(|ch| surrogate_of(ch).unwrap_or(ch as u32)).collect();
    repr_code_points(&cps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_match_unicodedata() {
        assert_eq!(category('A' as u32), "L");
        assert_eq!(category(0x0301), "M");
        assert_eq!(category(0x200B), "C");
        assert_eq!(category(0x2028), "Zl");
        assert_eq!(category(0x00A0), "Z");
        assert_eq!(category(0xD800), "C");
        assert_eq!(category(0x10FFFF), "C");
    }

    #[test]
    fn strip_uses_python_whitespace() {
        assert_eq!(strip("\u{1c} a \u{1f}"), "a");
        assert_eq!(strip("\u{a0}a\u{3000}"), "a");
        assert_eq!(strip("\u{200b}a"), "\u{200b}a");
    }

    #[test]
    fn decimal_values_cover_unicode_digits() {
        assert_eq!(decimal_value('7' as u32), Some(7));
        assert_eq!(decimal_value(0x0663), Some(3));
        assert_eq!(decimal_value(0xFF12), Some(2));
        assert_eq!(decimal_value(0x1D7CF), Some(1));
        assert_eq!(decimal_value('a' as u32), None);
        assert_eq!(decimal_value(0x00B2), None);
    }

    #[test]
    fn repr_matches_python() {
        assert_eq!(repr("200"), "'200'");
        assert_eq!(repr("it's"), "\"it's\"");
        assert_eq!(repr("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(repr("x\u{85}y\u{a0}z\u{200b}"), "'x\\x85y\\xa0z\\u200b'");
        assert_eq!(repr("caf\u{e9}"), "'caf\u{e9}'");
        assert_eq!(repr_code_points(&[0xD800]), "'\\ud800'");
        assert_eq!(repr("\u{e0001}"), "'\\U000e0001'");
    }
}
