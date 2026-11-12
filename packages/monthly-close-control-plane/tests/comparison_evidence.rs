// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use monthly_close_control_plane::{
    comparison::{self, CompareConfig},
    decimal::Decimal,
    engine::{self, ReviewConfig},
    pyjson::{self, Json},
    report, sha256, viewer,
};
use std::{collections::BTreeMap, fs, path::Path, time::SystemTime};

#[path = "support/temp.rs"]
mod temp;
use temp::TempDir;

fn trial_balance(date: &str, balance: &str) -> String {
    format!(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n\
         {date},Fabricated Varrock,Assets,A,Bank,100,0,0,{balance},0\n\
         {date},Fabricated Varrock,Revenue,R,Revenue,400,0,0,0,{balance}\n"
    )
}

fn write_pack(config: &ReviewConfig, directory: &Path) {
    let pack = engine::review_close(config).unwrap();
    let files = report::rendered(&pack).unwrap();
    fs::create_dir(directory).unwrap();
    for (name, bytes) in report::PACK_FILE_NAMES.into_iter().zip(files) {
        fs::write(directory.join(name), bytes).unwrap();
    }
    viewer::verify_pack(directory).unwrap();
}

fn inventory(directory: &Path) -> BTreeMap<String, (Vec<u8>, SystemTime)> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                (
                    fs::read(entry.path()).unwrap(),
                    entry.metadata().unwrap().modified().unwrap(),
                ),
            )
        })
        .collect()
}

#[test]
fn comparison_retains_grouped_required_controls_and_changed_financial_findings() {
    let temp = TempDir::new();
    for (name, date, amount) in [
        ("january", "2027-01-31", "1000"),
        ("february", "2027-02-28", "3000"),
        ("march", "2027-03-31", "6000"),
    ] {
        fs::write(
            temp.join(format!("{name}.csv")),
            trial_balance(date, amount),
        )
        .unwrap();
    }
    let previous = temp.join("previous");
    let current = temp.join("current");
    for (now, prior, output) in [
        ("february", "january", &previous),
        ("march", "february", &current),
    ] {
        let mut config = ReviewConfig::new(
            temp.join(format!("{now}.csv")),
            temp.join(format!("{prior}.csv")),
        );
        config.required_controls =
            vec!["account_mapping".into(), "subledger_reconciliation".into()];
        write_pack(&config, output);
    }
    let before = (inventory(&previous), inventory(&current));
    let result = comparison::compare_packs(&CompareConfig {
        previous_pack: previous.clone(),
        current_pack: current.clone(),
        previous_tb: temp.join("february.csv"),
        current_tb: temp.join("march.csv"),
        responses: None,
    })
    .unwrap();
    assert!(result.at("scope_changes").as_array().unwrap().is_empty());
    let findings = result.at("findings").as_array().unwrap();
    assert_eq!(findings.len(), 3);
    assert_grouped_required_controls(findings);
    assert_changed_variances(&result);
    assert_eq!((inventory(&previous), inventory(&current)), before);
}

fn assert_grouped_required_controls(findings: &[Json]) {
    let required = findings
        .iter()
        .find(|row| row.at("control").is_str("required_control"))
        .unwrap();
    assert!(required.at("change").is_str("RECURRING"));
    for side in ["previous", "current"] {
        let rows = required.at(side).as_array().unwrap();
        assert_eq!(
            rows.len(),
            2,
            "both omitted controls must survive their shared comparison key"
        );
        for name in ["account_mapping", "subledger_reconciliation"] {
            assert_eq!(
                rows.iter()
                    .filter(|row| row
                        .at("reason")
                        .as_pystr()
                        .unwrap()
                        .to_utf8()
                        .unwrap()
                        .contains(name))
                    .count(),
                1
            );
        }
    }
}

fn assert_changed_variances(result: &Json) {
    for (account, old, new) in [("A", "2000.00", "3000.00"), ("R", "-2000.00", "-3000.00")] {
        let finding = result
            .at("findings")
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row.at("account_id").is_str(account))
            .unwrap();
        assert!(finding.at("control").is_str("period_variance"));
        assert!(finding.at("change").is_str("CHANGED"));
        for (side, difference) in [("previous", old), ("current", new)] {
            let rows = finding.at(side).as_array().unwrap();
            assert_eq!(rows.len(), 1);
            assert!(rows[0].at("difference").is_str(difference));
        }
    }
    let queries = result.at("queries").as_array().unwrap();
    assert_eq!(queries.len(), 2);
    for query in queries {
        assert!(query.at("change").is_str("CHANGED"));
        assert!(pyjson::py_eq(
            query.at("previous").at("query_id"),
            query.at("current").at("query_id"),
            false
        ));
        assert!(pyjson::py_eq(
            query.at("query_id"),
            query.at("current").at("query_id"),
            false
        ));
    }
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one comparison scenario with its assertions"
)]
fn absent_findings_preserve_scope_and_source_boundaries_without_mutating_packs() {
    let temp = TempDir::new();
    for (name, date, amount) in [
        ("january", "2027-01-31", "1000"),
        ("february", "2027-02-28", "3000"),
        ("march", "2027-03-31", "3000"),
    ] {
        fs::write(
            temp.join(format!("{name}.csv")),
            trial_balance(date, amount),
        )
        .unwrap();
    }
    fs::write(
        temp.join("mapping.csv"),
        "AccountID,ReviewGroup\nA,Cash\nR,Revenue\n",
    )
    .unwrap();
    fs::write(
        temp.join("mapping-changed.csv"),
        "AccountID,ReviewGroup\nA,Cash\nR,Other revenue\n",
    )
    .unwrap();
    fs::write(
        temp.join("subledger.csv"),
        "Tenant,AccountID,SubledgerBalance\nFabricated Varrock,A,3000\n",
    )
    .unwrap();
    fs::write(temp.join("subledger-changed.csv"), "Tenant,AccountID,SubledgerBalance\nFabricated Varrock,A,3000\nFabricated Varrock,R,-3000\n").unwrap();
    let config = |current: &str, prior: &str| {
        let mut config = ReviewConfig::new(temp.join(current), temp.join(prior));
        config.mapping_path = Some(temp.join("mapping.csv"));
        config.subledger_path = Some(temp.join("subledger.csv"));
        config
    };
    let previous = temp.join("previous");
    write_pack(&config("february.csv", "january.csv"), &previous);
    let previous_before = inventory(&previous);
    for (case, expected_scope) in [
        ("same", None),
        ("threshold", Some("thresholds")),
        ("mapping", Some("account_mapping")),
        ("subledger", Some("subledger_population_not_verified")),
    ] {
        let mut current_config = config("march.csv", "february.csv");
        match case {
            "threshold" => current_config.absolute_threshold = Decimal::parse("4000").unwrap(),
            "mapping" => current_config.mapping_path = Some(temp.join("mapping-changed.csv")),
            "subledger" => current_config.subledger_path = Some(temp.join("subledger-changed.csv")),
            _ => {}
        }
        let current = temp.join(case);
        write_pack(&current_config, &current);
        let current_before = inventory(&current);
        let mut comparison = CompareConfig {
            previous_pack: previous.clone(),
            current_pack: current.clone(),
            previous_tb: temp.join("february.csv"),
            current_tb: temp.join("march.csv"),
            responses: None,
        };
        let result = comparison::compare_packs(&comparison).unwrap();
        let scope: Vec<_> = expected_scope.into_iter().map(Json::str).collect();
        assert!(
            pyjson::py_eq(result.at("scope_changes"), &Json::Array(scope), true),
            "{case}"
        );
        let expected_change = if expected_scope.is_some() {
            "NOT_COMPARABLE"
        } else {
            "NOT_RAISED"
        };
        let findings = result.at("findings").as_array().unwrap();
        assert_eq!(
            findings.len(),
            2,
            "{case}: both balanced accounts moved in February"
        );
        for (finding, account) in findings.iter().zip(["A", "R"]) {
            assert!(finding.at("control").is_str("period_variance"));
            assert!(finding.at("account_id").is_str(account));
            assert!(finding.at("change").is_str(expected_change), "{case}");
            assert_eq!(finding.at("previous").as_array().unwrap().len(), 1);
            assert!(finding.at("current").as_array().unwrap().is_empty());
        }
        let queries = result.at("queries").as_array().unwrap();
        assert_eq!(queries.len(), 2);
        assert!(
            queries
                .iter()
                .all(|query| query.at("change").is_str(expected_change))
        );
        assert_eq!(inventory(&previous), previous_before);
        assert_eq!(inventory(&current), current_before);

        // Same valid date and accounts, but different bytes from the recorded March source.
        fs::write(
            temp.join("mismatched.csv"),
            trial_balance("2027-03-31", "3001"),
        )
        .unwrap();
        comparison.current_tb = temp.join("mismatched.csv");
        let error = comparison::compare_packs(&comparison).unwrap_err();
        assert!(
            error.contains("trial balance does not match the pack's current source"),
            "{error}"
        );
        assert_eq!(inventory(&previous), previous_before);
        assert_eq!(inventory(&current), current_before);
    }
}

#[test]
fn a_financial_year_reset_does_not_turn_absent_variances_into_resolved_findings() {
    let temp = TempDir::new();
    for (date, amount) in [
        ("2026-05-31", "1000"),
        ("2026-06-30", "3000"),
        ("2026-07-31", "3000"),
    ] {
        fs::write(
            temp.join(format!("{date}.csv")),
            trial_balance(date, amount),
        )
        .unwrap();
    }
    let previous = temp.join("previous");
    let current = temp.join("current");
    write_pack(
        &ReviewConfig::new(temp.join("2026-06-30.csv"), temp.join("2026-05-31.csv")),
        &previous,
    );
    write_pack(
        &ReviewConfig::new(temp.join("2026-07-31.csv"), temp.join("2026-06-30.csv")),
        &current,
    );
    let before = [inventory(&previous), inventory(&current)];
    let result = comparison::compare_packs(&CompareConfig {
        previous_pack: previous.clone(),
        current_pack: current.clone(),
        previous_tb: temp.join("2026-06-30.csv"),
        current_tb: temp.join("2026-07-31.csv"),
        responses: None,
    })
    .unwrap();
    assert!(pyjson::py_eq(
        result.at("scope_changes"),
        &Json::Array(vec![Json::str("financial_year_reset")]),
        true
    ));
    let absent: Vec<_> = result
        .at("findings")
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item.at("control").is_str("period_variance"))
        .collect();
    assert_eq!(absent.len(), 2);
    assert!(
        absent
            .iter()
            .all(|item| item.at("change").is_str("NOT_COMPARABLE"))
    );
    assert_eq!([inventory(&previous), inventory(&current)], before);
}

fn equity_fixture() -> TempDir {
    let temp = TempDir::new();
    for (date, amount) in [
        ("2027-01-31", "1000"),
        ("2027-02-28", "3000"),
        ("2027-03-31", "3000"),
    ] {
        let equity = format!(
            "{date},Fabricated Varrock,Equity,E1,Capital,900,0,0,0,0\n{date},Fabricated Varrock,Equity,E2,Reserve,901,0,0,0,0\n"
        );
        fs::write(
            temp.join(format!("{date}.csv")),
            trial_balance(date, amount) + &equity,
        )
        .unwrap();
    }
    temp
}

fn equity_config(temp: &TempDir, prior: &str, current: &str) -> ReviewConfig {
    let mut config = ReviewConfig::new(
        temp.join(format!("{current}.csv")),
        temp.join(format!("{prior}.csv")),
    );
    config.equity_schedule_path = Some(temp.join("equity.json"));
    config.equity_currency = Some("AUD".into());
    config.equity_currency_evidence = Some("Fabricated ledger export currency declaration".into());
    config
}

fn equity_schedule(config: &ReviewConfig, opening: &str, closing: &str) -> Json {
    Json::object([
        ("schema_version", Json::int(1)),
        ("tenant", Json::str("Fabricated Varrock")),
        ("opening_date", Json::str(opening)),
        ("closing_date", Json::str(closing)),
        ("currency", Json::str("AUD")),
        ("basis", Json::str("ledger_equity")),
        (
            "prior_source_sha256",
            Json::str(&sha256(&fs::read(&config.prior_path).unwrap())),
        ),
        (
            "current_source_sha256",
            Json::str(&sha256(&fs::read(&config.current_path).unwrap())),
        ),
        ("equity_account_ids", Json::Array(vec![Json::str("E1")])),
        ("complete", Json::Bool(true)),
        ("movements", Json::Array(vec![])),
    ])
}

fn save_schedule(config: &ReviewConfig, schedule: &Json) {
    fs::write(
        config.equity_schedule_path.as_ref().unwrap(),
        pyjson::pretty(schedule),
    )
    .unwrap();
}

fn assert_scope_incomparable(result: &Json, scope: &str) {
    assert!(
        result
            .at("scope_changes")
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.is_str(scope)),
        "{result:?}"
    );
    let absent: Vec<_> = result
        .at("findings")
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item.at("control").is_str("period_variance"))
        .collect();
    assert_eq!(absent.len(), 2);
    assert!(
        absent
            .iter()
            .all(|item| item.at("change").is_str("NOT_COMPARABLE"))
    );
}

#[test]
fn changes_to_equity_accounts_currency_or_control_presence_make_absence_incomparable() {
    let temp = equity_fixture();
    let old_config = equity_config(&temp, "2027-01-31", "2027-02-28");
    save_schedule(
        &old_config,
        &equity_schedule(&old_config, "2027-01-31", "2027-02-28"),
    );
    let previous = temp.join("previous");
    write_pack(&old_config, &previous);
    let previous_before = inventory(&previous);
    for (case, scope) in [
        ("account", "equity_account_scope"),
        ("currency", "equity_account_scope"),
        ("presence", "equity_control_coverage"),
    ] {
        let mut config = equity_config(&temp, "2027-02-28", "2027-03-31");
        let mut schedule = equity_schedule(&config, "2027-02-28", "2027-03-31");
        match case {
            "account" => schedule.set("equity_account_ids", Json::Array(vec![Json::str("E2")])),
            "currency" => {
                schedule.set("currency", Json::str("USD"));
                config.equity_currency = Some("USD".into());
            }
            "presence" => {
                config.equity_schedule_path = None;
                config.equity_currency = None;
                config.equity_currency_evidence = None;
            }
            _ => unreachable!(),
        }
        if config.equity_schedule_path.is_some() {
            save_schedule(&config, &schedule);
        }
        let current = temp.join(case);
        write_pack(&config, &current);
        let current_before = inventory(&current);
        let result = comparison::compare_packs(&CompareConfig {
            previous_pack: previous.clone(),
            current_pack: current.clone(),
            previous_tb: temp.join("2027-02-28.csv"),
            current_tb: temp.join("2027-03-31.csv"),
            responses: None,
        })
        .unwrap();
        assert_scope_incomparable(&result, scope);
        assert_eq!(inventory(&previous), previous_before);
        assert_eq!(inventory(&current), current_before);
    }
}

fn equity_movement() -> Json {
    Json::object([
        ("movement_id", Json::str("M1")),
        ("date", Json::str("2027-02-15")),
        ("account_id", Json::str("E1")),
        ("debit", Json::str("0")),
        ("credit", Json::str("10")),
        ("description", Json::str("Fabricated capital contribution")),
        ("evidence_reference", Json::str("fabricated-capital-1")),
    ])
}

#[test]
fn malformed_equity_identifiers_movement_fields_and_intervals_are_refused() {
    let temp = equity_fixture();
    let config = equity_config(&temp, "2027-01-31", "2027-02-28");
    let baseline = equity_schedule(&config, "2027-01-31", "2027-02-28");
    let movement = equity_movement();
    let mut missing = movement.clone();
    let Json::Object(fields) = &mut missing else {
        panic!("movement")
    };
    fields.retain(|(key, _)| !key.is("evidence_reference"));
    let mut extra = movement.clone();
    extra.set("approved", Json::Bool(true));
    for (field, value, diagnostic) in [
        (
            "equity_account_ids",
            Json::Array(vec![Json::str("E1"), Json::str("E1")]),
            "Equity account IDs must be unique.",
        ),
        (
            "movements",
            Json::Array(vec![movement.clone(), movement]),
            "Equity movement IDs must be unique.",
        ),
        (
            "movements",
            Json::Array(vec![missing]),
            "Equity movement must contain exactly the documented fields.",
        ),
        (
            "movements",
            Json::Array(vec![extra]),
            "Equity movement must contain exactly the documented fields.",
        ),
        (
            "closing_date",
            Json::str("2027-01-31"),
            "Equity closing_date must follow opening_date.",
        ),
        (
            "closing_date",
            Json::str("2027-01-30"),
            "Equity closing_date must follow opening_date.",
        ),
    ] {
        let mut schedule = baseline.clone();
        schedule.set(field, value);
        save_schedule(&config, &schedule);
        let before = inventory(&temp.0);
        assert_eq!(engine::review_close(&config).unwrap_err(), diagnostic);
        assert_eq!(inventory(&temp.0), before);
    }
}

#[test]
fn stale_equity_metadata_or_missing_accounts_block_without_calculated_balances() {
    let temp = equity_fixture();
    let config = equity_config(&temp, "2027-01-31", "2027-02-28");
    let baseline = equity_schedule(&config, "2027-01-31", "2027-02-28");
    for (field, value, issue) in [
        (
            "tenant",
            Json::str("Other fabricated tenant"),
            "Schedule tenant does not match",
        ),
        (
            "opening_date",
            Json::str("2027-01-30"),
            "Schedule opening_date does not match",
        ),
        (
            "closing_date",
            Json::str("2027-03-01"),
            "Schedule closing_date does not match",
        ),
        (
            "prior_source_sha256",
            Json::str(&"0".repeat(64)),
            "Schedule prior_source_sha256 does not match",
        ),
        (
            "current_source_sha256",
            Json::str(&"0".repeat(64)),
            "Schedule current_source_sha256 does not match",
        ),
        (
            "equity_account_ids",
            Json::Array(vec![Json::str("missing")]),
            "must be present in both trial balances",
        ),
    ] {
        let mut schedule = baseline.clone();
        schedule.set(field, value);
        save_schedule(&config, &schedule);
        let before = inventory(&temp.0);
        let pack = engine::review_close(&config).unwrap();
        assert_eq!(inventory(&temp.0), before);
        let result = pack.equity_reconciliation.as_ref().unwrap();
        assert_equity_blocked(result, issue);
        let output = TempDir::new();
        write_pack(&config, &output.join("blocked"));
    }
}

fn assert_equity_blocked(result: &Json, issue: &str) {
    assert!(result.at("status").is_str("BLOCKED"));
    assert!(result.at("accounts").as_array().unwrap().is_empty());
    assert!(matches!(result.at("total"), Json::Null));
    let issues = result.at("issues").as_array().unwrap();
    assert_eq!(issues.len(), 1);
    assert!(issues[0].as_pystr().unwrap().os_text().contains(issue));
}
