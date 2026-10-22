//! Read-only verification of the three witnesses of a readiness run.
use crate::{
    Result,
    decimal::Decimal,
    engine, isodate,
    loader::json_text,
    models::*,
    publication, pycsv,
    pyjson::{self, Json, LoadError, PyStr},
    pypath, pytext,
    report::{self, ACKNOWLEDGEMENT_EFFECT, CSV_FIELDS, PACK_NAMES, REVIEW_BOUNDARY},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const JSON_NAME: &str = "readiness-pack.json";
const SUMMARY_NAME: &str = "readiness-summary.md";
const CSV_NAME: &str = "findings.csv";
const JSON_MEMBERS: [&str; 9] = [
    "acknowledgement",
    "engagement_type",
    "findings",
    "overall_status",
    "period_end",
    "preparer_initials",
    "review_boundary",
    "source_sha256",
    "thresholds",
];
const FINDING_HEADER: &str = "| Status | Code | Slot | Repeat | Reason |";
const FINDING_DIVIDER: &str = "| --- | --- | --- | --- | --- |";

fn string(value: &Json) -> Option<String> {
    value.as_pystr()?.to_utf8()
}
fn value_repr(value: &Json) -> String {
    match value {
        Json::Str(s) => pytext::repr_code_points(&s.0),
        Json::Bool(true) => "True".into(),
        Json::Bool(false) => "False".into(),
        Json::Null => "None".into(),
        _ => pyjson::dumps(value, true),
    }
}
fn list_repr(mut keys: Vec<PyStr>) -> String {
    keys.sort();
    format!(
        "[{}]",
        keys.iter()
            .map(|s| pytext::repr_code_points(&s.0))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
pub(crate) fn parse_json(payload: &[u8]) -> Result<Json> {
    let text = std::str::from_utf8(payload).map_err(|_| format!("{JSON_NAME}: not valid UTF-8"))?;
    let document = pyjson::loads(text).map_err(|e| match e {
        LoadError::DuplicateKey(key) => format!(
            "{JSON_NAME}: member {} appears more than once",
            pytext::repr_code_points(&key.0)
        ),
        LoadError::Invalid => format!("{JSON_NAME}: not valid JSON (invalid JSON)"),
        LoadError::Crash(msg) if msg.contains("Recursion") || msg.contains("recursion") => {
            format!("{JSON_NAME}: nested too deeply to read")
        }
        LoadError::Crash(msg) => format!("{JSON_NAME}: not readable ({msg})"),
    })?;
    let Json::Object(pairs) = &document else {
        return Err(format!("{JSON_NAME}: top level must be a JSON object"));
    };
    let members: BTreeSet<_> = pairs.iter().map(|(k, _)| k.os_text()).collect();
    let unknown: Vec<_> = members
        .iter()
        .filter(|s| !JSON_MEMBERS.contains(&s.as_str()) && s.as_str() != "controls_not_run")
        .cloned()
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "{JSON_NAME}: unknown top-level member(s): {}",
            unknown.join(", ")
        ));
    }
    let mut missing: Vec<_> = JSON_MEMBERS
        .iter()
        .filter(|&&s| !members.contains(s))
        .copied()
        .collect();
    missing.sort();
    if !missing.is_empty() {
        return Err(format!(
            "{JSON_NAME}: missing top-level member(s): {}",
            missing.join(", ")
        ));
    }
    Ok(document)
}
fn threshold(value: &Json) -> Result<Decimal> {
    let text = string(value)
        .ok_or_else(|| format!("{JSON_NAME}: thresholds.tieout_tolerance must be a string"))?;
    let value = Decimal::parse(&text).map_err(|_| {
        format!(
            "{JSON_NAME}: thresholds.tieout_tolerance is not a decimal: {}",
            pytext::repr(&text)
        )
    })?;
    if !value.is_finite() || value < Decimal::zero() {
        return Err(format!(
            "{JSON_NAME}: thresholds.tieout_tolerance must be finite and non-negative"
        ));
    }
    Ok(value)
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn verify_schema(document: &Json) -> Result<()> {
    let status = document.at("overall_status");
    if !["READY", "NOT_READY", "BLOCKED"]
        .iter()
        .any(|s| status.is_str(s))
    {
        return Err(format!(
            "{JSON_NAME}: overall_status must be one of READY, NOT_READY, BLOCKED; got {}",
            value_repr(status)
        ));
    }
    let engagement = document.at("engagement_type");
    if !["bas", "month_end", "year_end"]
        .iter()
        .any(|s| engagement.is_str(s))
    {
        return Err(format!(
            "{JSON_NAME}: engagement_type must be one of bas, month_end, year_end; got {}",
            value_repr(engagement)
        ));
    }
    for member in ["period_end", "preparer_initials", "review_boundary"] {
        if string(document.at(member)).is_none() {
            return Err(format!("{JSON_NAME}: {member} must be a string"));
        }
    }
    if !document.at("review_boundary").is_str(REVIEW_BOUNDARY) {
        return Err(format!(
            "{JSON_NAME}: review_boundary does not match the written contract"
        ));
    }
    let thresholds = document.at("thresholds");
    if !thresholds.has_exact_keys(&["tieout_tolerance"]) {
        return Err(format!(
            "{JSON_NAME}: thresholds must hold exactly tieout_tolerance"
        ));
    }
    threshold(thresholds.at("tieout_tolerance"))?;
    let Json::Object(sources) = document.at("source_sha256") else {
        return Err(format!("{JSON_NAME}: source_sha256 must be an object"));
    };
    for (label, info) in sources {
        if label.to_utf8().is_none() {
            return Err(format!(
                "{JSON_NAME}: source_sha256 key is not valid Unicode"
            ));
        }
        let label = pytext::repr_code_points(&label.0);
        if !info.has_exact_keys(&["filename", "sha256"]) {
            return Err(format!(
                "{JSON_NAME}: source_sha256[{label}] must hold exactly filename and sha256"
            ));
        }
        if string(info.at("filename")).is_none_or(|s| s.is_empty()) {
            return Err(format!(
                "{JSON_NAME}: source_sha256[{label}].filename must be a non-empty string"
            ));
        }
        if json_text(info.at("sha256")).is_none_or(|s| !digest(&s)) {
            return Err(format!(
                "{JSON_NAME}: source_sha256[{label}].sha256 is not a lowercase SHA-256 digest"
            ));
        }
    }
    let ack = document.at("acknowledgement");
    if !matches!(ack, Json::Null) {
        if !ack.has_exact_keys(&["reviewer_initials", "reviewed_on", "comment", "effect"]) {
            return Err(format!(
                "{JSON_NAME}: acknowledgement must be null or hold exactly comment, effect, reviewed_on, reviewer_initials"
            ));
        }
        for key in ["comment", "effect", "reviewed_on", "reviewer_initials"] {
            if string(ack.at(key)).is_none() {
                return Err(format!(
                    "{JSON_NAME}: acknowledgement.{key} must be a string"
                ));
            }
        }
        if !ack.at("effect").is_str(ACKNOWLEDGEMENT_EFFECT) {
            return Err(format!(
                "{JSON_NAME}: acknowledgement.effect disagrees with the written contract"
            ));
        }
    }
    let Json::Array(findings) = document.at("findings") else {
        return Err(format!("{JSON_NAME}: findings must be a list"));
    };
    for (i, item) in findings.iter().enumerate() {
        let Json::Object(pairs) = item else {
            return Err(format!("{JSON_NAME}: findings[{i}] must be an object"));
        };
        if !item.has_exact_keys(&CSV_FIELDS) {
            return Err(format!(
                "{JSON_NAME}: findings[{i}] does not hold the fields the writer emits: holds {}, expected {}",
                list_repr(pairs.iter().map(|(k, _)| k.clone()).collect()),
                list_repr(CSV_FIELDS.iter().map(|s| (*s).into()).collect())
            ));
        }
        if !["NOT_READY", "BLOCKED"]
            .iter()
            .any(|s| item.at("status").is_str(s))
        {
            return Err(format!(
                "{JSON_NAME}: findings[{i}].status is not a finding status"
            ));
        }
    }
    if let Some(controls) = document.get("controls_not_run") {
        let Json::Array(controls) = controls else {
            return Err(format!("{JSON_NAME}: controls_not_run must be a list"));
        };
        for (i, c) in controls.iter().enumerate() {
            if !c.has_exact_keys(&["slot", "filename", "reason"]) {
                return Err(format!(
                    "{JSON_NAME}: controls_not_run[{i}] does not hold the fields the writer emits: expected ['filename', 'reason', 'slot']"
                ));
            }
            for key in ["filename", "reason", "slot"] {
                if string(c.at(key)).is_none_or(|s| s.is_empty()) {
                    return Err(format!(
                        "{JSON_NAME}: controls_not_run[{i}].{key} must be a non-empty string"
                    ));
                }
            }
        }
    }
    Ok(())
}
fn read_csv(payload: &[u8]) -> Result<Vec<Vec<String>>> {
    let payload = payload.strip_prefix(b"\xef\xbb\xbf").unwrap_or(payload);
    let text = std::str::from_utf8(payload).map_err(|_| format!("{CSV_NAME}: not valid UTF-8"))?;
    let mut reader = pycsv::Reader::new(text);
    let csv_error = || format!("{CSV_NAME}: not valid CSV: field larger than field limit (131072)");
    if reader
        .next()
        .transpose()
        .map_err(|_| csv_error())?
        .as_deref()
        != Some(&CSV_FIELDS.map(str::to_string))
    {
        return Err(format!(
            "{CSV_NAME}: header row does not match the written contract"
        ));
    }
    let mut rows = Vec::new();
    for (i, row) in reader.enumerate() {
        let row = row.map_err(|_| csv_error())?;
        if row.len() != CSV_FIELDS.len() {
            return Err(format!(
                "{CSV_NAME}: row {} holds {} cells, but the header declares {}",
                i + 1,
                row.len(),
                CSV_FIELDS.len()
            ));
        }
        rows.push(row);
    }
    Ok(rows)
}
fn summary_evidence(summary: &str) -> Vec<SourceEvidence> {
    let mut found = Vec::new();
    for (at, _) in summary.match_indices("- `") {
        let mut text = &summary[at + 3..];
        let Some(end) = text.find('`') else {
            continue;
        };
        if end == 0 {
            continue;
        }
        let slot = &text[..end];
        text = &text[end..];
        let Some(rest) = text.strip_prefix("` (`") else {
            continue;
        };
        text = rest;
        let Some(end) = text.find('`') else {
            continue;
        };
        if end == 0 {
            continue;
        }
        let filename = &text[..end];
        text = &text[end..];
        let Some(rest) = text.strip_prefix("`): `") else {
            continue;
        };
        text = rest;
        let Some(hash) = text.get(..64) else {
            continue;
        };
        if digest(hash) && text.as_bytes().get(64) == Some(&b'`') {
            found.push(SourceEvidence {
                slot: slot.into(),
                filename: filename.into(),
                sha256: hash.into(),
            });
        }
    }
    found
}
fn md_cells(line: &str) -> Vec<String> {
    let mut chars = line.chars();
    let mut cells = Vec::new();
    let mut cell = String::new();
    while let Some(c) = chars.next() {
        if c == '\\' {
            cell.push(c);
            if let Some(next) = chars.next() {
                cell.push(next);
            }
        } else if c == '|' {
            cells.push(std::mem::take(&mut cell));
        } else {
            cell.push(c);
        }
    }
    cells.push(cell);
    if cells.first().is_some_and(|c| pytext::strip(c).is_empty()) {
        cells.remove(0);
    }
    if cells.last().is_some_and(|c| pytext::strip(c).is_empty()) {
        cells.pop();
    }
    cells.iter().map(|c| pytext::strip(c).into()).collect()
}
fn scope_values(summary: &str, label: &str) -> Vec<String> {
    let prefix = format!("- {label}: ");
    pytext::split_lines(summary)
        .iter()
        .filter_map(|line| line.strip_prefix(&prefix).map(str::to_owned))
        .collect()
}
fn verify_agreement(
    document: &Json,
    summary: &str,
    csv: &[Vec<String>],
) -> Result<Vec<SourceEvidence>> {
    let statuses: Vec<_> = summary
        .match_indices("**Overall status: ")
        .filter_map(|(at, _)| {
            let text = &summary[at + "**Overall status: ".len()..];
            ["READY", "NOT_READY", "BLOCKED"]
                .into_iter()
                .find(|s| text.starts_with(&format!("{s}**")))
        })
        .collect();
    if statuses.len() != 1 {
        return Err(format!(
            "{SUMMARY_NAME}: expected exactly one overall-status line, found {}",
            statuses.len()
        ));
    }
    let status = string(document.at("overall_status")).unwrap();
    if status != statuses[0] {
        return Err(format!(
            "overall status disagrees: {JSON_NAME} says {status}, {SUMMARY_NAME} says {}",
            statuses[0]
        ));
    }
    if !summary.contains(REVIEW_BOUNDARY) {
        return Err(format!(
            "{SUMMARY_NAME}: the review-boundary statement is missing or altered"
        ));
    }
    let evidence = summary_evidence(summary);
    let Json::Object(sources) = document.at("source_sha256") else {
        unreachable!()
    };
    if sources.is_empty() {
        if !evidence.is_empty() {
            return Err(format!(
                "source evidence disagrees: {SUMMARY_NAME} lists digests but {JSON_NAME} lists none"
            ));
        }
    } else {
        let mut hashes = BTreeMap::new();
        for e in &evidence {
            if hashes
                .insert(e.slot.clone(), (e.filename.clone(), e.sha256.clone()))
                .is_some()
            {
                return Err(format!(
                    "{SUMMARY_NAME}: source slot {} carries more than one evidence line",
                    pytext::repr(&e.slot)
                ));
            }
        }
        if hashes.is_empty() {
            return Err(format!(
                "{SUMMARY_NAME}: no source-evidence digest lines found"
            ));
        }
        let expected: BTreeMap<_, _> = sources
            .iter()
            .map(|(k, v)| {
                (
                    k.to_utf8().unwrap(),
                    (
                        string(v.at("filename")).unwrap(),
                        string(v.at("sha256")).unwrap(),
                    ),
                )
            })
            .collect();
        if hashes != expected {
            return Err(format!(
                "source evidence disagrees: {SUMMARY_NAME} and {JSON_NAME} list different source digests"
            ));
        }
    }
    let Json::Array(findings) = document.at("findings") else {
        unreachable!()
    };
    if csv.len() != findings.len() {
        return Err(format!(
            "finding counts disagree: {JSON_NAME} holds {}, {CSV_NAME} holds {} data rows",
            findings.len(),
            csv.len()
        ));
    }
    for (i, (item, row)) in findings.iter().zip(csv).enumerate() {
        for (col, field) in CSV_FIELDS.iter().enumerate() {
            let mut expected = string(item.at(field))
                .ok_or_else(|| format!("{JSON_NAME}: findings[{i}].{field} must be a string"))?;
            if ["slot", "reason", "reviewer_action"].contains(field) {
                expected = report::csv_safe(&expected);
            }
            if expected != row[col] {
                return Err(format!(
                    "{field} disagrees on finding {}: {JSON_NAME} says {}, {CSV_NAME} says {}",
                    i + 1,
                    pytext::repr(&expected),
                    pytext::repr(&row[col])
                ));
            }
        }
    }
    if findings.is_empty() {
        if summary.contains(FINDING_HEADER) {
            return Err(format!(
                "findings disagree: {SUMMARY_NAME} tabulates findings but {JSON_NAME} lists none"
            ));
        }
    } else {
        let lines = pytext::split_lines(summary);
        let headers: Vec<_> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| **line == FINDING_HEADER)
            .map(|(i, _)| i)
            .collect();
        if headers.len() != 1 {
            return Err(format!(
                "{SUMMARY_NAME}: expected exactly one findings-table header, found {}",
                headers.len()
            ));
        }
        let divider = headers[0] + 1;
        if lines.get(divider) != Some(&FINDING_DIVIDER) {
            return Err(format!(
                "{SUMMARY_NAME}: the findings table has no delimiter row"
            ));
        }
        let rows: Vec<_> = lines[divider + 1..]
            .iter()
            .take_while(|line| line.starts_with('|'))
            .map(|line| md_cells(line))
            .collect();
        if rows.len() != findings.len() {
            return Err(format!(
                "finding counts disagree: {JSON_NAME} holds {}, {SUMMARY_NAME} tabulates {}",
                findings.len(),
                rows.len()
            ));
        }
        for (i, (item, row)) in findings.iter().zip(rows).enumerate() {
            if row.len() != 5 {
                return Err(format!(
                    "{SUMMARY_NAME}: finding {} has {} cells, expected 5",
                    i + 1,
                    row.len()
                ));
            }
            let repeat = string(item.at("repeat")).unwrap();
            if !["true", "false"].contains(&repeat.as_str()) {
                return Err(format!(
                    "{JSON_NAME}: findings[{i}].repeat must be 'true' or 'false'"
                ));
            }
            let expected = [
                string(item.at("status")).unwrap(),
                string(item.at("code")).unwrap(),
                report::md_cell(&string(item.at("slot")).unwrap()),
                if repeat == "true" {
                    "yes".into()
                } else {
                    "no".into()
                },
                report::md_cell(&string(item.at("reason")).unwrap()),
            ];
            for ((column, expected), actual) in ["status", "code", "slot", "repeat", "reason"]
                .iter()
                .zip(expected)
                .zip(row)
            {
                if expected != actual {
                    return Err(format!(
                        "{column} disagrees on finding {}: {JSON_NAME} says {}, {SUMMARY_NAME} says {}",
                        i + 1,
                        pytext::repr(&expected),
                        pytext::repr(&actual)
                    ));
                }
            }
        }
    }
    let absent = |s: String| if s.is_empty() { "n/a".into() } else { s };
    let initials = string(document.at("preparer_initials")).unwrap();
    for (label, expected) in [
        (
            "Engagement type",
            string(document.at("engagement_type")).unwrap(),
        ),
        (
            "Period end",
            absent(string(document.at("period_end")).unwrap()),
        ),
        (
            "Preparer initials",
            if initials.is_empty() {
                "n/a".into()
            } else {
                report::md_cell(&initials)
            },
        ),
        (
            "Tie-out tolerance",
            format!(
                "${}",
                string(document.at("thresholds").at("tieout_tolerance")).unwrap()
            ),
        ),
    ] {
        let values = scope_values(summary, label);
        if values.len() != 1 {
            return Err(format!(
                "{SUMMARY_NAME}: expected exactly one {} scope line, found {}",
                pytext::repr(label),
                values.len()
            ));
        }
        if values[0] != expected {
            return Err(format!(
                "{} disagrees: {JSON_NAME} says {}, {SUMMARY_NAME} says {}",
                label.to_lowercase(),
                pytext::repr(&expected),
                pytext::repr(&values[0])
            ));
        }
    }
    let counts = scope_values(summary, "Findings");
    if counts.len() != 1 {
        return Err(format!(
            "{SUMMARY_NAME}: expected exactly one 'Findings' scope line, found {}",
            counts.len()
        ));
    }
    let blocked = findings
        .iter()
        .filter(|f| f.at("status").is_str("BLOCKED"))
        .count();
    let not_ready = findings
        .iter()
        .filter(|f| f.at("status").is_str("NOT_READY"))
        .count();
    let repeats = findings
        .iter()
        .filter(|f| f.at("repeat").is_str("true"))
        .count();
    let expected = format!(
        "{} total; {blocked} blocked; {not_ready} not ready; {repeats} repeats.",
        findings.len()
    );
    if counts[0] != expected {
        return Err(format!(
            "finding counts disagree: {JSON_NAME} says {}, {SUMMARY_NAME} says {}",
            pytext::repr(&expected),
            pytext::repr(&counts[0])
        ));
    }
    Ok(evidence)
}
pub(crate) fn canonical_pack(
    document: &Json,
    evidence: Vec<SourceEvidence>,
) -> Result<ReadinessPack> {
    let ack = document.at("acknowledgement");
    let acknowledgement = if matches!(ack, Json::Null) {
        None
    } else {
        let reviewed = string(ack.at("reviewed_on")).unwrap();
        let cps: Vec<_> = reviewed.chars().collect();
        if cps.len() != 10
            || cps[4] != '-'
            || cps[7] != '-'
            || cps
                .iter()
                .enumerate()
                .any(|(i, c)| i != 4 && i != 7 && pytext::decimal_value(*c as u32).is_none())
        {
            return Err(format!(
                "{JSON_NAME}: acknowledgement.reviewed_on must be an extended-format ISO date, YYYY-MM-DD"
            ));
        }
        let date = isodate::date_fromisoformat(&reviewed)
            .ok_or_else(|| format!("{JSON_NAME}: acknowledgement.reviewed_on is not a date"))?;
        Some(ReviewerAcknowledgement {
            reviewer_initials: string(ack.at("reviewer_initials")).unwrap(),
            reviewed_on: date,
            comment: string(ack.at("comment")).unwrap(),
        })
    };
    let Json::Array(findings) = document.at("findings") else {
        unreachable!()
    };
    let findings = findings
        .iter()
        .map(|f| Finding {
            code: string(f.at("code")).unwrap(),
            status: string(f.at("status")).unwrap(),
            slot: string(f.at("slot")).unwrap(),
            reason: string(f.at("reason")).unwrap(),
            reviewer_action: string(f.at("reviewer_action")).unwrap(),
            repeat: f.at("repeat").is_str("true"),
        })
        .collect();
    let controls_not_run = document.get("controls_not_run").map(|controls| {
        let Json::Array(controls) = controls else {
            unreachable!()
        };
        controls
            .iter()
            .map(|c| ControlNotRun {
                slot: string(c.at("slot")).unwrap(),
                filename: string(c.at("filename")).unwrap(),
                reason: string(c.at("reason")).unwrap(),
            })
            .collect()
    });
    Ok(ReadinessPack {
        status: string(document.at("overall_status")).unwrap(),
        engagement_type: string(document.at("engagement_type")).unwrap(),
        period_end: string(document.at("period_end")).unwrap(),
        preparer_initials: string(document.at("preparer_initials")).unwrap(),
        findings,
        source_evidence: evidence,
        tieout_tolerance: threshold(document.at("thresholds").at("tieout_tolerance"))?,
        acknowledgement,
        controls_not_run,
    })
}
pub struct VerifiedPack {
    pub document: Json,
    pub summary: String,
    pub artefact_sha256: BTreeMap<String, String>,
}
pub fn verify_pack(directory: &Path) -> Result<VerifiedPack> {
    let revision = publication::snapshot_revision(directory).map_err(|e| e.to_string())?;
    let mut payloads = Vec::new();
    for name in PACK_NAMES {
        let path = directory.join(name);
        let read = || -> Result<Vec<u8>> {
            let m = fs::symlink_metadata(&path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    format!("{name}: not found in {}", pypath::text(directory))
                } else {
                    format!("{name}: cannot read artefact: {e}")
                }
            })?;
            if !publication::single_link(&path, &m)
                .map_err(|e| format!("{name}: cannot read artefact: {e}"))?
            {
                return Err(format!("{name}: expected a regular, single-link artefact"));
            }
            fs::read(&path).map_err(|e| format!("{name}: cannot read artefact: {e}"))
        };
        payloads.push(read()?);
    }
    if publication::snapshot_revision(directory).map_err(|e| e.to_string())? != revision {
        return Err("pack publication changed during acquisition; retry".into());
    }
    let document = parse_json(&payloads[0])?;
    verify_schema(&document)?;
    let summary = std::str::from_utf8(&payloads[1])
        .map_err(|_| format!("{SUMMARY_NAME}: not valid UTF-8"))?
        .to_owned();
    let csv = read_csv(&payloads[2])?;
    let evidence = verify_agreement(&document, &summary, &csv)?;
    let pack = canonical_pack(&document, evidence)?;
    if pack.status != engine::overall(&pack.findings) {
        return Err(format!(
            "{JSON_NAME}: overall_status disagrees with the findings"
        ));
    }
    if summary.replace("\r\n", "\n") != report::as_markdown(&pack)? {
        return Err(format!(
            "{SUMMARY_NAME}: document does not match its canonical rendering"
        ));
    }
    let artefact_sha256 = PACK_NAMES
        .iter()
        .zip(payloads)
        .map(|(name, bytes)| ((*name).into(), crate::sha256(&bytes)))
        .collect();
    Ok(VerifiedPack {
        document,
        summary,
        artefact_sha256,
    })
}
pub fn render_review_sheet(directory: &Path, documents: &[PathBuf]) -> Result<String> {
    let pack = verify_pack(directory)?;
    let mut lines = vec![
        pack.summary
            .trim_end_matches(|c: char| pytext::is_space(c as u32))
            .to_owned(),
        "".into(),
        "## Artefact digests".into(),
        "".into(),
    ];
    for name in PACK_NAMES {
        lines.push(format!("- `{name}`: `{}`", pack.artefact_sha256[name]));
    }
    if !documents.is_empty() {
        let bundles = documents
            .iter()
            .map(|p| crate::documents::load_document(p))
            .collect::<Result<Vec<_>>>()?;
        let supplied: BTreeMap<_, _> = bundles
            .iter()
            .enumerate()
            .flat_map(|(i, b)| b.evidence(i + 1))
            .map(|e| (e.slot, (e.filename, e.sha256)))
            .collect();
        let Json::Object(sources) = pack.document.at("source_sha256") else {
            unreachable!()
        };
        let recorded: BTreeMap<_, _> = sources
            .iter()
            .filter(|(k, _)| k.to_utf8().unwrap().starts_with("document_"))
            .map(|(k, v)| {
                (
                    k.to_utf8().unwrap(),
                    (
                        string(v.at("filename")).unwrap(),
                        string(v.at("sha256")).unwrap(),
                    ),
                )
            })
            .collect();
        if supplied != recorded {
            return Err("Document bytes or order differ from the saved readiness pack.".into());
        }
        lines.extend(bundles.iter().map(crate::documents::render_document));
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}
