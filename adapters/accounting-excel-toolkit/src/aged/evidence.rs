use super::{Json, MANIFEST_FIELDS, hash, headers, json, records, string};
use crate::transactions::{
    date,
    text::{digit, space},
};

pub fn draft(content: &[u8]) -> Result<Json, String> {
    let records = records(content)?;
    let candidates = headers(&records);
    if candidates.len() != 1 {
        return Err("Expected one summary header before drafting source declarations.".into());
    }
    let metadata: Vec<_> = records[1..candidates[0].max(1)]
        .iter()
        .filter_map(|r| r.first())
        .map(|s| s.trim_matches(space))
        .filter(|s| !s.is_empty())
        .collect();
    draft_metadata(&records, &metadata, content)
}

fn draft_metadata(
    records: &[Vec<String>],
    metadata: &[&str],
    content: &[u8],
) -> Result<Json, String> {
    let entities: Vec<_> = metadata
        .iter()
        .filter(|s| !s.starts_with("As at ") && !s.starts_with("Ageing by "))
        .collect();
    let dates: Vec<_> = metadata
        .iter()
        .filter_map(|s| s.strip_prefix("As at "))
        .collect();
    let ageing: Vec<_> = metadata
        .iter()
        .filter_map(|s| s.strip_prefix("Ageing by "))
        .collect();
    let cutoff = if dates.len() == 1 {
        date::parse(dates[0], true)
            .map(|d| d.iso())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let mut manifest = Json::Object(
        MANIFEST_FIELDS
            .iter()
            .map(|key| ((*key).into(), Json::str("")))
            .collect(),
    );
    let title = records
        .first()
        .and_then(|r| r.first())
        .map_or("", |s| s.trim_matches(space));
    for (key, value) in [
        ("report_name", title),
        ("entity", if entities.len() == 1 { entities[0] } else { "" }),
        ("as_at", cutoff.as_str()),
        (
            "ageing_basis",
            if ageing.len() == 1 { ageing[0] } else { "" },
        ),
        ("source_format", "CSV"),
        ("source_sha256", hash(content).as_str()),
    ] {
        manifest.set(key, Json::str(value));
    }
    manifest.set("footer_record", Json::Null);
    manifest.set("on_screen_rows", Json::Null);
    manifest.set("on_screen_total", Json::str(""));
    manifest.set("draft_boundary",Json::str("Source declarations only. Independently confirm the timestamp, settings, footer role, on-screen observations and control before relying on the summary."));
    Ok(manifest)
}

fn source_records(issue: &str) -> Vec<Json> {
    let mut records = Vec::new();
    let mut start = 0;
    while let Some(pos) = issue[start..].find("record ") {
        let begin = start + pos + 7;
        let digits: Vec<_> = issue[begin..].chars().map_while(digit).collect();
        if !digits.is_empty() {
            let number = digits
                .iter()
                .fold(num_bigint::BigInt::from(0), |n, d| n * 10 + d);
            records.push(Json::Int(number));
        }
        start = begin;
    }
    records
}

pub fn requests(result: &Json) -> Json {
    let mut requests = Vec::new();
    for value in result
        .at("exceptions")
        .as_array()
        .expect("review exceptions")
    {
        let issue = string(value).expect("exception text");
        let lower = issue.to_lowercase();
        let action = if lower.contains("control") {
            "Obtain a matching dated receivables control and its source reference."
        } else if lower.contains("screen") {
            "Check the report on screen and independently record its population and total."
        } else if lower.contains("footer has not") || lower.contains("footer record is absent") {
            "Confirm the terminal footer's logical source record without changing contact rows."
        } else {
            "Recheck the identified source records and export settings; preserve the difference and re-export if needed."
        };
        requests.push(Json::object([
            ("source_records", Json::Array(source_records(&issue))),
            ("reason", value.clone()),
            ("owner", Json::str("Export preparer")),
            ("status", Json::str("UNRESOLVED")),
            ("next_action", Json::str(action)),
        ]));
    }
    for row in result.at("rows").as_array().expect("review rows") {
        requests.push(Json::object([("source_records",Json::Array(vec![row.at("source_record").clone()])),("contact",row.at("Contact").clone()),("reason",Json::str("Summary data cannot establish invoice or collection decisions.")),("owner",Json::str("Authorised debtor reviewer")),("status",Json::str("REVIEW")),("next_action",Json::str("Obtain dated Detail with stable contact/invoice identities, subsequent receipts and dispute evidence; record an authorised decision."))]));
    }
    Json::Array(requests)
}

pub fn briefing(result: &Json) -> String {
    let mut lines = vec![
        format!(
            "Summary: {}. Debtor decisions: {}.",
            json::str_value(result.at("summary_status")),
            json::str_value(result.at("debtor_decisions"))
        ),
        format!(
            "Source SHA-256: {}",
            json::str_value(result.at("source_sha256"))
        ),
        String::new(),
        "Supported arithmetic:".into(),
    ];
    for tie in result.at("ties").as_array().expect("review ties") {
        lines.push(format!(
            "{}: {} versus {}; difference {}; {}.",
            json::str_value(tie.at("check")),
            json::str_value(tie.at("left")),
            json::str_value(tie.at("right")),
            json::str_value(tie.at("difference")),
            if tie.at("agrees").as_bool() == Some(true) {
                "agrees"
            } else {
                "unresolved"
            }
        ));
    }
    lines.extend([
        String::new(),
        "Evidence requests (for preparation, not sending):".into(),
    ]);
    for request in requests(result).as_array().unwrap() {
        lines.push(json::dumps(request, true));
    }
    lines.extend([
        String::new(),
        json::str_value(result.at("boundary")),
        String::new(),
    ]);
    lines.join("\n")
}
