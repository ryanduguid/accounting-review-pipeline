//! The adapter's Power Query parsers load the exporter's canonical CSV shape.
//!
//! These checks read the root data-only contract: the exact 10-column header, the pinned
//! bytes, text account codes with a leading zero, Tenant plus AccountID as the row
//! identity, and the independent movement and YTD balance pairs.

mod common;

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use accounting_excel_toolkit::aged::hash;
use common::{Dec, Source, cell, records, repository};

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

fn contract() -> PathBuf {
    repository().join("contracts/xero-trial-balance-v1")
}

/// The sum of `debit - credit` over every row, by header name.
fn net(rows: &[Vec<String>], debit: &str, credit: &str) -> Dec {
    let column = |name: &str| HEADER.iter().position(|label| *label == name).unwrap();
    let (debit, credit) = (column(debit), column(credit));
    rows.iter()
        .map(|row| Dec::of(cell(row, debit)) - Dec::of(cell(row, credit)))
        .sum()
}

#[test]
fn data_only_contract_is_excel_importable_and_keeps_both_balance_pairs() {
    let expected = [
        (
            "passing.csv",
            "2cbe9997a8e7210936ff3c59b5d3fdb0041c1b375b0f9c88cf9ee30d0f356a09",
            "0",
            "0",
        ),
        (
            "failing_movement.csv",
            "702175df967b2854e7897cd27fdc4aca441e21b52438381108fabe88ff3153e4",
            "0.01",
            "0",
        ),
        (
            "failing_ytd.csv",
            "ec757f12d13866360fbab189228ebb425893c6f8b299809c6f8567bf5817c64b",
            "0",
            "0.01",
        ),
    ];
    for (name, digest, movement, ytd) in expected {
        let content = fs::read(contract().join("fixtures").join(name)).unwrap();
        assert!(
            !content.windows(2).any(|pair| pair == b"\r\n"),
            "{name} has CRLF"
        );
        assert_eq!(hash(&content), digest, "{name}");
        let text = String::from_utf8(content).unwrap();
        let mut table = records(name, &text).into_iter();
        let header = table.next().unwrap_or_default();
        assert_eq!(header, HEADER, "{name}");
        // DictReader skips blank records.
        let rows: Vec<_> = table.filter(|row| !row.is_empty()).collect();
        let identities: HashSet<_> = rows
            .iter()
            .map(|row| (cell(row, 1), cell(row, 3)))
            .collect();
        assert_eq!(
            identities.len(),
            rows.len(),
            "{name}: Tenant plus AccountID"
        );
        assert!(
            rows.iter().any(|row| cell(row, 5) == "090"),
            "{name} lost the 090 account code"
        );
        assert_eq!(net(&rows, "Debit", "Credit"), Dec::of(movement), "{name}");
        assert_eq!(net(&rows, "YTDDebit", "YTDCredit"), Dec::of(ytd), "{name}");
    }
}

#[test]
fn schema_file_is_exactly_the_canonical_header() {
    assert_eq!(
        fs::read(contract().join("schema.csv")).unwrap(),
        format!("{}\n", HEADER.join(",")).into_bytes()
    );
}

#[test]
fn documentation_uses_the_canonical_home_contract_and_active_ci() {
    let readme = Source::adapter("README.md");
    readme.has_all(&[
        "accounting-review-pipeline/tree/main/adapters/accounting-excel-toolkit",
        "../../contracts/xero-trial-balance-v1/",
        "../../.github/workflows/standard-library-components.yml",
        "accounting-review-pipeline/tree/main/packages/xero-trial-balance-export",
    ]);
    readme.lacks("accounting-excel-toolkit/actions/workflows/verify.yml");
}
