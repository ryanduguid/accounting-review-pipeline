// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `close-control prepost`, run on the fabricated example drafts against the
//! fabricated posted listing.

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use monthly_close_control_plane::pyjson::{self, Json};
use std::{path::PathBuf, process::Output};

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn example(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(name)
        .to_str()
        .unwrap()
        .to_owned()
}

fn prepost(drafts: &str, end: &str) -> Output {
    child_profile::command(BIN)
        .args(["prepost", "--drafts", drafts, "--posted"])
        .arg(example("journal_entries.csv"))
        .args(["--currency", "AUD", "--period-start", "2026-08-01"])
        .args(["--period-end", end])
        .output()
        .unwrap()
}

fn text(value: &Json) -> String {
    value.as_pystr().unwrap().to_utf8().unwrap()
}

fn result(output: &Output) -> Json {
    pyjson::loads(&String::from_utf8(output.stdout.clone()).unwrap()).unwrap()
}

/// Each draft's id, score and flag codes.
fn summary(result: &Json) -> Vec<String> {
    let journals = result.at("journals").as_array().unwrap();
    journals
        .iter()
        .map(|entry| {
            let codes: Vec<String> = entry
                .at("flags")
                .as_array()
                .unwrap()
                .iter()
                .map(|flag| text(flag.at("code")))
                .collect();
            format!(
                "{} {} {}",
                text(entry.at("transaction_id")),
                pyjson::canonical(entry.at("risk_score")),
                codes.join(",")
            )
        })
        .collect()
}

/// The example drafts reduced to the journals named in `keep`.
fn only(keep: &[&str], name: &str) -> PathBuf {
    let temp = std::env::temp_dir().join(format!(
        "closecontrol-prepost-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let path = temp.join("drafts.csv");
    let source = std::fs::read_to_string(example("prepost_drafts.csv")).unwrap();
    let kept: Vec<_> = source
        .lines()
        .enumerate()
        .filter(|(index, line)| *index == 0 || keep.iter().any(|id| line.contains(id)))
        .map(|(_, line)| line)
        .collect();
    std::fs::write(&path, kept.join("\n") + "\n").unwrap();
    path
}

#[test]
fn the_example_drafts_raise_each_flag_and_block_on_the_unbalanced_one() {
    let output = prepost(&example("prepost_drafts.csv"), "2026-08-31");
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let result = result(&output);
    assert!(result.at("status").is_str("BLOCKED"));
    assert_eq!(
        summary(&result),
        [
            "D-1 3 DUPLICATE_DRAFT",
            "D-2 3 DUPLICATE_OF_POSTED",
            "D-3 3 UNBALANCED",
            "D-4 5 UNUSUAL_ACCOUNT_PAIR,PERIOD_END,NO_NARRATION,ROUND_AMOUNT",
            "D-5 4 OUTSIDE_PERIOD,WEEKEND",
            "D-6 3 DUPLICATE_DRAFT",
            "D-7 0 ",
        ]
    );
    let first = &result.at("journals").as_array().unwrap()[0];
    let flag = &first.at("flags").as_array().unwrap()[0];
    assert_eq!(text(flag.at("suggested_fix")), "Keep one of the drafts.");
    assert_eq!(pyjson::canonical(flag.at("weight")), "3");
    let again = prepost(&example("prepost_drafts.csv"), "2026-08-31");
    assert_eq!(output.stdout, again.stdout, "output must be deterministic");
}

#[test]
fn flags_without_an_unbalanced_draft_are_review_and_none_pass() {
    let review = only(&[",D-4,"], "review");
    let output = prepost(review.to_str().unwrap(), "2026-08-31");
    std::fs::remove_dir_all(review.parent().unwrap()).unwrap();
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(result(&output).at("status").is_str("REVIEW"));

    let clean = only(&[",D-7,"], "pass");
    let output = prepost(clean.to_str().unwrap(), "2026-08-31");
    std::fs::remove_dir_all(clean.parent().unwrap()).unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let result = result(&output);
    assert!(result.at("status").is_str("PASS"));
    assert_eq!(pyjson::canonical(result.at("posted_count")), "7");
}

#[test]
fn a_reversed_period_or_missing_file_exits_1() {
    for (drafts, end) in [
        (example("prepost_drafts.csv"), "2026-07-31"),
        ("no-such-drafts.csv".to_owned(), "2026-08-31"),
    ] {
        let output = prepost(&drafts, end);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn help_names_every_option() {
    let output = child_profile::command(BIN)
        .args(["prepost", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(help, include_str!("../src/help/prepost-help.txt"));
}
