//! Port of tests/test_source_gates.py. The Python tests swapped files on disk
//! under a patched package root; here the bundled data is an in-memory copy the
//! test edits, so the read-count and path-swap tests reduce to the digest checks.

use std::path::Path;

use super::*;
use crate::gateway::{
    BalanceRow, load_context_snapshot, load_manifest, load_policy_snapshot, load_request_snapshot, load_tb_snapshot,
};
use crate::pyjson::canonical;
use crate::util::{Location, Result, sha256_hex, snapshot_file};

fn load_tb_path(path: &Path) -> Result<Vec<BalanceRow>> {
    let env = env(path.parent().unwrap());
    load_tb_snapshot(&snapshot_file(&env, &Location::Disk(path.to_path_buf()), "source CSV")?)
}

/// The bundled current CSV with literal substitutions applied, written to disk.
fn tb(dir: &TempDir, replacements: &[(&str, &str)]) -> std::path::PathBuf {
    let mut text = shipped(CURRENT_CSV);
    for (old, new) in replacements {
        assert!(text.contains(old), "{old}");
        text = text.replace(old, new);
    }
    let path = dir.path().join("edited.csv");
    write(&path, &text);
    path
}

fn materially_changed_current_csv(content: &str) -> String {
    let mut content = content.to_string();
    for (old, new) in [
        ("10000.00,0.00,50000.00,0.00", "20000.00,0.00,69000.00,0.00"),
        ("0.00,10000.00,0.00,71000.00", "0.00,20000.00,0.00,90000.00"),
    ] {
        assert!(content.contains(old));
        content = content.replace(old, new);
    }
    content
}

fn bundled(rel: &str) -> Location {
    Location::Bundled(rel.into())
}

fn put(env: &mut Env, rel: &str, value: &Json) {
    env.bundle.files.insert(rel.into(), dumps(value).into_bytes());
}

fn edit(env: &mut Env, rel: &str, change: impl FnOnce(&mut Json)) {
    let mut value = parse(std::str::from_utf8(&env.bundle.files[rel]).unwrap());
    change(&mut value);
    put(env, rel, &value);
}

/// Move a source's report date in both its CSV rows and its manifest.
fn redate(env: &mut Env, manifest: &str, new_date: &str) {
    let csv = text(get(
        &parse(std::str::from_utf8(&env.bundle.files[manifest]).unwrap()),
        &["export", "csv"],
    ));
    let content = String::from_utf8(env.bundle.files[&csv].clone()).unwrap();
    let old_date = content.lines().nth(1).unwrap().split(',').next().unwrap().to_string();
    let content = content.replace(&old_date, new_date);
    let digest = sha256_hex(content.as_bytes());
    env.bundle.files.insert(csv, content.into_bytes());
    edit(env, manifest, |m| {
        set(m, &["report", "as_at"], Json::str(new_date));
        set(m, &["export", "sha256"], Json::str(&digest));
    });
}

fn load_context(env: &Env) -> Result<(String, String)> {
    let (current, prior, _) = load_context_snapshot(env, &bundled(CONTEXT))?;
    Ok((current.rows[0].report_date.iso(), prior.rows[0].report_date.iso()))
}

// --- CSV contract

#[test]
fn test_reordered_header_is_refused_even_though_the_names_all_match() {
    let dir = TempDir::new();
    let path = tb(&dir, &[("AccountCode,Debit,Credit,", "AccountCode,Credit,Debit,")]);
    assert_refused(load_tb_path(&path), "canonical ten-column header");
}

#[test]
fn test_duplicate_account_id_is_refused() {
    let dir = TempDir::new();
    let path = tb(&dir, &[("acct-200", "acct-100")]);
    assert_refused(load_tb_path(&path), "duplicate AccountID");
}

#[test]
fn test_exported_codeless_bank_account_is_accepted() {
    let dir = TempDir::new();
    let rows = load_tb_path(&tb(&dir, &[("Demo Bank,1000,", "Demo Bank,,")])).unwrap();
    let bank = rows.iter().find(|r| r.account_id == "acct-100").unwrap();
    assert_eq!(bank.account_code, "");
}

#[test]
fn test_codeless_account_support_keeps_identity_fields_required() {
    for (field, old, new) in [
        ("Tenant", "Demo Entity Pty Ltd,Assets", ",Assets"),
        ("Section", ",Assets,acct-100", ",,acct-100"),
        ("AccountID", ",acct-100,Demo Bank", ",,Demo Bank"),
        ("AccountName", ",Demo Bank,1000,", ",,1000,"),
    ] {
        let dir = TempDir::new();
        let path = tb(&dir, &[(old, new)]);
        assert_refused(load_tb_path(&path), &format!("{field} must be a non-empty string"));
    }
}

#[test]
fn test_unbalanced_ytd_columns_fail_closed_even_when_the_movement_balances() {
    let dir = TempDir::new();
    let path = tb(&dir, &[("10000.00,0.00,50000.00", "10000.00,0.00,50001.00")]);
    assert_refused(load_tb_path(&path), "YTD debit and credit totals");
}

#[test]
fn test_a_second_tenant_in_one_csv_is_refused() {
    let dir = TempDir::new();
    let path = tb(&dir, &[("Demo Entity Pty Ltd,Equity", "Other Entity Pty Ltd,Equity")]);
    assert_refused(load_tb_path(&path), "exactly one tenant and report date");
}

#[test]
fn test_a_second_report_date_in_one_csv_is_refused() {
    let dir = TempDir::new();
    let path = tb(
        &dir,
        &[(
            "2026-06-30,Demo Entity Pty Ltd,Equity",
            "2026-06-29,Demo Entity Pty Ltd,Equity",
        )],
    );
    assert_refused(load_tb_path(&path), "exactly one tenant and report date");
}

// --- CSV read path

#[test]
fn test_a_source_csv_that_is_not_utf8_is_blocked_not_a_traceback() {
    let dir = TempDir::new();
    let path = dir.path().join("bad.csv");
    let bytes = shipped(CURRENT_CSV).into_bytes();
    let mut bad = Vec::new();
    let needle = b"Demo Sales";
    let at = bytes.windows(needle.len()).position(|w| w == needle).unwrap();
    bad.extend_from_slice(&bytes[..at]);
    bad.extend_from_slice(b"Demo \xff Sales");
    bad.extend_from_slice(&bytes[at + needle.len()..]);
    std::fs::write(&path, bad).unwrap();
    assert_refused(load_tb_path(&path), "source CSV is not valid UTF-8");
}

#[test]
fn test_a_source_csv_that_cannot_be_read_is_blocked_not_a_traceback() {
    let dir = TempDir::new();
    let env = env(dir.path());
    let result = snapshot_file(&env, &Location::Disk(dir.path().to_path_buf()), "source CSV");
    assert_refused(result.map(|_| ()), "source CSV cannot be read");
}

#[test]
fn test_a_source_csv_the_csv_module_refuses_is_blocked_not_a_traceback() {
    let dir = TempDir::new();
    let path = tb(&dir, &[("Demo Sales", &format!("\"{}", "x".repeat(131_073)))]);
    assert_refused(load_tb_path(&path), "source CSV cannot be parsed as CSV");
}

#[test]
fn test_a_manifest_csv_that_cannot_be_read_is_blocked_not_a_traceback() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    edit(&mut sandbox, CURRENT_MANIFEST, |m| {
        set(m, &["export", "csv"], Json::str("samples/inputs"))
    });
    assert_refused(
        load_manifest(&sandbox, &bundled(CURRENT_MANIFEST)).map(|_| ()),
        "manifest CSV cannot be read",
    );
}

#[test]
fn test_manifest_rows_and_digest_are_bound_to_one_csv_snapshot() {
    let dir = TempDir::new();
    let sandbox = env(dir.path());
    let original = shipped(CURRENT_CSV);
    assert_ne!(materially_changed_current_csv(&original), original);
    let source = load_manifest(&sandbox, &bundled(CURRENT_MANIFEST)).unwrap();
    assert_eq!(source.csv_snapshot.content, original.as_bytes());
    assert_eq!(source.csv_snapshot.sha256, sha256_hex(original.as_bytes()));
    let revenue = source.rows.iter().find(|r| r.account_id == "acct-300").unwrap();
    assert_eq!(revenue.ytd_credit.to_sci_string(), "71000.00");
}

// test_evaluation_reads_every_provenance_input_exactly_once: the bundled
// inputs are immutable bytes in memory, read once into each snapshot.

#[test]
fn test_a_path_swap_after_calculation_cannot_change_the_bound_run_digests() {
    let dir = TempDir::new();
    let sandbox = env(dir.path());
    let inputs = [
        ("context_sha256", CONTEXT),
        ("request_sha256", REQUEST),
        ("policy_sha256", POLICY),
        ("current_csv_sha256", CURRENT_CSV),
        ("prior_csv_sha256", "samples/inputs/sample-tb-2026-05-31.csv"),
        ("current_manifest_sha256", CURRENT_MANIFEST),
        ("prior_manifest_sha256", PRIOR_MANIFEST),
    ];
    let seed = Json::Object(
        inputs
            .iter()
            .map(|(name, rel)| (PyStr::from(*name), Json::str(&sha256_hex(&sandbox.bundle.files[*rel]))))
            .collect(),
    );
    let expected_run_id = format!("sha256:{}", sha256_hex(canonical(&seed).as_bytes()));
    let e = evaluate(&sandbox);
    assert_eq!(text(e.model.at("run_id")), expected_run_id);
    assert_eq!(text(get(&e.model, &["findings", "0", "current_ytd_net"])), "-71000.00");
    let digest = |name: &str| Json::str(&format!("sha256:{}", text(seed.at(name))));
    assert_eq!(
        canonical(e.receipt.at("policy_sha256")),
        canonical(&digest("policy_sha256"))
    );
    assert_eq!(
        canonical(e.receipt.at("request_sha256")),
        canonical(&digest("request_sha256"))
    );
    let expected_sources = Json::object([
        ("current", digest("current_csv_sha256")),
        ("prior", digest("prior_csv_sha256")),
        ("current_manifest", digest("current_manifest_sha256")),
        ("prior_manifest", digest("prior_manifest_sha256")),
    ]);
    assert_eq!(canonical(e.receipt.at("source_digests")), canonical(&expected_sources));
}

// --- manifest gates

#[test]
fn test_a_manifest_generated_at_without_an_offset_is_refused() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    edit(&mut sandbox, CURRENT_MANIFEST, |m| {
        set(m, &["export", "generated_at"], Json::str("2026-07-01T09:00:00"))
    });
    assert_refused(
        load_manifest(&sandbox, &bundled(CURRENT_MANIFEST)).map(|_| ()),
        "explicit UTC offset",
    );
}

#[test]
fn test_csv_that_does_not_match_the_declared_digest_is_refused() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    let renamed = shipped(CURRENT_CSV).replace("Demo Bank", "Demo Bank Account");
    sandbox.bundle.files.insert(CURRENT_CSV.into(), renamed.into_bytes());
    assert_refused(
        load_manifest(&sandbox, &bundled(CURRENT_MANIFEST)).map(|_| ()),
        "SHA-256 does not match",
    );
}

#[test]
fn test_manifest_as_at_that_disagrees_with_the_csv_is_refused() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    edit(&mut sandbox, CURRENT_MANIFEST, |m| {
        set(m, &["report", "as_at"], Json::str("2026-06-29"))
    });
    assert_refused(
        load_manifest(&sandbox, &bundled(CURRENT_MANIFEST)).map(|_| ()),
        "as_at and CSV ReportDate do not match",
    );
}

#[test]
fn test_only_synthetic_v1_manifests_are_accepted() {
    for (field, value) in [("mode", "live"), ("schema_version", "xero-source-manifest.v2")] {
        let dir = TempDir::new();
        let mut sandbox = env(dir.path());
        edit(&mut sandbox, CURRENT_MANIFEST, |m| set(m, &[field], Json::str(value)));
        assert_refused(
            load_manifest(&sandbox, &bundled(CURRENT_MANIFEST)).map(|_| ()),
            "synthetic mode is supported",
        );
    }
}

#[test]
fn test_only_accrual_aud_trial_balances_are_accepted() {
    for (field, value) in [("name", "Profit and Loss"), ("basis", "cash"), ("currency", "NZD")] {
        let dir = TempDir::new();
        let mut sandbox = env(dir.path());
        edit(&mut sandbox, CURRENT_MANIFEST, |m| {
            set(m, &["report", field], Json::str(value))
        });
        assert_refused(
            load_manifest(&sandbox, &bundled(CURRENT_MANIFEST)).map(|_| ()),
            "accrual AUD Trial Balance",
        );
    }
}

#[test]
fn test_a_manifest_entity_ref_carrying_a_lone_surrogate_is_blocked_not_a_traceback() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    for manifest in [CURRENT_MANIFEST, PRIOR_MANIFEST] {
        let raw = String::from_utf8(sandbox.bundle.files[manifest].clone()).unwrap();
        let entity = text(parse(&raw).at("entity_ref"));
        let edited = raw.replace(&format!("\"{entity}\""), "\"sample-entity-\\ud800\"");
        sandbox.bundle.files.insert(manifest.into(), edited.into_bytes());
    }
    let result = gateway::evaluate(&sandbox, Path::new(CONTEXT), Path::new(REQUEST), Path::new(POLICY));
    assert_refused(result.map(|_| ()), "entity_ref must be encodable as UTF-8 text");
}

// --- context gates

#[test]
fn test_current_and_prior_from_different_entities_are_refused() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    edit(&mut sandbox, PRIOR_MANIFEST, |m| {
        set(m, &["entity_ref"], Json::str("sample-entity-002"))
    });
    assert_refused(load_context(&sandbox), "different entity_ref");
}

#[test]
fn test_current_and_prior_report_settings_must_agree() {
    for (field, value) in [("tracking_filters", r#"["Sydney"]"#), ("include_drafts", "true")] {
        let dir = TempDir::new();
        let mut sandbox = env(dir.path());
        edit(&mut sandbox, PRIOR_MANIFEST, |m| {
            set(m, &["report", field], parse(value))
        });
        assert_refused(load_context(&sandbox), &format!("mismatch for {field}"));
    }
}

#[test]
fn test_prior_report_must_predate_the_current_report() {
    for prior in [CURRENT_MANIFEST, PRIOR_MANIFEST] {
        let dir = TempDir::new();
        let mut sandbox = env(dir.path());
        edit(&mut sandbox, CONTEXT, |c| {
            set(c, &["current_manifest"], Json::str(PRIOR_MANIFEST));
            set(c, &["prior_manifest"], Json::str(prior));
        });
        assert_refused(load_context(&sandbox), "must be earlier than the current report date");
    }
}

#[test]
fn test_a_comparison_across_the_1_july_ytd_reset_is_refused() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    redate(&mut sandbox, CURRENT_MANIFEST, "2026-07-31");
    assert_refused(load_context(&sandbox), "different financial years");
}

#[test]
fn test_a_year_on_year_comparison_of_the_same_date_is_still_allowed() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    redate(&mut sandbox, CURRENT_MANIFEST, "2027-05-31");
    assert_eq!(
        load_context(&sandbox).unwrap(),
        ("2027-05-31".to_string(), "2026-05-31".to_string())
    );
}

// --- policy and request gates

#[test]
fn test_request_bound_to_another_policy_is_refused() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    let (policy, _) = load_policy_snapshot(&sandbox, &bundled(POLICY)).unwrap();
    edit(&mut sandbox, REQUEST, |r| {
        set(r, &["policy_id"], Json::str("some-other-policy"))
    });
    assert_refused(
        load_request_snapshot(&sandbox, &bundled(REQUEST), &policy).map(|_| ()),
        "policy_id does not match",
    );
}

#[test]
fn test_policy_model_projection_must_match_the_code_contract() {
    for reorder in [true, false] {
        let dir = TempDir::new();
        let mut sandbox = env(dir.path());
        edit(&mut sandbox, POLICY, |p| {
            let projection = list_mut(p, &["model_projection"]);
            if reorder {
                projection.reverse();
            } else {
                projection.pop();
            }
        });
        assert_refused(
            load_policy_snapshot(&sandbox, &bundled(POLICY)).map(|_| ()),
            "model projection is unsupported",
        );
    }
}

#[test]
fn test_policy_max_results_outside_one_to_one_hundred_is_refused() {
    for value in ["0", "101", "-1", "\"25\"", "2.5", "true", "false"] {
        let dir = TempDir::new();
        let mut sandbox = env(dir.path());
        edit(&mut sandbox, POLICY, |p| {
            set(
                p,
                &["operations", "trial_balance_variance", "max_results"],
                parse(value),
            )
        });
        assert_refused(
            load_policy_snapshot(&sandbox, &bundled(POLICY)).map(|_| ()),
            "max_results must be an integer from 1 to 100",
        );
    }
}

#[test]
fn test_policy_max_results_at_either_bound_is_accepted() {
    for value in [1, 100] {
        let dir = TempDir::new();
        let mut sandbox = env(dir.path());
        edit(&mut sandbox, POLICY, |p| {
            set(
                p,
                &["operations", "trial_balance_variance", "max_results"],
                Json::int(value),
            )
        });
        let (policy, _) = load_policy_snapshot(&sandbox, &bundled(POLICY)).unwrap();
        assert_eq!(
            canonical(get(&policy, &["operations", "trial_balance_variance", "max_results"])),
            value.to_string()
        );
    }
}

/// Change the current CSV's bank row and re-bind its manifest digest.
fn rewrite_current_bank_row(env: &mut Env, replacement: &str) {
    let original = "10000.00,0.00,50000.00,0.00";
    let content = shipped(CURRENT_CSV);
    assert!(content.contains(original));
    let content = content.replacen(original, replacement, 1);
    let digest = sha256_hex(content.as_bytes());
    env.bundle.files.insert(CURRENT_CSV.into(), content.into_bytes());
    edit(env, CURRENT_MANIFEST, |m| {
        set(m, &["export", "sha256"], Json::str(&digest))
    });
}

#[test]
fn test_caller_decimal_context_cannot_hide_a_one_cent_imbalance() {
    // Rust has no ambient decimal context to change; the one-cent imbalance must still be refused.
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    rewrite_current_bank_row(&mut sandbox, "10000.01,0.00,50000.01,0.00");
    let result = gateway::evaluate(&sandbox, Path::new(CONTEXT), Path::new(REQUEST), Path::new(POLICY));
    assert_refused(result.map(|_| ()), "not exactly balanced");
}

#[test]
fn test_totals_beyond_the_context_precision_are_refused_not_rounded() {
    let dir = TempDir::new();
    let mut sandbox = env(dir.path());
    rewrite_current_bank_row(&mut sandbox, "10000.0000000000000000000000001,0.00,50000.00,0.00");
    let result = gateway::evaluate(&sandbox, Path::new(CONTEXT), Path::new(REQUEST), Path::new(POLICY));
    assert_refused(result.map(|_| ()), "28-digit precision");
}
