use crate::{
    decimal::Decimal,
    engine,
    loader::{self, SourceSnapshot},
    pyjson::{self, Json},
    test_support::TempDir,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn text(value: &Json) -> String {
    loader::json_text(value).unwrap()
}
fn array(value: &Json) -> &[Json] {
    let Json::Array(items) = value else {
        panic!("fixture array")
    };
    items
}
fn contract(folder: &str) -> Json {
    let bytes = SourceSnapshot::capture(
        &root().join(format!("evaluation/{folder}/expected_results.json")),
        "evaluation",
    )
    .unwrap();
    pyjson::loads(bytes.text("evaluation", false).unwrap()).unwrap()
}
fn files(path: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(path)
        .unwrap()
        .map(|p| p.unwrap())
        .filter(|p| p.file_type().unwrap().is_file())
        .map(|p| {
            (
                p.file_name().into_string().unwrap(),
                fs::read(p.path()).unwrap(),
            )
        })
        .collect()
}
#[test]
fn manager_review_and_missing_evidence_scenarios_remain_deterministic() {
    for folder in ["manager_review_gate", "missing_evidence"] {
        let contract = contract(folder);
        assert!(pyjson::py_eq(
            contract.at("schema_version"),
            &Json::int(1),
            false
        ));
        assert!(
            contract
                .at("product_release")
                .is_str(if folder == "manager_review_gate" {
                    "0.1.5"
                } else {
                    "0.1.8"
                })
        );
        if folder == "missing_evidence" {
            assert!(contract.at("practitioner_review").is_str("pending"));
        }
        for scenario in array(contract.at("scenarios")) {
            for _ in 0..2 {
                let pack = engine::review_pack(
                    &text(scenario.at("profile")),
                    &root().join(text(scenario.at("fixture"))),
                    None,
                    Decimal::parse("0.01").unwrap(),
                    &[],
                )
                .unwrap();
                assert_eq!(pack.status, text(scenario.at("expected_status")));
                assert_eq!(
                    pack.findings
                        .iter()
                        .map(|f| f.code.clone())
                        .collect::<Vec<_>>(),
                    array(scenario.at("expected_findings"))
                        .iter()
                        .map(text)
                        .collect::<Vec<_>>()
                );
                if let Some(expected) = scenario.get("expected_controls_not_run") {
                    assert_eq!(
                        pack.controls_not_run
                            .unwrap()
                            .iter()
                            .map(|c| c.slot.clone())
                            .collect::<Vec<_>>(),
                        array(expected).iter().map(text).collect::<Vec<_>>()
                    );
                }
            }
        }
    }
}
#[test]
fn missing_evidence_fixtures_contain_only_the_declared_deltas() {
    let contract = contract("missing_evidence");
    let base = files(&root().join(text(contract.at("base_fixture"))));
    let scenarios = array(contract.at("scenarios"));
    let declared: BTreeSet<_> = scenarios
        .iter()
        .map(|s| {
            PathBuf::from(text(s.at("fixture")))
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let actual: BTreeSet<_> = fs::read_dir(root().join("evaluation/missing_evidence/packs"))
        .unwrap()
        .map(|p| p.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(actual, declared);
    for scenario in scenarios {
        let pack = files(&root().join(text(scenario.at("fixture"))));
        let change = scenario.at("change");
        let names = |key| {
            change
                .get(key)
                .map(array)
                .unwrap_or(&[])
                .iter()
                .map(text)
                .collect::<BTreeSet<_>>()
        };
        let removed = names("removed");
        let emptied = names("emptied");
        assert_eq!(
            base.keys()
                .filter(|k| !pack.contains_key(*k))
                .cloned()
                .collect::<BTreeSet<_>>(),
            removed
        );
        assert!(pack.keys().all(|k| base.contains_key(k)));
        for (name, bytes) in pack {
            if emptied.contains(&name) {
                assert!(bytes.is_empty());
            } else if let Some(edit) = change.get("edited").and_then(|e| e.get(&name)) {
                let before = text(edit.at("from"));
                let after = text(edit.at("to"));
                let base_text = std::str::from_utf8(&base[&name]).unwrap();
                assert_eq!(base_text.matches(&before).count(), 1);
                assert_eq!(bytes, base_text.replacen(&before, &after, 1).as_bytes());
            } else {
                assert_eq!(bytes, base[&name]);
            }
        }
    }
}
#[test]
fn edited_evaluation_summary_is_refused_without_writing_any_artefact() {
    let contract = contract("missing_evidence");
    let case = contract.at("edited_artefact");
    let tmp = TempDir::new();
    let pack = engine::review_pack(
        &text(case.at("profile")),
        &root().join(text(case.at("fixture"))),
        None,
        Decimal::parse("0.01").unwrap(),
        &[],
    )
    .unwrap();
    let output = tmp.join("out");
    crate::report::write_review_pack(&pack, &output).unwrap();
    crate::viewer::verify_pack(&output).unwrap();
    let file = output.join(text(case.at("edit").at("file")));
    let before = fs::read_to_string(&file).unwrap();
    let old = text(case.at("edit").at("from"));
    let new = text(case.at("edit").at("to"));
    assert_eq!(before.matches(&old).count(), 1);
    fs::write(&file, before.replacen(&old, &new, 1)).unwrap();
    let saved = crate::report::PACK_NAMES.map(|n| fs::read(output.join(n)).unwrap());
    assert!(
        crate::viewer::verify_pack(&output)
            .err()
            .unwrap()
            .contains(&text(case.at("expected_error")))
    );
    assert_eq!(
        saved,
        crate::report::PACK_NAMES.map(|n| fs::read(output.join(n)).unwrap())
    );
}
