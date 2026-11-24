// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `close-control scorecard` over the fabricated examples' own results.

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;
#[path = "support/temp.rs"]
mod temp;

use monthly_close_control_plane::pyjson;
use std::{path::Path, process::Output};
use temp::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn close_control(args: &[&str]) -> Output {
    child_profile::command(BIN).args(args).output().unwrap()
}

fn example(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name)
        .to_str()
        .unwrap()
        .to_owned()
}

/// Run a command and save its standard output as `name` in `dir`.
fn saved(dir: &TempDir, name: &str, args: &[&str]) -> String {
    let output = close_control(args);
    let path = dir.join(name);
    std::fs::write(&path, output.stdout).unwrap();
    path.to_str().unwrap().to_owned()
}

#[test]
fn the_examples_score_by_category_and_exit_2() {
    let dir = TempDir::new();
    let fixed = saved(
        &dir,
        "fixed-assets.json",
        &[
            "fixed-assets",
            "--register",
            &example("fixed_asset_register.csv"),
            "--trial-balance",
            &example("fixed_asset_trial_balance.csv"),
            "--currency",
            "AUD",
        ],
    );
    let duplicates = saved(
        &dir,
        "duplicates.json",
        &[
            "duplicates",
            "--documents",
            &example("duplicates.csv"),
            "--currency",
            "AUD",
            "--period-start",
            "2026-09-01",
            "--period-end",
            "2026-09-30",
        ],
    );
    let output = close_control(&["scorecard", "--result", &fixed, "--result", &duplicates]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let card = pyjson::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert!(card.at("tenant").is_str("Varrock Ventures Pty Ltd"));
    assert_eq!(pyjson::canonical(card.at("score_percent")), "0");
    assert_eq!(pyjson::canonical(card.at("findings_total")), "7");
    let results = card.at("results").as_array().unwrap();
    assert!(results[0].at("category").is_str("fixed-assets"));
    assert_eq!(
        pyjson::canonical(results[1].at("codes").at("DUPLICATE_BILL")),
        "1"
    );
}

#[test]
fn results_all_at_pass_exit_0() {
    let dir = TempDir::new();
    let path = dir.join("coding.json");
    std::fs::write(
        &path,
        r#"{"status":"PASS","tenant":"Varrock","findings":[]}"#,
    )
    .unwrap();
    let output = close_control(&["scorecard", "--result", path.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
}

const INVALID_RESULTS: [(&str, &str); 8] = [
    ("arr.json", "[]"),
    ("no_status.json", r#"{"tenant":"Varrock","findings":[]}"#),
    ("no_tenant.json", r#"{"status":"PASS","findings":[]}"#),
    (
        "no_findings.json",
        r#"{"status":"PASS","tenant":"Varrock"}"#,
    ),
    (
        "bad_status.json",
        r#"{"status":1,"tenant":"Varrock","findings":[]}"#,
    ),
    (
        "bad_tenant.json",
        r#"{"status":"PASS","tenant":1,"findings":[]}"#,
    ),
    (
        "bad_findings.json",
        r#"{"status":"PASS","tenant":"Varrock","findings":{}}"#,
    ),
    (
        "done_status.json",
        r#"{"status":"DONE","tenant":"Varrock","findings":[]}"#,
    ),
];

fn write_result(dir: &TempDir, rel: &str, text: &str) -> String {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, text).unwrap();
    path.to_str().unwrap().to_owned()
}

fn assert_rejected(name: &str, paths: &[&str], message: &str) {
    let mut args = vec!["scorecard"];
    for path in paths {
        args.extend(["--result", path]);
    }
    let output = close_control(&args);
    assert_eq!(output.status.code(), Some(1), "{name}: {output:?}");
    assert!(output.stdout.is_empty(), "{name}: {output:?}");
    assert!(
        String::from_utf8(output.stderr).unwrap().contains(message),
        "{name}"
    );
}

#[test]
fn unusable_results_exit_1_with_empty_stdout() {
    let dir = TempDir::new();
    for (rel, body) in INVALID_RESULTS {
        let path = write_result(&dir, rel, body);
        assert_rejected(rel, &[&path], "not a result of");
    }
    let valid = write_result(
        &dir,
        "varrock.json",
        r#"{"status":"PASS","tenant":"Varrock","findings":[]}"#,
    );
    let falador = write_result(
        &dir,
        "falador.json",
        r#"{"status":"PASS","tenant":"Falador","findings":[]}"#,
    );
    let dup = write_result(
        &dir,
        "other/varrock.json",
        r#"{"status":"PASS","tenant":"Varrock","findings":[]}"#,
    );
    assert_rejected(
        "mixed_tenants",
        &[&valid, &falador],
        "summarise each tenant",
    );
    assert_rejected(
        "duplicate_stem",
        &[&valid, &dup],
        "a second result named 'varrock'",
    );
    assert_rejected(
        "missing_file",
        &[dir.join("absent.json").to_str().unwrap()],
        "Result does not exist",
    );
}

#[test]
fn help_names_every_option() {
    let output = close_control(&["scorecard", "--help"]);
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(help, include_str!("../src/help/scorecard-help.txt"));
}
