//! Validate one immutable byte snapshot, then retain the digest of those bytes.
use crate::{
    Result,
    decimal::Decimal,
    isodate::{self, Date},
    models::*,
    pycsv::DictReader,
    pyjson::{self, Json, LoadError},
    pypath, pytext,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub const TB_COLUMNS: [&str; 10] = [
    "ReportDate",
    "Tenant",
    "Section",
    "AccountID",
    "AccountName",
    "AccountCode",
    "Debit",
    "Credit",
    "YTDDebit",
    "YTDCredit",
];
pub const OPEN_COLUMNS: [&str; 7] = [
    "ItemID",
    "Severity",
    "Owner",
    "DueDate",
    "Status",
    "Description",
    "Resolution",
];
pub const ACTIVITY_COLUMNS: [&str; 2] = ["Label", "Amount"];
pub const GST_COLUMNS: [&str; 6] = [
    "Date",
    "AccountID",
    "AccountName",
    "Debit",
    "Credit",
    "Description",
];
pub const BANK_COLUMNS: [&str; 3] = ["AccountID", "StatementBalance", "GLBalance"];
pub const TIE_COLUMNS: [&str; 5] = [
    "StatementLine",
    "StatementAmount",
    "WorkpaperRef",
    "SourceFile",
    "Status",
];
pub const PRIOR_COLUMNS: [&str; 3] = ["FindingCode", "Slot", "Status"];
pub const ASSERTIONS: [&str; 5] = [
    "pack_complete",
    "tie_outs_done",
    "open_items_listed",
    "variances_explained",
    "self_reviewed",
];

#[derive(Clone, Debug)]
pub struct SourceSnapshot {
    pub path: PathBuf,
    pub content: Vec<u8>,
    pub sha256: String,
}

impl SourceSnapshot {
    pub fn new(path: &Path, content: Vec<u8>) -> Self {
        let sha256 = crate::sha256(&content);
        Self {
            path: path.to_owned(),
            content,
            sha256,
        }
    }
    pub fn capture(path: &Path, label: &str) -> Result<Self> {
        let content = fs::read(path).map_err(|e| {
            let path = pypath::text(path);
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("{label} does not exist: {path}.")
            } else {
                format!("{label} could not be read: {path} ({e}).")
            }
        })?;
        Ok(Self::new(path, content))
    }
    pub fn text(&self, label: &str, bom: bool) -> Result<&str> {
        let text = std::str::from_utf8(&self.content).map_err(|_| {
            format!(
                "{label} could not be read as UTF-8: {}.",
                pypath::text(&self.path)
            )
        })?;
        Ok(if bom {
            text.strip_prefix('\u{feff}').unwrap_or(text)
        } else {
            text
        })
    }
}

pub fn parse_iso_date(value: &str, field: &str, path: &Path, row: Option<usize>) -> Result<Date> {
    let bytes = value.as_bytes();
    let calendar = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    if calendar && let Some(date) = isodate::date_fromisoformat(value) {
        return Ok(date);
    }
    let where_ = row.map_or(String::new(), |n| format!("row {n} "));
    Err(format!(
        "{}:{where_}{field} must be an ISO date.",
        pypath::text(path)
    ))
}

fn accounting_number(raw: &str) -> bool {
    let raw = raw.strip_prefix(['-', '+']).unwrap_or(raw);
    let raw = raw.strip_prefix('$').unwrap_or(raw);
    let mut parts = raw.split('.');
    let integer = parts.next().unwrap_or("");
    let digits =
        |s: &str| !s.is_empty() && s.chars().all(|c| pytext::decimal_value(c as u32).is_some());
    if let Some(frac) = parts.next()
        && !digits(frac)
    {
        return false;
    }
    if parts.next().is_some() {
        return false;
    }
    let groups: Vec<_> = integer.split(',').collect();
    if groups.len() == 1 {
        return digits(integer);
    }
    (1..=3).contains(&groups[0].chars().count())
        && digits(groups[0])
        && groups[1..]
            .iter()
            .all(|s| s.chars().count() == 3 && digits(s))
}

pub fn parse_money(value: Option<&str>, field: &str, row: usize, path: &Path) -> Result<Decimal> {
    let raw = pytext::strip(value.unwrap_or(""));
    let invalid = || {
        format!(
            "{}: row {row} has invalid {field}: {}.",
            pypath::text(path),
            pytext::repr(raw)
        )
    };
    if !accounting_number(raw) {
        return Err(invalid());
    }
    Decimal::parse(&raw.replace(['$', ','], "")).map_err(|_| invalid())
}

struct CsvRow {
    number: usize,
    values: BTreeMap<String, Option<String>>,
    path: PathBuf,
}

impl CsvRow {
    fn text(&self, field: &str, empty: bool) -> Result<String> {
        let text = pytext::strip(self.values[field].as_deref().unwrap_or(""));
        let path = pypath::text(&self.path);
        if text.is_empty() && !empty {
            return Err(format!("{path}: row {} has an empty {field}.", self.number));
        }
        if pytext::control_or_format(text, false) {
            return Err(format!(
                "{path}: row {} {field} contains a control or formatting character.",
                self.number
            ));
        }
        Ok(text.into())
    }
    fn money(&self, field: &str) -> Result<Decimal> {
        parse_money(
            self.values[field].as_deref(),
            field,
            self.number,
            &self.path,
        )
    }
    fn date(&self, field: &str) -> Result<Date> {
        parse_iso_date(
            &self.text(field, false)?,
            field,
            &self.path,
            Some(self.number),
        )
    }
}

fn csv_rows(snapshot: &SourceSnapshot, required: &[&str], label: &str) -> Result<Vec<CsvRow>> {
    let path = pypath::text(&snapshot.path);
    let csv_error =
        || format!("{path}: {label} is not valid CSV: field larger than field limit (131072).");
    let mut reader = DictReader::new(snapshot.text(label, true)?).map_err(|_| csv_error())?;
    let headers = reader
        .fieldnames
        .as_ref()
        .ok_or_else(|| format!("{path}: CSV has no header row."))?;
    let duplicates: BTreeSet<_> = headers
        .iter()
        .filter(|name| headers.iter().filter(|other| other == name).count() > 1)
        .cloned()
        .collect();
    if !duplicates.is_empty() {
        return Err(format!(
            "{path}: duplicate column heading(s): {}.",
            duplicates.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    let expected: BTreeSet<_> = required.iter().copied().collect();
    let actual: BTreeSet<_> = headers.iter().map(String::as_str).collect();
    let missing: Vec<_> = expected.difference(&actual).copied().collect();
    let unexpected: Vec<_> = actual.difference(&expected).copied().collect();
    let mut detail = Vec::new();
    if !missing.is_empty() {
        detail.push(format!("missing {}", missing.join(", ")));
    }
    if !unexpected.is_empty() {
        detail.push(format!("unexpected {}", unexpected.join(", ")));
    }
    if !detail.is_empty() {
        return Err(format!(
            "{path}: canonical schema mismatch ({}).",
            detail.join("; ")
        ));
    }
    let headers = headers.clone();
    let mut rows = Vec::new();
    while let Some(row) = reader.next() {
        let row = row.map_err(|_| csv_error())?;
        let number = reader.line_num();
        if row.has_extra {
            return Err(format!(
                "{path}: row {number} has more fields than its header."
            ));
        }
        rows.push(CsvRow {
            number,
            values: headers.iter().cloned().zip(row.values).collect(),
            path: snapshot.path.clone(),
        });
    }
    Ok(rows)
}

pub fn load_canonical_tb(snapshot: &SourceSnapshot) -> Result<Vec<TrialBalanceRow>> {
    let path = pypath::text(&snapshot.path);
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for r in csv_rows(snapshot, &TB_COLUMNS, "Trial-balance file")? {
        let row = TrialBalanceRow {
            report_date: r.date("ReportDate")?,
            tenant: r.text("Tenant", false)?,
            section: r.text("Section", false)?,
            account_id: r.text("AccountID", false)?,
            account_name: r.text("AccountName", false)?,
            account_code: r.text("AccountCode", true)?,
            debit: r.money("Debit")?,
            credit: r.money("Credit")?,
            ytd_debit: r.money("YTDDebit")?,
            ytd_credit: r.money("YTDCredit")?,
        };
        if !seen.insert((row.tenant.clone(), row.account_id.clone())) {
            return Err(format!(
                "{path}: duplicate control key Tenant={}, AccountID={}.",
                pytext::repr(&row.tenant),
                pytext::repr(&row.account_id)
            ));
        }
        rows.push(row);
    }
    if rows.is_empty() {
        return Err(format!("{path}: no trial-balance rows were supplied."));
    }
    if rows.iter().any(|r| r.tenant != rows[0].tenant) {
        return Err(format!(
            "{path}: a canonical trial balance must contain exactly one tenant."
        ));
    }
    if rows.iter().any(|r| r.report_date != rows[0].report_date) {
        return Err(format!(
            "{path}: a canonical trial balance must contain exactly one ReportDate."
        ));
    }
    Ok(rows)
}

pub fn load_open_items(snapshot: &SourceSnapshot) -> Result<Vec<OpenItem>> {
    let path = pypath::text(&snapshot.path);
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for r in csv_rows(snapshot, &OPEN_COLUMNS, "Open-items file")? {
        let item_id = r.text("ItemID", false)?;
        if !seen.insert(item_id.clone()) {
            return Err(format!(
                "{path}: duplicate ItemID {}.",
                pytext::repr(&item_id)
            ));
        }
        let severity = r.text("Severity", false)?;
        if !["BLOCKING", "EXPLAIN", "TRIVIAL"].contains(&severity.as_str()) {
            return Err(format!(
                "{path}: row {} Severity must be BLOCKING, EXPLAIN, or TRIVIAL.",
                r.number
            ));
        }
        let status = r.text("Status", false)?;
        if !["OPEN", "CLEARED"].contains(&status.as_str()) {
            return Err(format!(
                "{path}: row {} Status must be OPEN or CLEARED.",
                r.number
            ));
        }
        let due_date = r.date("DueDate")?;
        rows.push(OpenItem {
            item_id,
            severity,
            due_date,
            status,
            owner: r.text("Owner", false)?,
            description: r.text("Description", false)?,
            resolution: r.text("Resolution", true)?,
        });
    }
    Ok(rows)
}

pub fn load_activity_statement(snapshot: &SourceSnapshot) -> Result<Vec<ActivityLabel>> {
    let path = pypath::text(&snapshot.path);
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for r in csv_rows(snapshot, &ACTIVITY_COLUMNS, "Activity-statement file")? {
        let label = r.text("Label", false)?;
        if !seen.insert(label.clone()) {
            return Err(format!("{path}: duplicate Label {}.", pytext::repr(&label)));
        }
        rows.push(ActivityLabel {
            label,
            amount: r.money("Amount")?,
        });
    }
    if rows.is_empty() {
        return Err(format!("{path}: no activity-statement rows were supplied."));
    }
    Ok(rows)
}

pub fn load_gst_control(snapshot: &SourceSnapshot) -> Result<Vec<GstControlRow>> {
    let mut rows = Vec::new();
    for r in csv_rows(snapshot, &GST_COLUMNS, "GST-control file")? {
        rows.push(GstControlRow {
            posted_on: r.date("Date")?,
            account_id: r.text("AccountID", false)?,
            account_name: r.text("AccountName", false)?,
            debit: r.money("Debit")?,
            credit: r.money("Credit")?,
            description: r.text("Description", true)?,
        });
    }
    if rows.is_empty() {
        return Err(format!(
            "{}: no GST-control rows were supplied.",
            pypath::text(&snapshot.path)
        ));
    }
    Ok(rows)
}

pub fn load_bank_rec(snapshot: &SourceSnapshot) -> Result<Vec<BankRecRow>> {
    let path = pypath::text(&snapshot.path);
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for r in csv_rows(snapshot, &BANK_COLUMNS, "Bank-rec file")? {
        let account_id = r.text("AccountID", false)?;
        if !seen.insert(account_id.clone()) {
            return Err(format!(
                "{path}: duplicate AccountID {}.",
                pytext::repr(&account_id)
            ));
        }
        rows.push(BankRecRow {
            account_id,
            statement_balance: r.money("StatementBalance")?,
            gl_balance: r.money("GLBalance")?,
        });
    }
    if rows.is_empty() {
        return Err(format!("{path}: no bank-rec rows were supplied."));
    }
    Ok(rows)
}

pub fn load_tie_out_matrix(snapshot: &SourceSnapshot) -> Result<Vec<TieOutRow>> {
    let path = pypath::text(&snapshot.path);
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for r in csv_rows(snapshot, &TIE_COLUMNS, "Tie-out file")? {
        let statement_line = r.text("StatementLine", false)?;
        if !seen.insert(statement_line.clone()) {
            return Err(format!(
                "{path}: duplicate StatementLine {}.",
                pytext::repr(&statement_line)
            ));
        }
        let status = r.text("Status", false)?;
        if !["TIED", "EXCEPTION", "UNSUPPORTED"].contains(&status.as_str()) {
            return Err(format!(
                "{path}: row {} Status must be TIED, EXCEPTION, or UNSUPPORTED.",
                r.number
            ));
        }
        r.money("StatementAmount")?;
        rows.push(TieOutRow {
            statement_line,
            workpaper_ref: r.text("WorkpaperRef", status != "TIED")?,
            source_file: r.text("SourceFile", status != "TIED")?,
            status,
        });
    }
    if rows.is_empty() {
        return Err(format!("{path}: no tie-out rows were supplied."));
    }
    Ok(rows)
}

pub fn load_prior_findings(snapshot: &SourceSnapshot) -> Result<Vec<PriorFinding>> {
    let path = pypath::text(&snapshot.path);
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for r in csv_rows(snapshot, &PRIOR_COLUMNS, "Prior-findings file")? {
        let code = r.text("FindingCode", false)?;
        let slot = r.text("Slot", false)?;
        let status = r.text("Status", false)?;
        if !["OPEN", "CLEARED"].contains(&status.as_str()) {
            return Err(format!(
                "{path}: row {} Status must be OPEN or CLEARED.",
                r.number
            ));
        }
        if !seen.insert((code.clone(), slot.clone())) {
            return Err(format!(
                "{path}: duplicate prior finding {} / {}.",
                pytext::repr(&code),
                pytext::repr(&slot)
            ));
        }
        rows.push(PriorFinding {
            finding_code: code,
            slot,
            status,
        });
    }
    Ok(rows)
}

fn json(snapshot: &SourceSnapshot, note: bool) -> Result<Json> {
    let path = pypath::text(&snapshot.path);
    let label = if note {
        "Review-note file"
    } else {
        "Self-review file"
    };
    let name = if note { "review note" } else { "self-review" };
    pyjson::loads(snapshot.text(label, true)?).map_err(|e| match e {
        LoadError::Invalid => format!("{path}: {name} is not valid JSON."),
        LoadError::DuplicateKey(key) => format!(
            "{path}: {name} could not be read (JSON member {} appears more than once.).",
            pytext::repr_code_points(&key.0)
        ),
        LoadError::Crash(msg) if msg.contains("recursion") || msg.contains("Recursion") => {
            format!("{path}: {name} is nested too deeply to read.")
        }
        LoadError::Crash(msg) => format!("{path}: {name} could not be read ({msg})."),
    })
}

pub fn json_text(value: &Json) -> Option<String> {
    value.as_pystr()?.to_utf8()
}

fn checked_initials(value: &Json, name: &str, path: &str) -> Result<String> {
    let text = value
        .as_pystr()
        .map(|s| s.to_utf8().unwrap_or_else(|| s.os_text()))
        .unwrap_or_default();
    let trimmed_length = value.as_pystr().map_or(0, |s| {
        let start = s.0.iter().take_while(|&&cp| pytext::is_space(cp)).count();
        let end = s.0[start..]
            .iter()
            .rev()
            .take_while(|&&cp| pytext::is_space(cp))
            .count();
        s.0.len() - start - end
    });
    if value.as_pystr().is_none() || pytext::strip(&text).is_empty() || trimmed_length > 12 {
        return Err(format!(
            "{path}: {name} must be a non-empty string of at most 12 characters."
        ));
    }
    if value
        .as_pystr()
        .unwrap()
        .0
        .iter()
        .any(|cp| (0xd800..=0xdfff).contains(cp))
        || pytext::control_or_format(&text, false)
    {
        return Err(format!(
            "{path}: {name} contains a control or formatting character."
        ));
    }
    Ok(pytext::strip(&text).into())
}

pub fn load_self_review(snapshot: &SourceSnapshot, profile: &str) -> Result<SelfReview> {
    let path = pypath::text(&snapshot.path);
    let value = json(snapshot, false)?;
    if !value.has_exact_keys(&[
        "preparer_initials",
        "prepared_on",
        "engagement_type",
        "period_end",
        "assertions",
    ]) {
        return Err(format!(
            "{path}: self-review must contain exactly preparer_initials, prepared_on, engagement_type, period_end, and assertions."
        ));
    }
    let initials = checked_initials(value.at("preparer_initials"), "preparer_initials", &path)?;
    if !value.at("engagement_type").is_str(profile) {
        return Err(format!(
            "{path}: engagement_type must match the selected profile {}.",
            pytext::repr(profile)
        ));
    }
    let prepared = value
        .at("prepared_on")
        .as_pystr()
        .map(|s| s.os_text())
        .ok_or_else(|| format!("{path}: prepared_on must be an ISO date string."))?;
    let period = value
        .at("period_end")
        .as_pystr()
        .map(|s| s.os_text())
        .ok_or_else(|| format!("{path}: period_end must be an ISO date string."))?;
    let prepared_on = parse_iso_date(&prepared, "prepared_on", &snapshot.path, None)?;
    let period_end = parse_iso_date(&period, "period_end", &snapshot.path, None)?;
    if prepared_on < period_end {
        return Err(format!(
            "{path}: prepared_on {} is earlier than period_end {}.",
            prepared_on.iso(),
            period_end.iso()
        ));
    }
    let assertions = value.at("assertions");
    if !assertions.has_exact_keys(&ASSERTIONS) {
        return Err(format!(
            "{path}: assertions must contain exactly {}.",
            ASSERTIONS.join(", ")
        ));
    }
    let assertions = ASSERTIONS
        .into_iter()
        .map(|name| match assertions.at(name) {
            Json::Bool(value) => Ok((name.into(), *value)),
            _ => Err(format!("{path}: assertions.{name} must be a JSON boolean.")),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SelfReview {
        preparer_initials: initials,
        prepared_on,
        engagement_type: profile.into(),
        period_end,
        assertions,
    })
}

pub fn load_reviewer_acknowledgement(
    path: Option<&Path>,
) -> Result<Option<ReviewerAcknowledgement>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let snapshot = SourceSnapshot::capture(path, "Review-note file")?;
    let value = json(&snapshot, true)?;
    let path_text = pypath::text(path);
    if !value.has_exact_keys(&["reviewer_initials", "reviewed_on", "comment"]) {
        return Err(format!(
            "{path_text}: review note must contain exactly reviewer_initials, reviewed_on, and comment."
        ));
    }
    let initials = checked_initials(
        value.at("reviewer_initials"),
        "reviewer_initials",
        &path_text,
    )?;
    let comment_value = value
        .at("comment")
        .as_pystr()
        .ok_or_else(|| format!("{path_text}: comment must be a non-empty string."))?;
    let comment = comment_value
        .to_utf8()
        .unwrap_or_else(|| comment_value.os_text());
    if pytext::strip(&comment).is_empty() {
        return Err(format!("{path_text}: comment must be a non-empty string."));
    }
    if comment_value
        .0
        .iter()
        .any(|cp| (0xd800..=0xdfff).contains(cp))
        || pytext::control_or_format(&comment, true)
    {
        return Err(format!(
            "{path_text}: comment contains a control or formatting character."
        ));
    }
    let reviewed = value
        .at("reviewed_on")
        .as_pystr()
        .map(|s| s.os_text())
        .ok_or_else(|| format!("{path_text}: reviewed_on must be an ISO date string."))?;
    let reviewed_on = parse_iso_date(&reviewed, "reviewed_on", path, None)?;
    Ok(Some(ReviewerAcknowledgement {
        reviewer_initials: initials,
        reviewed_on,
        comment: pytext::strip(&comment).into(),
    }))
}
