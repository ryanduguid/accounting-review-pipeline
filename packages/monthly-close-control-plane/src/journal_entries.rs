// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Journal-entry tests over a general-ledger listing: the entry
//! characteristics ASA 240 Appendix 4 lists, and a Benford first-digit test.
use crate::{
    Result,
    decimal::{self, Decimal},
    drivers::{self, Transaction, field},
    isodate::{self, Date},
    loader::SourceSnapshot,
    pyjson::Json,
    reconciliation,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const BOUNDARY: &str = "Review evidence only. A listed entry has a characteristic worth a reviewer's look; it is not evidence of fraud or error, and an entry not listed is not shown to be appropriate.";
/// The tests, in the order their findings are listed.
pub const TESTS: [&str; 7] = [
    "unbalanced_journal",
    "post_closing",
    "period_end",
    "no_description",
    "weekend",
    "round_amount",
    "seldom_used_account",
];
/// log10(1 + 1/d) for the first digits 1 to 9, to 10 places.
const BENFORD_EXPECTED: [&str; 9] = [
    "0.3010299957",
    "0.1760912591",
    "0.1249387366",
    "0.0969100130",
    "0.0791812460",
    "0.0669467896",
    "0.0579919470",
    "0.0511525224",
    "0.0457574906",
];
/// The fewest amounts for which every first digit expects at least 5
/// (5 / 0.0457574906 is 109.3).
pub const BENFORD_MINIMUM: usize = 110;
/// Nigrini's empirical first-digit mean absolute deviation bands, as arXiv
/// 1311.4787 states them: the upper bound of each band and its name; above
/// the last is nonconformity.
const BANDS: [(&str, &str); 3] = [
    ("0.006", "close conformity"),
    ("0.012", "acceptable conformity"),
    ("0.015", "marginal conformity"),
];

struct Line {
    account: String,
    item: Transaction,
}

fn finding(
    test: &str,
    id: &str,
    account: Option<&str>,
    date: Date,
    amount: &Decimal,
    reason: String,
) -> Json {
    Json::object([
        ("test", Json::str(test)),
        ("transaction_id", Json::str(id)),
        ("account_id", account.map_or(Json::Null, Json::str)),
        ("date", Json::str(&date.iso())),
        ("amount", Json::str(&reconciliation::fixed(amount))),
        ("reason", Json::str(&reason)),
    ])
}

/// Whether any line of a journal carries a description or reference.
pub(crate) fn has_narration(rows: &[&Json]) -> bool {
    rows.iter().any(|row| {
        !field(row, "Description").trim().is_empty() || !field(row, "Reference").trim().is_empty()
    })
}

/// A line of at least 1,000 that is a whole multiple of 1,000.
pub(crate) fn is_round(amount: &Decimal) -> bool {
    let magnitude = amount.copy_abs();
    magnitude >= Decimal::parse("1000").unwrap()
        && reconciliation::fixed(&magnitude).ends_with("000.00")
}

pub(crate) fn is_weekend(date: Date) -> bool {
    // Ordinal 1 is Monday 1 January 0001.
    (isodate::ordinal(date) + 6) % 7 >= 5
}

fn first_digit(amount: &Decimal) -> usize {
    let text = reconciliation::fixed(&amount.copy_abs());
    let digit = text.bytes().find(|b| (b'1'..=b'9').contains(b)).unwrap();
    usize::from(digit - b'0')
}

/// Nigrini's name for a first-digit mean absolute deviation.
fn conformity(mad: &Decimal) -> &'static str {
    BANDS
        .iter()
        .find(|(bound, _)| *mad <= Decimal::parse(bound).unwrap())
        .map_or("nonconformity", |(_, name)| name)
}

/// The Benford first-digit result, and whether it is nonconforming.
fn benford(lines: &[Line]) -> (Json, bool) {
    let mut counts = [0usize; 9];
    for line in lines {
        counts[first_digit(&line.item.amount) - 1] += 1;
    }
    let total = lines.len();
    let count_json = |index: usize| Json::int(counts[index] as i64);
    if total < BENFORD_MINIMUM {
        let result = Json::object([
            ("ran", Json::Bool(false)),
            (
                "reason",
                Json::str(&format!(
                    "{total} amount(s); the test needs at least {BENFORD_MINIMUM}, so that every first digit expects at least 5."
                )),
            ),
            ("amounts", Json::int(total as i64)),
            ("counts", Json::Array((0..9).map(count_json).collect())),
        ]);
        return (result, false);
    }
    let number = |value: usize| Decimal::parse(&value.to_string()).unwrap();
    let mut deviation = Decimal::zero();
    let mut digits = Vec::new();
    for (index, expected) in BENFORD_EXPECTED.iter().enumerate() {
        let expected = Decimal::parse(expected).unwrap();
        let observed = number(counts[index]).div(&number(total));
        let gap = observed.sub(&expected).value.copy_abs();
        deviation = deviation.add(&gap).value;
        digits.push(Json::object([
            ("digit", Json::int(index as i64 + 1)),
            ("count", count_json(index)),
            ("observed", Json::str(&observed.format_places(4).unwrap())),
            ("expected", Json::str(&expected.format_places(4).unwrap())),
        ]));
    }
    let mad = deviation.div(&number(9));
    let band = conformity(&mad);
    let result = Json::object([
        ("ran", Json::Bool(true)),
        ("amounts", Json::int(total as i64)),
        ("digits", Json::Array(digits)),
        (
            "mean_absolute_deviation",
            Json::str(&mad.format_places(6).unwrap()),
        ),
        ("conformity", Json::str(band)),
        ("review_above", Json::str(BANDS[2].0)),
    ]);
    (result, band == "nonconformity")
}

/// Test the journals in `path`, one tenant's canonical transaction CSV
/// holding every line of each journal; `TransactionID` names the journal.
pub fn journal_entry_tests(path: &Path, currency: &str, period_end: &str) -> Result<Json> {
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err("Declare a three-letter currency, such as AUD.".into());
    }
    let period_end =
        reconciliation::date(period_end).map_err(|error| format!("--period-end: {error}"))?;
    let source = SourceSnapshot::capture(path, "Transactions")?;
    decimal::with_precision(40, || review(&source, currency, period_end))
}

#[expect(
    clippy::too_many_lines,
    reason = "runs every journal test over one listing"
)]
fn review(source: &SourceSnapshot, currency: &str, period_end: Date) -> Result<Json> {
    let mut selection = drivers::transactions(source, currency)?;
    let tenants: BTreeSet<_> = selection.order.iter().map(|key| key.0.clone()).collect();
    if tenants.len() > 1 {
        return Err("Journal lines must belong to one tenant; test each tenant separately.".into());
    }
    let mut lines = Vec::new();
    for key in selection.order {
        for item in selection.by_account.remove(&key).unwrap() {
            lines.push(Line {
                account: key.1.clone(),
                item,
            });
        }
    }
    lines.sort_by(|a, b| (&a.item.identifier, &a.account).cmp(&(&b.item.identifier, &b.account)));
    let mut journals: BTreeMap<&str, Vec<&Line>> = BTreeMap::new();
    // The loader refuses a journal posting to one account twice, so an
    // account's line count is its journal count.
    let mut accounts: BTreeMap<&str, usize> = BTreeMap::new();
    for line in &lines {
        journals
            .entry(&line.item.identifier)
            .or_default()
            .push(line);
        *accounts.entry(&line.account).or_default() += 1;
    }
    let mut findings: BTreeMap<&str, Vec<Json>> = BTreeMap::new();
    let zero = Decimal::zero();
    for (id, members) in &journals {
        let first = members.iter().map(|line| line.item.date).min().unwrap();
        let last = members.iter().map(|line| line.item.date).max().unwrap();
        let debits = members
            .iter()
            .filter(|line| line.item.amount > zero)
            .fold(zero.clone(), |sum, line| sum.add(&line.item.amount).value);
        let net = members
            .iter()
            .fold(zero.clone(), |sum, line| sum.add(&line.item.amount).value);
        let mut add = |test, date, reason| {
            findings
                .entry(test)
                .or_default()
                .push(finding(test, id, None, date, &debits, reason));
        };
        if !net.is_zero() {
            add(
                "unbalanced_journal",
                first,
                format!(
                    "Debits less credits is {}; every line of a journal must be supplied, and its debits must equal its credits.",
                    reconciliation::fixed(&net)
                ),
            );
        }
        if last > period_end {
            add(
                "post_closing",
                last,
                format!(
                    "Dated after the period end, {}: a post-closing entry.",
                    period_end.iso()
                ),
            );
        }
        if members.iter().any(|line| line.item.date == period_end) {
            add(
                "period_end",
                period_end,
                "Dated on the last day of the period.".into(),
            );
        }
        let rows: Vec<&Json> = members.iter().map(|line| &line.item.row).collect();
        if !has_narration(&rows) {
            add(
                "no_description",
                first,
                "No line carries a description or reference.".into(),
            );
        }
        if let Some(date) = members
            .iter()
            .map(|line| line.item.date)
            .find(|date| is_weekend(*date))
        {
            add("weekend", date, "Dated on a Saturday or Sunday.".into());
        }
    }
    for line in &lines {
        let mut add = |test, reason| {
            findings.entry(test).or_default().push(finding(
                test,
                &line.item.identifier,
                Some(&line.account),
                line.item.date,
                &line.item.amount,
                reason,
            ));
        };
        if is_round(&line.item.amount) {
            add("round_amount", "A whole multiple of 1,000.".to_owned());
        }
        if journals.len() > 1 && accounts[line.account.as_str()] == 1 {
            add(
                "seldom_used_account",
                "The only journal in the listing that posts to this account.".to_owned(),
            );
        }
    }
    let (benford, nonconforming) = benford(&lines);
    let listed: Vec<Json> = TESTS
        .iter()
        .flat_map(|test| findings.remove(test).unwrap_or_default())
        .collect();
    let passed = !lines.is_empty() && listed.is_empty() && !nonconforming;
    Ok(Json::object([
        ("schema_version", Json::int(1)),
        ("status", Json::str(if passed { "PASS" } else { "REVIEW" })),
        (
            "tenant",
            tenants
                .into_iter()
                .next()
                .map_or(Json::Null, |tenant| Json::str(&tenant)),
        ),
        ("currency", Json::str(currency)),
        ("period_end", Json::str(&period_end.iso())),
        (
            "scope",
            Json::str(
                "Every line of each journal in the supplied listing; TransactionID names the journal.",
            ),
        ),
        ("boundary", Json::str(BOUNDARY)),
        (
            "source_sha256",
            Json::object([("transactions", Json::str(&source.sha256))]),
        ),
        ("line_count", Json::int(lines.len() as i64)),
        ("journal_count", Json::int(journals.len() as i64)),
        (
            "tests",
            Json::Array(TESTS.iter().map(|test| Json::str(test)).collect()),
        ),
        ("findings", Json::Array(listed)),
        ("benford", benford),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    const HEADER: &str =
        "Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit\n";
    type Row<'a> = (&'a str, &'a str, &'a str, &'a str, &'a str);

    /// Fabricated lines: journal, account, date, description and a signed
    /// amount, negative for a credit.
    fn listing(lines: &[Row]) -> SourceSnapshot {
        let mut text = HEADER.to_owned();
        for (id, account, date, description, amount) in lines {
            let (debit, credit) = amount
                .strip_prefix('-')
                .map_or((*amount, "0"), |credit| ("0", credit));
            text += &format!(
                "Fabricated Varrock,{account},AUD,{id},{date},,{description},{debit},{credit}\n"
            );
        }
        SourceSnapshot::new(Path::new("journals.csv"), text.into_bytes())
    }

    fn run(lines: &[Row]) -> Json {
        let end = Date::new(2026, 9, 30).unwrap();
        decimal::with_precision(40, || review(&listing(lines), "AUD", end)).unwrap()
    }

    fn listed(result: &Json) -> Vec<String> {
        result
            .at("findings")
            .as_array()
            .unwrap()
            .iter()
            .map(|item| format!("{}:{}", field(item, "test"), field(item, "transaction_id")))
            .collect()
    }

    /// Two balanced weekday journals that share both accounts.
    const CLEAN: [Row; 4] = [
        ("J1", "610", "2026-09-07", "Fabricated rent", "1234.56"),
        ("J1", "090", "2026-09-07", "Fabricated rent", "-1234.56"),
        ("J2", "610", "2026-09-08", "Fabricated rent", "250.10"),
        ("J2", "090", "2026-09-08", "Fabricated rent", "-250.10"),
    ];

    #[test]
    fn balanced_described_weekday_journals_pass() {
        let result = run(&CLEAN);
        assert!(result.at("status").is_str("PASS"), "{result:?}");
        assert!(result.at("tenant").is_str("Fabricated Varrock"));
        assert_eq!(result.at("line_count").as_plain_int(), Some(&4.into()));
        assert_eq!(result.at("journal_count").as_plain_int(), Some(&2.into()));
        assert!(listed(&result).is_empty());
        let benford = result.at("benford");
        assert_eq!(benford.at("ran").as_bool(), Some(false));
        assert!(benford.at("reason").is_str(
            "4 amount(s); the test needs at least 110, so that every first digit expects at least 5."
        ));
    }

    #[test]
    fn each_characteristic_is_listed_in_test_order() {
        let result = run(&[
            ("J1", "610", "2026-09-07", "Fabricated rent", "1234.56"),
            ("J1", "090", "2026-09-07", "Fabricated rent", "-1234.56"),
            ("J2", "610", "2026-09-08", "Fabricated rent", "250.10"),
            ("J2", "090", "2026-09-08", "Fabricated rent", "-250.00"),
            ("J3", "610", "2026-10-02", "Fabricated accrual", "300.15"),
            ("J3", "090", "2026-10-02", "Fabricated accrual", "-300.15"),
            ("J4", "610", "2026-09-30", "Fabricated accrual", "410.20"),
            ("J4", "090", "2026-09-30", "Fabricated accrual", "-410.20"),
            ("J5", "610", "2026-09-09", "", "120.30"),
            ("J5", "090", "2026-09-09", " ", "-120.30"),
            ("J6", "610", "2026-09-12", "Fabricated Saturday", "130.40"),
            ("J6", "090", "2026-09-12", "Fabricated Saturday", "-130.40"),
            ("J7", "610", "2026-09-10", "Fabricated transfer", "5000.00"),
            ("J7", "090", "2026-09-10", "Fabricated transfer", "-5000.00"),
            ("J8", "610", "2026-09-11", "Fabricated suspense", "140.50"),
            ("J8", "999", "2026-09-11", "Fabricated suspense", "-140.50"),
        ]);
        assert!(result.at("status").is_str("REVIEW"));
        assert_eq!(
            listed(&result),
            [
                "unbalanced_journal:J2",
                "post_closing:J3",
                "period_end:J4",
                "no_description:J5",
                "weekend:J6",
                "round_amount:J7",
                "round_amount:J7",
                "seldom_used_account:J8",
            ]
        );
        let findings = result.at("findings").as_array().unwrap();
        assert!(matches!(findings[0].at("account_id"), Json::Null));
        assert!(findings[0].at("amount").is_str("250.10"));
        assert!(findings[0].at("reason").is_str(
            "Debits less credits is 0.10; every line of a journal must be supplied, and its debits must equal its credits."
        ));
        assert!(findings[1].at("date").is_str("2026-10-02"));
        assert!(findings[5].at("account_id").is_str("090"));
        assert!(findings[6].at("amount").is_str("5000.00"));
        assert!(findings[7].at("account_id").is_str("999"));
    }

    #[test]
    fn one_journal_has_no_seldom_used_account_and_an_empty_listing_reviews() {
        let result = run(&CLEAN[..2]);
        assert!(result.at("status").is_str("PASS"), "{result:?}");
        let empty = run(&[]);
        assert!(empty.at("status").is_str("REVIEW"));
        assert!(matches!(empty.at("tenant"), Json::Null));
        assert_eq!(empty.at("journal_count").as_plain_int(), Some(&0.into()));
    }

    /// Balanced pairs whose first digits follow Benford's proportions over
    /// 1,000 journals or, when not conforming, all lead with 9.
    fn benford_listing(conforming: bool) -> Vec<(String, String)> {
        let shares = [301, 176, 125, 97, 79, 67, 58, 51, 46];
        let mut lines = Vec::new();
        let mut number = 0;
        for (index, share) in shares.iter().enumerate() {
            let digit = if conforming { index + 1 } else { 9 };
            for _ in 0..*share {
                number += 1;
                let amount = format!("{digit}{:02}.45", number % 100);
                lines.push((format!("J{number}"), amount.clone()));
                lines.push((format!("J{number}"), format!("-{amount}")));
            }
        }
        lines
    }

    fn run_benford(conforming: bool) -> Json {
        let owned = benford_listing(conforming);
        let rows: Vec<Row> = owned
            .iter()
            .map(|(id, amount)| {
                let account = if amount.starts_with('-') {
                    "090"
                } else {
                    "610"
                };
                (
                    id.as_str(),
                    account,
                    "2026-09-07",
                    "Fabricated",
                    amount.as_str(),
                )
            })
            .collect();
        run(&rows)
    }

    #[test]
    fn benford_conformity_passes_and_nonconformity_reviews() {
        let result = run_benford(true);
        let benford = result.at("benford");
        assert!(result.at("status").is_str("PASS"), "{benford:?}");
        assert_eq!(benford.at("amounts").as_plain_int(), Some(&2000.into()));
        assert!(benford.at("conformity").is_str("close conformity"));
        let first = &benford.at("digits").as_array().unwrap()[0];
        assert!(first.at("observed").is_str("0.3010"));
        assert!(first.at("expected").is_str("0.3010"));
        assert_eq!(first.at("count").as_plain_int(), Some(&602.into()));
        let result = run_benford(false);
        let benford = result.at("benford");
        assert!(result.at("status").is_str("REVIEW"));
        assert!(result.at("findings").as_array().unwrap().is_empty());
        assert!(benford.at("conformity").is_str("nonconformity"));
        assert!(benford.at("review_above").is_str("0.015"));
    }

    #[test]
    fn findings_follow_journal_and_account_order_whatever_the_line_order() {
        let lines = [
            ("J9", "999", "2026-09-12", "Fabricated", "-3000.00"),
            ("J9", "610", "2026-09-12", "Fabricated", "3000.00"),
            ("J1", "610", "2026-09-07", "Fabricated", "1234.56"),
            ("J1", "090", "2026-09-07", "Fabricated", "-1234.56"),
        ];
        let mut reversed = lines;
        reversed.reverse();
        let expected = [
            "weekend:J9",
            "round_amount:J9",
            "round_amount:J9",
            "seldom_used_account:J1",
            "seldom_used_account:J9",
        ];
        for order in [&lines, &reversed] {
            let result = run(order);
            assert_eq!(listed(&result), expected);
            let accounts: Vec<_> = result
                .at("findings")
                .as_array()
                .unwrap()
                .iter()
                .skip(1)
                .map(|item| field(item, "account_id"))
                .collect();
            assert_eq!(accounts, ["610", "999", "090", "999"]);
        }
    }

    #[test]
    fn benford_runs_from_110_amounts() {
        let rows = |journals: usize| -> Vec<(String, String)> {
            (0..journals)
                .flat_map(|index| {
                    let amount = format!("9{:02}.45", index % 100);
                    [
                        (format!("J{index}"), amount.clone()),
                        (format!("J{index}"), format!("-{amount}")),
                    ]
                })
                .collect()
        };
        for (journals, ran) in [(54, false), (55, true)] {
            let owned = rows(journals);
            let listing: Vec<Row> = owned
                .iter()
                .map(|(id, amount)| {
                    let account = if amount.starts_with('-') {
                        "090"
                    } else {
                        "610"
                    };
                    (
                        id.as_str(),
                        account,
                        "2026-09-07",
                        "Fabricated",
                        amount.as_str(),
                    )
                })
                .collect();
            let result = run(&listing);
            assert_eq!(result.at("benford").at("ran").as_bool(), Some(ran));
            assert!(result.at("findings").as_array().unwrap().is_empty());
            let status = if ran { "REVIEW" } else { "PASS" };
            assert!(result.at("status").is_str(status), "{journals}");
        }
    }

    #[test]
    fn conformity_bands_include_their_upper_bounds() {
        for (mad, band) in [
            ("0.006", "close conformity"),
            ("0.0061", "acceptable conformity"),
            ("0.012", "acceptable conformity"),
            ("0.015", "marginal conformity"),
            ("0.0151", "nonconformity"),
        ] {
            assert_eq!(conformity(&Decimal::parse(mad).unwrap()), band);
        }
    }

    #[test]
    fn weekends_and_first_digits() {
        for (day, weekend) in [(20, false), (21, true), (22, true), (23, false)] {
            assert_eq!(is_weekend(Date::new(2026, 11, day).unwrap()), weekend);
        }
        for (amount, digit) in [("0.05", 5), ("-0.70", 7), ("10.00", 1), ("-985.20", 9)] {
            assert_eq!(first_digit(&Decimal::parse(amount).unwrap()), digit);
        }
    }

    #[test]
    fn declarations_files_and_tenants_are_checked() {
        let temp = TempDir::new();
        let path = temp.join("journals.csv");
        std::fs::write(&path, &listing(&CLEAN).content).unwrap();
        let result = journal_entry_tests(&path, "AUD", "2026-09-30").unwrap();
        assert!(result.at("status").is_str("PASS"));
        let currency = "Declare a three-letter currency, such as AUD.";
        for (code, end, error) in [
            ("aud", "2026-09-30", currency),
            ("AUDD", "2026-09-30", currency),
            (
                "AUD",
                "30/09/2026",
                "--period-end: Dates must use YYYY-MM-DD.",
            ),
        ] {
            assert_eq!(journal_entry_tests(&path, code, end).unwrap_err(), error);
        }
        let missing = journal_entry_tests(&temp.join("absent.csv"), "AUD", "2026-09-30");
        assert!(
            missing
                .unwrap_err()
                .starts_with("Transactions does not exist: ")
        );
        let mut text = String::from_utf8(listing(&CLEAN).content).unwrap();
        text += "Fabricated Falador,610,AUD,J9,2026-09-07,,Fabricated,1.00,0\n";
        std::fs::write(&path, text).unwrap();
        assert_eq!(
            journal_entry_tests(&path, "AUD", "2026-09-30").unwrap_err(),
            "Journal lines must belong to one tenant; test each tenant separately."
        );
    }
}
