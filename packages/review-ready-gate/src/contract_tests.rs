// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use crate::{
    decimal::Decimal,
    engine, loader,
    models::SelfReview,
    pyjson::{self, Json, PyStr},
    report,
    test_support::{self, TempDir},
    viewer,
};
use std::{fs, path::Path};

fn set(value: &mut Json, key: &str, replacement: Json) {
    let Json::Object(fields) = value else {
        panic!("fixture object")
    };
    fields.iter_mut().find(|(k, _)| k.is(key)).unwrap().1 = replacement;
}
fn note(path: &Path, initials: &str, date: &str, comment: &str) {
    fs::write(
        path,
        pyjson::canonical(&Json::object([
            ("reviewer_initials", Json::str(initials)),
            ("reviewed_on", Json::str(date)),
            ("comment", Json::str(comment)),
        ])),
    )
    .unwrap();
}
#[test]
fn acknowledgement_schema_types_controls_and_duplicate_members_fail_closed() {
    let tmp = TempDir::new();
    let path = tmp.join("note.json");
    note(
        &path,
        " RD ",
        "2026-04-12",
        " A fabricated review.\nSecond line. ",
    );
    let baseline = fs::read_to_string(&path).unwrap();
    let valid = loader::load_reviewer_acknowledgement(Some(&path))
        .unwrap()
        .unwrap();
    assert_eq!(valid.reviewer_initials, "RD");
    assert_eq!(valid.comment, "A fabricated review.\nSecond line.");
    for (key, value) in [
        ("reviewer_initials", Json::Null),
        ("reviewer_initials", Json::str("")),
        ("reviewer_initials", Json::str("ABCDEFGHIJKLM")),
        ("reviewer_initials", Json::str("A\u{200b}")),
        ("comment", Json::Bool(true)),
        ("comment", Json::str(" \t")),
        ("comment", Json::str("text\u{202e}")),
        ("reviewed_on", Json::int(20260412)),
        ("reviewed_on", Json::str("20260412")),
        ("reviewed_on", Json::str("2026-W15-7")),
    ] {
        let mut document = pyjson::loads(&baseline).unwrap();
        set(&mut document, key, value);
        fs::write(&path, pyjson::canonical(&document)).unwrap();
        assert!(
            loader::load_reviewer_acknowledgement(Some(&path)).is_err(),
            "{key}"
        );
    }
    for bytes in [b"[]".as_slice(), b"{}", b"\xff", br#"{"reviewer_initials":"RD","reviewer_initials":"XY","reviewed_on":"2026-04-12","comment":"Review"}"#] {
        fs::write(&path, bytes).unwrap();
        assert!(loader::load_reviewer_acknowledgement(Some(&path)).is_err());
    }
    fs::write(&path, format!("\u{feff}{baseline}")).unwrap();
    assert!(loader::load_reviewer_acknowledgement(Some(&path)).is_ok());
}
#[test]
fn acknowledgement_requires_separation_then_date_and_never_changes_status() {
    let tmp = TempDir::new();
    let path = tmp.join("note.json");
    for (initials, date, expected) in [
        (" ab ", "2026-01-01", "must differ"),
        ("XY", "2026-03-30", "earlier"),
    ] {
        note(&path, initials, date, "Fabricated review.");
        let error = engine::review_pack(
            "bas",
            &test_support::examples().join("bas-ready"),
            Some(&path),
            Decimal::parse("0.01").unwrap(),
            &[],
        )
        .unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
    note(&path, "XY", "2026-04-12", "Fabricated review.");
    for example in ["bas-ready", "bas-not-ready", "bas-blocked"] {
        let input = test_support::examples().join(example);
        let original =
            engine::review_pack("bas", &input, None, Decimal::parse("0.01").unwrap(), &[]).unwrap();
        let acknowledged = engine::review_pack(
            "bas",
            &input,
            Some(&path),
            Decimal::parse("0.01").unwrap(),
            &[],
        )
        .unwrap();
        assert_eq!(original.status, acknowledged.status);
        assert_eq!(original.findings, acknowledged.findings);
        assert!(acknowledged.acknowledgement.is_some());
    }
}
fn saved(tmp: &TempDir) -> std::path::PathBuf {
    let output = tmp.join("pack");
    let pack = engine::review_pack(
        "bas",
        &test_support::examples().join("bas-not-ready"),
        None,
        Decimal::parse("0.01").unwrap(),
        &[],
    )
    .unwrap();
    report::write_review_pack(&pack, &output).unwrap();
    viewer::verify_pack(&output).unwrap();
    output
}
#[test]
fn viewer_refuses_each_invalid_finding_member() {
    let tmp = TempDir::new();
    let output = saved(&tmp);
    let path = output.join("readiness-pack.json");
    let original = fs::read_to_string(&path).unwrap();
    for mutation in ["non-string", "added", "missing", "pack-status", "repeat"] {
        let mut document = pyjson::loads(&original).unwrap();
        let Json::Array(findings) = document.at("findings").clone() else {
            unreachable!()
        };
        let mut findings = findings;
        match mutation {
            "non-string" => set(&mut findings[0], "reason", Json::int(1)),
            "pack-status" => set(&mut findings[0], "status", Json::str("READY")),
            "repeat" => set(&mut findings[0], "repeat", Json::str("yes")),
            "added" => {
                let Json::Object(fields) = &mut findings[0] else {
                    unreachable!()
                };
                fields.push(("extra".into(), Json::str("value")));
            }
            "missing" => {
                let Json::Object(fields) = &mut findings[0] else {
                    unreachable!()
                };
                fields.retain(|(key, _)| !key.is("reason"));
            }
            _ => unreachable!(),
        }
        set(&mut document, "findings", Json::Array(findings));
        fs::write(&path, pyjson::canonical(&document)).unwrap();
        assert!(viewer::verify_pack(&output).is_err(), "{mutation}");
    }
}
#[test]
fn viewer_refuses_missing_unreadable_ambiguous_and_wrong_width_payloads() {
    let tmp = TempDir::new();
    let output = saved(&tmp);
    for name in report::PACK_NAMES {
        let path = output.join(name);
        let original = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(viewer::verify_pack(&output).is_err(), "missing {name}");
        fs::write(&path, b"\xff").unwrap();
        assert!(viewer::verify_pack(&output).is_err(), "UTF-8 {name}");
        fs::write(&path, original).unwrap();
    }
    let json = output.join("readiness-pack.json");
    let original = fs::read(&json).unwrap();
    for bytes in [
        b"{".as_slice(),
        br#"{"overall_status":"READY","overall_status":"BLOCKED"}"#,
    ] {
        fs::write(&json, bytes).unwrap();
        assert!(viewer::verify_pack(&output).is_err());
    }
    fs::write(&json, original).unwrap();
    let csv = output.join("findings.csv");
    let original = fs::read_to_string(&csv).unwrap();
    let rows = crate::pycsv::Reader::new(original.trim_start_matches('\u{feff}'))
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    for surplus in [false, true] {
        let mut rows = rows.clone();
        if surplus {
            rows[1].push("extra".into());
        } else {
            rows[1].pop();
        }
        let data = format!(
            "\u{feff}{}",
            rows.iter()
                .map(|row| crate::pycsv::write_record(
                    &row.iter().map(String::as_str).collect::<Vec<_>>()
                ))
                .collect::<String>()
        );
        fs::write(&csv, data).unwrap();
        assert!(viewer::verify_pack(&output).is_err());
    }
}
/// The `example` self-review with `period_start` added, loaded for `profile`.
fn declaring_start(example: &str, profile: &str, start: Json) -> crate::Result<SelfReview> {
    let tmp = TempDir::new();
    let path = tmp.join("self_review.json");
    let source = test_support::examples()
        .join(example)
        .join("self_review.json");
    let mut document = pyjson::loads(&fs::read_to_string(source).unwrap()).unwrap();
    let Json::Object(fields) = &mut document else {
        panic!("fixture object")
    };
    fields.push((PyStr::from("period_start"), start));
    fs::write(&path, pyjson::canonical(&document)).unwrap();
    let snapshot = loader::SourceSnapshot::capture(&path, "self_review file").unwrap();
    loader::load_self_review(&snapshot, profile)
}
#[test]
fn a_bas_self_review_may_declare_a_period_start_up_to_its_period_end() {
    // The bas-ready example's period ends on 2026-03-31.
    for start in ["2026-01-01", "2026-03-31"] {
        let review = declaring_start("bas-ready", "bas", Json::str(start)).unwrap();
        assert_eq!(
            review.period_start.map(|date| date.iso()).as_deref(),
            Some(start)
        );
    }
    let plain = loader::SourceSnapshot::capture(
        &test_support::examples().join("bas-ready/self_review.json"),
        "self_review file",
    )
    .unwrap();
    let review = loader::load_self_review(&plain, "bas").unwrap();
    assert_eq!(review.period_start, None);
}
#[test]
fn a_period_start_that_is_late_malformed_or_outside_bas_is_refused() {
    for (start, expected) in [
        (
            Json::str("2026-04-01"),
            ": period_start 2026-04-01 is later than period_end 2026-03-31.",
        ),
        (Json::str("20260101"), ":period_start must be an ISO date."),
        (
            Json::str("2026-02-30"),
            ":period_start must be an ISO date.",
        ),
        (Json::Null, ": period_start must be an ISO date string."),
        (
            Json::int(20260101),
            ": period_start must be an ISO date string.",
        ),
    ] {
        let error = declaring_start("bas-ready", "bas", start).unwrap_err();
        assert!(error.ends_with(expected), "{error}");
    }
    for (example, profile) in [
        ("month-end-ready", "month_end"),
        ("year-end-ready", "year_end"),
    ] {
        let error = declaring_start(example, profile, Json::str("2026-01-01")).unwrap_err();
        assert!(
            error.contains("self-review must contain exactly"),
            "{profile}: {error}"
        );
    }
}

#[test]
fn viewer_refuses_malformed_evidence_suffixes_without_losing_valid_witnesses() {
    let tmp = TempDir::new();
    let output = saved(&tmp);
    let path = output.join("readiness-summary.md");
    let original = fs::read_to_string(&path).unwrap();
    for suffix in [
        "- `unclosed",
        "- ``",
        "- `slot` - `file`",
        "- `slot` (`unclosed",
        "- `slot` (``",
        "- `slot` (`file`); `digest`",
        "- `slot` (`file`): `short`",
    ] {
        // Keep malformed tokens last: the evidence reader searches past newlines.
        fs::write(&path, format!("{original}\n{suffix}")).unwrap();
        assert_eq!(
            viewer::verify_pack(&output).err().unwrap(),
            "readiness-summary.md: document does not match its canonical rendering",
            "{suffix}"
        );
    }
}

#[test]
fn viewer_refuses_digest_witnesses_when_json_lists_no_sources() {
    let tmp = TempDir::new();
    let output = saved(&tmp);
    let path = output.join("readiness-pack.json");
    let mut document = pyjson::loads(&fs::read_to_string(&path).unwrap()).unwrap();
    set(&mut document, "source_sha256", Json::Object(vec![]));
    fs::write(path, pyjson::canonical(&document)).unwrap();
    assert_eq!(
        viewer::verify_pack(&output).err().unwrap(),
        "source evidence disagrees: readiness-summary.md lists digests but readiness-pack.json lists none"
    );
}

fn escaped_finding_pack(tmp: &TempDir) -> std::path::PathBuf {
    let input = tmp.join("input");
    test_support::copy_example("bas-ready", &input);
    fs::write(
        input.join("open_items.csv"),
        "ItemID,Severity,Owner,DueDate,Status,Description,Resolution\n\
         OI-escaped,BLOCKING,AB,2026-04-08,OPEN,Fabricated pipe | and slash \\ witness,\n",
    )
    .unwrap();
    let pack =
        engine::review_pack("bas", &input, None, Decimal::parse("0.01").unwrap(), &[]).unwrap();
    let output = tmp.join("escaped-pack");
    report::write_review_pack(&pack, &output).unwrap();
    output
}

#[test]
fn viewer_preserves_literal_pipe_and_backslash_in_finding_text() {
    let tmp = TempDir::new();
    let output = escaped_finding_pack(&tmp);
    let verified = viewer::verify_pack(&output).unwrap();
    let Json::Array(findings) = verified.document.at("findings") else {
        panic!("findings array")
    };
    assert_eq!(findings.len(), 1);
    assert!(findings[0].at("reason").is_str(
        "Blocking open item OI-escaped still OPEN with no resolution: Fabricated pipe | and slash \\ witness"
    ));
    assert!(verified.summary.contains(
        "| NOT_READY | OPEN_ITEM_BLOCKING | open_items | no | Blocking open item OI-escaped still OPEN with no resolution: Fabricated pipe \\| and slash \\\\ witness |"
    ));
}

#[test]
fn viewer_refuses_invalid_repeat_even_when_json_and_csv_agree() {
    let tmp = TempDir::new();
    let output = escaped_finding_pack(&tmp);
    viewer::verify_pack(&output).unwrap();
    let path = output.join("readiness-pack.json");
    let mut document = pyjson::loads(&fs::read_to_string(&path).unwrap()).unwrap();
    let Json::Array(mut findings) = document.at("findings").clone() else {
        panic!("findings array")
    };
    set(&mut findings[0], "repeat", Json::str("yes"));
    set(&mut document, "findings", Json::Array(findings));
    fs::write(path, pyjson::canonical(&document)).unwrap();
    let path = output.join("findings.csv");
    let original = fs::read_to_string(&path).unwrap();
    let mut rows = crate::pycsv::Reader::new(original.trim_start_matches('\u{feff}'))
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let repeat = rows[0].iter().position(|cell| cell == "repeat").unwrap();
    rows[1][repeat] = "yes".into();
    let csv = rows
        .iter()
        .map(|row| crate::pycsv::write_record(&row.iter().map(String::as_str).collect::<Vec<_>>()))
        .collect::<String>();
    fs::write(path, format!("\u{feff}{csv}")).unwrap();
    assert_eq!(
        viewer::verify_pack(&output).err().unwrap(),
        "readiness-pack.json: findings[0].repeat must be 'true' or 'false'"
    );
}

#[test]
fn public_json_consumers_report_depth_and_integer_limits() {
    // Parsing and dropping nested JSON use the same explicit stack as parser tests.
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let tmp = TempDir::new();
            let output = saved(&tmp);
            let path = output.join("readiness-pack.json");
            let deep = format!("{}null{}", "[".repeat(1001), "]".repeat(1001));
            fs::write(&path, deep).unwrap();
            assert_eq!(
                viewer::verify_pack(&output).err().unwrap(),
                "readiness-pack.json: nested too deeply to read"
            );
            let integer = "1".repeat(4301);
            let limit = "Exceeds the limit (4300 digits) for integer string conversion: value has 4301 digits";
            fs::write(path, &integer).unwrap();
            assert_eq!(
                viewer::verify_pack(&output).err().unwrap(),
                format!("readiness-pack.json: not readable ({limit})")
            );
            let snapshot = loader::SourceSnapshot::new(
                Path::new("fabricated-self-review.json"),
                integer.into_bytes(),
            );
            assert_eq!(
                loader::load_self_review(&snapshot, "bas").unwrap_err(),
                format!("fabricated-self-review.json: self-review could not be read ({limit}).")
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn canonical_tb_requires_one_tenant_and_report_date() {
    let path = Path::new("fabricated-trial-balance.csv");
    let load = |tenant, date| {
        let csv = format!(
            "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n\
             2026-03-31,Demo,Assets,one,Fabricated cash,100,1.00,0,1.00,0\n\
             {date},{tenant},Equity,two,Fabricated equity,200,0,1.00,0,1.00\n"
        );
        loader::load_canonical_tb(&loader::SourceSnapshot::new(path, csv.into_bytes()))
    };
    assert_eq!(load("Demo", "2026-03-31").unwrap().len(), 2);
    assert_eq!(
        load("Other demo", "2026-03-31").unwrap_err(),
        "fabricated-trial-balance.csv: a canonical trial balance must contain exactly one tenant."
    );
    assert_eq!(
        load("Demo", "2026-04-30").unwrap_err(),
        "fabricated-trial-balance.csv: a canonical trial balance must contain exactly one ReportDate."
    );
}

#[test]
fn money_refuses_multiple_decimal_points_with_field_and_row_context() {
    let path = Path::new("fabricated-trial-balance.csv");
    assert_eq!(
        loader::parse_money(Some("1.23"), "Debit", 3, path).unwrap(),
        Decimal::parse("1.23").unwrap()
    );
    assert_eq!(
        loader::parse_money(Some("1.2.3"), "Debit", 3, path).unwrap_err(),
        "fabricated-trial-balance.csv: row 3 has invalid Debit: '1.2.3'."
    );
}

fn load_self_review_document(value: &Json) -> crate::Result<SelfReview> {
    loader::load_self_review(
        &loader::SourceSnapshot::new(
            Path::new("fabricated-self-review.json"),
            pyjson::canonical(value).into_bytes(),
        ),
        "bas",
    )
}

#[test]
fn self_review_preparation_cannot_precede_period_end() {
    let mut document = pyjson::loads(
        &fs::read_to_string(test_support::examples().join("bas-ready/self_review.json")).unwrap(),
    )
    .unwrap();
    set(&mut document, "prepared_on", Json::str("2026-03-31"));
    assert!(load_self_review_document(&document).is_ok());
    set(&mut document, "prepared_on", Json::str("2026-03-30"));
    assert_eq!(
        load_self_review_document(&document).unwrap_err(),
        "fabricated-self-review.json: prepared_on 2026-03-30 is earlier than period_end 2026-03-31."
    );
}

#[test]
fn each_self_review_assertion_requires_a_real_json_boolean() {
    let baseline = pyjson::loads(
        &fs::read_to_string(test_support::examples().join("bas-ready/self_review.json")).unwrap(),
    )
    .unwrap();
    assert!(load_self_review_document(&baseline).is_ok());
    for name in loader::ASSERTIONS {
        for invalid in [Json::str("true"), Json::Null] {
            let mut document = baseline.clone();
            let mut assertions = document.at("assertions").clone();
            set(&mut assertions, name, invalid);
            set(&mut document, "assertions", assertions);
            assert_eq!(
                load_self_review_document(&document).unwrap_err(),
                format!("fabricated-self-review.json: assertions.{name} must be a JSON boolean.")
            );
        }
        let mut document = baseline.clone();
        let mut assertions = document.at("assertions").clone();
        set(&mut assertions, name, Json::Bool(false));
        set(&mut document, "assertions", assertions);
        let loaded = load_self_review_document(&document).unwrap();
        assert!(
            !loaded
                .assertions
                .iter()
                .find(|(key, _)| key == name)
                .unwrap()
                .1
        );
    }
}

#[test]
fn reviewer_acknowledgement_reports_missing_files_and_directories() {
    let tmp = TempDir::new();
    let absent = tmp.join("absent.json");
    assert_eq!(
        loader::load_reviewer_acknowledgement(Some(&absent)).unwrap_err(),
        format!(
            "Review-note file does not exist: {}.",
            crate::pypath::text(&absent)
        )
    );
    let directory = tmp.join("directory.json");
    fs::create_dir(&directory).unwrap();
    let error = loader::load_reviewer_acknowledgement(Some(&directory)).unwrap_err();
    assert!(
        error.starts_with(&format!(
            "Review-note file could not be read: {} (",
            crate::pypath::text(&directory)
        )),
        "{error}"
    );
    assert!(error.ends_with(")."), "{error}");
}
