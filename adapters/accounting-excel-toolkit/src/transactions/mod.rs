mod amount;
pub mod cli;
pub(crate) mod csv;
pub(crate) mod date;
pub mod text;

use amount::Amount;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub use amount::parse as amount;
pub use date::parse as parse_date;

pub const HEADER: &[&str] = &[
    "Date",
    "Source",
    "Description",
    "Reference",
    "Debit",
    "Credit",
    "Running Balance",
    "Gross",
    "GST",
];
pub const OUTPUT: &[&str] = &[
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

pub fn records(content: &[u8]) -> Result<Vec<Vec<String>>, String> {
    let content = content.strip_prefix(b"\xef\xbb\xbf").unwrap_or(content);
    let text = std::str::from_utf8(content).map_err(|e| e.to_string())?;
    csv::Reader::new(text)
        .map(|record| record.map_err(|e| e.0))
        .collect()
}

fn metadata(rows: &[Vec<String>], tenant: &str) -> Result<(usize, date::Date, date::Date), String> {
    let headers: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.as_slice() == HEADER)
        .map(|(i, _)| i)
        .collect();
    if headers.len() != 1 {
        return Err("Expected exactly one observed 9-column Account Transactions header.".into());
    }
    let header = headers[0];
    let metadata: Vec<_> = rows[..header]
        .iter()
        .filter_map(|row| row.first())
        .filter(|s| !s.is_empty())
        .map(String::as_str)
        .collect();
    if !metadata.contains(&"Account Transactions") || !metadata.contains(&tenant) {
        return Err("Report title or tenant does not match supplied identity.".into());
    }
    let periods: Vec<_> = metadata
        .iter()
        .filter_map(|s| s.strip_prefix("For the period "))
        .collect();
    if periods.len() != 1 {
        return Err("Expected one explicit report period.".into());
    }
    let invalid = || "Expected 'For the period d Month yyyy to d Month yyyy'.".to_owned();
    let (left, right) = periods[0].split_once(" to ").ok_or_else(invalid)?;
    let start = date::parse(left, true).ok_or_else(invalid)?;
    let end = date::parse(right, true).ok_or_else(invalid)?;
    if start > end {
        return Err("Report period is reversed.".into());
    }
    Ok((header, start, end))
}

struct Report {
    selected: bool,
    complete: bool,
    sections: usize,
    detail: Vec<Value>,
    debit: Amount,
    credit: Amount,
}

impl Report {
    fn row(
        &mut self,
        row: &[String],
        index: usize,
        account: &str,
        start: date::Date,
        end: date::Date,
    ) -> Result<(), String> {
        if row.iter().all(String::is_empty) {
            return Ok(());
        }
        if row.len() != HEADER.len() {
            return Err(format!("Row {index}: expected exactly 9 fields."));
        }
        if row[0] == account && row[1..].iter().all(String::is_empty) {
            self.selected = true;
            self.sections += 1;
            return Ok(());
        }
        if !self.selected {
            return Ok(());
        }
        if row[0] == "Opening Balance" {
            return if self.detail.is_empty() {
                Ok(())
            } else {
                Err("Opening balance appeared after transaction detail.".into())
            };
        }
        if row[0] == format!("Total {account}") {
            if amount(&row[4])? != self.debit || amount(&row[5])? != self.credit {
                return Err("Account subtotal does not equal independently summed detail.".into());
            }
            self.selected = false;
            self.complete = true;
            return Ok(());
        }
        self.transaction(row, index, start, end)
    }

    fn transaction(
        &mut self,
        row: &[String],
        index: usize,
        start: date::Date,
        end: date::Date,
    ) -> Result<(), String> {
        let posted = date::parse(&row[0], false)
            .ok_or_else(|| format!("Row {index}: unexpected row in selected account section."))?;
        if posted < start || posted > end {
            return Err(format!(
                "Row {index}: posting date is outside report period."
            ));
        }
        let (dr, cr) = (amount(&row[4])?, amount(&row[5])?);
        if dr.positive() == cr.positive() {
            return Err(format!(
                "Row {index}: exactly one debit or credit must be positive."
            ));
        }
        self.debit = self.debit.add(dr)?;
        self.credit = self.credit.add(cr)?;
        self.detail.push(json!({"source_row": index, "Date": posted.iso(), "Source": row[1], "Reference": row[3], "Description": row[2], "Debit": dr.to_string(), "Credit": cr.to_string()}));
        Ok(())
    }
}

pub fn inspect_export(content: &[u8], tenant: &str, account_name: &str) -> Result<Value, String> {
    let rows = records(content)?;
    let (header, start, end) = metadata(&rows, tenant)?;
    let mut report = Report {
        selected: false,
        complete: false,
        sections: 0,
        detail: Vec::new(),
        debit: Amount::ZERO,
        credit: Amount::ZERO,
    };
    for (number, row) in rows[header + 1..].iter().enumerate() {
        report.row(row, header + number + 2, account_name, start, end)?;
    }
    if report.sections != 1 || !report.complete || report.selected {
        return Err("Expected exactly one complete selected account section.".into());
    }
    Ok(
        json!({"source_sha256": Sha256::digest(content).iter().map(|byte| format!("{byte:02x}")).collect::<String>(), "tenant": tenant, "account_name": account_name, "period_start": start.iso(), "period_end": end.iso(), "debit": report.debit.to_string(), "credit": report.credit.to_string(), "detail": report.detail}),
    )
}

fn mappings<'a>(
    mapping: &'a Value,
    report: &Value,
) -> Result<&'a serde_json::Map<String, Value>, String> {
    if !mapping.as_object().is_some_and(|object| {
        object.len() == 2 && object.contains_key("source_sha256") && object.contains_key("lines")
    }) || mapping["source_sha256"] != report["source_sha256"]
    {
        return Err("Line mapping must be bound to this exact source SHA-256.".into());
    }
    let detail = report["detail"].as_array().expect("inspection detail");
    let lines = mapping["lines"]
        .as_object()
        .filter(|lines| {
            lines.len() == detail.len()
                && detail
                    .iter()
                    .all(|row| lines.contains_key(&row["source_row"].to_string()))
        })
        .ok_or("Line mapping must cover every selected detail row exactly.")?;
    let ids: Vec<_> = lines
        .values()
        .map(|value| {
            value.as_str().filter(|s| {
                !s.is_empty()
                    && s.len() <= 80
                    && s.as_bytes()[0].is_ascii_alphanumeric()
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or("Mapping requires stable source-line IDs in the clearing schema.")?;
    if ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err("Mapped source-line IDs must be unique.".into());
    }
    Ok(lines)
}

pub fn convert(
    content: &[u8],
    mapping: &Value,
    tenant: &str,
    account_name: &str,
    account_id: &str,
    currency: &str,
) -> Result<Vec<Value>, String> {
    let report = inspect_export(content, tenant, account_name)?;
    if currency.len() != 3
        || !currency.bytes().all(|b| b.is_ascii_uppercase())
        || account_id.trim_matches(text::space).is_empty()
    {
        return Err("Supply a 3-letter currency and explicit account ID.".into());
    }
    let lines = mappings(mapping, &report)?;
    Ok(report["detail"].as_array().expect("inspection detail").iter().map(|row| {
        json!({"Date": row["Date"], "Reference": row["Reference"], "Description": row["Description"], "Debit": row["Debit"], "Credit": row["Credit"], "Tenant": tenant, "AccountID": account_id, "Currency": currency, "TransactionID": lines[&row["source_row"].to_string()]})
    }).collect())
}

pub fn output_csv(rows: &[Value]) -> Result<String, String> {
    for row in rows {
        for key in ["Tenant", "AccountID", "Reference", "Description"] {
            if row[key]
                .as_str()
                .expect("converted text")
                .trim_start_matches(text::space)
                .starts_with(['=', '+', '-', '@'])
            {
                return Err(format!(
                    "{key} starts with a spreadsheet formula prefix; review the source."
                ));
            }
        }
    }
    let mut output = csv::write_record(OUTPUT);
    for row in rows {
        output.push_str(&csv::write_record(
            &OUTPUT
                .iter()
                .map(|key| row[key].as_str().expect("converted field"))
                .collect::<Vec<_>>(),
        ));
    }
    Ok(output)
}
