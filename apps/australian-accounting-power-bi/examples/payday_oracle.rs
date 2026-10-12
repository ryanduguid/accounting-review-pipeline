//! Reads one request per line and prints the generator's answer, for
//! tests/differential/generator_differential.py to compare with Python:
//!
//! - `notional BASE DUE RECEIVED [Q4RATE]` prints notional earnings at two places,
//!   with the 2026 Q4 rate optionally replaced;
//! - `business START COUNT` prints the date COUNT business days later, or the refusal;
//! - `rate DAY` prints the annual rate and whether its quarter is published.

use std::io::{BufRead, Write};

use australian_accounting_power_bi::calendar::add_business_days;
use australian_accounting_power_bi::date::Date;
use australian_accounting_power_bi::decimal::Decimal;
use australian_accounting_power_bi::payday::{GicRates, notional_earnings};

fn date(text: &str) -> Date {
    let parts: Vec<u32> = text.split('-').map(|p| p.parse().expect("number")).collect();
    Date::new(parts[0] as i32, parts[1], parts[2])
}

fn answer(parts: &[&str]) -> String {
    match parts[0] {
        "notional" => {
            let mut rates = GicRates::published();
            if let Some(rate) = parts.get(4) {
                rates = rates.with_rate((2026, 4), Decimal::parse(rate));
            }
            notional_earnings(&rates, &Decimal::parse(parts[1]), date(parts[2]), date(parts[3])).to_fixed(2)
        }
        "business" => match add_business_days(date(parts[1]), parts[2].parse().expect("count")) {
            Ok(day) => day.to_string(),
            Err(message) => message,
        },
        "rate" => {
            let rates = GicRates::published();
            let day = date(parts[1]);
            format!("{} {}", rates.annual_rate(day).to_fixed(4), rates.is_published(day))
        }
        other => panic!("unknown request {other}"),
    }
}

fn main() {
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for line in std::io::stdin().lock().lines() {
        let line = line.expect("readable input");
        let parts: Vec<&str> = line.split(' ').collect();
        writeln!(out, "{}", answer(&parts)).expect("writable output");
    }
}
