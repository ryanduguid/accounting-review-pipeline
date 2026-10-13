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
        if patterns::full("_AMOUNT", value)?
            && !["tfn", "acn", "medicare", "abn"]
                .iter()
                .map(|kind| patterns::valid(kind, &digits))
                .collect::<Result<Vec<_>>>()?
                .contains(&true)
        {
            continue;
        }
        found.push(Span {
            start,
            end,
            kind: "number".into(),
            value: value.into(),
        });
    }
    Ok((found, stats))
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
        if patterns::intersects(&protected, start, end) {
            continue;
        }
        let u = lines.unknown("number", &s.value, start);
        if seen.insert((u.line, u.value.clone())) {
            found.push(u);
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
