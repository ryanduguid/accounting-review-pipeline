// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `close-control bas`, run on the fabricated example ledger as a reviewer
//! would.

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

fn bas(extra: &[&str]) -> Output {
    child_profile::command(BIN)
        .arg("bas")
        .args([
            "--ledger",
            &example("bas_ledger.csv"),
            "--tax-codes",
            &example("bas_tax_codes.csv"),
            "--currency",
            "AUD",
            "--period-start",
            "2026-07-01",
            "--period-end",
            "2026-09-30",
        ])
        .args(extra)
        .output()
        .unwrap()
}

fn text(value: &Json) -> String {
    value.as_pystr().unwrap().to_utf8().unwrap()
}

#[test]
fn the_example_ledger_drafts_every_label_and_blocks_on_its_unmapped_row() {
    let payroll = example("payroll.csv");
    let output = bas(&["--basis", "accruals", "--payroll", &payroll]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let result = pyjson::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert!(result.at("status").is_str("BLOCKED"));
    let labels: Vec<_> = result
        .at("labels")
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let rows: Vec<_> = item
                .at("rows")
                .as_array()
                .unwrap()
                .iter()
                .map(text)
                .collect();
            format!(
                "{} {} {}",
                text(item.at("label")),
                text(item.at("amount")),
                rows.join(" ")
            )
        })
        .collect();
    assert_eq!(
        labels,
        [
            "G1 8610.00 S-1 S-5 S-2 S-3 S-4 S-6",
            "G2 5000.00 S-3",
            "G3 300.00 S-4",
            "G10 2750.00 P-2",
            "G11 825.00 P-1 P-3 P-4",
            "1A 290.00 S-1 S-2 S-6",
            "1B 280.00 P-1 P-2",
            "W1 8533.08 payroll row 2 payroll row 3 payroll row 4 payroll row 5",
            "W2 1242.00 payroll row 2 payroll row 3 payroll row 4 payroll row 5",
        ]
    );
    let unplaced = &result.at("unplaced").as_array().unwrap()[0];
    assert_eq!(text(unplaced.at("line_id")), "U-1");
    assert!(result.at("findings").as_array().unwrap().is_empty());
    assert_eq!(
        pyjson::canonical(result.at("bas_excluded_rows")),
        "[\"X-1\"]"
    );
}

#[test]
fn a_bad_basis_or_missing_file_exits_1() {
    for args in [
        &["--basis", "payments"][..],
        &["--basis", "cash", "--payroll", "no-such-register.csv"],
    ] {
        let output = bas(args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn help_names_every_option() {
    let output = child_profile::command(BIN)
        .args(["bas", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(help, include_str!("../src/help/bas-help.txt"));
}
