// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! PAYG withholding and super guarantee reperformed on a payroll register's
//! pay lines, for pay dates in the 2026-27 income year only.
use crate::{
    Result,
    decimal::{self, Decimal, Rounding},
    isodate::{self, Date},
    loader::{self, SourceSnapshot},
    pyjson::{Json, PyStr},
    pytext, reconciliation,
};
use std::{collections::BTreeMap, path::Path};

pub const COLUMNS: [&str; 11] = [
    "Tenant",
    "EmployeeID",
    "PayDate",
    "Frequency",
    "Scale",
    "Adjusted",
    "Earnings",
    "Withheld",
    "QualifyingEarnings",
    "Super",
    "FundReceived",
];
pub const BOUNDARY: &str = "Review evidence only. A listed line is worth a reviewer's look; it does not show that withholding or super is wrong, and a register with nothing listed is not shown to comply. Withholding is reperformed by the ATO's Schedule 1 formulas before tax offsets and Medicare levy adjustments; super guarantee by its percentage of qualifying earnings, capped by the maximum contribution base over the lines supplied.";
/// The ATO sources the rates below were read from on 24 November 2026.
pub const SOURCES: [&str; 2] = [
    "ATO, Schedule 1 - Statement of formulas for calculating amounts to be withheld, published 17 June 2026, for payments made from 1 July 2026",
    "ATO, Super guarantee: 12.00% for 1 July 2026 to 30 June 2027 and a maximum contribution base of $270,830 for 2026-27",
];
/// The first and last pay dates the rates above cover.
const FIRST: (i32, u32, u32) = (2026, 7, 1);
const LAST: (i32, u32, u32) = (2027, 6, 30);
/// A recorded withholding more than this from the formula's is listed.
const PAYG_TOLERANCE: &str = "1.00";
const SG_PERCENT: &str = "0.12";
const MAXIMUM_BASE: &str = "270830";
/// Calendar days after the pay date within which a fund's receipt is always
/// inside the 7 business days the ATO allows.
const RECEIPT_DAYS: i32 = 7;

/// Each scale's weekly brackets: earnings `x` below the bound take `a` and `b`
/// in `y = a * x - b`; `None` withholds nothing. The last bracket has no bound.
type Bracket = (Option<&'static str>, Option<(&'static str, &'static str)>);
const SCALE_1: &[Bracket] = &[
    (Some("188"), Some(("0.1500", "0.1500"))),
    (Some("371"), Some(("0.2084", "11.0185"))),
    (Some("515"), Some(("0.1790", "0.1066"))),
    (Some("932"), Some(("0.3227", "74.1674"))),
    (Some("2246"), Some(("0.3200", "71.6508"))),
    (Some("3303"), Some(("0.3900", "228.8816"))),
    (None, Some(("0.4700", "493.1893"))),
];
const SCALE_2: &[Bracket] = &[
    (Some("362"), None),
    (Some("538"), Some(("0.1500", "54.3462"))),
    (Some("673"), Some(("0.2500", "108.2135"))),
    (Some("721"), Some(("0.1700", "54.3473"))),
    (Some("865"), Some(("0.1790", "60.8377"))),
    (Some("1282"), Some(("0.3227", "185.1935"))),
    (Some("2596"), Some(("0.3200", "181.7319"))),
    (Some("3653"), Some(("0.3900", "363.4627"))),
    (None, Some(("0.4700", "655.7704"))),
];
const SCALE_3: &[Bracket] = &[
    (Some("2596"), Some(("0.3000", "0.3000"))),
    (Some("3653"), Some(("0.3700", "181.7308"))),
    (None, Some(("0.4500", "474.0385"))),
];
const SCALE_5: &[Bracket] = &[
    (Some("362"), None),
    (Some("721"), Some(("0.1500", "54.3462"))),
    (Some("865"), Some(("0.1590", "60.8365"))),
    (Some("1282"), Some(("0.3027", "185.1923"))),
    (Some("2596"), Some(("0.3000", "181.7308"))),
    (Some("3653"), Some(("0.3700", "363.4615"))),
    (None, Some(("0.4500", "655.7692"))),
];
const SCALE_6: &[Bracket] = &[
    (Some("362"), None),
    (Some("721"), Some(("0.1500", "54.3462"))),
    (Some("865"), Some(("0.1590", "60.8365"))),
    (Some("908"), Some(("0.3027", "185.1923"))),
    (Some("1135"), Some(("0.3527", "230.6135"))),
    (Some("1282"), Some(("0.3127", "185.1923"))),
    (Some("2596"), Some(("0.3100", "181.7308"))),
    (Some("3653"), Some(("0.3800", "363.4615"))),
    (None, Some(("0.4600", "655.7692"))),
];

fn d(text: &str) -> Decimal {
    Decimal::parse(text).expect("literal decimal")
}

/// Whole dollars, cents ignored, of a non-negative amount.
fn whole(value: &Decimal) -> Decimal {
    let text = value.to_fixed_string().expect("bounded payroll amount");
    d(text.split('.').next().expect("split yields a part"))
}

fn nearest_dollar(value: &Decimal) -> Decimal {
    value
        .quantize(0, Rounding::HalfUp)
        .expect("bounded payroll amount")
}

/// The weekly amount for weekly earnings `x` on `scale`, rounded to the dollar.
fn weekly(scale: &[Bracket], x: &Decimal) -> Decimal {
    let (_, coefficients) = scale
        .iter()
        .find(|(bound, _)| bound.is_none_or(|bound| *x < d(bound)))
        .expect("the last bracket has no bound");
    let Some((a, b)) = coefficients else {
        return Decimal::zero();
    };
    let y = nearest_dollar(&d(a).mul(x).sub(&d(b)).value);
    if y > Decimal::zero() {
        y
    } else {
        Decimal::zero()
    }
}

/// The Schedule 1 withholding for one payment of `earnings`, before tax
/// offsets and Medicare levy adjustments.
fn withholding(frequency: &str, scale: &str, earnings: &Decimal) -> Decimal {
    let no_tfn = match scale {
        "4R" => Some("0.47"),
        "4F" => Some("0.45"),
        _ => None,
    };
    if let Some(rate) = no_tfn {
        return whole(&d(rate).mul(&whole(earnings)));
    }
    let brackets = match scale {
        "1" => SCALE_1,
        "2" => SCALE_2,
        "3" => SCALE_3,
        "5" => SCALE_5,
        _ => SCALE_6,
    };
    let cents = d("0.99");
    let monthly = || {
        let fraction = earnings.sub(&whole(earnings)).value;
        let earnings = if fraction == d("0.33") {
            earnings.add(&d("0.01")).value
        } else {
            earnings.clone()
        };
        earnings.mul(&d("3")).div(&d("13"))
    };
    let weekly_equivalent = match frequency {
        "WEEKLY" => earnings.clone(),
        "FORTNIGHTLY" => earnings.div(&d("2")),
        "MONTHLY" => monthly(),
        _ => earnings.div(&d("13")),
    };
    let y = weekly(brackets, &whole(&weekly_equivalent).add(&cents).value);
    match frequency {
        "WEEKLY" => y,
        "FORTNIGHTLY" => y.mul(&d("2")),
        "MONTHLY" => nearest_dollar(&y.mul(&d("13")).div(&d("3"))),
        _ => y.mul(&d("13")),
    }
}

struct Line {
    row: usize,
    employee: String,
    paid: Date,
    frequency: String,
    scale: String,
    adjusted: bool,
    earnings: Decimal,
    withheld: Decimal,
    qualifying: Decimal,
    paid_super: Decimal,
    received: Option<Date>,
}

fn load(source: &SourceSnapshot) -> Result<(String, Vec<Line>)> {
    let path = crate::pypath::text(&source.path);
    let mut tenant = None;
    let mut lines = Vec::new();
    for row in loader::csv_rows(source, &COLUMNS, "Payroll register")? {
        let row = row?;
        let name = row.text("Tenant", false)?;
        if *tenant.get_or_insert_with(|| name.clone()) != name {
            return Err(
                "A payroll register must belong to one tenant; review each tenant separately."
                    .into(),
            );
        }
        lines.push(line(&row, &path, source)?);
    }
    if lines.is_empty() {
        return Err(format!("{path}: no pay lines were supplied."));
    }
    Ok((tenant.unwrap_or_default(), lines))
}

/// A non-negative amount from the row.
fn amount(
    row: &loader::CsvRow,
    field: &str,
    path: &str,
    source: &SourceSnapshot,
) -> Result<Decimal> {
    let amount = reconciliation::money(&row.text(field, false)?, field, &source.path, row.number)?;
    if amount.cmp_value(&Decimal::zero()).is_lt() {
        return Err(format!(
            "{path}: row {} {field} must not be negative.",
            row.number
        ));
    }
    Ok(amount)
}

fn line(row: &loader::CsvRow, path: &str, source: &SourceSnapshot) -> Result<Line> {
    let first = Date::new(FIRST.0, FIRST.1, FIRST.2).expect("valid date");
    let last = Date::new(LAST.0, LAST.1, LAST.2).expect("valid date");
    let text = |field| row.text(field, field == "FundReceived");
    let refuse = |what: String| Err(format!("{path}: row {} {what}", row.number));
    let paid = row.date("PayDate")?;
    if paid < first || paid > last {
        return refuse(format!(
            "is paid on {}, outside 2026-27, the only income year whose withholding formulas and super guarantee rate were read from the ATO; review it by hand.",
            paid.iso()
        ));
    }
    let frequency = text("Frequency")?;
    if !["WEEKLY", "FORTNIGHTLY", "MONTHLY", "QUARTERLY"].contains(&frequency.as_str()) {
        return refuse("Frequency must be WEEKLY, FORTNIGHTLY, MONTHLY or QUARTERLY.".into());
    }
    let scale = text("Scale")?;
    if !["1", "2", "3", "4R", "4F", "5", "6"].contains(&scale.as_str()) {
        return refuse("Scale must be 1, 2, 3, 4R, 4F, 5 or 6.".into());
    }
    let adjusted = text("Adjusted")?;
    if !["Y", "N"].contains(&adjusted.as_str()) {
        return refuse("Adjusted must be Y or N.".into());
    }
    let money = |field| amount(row, field, path, source);
    let received = text("FundReceived")?;
    Ok(Line {
        row: row.number,
        employee: text("EmployeeID")?,
        paid,
        frequency,
        scale,
        adjusted: adjusted == "Y",
        earnings: money("Earnings")?,
        withheld: money("Withheld")?,
        qualifying: money("QualifyingEarnings")?,
        paid_super: money("Super")?,
        received: if received.is_empty() {
            None
        } else {
            Some(row.date("FundReceived")?)
        },
    })
}

fn finding(code: &str, line: &Line, figures: [(&str, &Decimal); 3], reason: String) -> Json {
    let mut fields = vec![
        ("code", Json::str(code)),
        ("row", Json::int(line.row as i64)),
        ("employee_id", Json::str(&line.employee)),
        ("pay_date", Json::str(&line.paid.iso())),
    ];
    fields.extend(
        figures
            .into_iter()
            .map(|(key, value)| (key, Json::str(&reconciliation::fixed(value)))),
    );
    fields.push(("reason", Json::str(&reason)));
    Json::Object(
        fields
            .into_iter()
            .map(|(key, value)| (PyStr::from(key), value))
            .collect(),
    )
}

/// The line's Schedule 1 withholding against the amount recorded.
fn payg_finding(line: &Line) -> Option<Json> {
    if line.adjusted {
        return None;
    }
    let expected = withholding(&line.frequency, &line.scale, &line.earnings);
    let difference = line.withheld.sub(&expected).value;
    (difference.abs() > d(PAYG_TOLERANCE)).then(|| {
        finding(
            "PAYG_DIFFERENCE",
            line,
            [
                ("expected", &expected),
                ("recorded", &line.withheld),
                ("difference", &difference),
            ],
            format!(
                "Withheld {} differs from the Schedule 1 formula's {} for {} scale {} earnings of {} by more than ${PAYG_TOLERANCE}.",
                reconciliation::fixed(&line.withheld),
                reconciliation::fixed(&expected),
                pytext::casefold(&line.frequency),
                line.scale,
                reconciliation::fixed(&line.earnings)
            ),
        )
    })
}

/// The line's super guarantee against the amount recorded, and when the fund
/// received it. `counted` is the line's qualifying earnings under the
/// maximum contribution base.
fn super_findings(line: &Line, counted: &Decimal) -> Vec<Json> {
    let required = counted
        .mul(&d(SG_PERCENT))
        .quantize(-2, Rounding::HalfUp)
        .expect("bounded payroll amount");
    let mut findings = Vec::new();
    let shortfall = required.sub(&line.paid_super).value;
    if shortfall > Decimal::zero() {
        findings.push(finding(
            "SG_SHORTFALL",
            line,
            [
                ("expected", &required),
                ("recorded", &line.paid_super),
                ("difference", &shortfall),
            ],
            format!(
                "Super {} is less than 12% of qualifying earnings of {} counted under the maximum contribution base ({}).",
                reconciliation::fixed(&line.paid_super),
                reconciliation::fixed(counted),
                reconciliation::fixed(&required)
            ),
        ));
    }
    if required.is_zero() {
        return findings;
    }
    let late =
        |received: Date| isodate::ordinal(received) - isodate::ordinal(line.paid) > RECEIPT_DAYS;
    let zero = Decimal::zero();
    match line.received {
        None => findings.push(finding(
            "SG_NOT_RECEIVED",
            line,
            [("expected", &required), ("recorded", &line.paid_super), ("difference", &zero)],
            "No fund receipt date is recorded for this pay line's super guarantee.".into(),
        )),
        Some(received) if late(received) => findings.push(finding(
            "SG_TIMING_REVIEW",
            line,
            [("expected", &required), ("recorded", &line.paid_super), ("difference", &zero)],
            format!(
                "The fund received the contribution on {}, more than {RECEIPT_DAYS} calendar days after the pay date. Whether that is within the 7 business days, or 20 for a new employee or fund, depends on public holidays and the employee's history, which this command does not read.",
                received.iso()
            ),
        )),
        Some(_) => {}
    }
    findings
}

pub fn payroll_reperformance(path: &Path) -> Result<Json> {
    let source = SourceSnapshot::capture(path, "Payroll register")?;
    decimal::with_precision(40, || review(&source))
}

fn review(source: &SourceSnapshot) -> Result<Json> {
    let (tenant, mut lines) = load(source)?;
    lines.sort_by_key(|line| (line.paid, line.row));
    let base = d(MAXIMUM_BASE);
    let mut counted_so_far: BTreeMap<&str, Decimal> = BTreeMap::new();
    let mut findings = Vec::new();
    for line in &lines {
        findings.extend(payg_finding(line));
        let so_far = counted_so_far
            .entry(line.employee.as_str())
            .or_insert_with(Decimal::zero);
        let left = base.sub(so_far).value;
        let counted = if line.qualifying > left {
            left
        } else {
            line.qualifying.clone()
        };
        *so_far = so_far.add(&counted).value;
        findings.extend(super_findings(line, &counted));
    }
    let adjusted = lines.iter().filter(|line| line.adjusted).count();
    Ok(Json::object([
        ("schema_version", Json::int(1)),
        (
            "status",
            Json::str(if findings.is_empty() {
                "PASS"
            } else {
                "REVIEW"
            }),
        ),
        ("tenant", Json::str(&tenant)),
        ("boundary", Json::str(BOUNDARY)),
        (
            "sources",
            Json::Array(SOURCES.iter().map(|s| Json::str(s)).collect()),
        ),
        (
            "source_sha256",
            Json::object([("register", Json::str(&source.sha256))]),
        ),
        ("line_count", Json::int(lines.len() as i64)),
        ("payg_not_reperformed", Json::int(adjusted as i64)),
        ("findings", Json::Array(findings)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "Tenant,EmployeeID,PayDate,Frequency,Scale,Adjusted,Earnings,Withheld,QualifyingEarnings,Super,FundReceived\n";

    fn run_text(text: &str) -> Result<Json> {
        let source = SourceSnapshot::new(Path::new("payroll.csv"), text.as_bytes().to_vec());
        decimal::with_precision(40, || review(&source))
    }

    /// `(employee, pay date, frequency, scale, earnings, withheld, super, received)`
    /// lines with qualifying earnings equal to earnings and no adjustment.
    type Row<'a> = (
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
    );
    fn register(rows: &[Row]) -> String {
        let mut text = HEADER.to_owned();
        for (employee, paid, frequency, scale, earnings, withheld, paid_super, received) in rows {
            text.push_str(&format!(
                "Fabricated Payroll,{employee},{paid},{frequency},{scale},N,{earnings},{withheld},{earnings},{paid_super},{received}\n"
            ));
        }
        text
    }

    fn codes(result: &Json) -> Vec<(String, String)> {
        result
            .at("findings")
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let text = |key| f.at(key).as_pystr().unwrap().to_utf8().unwrap();
                (text("code"), text("employee_id"))
            })
            .collect()
    }

    #[test]
    fn the_ato_worked_examples_reperform_before_offsets_and_levy_adjustments() {
        // Schedule 1 'Examples of calculating the withholding amounts', read 24 November 2026.
        for (frequency, scale, earnings, expected) in [
            ("WEEKLY", "2", "1333.45", "245"),
            ("FORTNIGHTLY", "5", "1299.30", "86"),
            ("MONTHLY", "2", "5400.33", "940"),
        ] {
            assert_eq!(
                withholding(frequency, scale, &d(earnings)),
                d(expected),
                "{frequency} {scale} {earnings}"
            );
        }
    }

    #[test]
    fn each_scale_frequency_and_bracket_edge_follows_the_formula() {
        for (frequency, scale, earnings, expected) in [
            // Below the threshold bracket of scale 2: nothing withheld.
            ("WEEKLY", "2", "361.99", "0"),
            // 0.15 * 362.99 - 54.3462 = 0.1023, rounded to 0.
            ("WEEKLY", "2", "362.00", "0"),
            // Scale 1: 0.15 * 0.99 - 0.15 is below nil.
            ("WEEKLY", "1", "0.00", "0"),
            // 0.47 * 3303.99 - 493.1893 = 1059.6860.
            ("WEEKLY", "1", "3303.00", "1060"),
            // Foreign resident: 0.30 * 1000.99 - 0.30 = 300.0.
            ("WEEKLY", "3", "1000.00", "300"),
            // Scale 6 bracket below 1,135: 0.3527 * 1000.99 - 230.6135 = 122.4357.
            ("WEEKLY", "6", "1000.00", "122"),
            // Quarterly: 13,000 / 13 = 1,000; scale 2 0.3227 * 1000.99 - 185.1935 = 137.83, so 138 * 13.
            ("QUARTERLY", "2", "13000.00", "1794"),
            // No TFN: 47% of whole dollars, cents ignored.
            ("FORTNIGHTLY", "4R", "1000.99", "470"),
            ("WEEKLY", "4F", "101.00", "45"),
        ] {
            assert_eq!(
                withholding(frequency, scale, &d(earnings)),
                d(expected),
                "{frequency} {scale} {earnings}"
            );
        }
    }

    #[test]
    fn a_register_that_agrees_passes() {
        let result = run_text(&register(&[(
            "E1",
            "2026-07-09",
            "WEEKLY",
            "2",
            "1333.45",
            "245.00",
            "160.01",
            "2026-07-16",
        )]))
        .unwrap();
        assert!(result.at("status").is_str("PASS"), "{:?}", codes(&result));
        assert_eq!(result.at("line_count").as_plain_int(), Some(&1.into()));
    }

    #[test]
    fn differences_shortfalls_and_receipts_are_listed() {
        let result = run_text(&register(&[
            // Withheld $2 under the formula; super a cent short; received on day 8.
            (
                "E1",
                "2026-07-09",
                "WEEKLY",
                "2",
                "1333.45",
                "243.00",
                "160.00",
                "2026-07-17",
            ),
            // Within $1 of the formula, and no receipt yet.
            (
                "E2",
                "2026-07-09",
                "WEEKLY",
                "2",
                "1333.45",
                "246.00",
                "160.01",
                "",
            ),
        ]))
        .unwrap();
        assert!(result.at("status").is_str("REVIEW"));
        let got = codes(&result);
        let pairs: Vec<(&str, &str)> = got.iter().map(|(c, e)| (c.as_str(), e.as_str())).collect();
        assert_eq!(
            pairs,
            [
                ("PAYG_DIFFERENCE", "E1"),
                ("SG_SHORTFALL", "E1"),
                ("SG_TIMING_REVIEW", "E1"),
                ("SG_NOT_RECEIVED", "E2"),
            ]
        );
        let first = &result.at("findings").as_array().unwrap()[0];
        assert!(first.at("expected").is_str("245.00"));
        assert!(first.at("difference").is_str("-2.00"));
    }

    #[test]
    fn adjusted_lines_skip_withholding_but_not_super() {
        let text = HEADER.to_owned()
            + "Fabricated Payroll,E1,2026-07-09,WEEKLY,2,Y,1333.45,219.00,1333.45,100.00,2026-07-10\n";
        let result = run_text(&text).unwrap();
        assert_eq!(codes(&result), [("SG_SHORTFALL".into(), "E1".into())]);
        assert_eq!(
            result.at("payg_not_reperformed").as_plain_int(),
            Some(&1.into())
        );
    }

    #[test]
    fn the_maximum_contribution_base_caps_the_year_in_date_order() {
        // 270,830 of qualifying earnings in 2 lines; the later one only partly counts.
        let text = HEADER.to_owned()
            + "Fabricated Payroll,E1,2027-06-30,QUARTERLY,4R,N,100000.00,47000.00,100000.00,3699.60,2027-06-30\n"
            + "Fabricated Payroll,E1,2026-07-01,QUARTERLY,4R,N,240000.00,112800.00,240000.00,28800.00,2026-07-02\n";
        let result = run_text(&text).unwrap();
        // Counted on the June line: 270,830 - 240,000 = 30,830; 12% is 3,699.60.
        assert!(result.at("status").is_str("PASS"), "{:?}", codes(&result));
        let short = text.replace("3699.60", "3699.59");
        assert_eq!(
            codes(&run_text(&short).unwrap()),
            [("SG_SHORTFALL".into(), "E1".into())]
        );
    }

    #[test]
    fn nil_super_needs_no_receipt() {
        let result = run_text(&register(&[(
            "E1",
            "2026-07-09",
            "WEEKLY",
            "2",
            "0.00",
            "0.00",
            "0.00",
            "",
        )]))
        .unwrap();
        assert!(result.at("status").is_str("PASS"), "{:?}", codes(&result));
    }

    #[test]
    fn malformed_registers_are_refused() {
        let line = |edit: &dyn Fn(String) -> String| {
            HEADER.to_owned()
                + &edit("Fabricated Payroll,E1,2026-07-09,WEEKLY,2,N,100.00,0.00,100.00,12.00,2026-07-09\n".into())
        };
        for (text, error) in [
            (HEADER.to_owned(), "payroll.csv: no pay lines were supplied.".to_owned()),
            (
                line(&|l| l.replace("2026-07-09,WEEKLY", "2026-06-30,WEEKLY")),
                "payroll.csv: row 2 is paid on 2026-06-30, outside 2026-27, the only income year whose withholding formulas and super guarantee rate were read from the ATO; review it by hand.".into(),
            ),
            (
                line(&|l| l.replace("2026-07-09,WEEKLY", "2027-07-01,WEEKLY")),
                "payroll.csv: row 2 is paid on 2027-07-01, outside 2026-27, the only income year whose withholding formulas and super guarantee rate were read from the ATO; review it by hand.".into(),
            ),
            (line(&|l| l.replace("WEEKLY", "DAILY")), "payroll.csv: row 2 Frequency must be WEEKLY, FORTNIGHTLY, MONTHLY or QUARTERLY.".into()),
            (line(&|l| l.replace(",2,N,", ",4,N,")), "payroll.csv: row 2 Scale must be 1, 2, 3, 4R, 4F, 5 or 6.".into()),
            (line(&|l| l.replace(",2,N,", ",2,maybe,")), "payroll.csv: row 2 Adjusted must be Y or N.".into()),
            (line(&|l| l.replace(",12.00,", ",-12.00,")), "payroll.csv: row 2 Super must not be negative.".into()),
            (
                line(&|l| l.clone() + &l.replace("Fabricated Payroll", "Other Payroll")),
                "A payroll register must belong to one tenant; review each tenant separately.".into(),
            ),
        ] {
            assert_eq!(run_text(&text).unwrap_err(), error);
        }
    }
}
