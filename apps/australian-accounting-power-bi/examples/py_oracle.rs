//! Reads one JSON request per line and answers with the test suite's reimplementation of
//! a Python behaviour, for tests/differential/python_semantics_differential.py:
//!
//! - `{"mode": "csv", "text": ...}`: `csv.reader` rows;
//! - `{"mode": "dict", "text": ...}`: `csv.DictReader` header and rows, `null` for restval;
//! - `{"mode": "date", "text": ...}`: `date.fromisoformat`, or `"error"`;
//! - `{"mode": "bpa", "text": ..., "object": {...}}`: the BPA rule evaluation, or `"error"`.

#[path = "../tests/common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};

use common::bpa::{Datum, Object, evaluate};
use serde_json::{Value, json};

/// The property names the differential's objects use.
const PROPERTIES: [&str; 10] = [
    "Name",
    "FormatString",
    "DataType",
    "IsHidden",
    "Description",
    "CrossFilteringBehavior",
    "Precedence",
    "A",
    "N",
    "F",
];

fn object(value: &Value) -> Object {
    let entries = value.as_object().expect("an object");
    entries
        .iter()
        .map(|(key, item)| {
            let datum = match item {
                Value::String(text) => Datum::Text(text.clone()),
                Value::Bool(flag) => Datum::Bool(*flag),
                other => Datum::Int(other.as_i64().expect("an integer")),
            };
            let name = PROPERTIES
                .iter()
                .find(|name| **name == key)
                .expect("a known property name");
            (*name, datum)
        })
        .collect()
}

fn answer(request: &Value) -> Value {
    let text = request["text"].as_str().expect("text");
    match request["mode"].as_str().expect("mode") {
        "csv" => json!(common::parse_csv(text)),
        "dict" => {
            let (header, rows) = common::dict_rows(text);
            let rows: Vec<BTreeMap<String, Option<String>>> = rows
                .iter()
                .map(|row| {
                    header
                        .iter()
                        .map(|name| (name.clone(), row.value(name).map(str::to_owned)))
                        .collect()
                })
                .collect();
            json!({"header": header, "rows": rows})
        }
        "date" => common::fromisoformat(text).map_or(json!("error"), |day| json!(day.to_string())),
        "bpa" => {
            let object = object(&request["object"]);
            catch_unwind(AssertUnwindSafe(|| evaluate(text, &object))).map_or(json!("error"), |result| json!(result))
        }
        other => panic!("unknown mode {other}"),
    }
}

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for line in std::io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.expect("readable input")).expect("a JSON request");
        writeln!(out, "{}", answer(&request)).expect("writable output");
    }
}
