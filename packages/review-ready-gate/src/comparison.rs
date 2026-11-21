// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use crate::{
    Result,
    decimal::Decimal,
    pyjson::{self, Json, PyStr},
    pytext, viewer,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const BOUNDARY: &str = "Comparison is a review aid. It does not approve a file, change a computed status or record who changed what. Packs carry no wall-clock time, so the reviewer records who made each change and why.";

const DETAILS: [&str; 4] = ["status", "repeat", "reason", "reviewer_action"];
fn text(value: &Json) -> String {
    value.as_pystr().unwrap().to_utf8().unwrap()
}
fn object(value: &Json) -> BTreeMap<String, &Json> {
    let Json::Object(pairs) = value else {
        unreachable!()
    };
    pairs
        .iter()
        .map(|(k, v)| (k.to_utf8().unwrap(), v))
        .collect()
}
fn findings(document: &Json) -> &[Json] {
    let Json::Array(items) = document.at("findings") else {
        unreachable!()
    };
    items
}
fn groups(document: &Json) -> BTreeMap<(String, String), Vec<&Json>> {
    let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for item in findings(document) {
        groups
            .entry((text(item.at("code")), text(item.at("slot"))))
            .or_default()
            .push(item);
    }
    groups
}
fn canonical<'a>(group: &[&'a Json]) -> Vec<[&'a PyStr; DETAILS.len()]> {
    let mut result: Vec<_> = group
        .iter()
        .map(|item| DETAILS.map(|key| item.at(key).as_pystr().unwrap()))
        .collect();
    result.sort_unstable();
    result
}
fn rendered_findings(group: Option<&[&Json]>) -> Json {
    Json::Array(
        group
            .into_iter()
            .flatten()
            .map(|item| Json::object(DETAILS.map(|key| (key, item.at(key).clone()))))
            .collect(),
    )
}
fn thresholds_differ(previous: &Json, current: &Json) -> bool {
    Decimal::parse(&text(previous.at("thresholds").at("tieout_tolerance"))).unwrap()
        != Decimal::parse(&text(current.at("thresholds").at("tieout_tolerance"))).unwrap()
}
fn coverage(document: &Json) -> Option<Vec<String>> {
    document.get("controls_not_run").map(|c| {
        let Json::Array(items) = c else {
            unreachable!()
        };
        let mut c: Vec<_> = items.iter().map(pyjson::canonical).collect();
        c.sort();
        c
    })
}
fn document_slot(slot: &str) -> bool {
    let Some(digits) = slot.strip_prefix("document_") else {
        return false;
    };
    digits.len() >= 3 && digits.bytes().all(|b| b.is_ascii_digit())
}
#[expect(
    clippy::too_many_lines,
    reason = "collects each finding's sources in one pass"
)]
fn document_sources(document: &Json) -> Result<Vec<(String, String)>> {
    let mut originals: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut order = Vec::new();
    for slot in findings(document)
        .iter()
        .map(|f| text(f.at("slot")))
        .filter(|s| s.starts_with("document_"))
    {
        if let std::collections::btree_map::Entry::Vacant(entry) = originals.entry(slot) {
            order.push(entry.key().clone());
            entry.insert(Vec::new());
        }
    }
    let Json::Object(sources) = document.at("source_sha256") else {
        unreachable!()
    };
    for (key, source) in sources {
        let key = key.to_utf8().unwrap();
        let label = key
            .rsplit_once('_')
            .filter(|(prefix, index)| {
                document_slot(prefix)
                    && !index.is_empty()
                    && index.bytes().all(|b| b.is_ascii_digit())
            })
            .map(|(slot, _)| slot);
        let filename = text(source.at("filename"));
        let filename = filename
            .split_once('/')
            .filter(|(slot, name)| document_slot(slot) && !name.is_empty() && !name.contains('\n'));
        if let Some(label) = label
            && filename.is_none_or(|(slot, _)| slot != label)
        {
            return Err(format!(
                "compare needs {key} to agree with its document filename"
            ));
        }
        if let Some((slot, name)) = filename {
            if !originals.contains_key(slot) {
                order.push(slot.into());
            }
            let digests = originals.entry(slot.into()).or_default();
            if ["original.txt", "original.pdf"].contains(&name) {
                digests.push(text(source.at("sha256")));
            }
        }
    }
    let mut result = Vec::new();
    for slot in order {
        let digests = originals.remove(&slot).unwrap();
        if digests.len() != 1 {
            return Err(format!(
                "compare needs {slot} to record exactly one original file"
            ));
        }
        result.push((slot, digests[0].clone()));
    }
    Ok(result)
}
fn unmatched_documents(previous: &Json, current: &Json) -> Result<BTreeSet<String>> {
    fn index_key(slot: &str) -> (usize, &str) {
        // Source slots have ASCII decimal indices. Compare their significant digits.
        let digits = slot.trim_start_matches("document_").trim_start_matches('0');
        (digits.len(), digits)
    }

    let mut unmatched: BTreeSet<_> = findings(previous)
        .iter()
        .map(|f| text(f.at("slot")))
        .filter(|s| s.starts_with("document_"))
        .collect();
    let mut available: BTreeMap<_, usize> = BTreeMap::new();
    for (_, digest) in document_sources(current)? {
        *available.entry(digest).or_default() += 1;
    }
    let mut before = document_sources(previous)?;
    before.sort_by(|(left, _), (right, _)| index_key(left).cmp(&index_key(right)));
    for (slot, digest) in before {
        let count = available.entry(digest).or_default();
        if *count > 0 {
            *count -= 1;
            unmatched.remove(&slot);
        }
    }
    Ok(unmatched)
}
#[expect(
    clippy::too_many_lines,
    reason = "checks each scope field of the 2 packs"
)]
fn scope_changes(previous: &Json, current: &Json) -> Result<Vec<Json>> {
    let mut changes = Vec::new();
    if thresholds_differ(previous, current) {
        changes.push(Json::object([
            ("member", Json::str("thresholds")),
            ("previous", previous.at("thresholds").clone()),
            ("current", current.at("thresholds").clone()),
        ]));
    }
    if coverage(previous) != coverage(current) {
        changes.push(Json::object([
            ("member", Json::str("controls_not_run")),
            (
                "previous",
                previous
                    .get("controls_not_run")
                    .unwrap_or(&Json::Null)
                    .clone(),
            ),
            (
                "current",
                current
                    .get("controls_not_run")
                    .unwrap_or(&Json::Null)
                    .clone(),
            ),
        ]));
    }
    let required = |document: &Json| {
        document
            .get("required_controls")
            .unwrap_or(&Json::Null)
            .clone()
    };
    if !pyjson::py_eq(&required(previous), &required(current), true) {
        changes.push(Json::object([
            ("member", Json::str("required_controls")),
            ("previous", required(previous)),
            ("current", required(current)),
        ]));
    }
    let sorted = |document| -> Result<Vec<String>> {
        let mut sources: Vec<_> = document_sources(document)?
            .into_iter()
            .map(|(_, digest)| digest)
            .collect();
        sources.sort();
        Ok(sources)
    };
    let before = sorted(previous)?;
    let after = sorted(current)?;
    if before != after {
        changes.push(Json::object([
            ("member", Json::str("document_coverage")),
            (
                "previous",
                Json::Array(before.iter().map(|s| Json::str(s)).collect()),
            ),
            (
                "current",
                Json::Array(after.iter().map(|s| Json::str(s)).collect()),
            ),
        ]));
    }
    Ok(changes)
}
fn source_changes(previous: &Json, current: &Json) -> Vec<Json> {
    let before = object(previous.at("source_sha256"));
    let after = object(current.at("source_sha256"));
    let slots: BTreeSet<_> = before.keys().chain(after.keys()).collect();
    slots
        .into_iter()
        .map(|slot| {
            let old = before.get(slot).copied();
            let new = after.get(slot).copied();
            let change = match (old, new) {
                (None, _) => "ADDED",
                (_, None) => "REMOVED",
                (Some(a), Some(b)) if pyjson::py_eq(a, b, false) => "UNCHANGED",
                _ => "CHANGED",
            };
            Json::object([
                ("slot", Json::str(slot)),
                ("change", Json::str(change)),
                ("previous", old.unwrap_or(&Json::Null).clone()),
                ("current", new.unwrap_or(&Json::Null).clone()),
            ])
        })
        .collect()
}
fn finding_changes(previous: &Json, current: &Json) -> Result<Vec<Json>> {
    let before = groups(previous);
    let after = groups(current);
    let thresholds_changed = thresholds_differ(previous, current);
    let coverage = current.get("controls_not_run");
    let not_run: BTreeSet<_> = match coverage {
        Some(Json::Array(items)) => items.iter().map(|c| text(c.at("slot"))).collect(),
        _ => BTreeSet::new(),
    };
    let unmatched = unmatched_documents(previous, current)?;
    let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
    Ok(keys
        .into_iter()
        .map(|key @ (code, slot)| {
            let old = before.get(key).map(Vec::as_slice);
            let new = after.get(key).map(Vec::as_slice);
            let change = match (old, new) {
                (None, _) => "NEW",
                (_, None)
                    if !thresholds_changed
                        && coverage.is_some()
                        && !not_run.contains(slot)
                        && !unmatched.contains(slot) =>
                {
                    "NOT_RAISED"
                }
                (_, None) => "NOT_COMPARABLE",
                (Some(a), Some(b)) if canonical(a) == canonical(b) => "RECURRING",
                _ => "CHANGED",
            };
            Json::object([
                ("code", Json::str(code)),
                ("slot", Json::str(slot)),
                ("change", Json::str(change)),
                ("previous", rendered_findings(old)),
                ("current", rendered_findings(new)),
            ])
        })
        .collect())
}
pub fn compare_packs(previous_directory: &Path, current_directory: &Path) -> Result<Json> {
    let previous = viewer::verify_pack(previous_directory)?;
    let current = viewer::verify_pack(current_directory)?;
    let before = &previous.document;
    let after = &current.document;
    for member in ["engagement_type", "period_end"] {
        if !pyjson::py_eq(before.at(member), after.at(member), false) {
            return Err(format!(
                "compare needs two runs of the same {}; the previous pack has {} and the current pack has {}",
                member.replace('_', " "),
                pytext::repr(&text(before.at(member))),
                pytext::repr(&text(after.at(member)))
            ));
        }
    }
    let run = |document: &Json, hashes: BTreeMap<String, String>| {
        Json::object([
            ("overall_status", document.at("overall_status").clone()),
            ("acknowledgement", document.at("acknowledgement").clone()),
            (
                "artefact_sha256",
                Json::Object(
                    hashes
                        .into_iter()
                        .map(|(name, hash)| {
                            (crate::pyjson::PyStr::from(name.as_str()), Json::str(&hash))
                        })
                        .collect(),
                ),
            ),
        ])
    };
    Ok(Json::object([
        ("schema_version", Json::int(1)),
        ("engagement_type", after.at("engagement_type").clone()),
        ("period_end", after.at("period_end").clone()),
        ("previous", run(before, previous.artefact_sha256)),
        ("current", run(after, current.artefact_sha256)),
        ("scope_changes", Json::Array(scope_changes(before, after)?)),
        ("sources", Json::Array(source_changes(before, after))),
        ("findings", Json::Array(finding_changes(before, after)?)),
        ("comparison_boundary", Json::str(BOUNDARY)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(slots: &[&str], sources: &[&str]) -> Json {
        Json::object([
            (
                "findings",
                Json::Array(
                    slots
                        .iter()
                        .map(|slot| Json::object([("slot", Json::str(slot))]))
                        .collect(),
                ),
            ),
            (
                "source_sha256",
                Json::Object(
                    sources
                        .iter()
                        .enumerate()
                        .map(|(i, slot)| {
                            (
                                crate::pyjson::PyStr::from(format!("evidence_{i}").as_str()),
                                Json::object([
                                    ("filename", Json::str(&format!("{slot}/original.txt"))),
                                    ("sha256", Json::str(&"a".repeat(64))),
                                ]),
                            )
                        })
                        .collect(),
                ),
            ),
        ])
    }
    #[test]
    fn equal_numeric_document_indices_keep_discovery_order() {
        for previous in [
            document(
                &["document_001", "document_0001"],
                &["document_0001", "document_001"],
            ),
            document(&[], &["document_001", "document_0001"]),
        ] {
            let expected = if findings(&previous).is_empty() {
                BTreeSet::new()
            } else {
                BTreeSet::from(["document_0001".into()])
            };
            assert_eq!(
                unmatched_documents(&previous, &document(&[], &["document_001"])).unwrap(),
                expected
            );
            assert_eq!(document_sources(&previous).unwrap()[0].0, "document_001");
        }
    }

    #[test]
    fn document_indices_sort_by_magnitude_and_significant_digits() {
        let lower = format!("document_{}{}", "0".repeat(4), "9".repeat(80));
        let higher = format!("document_1{}", "0".repeat(80));
        let highest = format!("document_2{}", "0".repeat(80));
        let previous_slots = [
            highest.as_str(),
            higher.as_str(),
            lower.as_str(),
            "document_000",
        ];
        let previous = document(&previous_slots, &previous_slots);
        let current = document(&[], &["document_001", "document_002", "document_003"]);

        assert_eq!(
            unmatched_documents(&previous, &current).unwrap(),
            BTreeSet::from([highest])
        );
    }

    #[test]
    fn repeated_findings_keep_each_packs_order_and_compare_by_detail() {
        fn finding(reason: &str) -> Json {
            Json::object([
                ("code", Json::str("BANK_REC_BREAK")),
                ("slot", Json::str("bank_rec")),
                ("status", Json::str("NOT_READY")),
                ("repeat", Json::str("")),
                ("reason", Json::str(reason)),
                ("reviewer_action", Json::str("Review the reconciliation")),
            ])
        }
        let make_pack = |reasons: &[&str]| {
            Json::object([
                (
                    "thresholds",
                    Json::object([("tieout_tolerance", Json::str("0.01"))]),
                ),
                (
                    "findings",
                    Json::Array(reasons.iter().copied().map(finding).collect()),
                ),
                ("source_sha256", Json::object([])),
            ])
        };
        let before_reasons = ["Écriture", "Écriture", "🔎 Review"];
        let after_reasons = ["🔎 Review", "Écriture", "Écriture"];
        let before = make_pack(&before_reasons);
        let unchanged = make_pack(&after_reasons);
        let changed = make_pack(&["🔎 Review", "Écriture", "🔎 Review"]);
        let recurring = finding_changes(&before, &unchanged).unwrap();
        assert_eq!(recurring.len(), 1);
        assert!(recurring[0].at("change").is_str("RECURRING"));
        for (version, reasons) in [("previous", before_reasons), ("current", after_reasons)] {
            let expected = Json::Array(
                reasons
                    .into_iter()
                    .map(|reason| {
                        Json::object([
                            ("status", Json::str("NOT_READY")),
                            ("repeat", Json::str("")),
                            ("reason", Json::str(reason)),
                            ("reviewer_action", Json::str("Review the reconciliation")),
                        ])
                    })
                    .collect(),
            );
            assert!(pyjson::py_eq(recurring[0].at(version), &expected, true));
        }
        let changed = finding_changes(&before, &changed).unwrap();
        assert_eq!(changed.len(), 1);
        assert!(changed[0].at("change").is_str("CHANGED"));
    }
}
