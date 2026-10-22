//! Verify every explicitly supplied pack before applying display filters.
use crate::{
    Result, isodate,
    pyjson::{self, Json, PyStr},
    pypath, pytext,
    viewer::{self, VerifiedPack},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub struct Filters {
    pub period: Option<PyStr>,
    pub status: Option<PyStr>,
    pub reviewer: Option<PyStr>,
}
struct Entry {
    path: PathBuf,
    dates: Vec<PyStr>,
    status: PyStr,
    reviewer: Option<PyStr>,
    row: String,
    digests: BTreeMap<String, String>,
}
fn cell(value: PyStr) -> String {
    let encoded = pyjson::dumps(&Json::Str(value), false);
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
fn joined(values: &[Json], empty: &str) -> PyStr {
    if values.is_empty() {
        return empty.into();
    }
    let mut result = Vec::new();
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            result.extend([44, 32]);
        }
        result.extend(&value.as_pystr().unwrap().0);
    }
    PyStr(result)
}
fn entry(path: PathBuf, pack: VerifiedPack) -> Entry {
    let document = &pack.document;
    let dates = document.at("current_report_dates").as_array().unwrap();
    let acknowledgement = document.at("acknowledgement");
    let reviewer = acknowledgement
        .get("reviewer_initials")
        .and_then(Json::as_pystr)
        .cloned();
    let reviewed = if let Some(reviewer) = &reviewer {
        let mut points = reviewer.0.clone();
        points.extend([32, 40]);
        points.extend(&acknowledgement.at("reviewed_on").as_pystr().unwrap().0);
        points.push(41);
        PyStr(points)
    } else {
        "not recorded".into()
    };
    let values = [
        path_text(&path),
        joined(dates, "not recorded"),
        document.at("overall_status").as_pystr().unwrap().clone(),
        document
            .at("exceptions")
            .as_array()
            .unwrap()
            .len()
            .to_string()
            .as_str()
            .into(),
        document.get("client_queries").map_or_else(
            || PyStr::from("not recorded"),
            |v| v.as_array().unwrap().len().to_string().as_str().into(),
        ),
        reviewed,
        document.get("controls_not_run").map_or_else(
            || PyStr::from("not recorded"),
            |v| joined(v.as_array().unwrap(), "none"),
        ),
    ];
    Entry {
        dates: dates
            .iter()
            .map(|v| v.as_pystr().unwrap().clone())
            .collect(),
        status: document.at("overall_status").as_pystr().unwrap().clone(),
        reviewer,
        row: format!(
            "| {} |",
            values.into_iter().map(cell).collect::<Vec<_>>().join(" | ")
        ),
        digests: pack.artefact_sha256,
        path,
    }
}
fn validate(filters: &Filters) -> Result<()> {
    if let Some(period) = &filters.period {
        let raw = period.to_utf8();
        if !raw
            .as_deref()
            .and_then(isodate::python_date_fromisoformat)
            .is_some_and(|date| period.is(&date.iso()))
        {
            return Err("Period must be a calendar date in YYYY-MM-DD form.".into());
        }
    }
    if filters.status.as_ref().is_some_and(|status| {
        !["PASS", "REVIEW", "BLOCKED"]
            .iter()
            .any(|value| status.is(value))
    }) {
        return Err("Status must be PASS, REVIEW or BLOCKED.".into());
    }
    if filters
        .reviewer
        .as_ref()
        .is_some_and(|reviewer| reviewer.0.iter().all(|&cp| pytext::is_space(cp)))
    {
        return Err("Reviewer must not be empty.".into());
    }
    Ok(())
}
pub fn render_review_queue(paths: &[PathBuf], filters: &Filters) -> Result<String> {
    render_with(paths, filters, &mut viewer::verify_pack)
}
pub fn render_with(
    paths: &[PathBuf],
    filters: &Filters,
    verify: &mut impl FnMut(&Path) -> Result<VerifiedPack>,
) -> Result<String> {
    if paths.is_empty() {
        return Err("Supply at least one pack directory.".into());
    }
    validate(filters)?;
    let mut resolved = Vec::new();
    for path in paths {
        let path = pypath::resolve(path, false)
            .map_err(|_| "Cannot resolve pack directory: path is unusable")?;
        if !resolved
            .iter()
            .any(|other: &PathBuf| pypath::same_path(other, &path))
        {
            resolved.push(path);
        }
    }
    resolved.sort_by_cached_key(|path| PyStr::from_os_text(&pypath::text(path)));
    let mut verified = Vec::new();
    for path in resolved {
        let pack = verify(&path)
            .map_err(|error| format!("Pack {}: {error}", pytext::repr_os(&pypath::text(&path))))?;
        verified.push(entry(path, pack));
    }
    let count = verified.len();
    let mut selected: Vec<_> = verified
        .into_iter()
        .filter(|entry| {
            filters
                .period
                .as_ref()
                .is_none_or(|period| entry.dates.contains(period))
                && filters
                    .status
                    .as_ref()
                    .is_none_or(|status| &entry.status == status)
                && filters
                    .reviewer
                    .as_ref()
                    .is_none_or(|reviewer| entry.reviewer.as_ref() == Some(reviewer))
        })
        .collect();
    selected.sort_by_cached_key(|entry| {
        (
            entry.dates.clone(),
            PyStr::from_os_text(&pypath::text(&entry.path)),
        )
    });
    let mut lines = vec![
        "Close Review Queue".into(), String::new(),
        format!("Verified {count} distinct pack(s); displaying {}.", selected.len()),
        "Recorded states are review evidence. This queue does not approve a close or change a finding, acknowledgement or source file.".into(),
        "Recorded reviewers are acknowledgement evidence, not preparer assignments. Owners and due dates are not recorded by the pack schema.".into(), String::new(),
    ];
    if selected.is_empty() {
        lines.push("No verified packs match the supplied filters.".into());
        return Ok(lines.join("\n"));
    }
    lines.extend([
        "| Pack | Report dates | State | Exceptions | Draft queries | Recorded reviewer | Controls not run |".into(),
        "| --- | --- | --- | --- | --- | --- | --- |".into(),
    ]);
    lines.extend(selected.iter().map(|entry| entry.row.clone()));
    lines.extend([
        String::new(),
        "Verified artefact SHA-256".into(),
        String::new(),
    ]);
    for entry in selected {
        lines.push(cell(path_text(&entry.path)));
        lines.extend(
            entry
                .digests
                .iter()
                .map(|(name, hash)| format!("  {name}: {hash}")),
        );
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bad_filters_read_nothing_and_valid_filters_cannot_hide_an_invalid_pack() {
        let paths = [PathBuf::from("bad-pack")];
        let mut reads = 0;
        let mut verify = |_: &Path| {
            reads += 1;
            Err("broken witness".into())
        };
        let filters = Filters {
            period: Some("20260930".into()),
            ..Default::default()
        };
        assert_eq!(
            render_with(&paths, &filters, &mut verify).unwrap_err(),
            "Period must be a calendar date in YYYY-MM-DD form."
        );
        let filters = Filters {
            status: Some("PASS".into()),
            ..Default::default()
        };
        assert!(
            render_with(&paths, &filters, &mut verify)
                .unwrap_err()
                .ends_with(": broken witness")
        );
        assert_eq!(reads, 1);
        assert_eq!(cell("x|y\n\u{1b}é".into()), r"x\|y\n\u001b\u00e9");
    }
}
