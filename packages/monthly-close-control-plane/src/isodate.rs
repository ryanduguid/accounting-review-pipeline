//! Extended Gregorian dates used by the input schemas and saved acknowledgements.
//! Calendar validation is adapted from the existing ledger gate's Date type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => 0,
        };
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

/// CPython's accepted basic and week-date forms. Schema loaders can then
/// distinguish a valid non-canonical date from an invalid calendar date.
pub fn python_date_fromisoformat(text: &str) -> Option<Date> {
    if let Some(date) = date_fromisoformat(text) {
        return Some(date);
    }
    let bytes = text.as_bytes();
    if !matches!(bytes.len(), 7 | 8 | 10) || !bytes.is_ascii() {
        return None;
    }
    let year: i32 = text.get(..4)?.parse().ok()?;
    if !(1..=9999).contains(&year) || !bytes[..4].iter().all(u8::is_ascii_digit) {
        return None;
    }
    if bytes.len() == 8 && bytes.iter().all(u8::is_ascii_digit) {
        return Date::new(year, text[4..6].parse().ok()?, text[6..].parse().ok()?);
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
    let leap = Date::new(year, 2, 29).is_some();
    let max_week = if (jan1 - 1) % 7 == 3 || (leap && (jan1 - 1) % 7 == 2) {
        53
    } else {
        52
    };
    if !(1..=max_week).contains(&week) || !(1..=7).contains(&day) {
        return None;
    }
    let jan4 = jan1 + 3;
    let target = jan4 - (jan4 - 1) % 7 + (week - 1) * 7 + day - 1;
    from_ordinal(target)
}

pub fn ordinal(date: Date) -> i32 {
    let y = date.year - 1;
    let mut days = y * 365 + y / 4 - y / 100 + y / 400 + date.day as i32;
    for month in 1..date.month {
        days += if month == 2 {
            if Date::new(date.year, 2, 29).is_some() {
                29
            } else {
                28
            }
        } else if matches!(month, 4 | 6 | 9 | 11) {
            30
        } else {
            31
        };
    }
    days
}

pub fn from_ordinal(value: i32) -> Option<Date> {
    if !(1..=3_652_059).contains(&value) {
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
    for month in 1..=12 {
        let length = (28..=31)
            .rev()
            .find(|&day| Date::new(low, month, day).is_some())?;
        if remaining <= length as i32 {
            return Date::new(low, month, remaining as u32);
        }
        remaining -= length as i32;
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
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
