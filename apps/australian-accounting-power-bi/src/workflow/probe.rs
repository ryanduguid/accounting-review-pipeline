// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Test-only comparison surface for the frozen Python workflow.

use super::{Result, archive, digest, export, json, ledger, schema};
use serde_json::{Value, json};
use std::path::Path;

/// Returns the same validated fabricated inputs embedded by the native workflow.
pub fn bundled_fixture_files() -> Result<std::collections::BTreeMap<String, Vec<u8>>> {
    let (_, mut sources) = ledger::bundled_sources()?;
    sources.insert(
        "shared-review-case.json".into(),
        include_bytes!("../../samples/shared-review-case.json").to_vec(),
    );
    Ok(sources)
}

pub fn evaluate(request: &Value) -> Value {
    match value(request) {
        Ok(value) => json!({"ok":true,"value":value}),
        Err(_) => json!({"ok":false}),
    }
}

fn value(request: &Value) -> Result<Value> {
    match request["op"].as_str().ok_or("Missing operation.")? {
        "json" => Ok(digest(&json::bytes(&json::parse(
            request["text"].as_str().unwrap().as_bytes(),
            "Input",
        )?))
        .into()),
        "schema" => {
            document(request["kind"].as_str().unwrap(), &request["value"])?;
            Ok(Value::Null)
        }
        "csv" => serde_json::to_value(ledger::rows(request["text"].as_str().unwrap().as_bytes())?)
            .map_err(|error| error.to_string()),
        "ledger" => ledger_case(request),
        "zip" => {
            let members: archive::Members = request["members"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(name, data)| (name.clone(), data.as_str().unwrap().as_bytes().to_vec()))
                .collect();
            Ok(digest(&archive::encode(&members)?).into())
        }
        "export" => export::verify(Path::new(request["path"].as_str().unwrap())),
        _ => Err("Unknown operation.".into()),
    }
}

fn document(kind: &str, value: &Value) -> Result<()> {
    match kind {
        "case" => schema::case(value),
        "close" => schema::close(value),
        "drivers" => schema::drivers(value),
        "readiness" => schema::readiness(value),
        "comparison" => schema::comparison(value),
        "receipt" => schema::receipt(value),
        "close-control" | "review-ready" => schema::producer(value, kind),
        "money" => schema::money(value, "Money", false),
        "nullable-money" => schema::money(value, "Money", true),
        _ => Err("Unknown schema.".into()),
    }
}

fn ledger_case(request: &Value) -> Result<Value> {
    let (mut case, mut source) = ledger::sources(Path::new(env!("CARGO_MANIFEST_DIR")))?;
    if let Some(changes) = request["case"].as_object() {
        for (key, value) in changes {
            case[key] = value.clone();
        }
    }
    if let Some(changes) = request["sources"].as_object() {
        for (name, value) in changes {
            source.insert(name.clone(), value.as_str().unwrap().as_bytes().to_vec());
        }
    }
    let month = request["month"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or("Unsupported month.")?;
    let (current, prior, transactions) = ledger::trial_balances(&case, &source, month)?;
    Ok(json!([digest(&current), digest(&prior), digest(&transactions)]))
}

#[cfg(test)]
mod tests {
    use super::super::files::Temporary;
    use super::*;

    #[test]
    fn unknown_requests_are_refused_and_export_and_source_requests_reach_their_checks() {
        assert_eq!(value(&json!({"op":"other"})).unwrap_err(), "Unknown operation.");
        let schema = json!({"op":"schema","kind":"other","value":null});
        assert_eq!(value(&schema).unwrap_err(), "Unknown schema.");
        let temporary = Temporary::new("pbi-probe-export-").unwrap();
        let path = temporary.0.join("export.zip");
        std::fs::write(&path, b"not a stored archive").unwrap();
        let export = json!({"op":"export","path":path.to_str().unwrap()});
        assert_eq!(
            value(&export).unwrap_err(),
            "Export ZIP is outside the canonical profile."
        );
        assert_eq!(evaluate(&export), json!({"ok":false}));
        let sources = json!({"sample-general-ledger.csv":"EntityID,PostingDate\n"});
        let ledger = json!({"op":"ledger","month":9,"sources":sources});
        assert_eq!(value(&ledger).unwrap_err(), "Ledger account population is not mapped.");
    }
}
