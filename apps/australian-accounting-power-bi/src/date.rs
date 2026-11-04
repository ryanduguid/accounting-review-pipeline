//! The `datetime.date` arithmetic the generator uses: day steps, weekdays and ISO text.

use std::fmt;

/// A proleptic Gregorian date, held as days since 1970-01-01.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    days: i64,
}

pub fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl Date {
    /// `date(year, month, day)`; the callers pass real dates only.
    pub fn new(year: i32, month: u32, day: u32) -> Date {
        assert!(
            (1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day),
            "invalid date"
        );
        // Days from civil (H. Hinnant), with March as the first month of the computational year.
        let y = i64::from(year) - i64::from(month <= 2);
        let era = y.div_euclid(400);
        let year_of_era = y - era * 400;
        let shifted_month = i64::from((month + 9) % 12);
        let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        Date {
            days: era * 146_097 + day_of_era - 719_468,
        }
    }

    /// (year, month, day).
    pub fn ymd(self) -> (i32, u32, u32) {
        let z = self.days + 719_468;
        let era = z.div_euclid(146_097);
        let day_of_era = z - era * 146_097;
        let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
        let month = if shifted_month < 10 {
            shifted_month + 3
        } else {
            shifted_month - 9
        };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        (year as i32, month as u32, day as u32)
    }

    pub fn year(self) -> i32 {
        self.ymd().0
    }

    pub fn month(self) -> u32 {
        self.ymd().1
    }

    /// `date + timedelta(days=n)`.
    pub fn add_days(self, n: i64) -> Date {
        Date { days: self.days + n }
    }

    /// `(other - self).days`.
    pub fn days_until(self, other: Date) -> i64 {
        other.days - self.days
    }

    /// `date.weekday()`: Monday is 0. 1970-01-01 was a Thursday.
    pub fn weekday(self) -> u32 {
        (self.days + 3).rem_euclid(7) as u32
    }
}

/// `date.isoformat()` for four-digit years.
impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = self.ymd();
        write!(f, "{year:04}-{month:02}-{day:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_steps_across_month_year_and_leap_boundaries() {
        let mut day = Date::new(1999, 12, 25);
        let mut previous = day.ymd();
        for _ in 0..(366 * 40) {
            day = day.add_days(1);
            let (y, m, d) = day.ymd();
            assert_eq!(Date::new(y, m, d), day);
            let expected = if previous.2 < days_in_month(previous.0, previous.1) {
                (previous.0, previous.1, previous.2 + 1)
            } else if previous.1 < 12 {
                (previous.0, previous.1 + 1, 1)
            } else {
                (previous.0 + 1, 1, 1)
            };
            assert_eq!((y, m, d), expected);
            previous = (y, m, d);
        }
    }

    #[test]
    fn weekdays_and_text_match_python() {
        assert_eq!(Date::new(2026, 10, 13).weekday(), 1);
        assert_eq!(Date::new(2024, 2, 29).weekday(), 3);
        assert_eq!(Date::new(2000, 1, 1).weekday(), 5);
        assert_eq!(Date::new(2027, 7, 4).to_string(), "2027-07-04");
        assert_eq!(Date::new(2026, 9, 28).days_until(Date::new(2026, 10, 3)), 5);
        assert!(!is_leap(2100) && is_leap(2000) && is_leap(2024) && !is_leap(2026));
    }
}
