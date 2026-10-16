use super::{
    BOUNDARY, CLOSE_FILES, READY_FILES, Result, digest, files, json,
    ledger::{self, Money, Row, Snapshot},
    producer, schema,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Verified {
    pub receipt: Value,
    pub snapshot: Snapshot,
}

pub fn ready_revision(run: &Path) -> Result<Option<String>> {
    let control = files::ordinary(&run.join("readiness/.reviewready"))?;
    match fs::symlink_metadata(&control) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(metadata) if !metadata.is_dir() => return Err("Readiness publication control is not a directory.".into()),
        Ok(_) => {}
    }
    let mut file = files::open_regular(&control.join("state.json"))?;
    if !files::single_link(&file)? {
        return Err("Readiness publication state is not a regular, single-link file.".into());
    }
    let mut data = Vec::new();
    file.read_to_end(&mut data).map_err(|error| error.to_string())?;
    let state = json::parse(&data, "Readiness publication state")?;
    let identifier = |value: &Value, length| {
        value.as_str().is_some_and(|text| {
            text.len() == length
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    };
    if !schema::exact(&state, &["format", "status", "revision", "transaction", "previous"])
        || state["format"].as_i64() != Some(1)
        || state["status"] != "settled"
        || !identifier(&state["revision"], 32)
        || (!state["transaction"].is_null() && !identifier(&state["transaction"], 32))
        || !state["previous"].as_array().is_some_and(|values| {
            values.len() == 3 && values.iter().all(|value| value.is_null() || identifier(value, 64))
        })
    {
        return Err("Readiness publication is unresolved or unsupported.".into());
    }
    Ok(Some(state["revision"].as_str().unwrap().to_owned()))
}

fn required() -> BTreeSet<String> {
    CLOSE_FILES
        .map(|name| format!("close/{name}"))
        .into_iter()
        .chain(READY_FILES.map(|name| format!("readiness/{name}")))
        .chain(
            [
                "inputs/trial_balance.csv",
                "inputs/prior_trial_balance.csv",
                "inputs/transactions.csv",
                "inputs/case.json",
                "drivers/variance-drivers.json",
                "drivers/variance-drivers.csv",
            ]
            .map(str::to_owned),
        )
        .collect()
}

pub fn snapshot_file<'a>(snapshot: &'a Snapshot, name: &str) -> Result<&'a [u8]> {
    snapshot
        .get(name)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("Required review artefact is missing: {name}"))
}

fn safe_receipt_path(name: &str) -> bool {
    name.split_once('/').is_some_and(|(folder, file)| {
        ["inputs", "close", "readiness", "drivers", "validation"].contains(&folder)
            && !file.is_empty()
            && !file.contains("..")
            && file
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
    })
}

fn snapshot(run: &Path, receipt: &Value) -> Result<Snapshot> {
    let hashes = receipt["files"].as_object().unwrap();
    if required().iter().any(|name| !hashes.contains_key(name)) {
        return Err("Required review artefacts are missing.".into());
    }
    let mut snapshot = Snapshot::new();
    for (name, hash) in hashes {
        if !safe_receipt_path(name) {
            return Err("Receipt path is outside the whitelist.".into());
        }
        let data = files::read(&run.join(name))?;
        if digest(&data) != hash.as_str().unwrap() {
            return Err(format!("Sealed artefact changed: {name}"));
        }
        snapshot.insert(name.clone(), data);
    }
    Ok(snapshot)
}

pub fn verify(run: &Path) -> Result<Verified> {
    let run = files::ordinary(run)?;
    let revision = ready_revision(&run)?;
    let data = files::read(&run.join("receipt.json"))?;
    if digest(&data).as_bytes() != files::read(&run.join("receipt.sha256"))? {
        return Err("Receipt digest differs.".into());
    }
    let receipt = json::parse(&data, "Receipt")?;
    schema::receipt(&receipt)?;
    let snapshot = snapshot(&run, &receipt)?;
    let case = &receipt["case"];
    let sources = verified_sources(case, &snapshot)?;
    let month: u32 = receipt["period"].as_str().unwrap()[5..7]
        .parse()
        .map_err(|_| "Invalid receipt period.")?;
    let (current, prior, transactions) = ledger::trial_balances(case, &sources, month)?;
    for (name, expected) in [
        ("inputs/trial_balance.csv", &current),
        ("inputs/prior_trial_balance.csv", &prior),
        ("inputs/transactions.csv", &transactions),
    ] {
        if snapshot_file(&snapshot, name)? != expected {
            return Err("Ledger and review sources do not tie.".into());
        }
    }
    if receipt["period"] != first_date(&current)? {
        return Err("Receipt period differs from the ledger case.".into());
    }
    let doc = json::parse(snapshot_file(&snapshot, "close/close-review-pack.json")?, "Close pack")?;
    let drivers = json::parse(
        snapshot_file(&snapshot, "drivers/variance-drivers.json")?,
        "Driver pack",
    )?;
    let readiness = json::parse(
        snapshot_file(&snapshot, "readiness/readiness-pack.json")?,
        "Readiness pack",
    )?;
    schema::close(&doc)?;
    schema::drivers(&drivers)?;
    schema::readiness(&readiness)?;
    validate_drivers(&drivers, &transactions, receipt["period"].as_str().unwrap())?;
    source_bindings(&doc, &drivers, &snapshot, &current, &prior, &transactions)?;
    invocations(&receipt)?;
    if ready_revision(&run)? != revision {
        return Err("Readiness publication changed during verification.".into());
    }
    Ok(Verified { receipt, snapshot })
}

fn verified_sources(case: &Value, snapshot: &Snapshot) -> Result<Snapshot> {
    let (expected_case, _) = ledger::bundled_sources()?;
    let input_case = json::parse(snapshot_file(snapshot, "inputs/case.json")?, "Case input")?;
    schema::case(&input_case)?;
    if input_case != *case || *case != expected_case {
        return Err("Case identity differs.".into());
    }
    let mut sources = Snapshot::new();
    for (name, hash) in case["source_sha256"].as_object().unwrap() {
        let data = snapshot_file(snapshot, &format!("inputs/{name}"))?;
        if digest(data) != hash.as_str().unwrap() {
            return Err("Fixture provenance differs.".into());
        }
        sources.insert(name.clone(), data.to_vec());
    }
    Ok(sources)
}

fn first_date(data: &[u8]) -> Result<String> {
    let rows = ledger::rows(data)?;
    Ok(ledger::field(rows.first().ok_or("Missing trial-balance rows.")?, "ReportDate")?.to_owned())
}

fn validate_drivers(drivers: &Value, transactions: &[u8], period: &str) -> Result<()> {
    let rows = ledger::rows(transactions)?;
    let mut population = BTreeMap::new();
    for row in &rows {
        population.insert(ledger::field(row, "TransactionID")?, row);
    }
    let mut accounts = BTreeSet::new();
    for account in drivers["accounts"].as_array().unwrap() {
        let tenant = account["tenant"].as_str().unwrap();
        let account_id = account["account_id"].as_str().unwrap();
        if !accounts.insert((tenant, account_id)) {
            return Err("Duplicate driver account identity.".into());
        }
        let mut eligible = BTreeMap::new();
        for (id, row) in &population {
            if ledger::field(row, "Tenant")? == tenant
                && ledger::field(row, "AccountID")? == account_id
                && ledger::field(row, "Date")?.get(..7) == period.get(..7)
            {
                eligible.insert(*id, *row);
            }
        }
        let details = account["drivers"].as_array().unwrap();
        let identifiers: BTreeSet<_> = details
            .iter()
            .map(|driver| driver["TransactionID"].as_str().unwrap())
            .collect();
        if identifiers.len() != details.len()
            || identifiers != eligible.keys().copied().collect()
            || account["transactions_in_window"].as_u64() != Some(eligible.len() as u64)
        {
            return Err("Driver journal population differs from the source.".into());
        }
        let total = eligible
            .values()
            .try_fold(Money::zero(), |total, row| Ok::<_, String>(total.add(&signed(row)?)))?;
        if total != ledger::money(account["transactions_total"].as_str().unwrap())?
            || ledger::money(account["movement"].as_str().unwrap())?.sub(&total)
                != ledger::money(account["unexplained"].as_str().unwrap())?
        {
            return Err("Driver totals differ from the source.".into());
        }
        validate_details(details, &population)?;
    }
    Ok(())
}

fn validate_details(details: &[Value], population: &BTreeMap<&str, &Row>) -> Result<()> {
    for driver in details {
        let original = population[driver["TransactionID"].as_str().unwrap()];
        if ["Date", "Reference", "Description"]
            .iter()
            .any(|name| driver[*name].as_str() != original.get(*name).map(String::as_str))
            || driver["Amount"] != signed(original)?.to_fixed(2)
        {
            return Err("Driver journal detail differs from the source.".into());
        }
    }
    Ok(())
}

fn signed(row: &Row) -> Result<Money> {
    Ok(ledger::money(ledger::field(row, "Debit")?)?.sub(&ledger::money(ledger::field(row, "Credit")?)?))
}

fn source_bindings(
    doc: &Value,
    drivers: &Value,
    snapshot: &Snapshot,
    current: &[u8],
    prior: &[u8],
    transactions: &[u8],
) -> Result<()> {
    if doc["source_sha256"]["current_trial_balance"] != digest(current)
        || doc["source_sha256"]["prior_trial_balance"] != digest(prior)
        || doc["current_report_dates"] != serde_json::json!([first_date(current)?])
        || doc["prior_report_dates"] != serde_json::json!([first_date(prior)?])
    {
        return Err("Close source binding differs.".into());
    }
    if drivers["source_sha256"]["transactions"] != digest(transactions) {
        return Err("Driver source binding differs.".into());
    }
    for name in CLOSE_FILES {
        if drivers["source_sha256"][format!("pack:{name}")]
            != digest(snapshot_file(snapshot, &format!("close/{name}"))?)
        {
            return Err("Driver source binding differs.".into());
        }
    }
    Ok(())
}

fn invocations(receipt: &Value) -> Result<()> {
    let records = receipt["invocations"].as_array().unwrap();
    let expected = [
        "review-ready",
        "review-ready",
        "close-control",
        "close-control",
        "close-control",
    ];
    if records.len() != expected.len()
        || records
            .iter()
            .zip(expected)
            .enumerate()
            .any(|(index, (record, command))| {
                record["command"] != command
                    || !record["exit"]
                        .as_i64()
                        .is_some_and(|code| code == 0 || ([0, 2].contains(&index) && code == 2))
            })
    {
        return Err("Required validation invocation did not succeed.".into());
    }
    for command in ["review-ready", "close-control"] {
        let hashes: BTreeSet<_> = records
            .iter()
            .filter(|record| record["command"] == command)
            .map(|record| record["executable_sha256"].as_str().unwrap())
            .collect();
        if hashes.len() != 1 || !hashes.iter().all(|hash| schema::hash(hash)) {
            return Err("Producer executable identity differs within the receipt.".into());
        }
    }
    Ok(())
}

pub fn build(run: &Path, bin_dir: &Path, month: u32, note: Option<&Path>) -> Result<()> {
    let run = files::new_output(run)?;
    let manifests = serde_json::json!({"close-control":producer::manifest(bin_dir, "close-control")?,"review-ready":producer::manifest(bin_dir, "review-ready")?});
    let (case, source) = ledger::bundled_sources()?;
    let (current, prior, transactions) = ledger::trial_balances(&case, &source, month)?;
    fs::create_dir_all(run.join("inputs")).map_err(|error| error.to_string())?;
    fs::create_dir(run.join("validation")).map_err(|error| error.to_string())?;
    for (name, data) in source {
        files::write_new(&run.join("inputs").join(name), &data)?;
    }
    files::write_new(&run.join("inputs/case.json"), &json::bytes(&case))?;
    for (name, data) in [
        ("trial_balance.csv", &current),
        ("prior_trial_balance.csv", &prior),
        ("transactions.csv", &transactions),
    ] {
        files::write_new(&run.join("inputs").join(name), data)?;
    }
    let period = first_date(&current)?;
    files::write_new(
        &run.join("inputs/open_items.csv"),
        b"ItemID,Severity,Owner,DueDate,Status,Description,Resolution\n",
    )?;
    let self_review = serde_json::json!({"preparer_initials":"DEMO","prepared_on":period,"engagement_type":"month_end","period_end":period,"assertions":{"pack_complete":true,"tie_outs_done":true,"open_items_listed":true,"variances_explained":true,"self_reviewed":true}});
    files::write_new(&run.join("inputs/self_review.json"), &json::bytes(&self_review))?;
    let invocations = build_commands(&run, bin_dir, &case, &manifests, note)?;
    let doc = json::parse(&files::read(&run.join("close/close-review-pack.json"))?, "Close pack")?;
    schema::close(&doc)?;
    if doc["source_sha256"]["current_trial_balance"] != digest(&current)
        || doc["source_sha256"]["prior_trial_balance"] != digest(&prior)
        || doc["current_report_dates"] != serde_json::json!([period])
        || doc["exceptions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tenant"] != "" && row["tenant"] != case["tenant"])
    {
        return Err("Close provenance or context differs from the case.".into());
    }
    let hashes = output_hashes(&run)?;
    let receipt = serde_json::json!({"schema_version":1,"case":case,"period":period,"invocations":invocations,"files":hashes,"boundary":BOUNDARY,"producer_manifests":manifests});
    let data = json::bytes(&receipt);
    files::write_new(&run.join("receipt.json"), &data)?;
    files::write_new(&run.join("receipt.sha256"), digest(&data).as_bytes())?;
    verify(&run)?;
    Ok(())
}

fn build_commands(
    run: &Path,
    bin_dir: &Path,
    case: &Value,
    manifests: &Value,
    note: Option<&Path>,
) -> Result<Vec<Value>> {
    let invoke = |command, args: Vec<String>, label, allowed: &[i32]| {
        producer::invoke(bin_dir, command, &args, run, label, allowed, &manifests[command])
    };
    let strings = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    let mut records = readiness_commands(run, bin_dir, &manifests["review-ready"])?;
    let mut args = strings(&[
        "review",
        "--current",
        "inputs/trial_balance.csv",
        "--prior",
        "inputs/prior_trial_balance.csv",
        "--output",
        "close",
    ]);
    if let Some(note) = note {
        files::write_new(&run.join("inputs/review-note.json"), &files::read(note)?)?;
        args.extend(strings(&["--review-note", "inputs/review-note.json"]));
    }
    records.push(invoke("close-control", args, "close", &[0, 2])?);
    records.push(close_view(run, bin_dir, &manifests["close-control"])?);
    records.push(invoke(
        "close-control",
        strings(&[
            "drivers",
            "--pack-dir",
            "close",
            "--transactions",
            "inputs/transactions.csv",
            "--currency",
            case["currency"].as_str().unwrap(),
            "--top",
            "100000",
            "--output",
            "drivers",
        ]),
        "drivers",
        &[0],
    )?);
    Ok(records)
}

fn close_view(run: &Path, bin_dir: &Path, manifest: &Value) -> Result<Value> {
    let mut before = Vec::new();
    for name in CLOSE_FILES {
        before.push(digest(&files::read(&run.join("close").join(name))?));
    }
    let args = ["view", "--pack-dir", "close"].map(str::to_owned);
    let record = producer::invoke(bin_dir, "close-control", &args, run, "close-view", &[0], manifest)?;
    for (name, expected) in CLOSE_FILES.into_iter().zip(before) {
        if digest(&files::read(&run.join("close").join(name))?) != expected {
            return Err("Close outputs changed during verification.".into());
        }
    }
    Ok(record)
}

fn readiness_commands(run: &Path, bin_dir: &Path, manifest: &Value) -> Result<Vec<Value>> {
    let invoke = |args: &[&str], label, allowed: &[i32]| {
        let args: Vec<_> = args.iter().map(|arg| (*arg).to_owned()).collect();
        producer::invoke(bin_dir, "review-ready", &args, run, label, allowed, manifest)
    };
    Ok(vec![
        invoke(
            &[
                "gate",
                "--profile",
                "month_end",
                "--pack",
                "inputs",
                "--output",
                "readiness",
            ],
            "readiness",
            &[0, 2],
        )?,
        invoke(&["view", "--pack-dir", "readiness"], "readiness-view", &[0])?,
    ])
}

fn output_hashes(run: &Path) -> Result<BTreeMap<String, String>> {
    let mut hashes = BTreeMap::new();
    for folder in ["inputs", "close", "readiness", "drivers", "validation"] {
        for entry in fs::read_dir(run.join(folder)).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            let name = format!("{folder}/{}", path.file_name().unwrap().to_string_lossy());
            if name != "readiness/.reviewready" {
                hashes.insert(name, digest(&files::read(&path)?));
            }
        }
    }
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsettled_or_linked_readiness_revision_is_not_admitted() {
        let cache = files::Temporary::new("pbi-run-test-").unwrap();
        assert_eq!(ready_revision(&cache.0).unwrap(), None);
        fs::create_dir_all(cache.0.join("readiness/.reviewready")).unwrap();
        let state = serde_json::json!({"format":1,"status":"settled","revision":"a".repeat(32),"transaction":null,"previous":[null,null,null]});
        let path = cache.0.join("readiness/.reviewready/state.json");
        fs::write(&path, json::bytes(&state)).unwrap();
        assert_eq!(ready_revision(&cache.0).unwrap(), Some("a".repeat(32)));
        let second_link = cache.0.join("state-link");
        fs::hard_link(&path, &second_link).unwrap();
        assert!(ready_revision(&cache.0).is_err());
        fs::remove_file(second_link).unwrap();
        let mut changed = state.clone();
        changed["transaction"] = "b".repeat(32).into();
        changed["previous"] = serde_json::json!(["c".repeat(64), null, "d".repeat(64)]);
        fs::write(&path, json::bytes(&changed)).unwrap();
        assert_eq!(ready_revision(&cache.0).unwrap(), Some("a".repeat(32)));
        for (key, value) in [
            ("format", serde_json::json!(true)),
            ("status", serde_json::json!("pending")),
            ("revision", serde_json::json!("A".repeat(32))),
            ("transaction", serde_json::json!("short")),
            ("previous", serde_json::json!([null, null])),
        ] {
            let mut malformed = state.clone();
            malformed[key] = value;
            fs::write(&path, json::bytes(&malformed)).unwrap();
            assert!(ready_revision(&cache.0).is_err(), "{key}");
        }
        fs::write(&path, json::bytes(&serde_json::json!({"status":"pending"}))).unwrap();
        assert!(ready_revision(&cache.0).is_err());
    }

    #[test]
    fn driver_validation_binds_the_complete_journal_population_and_detail() {
        let transactions = b"Tenant,AccountID,TransactionID,Date,Reference,Description,Debit,Credit\nDemo,200,J1:1,2024-09-05,J1,Sale,10.00,0.00\nDemo,200,J0:1,2024-08-05,J0,Prior,4.00,0.00\n";
        let baseline = serde_json::json!({"accounts":[{"tenant":"Demo","account_id":"200","transactions_in_window":1,"movement":"10.00","transactions_total":"10.00","unexplained":"0.00","drivers":[{"TransactionID":"J1:1","Date":"2024-09-05","Reference":"J1","Description":"Sale","Amount":"10.00"}]}]});
        validate_drivers(&baseline, transactions, "2024-09-30").unwrap();
        for mutation in 0..7 {
            let mut changed = baseline.clone();
            match mutation {
                0 => changed["accounts"][0]["drivers"] = serde_json::json!([]),
                1 => changed["accounts"][0]["drivers"]
                    .as_array_mut()
                    .unwrap()
                    .push(baseline["accounts"][0]["drivers"][0].clone()),
                2 => changed["accounts"][0]["drivers"][0]["TransactionID"] = "J0:1".into(),
                3 => changed["accounts"][0]["transactions_total"] = "9.00".into(),
                4 => changed["accounts"][0]["drivers"][0]["Description"] = "Altered".into(),
                5 => changed["accounts"][0]["drivers"][0]["Amount"] = "9.00".into(),
                _ => changed["accounts"]
                    .as_array_mut()
                    .unwrap()
                    .push(baseline["accounts"][0].clone()),
            }
            assert!(
                validate_drivers(&changed, transactions, "2024-09-30").is_err(),
                "{mutation}"
            );
        }
    }

    #[test]
    fn required_invocations_preserve_order_allowed_status_and_identity() {
        let commands = [
            "review-ready",
            "review-ready",
            "close-control",
            "close-control",
            "close-control",
        ];
        let records: Vec<_> = commands
            .into_iter()
            .map(|command| serde_json::json!({"command":command,"exit":0,"executable_sha256":"a".repeat(64)}))
            .collect();
        let baseline = serde_json::json!({"invocations":records});
        invocations(&baseline).unwrap();
        let mut review = baseline.clone();
        review["invocations"][0]["exit"] = 2.into();
        review["invocations"][2]["exit"] = 2.into();
        invocations(&review).unwrap();
        for mutation in 0..4 {
            let mut changed = baseline.clone();
            match mutation {
                0 => {
                    changed["invocations"].as_array_mut().unwrap().remove(0);
                }
                1 => changed["invocations"].as_array_mut().unwrap().swap(1, 2),
                2 => changed["invocations"][1]["exit"] = 2.into(),
                _ => changed["invocations"][4]["executable_sha256"] = "b".repeat(64).into(),
            }
            assert!(invocations(&changed).is_err(), "{mutation}");
        }
    }
}
