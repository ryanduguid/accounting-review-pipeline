// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `close-control statement`, run on the fabricated example statement as a
//! reviewer would.

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use monthly_close_control_plane::pyjson::{self, Json};
use std::{path::PathBuf, process::Output};

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn example() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/statement.csv")
        .to_str()
        .unwrap()
        .to_owned()
}

fn statement(args: &[&str]) -> Output {
    child_profile::command(BIN)
        .arg("statement")
        .args(args)
        .output()
        .unwrap()
}

fn text(value: &Json, key: &str) -> String {
    value.at(key).as_pystr().unwrap().to_utf8().unwrap()
}

#[test]
fn the_example_statement_lists_its_running_balance_break_and_exits_2() {
    let output = statement(&["--statement", &example(), "--currency", "AUD"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let result = pyjson::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(text(&result, "status"), "REVIEW");
    assert_eq!(
        text(&result, "account"),
        "Varrock Ventures operating account"
    );
    assert_eq!(result.at("page_count").as_plain_int(), Some(&2.into()));
    assert_eq!(result.at("line_count").as_plain_int(), Some(&5.into()));
    assert_eq!(text(&result, "opening_balance"), "12000.00");
    assert_eq!(text(&result, "closing_balance"), "10010.00");
    let listed: Vec<_> = result
        .at("findings")
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (text(f, "code"), text(f, "expected"), text(f, "difference")))
        .collect();
    assert_eq!(
        listed,
        [(
            "STATEMENT_RUNNING_BREAK".into(),
            "14205.00".into(),
            "-180.00".into()
        )]
    );
}

#[test]
fn a_missing_statement_or_bad_currency_exits_1() {
    for args in [
        ["--statement", "no-such-statement.csv", "--currency", "AUD"],
        ["--statement", &example(), "--currency", "NZD"],
    ] {
        let output = statement(&args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn help_names_every_option() {
    let output = statement(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(help, include_str!("../src/help/statement-help.txt"));
}
