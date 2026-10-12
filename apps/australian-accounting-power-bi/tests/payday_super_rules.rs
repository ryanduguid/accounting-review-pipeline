//! Australian Payday Super statutory calculation rules.

mod common;

use std::collections::{BTreeSet, HashMap};

use australian_accounting_power_bi::calendar::{
    NATIONAL_HOLIDAYS, add_business_days, is_holiday, is_national_business_day,
};
use australian_accounting_power_bi::date::Date;
use australian_accounting_power_bi::decimal::Decimal;
use australian_accounting_power_bi::payday::{ADMIN_UPLIFT_RATE, GicRates, days_in_year, notional_earnings, quarter};
use common::*;

/// Independently transcribed from Fair Work's 2026 calendar, checked 25 September 2026.
/// The full-date treatment of December part-day holidays is a sample assumption.
const INDEPENDENT_2026_HOLIDAYS: [((i32, u32, u32), &str); 7] = [
    ((2026, 8, 3), "Picnic Day (NT)"),
    ((2026, 9, 25), "Friday before the AFL Grand Final"),
    ((2026, 12, 24), "Part-day holiday: sample assumption"),
    ((2026, 12, 31), "Part-day holiday: sample assumption"),
    ((2026, 10, 5), "King's Birthday (Qld); Labour Day (NSW, SA, ACT)"),
    ((2026, 12, 25), "Christmas Day"),
    ((2026, 12, 28), "Boxing Day (Monday observance)"),
];

/// Holidays local areas may replace with another day. The ATO's Payday Super deadline
/// guidance treats a public holiday for only part of a state or territory as a business
/// day; WA regions substitute the King's Birthday and councils may substitute Cup Day.
const PART_STATE_2026_HOLIDAYS: [((i32, u32, u32), &str); 2] = [
    ((2026, 9, 28), "King's Birthday (WA)"),
    ((2026, 11, 3), "Melbourne Cup Day (Vic)"),
];

fn d(text: &str) -> Decimal {
    Decimal::parse(text)
}

fn date(text: &str) -> Date {
    let parts: Vec<u32> = text.split('-').map(|part| part.parse().unwrap()).collect();
    Date::new(parts[0] as i32, parts[1], parts[2])
}

fn ymd(day: (i32, u32, u32)) -> Date {
    Date::new(day.0, day.1, day.2)
}

/// `assertAlmostEqual(a, b, places=2)`: the difference rounds to zero at two places.
fn almost_equal(a: &Decimal, b: &Decimal) -> bool {
    a.sub(b).round(2).is_zero()
}

fn payroll_rows() -> Vec<HashMap<String, String>> {
    csv_rows(&samples().join("sample-payroll-super.csv"))
}

/// National business days after `start`, up to and including `end`.
fn count_business_days(start: Date, end: Date) -> u32 {
    let (mut days, mut current) = (0, start);
    while current < end {
        current = current.add_days(1);
        if current.weekday() < 5 && !is_holiday(current) {
            days += 1;
        }
    }
    days
}

/// Payday Super SG rate must be 12.0% (in force from 1 July 2025 onwards).
#[test]
fn sg_statutory_rate_is_12_percent() {
    for row in payroll_rows() {
        let earnings = d(&row["QualifyingEarnings_CodeQ"]);
        let liability = d(&row["SuperLiability_CodeL"]);
        let expected = earnings.mul(&d("0.12")).round(2);
        assert!(
            almost_equal(&liability, &expected),
            "Event {} super liability does not match 12%",
            row["EventID"]
        );
    }
}

/// Under Payday Super (from 1 July 2026) the due date is exactly 7 national business days after payday.
#[test]
fn statutory_due_date_post_1_july_2026() {
    let mut checked = 0;
    for row in payroll_rows() {
        let (payday, due) = (date(&row["PayDate"]), date(&row["StatutoryDueDate"]));
        if payday < Date::new(2026, 7, 1) {
            continue;
        }
        checked += 1;
        assert_eq!(
            count_business_days(payday, due),
            7,
            "Event {}: due {due} after payday {payday}",
            row["EventID"]
        );
        assert!(
            due.weekday() < 5 && !is_holiday(due),
            "Event {}: due date {due} is not a business day",
            row["EventID"]
        );
    }
    assert!(checked > 0, "Fixture has no post-1 July 2026 pay events to check");
}

/// The calendar is checked against its sources, not against itself: NT Picnic Day was
/// once missing, so a 29 July 2026 payday fell due on 7 August rather than 10 August.
#[test]
fn sample_calendar_carries_the_independently_sourced_2026_holidays() {
    for (day, name) in INDEPENDENT_2026_HOLIDAYS {
        assert!(
            NATIONAL_HOLIDAYS.contains(&day),
            "{name} on {day:?} must stop the sample business-day clock"
        );
    }
}

/// A holiday some regions do not observe is a business day for Payday Super.
#[test]
fn holidays_local_areas_can_replace_do_not_stop_the_clock() {
    for (day, name) in PART_STATE_2026_HOLIDAYS {
        assert!(!NATIONAL_HOLIDAYS.contains(&day), "{name} on {day:?} is a business day");
    }
}

/// Recompute the shipped due dates without touching the generator's holiday list.
#[test]
fn committed_due_dates_recompute_from_the_independent_calendar() {
    let independent: BTreeSet<Date> = INDEPENDENT_2026_HOLIDAYS.iter().map(|(day, _)| ymd(*day)).collect();
    let due_date = |payday: Date| {
        let (mut business_days, mut current) = (0, payday);
        while business_days < 7 {
            current = current.add_days(1);
            if current.weekday() < 5 && !independent.contains(&current) {
                business_days += 1;
            }
        }
        current
    };
    // Every due date for a payday on or before 16 December 2026 lands inside that window.
    let (start, end) = (Date::new(2026, 7, 1), Date::new(2026, 12, 16));
    let mut checked = BTreeSet::new();
    for row in payroll_rows() {
        let payday = date(&row["PayDate"]);
        if (start..=end).contains(&payday) {
            checked.insert(payday);
            assert_eq!(
                date(&row["StatutoryDueDate"]),
                due_date(payday),
                "Event {}",
                row["EventID"]
            );
        }
    }
    // The 4 paydays whose windows cross one of these holidays, and the 26 August control.
    for (payday, due) in [
        ("2026-07-29", "2026-08-10"),
        ("2026-09-23", "2026-10-06"),
        ("2026-09-30", "2026-10-12"),
        ("2026-08-26", "2026-09-04"),
        ("2026-09-16", "2026-09-28"),
    ] {
        assert_eq!(due_date(date(payday)), date(due), "{payday}");
    }
    assert!(checked.contains(&date("2026-07-29")) && checked.contains(&date("2026-08-26")));
}

#[test]
fn new_holiday_boundaries_and_calendar_ceiling() {
    for (payday, expected) in [
        ("2026-10-28", "2026-11-06"),
        ("2026-12-16", "2026-12-30"),
        ("2027-02-24", "2027-03-09"),
        ("2027-05-26", "2027-06-08"),
        ("2027-06-02", "2027-06-15"),
    ] {
        assert_eq!(
            add_business_days(date(payday), 7).unwrap().to_string(),
            expected,
            "{payday}"
        );
    }
    let refused = is_national_business_day(Date::new(2027, 9, 1)).unwrap_err();
    assert!(refused.contains("outside the verified sample calendar"), "{refused}");
}

#[test]
fn published_october_gic_rate_and_future_projection() {
    let rates = GicRates::published();
    assert_eq!(*rates.annual_rate(Date::new(2026, 10, 1)), d("0.1151"));
    assert!(rates.is_published(Date::new(2026, 12, 31)));
    assert_eq!(*rates.annual_rate(Date::new(2027, 1, 1)), d("0.1151"));
    assert!(!rates.is_published(Date::new(2027, 1, 1)));
}

/// TAA 1953 s 8AAD divides the annual rate by the days in the calendar year, leap years included.
#[test]
fn gic_divisor_is_days_in_the_calendar_year() {
    assert_eq!(days_in_year(2026), 365);
    assert_eq!(days_in_year(2024), 366);
    assert_eq!(days_in_year(2100), 365, "Century years are not leap years");
    assert_eq!(days_in_year(2000), 366, "Years divisible by 400 are leap years");
}

/// SG charge = individual final shortfall + notional earnings + administrative uplift (+ nil choice loading).
#[test]
fn sg_charge_has_the_four_payday_super_components() {
    let rates = GicRates::published();
    let mut checked = 0;
    for row in payroll_rows()
        .iter()
        .filter(|row| row["ComplianceStatus"] == "LATE_BREACH")
    {
        checked += 1;
        let (due, received) = (date(&row["StatutoryDueDate"]), date(&row["FundReceiptDate"]));
        let interest = notional_earnings(&rates, &d(&row["SuperLiability_CodeL"]), due, received);
        // s 18D: the fund received the contribution before any modelled assessment, so the
        // individual final shortfall is nil and only notional earnings carry the uplift.
        let shortfall = d("0.0");
        let uplift = shortfall.add(&interest).mul(&d(ADMIN_UPLIFT_RATE)).round(2);
        let event = &row["EventID"];
        assert!(
            almost_equal(&d(&row["GIC_NominalInterest"]), &interest),
            "Event {event} notional earnings"
        );
        let charge = shortfall.add(&interest).add(&uplift).round(2);
        assert!(
            almost_equal(&d(&row["SGC_Shortfall"]), &charge),
            "Event {event} SG charge components"
        );
    }
    assert!(checked > 0, "Fixture has no LATE_BREACH events to check");
}

/// s 18D: a late contribution received before assessment leaves only notional earnings
/// and uplift, so the charge never includes the base liability itself.
#[test]
fn received_contribution_reduces_the_final_shortfall_to_nil() {
    let mut checked = 0;
    for row in payroll_rows()
        .iter()
        .filter(|row| row["ComplianceStatus"] == "LATE_BREACH")
    {
        checked += 1;
        let (base, charge) = (d(&row["SuperLiability_CodeL"]), d(&row["SGC_Shortfall"]));
        assert!(
            charge < base,
            "Event {} charge includes the base liability",
            row["EventID"]
        );
        assert!(
            charge > d("0.0"),
            "Event {} is late so the charge cannot be nil",
            row["EventID"]
        );
    }
    assert!(checked > 0, "Fixture has no LATE_BREACH events to check");
}

/// The s 18D offset requires received > due, so a timely payment cannot zero a real shortfall.
#[test]
fn a_contribution_received_by_the_due_day_is_not_offset() {
    let (rates, due, base) = (GicRates::published(), Date::new(2026, 7, 15), d("1000.0"));
    assert_eq!(notional_earnings(&rates, &base, due, due), d("0.0"));
    assert_eq!(notional_earnings(&rates, &base, due, due.add_days(-3)), d("0.0"));
    assert!(notional_earnings(&rates, &base, due, due.add_days(1)) > d("0.0"));
}

/// GIC resets quarterly; a published quarter uses its own rate and others are projections.
#[test]
fn gic_rate_is_selected_per_quarter_not_per_period() {
    assert_eq!(quarter(Date::new(2026, 7, 1)), (2026, 3));
    assert_eq!(quarter(Date::new(2026, 9, 30)), (2026, 3));
    assert_eq!(quarter(Date::new(2026, 10, 1)), (2026, 4));
    let rates = GicRates::published();
    assert!(rates.is_published(Date::new(2026, 8, 12)));
    assert_eq!(
        rates.annual_rate(Date::new(2026, 8, 12)),
        rates.rate_for((2026, 3)).unwrap()
    );
    // April-June 2027 is not a published quarter, so it falls back to the stated projection.
    assert!(!rates.is_published(Date::new(2027, 5, 20)));
    assert_eq!(rates.annual_rate(Date::new(2027, 5, 20)), rates.projected());
}

/// An accrual spanning 30 September must not be flattened onto one quarter's rate.
#[test]
fn notional_earnings_use_each_days_own_rate_across_a_quarter_boundary() {
    let rates = GicRates::published().with_rate((2026, 4), d("0.2000"));
    let (due, received, base) = (Date::new(2026, 9, 28), Date::new(2026, 10, 3), d("1000.0"));
    let mut balance = base.clone();
    let mut day = due.add_days(1);
    while day <= received {
        let rate = if day.month() <= 9 { d("0.1143") } else { d("0.2000") };
        balance = balance.add(&balance.mul(&rate.div(&Decimal::from_int(days_in_year(day.year())))));
        day = day.add_days(1);
    }
    let expected = balance.sub(&base).round(2);
    assert!(almost_equal(
        &notional_earnings(&rates, &base, due, received),
        &expected
    ));
    // A single-rate model would differ, which is the defect this guards.
    let growth = d("1").add(&d("0.1143").div(&Decimal::from_int(days_in_year(2026))));
    let compounded = (0..due.days_until(received)).fold(d("1"), |total, _| total.mul(&growth));
    let flat = base.mul(&compounded.sub(&d("1"))).round(2);
    assert!(!almost_equal(&expected, &flat));
}

/// A receipt strictly after the due date is LATE_BREACH with a nonzero charge; otherwise ON_TIME with none.
#[test]
fn compliance_status_flagging() {
    for row in payroll_rows() {
        let (received, due) = (date(&row["FundReceiptDate"]), date(&row["StatutoryDueDate"]));
        let (status, charge, event) = (&row["ComplianceStatus"], d(&row["SGC_Shortfall"]), &row["EventID"]);
        if received > due {
            assert_eq!(status, "LATE_BREACH", "Event {event} should be flagged LATE_BREACH");
            assert!(charge > d("0.0"), "Late event {event} must have non-zero SGC shortfall");
        } else {
            assert_eq!(status, "ON_TIME", "Event {event} should be flagged ON_TIME");
            assert!(charge.is_zero(), "On-time event {event} must have 0.0 SGC shortfall");
        }
    }
}
