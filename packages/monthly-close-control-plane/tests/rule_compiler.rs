// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Integration tests for the deterministic override-to-rule compiler, backtester and replay engine.

use monthly_close_control_plane::{
    pyjson::{self, Json},
    queue::rule_compiler::{
        self, CompilePolicy, CompiledRuleSet, OpenException, OverrideDisposition, OverrideRecord,
        REPLAY_SCHEMA, RULESET_SCHEMA,
    },
};

fn rec(
    period: &str,
    account_id: &str,
    pattern: &str,
    disposition: OverrideDisposition,
) -> OverrideRecord {
    OverrideRecord {
        period: period.into(),
        tenant: "Fabricated Varrock".into(),
        control: "period_variance".into(),
        account_id: account_id.into(),
        pattern: pattern.into(),
        disposition,
        reviewer_initials: "RD".into(),
        reviewed_on: period.into(),
        note: "Verified against source schedule".into(),
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn compile_backtest_serialise_and_replay_deterministic_rules() {
    let history = vec![
        // Unanimous across 2 distinct periods -> promoted.
        rec(
            "2026-06-30",
            "100",
            "Month-end  Bank  Cutoff",
            OverrideDisposition::AcceptKnownTiming,
        ),
        rec(
            "2026-07-31",
            "100",
            "month-end bank cutoff",
            OverrideDisposition::AcceptKnownTiming,
        ),
        // Unanimous reclassify across 2 distinct periods -> promoted.
        rec(
            "2026-06-30",
            "500",
            "Cloud Hosting \"Prod\"\t\\Main\nLine\r",
            OverrideDisposition::Reclassify {
                target_account_id: "510".into(),
            },
        ),
        rec(
            "2026-07-31",
            "500",
            "cloud hosting \"prod\" \\main line",
            OverrideDisposition::Reclassify {
                target_account_id: "510".into(),
            },
        ),
        // Unanimous expected contra across 2 distinct periods -> promoted.
        rec(
            "2026-06-30",
            "155",
            "Accumulated Depreciation",
            OverrideDisposition::ExpectedContra,
        ),
        rec(
            "2026-07-31",
            "155",
            "accumulated depreciation",
            OverrideDisposition::ExpectedContra,
        ),
        // Unanimous approved variance note -> promoted (for backtest conflict demotion test).
        rec(
            "2026-06-30",
            "400",
            "Seasonal Freight",
            OverrideDisposition::ApprovedVarianceNote,
        ),
        rec(
            "2026-07-31",
            "400",
            "seasonal freight",
            OverrideDisposition::ApprovedVarianceNote,
        ),
        // Conflicting dispositions -> MUST be rejected.
        rec(
            "2026-06-30",
            "600",
            "Misc Clearing",
            OverrideDisposition::AcceptKnownTiming,
        ),
        rec(
            "2026-07-31",
            "600",
            "misc clearing",
            OverrideDisposition::ExpectedContra,
        ),
        // Two agreements in the SAME period -> rejected when require_distinct_periods = true.
        rec(
            "2026-07-31",
            "700",
            "Single Month Repeat",
            OverrideDisposition::AcceptKnownTiming,
        ),
        rec(
            "2026-07-31",
            "700",
            "Single Month Repeat",
            OverrideDisposition::AcceptKnownTiming,
        ),
    ];

    let ruleset = rule_compiler::compile_rules(&history, &CompilePolicy::default()).unwrap();
    assert_eq!(ruleset.rules.len(), 4);
    assert_eq!(ruleset.rejected.len(), 2);
    assert!(
        ruleset
            .rejected
            .iter()
            .any(|r| r.account_id == "600" && r.reason == "conflicting_dispositions")
    );
    assert!(
        ruleset
            .rejected
            .iter()
            .any(|r| r.account_id == "700" && r.reason == "insufficient_agreements")
    );

    // Allow same-period agreements when require_distinct_periods is explicitly false.
    let relaxed = rule_compiler::compile_rules(
        &history,
        &CompilePolicy {
            min_agreements: 2,
            require_distinct_periods: false,
        },
    )
    .unwrap();
    assert_eq!(relaxed.rules.len(), 5);

    // JSON round-trip & TOML export.
    let json = ruleset.to_json();
    assert!(json.at("schema").is_str(RULESET_SCHEMA));
    let loaded = CompiledRuleSet::from_json(&json).unwrap();
    assert_eq!(loaded.rules, ruleset.rules);
    assert_eq!(loaded.ruleset_sha256, ruleset.ruleset_sha256);

    let toml = ruleset.to_toml();
    assert!(toml.contains("[[rules]]"));
    assert!(toml.contains("reclassify:510"));
    assert!(toml.contains("\\\"prod\\\""));
    assert!(toml.contains("\\\\main"));

    // Backtesting against held-out August records: 1 agrees, 1 conflicts (demotes rule), 1 unmatched.
    let held_out = vec![
        rec(
            "2026-08-31",
            "100",
            "Month-end bank cutoff deposit",
            OverrideDisposition::AcceptKnownTiming,
        ),
        rec(
            "2026-08-31",
            "400",
            "Seasonal freight adjustment",
            OverrideDisposition::AcceptKnownTiming,
        ),
        rec(
            "2026-08-31",
            "999",
            "Unseen item",
            OverrideDisposition::ExpectedContra,
        ),
    ];
    let backtest = rule_compiler::backtest_rules(&ruleset, &held_out).unwrap();
    assert_eq!(backtest.total_records, 3);
    assert_eq!(backtest.matched_records, 2);
    assert_eq!(backtest.agreed_records, 1);
    assert_eq!(backtest.conflicted_records, 1);
    assert_eq!(backtest.demoted_rule_ids.len(), 1);
    assert_eq!(backtest.verified_rules.len(), 3);

    // Replay on September open items (including an ambiguous multi-rule conflict!).
    let mut ambig_records = vec![
        rec(
            "2026-06-30",
            "800",
            "alpha",
            OverrideDisposition::AcceptKnownTiming,
        ),
        rec(
            "2026-07-31",
            "800",
            "alpha",
            OverrideDisposition::AcceptKnownTiming,
        ),
        rec(
            "2026-06-30",
            "800",
            "beta",
            OverrideDisposition::ExpectedContra,
        ),
        rec(
            "2026-07-31",
            "800",
            "beta",
            OverrideDisposition::ExpectedContra,
        ),
    ];
    ambig_records.extend(history[..2].iter().cloned());
    let replay_set =
        rule_compiler::compile_rules(&ambig_records, &CompilePolicy::default()).unwrap();
    let open = vec![
        OpenException {
            item_id: "EX-1".into(),
            tenant: "Fabricated Varrock".into(),
            control: "period_variance".into(),
            account_id: "100".into(),
            text: "Late month-end bank cutoff transfer".into(),
        },
        OpenException {
            item_id: "EX-2".into(),
            tenant: "Fabricated Varrock".into(),
            control: "period_variance".into(),
            account_id: "800".into(),
            text: "Both alpha and beta matched".into(),
        },
        OpenException {
            item_id: "EX-3".into(),
            tenant: "Fabricated Varrock".into(),
            control: "period_variance".into(),
            account_id: "900".into(),
            text: "Genuine new variance".into(),
        },
    ];
    let report = rule_compiler::replay_rules(&replay_set, &open).unwrap();
    assert_eq!(report.rule_cleared.len(), 1);
    assert_eq!(report.rule_cleared[0].item_id, "EX-1");
    assert_eq!(report.ambiguous_items, vec!["EX-2".to_owned()]);
    assert_eq!(
        report.exceptions_remaining,
        vec!["EX-2".to_owned(), "EX-3".to_owned()]
    );
    let replay_json = report.to_json();
    assert!(replay_json.at("schema").is_str(REPLAY_SCHEMA));
    assert!(pyjson::pretty_utf8(&replay_json).unwrap().contains("EX-1"));
}

#[test]
#[allow(clippy::too_many_lines)]
fn rule_compiler_rejects_invalid_inputs_and_tampered_rulesets() {
    // Disposition parsing edge cases.
    assert!(OverrideDisposition::parse("unknown").is_err());
    assert!(OverrideDisposition::parse("reclassify:   ").is_err());
    assert_eq!(
        OverrideDisposition::parse("approved_variance_note").unwrap(),
        OverrideDisposition::ApprovedVarianceNote
    );

    // Policy & record validation errors.
    assert!(
        rule_compiler::compile_rules(
            &[],
            &CompilePolicy {
                min_agreements: 1,
                require_distinct_periods: true,
            }
        )
        .is_err()
    );
    let mut bad_date = rec(
        "2026-02-30",
        "100",
        "x",
        OverrideDisposition::AcceptKnownTiming,
    );
    assert!(rule_compiler::compile_rules(&[bad_date.clone()], &CompilePolicy::default()).is_err());
    bad_date.period = "2026-07-31".into();
    bad_date.reviewed_on = "2026-07-31".into();
    bad_date.note = "   ".into();
    assert!(rule_compiler::compile_rules(&[bad_date.clone()], &CompilePolicy::default()).is_err());
    bad_date.note = "ok".into();
    bad_date.pattern = "   ".into();
    assert!(rule_compiler::compile_rules(&[bad_date], &CompilePolicy::default()).is_err());

    // Tampered ruleset JSON & replay verification.
    let valid = rule_compiler::compile_rules(
        &[
            rec(
                "2026-06-30",
                "100",
                "cutoff",
                OverrideDisposition::AcceptKnownTiming,
            ),
            rec(
                "2026-07-31",
                "100",
                "cutoff",
                OverrideDisposition::AcceptKnownTiming,
            ),
        ],
        &CompilePolicy::default(),
    )
    .unwrap();

    // Tamper ruleset_sha256 before replay.
    let mut bad_set = valid.clone();
    bad_set.ruleset_sha256 = "0000".into();
    assert!(rule_compiler::replay_rules(&bad_set, &[]).is_err());

    // Wrong schema, missing fields, bad integers, bad periods, tampered rule_sha256, tampered ruleset_sha256.
    assert!(CompiledRuleSet::from_json(&Json::object([])).is_err());
    assert!(
        CompiledRuleSet::from_json(&Json::object([("schema", Json::str(RULESET_SCHEMA))])).is_err()
    );
    assert!(
        CompiledRuleSet::from_json(&Json::object([
            ("schema", Json::str(RULESET_SCHEMA)),
            ("ruleset_sha256", Json::str("abc")),
        ]))
        .is_err()
    );

    let mut bad_ruleset_sha = valid.to_json();
    bad_ruleset_sha.set("ruleset_sha256", Json::str("deadbeef"));
    assert!(
        CompiledRuleSet::from_json(&bad_ruleset_sha)
            .unwrap_err()
            .contains("ruleset_sha256")
    );

    let mut bad_rule_sha = valid.to_json();
    let Json::Array(ref mut rules) = bad_rule_sha.as_object().unwrap()[3].1.clone() else {
        unreachable!()
    };
    rules[0].set("rule_sha256", Json::str("deadbeef"));
    bad_rule_sha.set("rules", Json::Array(rules.clone()));
    assert!(
        CompiledRuleSet::from_json(&bad_rule_sha)
            .unwrap_err()
            .contains("SHA-256")
    );

    for (field, bad_val, expected) in [
        ("version", Json::str("v1"), "version"),
        ("support_count", Json::str("two"), "support_count"),
        (
            "supporting_periods",
            Json::str("none"),
            "supporting_periods must be an array",
        ),
        (
            "supporting_periods",
            Json::Array(vec![Json::int(1)]),
            "supporting_periods entry",
        ),
    ] {
        let mut tampered = valid.to_json();
        let mut arr = tampered.at("rules").as_array().unwrap().to_vec();
        arr[0].set(field, bad_val);
        tampered.set("rules", Json::Array(arr));
        assert!(
            CompiledRuleSet::from_json(&tampered)
                .unwrap_err()
                .contains(expected),
            "{field}"
        );
    }

    // Cover TOML escaping of \t, \r, \n via a direct rule check.
    let mut special = valid;
    special.rules[0].match_pattern = "a\\b\"c\nd\re\tf".into();
    let toml = special.to_toml();
    assert!(toml.contains("a\\\\b\\\"c\\nd\\re\\tf"));
}
