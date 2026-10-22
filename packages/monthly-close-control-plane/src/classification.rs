//! Compare supplied coding assertions while retaining the original discrepancy.
use crate::{
    Result,
    decimal::{self, Decimal},
    drivers::{self, Transaction, field},
    isodate::Date,
    loader::{self, SourceSnapshot},
    pycsv::Reader,
    pyjson::{self, Json, LoadError},
    pypath, pytext,
    reconciliation::{self, COLUMNS},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub const BOUNDARY: &str = "Supplied transaction selection and coding assertions only. References are labels, not authenticated bills or proof of appropriate accounting treatment. Agreement does not establish completeness, approve a correction or change any close finding or status.";
const FIELDS: [&str; 6] = [
    "original_account_id",
    "transaction_id",
    "current_account_id",
    "expected_account_id",
    "purpose",
    "evidence_reference",
];
type Key = (String, String);

fn selection(
    source: &SourceSnapshot,
    currency: &str,
    tenant: &str,
    start: Date,
    end: Date,
) -> Result<BTreeMap<Key, Transaction>> {
    let mut reader = Reader::strict_lf(source.text("Transactions", true)?);
    let malformed = |error| format!("Malformed classification transaction CSV: {error}");
    let header = reader
        .next()
        .transpose()
        .map_err(malformed)?
        .unwrap_or_default();
    if header.len() != COLUMNS.len()
        || !COLUMNS
            .iter()
            .all(|key| header.iter().any(|value| value == key))
    {
        return Err("Classification transactions need the exact canonical columns.".into());
    }
    for row in reader {
        let row = row.map_err(malformed)?;
        if !row.is_empty() && row.len() != COLUMNS.len() {
            return Err("Classification transaction rows need every explicit cell.".into());
        }
    }
    let mut selected = drivers::transactions(source, currency)?;
    let mut result = BTreeMap::new();
    for key in selected.order {
        if key.0 != tenant {
            return Err("Transactions must belong to the declared tenant.".into());
        }
        for row in selected.by_account.remove(&key).unwrap() {
            if row.date < start || end < row.date {
                return Err(
                    "Every transaction must fall within the declared selection period.".into(),
                );
            }
            result.insert((key.1.clone(), row.identifier.clone()), row);
        }
    }
    Ok(result)
}
fn posting(row: &Transaction) -> Json {
    let mut result = Json::Object(
        COLUMNS[..7]
            .iter()
            .map(|key| ((*key).into(), row.row.at(key).clone()))
            .collect(),
    );
    let zero = Decimal::zero();
    let magnitude = row.amount.copy_abs();
    result.set(
        "Debit",
        Json::str(&reconciliation::fixed(if row.amount > zero {
            &row.amount
        } else {
            &zero
        })),
    );
    result.set(
        "Credit",
        Json::str(&reconciliation::fixed(if row.amount < zero {
            &magnitude
        } else {
            &zero
        })),
    );
    result
}
pub fn classify(original: &Path, current: &Path, evidence: &Path, currency: &str) -> Result<Json> {
    classify_with(
        &[original.to_owned(), current.to_owned(), evidence.to_owned()],
        currency,
        &mut SourceSnapshot::capture,
    )
}
pub fn classify_with(
    paths: &[PathBuf; 3],
    currency: &str,
    capture: &mut impl FnMut(&Path, &str) -> Result<SourceSnapshot>,
) -> Result<Json> {
    let mut sources: Vec<SourceSnapshot> = Vec::new();
    let mut indices = [0; 3];
    for (index, path) in paths.iter().enumerate() {
        let resolved = pypath::resolve(path, false).map_err(|_| {
            format!(
                "Cannot resolve classification source: {}.",
                pypath::text(path)
            )
        })?;
        indices[index] = if let Some(found) = sources
            .iter()
            .position(|source| pypath::same_path(&source.path, &resolved))
        {
            found
        } else {
            sources.push(capture(&resolved, "Classification source")?);
            sources.len() - 1
        };
    }
    let [before, after, assertions] = indices.map(|index| &sources[index]);
    let text = assertions
        .text("Coding evidence", true)
        .map_err(|error| format!("Invalid coding evidence JSON: {error}"))?;
    let mut data = pyjson::loads_detailed(text).map_err(|(error, diagnostic)| {
        let message = match error {
            LoadError::Invalid => diagnostic.unwrap(),
            LoadError::DuplicateKey(key) => format!(
                "close-review-pack.json: member {} appears more than once",
                pytext::repr_code_points(&key.0)
            ),
            LoadError::Crash(reason) => reason,
        };
        format!("Invalid coding evidence JSON: {message}")
    })?;
    if !data.has_exact_keys(&[
        "schema_version",
        "tenant",
        "currency",
        "period_start",
        "period_end",
        "original_sha256",
        "current_sha256",
        "expectations",
    ]) || data.at("schema_version").as_plain_int() != Some(&1.into())
        || !data.at("original_sha256").is_str(&before.sha256)
        || !data.at("current_sha256").is_str(&after.sha256)
        || data.at("expectations").as_array().is_none()
    {
        return Err("Coding evidence must use schema 1 and bind both transaction digests.".into());
    }
    if currency.len() != 3
        || !currency.bytes().all(|b| b.is_ascii_uppercase())
        || !data.at("currency").is_str(currency)
    {
        return Err("Declare the matching three-letter currency explicitly.".into());
    }
    for key in ["tenant", "period_start", "period_end"] {
        if data.at(key).as_pystr().is_none() {
            return Err(format!("{key} must be text."));
        }
        let value = loader::json_input_text(data.at(key), key, 0, &assertions.path, false)?;
        data.set(key, Json::str(&value));
    }
    let start = loader::parse_iso_date(
        &field(&data, "period_start"),
        "period_start",
        &assertions.path,
        None,
    )?;
    let end = loader::parse_iso_date(
        &field(&data, "period_end"),
        "period_end",
        &assertions.path,
        None,
    )?;
    if start > end {
        return Err("period_start must not follow period_end.".into());
    }
    let tenant = field(&data, "tenant");
    let (originals, currents) = decimal::with_precision(40, || -> Result<_> {
        Ok((
            selection(before, currency, &tenant, start, end)?,
            selection(after, currency, &tenant, start, end)?,
        ))
    })?;
    let mut policies = BTreeMap::new();
    for (index, row) in data
        .at("expectations")
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        if !row.has_exact_keys(&FIELDS) {
            return Err("Each coding assertion needs exactly the documented fields.".into());
        }
        let Json::Object(members) = row else {
            unreachable!()
        };
        if members.iter().any(|(_, value)| value.as_pystr().is_none()) {
            return Err("Coding assertion fields must be text.".into());
        }
        let mut row = row.clone();
        for (key, value) in members {
            let key = key.to_utf8().unwrap();
            let text = loader::json_input_text(
                value,
                &key,
                index + 1,
                &assertions.path,
                ["expected_account_id", "purpose", "evidence_reference"].contains(&key.as_str()),
            )?;
            row.set(&key, Json::str(&text));
        }
        let key = (
            field(&row, "original_account_id"),
            field(&row, "transaction_id"),
        );
        if policies.contains_key(&key) || !originals.contains_key(&key) {
            return Err(
                "Coding assertion key is duplicated or absent from the original selection.".into(),
            );
        }
        policies.insert(key, row);
    }
    let targets: BTreeMap<_, _> = originals
        .keys()
        .map(|key| {
            (
                key.clone(),
                policies.get(key).map_or_else(
                    || key.clone(),
                    |policy| (field(policy, "current_account_id"), key.1.clone()),
                ),
            )
        })
        .collect();
    let mut counts = BTreeMap::new();
    for target in targets.values() {
        *counts.entry(target).or_insert(0) += 1;
    }
    let mut used = BTreeSet::new();
    let mut items = Vec::new();
    for (key, old) in &originals {
        let policy = policies.get(key);
        let expected = policy
            .filter(|policy| {
                ["expected_account_id", "purpose", "evidence_reference"]
                    .iter()
                    .all(|key| !field(policy, key).is_empty())
            })
            .map(|policy| field(policy, "expected_account_id"));
        let target = &targets[key];
        let new = currents.get(target);
        let mut reasons = Vec::new();
        if expected.is_none() {
            reasons.push("MISSING_CLASSIFICATION_EVIDENCE");
        }
        if let Some(new) = new {
            used.insert(target.clone());
            if counts[target] > 1 {
                reasons.push("CURRENT_TRANSACTION_REUSED");
            }
            if ["Date", "Reference", "Description"]
                .iter()
                .any(|key| !pyjson::py_eq(old.row.at(key), new.row.at(key), true))
                || old.amount != new.amount
            {
                reasons.push("TRANSACTION_FACTS_CHANGED");
            }
            if expected
                .as_ref()
                .is_some_and(|expected| !new.row.at("AccountID").is_str(expected))
            {
                reasons.push("CODING_DIFFERENCE");
            }
        } else {
            reasons.push("CURRENT_TRANSACTION_NOT_MATCHED");
        }
        items.push(Json::object([
            ("original", posting(old)),
            ("current", new.map_or(Json::Null, posting)),
            ("evidence", policy.cloned().unwrap_or(Json::Null)),
            (
                "original_disagrees_with_evidence",
                expected.map_or(Json::Null, |expected| {
                    Json::Bool(!old.row.at("AccountID").is_str(&expected))
                }),
            ),
            (
                "status",
                Json::str(if reasons.is_empty() { "PASS" } else { "REVIEW" }),
            ),
            (
                "reasons",
                Json::Array(reasons.into_iter().map(Json::str).collect()),
            ),
        ]));
    }
    let unmatched: Vec<_> = currents
        .iter()
        .filter(|(key, _)| !used.contains(*key))
        .map(|(_, row)| posting(row))
        .collect();
    let passed = !items.is_empty()
        && unmatched.is_empty()
        && items.iter().all(|item| item.at("status").is_str("PASS"));
    Ok(Json::object([
        ("schema_version", Json::int(1)),
        ("status", Json::str(if passed { "PASS" } else { "REVIEW" })),
        ("tenant", Json::str(&tenant)),
        ("currency", Json::str(currency)),
        ("period_start", Json::str(&start.iso())),
        ("period_end", Json::str(&end.iso())),
        (
            "scope",
            Json::str(
                "The same supplied transaction selection before and after possible recoding.",
            ),
        ),
        ("boundary", Json::str(BOUNDARY)),
        ("original_count", Json::int(originals.len() as i64)),
        ("current_count", Json::int(currents.len() as i64)),
        (
            "source_sha256",
            Json::object([
                ("original", Json::str(&before.sha256)),
                ("current", Json::str(&after.sha256)),
                ("coding_evidence", Json::str(&assertions.sha256)),
            ]),
        ),
        ("items", Json::Array(items)),
        ("unmatched_current", Json::Array(unmatched)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recoding_retains_the_original_discrepancy_and_captures_each_path_once() {
        let paths = ["original.csv", "current.csv", "coding.json"]
            .map(|name| pypath::resolve(Path::new(name), false).unwrap());
        let csv = "Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit\nSynthetic,OLD,AUD,T1,2026-09-01,R,Synthetic,12.34,0\n";
        let before = SourceSnapshot::new(&paths[0], csv.as_bytes().to_vec());
        let after = SourceSnapshot::new(&paths[1], csv.replace(",OLD,", ",NEW,").into_bytes());
        let evidence = Json::object([
            ("schema_version", Json::int(1)),
            ("tenant", Json::str("Synthetic")),
            ("currency", Json::str("AUD")),
            ("period_start", Json::str("2026-09-01")),
            ("period_end", Json::str("2026-09-30")),
            ("original_sha256", Json::str(&before.sha256)),
            ("current_sha256", Json::str(&after.sha256)),
            (
                "expectations",
                Json::Array(vec![Json::object([
                    ("original_account_id", Json::str("OLD")),
                    ("transaction_id", Json::str("T1")),
                    ("current_account_id", Json::str("NEW")),
                    ("expected_account_id", Json::str("NEW")),
                    ("purpose", Json::str("Synthetic coding")),
                    ("evidence_reference", Json::str("Synthetic receipt")),
                ])]),
            ),
        ]);
        let assertions = SourceSnapshot::new(&paths[2], pyjson::canonical(&evidence).into_bytes());
        let mut reads = Vec::new();
        let result = classify_with(&paths, "AUD", &mut |path, _| {
            reads.push(path.to_owned());
            Ok([&before, &after, &assertions]
                .into_iter()
                .find(|source| source.path == path)
                .unwrap()
                .clone())
        })
        .unwrap();
        assert!(result.at("status").is_str("PASS"));
        assert_eq!(
            result.at("items").as_array().unwrap()[0]
                .at("original_disagrees_with_evidence")
                .as_bool(),
            Some(true)
        );
        assert_eq!(reads, paths);
        let same = [paths[0].clone(), paths[0].clone(), paths[2].clone()];
        let mut reads = 0;
        assert!(
            classify_with(&same, "AUD", &mut |path, _| {
                reads += 1;
                Ok(if path == paths[0] {
                    before.clone()
                } else {
                    assertions.clone()
                })
            })
            .is_err()
        );
        assert_eq!(reads, 2);
    }
}
