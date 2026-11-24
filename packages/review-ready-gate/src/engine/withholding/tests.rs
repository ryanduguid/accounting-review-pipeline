// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use crate::{
    decimal::Decimal,
    engine::review_pack,
    models::ReadinessPack,
    test_support::{self, TempDir},
};
use std::{fs, path::PathBuf};

const STATEMENT: &str = "activity_statement.csv";
const CONTROL: &str = "paygw_control_gl.csv";
const TB: &str = "trial_balance.csv";

/// The ias-ready pack with each `(file, text)` written over its file.
fn pack(tmp: &TempDir, files: &[(&str, &str)]) -> PathBuf {
    let dir = tmp.join("pack");
    test_support::copy_example("ias-ready", &dir);
    for (name, text) in files {
        fs::write(dir.join(name), text).unwrap();
    }
    dir
}
fn run(profile: &str, dir: &std::path::Path) -> crate::Result<ReadinessPack> {
    review_pack(profile, dir, None, Decimal::parse("0.01").unwrap(), &[])
}
fn review(profile: &str, files: &[(&str, &str)]) -> ReadinessPack {
    let tmp = TempDir::new();
    run(profile, &pack(&tmp, files)).unwrap()
}
fn reasons(pack: &ReadinessPack) -> Vec<String> {
    pack.findings
        .iter()
        .map(|f| format!("{} {} {}", f.code, f.slot, f.reason))
        .collect()
}
fn example_tb() -> String {
    fs::read_to_string(test_support::examples().join("ias-ready").join(TB)).unwrap()
}
fn control(rows: &[(&str, &str)]) -> String {
    let mut text = "Date,AccountID,AccountName,Debit,Credit,Description\n".to_owned();
    for (date, credit) in rows {
        text.push_str(&format!(
            "{date},830,PAYG Withholding Payable,0.00,{credit},Pay run\n"
        ));
    }
    text
}

#[test]
fn an_ias_pack_with_no_gst_labels_is_ready() {
    let pack = review("ias", &[]);
    assert_eq!(pack.status, "READY", "{:?}", reasons(&pack));
    assert_eq!(pack.engagement_type, "ias");
    assert!(
        pack.source_evidence
            .iter()
            .any(|e| e.slot == "paygw_control_gl")
    );
}

#[test]
fn the_bas_profile_still_needs_gst_and_reads_an_optional_paygw_file() {
    let ready = run("bas", &test_support::examples().join("bas-ready")).unwrap();
    assert_eq!(ready.status, "READY");
    assert!(
        ready
            .controls_not_run
            .iter()
            .flatten()
            .any(|c| c.slot == "paygw_control_gl")
    );
    let tmp = TempDir::new();
    let dir = pack(&tmp, &[]);
    fs::write(
        dir.join("self_review.json"),
        fs::read_to_string(dir.join("self_review.json"))
            .unwrap()
            .replace("\"ias\"", "\"bas\""),
    )
    .unwrap();
    let pack = run("bas", &dir).unwrap();
    assert_eq!(
        reasons(&pack),
        [
            "MISSING_ARTEFACT gst_control_gl Required artefact gst_control_gl is not in the pack directory."
        ]
    );
}

#[test]
fn statement_arithmetic_breaks_are_named() {
    let statement = "Label,Amount\nW1,100\nW2,200\nW3,10\nW5,205\n4,200\n7D,-5\n1C,0\n";
    let got = reasons(&review("ias", &[(STATEMENT, statement)]));
    for expected in [
        "TIEOUT_BREAK activity_statement Activity statement label 7D is negative (-5); the ATO accepts no negative figures.",
        "TIEOUT_BREAK activity_statement Activity statement W5 (205) is not W2 + W4 + W3 (210.00).",
        "TIEOUT_BREAK activity_statement Activity statement label 4 (200) is not W5 (205); the W5 total is copied to 4.",
        "TIEOUT_BREAK activity_statement Activity statement W2 (200) exceeds W1 (100); W2 is withheld from the payments shown at W1.",
    ] {
        assert!(got.iter().any(|r| r == expected), "{expected}\n{got:#?}");
    }
    assert!(!got.iter().any(|r| r.contains("label 1C")), "{got:#?}");
}

#[test]
fn w2_without_w1_is_accepted_as_single_touch_payroll_reporting() {
    let got = reasons(&review(
        "ias",
        &[(
            STATEMENT,
            "Label,Amount
W2,5200
W5,5200
",
        )],
    ));
    assert!(got.is_empty(), "{got:?}");
}

fn tie_reasons(statement: &str, rows: &[(&str, &str)]) -> Vec<String> {
    review("ias", &[(STATEMENT, statement), (CONTROL, &control(rows))])
        .findings
        .iter()
        .filter(|f| f.slot == "paygw_control_gl" && f.reason.starts_with("PAYG withholding on"))
        .map(|f| f.reason.clone())
        .collect()
}

#[test]
fn whole_dollar_labels_tie_when_the_ledger_is_higher_by_under_a_dollar_each() {
    let w5 = "Label,Amount\nW1,24000\nW2,5200\nW5,5200\n";
    assert!(tie_reasons(w5, &[("2026-01-15", "5200.99")]).is_empty());
    // Rounding down never leaves the statement higher than the ledger.
    assert_eq!(
        tie_reasons(w5, &[("2026-01-15", "5199.60")]),
        [
            "PAYG withholding on the activity statement (W5 5200) does not tie to PAYG withholding control movement (5199.60); difference 0.40 exceeds tolerance 0.01 or the allowance of under $1 for each of the 1 whole-dollar withholding labels."
        ]
    );
    assert_eq!(
        tie_reasons(w5, &[("2026-01-15", "5201.00")]),
        [
            "PAYG withholding on the activity statement (W5 5200) does not tie to PAYG withholding control movement (5201.00); difference -1.00 exceeds tolerance 0.01 or the allowance of under $1 for each of the 1 whole-dollar withholding labels."
        ]
    );
    // W5 alone stands for 3 labels rounded down, though its arithmetic is listed.
    let alone = tie_reasons(
        "Label,Amount
W5,5200
",
        &[("2026-01-15", "5202.97")],
    );
    assert!(alone.is_empty(), "{alone:?}");
    assert_eq!(
        tie_reasons(
            "Label,Amount
W5,5200
",
            &[("2026-01-15", "5203.00")]
        )
        .len(),
        1
    );
    // Two labels, up to two dollars of rounding.
    let parts = "Label,Amount\nW1,24000\nW2,5200\nW4,100\n";
    assert!(tie_reasons(parts, &[("2026-01-15", "5301.98")]).is_empty());
    assert_eq!(tie_reasons(parts, &[("2026-01-15", "5302.00")]).len(), 1);
}

#[test]
fn labels_with_cents_get_no_allowance_and_a_hint_names_the_posting() {
    let got = tie_reasons(
        "Label,Amount\nW1,24000\nW2,5200.40\n",
        &[("2026-01-15", "5200.40"), ("2026-01-20", "12.00")],
    );
    assert_eq!(
        got,
        [
            "PAYG withholding on the activity statement (W2 + W4 + W3 5200.40) does not tie to PAYG withholding control movement (5212.40); difference -12.00 exceeds tolerance 0.01. Hint: the difference is the size of the 2026-01-20 posting 'Pay run' (credit 12.00); check that posting first."
        ]
    );
}

#[test]
fn a_control_file_with_no_withholding_labels_cannot_be_tied() {
    let w1_only = "Label,Amount\nW1,24000\n";
    assert!(
        reasons(&review("ias", &[(STATEMENT, w1_only)])).contains(
            &"TIEOUT_BREAK paygw_control_gl Activity statement has none of W2, W3, W4 or W5, so the PAYG withholding control file cannot be tied.".to_owned()
        )
    );
    // A large withholder reports W1 only and leaves the file out.
    let tmp = TempDir::new();
    let dir = pack(&tmp, &[(STATEMENT, w1_only)]);
    fs::remove_file(dir.join(CONTROL)).unwrap();
    let pack = run("ias", &dir).unwrap();
    assert_eq!(pack.status, "READY", "{:?}", reasons(&pack));
}

#[test]
fn the_control_accounts_roll_forward_to_the_trial_balance() {
    let tb = example_tb();
    let carried = tb
        .replace("0.00,5200.00,0.00,5200.00", "0.00,5200.00,0.00,7800.00")
        .replace("0.00,0.00,0.00,50000.00", "0.00,0.00,0.00,47400.00");
    assert_eq!(
        reasons(&review("ias", &[(TB, &carried)])),
        [
            "TIEOUT_BREAK paygw_control_gl PAYG withholding control accounts close at 7800.00 in the trial balance, carry in nil, as the activity statement declares no PAYGW_OPENING, and the control file moves them by 5200.00; difference 2600.00 exceeds tolerance 0.01. The file may be missing postings, or the accounts carried in a balance, such as an earlier period still unpaid, which PAYGW_OPENING declares."
        ]
    );
    let declared = "Label,Amount\nW1,24000\nW2,5200\nW5,5200\n4,5200\nPAYGW_OPENING,2600.00\n";
    assert_eq!(
        review("ias", &[(TB, &carried), (STATEMENT, declared)]).status,
        "READY"
    );
    let wrong = declared.replace("2600.00", "1000.00");
    assert!(
        reasons(&review("ias", &[(TB, &carried), (STATEMENT, &wrong)]))[0]
            .contains("carry in 1000.00, the activity statement's PAYGW_OPENING,")
    );
    let elsewhere = tb.replace(",830,", ",831,");
    assert_eq!(
        reasons(&review("ias", &[(TB, &elsewhere)])),
        [
            "TIEOUT_BREAK paygw_control_gl PAYG withholding control file accounts are not in the trial balance: 830. The control accounts cannot be rolled forward to the ledger."
        ]
    );
}

#[test]
fn postings_outside_the_declared_period_are_named() {
    let rows = [("2025-12-31", "2600.00"), ("2026-02-01", "2600.00")];
    let got: Vec<_> = reasons(&review("ias", &[(CONTROL, &control(&rows))]))
        .into_iter()
        .filter(|r| r.starts_with("PERIOD_ORDER"))
        .collect();
    assert_eq!(
        got,
        [
            "PERIOD_ORDER paygw_control_gl PAYG withholding control postings are dated before the declared period_start 2026-01-01: 2025-12-31.",
            "PERIOD_ORDER paygw_control_gl PAYG withholding control postings are dated after the declared period_end 2026-01-31: 2026-02-01.",
        ]
    );
}

#[test]
fn a_header_only_control_file_is_refused_in_its_own_words() {
    let tmp = TempDir::new();
    let dir = pack(
        &tmp,
        &[(
            CONTROL,
            "Date,AccountID,AccountName,Debit,Credit,Description\n",
        )],
    );
    let refusal = run("ias", &dir).unwrap_err();
    assert!(
        refusal.ends_with("no PAYG withholding control rows were supplied."),
        "{refusal}"
    );
}

const PRECISION: &str =
    "BAS amounts exceed the supported 28-digit precision, so net GST cannot be tied exactly.";
/// An amount with 28 significant digits, 2 of which overflow the context.
const BIG: &str = "60000000000000000000000000.01";

#[test]
fn withholding_beyond_28_digits_is_refused() {
    let tmp = TempDir::new();
    let statement = format!("Label,Amount\nW1,24000\nW2,{BIG}\nW4,{BIG}\n");
    let dir = pack(&tmp, &[(STATEMENT, &statement)]);
    assert_eq!(run("ias", &dir).unwrap_err(), PRECISION);
}

#[test]
fn control_balances_beyond_28_digits_are_refused() {
    let tmp = TempDir::new();
    let row = |id: &str, name: &str, debit: &str, credit: &str| {
        format!(
            "2026-01-31,Cedar and Pine Consulting Pty Ltd,Assets,{id},{name},{id},{debit},{credit},{debit},{credit}\n"
        )
    };
    let tb = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n".to_owned()
        + &row("100", "Bank", BIG, "0.00")
        + &row("830", "PAYGW", "0.00", BIG)
        + &row("101", "Bank 2", BIG, "0.00")
        + &row("831", "PAYGW 2", "0.00", BIG);
    let control = "Date,AccountID,AccountName,Debit,Credit,Description\n2026-01-15,830,PAYGW,0.00,1.00,Pay run\n2026-01-15,831,PAYGW 2,0.00,1.00,Pay run\n";
    let dir = pack(
        &tmp,
        &[
            (TB, &tb),
            (CONTROL, control),
            (STATEMENT, "Label,Amount\nW1,24000\n"),
        ],
    );
    assert_eq!(run("ias", &dir).unwrap_err(), PRECISION);
}

#[test]
fn an_account_in_both_control_files_is_not_rolled_forward() {
    // GST movement 100 carried in at 50, PAYG movement 50 carried in at 100:
    // each roll-forward alone ties the shared account's closing 150.
    let tb = example_tb()
        .replace("0.00,5200.00,0.00,5200.00", "0.00,150.00,0.00,150.00")
        .replace("24000.00,0.00,24000.00,0.00", "500.00,0.00,500.00,0.00")
        .replace("0.00,18800.00,31200.00,0.00", "0.00,350.00,49650.00,0.00");
    let gst = "Date,AccountID,AccountName,Debit,Credit,Description
2026-01-20,830,Tax payable,0.00,100.00,GST
";
    let statement = "Label,Amount
1A,100.00
1B,0.00
GST_OPENING,50.00
W1,500
W2,50
W5,50
4,50
PAYGW_OPENING,100.00
";
    let review = fs::read_to_string(test_support::examples().join("ias-ready/self_review.json"))
        .unwrap()
        .replace("\"ias\"", "\"bas\"");
    let tmp = TempDir::new();
    let dir = pack(
        &tmp,
        &[
            (TB, &tb),
            ("gst_control_gl.csv", gst),
            (CONTROL, &control(&[("2026-01-15", "50.00")])),
            (STATEMENT, statement),
            ("self_review.json", &review),
        ],
    );
    let pack = run("bas", &dir).unwrap();
    assert_eq!(
        reasons(&pack),
        [
            "TIEOUT_BREAK paygw_control_gl Accounts are in both the GST and the PAYG withholding control files: 830. Neither roll-forward explains their closing balance alone."
        ]
    );
}
