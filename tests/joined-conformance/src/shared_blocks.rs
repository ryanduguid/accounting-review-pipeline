// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Behavioural conformance for the blocks Monthly Close Controls and the
//! Workpaper Review Gate share.

use super::*;

/// Both answer alike, or both refuse what Monthly Close Controls refuses.
fn agree_or_both_refuse(request: &Json) {
    let expected = monthly(request);
    let native = ready(request);
    if is_error(&expected) {
        assert!(is_error(&native), "{request:?}: {native:?}");
    } else {
        assert!(
            pyjson::py_eq(&native, &expected, false),
            "{request:?}: {native:?} != {expected:?}"
        );
    }
}

fn op(name: &str, value: Json) -> Json {
    Json::object([("op", Json::str(name)), ("value", value)])
}

#[test]
fn money_grammar_and_exact_values_agree() {
    let values = [
        "",
        "0",
        "-0.00",
        "1,234.50",
        "(1,234.50)",
        "+1.2",
        "١٢.٥",
        "1e2",
        "12,34",
        "(+1)",
        "--1",
        "NaN",
        "1_000",
    ];
    agree_or_both_refuse(&op("money", Json::Null));
    for value in values {
        agree_or_both_refuse(&op("money", Json::str(value)));
    }
}

#[test]
fn money_report_precision_agrees() {
    for value in ["0", "-0", "1.2", "1.2300", "1234567890123456789.0123"] {
        let request = op("format_money", Json::str(value));
        assert!(
            pyjson::py_eq(&ready(&request), &monthly(&request), false),
            "{value}"
        );
    }
}

#[test]
fn calendar_dates_agree() {
    for value in [
        "2024-02-29",
        "2026-02-29",
        "20260923",
        "2026-W39-3",
        "2026-09-23",
        "0000-01-01",
        "9999-12-31",
    ] {
        agree_or_both_refuse(&op("date", Json::str(value)));
    }
}

/// (tenant, account_id, account_code) for each loaded row.
fn identities(answer: &Json) -> Json {
    let rows = answer.at("ok").as_array().unwrap();
    Json::Array(
        rows.iter()
            .map(|row| {
                let fields = ["tenant", "account_id", "account_code"];
                Json::Array(fields.map(|key| row.at(key).clone()).into())
            })
            .collect(),
    )
}

#[test]
fn csv_headers_width_and_identifiers_agree() {
    let header = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit";
    let row = "2026-09-23,Fixture,Assets,one,Cash,090,0,0,0,0";
    let cases = [
        format!("{header}\n{row}\n"),
        format!("{header},Extra\n{row},x\n"),
        format!("{header}\n{row},extra\n"),
        format!("{header}\n{}", row.rsplit_once(',').unwrap().0),
        format!("{}\n{row}", header.replace("Tenant", "AccountID")),
        format!("{header}\n{}", row.replace("one", "one\u{200b}")),
        format!("{header}\n{row}\n{row}"),
        format!("\u{feff}{header}\r\n{row}\r\n"),
    ];
    let folder = Scratch::new();
    let source = folder.0.join("input.csv");
    for content in cases {
        std::fs::write(&source, &content).unwrap();
        let request = trial_balance(&source);
        let expected = monthly(&request);
        let native = ready(&request);
        if is_error(&expected) {
            assert!(is_error(&native), "{content:?}: {native:?}");
        } else {
            let (native, expected) = (identities(&native), identities(&expected));
            assert!(pyjson::py_eq(&native, &expected, false), "{content:?}");
        }
    }
}

#[test]
fn both_saved_json_parsers_refuse_ambiguous_or_unreadable_documents() {
    let payloads: [&[u8]; 9] = [
        b"[]",
        b"null",
        b"{}",
        br#"{"unexpected":1}"#,
        br#"{"x":1,"x":2}"#,
        br#"{"x":{"a":1,"a":2}}"#,
        b"\xff",
        br#"{"x":"#,
        b"\xef\xbb\xbf{}",
    ];
    for payload in payloads {
        let request = parse_pack(payload);
        assert!(is_error(&monthly(&request)), "{payload:?}");
        assert!(is_error(&ready(&request)), "{payload:?}");
    }
}

fn parse_pack(payload: &[u8]) -> Json {
    let hex: String = payload.iter().map(|byte| format!("{byte:02x}")).collect();
    Json::object([
        ("op", Json::str("parse_pack_json")),
        ("hex", Json::str(&hex)),
    ])
}

/// A pack holding each named member once, then the first again when
/// `duplicate` is set.
fn pack(members: &[&str], duplicate: bool) -> Json {
    let mut fields: Vec<String> = members.iter().map(|m| format!("\"{m}\":0")).collect();
    if duplicate {
        fields.push(fields[0].clone());
    }
    parse_pack(format!("{{{}}}", fields.join(",")).as_bytes())
}

/// The documents above also fail the member checks, so they alone would not
/// notice a parser that stopped refusing duplicates.
#[test]
fn each_saved_json_parser_refuses_a_duplicate_its_member_checks_would_accept() {
    let close = [
        "acknowledgement",
        "current_report_dates",
        "exceptions",
        "overall_status",
        "prior_report_dates",
        "source_sha256",
        "thresholds",
    ];
    let gate = [
        "acknowledgement",
        "engagement_type",
        "findings",
        "overall_status",
        "period_end",
        "preparer_initials",
        "review_boundary",
        "source_sha256",
        "thresholds",
    ];
    assert!(!is_error(&monthly(&pack(&close, false))));
    assert!(is_error(&monthly(&pack(&close, true))));
    assert!(!is_error(&ready(&pack(&gate, false))));
    assert!(is_error(&ready(&pack(&gate, true))));
}

fn output_guard(path: &Path) -> Json {
    Json::object([("op", Json::str("output_guard")), ("path", path_text(path))])
}

#[test]
fn checkout_guards_agree_for_every_marker_and_ancestor() {
    let folder = Scratch::new();
    let root = folder.resolved();
    let outside = output_guard(&root.join("outside"));
    assert!(pyjson::py_eq(&ready(&outside), &monthly(&outside), false));
    for marker in [".git", ".hg", ".svn", ".bzr"] {
        let checkout = root.join(&marker[1..]);
        std::fs::create_dir_all(checkout.join(marker)).unwrap();
        let destination = output_guard(&checkout.join("nested").join("output"));
        assert!(is_error(&monthly(&destination)), "{marker}");
        assert!(is_error(&ready(&destination)), "{marker}");
    }
}
