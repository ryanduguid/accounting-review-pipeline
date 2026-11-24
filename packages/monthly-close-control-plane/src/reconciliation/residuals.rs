// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Deterministic residual taxonomy for unreconciled clearing items.

use crate::{
    Result,
    decimal::{self, Decimal},
    isodate::{self, Date},
    pyjson::{Json, PyStr},
    pytext, reconciliation,
};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "clearing-residuals-v1";
pub const BOUNDARY: &str = "Residual classification is a review aid only; it does not clear, allocate, or approve any item.";

const FX_TOKENS: [&str; 6] = ["fx", "forex", "usd", "eur", "gbp", "nzd"];
const FX_PHRASES: [&str; 8] = [
    "foreign exchange",
    "exchange rate",
    "exchange difference",
    "currency conversion",
    "spot rate",
    "fx variance",
    "fx adjustment",
    "fx gain",
];
const FEE_TOKENS: [&str; 3] = ["fee", "fees", "surcharge"];
const FEE_PHRASES: [&str; 10] = [
    "bank charge",
    "bank charges",
    "merchant fee",
    "processing fee",
    "settlement fee",
    "wire fee",
    "card fee",
    "stripe fee",
    "eftpos fee",
    "gateway fee",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResidualClass {
    Fx,
    BankFee,
    Partial,
    Timing,
    Duplicate,
    Unexplained,
}

impl ResidualClass {
    pub const ALL: [Self; 6] = [
        Self::Fx,
        Self::BankFee,
        Self::Partial,
        Self::Timing,
        Self::Duplicate,
        Self::Unexplained,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fx => "fx",
            Self::BankFee => "bank_fee",
            Self::Partial => "partial",
            Self::Timing => "timing",
            Self::Duplicate => "duplicate",
            Self::Unexplained => "unexplained",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResidualPolicy {
    pub fx_ratio_limit: Decimal,
    pub bank_fee_limit: Decimal,
    pub timing_window_days: i32,
    pub duplicate_window_days: i32,
}

impl Default for ResidualPolicy {
    fn default() -> Self {
        Self {
            fx_ratio_limit: Decimal::parse("0.05").unwrap(),
            bank_fee_limit: Decimal::parse("50.00").unwrap(),
            timing_window_days: 5,
            duplicate_window_days: 7,
        }
    }
}

impl ResidualPolicy {
    pub fn validate(&self) -> Result<()> {
        let zero = Decimal::zero();
        let one = Decimal::parse("1").unwrap();
        if !self.fx_ratio_limit.is_finite()
            || self.fx_ratio_limit < zero
            || self.fx_ratio_limit > one
        {
            return Err("fx_ratio_limit must be finite and between 0 and 1.".into());
        }
        if !self.bank_fee_limit.is_finite() || self.bank_fee_limit < zero {
            return Err("bank_fee_limit must be finite and non-negative.".into());
        }
        if self.timing_window_days < 0 || self.duplicate_window_days < 0 {
            return Err("Window day thresholds must be non-negative.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResidualItem {
    pub transaction_ids: Vec<String>,
    pub class: ResidualClass,
    pub reference: String,
    pub net_amount: Decimal,
    pub gross_amount: Decimal,
    pub latest_date: String,
    pub age_days: i32,
    pub rationale: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResidualSummary {
    pub class: ResidualClass,
    pub count: usize,
    pub net_total: Decimal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResidualReport {
    pub period_end: String,
    pub total_outstanding_items: usize,
    pub total_net_amount: Decimal,
    pub items: Vec<ResidualItem>,
    pub summary: Vec<ResidualSummary>,
}

fn item_to_json(item: &ResidualItem) -> Json {
    Json::object([
        (
            "transaction_ids",
            Json::Array(item.transaction_ids.iter().map(|s| Json::str(s)).collect()),
        ),
        ("class", Json::str(item.class.as_str())),
        ("reference", Json::str(&item.reference)),
        (
            "net_amount",
            Json::str(&reconciliation::fixed(&item.net_amount)),
        ),
        (
            "gross_amount",
            Json::str(&reconciliation::fixed(&item.gross_amount)),
        ),
        ("latest_date", Json::str(&item.latest_date)),
        ("age_days", Json::int(i64::from(item.age_days))),
        ("rationale", Json::str(&item.rationale)),
    ])
}

fn summary_to_json(entry: &ResidualSummary) -> Json {
    Json::object([
        ("class", Json::str(entry.class.as_str())),
        ("count", Json::int(entry.count as i64)),
        (
            "net_total",
            Json::str(&reconciliation::fixed(&entry.net_total)),
        ),
    ])
}

impl ResidualReport {
    pub fn to_json(&self) -> Json {
        Json::object([
            ("schema", Json::str(SCHEMA)),
            ("boundary", Json::str(BOUNDARY)),
            ("period_end", Json::str(&self.period_end)),
            (
                "total_outstanding_items",
                Json::int(self.total_outstanding_items as i64),
            ),
            (
                "total_net_amount",
                Json::str(&reconciliation::fixed(&self.total_net_amount)),
            ),
            (
                "items",
                Json::Array(self.items.iter().map(item_to_json).collect()),
            ),
            (
                "summary",
                Json::Array(self.summary.iter().map(summary_to_json).collect()),
            ),
        ])
    }
}

#[derive(Clone)]
struct Row {
    index: usize,
    id: String,
    date: Date,
    date_text: String,
    age_days: i32,
    reference: String,
    description: String,
    amount: Decimal,
}

fn read_str(object: &Json, key: &str) -> Result<String> {
    object
        .get(key)
        .and_then(Json::as_pystr)
        .and_then(PyStr::to_utf8)
        .ok_or_else(|| format!("Reconciliation pack field '{key}' must be UTF-8 text."))
}

fn parse_amount(raw: &str, field: &str) -> Result<Decimal> {
    Decimal::parse(raw)
        .ok()
        .filter(|d| d.is_finite() && *d >= Decimal::zero())
        .ok_or_else(|| format!("Reconciliation row has invalid {field} amount."))
}

fn parse_rows(pack: &Json, period_end: Date) -> Result<Vec<Row>> {
    let rows = pack
        .get("outstanding")
        .and_then(Json::as_array)
        .ok_or_else(|| "Reconciliation pack must contain an 'outstanding' array.".to_owned())?;
    let end_ord = isodate::ordinal(period_end);
    let mut parsed = Vec::with_capacity(rows.len());
    for (index, item) in rows.iter().enumerate() {
        let id = read_str(item, "TransactionID")?;
        let date_text = read_str(item, "Date")?;
        let date = reconciliation::date(&date_text)?;
        let age_days = end_ord - isodate::ordinal(date);
        if age_days < 0 {
            return Err(format!(
                "Outstanding item '{id}' is dated after period_end."
            ));
        }
        let debit = parse_amount(&read_str(item, "Debit")?, "Debit")?;
        let credit = parse_amount(&read_str(item, "Credit")?, "Credit")?;
        parsed.push(Row {
            index,
            id,
            date,
            date_text,
            age_days,
            reference: read_str(item, "Reference")?,
            description: read_str(item, "Description")?,
            amount: debit.sub(&credit).value,
        });
    }
    Ok(parsed)
}

fn normalised_text(reference: &str, description: &str) -> String {
    pytext::casefold(&format!("{reference} {description}"))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_cue(text: &str, tokens: &[&str], phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| text.contains(phrase))
        || text
            .split(|ch: char| !ch.is_alphanumeric())
            .any(|word| tokens.contains(&word))
}

fn opposing_label(
    combined: &str,
    reference: &str,
    net: &Decimal,
    gross: &Decimal,
    policy: &ResidualPolicy,
) -> (ResidualClass, String) {
    let abs_net = net.copy_abs();
    let fx_cap = gross.mul(&policy.fx_ratio_limit);
    if has_cue(combined, &FX_TOKENS, &FX_PHRASES) && abs_net <= fx_cap {
        return (
            ResidualClass::Fx,
            format!(
                "Opposing legs sharing reference '{reference}' carry an FX cue with residual {} within the FX ratio limit.",
                reconciliation::fixed(&abs_net)
            ),
        );
    }
    if has_cue(combined, &FEE_TOKENS, &FEE_PHRASES) && abs_net <= policy.bank_fee_limit {
        return (
            ResidualClass::BankFee,
            format!(
                "Opposing legs sharing reference '{reference}' carry a fee cue with residual {} within the bank fee limit.",
                reconciliation::fixed(&abs_net)
            ),
        );
    }
    (
        ResidualClass::Partial,
        format!(
            "Opposing legs sharing reference '{reference}' leave a partial residual of {}.",
            reconciliation::fixed(net)
        ),
    )
}

fn classify_opposing_group(
    group: &[&Row],
    reference: &str,
    policy: &ResidualPolicy,
) -> Option<ResidualItem> {
    let zero = Decimal::zero();
    let (mut debits, mut credits) = (Decimal::zero(), Decimal::zero());
    for row in group {
        if row.amount > zero {
            debits = debits.add(&row.amount).value;
        } else {
            credits = credits.add(&row.amount.copy_abs()).value;
        }
    }
    let net = debits.sub(&credits).value;
    if debits.is_zero() || credits.is_zero() || net.is_zero() {
        return None;
    }
    let gross = if debits >= credits { debits } else { credits };
    let combined: String = group
        .iter()
        .map(|row| normalised_text(&row.reference, &row.description))
        .collect::<Vec<_>>()
        .join(" ");
    let (class, rationale) = opposing_label(&combined, reference, &net, &gross, policy);
    Some(build_item(
        group,
        class,
        reference.to_owned(),
        net,
        gross,
        rationale,
    ))
}

fn build_item(
    rows: &[&Row],
    class: ResidualClass,
    reference: String,
    net_amount: Decimal,
    gross_amount: Decimal,
    rationale: String,
) -> ResidualItem {
    let mut ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    ids.sort();
    let latest = rows
        .iter()
        .max_by_key(|row| (isodate::ordinal(row.date), row.id.as_str()))
        .unwrap();
    ResidualItem {
        transaction_ids: ids,
        class,
        reference,
        net_amount,
        gross_amount,
        latest_date: latest.date_text.clone(),
        age_days: latest.age_days,
        rationale,
    }
}

fn duplicate_buckets<'a>(
    rows: &'a [Row],
    used: &BTreeSet<usize>,
) -> BTreeMap<(bool, String, String), Vec<&'a Row>> {
    let mut buckets: BTreeMap<(bool, String, String), Vec<&'a Row>> = BTreeMap::new();
    for row in rows {
        if used.contains(&row.index) || row.amount.is_zero() {
            continue;
        }
        let raw_key = if row.reference.trim().is_empty() {
            row.description.trim()
        } else {
            row.reference.trim()
        };
        let key_text = pytext::casefold(raw_key);
        if key_text.is_empty() {
            continue;
        }
        buckets
            .entry((
                row.amount > Decimal::zero(),
                reconciliation::fixed(&row.amount.copy_abs()),
                key_text,
            ))
            .or_default()
            .push(row);
    }
    buckets
}

fn classify_duplicates(
    rows: &[Row],
    used: &mut BTreeSet<usize>,
    policy: &ResidualPolicy,
) -> Vec<ResidualItem> {
    let buckets = duplicate_buckets(rows, used);
    let mut result = Vec::new();
    for ((_, amount_text, _), mut bucket) in buckets {
        bucket.sort_by_key(|row| (isodate::ordinal(row.date), row.id.as_str()));
        let mut cluster: Vec<&Row> = Vec::new();
        for row in bucket {
            let within = cluster.first().is_some_and(|prev| {
                isodate::ordinal(row.date) - isodate::ordinal(prev.date)
                    <= policy.duplicate_window_days
            });
            if cluster.is_empty() || within {
                cluster.push(row);
            } else {
                emit_duplicate_cluster(&cluster, &amount_text, used, &mut result);
                cluster.clear();
                cluster.push(row);
            }
        }
        emit_duplicate_cluster(&cluster, &amount_text, used, &mut result);
    }
    result
}

fn emit_duplicate_cluster(
    cluster: &[&Row],
    amount_text: &str,
    used: &mut BTreeSet<usize>,
    result: &mut Vec<ResidualItem>,
) {
    if cluster.len() < 2 {
        return;
    }
    for row in cluster {
        used.insert(row.index);
    }
    let net = cluster
        .iter()
        .fold(Decimal::zero(), |acc, row| acc.add(&row.amount).value);
    let gross = net.copy_abs();
    let reference = cluster[0].reference.clone();
    let rationale = format!(
        "{} same-side outstanding items share amount {amount_text} and key within the duplicate window.",
        cluster.len()
    );
    result.push(build_item(
        cluster,
        ResidualClass::Duplicate,
        reference,
        net,
        gross,
        rationale,
    ));
}

fn classify_singleton(row: &Row, policy: &ResidualPolicy) -> ResidualItem {
    let text = normalised_text(&row.reference, &row.description);
    let abs = row.amount.copy_abs();
    let (class, rationale) =
        if has_cue(&text, &FEE_TOKENS, &FEE_PHRASES) && abs <= policy.bank_fee_limit {
            (
                ResidualClass::BankFee,
                format!(
                    "Unmatched item '{}' carries a fee cue within the bank fee limit.",
                    row.id
                ),
            )
        } else if has_cue(&text, &[], &FX_PHRASES)
            || (has_cue(&text, &FX_TOKENS, &[]) && abs <= policy.bank_fee_limit)
        {
            (
                ResidualClass::Fx,
                format!("Unmatched item '{}' carries an FX cue.", row.id),
            )
        } else if row.age_days <= policy.timing_window_days {
            (
                ResidualClass::Timing,
                format!(
                    "Unmatched item '{}' is {} day(s) old, within the {}-day timing window.",
                    row.id, row.age_days, policy.timing_window_days
                ),
            )
        } else {
            (
                ResidualClass::Unexplained,
                format!(
                    "Unmatched item '{}' is {} day(s) old with no deterministic residual pattern.",
                    row.id, row.age_days
                ),
            )
        };
    build_item(
        &[row],
        class,
        row.reference.clone(),
        row.amount.clone(),
        abs,
        rationale,
    )
}

fn summarise(items: &[ResidualItem]) -> Vec<ResidualSummary> {
    let mut counts: BTreeMap<ResidualClass, (usize, Decimal)> = ResidualClass::ALL
        .into_iter()
        .map(|class| (class, (0, Decimal::zero())))
        .collect();
    for item in items {
        let entry = counts.get_mut(&item.class).unwrap();
        entry.0 += 1;
        entry.1 = entry.1.add(&item.net_amount).value;
    }
    ResidualClass::ALL
        .into_iter()
        .map(|class| {
            let (count, net_total) = counts.remove(&class).unwrap();
            ResidualSummary {
                class,
                count,
                net_total,
            }
        })
        .collect()
}

fn classify_rows(rows: &[Row], policy: &ResidualPolicy) -> Vec<ResidualItem> {
    let mut by_ref: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
    for row in rows {
        let trimmed = row.reference.trim();
        if !trimmed.is_empty() {
            by_ref.entry(trimmed.to_owned()).or_default().push(row);
        }
    }
    let mut used = BTreeSet::new();
    let mut items = Vec::new();
    for (reference, group) in by_ref {
        if let Some(item) = classify_opposing_group(&group, &reference, policy) {
            for row in group {
                used.insert(row.index);
            }
            items.push(item);
        }
    }
    items.extend(classify_duplicates(rows, &mut used, policy));
    for row in rows {
        if !used.contains(&row.index) {
            items.push(classify_singleton(row, policy));
        }
    }
    items.sort_by(|a, b| {
        a.class
            .cmp(&b.class)
            .then_with(|| a.transaction_ids.cmp(&b.transaction_ids))
    });
    items
}

pub fn classify_pack(pack: &Json, policy: &ResidualPolicy) -> Result<ResidualReport> {
    policy.validate()?;
    if !pack
        .get("schema")
        .is_some_and(|s| s.is_str("clearing-reconciliation-v1"))
    {
        return Err("Reconciliation pack must use schema 'clearing-reconciliation-v1'.".into());
    }
    let period_end_text = read_str(pack, "period_end")?;
    let period_end = reconciliation::date(&period_end_text)?;
    decimal::with_precision(40, || {
        let rows = parse_rows(pack, period_end)?;
        let items = classify_rows(&rows, policy);
        let total_net_amount = rows
            .iter()
            .fold(Decimal::zero(), |acc, row| acc.add(&row.amount).value);
        let summary = summarise(&items);
        Ok(ResidualReport {
            period_end: period_end_text,
            total_outstanding_items: rows.len(),
            total_net_amount,
            items,
            summary,
        })
    })
}
