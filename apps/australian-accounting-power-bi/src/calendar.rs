//! The verified sample business-day calendar behind Payday Super due dates.
//!
//! Reference snapshot checked 25 September 2026; see docs/compliance-methodology.md.
//! The supported receipt calendar is 1 January 2026 to 31 July 2027. Whole dates are
//! excluded for the December part-day holidays as a provisional sample assumption;
//! those events are labelled for calendar review in the report. Local substitute
//! holidays remain business days under the current-main rule: WA King's Birthday and
//! Melbourne Cup Day are deliberately absent.

use crate::date::Date;

pub const CALENDAR_START: (i32, u32, u32) = (2026, 1, 1);
pub const CALENDAR_END: (i32, u32, u32) = (2027, 7, 31);

/// Holidays that stop the national business-day clock, as (year, month, day).
pub const NATIONAL_HOLIDAYS: [(i32, u32, u32); 30] = [
    // 2026
    (2026, 3, 2),   // WA Labour Day
    (2026, 3, 9),   // Canberra Day; Labour Day; Eight Hours Day
    (2026, 4, 27),  // Anzac Day additional holiday
    (2026, 5, 4),   // May Day; Labour Day
    (2026, 6, 1),   // Reconciliation Day; WA Day
    (2026, 9, 25),  // Friday before the AFL Grand Final
    (2026, 12, 24), // Part-day holiday: sample excludes whole date
    (2026, 12, 31), // Part-day holiday: sample excludes whole date
    (2026, 1, 1),
    (2026, 1, 26),  // Australia Day
    (2026, 4, 3),   // Good Friday
    (2026, 4, 6),   // Easter Monday
    (2026, 4, 25),  // Anzac Day
    (2026, 6, 8),   // King's Birthday
    (2026, 8, 3),   // Picnic Day (NT)
    (2026, 10, 5),  // King's Birthday (Qld); Labour Day (NSW, SA, ACT)
    (2026, 12, 25), // Christmas Day
    (2026, 12, 28), // Boxing Day (Observed)
    // 2027
    (2027, 3, 1),  // WA Labour Day
    (2027, 3, 8),  // Canberra Day; Labour Day; Eight Hours Day
    (2027, 5, 3),  // May Day; Labour Day
    (2027, 5, 31), // Reconciliation Day
    (2027, 6, 7),  // WA Day
    (2027, 1, 1),
    (2027, 1, 26),
    (2027, 3, 26), // Good Friday
    (2027, 3, 29), // Easter Monday
    (2027, 4, 25), // Anzac Day
    (2027, 4, 26), // Anzac Day (Observed)
    (2027, 6, 14), // King's Birthday
];

pub fn is_holiday(day: Date) -> bool {
    NATIONAL_HOLIDAYS.contains(&day.ymd())
}

/// True for a weekday that is not a national holiday. Refuses dates outside the
/// verified calendar with Python's ValueError text.
pub fn is_national_business_day(day: Date) -> Result<bool, String> {
    let (start, end) = (CALENDAR_START, CALENDAR_END);
    if !(Date::new(start.0, start.1, start.2)..=Date::new(end.0, end.1, end.2)).contains(&day) {
        return Err(format!("Date outside the verified sample calendar: {day}"));
    }
    Ok(day.weekday() < 5 && !is_holiday(day))
}

/// The date `count` national business days after `start`.
pub fn add_business_days(start: Date, count: u32) -> Result<Date, String> {
    let (mut current, mut added) = (start, 0);
    while added < count {
        current = current.add_days(1);
        if is_national_business_day(current)? {
            added += 1;
        }
    }
    Ok(current)
}
