// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `--ignore-file`: out-of-period ledger rows are listed with their reasons,
//! never silently dropped, and a malformed ignore file fails the run.

use super::*;

const IGNORE: &str = "ignore.csv";

/// The example with 2 ledger rows dated before the ledger period starts.
fn fixture_with_early_rows() -> Fixture {
    let fixture = Fixture::new();
    fixture.edit_csv("ledger.csv", |rows| {
        for (id, amount) in [("L5", "500"), ("L6", "75.25")] {
            let mut early = rows[0].clone();
            put(&mut early, "ledger_id", id);
            put(&mut early, "date", "2026-06-20");
            put(&mut early, "amount", amount);
            put(&mut early, "cash_paid", amount);
            rows.push(early);
        }
    });
    fixture
}

fn ignore(fixture: &Fixture, body: &str) {
    std::fs::write(fixture.root().join(IGNORE), format!("ledger_id,reason\n{body}")).unwrap();
}

/// `main(["--input", ..., "--output", "out", "--ignore-file", ...])`: status, stdout and stderr.
fn run(fixture: &Fixture, ignore_path: &str) -> (i32, String, String) {
    let args: Vec<String> = ["--input", SOURCE, "--output", "out", "--ignore-file", ignore_path]
        .map(String::from)
        .to_vec();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let status = crate::cli::main("grant-workpaper", &args, fixture.root(), &mut stdout, &mut stderr);
    (
        status,
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
    )
}

#[test]
fn every_ignored_row_is_shown_with_its_reason_and_left_out_of_the_totals() {
    let fixture = fixture_with_early_rows();
    ignore(
        &fixture,
        "L5,Prior-year invoice paid in June\nL6,\"Accrued last year, reversed\"\n",
    );
    let (status, stdout, stderr) = run(&fixture, IGNORE);
    assert_eq!(status, 2, "{stderr}");
    assert_eq!(
        stdout,
        "grant-workpaper: REVIEW; 5 review items; 2 out-of-period ledger rows ignored\n"
    );
    let out = fixture.root().join("out");
    let written = pyjson::loads_bytes(&std::fs::read(out.join("workpaper.json")).unwrap()).unwrap();
    let shown: Vec<[String; 3]> = rows(&written, "ignored_items")
        .iter()
        .map(|row| ["ledger_id", "amount", "reason"].map(|key| text(row.at(key))))
        .collect();
    assert_eq!(
        shown,
        [
            ["L5", "500", "Prior-year invoice paid in June"],
            ["L6", "75.25", "Accrued last year, reversed"],
        ]
    );
    // The totals and the tie-out are the example's own.
    assert!(written.at("ledger_total").is_str("6500"));
    let tied: Vec<String> = rows(&written, "ledger_tieout")
        .iter()
        .map(|r| text(r.at("ledger_id")))
        .collect();
    assert_eq!(tied, ["L1", "L2", "L3", "L4"]);
    assert!(written.at("source_sha256").get("ignore_file").is_some());
    let csv = read_csv(&out.join("ignored_items.csv"));
    assert_eq!(csv.len(), 2);
    assert_eq!(get(&csv[1], "reason"), "Accrued last year, reversed");
    let report = std::fs::read_to_string(out.join("workpaper.md"))
        .unwrap()
        .replace("\r\n", "\n");
    assert!(report.contains("## Ignored out-of-period ledger rows"), "{report}");
    assert!(
        report.contains("- L5 (2026\\-06\\-20, 500): Prior\\-year invoice paid in June"),
        "{report}"
    );
    assert!(
        report.contains("- L6 (2026\\-06\\-20, 75\\.25): Accrued last year, reversed"),
        "{report}"
    );
}

#[test]
fn an_empty_ignore_file_still_says_so_on_the_workpaper() {
    let fixture = Fixture::new();
    ignore(&fixture, "");
    let (status, stdout, _) = run(&fixture, IGNORE);
    assert_eq!(status, 2);
    assert!(stdout.ends_with("; 0 out-of-period ledger rows ignored\n"), "{stdout}");
    let report = std::fs::read_to_string(fixture.root().join("out/workpaper.md"))
        .unwrap()
        .replace("\r\n", "\n");
    assert!(report.contains("ledger rows\n\nThe ignore file leaves these rows out of every total above.\n\nNone.\n"));
    assert_eq!(read_csv(&fixture.root().join("out/ignored_items.csv")).len(), 0);
}

#[test]
fn without_the_option_nothing_changes_and_an_early_row_still_fails() {
    let fixture = Fixture::new();
    let result = fixture.review().unwrap();
    assert!(result.get("ignored_items").is_none());
    assert!(result.at("source_sha256").get("ignore_file").is_none());
    let fixture = fixture_with_early_rows();
    let (status, stderr) = fixture.main("out");
    assert_eq!(status, 1);
    assert_eq!(
        stderr,
        "grant-workpaper: Ledger rows must fall inside the declared ledger period\n"
    );
}

/// Each malformed ignore file (`None`: no file at all) and the start of its error.
const MALFORMED: [(&str, Option<&[u8]>, &str); 9] = [
    ("missing file", None, "Ignore file: [Errno 2] No such file or directory"),
    (
        "wrong headings",
        Some(b"id,reason\nL5,x\n"),
        "Ignore file: Expected exact CSV headings: ledger_id reason",
    ),
    (
        "duplicate",
        Some(b"ledger_id,reason\nL5,a\nL5,b\nL6,c\n"),
        "Ignore file: Missing or duplicate ledger_id: 'L5'",
    ),
    (
        "blank reason",
        Some(b"ledger_id,reason\nL5,  \nL6,c\n"),
        "Ignore file: Reason must be visible text",
    ),
    (
        "unknown row",
        Some(b"ledger_id,reason\nL5,a\nL6,b\nL9,c\n"),
        "Ignore file: no ledger row 'L9'",
    ),
    (
        "in period",
        Some(b"ledger_id,reason\nL5,a\nL6,b\nL1,c\n"),
        "Ignore file: ledger row 'L1' is inside the declared ledger period",
    ),
    (
        "one left out",
        Some(b"ledger_id,reason\nL5,a\n"),
        "Ledger rows must fall inside the declared ledger period",
    ),
    ("bad bytes", Some(b"ledger_id,reason\nL5,\xff\n"), "Ignore file: "),
    (
        "extra field",
        Some(b"ledger_id,reason\nL5,a,b\nL6,c\n"),
        "Ignore file: Every CSV row must have the exact field count",
    ),
];

#[test]
fn a_malformed_ignore_file_fails_the_run_and_publishes_nothing() {
    for (label, body, expected) in MALFORMED {
        let fixture = fixture_with_early_rows();
        let path = match body {
            Some(body) => {
                std::fs::write(fixture.root().join(IGNORE), body).unwrap();
                IGNORE
            }
            None => "absent.csv",
        };
        let (status, stdout, stderr) = run(&fixture, path);
        assert_eq!(status, 1, "{label}: {stderr}");
        assert!(stdout.is_empty(), "{label}");
        assert!(
            stderr.starts_with(&format!("grant-workpaper: {expected}")),
            "{label}: {stderr}"
        );
        assert!(!fixture.root().join("out").exists(), "{label} published a workpaper");
    }
}

#[test]
fn an_allocation_to_an_ignored_row_fails_the_run() {
    let fixture = fixture_with_early_rows();
    fixture.edit_csv("allocations.csv", |rows| put(&mut rows[4], "ledger_id", "L5"));
    ignore(&fixture, "L5,a\nL6,b\n");
    let (status, _, stderr) = run(&fixture, IGNORE);
    assert_eq!(status, 1);
    assert_eq!(
        stderr,
        "grant-workpaper: Allocation references an ignored ledger row: 'L5'\n"
    );
    assert!(!fixture.root().join("out").exists());
}

#[test]
fn an_ignored_row_still_needs_valid_amounts() {
    let fixture = fixture_with_early_rows();
    fixture.edit_csv("ledger.csv", |rows| put(&mut rows[4], "cash_paid", "900"));
    ignore(&fixture, "L5,a\nL6,b\n");
    let (status, _, stderr) = run(&fixture, IGNORE);
    assert_eq!(status, 1);
    assert!(stderr.contains("cash paid cannot exceed the expense"), "{stderr}");
}
