//! Reads one JSON array per line, `["op", "a", "b"]`, applies the operation
//! with the gateway's decimal module and prints one JSON array of results.
//! tests/differential/decimal_differential.py compares it with CPython.

use std::io::{BufRead, Write};

use elizabeth_anne_alexander::decimal::{Decimal, Rounding};
use elizabeth_anne_alexander::pyjson::{self, Json};

fn text(value: Option<&Json>) -> String {
    value
        .and_then(Json::as_pystr)
        .and_then(|s| s.to_utf8())
        .unwrap_or_default()
}

fn show(value: &Decimal) -> Json {
    Json::str(&value.to_sci_string())
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = line.expect("readable input");
        let Ok(Json::Array(items)) = pyjson::loads(&line) else {
            panic!("bad request line")
        };
        let op = text(items.first());
        let a = Decimal::parse(&text(items.get(1)));
        let b = items.get(2).map(|v| Decimal::parse(&text(Some(v))));
        let result = match (op.as_str(), &a, &b) {
            ("parse", Err(_), _) => Json::Array(vec![Json::str("invalid")]),
            ("parse", Ok(x), _) if !x.is_finite() => Json::Array(vec![Json::str("special")]),
            ("parse", Ok(x), _) => Json::Array(vec![show(x), Json::Int(x.adjusted().into())]),
            (_, Ok(x), Some(Ok(y))) if op == "add" || op == "sub" => {
                let o = if op == "add" { x.add(y) } else { x.sub(y) };
                Json::Array(vec![show(&o.value), Json::Bool(o.inexact)])
            }
            ("mul", Ok(x), Some(Ok(y))) => Json::Array(vec![show(&x.mul(y))]),
            ("div", Ok(x), Some(Ok(y))) => Json::Array(vec![show(&x.div(y))]),
            ("cmp", Ok(x), Some(Ok(y))) => Json::Array(vec![Json::int(x.cmp_value(y) as i64)]),
            ("abs", Ok(x), _) => Json::Array(vec![show(&x.abs())]),
            ("fixed", Ok(x), _) => match x.to_fixed_string() {
                Some(text) => Json::Array(vec![Json::str(&text)]),
                None => Json::Array(vec![Json::str("MemoryError")]),
            },
            ("quant4up", Ok(x), _) => match x.quantize(-4, Rounding::HalfUp) {
                Ok(q) => Json::Array(vec![show(&q)]),
                Err(_) => Json::Array(vec![Json::str("InvalidOperation")]),
            },
            ("quant2", Ok(x), _) => match x.quantize(-2, Rounding::HalfEven) {
                Ok(q) => Json::Array(vec![show(&q)]),
                Err(_) => Json::Array(vec![Json::str("InvalidOperation")]),
            },
            _ => panic!("unsupported request {line}"),
        };
        writeln!(out, "{}", pyjson::canonical(&result)).expect("writable output");
    }
}
