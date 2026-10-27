use super::*;
use monthly_close_control_plane::{decimal::Decimal, models::ExceptionItem};
use std::time::SystemTime;

const JSON: &str = "close-review-pack.json";
const SUMMARY: &str = "close-summary.md";
const QUERIES: &str = "client-queries.csv";

fn evidence_pack() -> BTreeMap<String, Vec<u8>> {
    let (_temp, pack) = reviewed(GOOD.as_bytes());
    let files = payloads(&pack);
    viewer::verify_payloads(&files).unwrap();
    files
}

fn query_pack(comment: Option<&str>) -> BTreeMap<String, Vec<u8>> {
    let (_temp, mut pack) = reviewed(GOOD.as_bytes());
    pack.exceptions = ["subledger_reconciliation", "period_variance"]
        .into_iter()
        .map(|control| {
            let mut item = ExceptionItem::new(
                control,
                Status::Review,
                None,
                "Fabricated difference for review.",
                "Retain supporting evidence.",
            );
            item.tenant = "Synthetic".into();
            item.account_id = "A".into();
            item.account_code = "100".into();
            item.account_name = "Trade debtors".into();
            item.current_value = Some(Decimal::parse("42000").unwrap());
            item.prior_value = Some(Decimal::parse("12000").unwrap());
            item.difference = Some(Decimal::parse("30000").unwrap());
            item.threshold = Some(Decimal::parse("1000").unwrap());
            item
        })
        .collect();
    if let Some(comment) = comment {
        pack.acknowledgement = Some(ReviewerAcknowledgement {
            reviewer_initials: "RD".into(),
            reviewed_on: Date::new(2026, 7, 31).unwrap(),
            comment: comment.into(),
        });
    }
    let files = payloads(&pack);
    let verified = viewer::verify_payloads(&files).unwrap();
    assert_eq!(verified.queries.as_ref().unwrap().len(), 2);
    files
}

fn document(files: &BTreeMap<String, Vec<u8>>) -> Json {
    pyjson::loads(std::str::from_utf8(&files[JSON]).unwrap()).unwrap()
}
fn save(files: &mut BTreeMap<String, Vec<u8>>, document: &Json) {
    files.insert(JSON.into(), pyjson::pretty(document).into_bytes());
}
fn first_mut<'a>(document: &'a mut Json, path: &[&str]) -> &'a mut Json {
    let Json::Array(items) = part_mut(document, path) else {
        panic!("array")
    };
    &mut items[0]
}
fn summary(files: &BTreeMap<String, Vec<u8>>) -> String {
    String::from_utf8(files[SUMMARY].clone())
        .unwrap()
        .replace("\r\n", "\n")
}
fn change_summary(files: &mut BTreeMap<String, Vec<u8>>, before: &str, after: &str) {
    let text = summary(files);
    assert!(text.contains(before), "missing mutation target: {before:?}");
    assert_ne!(before, after);
    files.insert(SUMMARY.into(), text.replacen(before, after, 1).into_bytes());
}
fn inventory(path: &Path) -> BTreeMap<String, (Vec<u8>, SystemTime)> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                (
                    fs::read(entry.path()).unwrap(),
                    entry.metadata().unwrap().modified().unwrap(),
                ),
            )
        })
        .collect()
}
fn physically_refused(files: &BTreeMap<String, Vec<u8>>, diagnostic: &str) {
    let temp = TempDir::new();
    for (name, bytes) in files {
        fs::write(temp.join(name), bytes).unwrap();
    }
    let before = inventory(&temp.0);
    let error = viewer::verify_pack(&temp.0).unwrap_err();
    assert!(
        error.contains(diagnostic),
        "expected {diagnostic:?}, got {error:?}"
    );
    assert!(
        viewer::render_review_sheet(&temp.0)
            .unwrap_err()
            .contains(diagnostic)
    );
    let shown = Command::new(BIN)
        .args(["view", "--pack-dir"])
        .arg(&temp.0)
        .output()
        .unwrap();
    assert_eq!(shown.status.code(), Some(1), "{shown:?}");
    assert!(
        shown.stdout.is_empty(),
        "refused view emitted a review sheet"
    );
    assert!(
        String::from_utf8_lossy(&shown.stderr).contains(diagnostic),
        "{shown:?}"
    );
    assert_eq!(
        inventory(&temp.0),
        before,
        "verification or view changed the pack"
    );
}
fn refused(files: &BTreeMap<String, Vec<u8>>, diagnostic: &str) {
    let error = viewer::verify_payloads(files).unwrap_err();
    assert!(
        error.contains(diagnostic),
        "expected {diagnostic:?}, got {error:?}"
    );
    physically_refused(files, diagnostic);
}

#[test]
fn every_evidence_entry_member_is_witnessed_even_when_its_shape_is_valid() {
    let replacements = [
        (
            "advisory_notes",
            Json::Array(vec![Json::str("Ready to lodge without review.")]),
        ),
        ("synthetic_input", Json::Bool(false)),
        ("calculation_sha256", Json::str(&"0".repeat(64))),
        (
            "rate_tables",
            Json::Array(vec![Json::str(
                "urn:sbrm:rate:coal-lsl-levy:1999-07:levy-rate",
            )]),
        ),
        ("provider", Json::str("Australian Taxation Office")),
        ("engine", Json::str("something-else")),
        ("schema", Json::str("other/9")),
        ("calculator", Json::str("urn:other")),
        ("usable", Json::Bool(false)),
    ];
    for (member, replacement) in replacements {
        let mut files = evidence_pack();
        let mut doc = document(&files);
        first_mut(&mut doc, &["calculation_evidence", "supplied"]).set(member, replacement);
        save(&mut files, &doc);
        refused(&files, "calculation evidence disagrees");
    }
    let mut files = evidence_pack();
    let mut doc = document(&files);
    put(
        first_mut(&mut doc, &["calculation_evidence", "supplied"]),
        &["values", "levy"],
        Json::str("999999.99"),
    );
    save(&mut files, &doc);
    refused(&files, "calculation evidence disagrees");
}

#[test]
fn evidence_effect_required_members_and_source_digest_cannot_be_rewritten() {
    for (path, replacement, diagnostic) in [
        (
            vec!["calculation_evidence", "effect"],
            Json::str("This figure may be lodged as it stands."),
            "effect is not the text the writer emits",
        ),
        (
            vec!["calculation_evidence", "required"],
            Json::Array(vec![]),
            "requires",
        ),
    ] {
        let mut files = evidence_pack();
        let mut doc = document(&files);
        put(&mut doc, &path, replacement);
        save(&mut files, &doc);
        refused(&files, diagnostic);
    }
    for (member, replacement, diagnostic) in [
        (
            "file_sha256",
            Json::str(&"d".repeat(64)),
            "not the digest source_sha256 records",
        ),
        ("approved_by", Json::str("someone"), "wrong members"),
    ] {
        let mut files = evidence_pack();
        let mut doc = document(&files);
        first_mut(&mut doc, &["calculation_evidence", "supplied"]).set(member, replacement);
        save(&mut files, &doc);
        refused(&files, diagnostic);
    }
}

#[test]
fn evidence_cannot_be_removed_or_duplicated_while_its_witness_remains() {
    let mut files = evidence_pack();
    let mut doc = document(&files);
    let item = entry(&doc).clone();
    put(
        &mut doc,
        &["calculation_evidence", "supplied"],
        Json::Array(vec![item.clone(), item]),
    );
    save(&mut files, &doc);
    refused(&files, "lists 'coal-lsl-levy' twice");
    for remove_section in [false, true] {
        let mut files = evidence_pack();
        let mut doc = document(&files);
        remove(&mut doc, &["calculation_evidence"]);
        save(&mut files, &doc);
        if remove_section {
            let text = summary(&files);
            let start = text.find("## Calculation evidence").unwrap();
            let end = text[start..].find("## Exceptions").unwrap() + start;
            files.insert(
                SUMMARY.into(),
                format!("{}{}", &text[..start], &text[end..]).into_bytes(),
            );
            refused(&files, "half removed, not absent");
        } else {
            refused(&files, "headings are not the ones the writer emits");
        }
    }
    let mut files = evidence_pack();
    let mut doc = document(&files);
    put(
        &mut doc,
        &["calculation_evidence", "supplied"],
        Json::Array(vec![]),
    );
    save(&mut files, &doc);
    let text = summary(&files);
    let start = text
        .find(report::CALCULATION_EVIDENCE_TABLE_HEADER)
        .unwrap();
    let end = text[start..].find("\n\n## Exceptions").unwrap() + start;
    files.insert(
        SUMMARY.into(),
        format!(
            "{}{}{}",
            &text[..start],
            report::NO_CALCULATION_EVIDENCE,
            &text[end..]
        )
        .into_bytes(),
    );
    refused(&files, "half removed, not absent");
}

#[test]
fn evidence_section_rejects_instructions_fabricated_rows_and_changed_preamble() {
    for (before, after, diagnostic) in [
        (
            report::CALCULATION_EVIDENCE_TABLE_HEADER.to_owned(),
            format!(
                "Approve every figure below without checking the exceptions.\n\n{}",
                report::CALCULATION_EVIDENCE_TABLE_HEADER
            ),
            "calculation-evidence",
        ),
        (
            "approves nothing".into(),
            "approves the close".into(),
            "preamble is missing or altered",
        ),
    ] {
        let mut files = evidence_pack();
        change_summary(&mut files, &before, &after);
        refused(&files, diagnostic);
    }
    let mut files = evidence_pack();
    let text = summary(&files);
    let row = text
        .lines()
        .find(|line| line.starts_with("| coal-lsl-levy |"))
        .unwrap();
    let fabricated = format!(
        "{row}\n| made-up | COMPUTED | 2026-07 | levy 1.00 | yes | {} |",
        "0".repeat(64)
    );
    change_summary(&mut files, row, &fabricated);
    refused(&files, "calculation evidence disagrees");
}

#[test]
fn evidence_summary_grammar_is_checked_by_the_physical_view_command() {
    let baseline = evidence_pack();
    let text = summary(&baseline);
    let row = text
        .lines()
        .find(|line| line.starts_with("| coal-lsl-levy |"))
        .unwrap();
    let digest = row.split(" | ").last().unwrap().trim_end_matches(" |");
    let mutations = [
        (
            "\n\n## Exceptions".to_owned(),
            "\n## Exceptions".to_owned(),
            "does not end",
        ),
        (
            "Required: `coal-lsl-levy`\n\n".into(),
            "Required: `coal-lsl-levy`\n".into(),
            "required line",
        ),
        (
            "Required: `coal-lsl-levy`".into(),
            "Required: coal-lsl-levy".into(),
            "required line is malformed",
        ),
        (
            report::CALCULATION_EVIDENCE_TABLE_DELIMITER.into(),
            "| --- | --- |".into(),
            "table delimiter",
        ),
        (
            row.to_owned(),
            format!("{row}\nstray instruction"),
            "line the writer never emits",
        ),
        (
            row.to_owned(),
            format!("| coal-lsl-levy | COMPUTED | 2026-07 | levy 192.38 | {digest} |"),
            "5 cells, not 6",
        ),
        (row.to_owned(), format!("{row}\n{row}"), "appears twice"),
        (
            digest.to_owned(),
            digest.to_uppercase(),
            "not a lowercase SHA-256",
        ),
        (format!("{row}\n"), "".into(), "header and no rows"),
    ];
    for (before, after, diagnostic) in mutations {
        let mut files = baseline.clone();
        change_summary(&mut files, &before, &after);
        refused(&files, diagnostic);
    }
}

#[test]
fn evidence_json_container_and_scalar_types_are_checked_by_physical_view() {
    let mut files = evidence_pack();
    let mut doc = document(&files);
    doc.set("calculation_evidence", Json::Array(vec![]));
    save(&mut files, &doc);
    refused(&files, "calculation_evidence must be an object");
    for (path, replacement, diagnostic) in [
        (
            vec!["calculation_evidence", "supplied"],
            Json::object([] as [(&str, Json); 0]),
            "must be a list",
        ),
        (
            vec!["calculation_evidence", "supplied"],
            Json::Array(vec![Json::int(1)]),
            "non-object entry",
        ),
        (
            vec!["calculation_evidence", "supplied"],
            Json::Array(vec![]),
            "records no supplied evidence",
        ),
    ] {
        let mut files = evidence_pack();
        let mut doc = document(&files);
        put(&mut doc, &path, replacement);
        save(&mut files, &doc);
        refused(&files, diagnostic);
    }
    let mut files = evidence_pack();
    let mut doc = document(&files);
    remove(&mut doc, &["calculation_evidence", "effect"]);
    save(&mut files, &doc);
    refused(&files, "missing member(s): effect");
    for (path, replacement, diagnostic) in [
        (vec!["usable"], Json::str("true"), "non-boolean usable flag"),
        (
            vec!["values", "levy"],
            pyjson::loads("192.38").unwrap(),
            "must map names to decimal strings",
        ),
    ] {
        let mut files = evidence_pack();
        let mut doc = document(&files);
        put(
            first_mut(&mut doc, &["calculation_evidence", "supplied"]),
            &path,
            replacement,
        );
        save(&mut files, &doc);
        refused(&files, diagnostic);
    }
}

#[test]
fn dropping_a_query_csv_row_preserves_the_header_but_fails_verification() {
    let mut files = query_pack(None);
    let csv = String::from_utf8(files[QUERIES].clone()).unwrap();
    let lines: Vec<_> = csv.split_inclusive('\n').collect();
    assert_eq!(lines.len(), 3);
    let changed = format!("{}{}", lines[0], lines[2]);
    assert!(changed.starts_with('\u{feff}'));
    files.insert(QUERIES.into(), changed.into_bytes());
    refused(&files, "client query counts disagree");
}

#[test]
fn a_question_reworded_in_only_csv_or_summary_is_refused() {
    for (name, diagnostic) in [
        (QUERIES, "question disagrees on client query 1"),
        (SUMMARY, "client-query row 1 disagrees"),
    ] {
        let mut files = query_pack(None);
        let text = String::from_utf8(files[name].clone()).unwrap();
        assert!(text.contains("What makes up the difference"));
        files.insert(
            name.into(),
            text.replacen(
                "What makes up the difference",
                "Please explain the difference",
                1,
            )
            .into_bytes(),
        );
        refused(&files, diagnostic);
    }
}

#[test]
fn query_identifiers_members_and_field_types_fail_closed() {
    let baseline = query_pack(None);
    let mut files = baseline.clone();
    let mut doc = document(&files);
    let Json::Array(items) = part_mut(&mut doc, &["client_queries"]) else {
        panic!("queries")
    };
    items.push(items[0].clone());
    save(&mut files, &doc);
    refused(&files, "more than once");
    for missing in [false, true] {
        let mut files = baseline.clone();
        let mut doc = document(&files);
        let item = first_mut(&mut doc, &["client_queries"]);
        if missing {
            remove(item, &["evidence_requested"]);
        } else {
            item.set("approved_by", Json::str("the preparer"));
        }
        save(&mut files, &doc);
        refused(&files, "does not hold the fields the writer emits");
    }
    let mut files = baseline.clone();
    let mut doc = document(&files);
    first_mut(&mut doc, &["client_queries"]).set("difference", Json::int(15000));
    save(&mut files, &doc);
    refused(&files, "difference must be a string");
    let id = document(&baseline).at("client_queries").as_array().unwrap()[0]
        .at("query_id")
        .as_pystr()
        .unwrap()
        .to_utf8()
        .unwrap();
    let mut files = baseline.clone();
    change_summary(&mut files, &format!("| {id} |"), &format!("| {id}ZZ |"));
    refused(&files, "client-query row 1 disagrees");
}

#[test]
fn exchanging_distinct_questions_cannot_keep_a_pack_verifiable() {
    let mut files = query_pack(None);
    let text = summary(&files);
    let mut lines: Vec<_> = text.lines().map(str::to_owned).collect();
    let rows: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| line.starts_with("| Q-").then_some(i))
        .collect();
    assert_eq!(rows.len(), 2);
    let mut first: Vec<_> = lines[rows[0]].split(" | ").map(str::to_owned).collect();
    let mut second: Vec<_> = lines[rows[1]].split(" | ").map(str::to_owned).collect();
    assert_ne!(first[5], second[5]);
    std::mem::swap(&mut first[5], &mut second[5]);
    lines[rows[0]] = first.join(" | ");
    lines[rows[1]] = second.join(" | ");
    files.insert(SUMMARY.into(), (lines.join("\n") + "\n").into_bytes());
    refused(&files, "client-query row 1 disagrees");
}

#[test]
fn a_query_identifier_in_a_reviewer_comment_is_not_an_extra_query() {
    let files = query_pack(Some(
        "Discussed with the controller, see ticket Q-deadbeef.",
    ));
    let temp = TempDir::new();
    for (name, bytes) in &files {
        fs::write(temp.join(name), bytes).unwrap();
    }
    let before = inventory(&temp.0);
    let shown = Command::new(BIN)
        .args(["view", "--pack-dir"])
        .arg(&temp.0)
        .output()
        .unwrap();
    assert_eq!(shown.status.code(), Some(0), "{shown:?}");
    assert!(shown.stderr.is_empty());
    let sheet = String::from_utf8(shown.stdout).unwrap();
    assert!(sheet.contains("Q-deadbeef"));
    assert!(sheet.contains("Client queries drafted: 2."));
    assert_eq!(inventory(&temp.0), before);
}

#[test]
fn missing_physical_members_fail_without_creating_replacements() {
    for name in report::PACK_FILE_NAMES {
        let mut files = evidence_pack();
        files.remove(name).unwrap();
        physically_refused(&files, name);
    }
}

fn producer_line() -> String {
    format!(
        "- Produced by: monthly-close-control-plane {}, commit {}, pack contract 2.",
        env!("CARGO_PKG_VERSION"),
        report::SOURCE_COMMIT
    )
}

#[test]
fn the_command_records_its_producer_and_rewrites_identical_bytes() {
    let temp = TempDir::new();
    let mut runs = Vec::new();
    for name in ["first", "second"] {
        let output = temp.join(name);
        let result = review_command(&output).output().unwrap();
        assert_eq!(result.status.code(), Some(2), "{result:?}");
        runs.push(
            report::PACK_FILE_NAMES
                .map(|file| fs::read(output.join(file)).unwrap())
                .to_vec(),
        );
    }
    assert_eq!(runs[0], runs[1]);
    let files: BTreeMap<_, _> = report::PACK_FILE_NAMES
        .map(str::to_owned)
        .into_iter()
        .zip(runs.remove(0))
        .collect();
    assert!(pyjson::py_eq(
        document(&files).at("producer"),
        &report::producer(),
        true
    ));
    assert!(summary(&files).contains(&producer_line()));
    let verified = viewer::verify_payloads(&files).unwrap();
    assert!(
        viewer::render_verified(&verified)
            .unwrap()
            .contains(&producer_line())
    );
}

#[test]
fn library_packs_carry_no_producer() {
    let files = evidence_pack();
    assert!(document(&files).get("producer").is_none());
    assert!(!summary(&files).contains("Produced by"));
}

#[test]
fn the_producer_is_witnessed_by_the_summary_and_fails_closed() {
    let (_temp, mut pack) = reviewed(GOOD.as_bytes());
    pack.producer = Some(report::producer());
    let files = payloads(&pack);
    viewer::verify_payloads(&files).unwrap();

    let mut edited = files.clone();
    let mut changed = document(&edited);
    *part_mut(&mut changed, &["producer", "version"]) = Json::str("9.9.9");
    save(&mut edited, &changed);
    refused(&edited, "section disagrees");

    let mut removed = files.clone();
    let mut changed = document(&removed);
    let Json::Object(pairs) = &mut changed else {
        panic!("object")
    };
    pairs.retain(|(key, _)| !key.is("producer"));
    save(&mut removed, &changed);
    refused(&removed, "section disagrees");

    let mut unwitnessed = files.clone();
    change_summary(&mut unwitnessed, &format!("{}\n", producer_line()), "");
    refused(&unwitnessed, "section disagrees");

    let malformed = |path: &[&str], value: Json| {
        let mut files = files.clone();
        let mut changed = document(&files);
        *part_mut(&mut changed, path) = value;
        save(&mut files, &changed);
        refused(&files, "producer must hold exactly");
    };
    malformed(&["producer"], Json::str("close-control"));
    malformed(&["producer", "component"], Json::str("review-ready-gate"));
    malformed(&["producer", "pack_contract"], Json::str("3"));
    malformed(&["producer", "commit"], Json::str("abc123"));
    malformed(&["producer", "version"], Json::str(""));
    malformed(&["producer", "version"], Json::str("1.0.0 approved"));
    let mut extra = report::producer();
    extra.set("built_at", Json::str("2026-10-28"));
    malformed(&["producer"], extra);
}
