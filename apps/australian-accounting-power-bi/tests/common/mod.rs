//! Shared readers for the static PBIP, TMDL and fixture checks.

#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use australian_accounting_power_bi::date::Date;
use regex::Regex;
use serde_json::Value;

pub mod bpa;

pub const NAME: &str = "australian-accounting-power-bi";

/// The application directory. `POWERBI_APP_DIR` points the checks at a copy, which the
/// mutation differential uses; otherwise it is this crate's directory.
pub fn app() -> PathBuf {
    std::env::var_os("POWERBI_APP_DIR").map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from)
}

pub fn model() -> PathBuf {
    app().join(format!("{NAME}.SemanticModel"))
}

pub fn definition() -> PathBuf {
    model().join("definition")
}

pub fn tables_dir() -> PathBuf {
    definition().join("tables")
}

pub fn report() -> PathBuf {
    app().join(format!("{NAME}.Report"))
}

pub fn report_definition() -> PathBuf {
    report().join("definition")
}

pub fn samples() -> PathBuf {
    app().join("samples")
}

/// `Path.read_text(encoding="utf-8")`: universal newlines turn `\r\n` and `\r` into `\n`.
pub fn read(path: &Path) -> String {
    let text = fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// `str.splitlines()`: every line boundary Python recognises, with `\r\n` as one.
pub fn splitlines(text: &str) -> Vec<&str> {
    let boundary = |c: char| {
        matches!(
            c,
            '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}'
        )
    };
    let mut lines = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(boundary) {
        lines.push(&rest[..at]);
        let width = if rest[at..].starts_with("\r\n") {
            2
        } else {
            rest[at..].chars().next().unwrap().len_utf8()
        };
        rest = &rest[at + width..];
    }
    if !rest.is_empty() {
        lines.push(rest);
    }
    lines
}

pub fn read_json(path: &Path) -> Value {
    serde_json::from_str(&read(path)).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Sorted entries of `directory` whose file name ends with `suffix`.
pub fn files_ending(directory: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("{}: {error}", directory.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(suffix))
        })
        .collect();
    found.sort();
    found
}

/// The model's `tables/*.tmdl` files, sorted.
pub fn table_files() -> Vec<PathBuf> {
    files_ending(&tables_dir(), ".tmdl")
}

pub fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .expect("UTF-8 file stem")
        .to_owned()
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .expect("UTF-8 file name")
        .to_owned()
}

/// Sorted report page directories (`pages/*/` that hold a `page.json`).
pub fn page_dirs() -> Vec<PathBuf> {
    let pages = report_definition().join("pages");
    let mut found: Vec<PathBuf> = fs::read_dir(&pages)
        .expect("pages directory")
        .map(|entry| entry.expect("page entry").path())
        .filter(|path| path.join("page.json").is_file())
        .collect();
    found.sort();
    found
}

/// Sorted `visuals/*/visual.json` paths under one page directory.
pub fn visual_files(page: &Path) -> Vec<PathBuf> {
    let visuals = page.join("visuals");
    if !visuals.is_dir() {
        return Vec::new();
    }
    let mut found: Vec<PathBuf> = fs::read_dir(&visuals)
        .expect("visuals directory")
        .map(|entry| entry.expect("visual entry").path().join("visual.json"))
        .filter(|path| path.is_file())
        .collect();
    found.sort();
    found
}

/// Every `pages/*/visuals/*/visual.json`, sorted.
pub fn all_visual_files() -> Vec<PathBuf> {
    let mut pages: Vec<PathBuf> = fs::read_dir(report_definition().join("pages"))
        .expect("pages directory")
        .map(|entry| entry.expect("page entry").path())
        .filter(|path| path.is_dir())
        .collect();
    pages.sort();
    pages.iter().flat_map(|page| visual_files(page)).collect()
}

#[derive(Clone, Copy, PartialEq)]
enum CsvState {
    StartRecord,
    StartField,
    InField,
    InQuoted,
    QuoteInQuoted,
}

/// `csv.reader` over text opened with `newline=""`, excel dialect, not strict: a blank
/// line is an empty row, a quote inside an unquoted field is literal, text after a
/// closing quote continues the field, and an unterminated quote ends the last field.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    use CsvState::*;
    let (mut rows, mut fields, mut field, mut state) = (Vec::new(), Vec::new(), Field::default(), StartRecord);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let newline = c == '\r' || c == '\n';
        if newline && state != InQuoted {
            if state != StartRecord {
                fields.push(field.take());
            }
            rows.push(std::mem::take(&mut fields));
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            state = StartRecord;
            continue;
        }
        state = match (state, c) {
            (StartRecord | StartField, '"') => InQuoted,
            (StartRecord | StartField | InField | QuoteInQuoted, ',') => {
                fields.push(field.take());
                StartField
            }
            (InQuoted, '"') => QuoteInQuoted,
            (QuoteInQuoted, '"') | (InQuoted, _) => {
                field.add(c);
                InQuoted
            }
            _ => {
                field.add(c);
                InField
            }
        };
    }
    if state != StartRecord {
        fields.push(field.take());
        rows.push(fields);
    }
    rows
}

/// A field being read, held to csv.field_size_limit()'s default of 131,072 characters.
#[derive(Default)]
struct Field {
    text: String,
    length: usize,
}

impl Field {
    fn add(&mut self, c: char) {
        assert!(
            self.length < 131_072,
            "_csv.Error: field larger than field limit (131072)"
        );
        self.length += 1;
        self.text.push(c);
    }

    fn take(&mut self) -> String {
        self.length = 0;
        std::mem::take(&mut self.text)
    }
}

/// A `csv.DictReader` row. A short row's missing values are `None`, as DictReader's
/// restval leaves them; indexing one panics, as using it in the Python checks raised.
pub struct Row(HashMap<String, Option<String>>);

impl Row {
    /// The value under `key`: `None` for a short row. A key outside the header panics,
    /// like Python's KeyError.
    pub fn value(&self, key: &str) -> Option<&str> {
        self.0.get(key).unwrap_or_else(|| panic!("KeyError: {key}")).as_deref()
    }
}

impl std::ops::Index<&str> for Row {
    type Output = String;

    fn index(&self, key: &str) -> &String {
        let value = self.0.get(key).unwrap_or_else(|| panic!("KeyError: {key}"));
        value.as_ref().unwrap_or_else(|| panic!("None value under {key}"))
    }
}

/// `csv.DictReader` rows of text: the first record names the columns, even when it is
/// empty, and empty data records are skipped.
pub fn dict_rows(text: &str) -> (Vec<String>, Vec<Row>) {
    let mut records = parse_csv(text).into_iter();
    let header = records.next().unwrap_or_default();
    let rows = records
        .filter(|record| !record.is_empty())
        .map(|record| {
            let mut values = record.into_iter();
            Row(header.iter().map(|name| (name.clone(), values.next())).collect())
        })
        .collect();
    (header, rows)
}

/// `csv.DictReader` rows of a committed fixture.
pub fn csv_rows(path: &Path) -> Vec<Row> {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    dict_rows(&String::from_utf8(bytes).expect("UTF-8 fixture")).1
}

/// `csv.DictReader` rows of a fixture opened in text mode (no `newline=""`), whose
/// universal newlines turn `\r` and `\r\n` into `\n` even inside quoted fields.
pub fn csv_rows_text(path: &Path) -> Vec<Row> {
    dict_rows(&read(path)).1
}

/// `csv.reader` rows of a committed fixture, header included.
pub fn csv_records(path: &Path) -> Vec<Vec<String>> {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    parse_csv(&String::from_utf8(bytes).expect("UTF-8 fixture"))
}

/// `count` ASCII digits at the start of `text`, as CPython's parse_digits reads them.
fn leading_number(text: &[u8], count: usize) -> Option<(u32, &[u8])> {
    let digits = text.get(..count)?;
    digits.iter().all(u8::is_ascii_digit).then(|| {
        let value = digits
            .iter()
            .fold(0, |total, digit| total * 10 + u32::from(digit - b'0'));
        (value, &text[count..])
    })
}

/// The date of ISO `week` and `weekday` (1 is Monday) in ISO `year`, if the week exists.
fn iso_week_date(year: u32, week: u32, weekday: u32) -> Option<Date> {
    use australian_accounting_power_bi::date::is_leap;
    let year = i32::try_from(year).ok().filter(|y| (1..=9999).contains(y))?;
    let first = Date::new(year, 1, 1).weekday();
    let long_year = first == 3 || (first == 2 && is_leap(year));
    if week == 0 || week > 53 || (week == 53 && !long_year) || !(1..=7).contains(&weekday) {
        return None;
    }
    let january_fourth = Date::new(year, 1, 4);
    let monday = january_fourth.add_days(-i64::from(january_fourth.weekday()));
    let day = monday.add_days(i64::from((week - 1) * 7 + weekday - 1));
    (1..=9999).contains(&day.year()).then_some(day)
}

/// `datetime.date.fromisoformat` (CPython 3.11+): `YYYY-MM-DD`, `YYYYMMDD` and the ISO
/// week forms `YYYY-Www[-D]` and `YYYYWww[D]`; `None` where Python raises ValueError.
pub fn fromisoformat(text: &str) -> Option<Date> {
    use australian_accounting_power_bi::date::days_in_month;
    let bytes = text.as_bytes();
    if ![7, 8, 10].contains(&bytes.len()) {
        return None;
    }
    let (year, rest) = leading_number(bytes, 4)?;
    let separated = rest.first() == Some(&b'-');
    let rest = if separated { &rest[1..] } else { rest };
    if let Some(week_part) = rest.strip_prefix(b"W") {
        let (week, rest) = leading_number(week_part, 2)?;
        // CPython stops after the weekday digit and ignores anything that follows it.
        let weekday = match rest {
            [] => 1,
            [b'-', tail @ ..] if separated => leading_number(tail, 1)?.0,
            _ if separated => return None,
            _ => leading_number(rest, 1)?.0,
        };
        return iso_week_date(year, week, weekday);
    }
    let (month, rest) = leading_number(rest, 2)?;
    let rest = match (separated, rest) {
        (true, [b'-', tail @ ..]) => tail,
        (true, _) => return None,
        (false, _) => rest,
    };
    let (day, _) = leading_number(rest, 2)?;
    let year = i32::try_from(year).ok().filter(|y| (1..=9999).contains(y))?;
    ((1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day)).then(|| Date::new(year, month, day))
}

/// The name in a `\tmeasure Name = ...` line, quotes stripped.
pub fn measure_name(line: &str) -> String {
    let declaration = line.strip_prefix("\tmeasure ").expect("measure line");
    let name = declaration.split_once(" =").map_or(declaration, |(name, _)| name);
    name.trim_matches('\'').to_owned()
}

/// Indexes of lines that start a TMDL object, matched by `pattern`.
pub fn object_starts(lines: &[&str], pattern: &str) -> Vec<usize> {
    let starts = Regex::new(pattern).expect("object pattern");
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| starts.is_match(line))
        .map(|(i, _)| i)
        .collect()
}

/// The first object start after `index`, or the end of the file.
pub fn next_object(starts: &[usize], index: usize, len: usize) -> usize {
    starts.iter().copied().find(|&start| start > index).unwrap_or(len)
}

/// Object starts used by the description, format and BPA checks.
pub const OBJECT_START: &str = r"^\t(?:measure|column|partition|hierarchy|calculationGroup|annotation)\b";

/// Map every measure in a TMDL table file to its DAX expression, flattened to one line.
pub fn measure_expressions(path: &Path) -> HashMap<String, String> {
    let text = read(path);
    let lines = splitlines(&text);
    let starts = object_starts(
        &lines,
        r"^\t(?:measure|column|partition|hierarchy|calculationGroup|annotation|///)",
    );
    let property = Regex::new(r"^\t\t\w+:").expect("property pattern");
    let mut expressions = HashMap::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(declaration) = line.strip_prefix("\tmeasure ") else {
            continue;
        };
        let (_, first) = declaration.split_once(" =").expect("measure expression");
        let end = next_object(&starts, index, lines.len());
        // Keep the expression lines; drop trailing properties such as formatString and lineageTag.
        let body =
            std::iter::once(first).chain(lines[index + 1..end].iter().copied().filter(|c| !property.is_match(c)));
        let joined: Vec<&str> = body.map(str::trim).filter(|part| !part.is_empty()).collect();
        expressions.insert(measure_name(line), joined.join(" "));
    }
    expressions
}

/// Recursively collect (kind, table, field) bindings from PBIR query state.
pub fn query_field_bindings(node: &Value) -> Vec<(String, String, String)> {
    let mut bindings = Vec::new();
    match node {
        Value::Object(map) => {
            for kind in ["Column", "Measure"] {
                let Some(Value::Object(binding)) = map.get(kind) else {
                    continue;
                };
                let entity = binding
                    .get("Expression")
                    .and_then(|e| e.get("SourceRef"))
                    .and_then(|s| s.get("Entity"));
                if let (Some(Value::String(table)), Some(Value::String(field))) = (entity, binding.get("Property")) {
                    bindings.push((kind.to_owned(), table.clone(), field.clone()));
                }
            }
            for value in map.values() {
                bindings.extend(query_field_bindings(value));
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|value| bindings.extend(query_field_bindings(value))),
        _ => {}
    }
    bindings
}

/// Python's access rules over parsed JSON, so a check fails where the Python suite
/// raised: `value[key]` is a KeyError, IndexError or TypeError when the item is
/// missing, and `dict.get` is an AttributeError on anything but an object.
pub trait Py {
    fn at<I: serde_json::value::Index + std::fmt::Debug + Copy>(&self, index: I) -> &Value;
    fn py_get(&self, key: &str) -> Option<&Value>;
}

impl Py for Value {
    fn at<I: serde_json::value::Index + std::fmt::Debug + Copy>(&self, index: I) -> &Value {
        self.get(index).unwrap_or_else(|| panic!("no item {index:?} in {self}"))
    }

    fn py_get(&self, key: &str) -> Option<&Value> {
        let Value::Object(map) = self else {
            panic!("AttributeError: get on {self}")
        };
        map.get(key)
    }
}

/// Python truthiness of a JSON value.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

/// Python's `key in container` for a string key.
pub fn py_in(key: &str, container: &Value) -> bool {
    match container {
        Value::Object(map) => map.contains_key(key),
        Value::String(text) => text.contains(key),
        Value::Array(items) => items.iter().any(|item| item == key),
        other => panic!("TypeError: argument of type {other} is not iterable"),
    }
}

/// The `Literal.Value` string under `expr`, as PBIR stores formatting literals.
pub fn literal(node: &Value) -> &str {
    node.at("expr")
        .at("Literal")
        .at("Value")
        .as_str()
        .expect("literal value")
}

/// A JSON number as Python compares it: ints, floats and bools (an int subclass) by
/// value. Integers beyond 2^53 lose the exactness Python keeps; PBIR values are small.
pub fn num(node: &Value) -> f64 {
    match node {
        Value::Bool(flag) => f64::from(u8::from(*flag)),
        other => other
            .as_f64()
            .unwrap_or_else(|| panic!("TypeError: number expected, found {other}")),
    }
}

/// Python's `sorted` key comparison for numbers and tuples of numbers.
pub fn order<T: PartialOrd>(a: &T, b: &T) -> std::cmp::Ordering {
    a.partial_cmp(b).expect("comparable numbers")
}
