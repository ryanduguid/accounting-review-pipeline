// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Possible duplicate bills and payments, and recurring supplier bills
//! missing from the period, in a supplied register of bills and payments.
use crate::{
    Result,
    decimal::{self, Decimal},
    isodate::{self, Date},
    loader::{self, SourceSnapshot},
    pyjson::Json,
    pytext, reconciliation,
};
use std::{collections::BTreeMap, path::Path};

pub const COLUMNS: [&str; 8] = [
    "Tenant",
    "DocumentID",
    "Kind",
    "Supplier",
    "Reference",
    "Date",
    "Amount",
    "Currency",
];
pub const BOUNDARY: &str = "Review evidence only. A listed group is worth a reviewer's look; it does not show that a bill or payment is a duplicate or missing, and a register with nothing listed is not shown to be complete or free of duplicates.";
/// Bills or payments to one supplier for the same amount this many days
/// apart or closer are listed together.
pub const WINDOW_DAYS: i32 = 7;
/// The calendar months before the period in which a supplier must have been
/// billed every month for a bill to be expected in the period.
pub const RECURRING_MONTHS: i32 = 3;

struct Document {
    id: String,
    kind: String,
    supplier: String,
    reference: String,
    date: Date,
    amount: Decimal,
}

/// Supplier names compare once case and whitespace are ignored.
fn supplier_key(name: &str) -> String {
    pytext::casefold(name).split_whitespace().collect()
}

/// References compare ignoring case, whitespace and ASCII punctuation;
/// every other character, symbols and combining marks included, counts.
fn reference_key(reference: &str) -> String {
    pytext::casefold(reference)
        .chars()
        .filter(|ch| !ch.is_whitespace() && !ch.is_ascii_punctuation())
        .collect()
}

fn month_index(day: Date) -> i32 {
    day.year * 12 + day.month as i32 - 1
}

fn load(source: &SourceSnapshot, currency: &str) -> Result<(String, Vec<Document>)> {
    let path = crate::pypath::text(&source.path);
    let mut tenant = None;
    let mut documents = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for row in loader::csv_rows(source, &COLUMNS, "Documents")? {
        let row = row?;
        let text = |field| row.text(field, field == "Reference");
        let name = text("Tenant")?;
        if *tenant.get_or_insert_with(|| name.clone()) != name {
            return Err(
                "Documents must belong to one tenant; review each tenant separately.".into(),
            );
        }
        if text("Currency")? != currency {
            return Err(format!(
                "{path}: row {} is not in the declared --currency {}.",
                row.number,
                pytext::repr(currency)
            ));
        }
        let refuse = |what: String| Err(format!("{path}: row {} {what}", row.number));
        let kind = text("Kind")?;
        if !["BILL", "PAYMENT"].contains(&kind.as_str()) {
            return refuse("Kind must be BILL or PAYMENT.".into());
        }
        let id = text("DocumentID")?;
        if !seen.insert(id.clone()) {
            return refuse(format!("repeats DocumentID {}.", pytext::repr(&id)));
        }
        let amount = reconciliation::money(&text("Amount")?, "Amount", &source.path, row.number)?;
        if amount <= Decimal::zero() {
            return refuse("needs a positive Amount.".into());
        }
        documents.push(Document {
            id,
            kind,
            supplier: text("Supplier")?,
            reference: text("Reference")?,
            date: row.date("Date")?,
            amount,
        });
    }
    Ok((tenant.unwrap_or_default(), documents))
}

fn group_json(code: &str, members: &[&Document]) -> Json {
    let first = members[0];
    Json::object([
        ("code", Json::str(code)),
        ("supplier", Json::str(&first.supplier)),
        (
            "documents",
            Json::Array(
                members
                    .iter()
                    .map(|document| {
                        Json::object([
                            ("document_id", Json::str(&document.id)),
                            ("kind", Json::str(&document.kind)),
                            ("reference", Json::str(&document.reference)),
                            ("date", Json::str(&document.date.iso())),
                            (
                                "amount",
                                Json::str(&reconciliation::fixed(&document.amount)),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// Groups of 2 or more documents that share `key`, at least one of them in
/// the period.
fn same_key<'a>(
    documents: &[&'a Document],
    key: impl Fn(&Document) -> Option<String>,
    start: Date,
) -> Vec<Vec<&'a Document>> {
    let mut groups: BTreeMap<String, Vec<&Document>> = BTreeMap::new();
    for document in documents {
        if let Some(value) = key(document) {
            groups.entry(value).or_default().push(document);
        }
    }
    groups
        .into_values()
        .filter(|group| group.len() > 1 && group.iter().any(|document| document.date >= start))
        .collect()
}

/// Runs of one supplier's documents for one amount, each within
/// `WINDOW_DAYS` of the one before, with at least one in the period.
fn close_amounts<'a>(documents: &[&'a Document], start: Date) -> Vec<Vec<&'a Document>> {
    let mut runs = Vec::new();
    let amount = |document: &Document| {
        Some(format!(
            "{}|{}",
            supplier_key(&document.supplier),
            reconciliation::fixed(&document.amount)
        ))
    };
    for mut group in same_key(documents, amount, start) {
        group.sort_by(|a, b| (a.date, &a.id).cmp(&(b.date, &b.id)));
        let mut run: Vec<&Document> = Vec::new();
        for document in group {
            let near = run.last().is_some_and(|last| {
                isodate::ordinal(document.date) - isodate::ordinal(last.date) <= WINDOW_DAYS
            });
            if !near {
                runs.push(std::mem::take(&mut run));
            }
            run.push(document);
        }
        runs.push(run);
    }
    runs.retain(|run| run.len() > 1 && run.iter().any(|document| document.date >= start));
    runs
}

/// Suppliers billed in each of the `RECURRING_MONTHS` months before the
/// period's first month and not billed in the period.
fn missing_recurring(bills: &[&Document], start: Date, end: Date) -> Vec<Json> {
    let first = month_index(start);
    let mut months: BTreeMap<String, (String, std::collections::BTreeSet<i32>, bool)> =
        BTreeMap::new();
    for bill in bills {
        let entry = months
            .entry(supplier_key(&bill.supplier))
            .or_insert_with(|| (bill.supplier.clone(), Default::default(), false));
        entry.1.insert(month_index(bill.date));
        entry.2 |= start <= bill.date && bill.date <= end;
    }
    months
        .into_values()
        .filter(|(_, seen, billed)| {
            !billed && (first - RECURRING_MONTHS..first).all(|month| seen.contains(&month))
        })
        .map(|(supplier, _, _)| {
            Json::object([
                ("code", Json::str("MISSING_RECURRING_BILL")),
                ("supplier", Json::str(&supplier)),
                ("documents", Json::Array(Vec::new())),
            ])
        })
        .collect()
}

/// The `--currency`, `--period-start` and `--period-end` options of a
/// one-tenant register command, checked before anything is read.
pub(crate) fn period_options(currency: &str, start: &str, end: &str) -> Result<(Date, Date)> {
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err("Declare a three-letter currency, such as AUD.".into());
    }
    let start = reconciliation::date(start).map_err(|error| format!("--period-start: {error}"))?;
    let end = reconciliation::date(end).map_err(|error| format!("--period-end: {error}"))?;
    if start > end {
        return Err("--period-start must not follow --period-end.".into());
    }
    Ok((start, end))
}

/// The fields every one-tenant register result opens with.
pub(crate) fn period_result(
    passed: bool,
    tenant: &str,
    currency: &str,
    (start, end): (Date, Date),
    boundary: &str,
) -> Json {
    Json::object([
        ("schema_version", Json::int(1)),
        ("status", Json::str(if passed { "PASS" } else { "REVIEW" })),
        ("tenant", Json::str(tenant)),
        ("currency", Json::str(currency)),
        ("period_start", Json::str(&start.iso())),
        ("period_end", Json::str(&end.iso())),
        ("boundary", Json::str(boundary)),
    ])
}

/// Review the register in `path` for the period `start` to `end`.
pub fn duplicate_documents(path: &Path, currency: &str, start: &str, end: &str) -> Result<Json> {
    let (start, end) = period_options(currency, start, end)?;
    let source = SourceSnapshot::capture(path, "Documents")?;
    decimal::with_precision(40, || review(&source, currency, start, end))
}

fn review(source: &SourceSnapshot, currency: &str, start: Date, end: Date) -> Result<Json> {
    let (tenant, documents) = load(source, currency)?;
    if let Some(late) = documents.iter().find(|document| document.date > end) {
        return Err(format!(
            "DocumentID {} is dated after --period-end; supply documents to the period end only.",
            pytext::repr(&late.id)
        ));
    }
    let of_kind = |kind: &str| -> Vec<&Document> {
        documents
            .iter()
            .filter(|document| document.kind == kind)
            .collect()
    };
    let (bills, payments) = (of_kind("BILL"), of_kind("PAYMENT"));
    let reference = |document: &Document| {
        let key = reference_key(&document.reference);
        (!key.is_empty()).then(|| format!("{}|{key}", supplier_key(&document.supplier)))
    };
    let mut findings = Vec::new();
    for (code, groups) in [
        ("DUPLICATE_REFERENCE", same_key(&bills, reference, start)),
        ("DUPLICATE_BILL", close_amounts(&bills, start)),
        ("DUPLICATE_PAYMENT", close_amounts(&payments, start)),
    ] {
        findings.extend(groups.iter().map(|group| group_json(code, group)));
    }
    findings.extend(missing_recurring(&bills, start, end));
    let in_period = documents
        .iter()
        .filter(|document| document.date >= start)
        .count();
    let passed = in_period > 0 && findings.is_empty();
    let mut result = period_result(passed, &tenant, currency, (start, end), BOUNDARY);
    for (key, value) in [
        (
            "source_sha256",
            Json::object([("documents", Json::str(&source.sha256))]),
        ),
        ("document_count", Json::int(documents.len() as i64)),
        ("documents_in_period", Json::int(in_period as i64)),
        ("window_days", Json::int(i64::from(WINDOW_DAYS))),
        ("findings", Json::Array(findings)),
    ] {
        result.set(key, value);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{pyjson, test_support::TempDir};

    const HEADER: &str = "Tenant,DocumentID,Kind,Supplier,Reference,Date,Amount,Currency\n";
    type Row<'a> = (&'a str, &'a str, &'a str, &'a str, &'a str, &'a str);

    fn register(rows: &[Row]) -> String {
        let mut text = HEADER.to_owned();
        for (id, kind, supplier, reference, date, amount) in rows {
            text += &format!(
                "Fabricated Varrock,{id},{kind},{supplier},{reference},{date},{amount},AUD\n"
            );
        }
        text
    }

    fn run_text(text: &str) -> Result<Json> {
        let source = SourceSnapshot::new(Path::new("documents.csv"), text.as_bytes().to_vec());
        let start = Date::new(2026, 9, 1).unwrap();
        let end = Date::new(2026, 9, 30).unwrap();
        decimal::with_precision(40, || review(&source, "AUD", start, end))
    }

    fn run(rows: &[Row]) -> Json {
        run_text(&register(rows)).unwrap()
    }

    fn text(value: &Json) -> String {
        value.as_pystr().unwrap().to_utf8().unwrap()
    }

    /// Each finding's code, supplier and the documents it lists.
    fn listed(result: &Json) -> Vec<(String, String, Vec<String>)> {
        result
            .at("findings")
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| {
                let documents = finding.at("documents").as_array().unwrap();
                (
                    text(finding.at("code")),
                    text(finding.at("supplier")),
                    documents
                        .iter()
                        .map(|item| text(item.at("document_id")))
                        .collect(),
                )
            })
            .collect()
    }

    fn found(code: &str, supplier: &str, ids: &[&str]) -> (String, String, Vec<String>) {
        (
            code.into(),
            supplier.into(),
            ids.iter().map(|&id| id.into()).collect(),
        )
    }

    #[test]
    fn a_clean_register_passes() {
        let result = run(&[
            (
                "B1",
                "BILL",
                "Falador Freight",
                "FF-1",
                "2026-09-03",
                "10.00",
            ),
            (
                "B2",
                "BILL",
                "Falador Freight",
                "FF-2",
                "2026-09-20",
                "10.00",
            ),
            (
                "P1",
                "PAYMENT",
                "Falador Freight",
                "",
                "2026-09-10",
                "10.00",
            ),
            (
                "B3",
                "BILL",
                "Draynor Supplies",
                "DS-1",
                "2026-07-03",
                "5.00",
            ),
        ]);
        assert!(result.at("status").is_str("PASS"));
        assert_eq!(pyjson::canonical(result.at("documents_in_period")), "3");
        assert!(listed(&result).is_empty());
    }

    #[test]
    #[expect(clippy::too_many_lines, reason = "one fabricated row per line")]
    fn references_compare_on_letters_and_digits_for_one_supplier() {
        let result = run(&[
            (
                "B1",
                "BILL",
                "Falador Freight",
                "FF-7781",
                "2026-09-03",
                "10.00",
            ),
            (
                "B2",
                "BILL",
                "falador  FREIGHT",
                "ff 7781",
                "2026-09-28",
                "12.00",
            ),
            (
                "B3",
                "BILL",
                "Draynor Supplies",
                "FF-7781",
                "2026-09-04",
                "11.00",
            ),
            ("B4", "BILL", "Draynor Supplies", "", "2026-09-05", "13.00"),
            ("B5", "BILL", "Draynor Supplies", "-", "2026-09-25", "14.00"),
            (
                "B6",
                "BILL",
                "Draynor Supplies",
                "OLD-1",
                "2026-07-01",
                "15.00",
            ),
            (
                "B7",
                "BILL",
                "Draynor Supplies",
                "old 1",
                "2026-07-20",
                "16.00",
            ),
        ]);
        assert_eq!(
            listed(&result),
            [found(
                "DUPLICATE_REFERENCE",
                "Falador Freight",
                &["B1", "B2"]
            )]
        );
        assert!(result.at("status").is_str("REVIEW"));
    }

    #[test]
    fn symbols_and_accents_in_references_count() {
        let result = run(&[
            (
                "B1",
                "BILL",
                "Falador Freight",
                "\u{1f527}",
                "2026-09-01",
                "1.00",
            ),
            (
                "B2",
                "BILL",
                "Falador Freight",
                "\u{1f527}",
                "2026-09-20",
                "2.00",
            ),
            ("B3", "BILL", "Draynor", "e\u{301}1", "2026-09-02", "3.00"),
            ("B4", "BILL", "Draynor", "e1", "2026-09-21", "4.00"),
        ]);
        assert_eq!(
            listed(&result),
            [found(
                "DUPLICATE_REFERENCE",
                "Falador Freight",
                &["B1", "B2"]
            )]
        );
        assert_ne!(reference_key("A\u{2013}1"), reference_key("A-1"));
    }

    #[test]
    fn equal_amounts_chain_within_the_window() {
        let result = run(&[
            ("B1", "BILL", "Draynor", "1", "2026-08-20", "9.50"),
            ("B2", "BILL", "Draynor", "2", "2026-08-27", "9.5"),
            ("B3", "BILL", "Draynor", "3", "2026-09-03", "9.50"),
            ("B4", "BILL", "Draynor", "4", "2026-09-11", "9.50"),
            ("B5", "BILL", "Draynor", "5", "2026-08-01", "7.00"),
            ("B6", "BILL", "Draynor", "6", "2026-08-05", "7.00"),
            ("B9", "BILL", "Draynor", "9", "2026-09-20", "7.00"),
            ("B7", "BILL", "Draynor", "7", "2026-09-12", "8.00"),
            ("B8", "BILL", "Draynor", "8", "2026-09-30", "8.00"),
            ("P1", "PAYMENT", "Draynor", "1", "2026-09-29", "9.50"),
            ("P2", "PAYMENT", "draynor", "1", "2026-09-30", "9.50"),
        ]);
        assert_eq!(
            listed(&result),
            [
                found("DUPLICATE_BILL", "Draynor", &["B1", "B2", "B3"]),
                found("DUPLICATE_PAYMENT", "Draynor", &["P1", "P2"]),
            ]
        );
    }

    #[test]
    fn a_supplier_billed_each_prior_month_needs_a_bill_in_the_period() {
        let result = run(&[
            ("L6", "BILL", "Lumbridge Power", "L6", "2026-06-12", "1.00"),
            ("L7", "BILL", "Lumbridge Power", "L7", "2026-07-11", "2.00"),
            ("L8", "BILL", "lumbridge power", "L8", "2026-08-12", "3.00"),
            ("R6", "BILL", "Varrock Rent", "R6", "2026-06-01", "4.00"),
            ("R7", "BILL", "Varrock Rent", "R7", "2026-07-01", "5.00"),
            ("R8", "BILL", "Varrock Rent", "R8", "2026-08-01", "6.00"),
            ("R9", "BILL", "Varrock Rent", "R9", "2026-09-01", "7.00"),
            ("G6", "BILL", "Gap Co", "G6", "2026-06-01", "8.00"),
            ("G8", "BILL", "Gap Co", "G8", "2026-08-01", "9.00"),
            ("P8", "PAYMENT", "Paid Co", "P6", "2026-06-01", "10.00"),
            ("P7", "PAYMENT", "Paid Co", "P7", "2026-07-01", "11.00"),
            ("P6", "PAYMENT", "Paid Co", "P8", "2026-08-01", "12.00"),
        ]);
        assert_eq!(
            listed(&result),
            [found("MISSING_RECURRING_BILL", "Lumbridge Power", &[])]
        );
    }

    #[test]
    fn a_register_with_nothing_in_the_period_is_not_a_pass() {
        let result = run(&[("B1", "BILL", "Draynor", "1", "2026-08-20", "9.50")]);
        assert!(result.at("status").is_str("REVIEW"));
        assert!(listed(&result).is_empty());
        let empty = run(&[]);
        assert!(empty.at("status").is_str("REVIEW") && empty.at("tenant").is_str(""));
    }

    #[test]
    fn registers_that_cannot_be_reviewed_are_refused() {
        let row = |id: &str, kind: &str, date: &str, amount: &str| {
            format!("Fabricated Varrock,{id},{kind},Draynor,1,{date},{amount},AUD\n")
        };
        let first = row("B1", "BILL", "2026-09-01", "1");
        let cases = [
            (
                first.clone() + "Other,B2,BILL,D,1,2026-09-01,1,AUD\n",
                "one tenant",
            ),
            (
                "Fabricated Varrock,B1,BILL,D,1,2026-09-01,1,NZD\n".to_owned(),
                "declared --currency 'AUD'",
            ),
            (
                row("B1", "CREDIT", "2026-09-01", "1"),
                "Kind must be BILL or PAYMENT",
            ),
            (
                first.clone() + &row("B1", "PAYMENT", "2026-09-02", "1"),
                "repeats DocumentID 'B1'",
            ),
            (row("B1", "BILL", "2026-09-01", "0"), "positive Amount"),
            (row("B1", "BILL", "2026-09-01", "x"), "Amount"),
            (
                row("B1", "BILL", "2026-10-01", "1"),
                "'B1' is dated after --period-end",
            ),
        ];
        for (rows, message) in cases {
            let error = run_text(&(HEADER.to_owned() + &rows)).unwrap_err();
            assert!(error.contains(message), "{error}");
        }
    }

    #[test]
    fn declarations_and_files_are_checked_first() {
        let temp = TempDir::new();
        let file = temp.join("documents.csv");
        let missing = temp.join("missing.csv");
        std::fs::write(&file, register(&[])).unwrap();
        let check = |currency, start, end, path: &Path| {
            duplicate_documents(path, currency, start, end).unwrap_err()
        };
        let (start, end) = ("2026-09-01", "2026-09-30");
        assert!(check("aud", start, end, &file).contains("three-letter"));
        assert!(check("AUDX", start, end, &file).contains("three-letter"));
        assert!(check("AUD", "x", end, &file).starts_with("--period-start: "));
        assert!(check("AUD", start, "x", &file).starts_with("--period-end: "));
        assert!(check("AUD", "2026-10-01", end, &file).contains("must not follow"));
        assert!(check("AUD", start, end, &missing).contains("missing.csv"));
        let result = duplicate_documents(&file, "AUD", start, end).unwrap();
        assert!(
            result
                .at("source_sha256")
                .at("documents")
                .as_pystr()
                .is_some()
        );
    }
}
