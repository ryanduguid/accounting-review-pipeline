//! Test-only JSON-lines probe. Never part of a default build.
use evatt::{
    patterns,
    pyjson::{self, Json},
    pytext,
};
use std::io::{self, BufRead};

fn offset(text: &str, byte: usize) -> Json {
    Json::int(text[..byte].chars().count() as i64)
}

fn span(text: &str, start: usize, end: usize, value: &str) -> Json {
    Json::Array(vec![offset(text, start), offset(text, end), Json::str(value)])
}

fn captures(text: &str, regex: &fancy_regex::Regex) -> patterns::Result<Json> {
    let mut result = Vec::new();
    for item in regex.captures_iter(text) {
        let m = item.map_err(patterns::runtime)?;
        let groups = (0..m.len())
            .map(|i| {
                m.get(i)
                    .map_or(Json::Null, |m| span(text, m.start(), m.end(), m.as_str()))
            })
            .collect();
        result.push(Json::Array(groups));
    }
    Ok(Json::Array(result))
}

fn patterns_json(text: &str) -> patterns::Result<Json> {
    // Patterns before arbitration, including rejected bare checksum candidates.
    let mut result = Vec::new();
    for (name, regex) in evatt::probe_patterns() {
        result.push((name.into(), captures(text, regex)?));
    }
    Ok(Json::Object(result))
}

fn detector(request: &Json) -> patterns::Result<Json> {
    let text = request.at("text").as_pystr().unwrap().to_utf8().unwrap();
    let mut result = vec![("patterns".into(), patterns_json(&text)?)];
    let structured = patterns::structured_spans(&text)?
        .iter()
        .map(|s| {
            Json::Array(vec![
                offset(&text, s.start),
                offset(&text, s.end),
                Json::str(&s.kind),
                Json::str(&s.value),
            ])
        })
        .collect();
    result.push(("structured".into(), Json::Array(structured)));
    let names = patterns::person_names(&text)?
        .iter()
        .map(|s| span(&text, s.start, s.end, &s.value))
        .collect();
    result.push(("names".into(), Json::Array(names)));
    add_attempts(&text, &mut result)?;
    let email = evatt::email::spans(&text)
        .iter()
        .map(|s| span(&text, s.start, s.end, &s.value))
        .collect();
    result.push(("email_scanner".into(), Json::Array(email)));
    let ambiguous = patterns::ambiguous(&text)?
        .iter()
        .map(|a| {
            Json::Array(vec![
                offset(&text, a.start),
                offset(&text, a.digit_start),
                offset(&text, a.end),
                Json::str(&a.value),
            ])
        })
        .collect();
    result.push(("ambiguous".into(), Json::Array(ambiguous)));
    result.push(("nfc".into(), Json::str(&pytext::nfc(&text))));
    add_values(request, &text, &mut result)?;
    add_pipeline(request, &text, &mut result)?;
    Ok(Json::Object(result))
}

type Pairs = Vec<(pyjson::PyStr, Json)>;
fn add_attempts(text: &str, result: &mut Pairs) -> patterns::Result<()> {
    let attempts = patterns::name_attempts(text)?
        .iter()
        .map(|&(s, e, r, n)| Json::Array(vec![offset(text, s), offset(text, e), Json::Bool(r), offset(text, n)]))
        .collect();
    result.push(("name_attempts".into(), Json::Array(attempts)));
    Ok(())
}
fn add_values(request: &Json, text: &str, result: &mut Pairs) -> patterns::Result<()> {
    if let Some(Json::Array(values)) = request.get("values") {
        let mut matches = Vec::new();
        let mut folds = Vec::new();
        for value in values {
            let pyvalue = value.as_pystr().unwrap();
            let value = pyvalue.os_text();
            let regex = if pyvalue.to_utf8().is_none() {
                fancy_regex::Regex::new("(?!)").unwrap()
            } else {
                patterns::value_pattern(&pyvalue.to_utf8().unwrap())?
            };
            // The original exposes jN lookahead captures; only open/body/close
            // affect decisions. Report those plus the complete match.
            matches.push(value_captures(text, &regex)?);
            folds.push(Json::Str(pyjson::PyStr::from_os_text(&patterns::fold(&value)?)));
        }
        result.push(("values".into(), Json::Array(matches)));
        result.push(("folds".into(), Json::Array(folds)));
    }
    Ok(())
}
fn add_pipeline(request: &Json, text: &str, result: &mut Pairs) -> patterns::Result<()> {
    let entries = entries(request)?;
    let redacted = evatt::redact::redact(text, &entries)?;
    let counts = Json::Object(
        redacted
            .counts
            .iter()
            .map(|(k, n)| (k.as_str().into(), Json::int(*n as i64)))
            .collect(),
    );
    result.push(("redact".into(), Json::Array(vec![Json::str(&redacted.text), counts])));
    result.push(("unknowns".into(), unknowns(&redacted.unknowns)));
    result.push(("residual".into(), unknowns(&evatt::redact::residual(text)?)));
    let numbers = evatt::redact::numbers(text)?
        .iter()
        .map(|s| span(text, s.start, s.end, &s.value))
        .collect();
    result.push(("numbers".into(), Json::Array(numbers)));
    let findings = evatt::verify::findings(text, &entries)?
        .iter()
        .map(|f| Json::Array(vec![Json::str(&f.kind), Json::str(&f.value)]))
        .collect();
    result.push(("findings".into(), Json::Array(findings)));
    Ok(())
}

fn unknowns(found: &[evatt::redact::Unknown]) -> Json {
    Json::Array(
        found
            .iter()
            .map(|u| {
                Json::Array(vec![
                    Json::str(&u.kind),
                    Json::str(&u.value),
                    Json::int(u.line as i64),
                    Json::str(&u.context),
                ])
            })
            .collect(),
    )
}

fn entries(request: &Json) -> patterns::Result<Vec<evatt::entities::Entity>> {
    let Some(Json::Array(entries)) = request.get("entries") else {
        return Ok(Vec::new());
    };
    entries
        .iter()
        .map(|e| {
            Ok(evatt::entities::Entity {
                value: e.at("value").as_pystr().unwrap().clone(),
                placeholder: e.at("placeholder").as_pystr().unwrap().to_utf8().unwrap(),
                kind: e.at("kind").as_pystr().unwrap().to_utf8().unwrap(),
                added: e.at("added").as_pystr().unwrap().to_utf8().unwrap(),
            })
        })
        .collect()
}

fn value_captures(text: &str, regex: &fancy_regex::Regex) -> patterns::Result<Json> {
    let mut result = Vec::new();
    for item in regex.captures_iter(text) {
        let m = item.map_err(patterns::runtime)?;
        let whole = m.get(0).unwrap();
        result.push(Json::Array(vec![
            span(text, whole.start(), whole.end(), whole.as_str()),
            Json::str(m.name("open").unwrap().as_str()),
            Json::str(m.name("close").unwrap().as_str()),
            Json::Array(
                (0..m.len().saturating_sub(3))
                    .map(|i| {
                        let j = m.name(&format!("j{i}")).unwrap();
                        span(text, j.start(), j.end(), j.as_str())
                    })
                    .collect(),
            ),
        ]));
    }
    Ok(Json::Array(result))
}

fn main() {
    for line in io::stdin().lock().lines() {
        let answer = pyjson::loads(&line.unwrap())
            .map_err(|e| format!("{e:?}"))
            .and_then(|request| {
                if request.get("mode").is_some_and(|m| m.is_str("benchmark")) {
                    evatt::benchmark::measure(&request)
                } else {
                    detector(&request)
                }
            });
        let answer = answer.unwrap_or_else(|error| Json::object([("error", Json::str(&error))]));
        println!("{}", pyjson::canonical(&answer));
    }
}
