// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! The `prepayments` schedule: the unexpired part of each prepayment and the
//! outstanding accruals at the cutoff, tied to the ledger balances supplied.
use crate::{
    Result,
    decimal::{Decimal, Rounding},
    isodate::{self, Date},
    pyjson::{Json, PyStr},
    pytext,
    schedules::{ScheduleFields, add, date, exact, finding, money, number, object, rows, text},
};
use std::collections::BTreeMap;

fn count(value: i32) -> Decimal {
    Decimal::parse(&value.to_string()).unwrap()
}

/// Months from the first of `start`'s month to the end of `end`'s month.
fn months(start: Date, end: Date) -> i32 {
    (end.year - start.year) * 12 + end.month as i32 - start.month as i32 + 1
}

fn month_end(day: Date) -> bool {
    Date::new(day.year, day.month, day.day + 1).is_none()
}

/// The unexpired part of a prepayment at `cutoff`, to the cent, half up.
pub(crate) fn unexpired(
    amount: &Decimal,
    start: Date,
    end: Date,
    basis: &str,
    cutoff: Date,
) -> Result<Decimal> {
    let (remaining, total) = if basis == "daily" {
        let first = isodate::ordinal(start);
        let last = isodate::ordinal(end);
        let after = isodate::ordinal(cutoff).max(first - 1).min(last);
        (last - after, last - first + 1)
    } else {
        if start.day != 1 || !month_end(end) || !month_end(cutoff) {
            return Err(
                "A monthly prepayment needs a start on the first of a month, and its end and the cutoff on a month end.".into(),
            );
        }
        let total = months(start, end);
        let elapsed = if cutoff < start {
            0
        } else {
            months(start, cutoff).min(total)
        };
        (total - elapsed, total)
    };
    Ok(amount
        .mul(&count(remaining))
        .div(&count(total))
        .quantize(-2, Rounding::HalfUp)
        .expect("a schedule amount below 10^16 fits 80 digits"))
}

/// A prepayment's `basis`: daily or monthly.
pub(crate) fn basis(row: &Json) -> Result<String> {
    let basis = row.at("basis").as_pystr().and_then(PyStr::to_utf8);
    basis
        .filter(|basis| ["daily", "monthly"].contains(&basis.as_str()))
        .ok_or_else(|| "basis must be daily or monthly.".into())
}

pub(crate) fn evidence(row: &Json, identifier: &PyStr, findings: &mut Vec<Json>) -> Result<()> {
    let evidence = row
        .at("evidence")
        .as_pystr()
        .ok_or("evidence must be text; use an empty string for missing evidence.")?;
    if evidence.0.iter().all(|&cp| pytext::is_space(cp)) {
        findings.push(finding("MISSING_EVIDENCE", identifier));
    }
    Ok(())
}

/// Each row of `list` with its item id and account, its description checked
/// as the row is reached, so a row's errors come before the next row's.
pub(crate) fn items<'a>(
    data: &'a Json,
    list: &str,
    fields: &str,
) -> Result<impl ExactSizeIterator<Item = Result<(&'a Json, PyStr, PyStr)>>> {
    Ok(rows(data.at(list), fields, "item_id", list)?
        .iter()
        .map(|row| {
            let identifier = row.at("item_id").as_pystr().unwrap().clone();
            let account = text(row.at("account_id"), "account_id")?;
            text(row.at("description"), "description")?;
            Ok((row, identifier, account))
        }))
}

fn prepayments(
    data: &Json,
    cutoff: Date,
    expected: &mut BTreeMap<PyStr, Decimal>,
    findings: &mut Vec<Json>,
) -> Result<Vec<Json>> {
    let entries = items(
        data,
        "prepayments",
        "item_id account_id description amount start end basis evidence",
    )?;
    let mut workpaper = Vec::with_capacity(entries.len());
    for entry in entries {
        let (row, identifier, account) = entry?;
        let amount = money(row.at("amount"), "prepayment amount")?;
        let start = date(row.at("start"), "prepayment start")?;
        let end = date(row.at("end"), "prepayment end")?;
        let basis = basis(row)?;
        if amount <= Decimal::zero() || end < start {
            return Err(
                "A prepayment needs a positive amount and an end on or after its start.".into(),
            );
        }
        let left = unexpired(&amount, start, end, &basis, cutoff)?;
        add(expected, account, &left)?;
        evidence(row, &identifier, findings)?;
        let mut item = row.clone();
        item.set("unexpired", number(&left));
        workpaper.push(item);
    }
    Ok(workpaper)
}

fn accruals(
    data: &Json,
    period_start: Date,
    cutoff: Date,
    expected: &mut BTreeMap<PyStr, Decimal>,
    findings: &mut Vec<Json>,
) -> Result<Vec<Json>> {
    let entries = items(
        data,
        "accruals",
        "item_id account_id description amount accrued reversed evidence",
    )?;
    let mut workpaper = Vec::with_capacity(entries.len());
    for entry in entries {
        let (row, identifier, account) = entry?;
        let amount = money(row.at("amount"), "accrual amount")?;
        let accrued = date(row.at("accrued"), "accrued")?;
        let reversed = match row.at("reversed") {
            Json::Null => None,
            value => Some(date(value, "reversed")?),
        };
        if amount <= Decimal::zero()
            || accrued > cutoff
            || reversed.is_some_and(|day| day < accrued)
        {
            return Err("An accrual needs a positive amount, a date on or before the cutoff and no reversal before it.".into());
        }
        let outstanding = reversed.is_none_or(|day| day > cutoff);
        let balance = if outstanding {
            exact(Decimal::zero().sub(&amount))?
        } else {
            Decimal::zero()
        };
        add(expected, account, &balance)?;
        if outstanding && accrued < period_start {
            findings.push(finding("UNREVERSED_ACCRUAL", &identifier));
        }
        evidence(row, &identifier, findings)?;
        let mut item = row.clone();
        item.set("outstanding", number(&balance));
        workpaper.push(item);
    }
    Ok(workpaper)
}

/// Compare each account's expected balance with the ledger's.
fn accounts(
    data: &Json,
    expected: &BTreeMap<PyStr, Decimal>,
    findings: &mut Vec<Json>,
) -> Result<Vec<Json>> {
    let mut ledger = BTreeMap::new();
    for row in rows(
        data.at("ledger_balances"),
        "account_id balance",
        "account_id",
        "ledger_balances",
    )? {
        let account = row.at("account_id").as_pystr().unwrap().clone();
        ledger.insert(account, money(row.at("balance"), "ledger balance")?);
    }
    let mut result = Vec::new();
    for (account, balance) in &ledger {
        if !expected.contains_key(account) && !balance.is_zero() {
            let mut item = finding("UNSCHEDULED_BALANCE", account);
            item.set("ledger", number(balance));
            findings.push(item);
        }
    }
    for (account, scheduled) in expected {
        let Some(balance) = ledger.get(account) else {
            let mut item = finding("NO_LEDGER_BALANCE", account);
            item.set("expected", number(scheduled));
            findings.push(item);
            continue;
        };
        let difference = exact(balance.sub(scheduled))?;
        if !difference.is_zero() {
            let mut item = finding("BALANCE_DIFFERENCE", account);
            item.set("difference", number(&difference));
            findings.push(item);
        }
        result.push(Json::object([
            ("account_id", Json::Str(account.clone())),
            ("expected", number(scheduled)),
            ("ledger", number(balance)),
            ("difference", number(&difference)),
        ]));
    }
    Ok(result)
}

pub(crate) fn review(data: &Json, cutoff: Date) -> Result<ScheduleFields> {
    object(
        data,
        "schema_version tenant currency cutoff period_start prepayments accruals ledger_balances",
        "prepayments",
    )?;
    let tenant = text(data.at("tenant"), "tenant")?;
    let period_start = date(data.at("period_start"), "period_start")?;
    if period_start > cutoff {
        return Err("period_start must not follow the cutoff.".into());
    }
    let mut expected = BTreeMap::new();
    let mut findings = Vec::new();
    let prepaid = prepayments(data, cutoff, &mut expected, &mut findings)?;
    let accrued = accruals(data, period_start, cutoff, &mut expected, &mut findings)?;
    if prepaid.is_empty() && accrued.is_empty() {
        return Err(
            "Supply the prepayments and accruals; an empty schedule cannot establish a balance."
                .into(),
        );
    }
    let accounts = accounts(data, &expected, &mut findings)?;
    Ok(vec![
        ("tenant", Json::Str(tenant)),
        ("period_start", data.at("period_start").clone()),
        ("prepayments", Json::Array(prepaid)),
        ("accruals", Json::Array(accrued)),
        ("accounts", Json::Array(accounts)),
        ("findings", Json::Array(findings)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decimal, loader::SourceSnapshot, pyjson, schedules::review_snapshot};
    use std::path::Path;

    fn example() -> Json {
        let path = crate::test_support::examples().join("schedules/prepayments.json");
        pyjson::loads(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn run(data: &Json) -> crate::Result<Json> {
        let snapshot = SourceSnapshot::new(
            Path::new("prepayments.json"),
            pyjson::canonical(data).into_bytes(),
        );
        review_snapshot(&snapshot, "prepayments")
    }

    /// Set `field` of the `index`th row of `list`.
    fn edit(data: &mut Json, list: &str, index: usize, field: &str, value: Json) {
        let Json::Object(pairs) = data else { panic!() };
        let (_, rows) = pairs.iter_mut().find(|(key, _)| key.is(list)).unwrap();
        let Json::Array(rows) = rows else { panic!() };
        rows[index].set(field, value);
    }

    fn codes(result: &Json) -> Vec<String> {
        result
            .at("findings")
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                let text = |key| item.at(key).as_pystr().unwrap().to_utf8().unwrap();
                format!("{} {}", text("code"), text("id"))
            })
            .collect()
    }

    fn field(result: &Json, list: &str, index: usize, key: &str) -> String {
        result.at(list).as_array().unwrap()[index]
            .at(key)
            .as_pystr()
            .unwrap()
            .to_utf8()
            .unwrap()
    }

    #[test]
    fn each_item_rounds_before_the_account_nets_its_accruals() {
        let row = |id: &str| {
            format!(
                r#"{{"item_id":"{id}","account_id":"625","description":"Fabricated","amount":"0.05","start":"2026-09-30","end":"2026-10-01","basis":"daily","evidence":"e"}}"#
            )
        };
        let data = pyjson::loads(&format!(
            r#"{{"schema_version":1,"currency":"AUD","cutoff":"2026-09-30","tenant":"Fabricated","period_start":"2026-09-01","prepayments":[{},{}],"accruals":[{{"item_id":"A","account_id":"625","description":"Fabricated","amount":"0.01","accrued":"2026-09-30","reversed":null,"evidence":"e"}}],"ledger_balances":[{{"account_id":"625","balance":"0.05"}}]}}"#,
            row("P1"),
            row("P2")
        ))
        .unwrap();
        let result = run(&data).unwrap();
        assert!(result.at("status").is_str("PASS"));
        assert_eq!(field(&result, "prepayments", 0, "unexpired"), "0.03");
    }

    #[test]
    fn an_earlier_row_reports_its_error_before_a_later_row() {
        for (list, message) in [
            ("prepayments", "A prepayment needs a positive amount"),
            ("accruals", "An accrual needs a positive amount"),
        ] {
            let mut data = example();
            edit(&mut data, list, 0, "amount", Json::str("0.00"));
            edit(&mut data, list, 1, "description", Json::str(""));
            let error = run(&data).unwrap_err();
            assert!(error.starts_with(message), "{error}");
        }
    }

    #[test]
    fn the_example_amortises_and_ties_out_each_account() {
        let result = run(&example()).unwrap();
        assert!(result.at("status").is_str("REVIEW"));
        assert_eq!(
            codes(&result),
            [
                "UNREVERSED_ACCRUAL A2",
                "MISSING_EVIDENCE A2",
                "BALANCE_DIFFERENCE 626"
            ]
        );
        assert_eq!(field(&result, "prepayments", 0, "unexpired"), "900.00");
        assert_eq!(field(&result, "prepayments", 1, "unexpired"), "350.00");
        assert_eq!(field(&result, "accruals", 0, "outstanding"), "-3000.00");
        assert_eq!(field(&result, "accruals", 2, "outstanding"), "0");
        assert_eq!(field(&result, "accounts", 1, "difference"), "15.00");
        assert_eq!(field(&result, "accounts", 2, "expected"), "-3450.00");
    }

    #[test]
    fn a_tied_out_schedule_passes() {
        let mut data = example();
        edit(
            &mut data,
            "ledger_balances",
            1,
            "balance",
            Json::str("350.00"),
        );
        edit(
            &mut data,
            "ledger_balances",
            2,
            "balance",
            Json::str("-3000.00"),
        );
        edit(
            &mut data,
            "accruals",
            1,
            "reversed",
            Json::str("2026-07-01"),
        );
        edit(&mut data, "accruals", 1, "evidence", Json::str("bill-A2"));
        let result = run(&data).unwrap();
        assert!(result.at("status").is_str("PASS"), "{:?}", codes(&result));
    }

    #[test]
    fn missing_and_unscheduled_ledger_balances_are_findings() {
        let mut data = example();
        edit(
            &mut data,
            "ledger_balances",
            0,
            "account_id",
            Json::str("999"),
        );
        edit(
            &mut data,
            "ledger_balances",
            1,
            "balance",
            Json::str("350.00"),
        );
        edit(&mut data, "ledger_balances", 2, "balance", Json::str("0"));
        edit(
            &mut data,
            "accruals",
            1,
            "reversed",
            Json::str("2026-09-30"),
        );
        edit(&mut data, "accruals", 1, "evidence", Json::str("bill-A2"));
        edit(
            &mut data,
            "accruals",
            0,
            "reversed",
            Json::str("2026-09-30"),
        );
        let result = run(&data).unwrap();
        assert_eq!(
            codes(&result),
            ["UNSCHEDULED_BALANCE 999", "NO_LEDGER_BALANCE 625"]
        );
    }

    #[test]
    #[expect(clippy::too_many_lines, reason = "one table of amortisation cases")]
    fn amortisation_covers_both_bases_and_rounds_half_up() {
        let day = |text| isodate::date_fromisoformat(text).unwrap();
        let amount = |text| Decimal::parse(text).unwrap();
        let cases = [
            (
                "1200.00",
                "2026-07-01",
                "2027-06-30",
                "monthly",
                "2026-06-30",
                "1200.00",
            ),
            (
                "1200.00",
                "2026-07-01",
                "2027-06-30",
                "monthly",
                "2027-06-30",
                "0.00",
            ),
            (
                "1200.00",
                "2026-07-01",
                "2027-06-30",
                "monthly",
                "2027-07-31",
                "0.00",
            ),
            (
                "100.00",
                "2026-09-01",
                "2026-09-03",
                "daily",
                "2026-09-01",
                "66.67",
            ),
            (
                "0.05",
                "2026-09-01",
                "2026-09-02",
                "daily",
                "2026-09-01",
                "0.03",
            ),
            (
                "10.00",
                "2026-09-05",
                "2026-09-06",
                "daily",
                "2026-09-01",
                "10.00",
            ),
            (
                "10.00",
                "2026-09-05",
                "2026-09-06",
                "daily",
                "2026-09-30",
                "0.00",
            ),
        ];
        for (value, start, end, basis, cutoff, expected) in cases {
            let left = decimal::with_precision(80, || {
                unexpired(&amount(value), day(start), day(end), basis, day(cutoff)).unwrap()
            });
            assert_eq!(left.to_sci_string(), expected, "{start} {basis} {cutoff}");
        }
        for (start, end, cutoff) in [
            ("2026-07-02", "2027-06-30", "2026-09-30"),
            ("2026-07-01", "2027-06-29", "2026-09-30"),
            ("2026-07-01", "2027-06-30", "2026-09-29"),
        ] {
            let error = unexpired(
                &amount("1.00"),
                day(start),
                day(end),
                "monthly",
                day(cutoff),
            );
            assert!(error.unwrap_err().starts_with("A monthly prepayment needs"));
        }
    }

    #[test]
    #[expect(clippy::too_many_lines, reason = "one table of refusals")]
    fn malformed_schedules_are_refused() {
        let cases: [(&str, usize, &str, Json, &str); 9] = [
            (
                "prepayments",
                0,
                "basis",
                Json::str("weekly"),
                "basis must be daily or monthly.",
            ),
            (
                "prepayments",
                0,
                "amount",
                Json::str("0"),
                "A prepayment needs a positive amount and an end on or after its start.",
            ),
            (
                "prepayments",
                0,
                "end",
                Json::str("2026-06-30"),
                "A prepayment needs a positive amount and an end on or after its start.",
            ),
            (
                "prepayments",
                0,
                "evidence",
                Json::Null,
                "evidence must be text; use an empty string for missing evidence.",
            ),
            (
                "accruals",
                0,
                "accrued",
                Json::str("2026-10-01"),
                "An accrual needs a positive amount, a date on or before the cutoff and no reversal before it.",
            ),
            (
                "accruals",
                2,
                "reversed",
                Json::str("2026-08-30"),
                "An accrual needs a positive amount, a date on or before the cutoff and no reversal before it.",
            ),
            (
                "accruals",
                0,
                "amount",
                Json::str("-1.00"),
                "An accrual needs a positive amount, a date on or before the cutoff and no reversal before it.",
            ),
            (
                "accruals",
                0,
                "reversed",
                Json::str("soon"),
                "reversed: expected YYYY-MM-DD.",
            ),
            (
                "ledger_balances",
                0,
                "balance",
                Json::str("x"),
                "ledger balance: expected a signed decimal string with at most 2 places.",
            ),
        ];
        for (list, index, key, value, error) in cases {
            let mut data = example();
            edit(&mut data, list, index, key, value);
            assert_eq!(run(&data).unwrap_err(), error, "{list} {key}");
        }
        let mut data = example();
        data.set("period_start", Json::str("2026-10-01"));
        assert_eq!(
            run(&data).unwrap_err(),
            "period_start must not follow the cutoff."
        );
        let mut data = example();
        data.set("prepayments", Json::Array(Vec::new()));
        data.set("accruals", Json::Array(Vec::new()));
        assert_eq!(
            run(&data).unwrap_err(),
            "Supply the prepayments and accruals; an empty schedule cannot establish a balance."
        );
    }

    #[test]
    fn a_schedule_with_the_wrong_shape_is_refused() {
        for list in ["prepayments", "accruals", "ledger_balances"] {
            let mut data = example();
            data.set(list, Json::Null);
            assert_eq!(
                run(&data).unwrap_err(),
                format!("{list}: expected an array.")
            );
        }
        let mut data = example();
        data.set("note", Json::Null);
        assert!(
            run(&data)
                .unwrap_err()
                .starts_with("prepayments: expected exactly")
        );
    }
}
