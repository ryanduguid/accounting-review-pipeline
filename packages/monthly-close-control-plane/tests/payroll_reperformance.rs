// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `close-control payroll`, run on the fabricated example register as a
//! reviewer would.

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use monthly_close_control_plane::pyjson::{self, Json};
use std::{path::PathBuf, process::Output};

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn example() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/payroll.csv")
        .to_str()
        .unwrap()
        .to_owned()
}

fn payroll(args: &[&str]) -> Output {
    child_profile::command(BIN)
        .arg("payroll")
        .args(args)
        .output()
        .unwrap()
}

fn text(value: &Json, key: &str) -> String {
    value.at(key).as_pystr().unwrap().to_utf8().unwrap()
}

#[test]
fn the_example_register_lists_each_line_worth_review_and_exits_2() {
    let output = payroll(&["--register", &example()]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let result = pyjson::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(text(&result, "status"), "REVIEW");
    assert_eq!(text(&result, "tenant"), "Varrock Ventures Pty Ltd");
    assert_eq!(result.at("line_count").as_plain_int(), Some(&4.into()));
    assert_eq!(
        result.at("payg_not_reperformed").as_plain_int(),
        Some(&1.into())
    );
    let listed: Vec<_> = result
        .at("findings")
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                text(f, "code"),
                text(f, "employee_id"),
                text(f, "difference"),
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            ("SG_SHORTFALL".into(), "E4".into(), "10.00".into()),
            ("SG_TIMING_REVIEW".into(), "E4".into(), "0.00".into()),
            ("PAYG_DIFFERENCE".into(), "E2".into(), "-6.00".into()),
        ]
    );
}

#[test]
fn a_missing_register_exits_1() {
    let output = payroll(&["--register", "no-such-register.csv"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
}

#[test]
fn help_names_every_option() {
    let output = payroll(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(help, include_str!("../src/help/payroll-help.txt"));
}
