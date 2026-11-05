// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! evatt/redact.py, preserving code-point masks and the two line conventions.
use crate::{
    entities::{self, Entity},
    patterns::{self, Result, Span},
    pytext,
};
use std::collections::{BTreeMap, HashMap, HashSet};

pub type Counts = Vec<(String, usize)>;

fn count(counts: &mut Counts, kind: &str) {
    if let Some((_, n)) = counts.iter_mut().find(|(k, _)| k == kind) {
        *n += 1;
    } else {
        counts.push((kind.into(), 1));
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unknown {
    pub kind: String,
    pub value: String,
    pub line: usize,
    pub context: std::sync::Arc<str>,
}

pub struct Lines {
    pub lines: Vec<std::sync::Arc<str>>,
    pub starts: Vec<usize>,
}

impl Lines {
    pub fn new(text: &str) -> Self {
        let mut lines = Vec::new();
        let mut starts = Vec::new();
        let mut start = 0;
        let mut chars = text.char_indices().peekable();
        while let Some((i, ch)) = chars.next() {
            if !pytext::is_line_break(ch as u32) {
                continue;
            }
            let mut end = i + ch.len_utf8();
            if ch == '\r' && chars.peek().is_some_and(|&(_, c)| c == '\n') {
                end = chars.next().unwrap().0 + 1;
            }
            starts.push(start);
            lines.push(pytext::strip(&text[start..end]).into());
            start = end;
        }
        if start < text.len() {
            starts.push(start);
            lines.push(pytext::strip(&text[start..]).into());
        }
        Self { lines, starts }
    }

    pub fn locate(&self, pos: usize) -> (usize, std::sync::Arc<str>) {
        let index = self.starts.partition_point(|&s| s <= pos);
        if index == 0 {
            return (1, std::sync::Arc::from(""));
        }
        (index, self.lines[index - 1].clone())
    }

    fn unknown(&self, kind: &str, value: &str, pos: usize) -> Unknown {
        let (line, context) = self.locate(pos);
        Unknown {
            kind: kind.into(),
            value: value.into(),
            line,
            context,
        }
    }
}

/// Each mask writes one '#' per Python code point. `original` maps masked
/// UTF-8 byte boundaries back to the original document, composed across masks
/// in linear time. No byte-based filler changes lookbehind or word boundaries.
pub struct Mask {
    pub text: String,
    pub original: Vec<usize>,
}

impl Mask {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.into(),
            original: (0..=text.len()).collect(),
        }
    }

    pub fn blank(self, name: &str) -> Result<Self> {
        let mut text = String::new();
        let mut original = Vec::new();
        let mut cursor = 0;
        for m in patterns::matches(name, &self.text)? {
            let m = m.get(0).unwrap();
            text.push_str(&self.text[cursor..m.start()]);
            original.extend_from_slice(&self.original[cursor..m.start()]);
            for (i, _) in m.as_str().char_indices() {
                text.push('#');
                original.push(self.original[m.start() + i]);
            }
            cursor = m.end();
        }
        text.push_str(&self.text[cursor..]);
        original.extend_from_slice(&self.original[cursor..]);
        Ok(Self { text, original })
    }
}

fn normalise(kind: &str, value: &str) -> String {
    if kind == "email" {
        return pytext::casefold(pytext::strip(value));
    }
    let digits = patterns::digits(value);
    if kind == "medicare" {
        digits.chars().take(10).collect()
    } else {
        digits
    }
}

fn structured(text: &str) -> Result<(String, Counts)> {
    let mut assigned = HashMap::new();
    let mut ordinals = HashMap::new();
    let mut counts = Vec::new();
    let mut out = String::new();
    let mut cursor = 0;
    for span in patterns::structured_spans(text)? {
        let key = (span.kind.clone(), normalise(&span.kind, &span.value));
        let placeholder = assigned.entry(key).or_insert_with(|| {
            let ordinal = ordinals.entry(span.kind.clone()).or_insert(0);
            *ordinal += 1;
            format!("{}_{:02}", span.kind.to_ascii_uppercase(), ordinal)
        });
        count(&mut counts, &span.kind);
        out.push_str(&text[cursor..span.start]);
        if span.start > 0
            && !pytext::is_space(text[..span.start].chars().next_back().unwrap() as u32)
            && !pytext::is_word(text[span.start..].chars().next().unwrap() as u32)
        {
            out.push(' ');
        }
        out.push_str(placeholder);
        cursor = span.end;
    }
    out.push_str(&text[cursor..]);
    Ok((out, counts))
}

pub const NUMBER_SEPARATORS: &str = " \t-/.,\u{2010}\u{2011}\u{2012}\u{2013}\u{2014}\u{2015}\u{2212}";

fn decimal_int(text: &str) -> u32 {
    text.chars()
        .fold(0, |n, c| n * 10 + pytext::decimal_value(c as u32).unwrap())
}

fn date_end(text: &str) -> Result<usize> {
    let Some(m) = patterns::search("_DATE_LEAD", text, 0)? else {
        return Ok(0);
    };
    if m.get(0).unwrap().start() != 0 {
        return Ok(0);
    }
    let (y, mo, d) = if m.get(1).is_some() { (1, 2, 3) } else { (6, 5, 4) };
    let get = |i| decimal_int(m.get(i).unwrap().as_str());
    if entities::date_valid(get(y), get(mo), get(d)) {
        Ok(m.get(0).unwrap().end())
    } else {
        Ok(0)
    }
}

fn token_end(chars: &[(usize, char)], mut cursor: usize, stats: &mut ScanStats) -> usize {
    while cursor + 1 < chars.len() {
        stats.examined();
        if pytext::is_decimal(chars[cursor].1 as u32) {
            cursor += 1;
            continue;
        }
        let mut following = cursor;
        while following + 1 < chars.len() && following - cursor < 2 && NUMBER_SEPARATORS.contains(chars[following].1) {
            stats.examined();
            following += 1;
        }
        if following == cursor || following + 1 == chars.len() || !pytext::is_decimal(chars[following].1 as u32) {
            break;
        }
        cursor = following + 1;
    }
    cursor
}

/// Maximal numeric tokens: the cursor always advances. Each gap inspects at
/// most two separators plus its next digit. Date/amount checks run once per
/// complete token, never over suffixes. Total scanned characters are bounded.
#[derive(Default, Debug, Clone)]
pub struct ScanStats {
    pub candidate_starts: usize,
    pub characters_examined: usize,
}

impl ScanStats {
    #[inline]
    fn examined(&mut self) {
        #[cfg(any(test, feature = "probe"))]
        {
            self.characters_examined += 1;
        }
    }
    #[inline]
    fn candidate(&mut self) {
        #[cfg(any(test, feature = "probe"))]
        {
            self.candidate_starts += 1;
        }
    }
}

pub fn numbers(text: &str) -> Result<Vec<Span>> {
    numbers_counted(text).map(|(spans, _)| spans)
}

/// Words that say a figure beside them is an identifier, whatever it looks like.
const IDENTIFIER_LABELS: [&str; 7] = [
    "tfn",
    "abn",
    "acn",
    "medicare",
    "tax file",
    "business number",
    "company number",
];
/// How far along a line a label counts as near, in bytes.
const LABEL_REACH: usize = 40;
/// What `without_markup` writes over a tag or a character reference.
const MARKUP_BLANK: char = '\0';

#[derive(Clone, Copy, PartialEq)]
enum Gap {
    /// Crossed by the token scan.
    Token,
    /// Too wide for a token, or holding marks a token stops at, yet still inside
    /// one printed number as far as anyone can tell.
    Split,
    /// A line break in a wrap's shape. It joins whole groups only.
    Wrap,
    /// Beside money. It joins only near an identifier's label.
    Money,
    Break,
}

/// What lies between 2 digit runs. A letter or a digit ends a number, and so does
/// `#`, which is what a mask writes over a date, an address or a placeholder. Any
/// other same-line gap is a possible number; a line break needs a wrap's shape.
fn gap(text: &str) -> Result<Gap> {
    if text.chars().count() <= 2 && text.chars().all(|c| NUMBER_SEPARATORS.contains(c)) {
        return Ok(Gap::Token);
    }
    if text.chars().any(|c| c.is_alphanumeric() || c == '#') {
        return Ok(Gap::Break);
    }
    if !text.contains('\n') {
        return Ok(Gap::Split);
    }
    let marks: String = text
        .chars()
        .filter(|c| !('\u{2010}'..='\u{2015}').contains(c) && !['-', '\u{2212}', '\u{ad}', MARKUP_BLANK].contains(c))
        .collect();
    let one_line = marks.matches('\n').count() <= 1;
    Ok(if one_line && patterns::full("_JOIN_SPLIT", &marks)? {
        Gap::Wrap
    } else {
        Gap::Break
    })
}

/// `text` with its tags and character references blanked byte for byte, so
/// offsets still hold. A tag keeps its digits: a number can sit in an attribute.
/// The blank is `MARKUP_BLANK`, so a label can be read with its markup gone.
fn without_markup(text: &str, stats: &mut ScanStats) -> String {
    let mut bytes = text.as_bytes().to_vec();
    let mut cursor = 0;
    // No `<` before this offset can close, so each line is searched once.
    let mut unclosed_until = 0;
    while cursor < bytes.len() {
        let rest = &text.as_bytes()[cursor..];
        let length = match rest[0] {
            b'<' if cursor >= unclosed_until && rest.get(1).is_some_and(|b| b.is_ascii_alphabetic() || *b == b'/') => {
                let end = rest.iter().position(|b| {
                    stats.examined();
                    matches!(b, b'>' | b'\n')
                });
                let closed = end.filter(|end| rest[*end] == b'>');
                if closed.is_none() {
                    unclosed_until = cursor + end.unwrap_or(rest.len());
                }
                closed.map(|end| (end + 1, true))
            }
            b'&' => rest
                .iter()
                .take(12)
                .skip(1)
                .position(|b| !(b.is_ascii_alphanumeric() || *b == b'#'))
                .filter(|end| *end > 0 && rest[end + 1] == b';')
                .filter(|_| rest[1].is_ascii_alphabetic() || rest[1] == b'#')
                .map(|end| (end + 2, false)),
            _ => None,
        };
        let Some((length, keep_digits)) = length else {
            cursor += 1;
            continue;
        };
        for byte in &mut bytes[cursor..cursor + length] {
            if !(keep_digits && byte.is_ascii_digit()) {
                *byte = MARKUP_BLANK as u8;
            }
        }
        cursor += length;
    }
    String::from_utf8(bytes).expect("only whole characters between ASCII marks were blanked")
}

/// Each run of decimal digits, as byte offsets.
fn digit_runs(text: &str) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (start, ch) in text.char_indices() {
        if !pytext::is_decimal(ch as u32) {
            continue;
        }
        match runs.last_mut() {
            Some(last) if last.1 == start => last.1 += ch.len_utf8(),
            _ => runs.push((start, start + ch.len_utf8())),
        }
    }
    runs
}

fn passes_identifier_check(digits: &str) -> Result<bool> {
    for kind in ["tfn", "acn", "medicare", "abn"] {
        if patterns::valid(kind, digits)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether `text` holds an identifier's label as whole words, in any case.
fn names_identifier(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    IDENTIFIER_LABELS.iter().any(|label| {
        text.match_indices(label).any(|(at, _)| {
            !text[..at].chars().next_back().is_some_and(char::is_alphanumeric)
                && !text[at + label.len()..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric)
        })
    })
}

/// Whether a label sits near `span` on its own line.
fn near_label(text: &str, span: (usize, usize)) -> bool {
    let from = text.floor_char_boundary(span.0.saturating_sub(LABEL_REACH));
    let to = text.ceil_char_boundary(span.1 + LABEL_REACH);
    let before = text[from..span.0].rsplit('\n').next().unwrap_or_default();
    let after = text[span.1..to].split('\n').next().unwrap_or_default();
    // Markup can sit between a label's words or inside a word: `T<b>F</b>N`.
    [before, after].iter().any(|side| {
        let spaced = side.replace(MARKUP_BLANK, " ");
        let words: Vec<_> = spaced.split_whitespace().collect();
        names_identifier(&words.join(" ")) || names_identifier(&side.replace(MARKUP_BLANK, ""))
    })
}

/// Each 9, 10 and 11 digit window of one chain of runs that crosses a split gap
/// and passes a check, as the byte span of the runs it touches. A digit touching
/// the number stays in the span, and a window can start or end inside a run,
/// unless it crosses a wrap: an invalid number wrapped across lines must not halt
/// on some part of it.
fn chain_identifiers(
    text: &str,
    runs: &[(usize, usize)],
    gaps: &[Gap],
    stats: &mut ScanStats,
) -> Result<Vec<(usize, usize)>> {
    let mut found = Vec::new();
    if gaps.iter().all(|gap| *gap == Gap::Token) {
        return Ok(found);
    }
    let digits: Vec<(char, usize, usize)> = runs
        .iter()
        .enumerate()
        .flat_map(|(index, run)| {
            text[run.0..run.1]
                .char_indices()
                .map(move |(offset, c)| (c, index, run.0 + offset))
        })
        .collect();
    for width in 9..=11 {
        for window in digits.windows(width) {
            stats.candidate();
            let (first, last) = (window[0], window[width - 1]);
            let (head, tail) = (first.1, last.1);
            let crossed = &gaps[head..tail];
            let whole_runs = first.2 == runs[head].0 && last.2 + last.0.len_utf8() == runs[tail].1;
            if crossed.iter().any(|gap| *gap != Gap::Token) && (whole_runs || !crossed.contains(&Gap::Wrap)) {
                let joined: String = window.iter().map(|digit| digit.0).collect();
                let span = (runs[head].0, runs[tail].1);
                if passes_identifier_check(&joined)? && (!crossed.contains(&Gap::Money) || near_label(text, span)) {
                    found.push(span);
                }
            }
        }
    }
    Ok(found)
}

/// Which runs are written as money, as in `1,234.56`: digits, a point and 2 more,
/// with any comma groups before them.
fn money_runs(text: &str, runs: &[(usize, usize)]) -> Vec<bool> {
    let between = |index: usize| &text[runs[index].1..runs[index + 1].0];
    let width = |index: usize| runs[index].1 - runs[index].0;
    let mut money = vec![false; runs.len()];
    for last in 1..runs.len() {
        if between(last - 1) == "." && width(last) == 2 {
            let mut first = last - 1;
            while first > 0 && between(first - 1) == "," && width(first) == 3 {
                first -= 1;
            }
            money[first..=last].fill(true);
        }
    }
    money
}

/// Digit runs no token joins that together pass a TFN, ACN, Medicare or ABN
/// check, as byte offsets. A check digit passes about 1 time in 10, and every
/// window is a fresh try, so a chain of ordinary numbers would nearly always
/// halt. Money therefore joins only near an identifier's label; other numbers
/// that share a line or wrap across lines can halt, as the columnar rows of the
/// README's benchmark show. Each chain is walked once, so the work grows with the
/// digits in the text.
fn split_identifiers(text: &str, stats: &mut ScanStats) -> Result<Vec<(usize, usize)>> {
    let text = without_markup(text, stats);
    let runs = digit_runs(&text);
    let money = money_runs(&text, &runs);
    let gaps = runs
        .windows(2)
        .zip(money.windows(2))
        .map(|(pair, money)| {
            let between = &text[pair[0].1..pair[1].0];
            let inside = money[0] && money[1] && matches!(between, "." | ",");
            let plain = gap(between)?;
            Ok(if (money[0] || money[1]) && !inside && plain != Gap::Break {
                Gap::Money
            } else {
                plain
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut found = Vec::new();
    let mut first = 0;
    while first < runs.len() {
        let mut last = first;
        while last < gaps.len() && gaps[last] != Gap::Break {
            last += 1;
        }
        found.extend(chain_identifiers(
            &text,
            &runs[first..=last],
            &gaps[first..last],
            stats,
        )?);
        first = last + 1;
    }
    Ok(found)
}

/// Whether `tokens` already name every digit of `span`, so that a halt quotes
/// them apart and each stays one value in triage.
fn named_by_tokens(text: &str, tokens: &[(usize, usize)], span: (usize, usize)) -> bool {
    text[span.0..span.1]
        .char_indices()
        .filter(|(_, c)| pytext::is_decimal(*c as u32))
        .all(|(offset, _)| {
            let at = span.0 + offset;
            let next = tokens.partition_point(|&(_, end)| end <= at);
            tokens.get(next).is_some_and(|&(start, _)| start <= at)
        })
}

pub fn numbers_counted(text: &str) -> Result<(Vec<Span>, ScanStats)> {
    let mut stats = ScanStats::default();
    let mut chars: Vec<_> = text.char_indices().collect();
    chars.push((text.len(), '\0'));
    let mut cursor = 0;
    let mut found = Vec::new();
    while cursor + 1 < chars.len() {
        stats.examined();
        if !pytext::is_decimal(chars[cursor].1 as u32) {
            cursor += 1;
            continue;
        }
        stats.candidate();
        let mut start = chars[cursor].0;
        cursor += 1;
        cursor = token_end(&chars, cursor, &mut stats);
        let end = chars[cursor].0;
        let value = &text[start..end];
        let date = date_end(value)?;
        if date > 0 {
            let remainder = value[date..].trim_start_matches(|c| NUMBER_SEPARATORS.contains(c));
            start += value.len() - remainder.len();
        }
        let value = &text[start..end];
        let digits: String = value.chars().filter(|c| pytext::is_decimal(*c as u32)).collect();
        if digits.chars().count() < 8 {
            continue;
        }
        if patterns::full("_AMOUNT", value)? && !passes_identifier_check(&digits)? {
            continue;
        }
        found.push((start, end));
    }
    let split = split_identifiers(text, &mut stats)?;
    let split: Vec<_> = split
        .into_iter()
        .filter(|span| !named_by_tokens(text, &found, *span))
        .collect();
    found.extend(split);
    let spans = patterns::merge(found)
        .into_iter()
        .map(|(start, end)| Span {
            start,
            end,
            kind: "number".into(),
            value: text[start..end].into(),
        })
        .collect();
    Ok((spans, stats))
}

fn mapped_numbers(mask: &Mask) -> Result<Vec<(usize, usize)>> {
    Ok(numbers(&mask.text)?
        .iter()
        .map(|s| (mask.original[s.start], mask.original[s.end]))
        .collect())
}

struct EntityMatch {
    start: usize,
    end: usize,
    priority: usize,
    open: String,
    close: String,
}

fn entity_matches(text: &str, entries: &[Entity]) -> Result<Vec<EntityMatch>> {
    let mut found = Vec::new();
    for (priority, entity) in entries.iter().enumerate() {
        let value = entity.value.os_text();
        if pytext::strip(&value).is_empty() || patterns::has("PLACEHOLDER_CI", &value)? {
            continue;
        }
        if !patterns::full("PLACEHOLDER", &entity.placeholder)?
            || entity.placeholder.rsplit_once('_').map(|p| p.0) != entities::prefix(&entity.kind)
        {
            continue;
        }
        if patterns::value_parts(&value)?.is_empty() {
            continue;
        }
        for m in entities::matcher(entity)?.captures_iter(text) {
            let m = m.map_err(patterns::runtime)?;
            let whole = m.get(0).unwrap();
            found.push(EntityMatch {
                start: whole.start(),
                end: whole.end(),
                priority,
                open: m.name("open").unwrap().as_str().into(),
                close: m.name("close").unwrap().as_str().into(),
            });
        }
    }
    found.sort_by_key(|m| (m.start, std::cmp::Reverse(m.end - m.start), m.priority));
    Ok(found)
}

fn partial_number(numbers: &[(usize, usize)], start: usize, end: usize) -> bool {
    let first = numbers.partition_point(|&(_, e)| e <= start);
    let next = numbers.partition_point(|&(s, _)| s < end);
    first < next && (start > numbers[first].0 || end < numbers[next - 1].1)
}

fn replace_entities(text: &str, entries: &[Entity]) -> Result<(String, Counts)> {
    let protected: Vec<_> = patterns::matches("CARRIED_PLACEHOLDER", text)?
        .iter()
        .map(|m| {
            let t = m.name("token").unwrap();
            (t.start(), t.end())
        })
        .collect();
    let numeric = Mask::new(text)
        .blank("ADDRESS")?
        .blank("DOB")?
        .blank("CARRIED_PLACEHOLDER")?;
    let numbers = mapped_numbers(&numeric)?;
    let ambiguous = patterns::merge(patterns::ambiguous(text)?.iter().map(|a| (a.start, a.end)).collect());
    let mut counts = Vec::new();
    let mut out = String::new();
    let mut cursor = 0;
    let mut previous_close = String::new();
    let mut accepted = false;
    for m in entity_matches(text, entries)? {
        if m.start < cursor
            || patterns::intersects(&protected, m.start, m.end)
            || patterns::intersects(&ambiguous, m.start, m.end)
            || partial_number(&numbers, m.start, m.end)
        {
            continue;
        }
        let entity = &entries[m.priority];
        count(&mut counts, &entity.kind);
        if accepted && m.start == cursor && previous_close.is_empty() && m.open.is_empty() {
            out.push(' ');
        }
        out.push_str(&text[cursor..m.start]);
        out.push_str(&m.open.replace('_', "*"));
        out.push_str(&entity.placeholder);
        out.push_str(&m.close.replace('_', "*"));
        previous_close = m.close;
        cursor = m.end;
        accepted = true;
    }
    out.push_str(&text[cursor..]);
    Ok((out, counts))
}

fn number_unknowns(numeric: &Mask, ambiguous: &Mask, lines: &Lines) -> Result<Vec<Unknown>> {
    let ambiguous = patterns::ambiguous(&ambiguous.text)?
        .iter()
        .map(|a| patterns::Ambiguous {
            start: ambiguous.original[a.start],
            digit_start: ambiguous.original[a.digit_start],
            end: ambiguous.original[a.end],
            value: a.value.clone(),
        })
        .collect::<Vec<_>>();
    let protected = patterns::merge(ambiguous.iter().map(|a| (a.digit_start, a.end)).collect());
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for a in ambiguous {
        let u = lines.unknown("number", &a.value, a.digit_start);
        if seen.insert((u.line, u.value.clone())) {
            found.push(u);
        }
    }
    for s in numbers(&numeric.text)? {
        let (start, end) = (numeric.original[s.start], numeric.original[s.end]);
        // An ambiguous group can sit inside a valid number, and must not hide it.
        let digits: String = s.value.chars().filter(|c| pytext::is_decimal(*c as u32)).collect();
        if patterns::intersects(&protected, start, end) && !passes_identifier_check(&digits)? {
            continue;
        }
        let u = lines.unknown("number", &s.value, start);
        if seen.insert((u.line, u.value.clone())) {
            found.push(u);
        }
    }
    Ok(found)
}

/// A number whose last group starts an address: the address mask hides that group
/// from the sweep, so the halt would name the address and not the number.
fn numbers_into_addresses(text: &str, lines: &Lines) -> Result<Vec<Unknown>> {
    let addresses: Vec<_> = patterns::matches("ADDRESS", text)?
        .iter()
        .map(|m| m.get(0).unwrap().start())
        .collect();
    let mut found = Vec::new();
    if addresses.is_empty() {
        return Ok(found);
    }
    let numeric = Mask::new(text).blank("DOB")?.blank("CARRIED_PLACEHOLDER")?;
    for s in numbers(&numeric.text)? {
        let (start, end) = (numeric.original[s.start], numeric.original[s.end]);
        let next = addresses.partition_point(|&address| address <= start);
        if addresses.get(next).is_some_and(|&address| address < end) {
            found.push(lines.unknown("number", &s.value, start));
        }
    }
    Ok(found)
}

pub fn residual(text: &str) -> Result<Vec<Unknown>> {
    let lines = Lines::new(text);
    let mut found = Vec::new();
    for (kind, name) in [("address", "ADDRESS"), ("date", "DOB")] {
        for m in patterns::matches(name, text)? {
            let m = m.get(0).unwrap();
            found.push(lines.unknown(kind, m.as_str(), m.start()));
        }
    }
    let masked = Mask::new(text).blank("ADDRESS")?.blank("DOB")?;
    let mut names = BTreeMap::new();
    for s in patterns::person_names(&masked.text)? {
        if patterns::has("PLACEHOLDER", &s.value)? {
            continue;
        }
        let u = lines.unknown("name", &s.value, masked.original[s.start]);
        names.entry((u.line, u.value.clone())).or_insert(u);
    }
    let numeric = masked.blank("CARRIED_PLACEHOLDER")?;
    let ambiguous = Mask::new(text).blank("CARRIED_PLACEHOLDER")?;
    found.extend(number_unknowns(&numeric, &ambiguous, &lines)?);
    found.extend(numbers_into_addresses(text, &lines)?);
    found.extend(names.into_values());
    Ok(found)
}

fn input_placeholders(text: &str, redacted: &str) -> Result<Vec<Unknown>> {
    let input = patterns::matches("CARRIED_PLACEHOLDER", text)?;
    let carried: HashSet<_> = input.iter().map(|m| m.name("token").unwrap().as_str()).collect();
    if carried.is_empty() {
        return Ok(Vec::new());
    }
    let lines = Lines::new(redacted);
    let mut found = BTreeMap::new();
    for m in patterns::matches("CARRIED_PLACEHOLDER", redacted)? {
        let t = m.name("token").unwrap();
        if !carried.contains(t.as_str()) {
            continue;
        }
        let u = lines.unknown("placeholder", t.as_str(), t.start());
        found.entry((u.line, u.value.clone())).or_insert(u);
    }
    let survived: HashSet<_> = found.keys().map(|(_, v)| v.clone()).collect();
    if survived.len() < carried.len() {
        let input_lines = Lines::new(text);
        for m in input {
            let t = m.name("token").unwrap();
            if survived.contains(t.as_str()) {
                continue;
            }
            let (line, _) = input_lines.locate(t.start());
            let context = lines.lines.get(line - 1).cloned().unwrap_or_default();
            let u = Unknown {
                kind: "placeholder".into(),
                value: t.as_str().into(),
                line,
                context,
            };
            found.entry((line, u.value.clone())).or_insert(u);
        }
    }
    Ok(found.into_values().collect())
}

pub struct Redaction {
    pub text: String,
    pub counts: Counts,
    pub unknowns: Vec<Unknown>,
}

pub fn redact(text: &str, entries: &[Entity]) -> Result<Redaction> {
    let text = pytext::nfc(&text.replace("\r\n", "\n"));
    let (redacted, mut counts) = structured(&text)?;
    let (redacted, entities) = replace_entities(&redacted, entries)?;
    let mut unknowns = input_placeholders(&text, &redacted)?;
    unknowns.extend(residual(&redacted)?);
    for (kind, n) in entities {
        if let Some((_, old)) = counts.iter_mut().find(|(k, _)| k == &kind) {
            *old += n;
        } else {
            counts.push((kind, n));
        }
    }
    Ok(Redaction {
        text: redacted,
        counts,
        unknowns,
    })
}
