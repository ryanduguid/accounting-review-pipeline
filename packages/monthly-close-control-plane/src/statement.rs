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
        let name = row.text("Account", false)?;
        if *account.get_or_insert_with(|| name.clone()) != name {
            return Err(
                "A statement must be for one account; tie out each account separately.".into(),
            );
        }
        rows.push(parse(&row, &path, source, currency)?);
    }
    if rows.is_empty() {
        return Err(format!("{path}: no statement rows were supplied."));
    }
    Ok((account.unwrap_or_default(), rows))
}

fn parse(row: &loader::CsvRow, path: &str, source: &SourceSnapshot, currency: &str) -> Result<Row> {
    let text = |field| row.text(field, matches!(field, "Description" | "Amount"));
    let refuse = |what: String| Err(format!("{path}: row {} {what}", row.number));
    let money = |raw: &str, field| reconciliation::money(raw, field, &source.path, row.number);
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
        ("LINE", false) => Some(money(&raw, "Amount")?),
        (_, false) => {
            return refuse(format!(
                "has an Amount, but {kind} rows carry a Balance only."
            ));
        }
        (_, true) => None,
    };
    Ok(Row {
        number: row.number,
        page,
        kind,
        date: row.date("Date")?,
        amount,
        balance: money(&text("Balance")?, "Balance")?,
    })
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

/// The walk through a statement's rows in file order. Each page is an
/// OPENING row, its LINE rows and a CLOSING row; pages run 1, 2, 3 in order.
#[derive(Default)]
struct Walk {
    path: String,
    findings: Vec<Json>,
    /// The balance the open page has reached; `None` between pages.
    running: Option<Decimal>,
    last_closing: Option<Decimal>,
    page: u32,
    previous_date: Option<Date>,
    lines: i64,
}

impl Walk {
    fn row(&mut self, row: &Row) -> Result<()> {
        if self.previous_date.is_some_and(|date| row.date < date) {
            return Err(format!(
                "{}: row {} is dated before the row above it; supply the statement in its printed order.",
                self.path, row.number
            ));
        }
        self.previous_date = Some(row.date);
        if row.kind == "OPENING" {
            self.open(row)
        } else {
            self.carry(row)
        }
    }

    fn open(&mut self, row: &Row) -> Result<()> {
        if self.running.is_some() || row.page != self.page + 1 {
            return Err(format!(
                "{}: row {} opens page {} out of order; each page runs OPENING, LINE rows, CLOSING, from page 1 with none missing.",
                self.path, row.number, row.page
            ));
        }
        self.page = row.page;
        if let Some(closing) = &self.last_closing
            && row.balance != *closing
        {
            let reason = format!(
                "Page {} opens at {}, but page {} closed at {}.",
                row.page,
                reconciliation::fixed(&row.balance),
                row.page - 1,
                reconciliation::fixed(closing)
            );
            let finding = break_json("STATEMENT_PAGE_BREAK", row, closing, reason);
            self.findings.push(finding);
        }
        self.running = Some(row.balance.clone());
        Ok(())
    }

    /// A LINE carries the running balance forward; a CLOSING row ends the page.
    fn carry(&mut self, row: &Row) -> Result<()> {
        let Some(before) = self.running.take().filter(|_| row.page == self.page) else {
            return Err(format!(
                "{}: row {} is a {} row outside an opened page {}.",
                self.path, row.number, row.kind, row.page
            ));
        };
        let stated = reconciliation::fixed(&row.balance);
        let (expected, code, reason) = match &row.amount {
            Some(amount) => {
                let expected = before.add(amount).value;
                let reason = format!(
                    "The running balance {stated} is not the balance above it plus this line ({}).",
                    reconciliation::fixed(&expected)
                );
                (expected, "STATEMENT_RUNNING_BREAK", reason)
            }
            None => {
                let reason = format!(
                    "Page {} closes at {stated}, but its lines run to {}.",
                    row.page,
                    reconciliation::fixed(&before)
                );
                (before, "STATEMENT_CLOSING_BREAK", reason)
            }
        };
        if row.balance != expected {
            self.findings.push(break_json(code, row, &expected, reason));
        }
        if row.amount.is_some() {
            self.lines += 1;
            self.running = Some(row.balance.clone());
        } else {
            self.last_closing = Some(row.balance.clone());
        }
        Ok(())
    }
}

fn review(source: &SourceSnapshot, currency: &str) -> Result<Json> {
    let (account, rows) = load(source, currency)?;
    let mut walk = Walk {
        path: crate::pypath::text(&source.path),
        ..Walk::default()
    };
    for row in &rows {
        walk.row(row)?;
    }
    let (Some(closing), None) = (&walk.last_closing, &walk.running) else {
        return Err(format!(
            "{}: page {} has no CLOSING row; supply every page in full.",
            walk.path, walk.page
        ));
    };
    let status = if walk.findings.is_empty() {
        "PASS"
    } else {
        "REVIEW"
    };
    Ok(Json::object([
        ("schema_version", Json::int(1)),
        ("status", Json::str(status)),
        ("account", Json::str(&account)),
        ("currency", Json::str(currency)),
        ("boundary", Json::str(BOUNDARY)),
        (
            "source_sha256",
            Json::object([("statement", Json::str(&source.sha256))]),
        ),
        ("page_count", Json::int(i64::from(walk.page))),
        ("line_count", Json::int(walk.lines)),
        (
            "opening_balance",
            Json::str(&reconciliation::fixed(&rows[0].balance)),
        ),
        (
            "closing_balance",
            Json::str(&reconciliation::fixed(closing)),
        ),
        ("findings", Json::Array(walk.findings)),
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

    type Rows = Vec<(
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    )>;

    type Edit = fn(&mut Rows);

    fn refused(edit: Edit) -> String {
        let mut rows = TIES.to_vec();
        edit(&mut rows);
        run(&rows).unwrap_err()
    }

    #[test]
    fn pages_out_of_order_are_refused() {
        let order = "each page runs OPENING, LINE rows, CLOSING, from page 1 with none missing.";
        let missing_page = refused(|rows| rows[4..].iter_mut().for_each(|row| row.0 = "3"));
        assert_eq!(
            missing_page,
            format!("statement.csv: row 6 opens page 3 out of order; {order}")
        );
        let opened_twice =
            refused(|rows| rows.insert(1, ("1", "OPENING", "2026-09-01", "", "1.00")));
        assert_eq!(
            opened_twice,
            format!("statement.csv: row 3 opens page 1 out of order; {order}")
        );
        assert_eq!(
            refused(|rows| rows.insert(0, ("1", "LINE", "2026-09-01", "1.00", "1.00"))),
            "statement.csv: row 2 is a LINE row outside an opened page 1."
        );
        assert_eq!(
            refused(|rows| rows.truncate(6)),
            "statement.csv: page 2 has no CLOSING row; supply every page in full."
        );
        assert_eq!(
            refused(|rows| rows[2].2 = "2026-08-31"),
            "statement.csv: row 4 is dated before the row above it; supply the statement in its printed order."
        );
    }

    #[test]
    fn malformed_rows_are_refused() {
        let cases: [(Edit, &str); 6] = [
            (
                |rows| rows[0].3 = "1.00",
                "row 2 has an Amount, but OPENING rows carry a Balance only.",
            ),
            (|rows| rows[1].3 = "", "row 3 is a LINE with no Amount."),
            (
                |rows| rows[1].1 = "FEE",
                "row 3 Kind must be OPENING, LINE or CLOSING.",
            ),
            (
                |rows| rows[0].0 = "one",
                "row 2 Page must be a whole number from 1.",
            ),
            (|rows| rows.clear(), "no statement rows were supplied."),
            (
                |rows| rows[1].3 = "1.001",
                "Amount must be below 10^15 with at most two decimal places.",
            ),
        ];
        for (edit, error) in cases {
            assert_eq!(refused(edit), format!("statement.csv: {error}"));
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
