//! The fixed-policy review boundary (`gateway.py`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::VERSION;
use crate::decimal::{Decimal, Rounding};
use crate::isodate::{self, Date, TimestampError};
use crate::pycsv::DictReader;
use crate::pyjson::{self, Json, PyStr};
use crate::pytext;
use crate::util::{
    Env, Failure, FileSnapshot, Location, Result, canonical_json, load_json_exact, load_json_exact_snapshot,
    load_json_object, refuse, render, sha256_hex, snapshot_file,
};

pub const CANONICAL_COLUMNS: [&str; 10] = [
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
/// The model-facing keys each finding carries, in their declared order.
pub const MODEL_PROJECTION: [&str; 9] = [
    "finding_id",
    "evidence_ref",
    "account_ref",
    "section",
    "current_ytd_net",
    "prior_ytd_net",
    "delta",
    "percent_change",
    "review_reason",
];
pub(crate) const ALLOWED_DECISIONS: [&str; 3] = ["ACKNOWLEDGED", "NEEDS_EVIDENCE", "ESCALATED"];
pub const SIGN_CONVENTION: &str =
    "debit_positive: ytd_net = YTDDebit - YTDCredit, so revenue, liability, and equity balances are negative";

#[derive(Clone, Debug)]
pub struct BalanceRow {
    pub report_date: Date,
    pub tenant: String,
    pub section: String,
    pub account_id: String,
    pub account_name: String,
    pub account_code: String,
    pub debit: Decimal,
    pub credit: Decimal,
    pub ytd_debit: Decimal,
    pub ytd_credit: Decimal,
}

impl BalanceRow {
    pub fn ytd_net(&self) -> Decimal {
        self.ytd_debit.sub(&self.ytd_credit).value
    }
}

pub struct Source {
    pub manifest: Json,
    pub rows: Vec<BalanceRow>,
    pub manifest_snapshot: FileSnapshot,
    pub csv_snapshot: FileSnapshot,
}

/// `_non_empty`: a stripped, control-free, UTF-8 encodable string.
pub fn non_empty(value: &Json, field: &str) -> Result<String> {
    let Some(text) = value.as_pystr() else {
        return refuse(format!("{field} must be a non-empty string."));
    };
    let stripped = strip_cps(&text.0);
    if stripped.is_empty() {
        return refuse(format!("{field} must be a non-empty string."));
    }
    if stripped.iter().any(|&cp| cp < 32 || cp == 127) {
        return refuse(format!("{field} must not contain control characters."));
    }
    match PyStr(stripped.to_vec()).to_utf8() {
        Some(text) => Ok(text),
        None => refuse(format!("{field} must be encodable as UTF-8 text.")),
    }
}

/// `_optional_text`: a stripped, control-free string (lone surrogates allowed).
pub fn optional_text(value: &Json, field: &str) -> Result<PyStr> {
    let Some(text) = value.as_pystr() else {
        return refuse(format!("{field} must be a string."));
    };
    let stripped = strip_cps(&text.0);
    if stripped.iter().any(|&cp| cp < 32 || cp == 127) {
        return refuse(format!("{field} must not contain control characters."));
    }
    Ok(PyStr(stripped.to_vec()))
}

fn strip_cps(cps: &[u32]) -> &[u32] {
    let start = cps.iter().position(|&cp| !pytext::is_space(cp)).unwrap_or(cps.len());
    let end = cps
        .iter()
        .rposition(|&cp| !pytext::is_space(cp))
        .map_or(start, |i| i + 1);
    &cps[start..end.max(start)]
}

pub(crate) fn iso_timestamp(value: &Json, field: &str) -> Result<String> {
    let text = non_empty(value, field)?;
    match isodate::check_timestamp(&text) {
        Ok(()) => Ok(text),
        Err(TimestampError::NoOffset) => refuse(format!("{field} must include an explicit UTC offset or Z.")),
        Err(TimestampError::Invalid) => refuse(format!("{field} must be an ISO 8601 timestamp.")),
    }
}

/// `_decimal`: a finite decimal string within adjusted exponent [-18, 18].
pub fn decimal(value: &Json, field: &str) -> Result<Decimal> {
    let Some(text) = value.as_pystr() else {
        return refuse(format!("{field} must be a decimal string."));
    };
    let parsed = text.to_utf8().and_then(|t| Decimal::parse(&t).ok());
    let Some(result) = parsed else {
        return refuse(format!("{field} is not a valid decimal."));
    };
    if !result.is_finite() {
        return refuse(format!("{field} must be finite."));
    }
    if !result.is_zero() && !(-18..=18).contains(&result.adjusted()) {
        return refuse(format!("{field} is outside the supported magnitude range."));
    }
    Ok(result)
}

fn decimal_text(value: Option<&str>, field: &str) -> Result<Decimal> {
    match value {
        Some(text) => decimal(&Json::str(text), field),
        None => refuse(format!("{field} must be a decimal string.")),
    }
}

fn csv_text(value: Option<&str>) -> Json {
    value.map_or(Json::Null, Json::str)
}

pub fn load_tb_snapshot(snapshot: &FileSnapshot) -> Result<Vec<BalanceRow>> {
    let shown = snapshot.location.display();
    let content = snapshot
        .content
        .strip_prefix(b"\xef\xbb\xbf".as_slice())
        .unwrap_or(&snapshot.content);
    let Ok(text) = std::str::from_utf8(content) else {
        return refuse(format!("source CSV is not valid UTF-8 text: {shown}."));
    };
    let csv_error = || refuse(format!("source CSV cannot be parsed as CSV: {shown}."));
    let Ok(reader) = DictReader::new(text) else {
        return csv_error();
    };
    if reader
        .fieldnames
        .as_deref()
        .map(|f| f.iter().map(String::as_str).eq(CANONICAL_COLUMNS))
        != Some(true)
    {
        return refuse("CSV must have exactly the canonical ten-column header in its declared order.");
    }
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, raw) in reader.enumerate() {
        let line = index + 2;
        let Ok(raw) = raw else { return csv_error() };
        if raw.has_extra {
            return refuse(format!("CSV row {line} has more fields than its header."));
        }
        let field = |name: &str| raw.values[CANONICAL_COLUMNS.iter().position(|c| *c == name).unwrap()].as_deref();
        let text_field = |name: &str| non_empty(&csv_text(field(name)), &format!("CSV row {line} {name}"));
        let report_date_text = text_field("ReportDate")?;
        let Some(report_date) = isodate::date_fromisoformat(&report_date_text) else {
            return refuse(format!("CSV row {line} ReportDate must be ISO YYYY-MM-DD."));
        };
        let account_id = text_field("AccountID")?;
        if !seen.insert(account_id.clone()) {
            return refuse(format!("CSV has duplicate AccountID {}.", pytext::repr(&account_id)));
        }
        let tenant = text_field("Tenant")?;
        let section = text_field("Section")?;
        let account_name = text_field("AccountName")?;
        let account_code = optional_text(&csv_text(field("AccountCode")), &format!("CSV row {line} AccountCode"))?;
        let amount = |name: &str| decimal_text(field(name), &format!("CSV row {line} {name}"));
        rows.push(BalanceRow {
            report_date,
            tenant,
            section,
            account_id,
            account_name,
            account_code: account_code.to_utf8().expect("CSV text is valid UTF-8"),
            debit: amount("Debit")?,
            credit: amount("Credit")?,
            ytd_debit: amount("YTDDebit")?,
            ytd_credit: amount("YTDCredit")?,
        });
    }
    if rows.is_empty() {
        return refuse("CSV must contain at least one account row.");
    }
    let tenants: BTreeSet<_> = rows.iter().map(|r| &r.tenant).collect();
    let dates: BTreeSet<_> = rows.iter().map(|r| r.report_date).collect();
    if tenants.len() != 1 || dates.len() != 1 {
        return refuse("CSV must contain exactly one tenant and report date.");
    }
    // Sum exactly or refuse: a total the 28-digit context rounds can equal its
    // counterpart while the source amounts differ, which would pass an imbalance.
    let exact_sum = |pick: fn(&BalanceRow) -> &Decimal| -> Result<Decimal> {
        let mut total = Decimal::zero();
        for row in &rows {
            let outcome = total.add(pick(row));
            if outcome.inexact {
                return refuse(
                    "CSV totals exceed the supported 28-digit precision, so debits and credits cannot be compared exactly.",
                );
            }
            total = outcome.value;
        }
        Ok(total)
    };
    let debit = exact_sum(|r| &r.debit)?;
    let credit = exact_sum(|r| &r.credit)?;
    let ytd_debit = exact_sum(|r| &r.ytd_debit)?;
    let ytd_credit = exact_sum(|r| &r.ytd_credit)?;
    if debit != credit {
        return refuse("CSV movement debit and credit totals are not exactly balanced.");
    }
    if ytd_debit != ytd_credit {
        return refuse("CSV YTD debit and credit totals are not exactly balanced.");
    }
    Ok(rows)
}

fn report_shape_ok(report: &Json) -> bool {
    report.has_exact_keys(&[
        "name",
        "as_at",
        "basis",
        "currency",
        "tracking_filters",
        "include_drafts",
    ])
}

pub fn load_manifest(env: &Env, location: &Location) -> Result<Source> {
    let (manifest, manifest_snapshot) = load_json_exact_snapshot(
        env,
        location,
        &[
            "schema_version",
            "mode",
            "source_system",
            "entity_ref",
            "report",
            "export",
        ],
        "source manifest",
    )?;
    if !manifest.at("schema_version").is_str("xero-source-manifest.v1") || !manifest.at("mode").is_str("synthetic") {
        return refuse("Only xero-source-manifest.v1 in synthetic mode is supported.");
    }
    if !manifest.at("source_system").is_str("xero-trial-balance-export") {
        return refuse("Source manifest must declare xero-trial-balance-export.");
    }
    non_empty(manifest.at("entity_ref"), "entity_ref")?;
    let report = manifest.at("report");
    let export = manifest.at("export");
    if !report_shape_ok(report) {
        return refuse("source manifest report has an invalid shape.");
    }
    if !export.has_exact_keys(&["schema", "csv", "sha256", "generated_at"]) {
        return refuse("source manifest export has an invalid shape.");
    }
    if !report.at("name").is_str("Trial Balance")
        || !report.at("basis").is_str("accrual")
        || !report.at("currency").is_str("AUD")
    {
        return refuse("Only accrual AUD Trial Balance reports are supported in v0.1.");
    }
    if !matches!(report.at("tracking_filters"), Json::Array(_)) || !matches!(report.at("include_drafts"), Json::Bool(_))
    {
        return refuse("source manifest report tracking/draft fields are invalid.");
    }
    let as_at_text = non_empty(report.at("as_at"), "report.as_at")?;
    let Some(as_at) = isodate::date_fromisoformat(&as_at_text) else {
        return refuse("report.as_at must be an ISO date.");
    };
    let (Some(csv), Some(sha256)) = (export.at("csv").as_pystr(), export.at("sha256").as_pystr()) else {
        return refuse("source manifest export schema, csv, or sha256 is invalid.");
    };
    if !export.at("schema").is_str("xero-tb-csv.v1") {
        return refuse("source manifest export schema, csv, or sha256 is invalid.");
    }
    iso_timestamp(export.at("generated_at"), "export.generated_at")?;
    let csv_location = env.within_bundle(csv, "samples", "manifest CSV")?;
    let csv_snapshot = snapshot_file(env, &csv_location, "manifest CSV")?;
    // str.lower(): no non-ASCII character lowercases to a hex digit, so ASCII folding is equivalent.
    let declared: Vec<u32> = sha256
        .0
        .iter()
        .map(|&cp| if (0x41..=0x5A).contains(&cp) { cp + 32 } else { cp })
        .collect();
    if !PyStr(declared).is(&csv_snapshot.sha256) {
        return refuse("manifest CSV SHA-256 does not match the supplied source file.");
    }
    let rows = load_tb_snapshot(&csv_snapshot)?;
    if rows[0].report_date != as_at {
        return refuse("manifest report.as_at and CSV ReportDate do not match.");
    }
    Ok(Source {
        manifest,
        rows,
        manifest_snapshot,
        csv_snapshot,
    })
}

/// The Australian financial year ending 30 June that contains the date.
fn au_financial_year(value: Date) -> i32 {
    if value.month >= 7 { value.year + 1 } else { value.year }
}

pub fn load_context_snapshot(env: &Env, location: &Location) -> Result<(Source, Source, FileSnapshot)> {
    let (context, context_snapshot) = load_json_exact_snapshot(
        env,
        location,
        &["schema_version", "mode", "current_manifest", "prior_manifest"],
        "review context",
    )?;
    if !context.at("schema_version").is_str("xero-review-context.v1") || !context.at("mode").is_str("synthetic") {
        return refuse("Only xero-review-context.v1 in synthetic mode is supported.");
    }
    let current_name = non_empty(context.at("current_manifest"), "current_manifest")?;
    let current_path = env.within_bundle(&PyStr::from(current_name.as_str()), "samples", "current manifest")?;
    let prior_name = non_empty(context.at("prior_manifest"), "prior_manifest")?;
    let prior_path = env.within_bundle(&PyStr::from(prior_name.as_str()), "samples", "prior manifest")?;
    let current = load_manifest(env, &current_path)?;
    let prior = load_manifest(env, &prior_path)?;
    let (current_report, prior_report) = (current.manifest.at("report"), prior.manifest.at("report"));
    for field in ["basis", "currency", "tracking_filters", "include_drafts"] {
        if !pyjson::py_eq(current_report.at(field), prior_report.at(field), false) {
            return refuse(format!("Current/prior source context mismatch for {field}."));
        }
    }
    if !pyjson::py_eq(
        current.manifest.at("entity_ref"),
        prior.manifest.at("entity_ref"),
        false,
    ) {
        return refuse("Current/prior source context has different entity_ref values.");
    }
    let (current_date, prior_date) = (current.rows[0].report_date, prior.rows[0].report_date);
    if prior_date >= current_date {
        return refuse("Prior report date must be earlier than the current report date.");
    }
    let same_day_and_month = (current_date.month, current_date.day) == (prior_date.month, prior_date.day);
    if au_financial_year(current_date) != au_financial_year(prior_date) && !same_day_and_month {
        return refuse(
            "Current and prior reports fall in different financial years; YTD balances reset on 1 July and are not comparable.",
        );
    }
    Ok((current, prior, context_snapshot))
}

pub fn load_policy_snapshot(env: &Env, location: &Location) -> Result<(Json, FileSnapshot)> {
    let (policy, policy_snapshot) = load_json_exact_snapshot(
        env,
        location,
        &["schema_version", "policy_id", "operations", "model_projection"],
        "policy",
    )?;
    let projection_ok = match policy.at("model_projection") {
        Json::Array(items) => {
            items.len() == MODEL_PROJECTION.len()
                && items.iter().zip(MODEL_PROJECTION).all(|(item, key)| item.is_str(key))
        }
        _ => false,
    };
    if !policy.at("schema_version").is_str("xero-review-policy.v1") || !projection_ok {
        return refuse("Policy schema or model projection is unsupported.");
    }
    let operations = policy.at("operations");
    if !operations.has_exact_keys(&["trial_balance_variance"]) {
        return refuse("Policy must contain only the trial_balance_variance operation.");
    }
    let operation = operations.at("trial_balance_variance");
    if !operation.has_exact_keys(&[
        "allowed_sections",
        "minimum_absolute_delta",
        "minimum_percent_delta",
        "max_results",
    ]) {
        return refuse("Policy operation has an invalid shape.");
    }
    if !matches!(operation.at("allowed_sections"), Json::Array(items) if items.iter().all(|i| matches!(i, Json::Str(_))))
    {
        return refuse("Policy allowed_sections must be a list of strings.");
    }
    let max_ok = operation
        .at("max_results")
        .as_plain_int()
        .is_some_and(|n| pyjson::int_cmp(n, 1).is_ge() && pyjson::int_cmp(n, 100).is_le());
    if !max_ok {
        return refuse("Policy max_results must be an integer from 1 to 100.");
    }
    for field in ["minimum_absolute_delta", "minimum_percent_delta"] {
        if decimal(operation.at(field), field)? < Decimal::zero() {
            return refuse(format!("Policy {field} cannot be negative."));
        }
    }
    Ok((policy, policy_snapshot))
}

pub fn load_request_snapshot(env: &Env, location: &Location, policy: &Json) -> Result<(Json, FileSnapshot)> {
    let (request, request_snapshot) = load_json_exact_snapshot(
        env,
        location,
        &["schema_version", "request_id", "operation", "section", "policy_id"],
        "review request",
    )?;
    if !request.at("schema_version").is_str("xero-review-request.v1")
        || !request.at("operation").is_str("trial_balance_variance")
    {
        return refuse("Only the trial_balance_variance request is supported.");
    }
    if !pyjson::py_eq(request.at("policy_id"), policy.at("policy_id"), false) {
        return refuse("Request policy_id does not match the selected policy.");
    }
    let Json::Array(allowed) = policy
        .at("operations")
        .at("trial_balance_variance")
        .at("allowed_sections")
    else {
        unreachable!("validated by load_policy_snapshot");
    };
    if !allowed
        .iter()
        .any(|section| pyjson::py_eq(request.at("section"), section, true))
    {
        return refuse("Request section is not allowlisted by policy.");
    }
    non_empty(request.at("request_id"), "request_id")?;
    Ok((request, request_snapshot))
}

/// At least 2 decimal places, padded and never rounded.
pub fn decimal_string(value: &Decimal) -> Result<String> {
    let value = if value.exponent() > -2 {
        // Quantizing up to 2 places is exact for the bounded amounts that reach here.
        value
            .quantize(-2, Rounding::HalfEven)
            .expect("padding a bounded amount stays within precision")
    } else {
        value.clone()
    };
    value
        .to_fixed_string()
        .ok_or_else(|| Failure::Crash("MemoryError".into()))
}

/// A change ratio as a percentage quantized to 4 decimal places.
pub fn percent_string(ratio: &Decimal) -> Result<String> {
    match ratio
        .mul(&Decimal::parse("100").unwrap())
        .quantize(-4, Rounding::HalfUp)
    {
        Ok(value) => decimal_string(&value),
        Err(_) => refuse("Percentage change is outside the supported magnitude range."),
    }
}

fn row_values(row: Option<&BalanceRow>) -> Result<Json> {
    Ok(match row {
        None => Json::Null,
        Some(row) => Json::object([
            ("debit", Json::str(&decimal_string(&row.debit)?)),
            ("credit", Json::str(&decimal_string(&row.credit)?)),
            ("ytd_debit", Json::str(&decimal_string(&row.ytd_debit)?)),
            ("ytd_credit", Json::str(&decimal_string(&row.ytd_credit)?)),
        ]),
    })
}

/// Compare every account seen in either period, so one-sided accounts cannot hide.
pub fn variance_findings(
    current_rows: &[BalanceRow],
    prior_rows: &[BalanceRow],
    entity_ref: &PyStr,
    section: &Json,
    operation: &Json,
) -> Result<Vec<(Json, Json)>> {
    let absolute = decimal(operation.at("minimum_absolute_delta"), "minimum_absolute_delta")?;
    let minimum_percent =
        decimal(operation.at("minimum_percent_delta"), "minimum_percent_delta")?.div(&Decimal::parse("100").unwrap());
    let current_by_id: BTreeMap<&str, &BalanceRow> = current_rows.iter().map(|r| (r.account_id.as_str(), r)).collect();
    let prior_by_id: BTreeMap<&str, &BalanceRow> = prior_rows.iter().map(|r| (r.account_id.as_str(), r)).collect();
    let in_section = |text: &str| section.is_str(text);
    let drifted: Vec<&str> = current_by_id
        .iter()
        .filter_map(|(id, current)| {
            let prior = prior_by_id.get(id)?;
            (current.section != prior.section && (in_section(&current.section) || in_section(&prior.section)))
                .then_some(*id)
        })
        .collect();
    if !drifted.is_empty() {
        let listed: Vec<String> = drifted.iter().map(|id| pytext::repr(id)).collect();
        return refuse(format!(
            "Accounts changed section between periods ({}); review the source mapping before comparison.",
            listed.join(", ")
        ));
    }
    let report_date = current_rows[0].report_date;
    let ids: BTreeSet<&str> = current_by_id.keys().chain(prior_by_id.keys()).copied().collect();
    let entity = render(entity_ref);
    let mut findings = Vec::new();
    for account_id in ids {
        let current_row = current_by_id.get(account_id).copied();
        let prior_row = prior_by_id.get(account_id).copied();
        let row = current_row.or(prior_row).expect("the id came from one of the maps");
        if !in_section(&row.section) {
            continue;
        }
        let current_net = current_row.map_or_else(Decimal::zero, BalanceRow::ytd_net);
        let prior_net = prior_row.map_or_else(Decimal::zero, BalanceRow::ytd_net);
        let delta = current_net.sub(&prior_net).value;
        let percent = if prior_row.is_none() || prior_net.is_zero() {
            None
        } else {
            Some(delta.div(&prior_net).abs())
        };
        if delta.is_zero() || delta.abs() < absolute || percent.as_ref().is_some_and(|p| *p < minimum_percent) {
            continue;
        }
        let reason = match (current_row, prior_row) {
            (None, _) => {
                "Account exists only in the prior period; its balance movement exceeds the approved variance thresholds."
            }
            (_, None) => {
                "Account is new in the current period; its balance movement exceeds the approved variance thresholds."
            }
            _ => "Movement exceeds the approved variance thresholds.",
        };
        let account_ref = format!(
            "acct:{}",
            &sha256_hex(format!("{entity}:{account_id}").as_bytes())[..24]
        );
        let finding_id = format!(
            "finding:{}",
            &sha256_hex(format!("{account_ref}:{delta}:{}", report_date.iso()).as_bytes())[..24]
        );
        let evidence_ref = format!(
            "evidence:{}",
            &sha256_hex(format!("{finding_id}:reviewer-evidence").as_bytes())[..24]
        );
        let percent_change = match &percent {
            None => Json::Null,
            Some(p) => Json::str(&percent_string(p)?),
        };
        let model_item = Json::object([
            ("finding_id", Json::str(&finding_id)),
            ("evidence_ref", Json::str(&evidence_ref)),
            ("account_ref", Json::str(&account_ref)),
            ("section", Json::str(&row.section)),
            ("current_ytd_net", Json::str(&decimal_string(&current_net)?)),
            ("prior_ytd_net", Json::str(&decimal_string(&prior_net)?)),
            ("delta", Json::str(&decimal_string(&delta)?)),
            ("percent_change", percent_change),
            ("review_reason", Json::str(reason)),
        ]);
        let source_refs: Vec<Json> = [
            ("source:current", current_row.is_some()),
            ("source:prior", prior_row.is_some()),
        ]
        .into_iter()
        .filter(|&(_, present)| present)
        .map(|(name, _)| Json::str(name))
        .collect();
        let evidence_item = Json::object([
            ("finding_id", Json::str(&finding_id)),
            ("account_id", Json::str(account_id)),
            ("account_code", Json::str(&row.account_code)),
            ("account_name", Json::str(&row.account_name)),
            ("current_values", row_values(current_row)?),
            ("prior_values", row_values(prior_row)?),
            ("source_refs", Json::Array(source_refs)),
        ]);
        findings.push((model_item, evidence_item));
    }
    Ok(findings)
}

pub struct Evaluation {
    pub model: Json,
    pub evidence: Json,
    pub receipt: Json,
}

/// Resolve the context, request and policy paths against the bundled data.
fn resolve_bundled(env: &Env, path: &Path, subdir: &str, label: &str) -> Result<Location> {
    env.within_bundle(&PyStr::from_os_text(&crate::pypath::text(path)), subdir, label)
}

/// The variance calculation `evaluate` runs; tests substitute their own.
pub(crate) type Finder<'a> =
    dyn Fn(&[BalanceRow], &[BalanceRow], &PyStr, &Json, &Json) -> Result<Vec<(Json, Json)>> + 'a;

/// Evaluate the one fixed, read-only synthetic operation without network or ledger access.
pub fn evaluate(env: &Env, context_path: &Path, request_path: &Path, policy_path: &Path) -> Result<Evaluation> {
    evaluate_with(env, context_path, request_path, policy_path, &variance_findings)
}

pub(crate) fn evaluate_with(
    env: &Env,
    context_path: &Path,
    request_path: &Path,
    policy_path: &Path,
    finder: &Finder,
) -> Result<Evaluation> {
    let context_path = resolve_bundled(env, context_path, "samples", "context")?;
    let request_path = resolve_bundled(env, request_path, "samples", "request")?;
    let policy_path = resolve_bundled(env, policy_path, "policy", "policy")?;
    let (policy, policy_snapshot) = load_policy_snapshot(env, &policy_path)?;
    let (request, request_snapshot) = load_request_snapshot(env, &request_path, &policy)?;
    let (current, prior, context_snapshot) = load_context_snapshot(env, &context_path)?;
    let operation = policy.at("operations").at("trial_balance_variance");
    let entity_ref = current
        .manifest
        .at("entity_ref")
        .as_pystr()
        .expect("validated by non_empty");
    let all_findings = finder(&current.rows, &prior.rows, entity_ref, request.at("section"), operation)?;
    let max_results = operation
        .at("max_results")
        .as_plain_int()
        .and_then(num_traits::ToPrimitive::to_usize)
        .expect("1 to 100");
    let total_findings = all_findings.len();
    let truncated = total_findings > max_results;
    let findings = &all_findings[..total_findings.min(max_results)];
    let run_seed = Json::object([
        ("context_sha256", Json::str(&context_snapshot.sha256)),
        ("request_sha256", Json::str(&request_snapshot.sha256)),
        ("policy_sha256", Json::str(&policy_snapshot.sha256)),
        ("current_csv_sha256", Json::str(&current.csv_snapshot.sha256)),
        ("prior_csv_sha256", Json::str(&prior.csv_snapshot.sha256)),
        ("current_manifest_sha256", Json::str(&current.manifest_snapshot.sha256)),
        ("prior_manifest_sha256", Json::str(&prior.manifest_snapshot.sha256)),
    ]);
    let run_id = format!("sha256:{}", sha256_hex(&canonical_json(&run_seed)));
    let model = Json::object([
        ("schema_version", Json::str("xero-model-review-result.v1")),
        ("run_id", Json::str(&run_id)),
        ("mode", Json::str("synthetic")),
        ("status", Json::str("REVIEW_READY")),
        ("operation", request.at("operation").clone()),
        ("currency", current.manifest.at("report").at("currency").clone()),
        ("sign_convention", Json::str(SIGN_CONVENTION)),
        ("findings", Json::Array(findings.iter().map(|f| f.0.clone()).collect())),
        ("total_findings", Json::int(total_findings as i64)),
        ("truncated", Json::Bool(truncated)),
        (
            "limitations",
            Json::Array(vec![
                Json::str("This is a synthetic-data review result."),
                Json::str("No journal, payment, filing, or period-locking action is available."),
                Json::str("A human reviewer must assess each finding against source evidence."),
            ]),
        ),
    ]);
    let evidence = Json::object([
        ("schema_version", Json::str("xero-reviewer-evidence.v1")),
        ("run_id", Json::str(&run_id)),
        ("mode", Json::str("synthetic")),
        ("items", Json::Array(findings.iter().map(|f| f.1.clone()).collect())),
        ("total_findings", Json::int(total_findings as i64)),
        ("truncated", Json::Bool(truncated)),
    ]);
    let digest = |value: &Json| Json::str(&format!("sha256:{}", sha256_hex(&canonical_json(value))));
    let seed = |key: &str| {
        Json::str(&format!(
            "sha256:{}",
            run_seed.at(key).as_pystr().unwrap().to_utf8().unwrap()
        ))
    };
    let receipt = Json::object([
        ("schema_version", Json::str("xero-review-receipt.v1")),
        ("run_id", Json::str(&run_id)),
        ("mode", Json::str("synthetic")),
        ("policy_sha256", seed("policy_sha256")),
        ("request_sha256", seed("request_sha256")),
        (
            "source_digests",
            Json::object([
                ("current", seed("current_csv_sha256")),
                ("prior", seed("prior_csv_sha256")),
                ("current_manifest", seed("current_manifest_sha256")),
                ("prior_manifest", seed("prior_manifest_sha256")),
            ]),
        ),
        ("result_sha256", digest(&model)),
        ("evidence_sha256", digest(&evidence)),
        ("code_version", Json::str(VERSION)),
    ]);
    let all_rows: Vec<&BalanceRow> = current.rows.iter().chain(&prior.rows).collect();
    assert_model_is_redacted(&model, &all_rows)?;
    Ok(Evaluation {
        model,
        evidence,
        receipt,
    })
}

fn leaf_strings<'a>(value: &'a Json, out: &mut Vec<&'a PyStr>) {
    match value {
        Json::Str(s) => out.push(s),
        Json::Object(pairs) => pairs.iter().for_each(|(_, v)| leaf_strings(v, out)),
        Json::Array(items) => items.iter().for_each(|v| leaf_strings(v, out)),
        _ => {}
    }
}

fn amount_shape(text: &str) -> bool {
    let body = text.strip_prefix('-').unwrap_or(text);
    body.split_once('.').is_some_and(|(int, frac)| {
        !int.is_empty()
            && int.bytes().all(|b| b.is_ascii_digit())
            && frac.len() >= 2
            && frac.bytes().all(|b| b.is_ascii_digit())
    })
}

fn percent_shape(text: &str) -> bool {
    text.split_once('.').is_some_and(|(int, frac)| {
        !int.is_empty()
            && int.bytes().all(|b| b.is_ascii_digit())
            && frac.len() == 4
            && frac.bytes().all(|b| b.is_ascii_digit())
    })
}

pub(crate) fn assert_model_is_redacted(model: &Json, rows: &[&BalanceRow]) -> Result<()> {
    let disclosure = "Internal disclosure assertion failed: model result contains raw source display data.";
    let mut forbidden: BTreeSet<&str> = BTreeSet::new();
    for row in rows {
        forbidden.insert(&row.tenant);
        forbidden.insert(&row.account_name);
        if !row.account_code.is_empty() {
            forbidden.insert(&row.account_code);
        }
        forbidden.insert(&row.account_id);
    }
    let Json::Object(pairs) = model else {
        unreachable!("the model is an object")
    };
    let mut scanned = Vec::new();
    for (key, value) in pairs {
        if !key.is("findings") {
            leaf_strings(value, &mut scanned);
        }
    }
    if let Some(Json::Array(findings)) = model.get("findings") {
        for finding in findings {
            let Json::Object(items) = finding else { continue };
            for (key, value) in items {
                let shape: Option<fn(&str) -> bool> = match key.to_utf8().as_deref() {
                    Some("current_ytd_net" | "prior_ytd_net" | "delta") => Some(amount_shape),
                    Some("percent_change") => Some(percent_shape),
                    _ => None,
                };
                match shape {
                    None if key.is("section") => {}
                    None => leaf_strings(value, &mut scanned),
                    Some(check) => {
                        let fits = match value {
                            Json::Null => true,
                            Json::Str(s) => s.to_utf8().is_some_and(|t| check(&t)),
                            _ => false,
                        };
                        if !fits {
                            return refuse(disclosure);
                        }
                    }
                }
            }
        }
    }
    if scanned
        .iter()
        .any(|leaf| leaf.to_utf8().is_some_and(|t| forbidden.contains(t.as_str())))
    {
        return refuse(disclosure);
    }
    let serialised = pyjson::canonical(model);
    if ["\"account_code\"", "\"account_name\"", "\"tenant\""]
        .iter()
        .any(|key| serialised.contains(key))
    {
        return refuse("Internal disclosure assertion failed: model result contains a prohibited source field.");
    }
    Ok(())
}

pub struct ReviewInputs {
    pub evidence: PathBuf,
    pub receipt: PathBuf,
    pub decision: Location,
    pub model: PathBuf,
}

/// Human decisions may sit with the bundled samples or beside run outputs under CWD build/.
fn resolve_decision(env: &Env, path: &Path) -> Result<Location> {
    let raw = crate::pypath::text(path);
    if let Ok(found) = env.within_bundle(&PyStr::from_os_text(&raw), "samples", "human decision") {
        return Ok(found);
    }
    if let Ok(found) = env.within_build(path, "human decision", true) {
        return Ok(Location::Disk(found));
    }
    refuse(format!(
        "human decision must exist under the bundled samples/ data or the working directory's build/: {}.",
        crate::pypath::display(&raw)
    ))
}

/// Resolve the 4 files validate_review reads: evidence, receipt, decision and the model result beside the receipt.
pub fn review_input_paths(env: &Env, evidence: &Path, receipt: &Path, decision: &Path) -> Result<ReviewInputs> {
    let evidence = env.within_build(evidence, "reviewer evidence", true)?;
    let receipt = env.within_build(receipt, "receipt", true)?;
    let decision = resolve_decision(env, decision)?;
    let model = receipt.with_file_name("model-result.json");
    Ok(ReviewInputs {
        evidence,
        receipt,
        decision,
        model,
    })
}

pub fn validate_review(env: &Env, evidence_path: &Path, receipt_path: &Path, decision_path: &Path) -> Result<Json> {
    let inputs = review_input_paths(env, evidence_path, receipt_path, decision_path)?;
    let evidence = load_json_exact(
        env,
        &Location::Disk(inputs.evidence.clone()),
        &[
            "schema_version",
            "run_id",
            "mode",
            "items",
            "total_findings",
            "truncated",
        ],
        "reviewer evidence",
    )?;
    let receipt = load_json_exact(
        env,
        &Location::Disk(inputs.receipt.clone()),
        &[
            "schema_version",
            "run_id",
            "mode",
            "policy_sha256",
            "request_sha256",
            "source_digests",
            "result_sha256",
            "evidence_sha256",
            "code_version",
        ],
        "receipt",
    )?;
    let decision = load_json_exact(
        env,
        &inputs.decision,
        &["schema_version", "run_id", "reviewer_ref", "reviewed_at", "decisions"],
        "human decision",
    )?;
    if !evidence.at("schema_version").is_str("xero-reviewer-evidence.v1")
        || !receipt.at("schema_version").is_str("xero-review-receipt.v1")
        || !decision.at("schema_version").is_str("xero-human-review-decision.v1")
    {
        return refuse("A supplied review artefact has an unsupported schema version.");
    }
    if !evidence.at("mode").is_str("synthetic") || !receipt.at("mode").is_str("synthetic") {
        return refuse("Only synthetic review artefacts are supported.");
    }
    // Python chains ==: evidence == receipt and receipt == decision.
    if !(pyjson::py_eq(evidence.at("run_id"), receipt.at("run_id"), false)
        && pyjson::py_eq(receipt.at("run_id"), decision.at("run_id"), false))
    {
        return refuse("Decision, evidence, and receipt must refer to the same run_id.");
    }
    let sealed = |value: &Json, key: &str| {
        pyjson::py_eq(
            &Json::str(&format!("sha256:{}", sha256_hex(&canonical_json(value)))),
            receipt.at(key),
            false,
        )
    };
    if !sealed(&evidence, "evidence_sha256") {
        return refuse("Reviewer evidence does not match the receipt's evidence digest.");
    }
    if inputs.model.is_file() {
        let model = load_json_object(env, &Location::Disk(inputs.model.clone()), "model result")?;
        if !sealed(&model, "result_sha256") {
            return refuse("Model result beside the receipt does not match the receipt's result digest.");
        }
    }
    let evidence_fields = [
        "finding_id",
        "account_id",
        "account_code",
        "account_name",
        "current_values",
        "prior_values",
        "source_refs",
    ];
    let Json::Array(items) = evidence.at("items") else {
        return refuse("Reviewer evidence items must be a list of objects with the exact evidence shape.");
    };
    if !items.iter().all(|item| item.has_exact_keys(&evidence_fields)) {
        return refuse("Reviewer evidence items must be a list of objects with the exact evidence shape.");
    }
    for item in items {
        for field in ["account_id", "account_name"] {
            non_empty(item.at(field), &format!("evidence {field}"))?;
        }
        optional_text(item.at("account_code"), "evidence account_code")?;
        let mut expected_refs = Vec::new();
        for period in ["current", "prior"] {
            let values = item.at(&format!("{period}_values"));
            if matches!(values, Json::Null) {
                continue;
            }
            if !values.has_exact_keys(&["debit", "credit", "ytd_debit", "ytd_credit"]) {
                return refuse(format!(
                    "evidence {period}_values must contain the four monetary fields or be null."
                ));
            }
            let Json::Object(pairs) = values else { unreachable!() };
            for (field, value) in pairs {
                decimal(value, &format!("evidence {period}_values.{}", render(field)))?;
            }
            expected_refs.push(Json::str(&format!("source:{period}")));
        }
        if expected_refs.is_empty() || !pyjson::py_eq(item.at("source_refs"), &Json::Array(expected_refs), false) {
            return refuse("evidence source_refs must identify exactly the supplied current and prior values.");
        }
    }
    let mut finding_ids = Vec::new();
    for item in items {
        finding_ids.push(non_empty(item.at("finding_id"), "evidence finding_id")?);
    }
    let known: BTreeSet<&str> = finding_ids.iter().map(String::as_str).collect();
    if known.len() != finding_ids.len() {
        return refuse("Reviewer evidence contains duplicate finding IDs.");
    }
    let total_findings = match evidence.at("total_findings").as_plain_int() {
        Some(n) if *n >= num_bigint::BigInt::from(items.len()) => n.clone(),
        _ => {
            return refuse(
                "Reviewer evidence total_findings must be an integer at least as large as the visible item count.",
            );
        }
    };
    let truncated = match evidence.at("truncated") {
        Json::Bool(t) if *t == (total_findings > num_bigint::BigInt::from(items.len())) => *t,
        _ => return refuse("Reviewer evidence truncated must exactly describe whether findings were omitted."),
    };
    non_empty(decision.at("reviewer_ref"), "human decision reviewer_ref")?;
    iso_timestamp(decision.at("reviewed_at"), "human decision reviewed_at")?;
    let Json::Array(decisions) = decision.at("decisions") else {
        return refuse("Human decision decisions must be a list.");
    };
    if decisions.is_empty() && total_findings > num_bigint::BigInt::from(0) {
        return refuse("Human decision must contain at least one decision when the evidence carries findings.");
    }
    let mut decided: BTreeSet<String> = BTreeSet::new();
    for item in decisions {
        if !item.has_exact_keys(&["finding_id", "decision", "rationale"]) {
            return refuse("Each human decision must contain exactly finding_id, decision, and rationale.");
        }
        let finding_id = non_empty(item.at("finding_id"), "human decision finding_id")?;
        if !known.contains(finding_id.as_str()) || decided.contains(&finding_id) {
            return refuse("Human decision refers to an unknown or duplicate finding.");
        }
        let state_ok = ALLOWED_DECISIONS
            .iter()
            .any(|allowed| item.at("decision").is_str(allowed));
        let rationale_ok = item
            .at("rationale")
            .as_pystr()
            .is_some_and(|r| !strip_cps(&r.0).is_empty());
        if !state_ok || !rationale_ok {
            return refuse("Human decision state or rationale is invalid.");
        }
        non_empty(item.at("rationale"), "human decision rationale")?;
        decided.insert(finding_id);
    }
    let undecided = total_findings - num_bigint::BigInt::from(decided.len());
    let status = if undecided == num_bigint::BigInt::from(0) {
        "DECISION_RECORDED"
    } else {
        "PARTIAL_DECISION_RECORDED"
    };
    Ok(Json::object([
        ("schema_version", Json::str("xero-review-decision-validation.v1")),
        ("run_id", receipt.at("run_id").clone()),
        ("status", Json::str(status)),
        ("decision_count", Json::int(decided.len() as i64)),
        ("undecided_count", Json::Int(undecided)),
        ("visible_findings", Json::int(items.len() as i64)),
        ("truncated", Json::Bool(truncated)),
        ("completable", Json::Bool(!truncated)),
        (
            "limitation",
            Json::str(
                "Validation records a structurally valid human decision; it does not approve, resolve, post, pay, lodge, or lock anything.",
            ),
        ),
    ]))
}

impl From<Failure> for String {
    fn from(failure: Failure) -> String {
        match failure {
            Failure::Gateway(message) | Failure::Crash(message) => message,
        }
    }
}
