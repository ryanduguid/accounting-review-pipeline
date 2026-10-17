//! Synthetic document bytes, transcription observations and bound human review.
use crate::{
    Result,
    decimal::Decimal,
    loader::{self, SourceSnapshot},
    models::{Finding, SourceEvidence},
    pyjson::{self, Json, LoadError, PyStr},
    pypath, pytext,
};
use num_traits::ToPrimitive;
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub const INTAKE_NAME: &str = "document-intake.json";
pub const REVIEW_NAME: &str = "document-review.json";
pub const TEXT_NAME: &str = "extracted.txt";
pub const FIELDS: [(&str, &str); 6] = [
    ("supplier", "Supplier"),
    ("invoice_number", "Invoice number"),
    ("invoice_date", "Invoice date"),
    ("currency", "Currency"),
    ("total", "Total"),
    ("gst_observed", "GST"),
];
pub const BOUNDARY: &str = "Synthetic supporting evidence only. Review confirms transcription, not tax treatment, GST entitlement, ledger completeness or accounting approval.";
const MIB: usize = 1024 * 1024;

fn string(value: &Json, label: &str, empty: bool) -> Result<String> {
    let invalid = || {
        format!(
            "{label} must be a {}string of at most 500 characters.",
            if empty { "possibly empty " } else { "" }
        )
    };
    let pystr = value.as_pystr().ok_or_else(invalid)?;
    let text = pystr.to_utf8().unwrap_or_else(|| pystr.os_text());
    if pystr.0.len() > 500 || (!empty && pytext::strip(&text).is_empty()) {
        return Err(invalid());
    }
    if pystr.0.iter().any(|cp| (0xd800..=0xdfff).contains(cp))
        || pytext::control_or_format(&text, false)
    {
        return Err(format!(
            "{label} contains a control or formatting character."
        ));
    }
    Ok(text)
}
fn capture(path: &Path, limit: usize) -> Result<SourceSnapshot> {
    let mut content = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(limit as u64 + 1).read_to_end(&mut content))
        .map_err(|e| format!("Cannot read document input {}: {e}.", pypath::text(path)))?;
    if content.is_empty() || content.len() > limit {
        return Err(format!(
            "{} must contain between 1 and {limit} bytes.",
            pypath::decode_os(path.file_name().unwrap_or_default())
        ));
    }
    Ok(SourceSnapshot::new(path, content))
}
fn member(root: &Path, name: &str, limit: usize) -> Result<SourceSnapshot> {
    let path = root.join(name);
    let resolved = pypath::resolve(&path, false)
        .map_err(|_| format!("Cannot resolve document input {name}."))?;
    let root = pypath::resolve(root, false)
        .map_err(|_| format!("Cannot resolve document input {name}."))?;
    if !resolved
        .parent()
        .is_some_and(|p| pypath::same_path(p, &root))
    {
        return Err(format!("{name} must stay inside its document bundle."));
    }
    capture(&path, limit)
}
fn json(source: &SourceSnapshot) -> Result<Json> {
    let name = pypath::decode_os(source.path.file_name().unwrap_or_default());
    let value = pyjson::loads(source.text("Document JSON", false)?).map_err(|e| match e {
        LoadError::DuplicateKey(key) => format!(
            "Duplicate document JSON member: {}.",
            pytext::repr_code_points(&key.0)
        ),
        _ => format!("{name} is not valid document JSON."),
    })?;
    if !matches!(value, Json::Object(_)) {
        return Err(format!("{name} must hold a JSON object."));
    }
    Ok(value)
}
pub fn encoded(value: &Json) -> Vec<u8> {
    (pyjson::pretty(value) + "\n").into_bytes()
}
fn at<'a>(value: &'a Json, key: &str) -> &'a Json {
    value.get(key).unwrap_or(&Json::Null)
}

fn invoice_money(raw: &str) -> bool {
    let raw = raw.strip_prefix('-').unwrap_or(raw);
    let mut parts = raw.split('.');
    let integer = parts.next().unwrap_or("");
    if integer.is_empty()
        || integer.len() > 18
        || !integer.bytes().all(|b| b.is_ascii_digit())
        || (integer != "0" && integer.starts_with('0'))
    {
        return false;
    }
    if let Some(frac) = parts.next()
        && (frac.is_empty() || frac.len() > 2 || !frac.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    parts.next().is_none()
}
pub(crate) fn field_value(field: &str, value: &Json) -> Result<String> {
    let raw = string(value, field, false)?;
    let raw = pytext::strip(&raw);
    if field == "invoice_date" {
        return Ok(loader::parse_iso_date(raw, field, Path::new(TEXT_NAME), None)?.iso());
    }
    if field == "currency" && raw != "AUD" {
        return Err("This first document template supports explicit AUD only.".into());
    }
    if ["total", "gst_observed"].contains(&field) {
        if !invoice_money(raw) {
            return Err(format!(
                "{field} requires a decimal string with at most two places."
            ));
        }
        return Decimal::parse(raw)
            .unwrap()
            .format_places(2)
            .ok_or_else(|| "decimal output exceeds available memory".into());
    }
    Ok(raw.into())
}
pub(crate) fn extract(text: &str) -> Json {
    let mut fields: Vec<(PyStr, Json)> = FIELDS
        .iter()
        .map(|(name, _)| (PyStr::from(*name), Json::Array(Vec::new())))
        .collect();
    for (page, content) in text.split('\u{c}').enumerate() {
        for (line, mut raw_line) in pytext::split_lines(content).into_iter().enumerate() {
            let hashes = raw_line.bytes().take_while(|&b| b == b'#').count();
            if (1..=6).contains(&hashes) && raw_line.as_bytes().get(hashes) == Some(&b' ') {
                raw_line = raw_line[hashes..].trim_start_matches(' ');
            }
            for (index, (field, label)) in FIELDS.iter().enumerate() {
                let Some(raw) = raw_line.strip_prefix(&format!("{label}:")) else {
                    continue;
                };
                let raw = pytext::strip(raw);
                let (value, error) = match field_value(field, &Json::str(raw)) {
                    Ok(v) => (Json::str(&v), String::new()),
                    Err(e) => (Json::Null, e),
                };
                let row = Json::object([
                    ("page", Json::int(page as i64 + 1)),
                    ("line", Json::int(line as i64 + 1)),
                    ("raw", Json::str(raw)),
                    ("value", value),
                    ("error", Json::str(&error)),
                ]);
                let Json::Array(rows) = &mut fields[index].1 else {
                    unreachable!()
                };
                rows.push(row);
            }
        }
    }
    Json::Object(fields)
}
fn manifest(
    source: &SourceSnapshot,
    text: &SourceSnapshot,
    entity: &str,
    period: &str,
    origin: &str,
    version: &str,
) -> Result<Json> {
    string(&Json::str(entity), "entity", false)?;
    loader::parse_iso_date(period, "period_end", Path::new(INTAKE_NAME), None)?;
    let extension = source
        .path
        .extension()
        .map(pypath::decode_os)
        .map(|s| pytext::lower(&s))
        .unwrap_or_default();
    if !["txt", "pdf"].contains(&extension.as_str()) {
        return Err("Original documents must be .txt or .pdf files.".into());
    }
    if !["manual", "pdf-inspector"].contains(&origin) {
        return Err("text_origin must be manual or pdf-inspector.".into());
    }
    string(&Json::str(version), "text_version", origin == "manual")?;
    if extension == "txt" && source.content != text.content {
        return Err("A text original and its extracted text must have identical bytes.".into());
    }
    let extracted = text.text("Extracted text", false)?;
    Ok(Json::object([
        ("schema_version", Json::str("document-intake.v1")),
        ("mode", Json::str("synthetic")),
        ("entity_ref", Json::str(entity)),
        ("period_end", Json::str(period)),
        ("extractor", Json::str("labelled-invoice.v1")),
        ("text_origin", Json::str(origin)),
        ("text_version", Json::str(version)),
        (
            "source",
            Json::object([
                ("filename", Json::str(&format!("original.{extension}"))),
                ("sha256", Json::str(&source.sha256)),
            ]),
        ),
        (
            "text",
            Json::object([
                ("filename", Json::str(TEXT_NAME)),
                ("sha256", Json::str(&text.sha256)),
            ]),
        ),
        ("fields", extract(extracted)),
    ]))
}

pub struct IntakeOptions<'a> {
    pub source: &'a Path,
    pub text: &'a Path,
    pub output: &'a Path,
    pub entity: &'a str,
    pub period: &'a str,
    pub origin: &'a str,
    pub version: &'a str,
}
pub fn create_intake(options: IntakeOptions<'_>) -> Result<std::path::PathBuf> {
    let output = crate::report::require_output_outside_repository(options.output)?;
    if output.exists() {
        return Err(
            "Document intake requires a new output directory; existing files stay intact.".into(),
        );
    }
    let source = capture(options.source, 20 * MIB)?;
    let text = capture(options.text, MIB)?;
    let manifest = manifest(
        &source,
        &text,
        options.entity,
        options.period,
        options.origin,
        options.version,
    )?;
    let bytes = encoded(&manifest);
    if bytes.len() > MIB {
        return Err("Extracted observations exceed the 1 MiB intake limit.".into());
    }
    let Json::Object(fields) = manifest.at("fields") else {
        unreachable!()
    };
    let review_fields = fields
        .iter()
        .map(|(key, rows)| {
            let Json::Array(rows) = rows else {
                unreachable!()
            };
            let one = rows.len() == 1;
            (
                key.clone(),
                Json::object([
                    (
                        "value",
                        if one {
                            rows[0].at("value").clone()
                        } else {
                            Json::Null
                        },
                    ),
                    (
                        "page",
                        if one {
                            rows[0].at("page").clone()
                        } else {
                            Json::Null
                        },
                    ),
                    ("note", Json::str("")),
                ]),
            )
        })
        .collect();
    let review = Json::object([
        ("schema_version", Json::str("document-review.v1")),
        ("intake_sha256", Json::str(&crate::sha256(&bytes))),
        ("reviewer_initials", Json::str("")),
        ("reviewed_on", Json::str("")),
        ("source_text_checked", Json::Bool(false)),
        ("fields", Json::Object(review_fields)),
    ]);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::create_dir(&output).map_err(|e| e.to_string())?;
    let filename = loader::json_text(manifest.at("source").at("filename")).unwrap();
    for (name, content) in [
        (filename.as_str(), source.content),
        (TEXT_NAME, text.content),
        (INTAKE_NAME, bytes),
        ("document-review.example.json", encoded(&review)),
    ] {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join(name))
            .and_then(|mut f| f.write_all(&content))
            .map_err(|e| e.to_string())?;
    }
    Ok(output)
}

#[derive(Clone, Debug)]
pub struct DocumentBundle {
    pub manifest: Json,
    pub snapshots: Vec<SourceSnapshot>,
    pub review: Option<Json>,
    pub issues: Vec<String>,
}
impl DocumentBundle {
    pub fn evidence(&self, index: usize) -> Vec<SourceEvidence> {
        self.snapshots
            .iter()
            .enumerate()
            .map(|(n, s)| SourceEvidence {
                slot: format!("document_{index:03}_{}", n + 1),
                filename: format!(
                    "document_{index:03}/{}",
                    pypath::decode_os(s.path.file_name().unwrap_or_default())
                ),
                sha256: s.sha256.clone(),
            })
            .collect()
    }
}
pub fn load_document(root: &Path) -> Result<DocumentBundle> {
    let intake = member(root, INTAKE_NAME, MIB)?;
    let manifest_value = json(&intake)?;
    let source_info = at(&manifest_value, "source");
    let filename = loader::json_text(at(source_info, "filename")).unwrap_or_default();
    if !matches!(source_info, Json::Object(_))
        || !["original.txt", "original.pdf"].contains(&filename.as_str())
    {
        return Err("Document source must name original.txt or original.pdf.".into());
    }
    let source = member(root, &filename, 20 * MIB)?;
    let text = member(root, TEXT_NAME, MIB)?;
    let entity = string(at(&manifest_value, "entity_ref"), "entity_ref", false)?;
    let period = string(at(&manifest_value, "period_end"), "period_end", false)?;
    let origin = string(at(&manifest_value, "text_origin"), "text_origin", false)?;
    let version = string(
        at(&manifest_value, "text_version"),
        "text_version",
        origin == "manual",
    )?;
    let expected = manifest(&source, &text, &entity, &period, &origin, &version)?;
    if encoded(&manifest_value) != encoded(&expected) {
        return Err("Document intake does not match its sources or supported schema.".into());
    }
    let mut snapshots = vec![intake, source, text];
    if !root.join(REVIEW_NAME).exists() {
        return Ok(DocumentBundle {
            manifest: manifest_value,
            snapshots,
            review: None,
            issues: vec!["Human transcription review is missing.".into()],
        });
    }
    let review_source = member(root, REVIEW_NAME, MIB)?;
    let review = json(&review_source)?;
    let issues = review_issues(
        &review,
        &snapshots[0].sha256,
        &manifest_value,
        &snapshots[2],
    )?;
    snapshots.push(review_source);
    Ok(DocumentBundle {
        manifest: manifest_value,
        snapshots,
        review: Some(review),
        issues,
    })
}
fn review_issues(
    review: &Json,
    digest: &str,
    manifest: &Json,
    text: &SourceSnapshot,
) -> Result<Vec<String>> {
    if !review.has_exact_keys(&[
        "schema_version",
        "intake_sha256",
        "reviewer_initials",
        "reviewed_on",
        "source_text_checked",
        "fields",
    ]) || !review.at("schema_version").is_str("document-review.v1")
    {
        return Err("Unsupported document review schema.".into());
    }
    if !review.at("intake_sha256").is_str(digest) {
        return Err("Document review is bound to different intake bytes.".into());
    }
    let initials = string(review.at("reviewer_initials"), "reviewer_initials", true)?;
    let reviewed = string(review.at("reviewed_on"), "reviewed_on", true)?;
    if !reviewed.is_empty() {
        loader::parse_iso_date(&reviewed, "reviewed_on", Path::new(REVIEW_NAME), None)?;
    }
    let Json::Bool(checked) = review.at("source_text_checked") else {
        return Err("source_text_checked must be a boolean.".into());
    };
    let mut issues = Vec::new();
    if pytext::strip(&initials).is_empty() || reviewed.is_empty() || !checked {
        issues.push(
            "Record the reviewer, date and confirmation against the original document.".into(),
        );
    }
    let decisions = review.at("fields");
    if !decisions.has_exact_keys(&FIELDS.map(|(name, _)| name)) {
        return Err("Document review must include every field exactly once.".into());
    }
    let pages = text.text("Extracted text", false)?.matches('\u{c}').count() + 1;
    let mut total = None;
    let mut gst = None;
    for (name, _) in FIELDS {
        let decision = decisions.at(name);
        if !decision.has_exact_keys(&["value", "page", "note"]) {
            return Err(format!("Invalid review shape for {name}."));
        }
        let note = string(decision.at("note"), &format!("{name}.note"), true)?;
        let page = decision
            .at("page")
            .as_plain_int()
            .and_then(ToPrimitive::to_usize)
            .filter(|&n| n >= 1 && n <= pages);
        if !matches!(decision.at("page"), Json::Null) && page.is_none() {
            return Err(format!(
                "{name}.page must identify a page in the extracted text."
            ));
        }
        if matches!(decision.at("value"), Json::Null) {
            issues.push(format!("{name}: unresolved."));
            continue;
        }
        let value = field_value(name, decision.at("value"))?;
        let page =
            page.ok_or_else(|| format!("{name}.page must identify a page in the extracted text."))?;
        let Json::Array(rows) = manifest.at("fields").at(name) else {
            unreachable!()
        };
        if (rows.len() != 1
            || !rows[0].at("value").is_str(&value)
            || rows[0]
                .at("page")
                .as_plain_int()
                .and_then(ToPrimitive::to_usize)
                != Some(page))
            && pytext::strip(&note).is_empty()
        {
            issues.push(format!(
                "{name}: explain the correction or choice between observations."
            ));
        }
        if name == "total" {
            total = Some(Decimal::parse(&value).unwrap());
        }
        if name == "gst_observed" {
            gst = Some(Decimal::parse(&value).unwrap());
        }
    }
    if let (Some(total), Some(gst)) = (total, gst)
        && (gst.copy_abs() > total.copy_abs()
            || (!total.is_zero() && !gst.is_zero() && total.is_signed() != gst.is_signed()))
    {
        issues.push("Observed GST has a different sign from the total or exceeds it.".into());
    }
    Ok(issues)
}

pub fn document_controls(
    bundles: &[DocumentBundle],
    entity: Option<&str>,
    period: Option<&str>,
) -> (Vec<Finding>, Vec<SourceEvidence>) {
    let mut findings = Vec::new();
    let mut evidence = Vec::new();
    let mut seen = BTreeSet::new();
    for (i, bundle) in bundles.iter().enumerate() {
        let slot = format!("document_{:03}", i + 1);
        evidence.extend(bundle.evidence(i + 1));
        if entity.is_some_and(|e| !bundle.manifest.at("entity_ref").is_str(e))
            || period.is_some_and(|p| !bundle.manifest.at("period_end").is_str(p))
        {
            findings.push(Finding::new(
                "DOCUMENT_CONTEXT_MISMATCH",
                "BLOCKED",
                &slot,
                "Document entity or review period differs from the pack.",
                "Check the original and attach evidence for this entity and period.",
            ));
        }
        if !bundle.issues.is_empty() {
            findings.push(Finding::new(
                "DOCUMENT_REVIEW_REQUIRED",
                "NOT_READY",
                &slot,
                bundle.issues.join(" "),
                "Resolve transcription against the original; keep tax treatment separate.",
            ));
        }
        let digest = loader::json_text(bundle.manifest.at("source").at("sha256")).unwrap();
        if !seen.insert(digest) {
            findings.push(Finding::new(
                "DOCUMENT_DUPLICATE",
                "NOT_READY",
                &slot,
                "These original bytes already occur in the supplied documents.",
                "Review the duplicate attachment; no document was deleted or merged.",
            ));
        }
    }
    (findings, evidence)
}
pub fn render_document(bundle: &DocumentBundle) -> String {
    let m = &bundle.manifest;
    let mut lines = vec![
        "Document evidence".into(),
        BOUNDARY.into(),
        format!("Entity: {}", pyjson::dumps(m.at("entity_ref"), false)),
        format!(
            "Review period: {}",
            loader::json_text(m.at("period_end")).unwrap()
        ),
        format!(
            "Text extraction: {}",
            pyjson::dumps(
                &Json::Array(vec![
                    m.at("text_origin").clone(),
                    m.at("text_version").clone()
                ]),
                false
            )
        ),
    ];
    if let Some(r) = &bundle.review {
        lines.extend([
            format!(
                "Transcription reviewer: {}",
                pyjson::dumps(r.at("reviewer_initials"), false)
            ),
            format!("Reviewed on: {}", pyjson::dumps(r.at("reviewed_on"), false)),
            format!(
                "Original checked: {}",
                pyjson::dumps(r.at("source_text_checked"), false)
            ),
        ]);
    }
    for source in &bundle.snapshots {
        lines.push(format!(
            "{}: SHA-256 {}",
            pypath::decode_os(source.path.file_name().unwrap_or_default()),
            source.sha256
        ));
    }
    for (name, label) in FIELDS {
        lines.push(format!("{label}:"));
        let Json::Array(rows) = m.at("fields").at(name) else {
            unreachable!()
        };
        for row in rows {
            lines.push(format!(
                "  observed page {}, line {}: {}",
                pyjson::dumps(row.at("page"), false),
                pyjson::dumps(row.at("line"), false),
                pyjson::dumps(row.at("raw"), false)
            ));
            lines.push(format!(
                "  proposed: {}",
                pyjson::dumps(row.at("value"), false)
            ));
            if !row.at("error").is_str("") {
                lines.push(format!(
                    "  validation: {}",
                    pyjson::dumps(row.at("error"), false)
                ));
            }
        }
        if rows.is_empty() {
            lines.push("  no observation".into());
        }
        if let Some(r) = &bundle.review {
            lines.push(format!(
                "  review: {}",
                pyjson::dumps(r.at("fields").at(name), true)
            ));
        }
    }
    lines.extend(bundle.issues.iter().map(|s| format!("Unresolved: {s}")));
    if bundle.issues.is_empty() {
        lines.push(format!("Transcription review recorded. {BOUNDARY}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{self, TempDir};
    fn bundle(tmp: &TempDir, text: &str, entity: &str, suffix: &str) -> std::path::PathBuf {
        let source = tmp.join(format!("source.{suffix}"));
        let extracted = tmp.join("text.txt");
        fs::write(
            &source,
            if suffix == "pdf" {
                b"%PDF-fabricated".as_slice()
            } else {
                text.as_bytes()
            },
        )
        .unwrap();
        fs::write(&extracted, text).unwrap();
        create_intake(IntakeOptions {
            source: &source,
            text: &extracted,
            output: &tmp.join("bundle"),
            entity,
            period: "2026-03-31",
            origin: "manual",
            version: "",
        })
        .unwrap()
    }
    fn set(value: &mut Json, keys: &[&str], replacement: Json) {
        let Json::Object(pairs) = value else {
            panic!("test fixture object")
        };
        if keys.len() == 1 {
            if let Some((_, value)) = pairs.iter_mut().find(|(key, _)| key.is(keys[0])) {
                *value = replacement;
            } else {
                pairs.push((PyStr::from(keys[0]), replacement));
            }
        } else {
            set(
                &mut pairs.iter_mut().find(|(key, _)| key.is(keys[0])).unwrap().1,
                &keys[1..],
                replacement,
            );
        }
    }
    fn reviewed(path: &Path) -> Json {
        let mut value = json(
            &SourceSnapshot::capture(&path.join("document-review.example.json"), "review").unwrap(),
        )
        .unwrap();
        set(&mut value, &["reviewer_initials"], Json::str("XY"));
        set(&mut value, &["reviewed_on"], Json::str("2026-04-10"));
        set(&mut value, &["source_text_checked"], Json::Bool(true));
        fs::write(path.join(REVIEW_NAME), encoded(&value)).unwrap();
        value
    }
    fn source_text() -> String {
        fs::read_to_string(test_support::examples().join("document-intake/invoice.txt")).unwrap()
    }

    #[test]
    fn all_fifteen_fabricated_transcription_benchmarks() {
        let benchmark = SourceSnapshot::capture(
            &test_support::examples().join("document-intake/benchmark.json"),
            "benchmark",
        )
        .unwrap();
        let benchmark = json(&benchmark).unwrap();
        let Json::Array(cases) = benchmark.at("cases") else {
            panic!("benchmark list")
        };
        assert_eq!(cases.len(), 15);
        for case in cases {
            let tmp = TempDir::new();
            let mut text = source_text();
            let Json::Array(replacements) = case.at("replace") else {
                panic!("replace list")
            };
            for replacement in replacements {
                let Json::Array(pair) = replacement else {
                    panic!("replacement pair")
                };
                text = text.replace(
                    &loader::json_text(&pair[0]).unwrap(),
                    &loader::json_text(&pair[1]).unwrap(),
                );
            }
            text.push_str(&loader::json_text(case.at("append")).unwrap());
            let path = bundle(&tmp, &text, "Cedar and Pine Consulting Pty Ltd", "txt");
            let draft = json(
                &SourceSnapshot::capture(&path.join("document-review.example.json"), "draft")
                    .unwrap(),
            )
            .unwrap();
            let Json::Object(expected) = case.at("expected") else {
                panic!("expected object")
            };
            for (field, value) in expected {
                assert!(
                    pyjson::py_eq(
                        draft
                            .at("fields")
                            .get(&field.os_text())
                            .unwrap()
                            .at("value"),
                        value,
                        false
                    ),
                    "{}:{field:?}",
                    pyjson::canonical(case.at("id"))
                );
            }
            let mut unresolved: Vec<_> = FIELDS
                .iter()
                .filter(|(name, _)| matches!(draft.at("fields").at(name).at("value"), Json::Null))
                .map(|(name, _)| (*name).to_owned())
                .collect();
            unresolved.sort();
            let Json::Array(expected) = case.at("unresolved") else {
                panic!("unresolved list")
            };
            let mut expected: Vec<_> = expected
                .iter()
                .map(|v| loader::json_text(v).unwrap())
                .collect();
            expected.sort();
            assert_eq!(unresolved, expected);
            if let Some(page) = case.get("total_page") {
                assert!(pyjson::py_eq(
                    draft.at("fields").at("total").at("page"),
                    page,
                    false
                ));
            }
            assert_eq!(
                load_document(&path).unwrap().issues,
                ["Human transcription review is missing."]
            );
            reviewed(&path);
            let loaded = load_document(&path).unwrap();
            assert_eq!(
                loaded.issues.is_empty(),
                unresolved.is_empty(),
                "{}",
                pyjson::canonical(case.at("id"))
            );
        }
    }
    #[test]
    fn corrections_require_a_reason_and_keep_original_observations() {
        let tmp = TempDir::new();
        let path = bundle(
            &tmp,
            &source_text().replace("110.00", "unreadable"),
            "Demo",
            "txt",
        );
        let mut review = reviewed(&path);
        set(
            &mut review,
            &["fields", "total", "value"],
            Json::str("110.00"),
        );
        set(&mut review, &["fields", "total", "page"], Json::int(1));
        fs::write(path.join(REVIEW_NAME), encoded(&review)).unwrap();
        assert!(
            load_document(&path)
                .unwrap()
                .issues
                .iter()
                .any(|s| s.contains("explain the correction"))
        );
        set(
            &mut review,
            &["fields", "total", "note"],
            Json::str("Read 110.00 on the original page."),
        );
        fs::write(path.join(REVIEW_NAME), encoded(&review)).unwrap();
        let loaded = load_document(&path).unwrap();
        assert!(loaded.issues.is_empty());
        let Json::Array(rows) = loaded.manifest.at("fields").at("total") else {
            unreachable!()
        };
        assert!(rows[0].at("raw").is_str("unreadable"));
        let sheet = render_document(&loaded);
        assert!(sheet.contains("\"110.00\""));
        assert!(sheet.contains("\"unreadable\""));
    }
    #[test]
    fn malformed_human_review_values_and_pages_fail_closed() {
        for (keys, value, expected) in [
            (
                vec!["intake_sha256"],
                Json::str(&"0".repeat(64)),
                "different intake bytes",
            ),
            (vec!["source_text_checked"], Json::str("true"), "boolean"),
            (vec!["reviewed_on"], Json::str("01/02/2026"), "ISO date"),
            (
                vec!["fields", "total", "value"],
                Json::str("1e2"),
                "decimal string",
            ),
            (vec!["fields", "total", "value"], Json::Bool(true), "string"),
            (
                vec!["fields", "total", "value"],
                Json::Float(110.0),
                "string",
            ),
            (
                vec!["fields", "total", "page"],
                Json::Bool(true),
                "identify a page",
            ),
            (
                vec!["fields", "total", "page"],
                Json::int(2),
                "identify a page",
            ),
            (
                vec!["fields", "total", "page"],
                Json::Null,
                "identify a page",
            ),
            (
                vec!["fields", "total", "approved"],
                Json::Bool(true),
                "review shape",
            ),
            (vec!["approved"], Json::Bool(true), "review schema"),
            (
                vec!["reviewer_initials"],
                Json::str("XY\u{1b}[2J"),
                "control",
            ),
        ] {
            let tmp = TempDir::new();
            let path = bundle(&tmp, &source_text(), "Demo", "txt");
            let mut review = reviewed(&path);
            set(&mut review, &keys, value);
            fs::write(path.join(REVIEW_NAME), encoded(&review)).unwrap();
            assert!(
                load_document(&path).unwrap_err().contains(expected),
                "{keys:?}"
            );
        }
    }
    #[test]
    fn document_bytes_context_and_duplicate_occurrences_are_bound_to_the_pack() {
        let tmp = TempDir::new();
        let path = bundle(
            &tmp,
            &source_text(),
            "Cedar and Pine Consulting Pty Ltd",
            "txt",
        );
        reviewed(&path);
        let docs = vec![load_document(&path).unwrap(), load_document(&path).unwrap()];
        let (findings, evidence) =
            document_controls(&docs, Some("Other entity"), Some("2026-06-30"));
        assert_eq!(
            findings.iter().map(|f| f.code.as_str()).collect::<Vec<_>>(),
            [
                "DOCUMENT_CONTEXT_MISMATCH",
                "DOCUMENT_CONTEXT_MISMATCH",
                "DOCUMENT_DUPLICATE"
            ]
        );
        assert_eq!(evidence.len(), 8);
        let (findings, _) = document_controls(&docs[..1], None, None);
        assert!(findings.is_empty());
        let mut pack = test_support::ready_pack();
        let (findings, evidence) = document_controls(
            &docs[..1],
            Some("Cedar and Pine Consulting Pty Ltd"),
            Some("2026-03-31"),
        );
        pack.findings.extend(findings);
        pack.source_evidence.extend(evidence);
        let output = tmp.join("pack");
        crate::report::write_review_pack(&pack, &output).unwrap();
        assert!(crate::viewer::render_review_sheet(&output, std::slice::from_ref(&path)).is_ok());
        let mut review = reviewed(&path);
        set(&mut review, &["reviewer_initials"], Json::str("YZ"));
        fs::write(path.join(REVIEW_NAME), encoded(&review)).unwrap();
        assert!(
            crate::viewer::render_review_sheet(&output, &[path])
                .unwrap_err()
                .contains("Document bytes or order")
        );
    }
    #[test]
    fn original_text_intake_and_review_tampering_are_refused() {
        for filename in ["original.txt", TEXT_NAME, INTAKE_NAME] {
            let tmp = TempDir::new();
            let path = bundle(&tmp, &source_text(), "Demo", "txt");
            reviewed(&path);
            let text = fs::read_to_string(path.join(filename))
                .unwrap()
                .replace("110.00", "120.00");
            fs::write(path.join(filename), text).unwrap();
            assert!(load_document(&path).is_err());
        }
        let tmp = TempDir::new();
        let path = bundle(&tmp, &source_text(), "Demo", "pdf");
        reviewed(&path);
        let loaded = load_document(&path).unwrap();
        assert!(loaded.issues.is_empty());
        assert!(
            loaded
                .manifest
                .at("source")
                .at("filename")
                .is_str("original.pdf")
        );
    }
    #[test]
    fn intake_is_exclusive_and_original_sources_are_never_overwritten() {
        let tmp = TempDir::new();
        let path = bundle(&tmp, &source_text(), "Demo", "txt");
        let original = fs::read(tmp.join("source.txt")).unwrap();
        let options = IntakeOptions {
            source: &tmp.join("source.txt"),
            text: &tmp.join("text.txt"),
            output: &path,
            entity: "Demo",
            period: "2026-03-31",
            origin: "manual",
            version: "",
        };
        assert!(
            create_intake(options)
                .unwrap_err()
                .contains("new output directory")
        );
        assert_eq!(fs::read(tmp.join("source.txt")).unwrap(), original);
    }
}
