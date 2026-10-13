//! Fabricated vectors recorded from every reachable core call in the original
//! 0.2.0 suite, not generated from the Rust implementation. Version alone is
//! substituted in disclosure records. Cargo tests need no installed Python.
use evatt::{
    disclosure,
    entities::{self, Entity},
    patterns,
    pyjson::{self, Json, PyStr},
    pytext, redact, verify,
};

fn string(value: &Json) -> String {
    value.as_pystr().and_then(PyStr::to_utf8).unwrap_or_default()
}
fn list(value: &Json) -> &[Json] {
    let Json::Array(items) = value else { panic!("array") };
    items
}
fn value_entries(value: &Json) -> Vec<Entity> {
    list(value)
        .iter()
        .map(|e| Entity {
            value: e.at("value").as_pystr().unwrap().clone(),
            placeholder: string(e.at("placeholder")),
            kind: string(e.at("kind")),
            added: string(e.at("added")),
        })
        .collect()
}
fn count(text: &str, byte: usize) -> Json {
    Json::int(text[..byte].chars().count() as i64)
}
fn spans(text: &str, found: &[patterns::Span], kind: bool) -> Json {
    Json::Array(
        found
            .iter()
            .map(|s| {
                let mut row = vec![count(text, s.start), count(text, s.end)];
                if kind {
                    row.push(Json::str(&s.kind));
                }
                row.push(Json::str(&s.value));
                Json::Array(row)
            })
            .collect(),
    )
}
fn unknowns(found: &[redact::Unknown]) -> Json {
    Json::Array(
        found
            .iter()
            .map(|u| {
                Json::object([
                    ("kind", Json::str(&u.kind)),
                    ("value", Json::str(&u.value)),
                    ("line", Json::int(u.line as i64)),
                    ("context", Json::str(&u.context)),
                ])
            })
            .collect(),
    )
}
fn ok(value: Json) -> Json {
    Json::object([("ok", value)])
}
fn error(kind: &str, message: &str) -> Json {
    Json::object([("error", Json::str(kind)), ("message", Json::str(message))])
}
fn bytes(value: &Json) -> Vec<u8> {
    let hex = string(value.at("hex"));
    hex.as_bytes()
        .chunks(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}
fn encoded(value: &[u8]) -> Json {
    Json::object([(
        "hex",
        Json::str(&value.iter().map(|b| format!("{b:02x}")).collect::<String>()),
    )])
}

fn disclosure_call(row: &Json, args: &[Json]) -> Json {
    let mode = string(row.at("mode"));
    let kwargs = row.at("kwargs");
    let payload = bytes(&args[0]);
    let entries = value_entries(&args[1]);
    let result = if mode == "disclosure.create_record" {
        disclosure::create(
            &payload,
            &entries,
            &string(kwargs.at("destination")),
            &string(kwargs.at("decision_ref")),
        )
    } else {
        disclosure::check(
            &payload,
            &entries,
            &bytes(&args[2]),
            &string(kwargs.at("expected_destination")),
            &string(kwargs.at("expected_decision_ref")),
        )
        .map(|()| payload.clone())
    };
    match result {
        Ok(value) => ok(encoded(&value)),
        Err(disclosure::Error::Invalid(message)) => error("EvattError", &message),
        Err(disclosure::Error::Refused(message)) => error("DisclosureRefused", message),
    }
}

fn map_load(raw: &str) -> Json {
    let document = match pyjson::loads_plain(raw) {
        Ok(document) => document,
        Err(pyjson::LoadError::Decode(message)) => {
            return error("EvattError", &format!("cannot read entity map <map>: {message}"));
        }
        Err(pyjson::LoadError::Crash(message)) => return error("ValueError", &message),
        Err(other) => panic!("{other:?}"),
    };
    match entities::load(&document) {
        Ok(entries) => ok(Json::Array(
            entries
                .iter()
                .map(|e| {
                    Json::object([
                        ("value", Json::Str(e.value.clone())),
                        ("placeholder", Json::str(&e.placeholder)),
                        ("kind", Json::str(&e.kind)),
                        ("added", Json::str(&e.added)),
                    ])
                })
                .collect(),
        )),
        Err(message) => error("EvattError", &message),
    }
}

fn redaction(text: &str, entries: &[Entity], strict: bool) -> Json {
    let result = redact::redact(text, entries).unwrap();
    if strict && !result.unknowns.is_empty() {
        return Json::object([
            ("error", Json::str("Halt")),
            (
                "message",
                Json::str(&format!(
                    "{} unclassified candidate(s); nothing was written",
                    result.unknowns.len()
                )),
            ),
            ("unknowns", unknowns(&result.unknowns)),
        ]);
    }
    let counts = Json::Object(
        result
            .counts
            .iter()
            .map(|(k, n)| (k.as_str().into(), Json::int(*n as i64)))
            .collect(),
    );
    ok(Json::Array(vec![Json::str(&result.text), counts]))
}

fn evaluate(row: &Json) -> Json {
    let mode = string(row.at("mode"));
    let args = list(row.at("args"));
    if mode.starts_with("disclosure.") {
        return disclosure_call(row, args);
    }
    let pytext = args[0].as_pystr().unwrap();
    let text = pytext.to_utf8().unwrap_or_else(|| pytext.os_text());
    match mode.as_str() {
        "entities._fold" => ok(Json::Str(PyStr::from_os_text(
            &patterns::fold(&pytext.os_text()).unwrap(),
        ))),
        "entities.load" => map_load(&text),
        "patterns.structured_spans" => ok(spans(&text, &patterns::structured_spans(&text).unwrap(), true)),
        "patterns.person_name_spans" => ok(spans(&text, &patterns::person_names(&text).unwrap(), false)),
        "patterns._ambiguous_identifiers" => ok(Json::Array(
            patterns::ambiguous(&text)
                .unwrap()
                .iter()
                .map(|a| {
                    Json::object([
                        ("start", count(&text, a.start)),
                        ("digit_start", count(&text, a.digit_start)),
                        ("end", count(&text, a.end)),
                        ("value", Json::str(&a.value)),
                    ])
                })
                .collect(),
        )),
        "redact.residual" => ok(unknowns(&redact::residual(&text).unwrap())),
        "redact.redact" => redaction(
            &text,
            &value_entries(&args[1]),
            !matches!(row.at("kwargs").get("strict"), Some(Json::Bool(false))),
        ),
        "restore.restore" => ok(Json::str(&verify::restore(&text, &value_entries(&args[1])).unwrap())),
        "verify.findings" => ok(Json::Array(
            verify::findings(&text, &value_entries(&args[1]))
                .unwrap()
                .iter()
                .map(|f| Json::object([("kind", Json::str(&f.kind)), ("value", Json::str(&f.value))]))
                .collect(),
        )),
        _ => panic!("unknown vector mode {mode}"),
    }
}

#[test]
fn original_python_core_vectors() {
    let corpus = pyjson::loads(include_str!("fixtures/python-0.2.0.json")).unwrap();
    let mut failures = Vec::new();
    for (i, row) in list(&corpus).iter().enumerate() {
        let got = evaluate(row);
        if !pyjson::py_eq(&got, row.at("outcome"), false) {
            failures.push(format!(
                "{i} {} {}\nRust {}\nPython {}",
                string(row.at("mode")),
                string(row.at("test")),
                pyjson::canonical(&got),
                pyjson::canonical(row.at("outcome"))
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} discrepancies\n{}",
        failures.len(),
        failures.iter().take(12).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn surrogate_map_values_survive_all_reachable_paths() {
    let document=pyjson::loads_plain(r#"{"schema_version":1,"entries":[{"value":"Jane \ud800Roe","placeholder":"PERSON_01","kind":"person","added":"2026-10-13"}]}"#).unwrap();
    let entries = entities::load(&document).unwrap();
    assert_eq!(entries[0].value.0[5], 0xD800);
    assert!(!redact::redact("Jane Roe", &entries).unwrap().unknowns.is_empty());
    assert!(verify::findings("PERSON_01", &entries).unwrap().is_empty());
    assert!(verify::restore("PERSON_01", &entries).is_err());
    let record = disclosure::create(b"PERSON_01", &entries, "model:sample", "sample:1").unwrap();
    assert!(disclosure::check(b"PERSON_01", &entries, &record, "model:sample", "sample:1").is_ok());
    assert!(pyjson::canonical(&entities::document(&entries)).contains(r"\ud800"));
}

#[test]
fn code_point_masks_map_back_to_original_byte_offsets() {
    let text = "12 Grüner Street\u{2028}Zoë Nguyen\ntfn: 1234567\n82. next";
    let mask = redact::Mask::new(text).blank("ADDRESS").unwrap();
    assert_eq!(mask.text.split('\u{2028}').next().unwrap().len(), 16);
    let name = patterns::person_names(&mask.text).unwrap();
    assert_eq!(
        &text[mask.original[name[0].start]..mask.original[name[0].end]],
        "Zoë Nguyen"
    );
    let found = redact::residual(text).unwrap();
    assert!(found.iter().any(|u| u.kind == "name" && u.line == 2));
    assert!(found.iter().any(|u| u.kind == "number" && u.line == 3));
    assert_eq!(pytext::nfc("Jose\u{301}"), "José");
}

#[test]
fn unicode_16_combining_sequences_and_contextual_casing_match_cpython() {
    let corpus = pyjson::loads(include_str!("fixtures/unicode-16.json")).unwrap();
    for (i, row) in list(corpus.at("cases")).iter().enumerate() {
        let row = list(row);
        let input = string(&row[0]);
        for (column, got) in [
            pytext::nfc(&input),
            pytext::lower(&input),
            pytext::upper(&input),
            pytext::casefold(&input),
        ]
        .iter()
        .enumerate()
        {
            assert_eq!(
                *got,
                string(&row[column + 1]),
                "case {i}, column {column}, input {input:?}"
            );
        }
    }
}
