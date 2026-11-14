// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use monthly_close_control_plane::pytext;
use monthly_close_control_plane::{
    drivers,
    engine::{self, ReviewConfig},
    pyjson::{self, Json},
    report, viewer,
};
use std::{fs, path::Path, path::PathBuf};

#[path = "support/temp.rs"]
mod temp;
use temp::TempDir;

fn saved_date_pack(prior: &[&str], current: &[&str]) -> (TempDir, PathBuf, PathBuf) {
    let temp = TempDir::new();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
    let pack = engine::review_close(&ReviewConfig::new(
        examples.join("current_trial_balance.csv"),
        examples.join("prior_trial_balance.csv"),
    ))
    .unwrap();
    let directory = temp.join("pack");
    fs::create_dir(&directory).unwrap();
    for (name, content) in report::PACK_FILE_NAMES
        .into_iter()
        .zip(report::rendered(&pack).unwrap())
    {
        fs::write(directory.join(name), content).unwrap();
    }
    let path = directory.join("close-review-pack.json");
    let mut document = pyjson::loads(&fs::read_to_string(&path).unwrap()).unwrap();
    document.set(
        "prior_report_dates",
        Json::Array(prior.iter().map(|date| Json::str(date)).collect()),
    );
    document.set(
        "current_report_dates",
        Json::Array(current.iter().map(|date| Json::str(date)).collect()),
    );
    fs::write(path, pyjson::pretty(&document)).unwrap();
    let summary_path = directory.join("close-summary.md");
    let mut summary = fs::read_to_string(&summary_path).unwrap();
    for (label, original, dates) in [
        ("Prior", "2026-06-30", prior),
        ("Current", "2026-07-31", current),
    ] {
        let text = dates.join(", ");
        let original = format!("- {label} report date(s): {original}");
        assert_eq!(summary.matches(&original).count(), 1);
        summary = summary.replace(&original, &format!("- {label} report date(s): {text}"));
    }
    fs::write(summary_path, summary).unwrap();
    assert!(viewer::verify_pack(&directory).is_ok());
    let transactions = temp.join("transactions.csv");
    fs::write(
        &transactions,
        "Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit\n",
    )
    .unwrap();
    (temp, directory, transactions)
}

#[test]
fn saved_pack_dates_accept_compact_iso_and_report_exact_invalid_date_refusals() {
    let (_temp, directory, transactions) = saved_date_pack(&["20260630"], &["20260731"]);
    let result = drivers::variance_drivers(&directory, &transactions, "AUD", &1.into()).unwrap();
    assert!(result.at("window").at("after").is_str("2026-06-30"));
    assert!(result.at("window").at("through").is_str("2026-07-31"));
    for (bad, expected) in [
        ("bad", "Invalid isoformat string: 'bad'"),
        ("2026/07/31", "Invalid isoformat string: '2026/07/31'"),
        ("2026-0x-31", "Invalid isoformat string: '2026-0x-31'"),
        ("0000-07-31", "year must be in 1..9999, not 0"),
        ("2026-13-01", "month must be in 1..12, not 13"),
        (
            "2026-02-29",
            "day 29 must be in range 1..28 for month 2 in year 2026",
        ),
    ] {
        for (prior, current) in [([bad], ["2026-07-31"]), (["2026-06-30"], [bad])] {
            let (_temp, directory, transactions) = saved_date_pack(&prior, &current);
            assert_eq!(
                drivers::variance_drivers(&directory, &transactions, "AUD", &1.into()).unwrap_err(),
                expected
            );
        }
    }
}

#[test]
fn saved_pack_drivers_require_one_prior_and_one_current_report_date() {
    for (prior, current) in [
        (vec![], vec!["2026-07-31"]),
        (vec!["2026-06-30", "2026-06-29"], vec!["2026-07-31"]),
        (vec!["2026-06-30"], vec![]),
        (vec!["2026-06-30"], vec!["2026-07-31", "2026-08-31"]),
    ] {
        let (_temp, directory, transactions) = saved_date_pack(&prior, &current);
        assert_eq!(
            drivers::variance_drivers(&directory, &transactions, "AUD", &1.into()).unwrap_err(),
            "The pack must carry one prior and one current report date."
        );
    }
}

#[test]
fn saved_pack_without_evidence_refuses_an_orphan_calculation_table() {
    let (_temp, directory, _transactions) = saved_date_pack(&["2026-06-30"], &["2026-07-31"]);
    let verified = viewer::verify_pack(&directory).unwrap();
    assert!(verified.document.get("calculation_evidence").is_none());
    let path = directory.join("close-summary.md");
    let summary = fs::read_to_string(&path).unwrap();
    fs::write(
        path,
        format!(
            "{summary}\n| Calculation | Status | Period | Figures | Relied on | Entry digest |\n"
        ),
    )
    .unwrap();
    let expected = "close-summary.md: a calculation-evidence table is present while close-review-pack.json carries no calculation_evidence member";
    assert_eq!(viewer::verify_pack(&directory).unwrap_err(), expected);
    assert_eq!(
        viewer::render_review_sheet(&directory).unwrap_err(),
        expected
    );
}

#[test]
fn lowercase_preserves_python_unicode_context_and_expansion() {
    for (input, expected) in [
        ("", ""),
        ("AbC", "abc"),
        ("Σ", "σ"),
        ("ΟΣ", "ος"),
        ("ΟΣΑ", "οσα"),
        ("A\u{301}Σ\u{301}", "a\u{301}ς\u{301}"),
        ("AΣ\u{301}B", "aσ\u{301}b"),
        ("A Σ", "a σ"),
        ("AΣ B", "aς b"),
        ("İ", "i\u{307}"),
    ] {
        assert_eq!(pytext::lower(input), expected, "{input:?}");
    }
}
