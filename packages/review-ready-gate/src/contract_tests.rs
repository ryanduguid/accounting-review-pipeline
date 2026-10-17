use crate::{
    decimal::Decimal,
    engine, loader,
    pyjson::{self, Json},
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
