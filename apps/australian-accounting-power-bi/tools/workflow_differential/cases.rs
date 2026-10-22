use super::{Result, Value};
use serde_json::json;
use std::{fs, path::Path};

pub fn generate(run: &Path, mut seed: u64) -> Result<Vec<Value>> {
    let mut cases = Vec::new();
    json_cases(&mut cases, &mut seed);
    schema_cases(&mut cases, run)?;
    for month in 0..14 {
        cases.push(json!({"op":"ledger","month":month}));
    }
    for change in [
        json!({"financial_year_start":"2023-07-01"}),
        json!({"opening_journal":"missing"}),
        json!({"account_ids":{}}),
    ] {
        cases.push(json!({"op":"ledger","month":9,"case":change}));
    }
    for text in [
        "a,b\n1,2\n",
        "a,b\r\n\"x,y\",\"a\"\"b\"\r\n",
        "\u{feff}a,b\n1,2\n",
        "a\n\"line\nvalue\"\n",
        "a,b\n\n1,2\n",
    ] {
        cases.push(json!({"op":"csv","text":text}));
    }
    for members in [
        json!({}),
        json!({"a.txt":"hello\n"}),
        json!({"z.txt":"last","folder/a.txt":"<&\"' 雪\n"}),
    ] {
        cases.push(json!({"op":"zip","members":members}));
    }
    Ok(cases)
}

fn json_cases(cases: &mut Vec<Value>, seed: &mut u64) {
    for text in [
        "null",
        "true",
        "123",
        "-0.0",
        "[1,2,3]",
        "{\"雪\":\"<&\",\"a\":1.0}",
        "\u{feff}{\"a\":1}",
        "{\"a\":1,\"a\":2}",
        "NaN",
        "Infinity",
        "1e999",
        "{} garbage",
        "[",
        "{\"a\":\"[[]]\"}",
    ] {
        cases.push(json!({"op":"json","text":text}));
    }
    for depth in [63, 64, 65] {
        cases.push(json!({"op":"json","text":format!("{}0{}", "[".repeat(depth), "]".repeat(depth))}));
    }
    for count in [4299, 4300, 4301] {
        cases.push(json!({"op":"json","text":"9".repeat(count)}));
    }
    for _ in 0..2048 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let value = f64::from_bits(*seed);
        if value.is_finite() {
            cases.push(json!({"op":"json","text":format!("{{\"value\":{value:?}}}")}));
        }
    }
}

fn load(run: &Path, name: &str) -> Result<Value> {
    serde_json::from_slice(&fs::read(run.join(name)).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())
}

fn schema_cases(cases: &mut Vec<Value>, run: &Path) -> Result<()> {
    let receipt = load(run, "receipt.json")?;
    let fixtures = [
        ("case", load(run, "inputs/case.json")?),
        ("close", load(run, "close/close-review-pack.json")?),
        ("drivers", load(run, "drivers/variance-drivers.json")?),
        ("readiness", load(run, "readiness/readiness-pack.json")?),
        ("receipt", receipt.clone()),
        ("review-ready", receipt["producer_manifests"]["review-ready"].clone()),
        ("close-control", receipt["producer_manifests"]["close-control"].clone()),
    ];
    for (kind, baseline) in fixtures {
        cases.push(json!({"op":"schema","kind":kind,"value":baseline}));
        for value in [Value::Null, json!(true), json!(1), json!(""), json!([]), json!({})] {
            cases.push(json!({"op":"schema","kind":kind,"value":value}));
        }
        for key in baseline.as_object().unwrap().keys() {
            let mut missing = baseline.clone();
            missing.as_object_mut().unwrap().remove(key);
            cases.push(json!({"op":"schema","kind":kind,"value":missing}));
            for replacement in [
                Value::Null,
                json!(true),
                json!(1),
                json!(1.0),
                json!(""),
                json!([]),
                json!({}),
            ] {
                let mut changed = baseline.clone();
                changed[key] = replacement;
                cases.push(json!({"op":"schema","kind":kind,"value":changed}));
            }
        }
        for pointer in [
            "/thresholds",
            "/exceptions/0",
            "/client_queries/0",
            "/accounts/0",
            "/accounts/0/drivers/0",
            "/native",
            "/invocations/0",
        ] {
            nested_fields(cases, kind, &baseline, pointer);
        }
    }
    money_cases(cases);
    comparison_cases(cases);
    Ok(())
}

fn nested_fields(cases: &mut Vec<Value>, kind: &str, baseline: &Value, pointer: &str) {
    let Some(record) = baseline.pointer(pointer).and_then(Value::as_object) else {
        return;
    };
    for key in record.keys() {
        for replacement in [
            Value::Null,
            json!(true),
            json!(1),
            json!(1.0),
            json!(""),
            json!([]),
            json!({}),
        ] {
            let mut changed = baseline.clone();
            changed.pointer_mut(pointer).unwrap()[key] = replacement;
            cases.push(json!({"op":"schema","kind":kind,"value":changed}));
        }
    }
}

fn money_cases(cases: &mut Vec<Value>) {
    for kind in ["money", "nullable-money"] {
        for value in [
            Value::Null,
            json!(true),
            json!(1),
            json!(""),
            json!("0.00"),
            json!("-0.00"),
            json!("+1.00"),
            json!("1e2"),
            json!("1000.000"),
            json!("NaN"),
            json!("1.25"),
        ] {
            cases.push(json!({"op":"schema","kind":kind,"value":value}));
        }
    }
}

fn comparison_cases(cases: &mut Vec<Value>) {
    let finding = json!({"account_name":"Sales<&","current_value":"10.00","status":"REVIEW"});
    let query = json!({"query_id":"Q1","account_name":"Sales<&","control":"period_variance","account_id":"200"});
    for change in ["NEW", "CHANGED", "RECURRING", "NOT_RAISED", "NOT_COMPARABLE", "BAD"] {
        for sides in 0..4 {
            for scope in [vec![], vec!["period"]] {
                for group in ["findings", "queries"] {
                    let mut item =
                        json!({"change":change,"control":"period_variance","account_id":"200","query_id":"Q1"});
                    for (key, present) in [("previous", sides & 1 != 0), ("current", sides & 2 != 0)] {
                        item[key] = if group == "findings" {
                            if present { json!([finding.clone()]) } else { json!([]) }
                        } else if present {
                            query.clone()
                        } else {
                            Value::Null
                        };
                    }
                    let mut value = json!({"review_boundary":"Human review required","scope_changes":scope,"findings":[],"queries":[]});
                    value[group] = json!([item]);
                    cases.push(json!({"op":"schema","kind":"comparison","value":value}));
                }
            }
        }
    }
}
