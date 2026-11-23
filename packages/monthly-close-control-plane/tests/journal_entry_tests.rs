// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `close-control jet`, run on the fabricated example listing as a reviewer
//! would.

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use monthly_close_control_plane::pyjson::{self, Json};
use std::{path::PathBuf, process::Output};

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn example() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/journal_entries.csv")
}

fn jet(args: &[&str]) -> Output {
    child_profile::command(BIN)
        .arg("jet")
        .args(args)
        .output()
        .unwrap()
}

fn text(value: &Json, key: &str) -> String {
    value.at(key).as_pystr().unwrap().to_utf8().unwrap()
}

#[test]
fn the_example_listing_lists_each_characteristic_and_exits_2() {
    let example = example();
    let path = example.to_str().unwrap();
    let output = jet(&[
        "--transactions",
        path,
        "--currency",
        "AUD",
        "--period-end",
        "2026-07-31",
    ]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let result = pyjson::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(text(&result, "status"), "REVIEW");
    assert_eq!(text(&result, "tenant"), "Varrock Ventures Pty Ltd");
    assert_eq!(result.at("journal_count").as_plain_int(), Some(&7.into()));
    let listed: Vec<_> = result
        .at("findings")
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let account = match item.at("account_id") {
                Json::Null => String::new(),
                _ => format!(" {}", text(item, "account_id")),
            };
            format!(
                "{} {}{account}",
                text(item, "test"),
                text(item, "transaction_id")
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            "unbalanced_journal J-1005",
            "post_closing J-1007",
            "period_end J-1006",
            "no_description J-1006",
            "weekend J-1004",
            "round_amount J-1006 680",
            "round_amount J-1006 880",
            "seldom_used_account J-1006 680",
            "seldom_used_account J-1006 880",
        ]
    );
    assert_eq!(result.at("benford").at("ran").as_bool(), Some(false));
}

#[test]
fn a_clean_listing_exits_0_and_bad_input_exits_1() {
    let example = example();
    let path = example.to_str().unwrap();
    let output = jet(&[
        "--transactions",
        path,
        "--currency",
        "USD",
        "--period-end",
        "2026-07-31",
    ]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("not the declared --currency 'USD'"),
        "{error}"
    );

    let temp = std::env::temp_dir().join(format!("closecontrol-jet-{}", std::process::id()));
    std::fs::create_dir_all(&temp).unwrap();
    let clean = temp.join("clean.csv");
    let source = std::fs::read_to_string(&example).unwrap();
    let kept: Vec<_> = source
        .lines()
        .filter(|line| {
            line.starts_with("Tenant,") || line.contains(",J-1001,") || line.contains(",J-1003,")
        })
        .collect();
    std::fs::write(&clean, kept.join("\n") + "\n").unwrap();
    let output = jet(&[
        "--transactions",
        clean.to_str().unwrap(),
        "--currency",
        "AUD",
        "--period-end",
        "2026-07-31",
    ]);
    std::fs::remove_dir_all(&temp).unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let result = pyjson::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(text(&result, "status"), "PASS");

    let output = jet(&["--transactions", path, "--currency", "AUD"]);
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8(output.stderr)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        error.ends_with(
            "close-control jet: error: the following arguments are required: --period-end\n"
        ),
        "{error}"
    );
}

#[test]
fn help_names_every_option() {
    let output = jet(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(help, include_str!("../src/help/jet-help.txt"));
}
