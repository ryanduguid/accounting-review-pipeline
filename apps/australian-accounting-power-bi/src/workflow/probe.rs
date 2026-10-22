//! Test-only comparison surface for the frozen Python workflow.

use super::{Result, archive, digest, export, json, ledger, schema};
use serde_json::{Value, json};
use std::path::Path;

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
