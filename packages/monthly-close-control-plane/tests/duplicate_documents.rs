// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `close-control duplicates`, run on the fabricated example register as a
//! reviewer would.

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use monthly_close_control_plane::pyjson::{self, Json};
use std::{path::PathBuf, process::Output};

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn example() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/duplicates.csv")
        .to_str()
        .unwrap()
        .to_owned()
}

fn duplicates(args: &[&str]) -> Output {
    child_profile::command(BIN)
        .arg("duplicates")
        .args(args)
        .output()
        .unwrap()
}

fn run(currency: &str) -> Output {
    duplicates(&[
        "--documents",
        &example(),
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
fn the_example_register_lists_each_group_and_exits_2() {
    let output = run("AUD");
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let result = pyjson::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(text(&result, "status"), "REVIEW");
    assert_eq!(text(&result, "tenant"), "Varrock Ventures Pty Ltd");
    assert_eq!(result.at("document_count").as_plain_int(), Some(&13.into()));
    let listed: Vec<_> = result
        .at("findings")
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let ids: Vec<_> = item
                .at("documents")
                .as_array()
                .unwrap()
                .iter()
                .map(|document| text(document, "document_id"))
                .collect();
            format!(
                "{} {} [{}]",
                text(item, "code"),
                text(item, "supplier"),
                ids.join(" ")
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            "DUPLICATE_REFERENCE Falador Freight [B-201 B-202]",
            "DUPLICATE_BILL Draynor Supplies [B-301 B-302]",
            "DUPLICATE_PAYMENT Draynor Supplies [P-501 P-502]",
            "MISSING_RECURRING_BILL Lumbridge Power []",
        ]
    );
}

#[test]
fn a_register_with_nothing_listed_exits_0() {
    let temp = std::env::temp_dir().join(format!("closecontrol-dup-{}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    let clean = temp.join("clean.csv");
    let source = std::fs::read_to_string(example()).unwrap();
    let kept: Vec<_> = source
        .lines()
        .filter(|line| line.starts_with("Tenant,") || line.contains(",Varrock Rent,"))
        .collect();
    std::fs::write(&clean, kept.join("\n") + "\n").unwrap();
    let output = duplicates(&[
        "--documents",
        clean.to_str().unwrap(),
        "--currency",
        "AUD",
        "--period-start",
        "2026-09-01",
        "--period-end",
        "2026-09-30",
    ]);
    std::fs::remove_dir_all(&temp).unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let result = pyjson::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(text(&result, "status"), "PASS");
}

#[test]
fn another_currency_exits_1() {
    let output = run("USD");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("declared --currency 'USD'"), "{error}");
}

#[test]
fn a_missing_option_exits_1_with_the_usage() {
    let output = duplicates(&["--documents", &example(), "--currency", "AUD"]);
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        error.ends_with(
            "close-control duplicates: error: the following arguments are required: --period-start, --period-end\n"
        ),
        "{error}"
    );
}

#[test]
fn help_names_every_option() {
    let output = duplicates(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(help, include_str!("../src/help/duplicates-help.txt"));
}
