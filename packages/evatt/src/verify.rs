//! The same detection as redact, never an independent completeness check.
use crate::{
    entities::{self, Entity},
    patterns::{self, Result},
    pytext, redact,
};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub kind: String,
    pub value: String,
}

fn mentions(text: &str, entity: &Entity) -> Result<bool> {
    if patterns::value_parts(&entity.value.os_text())?.is_empty() {
        return Ok(false);
    }
    entities::matcher(entity)?.is_match(text).map_err(patterns::runtime)
}

fn carried(text: &str, entries: &[Entity]) -> Result<Vec<Finding>> {
    let assigned: HashMap<_, _> = entries.iter().map(|e| (e.placeholder.as_str(), e)).collect();
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for m in patterns::matches("CARRIED_PLACEHOLDER", text)? {
        let t = m.name("token").unwrap().as_str();
        if !seen.insert(t) {
            continue;
        }
        if !["CLIENT", "PERSON", "STAFF", "ENTITY"].contains(&t.rsplit_once('_').unwrap().0) {
            continue;
        }
        if !assigned.contains_key(t) || mentions(text, assigned[t])? {
            found.push(Finding {
                kind: "placeholder".into(),
                value: t.into(),
            });
        }
    }
    Ok(found)
}

pub fn findings(text: &str, entries: &[Entity]) -> Result<Vec<Finding>> {
    let text = pytext::nfc(&text.replace("\r\n", "\n"));
    let mut found: Vec<_> = patterns::structured_spans(&text)?
        .iter()
        .map(|s| Finding {
            kind: s.kind.clone(),
            value: s.value.clone(),
        })
        .collect();
    for e in entries {
        if mentions(&text, e)? {
            found.push(Finding {
                kind: e.kind.clone(),
                value: e.value.to_utf8().unwrap(),
            });
        }
    }
    found.extend(carried(&text, entries)?);
    let sanitised = redact::redact(&text, entries)?.text;
    found.extend(redact::residual(&sanitised)?.iter().map(|u| Finding {
        kind: u.kind.clone(),
        value: u.value.clone(),
    }));
    Ok(found)
}

pub fn restore(text: &str, entries: &[Entity]) -> Result<String> {
    let mut text = text.to_string();
    for e in entries {
        if !patterns::full("PLACEHOLDER", &e.placeholder)?
            || !["CLIENT", "PERSON", "STAFF", "ENTITY"].contains(&e.placeholder.rsplit_once('_').unwrap().0)
            || patterns::has("PLACEHOLDER", &e.value.os_text())?
        {
            continue;
        }
        let expression = format!(
            "(?<![A-Za-z0-9_]){}(?![A-Za-z0-9_])",
            patterns::literal(&e.placeholder, false)
        );
        let regex = fancy_regex::Regex::new(&expression).map_err(patterns::runtime)?;
        let mut out = String::new();
        let mut cursor = 0;
        for m in regex.find_iter(&text) {
            let m = m.map_err(patterns::runtime)?;
            out.push_str(&text[cursor..m.start()]);
            out.push_str(
                &e.value
                    .to_utf8()
                    .ok_or_else(|| "'utf-8' codec can't encode lone surrogate in restored text".to_string())?,
            );
            cursor = m.end();
        }
        out.push_str(&text[cursor..]);
        text = out;
    }
    Ok(text)
}
