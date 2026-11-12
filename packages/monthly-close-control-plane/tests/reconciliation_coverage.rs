// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! `reconciliation_coverage`: with `--require-control reconciliation_coverage`,
//! each balance-sheet account with a balance that nothing supplied ties out is
//! listed for review.

#[path = "support/temp.rs"]
mod temp;

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use monthly_close_control_plane::{pyjson::Json, viewer};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Output,
};
use temp::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_close-control");
const REQUIRE: [&str; 2] = ["--require-control", "reconciliation_coverage"];
const TENANT: &str = "Fabricated Varrock";

/// Section, account, YTD debit, YTD credit. `A3` has no balance, `R1` and `X1`
/// are profit and loss, and `L1` spells its section in lower case.
const ACCOUNTS: [(&str, &str, &str, &str); 7] = [
    ("Assets", "A1", "100.00", "0.00"),
    ("Assets", "A2", "50.00", "0.00"),
    ("Assets", "A3", "0.00", "0.00"),
    ("liabilities", "L1", "0.00", "30.00"),
    ("Equity", "E1", "0.00", "100.00"),
    ("Revenue", "R1", "0.00", "70.00"),
    ("Expenses", "X1", "50.00", "0.00"),
];

fn trial_balance(
    temp: &TempDir,
    name: &str,
    date: &str,
    extra: &[(&str, &str, &str, &str)],
) -> PathBuf {
    let mut csv = String::from(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n",
    );
    for (section, id, debit, credit) in ACCOUNTS.iter().chain(extra) {
        csv += &format!(
            "{date},{TENANT},{section},{id},Account {id},{id},0.00,0.00,{debit},{credit}\n"
        );
    }
    let path = temp.join(name);
    fs::write(&path, csv).unwrap();
    path
}

fn subledger(temp: &TempDir, rows: &[(&str, &str)]) -> PathBuf {
    let mut csv = String::from("Tenant,AccountID,SubledgerBalance\n");
    for (id, balance) in rows {
        csv += &format!("{TENANT},{id},{balance}\n");
    }
    let path = temp.join("subledger.csv");
    fs::write(&path, csv).unwrap();
    path
}

/// Reviews the fixture close, with `extra` accounts and arguments.
fn review(
    temp: &TempDir,
    accounts: &[(&str, &str, &str, &str)],
    args: &[&OsStr],
) -> (Output, PathBuf) {
    let output = temp.join("pack");
    let result = child_profile::command(BIN)
        .arg("review")
        .arg("--current")
        .arg(trial_balance(temp, "current.csv", "2026-08-31", accounts))
        .arg("--prior")
        .arg(trial_balance(temp, "prior.csv", "2026-07-31", accounts))
        .args(args)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    (result, output)
}

fn pack(output: &Path) -> Json {
    monthly_close_control_plane::pyjson::loads(
        &fs::read_to_string(output.join("close-review-pack.json")).unwrap(),
    )
    .unwrap()
}

fn text(value: &Json) -> String {
    value.as_pystr().unwrap().to_utf8().unwrap()
}

/// Each `reconciliation_coverage` exception as account, balance and reason.
fn listed(document: &Json) -> Vec<(String, String, String)> {
    document
        .at("exceptions")
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row.at("control").is_str("reconciliation_coverage"))
        .map(|row| {
            assert!(row.at("status").is_str("REVIEW"));
            (
                text(row.at("account_id")),
                text(row.at("current_value")),
                text(row.at("reason")),
            )
        })
        .collect()
}

fn accounts(document: &Json) -> Vec<String> {
    listed(document).into_iter().map(|(id, ..)| id).collect()
}

fn state(document: &Json) -> String {
    let runs = document.at("required_controls").as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert!(runs[0].at("name").is_str("reconciliation_coverage"));
    text(runs[0].at("state"))
}

#[test]
fn without_the_requirement_nothing_is_listed() {
    let temp = TempDir::new();
    let (result, output) = review(&temp, &[], &[]);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let document = pack(&output);
    assert!(document.at("overall_status").is_str("PASS"));
    assert!(document.get("required_controls").is_none());
    assert!(listed(&document).is_empty());
}

#[test]
fn an_uncovered_close_lists_each_balance_sheet_account_with_a_balance() {
    let temp = TempDir::new();
    let require = REQUIRE.map(OsStr::new);
    let (result, output) = review(&temp, &[], &require);
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let document = pack(&output);
    assert!(document.at("overall_status").is_str("REVIEW"));
    assert_eq!(state(&document), "ran");
    let found = listed(&document);
    let balances: Vec<_> = found
        .iter()
        .map(|(id, balance, _)| (id.as_str(), balance.as_str()))
        .collect();
    assert_eq!(
        balances,
        [
            ("A1", "100.00"),
            ("A2", "50.00"),
            ("E1", "-100.00"),
            ("L1", "-30.00")
        ]
    );
    for (_, _, reason) in &found {
        assert!(
            reason.starts_with("Balance-sheet account with a closing balance"),
            "{reason}"
        );
    }
    assert!(document.at("client_queries").as_array().unwrap().is_empty());
    let summary = fs::read_to_string(output.join("close-summary.md")).unwrap();
    assert!(summary.contains("- Required controls: reconciliation_coverage (ran)."));
    assert!(viewer::verify_pack(&output).is_ok());
}

#[test]
fn a_partly_covered_close_lists_only_the_accounts_nothing_ties_out() {
    let temp = TempDir::new();
    // A2's subledger differs: subledger_reconciliation raises that, and the
    // supplied row still covers it.
    let tied = subledger(&temp, &[("A1", "100.00"), ("A2", "45.00")]);
    let mut args = REQUIRE.map(OsStr::new).to_vec();
    args.extend([OsStr::new("--subledger"), tied.as_os_str()]);
    let (result, output) = review(&temp, &[], &args);
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let document = pack(&output);
    assert_eq!(state(&document), "ran");
    assert_eq!(accounts(&document), ["E1", "L1"]);
    let differences: Vec<_> = document
        .at("exceptions")
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row.at("control").is_str("subledger_reconciliation"))
        .map(|row| text(row.at("account_id")))
        .collect();
    assert_eq!(differences, ["A2"]);
    assert!(viewer::verify_pack(&output).is_ok());
}

#[test]
fn a_covered_close_passes_with_the_requirement_met() {
    let temp = TempDir::new();
    let tied = subledger(
        &temp,
        &[
            ("A1", "100.00"),
            ("A2", "50.00"),
            ("L1", "-30.00"),
            ("E1", "-100.00"),
        ],
    );
    let mut args = REQUIRE.map(OsStr::new).to_vec();
    args.extend([OsStr::new("--subledger"), tied.as_os_str()]);
    let (result, output) = review(&temp, &[], &args);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    let document = pack(&output);
    assert!(document.at("overall_status").is_str("PASS"));
    assert_eq!(state(&document), "ran");
    assert!(document.at("exceptions").as_array().unwrap().is_empty());
    assert!(viewer::verify_pack(&output).is_ok());
}

#[test]
fn an_account_in_an_unrecognised_section_is_listed_unless_something_ties_it_out() {
    let temp = TempDir::new();
    // Suspense's debit is balanced by a liability credit.
    let extra = [
        ("Suspense", "S1", "25.00", "0.00"),
        ("Liabilities", "L2", "0.00", "25.00"),
    ];
    let tied = subledger(
        &temp,
        &[
            ("A1", "100.00"),
            ("A2", "50.00"),
            ("L1", "-30.00"),
            ("L2", "-25.00"),
            ("E1", "-100.00"),
        ],
    );
    let mut args = REQUIRE.map(OsStr::new).to_vec();
    args.extend([OsStr::new("--subledger"), tied.as_os_str()]);
    let (result, output) = review(&temp, &extra, &args);
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let found = listed(&pack(&output));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0].0.as_str(), found[0].1.as_str()), ("S1", "25.00"));
    assert!(found[0].2.contains("cannot be told"), "{found:?}");

    let tied = subledger(
        &temp,
        &[
            ("A1", "100.00"),
            ("A2", "50.00"),
            ("L1", "-30.00"),
            ("L2", "-25.00"),
            ("E1", "-100.00"),
            ("S1", "25.00"),
        ],
    );
    fs::remove_dir_all(&output).unwrap();
    let mut args = REQUIRE.map(OsStr::new).to_vec();
    args.extend([OsStr::new("--subledger"), tied.as_os_str()]);
    let (result, output) = review(&temp, &extra, &args);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert!(listed(&pack(&output)).is_empty());
}

/// The example close with its subledger: an equity account counts as covered
/// once the equity reconciliation computed it.
#[test]
fn an_equity_account_the_equity_reconciliation_computed_is_covered() {
    let temp = TempDir::new();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let example = |name: &str| examples.join(name).to_str().unwrap().to_owned();
    let (current, prior) = (
        example("current_trial_balance.csv"),
        example("prior_trial_balance.csv"),
    );
    let digest = |path: &str| monthly_close_control_plane::sha256(&fs::read(path).unwrap());
    let schedule = temp.join("equity.json");
    fs::write(
        &schedule,
        format!(
            r#"{{"schema_version":1,"tenant":"Varrock Ventures Pty Ltd","opening_date":"2026-06-30",
            "closing_date":"2026-07-31","currency":"AUD","basis":"ledger_equity",
            "prior_source_sha256":"{}","current_source_sha256":"{}",
            "equity_account_ids":["900"],"complete":true,"movements":[]}}"#,
            digest(&prior),
            digest(&current)
        ),
    )
    .unwrap();
    let run = |output: &str, equity: bool| {
        let output = temp.join(output);
        let mut command = child_profile::command(BIN);
        command
            .args(["review", "--current", &current, "--prior", &prior])
            .args(["--subledger", &example("subledger_balances.csv")])
            .args(REQUIRE)
            .arg("--output")
            .arg(&output);
        if equity {
            command
                .arg("--equity-schedule")
                .arg(&schedule)
                .args(["--equity-currency", "AUD"])
                .args([
                    "--equity-currency-evidence",
                    "Fabricated report metadata: AUD",
                ]);
        }
        let result = command.output().unwrap();
        assert_eq!(result.status.code(), Some(2), "{result:?}");
        assert!(viewer::verify_pack(&output).is_ok());
        accounts(&pack(&output))
    };
    assert_eq!(run("without-equity", false), ["100", "900"]);
    assert_eq!(run("with-equity", true), ["100"]);
}
