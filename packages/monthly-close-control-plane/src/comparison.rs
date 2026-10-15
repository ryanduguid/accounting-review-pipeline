//! Read-only comparison of independently verified close runs.
use crate::{
    Result, isodate,
    loader::{self, SourceSnapshot},
    pyjson::{self, Json, LoadError, PyStr},
    pytext,
    viewer::{self, VerifiedPack},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub const BOUNDARY: &str = "A comparison records changes between supplied runs. NOT_RAISED does not mean resolved or approved. Responses do not alter findings or close a period.";
pub struct CompareConfig {
    pub previous_pack: PathBuf,
    pub current_pack: PathBuf,
    pub previous_tb: PathBuf,
    pub current_tb: PathBuf,
    pub responses: Option<PathBuf>,
}
struct Run {
    document: Json,
    hashes: BTreeMap<String, String>,
    tenant: String,
    accounts: BTreeSet<String>,
}
fn verified(pack: VerifiedPack, source: SourceSnapshot) -> Result<Run> {
    let document = pack.document;
    if !document
        .at("source_sha256")
        .get("current_trial_balance")
        .is_some_and(|v| v.is_str(&source.sha256))
    {
        return Err("Comparison trial balance does not match the pack's current source.".into());
    }
    let rows = loader::load_canonical_tb(&source)?;
    let expected = Json::Array(vec![Json::str(&rows[0].report_date.iso())]);
    if !pyjson::py_eq(document.at("current_report_dates"), &expected, true) {
        return Err("Comparison trial-balance date does not match the pack.".into());
    }
    let tenant = rows[0].tenant.clone();
    for item in document.at("exceptions").as_array().unwrap() {
        if !item.at("tenant").is_str("") && !item.at("tenant").is_str(&tenant) {
            return Err("Pack finding names another tenant.".into());
        }
    }
    Ok(Run {
        document,
        hashes: pack.artefact_sha256,
        tenant,
        accounts: rows.into_iter().map(|row| row.account_id).collect(),
    })
}
fn same(a: Option<&Json>, b: Option<&Json>) -> bool {
    pyjson::py_eq(a.unwrap_or(&Json::Null), b.unwrap_or(&Json::Null), true)
}
fn scope_changes(previous: &Json, current: &Json) -> Vec<&'static str> {
    let mut changes = Vec::new();
    for field in ["controls_not_run", "thresholds"] {
        if !same(previous.get(field), current.get(field)) {
            changes.push(field);
        }
    }
    if previous.get("controls_not_run").is_none() || current.get("controls_not_run").is_none() {
        changes.push("control_coverage_unknown");
    }
    for label in ["account_mapping", "mapping_policy", "balance_policy"] {
        if !same(
            previous.at("source_sha256").get(label),
            current.at("source_sha256").get(label),
        ) {
            changes.push(label);
        }
    }
    if !same(
        previous.at("source_sha256").get("subledger"),
        current.at("source_sha256").get("subledger"),
    ) {
        changes.push("subledger_population_not_verified");
    }
    let required = |doc: &Json| {
        doc.get("calculation_evidence")
            .and_then(|v| v.get("required"))
            .cloned()
            .unwrap_or_else(|| Json::Array(vec![]))
    };
    if !pyjson::py_eq(&required(previous), &required(current), true) {
        changes.push("required_calculations");
    }
    let equity = |doc: &Json| {
        doc.get("equity_reconciliation")
            .filter(|v| !matches!(v, Json::Null))
            .cloned()
    };
    match (equity(previous), equity(current)) {
        (Some(a), Some(b)) => {
            let accounts = |doc: &Json| {
                Json::Array(
                    doc.at("accounts")
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|row| row.at("account_id").clone())
                        .collect(),
                )
            };
            if !pyjson::py_eq(a.at("currency"), b.at("currency"), true)
                || !pyjson::py_eq(&accounts(&a), &accounts(&b), true)
            {
                changes.push("equity_account_scope");
            }
        }
        (None, None) => {}
        _ => changes.push("equity_control_coverage"),
    }
    if !pyjson::py_eq(
        previous.at("current_report_dates"),
        current.at("current_report_dates"),
        true,
    ) && !pyjson::py_eq(
        previous.at("current_report_dates"),
        current.at("prior_report_dates"),
        true,
    ) {
        changes.push("periods_not_contiguous");
    }
    let reset = |doc: &Json| {
        doc.at("exceptions")
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row.at("control").is_str("financial_year_reset"))
    };
    if reset(previous) != reset(current) {
        changes.push("financial_year_reset");
    }
    if [previous, current]
        .iter()
        .any(|doc| doc.at("overall_status").is_str("BLOCKED"))
    {
        changes.push("blocked_run");
    }
    changes
}
type FindingKey = (PyStr, PyStr, PyStr);
fn groups(document: &Json) -> BTreeMap<FindingKey, Vec<Json>> {
    let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for row in document.at("exceptions").as_array().unwrap() {
        let text = |key| row.at(key).as_pystr().unwrap().clone();
        groups
            .entry((text("control"), text("tenant"), text("account_id")))
            .or_default()
            .push(row.clone());
    }
    for values in groups.values_mut() {
        values.sort_by_cached_key(|item| pyjson::dumps(item, true));
    }
    groups
}
fn queries(document: &Json) -> BTreeMap<PyStr, Json> {
    document
        .get("client_queries")
        .and_then(Json::as_array)
        .unwrap_or(&[])
        .iter()
        .map(|row| (row.at("query_id").as_pystr().unwrap().clone(), row.clone()))
        .collect()
}
fn responses(
    path: Option<&Path>,
    hash: &str,
    queries: &BTreeMap<PyStr, Json>,
    period: &str,
    capture: &mut impl FnMut(&Path, &str) -> Result<SourceSnapshot>,
) -> Result<(Vec<Json>, Option<String>)> {
    let Some(path) = path else {
        return Ok((vec![], None));
    };
    let source = capture(path, "Response file")?;
    let text = source
        .text("Response file", true)
        .map_err(|error| format!("Invalid response JSON: {error}"))?;
    let document = pyjson::loads_detailed(text).map_err(|(error, diagnostic)| {
        let message = match error {
            LoadError::Invalid => diagnostic.unwrap(),
            LoadError::DuplicateKey(key) => format!(
                "close-review-pack.json: member {} appears more than once",
                pytext::repr_code_points(&key.0)
            ),
            LoadError::Crash(reason) => reason,
        };
        format!("Invalid response JSON: {message}")
    })?;
    if !document.has_exact_keys(&["schema_version", "pack_sha256", "responses"])
        || document.at("schema_version").as_plain_int() != Some(&1.into())
        || !document.at("pack_sha256").is_str(hash)
        || document.at("responses").as_array().is_none()
    {
        return Err("Responses must use schema 1 and bind to the current pack JSON digest.".into());
    }
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    for row in document.at("responses").as_array().unwrap() {
        if !row.has_exact_keys(&[
            "query_id",
            "explanation",
            "evidence_reference",
            "reviewer",
            "reviewed_on",
        ]) {
            return Err("A response must contain exactly the documented response fields.".into());
        }
        let Json::Object(fields) = row else {
            unreachable!()
        };
        if fields.iter().any(|(_, value)| {
            !value
                .as_pystr()
                .is_some_and(|text| text.0.iter().any(|&cp| !pytext::is_space(cp)))
        }) {
            return Err("Every response field must be a non-empty string.".into());
        }
        let key = row.at("query_id").as_pystr().unwrap();
        if seen.contains(key) || !queries.contains_key(key) {
            return Err("Response query_id is duplicated or absent from the current pack.".into());
        }
        let raw = row.at("reviewed_on").as_pystr().unwrap().to_utf8();
        let reviewed = raw
            .as_deref()
            .and_then(isodate::python_date_fromisoformat)
            .ok_or("Response reviewed_on must be an ISO date.")?;
        if raw.as_deref() != Some(reviewed.iso().as_str())
            || reviewed < isodate::date_fromisoformat(period).unwrap()
        {
            return Err(
                "Response date must be canonical and not precede the current close.".into(),
            );
        }
        seen.insert(key.clone());
        result.push(row.clone());
    }
    result.sort_by_key(|row| row.at("query_id").as_pystr().unwrap().clone());
    Ok((result, Some(source.sha256)))
}
pub fn compare_packs(config: &CompareConfig) -> Result<Json> {
    compare_with(
        config,
        &mut viewer::verify_pack,
        &mut SourceSnapshot::capture,
    )
}
pub fn compare_with(
    config: &CompareConfig,
    verify: &mut impl FnMut(&Path) -> Result<VerifiedPack>,
    capture: &mut impl FnMut(&Path, &str) -> Result<SourceSnapshot>,
) -> Result<Json> {
    let previous_pack = verify(&config.previous_pack)?;
    let previous = verified(
        previous_pack,
        capture(&config.previous_tb, "Comparison trial balance")?,
    )?;
    let current_pack = verify(&config.current_pack)?;
    let current = verified(
        current_pack,
        capture(&config.current_tb, "Comparison trial balance")?,
    )?;
    if previous.tenant != current.tenant {
        return Err("Compared packs must belong to the same tenant.".into());
    }
    let old_date = previous
        .document
        .at("current_report_dates")
        .as_array()
        .unwrap()[0]
        .as_pystr()
        .unwrap()
        .to_utf8()
        .unwrap();
    let new_date = current
        .document
        .at("current_report_dates")
        .as_array()
        .unwrap()[0]
        .as_pystr()
        .unwrap()
        .to_utf8()
        .unwrap();
    if new_date < old_date {
        return Err("Current pack cannot precede the previous pack.".into());
    }
    let mut coverage = scope_changes(&previous.document, &current.document);
    if previous.accounts != current.accounts {
        coverage.push("trial_balance_account_population");
    }
    let old_groups = groups(&previous.document);
    let new_groups = groups(&current.document);
    let mut findings = Vec::new();
    for key in old_groups
        .keys()
        .chain(new_groups.keys())
        .collect::<BTreeSet<_>>()
    {
        let before = old_groups.get(key).cloned().unwrap_or_default();
        let after = new_groups.get(key).cloned().unwrap_or_default();
        let before = Json::Array(before);
        let after = Json::Array(after);
        let change = if before.as_array().unwrap().is_empty() {
            "NEW"
        } else if after.as_array().unwrap().is_empty() {
            if coverage.is_empty() {
                "NOT_RAISED"
            } else {
                "NOT_COMPARABLE"
            }
        } else if pyjson::py_eq(&before, &after, true) {
            "RECURRING"
        } else {
            "CHANGED"
        };
        findings.push(Json::object([
            ("control", Json::Str(key.0.clone())),
            ("tenant", Json::Str(key.1.clone())),
            ("account_id", Json::Str(key.2.clone())),
            ("change", Json::str(change)),
            ("previous", before),
            ("current", after),
        ]));
    }
    let old_queries = queries(&previous.document);
    let new_queries = queries(&current.document);
    let mut changes = Vec::new();
    for key in old_queries
        .keys()
        .chain(new_queries.keys())
        .collect::<BTreeSet<_>>()
    {
        let old = old_queries.get(key);
        let new = new_queries.get(key);
        let change = if old.is_none() {
            "NEW"
        } else if new.is_none() {
            if coverage.is_empty() {
                "NOT_RAISED"
            } else {
                "NOT_COMPARABLE"
            }
        } else if same(old, new) {
            "RECURRING"
        } else {
            "CHANGED"
        };
        changes.push(Json::object([
            ("query_id", Json::Str(key.clone())),
            ("change", Json::str(change)),
            ("previous", old.cloned().unwrap_or(Json::Null)),
            ("current", new.cloned().unwrap_or(Json::Null)),
        ]));
    }
    let (notes, note_hash) = responses(
        config.responses.as_deref(),
        &current.hashes[viewer::PACK_NAMES[0]],
        &new_queries,
        &new_date,
        capture,
    )?;
    let hashes = |hashes: BTreeMap<String, String>| {
        Json::Object(
            hashes
                .into_iter()
                .map(|(name, hash)| (name.as_str().into(), Json::str(&hash)))
                .collect(),
        )
    };
    Ok(Json::object([
        ("schema_version", Json::int(1)),
        ("tenant", Json::str(&current.tenant)),
        ("previous_period", Json::str(&old_date)),
        ("current_period", Json::str(&new_date)),
        (
            "scope_changes",
            Json::Array(coverage.into_iter().map(Json::str).collect()),
        ),
        (
            "previous_status",
            previous.document.at("overall_status").clone(),
        ),
        (
            "current_status",
            current.document.at("overall_status").clone(),
        ),
        ("findings", Json::Array(findings)),
        ("queries", Json::Array(changes)),
        ("responses", Json::Array(notes)),
        (
            "response_sha256",
            note_hash.map_or(Json::Null, |s| Json::str(&s)),
        ),
        ("previous_artefact_sha256", hashes(previous.hashes)),
        ("current_artefact_sha256", hashes(current.hashes)),
        ("review_boundary", Json::str(BOUNDARY)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scope_changes_preserve_unknown_coverage_and_blocked_order() {
        let document = Json::object([
            ("thresholds", Json::object([])),
            ("source_sha256", Json::object([])),
            ("exceptions", Json::Array(vec![])),
            (
                "current_report_dates",
                Json::Array(vec![Json::str("2026-09-30")]),
            ),
            (
                "prior_report_dates",
                Json::Array(vec![Json::str("2026-08-31")]),
            ),
            ("overall_status", Json::str("PASS")),
        ]);
        assert_eq!(
            scope_changes(&document, &document),
            vec!["control_coverage_unknown"]
        );
        let mut current = document.clone();
        current.set("controls_not_run", Json::Array(vec![]));
        current.set("overall_status", Json::str("BLOCKED"));
        current.set(
            "current_report_dates",
            Json::Array(vec![Json::str("2026-11-30")]),
        );
        assert_eq!(
            scope_changes(&document, &current),
            vec![
                "controls_not_run",
                "control_coverage_unknown",
                "periods_not_contiguous",
                "blocked_run"
            ]
        );
    }
}
