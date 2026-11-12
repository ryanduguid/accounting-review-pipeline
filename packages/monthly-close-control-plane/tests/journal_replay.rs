// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! A fabricated exporter-to-close hand-off. The replay checks each journal
//! independently. The close command checks the derived closing balances.

#[path = "support/temp.rs"]
mod temp;

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

use monthly_close_control_plane::{decimal::Decimal, pycsv, pyjson, viewer};
use std::{collections::BTreeMap, fs};
use temp::TempDir;

const JOURNALS: &str = "Journal ID,Account ID,Debit,Credit\n\
    Opening,bank,1000.00,0.00\n\
    Opening,capital,0.00,1000.00\n\
    Invoice,cost,120.00,0.00\n\
    Invoice,payables,0.00,120.00\n\
    Payment,payables,40.00,0.00\n\
    Payment,bank,0.00,40.00\n";

fn replay(csv: &str) -> (BTreeMap<String, Decimal>, Vec<String>) {
    let mut rows = pycsv::Reader::strict(csv);
    assert_eq!(
        rows.next().unwrap().unwrap(),
        ["Journal ID", "Account ID", "Debit", "Credit"]
    );
    let mut accounts = BTreeMap::<_, Decimal>::new();
    let mut journals = BTreeMap::<_, Decimal>::new();
    for row in rows {
        let [journal, account, debit, credit]: [String; 4] = row.unwrap().try_into().unwrap();
        let debit = Decimal::parse(&debit).unwrap();
        let credit = Decimal::parse(&credit).unwrap();
        let net = debit.sub(&credit).value;
        let account_sum = accounts.entry(account).or_insert_with(Decimal::zero);
        *account_sum = account_sum.add(&net).value;
        let journal_sum = journals.entry(journal).or_insert_with(Decimal::zero);
        *journal_sum = journal_sum.add(&net).value;
    }
    let unbalanced = journals
        .into_iter()
        .filter_map(|(id, sum)| (!sum.is_zero()).then_some(id))
        .collect();
    (accounts, unbalanced)
}

fn trial_balance(date: &str, bank: &str, capital: &str, cost: &str, payables: &str) -> String {
    format!(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n\
         {date},Fabricated Studio,Assets,bank,Bank,090,0.00,0.00,{bank},0.00\n\
         {date},Fabricated Studio,Equity,capital,Capital,300,0.00,0.00,0.00,{capital}\n\
         {date},Fabricated Studio,Expenses,cost,Cost,500,0.00,0.00,{cost},0.00\n\
         {date},Fabricated Studio,Liabilities,payables,Payables,200,0.00,0.00,0.00,{payables}\n"
    )
}

fn check_close(journals: &str, cost: &str, payables: &str, failures: &[&str]) {
    let temp = TempDir::new();
    let (balances, _) = replay(journals);
    let mut subledger = "Tenant,AccountID,SubledgerBalance\n".to_owned();
    for (account, balance) in balances {
        subledger.push_str(&format!(
            "Fabricated Studio,{account},{}\n",
            balance.format_places(2).unwrap()
        ));
    }
    let current = temp.join("current.csv");
    let prior = temp.join("prior.csv");
    let subledger_path = temp.join("journal-balances.csv");
    let output = temp.join("pack");
    fs::write(
        &current,
        trial_balance("2026-08-31", "960.00", "1000.00", cost, payables),
    )
    .unwrap();
    fs::write(
        &prior,
        trial_balance("2026-07-31", "1000.00", "1000.00", "0.00", "0.00"),
    )
    .unwrap();
    fs::write(&subledger_path, subledger).unwrap();
    let run = child_profile::command(env!("CARGO_BIN_EXE_close-control"))
        .arg("review")
        .arg("--current")
        .arg(current)
        .arg("--prior")
        .arg(prior)
        .arg("--subledger")
        .arg(subledger_path)
        .args(["--require-control", "subledger_reconciliation"])
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert_eq!(
        run.status.code(),
        Some(if failures.is_empty() { 0 } else { 2 }),
        "{run:?}"
    );
    viewer::verify_pack(&output).unwrap();
    let pack =
        pyjson::loads(&fs::read_to_string(output.join("close-review-pack.json")).unwrap()).unwrap();
    assert_eq!(failed_accounts(&pack), failures);
}

/// Every exception must come from the subledger control: the trial balances
/// stay balanced, so any other control raising is a fixture or engine fault.
fn failed_accounts(pack: &pyjson::Json) -> Vec<String> {
    pack.at("exceptions")
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let text = |key: &str| item.at(key).as_pystr().unwrap().to_utf8().unwrap();
            assert_eq!(text("control"), "subledger_reconciliation");
            text("account_id")
        })
        .collect()
}

#[test]
fn balanced_journals_can_still_fail_the_closing_trial_balance() {
    assert!(replay(JOURNALS).1.is_empty());
    check_close(JOURNALS, "120.00", "80.00", &[]);
    check_close(JOURNALS, "127.00", "87.00", &["cost", "payables"]);
}

#[test]
fn an_altered_journal_fails_balancing_and_changes_the_bank_reconciliation() {
    let altered = JOURNALS.replace("Payment,bank,0.00,40.00", "Payment,bank,0.00,39.00");
    assert_eq!(replay(&altered).1, ["Payment"]);
    check_close(&altered, "120.00", "80.00", &["bank"]);
}
