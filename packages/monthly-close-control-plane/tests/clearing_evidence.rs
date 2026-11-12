// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use monthly_close_control_plane::{
    pyjson::{self, Json},
    reconciliation::{self, ReconcileConfig},
    reconciliation_report,
};
use std::{collections::BTreeMap, fs, path::Path, time::SystemTime};

#[path = "support/temp.rs"]
mod temp;
use temp::TempDir;

fn config(temp: &TempDir, rows: &str) -> ReconcileConfig {
    let transactions = temp.join("transactions.csv");
    fs::write(
        &transactions,
        reconciliation::COLUMNS.join(",") + "\n" + rows,
    )
    .unwrap();
    ReconcileConfig {
        transactions,
        tenant: "Synthetic".into(),
        account_id: "CLEAR".into(),
        currency: "AUD".into(),
        period_start: "2026-09-01".into(),
        period_end: "2026-09-30".into(),
        opening_balance: "0".into(),
        closing_balance: "0".into(),
        opening_items: None,
        decisions: None,
        decisions_escaped: false,
    }
}

fn inventory(path: &Path) -> BTreeMap<String, (Vec<u8>, SystemTime)> {
    fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                (
                    fs::read(entry.path()).unwrap(),
                    entry.metadata().unwrap().modified().unwrap(),
                ),
            )
        })
        .collect()
}

fn record_decisions(temp: &TempDir, config: &mut ReconcileConfig, rows: &str) {
    let path = temp.join("decisions.csv");
    fs::write(
        &path,
        reconciliation::DECISION_COLUMNS.join(",") + "\n" + rows,
    )
    .unwrap();
    config.decisions = Some(path);
}

#[test]
fn a_one_cent_allocation_never_clears_and_pending_or_rejected_items_keep_their_notes() {
    let temp = TempDir::new();
    let mut config = config(
        &temp,
        "Synthetic,CLEAR,AUD,T1,2026-09-01,Shared,Debit,100,0\nSynthetic,CLEAR,AUD,T2,2026-09-02,Shared,Credit,0,99.99\n",
    );
    config.closing_balance = "0.01".into();
    for decision in ["accept", "reject", ""] {
        record_decisions(
            &temp,
            &mut config,
            &format!(
                "G,T1,{decision},Checked source records\nG,T2,{decision},Checked source records\n"
            ),
        );
        let before = inventory(&temp.0);
        let result = reconciliation::reconcile(&config);
        assert_eq!(inventory(&temp.0), before);
        if decision == "accept" {
            assert_eq!(
                result.unwrap_err(),
                "Accepted group 'G' does not sum to zero."
            );
            continue;
        }
        let result = result.unwrap();
        assert!(result.at("status").is_str("REVIEW"));
        assert!(result.at("outstanding_total").is_str("0.01"));
        assert!(result.at("movement").is_str("0.01"));
        assert_eq!(result.at("outstanding").as_array().unwrap().len(), 2);
        assert!(result.at("suggestions").as_array().unwrap().is_empty());
        let groups = result.at("decisions").as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert!(groups[0].at("group").is_str("G"));
        assert!(groups[0].at("decision").is_str(decision));
        for id in ["T1", "T2"] {
            assert!(result.at("notes").at(id).is_str("Checked source records"));
        }
    }
}

#[test]
fn an_opening_item_after_its_recorded_period_end_is_refused_without_changing_inputs() {
    let temp = TempDir::new();
    let mut config = config(&temp, "Synthetic,CLEAR,AUD,T1,2026-09-30,Old,Receipt,1,0\n");
    config.closing_balance = "1".into();
    let september = reconciliation::reconcile(&config).unwrap();
    let carry = reconciliation_report::rendered(&september)
        .unwrap()
        .into_iter()
        .find(|(name, _)| name == "carry-forward.json")
        .unwrap()
        .1;
    let mut carry = pyjson::loads(std::str::from_utf8(&carry).unwrap()).unwrap();
    let mut item = carry.at("items").as_array().unwrap()[0].clone();
    item.set("Date", Json::str("2026-10-01"));
    carry.set("items", Json::Array(vec![item]));
    let path = temp.join("opening.json");
    fs::write(&path, pyjson::pretty(&carry)).unwrap();
    config.opening_items = Some(path);
    config.period_start = "2026-10-01".into();
    config.period_end = "2026-10-31".into();
    config.opening_balance = "1".into();
    fs::write(
        &config.transactions,
        reconciliation::COLUMNS.join(",") + "\n",
    )
    .unwrap();
    let before = inventory(&temp.0);
    assert_eq!(
        reconciliation::reconcile(&config).unwrap_err(),
        "Opening item dates cannot be later than their period end."
    );
    assert_eq!(inventory(&temp.0), before);
}

#[test]
fn malformed_opening_json_is_refused_without_changing_inputs() {
    let temp = TempDir::new();
    let mut config = config(&temp, "");
    let path = temp.join("opening.json");
    config.opening_items = Some(path.clone());
    for (content, diagnostic) in [
        (
            "{} x",
            "Opening items are not readable JSON: Extra data: line 1 column 4 (char 3)",
        ),
        (
            "{\"schema\":0,\"schema\":1}",
            "Opening items are not readable JSON: JSON member 'schema' appears more than once.",
        ),
    ] {
        fs::write(&path, content).unwrap();
        let before = inventory(&temp.0);
        assert_eq!(reconciliation::reconcile(&config).unwrap_err(), diagnostic);
        assert_eq!(inventory(&temp.0), before);
    }
}

#[test]
fn opening_items_require_every_transaction_field_as_text() {
    let temp = TempDir::new();
    let mut config = config(&temp, "Synthetic,CLEAR,AUD,T1,2026-09-30,Old,Receipt,1,0\n");
    config.closing_balance = "1".into();
    let september = reconciliation::reconcile(&config).unwrap();
    let carry = reconciliation_report::rendered(&september)
        .unwrap()
        .into_iter()
        .find(|(name, _)| name == "carry-forward.json")
        .unwrap()
        .1;
    let mut carry = pyjson::loads(std::str::from_utf8(&carry).unwrap()).unwrap();
    let mut item = carry.at("items").as_array().unwrap()[0].clone();
    let Json::Object(fields) = &mut item else {
        panic!("carried transaction")
    };
    fields.retain(|(key, _)| !key.is("Credit"));
    carry.set("items", Json::Array(vec![item]));
    let path = temp.join("opening.json");
    fs::write(&path, pyjson::pretty(&carry)).unwrap();
    config.opening_items = Some(path.clone());
    config.period_start = "2026-10-01".into();
    config.period_end = "2026-10-31".into();
    config.opening_balance = "1".into();
    fs::write(
        &config.transactions,
        reconciliation::COLUMNS.join(",") + "\n",
    )
    .unwrap();
    let before = inventory(&temp.0);
    assert_eq!(
        reconciliation::reconcile(&config).unwrap_err(),
        format!(
            "{}: row 1 must contain every transaction field as text.",
            monthly_close_control_plane::pypath::text(&path)
        )
    );
    assert_eq!(inventory(&temp.0), before);
}

#[test]
fn short_transaction_and_decision_rows_are_refused_without_changing_inputs() {
    let temp = TempDir::new();
    let short = config(&temp, "Synthetic,CLEAR,AUD,T1,2026-09-01,R,Debit,1\n");
    let before = inventory(&temp.0);
    assert_eq!(
        reconciliation::reconcile(&short).unwrap_err(),
        format!(
            "{}: row 2 must contain every transaction field as text.",
            monthly_close_control_plane::pypath::text(&short.transactions)
        )
    );
    assert_eq!(inventory(&temp.0), before);
    let mut complete = config(
        &temp,
        "Synthetic,CLEAR,AUD,T1,2026-09-01,R,Debit,1,0\nSynthetic,CLEAR,AUD,T2,2026-09-02,R,Credit,0,1\n",
    );
    record_decisions(&temp, &mut complete, "G,T1,accept\n");
    let before = inventory(&temp.0);
    assert_eq!(
        reconciliation::reconcile(&complete).unwrap_err(),
        "Every decisions row must contain all four fields."
    );
    assert_eq!(inventory(&temp.0), before);
}

#[test]
fn suggestions_do_not_reuse_retained_group_labels_or_clear_any_items() {
    let temp = TempDir::new();
    let mut config = config(
        &temp,
        "Synthetic,CLEAR,AUD,T1,2026-09-01,R1,Debit,10,0\nSynthetic,CLEAR,AUD,T2,2026-09-02,R1,Credit,0,10\nSynthetic,CLEAR,AUD,T3,2026-09-03,R2,Debit,20,0\nSynthetic,CLEAR,AUD,T4,2026-09-04,R2,Credit,0,20\nSynthetic,CLEAR,AUD,T5,2026-09-05,R3,Debit,30,0\nSynthetic,CLEAR,AUD,T6,2026-09-06,R3,Credit,0,30\n",
    );
    record_decisions(
        &temp,
        &mut config,
        "S1,T1,reject,Reviewed but unrelated\nS1,T2,reject,Reviewed but unrelated\nnew-S1,T3,,Pending investigation\nnew-S1,T4,,Pending investigation\n",
    );
    let before = inventory(&temp.0);
    let result = reconciliation::reconcile(&config).unwrap();
    assert_eq!(inventory(&temp.0), before);
    assert!(result.at("status").is_str("REVIEW"));
    assert_eq!(result.at("outstanding").as_array().unwrap().len(), 6);
    assert!(result.at("outstanding_total").is_str("0.00"));
    let suggestions = result.at("suggestions").as_array().unwrap();
    assert_eq!(suggestions.len(), 1);
    assert!(suggestions[0].at("group").is_str("new-new-S1"));
    assert!(pyjson::py_eq(
        suggestions[0].at("ids"),
        &Json::Array(vec![Json::str("T5"), Json::str("T6")]),
        true
    ));
    let groups = result.at("decisions").as_array().unwrap();
    assert_eq!(groups.len(), 2);
    assert!(groups[0].at("group").is_str("S1"));
    assert!(groups[0].at("decision").is_str("reject"));
    assert!(groups[1].at("group").is_str("new-S1"));
    assert!(groups[1].at("decision").is_str(""));
}
