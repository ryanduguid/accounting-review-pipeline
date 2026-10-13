//! Read-only entity map. No Python import API, assign, save or temporary writes.
use crate::{
    patterns::{self, Result},
    pyjson::{Json, PyStr},
    pytext,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    pub value: PyStr,
    pub placeholder: String,
    pub kind: String,
    pub added: String,
}

pub const PREFIXES: &[&str] = &[
    "CLIENT", "PERSON", "STAFF", "ENTITY", "TFN", "ABN", "ACN", "BSB", "MEDICARE", "EMAIL", "PHONE",
];

pub fn prefix(kind: &str) -> Option<&'static str> {
    match kind {
        "client" => Some("CLIENT"),
        "person" => Some("PERSON"),
        "staff" => Some("STAFF"),
        "entity" => Some("ENTITY"),
        _ => None,
    }
}

pub fn date_valid(year: u32, month: u32, day: u32) -> bool {
    if !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
        return false;
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=max).contains(&day)
}

fn exact_date(value: &Json) -> bool {
    let Some(text) = value.as_pystr().and_then(PyStr::to_utf8) else {
        return false;
    };
    if text.len() != 10 || text.as_bytes()[4] != b'-' || text.as_bytes()[7] != b'-' {
        return false;
    }
    if !text
        .bytes()
        .enumerate()
        .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return false;
    }
    date_valid(
        text[..4].parse().unwrap(),
        text[5..7].parse().unwrap(),
        text[8..].parse().unwrap(),
    )
}

pub fn repr(value: &Json) -> String {
    match value {
        Json::Null => "None".into(),
        Json::Bool(b) => if *b { "True" } else { "False" }.into(),
        Json::Str(s) => pytext::repr_code_points(&s.0),
        Json::Int(i) => i.to_string(),
        Json::Float(f) if f.is_nan() => "nan".into(),
        Json::Float(f) if f.is_infinite() => if *f > 0.0 { "inf" } else { "-inf" }.into(),
        Json::Float(f) => crate::pyjson::float_repr(*f),
        Json::Array(items) => format!("[{}]", items.iter().map(repr).collect::<Vec<_>>().join(", ")),
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

fn fields(entry: &Json) -> Result<(PyStr, String, String)> {
    let value = entry.at("value");
    let Some(value) = value.as_pystr().filter(|s| !pytext::strip(&s.os_text()).is_empty()) else {
        return Err("entity value must be a non-empty string".into());
    };
    let text = value.os_text();
    let quoted = pytext::repr_code_points(&value.0);
    if patterns::has("PLACEHOLDER_CI", &text)? {
        return Err(format!("entity value {quoted} is shaped like an assigned placeholder"));
    }
    if patterns::has("JOIN_EDGE", &text)? {
        return Err(format!(
            "entity value {quoted} starts or ends with whitespace or markup"
        ));
    }
    let parts = patterns::value_parts(&text)?;
    let upper: Vec<_> = parts.iter().map(|p| pytext::upper(p)).collect();
    if parts.is_empty()
        || (upper.iter().any(|p| PREFIXES.contains(&p.as_str()))
            && upper
                .iter()
                .all(|p| PREFIXES.contains(&p.as_str()) || p.chars().all(|c| pytext::is_digit(c as u32))))
    {
        return Err(format!("entity value {quoted} is made only of placeholder parts"));
    }
    let kind = entry.at("kind");
    let Some(kind_text) = kind.as_pystr().and_then(PyStr::to_utf8).filter(|k| prefix(k).is_some()) else {
        return Err(format!("unknown entity kind {}", repr(kind)));
    };
    if !exact_date(entry.at("added")) {
        return Err(format!(
            "entity added date must be YYYY-MM-DD, got {}",
            repr(entry.at("added"))
        ));
    }
    Ok((value.clone(), kind_text, patterns::fold(&text)?))
}

fn check_entry(entry: &Json, seen: &mut HashMap<String, PyStr>, placeholders: &mut HashSet<String>) -> Result<Entity> {
    let (value, kind, fold) = fields(entry)?;
    if let Some(previous) = seen.get(&fold) {
        let quoted = pytext::repr_code_points(&value.0);
        if previous == &value {
            return Err(format!("duplicate entity value {quoted}"));
        }
        return Err(format!(
            "entity value {quoted} is {} again once case, whitespace and composition are folded; pass two matches them as one value, so they cannot hold two placeholders",
            pytext::repr_code_points(&previous.0)
        ));
    }
    let placeholder = entry.at("placeholder");
    let text = placeholder.as_pystr().and_then(PyStr::to_utf8);
    let pattern = format!("MAP_{}", prefix(&kind).unwrap());
    let valid = text
        .as_deref()
        .map(|t| patterns::full(&pattern, t))
        .transpose()?
        .unwrap_or(false);
    if !valid {
        return Err(format!(
            "placeholder {} does not match kind {}",
            repr(placeholder),
            repr(entry.at("kind"))
        ));
    }
    let placeholder = text.unwrap();
    if !placeholders.insert(placeholder.clone()) {
        return Err(format!("duplicate placeholder {}", pytext::repr(&placeholder)));
    }
    seen.insert(fold, value.clone());
    Ok(Entity {
        value,
        placeholder,
        kind,
        added: entry.at("added").as_pystr().unwrap().to_utf8().unwrap(),
    })
}

pub fn load(document: &Json) -> Result<Vec<Entity>> {
    if !document.has_exact_keys(&["schema_version", "entries"]) {
        return Err("entity map must hold exactly schema_version and entries".into());
    }
    if document
        .at("schema_version")
        .as_plain_int()
        .is_none_or(|v| v != &1.into())
    {
        return Err(format!(
            "unsupported entity map schema {}",
            repr(document.at("schema_version"))
        ));
    }
    let Json::Array(entries) = document.at("entries") else {
        return Err("entity map entries must be a list".into());
    };
    let mut seen = HashMap::new();
    let mut placeholders = HashSet::new();
    let mut loaded = Vec::new();
    for entry in entries {
        if !entry.has_exact_keys(&["value", "placeholder", "kind", "added"]) {
            return Err("entity map entry must hold exactly ['added', 'kind', 'placeholder', 'value']".into());
        }
        loaded.push(check_entry(entry, &mut seen, &mut placeholders)?);
    }
    Ok(loaded)
}

pub fn document(entries: &[Entity]) -> Json {
    let entries = entries
        .iter()
        .map(|e| {
            Json::object([
                ("value", Json::Str(e.value.clone())),
                ("placeholder", Json::str(&e.placeholder)),
                ("kind", Json::str(&e.kind)),
                ("added", Json::str(&e.added)),
            ])
        })
        .collect();
    Json::object([("schema_version", Json::int(1)), ("entries", Json::Array(entries))])
}

pub fn matcher(entity: &Entity) -> Result<fancy_regex::Regex> {
    match entity.value.to_utf8() {
        Some(value) => patterns::value_pattern(&value),
        // A token holding a surrogate cannot match a UTF-8 input document.
        None => fancy_regex::Regex::new("(?!)").map_err(patterns::runtime),
    }
}
