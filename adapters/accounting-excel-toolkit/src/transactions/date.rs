//! The converter's three C-locale strptime forms and Gregorian calendar checks.
use super::text::{digit, space};

const MONTHS: &[&str] = &[
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    year: u32,
    month: u32,
    day: u32,
}

impl Date {
    fn new(year: u32, month: u32, day: u32) -> Option<Self> {
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap => 29,
            2 => 28,
            _ => 0,
        };
        ((1..=9999).contains(&year) && day >= 1 && day <= days).then_some(Self { year, month, day })
    }

    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

fn number(text: &str) -> Option<u32> {
    text.chars()
        .try_fold(0u32, |n, ch| n.checked_mul(10)?.checked_add(digit(ch)?))
}

fn day(text: &str) -> Option<u32> {
    let chars: Vec<_> = text.chars().collect();
    let valid = match chars.as_slice() {
        ['1'..='9'] => true,
        ['3', '0' | '1'] => true,
        ['1' | '2', last] => digit(*last).is_some(),
        ['0' | ' ', '1'..='9'] => true,
        _ => false,
    };
    valid
        .then(|| number(text.strip_prefix(' ').unwrap_or(text)))
        .flatten()
}

pub fn parse(text: &str, full_month: bool) -> Option<Date> {
    let (day_text, month_text, year_text) = if text.contains('/') && !full_month {
        let parts: Vec<_> = text.split('/').collect();
        if parts.len() != 3 || !parts[1].is_ascii() || !(1..=2).contains(&parts[1].len()) {
            return None;
        }
        (parts[0], parts[1], parts[2])
    } else {
        let start = usize::from(text.starts_with(' '));
        let split = text
            .char_indices()
            .find(|&(i, ch)| i >= start && space(ch))?
            .0;
        let rest = text[split..].trim_start_matches(space);
        let end = rest.find(space)?;
        (
            &text[..split],
            &rest[..end],
            rest[end..].trim_start_matches(space),
        )
    };
    if year_text.chars().count() != 4 {
        return None;
    }
    let month = if !full_month && text.contains('/') {
        number(month_text)?
    } else {
        let name = month_text.to_ascii_lowercase();
        MONTHS.iter().position(|month| {
            if full_month {
                *month == name
            } else {
                month[..3] == name
            }
        })? as u32
            + 1
    };
    Date::new(number(year_text)?, month, day(day_text)?)
}
