//! Port of tests/test_contributions.py.

use super::*;
use crate::workpaper::faults;

const HEADER: &str = "contribution_id,grant_id,date,kind,amount,evidence\n";

fn contributions(fixture: &Fixture, body: &str) {
    std::fs::write(fixture.source().join("contributions.csv"), format!("{HEADER}{body}")).unwrap();
}

fn contribution_rows(result: &Json) -> Vec<[String; 4]> {
    rows(result, "contributions")
        .iter()
        .map(|r| ["grant_id", "interest", "cash_contribution", "in_kind"].map(|k| text(r.at(k))))
        .collect()
}

fn hashed(result: &Json) -> bool {
    result.at("source_sha256").get("contributions.csv").is_some()
}

/// The refusal through `review` and through the CLI, which leaves no output.
fn assert_input_failure(fixture: &Fixture) {
    refused(fixture.review());
    let (status, stderr) = fixture.main("out");
    assert_eq!(status, 1);
    assert!(stderr.contains("grant-workpaper:"), "{stderr}");
    assert!(!fixture.root().join("out").exists());
}

#[test]
fn test_without_the_file_every_grant_reports_nil() {
    let fixture = Fixture::new();
    let result = fixture.review().unwrap();
    assert_eq!(
        contribution_rows(&result),
        [
            ["G-A", "0", "0", "0"].map(String::from),
            ["G-B", "0", "0", "0"].map(String::from)
        ]
    );
    assert!(!hashed(&result));
}

#[test]
fn test_contributions_are_reported_and_leave_cash_and_unspent_funding_alone() {
    let fixture = Fixture::new();
    let before = pyjson::dumps_indent2(fixture.review().unwrap().at("funding_movements"));
    contributions(
        &fixture,
        "C1,G-A,2026-09-30,interest,41.25,Fabricated bank statement interest\n\
         C2,G-A,2027-03-31,interest,38.75,Fabricated bank statement interest\n\
         C3,G-B,2026-11-01,cash_contribution,2500,Fabricated council co-contribution\n\
         C4,G-B,2026-12-10,in_kind,1200,\n",
    );
    let result = fixture.review().unwrap();
    assert_eq!(
        contribution_rows(&result),
        [
            ["G-A", "80.00", "0", "0"].map(String::from),
            ["G-B", "0", "2500", "1200"].map(String::from)
        ]
    );
    assert_eq!(pyjson::dumps_indent2(result.at("funding_movements")), before);
    assert!(findings(&result).contains(&("MISSING_CONTRIBUTION_EVIDENCE".into(), "C4".into())));
    assert!(hashed(&result));
}

#[test]
fn test_invalid_contributions_are_refused() {
    let fixture = Fixture::new();
    for (body, message) in [
        ("C1,G-X,2026-09-30,interest,10,E\n", "declared grant"),
        ("C1,G-A,2025-09-30,interest,10,E\n", "ledger period"),
        ("C1,G-A,2026-09-30,donation,10,E\n", "kind must be one of"),
        ("C1,G-A,2026-09-30,interest,0,E\n", "positive"),
        (
            "C1,G-A,2026-09-30,interest,10,E\nC1,G-A,2026-10-30,interest,5,E\n",
            "duplicate",
        ),
    ] {
        contributions(&fixture, body);
        let refusal = refused(fixture.review());
        assert!(refusal.contains(message), "{body}: {refusal}");
    }
}

#[test]
fn test_dangling_contribution_link_is_refused() {
    let fixture = Fixture::new();
    if symlink_file(
        &fixture.source().join("missing.csv"),
        &fixture.source().join("contributions.csv"),
    )
    .is_none()
    {
        return;
    }
    assert_input_failure(&fixture);
}

#[test]
fn test_readable_contribution_link_is_included() {
    let fixture = Fixture::new();
    let target = fixture.root().join("supplement.csv");
    std::fs::write(&target, format!("{HEADER}C1,G-A,2026-09-30,interest,12.50,E\n")).unwrap();
    if symlink_file(&target, &fixture.source().join("contributions.csv")).is_none() {
        return;
    }
    let result = fixture.review().unwrap();
    assert_eq!(contribution_rows(&result)[0][1], "12.50");
    assert!(hashed(&result));
}

#[test]
fn test_contribution_inspection_error_is_not_optional_absence() {
    let fixture = Fixture::new();
    contributions(&fixture, "C1,G-A,2026-09-30,interest,12.50,E\n");
    let _hook = faults::install(|point, _| match point {
        "inspect" => Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "fabricated inspection failure",
        )),
        _ => Ok(()),
    });
    assert_input_failure(&fixture);
}

#[test]
fn test_present_contribution_read_errors_are_refused() {
    let fixture = Fixture::new();
    contributions(&fixture, "C1,G-A,2026-09-30,interest,12.50,E\n");
    for kind in [std::io::ErrorKind::PermissionDenied, std::io::ErrorKind::NotFound] {
        let _hook = faults::install(move |point, _| match point {
            "read" => Err(std::io::Error::new(kind, "fabricated read failure")),
            _ => Ok(()),
        });
        assert_input_failure(&fixture);
    }
}

#[test]
fn test_the_csv_output_lists_every_grant() {
    let fixture = Fixture::new();
    contributions(&fixture, "C1,G-A,2026-09-30,interest,12.50,Fabricated interest\n");
    assert_eq!(fixture.main("out").0, 2);
    let output = fixture.root().join("out");
    let listed: Vec<(String, String)> = read_csv(&output.join("contributions.csv"))
        .iter()
        .map(|r| (get(r, "grant_id").to_string(), get(r, "interest").to_string()))
        .collect();
    assert_eq!(listed, [("G-A".into(), "12.50".into()), ("G-B".into(), "0".into())]);
    let markdown = std::fs::read_to_string(output.join("workpaper.md")).unwrap();
    assert!(markdown.contains("contributions.csv reports interest"));
}
