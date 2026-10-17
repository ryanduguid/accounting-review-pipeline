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
