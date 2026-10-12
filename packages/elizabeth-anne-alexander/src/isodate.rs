//! `datetime.date.fromisoformat` (CPython 3.11+ C implementation) and the
//! gateway's own timestamp grammar, which ends in `datetime.fromisoformat`.

use crate::pytext;

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

fn from_ordinal(mut n: i64) -> Option<Date> {
    if n < 1 {
        return None;
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
    (year <= 9999).then_some(Date {
        year,
        month,
        day: n as u32,
    })
}

impl Date {
    pub fn new(year: i32, month: u32, day: u32) -> Option<Date> {
        ((1..=9999).contains(&year) && (1..=12).contains(&month) && day >= 1 && day <= days_in_month(year, month))
            .then_some(Date { year, month, day })
    }

    /// `str(date)`.
    pub fn iso(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    fn next_day(&self) -> Option<Date> {
        from_ordinal(ordinal(self.year, self.month, self.day) + 1)
    }
}

fn digits(bytes: &[u8], at: usize, count: usize) -> Option<u32> {
    let slice = bytes.get(at..at + count)?;
    slice
        .iter()
        .all(u8::is_ascii_digit)
        .then(|| slice.iter().fold(0, |acc, b| acc * 10 + (b - b'0') as u32))
}

/// `date.fromisoformat(text)`.
pub fn date_fromisoformat(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    if ![7, 8, 10].contains(&bytes.len()) {
        return None;
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
                    return None;
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
            return None;
        }
        p += 1;
    }
    let day = digits(bytes, p, 2)?;
    Date::new(year, month, day)
}

fn iso_week_date(year: i32, week: u32, weekday: u32) -> Option<Date> {
    if !(1..=9999).contains(&year) {
        return None;
    }
    let first_weekday = (ordinal(year, 1, 1) + 6) % 7;
    let long_year = first_weekday == 3 || (first_weekday == 2 && is_leap(year));
    if week == 0 || week > 53 || (week == 53 && !long_year) || !(1..=7).contains(&weekday) {
        return None;
    }
    let jan4 = ordinal(year, 1, 4);
    let week1_monday = jan4 - (jan4 + 6) % 7;
    from_ordinal(week1_monday + (week as i64 - 1) * 7 + weekday as i64 - 1)
}

/// Matches `\d` (any Unicode decimal digit) `count` times; returns the ASCII digits or `None` if any is non-ASCII.
fn match_digits(chars: &[char], at: &mut usize, count: usize) -> Option<Option<String>> {
    let slice = chars.get(*at..*at + count)?;
    if !slice.iter().all(|&c| pytext::decimal_value(c as u32).is_some()) {
        return None;
    }
    *at += count;
    Some(slice.iter().all(char::is_ascii_digit).then(|| slice.iter().collect()))
}

fn match_char(chars: &[char], at: &mut usize, allowed: &[char]) -> Option<char> {
    let c = *chars.get(*at)?;
    allowed.contains(&c).then(|| {
        *at += 1;
        c
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum TimestampError {
    NoOffset,
    Invalid,
}

/// The gateway's `_iso_timestamp` grammar and validity check on stripped text.
pub fn check_timestamp(text: &str) -> Result<(), TimestampError> {
    let chars: Vec<char> = text.chars().collect();
    match match_full(&chars) {
        Some(parts) => validate(parts.ok_or(TimestampError::Invalid)?),
        None if matches_without_offset(&chars) => Err(TimestampError::NoOffset),
        None => Err(TimestampError::Invalid),
    }
}

struct Parts {
    date: [String; 3],
    clock: Vec<String>,
    fraction: Option<String>,
    offset: Option<(char, Vec<String>)>,
}

/// A left-to-right match over the timestamp grammar, noting any non-ASCII digit.
struct Scan<'c> {
    chars: &'c [char],
    at: usize,
    ascii: bool,
}

impl Scan<'_> {
    /// Exactly `n` decimal digits.
    fn digits(&mut self, n: usize) -> Option<String> {
        let got = match_digits(self.chars, &mut self.at, n)?;
        self.ascii &= got.is_some();
        Some(got.unwrap_or_default())
    }

    fn char(&mut self, allowed: &[char]) -> Option<char> {
        match_char(self.chars, &mut self.at, allowed)
    }

    /// `HH:MM` and an optional `:SS`, as the clock and the offset both allow.
    fn hours_minutes_seconds(&mut self) -> Option<Vec<String>> {
        let mut fields = vec![self.digits(2)?];
        self.char(&[':'])?;
        fields.push(self.digits(2)?);
        let mark = self.at;
        if self.char(&[':']).is_some() {
            match self.digits(2) {
                Some(seconds) => fields.push(seconds),
                None => self.at = mark,
            }
        }
        Some(fields)
    }

    /// An optional `.` and one to six digits.
    fn fraction(&mut self) -> Option<String> {
        if self.chars.get(self.at) != Some(&'.') {
            return None;
        }
        let start = self.at + 1;
        let mut end = start;
        while end < self.chars.len() && end - start < 6 && pytext::decimal_value(self.chars[end] as u32).is_some() {
            end += 1;
        }
        if end == start {
            return None;
        }
        self.ascii &= self.chars[start..end].iter().all(char::is_ascii_digit);
        self.at = end;
        Some(self.chars[start..end].iter().collect())
    }
}

/// `_TIMESTAMP.fullmatch`; the inner `None` means it matched with non-ASCII digits,
/// which `datetime.fromisoformat` then rejects.
fn match_full(chars: &[char]) -> Option<Option<Parts>> {
    let mut scan = Scan {
        chars,
        at: 0,
        ascii: true,
    };
    let year = scan.digits(4)?;
    scan.char(&['-'])?;
    let month = scan.digits(2)?;
    scan.char(&['-'])?;
    let day = scan.digits(2)?;
    scan.char(&['T', 't', ' '])?;
    let clock = scan.hours_minutes_seconds()?;
    let fraction = scan.fraction();
    let offset = match scan.char(&['Z', 'z', '+', '-'])? {
        'Z' | 'z' => None,
        sign => Some((sign, scan.hours_minutes_seconds()?)),
    };
    if scan.at != chars.len() {
        return None;
    }
    Some(scan.ascii.then_some(Parts {
        date: [year, month, day],
        clock,
        fraction,
        offset,
    }))
}

/// `_TIMESTAMP_WITHOUT_OFFSET.fullmatch`.
fn matches_without_offset(chars: &[char]) -> bool {
    let digit = |at: &mut usize, n: usize| match_digits(chars, at, n).is_some();
    let mut at = 0;
    if !(digit(&mut at, 4)
        && match_char(chars, &mut at, &['-']).is_some()
        && digit(&mut at, 2)
        && match_char(chars, &mut at, &['-']).is_some()
        && digit(&mut at, 2))
    {
        return false;
    }
    if at == chars.len() {
        return true;
    }
    if match_char(chars, &mut at, &['T', 't', ' ']).is_none()
        || !digit(&mut at, 2)
        || match_char(chars, &mut at, &[':']).is_none()
        || !digit(&mut at, 2)
    {
        return false;
    }
    if at < chars.len() && chars[at] == ':' {
        at += 1;
        if !digit(&mut at, 2) {
            return false;
        }
    }
    if at < chars.len() && chars[at] == '.' {
        at += 1;
        let start = at;
        while at < chars.len() && at - start < 6 && pytext::decimal_value(chars[at] as u32).is_some() {
            at += 1;
        }
        if at == start {
            return false;
        }
    }
    at == chars.len()
}

/// `datetime.fromisoformat` on the normalised form: field ranges, real dates,
/// an offset below 24 hours, and (Python 3.14+) `24:00:00` as the next midnight.
fn validate(parts: Parts) -> Result<(), TimestampError> {
    let n = |s: &str| s.parse::<u32>().expect("ASCII digits");
    let [year, month, day] = &parts.date;
    let date = Date::new(n(year) as i32, n(month), n(day)).ok_or(TimestampError::Invalid)?;
    let hour = n(&parts.clock[0]);
    let minute = n(&parts.clock[1]);
    let second = parts.clock.get(2).map_or(0, |s| n(s));
    let micro = parts.fraction.as_deref().map_or(0, n);
    if minute > 59 || second > 59 {
        return Err(TimestampError::Invalid);
    }
    if hour == 24 {
        if minute != 0 || second != 0 || micro != 0 || date.next_day().is_none() {
            return Err(TimestampError::Invalid);
        }
    } else if hour > 23 {
        return Err(TimestampError::Invalid);
    }
    if let Some((_, fields)) = parts.offset {
        let (h, m) = (n(&fields[0]), n(&fields[1]));
        let s = fields.get(2).map_or(0, |s| n(s));
        if h > 23 || m > 59 || s > 59 {
            return Err(TimestampError::Invalid);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_grammar_matches_cpython() {
        let ok = |s: &str| date_fromisoformat(s).map(|d| d.iso());
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
    }

    #[test]
    fn timestamps_match_the_gateway_grammar() {
        assert_eq!(check_timestamp("2026-06-30T10:00:00Z"), Ok(()));
        assert_eq!(check_timestamp("2026-06-30 10:00+10:00"), Ok(()));
        assert_eq!(check_timestamp("2026-06-30t10:00:00.5-00:00"), Ok(()));
        assert_eq!(check_timestamp("2026-06-30T24:00:00+00:00"), Ok(()));
        assert_eq!(check_timestamp("2026-06-30T10:00:00+23:59:59"), Ok(()));
        assert_eq!(check_timestamp("2026-06-30T10:00:00"), Err(TimestampError::NoOffset));
        assert_eq!(check_timestamp("2026-06-30"), Err(TimestampError::NoOffset));
        assert_eq!(check_timestamp("2026-06-30T10:00:60Z"), Err(TimestampError::Invalid));
        assert_eq!(
            check_timestamp("2026-06-30T10:00:00+24:00"),
            Err(TimestampError::Invalid)
        );
        assert_eq!(
            check_timestamp("2026-06-30T10:00:00.1234567Z"),
            Err(TimestampError::Invalid)
        );
        assert_eq!(check_timestamp("2026-06-30T10:00:00+10"), Err(TimestampError::Invalid));
        assert_eq!(
            check_timestamp("\u{663}026-06-30T10:00:00Z"),
            Err(TimestampError::Invalid)
        );
        assert_eq!(
            check_timestamp("\u{663}026-06-30T10:00:00"),
            Err(TimestampError::NoOffset)
        );
    }
}
