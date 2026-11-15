// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Stream contracts for the optional migration probe, not the production CLI.
#![cfg(feature = "probe")]

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;
#[path = "support/temp.rs"]
mod temp;

use monthly_close_control_plane::pyjson::{self, Json};
use std::{
    fs,
    io::Write,
    process::{Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const MAPPING: &str = r#"{"op":"csv","kind":"mapping","path":"mapping.csv","hex":"4163636f756e7449442c52657669657747726f75700a41312c436173680a"}"#;
const MAPPED: &str = r#"{"ok":{"A1":"Cash"}}"#;
const REFUSALS: &[(&str, &str)] = &[
    (
        r#"{"op":"money","value":null}"#,
        "input.csv: row 2 has invalid Amount: ''.",
    ),
    (r#"{"op":"fixture_unknown"}"#, "unsupported probe operation"),
    (
        r#"{"op":"csv","kind":"fixture_unknown","path":"fixture.csv","hex":""}"#,
        "unsupported probe CSV kind",
    ),
    (
        r#"{"op":"csv","kind":"mapping","path":"mapping.csv","hex":"0"}"#,
        "invalid probe hex",
    ),
    (r#"{"op":"verify","payloads":[]}"#, "invalid probe payloads"),
    (
        r#"{"op":"drivers","top":1,"payloads":[]}"#,
        "invalid probe payloads",
    ),
    (
        r#"{"op":"compare","previous":{"trial_balance":{"path":"prior.csv","hex":""},"payloads":[]},"current":{"trial_balance":{"path":"current.csv","hex":""},"payloads":{}}}"#,
        "invalid probe payloads",
    ),
    (
        r#"{"op":"cli","files":{"../escape-fixture.txt":""},"args":["--help"]}"#,
        "probe fixture path must stay inside its temporary directory",
    ),
    (
        r#"{"op":"cli","directories":["nested/fixture","../escape-fixture"],"args":["--help"]}"#,
        "probe fixture path must stay inside its temporary directory",
    ),
    (
        r#"{"op":"arithmetic","precision":"29","a":"1","b":"0","kind":"div"}"#,
        "invalid probe precision",
    ),
    (
        r#"{"op":"arithmetic","precision":"28","a":"1","b":"0","kind":"div"}"#,
        "invalid probe arithmetic",
    ),
];

fn run_probe(requests: &[&str]) -> Output {
    let scratch = temp::TempDir::new();
    let fixtures = scratch.join("fixtures");
    fs::create_dir(&fixtures).unwrap();
    let mut child = child_profile::command(env!("CARGO_BIN_EXE_close-control-probe"))
        .current_dir(&scratch.0)
        .env("TMPDIR", &fixtures)
        .env("TMP", &fixtures)
        .env("TEMP", &fixtures)
        .env_remove("GIT_WORK_TREE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{}\n", requests.join("\n")).as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("probe exceeded its deadline: {output:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(fs::read_dir(&fixtures).unwrap().count(), 0);
    fs::remove_dir(&fixtures).unwrap();
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
    output
}

#[test]
fn request_refusals_leave_the_next_mapping_request_usable() {
    let mut requests = Vec::new();
    let mut expected = Vec::new();
    for &(request, error) in REFUSALS {
        requests.extend([request, MAPPING]);
        expected.extend([format!(r#"{{"error":"{error}"}}"#), MAPPED.into()]);
    }
    let output = run_probe(&requests);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        expected.join("\n") + "\n"
    );
}

#[test]
fn malformed_json_stops_the_stream_before_the_following_request() {
    let output = run_probe(&[MAPPING, "{", MAPPING]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .replace("\r\n", "\n"),
        format!("{MAPPED}\n")
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.starts_with("invalid probe JSON: "), "{stderr}");
}

fn assert_probe_responses(requests: &[&str], expected: &[&str]) {
    let output = run_probe(requests);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), expected.len(), "{stdout}");
    for (actual, expected) in lines.into_iter().zip(expected) {
        assert!(
            pyjson::py_eq(
                &pyjson::loads(actual).unwrap(),
                &pyjson::loads(expected).unwrap(),
                true,
            ),
            "actual: {actual}\nexpected: {expected}",
        );
    }
}

#[test]
fn successful_money_and_directory_requests_leave_mapping_usable() {
    assert_probe_responses(
        &[
            r#"{"op":"format_money","value":"12"}"#,
            r#"{"op":"format_money","value":"-12.345"}"#,
            r#"{"op":"cli","directories":["nested/fixture"],"args":["--version"]}"#,
            MAPPING,
        ],
        &[
            r#"{"ok":"12.00"}"#,
            r#"{"ok":"-12.345"}"#,
            r#"{"ok":{"code":0,"stdout":"close-control 1.1.0\n","stderr":"","files":{}}}"#,
            MAPPED,
        ],
    );
}

#[test]
fn external_output_guard_returns_the_resolved_directory() {
    let external = temp::TempDir::new();
    let request = pyjson::canonical(&Json::object([
        ("op", Json::str("output_guard")),
        ("path", Json::str(external.0.to_str().unwrap())),
    ]));
    let output = run_probe(&[&request]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let response = pyjson::loads(stdout.trim()).unwrap();
    assert!(response.has_exact_keys(&["ok"]), "{stdout}");
    let path = response.at("ok").as_pystr().unwrap().to_utf8().unwrap();
    assert!(std::path::Path::new(&path).is_absolute(), "{path}");
    assert_eq!(
        fs::canonicalize(&path).unwrap(),
        fs::canonicalize(&external.0).unwrap(),
    );
    assert_eq!(fs::read_dir(&external.0).unwrap().count(), 0);
}

const INTEGRITY_CSV: &[u8] = b"ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n2026-07-31,Fixture,Assets,A1,Cash,100,3,1,8,2\n";
const INTEGRITY_RESPONSE: &str = r#"{"ok":{
    "rows":[{"report_date":"2026-07-31","tenant":"Fixture","section":"Assets",
        "account_id":"A1","account_name":"Cash","account_code":"100",
        "debit":"3","credit":"1","ytd_debit":"8","ytd_credit":"2"}],
    "findings":[
        {"control":"trial_balance_integrity","status":"BLOCKED","tenant":"",
         "account_id":"","account_code":"","account_name":"","review_group":"",
         "current_value":"3","prior_value":"1","difference":"2","threshold":"0",
         "percentage_change":null,
         "reason":"current movement debit and credit totals do not balance.",
         "reviewer_action":"Correct or re-export the source trial balance before relying on any review result."},
        {"control":"trial_balance_integrity","status":"BLOCKED","tenant":"",
         "account_id":"","account_code":"","account_name":"","review_group":"",
         "current_value":"8","prior_value":"2","difference":"6","threshold":"0",
         "percentage_change":null,
         "reason":"current YTD debit and credit totals do not balance.",
         "reviewer_action":"Correct or re-export the source trial balance before relying on any review result."}
    ]
}}"#;

#[test]
fn trial_balance_integrity_preserves_rows_and_reports_both_imbalances() {
    let hex: String = INTEGRITY_CSV
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let request = format!(r#"{{"op":"tb_integrity","path":"tb.csv","hex":"{hex}"}}"#);
    assert_probe_responses(&[&request, MAPPING], &[INTEGRITY_RESPONSE, MAPPED]);
}
