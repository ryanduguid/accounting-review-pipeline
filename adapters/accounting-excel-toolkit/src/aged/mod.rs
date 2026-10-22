pub mod amount;
pub mod cli;
pub mod date;
pub mod evidence;
mod isodate;
pub mod json;
mod path;
pub mod replay;
mod tables;
pub mod text;

use crate::transactions::{csv, date as transaction_date, text::space};
use amount::Amount;
pub use json::Json;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use sha2::{Digest, Sha256};

pub const BUCKETS: &[&str] = &["< 1 Month", "1 Month", "2 Months", "3 Months", "Older"];
pub const TITLE: &str = "Aged Receivables Summary";
pub const MANIFEST_FIELDS: &[&str] = &[
    "report_name",
    "entity",
    "generated_at",
    "as_at",
    "ageing_basis",
    "accounting_basis",
    "currency",
    "filters",
    "population",
    "source_format",
    "source_sha256",
];
const COMPARABLE: &[&str] = &[
    "entity",
    "as_at",
    "currency",
    "accounting_basis",
    "filters",
    "population",
];
const BOUNDARY: &str = "Summary arithmetic only. Detail, subsequent receipts and authorised review are required for debtor decisions.";
pub fn hash(content: &[u8]) -> String {
    Sha256::digest(content)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn string(value: &Json) -> Option<String> {
    value.as_pystr()?.to_utf8()
}
fn field(value: &Json, key: &str) -> Option<String> {
    string(value.get(key)?)
}
fn nonblank(text: &str) -> bool {
    !text.trim_matches(space).is_empty()
}
fn nonblank_value(value: &Json) -> bool {
    value
        .as_pystr()
        .is_some_and(|s| s.0.iter().any(|&cp| !char::from_u32(cp).is_some_and(space)))
}
fn issue(issues: &mut Vec<Json>, text: impl AsRef<str>) {
    issues.push(Json::str(text.as_ref()));
}
fn parsed_amount(value: &Json) -> Result<Amount, String> {
    let text = value
        .as_pystr()
        .ok_or("Amounts must be supplied as decimal text.")?
        .to_utf8()
        .ok_or("Expected a finite signed amount with at most 4 decimal places.")?;
    amount::parse(&text)
}
pub fn records(content: &[u8]) -> Result<Vec<Vec<String>>, String> {
    let text = std::str::from_utf8(content).map_err(|e| e.to_string())?;
    csv::Reader::strict(text.strip_prefix('\u{feff}').unwrap_or(text))
        .map(|r| r.map_err(|e| e.0))
        .collect()
}
fn headers(records: &[Vec<String>]) -> Vec<usize> {
    records
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            matches!(r.first().map(String::as_str), Some("Contact" | "Customer"))
                && r.iter().any(|s| s == "Total")
                && BUCKETS.iter().all(|name| r.iter().any(|s| s == name))
        })
        .map(|(i, _)| i)
        .collect()
}

mod metadata;
use metadata::metadata;

fn tie(label: &str, left: &Amount, right: &Amount, ties: &mut Vec<Json>, issues: &mut Vec<Json>) {
    let delta = left.sub(right);
    let agrees = delta.agrees();
    ties.push(Json::object([
        ("check", Json::str(label)),
        ("left", Json::str(&left.to_string())),
        ("right", Json::str(&right.to_string())),
        ("difference", Json::str(&delta.to_string())),
        ("agrees", Json::Bool(agrees)),
    ]));
    if !agrees {
        issue(issues, format!("{label}: unresolved difference {delta}."));
    }
}

mod review;
pub use review::inspect_export;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fabricated_three_way_tie_and_half_cent_boundary() {
        let source = include_bytes!("../../samples/sample-aged-receivables-review.csv");
        let manifest = json::loads(include_str!(
            "../../samples/sample-aged-receivables-manifest.json"
        ))
        .unwrap();
        let control = json::loads(include_str!(
            "../../samples/sample-aged-receivables-control.json"
        ))
        .unwrap();
        let review = inspect_export(source, &manifest, Some(&control)).unwrap();
        assert!(review.at("summary_status").is_str("PASS"));
        assert!(review.at("debtor_decisions").is_str("REVIEW"));
        assert_eq!(review.at("rows").as_array().unwrap().len(), 3);
        assert_eq!(review.at("ties").as_array().unwrap().len(), 13);
        for (text, expected) in [("0.0049", true), ("0.005", false), ("-0.005", false)] {
            assert_eq!(amount::parse(text).unwrap().agrees(), expected);
        }
        assert_eq!(amount::parse("-0.0000").unwrap().to_string(), "-0.0000");
        assert_eq!(
            amount::parse("999999999999999.9999")
                .unwrap()
                .add(&amount::parse("-999999999999999.9998").unwrap())
                .to_string(),
            "0.0001"
        );
    }
    #[test]
    fn strict_csv_retains_cr_and_logical_blank_records() {
        assert_eq!(
            records(b"a,b\r\r\"line\rcontinued\",c\r\n").unwrap(),
            vec![vec!["a", "b"], vec![], vec!["line\rcontinued", "c"]]
        );
        assert_eq!(
            records(b"\"unfinished").unwrap_err(),
            "unexpected end of data"
        );
        assert_eq!(records(b"\"a\"x,b").unwrap_err(), "',' expected after '\"'");
    }
    #[test]
    fn timestamps_have_a_timezone_and_valid_calendar() {
        for text in [
            "2026-10-01T00:00:00+10:00",
            "20261001T12Z",
            "2026-W40-4T24:00Z",
            "2026-10-01T12:34:56.123456789-00:30:12.1",
        ] {
            assert!(date::generated(text), "{text}");
        }
        for text in [
            "2026-10-01",
            "2026-10-01T12:30",
            "2026-02-29T12Z",
            "2026-10-01T12+24:00",
            "9999-12-31T24Z",
        ] {
            assert!(!date::generated(text), "{text}");
        }
    }
}
