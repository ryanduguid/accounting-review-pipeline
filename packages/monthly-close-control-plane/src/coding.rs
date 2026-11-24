// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Period lines coded to an account or tax code that departs from the usual
//! coding of the same contact, or of the same account, before the period.
use crate::{
    Result,
    decimal::{self, Decimal},
    isodate::Date,
    loader::{self, SourceSnapshot},
    pyjson::Json,
    pytext, reconciliation,
};
use std::{collections::BTreeMap, path::Path};

pub const COLUMNS: [&str; 8] = [
    "Tenant",
    "LineID",
    "Date",
    "Contact",
    "AccountID",
    "TaxCode",
    "Amount",
    "Currency",
];
pub const BOUNDARY: &str = "Review evidence only. A listed line departs from the coding used before the period; it is not shown to be miscoded, and a listing with nothing listed is not shown to be coded correctly.";
/// Earlier lines needed before a coding counts as usual.
pub const MINIMUM_HISTORY: usize = 3;
/// The share of earlier lines, in per cent, that one coding must carry.
pub const USUAL_SHARE_PERCENT: usize = 80;

struct Line {
    id: String,
    date: Date,
    contact: String,
    account: String,
    tax_code: String,
    amount: Decimal,
}

/// Contact names compare once case and whitespace are ignored.
fn contact_key(name: &str) -> String {
    pytext::casefold(name).split_whitespace().collect()
}

fn load(source: &SourceSnapshot, currency: &str, end: Date) -> Result<(String, Vec<Line>)> {
    let path = crate::pypath::text(&source.path);
    let mut tenant = None;
    let mut lines: Vec<Line> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for row in loader::csv_rows(source, &COLUMNS, "Lines")? {
        let row = row?;
        let text = |field| row.text(field, field == "Contact");
        let name = text("Tenant")?;
        if *tenant.get_or_insert_with(|| name.clone()) != name {
            return Err("Lines must belong to one tenant; review each tenant separately.".into());
        }
        let refuse = |what: String| Err(format!("{path}: row {} {what}", row.number));
        if text("Currency")? != currency {
            return refuse(format!(
                "is not in the declared --currency {}.",
                pytext::repr(currency)
            ));
        }
        let id = text("LineID")?;
        if !seen.insert(id.clone()) {
            return refuse(format!("repeats LineID {}.", pytext::repr(&id)));
        }
        let date = row.date("Date")?;
        if date > end {
            return refuse(
                "is dated after --period-end; supply lines to the period end only.".into(),
            );
        }
        lines.push(Line {
            id,
            date,
            contact: text("Contact")?,
            account: text("AccountID")?,
            tax_code: text("TaxCode")?,
            amount: reconciliation::money(&text("Amount")?, "Amount", &source.path, row.number)?,
        });
    }
    match tenant {
        Some(tenant) => Ok((tenant, lines)),
        None => Err(format!("{path}: no lines were supplied.")),
    }
}

/// For each key, the coding that carries at least `MINIMUM_HISTORY` earlier
/// lines and `USUAL_SHARE_PERCENT` of them, with its count and the total.
fn usual<'a, K: Ord>(
    history: impl Iterator<Item = (K, &'a str)>,
) -> BTreeMap<K, (&'a str, usize, usize)> {
    let mut counts: BTreeMap<K, BTreeMap<&str, usize>> = BTreeMap::new();
    for (key, value) in history {
        *counts.entry(key).or_default().entry(value).or_default() += 1;
    }
    counts
        .into_iter()
        .filter_map(|(key, values)| {
            let total = values.values().sum::<usize>();
            let (value, count) = values.into_iter().max_by_key(|(_, count)| *count)?;
            (count >= MINIMUM_HISTORY && count * 100 >= total * USUAL_SHARE_PERCENT)
                .then_some((key, (value, count, total)))
        })
        .collect()
}

/// The usual coding when `value` departs from it.
fn departs<'a>(
    usual: Option<&'a (&'a str, usize, usize)>,
    value: &str,
) -> Option<&'a (&'a str, usize, usize)> {
    usual.filter(|usual| usual.0 != value)
}

fn finding(code: &str, line: &Line, (value, count, total): &(&str, usize, usize)) -> Json {
    Json::object([
        ("code", Json::str(code)),
        ("line_id", Json::str(&line.id)),
        ("date", Json::str(&line.date.iso())),
        ("contact", Json::str(&line.contact)),
        ("account_id", Json::str(&line.account)),
        ("tax_code", Json::str(&line.tax_code)),
        ("amount", Json::str(&reconciliation::fixed(&line.amount))),
        ("usual", Json::str(value)),
        ("usual_lines", Json::int(*count as i64)),
        ("earlier_lines", Json::int(*total as i64)),
    ])
}

/// Review the lines in `path` for the period `start` to `end`.
pub fn coding_review(path: &Path, currency: &str, start: &str, end: &str) -> Result<Json> {
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err("Declare a three-letter currency, such as AUD.".into());
    }
    let start = reconciliation::date(start).map_err(|error| format!("--period-start: {error}"))?;
    let end = reconciliation::date(end).map_err(|error| format!("--period-end: {error}"))?;
    if start > end {
        return Err("--period-start must not follow --period-end.".into());
    }
    let source = SourceSnapshot::capture(path, "Lines")?;
    decimal::with_precision(40, || review(&source, currency, start, end))
}

fn review(source: &SourceSnapshot, currency: &str, start: Date, end: Date) -> Result<Json> {
    let (tenant, lines) = load(source, currency, end)?;
    let (earlier, period): (Vec<&Line>, Vec<&Line>) =
        lines.iter().partition(|line| line.date < start);
    let pair = |line: &Line| (contact_key(&line.contact), line.account.clone());
    let with_contact = || earlier.iter().filter(|line| !line.contact.is_empty());
    let accounts =
        usual(with_contact().map(|line| (contact_key(&line.contact), line.account.as_str())));
    let pair_codes = usual(with_contact().map(|line| (pair(line), line.tax_code.as_str())));
    let account_codes = usual(
        earlier
            .iter()
            .map(|line| (line.account.clone(), line.tax_code.as_str())),
    );
    let mut findings = Vec::new();
    for line in &period {
        let contact = !line.contact.is_empty();
        if contact
            && let Some(usual) = departs(accounts.get(&contact_key(&line.contact)), &line.account)
        {
            findings.push(finding("UNUSUAL_ACCOUNT", line, usual));
        }
        let pair_usual = contact.then(|| pair_codes.get(&pair(line))).flatten();
        if let Some(usual) = departs(pair_usual, &line.tax_code) {
            findings.push(finding("UNUSUAL_TAX_CODE", line, usual));
        } else if pair_usual.is_none()
            && let Some(usual) = departs(account_codes.get(&line.account), &line.tax_code)
        {
            findings.push(finding("UNUSUAL_ACCOUNT_TAX_CODE", line, usual));
        }
    }
    let passed = !period.is_empty() && findings.is_empty();
    Ok(Json::object([
        ("schema_version", Json::int(1)),
        ("status", Json::str(if passed { "PASS" } else { "REVIEW" })),
        ("tenant", Json::str(&tenant)),
        ("currency", Json::str(currency)),
        ("period_start", Json::str(&start.iso())),
        ("period_end", Json::str(&end.iso())),
        ("boundary", Json::str(BOUNDARY)),
        (
            "source_sha256",
            Json::object([("lines", Json::str(&source.sha256))]),
        ),
        ("line_count", Json::int(lines.len() as i64)),
        ("lines_in_period", Json::int(period.len() as i64)),
        ("minimum_history", Json::int(MINIMUM_HISTORY as i64)),
        ("usual_share_percent", Json::int(USUAL_SHARE_PERCENT as i64)),
        ("findings", Json::Array(findings)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pyjson;

    const HEADER: &str = "Tenant,LineID,Date,Contact,AccountID,TaxCode,Amount,Currency\n";
    /// `(line, date, contact, account, tax code)`; every amount is 10.00.
    type Row<'a> = (&'a str, &'a str, &'a str, &'a str, &'a str);

    fn listing(rows: &[Row]) -> String {
        let mut text = HEADER.to_owned();
        for (id, date, contact, account, tax_code) in rows {
            text += &format!(
                "Fabricated Varrock,{id},{date},{contact},{account},{tax_code},10.00,AUD\n"
            );
        }
        text
    }

    fn run_text(text: &str) -> Result<Json> {
        let source = SourceSnapshot::new(Path::new("lines.csv"), text.as_bytes().to_vec());
        let start = Date::new(2026, 9, 1).unwrap();
        let end = Date::new(2026, 9, 30).unwrap();
        decimal::with_precision(40, || review(&source, "AUD", start, end))
    }

    fn text(value: &Json) -> String {
        value.as_pystr().unwrap().to_utf8().unwrap()
    }

    /// Each finding as `CODE line usual (usual lines of earlier lines)`.
    fn listed(rows: &[Row]) -> Vec<String> {
        let result = run_text(&listing(rows)).unwrap();
        result
            .at("findings")
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                format!(
                    "{} {} {} ({} of {})",
                    text(item.at("code")),
                    text(item.at("line_id")),
                    text(item.at("usual")),
                    pyjson::canonical(item.at("usual_lines")),
                    pyjson::canonical(item.at("earlier_lines"))
                )
            })
            .collect()
    }

    /// 4 earlier power bills on account 610 with GST on expenses.
    const POWER: [Row; 4] = [
        ("H1", "2026-05-10", "Lumbridge Power", "610", "INPUT"),
        ("H2", "2026-06-10", "lumbridge  POWER", "610", "INPUT"),
        ("H3", "2026-07-10", "Lumbridge Power", "610", "INPUT"),
        ("H4", "2026-08-10", "Lumbridge Power", "610", "INPUT"),
    ];

    #[test]
    fn a_line_coded_as_before_passes() {
        let mut rows = POWER.to_vec();
        rows.push(("P1", "2026-09-10", "Lumbridge Power", "610", "INPUT"));
        let result = run_text(&listing(&rows)).unwrap();
        assert!(result.at("status").is_str("PASS"));
        assert_eq!(pyjson::canonical(result.at("lines_in_period")), "1");
    }

    #[test]
    fn a_contact_coded_to_another_account_or_tax_code_is_listed() {
        let mut rows = POWER.to_vec();
        rows.extend([
            ("P1", "2026-09-10", "Lumbridge Power", "620", "INPUT"),
            (
                "P2",
                "2026-09-11",
                "Lumbridge Power",
                "610",
                "EXEMPTEXPENSES",
            ),
        ]);
        assert_eq!(
            listed(&rows),
            [
                "UNUSUAL_ACCOUNT P1 610 (4 of 4)",
                "UNUSUAL_TAX_CODE P2 INPUT (4 of 4)",
            ]
        );
    }

    #[test]
    fn an_account_s_usual_tax_code_covers_a_new_contact_or_no_contact() {
        let mut rows = POWER.to_vec();
        rows.extend([
            (
                "P1",
                "2026-09-10",
                "Draynor Energy",
                "610",
                "EXEMPTEXPENSES",
            ),
            ("P2", "2026-09-12", "", "610", "BASEXCLUDED"),
            ("P3", "2026-09-13", "", "610", "INPUT"),
        ]);
        assert_eq!(
            listed(&rows),
            [
                "UNUSUAL_ACCOUNT_TAX_CODE P1 INPUT (4 of 4)",
                "UNUSUAL_ACCOUNT_TAX_CODE P2 INPUT (4 of 4)",
            ]
        );
    }

    #[test]
    fn too_little_or_mixed_history_sets_no_usual_coding() {
        // 3 of 4 is below 80%, and 2 lines are too few.
        let rows = [
            ("H1", "2026-06-01", "Falador Freight", "630", "INPUT"),
            ("H2", "2026-07-01", "Falador Freight", "630", "INPUT"),
            ("H3", "2026-07-15", "Falador Freight", "630", "INPUT"),
            (
                "H4",
                "2026-08-01",
                "Falador Freight",
                "640",
                "EXEMPTEXPENSES",
            ),
            ("H5", "2026-08-02", "Draynor Supplies", "650", "INPUT"),
            ("H6", "2026-08-03", "Draynor Supplies", "650", "INPUT"),
            ("P1", "2026-09-01", "Falador Freight", "640", "INPUT"),
            (
                "P2",
                "2026-09-02",
                "Draynor Supplies",
                "660",
                "GSTONIMPORTS",
            ),
        ];
        assert!(listed(&rows).is_empty());
        let result = run_text(&listing(&rows)).unwrap();
        assert!(result.at("status").is_str("PASS"));
    }

    #[test]
    fn exactly_the_usual_share_is_enough() {
        let mut rows = POWER.to_vec();
        rows.extend([
            ("H5", "2026-08-20", "Lumbridge Power", "611", "INPUT"),
            ("P1", "2026-09-10", "Lumbridge Power", "611", "INPUT"),
        ]);
        assert_eq!(listed(&rows), ["UNUSUAL_ACCOUNT P1 610 (4 of 5)"]);
    }

    #[test]
    fn a_separator_in_a_name_cannot_join_another_contact_s_history() {
        let joined = [
            ("H1", "2026-06-01", "A|B", "c", "X"),
            ("H2", "2026-07-01", "A|B", "c", "X"),
            ("H3", "2026-08-01", "A|B", "c", "X"),
        ];
        let mut rows = joined.to_vec();
        rows.extend([
            ("H4", "2026-06-02", "Z", "b|c", "Y"),
            ("H5", "2026-07-02", "Z", "b|c", "Y"),
            ("H6", "2026-08-02", "Z", "b|c", "Y"),
            ("P1", "2026-09-01", "A", "b|c", "X"),
        ]);
        assert_eq!(listed(&rows), ["UNUSUAL_ACCOUNT_TAX_CODE P1 Y (3 of 3)"]);
        let mut rows = joined.to_vec();
        rows.push(("P1", "2026-09-01", "A", "b|c", "Y"));
        assert!(listed(&rows).is_empty());
    }

    #[test]
    fn a_listing_with_nothing_in_the_period_needs_review() {
        let result = run_text(&listing(&POWER)).unwrap();
        assert!(result.at("status").is_str("REVIEW"));
        assert!(result.at("findings").as_array().unwrap().is_empty());
    }

    #[test]
    fn malformed_listings_are_refused() {
        let refused = |text: &str, message: &str| {
            let error = run_text(text).unwrap_err();
            assert!(error.contains(message), "{error}");
        };
        let one = listing(&POWER[..1]);
        refused(HEADER, "no lines were supplied");
        refused(&listing(&[POWER[0], POWER[0]]), "repeats LineID 'H1'");
        refused(
            &listing(&[("L", "2026-10-01", "A", "1", "INPUT")]),
            "after --period-end",
        );
        refused(
            &one.replace(",AUD\n", ",NZD\n"),
            "declared --currency 'AUD'",
        );
        refused(
            &(one.clone() + "Other,L2,2026-09-01,A,1,INPUT,1.00,AUD\n"),
            "one tenant",
        );
        refused(&one.replace(",10.00,", ",1.005,"), "two decimal places");
    }

    #[test]
    fn options_are_checked_before_reading() {
        let missing = Path::new("absent.csv");
        for (currency, start, end, message) in [
            ("aud", "2026-09-01", "2026-09-30", "three-letter currency"),
            (
                "AUD",
                "2026-9-01",
                "2026-09-30",
                "--period-start: Dates must use",
            ),
            ("AUD", "2026-09-01", "2026-09-31", "--period-end: day 31"),
            ("AUD", "2026-10-01", "2026-09-30", "must not follow"),
        ] {
            let error = coding_review(missing, currency, start, end).unwrap_err();
            assert!(error.contains(message), "{error}");
        }
    }
}
