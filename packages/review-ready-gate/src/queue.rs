// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Verify every explicitly supplied readiness pack before filtering or
//! displaying any of them. Adapted from Monthly Close Controls' `queue`;
//! components may not import each other, so the logic is copied.
use crate::{
    Result,
    pyjson::{self, Json, PyStr},
    pypath, pytext,
    viewer::{self, VerifiedPack},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub const STATUSES: [&str; 3] = ["READY", "NOT_READY", "BLOCKED"];

#[derive(Default)]
pub struct Options {
    /// Display only packs with this recorded status.
    pub status: Option<String>,
    pub summary: bool,
}
struct Entry {
    path: PathBuf,
    client: PyStr,
    period_end: PyStr,
    status: PyStr,
    findings_by_code: BTreeMap<PyStr, usize>,
    /// Slots of the controls that did not run; `None` for a pack that predates the record.
    not_run: Option<Vec<PyStr>>,
    row: String,
    digests: BTreeMap<String, String>,
}
/// Supplied text as a JSON string body with table separators escaped.
fn cell(value: &PyStr) -> String {
    let encoded = pyjson::dumps(&Json::Str(value.clone()), false);
    encoded[1..encoded.len() - 1].replace('|', "\\|")
}
fn path_text(path: &Path) -> PyStr {
    let text = pypath::text(path);
    PyStr::from_os_text(&if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text
    })
}
fn text(value: &Json) -> PyStr {
    value.as_pystr().expect("verified string").clone()
}
fn list(value: &Json) -> &[Json] {
    let Json::Array(items) = value else {
        unreachable!("verified list")
    };
    items
}
fn listed<'a>(values: impl IntoIterator<Item = &'a PyStr>, empty: &str) -> String {
    let cells: Vec<_> = values.into_iter().map(cell).collect();
    if cells.is_empty() {
        empty.into()
    } else {
        cells.join(", ")
    }
}
fn entry(path: PathBuf, pack: VerifiedPack) -> Entry {
    let document = &pack.document;
    let findings = list(document.at("findings"));
    let mut findings_by_code = BTreeMap::new();
    for finding in findings {
        *findings_by_code
            .entry(text(finding.at("code")))
            .or_default() += 1;
    }
    let not_run: Option<Vec<_>> = document
        .get("controls_not_run")
        .map(|controls| list(controls).iter().map(|c| text(c.at("slot"))).collect());
    let mut entry = Entry {
        client: path_text(path.file_name().map_or(path.as_path(), Path::new)),
        period_end: text(document.at("period_end")),
        status: text(document.at("overall_status")),
        findings_by_code,
        not_run,
        row: String::new(),
        digests: pack.artefact_sha256,
        path,
    };
    entry.row = format!(
        "| {} | {} | {} | {} | {} | {} | {} |",
        cell(&entry.client),
        cell(&text(document.at("engagement_type"))),
        cell(&entry.period_end),
        cell(&entry.status),
        findings.len(),
        listed(entry.findings_by_code.keys(), "none"),
        entry
            .not_run
            .as_ref()
            .map_or_else(|| "not recorded".into(), |slots| listed(slots, "none")),
    );
    entry
}
#[expect(
    clippy::too_many_lines,
    reason = "verifies each pack, then filters and renders the queue"
)]
pub fn render_queue(paths: &[PathBuf], options: &Options) -> Result<String> {
    let mut resolved: Vec<PathBuf> = Vec::new();
    for path in paths {
        let path = pypath::resolve(path, false)
            .map_err(|_| "Cannot resolve pack directory: path is unusable")?;
        if !resolved.iter().any(|other| pypath::same_path(other, &path)) {
            resolved.push(path);
        }
    }
    resolved.sort_by_cached_key(|path| PyStr::from_os_text(&pypath::text(path)));
    // Every pack is verified before any filter runs, so a filter cannot hide an edited pack.
    let mut verified = Vec::new();
    for path in resolved {
        let pack = viewer::verify_pack(&path)
            .map_err(|error| format!("Pack {}: {error}", pytext::repr_os(&pypath::text(&path))))?;
        verified.push(entry(path, pack));
    }
    let count = verified.len();
    let mut selected: Vec<_> = verified
        .into_iter()
        .filter(|entry| {
            options
                .status
                .as_ref()
                .is_none_or(|status| entry.status.is(status))
        })
        .collect();
    selected.sort_by_cached_key(|entry| (entry.period_end.clone(), path_text(&entry.path)));
    let mut lines = vec![
        "Review-ready Queue".into(),
        String::new(),
        format!("Verified {count} distinct pack(s); displaying {}.", selected.len()),
        "Recorded statuses are gate evidence. This queue does not approve a file or change a finding, acknowledgement or source file.".into(),
        "Clients are pack directory names; the pack does not record a client.".into(),
        String::new(),
    ];
    if selected.is_empty() {
        lines.push("No verified packs match the supplied filters.".into());
        return Ok(lines.join("\n"));
    }
    lines.extend(table(
        "| Client | Profile | Period end | Status | Findings | Finding codes | Controls not run |",
        selected.iter().map(|entry| entry.row.clone()).collect(),
        "",
    ));
    if options.summary {
        lines.extend(summary(&selected));
    }
    lines.extend(["".into(), "Verified artefact SHA-256".into()]);
    for entry in selected {
        lines.push(String::new());
        lines.push(cell(&path_text(&entry.path)));
        lines.extend(
            entry
                .digests
                .iter()
                .map(|(name, hash)| format!("  {name}: {hash}")),
        );
    }
    Ok(lines.join("\n"))
}
/// A Markdown table, or `empty` when it has no rows.
fn table(header: &str, rows: Vec<String>, empty: &str) -> Vec<String> {
    if rows.is_empty() {
        return vec![empty.into()];
    }
    let divider = vec!["---"; header.matches('|').count() - 1].join(" | ");
    let mut lines = vec![header.into(), format!("| {divider} |")];
    lines.extend(rows);
    lines
}
fn clients<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> String {
    listed(entries.into_iter().map(|entry| &entry.client), "none")
}
/// Roll the displayed packs up by status, by finding code and by control not run.
fn summary(entries: &[Entry]) -> Vec<String> {
    let mut lines = vec![
        String::new(),
        format!("Summary of the {} displayed pack(s)", entries.len()),
        String::new(),
    ];
    let statuses = STATUSES.map(|status| {
        let matching: Vec<_> = entries.iter().filter(|e| e.status.is(status)).collect();
        format!("| {status} | {} | {} |", matching.len(), clients(matching))
    });
    lines.extend(table("| Status | Packs | Clients |", statuses.into(), ""));
    let mut raised: BTreeMap<&PyStr, (usize, usize)> = BTreeMap::new();
    for (code, count) in entries.iter().flat_map(|e| &e.findings_by_code) {
        let totals = raised.entry(code).or_default();
        totals.0 += 1;
        totals.1 += count;
    }
    let raised = raised
        .into_iter()
        .map(|(code, (packs, count))| format!("| {} | {packs} | {count} |", cell(code)));
    lines.push(String::new());
    lines.extend(table(
        "| Finding code | Packs | Findings |",
        raised.collect(),
        "No displayed pack records a finding.",
    ));
    lines.push(String::new());
    lines.extend(not_run(entries));
    lines
}
/// The controls that did not run, with the clients that omitted each.
fn not_run(entries: &[Entry]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut skipped: BTreeMap<&PyStr, BTreeSet<usize>> = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        for slot in entry.not_run.iter().flatten() {
            skipped.entry(slot).or_default().insert(index);
        }
    }
    let skipped = skipped.into_iter().map(|(slot, indices)| {
        let matching = indices.iter().map(|&i| &entries[i]);
        format!(
            "| {} | {} | {} |",
            cell(slot),
            indices.len(),
            clients(matching)
        )
    });
    lines.extend(table(
        "| Control not run | Packs | Clients |",
        skipped.collect(),
        "No displayed pack records a control that did not run.",
    ));
    let unrecorded = entries.iter().filter(|e| e.not_run.is_none()).count();
    if unrecorded > 0 {
        lines.push(format!(
            "{unrecorded} displayed pack(s) do not record control coverage, so their omitted controls are not counted."
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cell_escapes_control_characters_non_ascii_text_and_table_separators() {
        assert_eq!(cell(&"x|y\n\u{1b}é".into()), r"x\|y\n\u001b\u00e9");
    }
}
