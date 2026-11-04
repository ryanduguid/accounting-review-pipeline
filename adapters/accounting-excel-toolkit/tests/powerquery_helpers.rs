// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Source pins on the shared Power Query helpers: `Fx.ABNIsValid`, `Fx.PromoteHeaderAt`
//! and `Fx.AUFinancialYear`, with specification ports where a vector documents the
//! contract.

mod common;

use std::fs;

use common::{Source, root};

const ABN: &str = "powerquery/Fx.ABNIsValid.pq";
const PROMOTER: &str = "powerquery/Fx.PromoteHeaderAt.pq";
const FINANCIAL_YEAR: &str = "powerquery/Fx.AUFinancialYear.pq";
const ABN_WEIGHTS: [u32; 11] = [10, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19];

#[test]
fn abn_validator_rejects_unexpected_formatting_and_conversion_errors() {
    let source = Source::adapter(ABN);
    source.has_all(&[
        "Text.From(abn ?? \"\", \"en-AU\")",
        "raw = if textAttempt[HasError] then \"\" else textAttempt[Value]",
        "allowedFormatting = Text.Select(raw, {\"0\"..\"9\", \" \"}) = raw",
        "digitsOnly = Text.Remove(raw, {\" \"})",
    ]);
    source.lacks("Text.Trim(");
}

/// Binding a guard is not using it.
///
/// An earlier version of this check compared one set against another and would have
/// passed with every guard removed from the M. Pin each clause into the `result`
/// conditional instead, so deleting one fails here.
#[test]
fn abn_format_guards_are_consulted_by_the_result_expression() {
    Source::adapter(ABN).matches(concat!(
        r"(?m)result\s*=\s*",
        r"if not allowedFormatting\s*",
        r"or Text\.Length\(digitsOnly\) <> 11\s*",
        r#"or Text\.Select\(digitsOnly, \{"0"\.\."9"\}\) <> digitsOnly\s*"#,
        r#"or Text\.StartsWith\(digitsOnly, "0"\) then\s*"#,
        r"false",
    ));
}

/// A port of the ATO checksum, pinned to the M's own constants.
fn abn_is_valid(value: &str) -> bool {
    if !value
        .chars()
        .all(|character| character.is_ascii_digit() || character == ' ')
    {
        return false;
    }
    let digits: Vec<u32> = value
        .chars()
        .filter_map(|character| character.to_digit(10))
        .collect();
    if digits.len() != 11 || digits[0] == 0 {
        return false;
    }
    let total: u32 = digits
        .iter()
        .zip(ABN_WEIGHTS)
        .enumerate()
        .map(|(index, (digit, weight))| (digit - u32::from(index == 0)) * weight)
        .sum();
    total.is_multiple_of(89)
}

/// This does not execute the M (there is no Power Query host in CI), so it is a
/// specification test: the vectors document the contract, and the constant assertions
/// fail if the M drifts away from it.
#[test]
fn abn_reference_contract_matches_the_power_query_constants() {
    let source = Source::adapter(ABN);
    let weights: Vec<_> = ABN_WEIGHTS.iter().map(u32::to_string).collect();
    let weights = format!("weights = {{{}}}", weights.join(", "));
    source.has_all(&[
        weights.as_str(),
        "Number.Mod(total, 89) = 0",
        // The weights and the modulus were pinned; the subtract-one step was not. Turning
        // it into "+ 1" keeps both of those assertions passing and rejects the ATO's own
        // published example, so every genuinely valid ABN in a supplier list is flagged
        // invalid.
        "adjusted = {digits{0} - 1} & List.Skip(digits, 1)",
        "adjusted{_} * weights{_}",
    ]);
    for value in ["51824753556", "51 824 753 556", " 51 824 753 556 "] {
        assert!(abn_is_valid(value), "must accept {value:?}");
    }
    for value in [
        "\t51 824 753 556",
        "51 824 753 556\n",
        "51\r824 753 556",
        "51\u{a0}824 753 556",
        "51\u{2007}824 753 556",
        "51\u{202f}824 753 556",
        "51-824-753-556",
        "5182475355A",
        "5182475355",
        "51824753557",
        "00000090000",
    ] {
        assert!(!abn_is_valid(value), "must reject {value:?}");
    }
}

/// Match the implementation, not the topic.
///
/// Forbidding the substring "ABN" everywhere else broke the moment any other query
/// called or even named this helper, which is exactly the composition the README
/// advertises.
#[test]
fn abn_checksum_is_implemented_once() {
    let mut implementations: Vec<_> = fs::read_dir(root().join("powerquery"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        // Any case, as Python's Path.glob("*.pq") matched on Windows.
        .filter(|path| {
            path.extension()
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case("pq"))
        })
        .filter(|path| common::read(path).contains("Number.Mod(total, 89)"))
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    implementations.sort();
    assert_eq!(implementations, ["Fx.ABNIsValid.pq"]);
}

/// M let-bindings are lazy.
///
/// Bound only inside the `Table.SelectRows` predicate, neither guard fires for a table
/// with no rows. `Checked` has to feed `Table.AddIndexColumn`.
#[test]
fn header_promoter_guards_run_before_any_row_is_examined() {
    let source = Source::adapter(PROMOTER);
    source.has_all(&[
        "if List.IsEmpty(ColumnNames) then",
        "Input table has no columns",
        "Table.AddIndexColumn(Checked,",
        // The blank check and the row matcher must use the same normalised value, or a
        // header pasted with a trailing space matches nothing.
        "Wanted = Text.Trim(FirstHeaderValue)",
        "else if Wanted = \"\" then",
        "each Text.Trim(Text.From(Record.Field(_, FirstColumn) ?? \"\")) = Wanted",
    ]);
    source.lacks("= FirstHeaderValue\n");
}

/// Finding the header row is guarded above; consuming the index it found is not, and
/// both halves of that are silent when wrong.
///
/// `List.Max` instead of `List.Min` slices at the LAST row carrying the header value, so
/// an export that repeats it (an account literally named "Account", a repeated section
/// header in a bank export) loses every data row above the repeat with no error.
/// `HeaderIdx + 1` in the `RemoveFirstN` drops the header row itself, so `PromoteHeaders`
/// promotes the first DATA row as the column names: the first account disappears and
/// every downstream column reference breaks.
///
/// `Xero.TrialBalance.pq` runs the same 2 steps over its own match list and is pinned
/// here with the promoter, so the pair cannot be corrected in one file and left wrong in
/// the other.
#[test]
fn header_slice_takes_the_first_match_and_keeps_the_header_row() {
    let promoter = Source::adapter(PROMOTER);
    promoter.has_all(&[
        "List.Min(Matches),",
        "Sliced = Table.RemoveFirstN(Raw, HeaderIdx),",
    ]);
    // The function's own header comment states this contract; pinning it beside the code
    // stops the 2 drifting apart.
    promoter.matches(concat!(
        r"(?m)Finds the first\s*//\s*row whose first cell equals FirstHeaderValue, ",
        r"drops everything above it, and\s*//\s*promotes it to headers\.",
    ));
    Source::adapter("powerquery/Xero.TrialBalance.pq").has_all(&[
        "List.Min(HeaderMatches),",
        "Sliced = Table.RemoveFirstN(Raw, HeaderIdx),",
    ]);
}

/// `Date.From` on a datetimezone returns the date of the value's LOCAL equivalent, so the
/// HOST's zone decided the answer: 9 am +10:00 on 1 July 2026 came out FY2027 on a Sydney
/// desktop and FY2026 on a UTC-hosted scheduled refresh. The offset has to be resolved
/// before the date is taken, and CONVERTED rather than dropped: `RemoveZone` on its own is
/// host-independent too, but it reads the UTC-stamped 2026-06-30T14:30:00Z (the shape the
/// Xero API returns for 12:30 am on 1 July in Sydney) as 30 June, which is the wrong FY on
/// the AU desktop that the old code got right.
#[test]
fn financial_year_switches_a_datetimezone_to_australian_eastern() {
    let source = Source::adapter(FINANCIAL_YEAR);
    source.matches(concat!(
        r"(?m)normalised =\s*",
        r"if Value\.Is\(d, type datetimezone\)\s*",
        r"then DateTimeZone\.RemoveZone\(DateTimeZone\.SwitchZone\(d, 10\)\)\s*",
        r"else d,",
    ));
    source.has("asDate = Date.From(normalised, \"en-AU\"),");
    source.lacks("Date.From(d, \"en-AU\")");
    source.before("normalised =", "asDate =");
    // The header comment has to say which clock the caller gets, and warn the states that
    // AEST is not their own local calendar.
    source.has_all(&[
        "CONVERTED to Australian Eastern Standard Time (+10:00)",
        "Residual:",
        "Adelaide/Darwin",
    ]);
}

/// A wall-clock time, with its UTC offset in minutes when it is zoned.
#[derive(Clone, Copy)]
struct Moment {
    year: i64,
    month: u32,
    day: u32,
    minute_of_day: i64,
    offset_minutes: Option<i64>,
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_from_march = (i64::from(month) + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The year and month of a day count from `days_from_civil`.
fn civil_year_month(days: i64) -> (i64, u32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let month = u32::try_from(if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    })
    .expect("a month from 1 to 12");
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month)
}

/// The `normalised` and `endYear` steps of `Fx.AUFinancialYear`, ported: a zoned value is
/// converted to +10:00 before its date is read.
fn helper_end_year(value: Moment) -> i64 {
    let mut days = days_from_civil(value.year, value.month, value.day);
    if let Some(offset) = value.offset_minutes {
        let minutes = days * 1440 + value.minute_of_day - offset + 600;
        days = minutes.div_euclid(1440);
    }
    let (year, month) = civil_year_month(days);
    year + i64::from(month >= 7)
}

/// Switching zone on its own leaves a datetimezone, which the `normalised` step above
/// converts straight back to +10, so 30 June 23:00 in Perth and 23:45 in Adelaide still
/// come back FY2027. The advice has to end in `RemoveZone`, which yields the plain
/// datetime the function passes through untouched.
#[test]
fn local_calendar_advice_says_to_remove_the_offset_as_well() {
    let perth = Moment {
        year: 2026,
        month: 6,
        day: 30,
        minute_of_day: 23 * 60,
        offset_minutes: Some(8 * 60),
    };
    let adelaide = Moment {
        minute_of_day: 23 * 60 + 45,
        offset_minutes: Some(9 * 60 + 30),
        ..perth
    };
    for switched in [perth, adelaide] {
        // SwitchZone alone: the value is still zoned, so AEST decides.
        assert_eq!(helper_end_year(switched), 2027);
        // SwitchZone then RemoveZone: the state's own calendar decides.
        let removed = Moment {
            offset_minutes: None,
            ..switched
        };
        assert_eq!(helper_end_year(removed), 2026);
    }

    let source = Source::adapter(FINANCIAL_YEAR);
    let readme = Source::adapter("README.md");
    for text in [&source, &readme] {
        text.has_all(&[
            "DateTimeZone.RemoveZone(DateTimeZone.SwitchZone(d, 8))",
            "DateTimeZone.RemoveZone(DateTimeZone.SwitchZone(d, 9, 30))",
        ]);
    }
    // The advice that stopped at SwitchZone cannot come back.
    source.lacks("switch zone yourself before");
    readme.lacks("unless you switch the zone first");
}
