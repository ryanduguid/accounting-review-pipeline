//! Local transaction allocations. Suggestions never clear an item.
use crate::{
    Result,
    decimal::{self, Decimal, Rounding},
    isodate::{self, Date},
    loader::{self, SourceSnapshot},
    pyjson::{self, Json, LoadError, PyStr},
    pypath, pytext,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub const COLUMNS: [&str; 9] = [
    "Tenant",
    "AccountID",
    "Currency",
    "TransactionID",
    "Date",
    "Reference",
    "Description",
    "Debit",
    "Credit",
];
pub const DECISION_COLUMNS: [&str; 4] = ["Group", "TransactionID", "Decision", "Note"];
pub const IDENTITY_COLUMNS: [&str; 3] = ["Tenant", "AccountID", "Currency"];
pub const BOUNDARY: &str =
    "Review evidence only. Matching records an allocation; it does not approve a close.";
fn field(value: &Json, key: &str) -> String {
    let value = value.at(key).as_pystr().unwrap();
    value.to_utf8().unwrap_or_else(|| value.os_text())
}
pub fn date(raw: &str) -> Result<Date> {
    let bytes = raw.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, c)| i != 4 && i != 7 && !c.is_ascii_digit())
    {
        return Err("Dates must use YYYY-MM-DD.".into());
    }
    if let Some(date) = isodate::date_fromisoformat(raw) {
        return Ok(date);
    }
    let year: i32 = raw[..4].parse().unwrap();
    let month: u32 = raw[5..7].parse().unwrap();
    let day: u32 = raw[8..].parse().unwrap();
    if year == 0 {
        return Err("year must be in 1..9999, not 0".into());
    }
    if !(1..=12).contains(&month) {
        return Err(format!("month must be in 1..12, not {month}"));
    }
    let maximum = (28..=31)
        .rev()
        .find(|&d| Date::new(year, month, d).is_some())
        .unwrap();
    Err(format!(
        "day {day} must be in range 1..{maximum} for month {month} in year {year}"
    ))
}
pub fn money(value: &str, field: &str, path: &Path, row: usize) -> Result<Decimal> {
    let amount = loader::parse_money(Some(value), field, row, path)?;
    if amount.abs() >= Decimal::parse("1000000000000000").unwrap()
        || !amount
            .quantize(-2, Rounding::HalfEven)
            .is_ok_and(|rounded| rounded == amount)
    {
        return Err(format!(
            "{}: {field} must be below 10^15 with at most two decimal places.",
            pypath::text(path)
        ));
    }
    Ok(amount)
}
pub fn fixed(value: &Decimal) -> String {
    value.format_places(2).expect("bounded transaction amount")
}
fn item(value: &Json, identity: &Json, path: &Path, row: usize) -> Result<Json> {
    if !value.has_exact_keys(&COLUMNS)
        || COLUMNS.iter().any(|key| value.at(key).as_pystr().is_none())
    {
        return Err(format!(
            "{}: row {row} must contain every transaction field as text.",
            pypath::text(path)
        ));
    }
    let mut result = Json::Object(
        COLUMNS
            .iter()
            .map(|key| {
                loader::json_input_text(
                    value.at(key),
                    key,
                    row,
                    path,
                    ["Reference", "Description"].contains(key),
                )
                .map(|v| (PyStr::from(*key), Json::str(&v)))
            })
            .collect::<Result<_>>()?,
    );
    if IDENTITY_COLUMNS
        .iter()
        .any(|key| !pyjson::py_eq(result.at(key), identity.at(key), false))
    {
        return Err(format!(
            "{}: row {row} belongs to a different tenant, account or currency.",
            pypath::text(path)
        ));
    }
    let identifier = field(&result, "TransactionID");
    let bytes = identifier.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 80
        || !bytes[0].is_ascii_alphanumeric()
        || bytes
            .iter()
            .any(|b| !b.is_ascii_alphanumeric() && !matches!(b, b'_' | b'.' | b':' | b'-'))
    {
        return Err(format!(
            "{}: row {row} has an invalid TransactionID.",
            pypath::text(path)
        ));
    }
    date(&field(&result, "Date"))?;
    let debit = money(&field(&result, "Debit"), "Debit", path, row)?;
    let credit = money(&field(&result, "Credit"), "Credit", path, row)?;
    if debit < Decimal::zero()
        || credit < Decimal::zero()
        || (debit > Decimal::zero()) == (credit > Decimal::zero())
    {
        return Err(format!(
            "{}: row {row} needs exactly one positive Debit or Credit.",
            pypath::text(path)
        ));
    }
    result.set("Debit", Json::str(&fixed(&debit)));
    result.set("Credit", Json::str(&fixed(&credit)));
    Ok(result)
}
fn total<'a>(items: impl IntoIterator<Item = &'a Json>) -> Decimal {
    items.into_iter().fold(Decimal::zero(), |sum, item| {
        sum.add(
            &Decimal::parse(&field(item, "Debit"))
                .unwrap()
                .sub(&Decimal::parse(&field(item, "Credit")).unwrap())
                .value,
        )
        .value
    })
}
#[derive(Default)]
struct Notes {
    order: Vec<String>,
    values: BTreeMap<String, String>,
}
impl Notes {
    fn insert(&mut self, key: String, value: String) {
        if !self.values.contains_key(&key) {
            self.order.push(key.clone());
        }
        self.values.insert(key, value);
    }
    fn into_json(mut self) -> Json {
        Json::Object(
            self.order
                .into_iter()
                .map(|key| {
                    let value = self.values.remove(&key).unwrap();
                    (PyStr::from(key.as_str()), Json::str(&value))
                })
                .collect(),
        )
    }
}
fn opening(source: &SourceSnapshot, identity: &Json, start: Date) -> Result<(Vec<Json>, Notes)> {
    let raw = source.text("Opening items", true)?;
    let payload = pyjson::loads_detailed(raw).map_err(|(e, diagnostic)| {
        format!(
            "Opening items are not readable JSON: {}",
            match e {
                LoadError::Invalid => diagnostic.unwrap(),
                LoadError::DuplicateKey(key) => format!(
                    "JSON member {} appears more than once.",
                    pytext::repr_code_points(&key.0)
                ),
                LoadError::Crash(reason) => reason,
            }
        )
    })?;
    if !payload.has_exact_keys(&[
        "schema",
        "identity",
        "period_end",
        "balance",
        "items",
        "notes",
    ]) || !payload.at("schema").is_str("clearing-carry-v1")
        || !pyjson::py_eq(payload.at("identity"), identity, true)
        || payload.at("period_end").as_pystr().is_none()
        || payload.at("balance").as_pystr().is_none()
        || payload.at("items").as_array().is_none()
        || !matches!(payload.at("notes"), Json::Object(_))
    {
        return Err("Opening items have an invalid schema or a different account identity.".into());
    }
    let previous = date(&field(&payload, "period_end"))?;
    if isodate::ordinal(start) - isodate::ordinal(previous) != 1 {
        return Err("Opening items must come from the immediately preceding period.".into());
    }
    let mut items = Vec::new();
    for (index, value) in payload.at("items").as_array().unwrap().iter().enumerate() {
        if !matches!(value, Json::Object(_)) {
            return Err("Opening items must be transaction objects.".into());
        }
        let row = item(value, identity, &source.path, index + 1)?;
        if date(&field(&row, "Date"))? > previous {
            return Err("Opening item dates cannot be later than their period end.".into());
        }
        items.push(row);
    }
    let balance = payload
        .at("balance")
        .as_pystr()
        .unwrap()
        .to_utf8()
        .ok_or_else(|| {
            format!(
                "configuration: row 0 has invalid amount: {}.",
                pyjson::repr(payload.at("balance"))
            )
        })?;
    if total(&items) != money(&balance, "amount", Path::new("configuration"), 0)? {
        return Err("Opening items do not equal their recorded balance.".into());
    }
    let identifiers: BTreeSet<_> = items.iter().map(|r| field(r, "TransactionID")).collect();
    let Json::Object(notes) = payload.at("notes") else {
        unreachable!()
    };
    if notes.iter().any(|(key, value)| {
        !key.to_utf8().is_some_and(|s| identifiers.contains(&s)) || value.as_pystr().is_none()
    }) {
        return Err("Opening review notes must name outstanding transaction IDs.".into());
    }
    let mut result = Notes::default();
    for (key, value) in notes {
        let note = loader::json_input_text(value, "Note", 0, &source.path, false)?;
        result.insert(key.to_utf8().unwrap(), note);
    }
    Ok((items, result))
}
#[derive(Clone, Debug)]
struct Allocation {
    group: String,
    ids: Vec<String>,
    decision: String,
    note: String,
}
impl Allocation {
    fn json(&self) -> Json {
        Json::object([
            ("group", Json::str(&self.group)),
            (
                "ids",
                Json::Array(self.ids.iter().map(|s| Json::str(s)).collect()),
            ),
            ("decision", Json::str(&self.decision)),
            ("note", Json::str(&self.note)),
        ])
    }
}
fn decisions(
    source: Option<&SourceSnapshot>,
    items: &BTreeMap<String, Json>,
    escaped: bool,
) -> Result<Vec<Allocation>> {
    let Some(source) = source else {
        return Ok(Vec::new());
    };
    let mut groups: BTreeMap<String, Vec<[String; 4]>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for row in loader::csv_rows(source, &DECISION_COLUMNS, "Decisions")? {
        let row = row?;
        if row.values.values().any(Option::is_none) {
            return Err("Every decisions row must contain all four fields.".into());
        }
        let mut record = [
            row.text("Group", false)?,
            row.text("TransactionID", false)?,
            row.text("Decision", true)?,
            row.text("Note", true)?,
        ];
        if escaped {
            for index in [0, 3] {
                if ["''", "'=", "'+", "'-", "'@"]
                    .iter()
                    .any(|s| record[index].starts_with(s))
                {
                    record[index].remove(0);
                }
            }
        }
        if !["", "accept", "reject"].contains(&record[2].as_str()) {
            return Err("Decision must be blank, accept or reject.".into());
        }
        if !items.contains_key(&record[1]) || !seen.insert(record[1].clone()) {
            return Err("Decisions contain an unknown or repeated transaction ID.".into());
        }
        groups.entry(record[0].clone()).or_default().push(record);
    }
    let mut result = Vec::new();
    for (group, records) in groups {
        let decision = records[0][2].clone();
        let note = records[0][3].clone();
        if records.len() < 2 || records.iter().any(|r| r[2] != decision || r[3] != note) {
            return Err(
                "Each group needs at least two items with the same decision and note.".into(),
            );
        }
        if !decision.is_empty() && note.is_empty() {
            return Err("Accepted and rejected groups need a review note.".into());
        }
        let mut ids: Vec<_> = records.into_iter().map(|r| r[1].clone()).collect();
        ids.sort_by_key(|key| (field(&items[key], "Date"), key.clone()));
        if decision == "accept" && !total(ids.iter().map(|key| &items[key])).is_zero() {
            return Err(format!(
                "Accepted group {} does not sum to zero.",
                pytext::repr(&group)
            ));
        }
        result.push(Allocation {
            group,
            ids,
            decision,
            note,
        });
    }
    Ok(result)
}
#[derive(Clone, Debug)]
pub struct ReconcileConfig {
    pub transactions: PathBuf,
    pub tenant: String,
    pub account_id: String,
    pub currency: String,
    pub period_start: String,
    pub period_end: String,
    pub opening_balance: String,
    pub closing_balance: String,
    pub opening_items: Option<PathBuf>,
    pub decisions: Option<PathBuf>,
    pub decisions_escaped: bool,
}
pub fn reconcile(config: &ReconcileConfig) -> Result<Json> {
    reconcile_with(config, &mut SourceSnapshot::capture)
}
pub fn reconcile_with(
    config: &ReconcileConfig,
    capture: &mut impl FnMut(&Path, &str) -> Result<SourceSnapshot>,
) -> Result<Json> {
    decimal::with_precision(40, || {
        if config.decisions_escaped && config.decisions.is_none() {
            return Err("--decisions-escaped requires --decisions.".into());
        }
        let identity = Json::Object(
            IDENTITY_COLUMNS
                .iter()
                .zip([&config.tenant, &config.account_id, &config.currency])
                .map(|(key, value)| {
                    loader::input_text(Some(value), key, 0, Path::new("configuration"), false)
                        .map(|v| (PyStr::from(*key), Json::str(&v)))
                })
                .collect::<Result<_>>()?,
        );
        let currency = field(&identity, "Currency");
        if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(
                "Currency must be a three-letter uppercase code; no conversion is performed."
                    .into(),
            );
        }
        let start = date(&config.period_start)?;
        let end = date(&config.period_end)?;
        if start > end {
            return Err("Period start cannot be after period end.".into());
        }
        let opening_balance = money(
            &config.opening_balance,
            "amount",
            Path::new("configuration"),
            0,
        )?;
        let closing_balance = money(
            &config.closing_balance,
            "amount",
            Path::new("configuration"),
            0,
        )?;
        let transactions = capture(&config.transactions, "Transactions")?;
        let opening_source = config
            .opening_items
            .as_deref()
            .map(|p| capture(p, "Opening items"))
            .transpose()?;
        let decisions_source = config
            .decisions
            .as_deref()
            .map(|p| capture(p, "Decisions"))
            .transpose()?;
        let (old, mut notes) = opening_source
            .as_ref()
            .map(|s| opening(s, &identity, start))
            .transpose()?
            .unwrap_or_default();
        let mut current = Vec::new();
        for row in loader::csv_rows(&transactions, &COLUMNS, "Transactions")? {
            let row = row?;
            if row.values.values().any(Option::is_none) {
                return Err(format!(
                    "{}: row {} must contain every transaction field as text.",
                    pypath::text(&config.transactions),
                    row.number
                ));
            }
            let values = Json::Object(
                COLUMNS
                    .iter()
                    .map(|key| {
                        (
                            PyStr::from(*key),
                            Json::str(row.values[*key].as_deref().unwrap()),
                        )
                    })
                    .collect(),
            );
            let value = item(&values, &identity, &config.transactions, row.number)?;
            let when = date(&field(&value, "Date"))?;
            if !(start <= when && when <= end) {
                return Err(format!(
                    "{}: row {} falls outside the reporting period.",
                    pypath::text(&config.transactions),
                    row.number
                ));
            }
            current.push(value);
        }
        let mut ordered: Vec<_> = old.iter().chain(&current).cloned().collect();
        ordered.sort_by_key(|row| (field(row, "Date"), field(row, "TransactionID")));
        if ordered.len() > 100000 {
            return Err(
                "A reconciliation supports at most 100,000 outstanding and current items.".into(),
            );
        }
        let items: BTreeMap<_, _> = ordered
            .iter()
            .map(|row| (field(row, "TransactionID"), row.clone()))
            .collect();
        if items.len() != ordered.len() {
            return Err(
                "Repeated TransactionID within or across opening and current items.".into(),
            );
        }
        let allocations = decisions(decisions_source.as_ref(), &items, config.decisions_escaped)?;
        let accepted: BTreeSet<_> = allocations
            .iter()
            .filter(|a| a.decision == "accept")
            .flat_map(|a| a.ids.iter().cloned())
            .collect();
        let grouped: BTreeSet<_> = allocations
            .iter()
            .flat_map(|a| a.ids.iter().cloned())
            .collect();
        for group in &allocations {
            if !group.note.is_empty() {
                for key in &group.ids {
                    notes.insert(key.clone(), group.note.clone());
                }
            }
        }
        let outstanding: Vec<_> = ordered
            .iter()
            .filter(|row| !accepted.contains(&field(row, "TransactionID")))
            .cloned()
            .collect();
        let mut references: BTreeMap<String, Vec<&Json>> = BTreeMap::new();
        for row in &outstanding {
            let reference = field(row, "Reference");
            if !reference.is_empty() && !grouped.contains(&field(row, "TransactionID")) {
                references.entry(reference).or_default().push(row);
            }
        }
        let mut suggestions = Vec::new();
        let mut used_groups: BTreeSet<_> = allocations.iter().map(|a| a.group.clone()).collect();
        for (reference, rows) in references {
            if (2..=20).contains(&rows.len()) && total(rows.iter().copied()).is_zero() {
                let mut group = format!("S{}", suggestions.len() + 1);
                while used_groups.contains(&group) {
                    group = format!("new-{group}");
                }
                used_groups.insert(group.clone());
                suggestions.push(Json::object([
                    ("group", Json::str(&group)),
                    (
                        "ids",
                        Json::Array(
                            rows.iter()
                                .map(|row| row.at("TransactionID").clone())
                                .collect(),
                        ),
                    ),
                    (
                        "reason",
                        Json::str(&format!(
                            "Shared reference {}; total 0.00.",
                            pytext::repr(&reference)
                        )),
                    ),
                ]));
            }
        }
        let movement = total(&current);
        let opening_difference = total(&old).sub(&opening_balance).value;
        let closing_difference = opening_balance
            .add(&movement)
            .value
            .sub(&closing_balance)
            .value;
        let status = if !opening_difference.is_zero() || !closing_difference.is_zero() {
            "BLOCKED"
        } else if !outstanding.is_empty() {
            "REVIEW"
        } else {
            "PASS"
        };
        let mut sources = vec![(PyStr::from("transactions"), transactions)];
        if let Some(source) = opening_source {
            sources.push((PyStr::from("opening_items"), source));
        }
        if let Some(source) = decisions_source {
            sources.push((PyStr::from("decisions"), source));
        }
        let sources = Json::Object(
            sources
                .into_iter()
                .map(|(key, source)| {
                    (
                        key,
                        Json::object([
                            (
                                "file",
                                Json::Str(PyStr::from_os_text(&pypath::decode_os(
                                    source.path.file_name().unwrap_or_default(),
                                ))),
                            ),
                            ("sha256", Json::str(&source.sha256)),
                        ]),
                    )
                })
                .collect(),
        );
        Ok(Json::object([
            ("schema", Json::str("clearing-reconciliation-v1")),
            ("status", Json::str(status)),
            ("identity", identity),
            ("period_start", Json::str(&config.period_start)),
            ("period_end", Json::str(&config.period_end)),
            ("opening_balance", Json::str(&fixed(&opening_balance))),
            ("closing_balance", Json::str(&fixed(&closing_balance))),
            ("movement", Json::str(&fixed(&movement))),
            ("opening_difference", Json::str(&fixed(&opening_difference))),
            ("closing_difference", Json::str(&fixed(&closing_difference))),
            ("outstanding_total", Json::str(&fixed(&total(&outstanding)))),
            ("transactions", Json::Array(ordered)),
            (
                "opening_ids",
                Json::Array(old.iter().map(|r| r.at("TransactionID").clone()).collect()),
            ),
            ("outstanding", Json::Array(outstanding)),
            ("suggestions", Json::Array(suggestions)),
            (
                "decisions",
                Json::Array(allocations.iter().map(Allocation::json).collect()),
            ),
            ("notes", notes.into_json()),
            ("review_boundary", Json::str(BOUNDARY)),
            ("source_sha256", sources),
        ]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> ReconcileConfig {
        ReconcileConfig {
            transactions: "transactions.csv".into(),
            tenant: "Synthetic".into(),
            account_id: "CLEAR".into(),
            currency: "AUD".into(),
            period_start: "2026-09-01".into(),
            period_end: "2026-09-30".into(),
            opening_balance: "0".into(),
            closing_balance: "0".into(),
            opening_items: None,
            decisions: None,
            decisions_escaped: false,
        }
    }
    const TRANSACTIONS: &str = "Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit\nSynthetic,CLEAR,AUD,T1,2026-09-01,Shared,Synthetic debit,100,0\nSynthetic,CLEAR,AUD,T2,2026-09-02,Shared,Synthetic credit,0,100\n";
    #[test]
    fn suggestions_leave_items_outstanding_until_an_explicit_accepted_allocation() {
        let mut config = config();
        let mut capture = |path: &Path, _: &str| {
            Ok(SourceSnapshot::new(
                path,
                if path == Path::new("transactions.csv") {
                    TRANSACTIONS.as_bytes().to_vec()
                } else {
                    b"Group,TransactionID,Decision,Note\nG,T1,accept,Synthetic review\nG,T2,accept,Synthetic review\n".to_vec()
                },
            ))
        };
        let suggested = reconcile_with(&config, &mut capture).unwrap();
        assert!(suggested.at("status").is_str("REVIEW"));
        assert_eq!(suggested.at("outstanding").as_array().unwrap().len(), 2);
        assert_eq!(suggested.at("suggestions").as_array().unwrap().len(), 1);
        config.decisions = Some("decisions.csv".into());
        let accepted = reconcile_with(&config, &mut capture).unwrap();
        assert!(accepted.at("status").is_str("PASS"));
        assert!(accepted.at("outstanding").as_array().unwrap().is_empty());
    }
    #[test]
    fn conflicting_balances_block_and_decision_configuration_fails_before_source_reads() {
        let mut config = config();
        config.closing_balance = "0.01".into();
        let result = reconcile_with(&config, &mut |path, _| {
            Ok(SourceSnapshot::new(path, TRANSACTIONS.as_bytes().to_vec()))
        })
        .unwrap();
        assert!(result.at("status").is_str("BLOCKED"));
        config.decisions_escaped = true;
        let mut reads = 0;
        assert_eq!(
            reconcile_with(&config, &mut |_, _| {
                reads += 1;
                Err("unexpected read".into())
            })
            .unwrap_err(),
            "--decisions-escaped requires --decisions."
        );
        assert_eq!(reads, 0);
    }
    #[test]
    fn money_limits_preserve_cent_equivalence_and_private_precision() {
        decimal::with_precision(40, || {
            assert_eq!(
                fixed(
                    &money(
                        "$999,999,999,999,999.9900",
                        "amount",
                        Path::new("configuration"),
                        0
                    )
                    .unwrap()
                ),
                "999999999999999.99"
            );
            for raw in ["1000000000000000", "-1000000000000000", "0.001"] {
                assert!(money(raw, "amount", Path::new("configuration"), 0).is_err());
            }
        });
    }
    #[test]
    fn more_than_one_hundred_thousand_items_refuses_before_allocation() {
        let mut csv = COLUMNS.join(",") + "\n";
        for index in 0..100001 {
            use std::fmt::Write;
            writeln!(
                csv,
                "Synthetic,CLEAR,AUD,T{index},2026-09-01,R,Synthetic,1,0"
            )
            .unwrap();
        }
        let error = reconcile_with(&config(), &mut |path, _| {
            Ok(SourceSnapshot::new(path, csv.as_bytes().to_vec()))
        })
        .unwrap_err();
        assert_eq!(
            error,
            "A reconciliation supports at most 100,000 outstanding and current items."
        );
    }
    #[test]
    fn rendered_carry_retains_partial_groups_and_unrelated_net_zero_items() {
        let mut config = config();
        config.period_start = "2026-07-01".into();
        config.period_end = "2026-07-31".into();
        config.closing_balance = "43".into();
        config.decisions = Some("decisions.csv".into());
        let csv = COLUMNS.join(",") + "\n";
        let july = csv.clone()
            + "Synthetic,CLEAR,AUD,sale,2026-07-01,Partial,Sale,100,0\nSynthetic,CLEAR,AUD,part,2026-07-02,Partial,Payment,0,60\nSynthetic,CLEAR,AUD,original,2026-07-03,Reversal,Original,20,0\nSynthetic,CLEAR,AUD,reversal,2026-07-04,Reversal,Reversal,0,20\nSynthetic,CLEAR,AUD,batch,2026-07-05,Batch,Receivable,200,0\nSynthetic,CLEAR,AUD,payout,2026-07-06,Batch,Payout,0,197\nSynthetic,CLEAR,AUD,unrelated-dr,2026-07-10,Customer-A,Receipt,50,0\nSynthetic,CLEAR,AUD,unrelated-cr,2026-07-10,Customer-B,Payment,0,50\n";
        let july_decisions = "Group,TransactionID,Decision,Note\nreversed,original,accept,Checked reversal\nreversed,reversal,accept,Checked reversal\n";
        let first = reconcile_with(&config, &mut |path, _| {
            Ok(SourceSnapshot::new(
                path,
                if path == Path::new("transactions.csv") {
                    july.as_bytes()
                } else {
                    july_decisions.as_bytes()
                }
                .to_vec(),
            ))
        })
        .unwrap();
        assert!(first.at("status").is_str("REVIEW"));
        assert_eq!(first.at("outstanding").as_array().unwrap().len(), 6);
        let carry = crate::reconciliation_report::rendered(&first)
            .unwrap()
            .into_iter()
            .find(|(name, _)| name == "carry-forward.json")
            .unwrap()
            .1;
        config.period_start = "2026-08-01".into();
        config.period_end = "2026-08-31".into();
        config.opening_balance = "43".into();
        config.closing_balance = "0".into();
        config.opening_items = Some("opening.json".into());
        let august = csv
            + "Synthetic,CLEAR,AUD,final,2026-08-02,Partial,Final payment,0,40\nSynthetic,CLEAR,AUD,fee,2026-08-03,Batch,Fee already posted,0,3\n";
        let decisions = "Group,TransactionID,Decision,Note\nsettled,sale,accept,Checked payments\nsettled,part,accept,Checked payments\nsettled,final,accept,Checked payments\nbatch,batch,accept,Checked source fee\nbatch,payout,accept,Checked source fee\nbatch,fee,accept,Checked source fee\n";
        let second = reconcile_with(&config, &mut |path, _| {
            Ok(SourceSnapshot::new(
                path,
                if path == Path::new("transactions.csv") {
                    august.as_bytes()
                } else if path == Path::new("opening.json") {
                    &carry
                } else {
                    decisions.as_bytes()
                }
                .to_vec(),
            ))
        })
        .unwrap();
        assert!(second.at("status").is_str("REVIEW"));
        assert!(second.at("outstanding_total").is_str("0.00"));
        let outstanding = second.at("outstanding").as_array().unwrap();
        assert_eq!(outstanding.len(), 2);
        assert!(outstanding[0].at("TransactionID").is_str("unrelated-cr"));
        assert!(outstanding[1].at("TransactionID").is_str("unrelated-dr"));
        let rendered = crate::reconciliation_report::rendered(&second).unwrap();
        let csv = &rendered
            .iter()
            .find(|(name, _)| name == "outstanding.csv")
            .unwrap()
            .1;
        assert_eq!(
            String::from_utf8(csv.clone())
                .unwrap()
                .matches(",52,")
                .count(),
            2
        );
    }
}
