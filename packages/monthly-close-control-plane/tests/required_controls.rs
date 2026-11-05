// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! `--require-control`: a named control that did not run keeps the pack from
//! `PASS`, and a script reading the exit code sees it.

#[path = "support/temp.rs"]
mod temp;

use monthly_close_control_plane::{pyjson, viewer};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use temp::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_close-control");
const ALWAYS_RUN: [&str; 5] = [
    "trial_balance_integrity",
    "financial_year_reset",
    "period_comparison",
    "period_variance",
    "account_metadata",
];
/// Each control whose input can be left out. `mapping_compatibility`,
/// `gst_reasonableness` and `equity_reconciliation` never reach
/// `controls_not_run` in this state, so that list cannot be what is checked.
const OPTIONAL: [&str; 7] = [
    "account_mapping",
    "mapping_compatibility",
    "balance_policy",
    "subledger_reconciliation",
    "gst_reasonableness",
    "calculation_evidence",
    "equity_reconciliation",
];

fn trial_balance(temp: &TempDir, name: &str, date: &str) -> PathBuf {
    let path = temp.join(name);
    let csv = format!(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n\
         {date},Fabricated Varrock,Assets,A1,Bank,090,0.00,0.00,100.00,0.00\n\
         {date},Fabricated Varrock,Equity,E1,Capital,300,0.00,0.00,0.00,100.00\n"
    );
    fs::write(&path, csv).unwrap();
    path
}

/// A close on which every control that runs passes.
fn review(temp: &TempDir, output: &str, extra: &[&str]) -> (Output, PathBuf) {
    let output = temp.join(output);
    let result = Command::new(BIN)
        .arg("review")
        .arg("--current")
        .arg(trial_balance(temp, "current.csv", "2026-08-31"))
        .arg("--prior")
        .arg(trial_balance(temp, "prior.csv", "2026-07-31"))
        .args(extra)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    (result, output)
}

fn pack(output: &Path) -> pyjson::Json {
    pyjson::loads(&fs::read_to_string(output.join("close-review-pack.json")).unwrap()).unwrap()
}

fn state_of(document: &pyjson::Json, name: &str) -> String {
    let runs = document.at("required_controls").as_array().unwrap();
    let run = runs.iter().find(|run| run.at("name").is_str(name)).unwrap();
    run.at("state").as_pystr().unwrap().to_utf8().unwrap()
}

fn stdout(result: &Output) -> String {
    String::from_utf8_lossy(&result.stdout).into_owned()
}

fn required_control_reasons(document: &pyjson::Json) -> Vec<String> {
    document
        .at("exceptions")
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row.at("control").is_str("required_control"))
        .map(|row| {
            assert!(row.at("status").is_str("REVIEW"));
            row.at("reason").as_pystr().unwrap().to_utf8().unwrap()
        })
        .collect()
}

#[test]
fn a_required_control_that_did_not_run_turns_a_pass_into_review() {
    let temp = TempDir::new();
    let (plain, output) = review(&temp, "plain", &[]);
    assert_eq!(plain.status.code(), Some(0), "{plain:?}");
    let document = pack(&output);
    assert!(document.at("overall_status").is_str("PASS"));
    assert!(document.get("required_controls").is_none());
    assert!(!stdout(&plain).contains("required control"), "{plain:?}");
    for name in OPTIONAL {
        let (result, output) = review(&temp, name, &["--require-control", name]);
        assert_eq!(result.status.code(), Some(2), "{name}: {result:?}");
        let document = pack(&output);
        assert!(document.at("overall_status").is_str("REVIEW"), "{name}");
        let reasons = required_control_reasons(&document);
        assert_eq!(reasons.len(), 1, "{name}");
        assert!(reasons[0].contains(name), "{name}: {reasons:?}");
        assert!(reasons[0].contains("was skipped"), "{reasons:?}");
        assert_eq!(state_of(&document, name), "skipped");
        assert!(
            stdout(&result).contains(&format!("required control {name}: skipped")),
            "{result:?}"
        );
        let summary = fs::read_to_string(output.join("close-summary.md")).unwrap();
        assert!(summary.contains(&format!("- Required controls: {name} (skipped).")));
        assert!(viewer::verify_pack(&output).is_ok(), "{name}");
    }
}

#[test]
fn a_required_control_that_ran_leaves_a_pass_alone() {
    let temp = TempDir::new();
    let mapping = temp.join("mapping.csv");
    fs::write(&mapping, "AccountID,ReviewGroup\nA1,Cash\nE1,Equity\n").unwrap();
    for name in ALWAYS_RUN {
        let (result, output) = review(&temp, name, &["--require-control", name]);
        assert_eq!(result.status.code(), Some(0), "{name}: {result:?}");
        assert!(pack(&output).at("overall_status").is_str("PASS"), "{name}");
        assert_eq!(state_of(&pack(&output), name), "ran");
        assert!(
            stdout(&result).contains(&format!("required control {name}: ran")),
            "{result:?}"
        );
        assert!(viewer::verify_pack(&output).is_ok(), "{name}");
    }
    let (result, output) = review(
        &temp,
        "mapped",
        &[
            "--mapping",
            mapping.to_str().unwrap(),
            "--require-control",
            "account_mapping",
        ],
    );
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert!(pack(&output).at("overall_status").is_str("PASS"));
}

#[test]
fn a_control_with_a_supplied_input_it_could_not_finish_on_is_failed_not_skipped() {
    let temp = TempDir::new();
    let mapping = temp.join("gst.csv");
    fs::write(&mapping, "AccountID,GSTClass\nA1,gst\nE1,bas_excluded\n").unwrap();
    let (result, output) = review(
        &temp,
        "gst",
        &[
            "--gst-mapping",
            mapping.to_str().unwrap(),
            "--require-control",
            "gst_reasonableness",
        ],
    );
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let document = pack(&output);
    assert_eq!(state_of(&document, "gst_reasonableness"), "failed");
    assert!(required_control_reasons(&document)[0].contains("failed:"));

    let subledger = temp.join("subledger.csv");
    fs::write(&subledger, "Tenant,AccountID,SubledgerBalance\n").unwrap();
    let (result, output) = review(
        &temp,
        "subledger",
        &[
            "--subledger",
            subledger.to_str().unwrap(),
            "--require-control",
            "subledger_reconciliation",
        ],
    );
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    assert_eq!(
        state_of(&pack(&output), "subledger_reconciliation"),
        "failed"
    );

    let evidence = temp.join("evidence.json");
    fs::write(&evidence, "not json").unwrap();
    let (result, output) = review(
        &temp,
        "evidence",
        &[
            "--calculation-evidence",
            evidence.to_str().unwrap(),
            "--require-control",
            "calculation_evidence",
        ],
    );
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    assert_eq!(state_of(&pack(&output), "calculation_evidence"), "failed");
}

#[test]
fn a_review_note_does_not_clear_an_unmet_requirement() {
    let temp = TempDir::new();
    let note = temp.join("note.json");
    fs::write(
        &note,
        br#"{"reviewer_initials":"RD","reviewed_on":"2026-09-01","comment":"Recorded review only."}"#,
    )
    .unwrap();
    let (result, output) = review(
        &temp,
        "noted",
        &[
            "--review-note",
            note.to_str().unwrap(),
            "--require-control=account_mapping",
        ],
    );
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    assert!(pack(&output).at("overall_status").is_str("REVIEW"));
}

#[test]
fn the_workbench_holds_the_requirement_too() {
    let temp = TempDir::new();
    let output = temp.join("workbench");
    let result = Command::new(BIN)
        .arg("workbench")
        .arg("--current")
        .arg(trial_balance(&temp, "current.csv", "2026-08-31"))
        .arg("--prior")
        .arg(trial_balance(&temp, "prior.csv", "2026-07-31"))
        .args(["--require-control", "account_mapping", "--output"])
        .arg(&output)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    assert_eq!(state_of(&pack(&output), "account_mapping"), "skipped");
}

#[test]
fn a_name_that_is_not_a_control_is_refused_before_any_pack_is_written() {
    let temp = TempDir::new();
    // `subledger` is the controls_not_run spelling, not the control's ID.
    for name in ["subledger", "required_control", "Account_Mapping", ""] {
        let (result, output) = review(&temp, "refused", &["--require-control", name]);
        assert_eq!(result.status.code(), Some(1), "{name}: {result:?}");
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains("is not a control"), "{name}: {error}");
        assert!(error.contains("subledger_reconciliation"), "{error}");
        assert!(!output.exists(), "{name}");
    }
}

#[test]
fn repeated_names_are_recorded_once_in_order() {
    let temp = TempDir::new();
    let (result, output) = review(
        &temp,
        "repeat",
        &[
            "--require-control=subledger_reconciliation",
            "--require-control=account_mapping",
            "--require-control=subledger_reconciliation",
        ],
    );
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let document = pack(&output);
    let required = document.at("required_controls").as_array().unwrap();
    assert!(required[0].at("name").is_str("account_mapping"));
    assert!(required[1].at("name").is_str("subledger_reconciliation"));
    assert_eq!(required.len(), 2);
    assert_eq!(required_control_reasons(&document).len(), 2);
    let summary = fs::read_to_string(output.join("close-summary.md")).unwrap();
    assert!(summary.contains(
        "- Required controls: account_mapping (skipped), subledger_reconciliation (skipped)."
    ));
}

#[test]
fn a_changed_requirement_is_a_scope_change_between_packs() {
    let temp = TempDir::new();
    let (_, previous) = review(&temp, "previous", &[]);
    let (_, current) = review(&temp, "current", &["--require-control=account_mapping"]);
    let trial_balance = temp.join("current.csv");
    let result = Command::new(BIN)
        .arg("compare")
        .arg("--previous-pack")
        .arg(&previous)
        .arg("--current-pack")
        .arg(&current)
        .arg("--previous-tb")
        .arg(&trial_balance)
        .arg("--current-tb")
        .arg(&trial_balance)
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&result.stdout);
    assert!(report.contains("required_controls"), "{result:?}");
}

#[test]
fn a_summary_stripped_of_its_requirement_fails_verification() {
    let temp = TempDir::new();
    let (_, output) = review(&temp, "tampered", &["--require-control=account_mapping"]);
    let summary = output.join("close-summary.md");
    let text = fs::read_to_string(&summary).unwrap();
    let line = "- Required controls: account_mapping (skipped).";
    assert!(text.contains(line));
    fs::write(&summary, text.replace(line, "- Required controls: none.")).unwrap();
    assert!(viewer::verify_pack(&output).is_err());
}

#[test]
fn a_pack_edited_to_say_a_skipped_control_ran_fails_verification() {
    let temp = TempDir::new();
    let (_, output) = review(&temp, "edited", &["--require-control=account_mapping"]);
    for (name, skipped, ran) in [
        (
            "close-review-pack.json",
            "\"state\": \"skipped\"",
            "\"state\": \"ran\"",
        ),
        (
            "close-summary.md",
            "account_mapping (skipped)",
            "account_mapping (ran)",
        ),
    ] {
        let file = output.join(name);
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains(skipped), "{name}");
        fs::write(&file, text.replace(skipped, ran)).unwrap();
    }
    let error = viewer::verify_pack(&output).err().unwrap();
    assert!(error.contains("required_controls lists 0"), "{error}");
}
