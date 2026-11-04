// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Both independently released loaders enforce the same calendar-date contract.

use super::*;

const NON_CALENDAR: [&str; 2] = ["20260923", "2026-W39-3"];

type Probe = fn(&Json) -> Json;

fn probes() -> [(&'static str, Probe); 2] {
    [
        ("Monthly Close Controls", monthly),
        ("Workpaper Review Gate", ready),
    ]
}

#[test]
fn non_calendar_trial_balance_date_is_rejected() {
    for (consumer, probe) in probes() {
        for value in NON_CALENDAR {
            let folder = Scratch::new();
            let source = folder.0.join("tb.csv");
            let content = format!(
                "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n\
                 {value},Sample entity,Assets,one,Cash,100,0,0,0,0\n"
            );
            std::fs::write(&source, content).unwrap();
            assert!(
                is_error(&probe(&trial_balance(&source))),
                "{consumer} {value}"
            );
        }
    }
}

#[test]
fn non_calendar_review_date_is_rejected() {
    for (consumer, probe) in probes() {
        for value in NON_CALENDAR {
            let folder = Scratch::new();
            let source = folder.0.join("review.json");
            let note = Json::object([
                ("reviewer_initials", Json::str("AB")),
                ("reviewed_on", Json::str(value)),
                ("comment", Json::str("Fabricated review.")),
            ]);
            std::fs::write(&source, pyjson::dumps(&note, false)).unwrap();
            let request = file_request("review_note", None, &source);
            assert!(is_error(&probe(&request)), "{consumer} {value}");
        }
    }
}
