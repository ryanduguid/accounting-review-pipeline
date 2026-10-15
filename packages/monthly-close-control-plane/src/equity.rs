//! Reconcile independent movement records against selected ledger equity balances.
use crate::{
    Result,
    decimal::{self, Decimal, Outcome},
    isodate::{self, Date},
    loader::SourceSnapshot,
    models::{ExceptionItem, Status, TrialBalanceRow},
    pyjson::{self, Json, LoadError, PyStr},
    pytext,
};
use std::collections::{BTreeMap, BTreeSet};

const FIELDS: &[&str] = &[
    "schema_version",
    "tenant",
    "opening_date",
    "closing_date",
    "currency",
    "basis",
    "prior_source_sha256",
    "current_source_sha256",
    "equity_account_ids",
    "complete",
    "movements",
];
const MOVEMENT_FIELDS: &[&str] = &[
    "movement_id",
    "date",
    "account_id",
    "debit",
    "credit",
    "description",
    "evidence_reference",
];
const RESULT_FIELDS: &[&str] = &[
    "schema_version",
    "basis",
    "tenant",
    "opening_date",
    "closing_date",
    "currency",
    "currency_evidence",
    "schedule_sha256",
    "status",
    "issues",
    "accounts",
    "total",
    "movements",
    "tolerance",
    "complete",
];
const NUMBERS: [&str; 5] = [
    "opening",
    "movement",
    "expected_close",
    "actual_close",
    "unexplained",
];
const ACCOUNT_FIELDS: &[&str] = &[
    "account_id",
    "movement_ids",
    "opening",
    "movement",
    "expected_close",
    "actual_close",
    "unexplained",
];

fn string(value: &Json) -> String {
    let value = value.as_pystr().expect("validated string");
    value.to_utf8().unwrap_or_else(|| value.os_text())
}
fn js(value: &str) -> Json {
    Json::str(value)
}
fn text(value: &Json, label: &str) -> Result<String> {
    let error = || format!("Equity {label} must be non-empty text without control characters.");
    let raw = value.as_pystr().ok_or_else(error)?;
    let value = raw.to_utf8().unwrap_or_else(|| raw.os_text());
    if pytext::strip(&value).is_empty() || raw.0.iter().any(|&cp| cp < 32) {
        return Err(error());
    }
    Ok(value)
}
fn date(value: &Json, label: &str) -> Result<Date> {
    let raw = text(value, label)?;
    let date = isodate::python_date_fromisoformat(&raw)
        .ok_or_else(|| format!("Equity {label} must be an ISO date."))?;
    if date.iso() != raw {
        return Err(format!("Equity {label} must use YYYY-MM-DD."));
    }
    Ok(date)
}
fn amount(value: &Json, label: &str) -> Result<Decimal> {
    let error = || format!("Equity {label} must be a non-negative decimal string.");
    let Some(raw) = value.as_pystr().and_then(PyStr::to_utf8) else {
        return Err(error());
    };
    let mut parts = raw.split('.');
    let whole = parts.next().unwrap();
    let fraction = parts.next();
    if raw.len() > 64
        || whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || fraction.is_some_and(|s| s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()))
        || parts.next().is_some()
    {
        return Err(error());
    }
    Decimal::parse(&raw).map_err(|_| error())
}
fn signed_amount(value: &Json, label: &str) -> Result<Decimal> {
    let raw = string(value);
    if let Some(positive) = raw.strip_prefix('-') {
        let number = amount(&Json::str(positive), label)?;
        // Unary minus applies the active context, as Decimal.__neg__ does.
        exact(Decimal::zero().sub(&number))
    } else {
        amount(value, label)
    }
}
fn currency(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|c| c.is_ascii_uppercase())
}
fn digest(value: &Json) -> bool {
    value.as_pystr().and_then(PyStr::to_utf8).is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}
fn fixed(value: &Decimal) -> String {
    value.to_fixed_string().expect("bounded equity decimal")
}
fn exact(value: Outcome) -> Result<Decimal> {
    if value.inexact || !value.value.is_finite() {
        Err("Equity arithmetic exceeds supported exact decimal precision.".into())
    } else {
        Ok(value.value)
    }
}
fn sum(values: impl IntoIterator<Item = Decimal>) -> Result<Decimal> {
    values
        .into_iter()
        .try_fold(Decimal::zero(), |a, b| exact(a.add(&b)))
}

pub fn load_schedule(snapshot: &SourceSnapshot) -> Result<Json> {
    let raw = snapshot.text("Equity schedule", true)?;
    let document = pyjson::loads_detailed(raw).map_err(|(error, diagnostic)| match error {
        LoadError::DuplicateKey(key) => format!(
            "Invalid equity schedule: Duplicate equity JSON field: {}.",
            key.os_text()
        ),
        LoadError::Invalid => format!(
            "Invalid equity schedule: {}",
            diagnostic.expect("syntax failure has a location")
        ),
        LoadError::Crash(reason) => format!("Invalid equity schedule: {reason}"),
    })?;
    if !document.has_exact_keys(FIELDS) {
        return Err("Equity schedule must contain exactly the documented fields.".into());
    }
    if document.at("schema_version").as_plain_int() != Json::int(1).as_plain_int() {
        return Err("Equity schema_version must be integer 1.".into());
    }
    if !document.at("basis").is_str("ledger_equity") || document.at("complete").as_bool().is_none()
    {
        return Err("Equity basis must be ledger_equity and complete must be boolean.".into());
    }
    text(document.at("tenant"), "tenant")?;
    if !document
        .at("currency")
        .as_pystr()
        .is_some_and(|s| currency(&s.os_text()))
    {
        return Err("Equity currency must be an uppercase 3-letter code.".into());
    }
    let opening = date(document.at("opening_date"), "opening_date")?;
    let closing = date(document.at("closing_date"), "closing_date")?;
    if closing <= opening {
        return Err("Equity closing_date must follow opening_date.".into());
    }
    for key in ["prior_source_sha256", "current_source_sha256"] {
        if !digest(document.at(key)) {
            return Err(format!("Equity {key} must be a lowercase SHA-256 digest."));
        }
    }
    let accounts = document
        .at("equity_account_ids")
        .as_array()
        .filter(|s| !s.is_empty())
        .ok_or("Equity equity_account_ids must be a non-empty list.")?;
    let account_ids = accounts
        .iter()
        .map(|v| text(v, "account ID"))
        .collect::<Result<Vec<_>>>()?;
    if account_ids.iter().collect::<BTreeSet<_>>().len() != account_ids.len() {
        return Err("Equity account IDs must be unique.".into());
    }
    let movements = document
        .at("movements")
        .as_array()
        .ok_or("Equity movements must be a list.")?;
    let mut seen = BTreeSet::new();
    for row in movements {
        if !row.has_exact_keys(MOVEMENT_FIELDS) {
            return Err("Equity movement must contain exactly the documented fields.".into());
        }
        for key in [
            "movement_id",
            "account_id",
            "description",
            "evidence_reference",
        ] {
            text(row.at(key), key)?;
        }
        if !seen.insert(string(row.at("movement_id"))) {
            return Err("Equity movement IDs must be unique.".into());
        }
        if !account_ids.contains(&string(row.at("account_id"))) {
            return Err("Equity movement account is outside the selected accounts.".into());
        }
        let when = date(row.at("date"), "movement date")?;
        if !(opening < when && when <= closing) {
            return Err("Equity movement date is outside the declared interval.".into());
        }
        let debit = amount(row.at("debit"), "debit")?;
        let credit = amount(row.at("credit"), "credit")?;
        if (debit > Decimal::zero()) == (credit > Decimal::zero()) {
            return Err("Equity movement must have exactly one positive side.".into());
        }
    }
    Ok(document)
}

fn finding(
    status: Status,
    reason: &str,
    row: Option<&TrialBalanceRow>,
    expected: Option<Decimal>,
    actual: Option<Decimal>,
    difference: Option<Decimal>,
    tolerance: Option<Decimal>,
) -> ExceptionItem {
    let mut item = ExceptionItem::new(
        "equity_reconciliation",
        status,
        row,
        reason,
        "Reconcile the independent movement records and retained source evidence.",
    );
    item.prior_value = expected;
    item.current_value = actual;
    item.difference = difference;
    item.threshold = tolerance;
    item
}

pub struct Inputs<'a> {
    pub prior: &'a [TrialBalanceRow],
    pub current: &'a [TrialBalanceRow],
    pub prior_sha256: &'a str,
    pub current_sha256: &'a str,
    pub currency: Option<&'a str>,
    pub currency_evidence: Option<&'a str>,
    pub tolerance: &'a Decimal,
}

pub fn reconcile(
    snapshot: &SourceSnapshot,
    inputs: Inputs<'_>,
) -> Result<(Json, Vec<ExceptionItem>)> {
    let document = load_schedule(snapshot)?;
    let mut issues = Vec::new();
    for (key, expected) in [
        ("tenant", inputs.current[0].tenant.clone()),
        ("opening_date", inputs.prior[0].report_date.iso()),
        ("closing_date", inputs.current[0].report_date.iso()),
        ("prior_source_sha256", inputs.prior_sha256.into()),
        ("current_source_sha256", inputs.current_sha256.into()),
    ] {
        if !document.at(key).is_str(&expected) {
            issues.push(format!(
                "Schedule {key} does not match the supplied trial-balance snapshots."
            ));
        }
    }
    if document.at("complete").as_bool() == Some(false) {
        issues.push("The preparer has not declared the selected schedule complete.".into());
    }
    if !inputs
        .currency
        .is_some_and(|s| currency(s) && document.at("currency").is_str(s))
    {
        issues.push(
            "Independent currency confirmation is missing or differs from the schedule.".into(),
        );
    }
    if inputs
        .currency_evidence
        .is_none_or(|s| pytext::strip(s).is_empty())
    {
        issues.push("Independent trial-balance currency evidence is missing.".into());
    } else if inputs
        .currency_evidence
        .unwrap()
        .chars()
        .any(|c| (c as u32) < 32)
    {
        return Err("Currency evidence must not contain control characters.".into());
    }
    let old: BTreeMap<_, _> = inputs
        .prior
        .iter()
        .map(|r| (r.account_id.as_str(), r))
        .collect();
    let new: BTreeMap<_, _> = inputs
        .current
        .iter()
        .map(|r| (r.account_id.as_str(), r))
        .collect();
    let mut selected: Vec<_> = document
        .at("equity_account_ids")
        .as_array()
        .unwrap()
        .iter()
        .map(|value| (string(value), value.as_pystr().unwrap().clone()))
        .collect();
    for (key, points) in &selected {
        match (old.get(key.as_str()), new.get(key.as_str())) {
            (Some(old), Some(new)) => {
                if old.section != new.section || pytext::casefold(&new.section) != "equity" {
                    issues.push(format!(
                        "Account {} must retain an explicit Equity section in both snapshots.",
                        pytext::repr_code_points(&points.0)
                    ));
                }
            }
            _ => issues.push(format!(
                "Account {} must be present in both trial balances.",
                pytext::repr_code_points(&points.0)
            )),
        }
    }
    let mut result = Json::object([
        ("schema_version", Json::int(1)),
        ("basis", Json::str("ledger_equity")),
        ("tenant", document.at("tenant").clone()),
        ("opening_date", document.at("opening_date").clone()),
        ("closing_date", document.at("closing_date").clone()),
        ("currency", js(inputs.currency.unwrap_or(""))),
        (
            "currency_evidence",
            js(inputs.currency_evidence.unwrap_or("")),
        ),
        ("schedule_sha256", Json::str(&snapshot.sha256)),
        ("complete", document.at("complete").clone()),
        (
            "status",
            Json::str(if issues.is_empty() { "PASS" } else { "BLOCKED" }),
        ),
        (
            "issues",
            Json::Array(issues.iter().map(|s| js(s)).collect()),
        ),
        ("accounts", Json::Array(Vec::new())),
        ("total", Json::Null),
        ("movements", document.at("movements").clone()),
        ("tolerance", Json::str(&fixed(inputs.tolerance))),
    ]);
    if !issues.is_empty() {
        return Ok((
            result,
            vec![finding(
                Status::Blocked,
                &issues.join(" "),
                None,
                None,
                None,
                None,
                None,
            )],
        ));
    }
    decimal::with_precision(28, || {
        selected.sort_by(|a, b| a.1.cmp(&b.1));
        let movements = document.at("movements").as_array().unwrap();
        let mut accounts = Vec::new();
        let mut findings = Vec::new();
        for (key, points) in selected {
            let old = old[key.as_str()];
            let new = new[key.as_str()];
            let opening = exact(old.ytd_credit.sub(&old.ytd_debit))?;
            let linked: Vec<_> = movements
                .iter()
                .filter(|r| string(r.at("account_id")) == key)
                .collect();
            let movement = sum(linked
                .iter()
                .map(|r| {
                    exact(amount(r.at("credit"), "credit")?.sub(&amount(r.at("debit"), "debit")?))
                })
                .collect::<Result<Vec<_>>>()?)?;
            let expected = exact(opening.add(&movement))?;
            let actual = exact(new.ytd_credit.sub(&new.ytd_debit))?;
            let difference = exact(actual.sub(&expected))?;
            accounts.push(Json::object([
                ("account_id", Json::Str(points)),
                ("opening", Json::str(&fixed(&opening))),
                ("movement", Json::str(&fixed(&movement))),
                ("expected_close", Json::str(&fixed(&expected))),
                ("actual_close", Json::str(&fixed(&actual))),
                ("unexplained", Json::str(&fixed(&difference))),
                (
                    "movement_ids",
                    Json::Array(linked.iter().map(|r| r.at("movement_id").clone()).collect()),
                ),
            ]));
            if difference.copy_abs() > *inputs.tolerance {
                findings.push(finding(Status::Review,&format!("Ledger equity differs from its independent movement schedule by {}; tolerance {}.",difference.to_sci_string(),inputs.tolerance.to_sci_string()),Some(new),Some(expected),Some(actual),Some(difference),Some(inputs.tolerance.clone())));
            }
        }
        let mut totals = Vec::new();
        for key in NUMBERS {
            totals.push((
                PyStr::from(key),
                Json::str(&fixed(&sum(accounts
                    .iter()
                    .map(|r| Decimal::parse(&string(r.at(key))).unwrap()))?)),
            ));
        }
        let total = Json::Object(totals);
        let unexplained = Decimal::parse(&string(total.at("unexplained"))).unwrap();
        if unexplained.copy_abs() > *inputs.tolerance && findings.is_empty() {
            findings.push(finding(
                Status::Review,
                "Aggregate equity difference exceeds the reconciliation tolerance.",
                None,
                Some(Decimal::parse(&string(total.at("expected_close"))).unwrap()),
                Some(Decimal::parse(&string(total.at("actual_close"))).unwrap()),
                Some(unexplained),
                Some(inputs.tolerance.clone()),
            ));
        }
        result.set("accounts", Json::Array(accounts));
        result.set("total", total);
        result.set(
            "status",
            Json::str(if findings.is_empty() {
                "PASS"
            } else {
                "REVIEW"
            }),
        );
        Ok((result, findings))
    })
}

pub fn result_digest(result: &Json) -> String {
    crate::sha256(pyjson::canonical(result).as_bytes())
}
fn cell(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('`', "\\`")
        .replace('*', "\\*")
}
pub fn summary_lines(result: &Json) -> Vec<String> {
    // Keep Python code points until the complete Markdown is encoded as UTF-8.
    let encoded = |value: &Json| value.as_pystr().unwrap().os_text();
    let currency = encoded(result.at("currency"));
    let evidence = encoded(result.at("currency_evidence"));
    let mut lines = vec![
        format!(
            "- Result: {}; currency: {}.",
            string(result.at("status")),
            cell(if currency.is_empty() {
                "unconfirmed"
            } else {
                &currency
            })
        ),
        format!(
            "- Currency evidence: {}.",
            cell(if evidence.is_empty() {
                "missing"
            } else {
                &evidence
            })
        ),
        format!("- Result digest: {}", result_digest(result)),
        String::new(),
        "| Account | Opening | Movement | Expected close | Actual close | Unexplained |".into(),
        "| --- | ---: | ---: | ---: | ---: | ---: |".into(),
    ];
    let cells = |row: &Json, fields: &[&str]| {
        fields
            .iter()
            .map(|k| cell(&encoded(row.at(k))))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    for row in result.at("accounts").as_array().unwrap() {
        lines.push(format!(
            "| {} |",
            cells(
                row,
                &[
                    "account_id",
                    "opening",
                    "movement",
                    "expected_close",
                    "actual_close",
                    "unexplained"
                ]
            )
        ));
    }
    if !matches!(result.at("total"), Json::Null) {
        lines.push(format!(
            "| Total | {} |",
            cells(result.at("total"), &NUMBERS)
        ));
    }
    lines.extend([
        String::new(),
        "| Movement | Account | Date | Debit | Credit | Description | Evidence |".into(),
        "| --- | --- | --- | ---: | ---: | --- | --- |".into(),
    ]);
    for row in result.at("movements").as_array().unwrap() {
        lines.push(format!(
            "| {} |",
            cells(
                row,
                &[
                    "movement_id",
                    "account_id",
                    "date",
                    "debit",
                    "credit",
                    "description",
                    "evidence_reference"
                ]
            )
        ));
    }
    lines.extend(
        result
            .at("issues")
            .as_array()
            .unwrap()
            .iter()
            .map(|v| format!("- Issue: {}", cell(&encoded(v)))),
    );
    lines
}

/// Verify the saved evidence independently of its claimed status and amounts.
pub fn validate_result(value: &Json, sources: &Json, exceptions: &[Json]) -> Result<()> {
    let status = value.get("status");
    if !value.has_exact_keys(RESULT_FIELDS)
        || value.at("schema_version").as_plain_int() != Json::int(1).as_plain_int()
        || !value.at("basis").is_str("ledger_equity")
        || !status.is_some_and(|s| ["PASS", "REVIEW", "BLOCKED"].iter().any(|v| s.is_str(v)))
        || value.at("complete").as_bool().is_none()
        || !value
            .at("issues")
            .as_array()
            .is_some_and(|s| s.iter().all(|v| v.as_pystr().is_some()))
        || value.at("accounts").as_array().is_none()
        || value.at("movements").as_array().is_none()
    {
        return Err("Malformed equity reconciliation result.".into());
    }
    for key in [
        "tenant",
        "opening_date",
        "closing_date",
        "currency",
        "currency_evidence",
        "schedule_sha256",
        "tolerance",
    ] {
        if value.at(key).as_pystr().is_none() {
            return Err("Equity result fields must be strings.".into());
        }
    }
    if !sources
        .get("equity_schedule")
        .is_some_and(|s| pyjson::py_eq(s, value.at("schedule_sha256"), false))
    {
        return Err("Equity schedule provenance disagrees with the pack.".into());
    }
    let accounts = value.at("accounts").as_array().unwrap();
    for row in accounts {
        if !row.has_exact_keys(ACCOUNT_FIELDS) {
            return Err("Malformed equity account evidence.".into());
        }
        if row.at("account_id").as_pystr().is_none() || row.at("movement_ids").as_array().is_none()
        {
            return Err("Malformed equity account identity.".into());
        }
        if NUMBERS.iter().any(|key| row.at(key).as_pystr().is_none()) {
            return Err("Equity account amounts must be decimal strings.".into());
        }
    }
    let total = value.at("total");
    if !matches!(total, Json::Null)
        && (!total.has_exact_keys(&NUMBERS)
            || NUMBERS.iter().any(|k| total.at(k).as_pystr().is_none()))
    {
        return Err("Malformed equity total evidence.".into());
    }
    let movements = value.at("movements").as_array().unwrap();
    for row in movements {
        if !row.has_exact_keys(MOVEMENT_FIELDS) {
            return Err("Malformed equity movement evidence.".into());
        }
        if MOVEMENT_FIELDS
            .iter()
            .any(|k| row.at(k).as_pystr().is_none())
        {
            return Err("Equity movement fields must be strings.".into());
        }
    }
    let tolerance = amount(value.at("tolerance"), "tolerance")?;
    if value.at("status").is_str("BLOCKED") {
        if !accounts.is_empty()
            || !matches!(total, Json::Null)
            || value.at("issues").as_array().unwrap().is_empty()
        {
            return Err("Blocked equity evidence must omit calculated balances.".into());
        }
    } else {
        if accounts.is_empty()
            || matches!(total, Json::Null)
            || !value.at("issues").as_array().unwrap().is_empty()
            || value.at("complete").as_bool() != Some(true)
        {
            return Err("Calculated equity evidence requires complete accounts and totals.".into());
        }
        decimal::with_precision(28, || -> Result<()> {
            let mut totals = NUMBERS.map(|_| Decimal::zero());
            let mut account_ids = BTreeSet::new();
            let mut movement_ids = BTreeSet::new();
            let mut needs_review = false;
            for row in movements {
                if !movement_ids.insert(text(row.at("movement_id"), "movement ID")?) {
                    return Err("Duplicate equity movement evidence.".into());
                }
                let debit = amount(row.at("debit"), "debit")?;
                let credit = amount(row.at("credit"), "credit")?;
                if (debit > Decimal::zero()) == (credit > Decimal::zero()) {
                    return Err("Equity movement must have exactly one positive side.".into());
                }
            }
            for row in accounts {
                let identifier = text(row.at("account_id"), "account ID")?;
                if !account_ids.insert(identifier.clone()) {
                    return Err("Duplicate equity account evidence.".into());
                }
                let numbers: Vec<_> = NUMBERS
                    .iter()
                    .map(|k| signed_amount(row.at(k), k))
                    .collect::<Result<_>>()?;
                let linked: Vec<_> = movements
                    .iter()
                    .filter(|r| string(r.at("account_id")) == identifier)
                    .collect();
                let movement = sum(linked
                    .iter()
                    .map(|r| {
                        exact(
                            amount(r.at("credit"), "credit")?.sub(&amount(r.at("debit"), "debit")?),
                        )
                    })
                    .collect::<Result<Vec<_>>>()?)?;
                let ids = Json::Array(linked.iter().map(|r| r.at("movement_id").clone()).collect());
                if !pyjson::py_eq(row.at("movement_ids"), &ids, true)
                    || numbers[1] != movement
                    || numbers[2] != exact(numbers[0].add(&movement))?
                    || numbers[4] != exact(numbers[3].sub(&numbers[2]))?
                {
                    return Err("Equity account arithmetic or movement links disagree.".into());
                }
                needs_review |= numbers[4].copy_abs() > tolerance;
                for (target, number) in totals.iter_mut().zip(numbers) {
                    *target = exact(target.add(&number))?;
                }
            }
            if movements
                .iter()
                .any(|r| !account_ids.contains(&string(r.at("account_id"))))
            {
                return Err("Equity movement references an unknown account.".into());
            }
            for (key, expected) in NUMBERS.iter().zip(&totals) {
                if signed_amount(total.at(key), key)? != *expected {
                    return Err("Equity totals disagree with the account evidence.".into());
                }
            }
            needs_review |= totals[4].copy_abs() > tolerance;
            if !value
                .at("status")
                .is_str(if needs_review { "REVIEW" } else { "PASS" })
            {
                return Err("Equity status disagrees with its arithmetic.".into());
            }
            Ok(())
        })
        .map_err(|e| {
            if e == "Equity arithmetic exceeds supported exact decimal precision." {
                "Equity evidence exceeds supported exact decimal precision.".into()
            } else {
                e
            }
        })?;
    }
    let statuses: Vec<_> = exceptions
        .iter()
        .filter(|r| {
            r.get("control")
                .is_some_and(|s| s.is_str("equity_reconciliation"))
        })
        .filter_map(|r| r.get("status"))
        .collect();
    let expected = if statuses.iter().any(|s| s.is_str("BLOCKED")) {
        "BLOCKED"
    } else if statuses.iter().any(|s| s.is_str("REVIEW")) {
        "REVIEW"
    } else {
        "PASS"
    };
    if !value.at("status").is_str(expected) {
        return Err("Equity result status disagrees with its findings.".into());
    }
    Ok(())
}
