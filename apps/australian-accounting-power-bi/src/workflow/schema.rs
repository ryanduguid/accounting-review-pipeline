use super::{BOUNDARY, CLOSE_FILES, PRODUCERS, Result, digest, json};
use serde_json::Value;

pub fn fields(value: &Value, names: &[&str], label: &str) -> Result<()> {
    if !value.is_object() || names.iter().any(|name| !value[*name].is_string()) {
        return Err(format!(
            "{label} requires an object with string fields: {}.",
            names.join(", ")
        ));
    }
    Ok(())
}

pub fn exact(value: &Value, names: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.len() == names.len() && names.iter().all(|key| object.contains_key(*key)))
}

pub fn rows<'a>(value: &'a Value, label: &str) -> Result<&'a Vec<Value>> {
    value
        .as_array()
        .filter(|rows| rows.iter().all(Value::is_object))
        .ok_or_else(|| format!("{label} requires a list of objects."))
}

pub fn hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn string_map(value: &Value, label: &str, hashes: bool, required: &[&str]) -> Result<()> {
    let Some(values) = value.as_object() else {
        return Err(format!("{label} requires a string map."));
    };
    if values
        .values()
        .any(|value| !value.as_str().is_some_and(|text| !hashes || hash(text)))
    {
        return Err(format!(
            "{label} requires a string map{}.",
            if hashes { " of lowercase SHA-256 values" } else { "" }
        ));
    }
    if required.iter().any(|name| !values.contains_key(*name)) {
        return Err(format!("{label} is missing required fields."));
    }
    Ok(())
}

pub fn case(value: &Value) -> Result<()> {
    fields(
        value,
        &[
            "case_id",
            "entity",
            "tenant",
            "basis",
            "currency",
            "financial_year_start",
            "opening_journal",
            "population",
        ],
        "Case",
    )?;
    string_map(&value["account_ids"], "Case account identifiers", false, &[])?;
    string_map(&value["source_sha256"], "Case source hashes", true, &[])
}

pub fn money(value: &Value, label: &str, nullable: bool) -> Result<()> {
    if !value.as_str().is_some_and(|text| {
        if nullable && text.is_empty() {
            return true;
        }
        let unsigned = text.strip_prefix('-').unwrap_or(text);
        unsigned
            .split_once('.')
            .is_some_and(|(whole, fraction)| digits(whole) && fraction.len() == 2 && digits(fraction))
    }) {
        return Err(format!("{label} requires a two-decimal money string."));
    }
    Ok(())
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

pub fn close(value: &Value) -> Result<()> {
    fields(value, &["overall_status"], "Close pack")?;
    string_map(
        &value["source_sha256"],
        "Close source hashes",
        true,
        &["current_trial_balance", "prior_trial_balance"],
    )?;
    for name in ["current_report_dates", "prior_report_dates", "controls_not_run"] {
        if !value[name]
            .as_array()
            .is_some_and(|values| values.iter().all(Value::is_string))
        {
            return Err(format!("Close {name} requires a list of strings."));
        }
    }
    if value["current_report_dates"].as_array().unwrap().is_empty()
        || value["prior_report_dates"].as_array().unwrap().is_empty()
    {
        return Err("Close report dates cannot be empty.".into());
    }
    if value
        .get("acknowledgement")
        .is_none_or(|value| !value.is_null() && !value.is_object())
    {
        return Err("Close acknowledgement requires an object or null.".into());
    }
    thresholds(&value["thresholds"])?;
    for item in rows(&value["exceptions"], "Close exceptions")? {
        fields(
            item,
            &[
                "control",
                "tenant",
                "account_id",
                "account_name",
                "status",
                "current_value",
                "prior_value",
                "difference",
                "threshold",
                "reason",
                "reviewer_action",
            ],
            "Close exception",
        )?;
        for name in ["current_value", "prior_value", "difference", "threshold"] {
            money(&item[name], &format!("Close {name}"), true)?;
        }
    }
    queries(&value["client_queries"])
}

fn thresholds(value: &Value) -> Result<()> {
    fields(value, &["absolute_variance", "percentage_variance"], "Close thresholds")?;
    money(&value["absolute_variance"], "Absolute threshold", false)?;
    if let Some(value) = value.get("reconciliation_tolerance") {
        money(value, "Reconciliation tolerance", false)?;
    }
    if !value["percentage_variance"]
        .as_str()
        .unwrap()
        .strip_suffix('%')
        .is_some_and(|text| {
            text.split_once('.')
                .is_some_and(|(whole, fraction)| digits(whole) && fraction.len() >= 2 && digits(fraction))
        })
    {
        return Err("Percentage threshold requires decimal percentage text.".into());
    }
    Ok(())
}

fn queries(value: &Value) -> Result<()> {
    let (mut ids, mut keys) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
    for item in rows(value, "Close queries")? {
        fields(
            item,
            &[
                "query_id",
                "control",
                "tenant",
                "account_id",
                "question",
                "evidence_requested",
            ],
            "Close query",
        )?;
        let id = item["query_id"].as_str().unwrap();
        let key = (
            item["control"].as_str().unwrap(),
            item["tenant"].as_str().unwrap(),
            item["account_id"].as_str().unwrap(),
        );
        if id.is_empty() || !ids.insert(id) || !keys.insert(key) {
            return Err("Duplicate or empty close query identity.".into());
        }
    }
    Ok(())
}

pub fn drivers(value: &Value) -> Result<()> {
    if !value.is_object() {
        return Err("Driver pack requires an object.".into());
    }
    let mut required = vec!["transactions".to_owned()];
    required.extend(CLOSE_FILES.map(|name| format!("pack:{name}")));
    let keys: Vec<_> = required.iter().map(String::as_str).collect();
    string_map(&value["source_sha256"], "Driver source hashes", true, &keys)?;
    for account in rows(&value["accounts"], "Driver accounts")? {
        fields(
            account,
            &["tenant", "account_id", "movement", "transactions_total", "unexplained"],
            "Driver account",
        )?;
        if account["transactions_in_window"].as_u64().is_none() {
            return Err("Driver transaction count requires a non-negative integer.".into());
        }
        for name in ["movement", "transactions_total", "unexplained"] {
            money(&account[name], "Driver amount", false)?;
        }
        for row in rows(&account["drivers"], "Driver rows")? {
            fields(
                row,
                &["TransactionID", "Date", "Reference", "Description", "Amount"],
                "Driver row",
            )?;
            money(&row["Amount"], "Driver amount", false)?;
        }
    }
    Ok(())
}

pub fn readiness(value: &Value) -> Result<()> {
    fields(value, &["overall_status"], "Readiness pack")?;
    let names = ["slot", "filename", "reason"];
    for control in rows(&value["controls_not_run"], "Readiness controls not run")? {
        fields(control, &names, "Readiness control not run")?;
        if !exact(control, &names) || names.iter().any(|name| control[*name].as_str().unwrap().is_empty()) {
            return Err("Readiness controls not run requires exactly its non-empty producer fields.".into());
        }
    }
    Ok(())
}

pub fn producer(value: &Value, command: &str) -> Result<()> {
    let Some((_, package, module, entry)) = PRODUCERS.iter().find(|(name, ..)| *name == command) else {
        return Err("Unsupported producer command.".into());
    };
    if value["manifest_version"].as_i64() == Some(2) {
        if !exact(
            value,
            &[
                "manifest_version",
                "command",
                "launcher_name",
                "launcher_sha256",
                "native",
            ],
        ) || value["command"] != command
        {
            return Err("Unsupported native producer manifest.".into());
        }
        fields(value, &["launcher_name", "launcher_sha256"], "Native producer")?;
        fields(&value["native"], &["package", "version"], "Native producer identity")?;
        launcher(value, command)?;
        if !exact(&value["native"], &["package", "version"])
            || value["native"]["package"] != *package
            || !native_version(value["native"]["version"].as_str().unwrap())
        {
            return Err("Native producer version contract differs.".into());
        }
        return Ok(());
    }
    legacy_producer(value, command, package, module, entry)
}

pub fn native_version(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| digits(part))
        && parts[0]
            .as_bytes()
            .first()
            .is_some_and(|byte| (b'1'..=b'9').contains(byte))
}

fn launcher(value: &Value, command: &str) -> Result<()> {
    let name = value["launcher_name"].as_str().unwrap();
    if (name != command && name != format!("{command}.exe")) || !hash(value["launcher_sha256"].as_str().unwrap()) {
        return Err("Producer executable identity differs.".into());
    }
    Ok(())
}

fn legacy_producer(value: &Value, command: &str, package: &str, module: &str, entry: &str) -> Result<()> {
    if !exact(
        value,
        &[
            "manifest_version",
            "command",
            "launcher_name",
            "launcher_sha256",
            "interpreter",
            "distribution",
            "package",
        ],
    ) {
        return Err("Producer manifest requires its complete identity fields; rebuild the review run.".into());
    }
    if value["manifest_version"].as_i64() != Some(1) || value["command"] != command {
        return Err("Unsupported producer manifest.".into());
    }
    fields(value, &["launcher_name", "launcher_sha256"], "Producer launcher")?;
    launcher(value, command)?;
    let interpreter = &value["interpreter"];
    let names = ["implementation", "version", "cache_tag", "executable_sha256"];
    fields(interpreter, &names, "Producer interpreter")?;
    if !exact(interpreter, &names) || !hash(interpreter["executable_sha256"].as_str().unwrap()) {
        return Err("Unsupported interpreter identity fields.".into());
    }
    let distribution = &value["distribution"];
    let names = ["name", "version", "requires_python", "entry_point"];
    fields(distribution, &names, "Producer distribution")?;
    if !exact(distribution, &names) || distribution["name"] != package || distribution["entry_point"] != entry {
        return Err("Producer distribution contract differs.".into());
    }
    fields(&value["package"], &["name"], "Producer package")?;
    if !exact(&value["package"], &["name", "files"]) || value["package"]["name"] != module {
        return Err("Producer package contract differs.".into());
    }
    string_map(&value["package"]["files"], "Producer package files", true, &[])?;
    let files = value["package"]["files"].as_object().unwrap();
    if files.is_empty() || files.keys().any(|name| !safe_package_path(name)) {
        return Err("Producer package file paths differ.".into());
    }
    Ok(())
}

fn safe_package_path(name: &str) -> bool {
    name.split('/').all(|part| {
        !part.is_empty()
            && ![".", "..", "__pycache__"].contains(&part)
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
    })
}

pub fn receipt(value: &Value) -> Result<()> {
    if !exact(
        value,
        &[
            "schema_version",
            "case",
            "period",
            "invocations",
            "files",
            "boundary",
            "producer_manifests",
        ],
    ) {
        return Err("Receipt requires complete producer identities; rebuild the review run.".into());
    }
    if value["schema_version"].as_i64() != Some(1) || value["boundary"] != BOUNDARY {
        return Err("Unsupported review receipt.".into());
    }
    case(&value["case"])?;
    fields(value, &["period", "boundary"], "Receipt")?;
    if !["2024-08-31", "2024-09-30"].contains(&value["period"].as_str().unwrap()) {
        return Err("Receipt period is outside the fixed review case.".into());
    }
    string_map(&value["files"], "Receipt files", true, &[])?;
    let manifests = &value["producer_manifests"];
    if !exact(manifests, &["close-control", "review-ready"]) {
        return Err("Receipt requires exactly the two producer manifests.".into());
    }
    for (command, ..) in PRODUCERS {
        producer(&manifests[command], command)?;
    }
    for invocation in rows(&value["invocations"], "Receipt invocations")? {
        invocation_document(invocation, manifests)?;
    }
    Ok(())
}

pub fn comparison(value: &Value) -> Result<()> {
    fields(value, &["review_boundary"], "Comparison")?;
    let scope = value["scope_changes"]
        .as_array()
        .filter(|items| {
            items.iter().all(|item| {
                item.as_str().is_some_and(|text| {
                    !text
                        .trim_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
                        .is_empty()
                })
            })
        })
        .ok_or("Comparison scope changes requires non-empty strings.")?;
    for group in ["findings", "queries"] {
        for item in rows(&value[group], &format!("Comparison {group}"))? {
            fields(item, &["change"], &format!("Comparison {group} record"))?;
            let change = item["change"].as_str().unwrap();
            if !["NEW", "CHANGED", "RECURRING", "NOT_RAISED", "NOT_COMPARABLE"].contains(&change) {
                return Err("Unrecognised comparison classification.".into());
            }
            let (previous, current, same) = comparison_sides(item, group)?;
            if !previous && !current {
                return Err("Comparison requires at least one populated side.".into());
            }
            if (change == "NEW" && (previous || !current))
                || (["NOT_RAISED", "NOT_COMPARABLE"].contains(&change) && (!previous || current))
                || (change == "NOT_RAISED" && !scope.is_empty())
                || (change == "NOT_COMPARABLE" && scope.is_empty())
                || (["CHANGED", "RECURRING"].contains(&change)
                    && (!previous || !current || same != (change == "RECURRING")))
            {
                return Err("Comparison classification contradicts its previous/current records.".into());
            }
        }
    }
    Ok(())
}

fn comparison_sides(item: &Value, group: &str) -> Result<(bool, bool, bool)> {
    if group == "findings" {
        fields(item, &["control", "account_id"], "Finding comparison")?;
        let mut previous = rows(&item["previous"], "Previous findings")?.clone();
        let mut current = rows(&item["current"], "Current findings")?.clone();
        for row in previous.iter().chain(&current) {
            fields(row, &["account_name", "current_value", "status"], "Displayed finding")?;
        }
        previous.sort_by_cached_key(json::sort_key);
        current.sort_by_cached_key(json::sort_key);
        Ok((
            !previous.is_empty(),
            !current.is_empty(),
            json::equal(&Value::Array(previous), &Value::Array(current)),
        ))
    } else {
        fields(item, &["query_id"], "Query comparison")?;
        let previous = item.get("previous").unwrap_or(&Value::Null);
        let current = item.get("current").unwrap_or(&Value::Null);
        for row in [previous, current] {
            if !row.is_null() {
                fields(row, &["account_name", "control", "account_id"], "Displayed query")?;
            }
        }
        Ok((!previous.is_null(), !current.is_null(), json::equal(previous, current)))
    }
}

fn invocation_document(value: &Value, manifests: &Value) -> Result<()> {
    fields(
        value,
        &[
            "command",
            "executable_sha256",
            "producer_manifest_sha256",
            "started",
            "finished",
        ],
        "Receipt invocation",
    )?;
    if !exact(
        value,
        &[
            "command",
            "args",
            "executable_sha256",
            "producer_manifest_sha256",
            "started",
            "finished",
            "exit",
        ],
    ) || value["exit"].as_i64().is_none()
    {
        return Err("Receipt invocation fields differ.".into());
    }
    if !value["args"]
        .as_array()
        .is_some_and(|args| args.iter().all(Value::is_string))
    {
        return Err("Receipt invocation arguments require strings.".into());
    }
    let command = value["command"].as_str().unwrap();
    if !PRODUCERS.iter().any(|(name, ..)| *name == command)
        || value["producer_manifest_sha256"] != digest(&json::bytes(&manifests[command]))
        || value["executable_sha256"] != manifests[command]["launcher_sha256"]
    {
        return Err("Receipt invocation producer identity differs.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_classifications_require_consistent_populated_sides() {
        let row = serde_json::json!({"account_name":"Sales","current_value":"10.00","status":"REVIEW"});
        for (change, previous, current, scope, accepted) in [
            ("NEW", vec![], vec![row.clone()], vec![], true),
            ("RECURRING", vec![row.clone()], vec![row.clone()], vec![], true),
            ("CHANGED", vec![row.clone()], vec![row.clone()], vec![], false),
            ("NOT_RAISED", vec![row.clone()], vec![], vec![], true),
            ("NOT_RAISED", vec![row.clone()], vec![], vec!["period"], false),
            ("NOT_COMPARABLE", vec![row.clone()], vec![], vec!["period"], true),
            ("NOT_COMPARABLE", vec![row.clone()], vec![], vec![], false),
            ("NEW", vec![], vec![], vec![], false),
        ] {
            let doc = serde_json::json!({"review_boundary":"Human review required","scope_changes":scope,"findings":[{"change":change,"control":"period_variance","account_id":"200","previous":previous,"current":current}],"queries":[]});
            assert_eq!(comparison(&doc).is_ok(), accepted, "{change}: {scope:?}");
        }
    }

    #[test]
    fn money_and_producer_identity_do_not_coerce_types() {
        for text in ["NaN", "Infinity", "1E+3", "+1000.00", "1000.000", " 1000.00", "-.00"] {
            assert!(money(&Value::String(text.into()), "money", false).is_err());
        }
        assert!(money(&Value::String("-0.00".into()), "money", false).is_ok());
        let native = serde_json::json!({"manifest_version":2,"command":"review-ready","launcher_name":"review-ready.exe","launcher_sha256":"0".repeat(64),"native":{"package":"review-ready-gate","version":"1.0.0"}});
        assert!(producer(&native, "review-ready").is_ok());
        for version in [serde_json::json!(true), serde_json::json!(2.0)] {
            let mut changed = native.clone();
            changed["manifest_version"] = version;
            assert!(producer(&changed, "review-ready").is_err());
        }
        let mut changed = native;
        changed["native"]["package"] = "monthly-close-control-plane".into();
        assert!(producer(&changed, "review-ready").is_err());
    }

    #[test]
    fn comparison_distinguishes_changed_records_and_reordered_recurring_groups() {
        let first = serde_json::json!({"account_name":"Sales","current_value":"10.00","status":"REVIEW"});
        let second = serde_json::json!({"account_name":"Sales","current_value":"20.00","status":"REVIEW"});
        let mut doc = serde_json::json!({"review_boundary":"Human review required","scope_changes":[],"findings":[{"control":"period_variance","account_id":"200","change":"RECURRING","previous":[first.clone(),second.clone()],"current":[second.clone(),first.clone()]}],"queries":[]});
        comparison(&doc).unwrap();
        doc["findings"][0]["current"][0]["current_value"] = "21.00".into();
        assert!(comparison(&doc).is_err());
        doc["findings"][0]["change"] = "CHANGED".into();
        comparison(&doc).unwrap();
        doc["queries"] = serde_json::json!([{"query_id":"Q1","change":"CHANGED","previous":{"account_name":"Sales","control":"period_variance","account_id":"200","question":"old"},"current":{"account_name":"Sales","control":"period_variance","account_id":"200","question":"new"}}]);
        comparison(&doc).unwrap();
        doc["queries"][0]["current"] = doc["queries"][0]["previous"].clone();
        assert!(comparison(&doc).is_err());
    }
}
