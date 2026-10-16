//! Validate one immutable byte snapshot, then retain the digest of those bytes.
use crate::{
    Result,
    decimal::Decimal,
    isodate::{self, Date},
    models::*,
    pycsv::DictReader,
    pyjson::{self, LoadError},
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
pub const MAPPING_COLUMNS: [&str; 2] = ["AccountID", "ReviewGroup"];
pub const MAPPING_POLICY_COLUMNS: [&str; 2] = ["Section", "ReviewGroup"];
pub const BALANCE_POLICY_COLUMNS: [&str; 3] = ["AccountID", "ExpectedBalance", "ExpectMovement"];
pub const SUBLEDGER_COLUMNS: [&str; 3] = ["Tenant", "AccountID", "SubledgerBalance"];
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
    )
    .replace(": ", ":"))
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

pub(crate) struct CsvRow {
    pub number: usize,
    pub values: BTreeMap<String, Option<String>>,
    pub path: PathBuf,
}

impl CsvRow {
    pub fn text(&self, field: &str, empty: bool) -> Result<String> {
        input_text(
            self.values[field].as_deref(),
            field,
            self.number,
            &self.path,
            empty,
        )
    }
    pub fn money(&self, field: &str) -> Result<Decimal> {
        parse_money(
            self.values[field].as_deref(),
            field,
            self.number,
            &self.path,
        )
    }
    pub fn date(&self, field: &str) -> Result<Date> {
        parse_iso_date(
            &self.text(field, false)?,
            field,
            &self.path,
            Some(self.number),
        )
    }
}

pub(crate) fn input_text(
    value: Option<&str>,
    field: &str,
    row: usize,
    path: &Path,
    empty: bool,
) -> Result<String> {
    let text = pytext::strip(value.unwrap_or(""));
    let path = pypath::text(path);
    if text.is_empty() && !empty {
        return Err(format!("{path}: row {row} has an empty {field}."));
    }
    if pytext::control_or_format(text, false) {
        return Err(format!(
            "{path}: row {row} {field} contains a control or formatting character."
        ));
    }
    Ok(text.into())
}

pub(crate) fn json_input_text(
    value: &pyjson::Json,
    field: &str,
    row: usize,
    path: &Path,
    empty: bool,
) -> Result<String> {
    let points = value.as_pystr().expect("validated text member");
    if points.0.iter().any(|&cp| (0xd800..=0xdfff).contains(&cp)) {
        return Err(format!(
            "{}: row {row} {field} contains a control or formatting character.",
            pypath::text(path)
        ));
    }
    input_text(points.to_utf8().as_deref(), field, row, path, empty)
}

pub(crate) struct CsvRows<'a> {
    reader: DictReader<'a>,
    headers: Vec<String>,
    path: PathBuf,
    label: String,
}

impl Iterator for CsvRows<'_> {
    type Item = Result<CsvRow>;

    fn next(&mut self) -> Option<Self::Item> {
        let row = self.reader.next()?;
        let number = self.reader.line_num();
        Some(
            row.map_err(|error| {
                format!(
                    "{}: {} is not valid CSV: {error}.",
                    pypath::text(&self.path),
                    self.label
                )
            })
            .and_then(|row| {
                if row.has_extra {
                    return Err(format!(
                        "{}: row {number} has more fields than its header.",
                        pypath::text(&self.path)
                    ));
                }
                Ok(CsvRow {
                    number,
                    values: self.headers.iter().cloned().zip(row.values).collect(),
                    path: self.path.clone(),
                })
            }),
        )
    }
}

pub(crate) fn csv_rows<'a>(
    snapshot: &'a SourceSnapshot,
    required: &[&str],
    label: &str,
) -> Result<CsvRows<'a>> {
    let path = pypath::text(&snapshot.path);
    let reader = DictReader::new(snapshot.text(label, true)?)
        .map_err(|error| format!("{path}: {label} is not valid CSV: {error}."))?;
    let headers = reader
        .fieldnames
        .as_ref()
        .ok_or_else(|| format!("{path}: CSV has no header row."))?;
    let mut seen = BTreeSet::new();
    let duplicates: BTreeSet<_> = headers
        .iter()
        .filter(|name| !seen.insert(name.as_str()))
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
    Ok(CsvRows {
        headers: headers.clone(),
        reader,
        path: snapshot.path.clone(),
        label: label.into(),
    })
}

pub fn load_canonical_tb(snapshot: &SourceSnapshot) -> Result<Vec<TrialBalanceRow>> {
    let path = pypath::text(&snapshot.path);
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for r in csv_rows(snapshot, &TB_COLUMNS, "Trial-balance file")? {
        let r = r?;
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

pub fn load_mapping(snapshot: Option<&SourceSnapshot>) -> Result<BTreeMap<String, String>> {
    let Some(snapshot) = snapshot else {
        return Ok(BTreeMap::new());
    };
    let mut mapping = BTreeMap::new();
    for row in csv_rows(snapshot, &MAPPING_COLUMNS, "Mapping file")? {
        let row = row?;
        let id = row.text("AccountID", false)?;
        let group = row.text("ReviewGroup", false)?;
        if mapping.insert(id.clone(), group).is_some() {
            return Err(format!(
                "{}: duplicate AccountID {}.",
                pypath::text(&snapshot.path),
                pytext::repr(&id)
            ));
        }
    }
    Ok(mapping)
}

pub fn load_mapping_policy(
    snapshot: &SourceSnapshot,
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut policy: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in csv_rows(snapshot, &MAPPING_POLICY_COLUMNS, "Mapping policy")? {
        let row = row?;
        let section = row.text("Section", false)?;
        let group = row.text("ReviewGroup", false)?;
        if !policy
            .entry(group.clone())
            .or_default()
            .insert(section.clone())
        {
            return Err(format!(
                "{}: row {} repeats Section={}, ReviewGroup={}.",
                pypath::text(&snapshot.path),
                row.number,
                pytext::repr(&section),
                pytext::repr(&group)
            ));
        }
    }
    if policy.is_empty() {
        return Err(format!(
            "{}: mapping policy has no permitted pairs.",
            pypath::text(&snapshot.path)
        ));
    }
    Ok(policy)
}

pub fn load_balance_policy(snapshot: &SourceSnapshot) -> Result<BTreeMap<String, (String, bool)>> {
    let path = pypath::text(&snapshot.path);
    let text = snapshot.text("Balance policy", true)?;
    let rows = crate::pycsv::Reader::strict(text)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| format!("{path}: balance policy is not valid CSV: {error}."))?;
    if rows.iter().flatten().any(|value| value.contains('"')) {
        return Err(format!(
            "{path}: balance policy values must not contain quote characters."
        ));
    }
    let mut policy = BTreeMap::new();
    for row in csv_rows(snapshot, &BALANCE_POLICY_COLUMNS, "Balance policy")? {
        let row = row?;
        let id = row.text("AccountID", false)?;
        let expected = row.text("ExpectedBalance", false)?;
        let movement = row.text("ExpectMovement", false)?;
        if !["debit", "credit", "nil", "any"].contains(&expected.as_str()) {
            return Err(format!(
                "{path}: row {} ExpectedBalance {} is not one of debit, credit, nil, any.",
                row.number,
                pytext::repr(&expected)
            ));
        }
        if !["yes", "no"].contains(&movement.as_str()) {
            return Err(format!(
                "{path}: row {} ExpectMovement {} is not yes or no.",
                row.number,
                pytext::repr(&movement)
            ));
        }
        if policy
            .insert(id.clone(), (expected, movement == "yes"))
            .is_some()
        {
            return Err(format!(
                "{path}: duplicate AccountID {}.",
                pytext::repr(&id)
            ));
        }
    }
    if policy.is_empty() {
        return Err(format!("{path}: balance policy has no accounts."));
    }
    Ok(policy)
}

pub fn load_subledger(
    snapshot: Option<&SourceSnapshot>,
) -> Result<BTreeMap<(String, String), Decimal>> {
    let Some(snapshot) = snapshot else {
        return Ok(BTreeMap::new());
    };
    let mut result = BTreeMap::new();
    for row in csv_rows(snapshot, &SUBLEDGER_COLUMNS, "Subledger file")? {
        let row = row?;
        let tenant = row.text("Tenant", false)?;
        let id = row.text("AccountID", false)?;
        if result.contains_key(&(tenant.clone(), id.clone())) {
            return Err(format!(
                "{}: duplicate subledger key Tenant={}, AccountID={}.",
                pypath::text(&snapshot.path),
                pytext::repr(&tenant),
                pytext::repr(&id)
            ));
        }
        result.insert((tenant, id), row.money("SubledgerBalance")?);
    }
    Ok(result)
}

fn strip_points(points: &[u32]) -> &[u32] {
    let start = points
        .iter()
        .position(|&cp| !pytext::is_space(cp))
        .unwrap_or(points.len());
    let end = points
        .iter()
        .rposition(|&cp| !pytext::is_space(cp))
        .map_or(start, |n| n + 1);
    &points[start..end]
}

pub fn load_reviewer_acknowledgement(
    snapshot: Option<&SourceSnapshot>,
) -> Result<Option<ReviewerAcknowledgement>> {
    let Some(snapshot) = snapshot else {
        return Ok(None);
    };
    let path = pypath::text(&snapshot.path);
    let payload =
        pyjson::loads(snapshot.text("Review-note file", true)?).map_err(|error| match error {
            LoadError::Invalid => format!("{path}: review note is not valid JSON."),
            LoadError::DuplicateKey(key) => format!(
                "{path}: review note could not be read (JSON member {} appears more than once.).",
                pytext::repr_code_points(&key.0)
            ),
            LoadError::Crash(reason)
                if reason.contains("recursion") || reason.contains("nested") =>
            {
                format!("{path}: review note is nested too deeply to read.")
            }
            LoadError::Crash(reason) => {
                format!("{path}: review note could not be read ({reason}).")
            }
        })?;
    if !payload.has_exact_keys(&["reviewer_initials", "reviewed_on", "comment"]) {
        return Err(format!(
            "{path}: review note must contain exactly reviewer_initials, reviewed_on, and comment."
        ));
    }
    let initials_error = || {
        format!("{path}: reviewer_initials must be a non-empty string of at most 12 characters.")
    };
    let initials = payload
        .at("reviewer_initials")
        .as_pystr()
        .ok_or_else(initials_error)?;
    let stripped_initials = strip_points(&initials.0);
    if stripped_initials.is_empty() || stripped_initials.len() > 12 {
        return Err(initials_error());
    }
    let comment_error = || format!("{path}: comment must be a non-empty string.");
    let comment = payload.at("comment").as_pystr().ok_or_else(comment_error)?;
    let stripped_comment = strip_points(&comment.0);
    if stripped_comment.is_empty() {
        return Err(comment_error());
    }
    if initials
        .0
        .iter()
        .any(|&cp| pytext::control_or_format_point(cp, false))
    {
        return Err(format!(
            "{path}: reviewer_initials contains a control or formatting character."
        ));
    }
    if comment
        .0
        .iter()
        .any(|&cp| pytext::control_or_format_point(cp, true))
    {
        return Err(format!(
            "{path}: comment contains a control or formatting character."
        ));
    }
    let reviewed_on = payload
        .at("reviewed_on")
        .as_pystr()
        .ok_or_else(|| format!("{path}: reviewed_on must be an ISO date string."))?
        .to_utf8()
        .unwrap_or_default();
    let reviewed_on = parse_iso_date(&reviewed_on, "reviewed_on", &snapshot.path, None)?;
    let literal = |points: &[u32]| {
        points
            .iter()
            .map(|&cp| char::from_u32(cp).expect("validated JSON text"))
            .collect::<String>()
    };
    Ok(Some(ReviewerAcknowledgement {
        reviewer_initials: literal(stripped_initials),
        reviewed_on,
        comment: literal(stripped_comment),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(text: &str) -> SourceSnapshot {
        SourceSnapshot::new(Path::new("input.csv"), text.as_bytes().to_vec())
    }

    #[test]
    fn snapshot_parsing_and_provenance_survive_source_replacement() {
        let temporary = crate::test_support::TempDir::new();
        let path = temporary.join("source.csv");
        let bytes =
            fs::read(crate::test_support::examples().join("current_trial_balance.csv")).unwrap();
        fs::write(&path, &bytes).unwrap();
        let captured = SourceSnapshot::capture(&path, "Trial-balance file").unwrap();
        fs::write(&path, b"broken input").unwrap();
        assert_eq!(captured.content, bytes);
        assert_eq!(captured.sha256, crate::sha256(&bytes));
        assert!(!load_canonical_tb(&captured).unwrap().is_empty());
        assert!(
            load_canonical_tb(&SourceSnapshot::capture(&path, "Trial-balance file").unwrap())
                .is_err()
        );
    }

    #[test]
    fn validation_precedence_and_strict_balance_policy_match_the_source() {
        let header = TB_COLUMNS.join(",");
        let input = format!(
            "{header}\n2026-03-31,Demo,Assets,A,B,001,invalid,0,0,0\n{}",
            "x".repeat(131_073)
        );
        let error = load_canonical_tb(&snapshot(&input)).unwrap_err();
        assert_eq!(error, "input.csv: row 2 has invalid Debit: 'invalid'.");
        let valid = snapshot("AccountID,ExpectedBalance,ExpectMovement\n\"ACCT,1\",debit,yes\n");
        assert_eq!(
            load_balance_policy(&valid).unwrap()["ACCT,1"],
            ("debit".into(), true)
        );
        for input in [
            "AccountID,ExpectedBalance,ExpectMovement\n\"open",
            "AccountID,ExpectedBalance,ExpectMovement\n\"ACCT\"stray,debit,yes\n",
        ] {
            assert!(
                load_balance_policy(&snapshot(input))
                    .unwrap_err()
                    .contains("balance policy is not valid CSV")
            );
        }
    }

    #[test]
    fn acknowledgements_preserve_literal_unicode_and_refuse_surrogates_and_duplicate_members() {
        let valid = snapshot(
            "{\"reviewer_initials\":\"RD\",\"reviewed_on\":\"2026-03-31\",\"comment\":\"Literal \u{10f7ff}\u{10f800}\\nreview\"}",
        );
        assert_eq!(
            load_reviewer_acknowledgement(Some(&valid))
                .unwrap()
                .unwrap()
                .comment,
            "Literal \u{10f7ff}\u{10f800}\nreview"
        );
        let surrogate =
            snapshot(r#"{"reviewer_initials":"RD","reviewed_on":"2026-03-31","comment":"\ud800"}"#);
        assert!(
            load_reviewer_acknowledgement(Some(&surrogate))
                .unwrap_err()
                .contains("comment contains a control or formatting character")
        );
        let duplicate = snapshot(
            r#"{"reviewer_initials":"RD","reviewed_on":"2026-03-31","comment":"first","comment":"second"}"#,
        );
        assert_eq!(
            load_reviewer_acknowledgement(Some(&duplicate)).unwrap_err(),
            "input.csv: review note could not be read (JSON member 'comment' appears more than once.)."
        );
    }
}
