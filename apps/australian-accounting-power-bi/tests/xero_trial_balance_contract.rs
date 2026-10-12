//! The reference application imports the exporter's canonical CSV shape.
//!
//! These checks read the root data-only contract: the pinned bytes, the exact 10-column
//! header, one report date and one tenant per file, a stable non-empty AccountID on
//! every row, a text AccountCode that keeps its leading zero, and decimal balance columns.

mod common;

use std::collections::HashSet;
use std::fs;

use common::*;
use regex::Regex;
use sha2::{Digest, Sha256};

const HEADER: [&str; 10] = [
    "ReportDate",
    "Tenant",
    "Section",
    "AccountID",
    "AccountName",
    "AccountCode",
    "Debit",
    "Credit",
    "YTDDebit",
    "YTDCredit",
];

/// Whether `decimal.Decimal(str)` accepts the text. CPython strips whitespace, drops
/// every underscore, then reads a number, Infinity or a NaN with optional payload.
fn is_decimal(text: &str) -> bool {
    let finite = Regex::new(r"^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$").unwrap();
    let special = Regex::new(r"(?i)^[+-]?(?:inf(?:inity)?|s?nan\d*)$").unwrap();
    let text = text.trim().replace('_', "");
    finite.is_match(&text) || special.is_match(&text)
}

#[test]
fn model_import_boundary_uses_the_pinned_data_only_shape() {
    let contract = app().join("../../contracts/xero-trial-balance-v1/fixtures");
    for (name, digest) in [
        (
            "passing.csv",
            "2cbe9997a8e7210936ff3c59b5d3fdb0041c1b375b0f9c88cf9ee30d0f356a09",
        ),
        (
            "failing_movement.csv",
            "702175df967b2854e7897cd27fdc4aca441e21b52438381108fabe88ff3153e4",
        ),
        (
            "failing_ytd.csv",
            "ec757f12d13866360fbab189228ebb425893c6f8b299809c6f8567bf5817c64b",
        ),
    ] {
        let path = contract.join(name);
        let bytes = fs::read(&path).unwrap();
        let actual: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(actual, digest, "{name}");
        let text = String::from_utf8(bytes).unwrap();
        let mut reader = csv::Reader::from_reader(text.strip_prefix('\u{feff}').unwrap_or(&text).as_bytes());
        assert_eq!(reader.headers().unwrap().iter().collect::<Vec<_>>(), HEADER, "{name}");
        let rows: Vec<csv::StringRecord> = reader.records().map(Result::unwrap).collect();
        let column = |label: &str| HEADER.iter().position(|h| *h == label).unwrap();
        let distinct = |label: &str| rows.iter().map(|r| r[column(label)].to_owned()).collect::<HashSet<_>>();
        assert_eq!(distinct("ReportDate").len(), 1, "{name}");
        assert_eq!(distinct("Tenant").len(), 1, "{name}");
        assert!(rows.iter().all(|r| !r[column("AccountID")].is_empty()), "{name}");
        let identities: HashSet<_> = rows
            .iter()
            .map(|r| (&r[column("Tenant")], &r[column("AccountID")]))
            .collect();
        assert_eq!(identities.len(), rows.len(), "{name}");
        assert!(distinct("AccountCode").contains("090"), "{name}");
        for row in &rows {
            for label in ["Debit", "Credit", "YTDDebit", "YTDCredit"] {
                assert!(
                    is_decimal(&row[column(label)]),
                    "{name}: {label} {}",
                    &row[column(label)]
                );
            }
        }
    }
}

#[test]
fn documentation_uses_the_canonical_home_contract_and_active_ci() {
    let readme = read(&app().join("README.md"));
    for expected in [
        "accounting-review-pipeline/tree/main/apps/australian-accounting-power-bi",
        "../../contracts/xero-trial-balance-v1/",
        "../../.github/workflows/standard-library-components.yml",
        "accounting-review-pipeline/tree/main/packages/xero-trial-balance-export",
    ] {
        assert!(readme.contains(expected), "{expected}");
    }
    assert!(!readme.contains("australian-accounting-power-bi/actions/workflows/verify.yml"));
}

#[test]
fn decimal_grammar_matches_python_conversion() {
    let accepted = [
        "0",
        "-1.50",
        "1_000.25",
        " 2 ",
        ".5",
        "5.",
        "1e3",
        "-Infinity",
        "nan",
        "sNaN12",
        "1__0",
        "_1",
        "in_f",
    ];
    for text in accepted {
        assert!(is_decimal(text), "{text}");
    }
    for text in ["", "_", "1,000", "1 0", "$5", "1.2.3", "e5", "0x10", "."] {
        assert!(!is_decimal(text), "{text}");
    }
}
