//! Double-entry integrity of the sample fixtures, and their regeneration.

mod common;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use australian_accounting_power_bi::decimal::Decimal;
use australian_accounting_power_bi::fixtures;
use common::*;

type Row = HashMap<String, String>;

const FIXTURE_FILES: [&str; 6] = [
    "sample-entities.csv",
    "sample-chart-of-accounts.csv",
    "sample-general-ledger.csv",
    "sample-budgets.csv",
    "sample-payroll-super.csv",
    "sample-ato-benchmarks.csv",
];

/// A fresh scratch directory under the system temporary directory.
fn scratch(label: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("pbi-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn sum(values: impl Iterator<Item = Decimal>) -> Decimal {
    values.fold(Decimal::zero(), |total, value| total.add(&value))
}

/// Every journal must have sum(Debits) == sum(Credits) and a zero net amount.
fn journals_balanced(samples: &Path) -> Result<(), String> {
    let mut journals: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for row in csv_rows(&samples.join("sample-general-ledger.csv")) {
        journals.entry(row["JournalID"].clone()).or_default().push(row);
    }
    if journals.is_empty() {
        return Err("GL fixture must contain journals".to_owned());
    }
    for (id, lines) in &journals {
        let column = |name: &str| sum(lines.iter().map(|line| Decimal::parse(&line[name])));
        let (debit, credit, net) = (column("Debit"), column("Credit"), column("Amount"));
        if debit != credit {
            return Err(format!(
                "Journal {id} is unbalanced: Debits ({}) != Credits ({})",
                debit.to_fixed(2),
                credit.to_fixed(2)
            ));
        }
        if !net.is_zero() {
            return Err(format!("Journal {id} net amount is non-zero: {}", net.to_fixed(2)));
        }
    }
    Ok(())
}

/// Each bilateral pair balances separately for its balance sheet and P&L legs.
fn intercompany_matched(samples: &Path) -> Result<(), String> {
    let classes: HashMap<String, String> = csv_rows(&samples.join("sample-chart-of-accounts.csv"))
        .into_iter()
        .map(|row| (row["AccountCode"].clone(), row["Class"].clone()))
        .collect();
    let rows: Vec<_> = csv_rows(&samples.join("sample-general-ledger.csv"))
        .into_iter()
        .filter(|row| row["IsIntercompany"] == "TRUE")
        .collect();
    if rows.is_empty() {
        return Err("no intercompany rows".to_owned());
    }
    let mut groups: BTreeMap<(String, String, &str), Vec<&Row>> = BTreeMap::new();
    for row in &rows {
        let mut pair = [row["EntityID"].as_str(), row["IntercompanyEntityID"].as_str()];
        pair.sort_unstable();
        let leg = if ["Revenue", "Expense"].contains(&classes[&row["AccountCode"]].as_str()) {
            "PL"
        } else {
            "BS"
        };
        groups
            .entry((row["PostingDate"].clone(), pair.join("/"), leg))
            .or_default()
            .push(row);
    }
    for (key, lines) in &groups {
        let entities: BTreeSet<&str> = lines.iter().map(|line| line["EntityID"].as_str()).collect();
        if entities.len() != 2 || !sum(lines.iter().map(|line| Decimal::parse(&line["Amount"]))).is_zero() {
            return Err(format!("Unmatched pair {key:?}"));
        }
    }
    Ok(())
}

#[test]
fn balance_oracle_rejects_a_cent_below_float_precision() {
    let root = scratch("cent");
    let ledger = "JournalID,Debit,Credit,Amount\nJNL1,10000000000000000.01,10000000000000000.00,0.01\n";
    fs::write(root.join("sample-general-ledger.csv"), ledger).unwrap();
    let outcome = journals_balanced(&root);
    fs::remove_dir_all(&root).unwrap();
    assert!(outcome.is_err_and(|message| message.contains("unbalanced")));
}

#[test]
fn sample_files_exist() {
    for name in FIXTURE_FILES {
        assert!(samples().join(name).is_file(), "Missing required fixture: {name}");
    }
}

#[test]
fn general_ledger_journals_strictly_balanced() {
    journals_balanced(&samples()).unwrap();
}

/// All account codes must be unique and properly categorised.
#[test]
fn chart_of_accounts_uniqueness() {
    let mut codes = BTreeSet::new();
    for row in csv_rows(&samples().join("sample-chart-of-accounts.csv")) {
        assert!(
            codes.insert(row["AccountCode"].clone()),
            "Duplicate account code: {}",
            row["AccountCode"]
        );
        assert!(["Asset", "Liability", "Equity", "Revenue", "Expense"].contains(&row["Class"].as_str()));
        assert!(["Debit", "Credit"].contains(&row["NormalBalance"].as_str()));
    }
}

#[test]
fn intercompany_transactions_match_across_group() {
    intercompany_matched(&samples()).unwrap();
}

#[test]
fn pair_oracle_rejects_a_missing_balanced_counterparty() {
    let text = read(&samples().join("sample-general-ledger.csv"));
    let records = csv_records(&samples().join("sample-general-ledger.csv"));
    let header = &records[0];
    let column = |name: &str| header.iter().position(|h| h == name).unwrap();
    let target = records[1..]
        .iter()
        .find(|row| row[column("EntityID")] == "ENT002" && row[column("IsIntercompany")] == "TRUE")
        .map(|row| row[column("JournalID")].clone())
        .unwrap();
    let kept: String = splitlines(&text)
        .into_iter()
        .filter(|line| !line.starts_with(&format!("{target},")))
        .map(|line| format!("{line}\r\n"))
        .collect();
    let root = scratch("pair");
    fs::copy(
        samples().join("sample-chart-of-accounts.csv"),
        root.join("sample-chart-of-accounts.csv"),
    )
    .unwrap();
    fs::write(root.join("sample-general-ledger.csv"), kept).unwrap();
    let (balanced, matched) = (journals_balanced(&root), intercompany_matched(&root));
    fs::remove_dir_all(&root).unwrap();
    balanced.unwrap();
    assert!(matched.is_err_and(|message| message.contains("Unmatched pair")));
}

/// README calls samples/ the deterministic output of the generator, so regenerating
/// must reproduce the committed bytes, LF line endings included.
#[test]
fn regenerating_reproduces_the_committed_fixtures_byte_for_byte() {
    let root = scratch("regenerate");
    fixtures::generate(&root).unwrap();
    let differing: Vec<&str> = FIXTURE_FILES
        .into_iter()
        .filter(|name| fs::read(root.join(name)).unwrap() != fs::read(samples().join(name)).unwrap())
        .collect();
    fs::remove_dir_all(&root).unwrap();
    assert!(
        differing.is_empty(),
        "Regenerated fixtures differ from the committed bytes: {}",
        differing.join(", ")
    );
}
