//! Development tools read only the declared fabricated fixtures and sample.
use crate::{
    Failure, Result,
    amount::{self, Amount, Totals},
    pycsv, report,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub fn component_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
pub fn fixture_root() -> PathBuf {
    component_root().join("../../contracts/xero-trial-balance-v1/fixtures")
}

pub fn declared_fixture(path: &Path) -> Result<PathBuf> {
    let raw = crate::pypath::decode_os(path.as_os_str());
    if cfg!(windows)
        && (raw.starts_with("\\\\.\\")
            || raw.starts_with("//./")
            || raw.starts_with("//?/")
            || raw.starts_with("\\\\?/")
            || raw.starts_with("//?\\"))
    {
        return Err(Failure {
            code: 2,
            stdout: String::new(),
            stderr: "error: fixture path must not use a Windows device namespace\n".into(),
        });
    }
    let candidate = crate::paths::resolved(path)?;
    let root = crate::paths::resolved(&fixture_root())?;
    if candidate == root || !candidate.starts_with(&root) {
        return Err(Failure {
            code: 2,
            stdout: String::new(),
            stderr: "error: fixture path must name a declared fabricated CSV fixture\n".into(),
        });
    }
    for name in ["passing.csv", "failing_movement.csv", "failing_ytd.csv"] {
        let owned = crate::paths::resolved(&fixture_root().join(name))?;
        if candidate == owned && owned.is_file() {
            return Ok(owned);
        }
    }
    Err(Failure {
        code: 2,
        stdout: String::new(),
        stderr: "error: fixture path must name a declared fabricated CSV fixture\n".into(),
    })
}

fn read_rows(path: &Path) -> Result<Vec<Vec<String>>> {
    let bytes = std::fs::read(path)
        .map_err(|_| Failure::message("error: fabricated CSV cannot be read"))?;
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Failure::message("error: fabricated CSV is not UTF-8"))?;
    let mut reader = pycsv::DictReader::new(text)
        .map_err(|_| Failure::message("error: malformed fabricated CSV"))?;
    if reader
        .fieldnames
        .as_ref()
        .is_none_or(|fields| fields.iter().map(String::as_str).ne(report::COLUMNS))
    {
        return Err(Failure::message(
            "error: fixture does not have the exact exporter header",
        ));
    }
    let rows = reader
        .by_ref()
        .map(|row| {
            row.map_err(|_| Failure::message("error: malformed fabricated CSV"))?
                .values
                .into_iter()
                .map(|v| {
                    v.ok_or_else(|| Failure::message("error: fabricated CSV has missing cells"))
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;
    if rows.is_empty() {
        return Err(Failure::message("error: fixture has no account rows"));
    }
    Ok(rows)
}
fn row_totals(rows: &[Vec<String>]) -> Result<Totals> {
    rows.iter()
        .try_fold(std::array::from_fn(|_| Amount::zero()), |totals, row| {
            let amounts = [
                Amount::parse(&row[6])?,
                Amount::parse(&row[7])?,
                Amount::parse(&row[8])?,
                Amount::parse(&row[9])?,
            ];
            amount::add_totals(&totals, &amounts)
        })
}
pub fn evaluate(path: &Path) -> Result<String> {
    let path = declared_fixture(path)?;
    let totals = row_totals(&read_rows(&path)?)?;
    amount::check_balanced(&totals, None)?;
    Ok(format!(
        "PASS: {} movement and YTD balance\n",
        crate::pypath::decode_os(path.file_name().unwrap())
    ))
}

pub struct Summary {
    pub tenant: String,
    pub date: String,
    pub rows: usize,
    pub columns: usize,
    pub movement: String,
    pub ytd: String,
}
pub fn summary(text: &str) -> Result<Summary> {
    let mut reader = pycsv::DictReader::new(text.trim_start_matches('\u{feff}'))
        .map_err(|_| Failure::message("the fabricated sample has malformed CSV"))?;
    let columns = reader.fieldnames.as_ref().map_or(0, Vec::len);
    if reader
        .fieldnames
        .as_ref()
        .is_none_or(|fields| fields.iter().map(String::as_str).ne(report::COLUMNS))
    {
        return Err(Failure::message(
            "the fabricated sample has unexpected columns",
        ));
    }
    let rows = reader
        .by_ref()
        .map(|row| {
            row.map_err(|_| Failure::message("the fabricated sample has malformed CSV"))?
                .values
                .into_iter()
                .map(|value| {
                    value.ok_or_else(|| Failure::message("the fabricated sample has missing cells"))
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;
    if rows.is_empty() {
        return Err(Failure::message(
            "the fabricated sample has no account rows",
        ));
    }
    let tenants: BTreeSet<_> = rows.iter().map(|row| row[1].clone()).collect();
    let dates: BTreeSet<_> = rows.iter().map(|row| row[0].clone()).collect();
    if tenants.len() != 1 {
        return Err(Failure::message(
            "the fabricated sample has multiple tenants",
        ));
    }
    if dates.len() != 1 {
        return Err(Failure::message(
            "the fabricated sample has multiple report dates",
        ));
    }
    let totals = row_totals(&rows)?;
    if !totals[0].equal(&totals[1]) {
        return Err(Failure::message(
            "the fabricated movement columns do not balance",
        ));
    }
    if !totals[2].equal(&totals[3]) {
        return Err(Failure::message(
            "the fabricated YTD columns do not balance",
        ));
    }
    Ok(Summary {
        tenant: tenants.into_iter().next().unwrap(),
        date: dates.into_iter().next().unwrap(),
        rows: rows.len(),
        columns,
        movement: format!("${}", totals[0].grouped_two()),
        ytd: format!("${}", totals[2].grouped_two()),
    })
}
fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
pub fn render_svg(summary: &Summary) -> String {
    let mut svg = include_str!("../tools/quick-proof.svg.template").to_owned();
    for (key, value) in [
        ("tenant", html(&summary.tenant)),
        ("date", html(&summary.date)),
        ("rows", summary.rows.to_string()),
        ("columns", summary.columns.to_string()),
        ("movement", html(&summary.movement)),
        ("ytd", html(&summary.ytd)),
    ] {
        svg = svg.replace(&format!("{{{key}}}"), &value);
    }
    svg
}
pub fn render_transcript(summary: &Summary) -> String {
    format!(
        "# Validated fabricated trial balance\n\nSource: [`samples/sample-output.csv`](../samples/sample-output.csv)\n\n- Tenant: {}\n- Report date: {}\n- Shape: {} account rows, {} columns\n- Movement: debit {} | credit {} | balanced\n- YTD: debit {} | credit {} | balanced\n\nReproduce the card and verify that both committed assets are current:\n\n```bash\ncargo run --locked --features tools --bin xero-tb-proof -- --check\n```\n\nThe source is fabricated. No Xero credentials or organisation data are used.\n",
        summary.tenant,
        summary.date,
        summary.rows,
        summary.columns,
        summary.movement,
        summary.movement,
        summary.ytd,
        summary.ytd
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn summary_proves_shape_balances_and_rejects_wrong_entities_dates_and_headers() {
        let sample = include_str!("../samples/sample-output.csv");
        let summary = summary(sample).unwrap();
        assert_eq!((summary.rows, summary.columns), (10, 10));
        assert_eq!(summary.movement, "$5,700.00");
        assert_eq!(summary.ytd, "$126,334.50");
        for (old, new, message) in [
            (
                "Catherby Fisheries Pty Ltd,Liabilities",
                "Other Synthetic Company,Liabilities",
                "multiple tenants",
            ),
            (
                "2026-06-30,Catherby Fisheries Pty Ltd,Liabilities",
                "2026-06-29,Catherby Fisheries Pty Ltd,Liabilities",
                "multiple report dates",
            ),
            (
                "AccountName,AccountCode",
                "Unexpected,AccountCode",
                "unexpected columns",
            ),
        ] {
            assert!(
                super::summary(&sample.replacen(old, new, 1))
                    .err()
                    .unwrap()
                    .stderr
                    .contains(message)
            );
        }
        assert!(render_svg(&summary).contains("10-row trial balance"));
        assert!(render_transcript(&summary).contains("$126,334.50"));
    }
    #[test]
    fn only_declared_fabricated_fixtures_are_read() {
        for (name, passing) in [
            ("passing.csv", true),
            ("failing_movement.csv", false),
            ("failing_ytd.csv", false),
        ] {
            assert_eq!(evaluate(&fixture_root().join(name)).is_ok(), passing);
        }
        assert!(declared_fixture(&component_root().join("samples/sample-output.csv")).is_err());
    }
}
