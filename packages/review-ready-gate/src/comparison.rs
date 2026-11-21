// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

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
const SKIPPED_CONTROL: [&str; 3] = ["slot", "filename", "reason"];

fn text(value: &Json) -> String {
    value.as_pystr().unwrap().to_utf8().unwrap()
}
fn object(value: &Json) -> BTreeMap<&PyStr, &Json> {
    let Json::Object(pairs) = value else {
        unreachable!()
    };
    pairs.iter().map(|(key, value)| (key, value)).collect()
}
fn findings(document: &Json) -> &[Json] {
    let Json::Array(items) = document.at("findings") else {
        unreachable!()
    };
    items
}
fn groups(document: &Json) -> BTreeMap<(&PyStr, &PyStr), Vec<&Json>> {
    let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for item in findings(document) {
        let code = item.at("code").as_pystr().unwrap();
        let slot = item.at("slot").as_pystr().unwrap();
        groups.entry((code, slot)).or_default().push(item);
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
fn coverage(document: &Json) -> Option<Vec<[&PyStr; SKIPPED_CONTROL.len()]>> {
    document.get("controls_not_run").map(|c| {
        let Json::Array(items) = c else {
            unreachable!()
        };
        let mut rows: Vec<_> = items
            .iter()
            .map(|item| SKIPPED_CONTROL.map(|key| item.at(key).as_pystr().unwrap()))
            .collect();
        rows.sort_unstable();
        rows
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
fn unmatched_documents(
    previous: &Json,
    before: &mut [(String, String)],
    after: &[(String, String)],
) -> BTreeSet<String> {
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
    for (_, digest) in after {
        *available.entry(digest).or_default() += 1;
    }
    before.sort_by(|(left, _), (right, _)| index_key(left).cmp(&index_key(right)));
    for (slot, digest) in before.iter() {
        if let Some(count) = available.get_mut(digest).filter(|count| **count > 0) {
            *count -= 1;
            unmatched.remove(slot);
        }
    }
    unmatched
}
#[expect(
    clippy::too_many_lines,
    reason = "checks each scope field of the 2 packs"
)]
fn scope_changes(
    previous: &Json,
    current: &Json,
    previous_sources: &[(String, String)],
    current_sources: &[(String, String)],
) -> Vec<Json> {
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
    let previous_required = previous.get("required_controls").unwrap_or(&Json::Null);
    let current_required = current.get("required_controls").unwrap_or(&Json::Null);
    if !pyjson::py_eq(previous_required, current_required, true) {
        changes.push(Json::object([
            ("member", Json::str("required_controls")),
            ("previous", previous_required.clone()),
            ("current", current_required.clone()),
        ]));
    }
    fn sorted_digests(sources: &[(String, String)]) -> Vec<&str> {
        let mut sources: Vec<_> = sources.iter().map(|(_, digest)| digest.as_str()).collect();
        sources.sort();
        sources
    }
    let before = sorted_digests(previous_sources);
    let after = sorted_digests(current_sources);
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
    changes
}
fn source_changes(previous: &Json, current: &Json) -> Vec<Json> {
    let before = object(previous.at("source_sha256"));
    let after = object(current.at("source_sha256"));
    let slots: BTreeSet<_> = before.keys().chain(after.keys()).copied().collect();
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
                ("slot", Json::Str(slot.clone())),
                ("change", Json::str(change)),
                ("previous", old.unwrap_or(&Json::Null).clone()),
                ("current", new.unwrap_or(&Json::Null).clone()),
            ])
        })
        .collect()
}
fn finding_changes(previous: &Json, current: &Json, unmatched: &BTreeSet<String>) -> Vec<Json> {
    let before = groups(previous);
    let after = groups(current);
    let thresholds_changed = thresholds_differ(previous, current);
    let coverage = current.get("controls_not_run");
    let not_run: BTreeSet<_> = match coverage {
        Some(Json::Array(items)) => items
            .iter()
            .map(|control| control.at("slot").as_pystr().unwrap())
            .collect(),
        _ => BTreeSet::new(),
    };
    let unmatched: BTreeSet<_> = unmatched
        .iter()
        .map(|slot| PyStr::from(slot.as_str()))
        .collect();
    let keys: BTreeSet<_> = before.keys().chain(after.keys()).copied().collect();
    keys.into_iter()
        .map(|key @ (code, slot)| {
            let old = before.get(&key).map(Vec::as_slice);
            let new = after.get(&key).map(Vec::as_slice);
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
                ("code", Json::Str(code.clone())),
                ("slot", Json::Str(slot.clone())),
                ("change", Json::str(change)),
                ("previous", rendered_findings(old)),
                ("current", rendered_findings(new)),
            ])
        })
        .collect()
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
    let mut previous_sources = document_sources(before)?;
    let current_sources = document_sources(after)?;
    let scope = scope_changes(before, after, &previous_sources, &current_sources);
    let unmatched = unmatched_documents(before, &mut previous_sources, &current_sources);
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
        ("scope_changes", Json::Array(scope)),
        ("sources", Json::Array(source_changes(before, after))),
        (
            "findings",
            Json::Array(finding_changes(before, after, &unmatched)),
        ),
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
            let mut previous_sources = document_sources(&previous).unwrap();
            let current_sources = document_sources(&document(&[], &["document_001"])).unwrap();
            assert_eq!(previous_sources[0].0, "document_001");
            assert_eq!(
                unmatched_documents(&previous, &mut previous_sources, &current_sources),
                expected
            );
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
            unmatched_documents(
                &previous,
                &mut document_sources(&previous).unwrap(),
                &document_sources(&current).unwrap()
            ),
            BTreeSet::from([highest])
        );
    }

    #[test]
    fn skipped_controls_compare_by_values_and_multiplicity() {
        let first = Json::object([
            ("slot", Json::str("tax")),
            ("filename", Json::str("résumé.csv")),
            ("reason", Json::str("Two lines\nwith a quote: \"")),
        ]);
        let second = Json::object([
            ("reason", Json::str("Source not supplied")),
            ("slot", Json::str("bank")),
            ("filename", Json::str("銀行.csv")),
        ]);
        let pack = |items| Json::object([("controls_not_run", Json::Array(items))]);
        let before = pack(vec![first.clone(), second.clone(), first.clone()]);
        let reordered = pack(vec![second.clone(), first.clone(), first.clone()]);
        let one_removed = pack(vec![first, second]);

        assert_eq!(coverage(&before), coverage(&reordered));
        assert_ne!(coverage(&before), coverage(&one_removed));
        assert_ne!(coverage(&pack(Vec::new())), coverage(&Json::Null));
    }

    #[test]
    fn finding_groups_preserve_unicode_order_and_skipped_scope() {
        let finding = |slot| {
            Json::object([
                ("code", Json::str("RULE")),
                ("slot", Json::str(slot)),
                ("status", Json::str("NOT_READY")),
                ("repeat", Json::str("false")),
                ("reason", Json::str("Unmatched balance")),
                ("reviewer_action", Json::str("Review source")),
            ])
        };
        let pack = |items, skipped| {
            Json::object([
                ("findings", Json::Array(items)),
                ("controls_not_run", Json::Array(skipped)),
                (
                    "thresholds",
                    Json::object([("tieout_tolerance", Json::str("0.01"))]),
                ),
            ])
        };
        let previous = pack(vec![finding("🔎"), finding("é"), finding("Z")], Vec::new());
        let current = pack(
            vec![finding("Z"), finding("𐍈")],
            vec![Json::object([("slot", Json::str("é"))])],
        );

        let changes = finding_changes(&previous, &current, &BTreeSet::from(["🔎".to_owned()]));
        let summary: Vec<_> = changes
            .iter()
            .map(|item| (text(item.at("slot")), text(item.at("change"))))
            .collect();
        assert_eq!(
            summary,
            [
                ("Z".to_owned(), "RECURRING".to_owned()),
                ("é".to_owned(), "NOT_COMPARABLE".to_owned()),
                ("𐍈".to_owned(), "NEW".to_owned()),
                ("🔎".to_owned(), "NOT_COMPARABLE".to_owned()),
            ]
        );
    }

    #[test]
    fn source_changes_keep_unicode_slot_order_and_unchanged_entries() {
        let before = Json::object([(
            "source_sha256",
            Json::object([
                ("🔎", Json::str("old")),
                ("é", Json::str("same")),
                ("z", Json::str("removed")),
            ]),
        )]);
        let after = Json::object([(
            "source_sha256",
            Json::object([
                ("𐍈", Json::str("new")),
                ("🔎", Json::str("new")),
                ("é", Json::str("same")),
            ]),
        )]);
        let rows = source_changes(&before, &after);
        let changes: Vec<_> = rows
            .iter()
            .map(|row| (text(row.at("slot")), text(row.at("change"))))
            .collect();
        assert_eq!(
            changes,
            [
                ("z", "REMOVED"),
                ("é", "UNCHANGED"),
                ("𐍈", "ADDED"),
                ("🔎", "CHANGED")
            ]
            .map(|(slot, change)| (slot.to_owned(), change.to_owned()))
        );
        assert!(matches!(rows[0].at("current"), Json::Null));
        assert!(matches!(rows[2].at("previous"), Json::Null));
        assert!(rows[3].at("previous").is_str("old"));
        assert!(rows[3].at("current").is_str("new"));
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
        let recurring = finding_changes(&before, &unchanged, &BTreeSet::new());
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
        let changed = finding_changes(&before, &changed, &BTreeSet::new());
        assert_eq!(changed.len(), 1);
        assert!(changed[0].at("change").is_str("CHANGED"));
    }
}
