//! Port of tests/test_workpaper.py.

use super::*;
use crate::workpaper::faults;

fn row<'a>(result: &'a Json, table: &str, grant_id: &str, budget_line: &str) -> &'a Json {
    rows(result, table)
        .iter()
        .find(|r| r.at("grant_id").is_str(grant_id) && r.at("budget_line").is_str(budget_line))
        .unwrap()
}

fn assert_refused<T: std::fmt::Debug>(outcome: workpaper::Result<T>, needle: &str) {
    let message = refused(outcome);
    assert!(message.contains(needle), "{message:?} lacks {needle:?}");
}

#[test]
fn test_independent_totals_and_cash_are_distinct() {
    let fixture = Fixture::new();
    let result = fixture.review().unwrap();
    assert!(result.at("ledger_total").is_str("6500"));
    assert!(result.at("allocated_total").is_str("6300"));
    assert!(result.at("unallocated_total").is_str("200"));
    let movements = rows(&result, "funding_movements");
    let pair = |m: &Json| {
        (
            text(m.at("closing_cash_allocation")),
            text(m.at("unspent_funding_workpaper")),
        )
    };
    assert_eq!(pair(&movements[0]), ("14220".into(), "9100".into()));
    assert_eq!(pair(&movements[1]), ("7680".into(), "7600".into()));
    let mut codes: Vec<String> = findings(&result).into_iter().map(|(c, _)| c).collect();
    codes.sort();
    codes.dedup();
    assert_eq!(
        codes,
        [
            "MISSING_ALLOCATION_EVIDENCE",
            "MISSING_SOURCE_EVIDENCE",
            "OUTSIDE_AGREEMENT_PERIOD",
            "UNALLOCATED_SOURCE",
            "UNAPPROVED_ALLOCATION"
        ]
    );
}

#[test]
fn test_shared_invoice_cannot_be_overallocated() {
    let fixture = Fixture::new();
    fixture.edit_csv("allocations.csv", |rows| put(&mut rows[0], "amount", "601"));
    assert_refused(fixture.review(), "exceed");
}

#[test]
fn test_cash_allocation_cannot_exceed_paid_source() {
    let fixture = Fixture::new();
    fixture.edit_csv("allocations.csv", |rows| put(&mut rows[0], "cash_allocated", "481"));
    assert_refused(fixture.review(), "exceed");
}

#[test]
fn test_grant_period_is_separate_from_financial_year() {
    let fixture = Fixture::new();
    let result = fixture.review().unwrap();
    let b = row(&result, "budget_lines", "G-B", "Delivery");
    assert!(b.at("allocated").is_str("2400"));
    assert!(b.at("within_period").is_str("400"));
    assert!(result.at("ledger_total").is_str("6500"));
}

#[test]
fn test_unapproved_allocation_does_not_hide_supported_work() {
    let fixture = Fixture::new();
    let result = fixture.review().unwrap();
    let a = row(&result, "budget_lines", "G-A", "Delivery");
    assert!(a.at("allocated").is_str("3600"));
    assert!(a.at("approved_evidenced_in_period").is_str("600"));
}

#[test]
fn test_duplicate_ledger_id_is_refused() {
    let fixture = Fixture::new();
    fixture.edit_csv("ledger.csv", |rows| rows.push(rows[0].clone()));
    assert_refused(fixture.review(), "duplicate");
}

#[test]
fn test_zero_is_valid_but_missing_or_nonfinite_is_not() {
    let fixture = Fixture::new();
    assert!(fixture.review().unwrap().at("status").is_str("REVIEW"));
    for value in ["", "NaN", "Infinity", "-1", "1e3"] {
        fixture.edit_csv("ledger.csv", |rows| put(&mut rows[0], "amount", value));
        refused(fixture.review());
    }
}

#[test]
fn test_duplicate_json_fields_and_unknown_budget_fail() {
    let fixture = Fixture::new();
    let path = fixture.source().join("agreements.json");
    let saved = std::fs::read(&path).unwrap();
    std::fs::write(&path, r#"{"schema_version":1,"schema_version":1}"#).unwrap();
    assert_refused(fixture.review(), "Duplicate");
    std::fs::write(&path, saved).unwrap();
    fixture.edit_csv("allocations.csv", |rows| put(&mut rows[0], "budget_line", "Unknown"));
    refused(fixture.review());
}

#[test]
fn test_deeply_nested_agreements_are_malformed_input_not_a_traceback() {
    // The Python test patched json.loads to raise RecursionError; the port
    // nests past the decoder's limit, on a stack as large as the binary's.
    let fixture = Fixture::new();
    std::fs::write(fixture.source().join("agreements.json"), "[".repeat(20_000)).unwrap();
    let (status, stderr) = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || fixture.main("out"))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(status, 1);
    assert!(stderr.contains("nested too deeply"), "{stderr}");
}

#[test]
fn test_clean_evidence_can_reconcile_without_approving_acquittal() {
    let fixture = Fixture::new();
    fixture.edit_csv("ledger.csv", |rows| put(&mut rows[2], "evidence", "Fabricated receipt"));
    fixture.edit_csv("allocations.csv", |rows| {
        put(&mut rows[2], "grant_id", "G-A");
        put(&mut rows[3], "approved", "yes");
        put(&mut rows[3], "evidence", "Fabricated reviewed allocation");
        put(&mut rows[4], "amount", "500");
        put(&mut rows[4], "cash_allocated", "500");
    });
    let result = fixture.review().unwrap();
    assert!(result.at("status").is_str("RECONCILED"));
    assert!(text(result.at("scope")).contains("No grant eligibility"));
}

#[test]
fn test_csv_text_is_inert_and_previous_run_is_immutable() {
    let fixture = Fixture::new();
    fixture.edit_csv("allocations.csv", |rows| put(&mut rows[0], "evidence", "=1+1"));
    let result = fixture.review().unwrap();
    let output = fixture.root().join("output");
    fixture.write(&result, "output").unwrap();
    assert!(
        std::fs::read_to_string(output.join("allocation_evidence.csv"))
            .unwrap()
            .contains("'=1+1")
    );
    let saved = std::fs::read(output.join("workpaper.json")).unwrap();
    assert_eq!(refused(fixture.write(&result, "output")), "output");
    assert_eq!(saved, std::fs::read(output.join("workpaper.json")).unwrap());
    let written = pyjson::loads_bytes(&saved).unwrap();
    assert!(rows(&written, "allocation_evidence")[0].at("evidence").is_str("=1+1"));
}

#[test]
fn test_budget_boundary_is_exact_to_the_cent() {
    // G-A Delivery carries 3,600 of allocations.
    let fixture = Fixture::new();
    let budget =
        |delivery: &str| Json::object([("Delivery", Json::str(delivery)), ("Administration", Json::str("2000"))]);
    fixture.edit_agreement("G-A", &[("budget", budget("3600"))]);
    assert!(!fixture.codes("G-A/Delivery").contains(&"OVER_BUDGET".into()));
    fixture.edit_agreement("G-A", &[("budget", budget("3599.99"))]);
    assert!(fixture.codes("G-A/Delivery").contains(&"OVER_BUDGET".into()));
}

#[test]
fn test_both_agreement_dates_are_inside_the_period() {
    // A2 allocates L1 to G-B, which starts 2026-10-01; A5 allocates L4 to
    // G-A, which ends 2027-06-30.
    let fixture = Fixture::new();
    fixture.edit_csv("ledger.csv", |rows| {
        put(&mut rows[0], "date", "2026-10-01");
        put(&mut rows[3], "date", "2027-06-30");
    });
    let outside: Vec<String> = findings(&fixture.review().unwrap())
        .into_iter()
        .filter(|(code, _)| code == "OUTSIDE_AGREEMENT_PERIOD")
        .map(|(_, reference)| reference)
        .collect();
    assert_eq!(outside, ["A3"]);
}

#[test]
fn test_cash_and_unspent_shortfalls_are_each_flagged_to_the_cent() {
    // G-B allocates 2,400 of expenditure and 2,320 of cash.
    let fixture = Fixture::new();
    let cases = [
        ("2319.99", ("opening_unspent_funding", "1000"), Some("-0.01")),
        ("2399.99", ("opening_cash", "5000"), None),
    ];
    for (received, (key, opening), closing_cash) in cases {
        fixture.reset();
        fixture.edit_csv("receipts.csv", |rows| put(&mut rows[1], "amount", received));
        fixture.edit_agreement("G-B", &[(key, Json::str(opening))]);
        let result = fixture.review().unwrap();
        assert!(
            findings(&result).contains(&("FUNDING_SHORTFALL".into(), "G-B".into())),
            "{received}"
        );
        if let Some(closing_cash) = closing_cash {
            fixture.write(&result, "output-cash").unwrap();
            let rows = read_csv(&fixture.root().join("output-cash/funding_movements.csv"));
            let row = rows.iter().find(|r| get(r, "grant_id") == "G-B").unwrap();
            // A number stays a number, so spreadsheet totals include it.
            assert_eq!(get(row, "closing_cash_allocation"), closing_cash);
        }
    }
}

#[test]
fn test_formula_like_text_is_escaped_including_at_and_signed_expressions() {
    for text in ["@SUM(A1)", "-1+1", "=1+1", "+1", "-001", "-01.5"] {
        assert_eq!(workpaper::safe_cell(text), format!("'{text}"), "{text}");
    }
    for number in ["-1320", "0", "12.50"] {
        assert_eq!(workpaper::safe_cell(number), number);
    }
}

#[test]
fn test_cli_codes_and_no_output_for_invalid_sources() {
    let fixture = Fixture::new();
    assert_eq!(fixture.main("valid").0, 2);
    fixture.edit_csv("allocations.csv", |rows| put(&mut rows[0], "amount", "9999"));
    assert_eq!(fixture.main("invalid").0, 1);
    assert!(!fixture.root().join("invalid").exists());
}

#[test]
fn test_json_identity_and_evidence_require_plain_nonblank_text() {
    let fixture = Fixture::new();
    let original = fixture.agreements();
    let string = |cps: &[u32]| Json::Str(PyStr(cps.to_vec()));
    let utf8 = |s: &str| Json::str(s);
    let values = [
        utf8("   "),
        Json::Int("123".into()),
        utf8("Valid\n## Fake report"),
        utf8("\u{200b}"),
        utf8("\u{80}"),
        utf8("\u{301}"),
        utf8("\u{2028}"),
        string(&[0xD800]),
        utf8("\u{301}A"),
        utf8("A \u{301}B"),
        utf8("A\u{a0}\u{301}B"),
    ];
    for field in ["entity", "grant_id", "agreement_evidence"] {
        for value in &values {
            let mut document = original.clone();
            let owner = if field == "entity" {
                &mut document
            } else {
                let Json::Array(agreements) = field_mut(&mut document, "agreements") else {
                    panic!("a list")
                };
                &mut agreements[0]
            };
            set(owner, field, value.clone());
            fixture.save_agreements(&document);
            assert_refused(fixture.review(), "text");
        }
    }
}

#[test]
fn test_csv_evidence_and_ids_refuse_invisible_controls() {
    let fixture = Fixture::new();
    for (name, field) in [
        ("ledger.csv", "evidence"),
        ("allocations.csv", "evidence"),
        ("receipts.csv", "evidence"),
        ("ledger.csv", "ledger_id"),
    ] {
        let original = std::fs::read(fixture.source().join(name)).unwrap();
        for value in ["\u{200b}", "\u{80}", "\u{202e}", "\u{301}", "\u{301}A", "A \u{301}B"] {
            fixture.edit_csv(name, |rows| put(&mut rows[0], field, value));
            refused(fixture.review());
            std::fs::write(fixture.source().join(name), &original).unwrap();
        }
    }
}

#[test]
fn test_valid_unicode_labels_and_evidence_are_preserved() {
    let fixture = Fixture::new();
    let mut document = fixture.agreements();
    let entity = "Caf\u{e9} Community \u{4e2d}\u{6587}";
    let evidence = "Cafe\u{301}\u{308} reviewed agreement 1";
    set(&mut document, "entity", Json::str(entity));
    let Json::Array(agreements) = field_mut(&mut document, "agreements") else {
        panic!("a list")
    };
    set(&mut agreements[0], "agreement_evidence", Json::str(evidence));
    fixture.save_agreements(&document);
    let result = fixture.review().unwrap();
    assert!(result.at("entity").is_str(entity));
    assert!(
        rows(&result, "allocation_evidence")[0]
            .at("agreement_evidence")
            .is_str(evidence)
    );
    fixture.write(&result, "unicode").unwrap();
    let written = std::fs::read(fixture.root().join("unicode/workpaper.json")).unwrap();
    assert!(pyjson::loads_bytes(&written).unwrap().at("entity").is_str(entity));
}

#[test]
fn test_concurrent_destination_is_not_replaced() {
    let fixture = Fixture::new();
    let output = fixture.root().join("output");
    let result = fixture.review().unwrap();
    // Another process creates the destination between the check and the rename.
    let _hook = faults::install(|point, destination| {
        if point == "publish" {
            std::fs::create_dir(destination)?;
        }
        Ok(())
    });
    refused(fixture.write(&result, "output"));
    assert!(output.is_dir());
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
    assert!(fixture.temporary_directories().is_empty());
}

#[test]
fn test_publish_refuses_existing_file_and_symlink() {
    let fixture = Fixture::new();
    let staged = fixture.root().join("staged");
    std::fs::create_dir(&staged).unwrap();
    std::fs::write(staged.join("evidence"), "complete").unwrap();
    let destination = fixture.root().join("existing");
    std::fs::write(&destination, "original").unwrap();
    refused(workpaper::publish_directory(&staged, &destination, "existing"));
    assert_eq!(std::fs::read_to_string(&destination).unwrap(), "original");
    assert!(staged.join("evidence").is_file());
    let link = fixture.root().join("link");
    if symlink_file(&destination, &link).is_none() {
        return;
    }
    refused(workpaper::publish_directory(&staged, &link, "link"));
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read_to_string(&destination).unwrap(), "original");
}

#[test]
fn test_publish_failure_leaves_no_partial_output() {
    let fixture = Fixture::new();
    let result = fixture.review().unwrap();
    let _hook = faults::install(|point, _| match point {
        "publish" => Err(std::io::Error::other("Unsupported filesystem")),
        _ => Ok(()),
    });
    assert_refused(fixture.write(&result, "output"), "Unsupported filesystem");
    assert!(!fixture.root().join("output").exists());
    assert!(fixture.temporary_directories().is_empty());
}

#[test]
fn test_markdown_labels_are_literal() {
    let fixture = Fixture::new();
    let mut result = fixture.review().unwrap();
    set(
        &mut result,
        "entity",
        Json::str("[Fabricated](https://example.invalid) *entity*"),
    );
    fixture.write(&result, "output").unwrap();
    let markdown = std::fs::read_to_string(fixture.root().join("output/workpaper.md")).unwrap();
    assert!(
        markdown.contains(r"\[Fabricated\]\(https://example\.invalid\) \*entity\*"),
        "{markdown}"
    );
}

#[test]
fn test_failed_write_leaves_no_final_pack_and_can_retry() {
    let fixture = Fixture::new();
    let result = fixture.review().unwrap();
    let output = fixture.root().join("output");
    {
        let _hook = faults::install(|point, path| match point {
            "write" if path.file_name().is_some_and(|n| n == "workpaper.md") => {
                Err(std::io::Error::other("Fabricated disk failure"))
            }
            _ => Ok(()),
        });
        assert_refused(fixture.write(&result, "output"), "disk failure");
    }
    assert!(!output.exists());
    assert!(fixture.temporary_directories().is_empty());
    fixture.write(&result, "output").unwrap();
    let written = pyjson::loads_bytes(&std::fs::read(output.join("workpaper.json")).unwrap()).unwrap();
    assert_eq!(pyjson::dumps_indent2(&written), pyjson::dumps_indent2(&result));
}
