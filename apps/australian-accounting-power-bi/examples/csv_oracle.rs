//! Reads JSON-encoded strings, one per line, and prints each one's `csv.reader` rows as
//! JSON using the test suite's reader. tests/differential/csv_differential.py compares
//! the result with Python's csv module.

#[path = "../tests/common/mod.rs"]
mod common;

use std::io::{BufRead, Write};

fn main() {
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for line in std::io::stdin().lock().lines() {
        let text: String = serde_json::from_str(&line.expect("readable input")).expect("a JSON string");
        let rows = common::parse_csv(&text);
        writeln!(out, "{}", serde_json::to_string(&rows).expect("JSON rows")).expect("writable output");
    }
}
