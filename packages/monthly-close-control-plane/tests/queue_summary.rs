// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! `queue --summary`: one screen of states, exceptions by control and
//! controls not run across several verified packs.

#[path = "support/temp.rs"]
mod temp;

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
};
use temp::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn trial_balance(temp: &TempDir, name: &str, date: &str, capital: &str) -> PathBuf {
    let path = temp.join(name);
    let csv = format!(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n\
         {date},Fabricated Varrock,Assets,A1,Bank,090,0.00,0.00,100.00,0.00\n\
         {date},Fabricated Varrock,Equity,E1,Capital,300,0.00,0.00,0.00,{capital}\n"
    );
    fs::write(&path, csv).unwrap();
    path
}

/// A pack in `client`, balanced unless `capital` says otherwise.
fn review(temp: &TempDir, client: &str, capital: &str, extra: &[&str]) -> PathBuf {
    let output = temp.join(client);
    let result = Command::new(BIN)
        .arg("review")
        .arg("--current")
        .arg(trial_balance(
            temp,
            &format!("{client}-current.csv"),
            "2026-08-31",
            capital,
        ))
        .arg("--prior")
        .arg(trial_balance(
            temp,
            &format!("{client}-prior.csv"),
            "2026-07-31",
            capital,
        ))
        .args(extra)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(output.join("close-review-pack.json").exists(), "{result:?}");
    output
}

/// 3 clients: one `PASS`, one `REVIEW` and one `BLOCKED`.
fn book(temp: &TempDir) -> Vec<PathBuf> {
    vec![
        review(temp, "client-a", "100.00", &[]),
        review(
            temp,
            "client-b",
            "100.00",
            &["--require-control", "account_mapping"],
        ),
        review(temp, "client-c", "90.00", &[]),
    ]
}

#[test]
fn queue_keeps_the_short_status_option() {
    let temp = TempDir::new();
    let packs = book(&temp);
    let full = queue(&packs, &["--status", "PASS"]);
    assert!(full.status.success(), "{full:?}");
    for args in [&["--s", "PASS"][..], &["--s=PASS"][..]] {
        let short = queue(&packs, args);
        assert_eq!(short.status.code(), full.status.code(), "{short:?}");
        assert_eq!(short.stdout, full.stdout);
        assert_eq!(short.stderr, full.stderr);
    }
}

fn queue(packs: &[PathBuf], extra: &[&str]) -> Output {
    let mut command = Command::new(BIN);
    command.arg("queue");
    for pack in packs {
        command.arg("--pack-dir").arg(pack);
    }
    command.args(extra).output().unwrap()
}

/// The lines from the summary heading up to the artefact digests.
fn summary(result: &Output) -> String {
    let stdout = String::from_utf8(result.stdout.clone()).unwrap();
    let start = stdout
        .find("Summary of the")
        .unwrap_or_else(|| panic!("no summary: {result:?}"));
    let end = stdout.find("\n\nVerified artefact SHA-256").unwrap();
    stdout[start..end].to_owned()
}

#[test]
fn the_summary_counts_each_state_control_and_omitted_control_across_the_book() {
    let temp = TempDir::new();
    let packs = book(&temp);
    let result = queue(&packs, &["--summary"]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert!(result.stderr.is_empty(), "{result:?}");
    let all = "client-a, client-b, client-c";
    assert_eq!(
        summary(&result),
        format!(
            "Summary of the 3 displayed pack(s)

Clients are pack directory names; the pack does not record a client.

| State | Packs | Clients |
| --- | --- | --- |
| PASS | 1 | client-a |
| REVIEW | 1 | client-b |
| BLOCKED | 1 | client-c |

| Control | Packs with exceptions | Exceptions |
| --- | --- | --- |
| required_control | 1 | 1 |
| trial_balance_integrity | 1 | 2 |

| Control not run | Packs | Clients |
| --- | --- | --- |
| account_mapping | 3 | {all} |
| balance_policy | 3 | {all} |
| calculation_evidence | 3 | {all} |
| subledger | 3 | {all} |"
        )
    );
}

#[test]
fn the_summary_counts_only_the_displayed_packs() {
    let temp = TempDir::new();
    let result = queue(&book(&temp), &["--summary", "--status", "PASS"]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let summary = summary(&result);
    assert!(
        summary.starts_with("Summary of the 1 displayed pack(s)"),
        "{summary}"
    );
    assert!(summary.contains("| REVIEW | 0 | none |"), "{summary}");
    assert!(summary.contains("| BLOCKED | 0 | none |"), "{summary}");
    assert!(
        summary.contains("No displayed pack records an exception."),
        "{summary}"
    );
    assert!(
        summary.contains("| subledger | 1 | client-a |"),
        "{summary}"
    );
}

#[test]
fn without_the_flag_the_queue_has_no_summary() {
    let temp = TempDir::new();
    let result = queue(&book(&temp), &[]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert!(!String::from_utf8_lossy(&result.stdout).contains("Summary of the"));
}

#[test]
fn an_edited_pack_fails_the_summary_even_when_a_filter_would_hide_it() {
    let temp = TempDir::new();
    let packs = book(&temp);
    fs::write(packs[1].join("close-summary.md"), "edited\n").unwrap();
    let result = queue(&packs, &["--summary", "--status", "PASS"]);
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert!(result.stdout.is_empty(), "{result:?}");
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("verification failed"),
        "{result:?}"
    );
}
