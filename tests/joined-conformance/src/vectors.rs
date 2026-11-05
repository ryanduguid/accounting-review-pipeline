// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! What the Xero contract's fixtures leave open: quoted CSV, leading-zero
//! account codes, Unicode text and numeric spellings. Each consumer keeps the
//! outcome `fixtures/expected_results.json` records for it, so a loader change
//! that moves one of these boundaries fails here. The Xero Ledger Review Gate
//! meets the same files in `test_gateway_satisfies_the_joined_vectors`.

use super::*;
use monthly_close_control_plane::decimal::Decimal;
use std::collections::BTreeSet;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
const CONTRACT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/xero-trial-balance-v1"
);
/// The order of each row in a scenario's `amounts`.
const AMOUNT_FIELDS: [&str; 4] = ["debit", "credit", "ytd_debit", "ytd_credit"];

fn scenarios_in(directory: &str) -> Vec<Json> {
    let path = Path::new(directory).join("expected_results.json");
    let expected = pyjson::loads(&std::fs::read_to_string(path).unwrap()).unwrap();
    expected.at("scenarios").as_array().unwrap().to_vec()
}

fn scenarios() -> Vec<Json> {
    scenarios_in(FIXTURES)
}

/// One consumer's answer against its recorded outcome.
fn assert_outcome(answer: &Json, outcome: &Json, scenario: &Json) {
    let name = text(scenario.at("id"));
    if !matches!(outcome.at("accept"), Json::Bool(true)) {
        let error = text(answer.at("error"));
        assert!(
            error.contains(&text(outcome.at("error_contains"))),
            "{name}: {error}"
        );
        return;
    }
    assert!(!is_error(answer), "{name}: {answer:?}");
    let rows = answer.at("ok").at("rows").as_array().unwrap();
    let tenant = text(scenario.at("tenant"));
    assert!(
        rows.iter().all(|row| text(row.at("tenant")) == tenant),
        "{name}"
    );
    let codes: Vec<String> = rows
        .iter()
        .map(|row| text(row.at("account_code")))
        .collect();
    let want: Vec<String> = scenario
        .at("account_codes")
        .as_array()
        .unwrap()
        .iter()
        .map(text)
        .collect();
    assert_eq!(codes, want, "{name}");
    let amounts = scenario.at("amounts").as_array().unwrap();
    assert_eq!(rows.len(), amounts.len(), "{name}");
    for (row, want) in rows.iter().zip(amounts) {
        for (field, want) in AMOUNT_FIELDS.iter().zip(want.as_array().unwrap()) {
            let got = Decimal::parse(&text(row.at(field))).unwrap();
            let want = Decimal::parse(&text(want)).unwrap();
            assert_eq!(got, want, "{name} {field}");
        }
    }
}

#[test]
fn the_fixture_folder_and_the_scenarios_name_the_same_files() {
    let on_disk: BTreeSet<String> = std::fs::read_dir(FIXTURES)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".csv"))
        .collect();
    let named: BTreeSet<String> = scenarios().iter().map(|s| text(s.at("fixture"))).collect();
    assert_eq!(on_disk, named);
}

fn assert_review_commands(fixture: &Path, scenario: &Json) {
    let outcome = scenario.at("review_commands");

    let close = monthly(&file_request("tb_integrity", None, fixture));
    assert_outcome(&close, outcome, scenario);

    let request = Json::object([
        ("op", Json::str("tb_integrity")),
        ("path", path_text(fixture)),
    ]);
    let readiness = ready(&request);
    assert_outcome(&readiness, outcome, scenario);

    if matches!(outcome.at("accept"), Json::Bool(true)) {
        let findings = close.at("ok").at("findings").as_array().unwrap();
        assert!(findings.is_empty(), "{fixture:?}");
        for side in ["movement", "ytd"] {
            let value = text(readiness.at("ok").at(side));
            assert!(
                Decimal::parse(&value).unwrap().is_zero(),
                "{fixture:?} {side}"
            );
        }
    }
}

#[test]
fn the_review_commands_keep_the_recorded_outcomes() {
    for scenario in scenarios() {
        assert_review_commands(
            &Path::new(FIXTURES).join(text(scenario.at("fixture"))),
            &scenario,
        );
    }
}

#[test]
fn the_review_commands_keep_the_contract_rejection_outcomes() {
    let recorded: Vec<Json> = scenarios_in(CONTRACT)
        .into_iter()
        .filter(|scenario| scenario.get("review_commands").is_some())
        .collect();
    assert_eq!(recorded.len(), 4);
    for scenario in recorded {
        let name = text(scenario.at("fixture"));
        assert_review_commands(&Path::new(CONTRACT).join("fixtures").join(name), &scenario);
    }
}
