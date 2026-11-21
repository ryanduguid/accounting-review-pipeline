// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Extended Gregorian dates used by the input schemas and saved acknowledgements.
//! Calendar validation is adapted from the existing ledger gate's Date type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

pub(crate) const MAX_ORDINAL: i32 = 3_652_059;

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

impl Date {
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        let days = days_in_month(year, month);
        ((1..=9999).contains(&year) && day >= 1 && day <= days).then_some(Self { year, month, day })
    }
    pub fn iso(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}
pub fn date_fromisoformat(text: &str) -> Option<Date> {
    let bytes = text.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(i, b)| i != 4 && i != 7 && !b.is_ascii_digit())
    {
        return None;
    }
    Date::new(
        text[..4].parse().ok()?,
        text[5..7].parse().ok()?,
        text[8..].parse().ok()?,
    )
}

pub(crate) fn basic_calendar_parts(text: &str) -> Option<[&str; 3]> {
    let bytes = text.as_bytes();
    // CPython accepts a 10-byte basic date after reading the first 8 digits.
    if matches!(bytes.len(), 8 | 10) && bytes[..8].iter().all(u8::is_ascii_digit) {
        Some([&text[..4], &text[4..6], &text[6..8]])
    } else {
        None
    }
}

/// CPython's accepted basic and week-date forms. Schema loaders can then
/// distinguish a valid non-canonical date from an invalid calendar date.
pub fn python_date_fromisoformat(text: &str) -> Option<Date> {
    if let Some(date) = date_fromisoformat(text) {
        return Some(date);
    }
    if let Some([year, month, day]) = basic_calendar_parts(text) {
        return Date::new(year.parse().ok()?, month.parse().ok()?, day.parse().ok()?);
    }
    python_week_ordinal(text).and_then(from_ordinal)
}

pub(crate) fn python_week_ordinal(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    if !matches!(bytes.len(), 7 | 8 | 10) {
        return None;
    }
    let year: i32 = text.get(..4)?.parse().ok()?;
    if !(1..=9999).contains(&year) || !bytes[..4].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let separated = bytes[4] == b'-';
    let start = if separated { 5 } else { 4 };
    if bytes[start] != b'W' {
        return None;
    }
    let week_text = text.get(start + 1..start + 3)?;
    if !week_text.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let week: i32 = week_text.parse().ok()?;
    let end = start + 3;
    let day = if bytes.len() == end {
        1
    } else {
        let pos = end + usize::from(separated);
        if separated && bytes.get(end) != Some(&b'-') {
            return None;
        }
        let digit = *bytes.get(pos)?;
        if !digit.is_ascii_digit() {
            return None;
        }
        (digit - b'0') as i32
    };
    let jan1 = ordinal(Date::new(year, 1, 1)?);
    let leap = days_in_month(year, 2) == 29;
    let max_week = if (jan1 - 1) % 7 == 3 || (leap && (jan1 - 1) % 7 == 2) {
        53
    } else {
        52
    };
    if !(1..=max_week).contains(&week) || !(1..=7).contains(&day) {
        return None;
    }
    let jan4 = jan1 + 3;
    Some(jan4 - (jan4 - 1) % 7 + (week - 1) * 7 + day - 1)
}

pub fn ordinal(date: Date) -> i32 {
    let y = date.year - 1;
    let mut days = y * 365 + y / 4 - y / 100 + y / 400 + date.day as i32;
    for month in 1..date.month {
        days += days_in_month(date.year, month) as i32;
    }
    days
}

pub fn from_ordinal(value: i32) -> Option<Date> {
    if !(1..=MAX_ORDINAL).contains(&value) {
        return None;
    }
    let mut low = 1;
    let mut high = 9999;
    while low < high {
        let middle = (low + high + 1) / 2;
        if ordinal(Date::new(middle, 1, 1)?) <= value {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let mut remaining = value - ordinal(Date::new(low, 1, 1)?) + 1;
    for month in 1..12 {
        let length = days_in_month(low, month) as i32;
        if remaining <= length {
            return Date::new(low, month, remaining as u32);
        }
        remaining -= length;
    }
    Date::new(low, 12, remaining as u32)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic_iso_dates_match_cpython_byte_boundaries() {
        for (input, expected) in [
            ("20260101é", Some("2026-01-01")),
            ("2024W011é", Some("2024-01-01")),
            ("20240101xx", Some("2024-01-01")),
            ("2024022977", Some("2024-02-29")),
            ("20240101éé", None),
            ("2024W011𐍈", None),
            ("20240101a", None),
            ("2024-W0é-1", None),
        ] {
            let actual = python_date_fromisoformat(input).map(|date| date.iso());
            assert_eq!(actual.as_deref(), expected, "{input}");
        }
    }

    #[test]
    fn week_dates_match_cpython_at_year_boundaries() {
        for (input, expected) in [
            ("2026-W01", Some("2025-12-29")),
            ("2026W011", Some("2025-12-29")),
            // CPython accepts the trailing bytes in this 10-byte basic form.
            ("2026W011xx", Some("2025-12-29")),
            ("2020-W53-7", Some("2021-01-03")),
            ("9999-W52-5", Some("9999-12-31")),
            ("9999-W52-7", None),
            ("2026-W0X-1", None),
            ("2026-W01X1", None),
            ("2026-W01-X", None),
        ] {
            let actual = python_date_fromisoformat(input).map(|date| date.iso());
            assert_eq!(actual.as_deref(), expected, "{input}");
        }
    }

    #[test]
    fn ordinals_count_the_leap_day_before_march() {
        let date = Date::new(2024, 3, 1).unwrap();
        assert_eq!(ordinal(date), 738_946);
        assert_eq!(from_ordinal(738_946), Some(date));
    }

    #[test]
    fn extended_calendar_only() {
        for date in ["0001-01-01", "2000-02-29", "2026-03-31", "9999-12-31"] {
            assert_eq!(date_fromisoformat(date).unwrap().iso(), date);
        }
        for date in [
            "0000-01-01",
            "1900-02-29",
            "2026-02-29",
            "2026-13-01",
            "2026-01-00",
            "20260331",
            "2026-W01-1",
            "٢٠٢٦-03-31",
        ] {
            assert_eq!(date_fromisoformat(date), None);
        }
    }
}
