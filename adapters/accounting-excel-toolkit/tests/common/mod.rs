//! Shared readers, assertions and exact decimals for the static source checks.
//!
//! CI has no Excel or Power Query host, so these checks read the adapter's sources and
//! fabricated samples as text and pin the guards they must keep. Run the fixture scenarios
//! in Excel and Power Query before releasing changes to the modules.

#![allow(dead_code)]

use std::fmt;
use std::fs;
use std::iter::Sum;
use std::ops::{Add, Sub};
use std::path::Path;

use regex::Regex;

/// The adapter directory.
pub fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The repository that holds the adapter.
pub fn repository() -> &'static Path {
    root()
        .ancestors()
        .nth(2)
        .expect("the adapter sits 2 levels below the repository root")
}

/// `Path.read_text(encoding="utf-8")`: universal newlines turn `\r\n` and `\r` into `\n`.
pub fn read(path: &Path) -> String {
    let text =
        fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Compile a pattern that belongs to a check.
pub fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|error| panic!("invalid pattern {pattern:?}: {error}"))
}

/// `csv.reader(..., strict=True)` records, through the adapter's own port of CPython's
/// reader. A leading BOM is dropped, as `utf-8-sig` drops it.
pub fn records(name: &str, text: &str) -> Vec<Vec<String>> {
    accounting_excel_toolkit::aged::records(text.as_bytes())
        .unwrap_or_else(|error| panic!("{name} is not well-formed CSV: {error}"))
}

/// The records of a sample read with `newline=""`, so line ends inside quoted fields
/// survive. Reading is strict, which refuses only quoting the samples must not carry.
pub fn sample_records(relative: &str) -> Vec<Vec<String>> {
    let bytes =
        fs::read(root().join(relative)).unwrap_or_else(|error| panic!("{relative}: {error}"));
    let text =
        String::from_utf8(bytes).unwrap_or_else(|error| panic!("{relative} is not UTF-8: {error}"));
    records(relative, &text)
}

/// The field a check reads, or a failure naming the short record.
pub fn cell(row: &[String], index: usize) -> &str {
    row.get(index).map_or_else(
        || panic!("record {row:?} has no field {index}"),
        String::as_str,
    )
}

/// The record-width rule `Xero.TrialBalance.pq`, `Xero.AgedReceivables.pq` and
/// `Xero.AgedPayables.pq` share, ported: every record from the header down carries the
/// header's field count, or the export is refused.
///
/// `Csv.Document` takes each record's width from `Columns` alone. Without this check a
/// short record was padded with nulls, so a missing figure loaded as a genuine zero, and
/// a surplus field was dropped in silence. Records are framed the way
/// `Lines.FromText(..., QuoteStyle.Csv, false)` and `Csv.Document` frame them: quoted
/// newlines stay inside one record, and a blank separator line carries no fields to
/// check. Returns the header width and the records below it.
pub fn record_widths(
    text: &str,
    header_names: &[&str],
) -> Result<(usize, Vec<Vec<String>>), String> {
    let framed: Vec<(usize, Vec<String>)> = records("the export", text)
        .into_iter()
        .enumerate()
        .map(|(index, row)| (index + 1, row))
        .filter(|(_, row)| !(row.len() <= 1 && row.concat().trim().is_empty()))
        .collect();
    let (header_number, header) = framed
        .iter()
        .find(|(_, row)| {
            row.first()
                .is_some_and(|first| header_names.contains(&first.trim()))
        })
        .ok_or("header row not found")?;
    let width = header.len();
    let mut body = Vec::new();
    for (number, row) in &framed {
        if number <= header_number {
            continue;
        }
        if row.len() != width {
            return Err(format!(
                "CSV record {number} has {} fields; the header has {width}",
                row.len()
            ));
        }
        body.push(row.clone());
    }
    Ok((width, body))
}

/// Text under check, named in every failure.
pub struct Source {
    pub name: String,
    pub text: String,
}

impl Source {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            text: text.into(),
        }
    }

    /// A file under the adapter directory, read as `read` reads it.
    pub fn adapter(relative: &str) -> Self {
        Self::new(relative, read(&root().join(relative)))
    }

    /// A file under the repository root, read as `read` reads it.
    pub fn repository(relative: &str) -> Self {
        Self::new(relative, read(&repository().join(relative)))
    }

    pub fn has(&self, needle: &str) {
        self.has_all(&[needle]);
    }

    /// Every needle, reporting all that are missing at once.
    pub fn has_all(&self, needles: &[&str]) {
        let missing: Vec<_> = needles
            .iter()
            .filter(|needle| !self.text.contains(**needle))
            .collect();
        assert!(missing.is_empty(), "{} is missing {missing:#?}", self.name);
    }

    pub fn lacks(&self, needle: &str) {
        self.lacks_all(&[needle]);
    }

    /// No needle, reporting all that are present at once.
    pub fn lacks_all(&self, needles: &[&str]) {
        let present: Vec<_> = needles
            .iter()
            .filter(|needle| self.text.contains(**needle))
            .collect();
        assert!(
            present.is_empty(),
            "{} still contains {present:#?}",
            self.name
        );
    }

    /// `re.search` finds each pattern, reporting all that do not match at once.
    pub fn matches_all(&self, patterns: &[&str]) {
        let unmatched: Vec<_> = patterns
            .iter()
            .filter(|pattern| !regex(pattern).is_match(&self.text))
            .collect();
        assert!(
            unmatched.is_empty(),
            "{} does not match {unmatched:#?}",
            self.name
        );
    }

    pub fn matches(&self, pattern: &str) {
        self.matches_all(&[pattern]);
    }

    pub fn never_matches(&self, pattern: &str) {
        let found = regex(pattern).find(&self.text);
        assert!(
            found.is_none(),
            "{} matches {pattern:?} at {:?}",
            self.name,
            found.map(|found| found.as_str())
        );
    }

    /// The first position of the needle, which must be present.
    pub fn index(&self, needle: &str) -> usize {
        self.text
            .find(needle)
            .unwrap_or_else(|| panic!("{} is missing {needle:?}", self.name))
    }

    /// The last position of the needle, which must be present.
    pub fn rindex(&self, needle: &str) -> usize {
        self.text
            .rfind(needle)
            .unwrap_or_else(|| panic!("{} is missing {needle:?}", self.name))
    }

    pub fn count(&self, needle: &str) -> usize {
        self.text.matches(needle).count()
    }

    pub fn before(&self, first: &str, second: &str) {
        self.in_order(&[first, second]);
    }

    /// Each needle's first occurrence comes after the previous needle's.
    pub fn in_order(&self, needles: &[&str]) {
        let positions: Vec<_> = needles.iter().map(|needle| self.index(needle)).collect();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{}: expected this order, found positions {positions:?}: {needles:#?}",
            self.name
        );
    }

    /// The text from the needle's first occurrence to the end.
    pub fn starting_at(&self, needle: &str) -> Self {
        Self::new(
            format!("{} from {needle:?}", self.name),
            &self.text[self.index(needle)..],
        )
    }

    /// The text from the needle's last occurrence to the end.
    pub fn starting_at_last(&self, needle: &str) -> Self {
        Self::new(
            format!("{} from the last {needle:?}", self.name),
            &self.text[self.rindex(needle)..],
        )
    }

    /// The text up to the needle's first occurrence.
    pub fn until(&self, needle: &str) -> Self {
        Self::new(
            format!("{} up to {needle:?}", self.name),
            &self.text[..self.index(needle)],
        )
    }

    /// The text from the first occurrence of `start` to the first occurrence of `end`.
    pub fn between(&self, start: &str, end: &str) -> Self {
        self.before(start, end);
        Self::new(
            format!("{} from {start:?} to {end:?}", self.name),
            &self.text[self.index(start)..self.index(end)],
        )
    }
}

/// Fails on any CR or LF left once the CRLF pairs are removed. Both halves matter: a
/// CR-only module survives a bare-LF check untouched and imports into the VBE as one line.
pub fn assert_crlf_only(relative: &str) {
    let bytes =
        fs::read(root().join(relative)).unwrap_or_else(|error| panic!("{relative}: {error}"));
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"\r\n") {
            index += 2;
            continue;
        }
        match bytes[index] {
            b'\r' => panic!("{relative} has a bare CR at byte {index}"),
            b'\n' => panic!("{relative} has a bare LF at byte {index}"),
            _ => index += 1,
        }
    }
}

/// An exact decimal for the sums the earlier suite took with `decimal.Decimal`.
///
/// It reads plain decimal text (surrounding whitespace, an optional sign, digits and an
/// optional point), which is all the fabricated samples carry, and refuses anything else
/// rather than read it some other way. Equality is numeric, as for `Decimal`, and
/// `Display` keeps the scale, as `str(Decimal)` does for plain values.
#[derive(Clone, Copy)]
pub struct Dec {
    units: i128,
    scale: u32,
    negative: bool,
}

impl Dec {
    pub const ZERO: Self = Self {
        units: 0,
        scale: 0,
        negative: false,
    };

    pub fn parse(text: &str) -> Result<Self, String> {
        let trimmed = text.trim();
        let (negative, unsigned) = match trimmed.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        };
        let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
        let digits = format!("{whole}{fraction}");
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(format!("{text:?} is not plain decimal text"));
        }
        let magnitude: i128 = digits
            .parse()
            .map_err(|_| format!("{text:?} is too large"))?;
        let scale =
            u32::try_from(fraction.len()).map_err(|_| format!("{text:?} has too many places"))?;
        Ok(Self {
            units: if negative { -magnitude } else { magnitude },
            scale,
            negative,
        })
    }

    /// `Decimal(text)` for text that must be an amount.
    pub fn of(text: &str) -> Self {
        Self::parse(text).unwrap_or_else(|error| panic!("{error}"))
    }

    fn at(self, scale: u32) -> i128 {
        10_i128
            .checked_pow(scale - self.scale)
            .and_then(|factor| self.units.checked_mul(factor))
            .expect("amounts stay within i128")
    }
}

impl Add for Dec {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        let scale = self.scale.max(other.scale);
        let units = self
            .at(scale)
            .checked_add(other.at(scale))
            .expect("sums stay within i128");
        Self {
            units,
            scale,
            negative: units < 0,
        }
    }
}

impl Sub for Dec {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        self + Self {
            units: -other.units,
            scale: other.scale,
            negative: !other.negative,
        }
    }
}

impl Sum for Dec {
    fn sum<I: Iterator<Item = Self>>(values: I) -> Self {
        values.fold(Self::ZERO, Add::add)
    }
}

impl PartialEq for Dec {
    fn eq(&self, other: &Self) -> bool {
        let scale = self.scale.max(other.scale);
        self.at(scale) == other.at(scale)
    }
}

impl fmt::Display for Dec {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scale = self.scale as usize;
        let digits = format!("{:0>width$}", self.units.unsigned_abs(), width = scale + 1);
        let (whole, fraction) = digits.split_at(digits.len() - scale);
        let sign = if self.negative { "-" } else { "" };
        if scale == 0 {
            write!(out, "{sign}{whole}")
        } else {
            write!(out, "{sign}{whole}.{fraction}")
        }
    }
}

impl fmt::Debug for Dec {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "Decimal('{self}')")
    }
}
