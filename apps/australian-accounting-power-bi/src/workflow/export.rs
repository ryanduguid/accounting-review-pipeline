use super::{
    BOUNDARY, CLOSE_FILES, MAX_EXPORT_BYTES, MAX_EXPORT_MEMBER, READY_FILES, Result, archive, digest, files, html,
    json, projection, run, schema,
};
use archive::Members;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const FORMAT: &str = "australian-accounting-review-evidence-export";
const STATEMENT: &str = "Unsigned internal member-byte consistency only. This does not authenticate provenance, prove producer execution, approve accounting or establish completeness of evidence or controls.";
const SOURCES: [&str; 6] = [
    "sample-ato-benchmarks.csv",
    "sample-budgets.csv",
    "sample-chart-of-accounts.csv",
    "sample-entities.csv",
    "sample-general-ledger.csv",
    "sample-payroll-super.csv",
];
const PROJECTIONS: [&str; 3] = [
    "sample-review-run.csv",
    "sample-review-exceptions.csv",
    "sample-review-evidence.csv",
];

fn fixed_inventory() -> BTreeSet<String> {
    SOURCES
        .into_iter()
        .chain([
            "case.json",
            "trial_balance.csv",
            "prior_trial_balance.csv",
            "transactions.csv",
            "open_items.csv",
            "self_review.json",
        ])
        .map(|name| format!("inputs/{name}"))
        .chain(CLOSE_FILES.map(|name| format!("close/{name}")))
        .chain(READY_FILES.map(|name| format!("readiness/{name}")))
        .chain([
            "drivers/variance-drivers.json".into(),
            "drivers/variance-drivers.csv".into(),
        ])
        .chain(
            ["readiness", "readiness-view", "close", "close-view", "drivers"]
                .into_iter()
                .flat_map(|label| ["stdout", "stderr"].map(|kind| format!("validation/{label}.{kind}.txt"))),
        )
        .collect()
}

fn inventory(receipt: &Value) -> Result<BTreeSet<String>> {
    let names: BTreeSet<_> = receipt["case"]["source_sha256"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    if names != SOURCES.into_iter().collect() {
        return Err("Export source inventory differs from the fixed case.".into());
    }
    Ok(fixed_inventory())
}

fn check_tree(run: &Path, expected: &BTreeSet<String>) -> Result<()> {
    let run = files::ordinary(run)?;
    let files: BTreeSet<_> = expected
        .iter()
        .cloned()
        .chain(["receipt.json".into(), "receipt.sha256".into()])
        .collect();
    let folders: BTreeSet<_> = expected
        .iter()
        .map(|name| name.split_once('/').unwrap().0.to_owned())
        .collect();
    let (mut seen_files, mut seen_folders, mut total) = (BTreeSet::new(), BTreeSet::new(), 0);
    visit(
        &run,
        &run,
        &files,
        &folders,
        &mut seen_files,
        &mut seen_folders,
        &mut total,
    )?;
    if seen_files != files || seen_folders != folders {
        return Err("Export physical inventory differs from the fixed case.".into());
    }
    Ok(())
}

fn visit(
    root: &Path,
    directory: &Path,
    files: &BTreeSet<String>,
    folders: &BTreeSet<String>,
    seen_files: &mut BTreeSet<String>,
    seen_folders: &mut BTreeSet<String>,
    total: &mut u64,
) -> Result<()> {
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_str()
            .ok_or("Export run contains an unlisted or non-regular path.")?;
        let name = if cfg!(windows) {
            relative.replace('\\', "/")
        } else {
            relative.to_owned()
        };
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if files::linked(&metadata) {
            return Err("Export run contains a linked path.".into());
        }
        if metadata.is_dir() && folders.contains(&name) {
            seen_folders.insert(name);
            visit(root, &path, files, folders, seen_files, seen_folders, total)?;
        } else if metadata.is_dir() && name == "readiness/.reviewready" {
            continue;
        } else if metadata.is_file() && files.contains(&name) {
            *total = total.saturating_add(metadata.len());
            if metadata.len() > MAX_EXPORT_MEMBER as u64 || *total > MAX_EXPORT_BYTES as u64 {
                return Err("Export run exceeds the fixed-case size limits.".into());
            }
            seen_files.insert(name);
        } else {
            return Err("Export run contains an unlisted or non-regular path.".into());
        }
    }
    Ok(())
}

pub fn export(run: &Path, bin_dir: &Path, output: &Path) -> Result<PathBuf> {
    let run = files::ordinary(run)?;
    let output = files::new_output(output)?;
    if output.starts_with(&run) {
        return Err("Export output must be outside the sealed run.".into());
    }
    let expected = fixed_inventory();
    check_tree(&run, &expected)?;
    let initial = json::parse(&files::read(&run.join("receipt.json"))?, "Receipt")?;
    schema::receipt(&initial)?;
    inventory(&initial)?;
    let verified = run::verify(&run)?;
    if !json::equal(&initial, &verified.receipt)
        || verified.snapshot.keys().cloned().collect::<BTreeSet<_>>() != expected
    {
        return Err("Export requires the exact fixed-case inventory, without a review note or extra files.".into());
    }
    let receipt = files::read(&run.join("receipt.json"))?;
    let receipt_hash = files::read(&run.join("receipt.sha256"))?;
    let captured = json::parse(&receipt, "Receipt")?;
    schema::receipt(&captured)?;
    if !json::equal(&captured, &verified.receipt) || receipt_hash != digest(&receipt).as_bytes() {
        return Err("Receipt changed during export.".into());
    }
    let mut members: Members = verified
        .snapshot
        .iter()
        .map(|(name, data)| (format!("run/{name}"), data.clone()))
        .collect();
    members.insert("run/receipt.json".into(), receipt.clone());
    members.insert("run/receipt.sha256".into(), receipt_hash.clone());
    capture_display(&mut members, bin_dir)?;
    seal_manifest(&mut members);
    let payload = archive::encode(&members)?;
    check_tree(&run, &expected)?;
    if run::verify(&run)? != verified
        || files::read(&run.join("receipt.json"))? != receipt
        || files::read(&run.join("receipt.sha256"))? != receipt_hash
    {
        return Err("Run changed during export.".into());
    }
    fs::create_dir_all(output.parent().unwrap()).map_err(|error| error.to_string())?;
    files::ordinary(&output)?;
    files::write_new(&output, &payload)?;
    Ok(output)
}

fn capture_display(members: &mut Members, bin_dir: &Path) -> Result<()> {
    let temporary = files::Temporary::new("review-export-")?;
    for (name, data) in members.iter() {
        let target = temporary.0.join(name.strip_prefix("run/").unwrap());
        fs::create_dir_all(target.parent().unwrap()).map_err(|error| error.to_string())?;
        files::write_new(&target, data)?;
    }
    members.insert(
        "review.html".into(),
        html::render(&temporary.0, bin_dir, None)?.into_bytes(),
    );
    let projected = projection::projection(&temporary.0)?;
    if projected.keys().map(String::as_str).collect::<BTreeSet<_>>() != PROJECTIONS.into_iter().collect() {
        return Err("Export projection inventory differs.".into());
    }
    members.extend(
        projected
            .into_iter()
            .map(|(name, data)| (format!("projections/{name}"), data)),
    );
    Ok(())
}

fn seal_manifest(members: &mut Members) {
    let inventory: serde_json::Map<_, _> = members
        .iter()
        .filter(|(name, _)| *name != "export-manifest.json")
        .map(|(name, data)| (name.clone(), json!({"size":data.len(),"sha256":digest(data)})))
        .collect();
    let manifest = json!({"format":FORMAT,"schema_version":1,"boundary":BOUNDARY,"integrity_statement":STATEMENT,"members":inventory});
    members.insert("export-manifest.json".into(), json::bytes(&manifest));
}

pub fn verify(path: &Path) -> Result<Value> {
    verify_bytes(&files::bounded_read(path, MAX_EXPORT_BYTES)?)
}

fn manifest(members: &Members) -> Result<Value> {
    let manifest_data = members
        .get("export-manifest.json")
        .map(Vec::as_slice)
        .unwrap_or_default();
    let manifest = json::parse(manifest_data, "Export manifest")?;
    if !schema::exact(
        &manifest,
        &["format", "schema_version", "boundary", "integrity_statement", "members"],
    ) || manifest["format"] != FORMAT
        || manifest["schema_version"].as_i64() != Some(1)
        || manifest["boundary"] != BOUNDARY
        || manifest["integrity_statement"] != STATEMENT
        || json::bytes(&manifest) != manifest_data
    {
        return Err("Unsupported export manifest.".into());
    }
    Ok(manifest)
}

fn verify_bytes(payload: &[u8]) -> Result<Value> {
    let members = archive::decode(payload)?;
    let manifest = manifest(&members)?;
    verify_members(&members, &manifest)?;
    let (Some(receipt_data), Some(receipt_hash)) = (members.get("run/receipt.json"), members.get("run/receipt.sha256"))
    else {
        return Err("Export receipt is missing.".into());
    };
    if receipt_hash != digest(receipt_data).as_bytes() {
        return Err("Export receipt digest differs.".into());
    }
    let receipt = json::parse(receipt_data, "Export receipt")?;
    schema::receipt(&receipt)?;
    let expected = inventory(&receipt)?;
    let names: BTreeSet<_> = [
        "export-manifest.json".into(),
        "review.html".into(),
        "run/receipt.json".into(),
        "run/receipt.sha256".into(),
    ]
    .into_iter()
    .chain(expected.iter().map(|name| format!("run/{name}")))
    .chain(PROJECTIONS.map(|name| format!("projections/{name}")))
    .collect();
    if receipt["files"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>()
        != expected
        || members.keys().cloned().collect::<BTreeSet<_>>() != names
    {
        return Err("Export fixed-case inventory differs.".into());
    }
    if receipt["files"]
        .as_object()
        .unwrap()
        .iter()
        .any(|(name, hash)| digest(&members[&format!("run/{name}")]) != hash.as_str().unwrap())
    {
        return Err("Export nested receipt binding differs.".into());
    }
    Ok(
        json!({"members":members.len(),"period":receipt["period"],"receipt_sha256":digest(receipt_data),"integrity_statement":STATEMENT}),
    )
}

fn verify_members(members: &Members, manifest: &Value) -> Result<()> {
    let Some(inventory) = manifest["members"].as_object() else {
        return Err("Export member inventory differs.".into());
    };
    if inventory.keys().map(String::as_str).collect::<BTreeSet<_>>()
        != members
            .keys()
            .filter(|name| *name != "export-manifest.json")
            .map(String::as_str)
            .collect()
    {
        return Err("Export member inventory differs.".into());
    }
    for (name, record) in inventory {
        let data = &members[name];
        if !schema::exact(record, &["size", "sha256"])
            || record["size"].as_u64() != Some(data.len() as u64)
            || record["sha256"] != digest(data)
        {
            return Err(format!("Export member bytes differ: {name}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unsigned_members() -> Members {
        let mut members: Members = fixed_inventory()
            .into_iter()
            .map(|name| (format!("run/{name}"), b"fabricated".to_vec()))
            .collect();
        let hashes: serde_json::Map<_, _> = members
            .iter()
            .map(|(name, data)| (name.strip_prefix("run/").unwrap().into(), digest(data).into()))
            .collect();
        let source_hashes: serde_json::Map<_, _> = SOURCES
            .map(|name| (name.into(), digest(b"fabricated").into()))
            .into_iter()
            .collect();
        let case = json!({"case_id":"unsigned","entity":"Fabricated","tenant":"Demo","basis":"Accrual","currency":"AUD","financial_year_start":"2024-07-01","opening_journal":"opening","population":"fabricated","account_ids":{},"source_sha256":source_hashes});
        let manifests: serde_json::Map<_, _> = [("close-control", "monthly-close-control-plane"), ("review-ready", "review-ready-gate")].map(|(command, package)| (command.into(), json!({"manifest_version":2,"command":command,"launcher_name":command,"launcher_sha256":"0".repeat(64),"native":{"package":package,"version":"1.0.0"}}))).into_iter().collect();
        let receipt = json::bytes(
            &json!({"schema_version":1,"case":case,"period":"2024-08-31","invocations":[],"files":hashes,"boundary":BOUNDARY,"producer_manifests":manifests}),
        );
        members.insert("run/receipt.sha256".into(), digest(&receipt).into_bytes());
        members.insert("run/receipt.json".into(), receipt);
        members.insert("review.html".into(), b"<p>Unsigned fabricated example</p>".to_vec());
        members.extend(PROJECTIONS.map(|name| (format!("projections/{name}"), b"fabricated\n".to_vec())));
        seal_manifest(&mut members);
        members
    }

    #[test]
    fn portable_verification_checks_unsigned_bytes_and_nested_receipt_without_producers() {
        let members = unsigned_members();
        let verified = verify_bytes(&archive::encode(&members).unwrap()).unwrap();
        assert_eq!(verified["members"], 38);
        assert_eq!(verified["integrity_statement"], STATEMENT);
        let mut changed = members.clone();
        changed.insert("run/inputs/trial_balance.csv".into(), b"tampered".to_vec());
        assert!(
            verify_bytes(&archive::encode(&changed).unwrap())
                .unwrap_err()
                .contains("member bytes differ")
        );
        seal_manifest(&mut changed);
        assert!(
            verify_bytes(&archive::encode(&changed).unwrap())
                .unwrap_err()
                .contains("nested receipt")
        );
        let mut changed = members;
        changed.insert("extra.txt".into(), b"extra".to_vec());
        seal_manifest(&mut changed);
        assert!(
            verify_bytes(&archive::encode(&changed).unwrap())
                .unwrap_err()
                .contains("fixed-case inventory")
        );
    }

    #[test]
    fn physical_admission_precedes_reading_receipts_and_only_skips_the_readiness_control() {
        let temporary = files::Temporary::new("pbi-export-test-").unwrap();
        let expected = fixed_inventory();
        for name in expected
            .iter()
            .map(String::as_str)
            .chain(["receipt.json", "receipt.sha256"])
        {
            let path = temporary.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        fs::create_dir(temporary.0.join("readiness/.reviewready")).unwrap();
        fs::write(temporary.0.join("readiness/.reviewready/state.json"), b"{}").unwrap();
        check_tree(&temporary.0, &expected).unwrap();
        #[cfg(unix)]
        {
            let extra = temporary.0.join("validation\\close.stdout.txt");
            fs::write(&extra, b"unlisted Unix filename").unwrap();
            assert!(check_tree(&temporary.0, &expected).unwrap_err().contains("unlisted"));
            fs::remove_file(extra).unwrap();
        }
        fs::create_dir(temporary.0.join("close/.reviewready")).unwrap();
        assert!(check_tree(&temporary.0, &expected).unwrap_err().contains("unlisted"));
    }

    #[test]
    fn canonical_manifests_refuse_coerced_types_extra_fields_and_changed_statements() {
        let original = unsigned_members();
        for mutation in 0..6 {
            let mut members = original.clone();
            let mut manifest = json::parse(&members["export-manifest.json"], "Manifest").unwrap();
            match mutation {
                0 => manifest["schema_version"] = true.into(),
                1 => manifest["boundary"] = "different boundary".into(),
                2 => manifest["extra"] = true.into(),
                3 => manifest["members"]["review.html"]["size"] = true.into(),
                4 => manifest["members"]["review.html"]["extra"] = true.into(),
                _ => manifest["integrity_statement"] = "Authenticated".into(),
            }
            members.insert("export-manifest.json".into(), json::bytes(&manifest));
            assert!(verify_bytes(&archive::encode(&members).unwrap()).is_err(), "{mutation}");
        }
        let mut members = original;
        let mut bytes = members["export-manifest.json"].clone();
        bytes.push(b' ');
        members.insert("export-manifest.json".into(), bytes);
        assert!(
            verify_bytes(&archive::encode(&members).unwrap())
                .unwrap_err()
                .contains("Unsupported export manifest")
        );
    }

    #[test]
    fn sealed_run_output_and_oversized_inputs_are_refused_before_receipt_parsing() {
        let temporary = files::Temporary::new("pbi-export-admission-").unwrap();
        let run = temporary.0.join("run");
        fs::create_dir(&run).unwrap();
        assert!(
            export(&run, Path::new("unused"), &run.join("output.zip"))
                .unwrap_err()
                .contains("outside the sealed run")
        );
        let expected = fixed_inventory();
        for name in expected
            .iter()
            .map(String::as_str)
            .chain(["receipt.json", "receipt.sha256"])
        {
            let path = run.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"invalid receipt").unwrap();
        }
        let file = fs::OpenOptions::new()
            .write(true)
            .open(run.join("inputs/case.json"))
            .unwrap();
        file.set_len(MAX_EXPORT_MEMBER as u64 + 1).unwrap();
        drop(file);
        assert!(
            export(&run, Path::new("unused"), &temporary.0.join("outside.zip"))
                .unwrap_err()
                .contains("size limits")
        );
        assert!(!temporary.0.join("outside.zip").exists());
    }
}
