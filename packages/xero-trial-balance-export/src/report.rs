//! Validate the Xero report before any output is written.
use crate::{
    Failure, Result,
    amount::{self, Amount, Totals},
    pyjson::Json,
    pytext,
};
use std::collections::{BTreeSet, HashSet};

pub const COLUMNS: [&str; 10] = [
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
pub type Record = Vec<(String, Json)>;
pub type OutputRow = [String; 10];

pub fn text(value: &Json) -> String {
    match value {
        Json::Null => "None".into(),
        Json::Bool(b) => if *b { "True" } else { "False" }.into(),
        Json::Int(n) => n.to_string(),
        Json::Float(f) => crate::pyjson::float_repr(*f),
        Json::Str(s) => s.os_text(),
        Json::Array(items) => format!(
            "[{}]",
            items.iter().map(repr).collect::<Vec<_>>().join(", ")
        ),
        Json::Object(pairs) => format!(
            "{{{}}}",
            pairs
                .iter()
                .map(|(k, v)| format!("{}: {}", pytext::repr_code_points(&k.0), repr(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn repr(value: &Json) -> String {
    match value {
        Json::Str(s) => pytext::repr_code_points(&s.0),
        _ => text(value),
    }
}

fn type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Int(_) => "int",
        Json::Float(_) => "float",
        Json::Str(_) => "str",
        Json::Array(_) => "list",
        Json::Object(_) => "dict",
    }
}

pub fn truth(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(v) => *v,
        Json::Int(n) => n != &num_bigint::BigInt::from(0),
        Json::Float(n) => *n != 0.0,
        Json::Str(s) => !s.0.is_empty(),
        Json::Array(v) => !v.is_empty(),
        Json::Object(v) => !v.is_empty(),
    }
}

pub fn get<'a>(record: &'a Record, name: &str) -> Option<&'a Json> {
    record
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

fn set(record: &mut Record, key: &str, value: Json) {
    if let Some((_, prior)) = record.iter_mut().find(|(name, _)| name == key) {
        *prior = value;
    } else {
        record.push((key.to_string(), value));
    }
}

fn objects<'a>(container: &'a Json, key: &str, where_: &str) -> Result<&'a [Json]> {
    match container.get(key) {
        None => Ok(&[]),
        Some(Json::Array(items)) if items.iter().all(|item| matches!(item, Json::Object(_))) => {
            Ok(items)
        }
        _ => Err(Failure::message(format!(
            "error: report {where_} has a {key} value that is not a list of objects. The API shape may have changed, or this is not a trial balance report."
        ))),
    }
}

fn cell_text(value: &Json, where_: &str) -> Result<String> {
    match value {
        Json::Null => Ok(String::new()),
        Json::Str(_) | Json::Int(_) | Json::Float(_) => Ok(text(value)),
        _ => Err(Failure::message(format!(
            "error: report {where_} has a cell Value that is not text or a number ({}). The API shape may have changed, or this is not a trial balance report.",
            type_name(value)
        ))),
    }
}

fn cell_values(row: &Json, where_: &str) -> Result<Vec<String>> {
    objects(row, "Cells", where_)?
        .iter()
        .map(|c| cell_text(c.get("Value").unwrap_or(&Json::Null), where_))
        .collect()
}

fn record(row: &Json, header: &[String], where_: &str) -> Result<Record> {
    let values = cell_values(row, where_)?;
    if header.len() != values.len() {
        return Err(Failure::message(format!(
            "error: report {where_} has a row of {} cells under {} header columns. The API shape may have changed, or this is not a trial balance report.",
            values.len(),
            header.len()
        )));
    }
    Ok(header
        .iter()
        .cloned()
        .zip(values.into_iter().map(|v| Json::str(&v)))
        .collect())
}

fn account_id(row: &Json, where_: &str) -> Result<String> {
    let Some(first) = objects(row, "Cells", where_)?.first() else {
        return Ok(String::new());
    };
    for attr in objects(first, "Attributes", where_)? {
        if attr.get("Id").is_some_and(|v| v.is_str("account")) {
            return cell_text(attr.get("Value").unwrap_or(&Json::Null), where_);
        }
    }
    Ok(String::new())
}

pub fn flatten(report: &Json) -> Result<(Vec<String>, Vec<Record>)> {
    let mut header = Vec::new();
    let mut flat = Vec::new();
    let mut controls: Vec<(Option<Vec<usize>>, Vec<Record>)> = Vec::new();
    for top in objects(report, "Rows", "top level")? {
        match top.get("RowType").and_then(Json::as_pystr) {
            Some(kind) if kind.is("Header") => {
                header = cell_values(top, "top level")?;
                let repeated: BTreeSet<_> = header
                    .iter()
                    .filter(|title| header.iter().filter(|t| t == title).count() > 1)
                    .collect();
                if !repeated.is_empty() {
                    return Err(Failure::message(format!(
                        "error: report header repeats the column title(s) {}, so the cells under them cannot be told apart. The API shape may have changed, or this is not a trial balance report.",
                        repeated
                            .iter()
                            .map(|t| amount::shown(t))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
            Some(kind) if kind.is("Section") => {
                let section = top.get("Title").cloned().unwrap_or_else(|| Json::str(""));
                let where_ = format!("section \"{}\"", amount::shown(&text(&section)));
                let (mut indices, mut summaries) = (Vec::new(), Vec::new());
                for row in objects(top, "Rows", &where_)? {
                    if !row
                        .get("RowType")
                        .is_some_and(|v| v.is_str("Row") || v.is_str("SummaryRow"))
                    {
                        continue;
                    }
                    let mut entry = record(row, &header, &where_)?;
                    if row.get("RowType").is_some_and(|v| v.is_str("SummaryRow")) {
                        summaries.push(entry);
                        continue;
                    }
                    set(&mut entry, "Section", section.clone());
                    set(
                        &mut entry,
                        "AccountID",
                        Json::str(&account_id(row, &where_)?),
                    );
                    indices.push(flat.len());
                    flat.push(entry);
                }
                let selected = if !truth(&section) && indices.is_empty() {
                    None
                } else {
                    Some(indices)
                };
                controls.push((selected, summaries));
            }
            Some(kind) if kind.is("SummaryRow") => {
                controls.push((None, vec![record(top, &header, "grand total")?]))
            }
            _ => {}
        }
    }
    for (indices, summaries) in controls {
        if summaries.is_empty() {
            continue;
        }
        let mut totals = std::array::from_fn(|_| Amount::zero());
        for index in indices.unwrap_or_else(|| (0..flat.len()).collect()) {
            totals = amount::add_totals(&totals, &row_amounts(&flat[index])?)?;
        }
        for summary in summaries {
            if !row_amounts(&summary)?
                .iter()
                .zip(&totals)
                .all(|(a, b)| a.equal(b))
            {
                return Err(Failure::message(
                    "error: Trial Balance summary contradicts the account rows. Nothing written.",
                ));
            }
        }
    }
    Ok((header, flat))
}

pub fn to_number(value: &Json) -> Result<Amount> {
    if matches!(value, Json::Null) {
        Ok(Amount::zero())
    } else {
        Amount::parse(&text(value))
    }
}

pub fn row_amounts(row: &Record) -> Result<Totals> {
    let parse = |key| to_number(get(row, key).unwrap_or(&Json::Null));
    Ok([
        parse("Debit")?,
        parse("Credit")?,
        parse("YTD Debit")?,
        parse("YTD Credit")?,
    ])
}

pub fn excel_safe(value: &str) -> String {
    let trigger = |text: &str| text.starts_with(['=', '+', '-', '@', '\t', '\r', '\n']);
    if trigger(value) || trigger(value.trim_start_matches(|ch: char| pytext::is_space(ch as u32))) {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

pub fn is_account_code(code: &str) -> bool {
    code.len() <= 10
        && !code.is_empty()
        && code.bytes().all(|b| b.is_ascii_alphanumeric())
        && code.bytes().any(|b| b.is_ascii_digit())
}

fn account_parts(account: &str) -> (&str, &str) {
    let trimmed = account.trim_end_matches(|c: char| pytext::is_space(c as u32));
    if !trimmed.ends_with(')') {
        return (account, "");
    }
    let Some(index) = trimmed.rfind('(') else {
        return (account, "");
    };
    let code = &trimmed[index + 1..trimmed.len() - 1];
    let name = trimmed[..index].trim_end_matches(|c: char| pytext::is_space(c as u32));
    if !is_account_code(code) || name.contains('\n') {
        return (account, "");
    }
    (name, code)
}

pub fn build_rows(rows: &[Record], tenant: &Json, date: &str) -> Result<(Vec<OutputRow>, Totals)> {
    let mut out = Vec::new();
    let mut totals = std::array::from_fn(|_| Amount::zero());
    let mut ids = HashSet::new();
    for row in rows {
        let id = get(row, "AccountID").map(text).unwrap_or_default();
        let key = pytext::casefold(pytext::strip(&id));
        if key.is_empty() || !ids.insert(key) {
            return Err(Failure::message(
                "error: Trial Balance has a missing or repeated account ID. Nothing written.",
            ));
        }
        let account = get(row, "Account").map(text).unwrap_or_default();
        let (name, code) = account_parts(&account);
        let amounts = row_amounts(row)?;
        totals = amount::add_totals(&totals, &amounts)?;
        out.push([
            date.into(),
            excel_safe(&tenant.get("tenantName").map(text).unwrap_or_default()),
            excel_safe(&get(row, "Section").map(text).unwrap_or_default()),
            excel_safe(&id),
            excel_safe(name),
            excel_safe(code),
            amounts[0].format(),
            amounts[1].format(),
            amounts[2].format(),
            amounts[3].format(),
        ]);
    }
    Ok((out, totals))
}

pub fn csv_bytes(rows: &[OutputRow]) -> Result<Vec<u8>> {
    let quote = |cell: &str| {
        if cell.contains([',', '"', '\r', '\n']) {
            format!("\"{}\"", cell.replace('"', "\"\""))
        } else {
            cell.to_string()
        }
    };
    let mut text = format!("\u{feff}{}\r\n", COLUMNS.join(","));
    for row in rows {
        let values = row.iter().map(|value| {
            crate::pyjson::PyStr::from_os_text(value).to_utf8()
                .map(|value| quote(&value))
                .ok_or_else(|| Failure::message("error: the export contains an unpaired Unicode surrogate and cannot be encoded as UTF-8; nothing was written."))
        }).collect::<Result<Vec<_>>>()?;
        text.push_str(&values.join(","));
        text.push_str("\r\n");
    }
    Ok(text.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_is_bom_crlf_and_escapes_free_text() {
        let row = std::array::from_fn(|i| {
            if i == 4 {
                "A,\"B\"".into()
            } else {
                String::new()
            }
        });
        let csv = csv_bytes(&[row]).unwrap();
        assert!(csv.starts_with(&[0xef, 0xbb, 0xbf]));
        assert!(String::from_utf8(csv).unwrap().contains("\"A,\"\"B\"\"\""));
    }
    #[test]
    fn csv_refuses_lone_surrogates_and_preserves_private_use_characters() {
        let mut row = std::array::from_fn(|_| String::new());
        row[4] = crate::pyjson::PyStr(vec![0xd800]).os_text();
        assert!(csv_bytes(&[row.clone()]).is_err());
        row[4] = crate::pyjson::PyStr::from("\u{10f7ff}\u{10f800}").os_text();
        let bytes = csv_bytes(&[row]).unwrap();
        assert!(
            String::from_utf8(bytes)
                .unwrap()
                .contains("\u{10f7ff}\u{10f800}")
        );
    }
    #[test]
    fn formula_and_account_controls() {
        for trigger in ["=x", " +1", "@x", "\tx", "\rx", "\nx"] {
            assert_eq!(excel_safe(trigger), format!("'{trigger}"));
        }
        assert_eq!(account_parts("Bank (090)"), ("Bank", "090"));
        assert_eq!(
            account_parts("Term Deposit (NAB)"),
            ("Term Deposit (NAB)", "")
        );
    }
}
