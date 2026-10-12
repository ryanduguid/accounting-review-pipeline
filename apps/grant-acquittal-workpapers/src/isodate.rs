//! `datetime.date.fromisoformat` (CPython 3.11+ C implementation).

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(year) => 29,
        _ => 28,
    }
}

/// Proleptic Gregorian ordinal with 0001-01-01 as day 1, as `date.toordinal`.
fn ordinal(year: i32, month: u32, day: u32) -> i64 {
    let y = (year - 1) as i64;
    let before_year = y * 365 + y / 4 - y / 100 + y / 400;
    let before_month: u32 = (1..month).map(|m| days_in_month(year, m)).sum();
    before_year + before_month as i64 + day as i64
}

fn from_ordinal(mut n: i64) -> Result<Date, ParseError> {
    if n < 1 {
        return Err(None);
    }
    let mut year = 1;
    // Step by 400-year cycles (146097 days), then single years.
    year += ((n - 1) / 146_097 * 400) as i32;
    n = (n - 1) % 146_097 + 1;
    loop {
        let len = if is_leap(year) { 366 } else { 365 };
        if n <= len {
            break;
        }
        n -= len;
        year += 1;
    }
    let mut month = 1;
    while n > days_in_month(year, month) as i64 {
        n -= days_in_month(year, month) as i64;
        month += 1;
    }
    Date::new(year, month, n as u32)
}

/// A failed parse: `None` for a malformed string, or the range message
/// `date()` raises for fields that parse but name no real date.
pub type ParseError = Option<String>;

impl Date {
    /// `date(year, month, day)` and its range checks, in CPython's order.
    pub fn new(year: i32, month: u32, day: u32) -> Result<Date, ParseError> {
        if !(1..=9999).contains(&year) {
            return Err(Some(format!("year must be in 1..9999, not {year}")));
        }
        if !(1..=12).contains(&month) {
            return Err(Some(format!("month must be in 1..12, not {month}")));
        }
        let last = days_in_month(year, month);
        if day < 1 || day > last {
            return Err(Some(format!(
                "day {day} must be in range 1..{last} for month {month} in year {year}"
            )));
        }
        Ok(Date { year, month, day })
    }

    /// `str(date)`.
    pub fn iso(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

fn digits(bytes: &[u8], at: usize, count: usize) -> Result<u32, ParseError> {
    let slice = bytes.get(at..at + count).ok_or(None)?;
    if !slice.iter().all(u8::is_ascii_digit) {
        return Err(None);
    }
    Ok(slice.iter().fold(0, |acc, b| acc * 10 + (b - b'0') as u32))
}

/// `date.fromisoformat(text)`.
pub fn date_fromisoformat(text: &str) -> Result<Date, ParseError> {
    let bytes = text.as_bytes();
    if ![7, 8, 10].contains(&bytes.len()) {
        return Err(None);
    }
    let year = digits(bytes, 0, 4)? as i32;
    let mut p = 4;
    let separated = bytes.get(p) == Some(&b'-');
    if separated {
        p += 1;
    }
    if bytes.get(p) == Some(&b'W') {
        p += 1;
        let week = digits(bytes, p, 2)?;
        p += 2;
        let weekday = if p < bytes.len() {
            if separated {
                if bytes[p] != b'-' {
                    return Err(None);
                }
                p += 1;
            }
            digits(bytes, p, 1)?
        } else {
            1
        };
        return iso_week_date(year, week, weekday);
    }
    let month = digits(bytes, p, 2)?;
    p += 2;
    if separated {
        if bytes.get(p) != Some(&b'-') {
            return Err(None);
        }
        p += 1;
    }
    let day = digits(bytes, p, 2)?;
    Date::new(year, month, day)
}

fn iso_week_date(year: i32, week: u32, weekday: u32) -> Result<Date, ParseError> {
    if !(1..=9999).contains(&year) {
        return Err(None);
    }
    let first_weekday = (ordinal(year, 1, 1) + 6) % 7;
    let long_year = first_weekday == 3 || (first_weekday == 2 && is_leap(year));
    if week == 0 || week > 53 || (week == 53 && !long_year) || !(1..=7).contains(&weekday) {
        return Err(None);
    }
    let jan4 = ordinal(year, 1, 4);
    let week1_monday = jan4 - (jan4 + 6) % 7;
    from_ordinal(week1_monday + (week as i64 - 1) * 7 + weekday as i64 - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_grammar_matches_cpython() {
        let ok = |s: &str| date_fromisoformat(s).ok().map(|d| d.iso());
        assert_eq!(ok("2026-06-30").as_deref(), Some("2026-06-30"));
        assert_eq!(ok("20260630").as_deref(), Some("2026-06-30"));
        assert_eq!(ok("2026-W26").as_deref(), Some("2026-06-22"));
        assert_eq!(ok("2026W26").as_deref(), Some("2026-06-22"));
        assert_eq!(ok("2026-W26-2").as_deref(), Some("2026-06-23"));
        assert_eq!(ok("2026W262").as_deref(), Some("2026-06-23"));
        assert_eq!(ok("2026-W53-1").as_deref(), Some("2026-12-28"));
        assert_eq!(ok("2020-W53-7").as_deref(), Some("2021-01-03"));
        assert_eq!(ok("0001-01-01").as_deref(), Some("0001-01-01"));
        for bad in [
            "2026-06",
            "2026-6-30",
            "+2026-06-30",
            "2026-06-30 ",
            "0000-01-01",
            "2026-02-29",
            "2027-W53",
        ] {
            assert_eq!(ok(bad), None, "{bad}");
        }
        let error = |s: &str| date_fromisoformat(s).expect_err(s);
        assert_eq!(error("2026-13-01").as_deref(), Some("month must be in 1..12, not 13"));
        assert_eq!(
            error("2026-02-29").as_deref(),
            Some("day 29 must be in range 1..28 for month 2 in year 2026")
        );
        assert_eq!(
            error("2026-01-00").as_deref(),
            Some("day 0 must be in range 1..31 for month 1 in year 2026")
        );
        assert_eq!(error("0000-01-01").as_deref(), Some("year must be in 1..9999, not 0"));
        assert_eq!(
            error("9999-W52-6").as_deref(),
            Some("year must be in 1..9999, not 10000")
        );
        for malformed in ["2026-W54", "2027-W53", "2026-W00-1", "2026-W10-8", "0000-W01-1"] {
            assert_eq!(error(malformed), None, "{malformed}");
        }
    }
}
