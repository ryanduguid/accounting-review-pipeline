// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use super::isodate::{date_fromisoformat, ordinal, python_date_fromisoformat};

pub fn cutoff(text: &str) -> bool {
    date_fromisoformat(text).is_some_and(|date| date.iso() == text)
}

fn components(text: &str) -> Option<([u32; 4], bool)> {
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
        pos += 1;
        if index == 0 {
            separated = next == Some(b':');
        }
        if matches!(next, Some(b'.' | b',')) {
            if index < 2 || pos >= bytes.len() {
                return None;
            }
            break;
        } else if pos >= bytes.len() {
            return Some((parts, next.is_none()));
        } else if separated && next == Some(b':') {
            if index == 2 {
                return None;
            }
        } else if !separated {
            pos -= 1;
        } else {
            return None;
        }
    }
    let (fraction, complete) = fraction(&bytes[pos..])?;
    parts[3] = fraction;
    Some((parts, complete))
}

fn fraction(bytes: &[u8]) -> Option<(u32, bool)> {
    let prefix = &bytes[..bytes.len().min(6)];
    if prefix.is_empty() || !prefix.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let value = prefix
        .iter()
        .fold(0, |n, digit| n * 10 + u32::from(digit - b'0'))
        * 10u32.pow(6 - prefix.len() as u32);
    let mut pos = prefix.len();
    while bytes.get(pos).is_some_and(u8::is_ascii_digit) {
        pos += 1;
    }
    Some((value, pos == bytes.len()))
}

pub fn generated(text: &str) -> bool {
    let Some((day, time)) = text.split_once('T') else {
        return false;
    };
    let Some(date) = python_date_fromisoformat(day) else {
        return false;
    };
    let zone = time.find(['-', '+', 'Z']);
    let Some(zone) = zone else {
        return false;
    };
    // The pinned Python parser tolerates trailing main-time bytes before a zone.
    let Some((parts, _)) = components(&time[..zone]) else {
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
    let Some((offset, complete)) = components(&time[zone + 1..]) else {
        return false;
    };
    complete && offset[0] < 24 && offset[1] < 60 && offset[2] < 60
}
