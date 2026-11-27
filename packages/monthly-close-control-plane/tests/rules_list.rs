// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use monthly_close_control_plane::models::Control;
use std::process::Output;

const BIN: &str = env!("CARGO_BIN_EXE_close-control");

fn rules(args: &[&str]) -> Output {
    child_profile::command(BIN)
        .arg("rules")
        .args(args)
        .output()
        .unwrap()
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().replace("\r\n", "\n")
}

#[test]
fn list_prints_every_control_with_when_it_runs_what_it_checks_and_its_source() {
    let output = rules(&["list"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty());
    let stdout = text(output.stdout);
    let blocks: Vec<&str> = stdout
        .split_terminator('\n')
        .collect::<Vec<_>>()
        .chunks(4)
        .take(Control::ALL.len())
        .map(|lines| lines[0])
        .collect();
    let ids: Vec<&str> = Control::ALL.iter().map(|c| c.as_str()).collect();
    assert_eq!(blocks, ids);
    assert_eq!(stdout.matches("\n  runs:   ").count(), ids.len());
    assert_eq!(stdout.matches("\n  checks: ").count(), ids.len());
    assert_eq!(stdout.matches("\n  source: ").count(), ids.len());
    assert!(
        stdout.contains("gst_reasonableness\n  runs:   with --gst-mapping and --gst-settlements\n")
    );
    assert!(stdout.ends_with("none approves a close.\n"));
}

#[test]
fn the_action_is_required_and_must_be_list() {
    for (args, message) in [
        (&[][..], "the following arguments are required: action"),
        (
            &["show"][..],
            "argument action: invalid choice: 'show' (choose from 'list')",
        ),
        (&["list", "extra"][..], "unrecognized arguments: extra"),
        (&["list", "--json"][..], "unrecognized arguments: --json"),
    ] {
        let output = rules(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(
            text(output.stderr).trim_end().ends_with(message),
            "{args:?}"
        );
    }
}

#[test]
fn help_matches_the_saved_text() {
    let output = rules(&["--help"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        text(output.stdout),
        include_str!("../src/help/rules-help.txt")
    );
}
