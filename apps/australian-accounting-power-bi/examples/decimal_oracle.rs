//! Reads one JSON array per line, `["op", "a", "b"]`, applies the generator's decimal
//! module and prints each result at 60 places, or `error` where it refuses.
//! tests/differential/decimal_differential.py compares it with CPython.

use std::io::{BufRead, Write};
use std::panic::catch_unwind;

use australian_accounting_power_bi::decimal::ContextDecimal;

fn answer<const P: u32>(op: &str, a: &str, b: &str) -> String {
    let (x, y) = (ContextDecimal::<P>::parse(a), || ContextDecimal::<P>::parse(b));
    match op {
        "parse" => x.to_fixed(60),
        "add" => x.add(&y()).to_fixed(60),
        "sub" => x.sub(&y()).to_fixed(60),
        "mul" => x.mul(&y()).to_fixed(60),
        "div" => x.div(&y()).to_fixed(60),
        "round2" => x.round(2).to_fixed(60),
        "neg" => x.neg().to_fixed(60),
        "fixed2" => x.to_fixed(2),
        "cmp" => format!("{:?}", x.cmp(&y())),
        other => panic!("unknown operation {other}"),
    }
}

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(
        args.is_empty() || args == ["--precision", "40"],
        "Use --precision 40 or no arguments"
    );
    let forty = !args.is_empty();
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for line in std::io::stdin().lock().lines() {
        let request: Vec<String> = serde_json::from_str(&line.expect("readable input")).expect("a JSON array");
        let result = catch_unwind(|| {
            if forty {
                answer::<40>(&request[0], &request[1], &request[2])
            } else {
                answer::<28>(&request[0], &request[1], &request[2])
            }
        })
        .unwrap_or_else(|_| "error".to_owned());
        writeln!(out, "{result}").expect("writable output");
    }
}
