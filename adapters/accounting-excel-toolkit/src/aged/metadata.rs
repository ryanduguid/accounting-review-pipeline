use super::*;

fn identity(
    records: &[Vec<String>],
    metadata: &[(usize, &str)],
    manifest: &Json,
    issues: &mut Vec<Json>,
) -> bool {
    let title = records
        .first()
        .and_then(|r| r.first())
        .map_or("", |s| s.trim_matches(space));
    if title != TITLE {
        issue(
            issues,
            "The first CSV record does not declare the supported report title.",
        );
    }
    if manifest
        .get("report_name")
        .is_some_and(|v| nonblank_value(v) && !v.is_str(TITLE))
    {
        issue(
            issues,
            "Manifest report_name does not declare the supported report title.",
        );
    }
    let entities: Vec<_> = metadata
        .iter()
        .filter(|(i, s)| *i != 0 && !s.starts_with("As at ") && !s.starts_with("Ageing by "))
        .map(|(_, s)| *s)
        .collect();
    let entity = entities.len() == 1
        && manifest
            .get("entity")
            .is_some_and(|v| v.is_str(entities[0]));
    if !entity {
        issue(
            issues,
            "Export metadata has a missing, duplicate or contradictory entity.",
        );
    }
    entity
}
fn cutoff(metadata: &[(usize, &str)], manifest: &Json, issues: &mut Vec<Json>) -> bool {
    if let Some(cutoff) = field(manifest, "as_at").filter(|s| date::cutoff(s)) {
        let dates: Vec<_> = metadata
            .iter()
            .filter_map(|(_, s)| s.strip_prefix("As at "))
            .collect();
        let agrees = dates.len() == 1
            && transaction_date::parse(dates[0], true).is_some_and(|d| d.iso() == cutoff);
        if !agrees {
            issue(issues, "Export and manifest cut-off dates do not agree.");
        }
        agrees
    } else {
        issue(issues, "Missing or unconfirmed ISO cut-off date.");
        false
    }
}
fn settings(metadata: &[(usize, &str)], manifest: &Json, issues: &mut Vec<Json>) {
    if !field(manifest, "generated_at").is_some_and(|s| date::generated(&s)) {
        issue(issues, "Missing or malformed generated timestamp.");
    }
    let ageing: Vec<_> = metadata
        .iter()
        .filter_map(|(_, s)| s.strip_prefix("Ageing by "))
        .collect();
    let default = Json::str("");
    let value = manifest.get("ageing_basis").unwrap_or(&default);
    let matches = if matches!(value, Json::Str(_)) {
        ageing.len() == 1 && value.is_str(ageing[0])
    } else {
        ageing == [json::str_value(value).as_str()]
    };
    if !matches {
        issue(
            issues,
            "Export metadata has a missing, duplicate or contradictory ageing basis.",
        );
    }
}
pub(super) fn metadata(
    records: &[Vec<String>],
    header: usize,
    manifest: &Json,
    digest: &str,
    issues: &mut Vec<Json>,
) -> Result<(bool, bool, Option<BigInt>), String> {
    for key in MANIFEST_FIELDS {
        if !manifest.get(key).is_some_and(nonblank_value) {
            issue(issues, format!("Missing manifest evidence: {key}."));
        }
    }
    if !manifest
        .get("source_sha256")
        .is_some_and(|v| v.is_str(digest))
    {
        issue(issues, "Manifest does not identify the exact source bytes.");
    }
    let metadata: Vec<_> = records[..header]
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.first().map(|s| (i, s.trim_matches(space))))
        .filter(|(_, s)| nonblank(s))
        .collect();
    let entity = identity(records, &metadata, manifest, issues);
    let cutoff = cutoff(&metadata, manifest, issues);
    settings(&metadata, manifest, issues);
    let footer = match manifest.get("footer_record") {
        None | Some(Json::Null) => {
            issue(issues, "Terminal export footer has not been identified.");
            None
        }
        Some(Json::Int(n)) if *n > BigInt::from(header + 1) => Some(n.clone()),
        _ => return Err("footer_record must identify a source record after the header.".into()),
    };
    Ok((entity, cutoff, footer))
}
