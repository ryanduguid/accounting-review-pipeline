use super::isodate::{date_fromisoformat, ordinal, python_date_fromisoformat};

pub fn cutoff(text: &str) -> bool {
    date_fromisoformat(text).is_some_and(|date| date.iso() == text)
}

fn components(text: &str) -> Option<[u32; 4]> {
    let bytes = text.as_bytes();
    let mut parts = [0; 4];
    let (mut pos, mut separated) = (0, false);
    for (index, part) in parts[..3].iter_mut().enumerate() {
        let digits = bytes.get(pos..pos + 2)?;
        if !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        *part = u32::from(digits[0] - b'0') * 10 + u32::from(digits[1] - b'0');
        pos += 2;
        let next = bytes.get(pos).copied();
        if index == 0 {
            separated = next == Some(b':');
        }
        if next.is_none() || index == 2 {
            break;
        }
        if separated && next != Some(b':') {
            return None;
        }
        pos += usize::from(separated);
    }
    if pos < bytes.len() {
        if !matches!(bytes[pos], b'.' | b',') {
            return None;
        }
        let fraction = &bytes[pos + 1..];
        if fraction.is_empty() || !fraction.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let prefix = &fraction[..fraction.len().min(6)];
        parts[3] = prefix
            .iter()
            .fold(0, |n, digit| n * 10 + u32::from(digit - b'0'))
            * 10u32.pow(6 - prefix.len() as u32);
    }
    Some(parts)
}

pub fn generated(text: &str) -> bool {
    let Some((day, time)) = text.split_once('T') else {
        return false;
    };
    let Some(date) = python_date_fromisoformat(day) else {
        return false;
    };
    let shape = if day.contains('W') {
        matches!(
            (day.contains('-'), day.len()),
            (true, 8 | 10) | (false, 7 | 8)
        )
    } else {
        matches!(day.len(), 8 | 10)
    };
    if !shape {
        return false;
    }
    let zone = time
        .find('-')
        .or_else(|| time.find('+'))
        .or_else(|| time.find('Z'));
    let Some(zone) = zone else {
        return false;
    };
    let Some(parts) = components(&time[..zone]) else {
        return false;
    };
    if parts[1] >= 60 || parts[2] >= 60 || parts[0] > 24 {
        return false;
    }
    if parts[0] == 24 && (parts[1..].iter().any(|&n| n != 0) || ordinal(date) == 3_652_059) {
        return false;
    }
    if time.as_bytes()[zone] == b'Z' {
        return zone + 1 == time.len();
    }
    let Some(offset) = components(&time[zone + 1..]) else {
        return false;
    };
    offset[0] < 24 && offset[1] < 60 && offset[2] < 60
}
