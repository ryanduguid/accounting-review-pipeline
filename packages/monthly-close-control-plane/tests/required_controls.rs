// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! `--require-control`: a named control that did not run keeps the pack from
//! `PASS`, and a script reading the exit code sees it.

#[path = "support/temp.rs"]
mod temp;

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use monthly_close_control_plane::{pyjson, viewer};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
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
    let result = child_profile::command(BIN)
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
fn a_header_only_mapping_is_failed() {
    let temp = TempDir::new();
    let mapping = temp.join("mapping.csv");
    fs::write(&mapping, "AccountID,ReviewGroup\n").unwrap();
    let (result, output) = review(
        &temp,
        "mapping",
        &[
            "--mapping",
            mapping.to_str().unwrap(),
            "--require-control",
            "account_mapping",
        ],
    );
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    assert_eq!(state_of(&pack(&output), "account_mapping"), "failed");
}

#[test]
fn a_required_calculation_with_no_evidence_file_is_skipped() {
    let temp = TempDir::new();
    let (result, output) = review(
        &temp,
        "calculation",
        &[
            "--require-calculation",
            "coal-lsl-levy",
            "--require-control",
            "calculation_evidence",
        ],
    );
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    assert_eq!(state_of(&pack(&output), "calculation_evidence"), "skipped");
    let shown = child_profile::command(BIN)
        .args(["view", "--pack-dir"])
        .arg(&output)
        .output()
        .unwrap();
    assert_eq!(shown.status.code(), Some(0), "{shown:?}");
    assert!(shown.stderr.is_empty(), "{shown:?}");
    let sheet = stdout(&shown);
    assert!(sheet.contains("calculation_evidence (skipped)"), "{sheet}");
    assert!(sheet.contains("- Required: coal-lsl-levy"), "{sheet}");
    assert!(sheet.contains("- Supplied: none. The exceptions say which calculation is missing."));
}

#[test]
fn mixed_tenants_or_dates_are_refused_before_a_pack_is_published() {
    for (replacement, diagnostic) in [
        (
            "2026-08-31,Another fabricated tenant,Equity",
            "exactly one tenant",
        ),
        (
            "2026-07-31,Fabricated Varrock,Equity",
            "exactly one ReportDate",
        ),
    ] {
        let temp = TempDir::new();
        let current = trial_balance(&temp, "current.csv", "2026-08-31");
        let prior = trial_balance(&temp, "prior.csv", "2026-07-31");
        let original = fs::read_to_string(&current).unwrap();
        let changed = original.replacen("2026-08-31,Fabricated Varrock,Equity", replacement, 1);
        assert_ne!(changed, original);
        fs::write(&current, &changed).unwrap();
        let prior_bytes = fs::read(&prior).unwrap();
        let output = temp.join("refused");
        let result = child_profile::command(BIN)
            .args(["review", "--current"])
            .arg(&current)
            .arg("--prior")
            .arg(&prior)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1), "{result:?}");
        assert!(result.stdout.is_empty(), "{result:?}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains(diagnostic),
            "{result:?}"
        );
        assert!(!output.exists());
        assert_eq!(fs::read_to_string(current).unwrap(), changed);
        assert_eq!(fs::read(prior).unwrap(), prior_bytes);
    }
}

#[test]
fn formula_like_account_names_remain_literal_across_the_pack_and_view() {
    for name in ["=BANK", "+BANK", "-BANK", "@BANK"] {
        let temp = TempDir::new();
        let current = trial_balance(&temp, "current.csv", "2026-08-31");
        let prior = trial_balance(&temp, "prior.csv", "2026-07-31");
        let original = fs::read_to_string(&current).unwrap();
        let changed = original.replacen(",A1,Bank,090,", &format!(",A1,{name},090,"), 1);
        assert_ne!(changed, original);
        fs::write(&current, changed).unwrap();
        let output = temp.join("pack");
        let result = child_profile::command(BIN)
            .args(["review", "--current"])
            .arg(current)
            .arg("--prior")
            .arg(prior)
            .arg("--output")
            .arg(&output)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(2), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        assert!(
            pack(&output)
                .at("exceptions")
                .as_array()
                .unwrap()
                .iter()
                .any(|row| {
                    row.at("control").is_str("account_metadata")
                        && row.at("account_name").is_str(name)
                })
        );
        let csv = fs::read_to_string(output.join("exceptions.csv")).unwrap();
        assert!(csv.contains(&format!("'{name}")), "{csv}");
        let shown = child_profile::command(BIN)
            .args(["view", "--pack-dir"])
            .arg(output)
            .output()
            .unwrap();
        assert_eq!(shown.status.code(), Some(0), "{shown:?}");
        assert!(shown.stderr.is_empty(), "{shown:?}");
        assert!(stdout(&shown).contains(name), "{shown:?}");
    }
}

/// A class for every account the example balances move.
fn gst_classes(path: &Path) {
    fs::write(
        path,
        "AccountID,GSTClass\n100,bas_excluded\n110,bas_excluded\n200,gst\n300,taxable_sales\n\
         400,creditable_purchases\n500,creditable_purchases\n900,bas_excluded\n",
    )
    .unwrap();
}

/// A complete equity schedule for the 2 balances: a fabricated tenant's account 900.
fn equity_schedule(path: &Path, prior: &str, current: &str) {
    let digest = |path: &str| monthly_close_control_plane::sha256(&fs::read(path).unwrap());
    fs::write(
        path,
        format!(
            r#"{{"schema_version":1,"tenant":"Varrock Ventures Pty Ltd","opening_date":"2026-06-30",
            "closing_date":"2026-07-31","currency":"AUD","basis":"ledger_equity",
            "prior_source_sha256":"{}","current_source_sha256":"{}",
            "equity_account_ids":["900"],"complete":true,"movements":[]}}"#,
            digest(prior),
            digest(current)
        ),
    )
    .unwrap();
}

/// The package's example close with every optional control's input: each runs,
/// and those that raise hold the pack at `REVIEW` or worse on their own account.
#[test]
fn every_optional_control_ran_and_its_own_exceptions_hold_the_pack() {
    let temp = TempDir::new();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let example = |name: &str| examples.join(name).to_str().unwrap().to_owned();
    let current = example("current_trial_balance.csv");
    let prior = example("prior_trial_balance.csv");
    let gst = temp.join("gst.csv");
    gst_classes(&gst);
    let settlements = temp.join("settlements.csv");
    fs::write(&settlements, "Reference,Amount\n").unwrap();
    let schedule = temp.join("equity.json");
    equity_schedule(&schedule, &prior, &current);
    let output = temp.join("all");
    let mut command = child_profile::command(BIN);
    command
        .args(["review", "--current", &current, "--prior", &prior])
        .args(["--mapping", &example("account_mapping.csv")])
        .args(["--mapping-policy", &example("mapping_policy.csv")])
        .args(["--balance-policy", &example("balance_policy.csv")])
        .args(["--subledger", &example("subledger_balances.csv")])
        .arg("--gst-mapping")
        .arg(&gst)
        .arg("--gst-settlements")
        .arg(&settlements)
        .args([
            "--calculation-evidence",
            &example("calculation-evidence-coal-lsl-levy.json"),
        ])
        .arg("--equity-schedule")
        .arg(&schedule)
        .args(["--equity-currency", "AUD"])
        .args([
            "--equity-currency-evidence",
            "Fabricated report metadata: AUD",
        ])
        .arg("--output")
        .arg(&output);
    for name in OPTIONAL {
        command.args(["--require-control", name]);
    }
    let result = command.output().unwrap();
    assert_eq!(result.status.code(), Some(2), "{result:?}");
    let document = pack(&output);
    for name in OPTIONAL {
        assert_eq!(state_of(&document, name), "ran", "{name}");
    }
    assert!(required_control_reasons(&document).is_empty());
    assert!(!document.at("exceptions").as_array().unwrap().is_empty());
    assert!(!document.at("overall_status").is_str("PASS"));
    assert!(viewer::verify_pack(&output).is_ok());
}

#[test]
fn the_new_option_makes_the_shorter_require_spellings_ambiguous() {
    let temp = TempDir::new();
    for spelling in ["--req", "--require", "--require-c"] {
        let (result, _) = review(&temp, "ambiguous", &[spelling, "x"]);
        assert_eq!(result.status.code(), Some(1), "{spelling}: {result:?}");
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains("ambiguous option"), "{spelling}: {error}");
    }
    let (result, _) = review(&temp, "unique", &["--require-ca", "not a slug"]);
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("--require-calculation"), "{error}");
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
    let result = child_profile::command(BIN)
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
    let result = child_profile::command(BIN)
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

/// A pack whose `account_mapping` ran and whose `balance_policy` was skipped, its
/// JSON put through `edit`, and what verification then says.
fn edited(edit: impl Fn(&str) -> String) -> String {
    let temp = TempDir::new();
    let mapping = temp.join("mapping.csv");
    fs::write(&mapping, "AccountID,ReviewGroup\nA1,Cash\nE1,Equity\n").unwrap();
    let (_, output) = review(
        &temp,
        "edited",
        &[
            "--mapping",
            mapping.to_str().unwrap(),
            "--require-control=account_mapping",
            "--require-control=balance_policy",
        ],
    );
    let file = output.join("close-review-pack.json");
    let text = fs::read_to_string(&file).unwrap();
    let changed = edit(&text);
    assert_ne!(changed, text);
    fs::write(&file, changed).unwrap();
    viewer::verify_pack(&output).err().unwrap()
}

const RAN: &str = "\"state\": \"ran\"";
const SKIPPED: &str = "\"state\": \"skipped\"";

#[test]
fn a_pack_whose_states_disagree_with_its_exceptions_fails_verification() {
    let swapped = edited(|text| {
        text.replace(RAN, "swap")
            .replace(SKIPPED, RAN)
            .replace("swap", SKIPPED)
    });
    assert!(swapped.contains("disagrees"), "{swapped}");
    let failed = edited(|text| text.replace(SKIPPED, "\"state\": \"failed\""));
    assert!(failed.contains("disagrees"), "{failed}");
    let all_ran = edited(|text| text.replace(SKIPPED, RAN));
    assert!(all_ran.contains("disagrees"), "{all_ran}");
}

#[test]
fn a_pack_naming_an_unknown_or_repeated_control_fails_verification() {
    let invented =
        edited(|text| text.replace("\"name\": \"balance_policy\"", "\"name\": \"made_up\""));
    assert!(invented.contains("not a control"), "{invented}");
    let repeated = edited(|text| {
        text.replace(
            "\"name\": \"balance_policy\"",
            "\"name\": \"account_mapping\"",
        )
    });
    assert!(repeated.contains("without repeats"), "{repeated}");
}

#[test]
fn an_exception_with_no_required_controls_member_fails_verification() {
    let error = edited(|text| {
        let mut document = pyjson::loads(text).unwrap();
        if let pyjson::Json::Object(pairs) = &mut document {
            pairs.retain(|(key, _)| !key.is("required_controls"));
        }
        pyjson::pretty(&document)
    });
    assert!(error.contains("names no required_controls"), "{error}");
}

#[test]
fn a_pack_edited_to_pass_with_a_control_unmet_fails_verification() {
    let error = edited(|text| {
        text.replace(
            "\"overall_status\": \"REVIEW\"",
            "\"overall_status\": \"PASS\"",
        )
        .replace("\"status\": \"REVIEW\"", "\"status\": \"PASS\"")
    });
    assert!(
        error.contains("PASS with a required control unmet"),
        "{error}"
    );
}
