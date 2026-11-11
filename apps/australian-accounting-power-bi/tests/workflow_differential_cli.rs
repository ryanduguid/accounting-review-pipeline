// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! The differential tool refuses what it cannot run, with a failing status.

#![cfg(feature = "probe")]

use std::process::Command;

#[test]
fn an_unknown_option_prints_the_usage_and_fails() {
    let output = Command::new(env!("CARGO_BIN_EXE_workflow-differential"))
        .arg("--unknown")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim_end(),
        "Use --python PATH --oracle APP --bin-dir PATH [--seed INTEGER]."
    );
}
