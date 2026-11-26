// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Local grant allocation and funding workpapers from explicitly reviewed
//! inputs: a line-by-line port of `grant_workpaper.py`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::ops::{Add, Sub};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::amount::Amount;
use crate::isodate::{self, Date};
use crate::oserror::{self, display, file_error, os_error};
use crate::pycsv::{self, DictReader};
use crate::pyjson::{self, Json, LoadError, PyStr};
use crate::pytext;

/// How a run stops early: an exception the script caught and printed (exit 1).
/// One it did not catch is a panic, which main's hook reports as an internal error.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    Error(String),
}

pub type Result<T> = std::result::Result<T, Failure>;

fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(Failure::Error(message.into()))
}

/// A CSV row as the script's dicts held it: (field, value) in header order.
pub type Record = Vec<(String, String)>;

fn field<'a>(record: &'a Record, name: &str) -> &'a str {
    &record
        .iter()
        .find(|(k, _)| k == name)
        .expect("rows() checked the headings")
        .1
}

fn cps(text: &str) -> Vec<u32> {
    text.chars().map(|c| c as u32).collect()
}

/// `money(value)` for a CSV string.
pub fn money(text: &str) -> Result<Amount> {
    money_cps(text.chars().map(u32::from)).ok_or_else(|| {
        Failure::Error(format!(
            "Expected a non-negative decimal string with at most 2 places: {}",
            pytext::repr(text)
        ))
    })
}

/// `money(value)` for a parsed JSON value.
pub fn money_json(value: &Json) -> Result<Amount> {
    let parsed = match value {
        Json::Str(text) => money_cps(text.0.iter().copied()),
        _ => None,
    };
    parsed.ok_or_else(|| {
        Failure::Error(format!(
            "Expected a non-negative decimal string with at most 2 places: {}",
            pyjson::repr(value)
        ))
    })
}

/// `re.fullmatch(r"\d{1,16}(?:\.\d{1,2})?", value)` then `Decimal(value)`; `\d` is any Unicode digit.
fn money_cps(text: impl Iterator<Item = u32>) -> Option<Amount> {
    let mut digits = [0; 18];
    let mut digit_count = 0;
    let mut point = None;
    for code_point in text {
        if code_point == u32::from('.') {
            if point.replace(digit_count).is_some() {
                return None;
            }
        } else {
            *digits.get_mut(digit_count)? = pytext::decimal_value(code_point)?;
            digit_count += 1;
        }
    }
    let whole = point.unwrap_or(digit_count);
    if !(1..=16).contains(&whole) || point.is_some() && !(1..=2).contains(&(digit_count - whole)) {
        return None;
    }
    Some(Amount::from_digits(&digits[..whole], &digits[whole..digit_count]))
}

/// `iso(value)`: a date written exactly as `YYYY-MM-DD`.
fn iso_cps(text: &[u32]) -> Result<Date> {
    let invalid = || Failure::Error(format!("Invalid isoformat string: {}", pytext::repr_code_points(text)));
    let utf8 = PyStr(text.to_vec()).to_utf8().ok_or_else(invalid)?;
    let date = isodate::date_fromisoformat(&utf8).map_err(|error| match error {
        Some(range) => Failure::Error(range),
        None => invalid(),
    })?;
    if date.iso() != utf8 {
        return fail("Use YYYY-MM-DD dates");
    }
    Ok(date)
}

pub fn iso(text: &str) -> Result<Date> {
    iso_cps(&cps(text))
}

pub fn iso_json(value: &Json) -> Result<Date> {
    let kind = match value {
        Json::Str(text) => return iso_cps(&text.0),
        Json::Null => "None",
        Json::Bool(_) => "bool",
        Json::Int(_) => "int",
        Json::Float(_) => "float",
        Json::Array(_) => "list",
        Json::Object(_) => "dict",
    };
    // Python 3.15 names the type of an invalid argument.
    fail(format!("fromisoformat() argument must be str, not {kind}"))
}

/// `text_field(value, label, required=...)` on code points.
fn text_cps(value: &[u32], label: &str, required: bool) -> Result<()> {
    let categories: Vec<&str> = value.iter().map(|&c| pytext::category(c)).collect();
    let visible = |c: &&str| matches!(c.chars().next(), Some('L' | 'N' | 'P' | 'S'));
    if categories
        .iter()
        .any(|c| c.starts_with('C') || *c == "Zl" || *c == "Zp")
        || (required && !categories.iter().any(visible))
    {
        return fail(format!(
            "{label} must be visible text without Unicode control or format characters"
        ));
    }
    let mut has_base = false;
    for category in &categories {
        if category.starts_with('M') {
            if !has_base {
                return fail(format!("{label} text has a combining mark without a base character"));
            }
        } else {
            has_base = visible(category);
        }
    }
    Ok(())
}

pub fn text_field(value: &str, label: &str, required: bool) -> Result<()> {
    text_cps(&cps(value), label, required)
}

fn text_json<'a>(value: &'a Json, label: &str) -> Result<&'a PyStr> {
    match value {
        Json::Str(text) => {
            text_cps(&text.0, label, true)?;
            Ok(text)
        }
        _ => fail(format!("{label} must be text")),
    }
}

/// `rows(content, fields, key)`.
pub fn rows(content: &[u8], fields: &str, key: &str) -> Result<Vec<Record>> {
    let body = content.strip_prefix(b"\xef\xbb\xbf".as_slice()).unwrap_or(content);
    let decoded = pyjson::decode_utf8(body, false).map_err(Failure::Error)?;
    let text: String = decoded
        .iter()
        .map(|&c| char::from_u32(c).expect("strict UTF-8 has no surrogates"))
        .collect();
    let csv_error = |error: pycsv::CsvError| Failure::Error(error.0);
    let reader = DictReader::new(&text).map_err(csv_error)?;
    let expected: Vec<&str> = fields.split(' ').collect();
    let headings = reader.fieldnames.clone();
    if headings
        .as_ref()
        .map(|h| h.iter().map(String::as_str).eq(expected.iter().copied()))
        != Some(true)
    {
        return fail(format!("Expected exact CSV headings: {fields}"));
    }
    let headings = headings.expect("checked above");
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for row in reader {
        let row = row.map_err(csv_error)?;
        if row.has_extra || row.values.iter().any(Option::is_none) {
            return fail("Every CSV row must have the exact field count");
        }
        let record: Record = headings
            .iter()
            .cloned()
            .zip(row.values.into_iter().map(Option::unwrap))
            .collect();
        let identifier = field(&record, key);
        if pytext::strip(identifier).is_empty() || !seen.insert(identifier.to_owned()) {
            return fail(format!("Missing or duplicate {key}: {}", pytext::repr(identifier)));
        }
        for (name, value) in &record {
            text_field(value, name, !pytext::strip(value).is_empty())?;
        }
        result.push(record);
    }
    Ok(result)
}

/// The script's inputs, read once.
pub struct Inputs {
    pub content: Vec<(&'static str, Vec<u8>)>,
}

const NAMES: [&str; 4] = ["agreements.json", "ledger.csv", "allocations.csv", "receipts.csv"];

/// `review(source)`.
pub fn review(source: &str, cwd: &Path) -> Result<Json> {
    review_ignoring(source, None, cwd)
}

/// `review(source)` with an optional ignore file of out-of-period ledger rows.
pub fn review_ignoring(source: &str, ignore: Option<&str>, cwd: &Path) -> Result<Json> {
    let mut content = read_inputs(source, cwd)?;
    if let Some(path) = ignore {
        let shown = display(path);
        let bytes = std::fs::read(cwd.join(oserror::encode_os(&shown)))
            .map_err(|e| Failure::Error(format!("Ignore file: {}", file_error(&e, &shown))))?;
        content.push(("ignore_file", bytes));
    }
    let bytes = |name: &str| &content.iter().find(|(n, _)| *n == name).expect("read above").1;
    let agreements = match pyjson::loads_bytes(bytes("agreements.json")) {
        Ok(value) => value,
        Err(LoadError::Value(message)) => return fail(message),
        Err(LoadError::Recursion) => return fail("agreements.json is nested too deeply to read"),
    };
    let (start, end) = check_header(&agreements)?;
    let ledger = rows(
        bytes("ledger.csv"),
        "ledger_id date description amount cash_paid evidence",
        "ledger_id",
    )?;
    let allocations = rows(
        bytes("allocations.csv"),
        "allocation_id ledger_id grant_id budget_line amount cash_allocated approved evidence",
        "allocation_id",
    )?;
    let receipts = rows(
        bytes("receipts.csv"),
        "receipt_id grant_id date amount evidence",
        "receipt_id",
    )?;
    let contributions = if content.iter().any(|(n, _)| *n == "contributions.csv") {
        rows(
            bytes("contributions.csv"),
            "contribution_id grant_id date kind amount evidence",
            "contribution_id",
        )?
    } else {
        Vec::new()
    };
    let ignored = match ignore {
        Some(_) => Some(ignore_entries(bytes("ignore_file"))?),
        None => None,
    };
    let agreement_rows = match agreements.at("agreements") {
        Json::Array(items) if !ledger.is_empty() && !items.is_empty() => items,
        _ => return fail("Supply ledger evidence and at least one reviewed agreement"),
    };
    calculate(&Sources {
        header: &agreements,
        agreements: agreement_rows,
        ledger: &ledger,
        allocations: &allocations,
        receipts: &receipts,
        contributions: &contributions,
        ignored: ignored.as_deref(),
        start,
        end,
        content: &content,
    })
}

/// The ignore file's entries: `ledger_id,reason`, each with a visible reason.
fn ignore_entries(content: &[u8]) -> Result<Vec<Record>> {
    let prefixed = |Failure::Error(message)| Failure::Error(format!("Ignore file: {message}"));
    let entries = rows(content, "ledger_id reason", "ledger_id").map_err(prefixed)?;
    for entry in &entries {
        text_field(field(entry, "reason"), "Reason", true).map_err(prefixed)?;
    }
    Ok(entries)
}

/// The four required files, then contributions.csv when present (a failure
/// to inspect it is not treated as absence).
fn read_inputs(source: &str, cwd: &Path) -> Result<Vec<(&'static str, Vec<u8>)>> {
    let root = cwd.join(oserror::encode_os(source));
    let mut content: Vec<(&'static str, Vec<u8>)> = Vec::new();
    for name in NAMES {
        let shown = oserror::join(source, name);
        let bytes = std::fs::read(root.join(name)).map_err(|e| Failure::Error(file_error(&e, &shown)))?;
        content.push((name, bytes));
    }
    // Optional: interest earned, cash co-contributions and in-kind contributions.
    let contribution = root.join("contributions.csv");
    let contribution_shown = oserror::join(source, "contributions.csv");
    let inspected = std::fs::symlink_metadata(&contribution);
    #[cfg(test)]
    let inspected = faults::check("inspect", &contribution).and(inspected);
    match inspected {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return fail(os_error(&error, &contribution_shown, None)),
        Ok(_) => {
            let read = std::fs::read(&contribution);
            #[cfg(test)]
            let read = faults::check("read", &contribution).and(read);
            let bytes = read.map_err(|e| Failure::Error(file_error(&e, &contribution_shown)))?;
            content.push(("contributions.csv", bytes));
        }
    }
    Ok(content)
}

/// The agreements file's top level; returns the ledger period.
fn check_header(agreements: &Json) -> Result<(Date, Date)> {
    let fields = [
        "schema_version",
        "entity",
        "currency",
        "ledger_start",
        "ledger_end",
        "amount_basis",
        "agreements",
    ];
    let schema_ok = agreements.has_exact_keys(&fields)
        && matches!(agreements.at("schema_version"), Json::Int(digits) if digits == "1");
    if !schema_ok {
        return fail("Unexpected agreement schema");
    }
    text_json(agreements.at("entity"), "Entity")?;
    if !agreements.at("currency").is_str("AUD") || !agreements.at("amount_basis").is_str("GST-inclusive") {
        return fail("Version 1 requires one named entity, AUD and explicitly GST-inclusive amounts");
    }
    let start = iso_json(agreements.at("ledger_start"))?;
    let end = iso_json(agreements.at("ledger_end"))?;
    if start > end {
        return fail("Ledger period is reversed");
    }
    Ok((start, end))
}

const CONTRIBUTION_KINDS: [&str; 3] = ["interest", "cash_contribution", "in_kind"];

fn finding(code: &str, reference: &str) -> Json {
    Json::object([("code", Json::str(code)), ("reference", Json::str(reference))])
}

fn record_json(record: &Record) -> Vec<(PyStr, Json)> {
    record
        .iter()
        .map(|(k, v)| (PyStr::from(k.as_str()), Json::str(v)))
        .collect()
}

/// A Python dict keyed by (grant, line) or similar, with `defaultdict(Decimal)`
/// lookups. Each total also keeps the source rows that added a non-zero amount
/// to it, in the order they were read.
#[derive(Default)]
struct Totals(BTreeMap<(String, String), Spend>);

impl Totals {
    fn get(&self, key: &(String, String)) -> Amount {
        self.0.get(key).map_or(Amount::ZERO, |total| total.amount)
    }

    fn rows(&self, key: &(String, String)) -> &[String] {
        self.0.get(key).map_or(&[], |total| total.rows.as_slice())
    }

    fn add(&mut self, key: (String, String), value: Amount, row: &str) {
        self.0.entry(key).or_default().add(value, row);
    }
}

/// A total summed across budget lines or ledger rows, with the rows behind it
/// in first-seen order.
struct Spend {
    amount: Amount,
    rows: Vec<String>,
    seen: HashSet<String>,
}

impl Default for Spend {
    fn default() -> Spend {
        Spend {
            amount: Amount::ZERO,
            rows: Vec::new(),
            seen: HashSet::new(),
        }
    }
}

impl Spend {
    fn add(&mut self, value: Amount, row: &str) {
        self.amount = self.amount.add(value);
        if !value.is_zero() && self.seen.insert(row.to_string()) {
            self.rows.push(row.to_string());
        }
    }
}

/// The rows behind a total, as a JSON list of their IDs.
fn row_list(rows: &[String]) -> Json {
    Json::Array(rows.iter().map(|row| Json::str(row)).collect())
}

/// The ledger period and the files the workpaper is calculated from.
struct Sources<'a> {
    header: &'a Json,
    agreements: &'a [Json],
    ledger: &'a [Record],
    allocations: &'a [Record],
    receipts: &'a [Record],
    contributions: &'a [Record],
    /// The ignore file's entries, when one was supplied.
    ignored: Option<&'a [Record]>,
    start: Date,
    end: Date,
    content: &'a [(&'static str, Vec<u8>)],
}

struct LedgerSource<'a> {
    record: &'a Record,
    date: Date,
    expense: Amount,
    paid: Amount,
    allocated: Amount,
    cash_allocated: Amount,
}

#[derive(Clone, Copy)]
struct Agreement<'a> {
    source: &'a Json,
    start: Date,
    end: Date,
    budget: &'a [(PyStr, Json)],
}

/// What `calculate` accumulates. Each stage runs in the script's order, so the
/// first refusal and the order of findings are unchanged.
struct Workings<'a> {
    grants: &'a [(String, Agreement<'a>)],
    findings: Vec<Json>,
    sources: HashMap<&'a str, LedgerSource<'a>>,
    totals: Totals,
    period_totals: Totals,
    approved_totals: Totals,
    paid_totals: Totals,
    /// Expenditure and cash allocated to each grant, keyed (grant, ""), so the
    /// rows behind a grant's totals keep allocation order across budget lines.
    grant_totals: Totals,
    grant_paid_totals: Totals,
    details: Vec<Json>,
    /// Out-of-period ledger rows the ignore file leaves out, with their reasons.
    ignored: Vec<Json>,
    ignored_ids: HashSet<&'a str>,
    funding: Totals,
    contributed: Totals,
}

fn calculate(input: &Sources) -> Result<Json> {
    let grants = agreements(input.agreements)?;
    let mut work = Workings {
        grants: &grants,
        findings: Vec::new(),
        sources: HashMap::with_capacity(input.ledger.len()),
        totals: Totals::default(),
        period_totals: Totals::default(),
        approved_totals: Totals::default(),
        paid_totals: Totals::default(),
        grant_totals: Totals::default(),
        grant_paid_totals: Totals::default(),
        details: Vec::new(),
        ignored: Vec::new(),
        ignored_ids: HashSet::new(),
        funding: Totals::default(),
        contributed: Totals::default(),
    };
    work.ledger(input.ledger, input.ignored.unwrap_or_default(), input.start, input.end)?;
    for row in input.allocations {
        work.allocation(row)?;
    }
    work.receipts(input.receipts, input.start, input.end)?;
    work.contributions(input.contributions, input.start, input.end)?;
    let contribution_rows = work.contribution_rows();
    let tieout = work.tieout(input.ledger);
    let (budget_rows, movements) = work.budgets()?;
    Ok(work.result(input, budget_rows, movements, contribution_rows, tieout))
}

/// The agreements block: complete, unique and with a reviewed budget.
fn agreements(agreements: &[Json]) -> Result<Vec<(String, Agreement<'_>)>> {
    let mut grants = Vec::with_capacity(agreements.len());
    let mut identifiers = HashSet::with_capacity(agreements.len());
    let fields = [
        "grant_id",
        "start",
        "end",
        "opening_cash",
        "opening_unspent_funding",
        "budget",
        "agreement_evidence",
    ];
    for grant in agreements {
        if !grant.has_exact_keys(&fields) {
            return fail("Agreement IDs must be unique with complete fields");
        }
        let identifier = text_json(grant.at("grant_id"), "Grant ID")?;
        if !identifiers.insert(identifier) {
            return fail("Agreement IDs must be unique with complete fields");
        }
        text_json(grant.at("agreement_evidence"), "Agreement evidence")?;
        let start = iso_json(grant.at("start"))?;
        let end = iso_json(grant.at("end"))?;
        if start > end {
            return fail("Agreement dates and reviewed summary evidence are required");
        }
        money_json(grant.at("opening_cash"))?;
        money_json(grant.at("opening_unspent_funding"))?;
        let budget = match grant.at("budget") {
            Json::Object(budget) if !budget.is_empty() => budget,
            _ => return fail("Supply the reviewed budget-line mapping"),
        };
        for (key, value) in budget {
            text_cps(&key.0, "Budget line", true)?;
            money_json(value)?;
        }
        grants.push((
            identifier.lossy(),
            Agreement {
                source: grant,
                start,
                end,
                budget,
            },
        ));
    }
    Ok(grants)
}

/// A ledger row's reason to be ignored: only a row dated outside the ledger
/// period may be ignored, and such a row must be.
fn ignore_reason<'r>(
    row: &Record,
    date: Date,
    reason: Option<&'r str>,
    start: Date,
    end: Date,
) -> Result<Option<&'r str>> {
    match (date < start || date > end, reason) {
        (true, None) => fail("Ledger rows must fall inside the declared ledger period"),
        (false, Some(_)) => fail(format!(
            "Ignore file: ledger row {} is inside the declared ledger period",
            pytext::repr(field(row, "ledger_id"))
        )),
        _ => Ok(reason),
    }
}

/// `start <= date(text) <= end`.
fn within(text: &str, start: Date, end: Date) -> Result<bool> {
    let date = iso(text)?;
    Ok(start <= date && date <= end)
}

fn blank(text: &str) -> bool {
    pytext::strip(text).is_empty()
}

impl<'a> Workings<'a> {
    fn grant(&self, id: &str) -> Option<Agreement<'a>> {
        self.grants.iter().find(|(g, _)| g == id).map(|(_, g)| *g)
    }

    fn ledger(&mut self, ledger: &'a [Record], ignored: &'a [Record], start: Date, end: Date) -> Result<()> {
        let reasons: HashMap<&str, &str> = ignored
            .iter()
            .map(|entry| (field(entry, "ledger_id"), field(entry, "reason")))
            .collect();
        for row in ledger {
            let id = field(row, "ledger_id");
            let date = iso(field(row, "date"))?;
            let reason = ignore_reason(row, date, reasons.get(id).copied(), start, end)?;
            let expense = money(field(row, "amount"))?;
            let paid = money(field(row, "cash_paid"))?;
            if expense <= Amount::ZERO || paid > expense {
                return fail("Source expenses must be positive; cash paid cannot exceed the expense");
            }
            if let Some(reason) = reason {
                let mut item = record_json(row);
                item.push((PyStr::from("reason"), Json::str(reason)));
                self.ignored.push(Json::Object(item));
                self.ignored_ids.insert(id);
                continue;
            }
            self.sources.insert(
                id,
                LedgerSource {
                    record: row,
                    date,
                    expense,
                    paid,
                    allocated: Amount::ZERO,
                    cash_allocated: Amount::ZERO,
                },
            );
            if blank(field(row, "evidence")) {
                self.findings.push(finding("MISSING_SOURCE_EVIDENCE", id));
            }
        }
        match ignored
            .iter()
            .find(|entry| !self.ignored_ids.contains(field(entry, "ledger_id")))
        {
            Some(entry) => fail(format!(
                "Ignore file: no ledger row {}",
                pytext::repr(field(entry, "ledger_id"))
            )),
            None => Ok(()),
        }
    }

    fn allocation(&mut self, row: &Record) -> Result<()> {
        let grant = self.grant(field(row, "grant_id"));
        let source = self.sources.get_mut(field(row, "ledger_id"));
        let (Some(source), Some(agreement)) = (source, grant) else {
            if self.ignored_ids.contains(field(row, "ledger_id")) {
                return fail(format!(
                    "Allocation references an ignored ledger row: {}",
                    pytext::repr(field(row, "ledger_id"))
                ));
            }
            return fail("Allocation references an unknown ledger row or grant");
        };
        let line = field(row, "budget_line");
        if !agreement.budget.iter().any(|(k, _)| k.is(line)) || !["yes", "no"].contains(&field(row, "approved")) {
            return fail("Use a reviewed budget line and approved yes/no");
        }
        let expense = money(field(row, "amount"))?;
        let paid = money(field(row, "cash_allocated"))?;
        if expense <= Amount::ZERO || paid > expense {
            return fail("Allocation expense must be positive and cover its cash allocation");
        }
        source.allocated = source.allocated.add(expense);
        source.cash_allocated = source.cash_allocated.add(paid);
        if source.allocated > source.expense || source.cash_allocated > source.paid {
            return fail("Allocations exceed a source expense or its cash paid");
        }
        let item = source.record;
        let item_date = source.date;
        let key = (field(row, "grant_id").to_string(), line.to_string());
        let ledger_id = field(row, "ledger_id");
        self.totals.add(key.clone(), expense, ledger_id);
        self.paid_totals.add(key.clone(), paid, ledger_id);
        let grant_key = (key.0.clone(), String::new());
        self.grant_totals.add(grant_key.clone(), expense, ledger_id);
        self.grant_paid_totals.add(grant_key, paid, ledger_id);
        let in_period = agreement.start <= item_date && item_date <= agreement.end;
        self.review_allocation(row, item, in_period, key, expense);
        self.details
            .push(allocation_detail(row, item, agreement.source, in_period));
        Ok(())
    }

    /// The allocation's review findings, and its share of approved, evidenced, in-period work.
    fn review_allocation(
        &mut self,
        row: &Record,
        item: &Record,
        in_period: bool,
        key: (String, String),
        expense: Amount,
    ) {
        let id = field(row, "allocation_id");
        let ledger_id = field(row, "ledger_id");
        if in_period {
            self.period_totals.add(key.clone(), expense, ledger_id);
        } else {
            self.findings.push(finding("OUTSIDE_AGREEMENT_PERIOD", id));
        }
        let supported = !blank(field(item, "evidence")) && !blank(field(row, "evidence"));
        if !supported {
            self.findings.push(finding("MISSING_ALLOCATION_EVIDENCE", id));
        }
        let approved = field(row, "approved") == "yes";
        if !approved {
            self.findings.push(finding("UNAPPROVED_ALLOCATION", id));
        }
        if in_period && supported && approved {
            self.approved_totals.add(key, expense, ledger_id);
        }
    }

    fn receipts(&mut self, receipts: &[Record], start: Date, end: Date) -> Result<()> {
        for row in receipts {
            if self.grant(field(row, "grant_id")).is_none() || !within(field(row, "date"), start, end)? {
                return fail("Funding receipt must reference a declared grant and ledger period");
            }
            let amount = money(field(row, "amount"))?;
            if amount <= Amount::ZERO {
                return fail("Funding receipts must be positive");
            }
            let key = (field(row, "grant_id").to_string(), String::new());
            self.funding.add(key, amount, field(row, "receipt_id"));
            if blank(field(row, "evidence")) {
                self.findings
                    .push(finding("MISSING_RECEIPT_EVIDENCE", field(row, "receipt_id")));
            }
        }
        Ok(())
    }

    /// Reported beside the grant, never added to its cash or unspent funding: whether
    /// interest must be spent or returned, and how a contribution counts, is the
    /// agreement's term for the reviewer to apply.
    fn contributions(&mut self, contributions: &[Record], start: Date, end: Date) -> Result<()> {
        for row in contributions {
            if self.grant(field(row, "grant_id")).is_none() || !within(field(row, "date"), start, end)? {
                return fail("Contribution must reference a declared grant and ledger period");
            }
            if !CONTRIBUTION_KINDS.contains(&field(row, "kind")) {
                return fail(format!(
                    "Contribution kind must be one of {}",
                    CONTRIBUTION_KINDS.join(", ")
                ));
            }
            let amount = money(field(row, "amount"))?;
            if amount <= Amount::ZERO {
                return fail("Contributions must be positive");
            }
            self.contributed.add(
                (field(row, "grant_id").to_string(), field(row, "kind").to_string()),
                amount,
                field(row, "contribution_id"),
            );
            if blank(field(row, "evidence")) {
                self.findings
                    .push(finding("MISSING_CONTRIBUTION_EVIDENCE", field(row, "contribution_id")));
            }
        }
        Ok(())
    }

    fn contribution_rows(&self) -> Vec<Json> {
        self.grants
            .iter()
            .map(|(id, _)| {
                let mut pairs = vec![(PyStr::from("grant_id"), Json::str(id))];
                for kind in CONTRIBUTION_KINDS {
                    let key = (id.clone(), kind.to_string());
                    pairs.push((PyStr::from(kind), Json::str(&self.contributed.get(&key).to_string())));
                    pairs.push((
                        PyStr::from(format!("{kind}_contribution_rows").as_str()),
                        row_list(self.contributed.rows(&key)),
                    ));
                }
                Json::Object(pairs)
            })
            .collect()
    }

    fn tieout(&mut self, ledger: &[Record]) -> Vec<Json> {
        let mut tieout = Vec::with_capacity(ledger.len());
        for row in ledger {
            let key = field(row, "ledger_id");
            let Some(source) = self.sources.get(key) else {
                continue; // an ignored out-of-period row
            };
            let unallocated = source.expense.sub(source.allocated);
            let unpaid = source.paid.sub(source.cash_allocated);
            tieout.push(Json::object([
                ("ledger_id", Json::str(key)),
                ("source_amount", Json::str(field(row, "amount"))),
                ("allocated", Json::str(&source.allocated.to_string())),
                ("unallocated", Json::str(&unallocated.to_string())),
                ("source_cash_paid", Json::str(field(row, "cash_paid"))),
                ("cash_allocated", Json::str(&source.cash_allocated.to_string())),
                ("cash_unallocated", Json::str(&unpaid.to_string())),
            ]));
            if !unallocated.is_zero() || !unpaid.is_zero() {
                self.findings.push(finding("UNALLOCATED_SOURCE", key));
            }
        }
        tieout
    }

    /// Budget lines and funding movements, grant by grant.
    fn budgets(&mut self) -> Result<(Vec<Json>, Vec<Json>)> {
        let mut budget_rows: Vec<Json> = Vec::new();
        let mut movements: Vec<Json> = Vec::new();
        for (identifier, agreement) in self.grants {
            let (spent, paid) = self.budget_lines(identifier, agreement.budget, &mut budget_rows)?;
            movements.push(self.movement(identifier, agreement.source, spent, paid)?);
        }
        Ok((budget_rows, movements))
    }

    /// One grant's budget lines; returns its expenditure and cash allocated.
    fn budget_lines(
        &mut self,
        identifier: &str,
        budget: &[(PyStr, Json)],
        rows: &mut Vec<Json>,
    ) -> Result<(Amount, Amount)> {
        let (mut spent, mut paid) = (Amount::ZERO, Amount::ZERO);
        for (line, value) in budget {
            let line_text = line.lossy();
            let key = (identifier.to_string(), line_text.clone());
            spent = spent.add(self.totals.get(&key));
            paid = paid.add(self.paid_totals.get(&key));
            let mut row = vec![
                (PyStr::from("grant_id"), Json::str(identifier)),
                (PyStr::from("budget_line"), Json::Str(line.clone())),
                (PyStr::from("budget"), value.clone()),
            ];
            for (name, totals) in [
                ("allocated", &self.totals),
                ("within_period", &self.period_totals),
                ("approved_evidenced_in_period", &self.approved_totals),
                ("cash_allocated", &self.paid_totals),
            ] {
                row.push((PyStr::from(name), Json::str(&totals.get(&key).to_string())));
                row.push((
                    PyStr::from(format!("{name}_ledger_rows").as_str()),
                    row_list(totals.rows(&key)),
                ));
            }
            rows.push(Json::Object(row));
            if self.totals.get(&key) > money_json(value)? {
                self.findings
                    .push(finding("OVER_BUDGET", &format!("{identifier}/{line_text}")));
            }
        }
        Ok((spent, paid))
    }

    fn movement(&mut self, identifier: &str, agreement: &Json, spent: Amount, paid: Amount) -> Result<Json> {
        let key = (identifier.to_string(), String::new());
        let received = self.funding.get(&key);
        let remaining = money_json(agreement.at("opening_unspent_funding"))?
            .add(received)
            .sub(spent);
        let cash = money_json(agreement.at("opening_cash"))?.add(received).sub(paid);
        if remaining.is_negative() || cash.is_negative() {
            self.findings.push(finding("FUNDING_SHORTFALL", identifier));
        }
        Ok(Json::Object(vec![
            (PyStr::from("grant_id"), Json::str(identifier)),
            (PyStr::from("opening_cash"), agreement.at("opening_cash").clone()),
            (
                PyStr::from("opening_unspent_funding"),
                agreement.at("opening_unspent_funding").clone(),
            ),
            (PyStr::from("funding_received"), Json::str(&received.to_string())),
            (
                PyStr::from("funding_received_receipt_rows"),
                row_list(self.funding.rows(&key)),
            ),
            (PyStr::from("expenditure_allocated"), Json::str(&spent.to_string())),
            (
                PyStr::from("expenditure_allocated_ledger_rows"),
                row_list(self.grant_totals.rows(&key)),
            ),
            (PyStr::from("cash_paid_allocated"), Json::str(&paid.to_string())),
            (
                PyStr::from("cash_paid_allocated_ledger_rows"),
                row_list(self.grant_paid_totals.rows(&key)),
            ),
            (PyStr::from("closing_cash_allocation"), Json::str(&cash.to_string())),
            (
                PyStr::from("unspent_funding_workpaper"),
                Json::str(&remaining.to_string()),
            ),
        ]))
    }

    fn result(
        self,
        input: &Sources,
        budget_rows: Vec<Json>,
        movements: Vec<Json>,
        contribution_rows: Vec<Json>,
        tieout: Vec<Json>,
    ) -> Json {
        let status = if self.findings.is_empty() {
            "RECONCILED"
        } else {
            "REVIEW"
        };
        let header = input.header;
        let mut fields = vec![
            (PyStr::from("schema_version"), Json::Int("1".into())),
            (PyStr::from("entity"), header.at("entity").clone()),
            (PyStr::from("currency"), header.at("currency").clone()),
            (PyStr::from("ledger_start"), header.at("ledger_start").clone()),
            (PyStr::from("ledger_end"), header.at("ledger_end").clone()),
            (PyStr::from("amount_basis"), header.at("amount_basis").clone()),
            (PyStr::from("source_sha256"), digests(input.content)),
            (PyStr::from("status"), Json::str(status)),
        ];
        fields.extend(self.ledger_totals(input.ledger));
        fields.extend([
            (PyStr::from("budget_lines"), Json::Array(budget_rows)),
            (PyStr::from("funding_movements"), Json::Array(movements)),
            (PyStr::from("contributions"), Json::Array(contribution_rows)),
            (PyStr::from("allocation_evidence"), Json::Array(self.details)),
            (PyStr::from("ledger_tieout"), Json::Array(tieout)),
            (PyStr::from("findings"), Json::Array(self.findings)),
            (PyStr::from("scope"), Json::str(SCOPE)),
        ]);
        if input.ignored.is_some() {
            let at = fields.len() - 2; // after ledger_tieout, before findings and scope
            fields.insert(at, (PyStr::from("ignored_items"), Json::Array(self.ignored)));
        }
        Json::Object(fields)
    }

    /// The ledger, allocated and unallocated totals, each beside the rows behind it.
    fn ledger_totals(&self, ledger: &[Record]) -> [(PyStr, Json); 6] {
        let (mut ledger_total, mut allocated_total) = (Spend::default(), Spend::default());
        let mut unallocated_rows = Vec::new();
        for row in ledger {
            let id = field(row, "ledger_id");
            let Some(source) = self.sources.get(id) else {
                continue; // an ignored out-of-period row
            };
            ledger_total.add(source.expense, id);
            allocated_total.add(source.allocated, id);
            if source.allocated != source.expense {
                unallocated_rows.push(id.to_string());
            }
        }
        let unallocated = ledger_total.amount.sub(allocated_total.amount);
        [
            (PyStr::from("ledger_total"), Json::str(&ledger_total.amount.to_string())),
            (PyStr::from("ledger_total_ledger_rows"), row_list(&ledger_total.rows)),
            (
                PyStr::from("allocated_total"),
                Json::str(&allocated_total.amount.to_string()),
            ),
            (
                PyStr::from("allocated_total_ledger_rows"),
                row_list(&allocated_total.rows),
            ),
            (PyStr::from("unallocated_total"), Json::str(&unallocated.to_string())),
            (
                PyStr::from("unallocated_total_ledger_rows"),
                row_list(&unallocated_rows),
            ),
        ]
    }
}

const SCOPE: &str = "Workpaper arithmetic and supplied review decisions only. No grant eligibility, income recognition, liability, acquittal approval or lodgement decision.";

fn allocation_detail(row: &Record, item: &Record, agreement: &Json, in_period: bool) -> Json {
    let mut detail = record_json(row);
    detail.push((PyStr::from("ledger_date"), Json::str(field(item, "date"))));
    detail.push((PyStr::from("source_evidence"), Json::str(field(item, "evidence"))));
    detail.push((
        PyStr::from("agreement_evidence"),
        agreement.at("agreement_evidence").clone(),
    ));
    detail.push((
        PyStr::from("within_period"),
        Json::str(if in_period { "yes" } else { "no" }),
    ));
    Json::Object(detail)
}

/// `{name: sha256(bytes)}` for every file read, in reading order.
fn digests(content: &[(&'static str, Vec<u8>)]) -> Json {
    Json::Object(
        content
            .iter()
            .map(|(name, bytes)| {
                let digest: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
                (PyStr::from(*name), Json::str(&digest))
            })
            .collect(),
    )
}

/// `safe_cell(value)`: escape text a spreadsheet would read as a formula, but leave numbers alone.
pub fn safe_cell(text: &str) -> String {
    escape_cell(text.to_owned())
}

fn escape_cell(mut text: String) -> String {
    if !pytext::strip_start(&text).starts_with(['=', '+', '-', '@']) {
        return text;
    }
    if let Some(body) = text.strip_prefix('-') {
        let digit = |ch: char| pytext::decimal_value(ch as u32).is_some();
        let (int, frac) = match body.split_once('.') {
            Some((int, frac)) => (int, Some(frac)),
            None => (body, None),
        };
        let int_ok = int == "0"
            || (int.as_bytes().first().is_some_and(|byte| (b'1'..=b'9').contains(byte)) && int[1..].chars().all(digit));
        let frac_ok = frac.is_none_or(|f| !f.is_empty() && f.chars().all(digit));
        if int_ok && frac_ok {
            return text;
        }
    }
    text.insert(0, '\'');
    text
}

/// `markdown(value)`: HTML-escape, then backslash-escape Markdown punctuation.
pub fn markdown(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            _ => escaped.push(ch),
        }
    }
    let mut out = String::with_capacity(escaped.len());
    for ch in escaped.chars() {
        if "\\`*_{}[]()#+.!|>~-".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Python text mode: `\n` becomes `\r\n` on Windows.
fn text_mode(text: &str) -> Vec<u8> {
    if cfg!(windows) {
        text.replace('\n', "\r\n").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let shown = oserror::path_text(path);
    #[cfg(test)]
    faults::check("write", path).map_err(|e| Failure::Error(file_error(&e, &shown)))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| Failure::Error(file_error(&e, &shown)))?;
    file.write_all(bytes)
        .map_err(|e| Failure::Error(file_error(&e, &shown)))
}

const TABLES: [&str; 6] = [
    "budget_lines",
    "funding_movements",
    "contributions",
    "allocation_evidence",
    "ledger_tieout",
    "findings",
];

fn array<'a>(value: &'a Json, key: &str) -> Result<&'a [Json]> {
    match value.get(key) {
        Some(Json::Array(items)) => Ok(items),
        _ => fail(format!("Invalid workpaper: {key} must be a list")),
    }
}

type TableRow = [(PyStr, Json)];

fn validated_rows<'a>(name: &str, rows: &'a [Json]) -> Result<Vec<&'a TableRow>> {
    let is_unicode = |points: &[u32]| points.iter().copied().all(|point| char::from_u32(point).is_some());
    let mut validated = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let Json::Object(fields) = row else {
            return fail(format!("Invalid workpaper: {name}[{index}] must be an object"));
        };
        for (key, value) in fields {
            if !is_unicode(&key.0) {
                return fail(format!(
                    "Invalid workpaper: {name}[{index}] field names must be valid Unicode"
                ));
            }
            if let Json::Str(text) = value
                && !is_unicode(&text.0)
            {
                return fail(format!(
                    "Invalid workpaper: {name}[{index}] field values must be valid Unicode"
                ));
            }
        }
        validated.push(fields.as_slice());
    }
    Ok(validated)
}

/// Every table to write, with `ignored_items` when an ignore file was used.
fn validated_tables(result: &Json) -> Result<Vec<(&'static str, Vec<&TableRow>)>> {
    let mut tables = Vec::with_capacity(TABLES.len() + 1);
    for name in TABLES {
        tables.push((name, validated_rows(name, array(result, name)?)?));
    }
    if result.get("ignored_items").is_some() {
        tables.push((
            "ignored_items",
            validated_rows("ignored_items", array(result, "ignored_items")?)?,
        ));
    }
    Ok(tables)
}

/// `write_files(result, output)`.
pub fn write_files(result: &Json, output: &Path) -> Result<()> {
    let tables = validated_tables(result)?;
    let report = markdown_report(result)?;
    write_validated_files(result, output, tables, &report)
}

fn write_validated_files(
    result: &Json,
    output: &Path,
    tables: Vec<(&str, Vec<&TableRow>)>,
    report: &[u8],
) -> Result<()> {
    write_new(
        &output.join("workpaper.json"),
        &text_mode(&(pyjson::dumps_indent2(result) + "\n")),
    )?;
    for (name, rows) in tables {
        write_new(&output.join(format!("{name}.csv")), csv_table(&rows).as_bytes())?;
    }
    write_new(&output.join("workpaper.md"), report)
}

/// `csv.DictWriter` over the union of the rows' keys, with formula-like cells escaped.
fn csv_table(table: &[&TableRow]) -> String {
    let mut keys = Vec::new();
    let mut columns = HashMap::new();
    for &row in table {
        for (key, _) in row {
            if let std::collections::hash_map::Entry::Vacant(column) = columns.entry(key.0.as_slice()) {
                column.insert(keys.len());
                keys.push(key);
            }
        }
    }
    let fields: Vec<String> = if keys.is_empty() {
        vec!["code".into(), "reference".into()]
    } else {
        keys.iter().map(|key| key.lossy()).collect()
    };
    let mut csv = pycsv::write_row(&fields);
    let mut cells = vec![String::new(); fields.len()];
    for row in table {
        // Match Json::get's first value if a Rust caller supplies duplicate keys.
        for (position, (key, value)) in row.iter().enumerate().rev() {
            let column = if keys.get(position) == Some(&key) {
                position
            } else {
                columns[key.0.as_slice()]
            };
            let text = match value {
                Json::Str(text) => text.lossy(),
                other => pyjson::repr(other),
            };
            cells[column] = escape_cell(text);
        }
        pycsv::append_row(&mut csv, &cells);
        cells.fill_with(String::new);
    }
    csv
}

fn required_report_text(value: &Json, key: &str) -> Result<String> {
    match value.get(key) {
        Some(Json::Str(text)) => text
            .to_utf8()
            .ok_or_else(|| Failure::Error(format!("Invalid workpaper: report field {key} must be valid Unicode"))),
        Some(other) if key != "scope" => Ok(pyjson::repr(other)),
        Some(_) => fail("Invalid workpaper: report field scope must be text"),
        None => fail(format!("Invalid workpaper: report field {key} is missing")),
    }
}

/// Render the complete report before creating any output.
fn markdown_report(result: &Json) -> Result<Vec<u8>> {
    let mut lines = vec![
        "# Grant acquittal workpaper".to_string(),
        String::new(),
        format!(
            "Entity: {}. Currency: AUD. Basis: GST-inclusive.",
            markdown(&required_report_text(result, "entity")?)
        ),
        format!(
            "Ledger period: {} to {}. Result: {}.",
            required_report_text(result, "ledger_start")?,
            required_report_text(result, "ledger_end")?,
            required_report_text(result, "status")?
        ),
        String::new(),
        format!(
            "Ledger expenditure {} = allocated {} + unallocated {}.",
            required_report_text(result, "ledger_total")?,
            required_report_text(result, "allocated_total")?,
            required_report_text(result, "unallocated_total")?
        ),
        String::new(),
        "See allocation_evidence.csv for each agreement, source expense and review decision; funding_movements.csv separates cash from expenditure.".into(),
        "contributions.csv reports interest, cash co-contributions and in-kind contributions per grant; none is added to cash or unspent funding.".into(),
        String::new(),
        "| Grant | Closing cash allocation | Unspent funding workpaper |".into(),
        "| --- | ---: | ---: |".into(),
    ];
    for row in array(result, "funding_movements")? {
        lines.push(format!(
            "| {} | {} | {} |",
            markdown(&required_report_text(row, "grant_id")?),
            required_report_text(row, "closing_cash_allocation")?,
            required_report_text(row, "unspent_funding_workpaper")?
        ));
    }
    lines.extend(ignored_lines(result)?);
    lines.extend([String::new(), "## Items requiring review".to_string(), String::new()]);
    let found = array(result, "findings")?;
    for row in found {
        lines.push(format!(
            "- {}: {}",
            markdown(&required_report_text(row, "code")?),
            markdown(&required_report_text(row, "reference")?)
        ));
    }
    if found.is_empty() {
        lines.push("No differences found in the supplied evidence.".into());
    }
    lines.extend([String::new(), required_report_text(result, "scope")?, String::new()]);
    Ok(text_mode(&lines.join("\n")))
}

/// The report's list of ignored out-of-period ledger rows, when an ignore file was used.
fn ignored_lines(result: &Json) -> Result<Vec<String>> {
    if result.get("ignored_items").is_none() {
        return Ok(Vec::new());
    }
    let items = array(result, "ignored_items")?;
    let mut lines = vec![
        String::new(),
        "## Ignored out-of-period ledger rows".to_string(),
        String::new(),
        "The ignore file leaves these rows out of every total above.".to_string(),
        String::new(),
    ];
    for row in items {
        lines.push(format!(
            "- {} ({}, {}): {}",
            markdown(&required_report_text(row, "ledger_id")?),
            markdown(&required_report_text(row, "date")?),
            markdown(&required_report_text(row, "amount")?),
            markdown(&required_report_text(row, "reason")?)
        ));
    }
    if items.is_empty() {
        lines.push("None.".into());
    }
    Ok(lines)
}

/// Atomically publish a complete directory without replacing any destination.
pub fn publish_directory(staged: &Path, output: &Path, output_shown: &str) -> Result<()> {
    publish(staged, output).map_err(|error| {
        if cfg!(windows) {
            Failure::Error(os_error(&error, &oserror::path_text(staged), Some(output_shown)))
        } else {
            Failure::Error(os_error(&error, output_shown, None))
        }
    })
}

#[cfg(target_os = "linux")]
fn publish(staged: &Path, output: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(staged.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
    let destination = CString::new(output.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
    // SAFETY: both arguments are valid NUL-terminated paths for the duration of the call.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "macos")]
fn publish(staged: &Path, output: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(staged.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
    let destination = CString::new(output.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
    // SAFETY: both arguments are valid NUL-terminated paths for the duration of the call.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    let result = unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn publish(staged: &Path, output: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let wide = |p: &Path| {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let (source, destination) = (wide(staged), wide(output));
    // SAFETY: both buffers are NUL-terminated UTF-16 paths that outlive the call. With
    // no flags MoveFileExW refuses an existing destination, as Python's os.rename does.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn publish(_staged: &Path, _output: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("Exclusive directory publication is unsupported"))
}

fn temporary_directory(parent: &Path) -> std::io::Result<PathBuf> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let parent = std::path::absolute(parent)?;
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    for attempt in 0u32..100 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let seed = nanos ^ std::process::id().rotate_left(13) ^ attempt.wrapping_mul(0x9E37_79B9);
        let name: String = (0..8)
            .map(|i| b"abcdefghijklmnopqrstuvwxyz0123456789_"[((seed >> (i * 4)) as usize + i * 7) % 37] as char)
            .collect();
        let candidate = parent.join(format!(".grant-workpaper-{name}"));
        #[cfg(test)]
        faults::check("stage_directory", &candidate)?;
        match builder.create(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
}

/// `Path.mkdir(parents=True, exist_ok=True)`, reporting the path that failed.
fn mkdir_parents(path: &Path) -> std::result::Result<(), (std::io::Error, PathBuf)> {
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match path.parent() {
                Some(parent) if parent != path && !parent.as_os_str().is_empty() => mkdir_parents(parent)?,
                _ => return Err((error, path.to_path_buf())),
            }
            match std::fs::create_dir(path) {
                Ok(()) => Ok(()),
                Err(_) if path.is_dir() => Ok(()),
                Err(error) => Err((error, path.to_path_buf())),
            }
        }
        Err(_) if path.is_dir() => Ok(()),
        Err(error) => Err((error, path.to_path_buf())),
    }
}

/// `write(result, output)`: stage every file, then publish the directory exclusively.
pub fn write(result: &Json, output_raw: &str, cwd: &Path) -> Result<()> {
    let output_shown = display(output_raw);
    // Path(output) drops a trailing separator and `.` parts before any check.
    let output = cwd.join(oserror::encode_os(&output_shown));
    if output.exists() || output.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
        return fail(output_shown);
    }
    let tables = validated_tables(result)?;
    let report = markdown_report(result)?;
    let parent = output
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| cwd.to_path_buf());
    mkdir_parents(&parent).map_err(|(error, at)| {
        let shown = at
            .strip_prefix(cwd)
            .map(oserror::path_text)
            .unwrap_or_else(|_| oserror::path_text(&at));
        Failure::Error(os_error(&error, &shown, None))
    })?;
    let temporary =
        temporary_directory(&parent).map_err(|e| Failure::Error(os_error(&e, &oserror::path_text(&parent), None)))?;
    let outcome = (|| -> Result<()> {
        let staged = temporary.join("complete");
        std::fs::create_dir(&staged).map_err(|e| Failure::Error(os_error(&e, &oserror::path_text(&staged), None)))?;
        write_validated_files(result, &staged, tables, &report)?;
        #[cfg(test)]
        faults::check("publish", &output).map_err(|e| Failure::Error(os_error(&e, &output_shown, None)))?;
        publish_directory(&staged, &output, &output_shown)
    })();
    let _ = std::fs::remove_dir_all(&temporary);
    outcome
}

/// Test-only failure injection at the points the Python tests patched:
/// inspecting and reading contributions.csv, creating staging directories,
/// writing a file, and publishing.
#[cfg(test)]
pub mod faults {
    use std::cell::RefCell;
    use std::path::Path;

    type Hook = Box<dyn FnMut(&str, &Path) -> std::io::Result<()>>;

    thread_local! {
        static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    /// Clears the hook when dropped, so a later test on the same thread runs clean.
    pub struct Installed;

    impl Drop for Installed {
        fn drop(&mut self) {
            HOOK.with(|hook| *hook.borrow_mut() = None);
        }
    }

    pub fn install(hook: impl FnMut(&str, &Path) -> std::io::Result<()> + 'static) -> Installed {
        HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
        Installed
    }

    pub fn check(point: &str, path: &Path) -> std::io::Result<()> {
        HOOK.with(|hook| hook.borrow_mut().as_mut().map_or(Ok(()), |hook| hook(point, path)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_cell_matches_python() {
        for text in ["@SUM(A1)", "-1+1", "=1+1", "+1", "-001", "-01.5", " =1"] {
            assert_eq!(safe_cell(text), format!("'{text}"), "{text}");
        }
        for number in ["-1320", "-0", "-0.00", "-1٢.٣", "0", "12.50", "plain"] {
            assert_eq!(safe_cell(number), number);
        }
    }

    #[test]
    fn csv_tables_keep_first_seen_fields_and_render_non_string_values() {
        let rows = [
            Json::object([
                ("reference", Json::str("first")),
                ("amount", Json::Int("42".into())),
                ("reference", Json::str("duplicate")),
            ]),
            Json::object([("reference", Json::str("second")), ("révisé", Json::Bool(true))]),
        ];
        assert_eq!(
            csv_table(&validated_rows("findings", &rows).unwrap()),
            "reference,amount,révisé\r\nfirst,42,\r\nsecond,,True\r\n"
        );
    }

    #[test]
    fn csv_table_appends_blank_and_quoted_records() {
        assert_eq!(csv_table(&[]), "code,reference\r\n");
        let rows = [
            Json::object([("reference", Json::str("first, \"A\"\r\nsecond"))]),
            Json::object([("reference", Json::str(""))]),
            Json::object([("reference", Json::str("\"\""))]),
        ];
        assert_eq!(
            csv_table(&validated_rows("findings", &rows).unwrap()),
            "reference\r\n\"first, \"\"A\"\"\r\nsecond\"\r\n\"\"\r\n\"\"\"\"\"\"\r\n"
        );
    }

    #[test]
    fn staging_refuses_a_missing_parent_without_creating_it() {
        let temp = crate::tests::TempDir::new();
        let parent = temp.path().join("missing-parent");
        let error = temporary_directory(&parent).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(!parent.exists());
    }

    #[test]
    fn staging_retries_an_occupied_name_and_preserves_existing_contents() {
        let temp = crate::tests::TempDir::new();
        let mut first = true;
        let _hook = faults::install(move |point, candidate| {
            assert_eq!(point, "stage_directory");
            if std::mem::take(&mut first) {
                std::fs::create_dir(candidate)?;
                std::fs::write(candidate.join("existing"), b"another run")?;
            }
            Ok(())
        });

        let staged = temporary_directory(temp.path()).unwrap();
        assert_eq!(std::fs::read_dir(&staged).unwrap().count(), 0);
        let other: Vec<_> = std::fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path != &staged)
            .collect();
        assert_eq!(other.len(), 1);
        assert_eq!(std::fs::read(other[0].join("existing")).unwrap(), b"another run");
    }

    #[test]
    fn staging_returns_already_exists_when_every_attempt_collides() {
        let temp = crate::tests::TempDir::new();
        let attempts = std::rc::Rc::new(std::cell::Cell::new(0));
        let called = attempts.clone();
        let _hook = faults::install(move |point, candidate| {
            assert_eq!(point, "stage_directory");
            called.set(called.get() + 1);
            std::fs::create_dir_all(candidate)
        });

        let error = temporary_directory(temp.path()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(attempts.get(), 100);
    }

    #[test]
    fn markdown_matches_python() {
        assert_eq!(
            markdown("[Fabricated](https://example.invalid) *entity*"),
            "\\[Fabricated\\]\\(https://example\\.invalid\\) \\*entity\\*"
        );
        assert_eq!(markdown("A & B's <x>"), "A &amp; B&\\#x27;s &lt;x&gt;");
    }

    #[test]
    fn money_accepts_what_the_regex_does() {
        assert_eq!(money("1000").unwrap().to_string(), "1000");
        assert_eq!(money("12.5").unwrap().to_string(), "12.5");
        assert_eq!(money("\u{661}\u{662}").unwrap().to_string(), "12");
        let full_width = "1234567890123456.０٩";
        assert_eq!(money(full_width).unwrap().to_string(), "1234567890123456.09");
        assert_eq!(
            money_json(&Json::str(full_width)).unwrap().to_string(),
            "1234567890123456.09"
        );
        for bad in [
            "",
            "NaN",
            "Infinity",
            "-1",
            "1e3",
            "1.234",
            "12345678901234567",
            " 1",
            "1.",
            ".1",
            "1..2",
            "1234567890123456789",
        ] {
            assert!(money(bad).is_err(), "{bad}");
        }
    }
}
