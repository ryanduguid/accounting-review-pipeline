//! Feature-gated frozen-oracle interface for the migration's fabricated tests.
pub use crate::pyjson::{Json, PyStr};
use crate::{
    Result, calculation_evidence,
    decimal::{self, Decimal},
    engine, equity,
    loader::{self, SourceSnapshot},
    models::*,
    pyjson, pypath, queries,
};
use std::{
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};

pub fn encode(value: &Json) -> String {
    pyjson::canonical(value)
}
pub fn decode(value: &str) -> Result<Json> {
    pyjson::loads(value).map_err(|e| format!("invalid probe JSON: {e:?}"))
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
        .and_then(PyStr::to_utf8)
        .ok_or_else(|| "probe requires UTF-8 text".into())
}
fn number(value: &Json) -> Result<Decimal> {
    Decimal::parse(&text(value)?).map_err(|_| "invalid probe decimal".into())
}
fn unhex(value: &str) -> Result<Vec<u8>> {
    if !value.is_ascii() || !value.len().is_multiple_of(2) {
        return Err("invalid probe hex".into());
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16)
                .map_err(|_| "invalid probe hex".into())
        })
        .collect()
}
fn snapshot(request: &Json) -> Result<SourceSnapshot> {
    let path = PathBuf::from(text(request.at("path"))?);
    Ok(SourceSnapshot::new(
        &path,
        unhex(&text(request.at("hex"))?)?,
    ))
}
fn payloads(value: &Json) -> Result<std::collections::BTreeMap<String, Vec<u8>>> {
    let Json::Object(pairs) = value else {
        return Err("invalid probe payloads".into());
    };
    pairs
        .iter()
        .map(|(name, value)| {
            Ok((
                name.to_utf8().ok_or("invalid probe filename")?,
                unhex(&text(value)?)?,
            ))
        })
        .collect()
}
fn cli(request: &Json) -> Result<Json> {
    use std::fs;
    let directory = crate::test_support::TempDir::new();
    let child = |name: &str| -> Result<PathBuf> {
        let path = Path::new(name);
        if path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err("probe fixture path must stay inside its temporary directory".into());
        }
        Ok(directory.join(path))
    };
    if let Some(Json::Object(files)) = request.get("files") {
        for (name, bytes) in files {
            let path = child(&name.to_utf8().ok_or("invalid fixture path")?)?;
            fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            fs::write(path, unhex(&text(bytes)?)?).map_err(|e| e.to_string())?;
        }
    }
    if let Some(directories) = request.get("directories").and_then(Json::as_array) {
        for name in directories {
            fs::create_dir_all(child(&text(name)?)?).map_err(|e| e.to_string())?;
        }
    }
    let root = pypath::text(&directory.0);
    let arguments = request
        .at("args")
        .as_array()
        .ok_or("probe CLI arguments must be a list")?
        .iter()
        .map(|arg| {
            let arg = arg
                .as_pystr()
                .ok_or("invalid probe argument")?
                .os_text()
                .replace("$TMP", &root);
            Ok(pypath::encode_os(&arg))
        })
        .collect::<Result<Vec<_>>>()?;
    let (code, stdout, stderr) = crate::stdio::capture(|| crate::cli::main(arguments));
    let resolved = pypath::text(
        &pypath::resolve(&directory.0, true)
            .map_err(|_| "cannot resolve probe temporary directory")?,
    );
    let normalise = |mut text: String| {
        for raw in [
            root.clone(),
            root.replace('\\', "/"),
            resolved.clone(),
            resolved.replace('\\', "/"),
        ] {
            let mut variants = vec![raw];
            for _ in 0..3 {
                let quoted = pyjson::dumps(&Json::str(variants.last().unwrap()), false);
                variants.push(quoted[1..quoted.len() - 1].to_owned());
            }
            for variant in variants.into_iter().rev() {
                text = text.replace(&variant, "$TMP");
            }
        }
        text
    };
    fn collect(
        path: &Path,
        root: &Path,
        files: &mut std::collections::BTreeMap<String, Vec<u8>>,
    ) -> Result<()> {
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                collect(&entry.path(), root, files)?;
            } else {
                files.insert(
                    entry
                        .path()
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    std::fs::read(entry.path()).map_err(|e| e.to_string())?,
                );
            }
        }
        Ok(())
    }
    let mut files = std::collections::BTreeMap::new();
    collect(&directory.0, &directory.0, &mut files)?;
    Ok(Json::object([
        ("code", Json::int(code.into())),
        ("stdout", Json::Str(PyStr::from_os_text(&normalise(stdout)))),
        ("stderr", Json::Str(PyStr::from_os_text(&normalise(stderr)))),
        (
            "files",
            Json::Object(
                files
                    .into_iter()
                    .map(|(path, bytes)| {
                        let bytes = std::str::from_utf8(&bytes)
                            .map(|text| normalise(text.to_owned()).into_bytes())
                            .unwrap_or(bytes);
                        (
                            path.as_str().into(),
                            Json::str(
                                &bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                            ),
                        )
                    })
                    .collect(),
            ),
        ),
    ]))
}
fn decimal_json(value: &Option<Decimal>) -> Json {
    value
        .as_ref()
        .map_or(Json::Null, |value| Json::str(&value.to_sci_string()))
}
pub fn row_json(row: TrialBalanceRow) -> Json {
    Json::object([
        ("report_date", Json::str(&row.report_date.iso())),
        ("tenant", Json::str(&row.tenant)),
        ("section", Json::str(&row.section)),
        ("account_id", Json::str(&row.account_id)),
        ("account_name", Json::str(&row.account_name)),
        ("account_code", Json::str(&row.account_code)),
        ("debit", Json::str(&row.debit.to_sci_string())),
        ("credit", Json::str(&row.credit.to_sci_string())),
        ("ytd_debit", Json::str(&row.ytd_debit.to_sci_string())),
        ("ytd_credit", Json::str(&row.ytd_credit.to_sci_string())),
    ])
}
pub fn exception_json(item: ExceptionItem) -> Json {
    Json::object([
        ("control", Json::str(&item.control)),
        ("status", Json::str(item.status.as_str())),
        ("tenant", Json::str(&item.tenant)),
        ("account_id", Json::str(&item.account_id)),
        ("account_code", Json::str(&item.account_code)),
        ("account_name", Json::str(&item.account_name)),
        ("current_value", decimal_json(&item.current_value)),
        ("prior_value", decimal_json(&item.prior_value)),
        ("difference", decimal_json(&item.difference)),
        ("threshold", decimal_json(&item.threshold)),
        ("percentage_change", decimal_json(&item.percentage_change)),
        ("reason", Json::str(&item.reason)),
        ("reviewer_action", Json::str(&item.reviewer_action)),
        ("review_group", Json::str(&item.review_group)),
    ])
}
fn query_json(item: ClientQuery) -> Json {
    Json::object([
        ("query_id", Json::str(&item.query_id)),
        ("control", Json::str(&item.control)),
        ("tenant", Json::str(&item.tenant)),
        ("account_id", Json::str(&item.account_id)),
        ("account_code", Json::str(&item.account_code)),
        ("account_name", Json::str(&item.account_name)),
        ("review_group", Json::str(&item.review_group)),
        ("difference", decimal_json(&item.difference)),
        ("question", Json::str(&item.question)),
        ("evidence_requested", Json::str(&item.evidence_requested)),
    ])
}
pub fn evidence_json(item: calculation_evidence::CalculationEvidence) -> Json {
    Json::object([
        ("label", Json::str(&item.label)),
        ("path", Json::str(&pypath::text(&item.path))),
        ("sha256", Json::str(&item.sha256)),
        ("schema", Json::str(&item.schema)),
        ("provider", Json::str(&item.provider)),
        ("calculator", Json::str(&item.calculator)),
        ("period", Json::str(&item.period)),
        ("status", Json::str(&item.status)),
        ("engine", Json::str(&item.engine)),
        ("calculation_sha256", Json::str(&item.calculation_sha256)),
        (
            "values",
            Json::Object(
                item.values
                    .into_iter()
                    .map(|(key, value)| {
                        (PyStr::from(key.as_str()), Json::str(&value.to_sci_string()))
                    })
                    .collect(),
            ),
        ),
        ("rate_tables", strings(item.rate_tables)),
        ("advisory_notes", strings(item.advisory_notes)),
        ("synthetic_input", Json::Bool(item.synthetic_input)),
        ("findings", strings(item.findings)),
    ])
}
fn strings(values: Vec<String>) -> Json {
    Json::Array(values.into_iter().map(|value| Json::str(&value)).collect())
}
fn pack_json(pack: engine::CloseReviewPack) -> Json {
    Json::object([
        ("status", Json::str(pack.status.as_str())),
        ("current_report_dates", strings(pack.current_report_dates)),
        ("prior_report_dates", strings(pack.prior_report_dates)),
        (
            "source_hashes",
            Json::Object(
                pack.source_hashes
                    .into_iter()
                    .map(|(k, v)| (PyStr::from(k.as_str()), Json::str(&v)))
                    .collect(),
            ),
        ),
        (
            "absolute_threshold",
            Json::str(&pack.absolute_threshold.to_sci_string()),
        ),
        (
            "percentage_threshold",
            Json::str(&pack.percentage_threshold.to_sci_string()),
        ),
        (
            "reconciliation_tolerance",
            Json::str(&pack.reconciliation_tolerance.to_sci_string()),
        ),
        (
            "exceptions",
            Json::Array(pack.exceptions.into_iter().map(exception_json).collect()),
        ),
        (
            "acknowledgement",
            pack.acknowledgement.map_or(Json::Null, |a| {
                Json::object([
                    ("reviewer_initials", Json::str(&a.reviewer_initials)),
                    ("reviewed_on", Json::str(&a.reviewed_on.iso())),
                    ("comment", Json::str(&a.comment)),
                ])
            }),
        ),
        (
            "calculation_evidence",
            Json::Array(
                pack.calculation_evidence
                    .into_iter()
                    .map(evidence_json)
                    .collect(),
            ),
        ),
        ("required_calculations", strings(pack.required_calculations)),
        ("controls_not_run", strings(pack.controls_not_run)),
        ("relied_on", strings(pack.relied_on.into_iter().collect())),
        (
            "equity_reconciliation",
            pack.equity_reconciliation.unwrap_or(Json::Null),
        ),
    ])
}
fn review(request: &Json) -> Result<engine::CloseReviewPack> {
    let current = snapshot(request.at("current"))?;
    let prior = snapshot(request.at("prior"))?;
    let mut config = engine::ReviewConfig::new(current.path.clone(), prior.path.clone());
    let mut inputs = std::collections::BTreeMap::from([
        (current.path.clone(), current),
        (prior.path.clone(), prior),
    ]);
    for (name, target) in [
        ("mapping", &mut config.mapping_path),
        ("mapping_policy", &mut config.mapping_policy_path),
        ("balance_policy", &mut config.balance_policy_path),
        ("subledger", &mut config.subledger_path),
        ("acknowledgement", &mut config.acknowledgement_path),
        ("equity_schedule", &mut config.equity_schedule_path),
    ] {
        if let Some(value) = request.get(name).filter(|s| !matches!(s, Json::Null)) {
            let source = snapshot(value)?;
            *target = Some(source.path.clone());
            inputs.insert(source.path.clone(), source);
        }
    }
    for (name, target) in [
        ("absolute", &mut config.absolute_threshold),
        ("percentage", &mut config.percentage_threshold),
        ("tolerance", &mut config.reconciliation_tolerance),
    ] {
        if let Some(value) = request.get(name) {
            *target = number(value)?;
        }
    }
    if let Some(value) = request.get("required") {
        config.required_calculations = value
            .as_array()
            .ok_or("invalid probe requirements")?
            .iter()
            .map(text)
            .collect::<Result<_>>()?;
    }
    for (name, target) in [
        ("currency", &mut config.equity_currency),
        ("currency_evidence", &mut config.equity_currency_evidence),
    ] {
        if let Some(value) = request.get(name).filter(|s| !matches!(s, Json::Null)) {
            *target = Some(text(value)?);
        }
    }
    if let Some(evidence) = request.get("evidence") {
        for value in evidence.as_array().ok_or("invalid probe evidence")? {
            let source = snapshot(value)?;
            config.calculation_evidence_paths.push(source.path.clone());
            inputs.insert(source.path.clone(), source);
        }
    }
    engine::review_with(&config, &mut |path, label| {
        inputs
            .get(path)
            .cloned()
            .ok_or_else(|| format!("{label} does not exist: {}.", pypath::text(path)))
    })
}
fn csv(request: &Json) -> Result<Json> {
    let source = snapshot(request)?;
    Ok(match text(request.at("kind"))?.as_str() {
        "trial_balance" => Json::Array(
            loader::load_canonical_tb(&source)?
                .into_iter()
                .map(row_json)
                .collect(),
        ),
        "mapping" => Json::Object(
            loader::load_mapping(Some(&source))?
                .into_iter()
                .map(|(key, value)| (PyStr::from(key.as_str()), Json::str(&value)))
                .collect(),
        ),
        "mapping_policy" => Json::Object(
            loader::load_mapping_policy(&source)?
                .into_iter()
                .map(|(key, values)| {
                    (
                        PyStr::from(key.as_str()),
                        strings(values.into_iter().collect()),
                    )
                })
                .collect(),
        ),
        "balance_policy" => Json::Object(
            loader::load_balance_policy(&source)?
                .into_iter()
                .map(|(key, (side, movement))| {
                    (
                        PyStr::from(key.as_str()),
                        Json::Array(vec![Json::str(&side), Json::Bool(movement)]),
                    )
                })
                .collect(),
        ),
        "subledger" => Json::Array(
            loader::load_subledger(Some(&source))?
                .into_iter()
                .map(|((tenant, id), amount)| {
                    Json::Array(vec![
                        Json::str(&tenant),
                        Json::str(&id),
                        Json::str(&amount.to_sci_string()),
                    ])
                })
                .collect(),
        ),
        _ => return Err("unsupported probe CSV kind".into()),
    })
}
fn operation(request: &Json) -> Result<Json> {
    if request.at("op").is_str("cli") {
        return cli(request);
    }
    match text(request.at("op"))?.as_str() {
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
        "format_money" => Ok(Json::str(&crate::report::money(&number(
            request.at("value"),
        )?)?)),
        "parse_pack_json" => crate::viewer::parse_json(&unhex(&text(request.at("hex"))?)?),
        "output_guard" => Ok(Json::Str(PyStr::from_os_text(&pypath::text(
            &crate::publication::require_output_outside_repository(&PathBuf::from(text(
                request.at("path"),
            )?))?,
        )))),
        "tb_integrity" => {
            let rows = loader::load_canonical_tb(&snapshot(request)?)?;
            let findings = engine::integrity_exceptions(&rows, "current")?
                .into_iter()
                .map(exception_json)
                .collect();
            Ok(Json::object([
                (
                    "rows",
                    Json::Array(rows.into_iter().map(row_json).collect()),
                ),
                ("findings", Json::Array(findings)),
            ]))
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
        "schedule" => {
            crate::schedules::review_snapshot(&snapshot(request)?, &text(request.at("kind"))?)
        }
        "reconciliation_date" => Ok(Json::str(
            &crate::reconciliation::date(&text(request.at("value"))?)?.iso(),
        )),
        "reconciliation_money" => decimal::with_precision(40, || {
            crate::reconciliation::money(
                &text(request.at("value"))?,
                "amount",
                Path::new("configuration"),
                0,
            )
            .map(|v| Json::str(&v.to_sci_string()))
        }),
        "reconcile" | "reconcile_render" => {
            use crate::reconciliation::{self, ReconcileConfig};
            let transactions = snapshot(request.at("transactions"))?;
            let mut inputs = std::collections::BTreeMap::from([(
                transactions.path.clone(),
                transactions.clone(),
            )]);
            let mut optional = |key: &str| -> Result<Option<PathBuf>> {
                let Some(value) = request.get(key).filter(|v| !matches!(v, Json::Null)) else {
                    return Ok(None);
                };
                let source = snapshot(value)?;
                let path = source.path.clone();
                inputs.insert(path.clone(), source);
                Ok(Some(path))
            };
            let opening_items = optional("opening_items")?;
            let decisions = optional("decisions")?;
            let config = ReconcileConfig {
                transactions: transactions.path,
                tenant: text(request.at("tenant"))?,
                account_id: text(request.at("account_id"))?,
                currency: text(request.at("currency"))?,
                period_start: text(request.at("period_start"))?,
                period_end: text(request.at("period_end"))?,
                opening_balance: text(request.at("opening_balance"))?,
                closing_balance: text(request.at("closing_balance"))?,
                opening_items,
                decisions,
                decisions_escaped: request
                    .get("decisions_escaped")
                    .and_then(Json::as_bool)
                    .unwrap_or(false),
            };
            let result = reconciliation::reconcile_with(&config, &mut |path, label| {
                inputs
                    .get(path)
                    .cloned()
                    .ok_or_else(|| format!("{label} does not exist: {}.", pypath::text(path)))
            })?;
            if request.at("op").is_str("reconcile_render") {
                Ok(Json::Object(
                    crate::reconciliation_report::rendered(&result)?
                        .into_iter()
                        .map(|(name, bytes)| {
                            (
                                PyStr::from(name.as_str()),
                                Json::str(
                                    &bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                                ),
                            )
                        })
                        .collect(),
                ))
            } else {
                Ok(result)
            }
        }
        "classify" => {
            let mut inputs = Vec::new();
            for key in ["original", "current", "coding_evidence"] {
                let mut source = snapshot(request.at(key))?;
                source.path =
                    pypath::resolve(&source.path, false).map_err(|_| "invalid probe path")?;
                inputs.push(source);
            }
            let paths = std::array::from_fn(|index| inputs[index].path.clone());
            crate::classification::classify_with(
                &paths,
                &text(request.at("currency"))?,
                &mut |path, _| {
                    inputs
                        .iter()
                        .find(|source| pypath::same_path(&source.path, path))
                        .cloned()
                        .ok_or_else(|| "probe classification source missing".into())
                },
            )
        }
        "compare" => {
            use crate::comparison::{self, CompareConfig};
            let previous = request.at("previous");
            let current = request.at("current");
            let old_tb = snapshot(previous.at("trial_balance"))?;
            let new_tb = snapshot(current.at("trial_balance"))?;
            let response = request
                .get("responses")
                .filter(|v| !matches!(v, Json::Null))
                .map(snapshot)
                .transpose()?;
            let config = CompareConfig {
                previous_pack: "previous-pack".into(),
                current_pack: "current-pack".into(),
                previous_tb: old_tb.path.clone(),
                current_tb: new_tb.path.clone(),
                responses: response.as_ref().map(|v| v.path.clone()),
            };
            comparison::compare_with(
                &config,
                &mut |path| {
                    crate::viewer::verify_payloads(&payloads(if path == config.previous_pack {
                        previous.at("payloads")
                    } else {
                        current.at("payloads")
                    })?)
                },
                &mut |path, label| {
                    let source = if label == "Response file" {
                        response.as_ref().unwrap()
                    } else if path == old_tb.path {
                        &old_tb
                    } else {
                        &new_tb
                    };
                    Ok(source.clone())
                },
            )
        }
        "queue" => {
            use crate::queue::{self, Filters};
            let packs = request.at("packs").as_array().unwrap();
            let paths = packs
                .iter()
                .map(|pack| text(pack.at("path")).map(PathBuf::from))
                .collect::<Result<Vec<_>>>()?;
            let optional = |key| {
                request
                    .get(key)
                    .filter(|v| !matches!(v, Json::Null))
                    .map(|v| v.as_pystr().unwrap().clone())
            };
            let filters = Filters {
                period: optional("period"),
                status: optional("status"),
                reviewer: optional("reviewer"),
            };
            let result = queue::render_with(&paths, &filters, &mut |path| {
                let pack = packs
                    .iter()
                    .find(|pack| {
                        pypath::resolve(&PathBuf::from(text(pack.at("path")).unwrap()), false)
                            .is_ok_and(|candidate| pypath::same_path(&candidate, path))
                    })
                    .unwrap();
                if let Some(error) = pack.get("error") {
                    return Err(text(error)?);
                }
                crate::viewer::verify_payloads(&payloads(pack.at("payloads"))?)
            })?;
            Ok(Json::str(&result))
        }
        "drivers" | "drivers_render" => {
            let top = request
                .at("top")
                .as_plain_int()
                .ok_or("invalid probe top")?;
            if top < &1.into() {
                return Err("--top must be at least 1.".into());
            }
            let Json::Object(pairs) = request.at("payloads") else {
                return Err("invalid probe payloads".into());
            };
            let payloads = pairs
                .iter()
                .map(|(name, value)| {
                    Ok((
                        name.to_utf8().ok_or("invalid probe filename")?,
                        unhex(&text(value)?)?,
                    ))
                })
                .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
            let pack = crate::viewer::verify_payloads(&payloads)?;
            let source = snapshot(request.at("transactions"))?;
            let result = crate::drivers::drivers_with(
                &pack,
                &source.path.clone(),
                &text(request.at("currency"))?,
                top,
                &mut |_, _| Ok(source.clone()),
            )?;
            if request.at("op").is_str("drivers_render") {
                Ok(Json::Object(
                    crate::drivers::rendered(&result)?
                        .into_iter()
                        .map(|(name, bytes)| {
                            (
                                PyStr::from(name.as_str()),
                                Json::str(
                                    &bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                                ),
                            )
                        })
                        .collect(),
                ))
            } else {
                Ok(result)
            }
        }
        "review" => Ok(pack_json(review(request)?)),
        "render" => Ok(strings(
            crate::report::rendered(&review(request)?)?
                .iter()
                .map(|bytes| bytes.iter().map(|b| format!("{b:02x}")).collect())
                .collect(),
        )),
        "verify" => {
            let Json::Object(pairs) = request.at("payloads") else {
                return Err("invalid probe payloads".into());
            };
            let payloads = pairs
                .iter()
                .map(|(name, value)| {
                    Ok((
                        name.to_utf8().ok_or("invalid probe filename")?,
                        unhex(&text(value)?)?,
                    ))
                })
                .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
            let verified = crate::viewer::verify_payloads(&payloads)?;
            let hashes = Json::Object(
                verified
                    .artefact_sha256
                    .iter()
                    .map(|(k, v)| (PyStr::from(k.as_str()), Json::str(v)))
                    .collect(),
            );
            let sheet = Json::Str(PyStr::from_os_text(&crate::viewer::render_verified(
                &verified,
            )?));
            Ok(Json::object([
                (
                    "verified",
                    Json::Array(vec![
                        verified.document,
                        Json::str(&verified.summary_text),
                        Json::Array(verified.exceptions),
                        verified.queries.map_or(Json::Null, Json::Array),
                        hashes.clone(),
                    ]),
                ),
                ("sheet", Json::Array(vec![sheet, hashes])),
            ]))
        }
        "review_note" => {
            let item = loader::load_reviewer_acknowledgement(Some(&snapshot(request)?))?.unwrap();
            Ok(Json::object([
                ("reviewer_initials", Json::str(&item.reviewer_initials)),
                ("reviewed_on", Json::str(&item.reviewed_on.iso())),
                ("comment", Json::str(&item.comment)),
            ]))
        }
        "integrity" => {
            let rows = loader::load_canonical_tb(&snapshot(request)?)?;
            Ok(Json::Array(
                engine::integrity_exceptions(&rows, "Current")?
                    .into_iter()
                    .map(exception_json)
                    .collect(),
            ))
        }
        "compare_core" => {
            let current = loader::load_canonical_tb(&snapshot(request.at("current"))?)?;
            let prior = loader::load_canonical_tb(&snapshot(request.at("prior"))?)?;
            let mut exceptions = engine::financial_year_reset_exceptions(
                current[0].report_date,
                prior[0].report_date,
            );
            let current = current.into_iter().map(|row| (row.key(), row)).collect();
            let prior = prior.into_iter().map(|row| (row.key(), row)).collect();
            exceptions.extend(engine::period_comparison_exceptions(
                &current,
                &prior,
                &number(request.at("absolute"))?,
                &number(request.at("percentage"))?,
            ));
            let queries = queries::derive_client_queries(&exceptions)?;
            Ok(Json::object([
                (
                    "exceptions",
                    Json::Array(exceptions.into_iter().map(exception_json).collect()),
                ),
                (
                    "queries",
                    Json::Array(queries.into_iter().map(query_json).collect()),
                ),
            ]))
        }
        "calculation_evidence" => Ok(evidence_json(calculation_evidence::load(&snapshot(
            request,
        )?)?)),
        "equity_load" => equity::load_schedule(&snapshot(request)?),
        "equity" => {
            let current_source = snapshot(request.at("current"))?;
            let prior_source = snapshot(request.at("prior"))?;
            let current = loader::load_canonical_tb(&current_source)?;
            let prior = loader::load_canonical_tb(&prior_source)?;
            let optional = |key: &str| match request.at(key) {
                Json::Null => Ok(None),
                v => text(v).map(Some),
            };
            let currency = optional("currency")?;
            let currency_evidence = optional("currency_evidence")?;
            let tolerance = number(request.at("tolerance"))?;
            let source = snapshot(request)?;
            let (result, findings) = equity::reconcile(
                &source,
                equity::Inputs {
                    prior: &prior,
                    current: &current,
                    prior_sha256: &prior_source.sha256,
                    current_sha256: &current_source.sha256,
                    currency: currency.as_deref(),
                    currency_evidence: currency_evidence.as_deref(),
                    tolerance: &tolerance,
                },
            )?;
            let exceptions: Vec<_> = findings.into_iter().map(exception_json).collect();
            let sources = Json::object([("equity_schedule", Json::str(&source.sha256))]);
            equity::validate_result(&result, &sources, &exceptions)?;
            Ok(Json::object([
                ("result", result.clone()),
                ("exceptions", Json::Array(exceptions)),
                (
                    "summary",
                    Json::Array(
                        equity::summary_lines(&result)
                            .iter()
                            .map(|s| Json::Str(PyStr::from_os_text(s)))
                            .collect(),
                    ),
                ),
            ]))
        }
        "equity_validate" => {
            equity::validate_result(
                request.at("result"),
                request.at("sources"),
                request
                    .at("exceptions")
                    .as_array()
                    .ok_or("invalid probe findings")?,
            )?;
            Ok(request.at("result").clone())
        }
        "covers_period" => {
            let evidence = calculation_evidence::load(&snapshot(request)?)?;
            let date = crate::isodate::date_fromisoformat(&text(request.at("date"))?)
                .ok_or("invalid probe date")?;
            Ok(calculation_evidence::covers_period(&evidence, date).map_or(Json::Null, Json::Bool))
        }
        "canonical_utf8" => Ok(Json::str(
            &pyjson::canonical_utf8(request.at("value")).map_err(str::to_owned)?,
        )),
        "arithmetic" => {
            let precision = text(request.at("precision"))?
                .parse::<i128>()
                .map_err(|_| "invalid probe precision")?;
            if ![28, 40, 80].contains(&precision) {
                return Err("invalid probe precision".into());
            }
            let a = number(request.at("a"))?;
            let b = number(request.at("b"))?;
            let kind = text(request.at("kind"))?;
            decimal::with_precision(precision, || match kind.as_str() {
                "add" | "sub" => {
                    let result = if kind == "add" { a.add(&b) } else { a.sub(&b) };
                    Ok(Json::object([
                        ("value", Json::str(&result.value.to_sci_string())),
                        ("inexact", Json::Bool(result.inexact)),
                    ]))
                }
                "mul" => Ok(Json::str(&a.mul(&b).to_sci_string())),
                "div" if !b.is_zero() => Ok(Json::str(&a.div(&b).to_sci_string())),
                _ => Err("invalid probe arithmetic".into()),
            })
        }
        _ => Err("unsupported probe operation".into()),
    }
}
pub fn evaluate(request: &Json) -> Json {
    match operation(request) {
        Ok(value) => Json::object([("ok", value)]),
        Err(error) => Json::object([("error", Json::str(&error))]),
    }
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
            .map_err(|error| error.to_string())
            .and_then(|line| decode(&line));
        let response = match result {
            Ok(request) => evaluate(&request),
            Err(error) => {
                eprintln!("{error}");
                return 1;
            }
        };
        if writeln!(stdout, "{}", encode(&response))
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return 1;
        }
    }
    0
}
