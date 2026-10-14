//! Retry-After date grammar, adapted from CPython 3.15 email._parseaddr.
use crate::pytext;
use chrono::{DateTime, NaiveDate, Utc};

fn integer(value: &str) -> Option<i64> {
    let value: String = value
        .chars()
        .filter(|&ch| ch != '_')
        .map(|ch| {
            pytext::decimal_value(ch as u32)
                .and_then(|n| char::from_digit(n, 10))
                .unwrap_or(ch)
        })
        .collect();
    value.parse().ok()
}

pub fn parse(value: &str) -> Option<DateTime<Utc>> {
    let mut fields: Vec<String> = value.split_whitespace().map(str::to_owned).collect();
    let first = fields.first()?;
    if first.ends_with(',')
        || ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
            .contains(&first.to_ascii_lowercase().as_str())
    {
        fields.remove(0);
    } else if let Some(index) = first.rfind(',') {
        fields[0] = first[index + 1..].into();
    }
    if fields.len() == 3 {
        let date: Vec<_> = fields[0].split('-').map(str::to_owned).collect();
        if date.len() == 3 {
            fields.splice(..1, date);
        }
    }
    if fields.len() == 4 {
        let time = &fields[3];
        if let Some(index) = time.find('+').or_else(|| time.find('-')).filter(|&i| i > 0) {
            let zone = time[index..].to_owned();
            fields[3] = time[..index].into();
            fields.push(zone);
        } else {
            fields.push(String::new());
        }
    }
    if fields.len() < 5 {
        return None;
    }
    let (mut day, mut month, mut year, mut time, mut zone) = (
        fields[0].clone(),
        fields[1].to_ascii_lowercase(),
        fields[2].clone(),
        fields[3].clone(),
        fields[4].clone(),
    );
    let months = [
        "jan",
        "feb",
        "mar",
        "apr",
        "may",
        "jun",
        "jul",
        "aug",
        "sep",
        "oct",
        "nov",
        "dec",
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
    if !months.contains(&month.as_str()) {
        (day, month) = (month, day.to_ascii_lowercase());
    }
    let month = (months.iter().position(|&m| m == month)? % 12 + 1) as u32;
    day = day.trim_end_matches(',').into();
    if year.find(':').is_some_and(|i| i > 0) {
        std::mem::swap(&mut year, &mut time);
    }
    year = year.trim_end_matches(',').into();
    if !year
        .chars()
        .next()
        .is_some_and(|c| pytext::decimal_value(c as u32).is_some())
    {
        std::mem::swap(&mut year, &mut zone);
    }
    let time = time.trim_end_matches(',');
    let parts: Vec<_> = if time.contains(':') {
        time.split(':').collect()
    } else {
        time.split('.').collect()
    };
    if !matches!(parts.len(), 2 | 3) {
        return None;
    }
    let mut year = integer(&year)?;
    if year < 100 {
        year += if year > 68 { 1900 } else { 2000 };
    }
    if !(1..=9999).contains(&year) {
        return None;
    }
    let date = NaiveDate::from_ymd_opt(year as i32, month, integer(&day)?.try_into().ok()?)?;
    let date = date.and_hms_opt(
        integer(parts[0])?.try_into().ok()?,
        integer(parts[1])?.try_into().ok()?,
        if parts.len() == 3 {
            integer(parts[2])?.try_into().ok()?
        } else {
            0
        },
    )?;
    let zone = zone.to_ascii_uppercase();
    let offset = match zone.as_str() {
        "UT" | "UTC" | "GMT" | "Z" => 0,
        "AST" | "EDT" => -400,
        "ADT" => -300,
        "EST" | "CDT" => -500,
        "CST" | "MDT" => -600,
        "MST" | "PDT" => -700,
        "PST" => -800,
        _ => integer(&zone).unwrap_or(0),
    };
    let absolute = offset.checked_abs()?;
    let seconds = (absolute / 100)
        .checked_mul(3600)?
        .checked_add((absolute % 100) * 60)?;
    if seconds >= 86400 {
        return None;
    }
    date.and_utc()
        .checked_sub_signed(chrono::Duration::seconds(if offset < 0 {
            -seconds
        } else {
            seconds
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn obsolete_naive_and_named_zone_dates() {
        for value in [
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
            "06 Nov 1994 08:49:37",
        ] {
            assert_eq!(parse(value).unwrap().timestamp(), 784111777);
        }
        assert_eq!(
            parse("Tue, 06 Nov 1994 08:49:37 EST").unwrap().timestamp(),
            784129777
        );
        assert!(parse("06 Nov 1994 25:00:00 GMT").is_none());
    }
}
