// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Acceptance checks for the fixed fabricated joined examples, not client data.

use monthly_close_control_plane::{
    decimal::Decimal,
    pyjson::{self, Json},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

pub const CUTOFFS: [&str; 4] = ["2026-09-30", "2026-10-31", "2026-11-30", "2026-12-31"];
pub const CASH: [&str; 4] = ["20000", "22000", "24500", "29500"];
pub const ROUTES: [&str; 4] = ["close-forecast", "quarter", "job-cash", "grant-cash"];
pub const GRANT_FINDINGS: [(&str, &str); 5] = [
    ("MISSING_SOURCE_EVIDENCE", "L3"),
    ("OUTSIDE_AGREEMENT_PERIOD", "A3"),
    ("MISSING_ALLOCATION_EVIDENCE", "A4"),
    ("UNAPPROVED_ALLOCATION", "A4"),
    ("UNALLOCATED_SOURCE", "L4"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct CallRecord {
    pub owner: String,
    pub kind: String,
    pub arguments: Vec<String>,
    pub exit_code: Option<i32>,
    pub expected_exit: i32,
    pub elapsed_seconds: f64,
    pub failure: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResultError {
    Io(String),
    Validation(String),
}

impl std::fmt::Display for ResultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(message) | Self::Validation(message) => f.write_str(message),
        }
    }
}

pub fn route_counts(route: &str) -> Option<&'static [(&'static str, usize)]> {
    match route {
        "close-forecast" => Some(&[("fpa", 2), ("close", 2)]),
        "quarter" => Some(&[("fpa", 1), ("close", 8)]),
        "job-cash" => Some(&[("wip", 2), ("fpa", 2)]),
        "grant-cash" => Some(&[("grants", 1), ("fpa", 1)]),
        _ => None,
    }
}

fn require(condition: bool, message: &str) -> Result<(), ResultError> {
    if condition {
        Ok(())
    } else {
        Err(ResultError::Validation(message.to_owned()))
    }
}

pub fn read_json(root: &Path, name: &str) -> Result<Json, ResultError> {
    let text = fs::read_to_string(root.join(name)).map_err(|e| ResultError::Io(e.to_string()))?;
    pyjson::loads(&text).map_err(|_| ResultError::Validation(format!("Invalid JSON in {name}")))
}

fn field<'a>(object: &'a Json, key: &str) -> Result<&'a Json, ResultError> {
    object
        .get(key)
        .ok_or_else(|| ResultError::Validation(format!("Missing field: {key}")))
}

fn json_str(value: &Json) -> Result<String, ResultError> {
    value
        .as_pystr()
        .and_then(|s| s.to_utf8())
        .ok_or_else(|| ResultError::Validation("Expected string value".to_owned()))
}

fn json_decimal(value: &Json) -> Result<Decimal, ResultError> {
    let text = match value {
        Json::Str(s) => s.to_utf8(),
        Json::Int(n) => Some(n.to_string()),
        Json::Float(f) if f.is_finite() => Some(pyjson::float_repr(*f)),
        _ => None,
    };
    text.and_then(|raw| Decimal::parse(&raw).ok())
        .ok_or_else(|| ResultError::Validation("Expected decimal value".to_owned()))
}

fn dec(raw: &str) -> Decimal {
    Decimal::parse(raw).expect("static decimal literal")
}

/// Check dates before using any producer-supplied value in an output path.
pub fn quarter_periods(periods: &Json) -> Result<(), ResultError> {
    let items = periods.as_array().ok_or_else(|| {
        ResultError::Validation("Quarter needs opening balances and three monthly periods".into())
    })?;
    require(
        items.len() == 4,
        "Quarter needs opening balances and three monthly periods",
    )?;
    for (item, (&cutoff, &cash)) in items.iter().zip(CUTOFFS.iter().zip(CASH.iter())) {
        let actual_cutoff = json_str(field(item, "cutoff")?)?;
        require(
            actual_cutoff == cutoff,
            "Quarter cutoffs differ from the fabricated fixture",
        )?;
        let balances = field(item, "balances")?;
        let actual_cash = json_decimal(field(balances, "cash")?)?;
        require(
            actual_cash == dec(cash),
            "Quarter cash balances differ from the fabricated fixture",
        )?;
    }
    Ok(())
}

fn validate_call_coverage(
    routes: &[&'static str],
    calls: &[CallRecord],
    projects: &BTreeMap<String, Option<String>>,
) -> Result<(), ResultError> {
    let mut expected_workflow: BTreeMap<&str, usize> = BTreeMap::new();
    for route in routes {
        let counts = route_counts(route)
            .ok_or_else(|| ResultError::Validation(format!("Unknown workflow: {route}")))?;
        for &(owner, count) in counts {
            *expected_workflow.entry(owner).or_insert(0) += count;
        }
    }
    let mut actual_workflow: BTreeMap<&str, usize> = BTreeMap::new();
    let mut actual_runtime: BTreeMap<&str, usize> = BTreeMap::new();
    for call in calls {
        if call.kind == "workflow" {
            *actual_workflow.entry(call.owner.as_str()).or_insert(0) += 1;
        } else if call.kind == "runtime" {
            *actual_runtime.entry(call.owner.as_str()).or_insert(0) += 1;
        }
    }
    require(
        actual_workflow == expected_workflow,
        "Workflow command coverage is incomplete",
    )?;
    let expected_runtime: BTreeMap<&str, usize> =
        projects.keys().map(|owner| (owner.as_str(), 1)).collect();
    require(
        actual_runtime == expected_runtime,
        "Runtime probe coverage is incomplete",
    )?;
    require(
        calls.iter().all(|c| {
            (c.kind == "workflow" || c.kind == "runtime")
                && c.failure.is_none()
                && c.exit_code == Some(c.expected_exit)
        }),
        "A command did not complete as expected",
    )
}

fn check_close(output: &Path, name: &str, label: &str) -> Result<String, ResultError> {
    let pack = read_json(output, name)?;
    let status_ok = field(&pack, "overall_status")?.is_str("REVIEW");
    let count = field(&pack, "exceptions")?
        .as_array()
        .map_or(0, <[Json]>::len);
    require(
        status_ok && count > 0,
        &format!("{label} must retain REVIEW findings"),
    )?;
    let noun = if count == 1 { "finding" } else { "findings" };
    Ok(format!("{label}: REVIEW ({count} {noun})."))
}

fn validate_close_forecast(output: &Path, lines: &mut Vec<String>) -> Result<(), ResultError> {
    lines.push(check_close(
        output,
        "lumbridge-close/close-review-pack.json",
        "Close",
    )?);
    let refresh = read_json(output, "lumbridge-refresh/review.json")?;
    let closed_ok = field(&refresh, "closed_through")?.is_str("2026-10-31");
    let cash_ok = json_decimal(field(field(&refresh, "actuals")?, "ending_cash")?)? == dec("-1960");
    require(
        closed_ok && cash_ok,
        "Cash refresh differs from the fixture",
    )?;
    lines.push("Cash refresh: AUD -1,960.".to_owned());
    Ok(())
}

fn validate_quarter(output: &Path, lines: &mut Vec<String>) -> Result<(), ResultError> {
    let quarter = read_json(output, "quarter/quarter.json")?;
    quarter_periods(field(&quarter, "periods")?)?;
    for (offset, &cutoff) in CUTOFFS[1..].iter().enumerate() {
        let index = offset + 1;
        lines.push(check_close(
            output,
            &format!("quarter/{cutoff}/close/close-review-pack.json"),
            &format!("Quarter {cutoff}"),
        )?);
        if index > 1 {
            let comparison = read_json(output, &format!("quarter/{cutoff}/comparison.json"))?;
            let prev_ok = field(&comparison, "previous_period")?.is_str(CUTOFFS[index - 1]);
            let curr_ok = field(&comparison, "current_period")?.is_str(cutoff);
            require(prev_ok && curr_ok, "Quarter comparison periods differ")?;
            let statuses_ok = field(&comparison, "previous_status")?.is_str("REVIEW")
                && field(&comparison, "current_status")?.is_str("REVIEW");
            require(statuses_ok, "Quarter comparisons must retain REVIEW")?;
        }
    }
    lines.push("Quarter cash: AUD 20,000 / 22,000 / 24,500 / 29,500.".to_owned());
    Ok(())
}

fn validate_job_cash(output: &Path, lines: &mut Vec<String>) -> Result<(), ResultError> {
    for (name, expected_cash) in [("job-base", "-434500"), ("job-extra-cost", "-544500")] {
        let doc = read_json(output, &format!("{name}/project-cash.json"))?;
        let weeks = field(&doc, "weeks")?.as_array().ok_or_else(|| {
            ResultError::Validation("WIP cash needs thirteen ordered weeks".into())
        })?;
        require(weeks.len() == 13, "WIP cash needs thirteen ordered weeks")?;
        for (i, week) in weeks.iter().enumerate() {
            let expected_week = i64::try_from(i + 1).expect("week index");
            let actual_week = field(week, "week")?.as_plain_int();
            require(
                actual_week.is_some_and(|w| *w == expected_week.into()),
                "WIP cash needs thirteen ordered weeks",
            )?;
            let expected_val = if i < 4 {
                dec("-140000")
            } else if i < 10 {
                dec("5500")
            } else {
                dec(expected_cash)
            };
            require(
                json_decimal(field(week, "ending_cash")?)? == expected_val,
                "WIP cash differs from the fixture",
            )?;
        }
    }
    lines.push(
        "Project cash at week 13: AUD -434,500 baseline; AUD -544,500 with extra completion costs."
            .to_owned(),
    );
    Ok(())
}

fn check_grant_findings(
    findings: &Json,
    expected: &BTreeSet<(&str, &str)>,
) -> Result<(), ResultError> {
    let items = findings
        .as_array()
        .ok_or_else(|| ResultError::Validation("Grant source findings changed".into()))?;
    require(
        items.len() == expected.len(),
        "Grant source findings changed",
    )?;
    let mut seen = BTreeSet::new();
    for item in items {
        let code = json_str(field(item, "code")?)?;
        let reference = json_str(field(item, "reference")?)?;
        seen.insert((code, reference));
    }
    let expected_owned: BTreeSet<(String, String)> = expected
        .iter()
        .map(|(c, r)| ((*c).to_owned(), (*r).to_owned()))
        .collect();
    require(seen == expected_owned, "Grant source findings changed")
}

fn validate_grant_cash(output: &Path, lines: &mut Vec<String>) -> Result<(), ResultError> {
    let cash = read_json(output, "grant-cash.json")?;
    let workpaper = read_json(output, "grants/workpaper.json")?;
    require(
        field(&cash, "workpaper_status")?.is_str("REVIEW")
            && field(&workpaper, "status")?.is_str("REVIEW"),
        "Grant REVIEW status was lost",
    )?;
    let expected: BTreeSet<(&str, &str)> = GRANT_FINDINGS.into_iter().collect();
    check_grant_findings(field(&cash, "source_findings")?, &expected)?;
    check_grant_findings(field(&workpaper, "findings")?, &expected)?;
    let forecast = field(&cash, "forecast")?;
    let liquidity = field(&cash, "liquidity")?;
    require(
        json_decimal(field(forecast, "closing_cash")?)? == dec("34800")
            && field(liquidity, "status")?.is_str("NO_SHORTFALL"),
        "Grant cash differs from the fixture",
    )?;
    let unplanned = field(&cash, "unplanned_commitments")?
        .as_array()
        .ok_or_else(|| ResultError::Validation("Grant unresolved commitments changed".into()))?;
    let commitments_ok = json_decimal(field(&cash, "known_unpaid_allocations")?)? == dec("3200")
        && json_decimal(field(&cash, "planned_commitment_cash")?)? == dec("200")
        && unplanned.len() == 1
        && field(&unplanned[0], "allocation_id")?.is_str("A4")
        && json_decimal(field(&unplanned[0], "amount")?)? == dec("3000");
    require(commitments_ok, "Grant unresolved commitments changed")?;
    lines.push(
        "Grant closing cash: AUD 34,800; NO_SHORTFALL. REVIEW remains with five source findings and AUD 3,000 unplanned commitments.".to_owned(),
    );
    Ok(())
}

/// Return display lines only after every selected route passes its contract.
pub fn validate(
    output: &Path,
    workflow: &str,
    calls: &[CallRecord],
    projects: &BTreeMap<String, Option<String>>,
) -> Result<Vec<String>, ResultError> {
    let routes: Vec<&'static str> = if workflow == "all" {
        ROUTES.to_vec()
    } else {
        vec![
            ROUTES
                .iter()
                .copied()
                .find(|&r| r == workflow)
                .ok_or_else(|| ResultError::Validation(format!("Unknown workflow: {workflow}")))?,
        ]
    };
    validate_call_coverage(&routes, calls, projects)?;
    let mut lines = Vec::new();
    if routes.contains(&"close-forecast") {
        validate_close_forecast(output, &mut lines)?;
    }
    if routes.contains(&"quarter") {
        validate_quarter(output, &mut lines)?;
    }
    if routes.contains(&"job-cash") {
        validate_job_cash(output, &mut lines)?;
    }
    if routes.contains(&"grant-cash") {
        validate_grant_cash(output, &mut lines)?;
    }
    Ok(lines)
}

fn safe_text(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    ' ' | '.' | ',' | ';' | ':' | '_' | '(' | ')' | '/' | '=' | '-'
                )
            {
                c
            } else {
                '?'
            }
        })
        .take(240)
        .collect()
}

pub fn write_platform_text(path: &Path, text: &str) -> io::Result<()> {
    let payload = if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.to_owned()
    };
    fs::write(path, payload)
}

/// Write bounded, escaped summary text; never include subprocess diagnostics.
pub fn summary(
    output: &Path,
    calls: &[CallRecord],
    projects: &BTreeMap<String, Option<String>>,
    lines: Option<&[String]>,
    failure: Option<&str>,
) -> io::Result<()> {
    let complete = lines.is_some() && failure.is_none();
    let ok_count = calls
        .iter()
        .filter(|c| c.failure.is_none() && c.exit_code == Some(c.expected_exit))
        .count();
    let mut text = vec![
        "# Joined accounting examples".to_owned(),
        String::new(),
        if complete {
            "Fixture checks passed."
        } else {
            "Run failed. Results are not verified."
        }
        .to_owned(),
        String::new(),
        format!(
            "Completed commands: {ok_count} of {} attempted.",
            calls.len()
        ),
        String::new(),
    ];
    if let Some(reason) = failure.filter(|s| !s.is_empty()) {
        text.push(format!("Failure: {}", safe_text(reason)));
        text.push(String::new());
    }
    for line in lines.unwrap_or(&[]) {
        text.push(format!("- {}", safe_text(line)));
    }
    text.extend([String::new(), "Source revisions:".to_owned(), String::new()]);
    for (owner, revision) in projects {
        let rev = revision
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("uncommitted");
        text.push(format!("- {}: {}", safe_text(owner), safe_text(rev)));
    }
    text.extend([
        String::new(),
        "Fabricated examples only. Passing integration does not approve a close, grant acquittal or payment.".to_owned(),
        String::new(),
    ]);
    write_platform_text(&output.join("summary.md"), &text.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::temp;

    fn save(root: &Path, name: &str, value: &Json) {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, pyjson::pretty_ordered(value)).unwrap();
    }

    fn calls_for(workflow: &str) -> (Vec<CallRecord>, BTreeMap<String, Option<String>>) {
        let selected = if workflow == "all" {
            ROUTES.to_vec()
        } else {
            vec![workflow]
        };
        let mut rows = Vec::new();
        let mut projects = BTreeMap::new();
        for route in selected {
            for &(owner, count) in route_counts(route).unwrap() {
                projects.insert(owner.to_owned(), Some("a".repeat(40)));
                for _ in 0..count {
                    rows.push(CallRecord {
                        owner: owner.to_owned(),
                        kind: "workflow".to_owned(),
                        arguments: Vec::new(),
                        exit_code: Some(0),
                        expected_exit: 0,
                        elapsed_seconds: 0.0,
                        failure: None,
                    });
                }
            }
        }
        for owner in projects.keys() {
            rows.push(CallRecord {
                owner: owner.clone(),
                kind: "runtime".to_owned(),
                arguments: Vec::new(),
                exit_code: Some(0),
                expected_exit: 0,
                elapsed_seconds: 0.0,
                failure: None,
            });
        }
        (rows, projects)
    }

    fn seed_fixture_results(root: &Path) {
        let pack = Json::object([
            ("overall_status", Json::str("REVIEW")),
            ("exceptions", Json::Array(vec![Json::object([])])),
        ]);
        save(root, "lumbridge-close/close-review-pack.json", &pack);
        save(
            root,
            "lumbridge-refresh/review.json",
            &Json::object([
                ("closed_through", Json::str("2026-10-31")),
                (
                    "actuals",
                    Json::object([("ending_cash", Json::str("-1960"))]),
                ),
            ]),
        );
        seed_quarter_results(root, &pack);
        seed_job_results(root);
        seed_grant_results(root);
    }

    fn seed_quarter_results(root: &Path, pack: &Json) {
        let periods = CUTOFFS
            .iter()
            .zip(CASH.iter())
            .map(|(date, cash)| {
                Json::object([
                    ("cutoff", Json::str(date)),
                    ("balances", Json::object([("cash", Json::str(cash))])),
                ])
            })
            .collect();
        save(
            root,
            "quarter/quarter.json",
            &Json::object([("periods", Json::Array(periods))]),
        );
        for (idx, cutoff) in CUTOFFS[1..].iter().enumerate() {
            let index = idx + 1;
            save(
                root,
                &format!("quarter/{cutoff}/close/close-review-pack.json"),
                pack,
            );
            if index > 1 {
                save(
                    root,
                    &format!("quarter/{cutoff}/comparison.json"),
                    &Json::object([
                        ("previous_period", Json::str(CUTOFFS[index - 1])),
                        ("current_period", Json::str(cutoff)),
                        ("previous_status", Json::str("REVIEW")),
                        ("current_status", Json::str("REVIEW")),
                    ]),
                );
            }
        }
    }

    fn seed_job_results(root: &Path) {
        for (label, cash) in [("job-base", "-434500"), ("job-extra-cost", "-544500")] {
            let weeks = (1..=13)
                .map(|i| {
                    let val = if i <= 4 {
                        "-140000"
                    } else if i <= 10 {
                        "5500"
                    } else {
                        cash
                    };
                    Json::object([("week", Json::int(i)), ("ending_cash", Json::str(val))])
                })
                .collect();
            save(
                root,
                &format!("{label}/project-cash.json"),
                &Json::object([("weeks", Json::Array(weeks))]),
            );
        }
    }

    fn seed_grant_results(root: &Path) {
        let findings: Vec<Json> = GRANT_FINDINGS
            .iter()
            .map(|(code, r)| Json::object([("code", Json::str(code)), ("reference", Json::str(r))]))
            .collect();
        save(
            root,
            "grants/workpaper.json",
            &Json::object([
                ("status", Json::str("REVIEW")),
                ("findings", Json::Array(findings.clone())),
            ]),
        );
        save(
            root,
            "grant-cash.json",
            &Json::object([
                ("workpaper_status", Json::str("REVIEW")),
                ("source_findings", Json::Array(findings)),
                (
                    "forecast",
                    Json::object([("closing_cash", Json::str("34800"))]),
                ),
                (
                    "liquidity",
                    Json::object([("status", Json::str("NO_SHORTFALL"))]),
                ),
                ("known_unpaid_allocations", Json::str("3200")),
                ("planned_commitment_cash", Json::str("200")),
                (
                    "unplanned_commitments",
                    Json::Array(vec![Json::object([
                        ("allocation_id", Json::str("A4")),
                        ("amount", Json::str("3000")),
                    ])]),
                ),
            ]),
        );
    }

    #[test]
    fn complete_selected_routes_validate_and_render() {
        for workflow in ["all", "quarter", "close-forecast", "job-cash", "grant-cash"] {
            let temp = temp::TempDir::new();
            seed_fixture_results(&temp.0);
            let (calls, projects) = calls_for(workflow);
            let lines = validate(&temp.0, workflow, &calls, &projects).unwrap();
            summary(&temp.0, &calls, &projects, Some(&lines), None).unwrap();
            let text = fs::read_to_string(temp.join("summary.md")).unwrap();
            assert!(text.contains("Fixture checks passed"), "{workflow}: {text}");
            assert!(text.contains("does not approve"));
            assert!(text.contains(&"a".repeat(40)));
        }
    }

    #[test]
    fn changed_outputs_are_rejected() {
        let mutations: Vec<(&str, &str)> = vec![
            ("quarter/quarter.json", r#"{"periods":[]}"#),
            (
                "quarter/2026-10-31/close/close-review-pack.json",
                r#"{"overall_status":"PASS","exceptions":[{}]}"#,
            ),
            (
                "quarter/2026-11-30/comparison.json",
                r#"{"previous_period":"2026-10-31","current_period":"2026-12-31","previous_status":"REVIEW","current_status":"REVIEW"}"#,
            ),
            (
                "lumbridge-close/close-review-pack.json",
                r#"{"overall_status":"REVIEW","exceptions":[]}"#,
            ),
            (
                "lumbridge-refresh/review.json",
                r#"{"closed_through":"2026-10-31","actuals":{"ending_cash":"0"}}"#,
            ),
            ("job-base/project-cash.json", r#"{"weeks":[]}"#),
            ("grant-cash.json", r#"{"workpaper_status":"RECONCILED"}"#),
            (
                "grants/workpaper.json",
                r#"{"status":"RECONCILED","findings":[]}"#,
            ),
        ];
        for (name, replacement) in mutations {
            let temp = temp::TempDir::new();
            seed_fixture_results(&temp.0);
            fs::write(temp.join(name), replacement).unwrap();
            let (calls, projects) = calls_for("all");
            assert!(
                matches!(
                    validate(&temp.0, "all", &calls, &projects),
                    Err(ResultError::Validation(_))
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn mutated_quarter_cash_and_traversal_cutoff_are_rejected() {
        for (cutoff, cash) in [("../../elsewhere", "20000"), ("2026-09-30", "22001")] {
            let temp = temp::TempDir::new();
            seed_fixture_results(&temp.0);
            let mut periods = Vec::new();
            for (i, (&c, &v)) in CUTOFFS.iter().zip(CASH.iter()).enumerate() {
                let c_val = if i == 0 { cutoff } else { c };
                let v_val = if i == 1 { cash } else { v };
                periods.push(Json::object([
                    ("cutoff", Json::str(c_val)),
                    ("balances", Json::object([("cash", Json::str(v_val))])),
                ]));
            }
            save(
                &temp.0,
                "quarter/quarter.json",
                &Json::object([("periods", Json::Array(periods))]),
            );
            let (calls, projects) = calls_for("all");
            assert!(matches!(
                validate(&temp.0, "all", &calls, &projects),
                Err(ResultError::Validation(_))
            ));
        }
    }

    #[test]
    fn missing_command_and_missing_output_are_rejected() {
        for kind in ["workflow", "runtime"] {
            let temp = temp::TempDir::new();
            seed_fixture_results(&temp.0);
            let (mut calls, projects) = calls_for("all");
            let pos = calls.iter().position(|c| c.kind == kind).unwrap();
            calls.remove(pos);
            let err = validate(&temp.0, "all", &calls, &projects).unwrap_err();
            assert!(err.to_string().contains("coverage"), "{err}");
        }
        let temp = temp::TempDir::new();
        seed_fixture_results(&temp.0);
        fs::remove_file(temp.join("quarter/2026-12-31/comparison.json")).unwrap();
        let (calls, projects) = calls_for("all");
        assert!(matches!(
            validate(&temp.0, "all", &calls, &projects),
            Err(ResultError::Io(_))
        ));
    }

    #[test]
    fn failure_summary_cannot_claim_success_or_inject_markup() {
        let temp = temp::TempDir::new();
        let mut projects = BTreeMap::new();
        projects.insert("close".to_owned(), Some("<script>\n# injected".to_owned()));
        summary(&temp.0, &[], &projects, None, Some("<script>\n# failed")).unwrap();
        let text = fs::read_to_string(temp.join("summary.md")).unwrap();
        assert!(text.contains("Results are not verified"));
        assert!(!text.contains("<script>"));
        assert!(!text.contains("\n# injected"));
        assert!(!text.contains("Fixture checks passed"));
    }
}
