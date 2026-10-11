//! Local grant allocation and funding workpapers from explicitly reviewed
//! inputs: a line-by-line port of `grant_workpaper.py`.

use std::collections::BTreeMap;
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

/// How a run stops early: an exception the script caught and printed (exit 1),
/// or one it did not catch, which CPython printed as a traceback (also exit 1).
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    Error(String),
    Crash(String),
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
    money_cps(&cps(text)).ok_or_else(|| {
        Failure::Error(format!(
            "Expected a non-negative decimal string with at most 2 places: {}",
            pytext::repr(text)
        ))
    })
}

/// `money(value)` for a parsed JSON value.
pub fn money_json(value: &Json) -> Result<Amount> {
    let parsed = match value {
        Json::Str(text) => money_cps(&text.0),
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
fn money_cps(text: &[u32]) -> Option<Amount> {
    let digits = |part: &[u32]| {
        part.iter()
            .map(|&c| pytext::decimal_value(c))
            .collect::<Option<Vec<u32>>>()
    };
    let (int, frac) = match text.iter().position(|&c| c == '.' as u32) {
        Some(at) => (&text[..at], Some(&text[at + 1..])),
        None => (text, None),
    };
    let int = digits(int).filter(|d| (1..=16).contains(&d.len()))?;
    let frac = match frac {
        Some(frac) => digits(frac).filter(|d| (1..=2).contains(&d.len()))?,
        None => Vec::new(),
    };
    Some(Amount::from_digits(&int, &frac))
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
    match value {
        Json::Str(text) => iso_cps(&text.0),
        // Python 3.15's wording; earlier versions said "fromisoformat: argument must be str".
        other => fail(format!(
            "fromisoformat() argument must be str, not {}",
            type_name(other)
        )),
    }
}

/// The name CPython's type errors give a parsed value's type.
fn type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "None",
        Json::Bool(_) => "bool",
        Json::Int(_) => "int",
        Json::Float(_) => "float",
        Json::Str(_) => "str",
        Json::Array(_) => "list",
        Json::Object(_) => "dict",
    }
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

pub fn text_json(value: &Json, label: &str) -> Result<()> {
    match value {
        Json::Str(text) => text_cps(&text.0, label, true),
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
    let mut seen: Vec<String> = Vec::new();
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
        if pytext::strip(identifier).is_empty() || seen.iter().any(|s| s == identifier) {
            return fail(format!("Missing or duplicate {key}: {}", pytext::repr(identifier)));
        }
        for (name, value) in &record {
            text_field(value, name, !pytext::strip(value).is_empty())?;
        }
        seen.push(identifier.to_string());
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
    let bytes = |name: &str| &content.iter().find(|(n, _)| *n == name).expect("read above").1;
    let agreements = match pyjson::loads_bytes(bytes("agreements.json")) {
        Ok(value) => value,
        Err(LoadError::Value(message)) => return fail(message),
        Err(LoadError::Recursion) => return fail("agreements.json is nested too deeply to read"),
    };
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
    let has_agreements = matches!(agreements.at("agreements"), Json::Array(items) if !items.is_empty());
    if ledger.is_empty() || !has_agreements {
        return fail("Supply ledger evidence and at least one reviewed agreement");
    }
    calculate(
        &agreements,
        &ledger,
        &allocations,
        &receipts,
        start,
        end,
        &content,
        &contributions,
    )
}

const CONTRIBUTION_KINDS: [&str; 3] = ["interest", "cash_contribution", "in_kind"];

fn finding(code: &str, reference: &str) -> Json {
    Json::object([("code", Json::str(code)), ("reference", Json::str(reference))])
}

fn text(value: &Json) -> String {
    match value {
        Json::Str(s) => s.lossy(),
        _ => unreachable!("validated as text"),
    }
}

fn record_json(record: &Record) -> Vec<(PyStr, Json)> {
    record
        .iter()
        .map(|(k, v)| (PyStr::from(k.as_str()), Json::str(v)))
        .collect()
}

/// A Python dict keyed by (grant, line) or similar, with `defaultdict(Decimal)` lookups.
#[derive(Default)]
struct Totals(BTreeMap<(String, String), Amount>);

impl Totals {
    fn get(&self, key: &(String, String)) -> Amount {
        self.0.get(key).copied().unwrap_or(Amount::ZERO)
    }

    fn add(&mut self, key: (String, String), value: Amount) {
        let total = self.get(&key).add(value);
        self.0.insert(key, total);
    }
}

#[allow(clippy::too_many_arguments)]
fn calculate(
    header: &Json,
    ledger: &[Record],
    allocations: &[Record],
    receipts: &[Record],
    start: Date,
    end: Date,
    content: &[(&'static str, Vec<u8>)],
    contributions: &[Record],
) -> Result<Json> {
    let mut grants: Vec<(String, &Json)> = Vec::new();
    let mut findings: Vec<Json> = Vec::new();
    let Json::Array(agreements) = header.at("agreements") else {
        unreachable!("checked by review")
    };
    for grant in agreements {
        let fields = [
            "grant_id",
            "start",
            "end",
            "opening_cash",
            "opening_unspent_funding",
            "budget",
            "agreement_evidence",
        ];
        if !grant.has_exact_keys(&fields) {
            return fail("Agreement IDs must be unique with complete fields");
        }
        text_json(grant.at("grant_id"), "Grant ID")?;
        let identifier = text(grant.at("grant_id"));
        if grants.iter().any(|(id, _)| *id == identifier) {
            return fail("Agreement IDs must be unique with complete fields");
        }
        text_json(grant.at("agreement_evidence"), "Agreement evidence")?;
        if iso_json(grant.at("start"))? > iso_json(grant.at("end"))? {
            return fail("Agreement dates and reviewed summary evidence are required");
        }
        money_json(grant.at("opening_cash"))?;
        money_json(grant.at("opening_unspent_funding"))?;
        let Json::Object(budget) = grant.at("budget") else {
            return fail("Supply the reviewed budget-line mapping");
        };
        if budget.is_empty() {
            return fail("Supply the reviewed budget-line mapping");
        }
        for (key, value) in budget {
            text_cps(&key.0, "Budget line", true)?;
            money_json(value)?;
        }
        grants.push((identifier, grant));
    }
    let grant = |id: &str| grants.iter().find(|(g, _)| g == id).map(|(_, g)| *g);
    let mut source_rows: Vec<(String, &Record)> = Vec::new();
    for row in ledger {
        let date = iso(field(row, "date"))?;
        if !(start <= date && date <= end) {
            return fail("Ledger rows must fall inside the declared ledger period");
        }
        let expense = money(field(row, "amount"))?;
        let paid = money(field(row, "cash_paid"))?;
        if expense <= Amount::ZERO || paid > expense {
            return fail("Source expenses must be positive; cash paid cannot exceed the expense");
        }
        source_rows.push((field(row, "ledger_id").to_string(), row));
        if pytext::strip(field(row, "evidence")).is_empty() {
            findings.push(finding("MISSING_SOURCE_EVIDENCE", field(row, "ledger_id")));
        }
    }
    let source = |id: &str| source_rows.iter().find(|(s, _)| s == id).map(|(_, r)| *r);
    let mut allocated: BTreeMap<String, Amount> = BTreeMap::new();
    let mut cash_allocated: BTreeMap<String, Amount> = BTreeMap::new();
    let (mut totals, mut period_totals, mut approved_totals, mut paid_totals) = (
        Totals::default(),
        Totals::default(),
        Totals::default(),
        Totals::default(),
    );
    let mut details: Vec<Json> = Vec::new();
    for row in allocations {
        let (Some(item), Some(agreement)) = (source(field(row, "ledger_id")), grant(field(row, "grant_id"))) else {
            return fail("Allocation references an unknown ledger row or grant");
        };
        let Json::Object(budget) = agreement.at("budget") else {
            unreachable!("checked above")
        };
        let line = field(row, "budget_line");
        if !budget.iter().any(|(k, _)| k.is(line)) || !["yes", "no"].contains(&field(row, "approved")) {
            return fail("Use a reviewed budget line and approved yes/no");
        }
        let expense = money(field(row, "amount"))?;
        let paid = money(field(row, "cash_allocated"))?;
        if expense <= Amount::ZERO || paid > expense {
            return fail("Allocation expense must be positive and cover its cash allocation");
        }
        let ledger_id = field(row, "ledger_id").to_string();
        let total = allocated.get(&ledger_id).copied().unwrap_or(Amount::ZERO).add(expense);
        allocated.insert(ledger_id.clone(), total);
        let cash = cash_allocated
            .get(&ledger_id)
            .copied()
            .unwrap_or(Amount::ZERO)
            .add(paid);
        cash_allocated.insert(ledger_id.clone(), cash);
        if total > money(field(item, "amount"))? || cash > money(field(item, "cash_paid"))? {
            return fail("Allocations exceed a source expense or its cash paid");
        }
        let key = (field(row, "grant_id").to_string(), line.to_string());
        totals.add(key.clone(), expense);
        paid_totals.add(key.clone(), paid);
        let item_date = iso(field(item, "date"))?;
        let in_period = iso_json(agreement.at("start"))? <= item_date && item_date <= iso_json(agreement.at("end"))?;
        if in_period {
            period_totals.add(key.clone(), expense);
        } else {
            findings.push(finding("OUTSIDE_AGREEMENT_PERIOD", field(row, "allocation_id")));
        }
        let supported =
            !pytext::strip(field(item, "evidence")).is_empty() && !pytext::strip(field(row, "evidence")).is_empty();
        if !supported {
            findings.push(finding("MISSING_ALLOCATION_EVIDENCE", field(row, "allocation_id")));
        }
        if field(row, "approved") != "yes" {
            findings.push(finding("UNAPPROVED_ALLOCATION", field(row, "allocation_id")));
        }
        if in_period && supported && field(row, "approved") == "yes" {
            approved_totals.add(key, expense);
        }
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
        details.push(Json::Object(detail));
    }
    let mut funding: BTreeMap<String, Amount> = BTreeMap::new();
    for row in receipts {
        let known = grant(field(row, "grant_id")).is_some();
        if !known
            || !{
                let date = iso(field(row, "date"))?;
                start <= date && date <= end
            }
        {
            return fail("Funding receipt must reference a declared grant and ledger period");
        }
        let amount = money(field(row, "amount"))?;
        if amount <= Amount::ZERO {
            return fail("Funding receipts must be positive");
        }
        let id = field(row, "grant_id").to_string();
        let total = funding.get(&id).copied().unwrap_or(Amount::ZERO).add(amount);
        funding.insert(id, total);
        if pytext::strip(field(row, "evidence")).is_empty() {
            findings.push(finding("MISSING_RECEIPT_EVIDENCE", field(row, "receipt_id")));
        }
    }
    // Reported beside the grant, never added to its cash or unspent funding: whether
    // interest must be spent or returned, and how a contribution counts, is the
    // agreement's term for the reviewer to apply.
    let mut contributed = Totals::default();
    for row in contributions {
        let known = grant(field(row, "grant_id")).is_some();
        if !known
            || !{
                let date = iso(field(row, "date"))?;
                start <= date && date <= end
            }
        {
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
        contributed.add(
            (field(row, "grant_id").to_string(), field(row, "kind").to_string()),
            amount,
        );
        if pytext::strip(field(row, "evidence")).is_empty() {
            findings.push(finding("MISSING_CONTRIBUTION_EVIDENCE", field(row, "contribution_id")));
        }
    }
    let contribution_rows: Vec<Json> = grants
        .iter()
        .map(|(id, _)| {
            let mut pairs = vec![(PyStr::from("grant_id"), Json::str(id))];
            for kind in CONTRIBUTION_KINDS {
                let total = contributed.get(&(id.clone(), kind.to_string()));
                pairs.push((PyStr::from(kind), Json::str(&total.to_string())));
            }
            Json::Object(pairs)
        })
        .collect();
    let mut tieout: Vec<Json> = Vec::new();
    for (key, row) in &source_rows {
        let allocated_here = allocated.get(key).copied().unwrap_or(Amount::ZERO);
        let cash_here = cash_allocated.get(key).copied().unwrap_or(Amount::ZERO);
        let unallocated = money(field(row, "amount"))?.sub(allocated_here);
        let unpaid = money(field(row, "cash_paid"))?.sub(cash_here);
        tieout.push(Json::object([
            ("ledger_id", Json::str(key)),
            ("source_amount", Json::str(field(row, "amount"))),
            ("allocated", Json::str(&allocated_here.to_string())),
            ("unallocated", Json::str(&unallocated.to_string())),
            ("source_cash_paid", Json::str(field(row, "cash_paid"))),
            ("cash_allocated", Json::str(&cash_here.to_string())),
            ("cash_unallocated", Json::str(&unpaid.to_string())),
        ]));
        if !unallocated.is_zero() || !unpaid.is_zero() {
            findings.push(finding("UNALLOCATED_SOURCE", key));
        }
    }
    let mut budget_rows: Vec<Json> = Vec::new();
    let mut movements: Vec<Json> = Vec::new();
    for (identifier, agreement) in &grants {
        let (mut spent, mut paid) = (Amount::ZERO, Amount::ZERO);
        let Json::Object(budget) = agreement.at("budget") else {
            unreachable!("checked above")
        };
        for (line, value) in budget {
            let line_text = line.lossy();
            let key = (identifier.clone(), line_text.clone());
            spent = spent.add(totals.get(&key));
            paid = paid.add(paid_totals.get(&key));
            budget_rows.push(Json::Object(vec![
                (PyStr::from("grant_id"), Json::str(identifier)),
                (PyStr::from("budget_line"), Json::Str(line.clone())),
                (PyStr::from("budget"), value.clone()),
                (PyStr::from("allocated"), Json::str(&totals.get(&key).to_string())),
                (
                    PyStr::from("within_period"),
                    Json::str(&period_totals.get(&key).to_string()),
                ),
                (
                    PyStr::from("approved_evidenced_in_period"),
                    Json::str(&approved_totals.get(&key).to_string()),
                ),
                (
                    PyStr::from("cash_allocated"),
                    Json::str(&paid_totals.get(&key).to_string()),
                ),
            ]));
            if totals.get(&key) > money_json(value)? {
                findings.push(finding("OVER_BUDGET", &format!("{identifier}/{line_text}")));
            }
        }
        let received = funding.get(identifier).copied().unwrap_or(Amount::ZERO);
        let remaining = money_json(agreement.at("opening_unspent_funding"))?
            .add(received)
            .sub(spent);
        let cash = money_json(agreement.at("opening_cash"))?.add(received).sub(paid);
        movements.push(Json::Object(vec![
            (PyStr::from("grant_id"), Json::str(identifier)),
            (PyStr::from("opening_cash"), agreement.at("opening_cash").clone()),
            (
                PyStr::from("opening_unspent_funding"),
                agreement.at("opening_unspent_funding").clone(),
            ),
            (PyStr::from("funding_received"), Json::str(&received.to_string())),
            (PyStr::from("expenditure_allocated"), Json::str(&spent.to_string())),
            (PyStr::from("cash_paid_allocated"), Json::str(&paid.to_string())),
            (PyStr::from("closing_cash_allocation"), Json::str(&cash.to_string())),
            (
                PyStr::from("unspent_funding_workpaper"),
                Json::str(&remaining.to_string()),
            ),
        ]));
        if remaining.is_negative() || cash.is_negative() {
            findings.push(finding("FUNDING_SHORTFALL", identifier));
        }
    }
    let mut ledger_total = Amount::ZERO;
    for row in ledger {
        ledger_total = ledger_total.add(money(field(row, "amount"))?);
    }
    let allocated_total = allocated.values().fold(Amount::ZERO, |acc, v| acc.add(*v));
    let digests = Json::Object(
        content
            .iter()
            .map(|(name, bytes)| {
                let digest: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
                (PyStr::from(*name), Json::str(&digest))
            })
            .collect(),
    );
    let status = if findings.is_empty() { "RECONCILED" } else { "REVIEW" };
    Ok(Json::Object(vec![
        (PyStr::from("schema_version"), Json::Int("1".into())),
        (PyStr::from("entity"), header.at("entity").clone()),
        (PyStr::from("currency"), header.at("currency").clone()),
        (PyStr::from("ledger_start"), header.at("ledger_start").clone()),
        (PyStr::from("ledger_end"), header.at("ledger_end").clone()),
        (PyStr::from("amount_basis"), header.at("amount_basis").clone()),
        (PyStr::from("source_sha256"), digests),
        (PyStr::from("status"), Json::str(status)),
        (PyStr::from("ledger_total"), Json::str(&ledger_total.to_string())),
        (PyStr::from("allocated_total"), Json::str(&allocated_total.to_string())),
        (
            PyStr::from("unallocated_total"),
            Json::str(&ledger_total.sub(allocated_total).to_string()),
        ),
        (PyStr::from("budget_lines"), Json::Array(budget_rows)),
        (PyStr::from("funding_movements"), Json::Array(movements)),
        (PyStr::from("contributions"), Json::Array(contribution_rows)),
        (PyStr::from("allocation_evidence"), Json::Array(details)),
        (PyStr::from("ledger_tieout"), Json::Array(tieout)),
        (PyStr::from("findings"), Json::Array(findings)),
        (
            PyStr::from("scope"),
            Json::str(
                "Workpaper arithmetic and supplied review decisions only. No grant eligibility, income recognition, liability, acquittal approval or lodgement decision.",
            ),
        ),
    ]))
}

/// `safe_cell(value)`: escape text a spreadsheet would read as a formula, but leave numbers alone.
pub fn safe_cell(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let digit = |c: &char| pytext::decimal_value(*c as u32).is_some();
    let body = chars.strip_prefix(&['-']).unwrap_or(&chars);
    let (int, frac) = match body.iter().position(|&c| c == '.') {
        Some(at) => (&body[..at], Some(&body[at + 1..])),
        None => (body, None),
    };
    let int_ok = int == ['0'] || (int.first().is_some_and(|c| ('1'..='9').contains(c)) && int[1..].iter().all(digit));
    let frac_ok = frac.is_none_or(|f| !f.is_empty() && f.iter().all(digit));
    if int_ok && frac_ok {
        return text.to_string();
    }
    if pytext::strip_start(text).starts_with(['=', '+', '-', '@']) {
        format!("'{text}")
    } else {
        text.to_string()
    }
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

fn get_text(row: &Json, key: &str) -> String {
    match row.get(key) {
        Some(Json::Str(s)) => s.lossy(),
        Some(Json::Int(digits)) => digits.clone(),
        Some(other) => pyjson::repr(other),
        None => String::new(),
    }
}

/// `write_files(result, output)`.
pub fn write_files(result: &Json, output: &Path) -> Result<()> {
    write_new(
        &output.join("workpaper.json"),
        &text_mode(&(pyjson::dumps_indent2(result) + "\n")),
    )?;
    for name in [
        "budget_lines",
        "funding_movements",
        "contributions",
        "allocation_evidence",
        "ledger_tieout",
        "findings",
    ] {
        let Json::Array(table) = result.at(name) else {
            unreachable!("built as lists")
        };
        let mut fields: Vec<String> = Vec::new();
        for row in table {
            if let Json::Object(pairs) = row {
                for (key, _) in pairs {
                    let key = key.lossy();
                    if !fields.contains(&key) {
                        fields.push(key);
                    }
                }
            }
        }
        if fields.is_empty() {
            fields = vec!["code".into(), "reference".into()];
        }
        let mut csv = pycsv::write_row(&fields);
        for row in table {
            let cells: Vec<String> = fields.iter().map(|key| safe_cell(&get_text(row, key))).collect();
            csv.push_str(&pycsv::write_row(&cells));
        }
        write_new(&output.join(format!("{name}.csv")), csv.as_bytes())?;
    }
    let entity = get_text(result, "entity");
    let mut lines = vec![
        "# Grant acquittal workpaper".to_string(),
        String::new(),
        format!("Entity: {}. Currency: AUD. Basis: GST-inclusive.", markdown(&entity)),
        format!(
            "Ledger period: {} to {}. Result: {}.",
            get_text(result, "ledger_start"),
            get_text(result, "ledger_end"),
            get_text(result, "status")
        ),
        String::new(),
        format!(
            "Ledger expenditure {} = allocated {} + unallocated {}.",
            get_text(result, "ledger_total"),
            get_text(result, "allocated_total"),
            get_text(result, "unallocated_total")
        ),
        String::new(),
        "See allocation_evidence.csv for each agreement, source expense and review decision; funding_movements.csv separates cash from expenditure.".into(),
        "contributions.csv reports interest, cash co-contributions and in-kind contributions per grant; none is added to cash or unspent funding.".into(),
        String::new(),
        "| Grant | Closing cash allocation | Unspent funding workpaper |".into(),
        "| --- | ---: | ---: |".into(),
    ];
    let Json::Array(movements) = result.at("funding_movements") else {
        unreachable!()
    };
    for row in movements {
        lines.push(format!(
            "| {} | {} | {} |",
            markdown(&get_text(row, "grant_id")),
            get_text(row, "closing_cash_allocation"),
            get_text(row, "unspent_funding_workpaper")
        ));
    }
    lines.extend(["".to_string(), "## Items requiring review".to_string(), String::new()]);
    let Json::Array(found) = result.at("findings") else {
        unreachable!()
    };
    for row in found {
        lines.push(format!(
            "- {}: {}",
            markdown(&get_text(row, "code")),
            markdown(&get_text(row, "reference"))
        ));
    }
    if found.is_empty() {
        lines.push("No differences found in the supplied evidence.".into());
    }
    lines.extend([String::new(), get_text(result, "scope"), String::new()]);
    write_new(&output.join("workpaper.md"), &text_mode(&lines.join("\n")))
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
    for attempt in 0u32..100 {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let seed = nanos ^ std::process::id().rotate_left(13) ^ attempt.wrapping_mul(0x9E37_79B9);
        let name: String = (0..8)
            .map(|i| b"abcdefghijklmnopqrstuvwxyz0123456789_"[((seed >> (i * 4)) as usize + i * 7) % 37] as char)
            .collect();
        let candidate = parent.join(format!(".grant-workpaper-{name}"));
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
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
        write_files(result, &staged)?;
        #[cfg(test)]
        faults::check("publish", &output).map_err(|e| Failure::Error(os_error(&e, &output_shown, None)))?;
        publish_directory(&staged, &output, &output_shown)
    })();
    let _ = std::fs::remove_dir_all(&temporary);
    outcome
}

/// Test-only failure injection at the points the Python tests patched:
/// inspecting and reading contributions.csv, writing a file, and publishing.
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
        for number in ["-1320", "0", "12.50", "plain"] {
            assert_eq!(safe_cell(number), number);
        }
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
        ] {
            assert!(money(bad).is_err(), "{bad}");
        }
    }
}
