// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! A bank statement's lines tied out: each running balance against the one
//! before it plus the line, each page's closing balance against its last
//! line, and each page's opening balance against the page before.
use crate::{
    Result,
    decimal::{self, Decimal},
    isodate::Date,
    loader::{self, SourceSnapshot},
    pyjson::Json,
    pytext, reconciliation,
};
use std::path::Path;

pub const COLUMNS: [&str; 8] = [
    "Account",
    "Page",
    "Kind",
    "Date",
    "Description",
    "Amount",
    "Balance",
    "Currency",
];
pub const BOUNDARY: &str = "Review evidence only. A statement whose lines tie shows that its own figures are arithmetically consistent, not that it is genuine, complete or reconciled to the ledger.";

struct Row {
    number: usize,
    page: u32,
    kind: String,
    date: Date,
    amount: Option<Decimal>,
    balance: Decimal,
}

fn load(source: &SourceSnapshot, currency: &str) -> Result<(String, Vec<Row>)> {
    let path = crate::pypath::text(&source.path);
    let mut account = None;
    let mut rows = Vec::new();
    for row in loader::csv_rows(source, &COLUMNS, "Statement")? {
        let row = row?;
        let text = |field| row.text(field, matches!(field, "Description" | "Amount"));
        let refuse = |what: String| Err(format!("{path}: row {} {what}", row.number));
        let name = text("Account")?;
        if *account.get_or_insert_with(|| name.clone()) != name {
            return Err(
                "A statement must be for one account; tie out each account separately.".into(),
            );
        }
        if text("Currency")? != currency {
            return refuse(format!(
                "is not in the declared --currency {}.",
                pytext::repr(currency)
            ));
        }
        let Ok(page) = text("Page")?.parse::<u32>() else {
            return refuse("Page must be a whole number from 1.".into());
        };
        let kind = text("Kind")?;
        if !["OPENING", "LINE", "CLOSING"].contains(&kind.as_str()) {
            return refuse("Kind must be OPENING, LINE or CLOSING.".into());
        }
        let raw = text("Amount")?;
        let amount = match (kind.as_str(), raw.is_empty()) {
            ("LINE", true) => return refuse("is a LINE with no Amount.".into()),
            ("LINE", false) => Some(reconciliation::money(
                &raw,
                "Amount",
                &source.path,
                row.number,
            )?),
            (_, false) => {
                return refuse(format!(
                    "has an Amount, but {kind} rows carry a Balance only."
                ));
            }
            (_, true) => None,
        };
        rows.push(Row {
            number: row.number,
            page,
            kind,
            date: row.date("Date")?,
            amount,
            balance: reconciliation::money(&text("Balance")?, "Balance", &source.path, row.number)?,
        });
    }
    if rows.is_empty() {
        return Err(format!("{path}: no statement rows were supplied."));
    }
    Ok((account.unwrap_or_default(), rows))
}

pub fn statement_tie_out(path: &Path, currency: &str) -> Result<Json> {
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err("Declare a three-letter currency, such as AUD.".into());
    }
    let source = SourceSnapshot::capture(path, "Statement")?;
    decimal::with_precision(40, || review(&source, currency))
}

fn break_json(code: &str, row: &Row, expected: &Decimal, reason: String) -> Json {
    Json::object([
        ("code", Json::str(code)),
        ("row", Json::int(row.number as i64)),
        ("page", Json::int(i64::from(row.page))),
        ("date", Json::str(&row.date.iso())),
        ("expected", Json::str(&reconciliation::fixed(expected))),
        ("stated", Json::str(&reconciliation::fixed(&row.balance))),
        (
            "difference",
            Json::str(&reconciliation::fixed(&row.balance.sub(expected).value)),
        ),
        ("reason", Json::str(&reason)),
    ])
}

/// Walks the rows in file order. Each page is an OPENING row, its LINE rows
/// and a CLOSING row; pages run 1, 2, 3 in order.
fn review(source: &SourceSnapshot, currency: &str) -> Result<Json> {
    let (account, rows) = load(source, currency)?;
    let path = crate::pypath::text(&source.path);
    let mut findings = Vec::new();
    let mut running: Option<Decimal> = None;
    let mut last_closing: Option<Decimal> = None;
    let mut page = 0;
    let mut previous_date: Option<Date> = None;
    let mut lines = 0;
    for row in &rows {
        if previous_date.is_some_and(|date| row.date < date) {
            return Err(format!(
                "{path}: row {} is dated before the row above it; supply the statement in its printed order.",
                row.number
            ));
        }
        previous_date = Some(row.date);
        match row.kind.as_str() {
            "OPENING" => {
                if running.is_some() || row.page != page + 1 {
                    return Err(format!(
                        "{path}: row {} opens page {} out of order; each page runs OPENING, LINE rows, CLOSING, from page 1 with none missing.",
                        row.number, row.page
                    ));
                }
                page = row.page;
                if let Some(closing) = &last_closing
                    && row.balance != *closing
                {
                    findings.push(break_json(
                        "STATEMENT_PAGE_BREAK",
                        row,
                        closing,
                        format!(
                            "Page {} opens at {}, but page {} closed at {}.",
                            row.page,
                            reconciliation::fixed(&row.balance),
                            row.page - 1,
                            reconciliation::fixed(closing)
                        ),
                    ));
                }
                running = Some(row.balance.clone());
            }
            kind => {
                let Some(before) = running.as_ref().filter(|_| row.page == page) else {
                    return Err(format!(
                        "{path}: row {} is a {kind} row outside an opened page {}.",
                        row.number, row.page
                    ));
                };
                let (code, expected) = match &row.amount {
                    Some(amount) => ("STATEMENT_RUNNING_BREAK", before.add(amount).value),
                    None => ("STATEMENT_CLOSING_BREAK", before.clone()),
                };
                if row.balance != expected {
                    let reason = if row.amount.is_some() {
                        format!(
                            "The running balance {} is not the balance above it plus this line ({}).",
                            reconciliation::fixed(&row.balance),
                            reconciliation::fixed(&expected)
                        )
                    } else {
                        format!(
                            "Page {} closes at {}, but its lines run to {}.",
                            row.page,
                            reconciliation::fixed(&row.balance),
                            reconciliation::fixed(&expected)
                        )
                    };
                    findings.push(break_json(code, row, &expected, reason));
                }
                if row.amount.is_some() {
                    lines += 1;
                    running = Some(row.balance.clone());
                } else {
                    last_closing = Some(row.balance.clone());
                    running = None;
                }
            }
        }
    }
    if running.is_some() {
        return Err(format!(
            "{path}: page {page} has no CLOSING row; supply every page in full."
        ));
    }
    let first = &rows[0];
    let closing = last_closing.expect("a closed page sets it");
    Ok(Json::object([
        ("schema_version", Json::int(1)),
        (
            "status",
            Json::str(if findings.is_empty() {
                "PASS"
            } else {
                "REVIEW"
            }),
        ),
        ("account", Json::str(&account)),
        ("currency", Json::str(currency)),
        ("boundary", Json::str(BOUNDARY)),
        (
            "source_sha256",
            Json::object([("statement", Json::str(&source.sha256))]),
        ),
        ("page_count", Json::int(i64::from(page))),
        ("line_count", Json::int(lines)),
        (
            "opening_balance",
            Json::str(&reconciliation::fixed(&first.balance)),
        ),
        (
            "closing_balance",
            Json::str(&reconciliation::fixed(&closing)),
        ),
        ("findings", Json::Array(findings)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "Account,Page,Kind,Date,Description,Amount,Balance,Currency\n";

    /// `(page, kind, date, amount, balance)` rows for one account in AUD.
    fn run(rows: &[(&str, &str, &str, &str, &str)]) -> Result<Json> {
        let mut text = HEADER.to_owned();
        for (page, kind, date, amount, balance) in rows {
            text.push_str(&format!(
                "Fabricated cheque account,{page},{kind},{date},Item,{amount},{balance},AUD\n"
            ));
        }
        let source = SourceSnapshot::new(Path::new("statement.csv"), text.into_bytes());
        decimal::with_precision(40, || review(&source, "AUD"))
    }

    fn codes(result: &Json) -> Vec<(String, String)> {
        result
            .at("findings")
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let text = |key| f.at(key).as_pystr().unwrap().to_utf8().unwrap();
                (text("code"), text("difference"))
            })
            .collect()
    }

    const TIES: [(&str, &str, &str, &str, &str); 7] = [
        ("1", "OPENING", "2026-09-01", "", "1000.00"),
        ("1", "LINE", "2026-09-02", "-250.00", "750.00"),
        ("1", "LINE", "2026-09-03", "100.50", "850.50"),
        ("1", "CLOSING", "2026-09-15", "", "850.50"),
        ("2", "OPENING", "2026-09-15", "", "850.50"),
        ("2", "LINE", "2026-09-20", "-50.50", "800.00"),
        ("2", "CLOSING", "2026-09-30", "", "800.00"),
    ];

    #[test]
    fn a_statement_whose_figures_tie_passes() {
        let result = run(&TIES).unwrap();
        assert!(result.at("status").is_str("PASS"), "{:?}", codes(&result));
        assert_eq!(result.at("page_count").as_plain_int(), Some(&2.into()));
        assert_eq!(result.at("line_count").as_plain_int(), Some(&3.into()));
        assert!(result.at("opening_balance").is_str("1000.00"));
        assert!(result.at("closing_balance").is_str("800.00"));
    }

    #[test]
    fn running_closing_and_page_breaks_are_listed() {
        let mut rows = TIES;
        rows[2].4 = "851.50"; // the running balance is a dollar out
        rows[3].4 = "851.50"; // so the closing follows it, and ties
        rows[4].4 = "850.50"; // page 2 opens where page 1 should have closed
        rows[6].4 = "799.00"; // page 2's closing is a dollar under its lines
        let result = run(&rows).unwrap();
        assert!(result.at("status").is_str("REVIEW"));
        assert_eq!(
            codes(&result),
            [
                ("STATEMENT_RUNNING_BREAK".into(), "1.00".into()),
                ("STATEMENT_PAGE_BREAK".into(), "-1.00".into()),
                ("STATEMENT_CLOSING_BREAK".into(), "-1.00".into()),
            ]
        );
        let reasons: Vec<_> = result
            .at("findings")
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.at("reason").as_pystr().unwrap().to_utf8().unwrap())
            .collect();
        assert_eq!(
            reasons,
            [
                "The running balance 851.50 is not the balance above it plus this line (850.50).",
                "Page 2 opens at 850.50, but page 1 closed at 851.50.",
                "Page 2 closes at 799.00, but its lines run to 800.00.",
            ]
        );
    }

    #[test]
    fn malformed_statements_are_refused() {
        let mut missing_page = TIES.to_vec();
        for row in &mut missing_page[4..] {
            row.0 = "3";
        }
        let mut unclosed = TIES.to_vec();
        unclosed.pop();
        let mut early = TIES.to_vec();
        early[2].2 = "2026-08-31";
        let mut opened_twice = TIES.to_vec();
        opened_twice.insert(1, ("1", "OPENING", "2026-09-01", "", "1000.00"));
        let mut stray = TIES.to_vec();
        stray.insert(0, ("1", "LINE", "2026-09-01", "1.00", "1.00"));
        let mut opening_amount = TIES.to_vec();
        opening_amount[0].3 = "1.00";
        let mut line_without_amount = TIES.to_vec();
        line_without_amount[1].3 = "";
        let mut bad_kind = TIES.to_vec();
        bad_kind[1].1 = "FEE";
        let mut bad_page = TIES.to_vec();
        bad_page[0].0 = "one";
        for (rows, error) in [
            (
                missing_page,
                "statement.csv: row 6 opens page 3 out of order; each page runs OPENING, LINE rows, CLOSING, from page 1 with none missing.",
            ),
            (
                unclosed,
                "statement.csv: page 2 has no CLOSING row; supply every page in full.",
            ),
            (
                early,
                "statement.csv: row 4 is dated before the row above it; supply the statement in its printed order.",
            ),
            (
                opened_twice,
                "statement.csv: row 3 opens page 1 out of order; each page runs OPENING, LINE rows, CLOSING, from page 1 with none missing.",
            ),
            (
                stray,
                "statement.csv: row 2 is a LINE row outside an opened page 1.",
            ),
            (
                opening_amount,
                "statement.csv: row 2 has an Amount, but OPENING rows carry a Balance only.",
            ),
            (
                line_without_amount,
                "statement.csv: row 3 is a LINE with no Amount.",
            ),
            (
                bad_kind,
                "statement.csv: row 3 Kind must be OPENING, LINE or CLOSING.",
            ),
            (
                bad_page,
                "statement.csv: row 2 Page must be a whole number from 1.",
            ),
            (
                Vec::new(),
                "statement.csv: no statement rows were supplied.",
            ),
        ] {
            assert_eq!(run(&rows).unwrap_err(), error);
        }
    }

    #[test]
    fn one_account_and_currency_only() {
        let other_account = HEADER.to_owned()
            + "A,1,OPENING,2026-09-01,,,1.00,AUD\n"
            + "B,1,CLOSING,2026-09-01,,,1.00,AUD\n";
        let source = SourceSnapshot::new(Path::new("statement.csv"), other_account.into_bytes());
        assert_eq!(
            decimal::with_precision(40, || review(&source, "AUD")).unwrap_err(),
            "A statement must be for one account; tie out each account separately."
        );
        let source = SourceSnapshot::new(
            Path::new("statement.csv"),
            (HEADER.to_owned() + "A,1,OPENING,2026-09-01,,,1.00,NZD\n").into_bytes(),
        );
        assert_eq!(
            decimal::with_precision(40, || review(&source, "AUD")).unwrap_err(),
            "statement.csv: row 2 is not in the declared --currency 'AUD'."
        );
        assert_eq!(
            statement_tie_out(Path::new("missing.csv"), "aud").unwrap_err(),
            "Declare a three-letter currency, such as AUD."
        );
    }
}
