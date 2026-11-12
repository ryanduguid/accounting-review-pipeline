// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

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

fn assert_refused_before_publication(fixture: &Fixture, expected: &str) {
    let evidence: Vec<_> = std::fs::read_dir(fixture.source())
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    assert_eq!(refused(fixture.review()), expected);
    assert_eq!(
        fixture.main("refused-input"),
        (1, format!("grant-workpaper: {expected}\n"))
    );
    assert!(!fixture.root().join("refused-input").exists());
    assert!(fixture.temporary_directories().is_empty());
    for (path, bytes) in evidence {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn agreement_header_refusals_preserve_source_and_publish_nothing() {
    for (field, value, message) in [
        (
            "currency",
            Json::str("USD"),
            "Version 1 requires one named entity, AUD and explicitly GST-inclusive amounts",
        ),
        (
            "amount_basis",
            Json::str("GST-exclusive"),
            "Version 1 requires one named entity, AUD and explicitly GST-inclusive amounts",
        ),
        ("ledger_start", Json::str("9999-12-31"), "Ledger period is reversed"),
        (
            "agreements",
            Json::Array(Vec::new()),
            "Supply ledger evidence and at least one reviewed agreement",
        ),
    ] {
        let fixture = Fixture::new();
        let mut header = fixture.agreements();
        set(&mut header, field, value);
        fixture.save_agreements(&header);
        assert_refused_before_publication(&fixture, message);
    }
    let fixture = Fixture::new();
    fixture.edit_csv("ledger.csv", Vec::clear);
    assert_refused_before_publication(&fixture, "Supply ledger evidence and at least one reviewed agreement");
}

#[test]
fn incomplete_duplicate_and_reversed_agreements_fail_before_publication() {
    for duplicate in [false, true] {
        let fixture = Fixture::new();
        let mut header = fixture.agreements();
        let Json::Array(agreements) = field_mut(&mut header, "agreements") else {
            panic!("agreements")
        };
        if duplicate {
            agreements.push(agreements[0].clone());
        } else {
            let Json::Object(fields) = &mut agreements[0] else {
                panic!("agreement")
            };
            fields.retain(|(key, _)| !key.is("agreement_evidence"));
        }
        fixture.save_agreements(&header);
        assert_refused_before_publication(&fixture, "Agreement IDs must be unique with complete fields");
    }
    for (field, value, message) in [
        (
            "start",
            Json::str("9999-12-31"),
            "Agreement dates and reviewed summary evidence are required",
        ),
        (
            "budget",
            Json::Object(Vec::new()),
            "Supply the reviewed budget-line mapping",
        ),
        (
            "budget",
            Json::Array(Vec::new()),
            "Supply the reviewed budget-line mapping",
        ),
        (
            "opening_cash",
            Json::Null,
            "Expected a non-negative decimal string with at most 2 places: None",
        ),
    ] {
        let fixture = Fixture::new();
        fixture.edit_agreement("G-A", &[(field, value)]);
        assert_refused_before_publication(&fixture, message);
    }
}

#[test]
fn ledger_and_allocation_refusals_precede_publication() {
    for (file, field, value, message) in [
        (
            "ledger.csv",
            "date",
            "0001-01-01",
            "Ledger rows must fall inside the declared ledger period",
        ),
        (
            "ledger.csv",
            "amount",
            "0",
            "Source expenses must be positive; cash paid cannot exceed the expense",
        ),
        (
            "ledger.csv",
            "cash_paid",
            "999999",
            "Source expenses must be positive; cash paid cannot exceed the expense",
        ),
        (
            "allocations.csv",
            "ledger_id",
            "unknown-source",
            "Allocation references an unknown ledger row or grant",
        ),
        (
            "allocations.csv",
            "grant_id",
            "unknown-grant",
            "Allocation references an unknown ledger row or grant",
        ),
        (
            "allocations.csv",
            "amount",
            "0",
            "Allocation expense must be positive and cover its cash allocation",
        ),
        (
            "allocations.csv",
            "cash_allocated",
            "999999",
            "Allocation expense must be positive and cover its cash allocation",
        ),
    ] {
        let fixture = Fixture::new();
        fixture.edit_csv(file, |rows| put(&mut rows[0], field, value));
        assert_refused_before_publication(&fixture, message);
    }
}

#[test]
fn date_money_and_quoted_csv_contracts_are_exact() {
    let whole = workpaper::money("1").unwrap();
    let cents = workpaper::money("1.00").unwrap();
    assert_eq!(whole, cents);
    assert_eq!((whole.to_string(), cents.to_string()), ("1".into(), "1.00".into()));
    for (input, message) in [
        ("20260630", "Use YYYY-MM-DD dates"),
        ("202x-06-30", "Invalid isoformat string: '202x-06-30'"),
        ("2026-W01X1", "Invalid isoformat string: '2026-W01X1'"),
        ("2026-02-29", "day 29 must be in range 1..28 for month 2 in year 2026"),
    ] {
        assert_eq!(refused(workpaper::iso(input)), message);
    }
    for (input, kind) in [
        ("null", "None"),
        ("true", "bool"),
        ("1", "int"),
        ("1.5", "float"),
        ("[]", "list"),
        ("{}", "dict"),
    ] {
        let value = pyjson::loads_bytes(input.as_bytes()).unwrap();
        assert_eq!(
            refused(workpaper::iso_json(&value)),
            format!("fromisoformat() argument must be str, not {kind}")
        );
    }
    let parsed = workpaper::rows(b"id,evidence\nx,\"a\"\"b\"\n", "id evidence", "id").unwrap();
    assert_eq!(
        parsed,
        vec![vec![("id".into(), "x".into()), ("evidence".into(), "a\"b".into())]]
    );
}

#[test]
fn csv_headers_width_and_field_size_are_checked_without_truncation() {
    for (input, expected) in [
        ("id,wrong\nx,evidence\n", "Expected exact CSV headings: id evidence"),
        ("id,evidence\nx\n", "Every CSV row must have the exact field count"),
        (
            "id,evidence\nx,proof,extra\n",
            "Every CSV row must have the exact field count",
        ),
    ] {
        assert_eq!(
            refused(workpaper::rows(input.as_bytes(), "id evidence", "id")),
            expected
        );
    }
    let evidence = "a".repeat(131_072);
    let accepted = format!("id,evidence\nx,{evidence}\n");
    let parsed = workpaper::rows(accepted.as_bytes(), "id evidence", "id").unwrap();
    assert_eq!(get(&parsed[0], "evidence"), evidence);
    let refused_input = format!("id,evidence\nx,{evidence}a\n");
    assert_eq!(
        refused(workpaper::rows(refused_input.as_bytes(), "id evidence", "id")),
        "field larger than field limit (131072)"
    );
    let fixture = Fixture::new();
    std::fs::write(fixture.source().join("agreements.json"), b"[1 2]").unwrap();
    assert_refused_before_publication(&fixture, "Expecting ',' delimiter: line 1 column 4 (char 3)");
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
fn inline_and_abbreviated_cli_paths_publish_the_same_reviewed_pack() {
    for arguments in [["--input=inputs", "--output=out"], ["--in=inputs", "--out=out"]] {
        let fixture = Fixture::new();
        let baseline = fixture.review().unwrap();
        fixture.write(&baseline, "expected-pack").unwrap();
        let args = arguments.map(String::from).to_vec();
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let code = crate::cli::main("grant-workpaper", &args, fixture.root(), &mut stdout, &mut stderr);
        assert_eq!(code, 2);
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            format!("grant-workpaper: REVIEW; {} review items\n", findings(&baseline).len())
        );
        assert!(stderr.is_empty());
        let mut expected_files: Vec<_> = std::fs::read_dir(fixture.root().join("expected-pack"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        let mut actual_files: Vec<_> = std::fs::read_dir(fixture.root().join("out"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        expected_files.sort();
        actual_files.sort();
        assert_eq!(actual_files, expected_files);
        for file in expected_files {
            assert_eq!(
                std::fs::read(fixture.root().join("out").join(&file)).unwrap(),
                std::fs::read(fixture.root().join("expected-pack").join(file)).unwrap()
            );
        }
        assert!(fixture.temporary_directories().is_empty());
    }
}

#[test]
fn split_receipts_preserve_funding_and_report_only_the_missing_evidence() {
    let fixture = Fixture::new();
    let mut expected_findings = findings(&fixture.review().unwrap());
    let agreements = fixture.agreements();
    let start = text(agreements.at("ledger_start"));
    let end = text(agreements.at("ledger_end"));
    fixture.edit_csv("receipts.csv", |rows| {
        let mut final_receipt = rows[0].clone();
        put(&mut rows[0], "amount", "4999.99");
        put(&mut rows[0], "date", &start);
        put(&mut final_receipt, "receipt_id", "R1-final-cent");
        put(&mut final_receipt, "amount", "0.01");
        put(&mut final_receipt, "date", &end);
        put(&mut final_receipt, "evidence", "");
        rows.push(final_receipt);
    });
    let result = fixture.review().unwrap();
    let movement = rows(&result, "funding_movements")
        .iter()
        .find(|row| row.at("grant_id").is_str("G-A"))
        .unwrap();
    assert!(movement.at("funding_received").is_str("5000.00"));
    assert!(movement.at("closing_cash_allocation").is_str("14220.00"));
    assert!(movement.at("unspent_funding_workpaper").is_str("9100.00"));
    expected_findings.push(("MISSING_RECEIPT_EVIDENCE".into(), "R1-final-cent".into()));
    expected_findings.sort();
    let mut actual_findings = findings(&result);
    actual_findings.sort();
    assert_eq!(actual_findings, expected_findings);
    assert!(result.at("status").is_str("REVIEW"));
}

#[test]
fn invalid_receipt_grants_dates_and_zero_amounts_fail_before_publication() {
    for (field, value, diagnostic) in [
        (
            "grant_id",
            "undeclared-grant",
            "Funding receipt must reference a declared grant and ledger period",
        ),
        (
            "date",
            "2026-06-30",
            "Funding receipt must reference a declared grant and ledger period",
        ),
        (
            "date",
            "2027-07-01",
            "Funding receipt must reference a declared grant and ledger period",
        ),
        ("amount", "0", "Funding receipts must be positive"),
    ] {
        let fixture = Fixture::new();
        fixture.edit_csv("receipts.csv", |rows| put(&mut rows[0], field, value));
        let (status, stderr) = fixture.main("refused-receipt");
        assert_eq!(status, 1, "{field}={value}: {stderr}");
        assert!(stderr.contains(diagnostic), "{stderr}");
        assert!(!fixture.root().join("refused-receipt").exists());
    }
}

#[test]
fn encoded_agreements_preserve_financial_results_and_refuse_malformed_bytes() {
    let fixture = Fixture::new();
    let baseline = fixture.review().unwrap();
    let path = fixture.source().join("agreements.json");
    let original = std::fs::read_to_string(&path).unwrap();
    let mut encoded = vec![0xff, 0xfe];
    encoded.extend(original.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&path, encoded).unwrap();
    let result = fixture.review().unwrap();
    assert!(result.at("ledger_total").is_str("6500"));
    assert!(result.at("allocated_total").is_str("6300"));
    assert_eq!(findings(&result), findings(&baseline));
    for (bytes, message) in [
        (
            &b"\xff\xfe\x00"[..],
            "'utf-16' codec can't decode byte 0x00 in position 2: truncated data",
        ),
        (&b"[]"[..], "Unexpected agreement schema"),
    ] {
        std::fs::write(&path, bytes).unwrap();
        let (status, stderr) = fixture.main("refused");
        assert_eq!(status, 1);
        assert!(stderr.contains(message), "{stderr}");
        assert!(!fixture.root().join("refused").exists());
    }
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
    let args = ["--input", SOURCE, "--output", "nested/new/output"]
        .map(String::from)
        .to_vec();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    assert_eq!(
        crate::cli::main("grant-workpaper", &args, fixture.root(), &mut stdout, &mut stderr),
        0
    );
    assert_eq!(stdout, b"grant-workpaper: RECONCILED; 0 review items\n");
    assert!(stderr.is_empty());
    let output = fixture.root().join("nested/new/output");
    assert_reconciled_example_pack(&output);
}

fn assert_reconciled_example_pack(output: &Path) {
    let mut files = std::fs::read_dir(output)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    files.sort();
    assert_eq!(
        files,
        [
            "allocation_evidence.csv",
            "budget_lines.csv",
            "contributions.csv",
            "findings.csv",
            "funding_movements.csv",
            "ledger_tieout.csv",
            "workpaper.json",
            "workpaper.md",
        ]
    );
    let written = pyjson::loads_bytes(&std::fs::read(output.join("workpaper.json")).unwrap()).unwrap();
    assert!(written.at("ledger_total").is_str("6500"));
    assert!(written.at("allocated_total").is_str("6500"));
    assert!(written.at("unallocated_total").is_str("0"));
    assert_eq!(
        std::fs::read_to_string(output.join("findings.csv"))
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["code,reference"]
    );
    assert!(
        std::fs::read_to_string(output.join("workpaper.md"))
            .unwrap()
            .contains("No differences found in the supplied evidence.")
    );
    assert_eq!(
        std::fs::read_dir(output.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        [std::ffi::OsString::from("output")]
    );
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
fn quoted_nonfinal_csv_fields_preserve_the_derived_financial_pack() {
    let fixture = Fixture::new();
    let mut expected = fixture.review().unwrap();
    let path = fixture.source().join("ledger.csv");
    let original = std::fs::read_to_string(&path).unwrap();
    let changed = original.replacen("Shared programme invoice", "\"Shared, programme invoice\"", 1);
    assert_ne!(changed, original);
    std::fs::write(path, changed).unwrap();
    let result = fixture.review().unwrap();
    assert_ne!(
        pyjson::dumps_indent2(expected.at("source_sha256")),
        pyjson::dumps_indent2(result.at("source_sha256"))
    );
    set(&mut expected, "source_sha256", result.at("source_sha256").clone());
    assert_eq!(pyjson::dumps_indent2(&result), pyjson::dumps_indent2(&expected));
    fixture.write(&result, "quoted-csv").unwrap();
    let saved = std::fs::read(fixture.root().join("quoted-csv/workpaper.json")).unwrap();
    assert_eq!(
        pyjson::dumps_indent2(&pyjson::loads_bytes(&saved).unwrap()),
        pyjson::dumps_indent2(&result)
    );
}

#[test]
fn raw_utf8_entity_is_preserved_and_its_quotes_are_literal_in_markdown() {
    let fixture = Fixture::new();
    let mut expected = fixture.review().unwrap();
    let entity = "\u{800} 한 \u{10000} \"Fabricated\"";
    let path = fixture.source().join("agreements.json");
    let original = std::fs::read_to_string(&path).unwrap();
    let changed = original.replacen("Fabricated Community Association", &entity.replace('"', "\\\""), 1);
    assert_ne!(changed, original);
    std::fs::write(path, changed).unwrap();
    let result = fixture.review().unwrap();
    assert!(result.at("entity").is_str(entity));
    set(&mut expected, "entity", Json::str(entity));
    set(&mut expected, "source_sha256", result.at("source_sha256").clone());
    assert_eq!(pyjson::dumps_indent2(&result), pyjson::dumps_indent2(&expected));
    fixture.write(&result, "raw-utf8").unwrap();
    let saved = std::fs::read(fixture.root().join("raw-utf8/workpaper.json")).unwrap();
    assert!(pyjson::loads_bytes(&saved).unwrap().at("entity").is_str(entity));
    let markdown = std::fs::read_to_string(fixture.root().join("raw-utf8/workpaper.md")).unwrap();
    assert!(
        markdown.contains("Entity: \u{800} 한 \u{10000} &quot;Fabricated&quot;. Currency: AUD. Basis: GST-inclusive.")
    );
}

#[test]
fn a_regular_file_parent_prevents_publication_without_changing_the_blocker() {
    let fixture = Fixture::new();
    let blocker = fixture.root().join("blocker");
    std::fs::write(&blocker, b"Fabricated output-parent sentinel").unwrap();
    let arguments = ["--input", SOURCE, "--output", "blocker/out"].map(String::from);
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let code = crate::cli::main("grant-workpaper", &arguments, fixture.root(), &mut stdout, &mut stderr);
    assert_eq!(code, 1);
    assert!(stdout.is_empty());
    let diagnostic = String::from_utf8(stderr).unwrap();
    assert!(diagnostic.contains("blocker"), "{diagnostic}");
    assert_eq!(std::fs::read(blocker).unwrap(), b"Fabricated output-parent sentinel");
    assert!(!fixture.root().join("blocker/out").exists());
    assert!(fixture.temporary_directories().is_empty());
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
