// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use super::{Child, TempDir, child_profile};
use std::{
    ffi::OsString,
    fs,
    io::Read,
    process::Stdio,
    time::{Duration, Instant},
};

fn refusal(tmp: &TempDir, args: &[OsString], expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut child = Child(
        child_profile::command(env!("CARGO_BIN_EXE_review-ready-differential"))
            .args(args)
            .current_dir(&tmp.0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "differential refusal timed out");
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = Vec::new();
    let mut stderr = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .unwrap();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert_eq!(status.code(), Some(1), "{stderr}");
    assert!(stdout.is_empty(), "{stdout:?}");
    assert_eq!(stderr.replace("\r\n", "\n"), expected);
}

#[test]
fn differential_refuses_wrong_argument_counts_without_creating_files() {
    let tmp = TempDir::new();
    for args in [
        vec![],
        vec!["absent-python".into()],
        vec!["unused".into(); 4],
    ] {
        refusal(
            &tmp,
            &args,
            "Error: \"usage: review-ready-differential CPYTHON ORACLE_COMPONENT [SEED]\"\n",
        );
        assert!(fs::read_dir(&tmp.0).unwrap().next().is_none());
    }
}

#[test]
fn differential_refuses_missing_oracle_source_before_starting_python() {
    for marker_directory in [false, true] {
        let tmp = TempDir::new();
        let marker = tmp.join("reviewready/cli.py");
        if marker_directory {
            fs::create_dir_all(&marker).unwrap();
        }
        refusal(
            &tmp,
            &[
                tmp.join("absent-python").into_os_string(),
                tmp.0.clone().into_os_string(),
            ],
            "Error: \"oracle component source is missing\"\n",
        );
        let entries: Vec<_> = fs::read_dir(&tmp.0)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        if marker_directory {
            assert_eq!(entries, [tmp.join("reviewready")]);
            let children: Vec<_> = fs::read_dir(tmp.join("reviewready"))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            assert_eq!(children.as_slice(), std::slice::from_ref(&marker));
            assert!(marker.is_dir());
            assert!(fs::read_dir(&marker).unwrap().next().is_none());
        } else {
            assert!(entries.is_empty());
        }
    }
}

#[test]
fn differential_refuses_invalid_seed_before_admitting_oracle_source() {
    let tmp = TempDir::new();
    refusal(
        &tmp,
        &[
            tmp.join("absent-python").into_os_string(),
            tmp.0.clone().into_os_string(),
            "not-a-seed".into(),
        ],
        "Error: ParseIntError { kind: InvalidDigit }\n",
    );
    assert!(fs::read_dir(&tmp.0).unwrap().next().is_none());
}
