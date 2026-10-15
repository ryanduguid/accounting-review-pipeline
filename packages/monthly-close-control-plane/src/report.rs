use crate::{
    Result,
    calculation_evidence::CalculationEvidence,
    decimal::{self, Decimal},
    engine::CloseReviewPack,
    equity,
    models::{ClientQuery, ExceptionItem, Status},
    pycsv,
    pyjson::{self, Json, PyStr},
    pytext,
};

pub const PACK_FILE_NAMES: [&str; 4] = [
    "close-review-pack.json",
    "close-summary.md",
    "exceptions.csv",
    "client-queries.csv",
];
pub const EXCEPTION_FIELDS: [&str; 14] = [
    "control",
    "status",
    "tenant",
    "account_id",
    "account_code",
    "account_name",
    "review_group",
    "current_value",
    "prior_value",
    "difference",
    "threshold",
    "percentage_change",
    "reason",
    "reviewer_action",
];
pub const QUERY_FIELDS: [&str; 10] = [
    "query_id",
    "control",
    "tenant",
    "account_id",
    "account_code",
    "account_name",
    "review_group",
    "difference",
    "question",
    "evidence_requested",
];
pub const ACKNOWLEDGEMENT_EFFECT: &str =
    "Acknowledgement is evidence of human review only; it does not approve or close a period.";
pub const CALCULATION_EVIDENCE_EFFECT: &str = "Evidence read from files. This pack made no calculation and contacted no service. A figure here supports review; it does not approve anything. `usable` is true only where the file hangs together, carries a figure and covers this close's period; read the exceptions for why one is false.";
pub const CALCULATION_EVIDENCE_HEADING: &str = "## Calculation evidence";
pub const CALCULATION_EVIDENCE_PREAMBLE: &str = "Read from files supplied to this run. This pack made no calculation and contacted no service. A figure here supports review and approves nothing.";
pub const CALCULATION_EVIDENCE_TABLE_HEADER: &str =
    "| Calculation | Status | Period | Figures | Relied on | Entry digest |";
pub const CALCULATION_EVIDENCE_TABLE_DELIMITER: &str = "| --- | --- | --- | --- | --- | --- |";
pub const NO_CALCULATION_EVIDENCE: &str = "A calculation was required for this close and no evidence file was supplied for it. The exceptions below say which.";
pub const CLIENT_QUERY_PREAMBLE: [&str; 4] = [
    "These are draft questions for the preparer, derived from the exceptions above. Nothing has been sent to anyone. Read and edit them before they reach a client, and record the answers in the firm's own tracker: this pack is evidence of one run, and editing it breaks the check that its files still agree.",
    "",
    "Answering every query does not close the period, and a query nobody raised is not evidence that nothing needs asking.",
    "",
];
pub const PRIOR_ZERO_PERCENTAGE: &str = "n/a (prior period zero)";

pub fn money(value: &Decimal) -> Result<String> {
    value
        .format_places(2.max(-value.exponent()))
        .ok_or_else(|| "decimal fixed-point output exceeds available memory".into())
}
fn optional_money(value: &Option<Decimal>) -> Result<String> {
    value
        .as_ref()
        .map(money)
        .transpose()
        .map(Option::unwrap_or_default)
}
pub fn percentage(value: &Decimal) -> Result<String> {
    decimal::with_precision(28, || {
        let scaled = value.mul(&Decimal::parse("100").unwrap());
        scaled
            .format_places(2.max(-scaled.adjusted()))
            .map(|s| format!("{s}%"))
            .ok_or_else(|| "decimal fixed-point output exceeds available memory".into())
    })
}
pub fn percentage_cell(item: &ExceptionItem) -> Result<String> {
    if item.percentage_change.is_none()
        && item.control == "period_variance"
        && item.prior_value.as_ref().is_some_and(Decimal::is_zero)
    {
        return Ok(PRIOR_ZERO_PERCENTAGE.into());
    }
    item.percentage_change
        .as_ref()
        .map(percentage)
        .transpose()
        .map(Option::unwrap_or_default)
}
pub fn csv_safe(value: &str) -> String {
    if value
        .trim_start_matches(|c: char| pytext::is_space(c as u32))
        .starts_with(['=', '+', '-', '@'])
    {
        format!("'{value}")
    } else {
        value.into()
    }
}
pub fn md_cell(value: &str) -> String {
    value
        .split(|c: char| pytext::is_space(c as u32))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('*', "\\*")
        .replace('`', "\\`")
}
pub fn note_lines(comment: &str) -> Vec<String> {
    if !comment.contains(['\r', '\n']) {
        return vec![format!("- Comment: {}", md_cell(comment))];
    }
    let mut lines = vec!["- Comment:".into()];
    for raw in pytext::split_lines(comment) {
        let mut line = md_cell(raw);
        if line.starts_with('#') {
            line.insert(0, '\\');
        }
        lines.push(format!("  > {line}").trim_end().into());
    }
    lines
}
pub fn exception_dict(item: &ExceptionItem) -> Result<Json> {
    Ok(Json::object([
        ("control", Json::str(&item.control)),
        ("status", Json::str(item.status.as_str())),
        ("tenant", Json::str(&item.tenant)),
        ("account_id", Json::str(&item.account_id)),
        ("account_code", Json::str(&item.account_code)),
        ("account_name", Json::str(&item.account_name)),
        ("review_group", Json::str(&item.review_group)),
        (
            "current_value",
            Json::str(&optional_money(&item.current_value)?),
        ),
        (
            "prior_value",
            Json::str(&optional_money(&item.prior_value)?),
        ),
        ("difference", Json::str(&optional_money(&item.difference)?)),
        ("threshold", Json::str(&optional_money(&item.threshold)?)),
        ("percentage_change", Json::str(&percentage_cell(item)?)),
        ("reason", Json::str(&item.reason)),
        ("reviewer_action", Json::str(&item.reviewer_action)),
    ]))
}
pub fn query_dict(item: &ClientQuery) -> Result<Json> {
    Ok(Json::object([
        ("query_id", Json::str(&item.query_id)),
        ("control", Json::str(&item.control)),
        ("tenant", Json::str(&item.tenant)),
        ("account_id", Json::str(&item.account_id)),
        ("account_code", Json::str(&item.account_code)),
        ("account_name", Json::str(&item.account_name)),
        ("review_group", Json::str(&item.review_group)),
        ("difference", Json::str(&optional_money(&item.difference)?)),
        ("question", Json::str(&item.question)),
        ("evidence_requested", Json::str(&item.evidence_requested)),
    ]))
}
fn strings(values: &[String]) -> Json {
    Json::Array(values.iter().map(|s| Json::str(s)).collect())
}
pub fn evidence_entry(pack: &CloseReviewPack, item: &CalculationEvidence) -> Result<Json> {
    Ok(Json::object([
        ("label", Json::str(&item.label)),
        ("schema", Json::str(&item.schema)),
        ("provider", Json::str(&item.provider)),
        ("calculator", Json::str(&item.calculator)),
        ("period", Json::str(&item.period)),
        ("status", Json::str(&item.status)),
        ("engine", Json::str(&item.engine)),
        ("calculation_sha256", Json::str(&item.calculation_sha256)),
        ("file_sha256", Json::str(&item.sha256)),
        ("synthetic_input", Json::Bool(item.synthetic_input)),
        ("rate_tables", strings(&item.rate_tables)),
        ("advisory_notes", strings(&item.advisory_notes)),
        (
            "values",
            Json::Object(
                item.values
                    .iter()
                    .map(|(key, value)| Ok((PyStr::from(key.as_str()), Json::str(&money(value)?))))
                    .collect::<Result<_>>()?,
            ),
        ),
        ("usable", Json::Bool(pack.relied_on.contains(&item.label))),
    ]))
}
pub fn entry_digest(entry: &Json) -> Result<String> {
    Ok(crate::sha256(
        pyjson::canonical_utf8(entry)
            .map_err(str::to_owned)?
            .as_bytes(),
    ))
}
pub fn as_json(pack: &CloseReviewPack) -> Result<Json> {
    let acknowledgement = pack.acknowledgement.as_ref().map_or(Json::Null, |a| {
        Json::object([
            ("reviewer_initials", Json::str(&a.reviewer_initials)),
            ("reviewed_on", Json::str(&a.reviewed_on.iso())),
            ("comment", Json::str(&a.comment)),
            ("effect", Json::str(ACKNOWLEDGEMENT_EFFECT)),
        ])
    });
    let mut result = Json::object([
        ("acknowledgement", acknowledgement),
        (
            "client_queries",
            Json::Array(
                pack.client_queries()?
                    .iter()
                    .map(query_dict)
                    .collect::<Result<_>>()?,
            ),
        ),
        ("controls_not_run", strings(&pack.controls_not_run)),
        ("current_report_dates", strings(&pack.current_report_dates)),
        (
            "exceptions",
            Json::Array(
                pack.exceptions
                    .iter()
                    .map(exception_dict)
                    .collect::<Result<_>>()?,
            ),
        ),
        ("overall_status", Json::str(pack.status.as_str())),
        ("prior_report_dates", strings(&pack.prior_report_dates)),
        (
            "source_sha256",
            Json::Object(
                pack.source_hashes
                    .iter()
                    .map(|(k, v)| (PyStr::from(k.as_str()), Json::str(v)))
                    .collect(),
            ),
        ),
        (
            "thresholds",
            Json::object([
                (
                    "absolute_variance",
                    Json::str(&money(&pack.absolute_threshold)?),
                ),
                (
                    "percentage_variance",
                    Json::str(&percentage(&pack.percentage_threshold)?),
                ),
                (
                    "reconciliation_tolerance",
                    Json::str(&money(&pack.reconciliation_tolerance)?),
                ),
            ]),
        ),
    ]);
    if !pack.calculation_evidence.is_empty() || !pack.required_calculations.is_empty() {
        let mut evidence: Vec<_> = pack.calculation_evidence.iter().collect();
        evidence.sort_by(|a, b| a.label.cmp(&b.label));
        result.set(
            "calculation_evidence",
            Json::object([
                ("required", strings(&pack.required_calculations)),
                (
                    "supplied",
                    Json::Array(
                        evidence
                            .into_iter()
                            .map(|e| evidence_entry(pack, e))
                            .collect::<Result<_>>()?,
                    ),
                ),
                ("effect", Json::str(CALCULATION_EVIDENCE_EFFECT)),
            ]),
        );
    }
    if let Some(equity) = &pack.equity_reconciliation {
        result.set("equity_reconciliation", equity.clone());
    }
    Ok(result)
}
fn absent(value: &str) -> &str {
    if value.is_empty() { "n/a" } else { value }
}
pub fn account_name(code: &str, name: &str, id: &str) -> String {
    let pieces: Vec<_> = [code, name].into_iter().filter(|s| !s.is_empty()).collect();
    if pieces.is_empty() {
        absent(id).into()
    } else {
        pieces.join(" / ")
    }
}
pub fn as_markdown(pack: &CloseReviewPack) -> Result<PyStr> {
    let queries = pack.client_queries()?;
    let blocked = pack
        .exceptions
        .iter()
        .filter(|e| e.status == Status::Blocked)
        .count();
    let review = pack
        .exceptions
        .iter()
        .filter(|e| e.status == Status::Review)
        .count();
    let controls = pack.controls_not_run.join(", ");
    let mut lines = vec!["# Monthly Close Review Pack".into(),String::new(),format!("**Overall status: {}**",pack.status.as_str()),String::new(),
        "This pack is a review aid. It does not approve a close, post a journal, make a payment, lodge a return, or lock a period.".into(),String::new(),"## Scope".into(),String::new(),
        format!("- Current report date(s): {}",pack.current_report_dates.join(", ")),format!("- Prior report date(s): {}",pack.prior_report_dates.join(", ")),
        format!("- Material variance thresholds: ${} and {}",money(&pack.absolute_threshold)?,percentage(&pack.percentage_threshold)?),
        format!("- Reconciliation tolerance: ${}",money(&pack.reconciliation_tolerance)?),format!("- Exceptions: {} total; {blocked} blocked; {review} requiring review.",pack.exceptions.len()),
        format!("- Client queries drafted: {}.",queries.len()),format!("- Controls not run: {}.",if controls.is_empty() {"none"} else {&controls}),String::new(),"## Source evidence".into(),String::new(),
    ];
    for (name, digest) in &pack.source_hashes {
        lines.push(format!("- `{name}`: `{digest}`"));
    }
    if !pack.calculation_evidence.is_empty() || !pack.required_calculations.is_empty() {
        lines.extend([
            String::new(),
            CALCULATION_EVIDENCE_HEADING.into(),
            String::new(),
            CALCULATION_EVIDENCE_PREAMBLE.into(),
            String::new(),
        ]);
        let required = pack
            .required_calculations
            .iter()
            .map(|s| format!("`{s}`"))
            .collect::<Vec<_>>()
            .join("; ");
        lines.extend([
            format!(
                "Required: {}",
                if required.is_empty() {
                    "none"
                } else {
                    &required
                }
            ),
            String::new(),
        ]);
        if pack.calculation_evidence.is_empty() {
            lines.push(NO_CALCULATION_EVIDENCE.into());
        } else {
            lines.extend([
                CALCULATION_EVIDENCE_TABLE_HEADER.into(),
                CALCULATION_EVIDENCE_TABLE_DELIMITER.into(),
            ]);
            let mut evidence: Vec<_> = pack.calculation_evidence.iter().collect();
            evidence.sort_by(|a, b| a.label.cmp(&b.label));
            for item in evidence {
                let figures = item
                    .values
                    .iter()
                    .map(|(name, value)| money(value).map(|v| format!("{name} {v}")))
                    .collect::<Result<Vec<_>>>()?
                    .join("; ");
                lines.push(format!(
                    "| {} | {} | {} | {} | {} | {} |",
                    md_cell(&item.label),
                    md_cell(&item.status),
                    md_cell(absent(&item.period)),
                    md_cell(absent(&figures)),
                    if pack.relied_on.contains(&item.label) {
                        "yes"
                    } else {
                        "no"
                    },
                    entry_digest(&evidence_entry(pack, item)?)?
                ));
            }
        }
    }
    let mut equity_range = None;
    if let Some(equity) = &pack.equity_reconciliation {
        lines.extend([
            String::new(),
            "## Equity reconciliation".into(),
            String::new(),
        ]);
        let start = lines.len();
        lines.extend(equity::summary_lines(equity));
        equity_range = Some(start..lines.len());
    }
    lines.extend([String::new(), "## Exceptions".into(), String::new()]);
    if pack.exceptions.is_empty() {
        lines.push("No exceptions were raised. A human must still decide whether the close is appropriate.".into());
    } else {
        lines.extend([
            "| Status | Control | Tenant | Account | Difference | Reason |".into(),
            "| --- | --- | --- | --- | ---: | --- |".into(),
        ]);
        for item in &pack.exceptions {
            lines.push(format!(
                "| {} | {} | {} | {} | {} | {} |",
                item.status.as_str(),
                item.control,
                md_cell(absent(&item.tenant)),
                md_cell(&account_name(
                    &item.account_code,
                    &item.account_name,
                    &item.account_id
                )),
                absent(&optional_money(&item.difference)?),
                md_cell(&item.reason)
            ));
        }
    }
    lines.extend([String::new(), "## Client queries".into(), String::new()]);
    lines.extend(CLIENT_QUERY_PREAMBLE.map(str::to_owned));
    if queries.is_empty() {
        lines.push("No exception raised a question for the client. Every exception in this pack, if any, is one the firm settles from its own records.".into());
    } else {
        lines.extend([
            "| Query | Control | Tenant | Account | Difference | Question | Evidence requested |"
                .into(),
            "| --- | --- | --- | --- | ---: | --- | --- |".into(),
        ]);
        for item in queries {
            lines.push(format!(
                "| {} | {} | {} | {} | {} | {} | {} |",
                item.query_id,
                item.control,
                md_cell(absent(&item.tenant)),
                md_cell(&account_name(
                    &item.account_code,
                    &item.account_name,
                    &item.account_id
                )),
                absent(&optional_money(&item.difference)?),
                md_cell(&item.question),
                md_cell(&item.evidence_requested)
            ));
        }
    }
    lines.extend([
        String::new(),
        "## Human acknowledgement".into(),
        String::new(),
    ]);
    if let Some(a) = &pack.acknowledgement {
        lines.extend([
            format!("- Reviewer initials: {}", md_cell(&a.reviewer_initials)),
            format!("- Reviewed on: {}", a.reviewed_on.iso()),
        ]);
        lines.extend(note_lines(&a.comment));
        lines.push("- Effect: acknowledgement records a human action only; it does not change the control status or approve a close.".into());
    } else {
        lines.push(
            "No reviewer acknowledgement was supplied. This does not create or imply an approval."
                .into(),
        );
    }
    lines.push(String::new());
    let mut points = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index != 0 {
            points.push(10);
        }
        let value = if equity_range.as_ref().is_some_and(|r| r.contains(&index)) {
            PyStr::from_os_text(line)
        } else {
            PyStr::from(line.as_str())
        };
        points.extend(value.0);
    }
    Ok(PyStr(points))
}
fn csv(records: impl IntoIterator<Item = Json>, fields: &[&str], guarded: &[&str]) -> String {
    let mut output = pycsv::write_record(fields);
    for record in records {
        let values: Vec<_> = fields
            .iter()
            .map(|key| {
                let raw = record.at(key).as_pystr().unwrap().to_utf8().unwrap();
                if guarded.contains(key) {
                    csv_safe(&raw)
                } else {
                    raw
                }
            })
            .collect();
        output.push_str(&pycsv::write_record(
            &values.iter().map(String::as_str).collect::<Vec<_>>(),
        ));
    }
    output
}
pub fn as_csv(pack: &CloseReviewPack) -> Result<String> {
    Ok(csv(
        pack.exceptions
            .iter()
            .map(exception_dict)
            .collect::<Result<Vec<_>>>()?,
        &EXCEPTION_FIELDS,
        &[
            "tenant",
            "account_id",
            "account_code",
            "account_name",
            "review_group",
        ],
    ))
}
pub fn as_query_csv(pack: &CloseReviewPack) -> Result<String> {
    Ok(csv(
        pack.client_queries()?
            .iter()
            .map(query_dict)
            .collect::<Result<Vec<_>>>()?,
        &QUERY_FIELDS,
        &[
            "tenant",
            "account_id",
            "account_code",
            "account_name",
            "review_group",
            "question",
            "evidence_requested",
        ],
    ))
}
pub fn rendered(pack: &CloseReviewPack) -> Result<[Vec<u8>; 4]> {
    let native = |s: String| {
        if cfg!(windows) {
            s.replace('\n', "\r\n").into_bytes()
        } else {
            s.into_bytes()
        }
    };
    let csv = |s: String| {
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend(s.as_bytes());
        bytes
    };
    let markdown = as_markdown(pack)?;
    let markdown = if cfg!(windows) {
        let mut points = Vec::with_capacity(markdown.0.len());
        for cp in markdown.0 {
            if cp == 10 {
                points.push(13);
            }
            points.push(cp);
        }
        PyStr(points)
    } else {
        markdown
    };
    Ok([
        native(pyjson::pretty(&as_json(pack)?) + "\n"),
        pyjson::encode_utf8(&markdown)?.into_bytes(),
        csv(as_csv(pack)?),
        csv(as_query_csv(pack)?),
    ])
}
