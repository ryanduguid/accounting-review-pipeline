use monthly_close_control_plane::{
    calculation_evidence::{self, CalculationEvidence},
    engine::{self, CloseReviewPack, ReviewConfig},
    isodate::Date,
    loader::SourceSnapshot,
    models::{ReviewerAcknowledgement, Status},
    publication,
    pyjson::{self, Json, PyStr},
    report, sha256, viewer,
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[path = "support/temp.rs"]
mod temp;
use temp::TempDir;

#[path = "support/pack_integrity.rs"]
mod pack_integrity;

const GOOD: &str = include_str!("../examples/calculation-evidence-coal-lsl-levy.json");
const REFUSED: &str = include_str!("../examples/calculation-evidence-refused.json");
const LABEL: &str = "coal-lsl-levy";
const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn record() -> Json {
    pyjson::loads(GOOD).unwrap()
}
fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")
}
fn part_mut<'a>(mut value: &'a mut Json, path: &[&str]) -> &'a mut Json {
    for name in path {
        let Json::Object(pairs) = value else {
            panic!("object at {name}")
        };
        value = &mut pairs.iter_mut().find(|(key, _)| key.is(name)).unwrap().1;
    }
    value
}
fn put(value: &mut Json, path: &[&str], replacement: Json) {
    part_mut(value, &path[..path.len() - 1]).set(path.last().unwrap(), replacement);
}
fn remove(value: &mut Json, path: &[&str]) {
    let Json::Object(pairs) = part_mut(value, &path[..path.len() - 1]) else {
        panic!("object")
    };
    pairs.retain(|(key, _)| !key.is(path.last().unwrap()));
}
fn bytes(mut value: Json, rehash: bool) -> Vec<u8> {
    if rehash {
        let canonical = pyjson::canonical_utf8(value.at("calculation")).unwrap();
        value.set(
            "calculation_sha256",
            Json::str(&sha256(canonical.as_bytes())),
        );
    }
    pyjson::pretty(&value).into_bytes()
}
fn loaded(content: &[u8]) -> Result<CalculationEvidence, String> {
    calculation_evidence::load(&SourceSnapshot::new(
        Path::new("evidence.json"),
        content.to_vec(),
    ))
}
fn config(path: &Path) -> ReviewConfig {
    let mut config = ReviewConfig::new(
        examples().join("current_trial_balance.csv"),
        examples().join("prior_trial_balance.csv"),
    );
    config.calculation_evidence_paths.push(path.into());
    config.required_calculations.push(LABEL.into());
    config
}
fn reviewed(content: &[u8]) -> (TempDir, CloseReviewPack) {
    let temp = TempDir::new();
    let path = temp.join("evidence.json");
    fs::write(&path, content).unwrap();
    let pack = engine::review_close(&config(&path)).unwrap();
    (temp, pack)
}
fn findings(pack: &CloseReviewPack) -> Vec<&str> {
    pack.exceptions
        .iter()
        .filter(|e| e.control == "calculation_evidence")
        .map(|e| e.reason.as_str())
        .collect()
}
fn entry(document: &Json) -> &Json {
    &document
        .at("calculation_evidence")
        .at("supplied")
        .as_array()
        .unwrap()[0]
}
fn payloads(pack: &CloseReviewPack) -> BTreeMap<String, Vec<u8>> {
    report::PACK_FILE_NAMES
        .into_iter()
        .map(str::to_owned)
        .zip(report::rendered(pack).unwrap())
        .collect()
}
fn review_command(output: &Path) -> Command {
    let mut command = Command::new(BIN);
    command
        .arg("review")
        .arg("--current")
        .arg(examples().join("current_trial_balance.csv"))
        .arg("--prior")
        .arg(examples().join("prior_trial_balance.csv"))
        .arg("--output")
        .arg(output);
    command
}

#[test]
fn committed_evidence_is_fabricated_and_the_july_figure_is_usable() {
    let good = loaded(GOOD.as_bytes()).unwrap();
    let refused = loaded(REFUSED.as_bytes()).unwrap();
    assert!(good.synthetic_input && refused.synthetic_input);
    assert!(good.usable());
    assert_eq!(good.status, "COMPUTED");
    assert_eq!(good.values["levy"].to_string(), "192.38");
    assert_eq!(good.sha256, sha256(GOOD.as_bytes()));
    assert_eq!(refused.status, "UPSTREAM_REFUSED");
    assert!(refused.values.is_empty() && !refused.usable());
    let (_temp, pack) = reviewed(GOOD.as_bytes());
    assert_eq!(pack.current_report_dates, ["2026-07-31"]);
    assert!(findings(&pack).is_empty());
    assert!(pack.relied_on.contains(LABEL));
    assert_eq!(pack.status, Status::Review);
    let document = report::as_json(&pack).unwrap();
    assert_eq!(entry(&document).at("usable").as_bool(), Some(true));
    assert!(entry(&document).at("values").at("levy").is_str("192.38"));
    assert_eq!(
        pack.source_hashes["calculation_evidence:coal-lsl-levy"],
        sha256(GOOD.as_bytes())
    );
    let rendered = payloads(&pack);
    let summary = std::str::from_utf8(&rendered["close-summary.md"]).unwrap();
    let row = format!(
        "| coal-lsl-levy | COMPUTED | {} | eligible_wages 7125.00; levy 192.38 | yes | {} |",
        good.period,
        report::entry_digest(entry(&document)).unwrap()
    );
    assert!(summary.lines().any(|line| line == row));
    assert!(summary.contains("approves nothing"));
    assert!(viewer::verify_payloads(&rendered).is_ok());
}

#[test]
fn valid_evidence_survives_physical_publication_and_read_only_display() {
    let (temp, pack) = reviewed(GOOD.as_bytes());
    let output = temp.join("pack");
    let paths = publication::write_review_pack(&pack, &output).unwrap();
    let expected = report::rendered(&pack).unwrap();
    for (path, bytes) in paths.iter().zip(&expected) {
        assert_eq!(fs::read(path).unwrap(), *bytes);
    }
    assert!(viewer::verify_pack(&output).is_ok());
    let shown = Command::new(BIN)
        .args(["view", "--pack-dir"])
        .arg(&output)
        .output()
        .unwrap();
    assert_eq!(shown.status.code(), Some(0), "{shown:?}");
    assert!(shown.stderr.is_empty());
    let sheet = String::from_utf8(shown.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(sheet.contains("\nCalculation evidence\n"));
    assert!(sheet.contains("- Required: coal-lsl-levy"));
    assert!(sheet.contains(&format!(
        "- coal-lsl-levy: COMPUTED, period {}, eligible_wages 7125.00; levy 192.38, relied on: yes",
        loaded(GOOD.as_bytes()).unwrap().period
    )));
    assert!(sheet.contains("A figure here supports review. It approves nothing"));
    for (path, bytes) in paths.iter().zip(&expected) {
        assert_eq!(fs::read(path).unwrap(), *bytes);
    }
}

#[test]
fn source_file_digest_changes_with_observation_but_calculation_digest_does_not() {
    let before = loaded(GOOD.as_bytes()).unwrap();
    let mut value = record();
    put(
        &mut value,
        &["observation", "captured_at"],
        Json::str("2026-08-02T04:12:09+00:00"),
    );
    let after = loaded(&bytes(value, false)).unwrap();
    assert!(after.usable());
    assert_eq!(before.calculation_sha256, after.calculation_sha256);
    assert_ne!(before.sha256, after.sha256);
}

#[test]
fn omitted_flags_and_findings_preserve_loading_and_literal_booleans() {
    for path in [
        vec!["calculation", "synthetic_input"],
        vec!["calculation", "validation", "accepted"],
        vec!["calculation", "validation"],
        vec!["calculation", "validation", "findings"],
    ] {
        let mut value = record();
        remove(&mut value, &path);
        let content = bytes(value, true);
        let evidence = loaded(&content).unwrap();
        assert!(evidence.usable() && evidence.findings.is_empty());
        assert_eq!(
            evidence.synthetic_input,
            path.last() != Some(&"synthetic_input")
        );
        let (_temp, pack) = reviewed(&content);
        assert!(findings(&pack).is_empty());
    }
    let mut value = record();
    for path in [
        vec!["calculation", "synthetic_input"],
        vec!["calculation", "validation", "accepted"],
        vec!["calculation", "validation", "findings"],
    ] {
        remove(&mut value, &path);
    }
    let evidence = loaded(&bytes(value, true)).unwrap();
    assert!(!evidence.synthetic_input);
    assert!(evidence.findings.is_empty() && evidence.usable());
    for flag in [false, true] {
        let mut value = record();
        put(
            &mut value,
            &["calculation", "synthetic_input"],
            Json::Bool(flag),
        );
        let evidence = loaded(&bytes(value, true)).unwrap();
        assert_eq!(evidence.synthetic_input, flag);
        assert!(evidence.usable());
    }
}

#[test]
fn present_non_boolean_flags_are_unreadable_and_block_the_full_review() {
    for path in [
        vec!["calculation", "synthetic_input"],
        vec!["calculation", "validation", "accepted"],
    ] {
        for literal in [
            r#""false""#,
            r#""true""#,
            "0",
            "1",
            "0.0",
            "1.0",
            "null",
            "[]",
            "{}",
        ] {
            let mut value = record();
            put(&mut value, &path, pyjson::loads(literal).unwrap());
            let content = bytes(value, true);
            assert!(
                loaded(&content).unwrap_err().contains("must be a boolean"),
                "{path:?} {literal}"
            );
            let (_temp, pack) = reviewed(&content);
            assert_eq!(pack.status, Status::Blocked, "{path:?} {literal}");
            assert!(pack.relied_on.is_empty());
            assert!(
                findings(&pack)
                    .iter()
                    .any(|s| s.contains("could not be read"))
            );
            let field = path[1..].join(".");
            assert!(
                findings(&pack).iter().any(|s| s.contains(&field)),
                "{path:?}"
            );
        }
    }
}

#[test]
fn schema_text_money_and_array_shapes_are_refused() {
    let mut wrong_block = record();
    wrong_block.set("calculation", Json::str("wrong type"));
    assert!(
        loaded(&bytes(wrong_block, false))
            .unwrap_err()
            .contains("no calculation block")
    );
    let cases = [
        (
            vec!["schema"],
            Json::str("future-evidence/9"),
            "unknown schema",
        ),
        (
            vec!["calculation", "call", "status"],
            Json::str(&"x".repeat(61)),
            "longer than 60",
        ),
        (
            vec!["calculation", "normalised", "values", "levy"],
            Json::int(192),
            "decimal string",
        ),
        (
            vec!["calculation", "normalised", "values", "levy"],
            Json::str("1,000.00"),
            "decimal",
        ),
        (
            vec!["calculation", "normalised", "values", "levy"],
            Json::str("1.2.3"),
            "decimal",
        ),
        (
            vec!["calculation", "upstream", "manifest", "rate_table_uris"],
            Json::str("rate"),
            "not an array",
        ),
        (
            vec!["calculation", "upstream", "advisory", "notes"],
            Json::str("advice"),
            "not an array",
        ),
        (
            vec!["calculation", "validation", "findings"],
            Json::str("issue"),
            "not an array",
        ),
    ];
    for (path, replacement, message) in cases {
        let mut value = record();
        put(&mut value, &path, replacement);
        let content = bytes(value, true);
        assert!(loaded(&content).unwrap_err().contains(message), "{path:?}");
        let (_temp, pack) = reviewed(&content);
        assert_eq!(pack.status, Status::Blocked);
        assert!(pack.relied_on.is_empty());
    }
    let mut value = record();
    remove(&mut value, &["calculation"]);
    assert!(
        loaded(&bytes(value, false))
            .unwrap_err()
            .contains("no calculation block")
    );
    assert!(
        loaded(&bytes(Json::Array(vec![record()]), false))
            .unwrap_err()
            .contains("JSON object")
    );
}

#[test]
fn labels_must_be_lower_case_slugs() {
    for label in [
        "",
        "UPPER",
        "leading-",
        "-trailing",
        "two--hyphens",
        "has space",
        "../label",
        "non_ascii_é",
        "coal_lsl",
    ] {
        let mut value = record();
        put(&mut value, &["calculation", "label"], Json::str(label));
        assert!(
            loaded(&bytes(value, true))
                .unwrap_err()
                .contains("not a slug"),
            "{label:?}"
        );
    }
}

#[test]
fn directional_format_and_control_characters_are_not_hidden_in_evidence_text() {
    for cp in [0x2066, 0x2069, 0x061c, 0x202e, 0x200b] {
        let mut value = record();
        let note = format!("visible{}text", char::from_u32(cp).unwrap());
        put(
            &mut value,
            &["calculation", "upstream", "advisory", "notes"],
            Json::Array(vec![Json::str(&note)]),
        );
        assert!(
            loaded(&bytes(value, true))
                .unwrap_err()
                .contains("control or formatting")
        );
    }
    for cp in [
        0x00ad, 0x2060, 0xfeff, 0xfff9, 0x0090, 0x0085, 0x2028, 0x2029,
    ] {
        for path in [
            vec!["calculation", "call", "status"],
            vec!["calculation", "provider", "name"],
        ] {
            let mut value = record();
            put(
                &mut value,
                &path,
                Json::str(&format!("A{}B", char::from_u32(cp).unwrap())),
            );
            assert!(
                loaded(&bytes(value, true))
                    .unwrap_err()
                    .contains("control or formatting"),
                "{cp:x} {path:?}"
            );
        }
    }
}

#[test]
fn nested_advisory_manifest_and_validation_findings_are_preserved() {
    let cases = [
        (
            vec!["calculation", "upstream", "advisory", "notes"],
            pyjson::loads("[null]").unwrap(),
            "not text",
        ),
        (
            vec!["calculation", "upstream", "advisory", "notes"],
            pyjson::loads(r#"["", "   "]"#).unwrap(),
            "no advisory",
        ),
        (
            vec!["calculation", "upstream", "advisory"],
            pyjson::loads("{}").unwrap(),
            "no advisory",
        ),
        (
            vec!["calculation", "upstream", "manifest"],
            pyjson::loads("{}").unwrap(),
            "no manifest",
        ),
        (
            vec!["calculation", "upstream", "manifest", "rate_table_uris"],
            pyjson::loads("[true]").unwrap(),
            "not an object",
        ),
        (
            vec!["calculation", "upstream", "manifest", "rate_table_uris"],
            pyjson::loads("[{}]").unwrap(),
            "no uri string",
        ),
        (
            vec!["calculation", "validation", "findings"],
            pyjson::loads("[false]").unwrap(),
            "not text",
        ),
        (
            vec!["calculation", "validation", "findings"],
            pyjson::loads(r#"["producer concern"]"#).unwrap(),
            "producer concern",
        ),
    ];
    for (path, replacement, message) in cases {
        let mut value = record();
        put(&mut value, &path, replacement);
        let content = bytes(value, true);
        assert!(
            loaded(&content)
                .unwrap()
                .findings
                .iter()
                .any(|s| s.contains(message)),
            "{path:?}"
        );
        let (_temp, pack) = reviewed(&content);
        assert_eq!(pack.status, Status::Blocked);
        assert!(pack.relied_on.is_empty());
    }
    let mut value = record();
    put(
        &mut value,
        &["calculation", "upstream", "advisory", "notes"],
        pyjson::loads(r#"["", "Boundary statement"]"#).unwrap(),
    );
    assert!(loaded(&bytes(value, true)).unwrap().usable());
}

#[test]
fn a_refusal_needs_no_advisory_but_cannot_clear_other_producer_findings() {
    let mut value = record();
    put(
        &mut value,
        &["calculation", "call", "status"],
        Json::str("UPSTREAM_REFUSED"),
    );
    put(
        &mut value,
        &["calculation", "normalised", "values"],
        pyjson::loads("{}").unwrap(),
    );
    put(
        &mut value,
        &["calculation", "validation", "accepted"],
        Json::Bool(false),
    );
    put(
        &mut value,
        &["calculation", "upstream", "advisory"],
        pyjson::loads("{}").unwrap(),
    );
    let content = bytes(value.clone(), true);
    let evidence = loaded(&content).unwrap();
    assert!(evidence.findings.is_empty() && !evidence.usable());
    let (_temp, pack) = reviewed(&content);
    assert_eq!(pack.status, Status::Review);
    assert!(pack.relied_on.is_empty());
    put(
        &mut value,
        &["calculation", "validation", "findings"],
        pyjson::loads(r#"["producer concern"]"#).unwrap(),
    );
    let (_temp, pack) = reviewed(&bytes(value, true));
    assert_eq!(pack.status, Status::Blocked);
    assert!(
        findings(&pack)
            .iter()
            .any(|s| s.contains("producer concern"))
    );
}

#[test]
fn advisory_blanks_and_substantive_notes_preserve_the_original_findings() {
    for notes in [
        vec![],
        vec![""],
        vec![" "],
        vec!["", "   "],
        vec!["\u{2003}"],
    ] {
        let mut value = record();
        put(
            &mut value,
            &["calculation", "upstream", "advisory", "notes"],
            Json::Array(notes.iter().map(|s| Json::str(s)).collect()),
        );
        let content = bytes(value, true);
        let evidence = loaded(&content).unwrap();
        assert!(!evidence.usable());
        assert!(evidence.findings.iter().any(|s| s.contains("no advisory")));
        let (_temp, pack) = reviewed(&content);
        assert_eq!(pack.status, Status::Blocked);
    }
    let notes = [
        " ",
        "Review the supplied facts before relying on this estimate.",
    ];
    let mut value = record();
    put(
        &mut value,
        &["calculation", "upstream", "advisory", "notes"],
        Json::Array(notes.iter().map(|s| Json::str(s)).collect()),
    );
    let evidence = loaded(&bytes(value, true)).unwrap();
    assert!(evidence.usable());
    assert_eq!(evidence.advisory_notes, notes);
    let mut value = record();
    put(
        &mut value,
        &["calculation", "validation", "accepted"],
        Json::Bool(false),
    );
    put(
        &mut value,
        &["calculation", "validation", "findings"],
        Json::Array(vec![Json::str("fabricated existing concern")]),
    );
    let evidence = loaded(&bytes(value, true)).unwrap();
    assert!(!evidence.usable());
    assert!(
        evidence
            .findings
            .iter()
            .any(|s| s.contains("accepted=false"))
    );
    assert!(
        evidence
            .findings
            .iter()
            .any(|s| s.contains("fabricated existing concern"))
    );
    for (path, replacement) in [
        (
            vec!["calculation", "upstream", "advisory", "notes"],
            "[null]",
        ),
        (
            vec!["calculation", "upstream", "manifest", "rate_table_uris"],
            "[true]",
        ),
    ] {
        let mut value = record();
        put(&mut value, &path, pyjson::loads(replacement).unwrap());
        let evidence = loaded(&bytes(value, true)).unwrap();
        if path.last() == Some(&"notes") {
            assert!(evidence.advisory_notes.is_empty());
        } else {
            assert!(evidence.rate_tables.is_empty());
        }
        assert!(!evidence.usable());
    }
}

#[test]
fn a_changed_digest_blocks_the_pack_and_acknowledgement_does_not_clear_it() {
    let mut value = record();
    put(
        &mut value,
        &["calculation", "normalised", "values", "levy"],
        Json::str("999999.99"),
    );
    let content = bytes(value.clone(), false);
    assert!(
        loaded(&content)
            .unwrap()
            .findings
            .iter()
            .any(|s| s.contains("does not match"))
    );
    let (temp, pack) = reviewed(&content);
    assert_eq!(pack.status, Status::Blocked);
    assert!(pack.relied_on.is_empty());
    let before = findings(&pack)
        .iter()
        .map(|s| (*s).to_owned())
        .collect::<Vec<_>>();
    let note = temp.join("acknowledgement.json");
    fs::write(&note, br#"{"reviewer_initials":"RD","reviewed_on":"2026-07-31","comment":"Recorded review only"}"#).unwrap();
    let mut with_note = config(&temp.join("evidence.json"));
    with_note.acknowledgement_path = Some(note);
    let pack = engine::review_close(&with_note).unwrap();
    assert_eq!(
        pack.acknowledgement.as_ref().unwrap().reviewer_initials,
        "RD"
    );
    assert_eq!(pack.status, Status::Blocked);
    assert_eq!(before, findings(&pack));
    let rendered = payloads(&pack);
    assert!(viewer::verify_payloads(&rendered).is_ok());
    with_note.calculation_evidence_paths.clear();
    let missing = engine::review_close(&with_note).unwrap();
    assert!(missing.acknowledgement.is_some());
    assert_eq!(missing.status, Status::Review);
    assert!(missing.relied_on.is_empty());
    assert!(
        findings(&missing)
            .iter()
            .any(|reason| reason.contains("required"))
    );
    assert!(loaded(&bytes(value, true)).unwrap().usable());
}

#[test]
fn an_unreconciled_depreciation_contract_failure_remains_blocked() {
    let mut value = record();
    for (path, replacement) in [
        (
            vec!["calculation", "label"],
            Json::str("accounting-depreciation"),
        ),
        (
            vec!["calculation", "call", "calculator"],
            Json::str("urn:sbrm:calculator:depreciation:range"),
        ),
        (
            vec!["calculation", "call", "period"],
            Json::str("urn:sbrm:period:depreciation:unscoped"),
        ),
        (
            vec!["calculation", "call", "status"],
            Json::str("CONTRACT_FAILURE"),
        ),
        (
            vec!["calculation", "validation", "accepted"],
            Json::Bool(false),
        ),
        (
            vec!["calculation", "validation", "findings"],
            Json::Array(vec![Json::str(
                "movement does not close: opening 0.00 plus additions 0.00 less depreciation 12000.00 is -12000.00, against a closing balance of 108000.00",
            )]),
        ),
        (
            vec!["calculation", "normalised", "values"],
            Json::object([] as [(&str, Json); 0]),
        ),
    ] {
        put(&mut value, &path, replacement);
    }
    let temp = TempDir::new();
    let path = temp.join("depreciation.json");
    fs::write(&path, bytes(value, true)).unwrap();
    let mut config = config(&path);
    config.required_calculations = vec!["accounting-depreciation".into()];
    let pack = engine::review_close(&config).unwrap();
    assert_eq!(pack.status, Status::Blocked);
    assert!(pack.relied_on.is_empty());
    assert!(
        findings(&pack)
            .iter()
            .any(|reason| reason.contains("movement does not close"))
    );
}

#[test]
fn missing_rates_or_figures_are_never_saved_as_relied_upon() {
    for (path, replacement, expected_status, message) in [
        (
            vec!["calculation", "upstream", "manifest", "rate_table_uris"],
            "[]",
            Status::Review,
            "names no rate table",
        ),
        (
            vec!["calculation", "normalised", "values"],
            "{}",
            Status::Blocked,
            "no normalised value",
        ),
        (
            vec!["calculation", "normalised"],
            "[]",
            Status::Blocked,
            "no normalised value",
        ),
    ] {
        let mut value = record();
        put(&mut value, &path, pyjson::loads(replacement).unwrap());
        let (_temp, pack) = reviewed(&bytes(value, true));
        assert_eq!(pack.status, expected_status);
        assert!(pack.relied_on.is_empty());
        assert!(findings(&pack).iter().any(|s| s.contains(message)));
        let document = report::as_json(&pack).unwrap();
        assert_eq!(entry(&document).at("usable").as_bool(), Some(false));
    }
}

#[test]
fn period_coverage_distinguishes_invalid_nonmatching_and_inclusive_boundaries() {
    let evidence = loaded(GOOD.as_bytes()).unwrap();
    for (period, date, expected) in [
        ("2026-07", (2026, 7, 31), Some(true)),
        ("2026-06", (2026, 7, 31), Some(false)),
        ("2026-07", (2026, 8, 1), Some(false)),
        ("0001-01", (1, 1, 1), Some(true)),
        ("9999-12", (9999, 12, 31), Some(true)),
        ("fy0001", (1, 6, 30), None),
        ("fy0002", (1, 7, 1), Some(true)),
        ("fy9999", (9999, 6, 30), Some(true)),
        ("fy2027", (2026, 7, 1), Some(true)),
        ("fy2027", (2027, 6, 30), Some(true)),
        ("fy2027", (2026, 6, 30), Some(false)),
        ("fy2027", (2027, 7, 1), Some(false)),
        ("urn:fbt:fy2027", (2026, 4, 1), Some(true)),
        ("urn:fbt:fy2027", (2027, 3, 31), Some(true)),
        ("urn:fbt:fy2027", (2026, 3, 31), Some(false)),
        ("urn:fbt:fy2027", (2027, 4, 1), Some(false)),
        ("2026-13", (2026, 7, 31), None),
        ("2026-00", (2026, 7, 31), None),
        ("2026-99", (2026, 7, 31), None),
        ("２０２６-０７", (2026, 7, 31), None),
        ("2026-²⁷", (2026, 7, 31), None),
        ("fy²⁰²⁶", (2026, 7, 31), None),
        ("fy２０２６", (2026, 7, 31), None),
        ("fy0000", (2026, 7, 31), None),
        ("fy10000", (2026, 7, 31), None),
        ("0000-01", (2026, 7, 31), None),
        (
            "urn:sbrm:period:coal-lsl-levy:2026-06",
            (2026, 6, 30),
            Some(true),
        ),
        (
            "urn:sbrm:period:coal-lsl-levy:2026-05",
            (2026, 6, 30),
            Some(false),
        ),
        ("urn:sbrm:period:div7a:fy2026", (2026, 6, 30), Some(true)),
        ("urn:sbrm:period:div7a:fy2026", (2026, 7, 31), Some(false)),
        ("urn:sbrm:period:fbt:fy2026", (2026, 3, 31), Some(true)),
        ("urn:sbrm:period:fbt:fy2026", (2026, 6, 30), Some(false)),
        ("urn:sbrm:period:depreciation:unscoped", (2026, 6, 30), None),
        ("", (2026, 6, 30), None),
    ] {
        let mut current = evidence.clone();
        current.period = period.into();
        assert_eq!(
            calculation_evidence::covers_period(
                &current,
                Date::new(date.0, date.1, date.2).unwrap()
            ),
            expected,
            "{period}"
        );
    }
    for (period, message) in [
        ("2026-13", "cannot read as a period"),
        ("urn:sbrm:period:div7a:fy0000", "cannot read as a period"),
        ("2026-06", "does not include"),
    ] {
        let mut value = record();
        put(
            &mut value,
            &["calculation", "call", "period"],
            Json::str(period),
        );
        let (_temp, pack) = reviewed(&bytes(value, true));
        assert_eq!(pack.status, Status::Review);
        assert!(pack.relied_on.is_empty());
        assert!(findings(&pack).iter().any(|s| s.contains(message)));
    }
}

#[test]
fn duplicate_keys_are_unreadable_in_both_orders_and_at_nested_depths() {
    for pair in [
        r#""status":"COMPUTED","status":"UPSTREAM_REFUSED""#,
        r#""status":"UPSTREAM_REFUSED","status":"COMPUTED""#,
        r#""status":"COMPUTED","status":"COMPUTED""#,
        r#""status":"COMPUTED","\u0073tatus":"COMPUTED""#,
    ] {
        let content = GOOD
            .replacen(r#""status": "COMPUTED""#, pair, 1)
            .into_bytes();
        let (temp, pack) = reviewed(&content);
        let (items, unreadable) = calculation_evidence::load_all(&[temp.join("evidence.json")]);
        assert!(items.is_empty());
        assert!(unreadable[0].contains("more than once"));
        assert_eq!(pack.status, Status::Blocked);
        assert!(pack.relied_on.is_empty());
    }
    let raw =
        br#"{"schema":"lodgeit-calculation-evidence/1","calculation":{"rows":[{"a":1,"a":2}]}}"#;
    assert!(loaded(raw).unwrap_err().contains("more than once"));
    let mut bom = b"\xef\xbb\xbf".to_vec();
    bom.extend_from_slice(GOOD.as_bytes());
    assert!(loaded(&bom).unwrap().usable());
    let mut value = record();
    put(
        &mut value,
        &["observation"],
        pyjson::loads(r#"{"status":1,"nested":{"status":2}}"#).unwrap(),
    );
    assert!(loaded(&bytes(value, false)).unwrap().usable());
}

#[test]
fn load_all_translates_missing_invalid_deep_and_huge_inputs_without_silent_drops() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let temp = TempDir::new();
            let cases = [
                (b"{".to_vec(), "not valid JSON"),
                (
                    format!("{}null{}", "[".repeat(1002), "]".repeat(1002)).into_bytes(),
                    "nested too deeply",
                ),
                (
                    format!("{{\"number\":{}}}", "9".repeat(4301)).into_bytes(),
                    "could not be read (Exceeds the limit (4300 digits)",
                ),
            ];
            for (index, (content, diagnostic)) in cases.into_iter().enumerate() {
                let path = temp.join(format!("bad-{index}.json"));
                fs::write(&path, content).unwrap();
                let (items, errors) = calculation_evidence::load_all(&[path]);
                assert!(items.is_empty());
                assert_eq!(errors.len(), 1);
                assert!(errors[0].contains(diagnostic), "{errors:?}");
            }
            let (items, errors) = calculation_evidence::load_all(&[temp.join("missing.json")]);
            assert!(items.is_empty());
            assert_eq!(errors.len(), 1);
            assert!(errors[0].contains("does not exist"));
            let mut value = record();
            put(
                &mut value,
                &["calculation", "ignored"],
                Json::Str(PyStr(vec![0xd800])),
            );
            let path = temp.join("surrogate.json");
            fs::write(&path, bytes(value, false)).unwrap();
            let (items, errors) = calculation_evidence::load_all(&[path]);
            assert!(items.is_empty());
            assert!(errors[0].contains("cannot be canonicalised"));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn actual_review_records_the_figure_and_missing_evidence_is_blocked() {
    let temp = TempDir::new();
    for (name, path, expected) in [
        (
            "good",
            examples().join("calculation-evidence-coal-lsl-levy.json"),
            "REVIEW",
        ),
        ("missing", temp.join("missing.json"), "BLOCKED"),
    ] {
        let output = temp.join(name);
        let result = review_command(&output)
            .arg("--calculation-evidence")
            .arg(path)
            .args(["--require-calculation", LABEL])
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2), "{result:?}");
        let document =
            pyjson::loads(&fs::read_to_string(output.join("close-review-pack.json")).unwrap())
                .unwrap();
        assert!(document.at("overall_status").is_str(expected));
        if name == "good" {
            assert_eq!(entry(&document).at("usable").as_bool(), Some(true));
            assert!(viewer::verify_pack(&output).is_ok());
        } else {
            assert!(
                document
                    .at("exceptions")
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item.at("control").is_str("calculation_evidence")
                        && item.at("status").is_str("BLOCKED"))
            );
            assert!(
                document
                    .at("calculation_evidence")
                    .at("supplied")
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    }
}

#[test]
fn evidence_collision_refuses_to_overwrite_or_leave_other_pack_fragments() {
    let temp = TempDir::new();
    let output = temp.join("pack");
    fs::create_dir(&output).unwrap();
    let path = output.join("close-review-pack.json");
    fs::write(&path, GOOD).unwrap();
    let before = fs::read(&path).unwrap();
    let result = review_command(&output)
        .arg("--calculation-evidence")
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert!(result.stdout.is_empty());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("would destroy it")
    );
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(fs::read_dir(output).unwrap().count(), 1);
}
