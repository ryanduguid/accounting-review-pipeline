//! Review explicitly mapped schedules without posting or approving accounting.
use crate::{
    Result,
    decimal::{self, Decimal, Outcome},
    isodate::{self, Date},
    loader::SourceSnapshot,
    pyjson::{self, Json, LoadError, PyStr},
    pytext,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

fn object<'a>(value: &'a Json, fields: &str, label: &str) -> Result<&'a Json> {
    if !value.has_exact_keys(&fields.split_whitespace().collect::<Vec<_>>()) {
        return Err(format!("{label}: expected exactly {fields}."));
    }
    Ok(value)
}
fn text(value: &Json, label: &str) -> Result<PyStr> {
    let error = || format!("{label}: expected non-blank text without outer spaces or controls.");
    let raw = value.as_pystr().ok_or_else(error)?;
    if raw.0.is_empty()
        || raw.0.iter().any(|&cp| cp < 32)
        || raw.0.first().is_some_and(|&cp| pytext::is_space(cp))
        || raw.0.last().is_some_and(|&cp| pytext::is_space(cp))
    {
        return Err(error());
    }
    Ok(raw.clone())
}
fn id(row: &Json, key: &str) -> PyStr {
    row.at(key).as_pystr().unwrap().clone()
}
fn money(value: &Json, label: &str) -> Result<Decimal> {
    let error = || format!("{label}: expected a signed decimal string with at most 2 places.");
    let raw = value.as_pystr().ok_or_else(error)?;
    let points = raw.0.strip_prefix(&[45]).unwrap_or(&raw.0);
    let mut parts = points.split(|&cp| cp == 46);
    let whole = parts.next().unwrap();
    let fraction = parts.next();
    if !(1..=16).contains(&whole.len())
        || whole.iter().any(|&cp| pytext::decimal_value(cp).is_none())
        || fraction.is_some_and(|s| {
            !(1..=2).contains(&s.len()) || s.iter().any(|&cp| pytext::decimal_value(cp).is_none())
        })
        || parts.next().is_some()
    {
        return Err(error());
    }
    Decimal::parse(&raw.to_utf8().unwrap()).map_err(|_| error())
}
fn date(value: &Json, label: &str) -> Result<Date> {
    value
        .as_pystr()
        .and_then(PyStr::to_utf8)
        .and_then(|s| isodate::date_fromisoformat(&s))
        .ok_or_else(|| format!("{label}: expected YYYY-MM-DD."))
}
fn rows<'a>(value: &'a Json, fields: &str, key: &str, label: &str) -> Result<&'a [Json]> {
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{label}: expected an array."))?;
    let mut seen = BTreeSet::new();
    for row in rows {
        object(row, fields, label)?;
        let identifier = text(row.at(key), &format!("{label}.{key}"))?;
        if !seen.insert(identifier.clone()) {
            return Err(format!(
                "{label}: duplicate {key} {}.",
                identifier.os_text()
            ));
        }
    }
    Ok(rows)
}
fn exact(outcome: Outcome) -> Result<Decimal> {
    if outcome.inexact {
        return Err("[<class 'decimal.Inexact'>]".into());
    }
    if !outcome.value.is_finite() {
        return Err("[<class 'decimal.InvalidOperation'>]".into());
    }
    Ok(outcome.value)
}
fn sum(values: impl IntoIterator<Item = Decimal>) -> Result<Decimal> {
    values
        .into_iter()
        .try_fold(Decimal::zero(), |a, b| exact(a.add(&b)))
}
fn add<K: Ord>(values: &mut BTreeMap<K, Decimal>, key: K, amount: &Decimal) -> Result<()> {
    let total = values.entry(key).or_insert_with(Decimal::zero);
    *total = exact(total.add(amount))?;
    Ok(())
}
fn number(value: &Decimal) -> Json {
    Json::str(&value.to_sci_string())
}
fn finding(code: &str, key: &PyStr) -> Json {
    Json::object([("code", Json::str(code)), ("id", Json::Str(key.clone()))])
}
fn joined(values: &[PyStr], separator: &str) -> PyStr {
    let mut points = Vec::new();
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            points.extend(separator.chars().map(u32::from));
        }
        points.extend(&value.0);
    }
    PyStr(points)
}

pub fn review_schedule(path: &Path, kind: &str) -> Result<Json> {
    review_snapshot(&SourceSnapshot::capture(path, "schedule")?, kind)
}
pub fn review_snapshot(snapshot: &SourceSnapshot, kind: &str) -> Result<Json> {
    let parse = || -> Result<Json> {
        let raw = snapshot.text("schedule", true)?;
        pyjson::loads_detailed(raw).map_err(|(error, diagnostic)| match error {
            LoadError::Invalid => diagnostic.unwrap(),
            LoadError::DuplicateKey(key) => format!("Duplicate JSON field: {}.", key.os_text()),
            LoadError::Crash(reason) => reason,
        })
    };
    let data = parse().map_err(|e| format!("Invalid schedule JSON: {e}."))?;
    if !["expenses", "migration", "interentity"].contains(&kind) {
        return Err("Unknown schedule kind.".into());
    }
    if data.get("schema_version").and_then(Json::as_plain_int) != Json::int(1).as_plain_int() {
        return Err("schema_version must be integer 1.".into());
    }
    let currency = data
        .get("currency")
        .and_then(Json::as_pystr)
        .filter(|s| s.0.len() == 3 && s.0.iter().all(|&cp| (65..=90).contains(&cp)))
        .ok_or("currency must be an explicit 3-letter code.")?;
    date(data.get("cutoff").unwrap_or(&Json::Null), "cutoff")?;
    let result = decimal::with_precision(80, || match kind {
        "expenses" => expenses(&data),
        "migration" => migration(&data),
        _ => interentity(&data),
    })?;
    let mut output = Json::object([
        ("schema_version", Json::int(1)),
        ("kind", Json::str(kind)),
        ("currency", Json::Str(currency.clone())),
        ("cutoff", data.at("cutoff").clone()),
        ("source_sha256", Json::str(&snapshot.sha256)),
        (
            "status",
            Json::str(if result.at("findings").as_array().unwrap().is_empty() {
                "PASS"
            } else {
                "REVIEW"
            }),
        ),
        (
            "scope",
            Json::str("Supplied records and explicit mappings only; no posting or approval."),
        ),
    ]);
    let Json::Object(pairs) = result else {
        unreachable!()
    };
    for (key, value) in pairs {
        output.set(&key.to_utf8().unwrap(), value);
    }
    Ok(output)
}

fn expenses(data: &Json) -> Result<Json> {
    object(
        data,
        "schema_version tenant currency cutoff invoices claims payments ledger_balance",
        "expenses",
    )?;
    let tenant = text(data.at("tenant"), "tenant")?;
    let invoices = rows(
        data.at("invoices"),
        "invoice_id supplier reference amount evidence",
        "invoice_id",
        "invoices",
    )?;
    if invoices.is_empty() {
        return Err(
            "Supply the invoice population; empty evidence cannot establish completeness.".into(),
        );
    }
    let claims = rows(
        data.at("claims"),
        "claim_id invoice_id employee amount approved",
        "claim_id",
        "claims",
    )?;
    let payments = rows(
        data.at("payments"),
        "payment_id claim_id amount date reference",
        "payment_id",
        "payments",
    )?;
    let mut findings = Vec::new();
    let mut amounts = BTreeMap::new();
    let mut allocated = BTreeMap::new();
    let mut paid = BTreeMap::new();
    let mut references: BTreeMap<(PyStr, PyStr), Vec<PyStr>> = BTreeMap::new();
    let mut reference_order = Vec::new();
    for row in invoices {
        let amount = money(row.at("amount"), "invoice amount")?;
        if amount <= Decimal::zero() {
            return Err("Invoice amount must be positive.".into());
        }
        let identifier = id(row, "invoice_id");
        amounts.insert(identifier.clone(), amount);
        let reference = (
            text(row.at("supplier"), "supplier")?,
            text(row.at("reference"), "reference")?,
        );
        if !references.contains_key(&reference) {
            reference_order.push(reference.clone());
        }
        references
            .entry(reference)
            .or_default()
            .push(identifier.clone());
        let evidence = row
            .at("evidence")
            .as_pystr()
            .ok_or("evidence must be text; use an empty string for missing evidence.")?;
        if evidence.0.iter().all(|&cp| pytext::is_space(cp)) {
            findings.push(finding("MISSING_RECEIPT", &identifier));
        }
    }
    for key in reference_order {
        let identifiers = &references[&key];
        if identifiers.len() > 1 {
            findings.push(finding(
                "DUPLICATE_INVOICE_REFERENCE",
                &joined(identifiers, ", "),
            ));
        }
    }
    for row in claims {
        text(row.at("employee"), "employee")?;
        let invoice = text(row.at("invoice_id"), "invoice_id")?;
        let amount = money(row.at("amount"), "claim amount")?;
        if amount <= Decimal::zero() || row.at("approved").as_bool().is_none() {
            return Err("Claims require a positive amount and boolean approved.".into());
        }
        add(&mut allocated, invoice.clone(), &amount)?;
        if !amounts.contains_key(&invoice) {
            findings.push(finding("UNKNOWN_INVOICE", &id(row, "claim_id")));
        }
        if row.at("approved").as_bool() == Some(false) {
            findings.push(finding("UNAPPROVED_CLAIM", &id(row, "claim_id")));
        }
    }
    for row in invoices {
        let identifier = id(row, "invoice_id");
        let allocation = allocated
            .entry(identifier.clone())
            .or_insert_with(Decimal::zero);
        let difference = exact(amounts[&identifier].sub(allocation))?;
        if !difference.is_zero() {
            let mut item = finding(
                if difference < Decimal::zero() {
                    "OVER_ALLOCATED"
                } else {
                    "UNALLOCATED_INVOICE"
                },
                &identifier,
            );
            item.set("difference", number(&difference));
            findings.push(item);
        }
    }
    let claim_ids: BTreeSet<_> = claims.iter().map(|r| id(r, "claim_id")).collect();
    for row in payments {
        let claim = text(row.at("claim_id"), "payment claim_id")?;
        text(row.at("reference"), "payment reference")?;
        let amount = money(row.at("amount"), "payment amount")?;
        if amount <= Decimal::zero()
            || date(row.at("date"), "payment date")? > date(data.at("cutoff"), "cutoff")?
        {
            return Err("Payments require a positive amount and date on or before cutoff.".into());
        }
        add(&mut paid, claim.clone(), &amount)?;
        if !claim_ids.contains(&claim) {
            let mut item = finding("UNLINKED_PAYMENT", &id(row, "payment_id"));
            item.set("amount", number(&amount));
            findings.push(item);
        }
    }
    let mut workpaper = Vec::new();
    for row in claims {
        let identifier = id(row, "claim_id");
        let payment = paid.entry(identifier.clone()).or_insert_with(Decimal::zero);
        let remaining = exact(money(row.at("amount"), "claim amount")?.sub(payment))?;
        let mut item = row.clone();
        item.set("paid", number(payment));
        item.set("remaining", number(&remaining));
        workpaper.push(item);
        if remaining < Decimal::zero()
            || (remaining > Decimal::zero() && row.at("approved").as_bool() == Some(true))
        {
            let mut item = finding(
                if remaining < Decimal::zero() {
                    "OVERPAID"
                } else {
                    "UNPAID_APPROVED"
                },
                &identifier,
            );
            item.set("amount", number(&remaining));
            findings.push(item);
        }
    }
    let expected = exact(
        sum(claims
            .iter()
            .map(|r| money(r.at("amount"), "claim amount"))
            .collect::<Result<Vec<_>>>()?)?
        .sub(&sum(paid.values().cloned())?),
    )?;
    let difference = exact(money(data.at("ledger_balance"), "ledger_balance")?.sub(&expected))?;
    if !difference.is_zero() {
        let mut item = finding("CLEARING_DIFFERENCE", &tenant);
        item.set("difference", number(&difference));
        findings.push(item);
    }
    Ok(Json::object([
        ("tenant", Json::Str(tenant)),
        ("claims", Json::Array(workpaper)),
        ("payments", Json::Array(payments.to_vec())),
        ("invoices", Json::Array(invoices.to_vec())),
        ("expected_ledger_balance", number(&expected)),
        ("ledger_balance", data.at("ledger_balance").clone()),
        ("ledger_difference", number(&difference)),
        ("findings", Json::Array(findings)),
    ]))
}

type MigrationKey = (PyStr, PyStr, PyStr, PyStr);
fn migration(data: &Json) -> Result<Json> {
    object(
        data,
        "schema_version tenant currency cutoff mapping before after",
        "migration",
    )?;
    let tenant = text(data.at("tenant"), "tenant")?;
    let mappings = rows(
        data.at("mapping"),
        "mapping_id source_account target_account weight",
        "mapping_id",
        "mapping",
    )?;
    let before = rows(
        data.at("before"),
        "row_id item_id account amount reference tax_code",
        "row_id",
        "before",
    )?;
    let after = rows(
        data.at("after"),
        "row_id item_id account amount reference tax_code",
        "row_id",
        "after",
    )?;
    if before.is_empty() {
        return Err("Supply the pre-migration population.".into());
    }
    let mut weights: BTreeMap<PyStr, Vec<(PyStr, Decimal)>> = BTreeMap::new();
    let mut pairs = BTreeSet::new();
    for row in mappings {
        let source = text(row.at("source_account"), "source_account")?;
        let target = text(row.at("target_account"), "target_account")?;
        let valid = row.at("weight").as_pystr().is_some_and(|s| {
            let p = &s.0;
            if p == &[48] || p == &[49] {
                return true;
            }
            if !(3..=8).contains(&p.len()) || p[1] != 46 {
                return false;
            }
            (p[0] == 48 && p[2..].iter().all(|&cp| pytext::decimal_value(cp).is_some()))
                || (p[0] == 49 && p[2..].iter().all(|&cp| cp == 48))
        });
        if !valid {
            return Err("Mapping weight must be a decimal string in (0, 1].".into());
        }
        let weight =
            Decimal::parse(&row.at("weight").as_pystr().unwrap().to_utf8().unwrap()).unwrap();
        if weight <= Decimal::zero() || !pairs.insert((source.clone(), target.clone())) {
            return Err("Mapping weights must be positive and source/target pairs unique.".into());
        }
        weights.entry(source).or_default().push((target, weight));
    }
    for rows in weights.values() {
        if sum(rows.iter().map(|(_, w)| w.clone()))? != Decimal::parse("1").unwrap() {
            return Err("Each source account's mapping weights must sum to 1.".into());
        }
    }
    let mut expected: BTreeMap<MigrationKey, Decimal> = BTreeMap::new();
    let mut actual = BTreeMap::new();
    let mut findings = Vec::new();
    for (rows, values, source_side) in [(before, &mut expected, true), (after, &mut actual, false)]
    {
        let mut seen = BTreeSet::new();
        for row in rows {
            for key in ["item_id", "account", "reference", "tax_code"] {
                text(row.at(key), key)?;
            }
            let amount = money(row.at("amount"), "amount")?;
            let account = id(row, "account");
            if !seen.insert((id(row, "item_id"), account.clone())) {
                let mut item = finding("DUPLICATE_ITEM", &id(row, "row_id"));
                item.set(
                    "side",
                    Json::str(if source_side { "before" } else { "after" }),
                );
                findings.push(item);
            }
            let destinations = if source_side {
                weights.get(&account).cloned().unwrap_or_default()
            } else {
                vec![(account.clone(), Decimal::parse("1").unwrap())]
            };
            if destinations.is_empty() {
                findings.push(finding("UNMAPPED_ACCOUNT", &account));
            }
            for (account, weight) in destinations {
                add(
                    values,
                    (
                        id(row, "item_id"),
                        account,
                        id(row, "reference"),
                        id(row, "tax_code"),
                    ),
                    &exact(amount.mul_outcome(&weight))?,
                )?;
            }
        }
    }
    let keys: BTreeSet<_> = expected.keys().chain(actual.keys()).cloned().collect();
    let mut differences = Vec::new();
    for key in keys {
        let old = expected.get(&key).cloned().unwrap_or_else(Decimal::zero);
        let new = actual.get(&key).cloned().unwrap_or_else(Decimal::zero);
        let difference = exact(new.sub(&old))?;
        if !expected.contains_key(&key) || !actual.contains_key(&key) || !difference.is_zero() {
            differences.push(Json::object([
                ("item_id", Json::Str(key.0.clone())),
                ("account", Json::Str(key.1.clone())),
                ("reference", Json::Str(key.2.clone())),
                ("tax_code", Json::Str(key.3.clone())),
                ("expected", number(&old)),
                ("actual", number(&new)),
                ("difference", number(&difference)),
                (
                    "presence",
                    Json::str(
                        if expected.contains_key(&key) && actual.contains_key(&key) {
                            "both"
                        } else if expected.contains_key(&key) {
                            "before"
                        } else {
                            "after"
                        },
                    ),
                ),
            ]));
        }
    }
    if !differences.is_empty() {
        findings.push(finding(
            "ITEM_DIFFERENCES",
            &PyStr::from(differences.len().to_string().as_str()),
        ));
    }
    Ok(Json::object([
        ("tenant", Json::Str(tenant)),
        ("mapping", Json::Array(mappings.to_vec())),
        ("differences", Json::Array(differences)),
        (
            "before_total",
            number(&sum(before
                .iter()
                .map(|r| money(r.at("amount"), "amount"))
                .collect::<Result<Vec<_>>>()?)?),
        ),
        ("after_total", number(&sum(actual.values().cloned())?)),
        ("findings", Json::Array(findings)),
        (
            "policy",
            Json::str(
                "Item IDs, references and tax codes must be preserved exactly. Weights allocate amounts without rounding.",
            ),
        ),
    ]))
}

fn interentity(data: &Json) -> Result<Json> {
    object(
        data,
        "schema_version currency cutoff entities records",
        "interentity",
    )?;
    let entities = data
        .at("entities")
        .as_array()
        .filter(|s| s.len() >= 2)
        .ok_or("Supply at least 2 explicit entity IDs.")?
        .iter()
        .map(|v| text(v, "entity"))
        .collect::<Result<Vec<_>>>()?;
    if entities.iter().collect::<BTreeSet<_>>().len() != entities.len() {
        return Err("Entity IDs must be unique.".into());
    }
    let records = rows(
        data.at("records"),
        "row_id entity counterparty account reference date amount disputed evidence",
        "row_id",
        "records",
    )?;
    if records.is_empty() {
        return Err(
            "Supply both entities' records; empty evidence cannot establish agreement.".into(),
        );
    }
    type Group = (Vec<Json>, Vec<Json>);
    let mut groups: BTreeMap<(PyStr, PyStr, PyStr), Group> = BTreeMap::new();
    let mut findings = Vec::new();
    let cutoff = date(data.at("cutoff"), "cutoff")?;
    for row in records {
        for key in ["entity", "counterparty", "account", "reference", "evidence"] {
            text(row.at(key), key)?;
        }
        let entity = id(row, "entity");
        let counterparty = id(row, "counterparty");
        if !entities.contains(&entity)
            || !entities.contains(&counterparty)
            || entity == counterparty
        {
            return Err("Each record requires 2 distinct declared entity IDs.".into());
        }
        money(row.at("amount"), "amount")?;
        if date(row.at("date"), "date")? > cutoff || row.at("disputed").as_bool().is_none() {
            return Err("Records require a date on or before cutoff and boolean disputed.".into());
        }
        let (left, right) = if entity < counterparty {
            (entity.clone(), counterparty)
        } else {
            (counterparty, entity.clone())
        };
        let group = groups
            .entry((left.clone(), right, id(row, "reference")))
            .or_default();
        if entity == left {
            group.0.push(row.clone());
        } else {
            group.1.push(row.clone());
        }
        if row.at("disputed").as_bool() == Some(true) {
            findings.push(finding("DISPUTED", &id(row, "row_id")));
        }
    }
    let mut workpaper = Vec::new();
    for ((left, right, reference), (left_rows, right_rows)) in groups {
        let a = sum(left_rows
            .iter()
            .map(|r| money(r.at("amount"), "amount"))
            .collect::<Result<Vec<_>>>()?)?;
        let b = sum(right_rows
            .iter()
            .map(|r| money(r.at("amount"), "amount"))
            .collect::<Result<Vec<_>>>()?)?;
        let difference = exact(a.add(&b))?;
        let status = if left_rows.is_empty() || right_rows.is_empty() {
            "UNMATCHED"
        } else if !difference.is_zero() {
            "DIFFERENCE"
        } else {
            "AGREES"
        };
        if status != "AGREES" {
            let mut item = finding(
                status,
                &joined(&[left.clone(), right.clone(), reference.clone()], "/"),
            );
            item.set("difference", number(&difference));
            findings.push(item);
        }
        workpaper.push(Json::object([
            ("entity", Json::Str(left.clone())),
            ("counterparty", Json::Str(right.clone())),
            ("reference", Json::Str(reference)),
            ("entity_balance", number(&a)),
            ("counterparty_balance", number(&b)),
            ("difference", number(&difference)),
            ("status", Json::str(status)),
            (
                "records",
                Json::Object(vec![
                    (left, Json::Array(left_rows)),
                    (right, Json::Array(right_rows)),
                ]),
            ),
        ]));
    }
    Ok(Json::object([
        (
            "entities",
            Json::Array(entities.into_iter().map(Json::Str).collect()),
        ),
        ("pairs", Json::Array(workpaper)),
        ("findings", Json::Array(findings)),
        (
            "policy",
            Json::str(
                "Debit-positive balances; exact reference matching only. No inferred identities or elimination entries.",
            ),
        ),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn money_accepts_unicode_digits_and_refuses_unbounded_or_fractional_inputs() {
        assert_eq!(
            money(&Json::str("-１２.３４"), "amount")
                .unwrap()
                .to_sci_string(),
            "-12.34"
        );
        for value in ["12345678901234567", "1.001", "+1", "1e2", "-", ".1", "1."] {
            assert!(money(&Json::str(value), "amount").is_err());
        }
    }
    #[test]
    fn snapshot_provenance_binds_exact_bytes_and_schema_failures_precede_calculations() {
        let raw = br#"{"schema_version":true,"currency":"AUD","cutoff":"2026-09-30"}"#;
        let source = SourceSnapshot::new(Path::new("schedule.json"), raw.to_vec());
        assert_eq!(
            review_snapshot(&source, "expenses").unwrap_err(),
            "schema_version must be integer 1."
        );
        assert_eq!(
            review_snapshot(&source, "unknown").unwrap_err(),
            "Unknown schedule kind."
        );
        let bad = SourceSnapshot::new(Path::new("schedule.json"), b"{".to_vec());
        assert!(
            review_snapshot(&bad, "unknown")
                .unwrap_err()
                .starts_with("Invalid schedule JSON:")
        );
    }
}
