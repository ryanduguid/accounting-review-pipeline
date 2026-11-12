// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Python string representation reused from Monthly Close.
use super::tables::PY_NOT_PRINTABLE;
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
const ESCAPE: char = '\u{10F7FF}';
const CARRY_OFFSET: u32 = 0x10_F800 - 0xD800;

/// Append a code point (a Unicode scalar value or a lone surrogate) to OS-derived text.
pub fn push_code_point(out: &mut String, cp: u32) {
    match char::from_u32(cp) {
        Some(ESCAPE) => out.extend([ESCAPE, ESCAPE]),
        Some(ch) => out.push(ch),
        None => out.extend([
            ESCAPE,
            char::from_u32(cp + CARRY_OFFSET).expect("surrogates map to scalar values"),
        ]),
    }
}

/// The code points text stands for, reading the escapes `push_code_point` writes.
pub fn code_points(text: &str) -> impl Iterator<Item = u32> + '_ {
    let mut chars = text.chars().peekable();
    std::iter::from_fn(move || {
        let ch = chars.next()?;
        if ch != ESCAPE {
            return Some(ch as u32);
        }
        match chars.peek().copied() {
            Some(ESCAPE) => {
                chars.next();
                Some(ESCAPE as u32)
            }
            Some(next) if (0x10_F800..=0x10_FFFF).contains(&(next as u32)) => {
                chars.next();
                Some(next as u32 - CARRY_OFFSET)
            }
            _ => Some(ESCAPE as u32),
        }
    })
}

pub fn is_space(cp: u32) -> bool {
    char::from_u32(cp).is_some_and(crate::transactions::text::space)
}

/// `str.strip()` with no argument.
pub fn strip(text: &str) -> &str {
    text.trim_matches(|ch: char| is_space(ch as u32))
}

/// The digit value of a Unicode decimal digit (category Nd), as `unicodedata.decimal`.
pub fn decimal_value(cp: u32) -> Option<u32> {
    let index = crate::transactions::text::DECIMAL_ZEROS.partition_point(|&zero| zero <= cp);
    let zero = *crate::transactions::text::DECIMAL_ZEROS.get(index.checked_sub(1)?)?;
    (cp - zero < 10).then_some(cp - zero)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carriers_keep_literal_private_use_text_and_surrogates_distinct() {
        let original = [0x10_F7FF, 0x10_F800, 0xD800, 0xDFFF, 0x1_F600];
        let mut encoded = String::new();
        for cp in original {
            push_code_point(&mut encoded, cp);
        }
        assert_eq!(code_points(&encoded).collect::<Vec<_>>(), original);
        for text in ["\u{10f7ff}", "\u{10f7ff}x"] {
            assert_eq!(
                code_points(text).collect::<Vec<_>>(),
                text.chars().map(|ch| ch as u32).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn whitespace_digits_and_representations_match_python_strings() {
        assert_eq!(strip("\u{a0}\t x \u{85}"), "x");
        assert!(!is_space(0xD800));
        for (cp, expected) in [(0x37, Some(7)), (0x663, Some(3)), (0x41, None), (0, None)] {
            assert_eq!(decimal_value(cp), expected);
        }
        for (points, expected) in [
            (vec![0x61, 0x27, 0x62], "\"a'b\""),
            (vec![0x61, 0x27, 0x62, 0x22, 0x63], "'a\\'b\"c'"),
            (vec![0x5C, 0x09, 0x0A, 0x0D], "'\\\\\\t\\n\\r'"),
            (
                vec![0, 0x7F, 0x2028, 0xD800, 0x10_FFFF],
                "'\\x00\\x7f\\u2028\\ud800\\U0010ffff'",
            ),
        ] {
            assert_eq!(repr_code_points(&points), expected);
        }
    }
}
