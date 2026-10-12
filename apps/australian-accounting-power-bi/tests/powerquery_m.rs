//! Static syntax and fixture checks for TMDL-embedded Power Query expressions.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use regex::Regex;

/// The fenced M source of each named TMDL expression. A body runs from the line after
/// its opening fence to the first line that holds only a closing fence.
fn named_expressions() -> BTreeMap<String, String> {
    let text = read(&definition().join("expressions.tmdl"));
    let header = Regex::new(r"(?m)^expression\s+('[^']+'|[^\s=]+)\s*=\s*```[\t ]*\r?\n").unwrap();
    let fence = Regex::new(r"(?m)^[\t ]*```[\t ]*$").unwrap();
    let mut expressions = BTreeMap::new();
    let mut at = 0;
    while let Some(found) = header.captures_at(&text, at) {
        let whole = found.get(0).unwrap();
        let Some(end) = fence.find_at(&text, whole.end()) else {
            at = whole.start() + 1;
            continue;
        };
        let name = found[1].trim_matches('\'').to_owned();
        assert!(!expressions.contains_key(&name), "Duplicate named expression: {name}");
        expressions.insert(name, text[whole.end()..end.start()].to_owned());
        at = end.start();
    }
    expressions
}

/// The indentation of an M let binding (`Name =` or `#"Quoted name" =`, never `==`).
fn binding_indent(line: &str) -> Option<&str> {
    let binding = Regex::new(r#"^([\t ]*)(?:[A-Za-z_]\w*|#"[^"]+")\s*="#).unwrap();
    let found = binding.captures(line)?;
    if line[found.get(0).unwrap().end()..].starts_with('=') {
        return None;
    }
    Some(found.get(1).unwrap().as_str())
}

#[test]
fn expected_named_expressions_are_embedded_in_tmdl() {
    assert!(
        definition().join("expressions.tmdl").is_file(),
        "expressions.tmdl must exist"
    );
    let names: Vec<String> = named_expressions().into_keys().collect();
    assert_eq!(
        names,
        [
            "Dim_Date_AU",
            "Fx_MoneyDisplay",
            "Fx_RequireRows",
            "Fx_ValidateABN",
            "IndustryLookup",
            "Source_Dim_Account",
            "Source_Dim_Entity",
            "Source_Fact_ATOBenchmark",
            "Source_Fact_Budget",
            "Source_Fact_GeneralLedger",
            "Source_Fact_PayrollSuper",
        ]
    );
}

/// Every M script must have balanced let ... in blocks.
#[test]
fn named_expressions_have_balanced_let_in_blocks() {
    let let_line = Regex::new(r"(?m)^[\t ]*let[\t ]*$").unwrap();
    let in_line = Regex::new(r"(?m)^[\t ]*in[\t ]*$").unwrap();
    for (name, content) in named_expressions() {
        let lets = let_line.find_iter(&content).count();
        assert!(lets >= 1, "{name} missing 'let'");
        assert_eq!(
            lets,
            in_line.find_iter(&content).count(),
            "{name} has unbalanced let/in blocks"
        );
    }
}

/// A binding that follows another without a comma is an M syntax error. A binding is a
/// line at the indentation of the first binding after `let`.
#[test]
fn let_bindings_are_separated_by_commas() {
    for (name, content) in named_expressions() {
        let lines = splitlines(&content);
        for (index, line) in lines.iter().enumerate() {
            if line.trim() != "let" {
                continue;
            }
            let first = lines[index + 1..]
                .iter()
                .find(|text| !text.trim().is_empty())
                .copied()
                .unwrap_or("");
            let Some(indent) = binding_indent(first) else { continue };
            let mut previous: Option<&str> = None;
            for text in &lines[index + 1..] {
                let stripped = text.trim();
                if stripped.is_empty() || stripped.starts_with("//") {
                    continue;
                }
                let deeper = text.starts_with(&format!("{indent} ")) || text.starts_with(&format!("{indent}\t"));
                if text.starts_with(indent) && !deeper {
                    if stripped == "in" {
                        break;
                    }
                    if let (Some(_), Some(before)) = (binding_indent(text), previous) {
                        let shown: String = stripped.chars().take(40).collect();
                        assert!(before.trim_end().ends_with(','), "{name}: no comma before '{shown}'");
                    }
                }
                previous = Some(text);
            }
        }
    }
}

/// The ABN expression uses the statutory weights and modulus 89.
#[test]
fn abn_validator_m_logic() {
    let content = &named_expressions()["Fx_ValidateABN"];
    assert!(content.contains("10, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19"));
    assert!(content.contains("Number.Mod"));
    assert!(content.contains("89"));
}

#[test]
fn abn_validator_refuses_unapproved_characters() {
    let content = &named_expressions()["Fx_ValidateABN"];
    assert!(content.contains(r#"Text.Remove(Text.From(abnInput), {" ", "-"})"#));
    assert!(content.contains(r#"Text.Select(RawText, {"0".."9"}) <> RawText"#));
}

#[test]
fn csv_imports_raise_cell_errors_before_model_loading() {
    for (name, content) in named_expressions() {
        if name.starts_with("Source_") {
            assert!(content.contains("Table.SelectRowsWithErrors("), "{name}");
            assert!(content.contains(&format!(r#"error Error.Record("{name}""#)), "{name}");
        }
    }
}

#[test]
fn financial_imports_use_fixed_decimal_columns() {
    let expressions = named_expressions();
    let cases: [(&str, &[&str]); 3] = [
        ("Source_Fact_Budget", &["BudgetAmount"]),
        ("Source_Fact_GeneralLedger", &["Debit", "Credit", "Amount"]),
        (
            "Source_Fact_PayrollSuper",
            &[
                "GrossEarnings",
                "QualifyingEarnings_CodeQ",
                "SuperLiability_CodeL",
                "SGC_Shortfall",
                "GIC_NominalInterest",
            ],
        ),
    ];
    for (name, columns) in cases {
        for column in columns {
            assert!(
                expressions[name].contains(&format!(r#"{{"{column}", Currency.Type}}"#)),
                "{name} {column}"
            );
        }
    }
    assert!(!read(&definition().join("model.tmdl")).contains("returnErrorValuesAsNull"));
}

/// Run Fx_ValidateABN's own weights over every ABN in the entity master, so no
/// fabricated group entity fails the model's own validator.
#[test]
fn sample_entity_abns_pass_the_shipped_checksum() {
    let source = Regex::new(r"Weights = \{([^}]*)\}").unwrap();
    let expressions = named_expressions();
    let weights_text = source
        .captures(&expressions["Fx_ValidateABN"])
        .expect("Fx_ValidateABN must declare its weights");
    let weights: Vec<i64> = weights_text[1].split(',').map(|w| w.trim().parse().unwrap()).collect();
    assert_eq!(weights.len(), 11);
    let mut rejected = Vec::new();
    for row in csv_rows(&samples().join("sample-entities.csv")) {
        let mut digits: Vec<i64> = row["ABN"]
            .chars()
            .filter_map(|c| c.to_digit(10))
            .map(i64::from)
            .collect();
        let valid = digits.len() == 11 && {
            digits[0] -= 1;
            digits.iter().zip(&weights).map(|(d, w)| d * w).sum::<i64>() % 89 == 0
        };
        if !valid {
            rejected.push(format!("{}:{}", row["EntityID"], row["ABN"]));
        }
    }
    assert!(
        rejected.is_empty(),
        "sample-entities.csv holds ABNs that fail the modulus-89 checksum: {rejected:?}"
    );
}

#[test]
fn csv_source_column_counts_match_fixture_rows() {
    let width = Regex::new(r"\bColumns=(\d+)\b").unwrap();
    let expressions = named_expressions();
    for (query, sample) in [
        ("Source_Dim_Account", "sample-chart-of-accounts.csv"),
        ("Source_Dim_Entity", "sample-entities.csv"),
        ("Source_Fact_ATOBenchmark", "sample-ato-benchmarks.csv"),
        ("Source_Fact_Budget", "sample-budgets.csv"),
        ("Source_Fact_GeneralLedger", "sample-general-ledger.csv"),
        ("Source_Fact_PayrollSuper", "sample-payroll-super.csv"),
    ] {
        let declared: usize = width
            .captures(&expressions[query])
            .unwrap_or_else(|| panic!("{query} must declare its CSV width"))[1]
            .parse()
            .unwrap();
        let widths: BTreeSet<usize> = csv_records(&samples().join(sample)).iter().map(Vec::len).collect();
        assert_eq!(
            widths,
            BTreeSet::from([declared]),
            "{query} declares {declared} columns but {sample} has row widths {widths:?}"
        );
    }
}

#[test]
fn dim_account_preserves_quoted_account_name() {
    assert!(named_expressions()["Source_Dim_Account"].contains("QuoteStyle=QuoteStyle.Csv"));
    let records = csv_records(&samples().join("sample-chart-of-accounts.csv"));
    let account = records.iter().find(|row| row[0] == "200").expect("account 200");
    assert_eq!(account.len(), 9);
    assert_eq!(account[1], "Property, Plant & Equipment");
}

// Table.SelectRowsWithErrors sees conversion errors only. An invalid ABN is still valid
// text, and a validly typed ledger can still hold an unbalanced journal, so both need
// their own refresh guards.

#[test]
fn entity_import_applies_the_abn_checksum() {
    let content = &named_expressions()["Source_Dim_Entity"];
    assert!(content.contains("Fx_ValidateABN([ABN])"));
    assert!(content.contains(r#"error Error.Record("Source_Dim_Entity", "Invalid ABN""#));
}

#[test]
fn general_ledger_import_refuses_an_unbalanced_journal() {
    let content = &named_expressions()["Source_Fact_GeneralLedger"];
    assert!(content.contains(r#"Table.Group(TypedTable, {"JournalID"}"#));
    assert!(content.contains("[TotalDebit] <> [TotalCredit]"));
    assert!(content.contains(r#"error Error.Record("Source_Fact_GeneralLedger", "Unbalanced journal""#));
    // Grouped by JournalID alone: adding EntityID would impose a per-entity balance
    // and refuse a cross-entity journal that balances overall.
    assert!(!content.contains(r#"{"JournalID", "EntityID"}"#));
}
