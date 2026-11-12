// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;
#[path = "support/temp.rs"]
mod temp;

use monthly_close_control_plane::{reconciliation, viewer};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

fn sources(temp: &temp::TempDir, account_name: &str) {
    for (name, date) in [("current", "2026-09-30"), ("prior", "2026-08-31")] {
        fs::write(temp.join(format!("{name}.csv")), format!(
            "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n\
             {date},Synthetic,Assets,A,{account_name},100,0,0,100,0\n\
             {date},Synthetic,Equity,E,Capital,300,0,0,0,100\n"
        )).unwrap();
    }
}

fn review_command(temp: &temp::TempDir, output: &Path) -> Command {
    let mut command = child_profile::command(env!("CARGO_BIN_EXE_close-control"));
    command
        .args(["review", "--current"])
        .arg(temp.join("current.csv"))
        .arg("--prior")
        .arg(temp.join("prior.csv"))
        .arg("--output")
        .arg(output)
        .env_remove("GIT_WORK_TREE");
    command
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, SystemTime)> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, (Vec<u8>, SystemTime)>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), files);
            } else {
                files.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    (
                        fs::read(entry.path()).unwrap(),
                        entry.metadata().unwrap().modified().unwrap(),
                    ),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[test]
fn the_csv_field_limit_counts_unicode_characters_and_preserves_an_existing_pack() {
    let temp = temp::TempDir::new();
    sources(&temp, &"é".repeat(131_072));
    let output = temp.join("output");
    let result = review_command(&temp, &output).output().unwrap();
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert!(viewer::verify_pack(&output).is_ok());
    sources(&temp, &"é".repeat(131_073));
    let before = snapshot(&temp.0);
    let result = review_command(&temp, &output).output().unwrap();
    assert_eq!(result.status.code(), Some(1), "{result:?}");
    assert!(result.stdout.is_empty());
    let error = String::from_utf8(result.stderr).unwrap();
    assert!(
        error.contains(
            "Trial-balance file is not valid CSV: field larger than field limit (131072)."
        ),
        "{error}"
    );
    assert_eq!(snapshot(&temp.0), before);
}

fn nested_fixture() -> temp::TempDir {
    let temp = temp::TempDir::new();
    sources(&temp, "Bank");
    let result = review_command(&temp, &temp.join("pack")).output().unwrap();
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let view = temp.join("view-pack");
    fs::create_dir(&view).unwrap();
    for name in viewer::PACK_NAMES {
        fs::copy(temp.join("pack").join(name), view.join(name)).unwrap();
    }
    fs::write(temp.join("transactions.csv"), reconciliation::COLUMNS.join(",") + "\nSynthetic,CLEAR,AUD,T1,2026-09-01,R,Debit,1,0\nSynthetic,CLEAR,AUD,T2,2026-09-02,R,Credit,0,1\n").unwrap();
    temp
}

fn nested_command(temp: &temp::TempDir, case: &str) -> Command {
    let mut command = child_profile::command(env!("CARGO_BIN_EXE_close-control"));
    let json = temp.join("nested.json");
    match case {
        "view" => {
            command
                .args(["view", "--pack-dir"])
                .arg(temp.join("view-pack"));
        }
        "review-note" | "equity" => {
            command = review_command(temp, &temp.join("refused"));
            command
                .arg(if case == "equity" {
                    "--equity-schedule"
                } else {
                    "--review-note"
                })
                .arg(json);
        }
        "schedule" => {
            command
                .args(["schedule", "--kind", "expenses", "--input"])
                .arg(json);
        }
        "classify" => {
            command
                .args(["classify", "--original-transactions"])
                .arg(temp.join("transactions.csv"))
                .arg("--current-transactions")
                .arg(temp.join("transactions.csv"))
                .arg("--coding-evidence")
                .arg(json)
                .args(["--currency", "AUD"]);
        }
        "compare" => {
            command = comparison_command(temp, &json);
        }
        "reconcile" => {
            command = reconciliation_command(temp, &json);
        }
        other => panic!("Unknown fixture: {other}"),
    }
    command.env_remove("GIT_WORK_TREE");
    command
}

fn comparison_command(temp: &temp::TempDir, json: &Path) -> Command {
    let mut command = child_profile::command(env!("CARGO_BIN_EXE_close-control"));
    command
        .args(["compare", "--previous-pack"])
        .arg(temp.join("pack"))
        .arg("--current-pack")
        .arg(temp.join("pack"))
        .arg("--previous-tb")
        .arg(temp.join("current.csv"))
        .arg("--current-tb")
        .arg(temp.join("current.csv"))
        .arg("--responses")
        .arg(json);
    command
}

fn reconciliation_command(temp: &temp::TempDir, json: &Path) -> Command {
    let mut command = child_profile::command(env!("CARGO_BIN_EXE_close-control"));
    command
        .args(["reconcile", "--transactions"])
        .arg(temp.join("transactions.csv"))
        .args([
            "--tenant",
            "Synthetic",
            "--account-id",
            "CLEAR",
            "--currency",
            "AUD",
            "--period-start",
            "2026-09-01",
            "--period-end",
            "2026-09-30",
            "--opening-balance",
            "0",
            "--closing-balance",
            "0",
            "--opening-items",
        ])
        .arg(json)
        .arg("--output")
        .arg(temp.join("refused"));
    command
}

fn assert_nested_refusal(temp: &temp::TempDir, case: &str, depth: usize, diagnostic: &str) {
    let json = "[".repeat(depth) + "0" + &"]".repeat(depth);
    fs::write(temp.join("nested.json"), &json).unwrap();
    fs::write(temp.join("view-pack/close-review-pack.json"), &json).unwrap();
    let before = snapshot(&temp.0);
    let result = nested_command(temp, case).output().unwrap();
    assert_eq!(result.status.code(), Some(1), "{case}/{depth}: {result:?}");
    assert!(result.stdout.is_empty(), "{case}/{depth}");
    let error = String::from_utf8(result.stderr).unwrap();
    assert!(error.contains(diagnostic), "{case}/{depth}: {error}");
    assert_eq!(snapshot(&temp.0), before, "{case}/{depth}");
    assert!(!temp.join("refused").exists());
}

#[test]
fn deeply_nested_json_is_refused_at_each_real_command_boundary_without_writing() {
    let temp = nested_fixture();
    for (case, shallow, deep) in [
        (
            "view",
            "top level must be a JSON object",
            "nested too deeply to read",
        ),
        (
            "review-note",
            "review note must contain exactly",
            "review note is nested too deeply to read",
        ),
        (
            "classify",
            "Coding evidence must use schema 1",
            "Invalid coding evidence JSON: maximum recursion depth",
        ),
        (
            "schedule",
            "schema_version must be integer 1",
            "Invalid schedule JSON: maximum recursion depth",
        ),
        (
            "equity",
            "Equity schedule must contain exactly",
            "Invalid equity schedule: maximum recursion depth",
        ),
        (
            "compare",
            "Responses must use schema 1",
            "Invalid response JSON: maximum recursion depth",
        ),
        (
            "reconcile",
            "Opening items have an invalid schema",
            "Opening items are not readable JSON: maximum recursion depth",
        ),
    ] {
        for (depth, diagnostic) in [(500, shallow), (1001, deep)] {
            assert_nested_refusal(&temp, case, depth, diagnostic);
        }
    }
}

#[cfg(windows)]
fn native_path_spellings(source: &Path) -> Vec<String> {
    let text = source.to_str().unwrap().replace('/', r"\");
    let ordinary = if let Some(tail) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{tail}")
    } else {
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
    };
    if let Some(tail) = ordinary.strip_prefix(r"\\") {
        // A UNC fixture has no equivalent current-drive-rooted spelling.
        vec![ordinary.clone(), format!(r"\\?\UNC\{tail}")]
    } else {
        assert_eq!(
            ordinary.as_bytes()[1],
            b':',
            "expected a drive or UNC fixture"
        );
        vec![
            ordinary.clone(),
            ordinary[2..].to_owned(),
            format!(r"\\?\{ordinary}"),
        ]
    }
}

#[cfg(windows)]
#[test]
fn supported_windows_path_spellings_bind_the_same_real_trial_balance() {
    let temp = temp::TempDir::new();
    sources(&temp, "Bank");
    let source = temp.join("current.csv");
    // Independently calculated from the fabricated LF CSV bytes.
    let digest = "4ac093f68d3dd1b65a6ca2ccca043dd92a89f0774d46ecc88e2064f7325a1b29";
    let before = snapshot(&temp.0);
    for (index, spelling) in native_path_spellings(&source).into_iter().enumerate() {
        let output = temp.join(format!("output-{index}"));
        let mut command = child_profile::command(env!("CARGO_BIN_EXE_close-control"));
        let result = command
            .args(["review", "--current", &spelling])
            .arg("--prior")
            .arg(temp.join("prior.csv"))
            .arg("--output")
            .arg(&output)
            .current_dir(&temp.0)
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(0), "{spelling}: {result:?}");
        let verified = viewer::verify_pack(&output).unwrap();
        assert!(verified.document.at("overall_status").is_str("PASS"));
        assert!(
            verified
                .document
                .at("exceptions")
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            verified
                .document
                .at("source_sha256")
                .at("current_trial_balance")
                .is_str(digest)
        );
    }
    let after = snapshot(&temp.0);
    for (path, bytes_and_mtime) in before {
        assert_eq!(after.get(&path), Some(&bytes_and_mtime));
    }
}

#[test]
fn separate_negative_thresholds_reach_the_decimal_validator() {
    let invalid_negative = "must be a finite non-negative decimal";
    let cases = [
        ("-0.5", invalid_negative),
        ("-1", invalid_negative),
        ("-.5", invalid_negative),
        ("-1e2", invalid_negative),
        ("-0.5.0", "not a decimal: '-0.5.0'"),
        ("-.5.0", "not a decimal: '-.5.0'"),
        ("--prior", "expected one argument"),
        ("-x1", "expected one argument"),
    ];
    for (value, diagnostic) in cases.into_iter().chain([("-0.0", "")]) {
        let directory = temp::TempDir::new();
        let output = child_profile::command(env!("CARGO_BIN_EXE_close-control"))
            .args(["review", "--absolute-threshold", value])
            .current_dir(directory.join("."))
            .env("COLUMNS", "80")
            .env("NO_COLOR", "1")
            .env("PYTHON_COLORS", "0")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{value}");
        assert!(output.stdout.is_empty(), "{value}");
        let stderr = String::from_utf8(output.stderr)
            .unwrap()
            .replace("\r\n", "\n");
        let diagnostic = if diagnostic.is_empty() {
            "the following arguments are required: --current, --prior, --output".to_owned()
        } else {
            format!("argument --absolute-threshold: {diagnostic}")
        };
        assert!(
            stderr.starts_with("usage: close-control review"),
            "{stderr}"
        );
        assert!(
            stderr.ends_with(&format!("close-control review: error: {diagnostic}\n")),
            "{stderr}"
        );
        assert!(std::fs::read_dir(&directory.0).unwrap().next().is_none());
    }
}
