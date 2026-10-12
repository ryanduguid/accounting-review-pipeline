//! Port of tests/test_gateway.py.

use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};

use super::*;
use crate::decimal::Decimal;
use crate::gateway::{
    ALLOWED_DECISIONS, BalanceRow, MODEL_PROJECTION, assert_model_is_redacted, decimal, iso_timestamp, percent_string,
    validate_review, variance_findings,
};
use crate::isodate::Date;
use crate::persist::{self, Ops, REAL_OPS};
use crate::pyjson::canonical;
use crate::util::{Location, sha256_hex, snapshot_file};

fn d(text: &str) -> Decimal {
    Decimal::parse(text).unwrap()
}

fn row(account_id: &str, section: &str, ytd_credit: &str) -> BalanceRow {
    BalanceRow {
        report_date: Date::new(2026, 6, 30).unwrap(),
        tenant: "Unit Test Tenant".into(),
        section: section.into(),
        account_id: account_id.into(),
        account_name: format!("Name {account_id}"),
        account_code: "9999".into(),
        debit: d("0.00"),
        credit: d("0.00"),
        ytd_debit: d("0.00"),
        ytd_credit: d(ytd_credit),
    }
}

fn operation() -> Json {
    parse(
        r#"{"allowed_sections": ["Revenue"], "minimum_absolute_delta": "1000.00", "minimum_percent_delta": "15.00", "max_results": 25}"#,
    )
}

fn findings(current: &[BalanceRow], prior: &[BalanceRow], operation: &Json) -> crate::util::Result<Vec<(Json, Json)>> {
    variance_findings(
        current,
        prior,
        &PyStr::from("entity-1"),
        &Json::str("Revenue"),
        operation,
    )
}

fn redact(model: &Json, rows: &[BalanceRow]) -> crate::util::Result<()> {
    let refs: Vec<&BalanceRow> = rows.iter().collect();
    assert_model_is_redacted(model, &refs)
}

/// `evaluate` with the real findings replaced by `count` clones of the first one.
fn evaluate_cloned(env: &Env, count: usize) -> Evaluation {
    let clone = |c: &[BalanceRow], p: &[BalanceRow], e: &PyStr, s: &Json, o: &Json| {
        let (model, evidence) = variance_findings(c, p, e, s, o)?.remove(0);
        Ok((0..count)
            .map(|i| {
                let id = Json::str(&format!("finding:clone-{i:02}"));
                let (mut m, mut v) = (model.clone(), evidence.clone());
                set(&mut m, &["finding_id"], id.clone());
                set(&mut v, &["finding_id"], id);
                (m, v)
            })
            .collect())
    };
    gateway::evaluate_with(env, Path::new(CONTEXT), Path::new(REQUEST), Path::new(POLICY), &clone).unwrap()
}

struct Paths {
    model: PathBuf,
    evidence: PathBuf,
    receipt: PathBuf,
    decision: PathBuf,
}

fn write_run(env: &Env, evaluation: &Evaluation) -> Paths {
    let out = persist::write_evaluation(
        env,
        &evaluation.model,
        &evaluation.evidence,
        &evaluation.receipt,
        Path::new("build/run"),
    )
    .unwrap();
    Paths {
        model: out.model,
        evidence: out.evidence,
        receipt: out.receipt,
        decision: env.cwd.join(DECISION_BUNDLED),
    }
}

const DECISION_BUNDLED: &str = "samples/decisions/sample-review-decision.json";

fn validate(env: &Env, paths: &Paths) -> crate::util::Result<Json> {
    let decision = if paths.decision.ends_with(DECISION_BUNDLED) {
        PathBuf::from(DECISION)
    } else {
        paths.decision.clone()
    };
    validate_review(env, &paths.evidence, &paths.receipt, &decision)
}

fn decision_json(run_id: &Json, decisions: Json) -> Json {
    let mut decision = parse(
        r#"{"schema_version": "xero-human-review-decision.v1", "run_id": null, "reviewer_ref": "unit-test-reviewer", "reviewed_at": "2026-08-09T00:00:00Z", "decisions": []}"#,
    );
    set(&mut decision, &["run_id"], run_id.clone());
    set(&mut decision, &["decisions"], decisions);
    decision
}

fn one_decision(finding_id: &Json, state: &str, rationale: &str) -> Json {
    let mut entry = parse(r#"{"finding_id": null, "decision": null, "rationale": null}"#);
    set(&mut entry, &["finding_id"], finding_id.clone());
    set(&mut entry, &["decision"], Json::str(state));
    set(&mut entry, &["rationale"], Json::str(rationale));
    Json::Array(vec![entry])
}

/// A completed run in build/run plus a decision file recording one state.
fn decided_run(env: &Env, state: &str) -> Paths {
    let evaluation = evaluate(env);
    let mut paths = write_run(env, &evaluation);
    let finding = get(&evaluation.evidence, &["items", "0", "finding_id"]);
    let decision = decision_json(
        evaluation.receipt.at("run_id"),
        one_decision(finding, state, "Fabricated demo variance reviewed."),
    );
    paths.decision = env.cwd.join("build/run/decision.json");
    write(&paths.decision, &format!("{}\n", pyjson::pretty(&decision)));
    paths
}

type Edit<'a> = Option<&'a dyn Fn(&mut Json)>;

/// A completed run whose artefacts are edited and then resealed against each other.
fn resealed_run(env: &Env, evidence_edit: Edit, receipt_edit: Edit, decision_edit: Edit) -> Paths {
    let evaluation = evaluate(env);
    let mut paths = write_run(env, &evaluation);
    let mut evidence = read_json(&paths.evidence);
    let mut receipt = read_json(&paths.receipt);
    let finding = get(&evidence, &["items", "0", "finding_id"]).clone();
    let mut decision = decision_json(
        receipt.at("run_id"),
        one_decision(&finding, "ACKNOWLEDGED", "Fabricated demo variance reviewed."),
    );
    if let Some(edit) = evidence_edit {
        edit(&mut evidence);
    }
    if let Some(edit) = decision_edit {
        edit(&mut decision);
    }
    set(
        &mut receipt,
        &["evidence_sha256"],
        Json::str(&format!("sha256:{}", sha256_hex(canonical(&evidence).as_bytes()))),
    );
    if let Some(edit) = receipt_edit {
        edit(&mut receipt);
    }
    write_json(&paths.evidence, &evidence);
    write_json(&paths.receipt, &receipt);
    paths.decision = env.cwd.join("build/run/decision.json");
    write(&paths.decision, &format!("{}\n", pyjson::pretty(&decision)));
    paths
}

#[test]
fn test_bundled_data_resolves_from_the_package() {
    let bundle = Bundle::shipped();
    assert!(bundle.files.contains_key(POLICY));
    assert!(bundle.files.contains_key(CURRENT_CSV));
}

#[test]
fn test_policy_bound_evaluation_returns_one_redacted_revenue_finding() {
    let dir = TempDir::new();
    let Evaluation {
        model,
        evidence,
        receipt,
    } = evaluate(&env(dir.path()));
    assert_eq!(text(model.at("status")), "REVIEW_READY");
    let Json::Array(found) = model.at("findings") else {
        panic!()
    };
    assert_eq!(found.len(), 1);
    assert_eq!(text(get(&model, &["findings", "0", "section"])), "Revenue");
    // 11000 / 60000, expressed in per cent and quantized to 4 decimal places.
    assert_eq!(text(get(&model, &["findings", "0", "percent_change"])), "18.3333");
    assert_eq!(canonical(model.at("total_findings")), "1");
    assert_eq!(canonical(model.at("truncated")), "false");
    let model_text = canonical(&model);
    for leaked in ["Demo Entity Pty Ltd", "Demo Sales", "acct-300"] {
        assert!(!model_text.contains(leaked));
    }
    assert_eq!(text(get(&evidence, &["items", "0", "account_name"])), "Demo Sales");
    assert_eq!(canonical(receipt.at("run_id")), canonical(model.at("run_id")));
    assert_eq!(text(receipt.at("code_version")), crate::VERSION);
}

#[test]
fn test_blank_account_code_is_not_a_redaction_sentinel() {
    let mut codeless = row("codeless", "Revenue", "0.00");
    codeless.account_code = String::new();
    redact(&parse(r#"{"optional_display_value": ""}"#), &[codeless]).unwrap();
}

#[test]
fn test_evaluation_writes_only_below_cwd_build_and_decision_can_be_validated() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let evaluation = evaluate(&env);
    let out = persist::write_evaluation(
        &env,
        &evaluation.model,
        &evaluation.evidence,
        &evaluation.receipt,
        Path::new("build/test-output"),
    )
    .unwrap();
    for path in [&out.model, &out.evidence, &out.receipt] {
        assert!(path.exists());
        assert!(path.starts_with(dir.path().join("build")));
    }
    let result = validate_review(&env, &out.evidence, &out.receipt, Path::new(DECISION)).unwrap();
    assert_eq!(text(result.at("status")), "DECISION_RECORDED");
    assert_eq!(canonical(result.at("undecided_count")), "0");
}

#[test]
fn test_tampered_evidence_fails_the_receipt_digest() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let paths = write_run(&env, &evaluate(&env));
    let mut tampered = read_json(&paths.evidence);
    set(
        &mut tampered,
        &["items", "0", "account_name"],
        Json::str("Someone Else Entirely"),
    );
    write_json(&paths.evidence, &tampered);
    assert_refused(validate(&env, &paths), "evidence digest");
}

#[test]
fn test_partially_decided_findings_are_counted_as_undecided() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let evaluation = evaluate_cloned(&env, 2);
    let mut paths = write_run(&env, &evaluation);
    let decision = decision_json(
        evaluation.receipt.at("run_id"),
        one_decision(
            &Json::str("finding:clone-00"),
            "ACKNOWLEDGED",
            "First finding reviewed.",
        ),
    );
    paths.decision = dir.path().join("build/run/decision.json");
    write_json(&paths.decision, &decision);
    let result = validate(&env, &paths).unwrap();
    assert_eq!(text(result.at("status")), "PARTIAL_DECISION_RECORDED");
    assert_eq!(canonical(result.at("decision_count")), "1");
    assert_eq!(canonical(result.at("undecided_count")), "1");
    assert_eq!(canonical(result.at("truncated")), "false");
    assert_eq!(canonical(result.at("completable")), "true");
}

#[test]
fn test_a_truncated_run_reports_that_it_cannot_be_completed() {
    let dir = TempDir::new();
    let env = env(dir.path());
    // The bundled policy caps results at 25.
    let evaluation = evaluate_cloned(&env, 30);
    let mut paths = write_run(&env, &evaluation);
    assert_eq!(canonical(evaluation.evidence.at("truncated")), "true");
    let first = get(&evaluation.evidence, &["items", "0", "finding_id"]);
    let decision = decision_json(
        evaluation.receipt.at("run_id"),
        one_decision(first, "ACKNOWLEDGED", "Only visible finding reviewed."),
    );
    paths.decision = dir.path().join("build/run/decision.json");
    write_json(&paths.decision, &decision);
    let result = validate(&env, &paths).unwrap();
    assert_eq!(text(result.at("status")), "PARTIAL_DECISION_RECORDED");
    assert_eq!(canonical(result.at("visible_findings")), "25");
    assert_eq!(canonical(result.at("undecided_count")), "29");
    assert_eq!(canonical(result.at("truncated")), "true");
    assert_eq!(canonical(result.at("completable")), "false");
}

#[test]
fn test_decision_file_is_accepted_from_cwd_build() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let mut paths = write_run(&env, &evaluate(&env));
    paths.decision = dir.path().join("build/run/decision.json");
    write(&paths.decision, &shipped(DECISION));
    assert_eq!(text(validate(&env, &paths).unwrap().at("status")), "DECISION_RECORDED");
}

#[test]
fn test_decision_file_outside_samples_and_build_is_blocked() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let mut paths = write_run(&env, &evaluate(&env));
    paths.decision = dir.path().join("decision.json");
    write(&paths.decision, &shipped(DECISION));
    assert_refused(validate(&env, &paths), "human decision must exist under");
}

#[test]
fn test_evaluation_output_outside_cwd_build_is_blocked() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let e = evaluate(&env);
    let result = persist::write_evaluation(&env, &e.model, &e.evidence, &e.receipt, &dir.path().join("elsewhere"));
    assert_refused(result.map(|_| ()), "output directory must stay within");
}

#[test]
fn test_model_findings_carry_exactly_the_declared_projection() {
    let dir = TempDir::new();
    let model = evaluate(&env(dir.path())).model;
    let Json::Array(found) = model.at("findings") else {
        panic!()
    };
    for finding in found {
        let Json::Object(pairs) = finding else { panic!() };
        let keys: Vec<String> = pairs.iter().map(|(k, _)| k.to_utf8().unwrap()).collect();
        assert_eq!(keys, MODEL_PROJECTION);
    }
}

#[test]
fn test_account_present_only_in_the_current_period_is_reported() {
    let current = [
        row("acct-1", "Revenue", "18500.00"),
        row("acct-new", "Revenue", "5000.00"),
    ];
    let prior = [row("acct-1", "Revenue", "18000.00")];
    let found = findings(&current, &prior, &operation()).unwrap();
    assert_eq!(found.len(), 1);
    let (model, evidence) = &found[0];
    assert_eq!(text(model.at("prior_ytd_net")), "0.00");
    assert_eq!(text(model.at("delta")), "-5000.00");
    assert!(matches!(model.at("percent_change"), Json::Null));
    assert!(text(model.at("review_reason")).contains("new in the current period"));
    assert_eq!(text(evidence.at("account_id")), "acct-new");
    assert!(matches!(evidence.at("prior_values"), Json::Null));
    assert!(!matches!(evidence.at("current_values"), Json::Null));
    assert_eq!(canonical(evidence.at("source_refs")), r#"["source:current"]"#);
}

#[test]
fn test_account_present_only_in_the_prior_period_is_reported() {
    let current = [row("acct-1", "Revenue", "18000.00")];
    let prior = [
        row("acct-1", "Revenue", "18000.00"),
        row("acct-gone", "Revenue", "9000.00"),
    ];
    let found = findings(&current, &prior, &operation()).unwrap();
    assert_eq!(found.len(), 1);
    let (model, evidence) = &found[0];
    assert_eq!(text(model.at("current_ytd_net")), "0.00");
    assert_eq!(text(model.at("prior_ytd_net")), "-9000.00");
    assert_eq!(text(model.at("delta")), "9000.00");
    assert_eq!(text(model.at("percent_change")), "100.0000");
    assert!(text(model.at("review_reason")).contains("only in the prior period"));
    assert_eq!(text(evidence.at("account_id")), "acct-gone");
    assert!(matches!(evidence.at("current_values"), Json::Null));
    assert!(!matches!(evidence.at("prior_values"), Json::Null));
    assert_eq!(canonical(evidence.at("source_refs")), r#"["source:prior"]"#);
}

#[test]
fn test_findings_beyond_max_results_are_disclosed_not_silently_dropped() {
    let dir = TempDir::new();
    let Evaluation { model, evidence, .. } = evaluate_cloned(&env(dir.path()), 27);
    assert_eq!(canonical(model.at("total_findings")), "27");
    assert_eq!(canonical(model.at("truncated")), "true");
    assert!(matches!(model.at("findings"), Json::Array(items) if items.len() == 25));
    assert_eq!(canonical(evidence.at("total_findings")), "27");
    assert_eq!(canonical(evidence.at("truncated")), "true");
    assert!(matches!(evidence.at("items"), Json::Array(items) if items.len() == 25));
}

#[test]
fn test_one_sided_account_outside_the_requested_section_is_not_reported() {
    let current = [
        row("acct-1", "Revenue", "18000.00"),
        row("acct-asset", "Assets", "7000.00"),
    ];
    let prior = [row("acct-1", "Revenue", "18000.00")];
    assert!(findings(&current, &prior, &operation()).unwrap().is_empty());
}

#[test]
fn test_a_movement_over_the_absolute_floor_but_under_the_percent_floor_is_not_reported() {
    // -61,500 against -60,000: over the 1000.00 floor, 2.50% against 15.00.
    let current = [row("acct-1", "Revenue", "61500.00")];
    let prior = [row("acct-1", "Revenue", "60000.00")];
    assert!(findings(&current, &prior, &operation()).unwrap().is_empty());
}

#[test]
fn test_a_movement_exactly_at_the_percent_floor_is_reported() {
    let current = [row("acct-1", "Revenue", "69000.00")];
    let prior = [row("acct-1", "Revenue", "60000.00")];
    let found = findings(&current, &prior, &operation()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(text(found[0].0.at("delta")), "-9000.00");
    assert_eq!(text(found[0].0.at("percent_change")), "15.0000");
}

#[test]
fn test_account_section_drift_fails_closed() {
    let current = [row("acct-1", "Assets", "18000.00")];
    let prior = [row("acct-1", "Revenue", "9000.00")];
    assert_refused(findings(&current, &prior, &operation()), "changed section");
}

const DRIFT_MESSAGE: &str = "Accounts changed section between periods ('acct-a', 'acct-b', 'acct-c', 'acct-d', 'acct-e'); review the source mapping before comparison.";

#[test]
fn test_section_drift_message_names_every_account_in_sorted_order() {
    let names = ["c", "a", "b", "e", "d"];
    let current: Vec<BalanceRow> = names
        .iter()
        .map(|n| row(&format!("acct-{n}"), "Assets", "18000.00"))
        .collect();
    let prior: Vec<BalanceRow> = names
        .iter()
        .map(|n| row(&format!("acct-{n}"), "Revenue", "9000.00"))
        .collect();
    assert_eq!(refusal(findings(&current, &prior, &operation())), DRIFT_MESSAGE);
}

// test_section_drift_message_is_identical_under_different_string_hash_seeds:
// Rust iterates ordered maps here, so there is no hash seed to vary; the
// sorted-order test above pins the message.

#[test]
fn test_section_drift_outside_requested_section_does_not_block_review() {
    let current = [
        row("acct-revenue", "Revenue", "18000.00"),
        row("acct-unrelated", "Liabilities", "7000.00"),
    ];
    let prior = [
        row("acct-revenue", "Revenue", "18000.00"),
        row("acct-unrelated", "Assets", "7000.00"),
    ];
    assert!(findings(&current, &prior, &operation()).unwrap().is_empty());
}

#[test]
fn test_review_timestamps_require_an_explicit_timezone() {
    for value in [
        "2026-08-09",
        "2026-08-09T00:00:00",
        "2026-08-09 00:00:00",
        "2026-08-09T00:00",
    ] {
        assert_refused(iso_timestamp(&Json::str(value), "reviewed_at"), "explicit UTC offset");
    }
}

fn load_tb_text(dir: &TempDir, text: &str) -> crate::util::Result<Vec<BalanceRow>> {
    let path = dir.path().join("bad.csv");
    write(&path, text);
    let env = env(dir.path());
    gateway::load_tb_snapshot(&snapshot_file(&env, &Location::Disk(path), "source CSV")?)
}

#[test]
fn test_unbalanced_source_fails_closed() {
    let dir = TempDir::new();
    let source = shipped(CURRENT_CSV).replace(",10000.00,0.00,50000.00", ",10000.01,0.00,50000.00");
    assert_refused(load_tb_text(&dir, &source), "movement debit and credit totals");
}

#[test]
fn test_empty_report_date_is_reported_as_empty_not_bad_iso() {
    let dir = TempDir::new();
    let source = shipped(CURRENT_CSV).replacen(
        "2026-06-30,Demo Entity Pty Ltd,Assets,acct-100",
        ",Demo Entity Pty Ltd,Assets,acct-100",
        1,
    );
    assert_refused(load_tb_text(&dir, &source), "ReportDate must be a non-empty string");
}

const EVIDENCE_ITEM: &str = r#"{"finding_id": "x", "account_id": "acct-1", "account_code": "9999", "account_name": "Name acct-1", "current_values": null, "prior_values": null, "source_refs": ["source:current"]}"#;

#[test]
fn test_malformed_evidence_items_are_blocked() {
    let mut extra = parse(EVIDENCE_ITEM);
    set(&mut extra, &["unexpected"], Json::str("extra"));
    let mut missing = parse(EVIDENCE_ITEM);
    remove(&mut missing, &["source_refs"]);
    for items in [
        Json::Array(vec![Json::str("not-a-dict")]),
        Json::Array(vec![extra]),
        Json::Array(vec![missing]),
    ] {
        let dir = TempDir::new();
        let env = env(dir.path());
        let mut evidence = parse(
            r#"{"schema_version": "xero-reviewer-evidence.v1", "run_id": "sha256:unit-test", "mode": "synthetic", "items": [], "total_findings": 1, "truncated": false}"#,
        );
        set(&mut evidence, &["items"], items);
        let mut receipt = parse(
            r#"{"schema_version": "xero-review-receipt.v1", "run_id": "sha256:unit-test", "mode": "synthetic", "policy_sha256": "sha256:0", "request_sha256": "sha256:0", "source_digests": {}, "result_sha256": "sha256:0", "evidence_sha256": null, "code_version": "0.1.0"}"#,
        );
        set(
            &mut receipt,
            &["evidence_sha256"],
            Json::str(&format!("sha256:{}", sha256_hex(canonical(&evidence).as_bytes()))),
        );
        let decision = decision_json(
            &Json::str("sha256:unit-test"),
            one_decision(&Json::str("x"), "ACKNOWLEDGED", "r"),
        );
        let build = dir.path().join("build");
        write(&build.join("evidence.json"), &canonical(&evidence));
        write(&build.join("receipt.json"), &canonical(&receipt));
        write(&build.join("decision.json"), &canonical(&decision));
        let result = validate_review(
            &env,
            &build.join("evidence.json"),
            &build.join("receipt.json"),
            &build.join("decision.json"),
        );
        assert_refused(result, "evidence items");
    }
}

#[test]
fn test_unknown_section_is_denied_by_policy() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    let mut request = shipped_json(REQUEST);
    set(&mut request, &["section"], Json::str("Assets"));
    sandbox
        .bundle
        .files
        .insert(REQUEST.into(), canonical(&request).into_bytes());
    let (policy, _) = gateway::load_policy_snapshot(&sandbox, &Location::Bundled(POLICY.into())).unwrap();
    let result = gateway::load_request_snapshot(&sandbox, &Location::Bundled(REQUEST.into()), &policy);
    assert_refused(result.map(|_| ()), "not allowlisted");
}

#[test]
fn test_numeric_account_id_inside_an_amount_is_not_a_disclosure() {
    redact(
        &parse(r#"{"findings": [{"delta": "-11000.00"}]}"#),
        &[row("100", "Revenue", "0.00")],
    )
    .unwrap();
}

#[test]
fn test_exact_account_id_leaf_still_trips_the_disclosure_check() {
    let result = redact(
        &parse(r#"{"findings": [{"delta": "100"}]}"#),
        &[row("100", "Revenue", "0.00")],
    );
    assert_refused(result, "raw source display data");
}

#[test]
fn test_a_model_number_equal_to_an_account_code_is_not_a_disclosure() {
    for code in ["200", "200.00", "16.6667"] {
        let mut prior = row("acct-1", "Revenue", "1200");
        prior.account_code = code.into();
        prior.ytd_debit = d("0");
        let mut current = row("acct-1", "Revenue", "1000");
        current.account_code = code.into();
        current.ytd_debit = d("0");
        let mut op = operation();
        set(&mut op, &["minimum_absolute_delta"], Json::str("100"));
        set(&mut op, &["minimum_percent_delta"], Json::str("0"));
        let found = variance_findings(
            std::slice::from_ref(&current),
            std::slice::from_ref(&prior),
            &PyStr::from("entity:unit"),
            &Json::str("Revenue"),
            &op,
        )
        .unwrap();
        let model = Json::object([("findings", Json::Array(found.into_iter().map(|f| f.0).collect()))]);
        assert_eq!(text(get(&model, &["findings", "0", "delta"])), "200.00");
        assert_eq!(text(get(&model, &["findings", "0", "prior_ytd_net"])), "-1200.00");
        assert_eq!(text(get(&model, &["findings", "0", "percent_change"])), "16.6667");
        redact(&model, &[current, prior]).unwrap();
    }
}

#[test]
fn test_a_model_number_out_of_shape_trips_the_disclosure_check() {
    for (field, leaf) in [
        ("delta", "Name acct-1"),
        ("delta", "200"),
        ("current_ytd_net", "1e3"),
        ("percent_change", "16.67"),
    ] {
        let model = Json::object([("findings", Json::Array(vec![Json::object([(field, Json::str(leaf))])]))]);
        assert_refused(
            redact(&model, &[row("acct-1", "Revenue", "0.00")]),
            "raw source display data",
        );
    }
}

#[test]
fn test_every_source_display_value_the_readme_names_is_forbidden_as_a_leaf() {
    for leaf in ["Unit Test Tenant", "Name acct-1", "9999"] {
        let model = Json::object([(
            "findings",
            Json::Array(vec![Json::object([("review_reason", Json::str(leaf))])]),
        )]);
        assert_refused(
            redact(&model, &[row("acct-1", "Revenue", "0.00")]),
            "raw source display data",
        );
    }
}

#[test]
fn test_a_prohibited_source_field_name_trips_the_disclosure_check() {
    for field in ["tenant", "account_name", "account_code"] {
        let model = Json::object([(
            "findings",
            Json::Array(vec![Json::object([(field, Json::str("a value no row carries"))])]),
        )]);
        assert_refused(
            redact(&model, &[row("acct-1", "Revenue", "0.00")]),
            "prohibited source field",
        );
    }
}

#[test]
fn test_the_decision_allowlist_is_exactly_the_three_documented_states() {
    assert_eq!(ALLOWED_DECISIONS, ["ACKNOWLEDGED", "NEEDS_EVIDENCE", "ESCALATED"]);
}

#[test]
fn test_a_decision_asserting_an_accounting_action_is_refused() {
    for state in ["APPROVED", "RESOLVED", "POSTED", "PAID", "LODGED", "LOCKED"] {
        let dir = TempDir::new();
        let env = env(dir.path());
        let paths = decided_run(&env, state);
        assert_refused(validate(&env, &paths), "decision state or rationale is invalid");
    }
}

#[test]
fn test_each_documented_review_state_is_recorded() {
    for state in ALLOWED_DECISIONS {
        let dir = TempDir::new();
        let env = env(dir.path());
        let paths = decided_run(&env, state);
        assert_eq!(text(validate(&env, &paths).unwrap().at("status")), "DECISION_RECORDED");
    }
}

#[test]
fn test_the_resealed_pack_helper_leaves_a_run_that_still_validates() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let paths = resealed_run(&env, None, None, None);
    assert_eq!(text(validate(&env, &paths).unwrap().at("status")), "DECISION_RECORDED");
}

fn refused_after(evidence: Edit, receipt: Edit, decision: Edit, fragment: &str) {
    let dir = TempDir::new();
    let env = env(dir.path());
    let paths = resealed_run(&env, evidence, receipt, decision);
    assert_refused(validate(&env, &paths), fragment);
}

#[test]
fn test_a_decision_naming_a_finding_the_evidence_does_not_carry_is_refused() {
    let fabricate = |d: &mut Json| set(d, &["decisions", "0", "finding_id"], Json::str("finding:never-emitted"));
    refused_after(None, None, Some(&fabricate), "unknown or duplicate finding");
}

#[test]
fn test_the_same_finding_decided_twice_is_refused() {
    let twice = |d: &mut Json| {
        let mut again = get(d, &["decisions", "0"]).clone();
        set(&mut again, &["decision"], Json::str("ESCALATED"));
        set(&mut again, &["rationale"], Json::str("And again."));
        list_mut(d, &["decisions"]).push(again);
    };
    refused_after(None, None, Some(&twice), "unknown or duplicate finding");
}

#[test]
fn test_a_reviewed_at_without_an_offset_is_refused_through_validate_review() {
    let strip = |d: &mut Json| set(d, &["reviewed_at"], Json::str("2026-08-09T00:00:00"));
    refused_after(None, None, Some(&strip), "explicit UTC offset");
}

#[test]
fn test_a_review_artefact_not_marked_synthetic_is_refused() {
    let relabel = |p: &mut Json| set(p, &["mode"], Json::str("live"));
    refused_after(Some(&relabel), None, None, "Only synthetic review artefacts");
    refused_after(None, Some(&relabel), None, "Only synthetic review artefacts");
}

#[test]
fn test_a_run_id_that_disagrees_across_the_pack_is_refused() {
    let restamp = |p: &mut Json| set(p, &["run_id"], Json::str("sha256:some-other-run"));
    refused_after(Some(&restamp), None, None, "must refer to the same run_id");
    refused_after(None, Some(&restamp), None, "must refer to the same run_id");
    refused_after(None, None, Some(&restamp), "must refer to the same run_id");
}

#[test]
fn test_reviewer_evidence_carrying_one_finding_twice_is_refused() {
    let duplicate = |e: &mut Json| {
        let first = get(e, &["items", "0"]).clone();
        list_mut(e, &["items"]).push(first);
        set(e, &["total_findings"], Json::int(2));
    };
    refused_after(Some(&duplicate), None, None, "duplicate finding IDs");
}

#[test]
fn test_evidence_total_findings_that_is_boolean_or_below_the_item_count_is_refused() {
    for value in [Json::Bool(true), Json::int(0)] {
        let retotal = move |e: &mut Json| set(e, &["total_findings"], value.clone());
        refused_after(
            Some(&retotal),
            None,
            None,
            "total_findings must be an integer at least as large",
        );
    }
}

#[test]
fn test_a_blank_or_control_character_reviewer_ref_is_refused() {
    for reviewer in ["", "   ", "demo\nreviewer"] {
        let rename = move |d: &mut Json| set(d, &["reviewer_ref"], Json::str(reviewer));
        refused_after(None, None, Some(&rename), "human decision reviewer_ref");
    }
}

#[test]
fn test_a_control_character_in_a_decision_rationale_is_refused() {
    for rationale in ["reviewed\nACKNOWLEDGED by the reviewer", "reviewed\u{7f}"] {
        let rewrite = move |d: &mut Json| set(d, &["decisions", "0", "rationale"], Json::str(rationale));
        refused_after(
            None,
            None,
            Some(&rewrite),
            "human decision rationale must not contain control characters",
        );
    }
}

#[test]
fn test_a_decision_state_that_is_not_a_string_is_refused_not_a_type_error() {
    let listify = |d: &mut Json| set(d, &["decisions", "0", "decision"], parse(r#"["ACKNOWLEDGED"]"#));
    refused_after(None, None, Some(&listify), "decision state or rationale is invalid");
}

#[test]
fn test_a_decisions_field_that_is_not_a_list_is_refused() {
    for decisions in ["{}", r#""ACKNOWLEDGED""#, "null"] {
        let replace = move |d: &mut Json| set(d, &["decisions"], parse(decisions));
        refused_after(None, None, Some(&replace), "decisions must be a list");
    }
}

#[test]
fn test_a_decision_file_that_records_no_decision_against_findings_is_refused() {
    let empty = |d: &mut Json| set(d, &["decisions"], Json::Array(vec![]));
    refused_after(
        None,
        None,
        Some(&empty),
        "at least one decision when the evidence carries findings",
    );
}

fn clean_run(env: &Env) -> (Paths, Json) {
    let none = |_: &[BalanceRow], _: &[BalanceRow], _: &PyStr, _: &Json, _: &Json| Ok(Vec::new());
    let evaluation =
        gateway::evaluate_with(env, Path::new(CONTEXT), Path::new(REQUEST), Path::new(POLICY), &none).unwrap();
    assert_eq!(canonical(evaluation.model.at("total_findings")), "0");
    assert_eq!(canonical(evaluation.evidence.at("items")), "[]");
    assert_eq!(canonical(evaluation.evidence.at("truncated")), "false");
    let paths = write_run(env, &evaluation);
    (paths, evaluation.receipt)
}

#[test]
fn test_a_clean_run_with_zero_findings_is_recorded_end_to_end() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let (mut paths, receipt) = clean_run(&env);
    paths.decision = dir.path().join("build/run/decision.json");
    write_json(
        &paths.decision,
        &decision_json(receipt.at("run_id"), Json::Array(vec![])),
    );
    let result = validate(&env, &paths).unwrap();
    assert_eq!(text(result.at("status")), "DECISION_RECORDED");
    for (key, value) in [
        ("decision_count", "0"),
        ("undecided_count", "0"),
        ("visible_findings", "0"),
        ("truncated", "false"),
        ("completable", "true"),
    ] {
        assert_eq!(canonical(result.at(key)), value);
    }
}

#[test]
fn test_a_decision_naming_a_finding_against_a_clean_run_is_refused() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let (mut paths, receipt) = clean_run(&env);
    paths.decision = dir.path().join("build/run/decision.json");
    let decisions = one_decision(
        &Json::str("finding:never-emitted"),
        "ACKNOWLEDGED",
        "Nothing to review.",
    );
    write_json(&paths.decision, &decision_json(receipt.at("run_id"), decisions));
    assert_refused(validate(&env, &paths), "unknown or duplicate finding");
}

#[test]
fn test_malformed_human_decision_entries_are_blocked() {
    for mangle in ["not-a-dict", "extra-field", "missing-field"] {
        let edit = move |d: &mut Json| {
            let entry = get(d, &["decisions", "0"]).clone();
            let replacement = match mangle {
                "not-a-dict" => entry.at("finding_id").clone(),
                "extra-field" => {
                    let mut e = entry.clone();
                    set(&mut e, &["unexpected"], Json::str("extra"));
                    e
                }
                _ => {
                    let mut e = entry.clone();
                    remove(&mut e, &["rationale"]);
                    e
                }
            };
            set(d, &["decisions", "0"], replacement);
        };
        refused_after(None, None, Some(&edit), "Each human decision must contain exactly");
    }
}

#[test]
fn test_a_truncated_flag_that_contradicts_the_item_counts_is_refused() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let paths = write_run(&env, &evaluate(&env));
    let mut tampered = read_json(&paths.evidence);
    set(&mut tampered, &["truncated"], Json::Bool(true));
    write_json(&paths.evidence, &tampered);
    let mut resealed = read_json(&paths.receipt);
    set(
        &mut resealed,
        &["evidence_sha256"],
        Json::str(&format!("sha256:{}", sha256_hex(canonical(&tampered).as_bytes()))),
    );
    write_json(&paths.receipt, &resealed);
    assert_refused(validate(&env, &paths), "truncated must exactly describe");
}

#[test]
fn test_evaluate_refuses_to_emit_a_model_result_carrying_raw_source_data() {
    let dir = TempDir::new();
    let leaky = |c: &[BalanceRow], p: &[BalanceRow], e: &PyStr, s: &Json, o: &Json| {
        let (mut model, evidence) = variance_findings(c, p, e, s, o)?.remove(0);
        set(&mut model, &["review_reason"], evidence.at("account_name").clone());
        Ok(vec![(model, evidence)])
    };
    let result = gateway::evaluate_with(
        &env(dir.path()),
        Path::new(CONTEXT),
        Path::new(REQUEST),
        Path::new(POLICY),
        &leaky,
    );
    assert_refused(result.map(|_| ()), "Internal disclosure assertion failed");
}

#[test]
fn test_model_result_states_its_currency_and_sign_convention() {
    let dir = TempDir::new();
    let model = evaluate(&env(dir.path())).model;
    assert_eq!(text(model.at("currency")), "AUD");
    assert!(text(model.at("sign_convention")).contains("ytd_net = YTDDebit - YTDCredit"));
    // Demo revenue rose from 60,000 to 71,000: a delta of -11000.00, debit-positive.
    assert_eq!(text(get(&model, &["findings", "0", "delta"])), "-11000.00");
}

#[test]
fn test_accepted_timestamp_forms_do_not_depend_on_the_interpreter() {
    for value in [
        "2026-08-09T00:00:00Z",
        "2026-08-09T00:00:00z",
        "2026-08-09T14:30:00+10:00",
        "2026-08-09T00:00:00-05:30",
        "2026-08-09T00:00:00.123+00:00",
        "2026-08-09T00:00:00.123456+00:00",
        "2026-08-09T00:00:00.1+00:00",
        "2026-08-09 00:00:00+00:00",
        "2026-08-09t00:00:00+00:00",
        "2026-08-09T00:00+00:00",
        "2026-08-09T00:00:00+10:00:30",
    ] {
        assert_eq!(iso_timestamp(&Json::str(value), "reviewed_at").unwrap(), value);
    }
}

#[test]
fn test_rejected_timestamp_forms_do_not_depend_on_the_interpreter() {
    for value in [
        "2026-08-09T00:00:00+10",
        "20260809T000000+0000",
        "2026-W32-7T00:00:00+00:00",
        "2026-08-09T00:00:00.123456789+00:00",
        "2026-08-09X00:00:00+00:00",
        "2026-08-09T00:00:00Z extra",
    ] {
        assert_refused(
            iso_timestamp(&Json::str(value), "reviewed_at"),
            "must be an ISO 8601 timestamp",
        );
    }
}

fn names(dir: &Path) -> std::collections::BTreeSet<PathBuf> {
    fn walk(dir: &Path, out: &mut std::collections::BTreeSet<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            out.insert(entry.path());
            if entry.path().is_dir() {
                walk(&entry.path(), out);
            }
        }
    }
    let mut out = std::collections::BTreeSet::new();
    walk(dir, &mut out);
    out
}

#[test]
fn test_evaluation_preserves_unrelated_stage_names() {
    for failure_at in [None, Some(1), Some(2), Some(3)] {
        let dir = TempDir::new();
        let env = env(dir.path());
        let e = evaluate(&env);
        let output = Path::new("build/run");
        let paths = persist::write_evaluation(&env, &e.model, &e.evidence, &e.receipt, output).unwrap();
        let finals = [&paths.model, &paths.evidence, &paths.receipt];
        let originals: Vec<Vec<u8>> = finals.iter().map(|p| std::fs::read(p).unwrap()).collect();
        for path in finals {
            std::fs::write(
                path.with_file_name(format!("{}.partial", path.file_name().unwrap().to_string_lossy())),
                b"unrelated work\n",
            )
            .unwrap();
        }
        let before = names(&dir.path().join("build/run"));
        let writes = Cell::new(0);
        let failing = |path: &Path, bytes: &[u8]| {
            writes.set(writes.get() + 1);
            if Some(writes.get()) == failure_at {
                std::fs::write(path, "partial write")?;
                return Err(io::Error::other("injected staging failure"));
            }
            std::fs::write(path, bytes)
        };
        let ops = Ops {
            write: &failing,
            ..REAL_OPS
        };
        let result = persist::write_evaluation_with(&env, &e.model, &e.evidence, &e.receipt, output, &ops);
        match failure_at {
            None => assert!(result.is_ok()),
            Some(_) => assert_refused(result.map(|_| ()), "injected staging failure"),
        }
        let after: Vec<Vec<u8>> = finals.iter().map(|p| std::fs::read(p).unwrap()).collect();
        assert_eq!(after, originals);
        for path in finals {
            let stage = path.with_file_name(format!("{}.partial", path.file_name().unwrap().to_string_lossy()));
            assert_eq!(std::fs::read(stage).unwrap(), b"unrelated work\n");
        }
        assert_eq!(names(&dir.path().join("build/run")), before);
    }
}

#[cfg(unix)]
#[test]
fn test_evaluation_does_not_follow_an_unrelated_stage_hard_link() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let output = dir.path().join("build/run");
    std::fs::create_dir_all(&output).unwrap();
    let original = dir.path().join("unrelated.txt");
    std::fs::write(&original, b"unrelated fabricated file\n").unwrap();
    let stage = output.join("model-result.json.partial");
    std::fs::hard_link(&original, &stage).unwrap();
    let e = evaluate(&env);
    let paths = persist::write_evaluation(&env, &e.model, &e.evidence, &e.receipt, Path::new("build/run")).unwrap();
    assert_eq!(std::fs::read(&original).unwrap(), b"unrelated fabricated file\n");
    assert_eq!(
        crate::pypath::file_id(&stage).unwrap(),
        crate::pypath::file_id(&original).unwrap()
    );
    assert_ne!(
        crate::pypath::file_id(&paths.model).unwrap(),
        crate::pypath::file_id(&original).unwrap()
    );
}

#[test]
fn test_overlapping_evaluations_keep_their_stages_separate() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let e = evaluate(&env);
    let output = Path::new("build/run");
    let interleaved = Cell::new(false);
    let rename = |from: &Path, to: &Path| {
        if !interleaved.get() {
            interleaved.set(true);
            persist::write_evaluation(&env, &e.model, &e.evidence, &e.receipt, output).unwrap();
        }
        std::fs::rename(from, to)
    };
    let ops = Ops {
        rename: &rename,
        ..REAL_OPS
    };
    let paths = persist::write_evaluation_with(&env, &e.model, &e.evidence, &e.receipt, output, &ops).unwrap();
    assert!(interleaved.get());
    for (path, expected) in [
        (&paths.model, &e.model),
        (&paths.evidence, &e.evidence),
        (&paths.receipt, &e.receipt),
    ] {
        assert_eq!(canonical(&read_json(path)), canonical(expected));
    }
    let present: Vec<PathBuf> = std::fs::read_dir(dir.path().join("build/run"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(present.len(), 3);
}

#[cfg(unix)]
#[test]
fn test_evaluation_replaces_a_final_hard_link_without_changing_its_other_name() {
    for name in ["model-result.json", "reviewer-evidence.json", "receipt.json"] {
        let dir = TempDir::new();
        let env = env(dir.path());
        let output = dir.path().join("build/run");
        std::fs::create_dir_all(&output).unwrap();
        let original = dir.path().join("unrelated.txt");
        std::fs::write(&original, b"unrelated fabricated file\n").unwrap();
        std::fs::hard_link(&original, output.join(name)).unwrap();
        let e = evaluate(&env);
        persist::write_evaluation(&env, &e.model, &e.evidence, &e.receipt, Path::new("build/run")).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), b"unrelated fabricated file\n");
        assert_ne!(
            crate::pypath::file_id(&output.join(name)).unwrap(),
            crate::pypath::file_id(&original).unwrap()
        );
    }
}

#[test]
fn test_staging_cleanup_failure_preserves_the_original_write_result() {
    for write_fails in [false, true] {
        let dir = TempDir::new();
        let env = env(dir.path());
        let e = evaluate(&env);
        let stages = std::cell::RefCell::new(Vec::new());
        let cleanup = |path: &Path| {
            stages.borrow_mut().push(path.to_path_buf());
            Err(io::Error::other("injected cleanup failure"))
        };
        let write_failure = |_: &Path, _: &[u8]| Err(io::Error::other("original write failure"));
        let ops = if write_fails {
            Ops {
                write: &write_failure,
                cleanup: &cleanup,
                ..REAL_OPS
            }
        } else {
            Ops {
                cleanup: &cleanup,
                ..REAL_OPS
            }
        };
        let result =
            persist::write_evaluation_with(&env, &e.model, &e.evidence, &e.receipt, Path::new("build/run"), &ops);
        if write_fails {
            assert_refused(result.map(|_| ()), "original write failure");
        } else {
            let paths = result.unwrap();
            assert_eq!(canonical(&read_json(&paths.receipt)), canonical(&e.receipt));
        }
        assert_eq!(stages.borrow().len(), 1);
        for stage in stages.borrow().iter() {
            assert_eq!(stage.parent().unwrap(), dir.path().join("build/run"));
        }
    }
}

#[test]
fn test_an_interrupted_rerun_leaves_the_previous_run_intact() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let e = evaluate(&env);
    let output = Path::new("build/run");
    let paths = persist::write_evaluation(&env, &e.model, &e.evidence, &e.receipt, output).unwrap();
    let before: Vec<Vec<u8>> = [&paths.model, &paths.evidence, &paths.receipt]
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    let writes = Cell::new(0);
    let failing = |path: &Path, bytes: &[u8]| {
        writes.set(writes.get() + 1);
        if writes.get() == 2 {
            return Err(io::Error::from_raw_os_error(28));
        }
        std::fs::write(path, bytes)
    };
    let mut second = e.model.clone();
    set(&mut second, &["run_id"], Json::str("sha256:a-different-run"));
    let ops = Ops {
        write: &failing,
        ..REAL_OPS
    };
    let result = persist::write_evaluation_with(&env, &second, &e.evidence, &e.receipt, output, &ops);
    assert_refused(result.map(|_| ()), "run output cannot be written");
    let after: Vec<Vec<u8>> = [&paths.model, &paths.evidence, &paths.receipt]
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    assert_eq!(after, before);
    let mut present: Vec<String> = std::fs::read_dir(dir.path().join("build/run"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    present.sort();
    assert_eq!(present, ["model-result.json", "receipt.json", "reviewer-evidence.json"]);
}

#[test]
fn test_a_pack_mixed_by_a_failed_move_is_refused_by_validate_review() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let e = evaluate(&env);
    let output = Path::new("build/run");
    let paths = persist::write_evaluation(&env, &e.model, &e.evidence, &e.receipt, output).unwrap();
    let moves = Cell::new(0);
    let failing = |from: &Path, to: &Path| {
        moves.set(moves.get() + 1);
        if moves.get() == 2 {
            return Err(io::Error::from_raw_os_error(13));
        }
        std::fs::rename(from, to)
    };
    let mut second = e.model.clone();
    set(&mut second, &["run_id"], Json::str("sha256:a-different-run"));
    let ops = Ops {
        rename: &failing,
        ..REAL_OPS
    };
    let result = persist::write_evaluation_with(&env, &second, &e.evidence, &e.receipt, output, &ops);
    assert_refused(result.map(|_| ()), "run output cannot be written");
    assert_eq!(text(read_json(&paths.model).at("run_id")), "sha256:a-different-run");
    assert_eq!(
        canonical(read_json(&paths.receipt).at("run_id")),
        canonical(e.receipt.at("run_id"))
    );
    let paths = Paths {
        model: paths.model,
        evidence: paths.evidence,
        receipt: paths.receipt,
        decision: dir.path().join(DECISION_BUNDLED),
    };
    assert_refused(validate(&env, &paths), "does not match the receipt's result digest");
}

#[test]
fn test_a_reviewer_holding_only_the_evidence_and_receipt_can_still_validate() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let paths = write_run(&env, &evaluate(&env));
    std::fs::remove_file(&paths.model).unwrap();
    assert_eq!(text(validate(&env, &paths).unwrap().at("status")), "DECISION_RECORDED");
}

#[test]
fn test_the_scope_note_names_exactly_the_artefacts_that_carry_the_mode_marker() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let e = evaluate(&env);
    let paths = write_run(&env, &e);
    let validation = validate(&env, &paths).unwrap();
    for artefact in [
        &shipped_json(CURRENT_MANIFEST),
        &shipped_json(CONTEXT),
        &e.model,
        &e.evidence,
        &e.receipt,
    ] {
        assert_eq!(text(artefact.at("mode")), "synthetic");
    }
    assert!(validation.get("mode").is_none());
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    assert!(readme.contains("Every source manifest, review context, model result, reviewer evidence, and receipt is marked `mode: synthetic`."));
    assert!(readme.contains("The `validate-review` output carries no `mode` key either"));
}

#[test]
fn test_the_three_artefacts_the_scope_note_exempts_reject_an_added_mode_key() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    let mut policy = shipped_json(POLICY);
    set(&mut policy, &["mode"], Json::str("live"));
    sandbox
        .bundle
        .files
        .insert(POLICY.into(), canonical(&policy).into_bytes());
    let result = gateway::load_policy_snapshot(&sandbox, &Location::Bundled(POLICY.into()));
    assert_refused(
        result.map(|_| ()),
        "policy must contain exactly: model_projection, operations, policy_id, schema_version",
    );

    let shipped_env = env(dir.path());
    let (shipped_policy, _) = gateway::load_policy_snapshot(&shipped_env, &Location::Bundled(POLICY.into())).unwrap();
    let mut request = shipped_json(REQUEST);
    set(&mut request, &["mode"], Json::str("live"));
    sandbox
        .bundle
        .files
        .insert(REQUEST.into(), canonical(&request).into_bytes());
    let result = gateway::load_request_snapshot(&sandbox, &Location::Bundled(REQUEST.into()), &shipped_policy);
    assert_refused(
        result.map(|_| ()),
        "review request must contain exactly: operation, policy_id, request_id, schema_version, section",
    );

    let mark_live = |d: &mut Json| set(d, &["mode"], Json::str("live"));
    refused_after(
        None,
        None,
        Some(&mark_live),
        "human decision must contain exactly: decisions, reviewed_at, reviewer_ref, run_id, schema_version",
    );
}

#[test]
fn test_output_directory_occupied_by_a_file_is_blocked_not_a_traceback() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let e = evaluate(&env);
    write(&dir.path().join("build/occupied"), "not a directory\n");
    let result = persist::write_evaluation(&env, &e.model, &e.evidence, &e.receipt, Path::new("build/occupied"));
    assert_refused(result.map(|_| ()), "run output cannot be written");
}

#[test]
fn test_v01_source_does_not_import_a_network_client() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    let dependencies = manifest
        .split("[dependencies]")
        .nth(1)
        .unwrap()
        .split("[features]")
        .next()
        .unwrap();
    for forbidden in ["reqwest", "hyper", "ureq", "curl", "tokio", "mcp", "rustls", "openssl"] {
        assert!(!dependencies.contains(forbidden), "{forbidden}");
    }
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
        .unwrap()
        .flatten()
    {
        if entry.path().extension().is_some_and(|e| e == "rs") {
            let source = std::fs::read_to_string(entry.path()).unwrap();
            for forbidden in ["std::net", "TcpStream", "UdpSocket"] {
                assert!(
                    !source.contains(forbidden),
                    "{} uses {forbidden}",
                    entry.path().display()
                );
            }
        }
    }
}

#[test]
fn test_an_absurd_magnitude_is_refused_not_raised_out_of_the_balance_gate() {
    assert_refused(
        decimal(&Json::str("1E+1000000"), "YTDDebit"),
        "supported magnitude range",
    );
    assert_eq!(decimal(&Json::str("0.00"), "YTDDebit").unwrap().to_sci_string(), "0.00");
    assert_eq!(
        decimal(&Json::str("-999999999999999999"), "YTDDebit")
            .unwrap()
            .to_sci_string(),
        "-999999999999999999"
    );
}

#[test]
fn test_an_absurd_percentage_is_refused_not_raised_out_of_the_projection() {
    assert_refused(percent_string(&d("1E+30")), "supported magnitude range");
    assert_eq!(percent_string(&d("0.183333")).unwrap(), "18.3333");
}

#[test]
fn test_a_non_iterable_model_projection_is_refused_not_a_type_error() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    let mut policy = shipped_json(POLICY);
    set(&mut policy, &["model_projection"], Json::int(9));
    sandbox
        .bundle
        .files
        .insert(POLICY.into(), canonical(&policy).into_bytes());
    let result = gateway::load_policy_snapshot(&sandbox, &Location::Bundled(POLICY.into()));
    assert_refused(result.map(|_| ()), "schema or model projection is unsupported");
}

#[test]
fn test_the_run_id_moves_when_only_the_source_manifest_changes() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    let baseline = evaluate(&sandbox);
    let Json::Object(digests) = baseline.receipt.at("source_digests") else {
        panic!()
    };
    let mut keys: Vec<String> = digests.iter().map(|(k, _)| k.to_utf8().unwrap()).collect();
    keys.sort();
    assert_eq!(keys, ["current", "current_manifest", "prior", "prior_manifest"]);
    for manifest in [CURRENT_MANIFEST, PRIOR_MANIFEST] {
        let mut payload = shipped_json(manifest);
        set(&mut payload, &["entity_ref"], Json::str("sample-entity-002"));
        sandbox
            .bundle
            .files
            .insert(manifest.into(), dumps(&payload).into_bytes());
    }
    let moved = evaluate(&sandbox);
    assert_ne!(
        canonical(moved.model.at("run_id")),
        canonical(baseline.model.at("run_id"))
    );
}

#[test]
fn test_malformed_resealed_evidence_is_refused() {
    let cases: [(&str, &str); 10] = [
        ("account_id", "7"),
        ("account_code", "[]"),
        ("account_name", "null"),
        ("current_values", "1"),
        ("prior_values", "{}"),
        (
            "current_values",
            r#"{"debit": 0.1, "credit": "0", "ytd_debit": "0", "ytd_credit": "0"}"#,
        ),
        (
            "current_values",
            r#"{"debit": "NaN", "credit": "0", "ytd_debit": "0", "ytd_credit": "0"}"#,
        ),
        ("source_refs", r#""source:current""#),
        ("source_refs", r#"["source:unknown"]"#),
        ("source_refs", r#"["source:current", "source:current"]"#),
    ];
    for (field, value) in cases {
        let edit = move |e: &mut Json| set(e, &["items", "0", field], parse(value));
        let dir = TempDir::new();
        let env = env(dir.path());
        let paths = resealed_run(&env, Some(&edit), None, None);
        refusal(validate(&env, &paths));
    }
}

fn row_named(name: &str, section: &str) -> BalanceRow {
    let mut r = row("acct-1", section, "0.00");
    r.account_name = name.into();
    r
}

#[test]
fn test_an_account_named_for_its_own_section_is_not_a_disclosure() {
    let model = parse(r#"{"findings": [{"section": "Revenue", "review_reason": "variance"}]}"#);
    redact(&model, &[row_named("Revenue", "Revenue")]).unwrap();
}

#[test]
fn test_the_section_exemption_does_not_reach_any_other_finding_field() {
    let model = parse(r#"{"findings": [{"review_reason": "Revenue"}]}"#);
    assert_refused(
        redact(&model, &[row_named("Revenue", "Revenue")]),
        "raw source display data",
    );
}
