#[cfg(any(test, feature = "probe"))]
use super::files;
use super::{Result, digest, json, pycsv, schema};
use crate::{date::days_in_month, decimal::ContextDecimal};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(any(test, feature = "probe"))]
use std::path::Path;

pub type Money = ContextDecimal<40>;
pub type Snapshot = BTreeMap<String, Vec<u8>>;
pub type Row = BTreeMap<String, String>;
pub const TB_COLUMNS: [&str; 10] = [
    "ReportDate",
    "Tenant",
    "Section",
    "AccountID",
    "AccountName",
    "AccountCode",
    "Debit",
    "Credit",
    "YTDDebit",
    "YTDCredit",
];

#[cfg(any(test, feature = "probe"))]
pub fn sources(app: &Path) -> Result<(Value, Snapshot)> {
    sources_with(|name| files::read(&app.join("samples").join(name)))
}

pub fn bundled_sources() -> Result<(Value, Snapshot)> {
    let bundled: [(&str, &[u8]); 7] = [
        (
            "shared-review-case.json",
            include_bytes!("../../samples/shared-review-case.json"),
        ),
        (
            "sample-ato-benchmarks.csv",
            include_bytes!("../../samples/sample-ato-benchmarks.csv"),
        ),
        ("sample-budgets.csv", include_bytes!("../../samples/sample-budgets.csv")),
        (
            "sample-chart-of-accounts.csv",
            include_bytes!("../../samples/sample-chart-of-accounts.csv"),
        ),
        (
            "sample-entities.csv",
            include_bytes!("../../samples/sample-entities.csv"),
        ),
        (
            "sample-general-ledger.csv",
            include_bytes!("../../samples/sample-general-ledger.csv"),
        ),
        (
            "sample-payroll-super.csv",
            include_bytes!("../../samples/sample-payroll-super.csv"),
        ),
    ];
    sources_with(|name| {
        bundled
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, data)| data.to_vec())
            .ok_or_else(|| "Case source is outside the fixture whitelist.".into())
    })
}

fn sources_with(mut read: impl FnMut(&str) -> Result<Vec<u8>>) -> Result<(Value, Snapshot)> {
    let case = json::parse(&read("shared-review-case.json")?, "Shared case")?;
    schema::case(&case)?;
    let mut source = Snapshot::new();
    for (name, expected) in case["source_sha256"].as_object().unwrap() {
        if !name
            .strip_prefix("sample-")
            .and_then(|name| name.strip_suffix(".csv"))
            .is_some_and(|name| !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-'))
        {
            return Err("Case source is outside the fixture whitelist.".into());
        }
        let data = read(name)?;
        if digest(&data) != expected.as_str().unwrap() {
            return Err(format!("Pinned fabricated source changed: {name}"));
        }
        source.insert(name.clone(), data);
    }
    if source.len() != 6 {
        return Err("Expected the original six fabricated sources.".into());
    }
    Ok((case, source))
}

pub fn rows(data: &[u8]) -> Result<Vec<Row>> {
    let data = data.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(data);
    let text = std::str::from_utf8(data).map_err(|error| error.to_string())?;
    let mut records = pycsv::Reader::new(text);
    let headers = records
        .next()
        .transpose()
        .map_err(|_| "Invalid CSV header.")?
        .unwrap_or_default();
    records
        .filter_map(|record| match record {
            Ok(record) if record.is_empty() => None,
            Ok(record) => Some(Ok(headers.iter().cloned().zip(record).collect())),
            Err(_) => Some(Err("Invalid CSV data.".into())),
        })
        .collect()
}

pub fn csv(columns: &[&str], rows: &[Vec<String>]) -> Vec<u8> {
    let header: Vec<_> = columns.iter().map(|column| (*column).to_owned()).collect();
    std::iter::once(&header)
        .chain(rows)
        .map(|row| {
            row.iter()
                .map(|value| {
                    if value.contains([',', '"', '\n']) || (row.len() == 1 && value.is_empty()) {
                        format!("\"{}\"", value.replace('"', "\"\""))
                    } else {
                        value.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
                + "\n"
        })
        .collect::<String>()
        .into_bytes()
}

pub fn field<'a>(row: &'a Row, name: &str) -> Result<&'a str> {
    row.get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("Missing CSV field: {name}"))
}

pub fn money(text: &str) -> Result<Money> {
    Money::try_parse(text).ok_or_else(|| "Invalid or non-finite decimal amount.".into())
}

fn case_text<'a>(case: &'a Value, name: &str) -> &'a str {
    case[name].as_str().expect("validated case field")
}

pub fn trial_balances(case: &Value, source: &Snapshot, month: u32) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    schema::case(case)?;
    if case["financial_year_start"] != "2024-07-01" || ![8, 9].contains(&month) {
        return Err("This case supports August and September 2024 only.".into());
    }
    let chart = rows(
        source
            .get("sample-chart-of-accounts.csv")
            .ok_or("Missing chart of accounts.")?,
    )?;
    let end = period_end(month);
    let ledger = rows(
        source
            .get("sample-general-ledger.csv")
            .ok_or("Missing general ledger.")?,
    )?
    .into_iter()
    .filter_map(|row| match (field(&row, "EntityID"), field(&row, "PostingDate")) {
        (Ok(entity), Ok(date))
            if entity == case_text(case, "entity")
                && date >= case_text(case, "financial_year_start")
                && date <= end.as_str() =>
        {
            Some(Ok(row))
        }
        (Ok(_), Ok(_)) => None,
        _ => Some(Err("Missing ledger identity or date.".into())),
    })
    .collect::<Result<Vec<_>>>()?;
    if ledger.is_empty()
        || ledger
            .iter()
            .any(|row| field(row, "AccountCode").is_ok_and(|code| case["account_ids"].get(code).is_none()))
    {
        return Err("Ledger account population is not mapped.".into());
    }
    validate_journals(case, &ledger)?;
    let transactions = transactions(case, &ledger)?;
    Ok((
        balance(case, &chart, &ledger, month)?,
        balance(case, &chart, &ledger, month - 1)?,
        transactions,
    ))
}

fn period_end(month: u32) -> String {
    format!("2024-{month:02}-{}", days_in_month(2024, month))
}

fn validate_journals(case: &Value, ledger: &[Row]) -> Result<()> {
    let (mut seen, mut journals) = (BTreeSet::new(), BTreeMap::<&str, Money>::new());
    for row in ledger {
        let (journal, line) = (field(row, "JournalID")?, field(row, "LineNumber")?);
        let (debit, credit) = (money(field(row, "Debit")?)?, money(field(row, "Credit")?)?);
        let zero = Money::zero();
        if !seen.insert((journal, line))
            || debit < zero
            || credit < zero
            || (debit > zero) == (credit > zero)
            || debit.sub(&credit) != money(field(row, "Amount")?)?
        {
            return Err("Invalid or duplicated journal line.".into());
        }
        let total = journals.entry(journal).or_insert_with(Money::zero);
        *total = total.add(&debit).sub(&credit);
    }
    if journals.values().any(|value| !value.is_zero()) || !journals.contains_key(case_text(case, "opening_journal")) {
        return Err("Unbalanced journal or missing explicit opening.".into());
    }
    Ok(())
}

fn total(rows: &[&Row], column: &str) -> Result<Money> {
    rows.iter()
        .try_fold(Money::zero(), |sum, row| Ok(sum.add(&money(field(row, column)?)?)))
}

fn balance(case: &Value, chart: &[Row], ledger: &[Row], month: u32) -> Result<Vec<u8>> {
    let end = period_end(month);
    let mut output = Vec::new();
    let (mut current_net, mut ytd_net) = (Money::zero(), Money::zero());
    for account in chart {
        let code = field(account, "AccountCode")?;
        let mut eligible = Vec::new();
        let mut current = Vec::new();
        for row in ledger {
            let date = field(row, "PostingDate")?;
            if field(row, "AccountCode")? == code
                && date >= case_text(case, "financial_year_start")
                && date <= end.as_str()
            {
                eligible.push(row);
                if date.get(..7) == end.get(..7) {
                    current.push(row);
                }
            }
        }
        let values = [
            total(&current, "Debit")?,
            total(&current, "Credit")?,
            total(&eligible, "Debit")?,
            total(&eligible, "Credit")?,
        ];
        current_net = current_net.add(&money(&values[0].to_fixed(2))?.sub(&money(&values[1].to_fixed(2))?));
        ytd_net = ytd_net.add(&money(&values[2].to_fixed(2))?.sub(&money(&values[3].to_fixed(2))?));
        let identifier = case["account_ids"][code]
            .as_str()
            .ok_or("Ledger account population is not mapped.")?;
        let mut row = vec![
            end.clone(),
            case_text(case, "tenant").into(),
            field(account, "Class")?.into(),
            identifier.into(),
            field(account, "AccountName")?.into(),
            code.into(),
        ];
        row.extend(values.map(|value| value.to_fixed(2)));
        output.push(row);
    }
    if !current_net.is_zero() || !ytd_net.is_zero() {
        return Err("Trial balance does not satisfy the exporter contract.".into());
    }
    Ok(csv(&TB_COLUMNS, &output))
}

fn transactions(case: &Value, ledger: &[Row]) -> Result<Vec<u8>> {
    let columns = [
        "Tenant",
        "AccountID",
        "Currency",
        "TransactionID",
        "Date",
        "Reference",
        "Description",
        "Debit",
        "Credit",
    ];
    let values = ledger
        .iter()
        .map(|row| {
            let id = case["account_ids"][field(row, "AccountCode")?]
                .as_str()
                .ok_or("Ledger account population is not mapped.")?;
            let journal = field(row, "JournalID")?;
            Ok(vec![
                case_text(case, "tenant").into(),
                id.into(),
                case_text(case, "currency").into(),
                format!("{journal}:{}", field(row, "LineNumber")?),
                field(row, "PostingDate")?.into(),
                journal.into(),
                field(row, "Description")?.into(),
                field(row, "Debit")?.into(),
                field(row, "Credit")?.into(),
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(csv(&columns, &values))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cents(text: &str) -> i128 {
        let (sign, text) = text.strip_prefix('-').map_or((1, text), |text| (-1, text));
        let (whole, fraction) = text.split_once('.').unwrap();
        assert_eq!(fraction.len(), 2);
        sign * (whole.parse::<i128>().unwrap() * 100 + fraction.parse::<i128>().unwrap())
    }

    #[test]
    fn current_prior_and_transactions_tie_to_independently_grouped_integer_cents() {
        let (case, source) = bundled_sources().unwrap();
        let (current, prior, transactions) = trial_balances(&case, &source, 9).unwrap();
        let ledger = rows(&source["sample-general-ledger.csv"]).unwrap();
        for (data, period) in [(&current, "2024-09-30"), (&prior, "2024-08-31")] {
            let balances = rows(data).unwrap();
            for balance in &balances {
                let included: Vec<_> = ledger
                    .iter()
                    .filter(|row| {
                        row["EntityID"] == "ENT001"
                            && row["AccountCode"] == balance["AccountCode"]
                            && row["PostingDate"].as_str() >= "2024-07-01"
                            && row["PostingDate"].as_str() <= period
                    })
                    .collect();
                for side in ["Debit", "Credit"] {
                    let ytd: i128 = included.iter().map(|row| cents(&row[side])).sum();
                    let movement: i128 = included
                        .iter()
                        .filter(|row| row["PostingDate"][..7] == period[..7])
                        .map(|row| cents(&row[side]))
                        .sum();
                    assert_eq!(cents(&balance[&format!("YTD{side}")]), ytd);
                    assert_eq!(cents(&balance[side]), movement);
                }
            }
            assert_eq!(
                balances
                    .iter()
                    .map(|row| cents(&row["Debit"]) - cents(&row["Credit"]))
                    .sum::<i128>(),
                0
            );
        }
        let expected: BTreeSet<_> = ledger
            .iter()
            .filter(|row| {
                row["EntityID"] == "ENT001"
                    && row["PostingDate"].as_str() >= "2024-07-01"
                    && row["PostingDate"].as_str() <= "2024-09-30"
            })
            .map(|row| format!("{}:{}", row["JournalID"], row["LineNumber"]))
            .collect();
        let actual: BTreeSet<_> = rows(&transactions)
            .unwrap()
            .into_iter()
            .map(|row| row["TransactionID"].clone())
            .collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn exact_opening_bridge_preserves_both_sides_of_a_balanced_reversal() {
        let (case, mut source) = bundled_sources().unwrap();
        let (current, prior, _) = trial_balances(&case, &source, 9).unwrap();
        let (current, prior) = (rows(&current).unwrap(), rows(&prior).unwrap());
        for row in &current {
            let before = prior
                .iter()
                .find(|before| before["AccountID"] == row["AccountID"])
                .unwrap();
            assert_eq!(
                cents(&before["YTDDebit"]) - cents(&before["YTDCredit"]) + cents(&row["Debit"]) - cents(&row["Credit"]),
                cents(&row["YTDDebit"]) - cents(&row["YTDCredit"])
            );
        }
        let mut original = rows(&source["sample-general-ledger.csv"]).unwrap();
        let prototype = original.iter().find(|row| row["EntityID"] == "ENT001").unwrap().clone();
        for (line, debit, credit, amount) in [("1", "35.00", "0.00", "35.00"), ("2", "0.00", "35.00", "-35.00")] {
            let mut row = prototype.clone();
            for (key, value) in [
                ("JournalID", "TEST-REVERSAL"),
                ("LineNumber", line),
                ("PostingDate", "2024-09-30"),
                ("Debit", debit),
                ("Credit", credit),
                ("Amount", amount),
            ] {
                row.insert(key.into(), value.into());
            }
            original.push(row);
        }
        let columns: Vec<_> = original[0].keys().map(String::as_str).collect();
        let data = csv(
            &columns,
            &original
                .iter()
                .map(|row| columns.iter().map(|key| row[*key].clone()).collect())
                .collect::<Vec<_>>(),
        );
        source.insert("sample-general-ledger.csv".into(), data);
        let altered = rows(&trial_balances(&case, &source, 9).unwrap().0).unwrap();
        let before = current
            .iter()
            .find(|row| row["AccountCode"] == prototype["AccountCode"])
            .unwrap();
        let after = altered
            .iter()
            .find(|row| row["AccountCode"] == prototype["AccountCode"])
            .unwrap();
        for side in ["Debit", "Credit"] {
            assert_eq!(cents(&after[side]) - cents(&before[side]), 3500);
        }
    }

    #[test]
    fn pinned_ledgers_have_separate_current_and_ytd_amounts() {
        let (case, source) = sources(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        assert_eq!(bundled_sources().unwrap(), (case.clone(), source.clone()));
        for (month, expected) in [
            (
                8,
                [
                    "56dcadac908ee78c3cb87e6b379298d40039ffe1f089e4d2116a2dc06d159360",
                    "e83238353b0cfc473b11e351100419d4973616315d3cc886bb08b83372d926b9",
                    "16fc0ef14e45fa74a2862b9c5e8b0d9d69b419d9192b419f8fa9ca7bc1f7fbc0",
                ],
            ),
            (
                9,
                [
                    "71cb6b6d10efed15b36f42498bd5d6e788e24d93da02918d823bbf664fd7ab45",
                    "56dcadac908ee78c3cb87e6b379298d40039ffe1f089e4d2116a2dc06d159360",
                    "6e60fab31c004f4252792ed455b3c9c7e4e41e13c737e920998651dbb5b5cf39",
                ],
            ),
        ] {
            let (current, prior, transactions) = trial_balances(&case, &source, month).unwrap();
            assert_eq!([digest(&current), digest(&prior), digest(&transactions)], expected);
            assert_eq!(rows(&current).unwrap().len(), 30);
            assert!(!prior.is_empty());
            assert!(!rows(&transactions).unwrap().is_empty());
            assert_ne!(current, prior);
        }
        assert!(trial_balances(&case, &source, 7).is_err());
    }

    #[test]
    fn journal_validation_refuses_duplication_invalid_money_and_missing_opening() {
        let (case, source) = sources(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
        let original = std::str::from_utf8(&source["sample-general-ledger.csv"]).unwrap();
        let duplicate = format!("{original}{}\n", original.lines().nth(1).unwrap());
        for changed in [
            duplicate,
            original.replacen("350000.00,0.00,350000.00", "350000.00,0.00,1.00", 1),
            original.replacen("350000.00,0.00,350000.00", "NaN,0.00,NaN", 1),
        ] {
            assert_ne!(changed, original);
            let mut source = source.clone();
            source.insert("sample-general-ledger.csv".into(), changed.into_bytes());
            assert!(trial_balances(&case, &source, 9).is_err());
        }
        let mut changed = case;
        changed["opening_journal"] = "absent".into();
        assert!(trial_balances(&changed, &source, 9).is_err());
    }

    #[test]
    fn workflow_csv_uses_python_lf_quoting() {
        let data = csv(
            &["first", "second"],
            &[vec!["line\rreturn".into(), "comma,value".into()]],
        );
        assert_eq!(data, b"first,second\nline\rreturn,\"comma,value\"\n");
    }
}
