//! Shared readers for the static PBIP, TMDL and fixture checks.

#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::Value;

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

/// `csv.DictReader` rows of a committed fixture.
pub fn csv_rows(path: &Path) -> Vec<HashMap<String, String>> {
    let mut reader = csv::Reader::from_path(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let headers = reader.headers().expect("CSV header").clone();
    reader
        .records()
        .map(|record| {
            let record = record.expect("CSV record");
            headers
                .iter()
                .zip(record.iter())
                .map(|(k, v)| (k.to_owned(), v.to_owned()))
                .collect()
        })
        .collect()
}

/// `csv.reader` rows of a committed fixture, header included.
pub fn csv_records(path: &Path) -> Vec<Vec<String>> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_path(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    reader
        .records()
        .map(|record| record.expect("CSV record").iter().map(str::to_owned).collect())
        .collect()
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

pub fn int(node: &Value) -> i64 {
    node.as_i64()
        .unwrap_or_else(|| panic!("integer expected, found {node}"))
}
