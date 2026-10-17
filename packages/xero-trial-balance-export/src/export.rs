use crate::{
    Failure, Result, amount,
    http::{self, Credentials, Http},
    publication,
    pyjson::Json,
    pytext, report,
    tokens::TokenSession,
};
use std::path::Path;
use unicode_normalization::UnicodeNormalization;

pub fn validated_connections(value: &Json) -> Result<&[Json]> {
    let Json::Array(items) = value else {
        return Err(Failure::message(
            "error: Xero connections response is not a list.",
        ));
    };
    for (index, item) in items.iter().enumerate() {
        if !matches!(item, Json::Object(_)) {
            return Err(Failure::message(format!(
                "error: Xero connection {} is not an object.",
                index + 1
            )));
        }
        for field in ["tenantId", "tenantName"] {
            if !item.get(field).and_then(Json::as_pystr).is_some_and(|s| {
                let value = s.os_text();
                !pytext::strip(&value).is_empty()
                    && !s.0.iter().any(|&cp| cp <= 31 || (127..=159).contains(&cp))
            }) {
                return Err(Failure::message(format!(
                    "error: Xero connection {} has an invalid {field}.",
                    index + 1
                )));
            }
        }
    }
    Ok(items)
}

fn tenant_ids(items: &[&Json]) -> String {
    items
        .iter()
        .map(|item| report::text(item.at("tenantId")))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn select_tenant<'a>(connections: &'a [Json], requested: Option<&str>) -> Result<&'a Json> {
    let ambiguity = |requested: &str, matches: &[&Json]| {
        Failure::message(format!(
            "\"{requested}\" matches more than one organisation ({}; tenant ids: {}) - narrow it.",
            matches.len(),
            tenant_ids(matches)
        ))
    };
    if let Some(requested) = requested.filter(|v| !v.is_empty()) {
        let needle = pytext::lower(requested);
        let ids: Vec<_> = connections
            .iter()
            .filter(|item| needle == pytext::lower(&report::text(item.at("tenantId"))))
            .collect();
        if ids.len() == 1 {
            return Ok(ids[0]);
        }
        if ids.len() > 1 {
            return Err(ambiguity(requested, &ids));
        }
        let matches: Vec<_> = connections
            .iter()
            .filter(|item| pytext::lower(&report::text(item.at("tenantName"))).contains(&needle))
            .collect();
        return match matches.as_slice() {
            [] => Err(Failure::message(format!(
                "No tenant matching \"{requested}\". {} organisation(s) connected; tenant ids: {}.",
                connections.len(),
                tenant_ids(&connections.iter().collect::<Vec<_>>())
            ))),
            [one] => Ok(one),
            _ => Err(ambiguity(requested, &matches)),
        };
    }
    if connections.len() > 1 {
        return Err(Failure::message(format!(
            "More than one organisation connected ({}; tenant ids: {}) - pick one with --tenant.",
            connections.len(),
            tenant_ids(&connections.iter().collect::<Vec<_>>())
        )));
    }
    connections.first().ok_or_else(|| {
        Failure::message("No Xero organisations authorised for this app - run xero-tb-auth again.")
    })
}

fn filename_segment(value: &str) -> String {
    let mut out = String::new();
    let mut unsafe_run = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
            unsafe_run = false;
        } else if !unsafe_run {
            out.push('-');
            unsafe_run = true;
        }
    }
    out
}

pub fn default_filename(tenant: &Json, date: &str, basis: &str) -> String {
    let name: String = report::text(tenant.at("tenantName")).nfc().collect();
    let stem = filename_segment(&name)
        .trim_matches('-')
        .to_ascii_lowercase();
    let discriminator: String = filename_segment(&report::text(tenant.at("tenantId")))
        .chars()
        .take(8)
        .collect();
    let entity = [stem.as_str(), discriminator.trim_matches('-')]
        .into_iter()
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("{entity}-tb-{date}-{basis}.csv")
}

pub fn validate_report_identity(report: &Json, requested: &str) -> Result<()> {
    if ["ReportID", "ReportType"]
        .iter()
        .any(|key| !report.get(key).is_some_and(|v| v.is_str("TrialBalance")))
    {
        return Err(Failure::message(
            "error: returned report is not identified as a Trial Balance.",
        ));
    }
    let Some(Json::Array(titles)) = report.get("ReportTitles") else {
        return Err(Failure::message(
            "error: Trial Balance ReportTitles must be a list of text titles.",
        ));
    };
    if !titles.iter().all(|v| matches!(v, Json::Str(_))) {
        return Err(Failure::message(
            "error: Trial Balance ReportTitles must be a list of text titles.",
        ));
    }
    let as_at: Vec<_> = titles
        .iter()
        .map(report::text)
        .filter(|t| t.starts_with("As at "))
        .collect();
    let actual = match as_at.as_slice() {
        [one] => as_at_date(one),
        _ => None,
    };
    let Some(actual) = actual else {
        return Err(Failure::message(
            "error: Trial Balance needs one valid 'As at D Month YYYY' title.",
        ));
    };
    if actual != requested {
        return Err(Failure::message(
            "error: Trial Balance as-at title does not match the requested date.",
        ));
    }
    Ok(())
}

fn as_at_date(title: &str) -> Option<String> {
    let fields: Vec<_> = title.strip_prefix("As at ")?.split(' ').collect();
    if fields.len() != 3
        || !(1..=2).contains(&fields[0].len())
        || fields[2].len() != 4
        || !fields[0]
            .bytes()
            .chain(fields[2].bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let months = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let month = months.iter().position(|&m| m == fields[1])? as u32 + 1;
    let year: i32 = fields[2].parse().ok()?;
    if !(1..=9999).contains(&year) {
        return None;
    }
    let date = chrono::NaiveDate::from_ymd_opt(year, month, fields[0].parse().ok()?)?;
    Some(date.format("%Y-%m-%d").to_string())
}

pub fn validated_rows(payload: &Json, date: &str) -> Result<Vec<report::Record>> {
    if !matches!(payload, Json::Object(_)) {
        return Err(Failure::message(
            "error: Xero Trial Balance response is not a JSON object.",
        ));
    }
    let reports = payload.get("Reports").unwrap_or(&Json::Null);
    if !report::truth(reports) {
        return Err(Failure::message(
            "Empty Reports payload - check the date parameter and API scopes.",
        ));
    }
    let Json::Array(reports) = reports else {
        return Err(Failure::message(
            "error: Xero Trial Balance response has an unexpected Reports shape.",
        ));
    };
    if !reports
        .first()
        .is_some_and(|r| matches!(r, Json::Object(_)))
    {
        return Err(Failure::message(
            "error: Xero Trial Balance response has an unexpected Reports shape.",
        ));
    }
    if reports.len() != 1 {
        return Err(Failure::message(
            "error: Xero Trial Balance response must contain exactly one report.",
        ));
    }
    validate_report_identity(&reports[0], date)?;
    let (header, rows) = report::flatten(&reports[0])?;
    if rows.is_empty() {
        return Err(Failure::message(
            "Report contained no account rows - nothing to export.",
        ));
    }
    let mut missing: Vec<_> = ["Account", "Debit", "Credit", "YTD Debit", "YTD Credit"]
        .into_iter()
        .filter(|&c| !header.iter().any(|h| h == c))
        .collect();
    missing.sort();
    if !missing.is_empty() {
        return Err(Failure::message(format!(
            "Unexpected report columns - missing [{}]. Has the API shape changed?",
            missing
                .iter()
                .map(|v| pytext::repr(v))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(rows)
}

pub fn fetch_rows(
    http: &impl Http,
    session: &TokenSession,
    credentials: &Credentials,
    tenant: &Json,
    date: &str,
    cash: bool,
) -> Result<Vec<report::Record>> {
    let mut params = vec![("date", date)];
    if cash {
        params.push(("paymentsOnly", "true"));
    }
    let payload = http::api_get(
        http,
        session,
        credentials,
        http::REPORT_URL,
        Some(&report::text(tenant.at("tenantId"))),
        &params,
    )?;
    validated_rows(&payload, date)
}

pub fn publish(
    rows: &[report::Record],
    tenant: &Json,
    date: &str,
    path: &Path,
    cash: bool,
    quiet: bool,
    no_manifest: bool,
) -> Result<()> {
    let (output, totals) = report::build_rows(rows, tenant, date)?;
    amount::check_balanced(&totals, Some(output.len()))?;
    let aside = publication::set_aside_manifest(path, quiet)?;
    let digest = match publication::write_csv(&output, path, quiet) {
        Ok(digest) => digest,
        Err(error) => {
            if let Some(aside) = aside {
                publication::restore_manifest(&aside, path, quiet);
            }
            return Err(error);
        }
    };
    if let Some(aside) = aside {
        publication::discard_manifest(&aside, quiet);
    }
    if quiet {
        crate::outln!(
            "Wrote {} accounts to {} (filename withheld under --quiet)",
            output.len(),
            crate::pypath::text(path.parent().unwrap())
        );
    } else {
        crate::outln!(
            "Wrote {} accounts to {}",
            output.len(),
            crate::pypath::text(path)
        );
        crate::outln!(
            "Balance check OK: movement debits = credits = {}; YTD = {}",
            totals[0].grouped_two(),
            totals[2].grouped_two()
        );
    }
    if !no_manifest {
        let manifest = publication::write_manifest(
            path,
            &digest,
            tenant,
            date,
            if cash { "cash" } else { "accrual" },
            quiet,
        )?;
        if quiet {
            crate::outln!("Wrote manifest beside the export (filename withheld under --quiet)");
        } else {
            crate::outln!("Wrote manifest {}", crate::pypath::text(&manifest));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        publication::{self, Stage},
        stdio,
        testing::Scratch,
    };
    use sha2::Digest;
    use std::fs;

    fn fabricated() -> (Vec<report::Record>, Json) {
        let row = |id: &str, debit: &str, credit: &str, ytd_debit: &str, ytd_credit: &str| {
            [
                ("AccountID", id),
                ("Account", "Synthetic Account (200)"),
                ("Section", "Assets"),
                ("Debit", debit),
                ("Credit", credit),
                ("YTD Debit", ytd_debit),
                ("YTD Credit", ytd_credit),
            ]
            .into_iter()
            .map(|(key, value)| (key.into(), Json::str(value)))
            .collect()
        };
        (
            vec![
                row("synthetic-1", "1.250", "0", "12.50", "0"),
                row("synthetic-2", "0", "1.250", "0", "12.50"),
            ],
            Json::object([
                ("tenantId", Json::str("synthetic-tenant")),
                ("tenantName", Json::str("Synthetic Organisation")),
            ]),
        )
    }

    #[test]
    fn publish_reports_totals_then_quiet_rerun_removes_the_prior_sidecar() {
        let scratch = Scratch::new();
        let path = scratch.0.join("synthetic-organisation.csv");
        let (rows, tenant) = fabricated();
        let (result, stdout, stderr) =
            stdio::capture(|| publish(&rows, &tenant, "2026-06-30", &path, true, false, false));
        result.unwrap();
        assert!(stderr.is_empty());
        assert!(stdout.contains("Wrote 2 accounts to "));
        assert!(stdout.contains("movement debits = credits = 1.25; YTD = 12.50"));
        assert!(stdout.contains("Wrote manifest "));
        let bytes = fs::read(&path).unwrap();
        let sidecar = publication::manifest_path(&path);
        let manifest = crate::pyjson::loads(&fs::read_to_string(&sidecar).unwrap()).unwrap();
        assert!(manifest.at("report").at("basis").is_str("cash"));
        let digest: String = sha2::Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert!(manifest.at("export").at("sha256").is_str(&digest));
        let (result, stdout, stderr) =
            stdio::capture(|| publish(&rows, &tenant, "2026-06-30", &path, false, true, true));
        result.unwrap();
        assert!(stderr.is_empty());
        assert!(stdout.contains("Wrote 2 accounts to "));
        assert!(stdout.contains("filename withheld under --quiet"));
        for hidden in [
            "synthetic-organisation",
            "Synthetic Organisation",
            "1.25",
            "12.50",
            "manifest",
        ] {
            assert!(!stdout.contains(hidden));
        }
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(!sidecar.exists());
        assert!(!publication::append(&sidecar, ".previous").exists());
        assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
    }

    #[test]
    fn invalid_totals_or_ids_leave_the_prior_pair_unchanged() {
        let scratch = Scratch::new();
        let path = scratch.0.join("prior.csv");
        let sidecar = publication::manifest_path(&path);
        fs::write(&path, b"synthetic prior CSV").unwrap();
        fs::write(&sidecar, b"synthetic prior manifest").unwrap();
        let (mut rows, tenant) = fabricated();
        rows[0]
            .iter_mut()
            .find(|(key, _)| key == "Debit")
            .unwrap()
            .1 = Json::str("2.00");
        for invalid in ["Debit", "AccountID"] {
            if invalid == "AccountID" {
                rows[0]
                    .iter_mut()
                    .find(|(key, _)| key == "AccountID")
                    .unwrap()
                    .1 = Json::str("");
            }
            let (result, out, err) = stdio::capture(|| {
                publish(&rows, &tenant, "2026-06-30", &path, false, false, false)
            });
            assert!(result.is_err());
            assert!(out.is_empty());
            assert!(err.is_empty());
            assert_eq!(fs::read(&path).unwrap(), b"synthetic prior CSV");
            assert_eq!(fs::read(&sidecar).unwrap(), b"synthetic prior manifest");
            assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 2);
        }
    }

    #[test]
    fn csv_failure_restores_the_prior_pair_and_keeps_complete_recovery_only() {
        let (rows, tenant) = fabricated();
        for stage in [Stage::Write, Stage::Flush, Stage::Replace] {
            let scratch = Scratch::new();
            let path = scratch.0.join("private-synthetic-name.csv");
            let sidecar = publication::manifest_path(&path);
            fs::write(&path, b"synthetic prior CSV").unwrap();
            fs::write(&sidecar, b"synthetic prior manifest").unwrap();
            let (result, out, err) = stdio::capture(|| {
                publication::with_failure(stage, || {
                    publish(&rows, &tenant, "2026-06-30", &path, false, true, false)
                })
            });
            let failure = result.unwrap_err();
            assert!(out.is_empty());
            assert!(err.is_empty());
            assert!(!failure.stderr.contains("private-synthetic-name"));
            assert_eq!(fs::read(&path).unwrap(), b"synthetic prior CSV");
            assert_eq!(fs::read(&sidecar).unwrap(), b"synthetic prior manifest");
            assert!(!publication::append(&sidecar, ".previous").exists());
            assert_eq!(
                fs::read_dir(&scratch.0).unwrap().count(),
                if stage == Stage::Write { 2 } else { 3 }
            );
        }
    }

    #[test]
    fn manifest_failure_keeps_the_completed_new_csv_and_recovery_manifest() {
        let (rows, tenant) = fabricated();
        for stage in [Stage::Write, Stage::Flush, Stage::Replace] {
            let scratch = Scratch::new();
            let path = scratch.0.join("private-synthetic-name.csv");
            let sidecar = publication::manifest_path(&path);
            fs::write(&path, b"synthetic prior CSV").unwrap();
            fs::write(&sidecar, b"synthetic prior manifest").unwrap();
            let (result, out, err) = stdio::capture(|| {
                publication::with_failure_after(stage, 1, || {
                    publish(&rows, &tenant, "2026-06-30", &path, false, true, false)
                })
            });
            let failure = result.unwrap_err();
            assert!(out.contains("Wrote 2 accounts"));
            assert!(err.is_empty());
            assert!(failure.stderr.contains("export is complete"));
            assert!(!failure.stderr.contains("private-synthetic-name"));
            assert!(fs::read(&path).unwrap().starts_with(&[0xef, 0xbb, 0xbf]));
            assert!(!sidecar.exists());
            assert!(!publication::append(&sidecar, ".previous").exists());
            assert_eq!(
                fs::read_dir(&scratch.0).unwrap().count(),
                if stage == Stage::Write { 1 } else { 2 }
            );
        }
    }
    #[test]
    fn tenant_selection_and_filename_discriminator() {
        let connections = Json::Array(vec![
            Json::object([
                ("tenantId", Json::str("abcd1234-x")),
                ("tenantName", Json::str("Ngā Company")),
            ]),
            Json::object([
                ("tenantId", Json::str("abcd5678-y")),
                ("tenantName", Json::str("Ngā Company")),
            ]),
        ]);
        let connections = validated_connections(&connections).unwrap();
        assert!(select_tenant(connections, None).is_err());
        assert!(select_tenant(connections, Some("Company")).is_err());
        let selected = select_tenant(connections, Some("ABCD1234-X")).unwrap();
        assert_eq!(
            default_filename(selected, "2026-06-30", "accrual"),
            "ng-company-abcd1234-tb-2026-06-30-accrual.csv"
        );
    }
}
