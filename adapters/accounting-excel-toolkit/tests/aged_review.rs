use accounting_excel_toolkit::aged::{
    Json, amount, date, evidence, inspect_export, json, records, replay,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const SOURCE: &[u8] = include_bytes!("../samples/sample-aged-receivables-review.csv");
const MANIFEST: &str = include_str!("../samples/sample-aged-receivables-manifest.json");
const CONTROL: &str = include_str!("../samples/sample-aged-receivables-control.json");
const HEADER: &str = "Contact,Current,< 1 Month,1 Month,2 Months,3 Months,Older,Total";
const VERIFIER: &str = env!("CARGO_BIN_EXE_xero-aged-receivables");
const EVIDENCE: &str = env!("CARGO_BIN_EXE_debtor-evidence");
const REPLAY: &str = env!("CARGO_BIN_EXE_replay-aged-review");
static SERIAL: AtomicUsize = AtomicUsize::new(0);

struct Temp {
    root: PathBuf,
    parent: PathBuf,
}

impl Temp {
    fn new() -> Self {
        Self::inside(&std::env::temp_dir())
    }

    fn inside(parent: &Path) -> Self {
        let parent = fs::canonicalize(parent).unwrap();
        for _ in 0..100 {
            let root = parent.join(format!(
                "fabricated-aged-tests-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&root) {
                Ok(()) => return Self { root, parent },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("{error}"),
            }
        }
        panic!("could not allocate a test directory");
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        if self.root.parent() == Some(self.parent.as_path())
            && fs::symlink_metadata(&self.root).is_ok_and(|m| !m.file_type().is_symlink())
        {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn digest(source: &[u8]) -> String {
    Sha256::digest(source)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn manifest(source: &[u8]) -> Json {
    let mut value = json::loads(MANIFEST).unwrap();
    value.set("source_sha256", Json::str(&digest(source)));
    value
}

fn control() -> Json {
    json::loads(CONTROL).unwrap()
}

fn changed(old: &str, new: &str) -> Vec<u8> {
    let source = std::str::from_utf8(SOURCE).unwrap();
    assert!(source.contains(old), "missing fixture text: {old}");
    source.replace(old, new).into_bytes()
}

fn csv(rows: &[Vec<String>]) -> Vec<u8> {
    let mut output = String::new();
    for row in rows {
        let fields: Vec<_> = row
            .iter()
            .map(|field| {
                if field.contains([',', '"', '\r', '\n']) || (row.len() == 1 && field.is_empty()) {
                    format!("\"{}\"", field.replace('"', "\"\""))
                } else {
                    field.clone()
                }
            })
            .collect();
        output.push_str(&fields.join(","));
        output.push_str("\r\n");
    }
    output.into_bytes()
}

fn review(source: &[u8]) -> Json {
    review_with(source, &[])
}

fn review_with(source: &[u8], changes: &[(&str, Json)]) -> Json {
    let mut value = manifest(source);
    for (key, field) in changes {
        value.set(key, field.clone());
    }
    inspect_export(source, &value, Some(&control())).unwrap()
}

fn text(value: &Json) -> String {
    value.as_pystr().unwrap().to_utf8().unwrap()
}

fn array<'a>(value: &'a Json, key: &str) -> &'a [Json] {
    value.at(key).as_array().unwrap()
}

fn fields(value: &Json, collection: &str, field: &str) -> Vec<String> {
    array(value, collection)
        .iter()
        .map(|row| text(row.at(field)))
        .collect()
}

fn status(value: &Json, expected: &str) {
    assert!(value.at("summary_status").is_str(expected), "{value:?}");
    assert!(value.at("debtor_decisions").is_str("REVIEW"));
}

fn same(left: &Json, right: &Json) {
    assert!(json::py_eq(left, right, true), "{left:?} != {right:?}");
}

fn all_agree(value: &Json) {
    assert!(
        array(value, "ties")
            .iter()
            .all(|tie| tie.at("agrees").as_bool() == Some(true))
    );
}

fn failed(value: &Json) -> Vec<String> {
    array(value, "ties")
        .iter()
        .filter(|tie| tie.at("agrees").as_bool() == Some(false))
        .map(|tie| text(tie.at("check")))
        .collect()
}

fn tie<'a>(value: &'a Json, check: &str) -> &'a Json {
    array(value, "ties")
        .iter()
        .find(|tie| tie.at("check").is_str(check))
        .unwrap()
}

fn has_exception(value: &Json, needle: &str) -> bool {
    array(value, "exceptions")
        .iter()
        .any(|issue| text(issue).contains(needle))
}

fn no_control_tie(value: &Json) {
    assert!(
        !fields(value, "ties", "check").contains(&"export footer to receivables control".into())
    );
}

fn no_column_ties(value: &Json) {
    assert!(
        !fields(value, "ties", "check")
            .iter()
            .any(|name| name.starts_with("contact column "))
    );
}

fn remove(value: &mut Json, key: &str) {
    let Json::Object(fields) = value else {
        panic!("expected object")
    };
    fields.retain(|(name, _)| !name.is(key));
}

fn terminal(text: &str) -> Vec<u8> {
    if cfg!(windows) {
        text.replace('\n', "\r\n").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

fn refused(output: &Output, code: i32, message: &str) {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert!(output.stdout.is_empty(), "unexpected partial output");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(message),
        "{output:?}"
    );
}

fn verifier(temp: &Temp, source: &[u8], manifest: &Json, control: Option<&Json>) -> Output {
    let source = temp.write("source.csv", source);
    let manifest = temp.write("manifest.json", json::dumps(manifest, false).as_bytes());
    let mut command = Command::new(VERIFIER);
    command.arg(source).arg("--manifest").arg(manifest);
    if let Some(control) = control {
        command
            .arg("--control")
            .arg(temp.write("control.json", json::dumps(control, false).as_bytes()));
    }
    command.output().unwrap()
}

fn verifier_inputs_unchanged(temp: &Temp, source: &[u8], manifest: &Json, control: &Json) {
    assert_eq!(fs::read(temp.root.join("source.csv")).unwrap(), source);
    assert_eq!(
        fs::read(temp.root.join("manifest.json")).unwrap(),
        json::dumps(manifest, false).as_bytes()
    );
    assert_eq!(
        fs::read(temp.root.join("control.json")).unwrap(),
        json::dumps(control, false).as_bytes()
    );
    assert_eq!(fs::read_dir(&temp.root).unwrap().count(), 3);
}

fn review_files(output_dir: &Path, result: &Json) {
    for (filename, expected) in [
        ("summary-review.json", json::pretty_ordered(result) + "\n"),
        (
            "evidence-requests.json",
            json::pretty_ordered(&evidence::requests(result)) + "\n",
        ),
        ("summary-review.txt", evidence::briefing(result)),
    ] {
        assert_eq!(
            fs::read(output_dir.join(filename)).unwrap(),
            terminal(&expected)
        );
    }
    assert_eq!(fs::read_dir(output_dir).unwrap().count(), 3);
}

fn output_cli(tool: &str, source: &Path, output: &Path) -> Output {
    let mut command = Command::new(tool);
    if tool == EVIDENCE {
        command.arg("draft").arg(source);
    } else {
        command.args(["--case", "clean"]);
    }
    command.arg("--output").arg(output).output().unwrap()
}

#[test]
fn three_way_tie_preserves_duplicate_labels_credit_and_total_contact() {
    let result = review(SOURCE);
    status(&result, "PASS");
    assert_eq!(
        fields(&result, "rows", "Contact"),
        ["00123", "00123", "Total"]
    );
    for (row, number) in array(&result, "rows").iter().zip([7, 8, 9]) {
        same(row.at("source_record"), &Json::int(number));
    }
    assert!(array(&result, "rows")[1].at("Total").is_str("-25"));
    all_agree(&result);
    assert_eq!(array(&result, "ties").len(), 13);
    assert!(
        array(&result, "ties")
            .last()
            .unwrap()
            .at("left")
            .is_str("85")
    );
    for (column, total) in [
        ("Current", "100"),
        ("< 1 Month", "-25"),
        ("1 Month", "10"),
        ("2 Months", "0"),
        ("3 Months", "0"),
        ("Older", "0"),
    ] {
        assert!(
            tie(
                &result,
                &format!("contact column {column} to export footer {column}")
            )
            .at("left")
            .is_str(total)
        );
    }
}

#[test]
fn permutations_preserve_supported_totals() {
    let rows = records(SOURCE).unwrap();
    for indices in [
        [6, 7, 8],
        [6, 8, 7],
        [7, 6, 8],
        [7, 8, 6],
        [8, 6, 7],
        [8, 7, 6],
    ] {
        let mut reordered = rows.clone();
        for (target, from) in (6..9).zip(indices) {
            reordered[target] = rows[from].clone();
        }
        let result = review(&csv(&reordered));
        status(&result, "PASS");
        assert_eq!(array(&result, "rows").len(), 3);
        for (row, number) in array(&result, "rows").iter().zip([7, 8, 9]) {
            same(row.at("source_record"), &Json::int(number));
        }
    }
}

#[test]
fn split_credit_and_reversed_signs_preserve_supported_totals() {
    let rows = records(SOURCE).unwrap();
    let mut split = rows.clone();
    let mut first = split[7].clone();
    let mut second = first.clone();
    first[2] = "-10".into();
    first[7] = "-10".into();
    second[2] = "-15".into();
    second[7] = "-15".into();
    split.splice(7..8, [first, second]);
    let result = review_with(
        &csv(&split),
        &[
            ("footer_record", Json::int(11)),
            ("on_screen_rows", Json::int(4)),
        ],
    );
    status(&result, "PASS");
    assert_eq!(
        fields(&result, "rows", "Total"),
        ["100", "-10", "-15", "10"]
    );
    assert_eq!(array(&result, "ties").len(), 14);
    let mut negated = rows;
    for row in &mut negated[6..] {
        for field in &mut row[1..] {
            *field = if field.is_empty() || field == "0" {
                "0".into()
            } else if let Some(unsigned) = field.strip_prefix('-') {
                unsigned.into()
            } else {
                format!("-{field}")
            };
        }
    }
    let source = csv(&negated);
    let mut manifest = manifest(&source);
    manifest.set("on_screen_total", Json::str("-85"));
    let mut control = control();
    control.set("amount", Json::str("-85"));
    let result = inspect_export(&source, &manifest, Some(&control)).unwrap();
    status(&result, "PASS");
    assert_eq!(fields(&result, "rows", "Total"), ["-100", "25", "-10"]);
}

#[test]
fn optional_current_bom_quoted_commas_and_newlines() {
    let mut rows = records(SOURCE).unwrap();
    for row in &mut rows[5..] {
        row.remove(1);
    }
    rows[6][1] = "100".into();
    rows[6][0] = " 001, Caf\u{e9}\nBranch ".into();
    rows[9][1] = "75".into();
    let source = [b"\xef\xbb\xbf".as_slice(), &csv(&rows)].concat();
    let result = review(&source);
    status(&result, "PASS");
    assert!(
        array(&result, "rows")[0]
            .at("Contact")
            .is_str(" 001, Caf\u{e9}\nBranch ")
    );
    let checks = fields(&result, "ties", "check");
    assert_eq!(
        checks
            .iter()
            .filter(|s| s.starts_with("contact column "))
            .count(),
        5
    );
    assert!(!checks.iter().any(|s| s.contains("Current")));
    rows[9][1] = "65".into();
    rows[9][5] = "10".into();
    let result = review(&csv(&rows));
    status(&result, "REVIEW");
    assert_eq!(
        failed(&result),
        [
            "contact column < 1 Month to export footer < 1 Month",
            "contact column Older to export footer Older"
        ]
    );
}

#[test]
fn offsetting_footer_errors_are_reported_for_every_ageing_column() {
    let baseline = review(SOURCE);
    let rows = records(SOURCE).unwrap();
    for index in 1..7 {
        let other = if index == 6 { 1 } else { 6 };
        let mut altered = rows.clone();
        let current: i32 = altered[9][index].parse().unwrap();
        let other_current: i32 = altered[9][other].parse().unwrap();
        altered[9][index] = (current - 10).to_string();
        altered[9][other] = (other_current + 10).to_string();
        let result = review(&csv(&altered));
        status(&result, "REVIEW");
        same(result.at("rows"), baseline.at("rows"));
        let mut expected = vec![
            format!(
                "contact column {} to export footer {}",
                rows[5][index], rows[5][index]
            ),
            format!(
                "contact column {} to export footer {}",
                rows[5][other], rows[5][other]
            ),
        ];
        let mut actual = failed(&result);
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected);
        assert_eq!(array(&result, "exceptions").len(), 2);
        let other_ties: Vec<_> = array(&result, "ties")
            .iter()
            .filter(|t| !text(t.at("check")).starts_with("contact column "))
            .collect();
        assert_eq!(other_ties.len(), 7);
        assert!(
            other_ties
                .iter()
                .all(|t| t.at("agrees").as_bool() == Some(true))
        );
    }
}

#[test]
fn column_ties_keep_both_signed_half_cent_boundaries() {
    for (current, older, delta, opposite, agrees) in [
        ("99.9951", "0.0049", "0.0049", "-0.0049", true),
        ("99.995", "0.005", "0.005", "-0.005", false),
        ("100.005", "-0.005", "-0.005", "0.005", false),
    ] {
        let mut rows = records(SOURCE).unwrap();
        rows[9][1] = current.into();
        rows[9][6] = older.into();
        let result = review(&csv(&rows));
        status(&result, if agrees { "PASS" } else { "REVIEW" });
        for (column, difference) in [("Current", delta), ("Older", opposite)] {
            let tie = tie(
                &result,
                &format!("contact column {column} to export footer {column}"),
            );
            assert!(tie.at("difference").is_str(difference));
            assert_eq!(tie.at("agrees").as_bool(), Some(agrees));
        }
        assert!(
            array(&result, "ties")
                .iter()
                .filter(|t| !text(t.at("check")).starts_with("contact column "))
                .all(|t| t.at("agrees").as_bool() == Some(true))
        );
    }
}

#[test]
fn unsupported_titles_do_not_skip_arithmetic_or_preserved_rows() {
    let baseline = review(SOURCE);
    for title in [
        "Aged Payables Summary",
        "Unrelated Summary",
        "Aged Receivables Detail",
        "Aged Receivables Summary - Custom",
        "aged receivables summary",
    ] {
        let result = review_with(
            &changed("Aged Receivables Summary", title),
            &[("report_name", Json::str(title))],
        );
        status(&result, "REVIEW");
        all_agree(&result);
        same(result.at("rows"), baseline.at("rows"));
        same(
            result.at("source_sha256"),
            result.at("manifest").at("source_sha256"),
        );
        same(
            result.at("exceptions"),
            &Json::Array(vec![
                Json::str("The first CSV record does not declare the supported report title."),
                Json::str("Manifest report_name does not declare the supported report title."),
            ]),
        );
    }
}

#[test]
fn source_and_manifest_titles_are_checked_independently() {
    for (source_title, manifest_title, expected) in [
        (
            "Unrelated Summary",
            "Aged Receivables Summary",
            "first CSV record",
        ),
        (
            "Aged Receivables Summary",
            "Aged Payables Summary",
            "Manifest report_name",
        ),
        (
            "Aged Receivables Summary",
            " Aged Receivables Summary ",
            "Manifest report_name",
        ),
        ("", "Aged Receivables Summary", "first CSV record"),
    ] {
        let result = review_with(
            &changed("Aged Receivables Summary", source_title),
            &[("report_name", Json::str(manifest_title))],
        );
        status(&result, "REVIEW");
        all_agree(&result);
        assert_eq!(array(&result, "exceptions").len(), 1);
        assert!(has_exception(&result, expected));
    }
    status(
        &review(&changed(
            "Aged Receivables Summary",
            " Aged Receivables Summary ",
        )),
        "PASS",
    );
}

#[test]
fn canonical_title_in_other_metadata_cannot_rescue_first_record() {
    for title in ["Unrelated Summary", ""] {
        let source = String::from_utf8(changed("Aged Receivables Summary", title))
            .unwrap()
            .replace("Synthetic Entity A", "Aged Receivables Summary");
        let mut manifest = manifest(source.as_bytes());
        manifest.set("entity", Json::str("Aged Receivables Summary"));
        let mut control = control();
        control.set("entity", Json::str("Aged Receivables Summary"));
        let result = inspect_export(source.as_bytes(), &manifest, Some(&control)).unwrap();
        status(&result, "REVIEW");
        all_agree(&result);
        same(
            result.at("exceptions"),
            &Json::Array(vec![Json::str(
                "The first CSV record does not declare the supported report title.",
            )]),
        );
    }
}

#[test]
fn large_signed_amounts_remain_exact_without_ambient_precision() {
    let mut rows = records(SOURCE).unwrap();
    for index in [6, 9] {
        rows[index][1] = "999999999999999.9999".into();
    }
    rows[6][7] = "999999999999999.9999".into();
    for index in [7, 9] {
        rows[index][2] = "-999999999999999.9998".into();
    }
    rows[7][7] = "-999999999999999.9998".into();
    rows[9][7] = "10.0001".into();
    let source = csv(&rows);
    let mut manifest = manifest(&source);
    manifest.set("on_screen_total", Json::str("10.0001"));
    let mut control = control();
    control.set("amount", Json::str("10.0001"));
    let result = inspect_export(&source, &manifest, Some(&control)).unwrap();
    status(&result, "PASS");
    all_agree(&result);
    for (check, expected) in [
        (
            "contact column Current to export footer Current",
            "999999999999999.9999",
        ),
        (
            "contact column < 1 Month to export footer < 1 Month",
            "-999999999999999.9998",
        ),
        ("contact totals to export footer", "10.0001"),
    ] {
        assert!(tie(&result, check).at("left").is_str(expected));
    }
}

#[test]
fn other_metadata_cannot_masquerade_as_the_source_entity() {
    for entity in [
        "Aged Receivables Summary",
        "As at 30 September 2026",
        "Ageing by due date",
        "Synthetic Entity B",
    ] {
        let mut manifest = manifest(SOURCE);
        manifest.set("entity", Json::str(entity));
        let mut control = control();
        control.set("entity", Json::str(entity));
        let result = inspect_export(SOURCE, &manifest, Some(&control)).unwrap();
        status(&result, "REVIEW");
        no_control_tie(&result);
        all_agree(&result);
        assert!(has_exception(&result, "contradictory entity"));
        assert_eq!(array(&result, "rows").len(), 3);
    }
}

#[test]
fn missing_duplicate_or_contradictory_entity_records_need_review() {
    for (entity, footer) in [
        ("", 10),
        ("Synthetic Entity A\nSynthetic Entity A", 11),
        ("Synthetic Entity A\nSynthetic Entity B", 11),
    ] {
        let result = review_with(
            &changed("Synthetic Entity A", entity),
            &[("footer_record", Json::int(footer))],
        );
        status(&result, "REVIEW");
        no_control_tie(&result);
        all_agree(&result);
        assert!(has_exception(&result, "contradictory entity"));
        assert_eq!(
            fields(&result, "rows", "Contact"),
            ["00123", "00123", "Total"]
        );
    }
}

#[test]
fn control_tie_requires_source_confirmed_cutoff() {
    for (date, footer) in [
        ("As at 31 August 2026", 10),
        ("", 10),
        ("As at 30 September 2026\nAs at 30 September 2026", 11),
    ] {
        let result = review_with(
            &changed("As at 30 September 2026", date),
            &[("footer_record", Json::int(footer))],
        );
        status(&result, "REVIEW");
        no_control_tie(&result);
        all_agree(&result);
    }
    let mut manifest = manifest(SOURCE);
    manifest.set("as_at", Json::str("2026-08-31"));
    let mut control = control();
    control.set("as_at", Json::str("2026-08-31"));
    let result = inspect_export(SOURCE, &manifest, Some(&control)).unwrap();
    status(&result, "REVIEW");
    no_control_tie(&result);
    all_agree(&result);
}

#[test]
fn distinct_entity_record_can_move_within_preheader_metadata() {
    let mut rows = records(SOURCE).unwrap();
    rows.swap(1, 2);
    let result = review(&csv(&rows));
    status(&result, "PASS");
    all_agree(&result);
    assert_eq!(array(&result, "ties").len(), 13);
}

#[test]
fn cli_preserves_pass_and_review_exits() {
    let temp = Temp::new();
    for (source, title, code, expected) in [
        (SOURCE.to_vec(), "Aged Receivables Summary", 0, "PASS"),
        (
            changed("Aged Receivables Summary", "Aged Payables Summary"),
            "Aged Payables Summary",
            2,
            "REVIEW",
        ),
        (
            changed("Aged Receivables Summary", "Unrelated Summary"),
            "Unrelated Summary",
            2,
            "REVIEW",
        ),
        (
            changed("Total,100,-25,10,0,0,0,85", "Total,90,-25,10,0,0,10,85"),
            "Aged Receivables Summary",
            2,
            "REVIEW",
        ),
    ] {
        let mut manifest = manifest(&source);
        manifest.set("report_name", Json::str(title));
        let control = control();
        let output = verifier(&temp, &source, &manifest, Some(&control));
        assert_eq!(output.status.code(), Some(code), "{output:?}");
        assert!(output.stderr.is_empty());
        let result = inspect_export(&source, &manifest, Some(&control)).unwrap();
        status(&result, expected);
        assert_eq!(
            output.stdout,
            terminal(&(json::pretty_utf8(&result).unwrap() + "\n"))
        );
        verifier_inputs_unchanged(&temp, &source, &manifest, &control);
    }
}

#[test]
fn cli_preserves_invalid_input_exit_and_exact_error_bytes() {
    let temp = Temp::new();
    let source = changed("00123,100,,0,0,0,0,100", "00123,100");
    let manifest = manifest(&source);
    let control = control();
    let output = verifier(&temp, &source, &manifest, Some(&control));
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        terminal(
            "{\"summary_status\": \"INVALID_INPUT\", \"error\": \"Source record 7: field count does not match the header.\"}\n"
        )
    );
    verifier_inputs_unchanged(&temp, &source, &manifest, &control);
}

#[test]
fn blank_is_zero_but_short_extra_and_malformed_rows_are_invalid() {
    assert!(
        array(&review(SOURCE), "rows")[0]
            .at("< 1 Month")
            .is_str("0")
    );
    for row in [
        "00123,100,0,0,0,0,100",
        "00123,100,,0,0,0,0,100,extra",
        "\"unterminated,100,,0,0,0,0,100",
    ] {
        let source = changed("00123,100,,0,0,0,0,100", row);
        assert!(
            inspect_export(&source, &manifest(&source), Some(&control())).is_err(),
            "{row}"
        );
    }
}

#[test]
fn invalid_amounts_and_headers_are_refused() {
    for amount in [
        "NaN",
        "Infinity",
        "-Infinity",
        "TBC",
        "1e3",
        "0.00001",
        "(25.00)",
        "1,00",
    ] {
        let mut rows = records(SOURCE).unwrap();
        rows[6][1] = amount.into();
        let source = csv(&rows);
        assert!(
            inspect_export(&source, &manifest(&source), Some(&control())).is_err(),
            "{amount}"
        );
    }
    for header in [
        "Contact,Current,< 1 Month,1 Month,2 Months,3 Months,Total,Total",
        "Contact,Customer,< 1 Month,1 Month,2 Months,3 Months,Older,Total",
    ] {
        let source = changed(HEADER, header);
        assert!(inspect_export(&source, &manifest(&source), Some(&control())).is_err());
    }
}

#[test]
fn half_cent_boundary_uses_signed_unrounded_differences() {
    for (amount, agrees, difference) in [
        ("100.0049", true, "0.0049"),
        ("100.005", false, "0.005"),
        ("99.995", false, "-0.005"),
    ] {
        let result = review(&changed("00123,100,", &format!("00123,{amount},")));
        status(&result, if agrees { "PASS" } else { "REVIEW" });
        let first = &array(&result, "ties")[0];
        assert_eq!(first.at("agrees").as_bool(), Some(agrees));
        assert!(first.at("difference").is_str(difference));
    }
}

#[test]
fn three_differences_are_kept_separate() {
    let source = changed("00123,100,,0,0,0,0,100", "00123,100,,0,0,0,0,101");
    let mut control = control();
    control.set("amount", Json::str("84"));
    let result = inspect_export(&source, &manifest(&source), Some(&control)).unwrap();
    assert_eq!(
        failed(&result),
        [
            "record 7 buckets to total",
            "contact totals to export footer",
            "export footer to receivables control"
        ]
    );
}

#[test]
fn missing_control_and_mismatched_metadata_remain_review() {
    status(
        &inspect_export(SOURCE, &manifest(SOURCE), None).unwrap(),
        "REVIEW",
    );
    for (key, value) in [
        ("entity", "Synthetic Entity B"),
        ("as_at", "2026-08-31"),
        ("currency", "NZD"),
        ("filters", "one tracking category"),
        ("population", "all assets"),
        ("accounting_basis", "cash"),
        ("sign_convention", "credit-positive"),
        ("source", ""),
    ] {
        let mut control = control();
        control.set(key, Json::str(value));
        let result = inspect_export(SOURCE, &manifest(SOURCE), Some(&control)).unwrap();
        status(&result, "REVIEW");
        no_control_tie(&result);
    }
}

#[test]
fn missing_provenance_footer_or_screen_check_is_not_completion() {
    for key in [
        "generated_at",
        "source_sha256",
        "on_screen_rows",
        "on_screen_total",
        "footer_record",
    ] {
        let mut manifest = manifest(SOURCE);
        remove(&mut manifest, key);
        let result = inspect_export(SOURCE, &manifest, Some(&control())).unwrap();
        status(&result, "REVIEW");
        if key == "footer_record" {
            no_column_ties(&result);
        }
    }
    let result = review_with(SOURCE, &[("footer_record", Json::int(999))]);
    status(&result, "REVIEW");
    no_column_ties(&result);
    for value in [Json::int(7), Json::Bool(true)] {
        let mut manifest = manifest(SOURCE);
        manifest.set("footer_record", value);
        assert!(inspect_export(SOURCE, &manifest, Some(&control())).is_err());
    }
}

#[test]
fn duplicate_json_fields_are_invalid_at_every_object_depth() {
    for text in [
        r#"{"amount":"1","amount":"2"}"#,
        r#"{"a":{"amount":"1","amount":"2"}}"#,
        r#"[{"x":1,"\u0078":2}]"#,
    ] {
        assert!(
            matches!(json::loads(text), Err(json::LoadError::DuplicateKey(_))),
            "{text}"
        );
    }
    let temp = Temp::new();
    let source = temp.write("source.csv", SOURCE);
    for key in ["manifest", "control"] {
        let evidence = temp.write("duplicate.json", b"{\"amount\":\"1\",\"amount\":\"2\"}");
        let valid = temp.write(
            "valid.json",
            json::dumps(&manifest(SOURCE), false).as_bytes(),
        );
        let mut command = Command::new(VERIFIER);
        command.arg(&source).arg("--manifest");
        if key == "manifest" {
            command.arg(&evidence);
        } else {
            command.arg(&valid).arg("--control").arg(&evidence);
        }
        let output = command.output().unwrap();
        refused(&output, 1, "Duplicate JSON field.");
        assert_eq!(
            output.stderr,
            terminal(
                "{\"summary_status\": \"INVALID_INPUT\", \"error\": \"Duplicate JSON field.\"}\n"
            )
        );
        assert_eq!(
            fs::read(&evidence).unwrap(),
            b"{\"amount\":\"1\",\"amount\":\"2\"}"
        );
    }
}

#[test]
fn record_and_screen_counts_require_json_integers() {
    for value in [
        Json::Bool(true),
        Json::Bool(false),
        Json::str("10"),
        Json::Float(10.0),
    ] {
        let mut manifest = manifest(SOURCE);
        manifest.set("footer_record", value);
        assert!(inspect_export(SOURCE, &manifest, Some(&control())).is_err());
    }
    for value in [
        Json::Bool(true),
        Json::Bool(false),
        Json::str("3"),
        Json::Float(3.0),
        Json::int(-1),
    ] {
        let result = review_with(SOURCE, &[("on_screen_rows", value)]);
        status(&result, "REVIEW");
        same(
            result.at("exceptions"),
            &Json::Array(vec![Json::str(
                "On-screen contact row count is missing or does not agree.",
            )]),
        );
    }
}

#[test]
fn section_rows_and_incomplete_timestamp_or_screen_total_need_review() {
    let result = review(&changed("00123,100,,0,0,0,0,100", "Section A,,,,,,,"));
    status(&result, "REVIEW");
    assert!(array(&result, "rows")[0].at("Contact").is_str("Section A"));
    assert!(has_exception(&result, "sectioned exports"));
    for (key, value) in [
        ("generated_at", "2026-10-01"),
        ("generated_at", "2026-10-01T09:00:00"),
        ("on_screen_total", ""),
    ] {
        status(&review_with(SOURCE, &[(key, Json::str(value))]), "REVIEW");
    }
}

#[test]
fn duplicate_or_contradictory_ageing_metadata_never_passes() {
    for extra in ["Ageing by due date", "Ageing by Invoice Date"] {
        let source = changed(
            "Ageing by due date",
            &format!("Ageing by due date\n{extra}"),
        );
        let result = review_with(&source, &[("footer_record", Json::int(11))]);
        status(&result, "REVIEW");
        all_agree(&result);
        assert!(has_exception(&result, "contradictory ageing basis"));
        assert_eq!(
            fields(&result, "rows", "Contact"),
            ["00123", "00123", "Total"]
        );
        assert!(array(&result, "rows")[1].at("Total").is_str("-25"));
    }
}

#[test]
fn output_clis_reject_repository_paths_before_creating_parents() {
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let temp = Temp::inside(checkout);
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/sample-aged-receivables-review.csv");
    for (tool, name, code) in [(EVIDENCE, "evidence", 1), (REPLAY, "replay", 2)] {
        let output = temp.root.join(name).join("new-output");
        refused(
            &output_cli(tool, &source, &output),
            code,
            "outside the checkout",
        );
        assert!(!output.parent().unwrap().exists());
    }
}

#[test]
fn output_clis_accept_new_external_outputs() {
    let temp = Temp::new();
    let source = temp.write("source.csv", SOURCE);
    for (tool, name, code) in [(EVIDENCE, "evidence", 2), (REPLAY, "replay", 0)] {
        let output = temp.root.join(name).join("new-output");
        let result = output_cli(tool, &source, &output);
        assert_eq!(result.status.code(), Some(code), "{result:?}");
        assert!(result.stderr.is_empty());
        assert!(output.exists());
        assert_eq!(
            result.stdout,
            terminal(if tool == EVIDENCE {
                "Source-only draft: REVIEW.\n"
            } else {
                "FAILURE REPLAY QUALIFIED\n"
            })
        );
    }
    assert_eq!(fs::read(source).unwrap(), SOURCE);
}

#[test]
fn output_clis_preserve_existing_external_file() {
    let temp = Temp::new();
    let source = temp.write("source.csv", SOURCE);
    let output = temp.write("existing-output", b"Existing operator evidence.\n");
    for (tool, code) in [(EVIDENCE, 1), (REPLAY, 2)] {
        refused(
            &output_cli(tool, &source, &output),
            code,
            "Output already exists",
        );
        assert_eq!(fs::read(&output).unwrap(), b"Existing operator evidence.\n");
    }
}

#[test]
fn failure_catalogue_runs_actual_cli_and_expected_exits() {
    let result = replay::pack("all", Path::new(VERIFIER)).unwrap();
    let cases = array(&result, "cases");
    assert_eq!(cases.len(), 12);
    let expected = [
        ("clean", 0, "PASS"),
        ("wrong-hash", 2, "REVIEW"),
        ("row-total", 2, "REVIEW"),
        ("column-offset", 2, "REVIEW"),
        ("wrong-period", 2, "REVIEW"),
        ("missing-control", 2, "REVIEW"),
        ("missing-footer", 2, "REVIEW"),
        ("short-record", 1, "INVALID_INPUT"),
        ("extra-record", 1, "INVALID_INPUT"),
        ("bad-amount", 1, "INVALID_INPUT"),
        ("unterminated-quote", 1, "INVALID_INPUT"),
        ("after-footer", 1, "INVALID_INPUT"),
    ];
    for (case, (name, code, status)) in cases.iter().zip(expected) {
        assert!(case.at("case").is_str(name));
        same(case.at("expected_exit"), &Json::int(code));
        same(case.at("actual_exit"), &Json::int(code));
        assert!(case.at("response").at("summary_status").is_str(status));
        if code != 1 {
            assert!(case.at("response").at("debtor_decisions").is_str("REVIEW"));
        }
    }
    assert!(replay::pack("unknown", Path::new(VERIFIER)).is_err());
    let clean = replay::pack("clean", Path::new(VERIFIER)).unwrap();
    assert_eq!(array(&clean, "cases").len(), 1);
}

#[test]
fn source_draft_cannot_confirm_independent_evidence() {
    let draft = evidence::draft(SOURCE).unwrap();
    assert!(draft.at("entity").is_str("Synthetic Entity A"));
    assert!(draft.at("as_at").is_str("2026-09-30"));
    assert!(draft.at("source_format").is_str("CSV"));
    assert!(draft.at("source_sha256").is_str(&digest(SOURCE)));
    for key in [
        "generated_at",
        "accounting_basis",
        "currency",
        "filters",
        "population",
        "on_screen_total",
    ] {
        assert!(draft.at(key).is_str(""));
    }
    assert!(matches!(draft.at("footer_record"), Json::Null));
    assert!(matches!(draft.at("on_screen_rows"), Json::Null));
    let result = inspect_export(SOURCE, &draft, None).unwrap();
    status(&result, "REVIEW");
    assert!(!array(&result, "exceptions").is_empty());
}

#[test]
fn requests_preserve_duplicate_rows_roles_and_untrusted_text() {
    let mut result = review(SOURCE);
    let requests = evidence::requests(&result);
    let requests = requests.as_array().unwrap();
    assert_eq!(requests.len(), 3);
    for ((request, contact), number) in requests
        .iter()
        .zip(["00123", "00123", "Total"])
        .zip([7, 8, 9])
    {
        same(
            request.at("source_records"),
            &Json::Array(vec![Json::int(number)]),
        );
        assert!(request.at("contact").is_str(contact));
        assert!(request.at("owner").is_str("Authorised debtor reviewer"));
        assert!(request.at("status").is_str("REVIEW"));
    }
    let mut rows = array(&result, "rows").to_vec();
    rows[0].set("Contact", Json::str("=bad|\n\u{1b}[31m"));
    result.set("rows", Json::Array(rows));
    let briefing = evidence::briefing(&result);
    assert!(briefing.contains("\\n\\u001b[31m"));
    assert!(!briefing.contains('\u{1b}'));
    assert!(briefing.contains("Summary: PASS. Debtor decisions: REVIEW.\n"));
    assert!(result.at("debtor_decisions").is_str("REVIEW"));
}

#[test]
fn new_external_output_and_existing_output_refused() {
    let temp = Temp::new();
    let source = temp.write("source.csv", SOURCE);
    let output = temp.root.join("draft");
    assert_eq!(
        output_cli(EVIDENCE, &source, &output).status.code(),
        Some(2)
    );
    let draft = output.join("manifest-draft.json");
    let bytes = fs::read(&draft).unwrap();
    assert_eq!(
        bytes,
        terminal(&(json::pretty_ordered(&evidence::draft(SOURCE).unwrap()) + "\n"))
    );
    refused(
        &output_cli(EVIDENCE, &source, &output),
        1,
        "Output already exists",
    );
    assert_eq!(fs::read(draft).unwrap(), bytes);
    assert_eq!(fs::read_dir(&output).unwrap().count(), 1);
}

#[test]
fn signed_amounts_keep_four_places_zero_sign_and_unicode_decimal_digits() {
    for (input, expected) in [
        ("", "0"),
        (" \t\u{a0}", "0"),
        ("-0", "-0"),
        ("-0.0000", "-0.0000"),
        ("+0001.2300", "1.2300"),
        (" \u{a0}-1,234.5678\t", "-1234.5678"),
        (
            "\u{661},\u{662}\u{663}\u{664}.\u{665}\u{660}\u{660}\u{660}",
            "1234.5000",
        ),
        ("-\u{ff11}\u{ff12}.\u{ff13}\u{ff14}", "-12.34"),
        ("\u{1d7d9}.\u{1d7da}", "1.2"),
    ] {
        assert_eq!(
            amount::parse(input).unwrap().to_string(),
            expected,
            "{input:?}"
        );
    }
    let negative = amount::parse("-0.0000").unwrap();
    let positive = amount::parse("0.00").unwrap();
    assert_eq!(negative.add(&negative).to_string(), "-0.0000");
    assert_eq!(negative.add(&positive).to_string(), "0.0000");
    assert_eq!(negative.sub(&positive).to_string(), "-0.0000");
    assert_eq!(
        amount::parse("999999999999999.9999")
            .unwrap()
            .add(&amount::parse("-999999999999999.9998").unwrap())
            .to_string(),
        "0.0001"
    );
    for (input, agrees) in [
        ("0.0049", true),
        ("-0.0049", true),
        ("0.005", false),
        ("-0.005", false),
    ] {
        assert_eq!(amount::parse(input).unwrap().agrees(), agrees);
    }
}

#[test]
fn malformed_signed_amounts_are_refused() {
    for input in [
        "-",
        "+",
        ".1",
        "1.",
        "1e3",
        "NaN",
        "Infinity",
        "1.00001",
        "1,00",
        "1234,567",
        "1 000",
        "(25)",
        "1000000000000000",
        "\u{b2}",
        "1\u{66b}00",
    ] {
        assert!(amount::parse(input).is_err(), "accepted {input:?}");
    }
}

#[test]
fn strict_csv_preserves_logical_records_crlf_quotes_bom_and_field_limits() {
    let rows =
        records(b"\xef\xbb\xbfa,b\r\r\"line\rcontinued\",\"say \"\"yes\"\"\"\r\nlast,\n").unwrap();
    assert_eq!(
        rows,
        vec![
            vec!["a", "b"],
            vec![],
            vec!["line\rcontinued", "say \"yes\""],
            vec!["last", ""]
        ]
    );
    assert_eq!(
        records(b"\"line\r\ncontinued\",tail\r\n").unwrap(),
        vec![vec!["line\r\ncontinued", "tail"]]
    );
    assert_eq!(
        records(b"\"unfinished").unwrap_err(),
        "unexpected end of data"
    );
    assert_eq!(records(b"\"a\"x,b").unwrap_err(), "',' expected after '\"'");
    assert!(records(b"\xff").is_err());
    assert_eq!(records(b"").unwrap().len(), 0);
    let allowed = "\u{e9}".repeat(131_072);
    assert_eq!(records(allowed.as_bytes()).unwrap()[0][0], allowed);
    assert_eq!(
        records(format!("{allowed}x").as_bytes()).unwrap_err(),
        "field larger than field limit (131072)"
    );
    let baseline = review(SOURCE);
    let lf = std::str::from_utf8(SOURCE).unwrap().replace("\r\n", "\n");
    for source in [
        lf.clone().into_bytes(),
        lf.replace('\n', "\r").into_bytes(),
        lf.replace('\n', "\r\n").into_bytes(),
    ] {
        let result = review(&source);
        status(&result, "PASS");
        same(result.at("rows"), baseline.at("rows"));
        assert!(result.at("source_sha256").is_str(&digest(&source)));
    }
}

#[test]
fn timestamps_require_timezone_valid_calendar_and_boundary_time() {
    for timestamp in [
        "2026-10-01T09:00:00+10:00",
        "20261001T12Z",
        "2026-W40-4T24:00Z",
        "2026-10-01T12:34:56.123456789-00:30:12.1",
        "0001-01-01T00Z",
        "9999-12-31T23:59:59.999999Z",
        "2000-02-29T00:00+23:59",
        "2026-10-01T24:00:00.000000Z",
    ] {
        assert!(date::generated(timestamp), "{timestamp}");
        status(
            &review_with(SOURCE, &[("generated_at", Json::str(timestamp))]),
            "PASS",
        );
    }
    for timestamp in [
        "2026-10-01",
        "2026-10-01T09:00:00",
        "2026-02-29T12Z",
        "1900-02-29T12Z",
        "2026-10-01T12+24:00",
        "9999-12-31T24Z",
        "0000-01-01T00Z",
        "2026-10-01T24:00:00.000001Z",
        "2026-10-01T23:60Z",
        "2026-10-01T23:59:60Z",
        "2026-10-01T12Zextra",
    ] {
        assert!(!date::generated(timestamp), "{timestamp}");
        let result = review_with(SOURCE, &[("generated_at", Json::str(timestamp))]);
        status(&result, "REVIEW");
        assert!(has_exception(&result, "timestamp"));
    }
    for cutoff in ["0001-01-01", "9999-12-31", "2000-02-29", "2026-09-30"] {
        assert!(date::cutoff(cutoff));
    }
    for cutoff in [
        "0000-01-01",
        "10000-01-01",
        "1900-02-29",
        "2026-09-31",
        "2026-9-30",
        "2026-09-30T00Z",
    ] {
        assert!(!date::cutoff(cutoff));
    }
}

#[test]
fn footer_identity_is_logical_and_percentage_rows_only_follow_the_footer() {
    let mut rows = records(SOURCE).unwrap();
    rows[6][0] = "First\nbranch".into();
    let result = review(&csv(&rows));
    status(&result, "PASS");
    same(array(&result, "rows")[0].at("source_record"), &Json::int(7));
    assert_eq!(
        fields(&result, "rows", "Contact"),
        ["First\nbranch", "00123", "Total"]
    );
    rows.push(
        ["Percentage of total", "unparsed", "", "", "", "", "", ""]
            .map(str::to_owned)
            .to_vec(),
    );
    rows.push(Vec::new());
    status(&review(&csv(&rows)), "PASS");
    rows[10][0] = "Unexplained".into();
    let source = csv(&rows);
    assert!(
        inspect_export(&source, &manifest(&source), Some(&control()))
            .unwrap_err()
            .contains("Unexpected record after")
    );
    rows[10][0] = "Percentage of total".into();
    rows[10].pop();
    let source = csv(&rows);
    assert!(
        inspect_export(&source, &manifest(&source), Some(&control()))
            .unwrap_err()
            .contains("field count")
    );
    let mut manifest = manifest(SOURCE);
    manifest.set("footer_record", Json::int(8));
    assert!(
        inspect_export(SOURCE, &manifest, Some(&control()))
            .unwrap_err()
            .contains("Total label")
    );
    manifest.set("footer_record", Json::int(9));
    assert!(
        inspect_export(SOURCE, &manifest, Some(&control()))
            .unwrap_err()
            .contains("Unexpected record after")
    );
}

#[test]
fn header_order_customer_alias_and_draft_validation_boundary_are_preserved() {
    let mut rows = records(SOURCE).unwrap();
    for row in &mut rows[5..] {
        let old = row.clone();
        *row = [0, 7, 6, 5, 4, 3, 2, 1]
            .map(|index| old[index].clone())
            .to_vec();
    }
    rows[5][0] = "Customer".into();
    let result = review(&csv(&rows));
    status(&result, "PASS");
    all_agree(&result);
    assert_eq!(
        fields(&result, "rows", "Contact"),
        ["00123", "00123", "Total"]
    );
    assert_eq!(
        array(&result, "rows")[0]
            .keys()
            .unwrap()
            .iter()
            .map(|k| k.to_utf8().unwrap())
            .collect::<Vec<_>>(),
        [
            "source_record",
            "Contact",
            "Older",
            "3 Months",
            "2 Months",
            "1 Month",
            "< 1 Month",
            "Current",
            "Total"
        ]
    );
    let source = changed(HEADER, &format!("{HEADER},Extra"));
    assert!(evidence::draft(&source).is_ok());
    assert!(inspect_export(&source, &manifest(&source), Some(&control())).is_err());
    let source = changed(HEADER, &format!("{HEADER},Older"));
    assert!(evidence::draft(&source).is_ok());
    assert!(inspect_export(&source, &manifest(&source), Some(&control())).is_err());
    let source = [SOURCE, format!("{HEADER}\n").as_bytes()].concat();
    assert!(evidence::draft(&source).is_err());
}

#[test]
fn draft_with_header_at_logical_record_zero_has_no_preheader_metadata() {
    let source = format!("{HEADER}\r\n00123,100,,0,0,0,0,100\r\n");
    let draft = evidence::draft(source.as_bytes()).unwrap();
    assert!(draft.at("report_name").is_str("Contact"));
    for key in ["entity", "as_at", "ageing_basis"] {
        assert!(draft.at(key).is_str(""), "{key}");
    }
    assert!(draft.at("source_sha256").is_str(&digest(source.as_bytes())));
    assert!(matches!(draft.at("footer_record"), Json::Null));
    assert!(matches!(draft.at("on_screen_rows"), Json::Null));
}

#[test]
fn json_metadata_preserves_type_order_large_integers_and_surrogate_escapes() {
    let mut manifest = manifest(SOURCE);
    let extra = json::loads(r#"{"z":-0.0,"a":[true,3.0,999999999999999999999999999999999999999999999999999999999999,"\ud800",NaN,Infinity,-Infinity],"text":"\u00e9"}"#).unwrap();
    manifest.set("operator_metadata", extra);
    let mut control = control();
    control.set(
        "operator_metadata",
        json::loads(r#"{"notes":[null,false,1e100],"literal":"\udfff"}"#).unwrap(),
    );
    let manifest_bytes = json::dumps(&manifest, false);
    let control_bytes = json::dumps(&control, false);
    let result = inspect_export(SOURCE, &manifest, Some(&control)).unwrap();
    status(&result, "PASS");
    assert_eq!(json::dumps(result.at("manifest"), false), manifest_bytes);
    assert_eq!(json::dumps(result.at("control"), false), control_bytes);
    assert_eq!(json::dumps(&manifest, false), manifest_bytes);
    assert_eq!(json::dumps(&control, false), control_bytes);
    assert!(json::pretty_ordered(&result).contains("\\ud800"));
}

#[test]
fn evidence_review_writes_exact_files_and_preserves_sources_for_pass_and_review() {
    let temp = Temp::new();
    let source = changed(
        "00123,100,,0,0,0,0,100",
        "\"001, Caf\u{e9}\nBranch\",100,,0,0,0,0,100",
    );
    let source_path = temp.write("source.csv", &source);
    let manifest = manifest(&source);
    let control = control();
    let manifest_bytes = json::dumps(&manifest, false).into_bytes();
    let control_bytes = json::dumps(&control, false).into_bytes();
    let manifest_path = temp.write("manifest.json", &manifest_bytes);
    let control_path = temp.write("control.json", &control_bytes);
    for (name, with_control, expected_code, expected_status) in
        [("pass", true, 0, "PASS"), ("review", false, 2, "REVIEW")]
    {
        let output_dir = temp.root.join(name);
        let mut command = Command::new(EVIDENCE);
        command
            .arg("review")
            .arg(&source_path)
            .arg("--manifest")
            .arg(&manifest_path)
            .arg("--output")
            .arg(&output_dir);
        if with_control {
            command.arg("--control").arg(&control_path);
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(expected_code), "{output:?}");
        assert!(output.stderr.is_empty());
        assert_eq!(
            output.stdout,
            terminal(&format!(
                "Summary: {expected_status}; debtor decisions: REVIEW.\n"
            ))
        );
        let result = inspect_export(
            &source,
            &manifest,
            if with_control { Some(&control) } else { None },
        )
        .unwrap();
        status(&result, expected_status);
        review_files(&output_dir, &result);
    }
    assert_eq!(fs::read(source_path).unwrap(), source);
    assert_eq!(fs::read(manifest_path).unwrap(), manifest_bytes);
    assert_eq!(fs::read(control_path).unwrap(), control_bytes);
}

#[test]
fn verifier_emits_literal_unicode_and_invalid_input_never_creates_evidence() {
    let temp = Temp::new();
    let source = changed(
        "00123,100,,0,0,0,0,100",
        "Caf\u{e9} \u{1f680},100,,0,0,0,0,100",
    );
    let output = verifier(&temp, &source, &manifest(&source), Some(&control()));
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let body = String::from_utf8(output.stdout).unwrap();
    assert!(body.contains("Caf\u{e9} \u{1f680}"));
    assert!(!body.contains("\\ud83d"));
    assert!(
        json::loads(&body)
            .unwrap()
            .at("summary_status")
            .is_str("PASS")
    );
    let source_path = temp.write("source.csv", b"\xff");
    let output_path = temp.root.join("missing-parent/evidence");
    let output = output_cli(EVIDENCE, &source_path, &output_path);
    refused(&output, 1, "INVALID_INPUT");
    assert!(!output_path.parent().unwrap().exists());
    assert_eq!(fs::read(source_path).unwrap(), b"\xff");
}

#[test]
fn evidence_draft_refusals_do_not_create_parents_or_overwrite_source() {
    let temp = Temp::new();
    let source = temp.write("source.csv", SOURCE);
    let manifest = temp.write("manifest.json", MANIFEST.as_bytes());
    let output_path = temp.root.join("missing-parent/draft");
    let output = Command::new(EVIDENCE)
        .arg("draft")
        .arg(&source)
        .arg("--manifest")
        .arg(&manifest)
        .arg("--output")
        .arg(&output_path)
        .output()
        .unwrap();
    refused(&output, 1, "source declarations only");
    assert!(!output_path.parent().unwrap().exists());
    for tool in [EVIDENCE, REPLAY] {
        let output = output_cli(tool, &source, &source);
        refused(
            &output,
            if tool == REPLAY { 2 } else { 1 },
            "Output already exists",
        );
        assert_eq!(fs::read(&source).unwrap(), SOURCE);
    }
    let output = Command::new(EVIDENCE)
        .arg("review")
        .arg(&source)
        .arg("--output")
        .arg(&output_path)
        .output()
        .unwrap();
    refused(&output, 1, "requires a manifest");
    assert!(!output_path.parent().unwrap().exists());
}

#[test]
fn evidence_requests_keep_preparer_roles_and_specific_remedies() {
    let mut result = review(SOURCE);
    result.set(
        "exceptions",
        Json::Array(
            [
                "Source record 7: difference and record 9",
                "Receivables control evidence is missing.",
                "On-screen report total was not checked.",
                "Terminal export footer has not been identified.",
            ]
            .map(Json::str)
            .to_vec(),
        ),
    );
    let requests = evidence::requests(&result);
    let requests = requests.as_array().unwrap();
    assert_eq!(requests.len(), 7);
    same(
        requests[0].at("source_records"),
        &Json::Array(vec![Json::int(7), Json::int(9)]),
    );
    for request in &requests[..4] {
        assert!(request.at("owner").is_str("Export preparer"));
        assert!(request.at("status").is_str("UNRESOLVED"));
    }
    for (request, expected) in requests[..4].iter().zip([
        "Recheck the identified source records",
        "Obtain a matching dated receivables control",
        "Check the report on screen",
        "Confirm the terminal footer's logical source record",
    ]) {
        assert!(text(request.at("next_action")).starts_with(expected));
    }
}

#[test]
fn command_help_usage_missing_files_and_replay_selection_use_the_right_streams() {
    for tool in [VERIFIER, EVIDENCE, REPLAY] {
        let help = Command::new(tool).arg("--help").output().unwrap();
        assert_eq!(help.status.code(), Some(0));
        assert!(help.stderr.is_empty());
        assert!(help.stdout.starts_with(b"usage:"));
        let usage = Command::new(tool).output().unwrap();
        refused(&usage, 2, "usage:");
    }
    let temp = Temp::new();
    let missing = temp.root.join("missing.csv");
    let manifest = temp.write("manifest.json", MANIFEST.as_bytes());
    let result = Command::new(VERIFIER)
        .arg(missing)
        .arg("--manifest")
        .arg(manifest)
        .output()
        .unwrap();
    refused(&result, 1, "INVALID_INPUT");
    let output_path = temp.root.join("not-created/replay.json");
    let unknown = Command::new(REPLAY)
        .args(["--case", "unknown", "--output"])
        .arg(&output_path)
        .output()
        .unwrap();
    refused(&unknown, 2, "unknown replay case");
    assert!(!output_path.parent().unwrap().exists());
    let output = Command::new(REPLAY)
        .args(["--case", "clean", "--output"])
        .arg(&output_path)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, terminal("FAILURE REPLAY QUALIFIED\n"));
    let expected = replay::pack("clean", Path::new(VERIFIER)).unwrap();
    assert_eq!(
        fs::read(output_path).unwrap(),
        terminal(&(json::pretty_ordered(&expected) + "\n"))
    );
}

#[cfg(unix)]
fn directory_link(target: &Path, path: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

#[cfg(windows)]
fn directory_link(target: &Path, path: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, path)
}

#[cfg(any(unix, windows))]
#[test]
fn output_containment_resolves_directory_links_before_creation() {
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let external = Temp::new();
    let source = external.write("source.csv", SOURCE);
    let checkout_alias = external.root.join("checkout-alias");
    if let Err(error) = directory_link(checkout, &checkout_alias) {
        if cfg!(windows) && error.kind() == std::io::ErrorKind::PermissionDenied {
            eprintln!(
                "directory-link containment check skipped: Windows symlink privilege is unavailable"
            );
            return;
        }
        panic!("cannot create test directory link: {error}");
    }
    let name = format!(
        "fabricated-aged-alias-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    );
    for (tool, suffix, code) in [(EVIDENCE, "evidence", 1), (REPLAY, "replay", 2)] {
        let output = checkout_alias.join(&name).join(suffix);
        refused(
            &output_cli(tool, &source, &output),
            code,
            "outside the checkout",
        );
        assert!(!checkout.join(&name).exists());
    }
    fs::remove_dir(&checkout_alias)
        .or_else(|_| fs::remove_file(&checkout_alias))
        .unwrap();
    let within = Temp::inside(checkout);
    let external_alias = within.root.join("external-alias");
    directory_link(&external.root, &external_alias).unwrap();
    let output = external_alias.join("resolved-external");
    let result = output_cli(EVIDENCE, &source, &output);
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    assert!(result.stderr.is_empty());
    assert!(
        external
            .root
            .join("resolved-external/manifest-draft.json")
            .is_file()
    );
    fs::remove_dir(&external_alias)
        .or_else(|_| fs::remove_file(&external_alias))
        .unwrap();
}

#[test]
fn aged_test_source_remains_ascii() {
    assert!(
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/aged_review.rs"))
            .unwrap()
            .is_ascii()
    );
}
