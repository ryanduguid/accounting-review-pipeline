//! Validate supplied calculation records from disk without calling a calculator.
use crate::{
    Result,
    decimal::Decimal,
    isodate::Date,
    loader::SourceSnapshot,
    pyjson::{self, Json, LoadError, PyStr},
    pypath, pytext, sha256,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const SCHEMA: &str = "lodgeit-calculation-evidence/1";
const MAX_BYTES: usize = 2 * 1024 * 1024;
static NULL: Json = Json::Null;

#[derive(Clone, Debug)]
pub struct CalculationEvidence {
    pub label: String,
    pub path: PathBuf,
    pub sha256: String,
    pub schema: String,
    pub provider: String,
    pub calculator: String,
    pub period: String,
    pub status: String,
    pub engine: String,
    pub calculation_sha256: String,
    pub values: BTreeMap<String, Decimal>,
    pub rate_tables: Vec<String>,
    pub advisory_notes: Vec<String>,
    pub synthetic_input: bool,
    pub findings: Vec<String>,
}

impl CalculationEvidence {
    pub fn usable(&self) -> bool {
        self.findings.is_empty() && self.status == "COMPUTED"
    }
}

fn field<'a>(value: &'a Json, key: &str) -> &'a Json {
    value.get(key).unwrap_or(&NULL)
}
fn block<'a>(value: &'a Json, key: &str) -> &'a Json {
    match value.get(key) {
        Some(value @ Json::Object(_)) => value,
        _ => &NULL,
    }
}
fn type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "NoneType",
        Json::Bool(_) => "bool",
        Json::Int(_) => "int",
        Json::Float(_) => "float",
        Json::Str(_) => "str",
        Json::Array(_) => "list",
        Json::Object(_) => "dict",
    }
}
fn truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(value) => *value,
        Json::Int(value) => value != &num_bigint::BigInt::from(0),
        Json::Float(value) => *value != 0.0,
        Json::Str(value) => !value.0.is_empty(),
        Json::Array(value) => !value.is_empty(),
        Json::Object(value) => !value.is_empty(),
    }
}
fn text(value: &Json, name: &str, path: &Path, limit: usize) -> Result<String> {
    let path = pypath::text(path);
    let value = value
        .as_pystr()
        .ok_or_else(|| format!("{path}: {name} must be a string."))?;
    if value.0.len() > limit {
        return Err(format!("{path}: {name} is longer than {limit} characters."));
    }
    if value
        .0
        .iter()
        .any(|&cp| matches!(cp, 0x2028 | 0x2029) || pytext::control_or_format_point(cp, false))
    {
        return Err(format!(
            "{path}: {name} contains a control or formatting character."
        ));
    }
    Ok(value.to_utf8().expect("validated evidence text"))
}
fn or_empty_text(value: &Json, name: &str, path: &Path, limit: usize) -> Result<String> {
    if truthy(value) {
        text(value, name, path, limit)
    } else {
        Ok(String::new())
    }
}
pub fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}
fn array<'a>(value: &'a Json, name: &str, path: &Path) -> Result<&'a [Json]> {
    match value {
        Json::Null => Ok(&[]),
        Json::Array(items) => Ok(items),
        _ => Err(format!(
            "{}: {name} is {}, not an array.",
            pypath::text(path),
            type_name(value)
        )),
    }
}
fn decimal(value: &Json, name: &str, path: &Path) -> Result<Decimal> {
    let path = pypath::text(path);
    let Some(value) = value.as_pystr() else {
        return Err(format!(
            "{path}: {name} is {}, not a decimal string. A JSON number has already lost whatever the calculator meant by it.",
            type_name(value)
        ));
    };
    if value.0.is_empty()
        || value
            .0
            .iter()
            .any(|&cp| !(cp <= 127 && (cp as u8).is_ascii_digit() || matches!(cp, 45 | 46)))
    {
        return Err(format!("{path}: {name} is not a decimal string."));
    }
    Decimal::parse(&value.to_utf8().unwrap())
        .map_err(|_| format!("{path}: {name} is not a decimal."))
}

pub fn load(snapshot: &SourceSnapshot) -> Result<CalculationEvidence> {
    let source = &snapshot.path;
    let path = pypath::text(source);
    if snapshot.content.len() > MAX_BYTES {
        return Err(format!("{path}: evidence file exceeds {MAX_BYTES} bytes."));
    }
    let record =
        pyjson::loads(snapshot.text("Calculation-evidence file", true)?).map_err(|error| {
            match error {
                LoadError::Invalid => format!("{path}: evidence is not valid JSON."),
                LoadError::DuplicateKey(key) => format!(
                    "{path}: evidence could not be read (JSON member {} appears more than once.).",
                    pytext::repr_code_points(&key.0)
                ),
                LoadError::Crash(reason)
                    if reason.contains("recursion") || reason.contains("nested") =>
                {
                    format!("{path}: evidence is nested too deeply to read.")
                }
                LoadError::Crash(reason) => {
                    format!("{path}: evidence could not be read ({reason}).")
                }
            }
        })?;
    if !matches!(record, Json::Object(_)) {
        return Err(format!("{path}: evidence must be a JSON object."));
    }
    let schema = text(field(&record, "schema"), "schema", source, 120)?;
    if schema != SCHEMA {
        return Err(format!(
            "{path}: evidence schema {} is not one this pipeline reads ({SCHEMA}). An unknown schema is refused rather than guessed at.",
            pytext::repr(&schema)
        ));
    }
    let calculation = field(&record, "calculation");
    if !matches!(calculation, Json::Object(_)) {
        return Err(format!("{path}: evidence carries no calculation block."));
    }
    let synthetic_input = match calculation.get("synthetic_input") {
        None => false,
        Some(Json::Bool(value)) => *value,
        _ => return Err(format!("{path}: synthetic_input must be a boolean.")),
    };
    let recorded_digest = text(
        field(&record, "calculation_sha256"),
        "calculation_sha256",
        source,
        64,
    )?;
    let actual_digest = sha256(
        pyjson::canonical_utf8(calculation)
            .map_err(|_| format!("{path}: the calculation block cannot be canonicalised."))?
            .as_bytes(),
    );
    let mut findings = Vec::new();
    if actual_digest != recorded_digest {
        findings.push(format!("calculation_sha256 {recorded_digest} does not match the calculation block ({actual_digest}). The file has been changed since it was produced."));
    }
    let call = block(calculation, "call");
    let provider = block(calculation, "provider");
    let upstream = block(calculation, "upstream");
    let engine = block(calculation, "engine");
    let normalised = block(calculation, "normalised");
    let default_label = Json::Str(PyStr::from_os_text(
        &source
            .file_stem()
            .map(pypath::decode_os)
            .unwrap_or_default(),
    ));
    let label = text(
        calculation.get("label").unwrap_or(&default_label),
        "label",
        source,
        120,
    )?;
    if !valid_label(&label) {
        return Err(format!(
            "{path}: label {} is not a slug. A label names a required calculation and keys a digest in the pack, so it is lower-case letters, digits and single hyphens.",
            pytext::repr(&label)
        ));
    }
    let status = text(
        call.get("status").unwrap_or(&Json::str("UNKNOWN")),
        "call.status",
        source,
        60,
    )?;
    let calculator = or_empty_text(field(call, "calculator"), "call.calculator", source, 200)?;
    let period = or_empty_text(field(call, "period"), "call.period", source, 200)?;
    let provider_name = or_empty_text(field(provider, "name"), "provider.name", source, 120)?;
    let engine_name = if truthy(field(engine, "name")) {
        field(engine, "name")
    } else {
        field(provider, "contract_snapshot")
    };
    let engine_name = or_empty_text(engine_name, "engine", source, 200)?;
    let manifest = field(upstream, "manifest");
    let advisory = field(upstream, "advisory");
    if status == "COMPUTED" && (!matches!(manifest, Json::Object(_)) || !truthy(manifest)) {
        findings.push("the evidence records a computed figure with no manifest, so nothing names the rate tables it consumed".into());
    }
    let mut rate_tables = Vec::new();
    if matches!(manifest, Json::Object(_)) {
        for (index, entry) in array(
            field(manifest, "rate_table_uris"),
            "manifest.rate_table_uris",
            source,
        )?
        .iter()
        .enumerate()
        {
            let name = format!("manifest.rate_table_uris[{index}]");
            if !matches!(entry, Json::Object(_)) {
                findings.push(format!("the manifest entry at {name} is {}, not an object, so nothing there names a rate table", type_name(entry)));
                continue;
            }
            if field(entry, "uri").as_pystr().is_none() {
                findings.push(format!("the manifest entry at {name} carries no uri string, so nothing there names a rate table"));
                continue;
            }
            rate_tables.push(text(
                field(entry, "uri"),
                &format!("{name}.uri"),
                source,
                400,
            )?);
        }
    }
    let mut advisory_notes = Vec::new();
    if matches!(advisory, Json::Object(_)) {
        for (index, note) in array(field(advisory, "notes"), "advisory.notes", source)?
            .iter()
            .enumerate()
        {
            if note.as_pystr().is_none() {
                findings.push(format!("the advisory note at advisory.notes[{index}] is {}, not text, so the calculator's boundary statement is not readable", type_name(note)));
                continue;
            }
            advisory_notes.push(text(
                note,
                &format!("advisory.notes[{index}]"),
                source,
                600,
            )?);
        }
    }
    if status == "COMPUTED"
        && !advisory_notes
            .iter()
            .any(|note| !pytext::strip(note).is_empty())
    {
        findings.push("the evidence records a computed figure with no advisory, so the calculator's own boundary statement is missing".into());
    }
    let mut values = BTreeMap::new();
    if let Json::Object(pairs) = block(normalised, "values") {
        let mut pairs: Vec<_> = pairs.iter().collect();
        pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
        for (name, amount) in pairs {
            let name = text(
                &Json::Str(name.clone()),
                "normalised.values key",
                source,
                80,
            )?;
            let amount = decimal(amount, &format!("normalised.values.{name}"), source)?;
            values.insert(name, amount);
        }
    }
    if status == "COMPUTED" && values.is_empty() {
        findings.push("the evidence records a computed figure and carries no normalised value, so there is no figure in it. A label alone does not satisfy a required calculation.".into());
    }
    let validation = block(calculation, "validation");
    if let Some(accepted) = validation.get("accepted") {
        match accepted {
            Json::Bool(false) if status == "COMPUTED" => findings.push(
                "the producer records a COMPUTED result with validation.accepted=false".into(),
            ),
            Json::Bool(_) => {}
            _ => return Err(format!("{path}: validation.accepted must be a boolean.")),
        }
    }
    if truthy(validation) {
        for (index, finding) in array(field(validation, "findings"), "validation.findings", source)?
            .iter()
            .enumerate()
        {
            let name = format!("validation.findings[{index}]");
            if finding.as_pystr().is_none() {
                findings.push(format!("the producer recorded a validation finding at {name} that is {}, not text, so what it found cannot be read", type_name(finding)));
                continue;
            }
            findings.push(format!(
                "the producer recorded a validation finding: {}",
                text(finding, &name, source, 600)?
            ));
        }
    }
    Ok(CalculationEvidence {
        label,
        path: source.clone(),
        sha256: snapshot.sha256.clone(),
        schema,
        provider: provider_name,
        calculator,
        period,
        status,
        engine: engine_name,
        calculation_sha256: recorded_digest,
        values,
        rate_tables,
        advisory_notes,
        synthetic_input,
        findings,
    })
}

pub fn covers_period(evidence: &CalculationEvidence, report_date: Date) -> Option<bool> {
    let tail = evidence.period.rsplit(':').next().unwrap_or("");
    let bytes = tail.as_bytes();
    if bytes.len() == 7
        && bytes[4] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || b.is_ascii_digit())
    {
        let month = Date::new(tail[..4].parse().ok()?, tail[5..].parse().ok()?, 1)?;
        return Some((month.year, month.month) == (report_date.year, report_date.month));
    }
    if bytes.len() == 6 && tail.starts_with("fy") && bytes[2..].iter().all(u8::is_ascii_digit) {
        let ending: i32 = tail[2..].parse().ok()?;
        let (start_month, end_month, end_day) = if evidence.period.contains("fbt") {
            (4, 3, 31)
        } else {
            (7, 6, 30)
        };
        let start = Date::new(ending - 1, start_month, 1)?;
        let end = Date::new(ending, end_month, end_day)?;
        return Some(start <= report_date && report_date <= end);
    }
    None
}

pub fn load_all(paths: &[PathBuf]) -> (Vec<CalculationEvidence>, Vec<String>) {
    load_all_with(paths, &mut SourceSnapshot::capture)
}

pub(crate) fn load_all_with(
    paths: &[PathBuf],
    capture: &mut impl FnMut(&Path, &str) -> Result<SourceSnapshot>,
) -> (Vec<CalculationEvidence>, Vec<String>) {
    let mut loaded = Vec::new();
    let mut unreadable = Vec::new();
    let mut seen = BTreeSet::new();
    for path in paths {
        let item = capture(path, "Calculation-evidence file").and_then(|source| load(&source));
        match item {
            Err(error) => unreadable.push(error),
            Ok(item) if !seen.insert(item.label.clone()) => unreadable.push(format!("{}: more than one evidence file carries the label {}; a required calculation cannot be matched to two files.", pypath::text(path), pytext::repr(&item.label))),
            Ok(item) => loaded.push(item),
        }
    }
    (loaded, unreadable)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn size_limit_is_checked_before_json_and_utf8_decoding() {
        let path = Path::new("evidence.json");
        let boundary = load(&SourceSnapshot::new(path, vec![b' '; MAX_BYTES])).unwrap_err();
        assert!(boundary.contains("not valid JSON"));
        let oversized = load(&SourceSnapshot::new(path, vec![0xff; MAX_BYTES + 1])).unwrap_err();
        assert!(oversized.contains("evidence file exceeds 2097152 bytes"));
    }
}
