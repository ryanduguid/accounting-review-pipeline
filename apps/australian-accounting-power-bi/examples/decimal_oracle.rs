//! Reads `op a b` lines, applies the generator's decimal module and prints each result
//! at 60 places. tests/differential/decimal_differential.py compares it with CPython.

use std::io::{BufRead, Write};

use australian_accounting_power_bi::decimal::Decimal;

fn main() {
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("readable input");
        let parts: Vec<&str> = line.split(' ').collect();
        let a = Decimal::parse(parts[1]);
        let b = || Decimal::parse(parts[2]);
        let result = match parts[0] {
            "add" => a.add(&b()).to_fixed(60),
            "sub" => a.sub(&b()).to_fixed(60),
            "mul" => a.mul(&b()).to_fixed(60),
            "div" => a.div(&b()).to_fixed(60),
            "round2" => a.round(2).to_fixed(60),
            "neg" => a.neg().to_fixed(60),
            "fixed2" => a.to_fixed(2),
            "cmp" => format!("{:?}", a.cmp(&b())),
            other => panic!("unknown operation {other}"),
        };
        writeln!(out, "{result}").expect("writable output");
    }
}
