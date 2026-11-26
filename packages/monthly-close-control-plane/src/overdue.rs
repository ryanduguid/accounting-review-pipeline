// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Open receivables overdue against the customer's own payment history: days
//! past due compared with how late that customer has paid before, not with a
//! fixed ageing bucket.
use crate::{
    Result,
    decimal::{self, Decimal},
    duplicates,
    isodate::{self, Date},
    loader::{self, CsvRow, SourceSnapshot},
    pyjson::Json,
    pytext, reconciliation,
};
use std::{collections::BTreeMap, path::Path};

pub const COLUMNS: [&str; 8] = [
    "Tenant",
    "InvoiceID",
    "Customer",
    "IssueDate",
    "DueDate",
    "PaidDate",
    "Amount",
    "Currency",
];
pub const BOUNDARY: &str = "Review evidence only. A listed invoice is later than its customer usually pays; it is not shown to be doubtful or impaired, and an invoice not listed is not shown to be collectable.";
/// What an overdue invoice is found to be, in the order they are counted.
const CODES: [&str; 3] = [
    "OVERDUE_AGAINST_HISTORY",
    "NOT_ENOUGH_HISTORY",
    "WITHIN_HISTORY",
];
/// Paid invoices a customer needs before an overdue invoice is judged.
pub const MINIMUM_HISTORY: usize = 3;
/// Multiples of the customer's spread allowed past their median.
pub const SPREAD_MULTIPLE: i64 = 3;
/// Days allowed past the median however steady the customer is.
pub const MINIMUM_MARGIN_DAYS: i64 = 7;

struct Invoice {
    id: String,
    customer: String,
    due: Date,
    paid: Option<Date>,
    amount: Decimal,
}

/// Signed days from `from` to `to`.
fn days(from: Date, to: Date) -> i64 {
    i64::from(isodate::ordinal(to) - isodate::ordinal(from))
}

fn load(source: &SourceSnapshot, currency: &str, as_at: Date) -> Result<(String, Vec<Invoice>)> {
    let path = crate::pypath::text(&source.path);
    let mut tenant = None;
    let mut invoices = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for row in loader::csv_rows(source, &COLUMNS, "Invoices")? {
        let row = row?;
        let name = row.text("Tenant", false)?;
        if *tenant.get_or_insert_with(|| name.clone()) != name {
            return Err(
                "Invoices must belong to one tenant; review each tenant separately.".into(),
            );
        }
        let refuse = |what: String| Err(format!("{path}: row {} {what}", row.number));
        if row.text("Currency", false)? != currency {
            return refuse(format!(
                "is not in the declared --currency {}.",
                pytext::repr(currency)
            ));
        }
        let id = row.text("InvoiceID", false)?;
        if !seen.insert(id.clone()) {
            return refuse(format!("repeats InvoiceID {}.", pytext::repr(&id)));
        }
        invoices.push(
            invoice(&row, id, as_at)
                .map_err(|what| format!("{path}: row {} {what}", row.number))?,
        );
    }
    Ok((tenant.unwrap_or_default(), invoices))
}

/// One register row's invoice, or the reason it is refused.
fn invoice(row: &CsvRow, id: String, as_at: Date) -> Result<Invoice> {
    let amount =
        reconciliation::money(&row.text("Amount", false)?, "Amount", &row.path, row.number)?;
    if amount <= Decimal::zero() {
        return Err("needs a positive Amount; leave credit notes out.".into());
    }
    let issued = row.date("IssueDate")?;
    let due = row.date("DueDate")?;
    let paid = match row.text("PaidDate", true)?.as_str() {
        "" => None,
        _ => Some(row.date("PaidDate")?),
    };
    if issued > as_at {
        return Err("is issued after --as-at; supply invoices issued by then.".into());
    }
    if due < issued || paid.is_some_and(|paid| paid < issued) {
        return Err("has a DueDate or PaidDate before its IssueDate.".into());
    }
    Ok(Invoice {
        id,
        customer: row.text("Customer", false)?,
        due,
        paid,
        amount,
    })
}

/// The median of `values`, which must not be empty.
fn median(values: &mut [Decimal]) -> Decimal {
    values.sort_by(Decimal::cmp_value);
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        return values[middle].clone();
    }
    let sum = values[middle - 1].add(&values[middle]).value;
    sum.div(&Decimal::parse("2").unwrap())
}

fn whole(value: i64) -> Decimal {
    Decimal::parse(&value.to_string()).unwrap()
}

fn halves(value: &Decimal) -> String {
    value.format_places(1).expect("bounded day count")
}

/// One customer's paid history at the as-at date, with the median days late,
/// the spread and the threshold when anything has been paid.
struct History<'a> {
    paid: Vec<&'a Invoice>,
    days: Option<[Decimal; 3]>,
}

/// Days late of each invoice paid by `as_at`, the median, the median absolute
/// deviation from it (the spread) and the days overdue above which an open
/// invoice is listed.
fn history<'a>(paid: Vec<&'a Invoice>) -> History<'a> {
    if paid.is_empty() {
        return History { paid, days: None };
    }
    let mut late: Vec<Decimal> = paid
        .iter()
        .map(|invoice| whole(days(invoice.due, invoice.paid.unwrap())))
        .collect();
    let middle = median(&mut late);
    let mut deviations: Vec<Decimal> = late
        .iter()
        .map(|days| days.sub(&middle).value.copy_abs())
        .collect();
    let spread = median(&mut deviations);
    let allowed = spread.mul(&whole(SPREAD_MULTIPLE));
    let minimum = whole(MINIMUM_MARGIN_DAYS);
    let margin = if allowed > minimum { allowed } else { minimum };
    let threshold = middle.add(&margin).value;
    History {
        paid,
        days: Some([middle, spread, threshold]),
    }
}

fn history_json(history: &History) -> Json {
    let mut entry = Json::object([
        ("paid_invoices", Json::int(history.paid.len() as i64)),
        (
            "paid_invoice_ids",
            Json::Array(
                history
                    .paid
                    .iter()
                    .map(|invoice| Json::str(&invoice.id))
                    .collect(),
            ),
        ),
    ]);
    let names = ["median_days_late", "spread_days", "threshold_days"];
    for (index, name) in names.into_iter().enumerate() {
        let value = history
            .days
            .as_ref()
            .map_or(Json::Null, |days| Json::str(&halves(&days[index])));
        entry.set(name, value);
    }
    entry
}

/// Review the invoice register in `path` as at `as_at`.
pub fn overdue_against_history(path: &Path, currency: &str, as_at: &str) -> Result<Json> {
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err("Declare a three-letter currency, such as AUD.".into());
    }
    let as_at = reconciliation::date(as_at).map_err(|error| format!("--as-at: {error}"))?;
    let source = SourceSnapshot::capture(path, "Invoices")?;
    decimal::with_precision(40, || review(&source, currency, as_at))
}

/// Each customer's history, keyed by the customer's matching name.
fn histories(invoices: &[Invoice], as_at: Date) -> BTreeMap<String, History<'_>> {
    let mut paid: BTreeMap<String, Vec<&Invoice>> = BTreeMap::new();
    for invoice in invoices {
        let entry = paid
            .entry(duplicates::supplier_key(&invoice.customer))
            .or_default();
        if invoice.paid.is_some_and(|day| day <= as_at) {
            entry.push(invoice);
        }
    }
    paid.into_iter()
        .map(|(key, paid)| (key, history(paid)))
        .collect()
}

/// What an overdue invoice is found to be against its customer's history.
fn judged(overdue: i64, history: &History) -> &'static str {
    if history.paid.len() < MINIMUM_HISTORY {
        "NOT_ENOUGH_HISTORY"
    } else if history
        .days
        .as_ref()
        .is_some_and(|days| whole(overdue) > days[2])
    {
        "OVERDUE_AGAINST_HISTORY"
    } else {
        "WITHIN_HISTORY"
    }
}

/// The fields the result opens with.
fn header(
    status: &str,
    tenant: &str,
    currency: &str,
    as_at: Date,
    source: &SourceSnapshot,
) -> Json {
    Json::object([
        ("schema_version", Json::int(1)),
        ("status", Json::str(status)),
        ("tenant", Json::str(tenant)),
        ("currency", Json::str(currency)),
        ("as_at", Json::str(&as_at.iso())),
        ("boundary", Json::str(BOUNDARY)),
        ("source_sha256", Json::str(&source.sha256)),
        (
            "rule",
            Json::object([
                ("minimum_history", Json::int(MINIMUM_HISTORY as i64)),
                ("spread_multiple", Json::int(SPREAD_MULTIPLE)),
                ("minimum_margin_days", Json::int(MINIMUM_MARGIN_DAYS)),
            ]),
        ),
    ])
}

fn review(source: &SourceSnapshot, currency: &str, as_at: Date) -> Result<Json> {
    let (tenant, invoices) = load(source, currency, as_at)?;
    let histories = histories(&invoices, as_at);
    let mut listed = Vec::new();
    let mut counts: BTreeMap<&str, i64> = CODES.map(|code| (code, 0)).into();
    let mut open = 0;
    for invoice in &invoices {
        if invoice.paid.is_some_and(|paid| paid <= as_at) {
            continue;
        }
        open += 1;
        let overdue = days(invoice.due, as_at);
        if overdue <= 0 {
            continue;
        }
        let history = &histories[&duplicates::supplier_key(&invoice.customer)];
        let code = judged(overdue, history);
        *counts.get_mut(code).unwrap() += 1;
        listed.push(Json::object([
            ("code", Json::str(code)),
            ("invoice_id", Json::str(&invoice.id)),
            ("customer", Json::str(&invoice.customer)),
            ("amount", Json::str(&reconciliation::fixed(&invoice.amount))),
            ("due_date", Json::str(&invoice.due.iso())),
            ("days_overdue", Json::int(overdue)),
            ("history", history_json(history)),
        ]));
    }
    let status = if counts["NOT_ENOUGH_HISTORY"] > 0 {
        "BLOCKED"
    } else if counts["OVERDUE_AGAINST_HISTORY"] > 0 {
        "REVIEW"
    } else {
        "PASS"
    };
    let mut result = header(status, &tenant, currency, as_at, source);
    for (key, value) in [
        ("invoice_count", Json::int(invoices.len() as i64)),
        ("open_count", Json::int(open)),
        (
            "counts",
            Json::object(CODES.map(|code| (code, Json::int(counts[code])))),
        ),
        ("overdue", Json::Array(listed)),
    ] {
        result.set(key, value);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::review;
    use crate::{
        isodate::Date,
        loader::SourceSnapshot,
        pyjson::{self, Json},
    };
    use std::path::Path;

    const HEAD: &str = "Tenant,InvoiceID,Customer,IssueDate,DueDate,PaidDate,Amount,Currency\n";

    fn run(rows: &str) -> crate::Result<Json> {
        let source = SourceSnapshot::new(Path::new("r.csv"), format!("{HEAD}{rows}").into_bytes());
        review(&source, "AUD", Date::new(2026, 9, 30).unwrap())
    }

    fn text(value: &Json) -> String {
        value.as_pystr().unwrap().to_utf8().unwrap()
    }

    /// Paid invoices for `customer`, each due on the 1st of a month from
    /// April and paid the given days late.
    fn paid(customer: &str, late: &[u32]) -> String {
        late.iter()
            .enumerate()
            .map(|(index, days)| {
                let month = 4 + index as u32;
                format!(
                    "T,P-{customer}-{index},{customer},2026-0{}-01,2026-0{month}-01,2026-0{month}-{:02},100.00,AUD\n",
                    month - 1,
                    1 + days
                )
            })
            .collect()
    }

    fn only(result: &Json) -> &Json {
        let listed = result.at("overdue").as_array().unwrap();
        assert_eq!(listed.len(), 1, "{}", pyjson::canonical(result));
        &listed[0]
    }

    #[test]
    fn an_even_history_takes_the_mean_of_the_middle_pair() {
        let rows =
            paid("Steady", &[2, 3, 6, 9]) + "T,O-1,Steady,2026-08-01,2026-09-01,,50.00,AUD\n";
        let result = run(&rows).unwrap();
        let entry = only(&result);
        let history = entry.at("history");
        assert_eq!(text(history.at("median_days_late")), "4.5");
        assert_eq!(text(history.at("spread_days")), "2.0");
        assert_eq!(text(history.at("threshold_days")), "11.5");
        assert!(entry.at("code").is_str("OVERDUE_AGAINST_HISTORY"));
        assert!(result.at("status").is_str("REVIEW"));
    }

    #[test]
    fn a_steady_payer_gets_the_minimum_margin_and_a_slow_one_its_spread() {
        let steady = paid("Prompt", &[0, 0, 0]) + "T,O-1,Prompt,2026-08-20,2026-09-23,,50.00,AUD\n";
        let entry_result = run(&steady).unwrap();
        let entry = only(&entry_result);
        assert_eq!(text(entry.at("history").at("threshold_days")), "7.0");
        assert!(entry.at("code").is_str("WITHIN_HISTORY"));
        assert!(entry_result.at("status").is_str("PASS"));
        let slow = paid("Slow", &[10, 20, 25]) + "T,O-1,Slow,2026-08-01,2026-09-01,,50.00,AUD\n";
        let result = run(&slow).unwrap();
        let entry = only(&result);
        assert_eq!(text(entry.at("history").at("threshold_days")), "35.0");
        assert!(entry.at("code").is_str("WITHIN_HISTORY"));
    }

    #[test]
    fn customer_names_match_ignoring_case_and_spacing() {
        let rows = paid("Early", &[0, 0, 0]).replace(",Early,", ",early  ,")
            + "T,O-1,EARLY,2026-08-01,2026-09-01,,50.00,AUD\n";
        let result = run(&rows).unwrap();
        let entry = only(&result);
        assert_eq!(
            pyjson::canonical(entry.at("history").at("paid_invoices")),
            "3"
        );
    }

    #[test]
    fn too_little_history_blocks_and_payments_after_as_at_are_open() {
        let rows = "T,A-1,New,2026-07-01,2026-08-01,2026-10-02,50.00,AUD\nT,A-2,New,2026-07-01,2026-07-15,2026-07-20,50.00,AUD\n";
        let result = run(rows).unwrap();
        let entry = only(&result);
        assert!(entry.at("code").is_str("NOT_ENOUGH_HISTORY"));
        assert_eq!(pyjson::canonical(entry.at("days_overdue")), "60");
        assert_eq!(
            pyjson::canonical(entry.at("history").at("paid_invoices")),
            "1"
        );
        assert!(result.at("status").is_str("BLOCKED"));
        assert_eq!(pyjson::canonical(result.at("open_count")), "1");
    }

    #[test]
    fn a_customer_with_nothing_paid_has_no_history_figures() {
        let result = run("T,A-1,New,2026-07-01,2026-08-01,,50.00,AUD
")
        .unwrap();
        let history = only(&result).at("history");
        assert!(matches!(history.at("median_days_late"), Json::Null));
        assert!(matches!(history.at("threshold_days"), Json::Null));
    }

    #[test]
    fn an_early_payer_is_judged_from_a_negative_median() {
        let rows = "T,E-1,Early,2026-05-01,2026-06-10,2026-06-05,10.00,AUD
                    T,E-2,Early,2026-06-01,2026-07-10,2026-07-06,10.00,AUD
                    T,E-3,Early,2026-07-01,2026-08-10,2026-08-04,10.00,AUD
                    T,E-4,Early,2026-08-20,2026-09-27,,10.00,AUD
                    T,E-5,Early,2026-08-30,2026-09-30,,10.00,AUD
";
        let result = run(rows).unwrap();
        let entry = only(&result);
        assert_eq!(text(entry.at("invoice_id")), "E-4");
        assert_eq!(text(entry.at("history").at("median_days_late")), "-5.0");
        assert_eq!(text(entry.at("history").at("threshold_days")), "2.0");
        assert!(entry.at("code").is_str("OVERDUE_AGAINST_HISTORY"));
        assert_eq!(pyjson::canonical(result.at("open_count")), "2");
    }

    #[test]
    fn a_lower_case_currency_is_refused_before_reading() {
        let error = super::overdue_against_history(Path::new("none.csv"), "aud", "2026-09-30")
            .unwrap_err()
            .to_string();
        assert!(error.contains("three-letter currency"), "{error}");
    }

    #[test]
    fn bad_rows_are_refused() {
        for (row, expected) in [
            (
                "T,X,C,2026-07-01,2026-06-01,,1.00,AUD\n",
                "before its IssueDate",
            ),
            (
                "T,X,C,2026-07-01,2026-08-01,2026-06-01,1.00,AUD\n",
                "before its IssueDate",
            ),
            ("T,X,C,2026-10-01,2026-11-01,,1.00,AUD\n", "after --as-at"),
            (
                "T,X,C,2026-07-01,2026-08-01,,-1.00,AUD\n",
                "positive Amount",
            ),
            ("T,X,C,2026-07-01,2026-08-01,,1.00,NZD\n", "--currency"),
            (
                "T,X,C,2026-07-01,2026-08-01,,1.00,AUD\nT,X,C,2026-07-01,2026-08-01,,1.00,AUD\n",
                "repeats InvoiceID",
            ),
            (
                "T,X,C,2026-07-01,2026-08-01,,1.00,AUD\nU,Y,C,2026-07-01,2026-08-01,,1.00,AUD\n",
                "one tenant",
            ),
        ] {
            let error = run(row).unwrap_err().to_string();
            assert!(error.contains(expected), "{row}: {error}");
        }
    }
}
