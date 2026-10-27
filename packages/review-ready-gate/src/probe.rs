//! Feature-gated behaviour oracle interface used only by migration checks.
pub use crate::pyjson::{Json, PyStr};
use crate::{
    Result,
    decimal::Decimal,
    loader::{self, SourceSnapshot},
    models::*,
    pyjson, pypath,
};
use std::{
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};

pub fn encode(value: &Json) -> String {
    pyjson::canonical(value)
}
pub fn decimal_digits() -> Vec<char> {
    crate::tables::DECIMAL_ZEROS
        .iter()
        .flat_map(|&zero| (zero..zero + 10).filter_map(char::from_u32))
        .collect()
}

fn text(value: &Json) -> Result<String> {
    value
        .as_pystr()
        .map(|s| s.to_utf8().unwrap_or_else(|| s.os_text()))
        .ok_or_else(|| "probe requires a string".into())
}
fn path(value: &Json) -> Result<PathBuf> {
    Ok(PathBuf::from(pypath::encode_os(&pypath::display(&text(
        value,
    )?))))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) || !value.is_ascii() {
        return Err("invalid probe hex".into());
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| {
            u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16)
                .map_err(|_| "invalid probe hex".into())
        })
        .collect()
}
fn snapshot(request: &Json) -> Result<SourceSnapshot> {
    Ok(SourceSnapshot::new(
        &path(request.at("path"))?,
        unhex(&text(request.at("hex"))?)?,
    ))
}
fn tb_json(r: TrialBalanceRow) -> Json {
    Json::object([
        ("report_date", Json::str(&r.report_date.iso())),
        ("tenant", Json::str(&r.tenant)),
        ("section", Json::str(&r.section)),
        ("account_id", Json::str(&r.account_id)),
        ("account_name", Json::str(&r.account_name)),
        ("account_code", Json::str(&r.account_code)),
        ("debit", Json::str(&r.debit.to_sci_string())),
        ("credit", Json::str(&r.credit.to_sci_string())),
        ("ytd_debit", Json::str(&r.ytd_debit.to_sci_string())),
        ("ytd_credit", Json::str(&r.ytd_credit.to_sci_string())),
    ])
}
fn csv(request: &Json) -> Result<Json> {
    let snapshot = snapshot(request)?;
    let kind = text(request.at("kind"))?;
    let rows = match kind.as_str() {
        "trial_balance" => loader::load_canonical_tb(&snapshot)?
            .into_iter()
            .map(tb_json)
            .collect(),
        "open_items" => loader::load_open_items(&snapshot)?
            .into_iter()
            .map(|r| {
                Json::object([
                    ("item_id", Json::str(&r.item_id)),
                    ("severity", Json::str(&r.severity)),
                    ("owner", Json::str(&r.owner)),
                    ("due_date", Json::str(&r.due_date.iso())),
                    ("status", Json::str(&r.status)),
                    ("description", Json::str(&r.description)),
                    ("resolution", Json::str(&r.resolution)),
                ])
            })
            .collect(),
        "activity_statement" => loader::load_activity_statement(&snapshot)?
            .into_iter()
            .map(|r| {
                Json::object([
                    ("label", Json::str(&r.label)),
                    ("amount", Json::str(&r.amount.to_sci_string())),
                ])
            })
            .collect(),
        "gst_control_gl" => loader::load_gst_control(&snapshot)?
            .into_iter()
            .map(|r| {
                Json::object([
                    ("posted_on", Json::str(&r.posted_on.iso())),
                    ("account_id", Json::str(&r.account_id)),
                    ("account_name", Json::str(&r.account_name)),
                    ("debit", Json::str(&r.debit.to_sci_string())),
                    ("credit", Json::str(&r.credit.to_sci_string())),
                    ("description", Json::str(&r.description)),
                ])
            })
            .collect(),
        "bank_rec" => loader::load_bank_rec(&snapshot)?
            .into_iter()
            .map(|r| {
                Json::object([
                    ("account_id", Json::str(&r.account_id)),
                    (
                        "statement_balance",
                        Json::str(&r.statement_balance.to_sci_string()),
                    ),
                    ("gl_balance", Json::str(&r.gl_balance.to_sci_string())),
                ])
            })
            .collect(),
        "tie_out_matrix" => loader::load_tie_out_matrix(&snapshot)?
            .into_iter()
            .map(|r| {
                Json::object([
                    ("statement_line", Json::str(&r.statement_line)),
                    ("workpaper_ref", Json::str(&r.workpaper_ref)),
                    ("source_file", Json::str(&r.source_file)),
                    ("status", Json::str(&r.status)),
                ])
            })
            .collect(),
        "prior_findings" => loader::load_prior_findings(&snapshot)?
            .into_iter()
            .map(|r| {
                Json::object([
                    ("finding_code", Json::str(&r.finding_code)),
                    ("slot", Json::str(&r.slot)),
                    ("status", Json::str(&r.status)),
                ])
            })
            .collect(),
        _ => return Err("unsupported probe CSV kind".into()),
    };
    Ok(Json::Array(rows))
}
fn operation(request: &Json) -> Result<Json> {
    match text(request.at("op"))?.as_str() {
        "cli" => {
            let Json::Array(args) = request.at("args") else {
                return Err("probe CLI arguments must be a list".into());
            };
            let args = args
                .iter()
                .map(|value| text(value).map(std::ffi::OsString::from))
                .collect::<Result<Vec<_>>>()?;
            let (code, out, err) = crate::stdio::capture(|| crate::cli::main(args));
            let mut result = vec![
                ("code", Json::int(code.into())),
                ("stdout", Json::str(&out)),
                ("stderr", Json::str(&err)),
            ];
            if let Some(output) = request.get("output_files") {
                let output = path(output)?;
                result.push((
                    "files",
                    Json::Array(
                        crate::report::PACK_NAMES
                            .iter()
                            .map(|name| {
                                fs::read(output.join(name))
                                    .map_or(Json::Null, |bytes| Json::str(&hex(&bytes)))
                            })
                            .collect(),
                    ),
                ));
            }
            Ok(Json::Object(
                result
                    .into_iter()
                    .map(|(key, value)| (PyStr::from(key), value))
                    .collect(),
            ))
        }
        "money" => {
            let value = if matches!(request.at("value"), Json::Null) {
                None
            } else {
                Some(text(request.at("value"))?)
            };
            Ok(Json::str(
                &loader::parse_money(value.as_deref(), "Amount", 2, Path::new("input.csv"))?
                    .to_sci_string(),
            ))
        }
        "date" => Ok(Json::str(
            &loader::parse_iso_date(
                &text(request.at("value"))?,
                "Date",
                Path::new("input.csv"),
                Some(2),
            )?
            .iso(),
        )),
        "csv" => csv(request),
        "parse_pack_json" => crate::viewer::parse_json(&unhex(&text(request.at("hex"))?)?),
        "output_guard" => Ok(Json::str(
            &crate::report::require_output_outside_repository(&path(request.at("path"))?)?
                .to_string_lossy(),
        )),
        "format_money" => {
            let value = Decimal::parse(&text(request.at("value"))?)
                .map_err(|_| "invalid probe decimal".to_owned())?;
            Ok(Json::str(&crate::report::money(&value)?))
        }
        "tb_integrity" => {
            let path = path(request.at("path"))?;
            let rows =
                loader::load_canonical_tb(&SourceSnapshot::capture(&path, "Trial-balance file")?)?;
            let (movement, ytd) = crate::engine::tb_balanced(&rows)?;
            Ok(Json::object([
                ("movement", Json::str(&movement.to_sci_string())),
                ("ytd", Json::str(&ytd.to_sci_string())),
                ("rows", Json::Array(rows.into_iter().map(tb_json).collect())),
            ]))
        }
        "review_note" => {
            let r =
                loader::load_reviewer_acknowledgement(Some(&path(request.at("path"))?))?.unwrap();
            Ok(Json::object([
                ("reviewer_initials", Json::str(&r.reviewer_initials)),
                ("reviewed_on", Json::str(&r.reviewed_on.iso())),
                ("comment", Json::str(&r.comment)),
            ]))
        }
        "self_review" => {
            let r = loader::load_self_review(&snapshot(request)?, &text(request.at("profile"))?)?;
            Ok(Json::object([
                ("preparer_initials", Json::str(&r.preparer_initials)),
                ("prepared_on", Json::str(&r.prepared_on.iso())),
                ("engagement_type", Json::str(&r.engagement_type)),
                ("period_end", Json::str(&r.period_end.iso())),
                (
                    "assertions",
                    Json::Object(
                        r.assertions
                            .iter()
                            .map(|(k, v)| (PyStr::from(k.as_str()), Json::Bool(*v)))
                            .collect(),
                    ),
                ),
            ]))
        }
        "review_pack" => {
            let dir = path(request.at("pack"))?;
            let profile = text(request.at("profile"))?;
            let tolerance = Decimal::parse(&text(request.at("tolerance"))?)
                .map_err(|_| "invalid probe tolerance".to_owned())?;
            let note = request.get("note").map(path).transpose()?;
            let documents = match request.get("documents") {
                Some(Json::Array(paths)) => paths.iter().map(path).collect::<Result<Vec<_>>>()?,
                _ => Vec::new(),
            };
            let pack =
                crate::engine::review_pack(&profile, &dir, note.as_deref(), tolerance, &documents)?;
            let rendered = crate::report::rendered(&pack)?;
            Ok(Json::object([
                ("pack", crate::report::as_json(&pack)?),
                (
                    "files",
                    Json::Array(
                        rendered
                            .iter()
                            .map(|bytes| Json::str(&hex(bytes)))
                            .collect(),
                    ),
                ),
            ]))
        }
        "document_value" => Ok(Json::str(&crate::documents::field_value(
            &text(request.at("field"))?,
            request.at("value"),
        )?)),
        "document_extract" => Ok(crate::documents::extract(&text(request.at("text"))?)),
        "view" => {
            let pack = crate::viewer::verify_pack(&path(request.at("pack"))?)?;
            Ok(pack.document)
        }
        "compare" => crate::comparison::compare_packs(
            &path(request.at("previous"))?,
            &path(request.at("current"))?,
        ),
        _ => Err("unsupported probe operation".into()),
    }
}
pub fn evaluate(request: &str) -> Result<String> {
    let request = pyjson::loads(request).map_err(|e| format!("invalid probe request: {e:?}"))?;
    let result = match operation(&request) {
        Ok(value) => Json::object([("ok", value)]),
        Err(e) => Json::object([("error", Json::str(&e))]),
    };
    Ok(pyjson::canonical(&result))
}
/// Set by the probe binary: the frozen Python baseline predates the pack's
/// `producer` block, so CLI runs compared with it record none.
pub static OMIT_PRODUCER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn main() -> i32 {
    OMIT_PRODUCER.store(true, std::sync::atomic::Ordering::Relaxed);
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let result = line
            .map_err(|e| e.to_string())
            .and_then(|line| evaluate(&line));
        let result = match result {
            Ok(value) => value,
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        };
        if writeln!(stdout, "{result}")
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return 1;
        }
    }
    0
}
pub fn equivalent(left: &str, right: &str) -> Result<bool> {
    let a = pyjson::loads(left).map_err(|e| format!("invalid oracle result: {e:?}"))?;
    let b = pyjson::loads(right).map_err(|e| format!("invalid native result: {e:?}"))?;
    Ok(pyjson::canonical(&a) == pyjson::canonical(&b))
}

pub fn decode(text: &str) -> Result<Json> {
    pyjson::loads(text).map_err(|e| format!("invalid fixture JSON: {e:?}"))
}

pub fn example_document(profile: &str, directory: &Path, tolerance: &str) -> Result<Json> {
    example_document_with_documents(profile, directory, tolerance, &[])
}
pub fn example_document_with_documents(
    profile: &str,
    directory: &Path,
    tolerance: &str,
    documents: &[PathBuf],
) -> Result<Json> {
    let tolerance =
        Decimal::parse(tolerance).map_err(|_| "invalid fixture tolerance".to_owned())?;
    crate::report::as_json(&crate::engine::review_pack(
        profile, directory, None, tolerance, documents,
    )?)
}

pub fn render_pack(document: &Json) -> Result<[Vec<u8>; 3]> {
    crate::viewer::verify_schema(document)?;
    let Json::Array(findings) = document.at("findings") else {
        unreachable!()
    };
    for item in findings {
        for field in crate::report::CSV_FIELDS {
            if item
                .at(field)
                .as_pystr()
                .and_then(|s| s.to_utf8())
                .is_none()
            {
                return Err("fixture finding fields must be strings".into());
            }
        }
    }
    let Json::Object(sources) = document.at("source_sha256") else {
        unreachable!()
    };
    let evidence = sources
        .iter()
        .map(|(slot, value)| SourceEvidence {
            slot: slot.to_utf8().unwrap(),
            filename: value.at("filename").as_pystr().unwrap().to_utf8().unwrap(),
            sha256: value.at("sha256").as_pystr().unwrap().to_utf8().unwrap(),
        })
        .collect();
    crate::report::rendered(&crate::viewer::canonical_pack(document, evidence)?)
}
