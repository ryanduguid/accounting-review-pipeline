// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: FSL-1.1-ALv2

use monthly_close_control_plane::{
    comparison::{self, CompareConfig},
    decimal::Decimal,
    engine::{self, ReviewConfig},
    pyjson::{self, Json},
    report, viewer,
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
