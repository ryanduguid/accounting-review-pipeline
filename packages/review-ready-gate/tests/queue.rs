// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! `review-ready queue`: several verified packs on one screen.

#[path = "support/temp.rs"]
mod temp;

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use temp::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_review-ready");

/// The fabricated `bas` examples gated into `temp`: one each of `READY`,
/// `NOT_READY` and `BLOCKED`.
fn book(temp: &TempDir) -> Vec<PathBuf> {
    ["bas-ready", "bas-not-ready", "bas-blocked"]
        .into_iter()
        .map(|example| {
            let output = temp.join(example);
            let result = Command::new(BIN)
                .args(["gate", "--profile", "bas", "--pack"])
                .arg(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("examples")
                        .join(example),
                )
                .arg("--output")
                .arg(&output)
                .env_remove("GIT_WORK_TREE")
                .output()
                .unwrap();
            assert!(output.join("readiness-pack.json").exists(), "{result:?}");
            output
        })
        .collect()
}

fn queue(packs: &[PathBuf], extra: &[&str]) -> Output {
    let mut command = Command::new(BIN);
    command.arg("queue");
    for pack in packs {
        command.arg("--pack-dir").arg(pack);
    }
    command.args(extra).output().unwrap()
}

/// Standard output with `\n` line endings: on Windows the CLI writes `\r\n`.
fn stdout(result: &Output) -> String {
    String::from_utf8(result.stdout.clone())
        .unwrap()
        .replace("\r\n", "\n")
}

/// Everything before the artefact digests, whose paths and hashes vary by run.
fn listing(result: &Output) -> String {
    let text = stdout(result);
    let end = text
        .find("\n\nVerified artefact SHA-256")
        .unwrap_or_else(|| panic!("no digests: {result:?}"));
    text[..end].to_owned()
}

const TABLE: &str = "Review-ready Queue

Verified 3 distinct pack(s); displaying 3.
Recorded statuses are gate evidence. This queue does not approve a file or change a finding, acknowledgement or source file.
Clients are pack directory names; the pack does not record a client.

| Client | Profile | Period end | Status | Findings | Finding codes | Controls not run |
| --- | --- | --- | --- | --- | --- | --- |
| bas-blocked | bas | 2026-03-31 | BLOCKED | 1 | TB_UNBALANCED | prior_findings |
| bas-not-ready | bas | 2026-03-31 | NOT_READY | 3 | MISSING_ARTEFACT, OPEN_ITEM_BLOCKING, SELF_REVIEW_INCOMPLETE | none |
| bas-ready | bas | 2026-03-31 | READY | 0 | none | prior_findings |";

#[test]
fn the_queue_lists_each_status_with_its_finding_codes_and_exits_zero() {
    let temp = TempDir::new();
    let result = queue(&book(&temp), &[]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert!(result.stderr.is_empty(), "{result:?}");
    assert_eq!(listing(&result), TABLE);
    let text = stdout(&result);
    for example in ["bas-ready", "bas-not-ready", "bas-blocked"] {
        assert!(
            text.contains(&format!("/{example}\n  findings.csv: ")),
            "{text}"
        );
    }
    assert!(text.ends_with(
        "\nreview-ready queue: display is a review aid; it does not approve a file or change any computed status.\n"
    ));
}

#[test]
fn the_summary_counts_statuses_codes_and_omitted_controls() {
    let temp = TempDir::new();
    let result = queue(&book(&temp), &["--summary"]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert_eq!(
        listing(&result),
        format!(
            "{TABLE}

Summary of the 3 displayed pack(s)

| Status | Packs | Clients |
| --- | --- | --- |
| READY | 1 | bas-ready |
| NOT_READY | 1 | bas-not-ready |
| BLOCKED | 1 | bas-blocked |

| Finding code | Packs | Findings |
| --- | --- | --- |
| MISSING_ARTEFACT | 1 | 1 |
| OPEN_ITEM_BLOCKING | 1 | 1 |
| SELF_REVIEW_INCOMPLETE | 1 | 1 |
| TB_UNBALANCED | 1 | 1 |

| Control not run | Packs | Clients |
| --- | --- | --- |
| prior_findings | 2 | bas-blocked, bas-ready |"
        )
    );
}

#[test]
fn a_status_filter_narrows_the_list_and_its_summary() {
    let temp = TempDir::new();
    let result = queue(&book(&temp), &["--status", "NOT_READY", "--summary"]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let text = listing(&result);
    assert!(
        text.contains("Verified 3 distinct pack(s); displaying 1."),
        "{text}"
    );
    assert!(text.contains("\n| bas-not-ready | bas |"), "{text}");
    assert!(!text.contains("\n| bas-ready |"), "{text}");
    assert!(text.contains("| READY | 0 | none |"), "{text}");
    assert!(text.contains("| NOT_READY | 1 | bas-not-ready |"), "{text}");
}

#[test]
fn an_edited_pack_fails_the_whole_queue_even_when_a_filter_would_hide_it() {
    let temp = TempDir::new();
    let packs = book(&temp);
    let summary = packs[2].join("readiness-summary.md");
    let edited = fs::read_to_string(&summary)
        .unwrap()
        .replace("BLOCKED", "READY");
    fs::write(&summary, edited).unwrap();
    let result = queue(&packs, &["--status", "READY", "--summary"]);
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert!(result.stdout.is_empty(), "{result:?}");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.starts_with("review-ready queue: verification failed: Pack ")
            && stderr.contains("bas-blocked"),
        "{stderr}"
    );
}

#[test]
fn a_repeated_pack_is_listed_once_and_a_bad_status_is_a_usage_error() {
    let temp = TempDir::new();
    let packs = book(&temp);
    let result = queue(&[packs[0].clone(), packs[0].clone()], &[]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert!(stdout(&result).contains("Verified 1 distinct pack(s); displaying 1."));
    let result = queue(&packs, &["--status", "PASS"]);
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("argument --status: invalid choice"));
    let result = queue(&[], &[]);
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("the following arguments are required: --pack-dir")
    );
}
