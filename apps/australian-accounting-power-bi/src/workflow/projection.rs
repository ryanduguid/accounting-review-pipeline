use super::{BOUNDARY, Result, digest, json, ledger, run};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

const CONTEXT: [&str; 6] = ["RunID", "Entity", "Tenant", "Period", "Basis", "Currency"];
const FINDING: [&str; 14] = [
    "ExceptionKey",
    "Control",
    "Status",
    "AccountID",
    "Account",
    "Current",
    "Prior",
    "Difference",
    "Threshold",
    "Reason",
    "Action",
    "Question",
    "EvidenceRequested",
    "EvidenceState",
];
const EVIDENCE: [&str; 7] = [
    "ExceptionKey",
    "AccountID",
    "TransactionID",
    "Date",
    "Reference",
    "Description",
    "Amount",
];
const RECONCILED: &str = "Journal rows reconciled";
const UNAVAILABLE: &str = "Line-level evidence is unavailable under this control contract";

fn text(value: &Value, name: &str) -> String {
    value[name].as_str().expect("validated projection field").to_owned()
}

pub fn projection(path: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let verified = run::verify(path)?;
    let receipt = &verified.receipt;
    let snapshot = &verified.snapshot;
    let close = run::snapshot_file(snapshot, "close/close-review-pack.json")?;
    let doc = json::parse(close, "Close pack")?;
    let drivers = json::parse(
        run::snapshot_file(snapshot, "drivers/variance-drivers.json")?,
        "Driver pack",
    )?;
    let readiness = json::parse(
        run::snapshot_file(snapshot, "readiness/readiness-pack.json")?,
        "Readiness pack",
    )?;
    let case = &receipt["case"];
    let context = vec![
        digest(close),
        text(case, "entity"),
        text(case, "tenant"),
        text(receipt, "period"),
        text(case, "basis"),
        text(case, "currency"),
    ];
    let FindingRows {
        findings,
        evidence,
        unmapped,
    } = finding_rows(&context, &doc, &drivers)?;
    if findings.is_empty() {
        return Err("The fixed review case requires at least one finding.".into());
    }
    let (run_columns, run_row) = run_row(context, receipt, &doc, &readiness, snapshot, unmapped)?;
    let finding_columns: Vec<_> = CONTEXT.into_iter().chain(FINDING).collect();
    let evidence_columns: Vec<_> = CONTEXT.into_iter().chain(EVIDENCE).collect();
    Ok(BTreeMap::from([
        ("sample-review-run.csv".into(), ledger::csv(&run_columns, &[run_row])),
        (
            "sample-review-exceptions.csv".into(),
            ledger::csv(&finding_columns, &findings),
        ),
        (
            "sample-review-evidence.csv".into(),
            ledger::csv(&evidence_columns, &evidence),
        ),
    ]))
}

fn identity<'a>(value: &'a Value, fields: &[&str]) -> Vec<&'a str> {
    fields
        .iter()
        .map(|field| value[*field].as_str().expect("validated identity"))
        .collect()
}

struct FindingRows {
    findings: Vec<Vec<String>>,
    evidence: Vec<Vec<String>>,
    unmapped: usize,
}

fn finding_rows(context: &[String], doc: &Value, drivers: &Value) -> Result<FindingRows> {
    let driver_index: BTreeMap<_, _> = drivers["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|account| (identity(account, &["tenant", "account_id"]), account))
        .collect();
    let query_index: BTreeMap<_, _> = doc["client_queries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|query| (identity(query, &["control", "tenant", "account_id"]), query))
        .collect();
    let (mut findings, mut evidence, mut unmapped) = (Vec::new(), Vec::new(), 0);
    for (index, item) in doc["exceptions"].as_array().unwrap().iter().enumerate() {
        let key = format!("{}:{index}", context[0]);
        let query = query_index
            .get(&identity(item, &["control", "tenant", "account_id"]))
            .copied();
        let driver = if item["control"] == "period_variance" {
            driver_index.get(&identity(item, &["tenant", "account_id"])).copied()
        } else {
            None
        };
        let available = match driver {
            Some(driver) => {
                ledger::money(driver["unexplained"].as_str().unwrap())?.is_zero()
                    && driver["transactions_in_window"].as_u64()
                        == Some(driver["drivers"].as_array().unwrap().len() as u64)
            }
            None => false,
        };
        unmapped += usize::from(!available);
        findings.push(finding_row(context, &key, item, query, available));
        if available {
            for driver in driver.unwrap()["drivers"].as_array().unwrap() {
                let mut row = context.to_vec();
                row.extend([key.clone(), text(item, "account_id")]);
                row.extend(
                    ["TransactionID", "Date", "Reference", "Description", "Amount"].map(|name| text(driver, name)),
                );
                evidence.push(row);
            }
        }
    }
    Ok(FindingRows {
        findings,
        evidence,
        unmapped,
    })
}

fn finding_row(context: &[String], key: &str, item: &Value, query: Option<&Value>, available: bool) -> Vec<String> {
    let mut row = context.to_vec();
    row.push(key.to_owned());
    row.extend(
        [
            "control",
            "status",
            "account_id",
            "account_name",
            "current_value",
            "prior_value",
            "difference",
            "threshold",
            "reason",
            "reviewer_action",
        ]
        .map(|name| text(item, name)),
    );
    row.extend(["question", "evidence_requested"].map(|name| query.map(|query| text(query, name)).unwrap_or_default()));
    row.push(if available { RECONCILED } else { UNAVAILABLE }.into());
    row
}

fn run_row(
    mut context: Vec<String>,
    receipt: &Value,
    doc: &Value,
    ready: &Value,
    snapshot: &ledger::Snapshot,
    unmapped: usize,
) -> Result<(Vec<&'static str>, Vec<String>)> {
    let columns = [
        "CloseStatus",
        "ReadinessStatus",
        "PriorPeriod",
        "AbsoluteThreshold",
        "PercentageThreshold",
        "ControlsNotRun",
        "ReadinessControlsNotRun",
        "Acknowledgement",
        "UnmappedExceptions",
        "SourceTBHash",
        "SourceGLHash",
        "Verification",
        "Boundary",
    ];
    context.extend([
        text(doc, "overall_status"),
        text(ready, "overall_status"),
        doc["prior_report_dates"][0].as_str().unwrap().to_owned(),
        text(&doc["thresholds"], "absolute_variance"),
        text(&doc["thresholds"], "percentage_variance"),
        doc["controls_not_run"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>()
            .join("; "),
        json::inline(&ready["controls_not_run"]),
        json::inline(&doc["acknowledgement"]),
        unmapped.to_string(),
        digest(run::snapshot_file(snapshot, "inputs/trial_balance.csv")?),
        text(&receipt["case"]["source_sha256"], "sample-general-ledger.csv"),
        "CLI verified synthetic case; unsigned local receipt".into(),
        BOUNDARY.into(),
    ]);
    Ok((CONTEXT.into_iter().chain(columns).collect(), context))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn journal_evidence_requires_the_explicit_control_and_complete_driver_population() {
        let item = json!({"control":"period_variance","tenant":"demo","account_id":"200","account_name":"Sales","status":"REVIEW","current_value":"100.00","prior_value":"0.00","difference":"100.00","threshold":"10.00","reason":"difference","reviewer_action":"inspect"});
        let account = json!({"tenant":"demo","account_id":"200","unexplained":"0.00","transactions_in_window":1,"drivers":[{"TransactionID":"journal-1","Date":"2024-09-01","Reference":"sale","Description":"Fabricated sale","Amount":"100.00"}]});
        for (control, account_id, unexplained, count, available) in [
            ("period_variance", "200", "0.00", 1, true),
            ("unsupported_control", "200", "0.00", 1, false),
            ("period_variance", "unmatched", "0.00", 1, false),
            ("period_variance", "200", "0.01", 1, false),
            ("period_variance", "200", "0.00", 2, false),
        ] {
            let (mut item, mut account) = (item.clone(), account.clone());
            item["control"] = control.into();
            account["account_id"] = account_id.into();
            account["unexplained"] = unexplained.into();
            account["transactions_in_window"] = count.into();
            let FindingRows {
                findings,
                evidence,
                unmapped,
            } = finding_rows(
                &vec!["run".into(); 6],
                &json!({"exceptions":[item],"client_queries":[]}),
                &json!({"accounts":[account]}),
            )
            .unwrap();
            assert_eq!(unmapped, usize::from(!available));
            assert_eq!(evidence.len(), usize::from(available));
            assert_eq!(
                findings[0].last().unwrap(),
                if available { RECONCILED } else { UNAVAILABLE }
            );
            if available {
                assert_eq!(evidence[0][8], "journal-1");
            }
        }
    }
}
