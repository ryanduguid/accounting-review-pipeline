// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `close-control coding`, run on the fabricated example lines as a reviewer
//! would.

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;
#[path = "support/temp.rs"]
mod temp;

use monthly_close_control_plane::pyjson::{self, Json};
use std::{path::PathBuf, process::Output};
use temp::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn example() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/coding_lines.csv")
        .to_str()
        .unwrap()
        .to_owned()
}

fn coding(args: &[&str]) -> Output {
    child_profile::command(BIN)
        .arg("coding")
        .args(args)
        .output()
        .unwrap()
}

fn run(lines: &str, currency: &str) -> Output {
    coding(&[
        "--lines",
        lines,
        "--currency",
        currency,
        "--period-start",
        "2026-09-01",
        "--period-end",
        "2026-09-30",
    ])
}

fn text(value: &Json, key: &str) -> String {
    value.at(key).as_pystr().unwrap().to_utf8().unwrap()
}

#[test]
fn the_example_lists_each_departure_and_exits_2() {
    let output = run(&example(), "AUD");
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let result = pyjson::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(text(&result, "status"), "REVIEW");
    assert_eq!(result.at("lines_in_period").as_plain_int(), Some(&4.into()));
    let listed: Vec<_> = result
        .at("findings")
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            format!(
                "{} {} {} -> usual {}",
                text(item, "code"),
                text(item, "line_id"),
                text(item, "contact"),
                text(item, "usual")
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            "UNUSUAL_ACCOUNT L-104 Lumbridge Power -> usual 610",
            "UNUSUAL_TAX_CODE L-204 Draynor Bank -> usual EXEMPTEXPENSES",
            "UNUSUAL_ACCOUNT_TAX_CODE L-304 Edgeville Express -> usual INPUT",
        ]
    );
}

#[test]
fn lines_coded_as_before_exit_0() {
    let temp = TempDir::new();
    let clean = temp.join("clean.csv");
    let source = std::fs::read_to_string(example()).unwrap();
    let kept: Vec<_> = source
        .lines()
        .filter(|line| {
            !["L-104", "L-204", "L-304"]
                .iter()
                .any(|id| line.contains(id))
        })
        .collect();
    std::fs::write(&clean, kept.join("\n") + "\n").unwrap();
    let output = run(clean.to_str().unwrap(), "AUD");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let result = pyjson::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(text(&result, "status"), "PASS");
}

#[test]
fn another_currency_exits_1() {
    let output = run(&example(), "USD");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("declared --currency 'USD'"), "{error}");
}

#[test]
fn a_missing_listing_exits_1() {
    let temp = TempDir::new();
    let output = run(temp.join("absent.csv").to_str().unwrap(), "AUD");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("Lines does not exist"), "{error}");
}

#[test]
fn a_missing_option_exits_1_with_the_usage() {
    let output = coding(&["--lines", &example(), "--currency", "AUD"]);
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        error.ends_with(
            "close-control coding: error: the following arguments are required: --period-start, --period-end\n"
        ),
        "{error}"
    );
}

#[test]
fn help_names_every_option() {
    let output = coding(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(help, include_str!("../src/help/coding-help.txt"));
}
