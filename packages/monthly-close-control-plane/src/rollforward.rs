// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! The `rollforward` schedule: each prepayment, deferred revenue and accrual
//! account rolled from its opening balance through the period's additions and
//! releases to its closing balance, tied to the ledger at both ends.
use crate::{
    Result,
    decimal::Decimal,
    isodate::{self, Date},
    prepayments::{accrual_items, basis, evidence, items, unexpired},
    pyjson::{Json, PyStr},
    schedules::{ScheduleFields, date, exact, finding, money, number, object, rows, text},
};
use std::collections::{BTreeMap, BTreeSet};

const DEFERRAL_FIELDS: &str =
    "item_id account_id description amount recorded start end basis evidence";
const ROLL: [&str; 4] = ["opening", "additions", "releases", "closing"];

/// Opening, additions, releases and closing, debit-positive: closing is the
/// sum of the other 3.
type Roll = [Decimal; 4];

fn zero_roll() -> Roll {
    std::array::from_fn(|_| Decimal::zero())
}

fn negate(value: &Decimal) -> Result<Decimal> {
    exact(Decimal::zero().sub(value))
}

/// The roll whose releases make opening plus additions reach closing.
fn roll(opening: Decimal, additions: Decimal, closing: Decimal) -> Result<Roll> {
    let releases = exact(exact(closing.sub(&opening))?.sub(&additions))?;
    Ok([opening, additions, releases, closing])
}

fn record(totals: &mut BTreeMap<PyStr, Roll>, account: PyStr, item: &Roll) -> Result<()> {
    let total = totals.entry(account).or_insert_with(zero_roll);
    for (sum, value) in total.iter_mut().zip(item) {
        *sum = exact(sum.add(value))?;
    }
    Ok(())
}

fn with_roll(row: &Json, item: &Roll) -> Json {
    let mut row = row.clone();
    for (key, value) in ROLL.iter().zip(item) {
        row.set(key, number(value));
    }
    row
}

/// Prepayments, which are assets, or deferred revenue, a liability whose
/// balances are credits.
fn deferrals(
    data: &Json,
    list: &str,
    period: (Date, Date),
    totals: &mut BTreeMap<PyStr, Roll>,
    findings: &mut Vec<Json>,
) -> Result<Vec<Json>> {
    let (period_start, cutoff) = period;
    let credit = list == "deferred_revenue";
    let before = isodate::from_ordinal(isodate::ordinal(period_start) - 1)
        .ok_or("period_start must follow 0001-01-01.")?;
    let entries = items(data, list, DEFERRAL_FIELDS)?;
    let mut workpaper = Vec::with_capacity(entries.len());
    for entry in entries {
        let (row, identifier, account) = entry?;
        let amount = money(row.at("amount"), &format!("{list} amount"))?;
        let recorded = date(row.at("recorded"), &format!("{list} recorded"))?;
        let start = date(row.at("start"), &format!("{list} start"))?;
        let end = date(row.at("end"), &format!("{list} end"))?;
        let basis = basis(row)?;
        if amount <= Decimal::zero() || end < start || recorded > cutoff {
            return Err(format!(
                "Each of {list} needs a positive amount, an end on or after its start and a recorded date on or before the cutoff."
            ));
        }
        if basis == "monthly" && period_start.day != 1 {
            return Err("A monthly item needs period_start on the first of a month.".into());
        }
        let closing = unexpired(&amount, start, end, &basis, cutoff)?;
        // An item recorded before the period is brought forward at its
        // unexpired part the day before; a later one is added in full.
        let (opening, additions) = if recorded < period_start {
            (
                unexpired(&amount, start, end, &basis, before)?,
                Decimal::zero(),
            )
        } else {
            (Decimal::zero(), amount)
        };
        let mut item = roll(opening, additions, closing)?;
        if credit {
            for value in &mut item {
                *value = negate(value)?;
            }
        }
        record(totals, account, &item)?;
        evidence(row, &identifier, findings)?;
        workpaper.push(with_roll(row, &item));
    }
    Ok(workpaper)
}

fn accruals(
    data: &Json,
    (period_start, cutoff): (Date, Date),
    totals: &mut BTreeMap<PyStr, Roll>,
    findings: &mut Vec<Json>,
) -> Result<Vec<Json>> {
    let entries = accrual_items(data, cutoff)?;
    let mut workpaper = Vec::with_capacity(entries.len());
    for entry in entries {
        let (row, identifier, account, (amount, accrued, reversed)) = entry?;
        let credit = negate(&amount)?;
        let open_after = |day: Date| reversed.is_none_or(|reversal| reversal > day);
        let brought_forward = accrued < period_start;
        let pick = |held: bool| {
            if held {
                credit.clone()
            } else {
                Decimal::zero()
            }
        };
        let opening = pick(brought_forward && reversed.is_none_or(|day| day >= period_start));
        let closing = pick(open_after(cutoff));
        let item = roll(opening, pick(!brought_forward), closing)?;
        if brought_forward && open_after(cutoff) {
            findings.push(finding("UNREVERSED_ACCRUAL", &identifier));
        }
        record(totals, account, &item)?;
        evidence(row, &identifier, findings)?;
        workpaper.push(with_roll(row, &item));
    }
    Ok(workpaper)
}

fn ledger(data: &Json, list: &str) -> Result<BTreeMap<PyStr, Decimal>> {
    let mut balances = BTreeMap::new();
    for row in rows(data.at(list), "account_id balance", "account_id", list)? {
        let account = row.at("account_id").as_pystr().unwrap().clone();
        balances.insert(account, money(row.at("balance"), list)?);
    }
    Ok(balances)
}

/// Each scheduled account's roll beside the ledger's opening and closing
/// balances, with a finding for each end that does not tie.
fn accounts(
    data: &Json,
    totals: &BTreeMap<PyStr, Roll>,
    findings: &mut Vec<Json>,
) -> Result<Vec<Json>> {
    let opening = ledger(data, "opening_balances")?;
    let closing = ledger(data, "ledger_balances")?;
    let ledger_accounts: BTreeSet<&PyStr> = opening
        .iter()
        .chain(&closing)
        .filter(|(_, balance)| !balance.is_zero())
        .map(|(account, _)| account)
        .collect();
    for account in ledger_accounts {
        if !totals.contains_key(account) {
            findings.push(finding("UNSCHEDULED_BALANCE", account));
        }
    }
    let mut result = Vec::new();
    for (account, item) in totals {
        let mut row = Json::object([("account_id", Json::Str(account.clone()))]);
        for (key, value) in ROLL.iter().zip(item) {
            row.set(key, number(value));
        }
        for (end, balances, scheduled, code) in [
            ("opening", &opening, &item[0], "OPENING_DIFFERENCE"),
            ("closing", &closing, &item[3], "BALANCE_DIFFERENCE"),
        ] {
            let Some(balance) = balances.get(account) else {
                let mut missing = finding("NO_LEDGER_BALANCE", account);
                missing.set("end", Json::str(end));
                findings.push(missing);
                row.set(&format!("ledger_{end}"), Json::Null);
                continue;
            };
            let difference = exact(balance.sub(scheduled))?;
            if !difference.is_zero() {
                let mut item = finding(code, account);
                item.set("difference", number(&difference));
                findings.push(item);
            }
            row.set(&format!("ledger_{end}"), number(balance));
            row.set(&format!("{end}_difference"), number(&difference));
        }
        result.push(row);
    }
    Ok(result)
}

pub(crate) fn review(data: &Json, cutoff: Date) -> Result<ScheduleFields> {
    object(
        data,
        "schema_version tenant currency cutoff period_start prepayments deferred_revenue accruals opening_balances ledger_balances",
        "rollforward",
    )?;
    let tenant = text(data.at("tenant"), "tenant")?;
    let period_start = date(data.at("period_start"), "period_start")?;
    if period_start > cutoff {
        return Err("period_start must not follow the cutoff.".into());
    }
    let period = (period_start, cutoff);
    let mut totals = BTreeMap::new();
    let mut findings = Vec::new();
    let prepaid = deferrals(data, "prepayments", period, &mut totals, &mut findings)?;
    let deferred = deferrals(data, "deferred_revenue", period, &mut totals, &mut findings)?;
    let accrued = accruals(data, period, &mut totals, &mut findings)?;
    if totals.is_empty() {
        return Err(
            "Supply the prepayments, deferred revenue and accruals; an empty schedule cannot establish a balance."
                .into(),
        );
    }
    let accounts = accounts(data, &totals, &mut findings)?;
    Ok(vec![
        ("tenant", Json::Str(tenant)),
        ("period_start", data.at("period_start").clone()),
        ("prepayments", Json::Array(prepaid)),
        ("deferred_revenue", Json::Array(deferred)),
        ("accruals", Json::Array(accrued)),
        ("accounts", Json::Array(accounts)),
        ("findings", Json::Array(findings)),
    ])
}

#[cfg(test)]
mod tests {
    use crate::pyjson::Json;
    use crate::{loader::SourceSnapshot, pyjson, schedules::review_snapshot};
    use std::path::Path;

    fn example() -> Json {
        let path = crate::test_support::examples().join("schedules/rollforward.json");
        pyjson::loads(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn run(data: &Json) -> crate::Result<Json> {
        let snapshot = SourceSnapshot::new(
            Path::new("rollforward.json"),
            pyjson::canonical(data).into_bytes(),
        );
        review_snapshot(&snapshot, "rollforward")
    }

    fn set(data: &mut Json, field: &str, value: Json) {
        data.set(field, value);
    }

    /// Set `field` of the `index`th row of `list`.
    fn edit(data: &mut Json, list: &str, index: usize, field: &str, value: Json) {
        let Json::Object(pairs) = data else { panic!() };
        let (_, rows) = pairs.iter_mut().find(|(key, _)| key.is(list)).unwrap();
        let Json::Array(rows) = rows else { panic!() };
        rows[index].set(field, value);
    }

    fn text(value: &Json) -> String {
        value.as_pystr().unwrap().to_utf8().unwrap()
    }

    fn codes(result: &Json) -> Vec<String> {
        let items = result.at("findings").as_array().unwrap();
        items
            .iter()
            .map(|item| format!("{} {}", text(item.at("code")), text(item.at("id"))))
            .collect()
    }

    /// Each row of `list` as `id opening additions releases closing`.
    fn rolls(result: &Json, list: &str, id: &str) -> Vec<String> {
        let items = result.at(list).as_array().unwrap();
        items
            .iter()
            .map(|item| {
                let parts: Vec<_> = super::ROLL.iter().map(|key| text(item.at(key))).collect();
                format!("{} {}", text(item.at(id)), parts.join(" "))
            })
            .collect()
    }

    #[test]
    fn the_example_rolls_each_account_and_finds_the_unamortised_licence() {
        let result = run(&example()).unwrap();
        assert!(result.at("status").is_str("REVIEW"));
        assert_eq!(
            rolls(&result, "prepayments", "item_id"),
            ["P1 1000.00 0 -100.00 900.00", "P2 0 365.00 -15.00 350.00"]
        );
        assert_eq!(
            rolls(&result, "deferred_revenue", "item_id"),
            ["D1 -2400.00 0 200.00 -2200.00", "D2 0 -500.00 0.00 -500.00"]
        );
        assert_eq!(
            rolls(&result, "accruals", "item_id"),
            ["A1 0 -3000.00 0.00 -3000.00", "A2 -200.00 0 200.00 0"]
        );
        assert_eq!(
            rolls(&result, "accounts", "account_id"),
            [
                "625 1000.00 0 -100.00 900.00",
                "626 0 365.00 -15.00 350.00",
                "826 -200.00 -3000.00 200.00 -3000.00",
                "840 -2400.00 -500.00 200.00 -2700.00",
            ]
        );
        assert_eq!(codes(&result), ["BALANCE_DIFFERENCE 626"]);
        let finding = &result.at("findings").as_array().unwrap()[0];
        assert_eq!(text(finding.at("difference")), "15.00");
    }

    #[test]
    fn a_tied_schedule_passes() {
        let mut data = example();
        edit(
            &mut data,
            "ledger_balances",
            1,
            "balance",
            Json::str("350.00"),
        );
        let result = run(&data).unwrap();
        assert!(result.at("status").is_str("PASS"), "{result:?}");
        let accounts = result.at("accounts").as_array().unwrap();
        assert_eq!(text(accounts[0].at("opening_difference")), "0.00");
        assert_eq!(text(accounts[1].at("closing_difference")), "0.00");
    }

    #[test]
    fn each_end_that_does_not_tie_is_listed() {
        let mut data = example();
        edit(
            &mut data,
            "opening_balances",
            0,
            "balance",
            Json::str("1100.00"),
        );
        edit(
            &mut data,
            "opening_balances",
            3,
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
        edit(&mut data, "accruals", 0, "evidence", Json::str(" "));
        edit(&mut data, "accruals", 1, "reversed", Json::Null);
        edit(
            &mut data,
            "ledger_balances",
            2,
            "balance",
            Json::str("-3200.00"),
        );
        let result = run(&data).unwrap();
        assert_eq!(
            codes(&result),
            [
                "MISSING_EVIDENCE A1",
                "UNREVERSED_ACCRUAL A2",
                "UNSCHEDULED_BALANCE 999",
                "OPENING_DIFFERENCE 625",
                "NO_LEDGER_BALANCE 840",
            ]
        );
        let accounts = result.at("accounts").as_array().unwrap();
        assert!(matches!(accounts[3].at("ledger_opening"), Json::Null));
    }

    #[test]
    fn items_on_the_period_boundaries_roll_as_their_dates_say() {
        let mut data = example();
        // P2 is recorded after its service ended, D2 part-way through its
        // service; A1 is reversed on the cutoff, A2 the day before the period.
        for (list, index, field, value) in [
            ("prepayments", 1, "amount", "62.00"),
            ("prepayments", 1, "start", "2026-07-01"),
            ("prepayments", 1, "end", "2026-08-31"),
            ("deferred_revenue", 1, "start", "2026-09-01"),
            ("deferred_revenue", 1, "end", "2026-09-30"),
            ("accruals", 0, "accrued", "2026-08-15"),
            ("accruals", 0, "reversed", "2026-09-30"),
            ("accruals", 1, "accrued", "2026-08-01"),
            ("accruals", 1, "reversed", "2026-08-31"),
        ] {
            edit(&mut data, list, index, field, Json::str(value));
        }
        let result = run(&data).unwrap();
        assert_eq!(
            rolls(&result, "prepayments", "item_id")[1],
            "P2 0 62.00 -62.00 0.00"
        );
        assert_eq!(
            rolls(&result, "deferred_revenue", "item_id")[1],
            "D2 0 -500.00 500.00 0.00"
        );
        assert_eq!(
            rolls(&result, "accruals", "item_id"),
            ["A1 -3000.00 0 3000.00 0", "A2 0 0 0 0"]
        );
    }

    #[test]
    fn an_accrual_raised_and_reversed_in_the_period_nets_to_nil() {
        let mut data = example();
        edit(&mut data, "accruals", 1, "accrued", Json::str("2026-09-02"));
        edit(
            &mut data,
            "accruals",
            1,
            "reversed",
            Json::str("2026-09-20"),
        );
        edit(
            &mut data,
            "opening_balances",
            2,
            "balance",
            Json::str("0.00"),
        );
        let result = run(&data).unwrap();
        assert_eq!(
            rolls(&result, "accruals", "item_id")[1],
            "A2 0 -200.00 200.00 0"
        );
    }

    #[test]
    fn malformed_schedules_are_refused() {
        let refused = |change: &dyn Fn(&mut Json), message: &str| {
            let mut data = example();
            change(&mut data);
            let error = run(&data).unwrap_err();
            assert!(error.contains(message), "{error}");
        };
        refused(
            &|d| edit(d, "prepayments", 0, "recorded", Json::str("2026-10-01")),
            "Each of prepayments needs",
        );
        refused(
            &|d| edit(d, "deferred_revenue", 0, "amount", Json::str("0.00")),
            "Each of deferred_revenue needs",
        );
        refused(
            &|d| set(d, "period_start", Json::str("2026-09-02")),
            "first of a month",
        );
        refused(
            &|d| set(d, "period_start", Json::str("2026-10-01")),
            "must not follow the cutoff",
        );
        refused(
            &|d| set(d, "period_start", Json::str("0001-01-01")),
            "must follow 0001-01-01",
        );
        refused(
            &|d| edit(d, "accruals", 0, "accrued", Json::str("2026-10-01")),
            "An accrual needs",
        );
        refused(
            &|d| edit(d, "accruals", 1, "reversed", Json::str("2026-08-01")),
            "An accrual needs",
        );
        refused(
            &|d| set(d, "opening_balances", Json::Null),
            "opening_balances",
        );
        refused(
            &|d| {
                for list in ["prepayments", "deferred_revenue", "accruals"] {
                    set(d, list, Json::Array(Vec::new()));
                }
            },
            "an empty schedule",
        );
        refused(&|d| set(d, "extra", Json::Null), "expected exactly");
    }
}
