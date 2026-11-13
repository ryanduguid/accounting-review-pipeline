// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Stream contracts for the optional migration probe, not the production CLI.
#![cfg(feature = "probe")]

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;
#[path = "support/temp.rs"]
mod temp;

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
