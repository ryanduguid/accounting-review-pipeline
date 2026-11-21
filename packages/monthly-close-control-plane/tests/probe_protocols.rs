// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

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
    path::Path,
    process::{Child, Command, Output, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

// Keep another child from inheriting the reader before the disconnect fixture closes it.
// Release the lock before request I/O or child waits.
static PROBE_SPAWN: Mutex<()> = Mutex::new(());

const MAPPING: &str = r#"{"op":"csv","kind":"mapping","path":"mapping.csv","hex":"4163636f756e7449442c52657669657747726f75700a41312c436173680a"}"#;
const MAPPED: &str = r#"{"ok":{"A1":"Cash"}}"#;
const REFUSALS: &[(&str, &str)] = &[
    (
        r#"{"op":"format_money","value":"1E+999999999999999999"}"#,
        "decimal fixed-point output exceeds available memory",
    ),
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
        r#"{"op":"arithmetic","precision":"28","a":"1","b":"0","kind":"unknown"}"#,
        "invalid probe arithmetic",
    ),
    (
        r#"{"op":"arithmetic","precision":"28","a":"1","b":"0","kind":"div"}"#,
        "[<class 'decimal.DivisionByZero'>]",
    ),
    (
        r#"{"op":"arithmetic","precision":"28","a":"0","b":"-0.00","kind":"div"}"#,
        "[<class 'decimal.DivisionUndefined'>]",
    ),
];

fn probe_command(scratch: &temp::TempDir, fixtures: &Path) -> Command {
    let mut command = child_profile::command(env!("CARGO_BIN_EXE_close-control-probe"));
    command
        .current_dir(&scratch.0)
        .env("TMPDIR", fixtures)
        .env("TMP", fixtures)
        .env("TEMP", fixtures)
        .env_remove("GIT_WORK_TREE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn wait_probe(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(15);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("probe exceeded its deadline: {output:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

fn run_probe(requests: &[&str]) -> Output {
    let scratch = temp::TempDir::new();
    let fixtures = scratch.join("fixtures");
    fs::create_dir(&fixtures).unwrap();
    let mut child = {
        let _lock = PROBE_SPAWN.lock().unwrap();
        probe_command(&scratch, &fixtures).spawn().unwrap()
    };
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{}\n", requests.join("\n")).as_bytes())
        .unwrap();
    let output = wait_probe(child);
    assert_eq!(fs::read_dir(&fixtures).unwrap().count(), 0);
    fs::remove_dir(&fixtures).unwrap();
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
    output
}

#[test]
fn closed_output_exits_without_waiting_for_input_eof() {
    let scratch = temp::TempDir::new();
    let fixtures = scratch.join("fixtures");
    fs::create_dir(&fixtures).unwrap();
    let mut child = {
        let _lock = PROBE_SPAWN.lock().unwrap();
        let mut child = probe_command(&scratch, &fixtures).spawn().unwrap();
        drop(child.stdout.take().unwrap());
        child
    };
    let mut input = child.stdin.take().unwrap();
    if let Err(error) = writeln!(input, "{MAPPING}").and_then(|()| input.flush()) {
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        panic!("could not write probe request: {error}; {output:?}");
    }
    let output = wait_probe(child);
    drop(input);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(fs::read_dir(&fixtures).unwrap().count(), 0);
    fs::remove_dir(&fixtures).unwrap();
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
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

#[test]
fn checkout_output_refusal_preserves_the_directory_and_next_request() {
    let checkout = temp::TempDir::new();
    fs::create_dir(checkout.join(".git")).unwrap();
    let root = fs::canonicalize(&checkout.0).unwrap();
    let root = root.to_str().unwrap();
    let root = if let Some(tail) = root.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{tail}")
    } else {
        root.strip_prefix(r"\\?\").unwrap_or(root).to_owned()
    };
    let request = pyjson::canonical(&Json::object([
        ("op", Json::str("output_guard")),
        ("path", Json::str(root.as_str())),
    ]));
    let error = format!(
        "{root} is inside the version-control checkout at {root}, which holds .git. A review pack names a client's accounts and balances, so it belongs in an access-controlled directory outside version control. Point --output somewhere outside that checkout."
    );
    let response = pyjson::canonical(&Json::object([("error", Json::str(&error))]));
    assert_probe_responses(&[&request, MAPPING], &[&response, MAPPED]);
    let entries: Vec<_> = fs::read_dir(&checkout.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, [".git"]);
    assert_eq!(fs::read_dir(checkout.join(".git")).unwrap().count(), 0);
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
