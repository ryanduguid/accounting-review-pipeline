use crate::{
    Result,
    decimal::{self, Decimal},
    isodate, publication, pycsv,
    pyjson::{self, Json, PyStr},
    reconciliation::{self, COLUMNS, DECISION_COLUMNS},
    report,
};
use std::path::{Path, PathBuf};

fn text(value: &Json) -> String {
    value.as_pystr().unwrap().to_utf8().unwrap()
}
fn field(value: &Json, key: &str) -> String {
    text(value.at(key))
}
fn escaped(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
pub fn csv(columns: &[&str], rows: &[Vec<String>]) -> String {
    let mut output = pycsv::write_record_lf(columns);
    for row in rows {
        output.push_str(&pycsv::write_record_lf(
            &row.iter().map(String::as_str).collect::<Vec<_>>(),
        ));
    }
    output
}
fn decision_csv_safe(value: &str) -> String {
    if value.starts_with('\'') {
        format!("'{value}")
    } else {
        report::csv_safe(value)
    }
}
pub fn render_html(pack: &Json) -> Result<String> {
    decimal::with_precision(40, || {
        let end = reconciliation::date(&field(pack, "period_end"))?;
        let notes = pack.at("notes");
        let rows = pack.at("outstanding").as_array().unwrap();
        let mut outstanding = String::new();
        for row in rows {
            let amount = Decimal::parse(&field(row, "Debit"))
                .unwrap()
                .sub(&Decimal::parse(&field(row, "Credit")).unwrap())
                .value;
            let age = isodate::ordinal(end)
                - isodate::ordinal(reconciliation::date(&field(row, "Date"))?);
            let values = [
                field(row, "TransactionID"),
                field(row, "Date"),
                age.to_string(),
                field(row, "Reference"),
                field(row, "Description"),
                reconciliation::fixed(&amount),
                notes
                    .get(&field(row, "TransactionID"))
                    .map(text)
                    .unwrap_or_default(),
            ];
            outstanding.push_str("<tr>");
            for value in values {
                outstanding.push_str(&format!("<td>{}</td>", escaped(&value)));
            }
            outstanding.push_str("</tr>");
        }
        let mut suggestions = String::new();
        for group in pack.at("suggestions").as_array().unwrap() {
            let ids = group
                .at("ids")
                .as_array()
                .unwrap()
                .iter()
                .map(text)
                .collect::<Vec<_>>()
                .join(", ");
            suggestions.push_str(&format!(
                "<li><strong>{}</strong>: {}. {}</li>",
                escaped(&field(group, "group")),
                escaped(&ids),
                escaped(&field(group, "reason"))
            ));
        }
        let mut decisions = String::new();
        for group in pack.at("decisions").as_array().unwrap() {
            let decision = field(group, "decision");
            let ids = group
                .at("ids")
                .as_array()
                .unwrap()
                .iter()
                .map(text)
                .collect::<Vec<_>>()
                .join(", ");
            decisions.push_str(&format!(
                "<li><strong>{}</strong> {}: {}. {}</li>",
                escaped(if decision.is_empty() {
                    "pending"
                } else {
                    &decision
                }),
                escaped(&field(group, "group")),
                escaped(&ids),
                escaped(&field(group, "note"))
            ));
        }
        let Json::Object(identity_fields) = pack.at("identity") else {
            unreachable!()
        };
        let identity = identity_fields
            .iter()
            .map(|(_, v)| text(v))
            .collect::<Vec<_>>()
            .join(" / ");
        let blocked = if pack.at("status").is_str("BLOCKED") {
            "<p class='notice'>Balance checks failed. Resolve the differences before carrying items forward. This run has no carry-forward file.</p>"
        } else {
            ""
        };
        let mut totals = String::new();
        for (label, key) in [
            ("Opening ledger balance", "opening_balance"),
            ("Current movement", "movement"),
            ("Closing ledger balance", "closing_balance"),
            ("Outstanding items", "outstanding_total"),
            ("Opening items minus opening ledger", "opening_difference"),
            (
                "Opening ledger + movement minus closing ledger",
                "closing_difference",
            ),
        ] {
            totals.push_str(&format!(
                "<tr><th scope='row'>{label}</th><td>{}</td></tr>",
                escaped(&field(pack, key))
            ));
        }
        Ok(format!(
            include_str!("reconciliation.html"),
            identity = escaped(&identity),
            period_start = escaped(&field(pack, "period_start")),
            period_end = escaped(&field(pack, "period_end")),
            status = escaped(&field(pack, "status")),
            count = rows.len(),
            boundary = escaped(&field(pack, "review_boundary")),
            blocked = blocked,
            totals = totals,
            outstanding = outstanding,
            suggestions = if suggestions.is_empty() {
                "<li>No suggested matches.</li>"
            } else {
                &suggestions
            },
            decisions = if decisions.is_empty() {
                "<li>No review groups supplied.</li>"
            } else {
                &decisions
            }
        ))
    })
}
pub fn rendered(pack: &Json) -> Result<Vec<(String, Vec<u8>)>> {
    let end = reconciliation::date(&field(pack, "period_end"))?;
    let notes = pack.at("notes");
    let mut outstanding_rows = Vec::new();
    for row in pack.at("outstanding").as_array().unwrap() {
        let mut values: Vec<_> = COLUMNS
            .iter()
            .map(|key| {
                let raw = field(row, key);
                if ["Debit", "Credit"].contains(key) {
                    raw
                } else {
                    report::csv_safe(&raw)
                }
            })
            .collect();
        values.push(
            (isodate::ordinal(end) - isodate::ordinal(reconciliation::date(&field(row, "Date"))?))
                .to_string(),
        );
        values.push(report::csv_safe(
            &notes
                .get(&field(row, "TransactionID"))
                .map(text)
                .unwrap_or_default(),
        ));
        outstanding_rows.push(values);
    }
    let mut decision_rows = Vec::new();
    for group in pack.at("decisions").as_array().unwrap() {
        for id in group.at("ids").as_array().unwrap() {
            decision_rows.push(vec![
                decision_csv_safe(&field(group, "group")),
                text(id),
                field(group, "decision"),
                decision_csv_safe(&field(group, "note")),
            ]);
        }
    }
    for group in pack.at("suggestions").as_array().unwrap() {
        for id in group.at("ids").as_array().unwrap() {
            decision_rows.push(vec![
                report::csv_safe(&field(group, "group")),
                text(id),
                String::new(),
                String::new(),
            ]);
        }
    }
    let columns: Vec<_> = COLUMNS
        .into_iter()
        .chain(["AgeDays", "ReviewNote"])
        .collect();
    let bom = |value: String| {
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend(value.as_bytes());
        bytes
    };
    let mut files = vec![
        (
            "reconciliation.json".into(),
            (pyjson::pretty_utf8(pack)? + "\n").into_bytes(),
        ),
        ("review.html".into(), render_html(pack)?.into_bytes()),
        (
            "outstanding.csv".into(),
            bom(csv(&columns, &outstanding_rows)),
        ),
        (
            "suggestions.csv".into(),
            bom(csv(&DECISION_COLUMNS, &decision_rows)),
        ),
    ];
    if !pack.at("status").is_str("BLOCKED") {
        let outstanding = pack.at("outstanding").as_array().unwrap();
        let carry_notes = Json::Object(
            outstanding
                .iter()
                .filter_map(|row| {
                    let key = field(row, "TransactionID");
                    notes
                        .get(&key)
                        .map(|value| (PyStr::from(key.as_str()), value.clone()))
                })
                .collect(),
        );
        let carry = Json::object([
            ("schema", Json::str("clearing-carry-v1")),
            ("identity", pack.at("identity").clone()),
            ("period_end", pack.at("period_end").clone()),
            ("balance", pack.at("closing_balance").clone()),
            ("items", pack.at("outstanding").clone()),
            ("notes", carry_notes),
        ]);
        files.push((
            "carry-forward.json".into(),
            (pyjson::pretty_utf8(&carry)? + "\n").into_bytes(),
        ));
    }
    Ok(files)
}
pub fn write_reconciliation(pack: &Json, output: &Path) -> Result<PathBuf> {
    Ok(publication::write_new_files(output, || rendered(pack))?.join("review.html"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decision_export_reversibly_guards_literal_apostrophes_and_formulas() {
        assert_eq!(decision_csv_safe("'literal"), "''literal");
        assert_eq!(decision_csv_safe(" =formula"), "' =formula");
        assert_eq!(decision_csv_safe("ordinary"), "ordinary");
        assert_eq!(
            csv(&["one", "two"], &[vec!["a,b".into(), "\"quoted\"".into()]]),
            "one,two\n\"a,b\",\"\"\"quoted\"\"\"\n"
        );
    }
}
