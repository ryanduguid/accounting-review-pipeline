// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `close-control fixed-assets`, run on the fabricated example register and
//! trial balance as a reviewer would.

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;
#[path = "support/temp.rs"]
mod temp;

use monthly_close_control_plane::pyjson::{self, Json};
use std::{path::PathBuf, process::Output};
use temp::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn example(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name)
        .to_str()
        .unwrap()
        .to_owned()
}

fn fixed_assets(args: &[&str]) -> Output {
    child_profile::command(BIN)
        .arg("fixed-assets")
        .args(args)
        .output()
        .unwrap()
}

fn run(trial_balance: &str, currency: &str) -> Output {
    fixed_assets(&[
        "--register",
        &example("fixed_asset_register.csv"),
        "--trial-balance",
        trial_balance,
        "--currency",
        currency,
    ])
}

fn text(value: &Json, key: &str) -> String {
    value.at(key).as_pystr().unwrap().to_utf8().unwrap()
}

#[test]
fn the_example_lists_the_unposted_disposal_and_exits_2() {
    let output = run(&example("fixed_asset_trial_balance.csv"), "AUD");
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let result = pyjson::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(text(&result, "status"), "REVIEW");
    assert_eq!(text(&result, "report_date"), "2026-09-30");
    assert_eq!(
        result.at("assets_in_service").as_plain_int(),
        Some(&4.into())
    );
    let listed: Vec<_> = result
        .at("findings")
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let subject = item
                .get("account_id")
                .unwrap_or_else(|| item.at("asset_id"));
            let difference = item.get("difference").map_or(String::new(), |value| {
                format!(" {}", value.as_pystr().unwrap().to_utf8().unwrap())
            });
            format!(
                "{} {}{difference}",
                text(item, "code"),
                subject.as_pystr().unwrap().to_utf8().unwrap()
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            "DEPRECIATION_EXCEEDS_COST FA-004",
            "COST_DIFFERENCE 710 28000.00",
            "ACCUMULATED_DEPRECIATION_DIFFERENCE 711 24500.00",
        ]
    );
}

#[test]
fn a_ledger_that_ties_exits_0() {
    let temp = TempDir::new();
    let tied = temp.join("tied.csv");
    let source = std::fs::read_to_string(example("fixed_asset_trial_balance.csv")).unwrap();
    // The disposal journal posted and the laptops' depreciation corrected.
    let fixed = source
        .replace(",76500.00,0.00\n", ",48500.00,0.00\n")
        .replace(",0.00,36250.00\n", ",0.00,11750.00\n")
        .replace(",0.00,5480.00\n", ",0.00,4780.00\n");
    std::fs::write(&tied, fixed).unwrap();
    let register = temp.join("register.csv");
    let rows = std::fs::read_to_string(example("fixed_asset_register.csv")).unwrap();
    std::fs::write(&register, rows.replace(",5100.00,", ",4400.00,")).unwrap();
    let output = fixed_assets(&[
        "--register",
        register.to_str().unwrap(),
        "--trial-balance",
        tied.to_str().unwrap(),
        "--currency",
        "AUD",
    ]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let result = pyjson::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(text(&result, "status"), "PASS");
}

#[test]
fn another_currency_exits_1() {
    let output = run(&example("fixed_asset_trial_balance.csv"), "USD");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("declared --currency 'USD'"), "{error}");
}

#[test]
fn a_missing_trial_balance_exits_1() {
    let temp = TempDir::new();
    let output = run(temp.join("absent.csv").to_str().unwrap(), "AUD");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("Trial-balance file does not exist"),
        "{error}"
    );
}

#[test]
fn a_missing_option_exits_1_with_the_usage() {
    let output = fixed_assets(&["--register", &example("fixed_asset_register.csv")]);
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        error.ends_with(
            "close-control fixed-assets: error: the following arguments are required: --trial-balance, --currency\n"
        ),
        "{error}"
    );
}

#[test]
fn help_names_every_option() {
    let output = fixed_assets(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(help, include_str!("../src/help/fixed-assets-help.txt"));
}
