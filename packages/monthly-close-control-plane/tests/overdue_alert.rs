// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `close-control overdue`, run on the fabricated example register as a
//! reviewer would.

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use monthly_close_control_plane::pyjson::{self, Json};
use std::{path::PathBuf, process::Output};

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn example() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("receivables.csv")
        .to_str()
        .unwrap()
        .to_owned()
}

fn overdue(invoices: &str, as_at: &str) -> Output {
    child_profile::command(BIN)
        .args(["overdue", "--invoices", invoices, "--currency", "AUD"])
        .args(["--as-at", as_at])
        .output()
        .unwrap()
}

fn text(value: &Json) -> String {
    value.as_pystr().unwrap().to_utf8().unwrap()
}

fn result(output: &Output) -> Json {
    pyjson::loads(&String::from_utf8(output.stdout.clone()).unwrap()).unwrap()
}

/// The example register without the rows that mention `drop`.
fn without(drop: &str, name: &str) -> PathBuf {
    let temp = std::env::temp_dir().join(format!(
        "closecontrol-overdue-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let path = temp.join("invoices.csv");
    let source = std::fs::read_to_string(example()).unwrap();
    let kept: Vec<_> = source.lines().filter(|line| !line.contains(drop)).collect();
    std::fs::write(&path, kept.join("\n") + "\n").unwrap();
    path
}

#[test]
fn the_example_lists_each_overdue_invoice_against_its_customer() {
    let output = overdue(&example(), "2026-09-30");
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let result = result(&output);
    assert!(result.at("status").is_str("BLOCKED"));
    let listed: Vec<String> = result
        .at("overdue")
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let history = entry.at("history");
            format!(
                "{} {} {} {}",
                text(entry.at("invoice_id")),
                text(entry.at("code")),
                pyjson::canonical(entry.at("days_overdue")),
                text(history.at("threshold_days"))
            )
        })
        .collect();
    assert_eq!(
        listed,
        [
            "INV-3008 WITHIN_HISTORY 30 48.0",
            "INV-3009 OVERDUE_AGAINST_HISTORY 20 12.0",
            "INV-3010 NOT_ENOUGH_HISTORY 15 12.0",
            "INV-3011 WITHIN_HISTORY 10 12.0",
        ]
    );
    let lumbridge = result.at("overdue").as_array().unwrap()[1].at("history");
    let ids: Vec<String> = lumbridge
        .at("paid_invoice_ids")
        .as_array()
        .unwrap()
        .iter()
        .map(text)
        .collect();
    assert_eq!(ids, ["INV-3001", "INV-3003", "INV-3006"]);
    assert_eq!(text(lumbridge.at("median_days_late")), "5.0");
    assert_eq!(text(lumbridge.at("spread_days")), "1.0");
    assert_eq!(pyjson::canonical(result.at("open_count")), "5");
    let again = overdue(&example(), "2026-09-30");
    assert_eq!(output.stdout, again.stdout, "output must be deterministic");
}

#[test]
fn without_the_thin_history_it_is_review_and_earlier_it_passes() {
    let path = without("Draynor", "review");
    let output = overdue(path.to_str().unwrap(), "2026-09-30");
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(result(&output).at("status").is_str("REVIEW"));
    let output = overdue(path.to_str().unwrap(), "2026-09-20");
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(result(&output).at("status").is_str("PASS"));
}

#[test]
fn a_bad_date_or_missing_file_exits_1() {
    for (invoices, as_at) in [
        (example(), "2026-09-31"),
        (example(), "2026-09-01"),
        ("no-such-register.csv".to_owned(), "2026-09-30"),
    ] {
        let output = overdue(&invoices, as_at);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn help_names_every_option() {
    let output = child_profile::command(BIN)
        .args(["overdue", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(help, include_str!("../src/help/overdue-help.txt"));
}
