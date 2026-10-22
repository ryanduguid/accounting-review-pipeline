use super::{BOUNDARY, Result, json, ledger, producer, projection, run, schema};
use ledger::{Row, Snapshot};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};

/// Sealed report policy. The rendered page, including the copy stored in an
/// export, matches the frozen Python report byte for byte. Every interpolated
/// value is escaped and the page loads no script.
const REPORT_POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'";

fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn field(value: &Value, key: &str) -> String {
    esc(value[key].as_str().expect("validated display field"))
}

pub fn render(run: &Path, bin_dir: &Path, previous: Option<&Path>) -> Result<String> {
    let verified = run::verify(run)?;
    let manifests = &verified.receipt["producer_manifests"];
    for (command, folder) in [("close-control", "close"), ("review-ready", "readiness")] {
        producer::pinned(
            bin_dir,
            command,
            &[
                "view".into(),
                "--pack-dir".into(),
                run.join(folder).to_string_lossy().into_owned(),
            ],
            &manifests[command],
        )?;
    }
    let projected = projection::projection(run)?;
    let comparison = previous
        .map(|previous| changes(run, previous, bin_dir, &verified))
        .transpose()?;
    if run::verify(run)?.snapshot != verified.snapshot {
        return Err("Current run changed during display.".into());
    }
    document(&projected, &verified.receipt, comparison.as_ref())
}

fn changes(run: &Path, previous: &Path, bin_dir: &Path, verified: &run::Verified) -> Result<(String, Value)> {
    let old = run::verify(previous)?;
    let receipt = &verified.receipt;
    if old.receipt["case"] != receipt["case"] {
        return Err("Comparison context differs.".into());
    }
    let manifest = &receipt["producer_manifests"]["close-control"];
    if old.receipt["producer_manifests"]["close-control"] != *manifest {
        return Err("Comparison producer installed identities differ.".into());
    }
    let args = [
        "compare".into(),
        "--previous-pack".into(),
        previous.join("close").to_string_lossy().into_owned(),
        "--current-pack".into(),
        run.join("close").to_string_lossy().into_owned(),
        "--previous-tb".into(),
        previous.join("inputs/trial_balance.csv").to_string_lossy().into_owned(),
        "--current-tb".into(),
        run.join("inputs/trial_balance.csv").to_string_lossy().into_owned(),
    ];
    let comparison = json::parse(
        &producer::pinned(bin_dir, "close-control", &args, manifest)?,
        "Comparison",
    )?;
    schema::comparison(&comparison)?;
    let names: BTreeSet<_> = receipt["files"]
        .as_object()
        .unwrap()
        .keys()
        .chain(old.receipt["files"].as_object().unwrap().keys())
        .collect();
    let changed: Vec<_> = names
        .into_iter()
        .filter(|name| name.starts_with("inputs/") && receipt["files"].get(*name) != old.receipt["files"].get(*name))
        .collect();
    let close = |snapshot: &Snapshot| {
        json::parse(
            run::snapshot_file(snapshot, "close/close-review-pack.json")?,
            "Close pack",
        )
    };
    let change = json!({"changed_inputs":changed,"findings":comparison["findings"],"queries":comparison["queries"],"scope_changes":comparison["scope_changes"],"acknowledgement_changed":!json::equal(&close(&old.snapshot)?["acknowledgement"], &close(&verified.snapshot)?["acknowledgement"]),"meaning":comparison["review_boundary"]});
    if run::verify(previous)?.snapshot != old.snapshot {
        return Err("Previous run changed during display.".into());
    }
    Ok((old.receipt["period"].as_str().unwrap().to_owned(), change))
}

fn document(projected: &Snapshot, receipt: &Value, comparison: Option<&(String, Value)>) -> Result<String> {
    let findings = ledger::rows(&projected["sample-review-exceptions.csv"])?;
    let evidence = ledger::rows(&projected["sample-review-evidence.csv"])?;
    let context = ledger::rows(&projected["sample-review-run.csv"])?;
    let context = &context[0];
    let currency = esc(&context["Currency"]);
    let index_links: String = findings
        .iter()
        .enumerate()
        .map(|(index, item)| {
            format!(
                "<li><a href=\"#finding-{}\">{}<span>{} · {}</span></a></li>",
                index + 1,
                esc(&item["Account"]),
                esc(&item["Control"]),
                esc(&item["Status"])
            )
        })
        .collect();
    let sections: String = findings
        .iter()
        .enumerate()
        .map(|(index, item)| finding(index + 1, item, findings.get(index + 1), &evidence, &currency))
        .collect();
    let csv = std::str::from_utf8(&projected["sample-review-run.csv"]).map_err(|error| error.to_string())?;
    let headers = super::pycsv::Reader::new(csv)
        .next()
        .unwrap()
        .map_err(|_| "Invalid projection header.")?;
    let metadata: String = headers
        .iter()
        .map(|key| format!("<dt>{}</dt><dd>{}</dd>", esc(key), esc(&context[key])))
        .collect();
    let heading_context = ["Tenant", "Period", "Basis"].map(|key| esc(&context[key])).join(" · ");
    let (comparison_html, comparison_record) = comparison_fragments(&context["Period"], comparison);
    Ok(format!(
        include_str!("review.html"),
        heading_context = heading_context,
        boundary = esc(BOUNDARY),
        finding_count = findings.len(),
        currency = currency,
        index_links = index_links,
        sections = sections,
        metadata = metadata,
        comparison_html = comparison_html,
        comparison_record = comparison_record,
        hashes = esc(&json::pretty(&receipt["files"])),
        content_security_policy = REPORT_POLICY,
    ))
}

fn comparison_fragments(current: &str, comparison: Option<&(String, Value)>) -> (String, String) {
    match comparison {
        None => ("<p>No previous verified run supplied.</p>".into(), String::new()),
        Some((old_period, change)) => (
            comparison_html(current, old_period, change),
            format!(
                "<section id=\"comparison-record\" tabindex=\"-1\"><h2>Full comparison record</h2><pre>{}</pre><nav class=\"finding-nav\" aria-label=\"Return from comparison record\"><a href=\"#run-changes\">Back to comparison summary</a></nav></section>",
                esc(&json::pretty(change))
            ),
        ),
    }
}

fn finding(number: usize, item: &Row, following: Option<&Row>, evidence: &[Row], currency: &str) -> String {
    let fields = [
        ("Action", "Review action"),
        ("Reason", "Reason"),
        ("Account", "Account"),
        ("Status", "Status"),
        ("Current", "Current"),
        ("Prior", "Prior"),
        ("Difference", "Difference"),
        ("Threshold", "Threshold"),
        ("Question", "Question"),
        ("EvidenceRequested", "Evidence requested"),
        ("EvidenceState", "Evidence state"),
    ];
    let detail: String = fields
        .iter()
        .map(|(key, label)| format!("<dt>{label}</dt><dd>{}</dd>", esc(&item[*key])))
        .collect();
    let rows: Vec<_> = evidence
        .iter()
        .filter(|row| row["ExceptionKey"] == item["ExceptionKey"])
        .collect();
    let journal = journal(number, &rows, currency);
    let following = following
        .map(|item| {
            format!(
                "<a href=\"#finding-{}\">Next finding: {}</a>",
                number + 1,
                esc(&item["Account"])
            )
        })
        .unwrap_or_default();
    format!(
        "<article class=\"finding\" id=\"finding-{number}\" tabindex=\"-1\" aria-labelledby=\"finding-{number}-title\"><h2 id=\"finding-{number}-title\">{number}. {}</h2><p class=\"control\">Control: {}</p><dl>{detail}</dl><h3>Journal evidence</h3>{journal}<nav class=\"finding-nav\" aria-label=\"Navigation after finding {number}\"><a href=\"#finding-index\">Back to finding index</a>{following}</nav></article>",
        esc(&item["Account"]),
        esc(&item["Control"])
    )
}

fn journal(number: usize, rows: &[&Row], currency: &str) -> String {
    if rows.is_empty() {
        return "<p>No journal rows are supplied for this finding. See the evidence state above.</p>".into();
    }
    let lines: String = rows
        .iter()
        .map(|row| {
            let cells: String = ["TransactionID", "Date", "Reference", "Description", "Amount"]
                .iter()
                .map(|key| {
                    format!(
                        "<td{}>{}</td>",
                        if *key == "Amount" { " class=\"amount\"" } else { "" },
                        esc(&row[*key])
                    )
                })
                .collect();
            format!("<tr>{cells}</tr>")
        })
        .collect();
    let count = rows.len();
    let suffix = if count == 1 { "row" } else { "rows" };
    format!(
        "<p class=\"scroll-hint\">On a narrow screen, scroll the journal table sideways to read every column.</p><div class=\"journal-scroll\" tabindex=\"0\" role=\"region\" aria-label=\"Journal evidence for finding {number}\"><table><caption>{count} reconciled journal {suffix}. Amounts in {currency}.</caption><thead><tr><th scope=\"col\">Journal line</th><th scope=\"col\">Date</th><th scope=\"col\">Reference</th><th scope=\"col\">Description</th><th scope=\"col\" class=\"amount\">Amount</th></tr></thead><tbody>{lines}</tbody></table></div>"
    )
}

const LABELS: [(&str, &str); 5] = [
    ("NEW", "New"),
    ("CHANGED", "Changed"),
    ("RECURRING", "Recurring"),
    ("NOT_RAISED", "Not raised"),
    ("NOT_COMPARABLE", "Not comparable"),
];

fn label(row: &Value) -> &'static str {
    LABELS
        .iter()
        .find(|(key, _)| row["change"] == *key)
        .expect("validated classification")
        .1
}

fn comparison_counts(findings: &[Value], queries: &[Value]) -> String {
    let counts: String = LABELS
        .iter()
        .map(|(key, label)| {
            let cells: String = [findings, queries]
                .map(|rows| {
                    format!(
                        "<td class=\"amount\">{}</td>",
                        rows.iter().filter(|row| row["change"] == *key).count()
                    )
                })
                .concat();
            format!("<tr><th scope=\"row\" data-change=\"{key}\">{label}</th>{cells}</tr>")
        })
        .collect();
    format!(
        "<div class=\"journal-scroll comparison\" tabindex=\"0\" role=\"region\" aria-label=\"Change counts\"><table><caption>Change counts</caption><thead><tr><th scope=\"col\">Classification</th><th scope=\"col\" class=\"amount\">Finding groups</th><th scope=\"col\" class=\"amount\">Queries</th></tr></thead><tbody>{counts}</tbody></table></div>"
    )
}

fn comparison_html(current: &str, previous: &str, change: &Value) -> String {
    let (findings, queries) = (
        change["findings"].as_array().unwrap(),
        change["queries"].as_array().unwrap(),
    );
    let table = comparison_counts(findings, queries);
    let finding_changes: String = findings
        .iter()
        .map(|row| comparison_finding(row, current, previous))
        .collect();
    let query_changes: String = queries.iter().map(comparison_query).collect();
    let list = |title: &str, body: String, empty: &str| {
        if body.is_empty() {
            empty.to_owned()
        } else {
            format!("<h3>{title}</h3><ul class=\"comparison-records\" role=\"list\">{body}</ul>")
        }
    };
    let joined = |key: &str, empty: &str| {
        let text = change[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>()
            .join(", ");
        esc(if text.is_empty() { empty } else { &text })
    };
    format!(
        "<p>Previous run: <strong>{}</strong>. Current run: <strong>{}</strong>.</p><p>{}</p>{table}<p>Change labels compare complete records. Changed does not necessarily mean a balance or status changed. Recurring means the complete record is identical.</p><dl><dt>Scope changes</dt><dd>{}</dd><dt>Changed input files</dt><dd>{}</dd><dt>Acknowledgement changed</dt><dd>{}</dd></dl>{}<p>Values and statuses retain the producer's exact text and units. Each record is shown separately. Full reasons, actions and query details appear in the full comparison record.</p>{}<nav class=\"finding-nav\" aria-label=\"Comparison details\"><a href=\"#comparison-record\">Read full comparison record</a></nav>",
        esc(previous),
        esc(current),
        field(change, "meaning"),
        joined("scope_changes", "No scope changes reported by the producer."),
        joined("changed_inputs", "No input file changes recorded."),
        if change["acknowledgement_changed"].as_bool().unwrap() {
            "Yes"
        } else {
            "No"
        },
        list(
            "Finding comparison",
            finding_changes,
            "<p>No finding records occur in either run.</p>"
        ),
        list(
            "Query comparison",
            query_changes,
            "<p>No query records occur in either run.</p>"
        )
    )
}

fn finding_values(items: &[Value]) -> String {
    if items.is_empty() {
        return "No finding raised in this run.".into();
    }
    items.iter().map(|item| format!("<div class=\"comparison-value\"><span class=\"amount\">{}</span><span class=\"control\">Status: {}</span></div>", if item["current_value"] == "" { "Empty producer value".into() } else { field(item,"current_value") }, field(item,"status"))).collect()
}

fn comparison_finding(row: &Value, current: &str, previous: &str) -> String {
    let old = row["previous"].as_array().unwrap();
    let new = row["current"].as_array().unwrap();
    let name = if new.is_empty() { &old[0] } else { &new[0] };
    format!(
        "<li><h4>{}</h4><p class=\"control\">{} · {}</p><dl><div><dt>Change</dt><dd>{}</dd></div><div class=\"comparison-period\"><dt>Value in {}</dt><dd>{}</dd></div><div class=\"comparison-period\"><dt>Value in {}</dt><dd>{}</dd></div></dl></li>",
        field(name, "account_name"),
        field(row, "control"),
        field(row, "account_id"),
        label(row),
        esc(previous),
        finding_values(old),
        esc(current),
        finding_values(new)
    )
}

fn comparison_query(row: &Value) -> String {
    let item = if row["current"].is_null() {
        &row["previous"]
    } else {
        &row["current"]
    };
    format!(
        "<li><h4>{}</h4><p class=\"control\">{} · {}</p><dl><div><dt>Change</dt><dd>{}</dd></div><div><dt>Query ID</dt><dd>{}</dd></div></dl></li>",
        field(item, "account_name"),
        field(item, "control"),
        field(item, "account_id"),
        label(row),
        field(row, "query_id")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displayed_text_is_escaped_and_empty_evidence_is_explicit() {
        assert_eq!(esc("<&\"'>"), "&lt;&amp;&quot;&#x27;&gt;");
        assert!(journal(1, &[], "AUD").contains("No journal rows"));
        let values = finding_values(&[json!({"current_value":"<script>","status":"REVIEW"})]);
        assert!(!values.contains("<script>"));
        assert!(values.contains("&lt;script&gt;"));
        let template = include_str!("review.html");
        for retained in [
            ":focus-visible",
            "aria-label=\"Report sections\"",
            "prefers-color-scheme:dark",
            "@media print",
        ] {
            assert!(template.contains(retained));
        }
        assert!(!template.contains("unsafe-inline"));
        let rendered = format!(
            include_str!("review.html"),
            heading_context = "",
            boundary = "",
            finding_count = 0,
            currency = "",
            index_links = "",
            sections = "",
            metadata = "",
            comparison_html = "",
            comparison_record = "",
            hashes = "",
            content_security_policy = REPORT_POLICY,
        );
        let marker = format!("Content-Security-Policy\" content=\"{REPORT_POLICY}\"");
        assert_eq!(rendered.matches(&marker).count(), 1);
        assert_eq!(rendered.matches("unsafe-inline").count(), 1);
    }

    #[test]
    fn finding_destinations_are_distinct_and_all_dynamic_detail_is_escaped() {
        let hostile = "<script>&\"' same account";
        let mut item = ledger::rows(include_bytes!("../../samples/sample-review-exceptions.csv"))
            .unwrap()
            .remove(0);
        let mut evidence = ledger::rows(include_bytes!("../../samples/sample-review-evidence.csv"))
            .unwrap()
            .remove(0);
        for value in item.values_mut().chain(evidence.values_mut()) {
            *value = hostile.into();
        }
        let amount = "-123456789012345678901234567890.01";
        item.insert("Current".into(), amount.into());
        evidence.insert("Amount".into(), amount.into());
        let output = finding(1, &item, Some(&item), &[evidence.clone()], &esc(hostile))
            + &finding(2, &item, None, &[evidence], &esc(hostile));
        assert!(!output.contains("<script>"));
        assert!(output.contains(&esc(hostile)));
        assert!(output.contains(amount));
        for id in ["finding-1", "finding-2"] {
            assert!(output.contains(&format!("id=\"{id}\"")));
            assert!(output.contains(&format!("href=\"#{id}\"")) || id == "finding-1");
        }
        assert!(output.contains("scope=\"col\""));
        assert!(output.contains("Back to finding index"));
    }

    #[test]
    fn comparison_records_escape_fields_and_distinguish_an_empty_previous_run() {
        let empty = json!({"findings":[],"queries":[],"scope_changes":[],"changed_inputs":[],"acknowledgement_changed":false,"meaning":"Human review required"});
        let absent = comparison_fragments("2024-09-30", None);
        let present = comparison_fragments("2024-09-30", Some(&("2024-08-31".into(), empty)));
        assert!(absent.0.contains("No previous verified run supplied"));
        assert!(!present.0.contains("No previous verified run supplied"));
        assert!(present.0.contains("No finding records occur in either run"));
        let hostile = "<script>&\"'";
        let row = json!({"account_name":hostile,"current_value":"-123456789012345678901234567890.01","status":hostile});
        let change = json!({"findings":[{"change":"CHANGED","control":hostile,"account_id":hostile,"previous":[row.clone()],"current":[row]}],"queries":[{"change":"NEW","query_id":hostile,"previous":null,"current":{"account_name":hostile,"control":hostile,"account_id":hostile}}],"scope_changes":[hostile],"changed_inputs":["inputs/removed.txt"],"acknowledgement_changed":true,"meaning":hostile});
        let (summary, record) = comparison_fragments(hostile, Some(&(hostile.into(), change)));
        for output in [&summary, &record] {
            assert!(!output.contains("<script>"));
            assert!(output.contains("&lt;script&gt;"));
        }
        assert!(summary.contains("inputs/removed.txt"));
        assert!(summary.contains("Acknowledgement changed</dt><dd>Yes"));
        assert!(summary.contains("-123456789012345678901234567890.01"));
    }
}
