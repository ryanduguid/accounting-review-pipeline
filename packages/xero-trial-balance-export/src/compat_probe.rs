//! Test-only value probe; never compiled into either distributed command.
use crate::{
    Failure, Result,
    amount::{self, Amount},
    auth, cli, export, http, publication,
    pyjson::{self, Json},
    report, tokens,
};
use base64::Engine;
use sha2::Digest;
use std::io::{self, BufRead, Write};

fn array(values: impl IntoIterator<Item = Json>) -> Json {
    Json::Array(values.into_iter().collect())
}
fn strings(values: impl IntoIterator<Item = String>) -> Json {
    array(values.into_iter().map(|s| value_string(&s)))
}
fn value_string(value: &str) -> Json {
    Json::Str(pyjson::PyStr::from_os_text(value))
}
fn record(value: &Json) -> report::Record {
    match value {
        Json::Object(pairs) => pairs
            .iter()
            .map(|(k, v)| (k.os_text(), v.clone()))
            .collect(),
        _ => Vec::new(),
    }
}
fn records(value: &Json) -> Vec<report::Record> {
    match value {
        Json::Array(items) => items.iter().map(record).collect(),
        _ => Vec::new(),
    }
}
fn record_json(value: &report::Record) -> Json {
    Json::Object(
        value
            .iter()
            .map(|(k, v)| (pyjson::PyStr::from_os_text(k), v.clone()))
            .collect(),
    )
}
fn amounts(value: &Json) -> Result<amount::Totals> {
    let Json::Array(values) = value else {
        return Err(Failure::message("expected four amounts"));
    };
    if values.len() != 4 {
        return Err(Failure::message("expected four amounts"));
    }
    let parse = |value: &Json| {
        amount::parse_literal(&report::text(value))
            .ok_or_else(|| Failure::message("invalid internal decimal"))
    };
    Ok([
        parse(&values[0])?,
        parse(&values[1])?,
        parse(&values[2])?,
        parse(&values[3])?,
    ])
}

pub fn execute(input: &Json) -> Result<Json> {
    let op = report::text(input.at("op"));
    let args = match input.get("args") {
        Some(Json::Array(values)) => values.as_slice(),
        _ => &[],
    };
    let arg = |index: usize| args.get(index).unwrap_or(&Json::Null);
    let string = |index| report::text(arg(index));
    match op.as_str() {
        "to_number" | "format_amount" => Ok(Json::str(&report::to_number(arg(0))?.format())),
        "is_account_code" => Ok(Json::Bool(report::is_account_code(&string(0)))),
        "iso_date" => Ok(Json::str(&cli::iso_date(&string(0))?)),
        "excel_safe" => Ok(value_string(&report::excel_safe(&string(0)))),
        "validate_report_identity" => {
            export::validate_report_identity(arg(0), &string(1))?;
            Ok(Json::Null)
        }
        "flatten_report" => {
            let (header, rows) = report::flatten(arg(0))?;
            Ok(array([
                strings(header),
                array(rows.iter().map(record_json)),
            ]))
        }
        "validated_connections" => {
            export::validated_connections(arg(0))?;
            Ok(arg(0).clone())
        }
        "select_tenant" => {
            let Json::Array(connections) = arg(0) else {
                return Err(Failure::message("expected connections"));
            };
            let requested = string(1);
            Ok(export::select_tenant(
                connections,
                if matches!(arg(1), Json::Null) {
                    None
                } else {
                    Some(&requested)
                },
            )?
            .clone())
        }
        "default_output_filename" => Ok(Json::str(&export::default_filename(
            arg(0),
            &string(1),
            &string(2),
        ))),
        "row_amounts" => Ok(strings(
            report::row_amounts(&record(arg(0)))?
                .iter()
                .map(Amount::format),
        )),
        "add_totals" => Ok(strings(
            amount::add_totals(&amounts(arg(0))?, &amounts(arg(1))?)?
                .iter()
                .map(Amount::format),
        )),
        "check_balanced" => {
            let count = input
                .get("account_count")
                .and_then(tokens::number)
                .map(|n| n as usize);
            amount::check_balanced(&amounts(arg(0))?, count)?;
            Ok(Json::Null)
        }
        "build_rows" => {
            let (rows, totals) = report::build_rows(&records(arg(0)), arg(1), &string(2))?;
            let rows = rows.into_iter().map(|row| {
                Json::Object(
                    report::COLUMNS
                        .iter()
                        .zip(row)
                        .map(|(k, v)| (pyjson::PyStr::from(*k), value_string(&v)))
                        .collect(),
                )
            });
            Ok(array([
                array(rows),
                strings(totals.iter().map(Amount::format)),
            ]))
        }
        "retry_after_seconds" => {
            let now = input
                .get("now")
                .and_then(tokens::number)
                .and_then(|n| {
                    chrono::DateTime::from_timestamp(
                        n.floor() as i64,
                        ((n.fract() * 1_000_000_000.0).round() as u32).min(999_999_999),
                    )
                })
                .unwrap_or_else(chrono::Utc::now);
            let value = string(0);
            Ok(Json::int(http::retry_after(
                if matches!(arg(0), Json::Null) {
                    None
                } else {
                    Some(&value)
                },
                now,
            ) as i64))
        }
        "validate_rotated_response" => {
            let (value, warning) = tokens::validate_rotated(arg(0))?;
            Ok(Json::object([
                ("value", value),
                ("stderr", Json::str(&warning)),
            ]))
        }
        "validate_token_response" => {
            tokens::validate_cached(
                arg(0),
                &string(1),
                input.get("cached").is_some_and(report::truth),
            )?;
            Ok(arg(0).clone())
        }
        "callback_server_config" => {
            let config = auth::callback_config(&string(0))?;
            Ok(array([
                Json::str(&config.host),
                Json::int(i64::from(config.port)),
                Json::str(&config.path),
            ]))
        }
        "manifest" => Ok(publication::manifest(
            std::path::Path::new(&string(0)),
            &string(1),
            arg(2),
            &string(3),
            &string(4),
            &string(5),
        )),
        "csv_artifact" => {
            let (rows, totals) = report::build_rows(&records(arg(0)), arg(1), &string(2))?;
            amount::check_balanced(&totals, Some(rows.len()))?;
            for key in ["tenantId", "tenantName"] {
                if arg(1).get(key).is_none() {
                    return Err(Failure::message(format!("'{key}'")));
                }
            }
            let bytes = report::csv_bytes(&rows)?;
            let digest: String = sha2::Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let manifest = publication::manifest(
                std::path::Path::new("report.csv"),
                &digest,
                arg(1),
                &string(2),
                "accrual",
                "2026-06-30T12:34:56+00:00",
            );
            Ok(Json::object([
                (
                    "csv",
                    Json::str(&base64::engine::general_purpose::STANDARD.encode(bytes)),
                ),
                (
                    "manifest",
                    Json::str(&format!("{}\n", pyjson::pretty(&manifest))),
                ),
            ]))
        }
        "decode_token_cache" => {
            let (tokens, legacy) = tokens::decode_cache(string(0).as_bytes())?;
            Ok(array([tokens, Json::Bool(legacy)]))
        }
        "json_response" => {
            pyjson::loads_response(&string(0)).map_err(|_| Failure::message("invalid JSON"))
        }
        _ => Err(Failure::message(format!("unknown probe operation: {op}"))),
    }
}

/// The removed Python entrypoints become their native command names in guidance.
pub fn migrate_guidance(value: &mut Json) {
    if let Json::Object(fields) = value
        && let Some((_, Json::Str(text))) = fields.iter_mut().find(|(key, _)| key.is("stderr"))
    {
        *text = pyjson::PyStr::from(
            text.os_text()
                .replace("python auth.py", "xero-tb-auth")
                .replace("Run auth.py", "Run xero-tb-auth")
                .replace("run auth.py again", "run xero-tb-auth again")
                .as_str(),
        );
    }
}

pub fn run() {
    for line in io::stdin().lock().lines() {
        let output = match line.ok().and_then(|line| pyjson::loads(&line).ok()) {
            Some(value) => match execute(&value) {
                Ok(value) => Json::object([("ok", value)]),
                Err(error) => Json::object([
                    ("code", Json::int(i64::from(error.code))),
                    ("stdout", Json::str(&error.stdout)),
                    ("stderr", Json::str(&error.stderr)),
                ]),
            },
            None => Json::object([
                ("code", Json::int(1)),
                ("stderr", Json::str("invalid probe JSON")),
            ]),
        };
        let mut stdout = io::stdout().lock();
        if writeln!(stdout, "{}", pyjson::canonical(&output))
            .and_then(|()| stdout.flush())
            .is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod regression {
    use super::*;
    #[test]
    fn captured_python_test_behaviours() {
        let captured =
            pyjson::loads(include_str!("../tests/fixtures/python-value-cases.json")).unwrap();
        let Json::Array(cases) = captured.at("cases") else {
            panic!("case array");
        };
        let mut differences = Vec::new();
        for case in cases {
            let input = case.at("request");
            let result = match execute(input) {
                Ok(value) => Json::object([("ok", value)]),
                Err(error) => Json::object([
                    ("code", Json::int(i64::from(error.code))),
                    ("stdout", Json::str(&error.stdout)),
                    ("stderr", Json::str(&error.stderr)),
                ]),
            };
            let mut expected = case.at("expected").clone();
            migrate_guidance(&mut expected);
            if !pyjson::py_eq(&result, &expected, true) {
                let shorten = |value| {
                    pyjson::canonical(value)
                        .chars()
                        .take(300)
                        .collect::<String>()
                };
                differences.push(format!(
                    "{} / {}: expected {} got {}",
                    report::text(case.at("test")),
                    report::text(input.at("op")),
                    shorten(case.at("expected")),
                    shorten(&result)
                ));
            }
        }
        assert!(
            differences.is_empty(),
            "{} differences among {} cases:\n{}",
            differences.len(),
            cases.len(),
            differences
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
