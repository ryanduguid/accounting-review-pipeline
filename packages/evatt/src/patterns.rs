//! Frozen evatt/patterns.py at snapshot efa253847e4ecdca5ae9b4abec045c0b16b6835c. Expressions originally copied
//! from au-tax-legislation-corpus/fadden/pii_patterns.py; preserve that provenance.
//! Internal regex priority is independent of the later span arbitration.

use crate::{pytext, regexes};
use fancy_regex::{Captures, Regex};
use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

pub type Result<T> = std::result::Result<T, String>;

static COMPILED: LazyLock<HashMap<&'static str, Regex>> = LazyLock::new(|| {
    // Labelled expressions have a constant number of qualifier/whole-markup
    // parses. Mandatory tokens separate whitespace runs, and terminal pins
    // consume whole emphasis runs. At most a constant number of alternatives
    // revisit each character at a label; captures hold at most eleven digits.
    // See DECISIONS.md for the grammar argument. EMAIL is a bounded scanner.
    regexes::PATTERNS
        .iter()
        .map(|&(name, expression)| {
            let regex = fancy_regex::RegexBuilder::new(&regexes::expand(expression))
                .backtrack_limit(100_000_000)
                .build()
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            (name, regex)
        })
        .collect()
});

pub fn pattern(name: &str) -> &'static Regex {
    &COMPILED[name]
}

pub fn runtime(error: fancy_regex::Error) -> String {
    format!("detector refused input: {error}")
}

pub fn matches<'a>(name: &str, text: &'a str) -> Result<Vec<Captures<'a, str>>> {
    checked_matches(pattern(name), text)
}

fn checked_matches<'a>(regex: &Regex, text: &'a str) -> Result<Vec<Captures<'a, str>>> {
    regex.captures_iter(text).map(|item| item.map_err(runtime)).collect()
}

pub fn search<'a>(name: &str, text: &'a str, pos: usize) -> Result<Option<Captures<'a, str>>> {
    // from_pos preserves the whole haystack for every lookbehind.
    pattern(name).captures_from_pos(text, pos).map_err(runtime)
}

pub fn has(name: &str, text: &str) -> Result<bool> {
    pattern(name).is_match(text).map_err(runtime)
}

pub fn full(name: &str, text: &str) -> Result<bool> {
    Ok(search(name, text, 0)?.is_some_and(|m| {
        let whole = m.get(0).unwrap();
        whole.start() == 0 && whole.end() == text.len()
    }))
}

pub fn digits(value: &str) -> String {
    value.chars().filter(|&c| pytext::is_digit(c as u32)).collect()
}

pub fn valid(kind: &str, digits: &str) -> Result<bool> {
    let expected = match kind {
        "abn" => 11,
        "medicare" => 10,
        "bsb" => 6,
        _ => 9,
    };
    if digits.chars().count() != expected || !digits.chars().all(|c| pytext::is_digit(c as u32)) {
        return Ok(false);
    }
    if kind == "bsb" {
        return Ok(true);
    }
    if kind == "medicare" && !digits.starts_with(['2', '3', '4', '5', '6']) {
        return Ok(false);
    }
    let values: Vec<i32> = digits
        .chars()
        .map(|c| {
            pytext::decimal_value(c as u32)
                .map(|n| n as i32)
                .ok_or_else(|| "invalid literal for int() in identifier checksum".to_string())
        })
        .collect::<Result<_>>()?;
    let weights: &[i32] = match kind {
        "tfn" => &[1, 4, 3, 7, 5, 8, 6, 9, 10],
        "abn" => &[10, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19],
        "acn" => &[8, 7, 6, 5, 4, 3, 2, 1],
        "medicare" => &[1, 3, 7, 9, 1, 3, 7, 9],
        _ => return Ok(true),
    };
    let sum: i32 = values.iter().zip(weights).map(|(v, w)| v * w).sum();
    Ok(match kind {
        "tfn" => sum % 11 == 0,
        "abn" => (sum - 10) % 89 == 0,
        "acn" => (10 - sum % 10) % 10 == values[8],
        "medicare" => sum % 10 == values[8],
        _ => true,
    })
}

pub const STRUCTURED: &[(&str, &str)] = &[
    ("abn", "ABN_LABELLED"),
    ("medicare", "MEDICARE_LABELLED"),
    ("tfn", "TFN_LABELLED"),
    ("acn", "ACN_LABELLED"),
    ("bsb", "BSB_LABELLED"),
    ("abn", "ABN"),
    ("medicare", "MEDICARE"),
    ("tfn", "TFN_BARE"),
    ("acn", "ACN"),
    ("bsb", "BSB"),
    ("email", "EMAIL"),
    ("phone", "PHONE"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ambiguous {
    pub start: usize,
    pub digit_start: usize,
    pub end: usize,
    pub value: String,
}

pub fn ambiguous(text: &str) -> Result<Vec<Ambiguous>> {
    // The complete marker index is built once, then queried by binary search.
    // Each identifier capture has <=11 digits and <=2 characters per gap.
    let markers: Vec<_> = matches("_LINE_LIST_MARKER", text)?
        .iter()
        .map(|m| m.get(0).unwrap().start())
        .collect();
    let mut found = BTreeMap::new();
    let kinds = ["abn", "medicare", "tfn", "acn", "abn", "medicare", "tfn", "acn"];
    for (i, kind) in kinds.iter().enumerate() {
        for m in matches(&format!("PROBE_{i}"), text)? {
            let d = m.get(1).unwrap();
            let nearest = markers.partition_point(|&s| s < d.start());
            if markers.get(nearest).is_none_or(|&s| s >= d.end()) {
                continue;
            }
            if i >= 4 && !valid(kind, &digits(d.as_str()))? {
                continue;
            }
            let key = (m.get(0).unwrap().start(), d.start(), d.end());
            found.insert(
                key,
                Ambiguous {
                    start: key.0,
                    digit_start: key.1,
                    end: key.2,
                    value: d.as_str().into(),
                },
            );
        }
    }
    let mut values: Vec<_> = found.into_values().collect();
    values.sort_by_key(|a| (a.start, a.end));
    Ok(values)
}

pub fn merge(mut intervals: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    intervals.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (start, end) in intervals {
        if let Some(last) = merged.last_mut()
            && start <= last.1
        {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
}

pub fn intersects(intervals: &[(usize, usize)], start: usize, end: usize) -> bool {
    let index = intervals.partition_point(|&(s, _)| s < end);
    index > 0 && intervals[index - 1].1 > start
}

pub fn structured_spans(text: &str) -> Result<Vec<Span>> {
    let protected = merge(ambiguous(text)?.iter().map(|a| (a.start, a.end)).collect());
    let mut found = Vec::new();
    for (priority, &(kind, name)) in STRUCTURED.iter().enumerate() {
        if kind == "email" {
            for span in crate::email::spans(text) {
                if !intersects(&protected, span.start, span.end) {
                    found.push((span.start, span.end, kind, priority));
                }
            }
            continue;
        }
        for m in matches(name, text)? {
            let group = usize::from(m.len() > 1);
            let d = m.get(group).unwrap();
            if (5..10).contains(&priority) && !valid(kind, &digits(d.as_str()))? {
                continue;
            }
            let end = if kind == "medicare" {
                m.name("reference").map_or(d.end(), |r| r.end())
            } else {
                d.end()
            };
            if intersects(&protected, d.start(), end) {
                continue;
            }
            found.push((d.start(), end, kind, priority));
        }
    }
    found.sort_by_key(|&(s, e, _, p)| (s, std::cmp::Reverse(e - s), p));
    let mut taken = Vec::new();
    let mut consumed = 0;
    for (start, end, kind, _) in found {
        if end <= consumed {
            continue;
        }
        let start = start.max(consumed);
        taken.push(Span {
            start,
            end,
            kind: kind.into(),
            value: text[start..end].into(),
        });
        consumed = end;
    }
    Ok(taken)
}

pub fn statutory(value: &str) -> Result<bool> {
    if has("STATUTORY", value)? {
        return Ok(true);
    }
    let uppercase = |t: &str| t.len() > 1 && t.chars().all(|c| pytext::is_upper(c as u32));
    let mut all_caps = false;
    let mut cursor = 0;
    for m in matches("_STATUTORY_SPLIT", value)? {
        let m = m.get(0).unwrap();
        all_caps |= uppercase(&value[cursor..m.start()]);
        cursor = m.end();
    }
    all_caps |= uppercase(&value[cursor..]);
    Ok(all_caps && has("_STATUTORY_CI", value)?)
}

/// Searches retain lookbehind context; rejected windows resume at token two.
/// Tokens have fixed maximum length; the number of windows per start is <=2.
/// Whitespace can be revisited by only a bounded number of adjacent tokens.
pub fn person_names(text: &str) -> Result<Vec<Span>> {
    scan_names(text, |_, _, _, _| {})
}

#[cfg(feature = "probe")]
pub fn name_attempts(text: &str) -> Result<Vec<(usize, usize, bool, usize)>> {
    let mut attempts = Vec::new();
    scan_names(text, |start, end, rejected, next| {
        attempts.push((start, end, rejected, next))
    })?;
    Ok(attempts)
}

fn scan_names(text: &str, mut visit: impl FnMut(usize, usize, bool, usize)) -> Result<Vec<Span>> {
    let mut found = Vec::new();
    let mut pos = 0;
    while let Some(m) = search("NAME", text, pos)? {
        let whole = m.get(0).unwrap();
        let candidate = whole.as_str();
        if !statutory(candidate)? {
            found.push(Span {
                start: whole.start(),
                end: whole.end(),
                kind: "name".into(),
                value: candidate.into(),
            });
            pos = whole.end();
            visit(whole.start(), whole.end(), false, pos);
            continue;
        }
        let tokens: Vec<_> = matches("_NAME_TOKEN", candidate)?
            .iter()
            .map(|m| {
                let t = m.get(0).unwrap();
                (t.start(), t.end())
            })
            .collect();
        let mut kept = false;
        if tokens.len() == 3 {
            for pair in tokens.windows(2) {
                let (start, end) = (pair[0].0, pair[1].1);
                if !statutory(&candidate[start..end])? {
                    found.push(Span {
                        start: whole.start() + start,
                        end: whole.start() + end,
                        kind: "name".into(),
                        value: candidate[start..end].into(),
                    });
                    kept = true;
                }
            }
        }
        pos = if kept { whole.end() } else { whole.start() + tokens[1].0 };
        visit(whole.start(), whole.end(), true, pos);
    }
    Ok(found)
}

pub fn value_parts(value: &str) -> Result<Vec<String>> {
    let value = pytext::nfc(value);
    let mut parts = Vec::new();
    let mut cursor = 0;
    for m in matches("_JOIN_SPLIT", &value)? {
        let m = m.get(0).unwrap();
        if m.start() > cursor {
            parts.push(value[cursor..m.start()].into());
        }
        cursor = m.end();
    }
    if cursor < value.len() {
        parts.push(value[cursor..].into());
    }
    Ok(parts)
}

pub fn literal(value: &str, insensitive: bool) -> String {
    value
        .chars()
        .map(|ch| {
            let members = if insensitive {
                pytext::ignorecase(ch as u32)
            } else {
                &[]
            };
            if members.is_empty() {
                format!("\\x{{{:X}}}", ch as u32)
            } else {
                format!(
                    "[{}]",
                    members.iter().map(|cp| format!("\\x{{{cp:X}}}")).collect::<String>()
                )
            }
        })
        .collect()
}

pub fn value_pattern(value: &str) -> Result<Regex> {
    static CACHE: LazyLock<std::sync::Mutex<HashMap<String, Regex>>> =
        LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));
    let mut cache = CACHE.lock().map_err(|_| "detector cache is unavailable".to_string())?;
    if let Some(regex) = cache.get(value) {
        return Ok(regex.clone());
    }
    let regex = compile_value(value)?;
    cache.insert(value.into(), regex.clone());
    Ok(regex)
}

fn compile_value(value: &str) -> Result<Regex> {
    let parts = value_parts(value)?;
    if parts.is_empty() {
        return Err("entity value has no token to match".into());
    }
    let mut body = literal(&parts[0], true);
    for (i, part) in parts[1..].iter().enumerate() {
        // Atomic join: a token beginning '<br' cannot backtrack into a joiner.
        // Alternatives own different first characters; newline/BR owns its
        // following spaces. Runs are consumed whole, once per token boundary.
        body.push_str(&format!("(?P<j{i}>(?>{}+)){}", regexes::JOIN_ATOM, literal(part, true)));
    }
    let word = regexes::WORD;
    fancy_regex::RegexBuilder::new(&format!(
        "(?P<open>(?<![{word}*])[*_]*|(?<=\\*)_*){body}(?P<close>[*_]*)(?!{word})"
    ))
    .backtrack_limit(100_000_000)
    .build()
    .map_err(runtime)
}

pub fn fold(value: &str) -> Result<String> {
    let text = value_parts(value)?.join(" ").replace(['İ', 'ı'], "i");
    Ok(pytext::casefold(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backtrack_exhaustion_propagates_instead_of_becoming_no_match() {
        let regex = fancy_regex::RegexBuilder::new(r"(?=a)(a+)+$")
            .backtrack_limit(10)
            .build()
            .unwrap();
        let result = checked_matches(&regex, "aaaaaaaaaaaaaaaaaaaaX");
        let error = result.unwrap_err();
        assert!(error.starts_with("detector refused input:"));
        assert!(error.contains("backtrack"));
    }
}
