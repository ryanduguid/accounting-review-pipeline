// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::*;
use crate::{
    engine,
    pyjson::{self, Json},
    report,
    test_support::{self, TempDir},
    viewer,
};
use std::fs;

fn tolerance() -> Decimal {
    Decimal::parse("0.01").unwrap()
}

fn copy_pack(tmp: &TempDir, example: &str, name: &str) -> std::path::PathBuf {
    let dir = tmp.join(name);
    test_support::copy_example(example, &dir);
    dir
}

fn minimal_zip(entries: &[&str]) -> Vec<u8> {
    let mut local = Vec::new();
    let mut central = Vec::new();
    for name in entries {
        let offset = local.len() as u32;
        let name_bytes = name.as_bytes();
        let name_len = (name_bytes.len() as u16).to_le_bytes();
        local.extend_from_slice(b"PK\x03\x04");
        local.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        local.extend_from_slice(&[0; 12]);
        local.extend_from_slice(&name_len);
        local.extend_from_slice(&[0, 0]);
        local.extend_from_slice(name_bytes);

        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&name_len);
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }
    let cd_offset = (local.len() as u32).to_le_bytes();
    let cd_size = (central.len() as u32).to_le_bytes();
    let count = (entries.len() as u16).to_le_bytes();
    let mut out = local;
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&count);
    out.extend_from_slice(&count);
    out.extend_from_slice(&cd_size);
    out.extend_from_slice(&cd_offset);
    out.extend_from_slice(&[0, 0]);
    out
}

#[test]
fn self_review_contradictions_check_no_with_figures_yes_without_and_required_artefacts() {
    let tmp = TempDir::new();
    let dir = copy_pack(&tmp, "bas-ready", "bas-contra");
    let review = r#"{
  "preparer_initials": "AB",
  "prepared_on": "2026-04-10",
  "engagement_type": "bas",
  "period_end": "2026-03-31",
  "assertions": {
    "pack_complete": true,
    "tie_outs_done": true,
    "open_items_listed": true,
    "variances_explained": true,
    "self_reviewed": true
  },
  "items": [
    {"name": "gst_sales", "answer": false, "covers": ["1A"]},
    {"name": "cgt_events", "answer": true, "covers": ["9999"], "requires_artefact": "cgt_schedule.csv"},
    {"name": "bank_active", "answer": true, "covers": ["1000"], "requires_artefact": "support.txt"},
    {"name": "zero_label", "answer": false, "covers": ["NO_SUCH"]}
  ]
}"#;
    fs::write(dir.join("self_review.json"), review).unwrap();
    fs::write(dir.join("support.txt"), b"supporting schedule").unwrap();
    let pack = engine::review_pack("bas", &dir, None, tolerance(), &[]).unwrap();
    assert_eq!(pack.status, "NOT_READY");
    let contras: Vec<_> = pack
        .findings
        .iter()
        .filter(|f| f.code == "SELF_REVIEW_CONTRADICTION")
        .collect();
    assert_eq!(contras.len(), 3, "{contras:?}");
    assert!(contras[0].reason.contains("gst_sales") && contras[0].reason.contains("1A="));
    assert!(contras[1].reason.contains("cgt_events") && contras[1].reason.contains("9999"));
    assert!(contras[2].reason.contains("cgt_schedule.csv"));
    assert_eq!(contras[0].evidence.len(), 1);
    assert_eq!(contras[0].evidence[0].file, "self_review.json");
    assert_eq!(contras[0].evidence[0].locator, "item:gst_sales");
}

#[test]
fn self_review_movement_only_tb_account_triggers_contradiction_and_validation_rejects_bad_items() {
    let tmp = TempDir::new();
    let dir = copy_pack(&tmp, "month-end-ready", "mov");
    let tb = "\
ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit
2026-06-30,Varrock Ventures Pty Ltd,Assets,100,Operating Bank,1000,100.00,0.00,120000.00,0.00
2026-06-30,Varrock Ventures Pty Ltd,Equity,900,Retained Earnings,3000,0.00,100.00,0.00,120000.00
2026-06-30,Varrock Ventures Pty Ltd,Expenses,550,Clearing Movement,5500,50.00,0.00,0.00,0.00
2026-06-30,Varrock Ventures Pty Ltd,Expenses,551,Clearing Offset,5501,0.00,50.00,0.00,0.00
";
    fs::write(dir.join("trial_balance.csv"), tb).unwrap();
    let review = r#"{
  "preparer_initials": "CD",
  "prepared_on": "2026-07-05",
  "engagement_type": "month_end",
  "period_end": "2026-06-30",
  "assertions": {
    "pack_complete": true,
    "tie_outs_done": true,
    "open_items_listed": true,
    "variances_explained": true,
    "self_reviewed": true
  },
  "items": [{"name": "clearing", "answer": false, "covers": ["5500"]}]
}"#;
    fs::write(dir.join("self_review.json"), review).unwrap();
    let pack = engine::review_pack("month_end", &dir, None, tolerance(), &[]).unwrap();
    assert!(
        pack.findings
            .iter()
            .any(|f| f.code == "SELF_REVIEW_CONTRADICTION" && f.reason.contains("5500=50.00"))
    );

    let bad_cases = [
        r#""items": "not-a-list""#,
        r#""items": [{}]"#,
        r#""items": [{"name": "", "answer": true, "covers": ["1A"]}]"#,
        r#""items": [{"name": 7, "answer": true, "covers": ["1A"]}]"#,
        r#""items": [{"name": "x", "answer": "yes", "covers": ["1A"]}]"#,
        r#""items": [{"name": "x", "answer": true, "covers": []}]"#,
        r#""items": [{"name": "x", "answer": true, "covers": ["1A"], "requires_artefact": "../bad.csv"}]"#,
        r#""schedules": ["unknown_schedule"]"#,
        r#""schedules": "not-a-list""#,
    ];
    for extra in bad_cases {
        let bad = format!(
            "{{\"preparer_initials\":\"CD\",\"prepared_on\":\"2026-07-05\",\"engagement_type\":\"month_end\",\"period_end\":\"2026-06-30\",\"assertions\":{{\"pack_complete\":true,\"tie_outs_done\":true,\"open_items_listed\":true,\"variances_explained\":true,\"self_reviewed\":true}},{extra}}}"
        );
        let path = tmp.join("bad_self_review.json");
        fs::write(&path, bad).unwrap();
        let snap = SourceSnapshot::capture(&path, "self_review").unwrap();
        assert!(
            crate::loader::load_self_review(&snap, "month_end").is_err(),
            "{extra}"
        );
    }
}

#[test]
fn bank_coverage_classifies_accounts_blocks_year_end_and_lists_month_end_controls_not_run() {
    let tmp = TempDir::new();
    let tb = "\
ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit
2026-06-30,Varrock Ventures Pty Ltd,Bank,100,Operating Bank,1000,15000.00,0.00,120000.00,0.00
2026-06-30,Varrock Ventures Pty Ltd,Bank,101,Savings Bank,1001,1000.00,0.00,5000.00,0.00
2026-06-30,Varrock Ventures Pty Ltd,Bank,102,Trust Bank,1002,500.00,0.00,2000.00,0.00
2026-06-30,Varrock Ventures Pty Ltd,Bank,103,Term Deposit,1003,0.00,0.00,3000.00,0.00
2026-06-30,Varrock Ventures Pty Ltd,Bank,104,FX Clearing,1004,0.00,0.00,1000.00,0.00
2026-06-30,Varrock Ventures Pty Ltd,Equity,900,Retained Earnings,3000,0.00,16500.00,0.00,131000.00
";
    let rec = "\
AccountID,StatementBalance,GLBalance,AsAtDate,Status
100,120000.00,120000.00,2026-06-30,RECONCILED
101,4900.00,5000.00,2026-06-30,DIFFERENCE
102,2000.00,1900.00,2026-06-30,
103,,,2026-06-30,UNVERIFIED
104,1000.00,1000.00,2026-05-31,RECONCILED
";
    let dir = copy_pack(&tmp, "year-end-ready", "ye-bank");
    fs::write(dir.join("trial_balance.csv"), tb).unwrap();
    fs::write(dir.join("bank_rec.csv"), rec).unwrap();
    let pack = engine::review_pack("year_end", &dir, None, tolerance(), &[]).unwrap();
    assert_eq!(pack.status, "BLOCKED");
    let unverified: Vec<_> = pack
        .findings
        .iter()
        .filter(|f| f.code == "BANK_REC_UNVERIFIED")
        .collect();
    assert_eq!(unverified.len(), 2, "{unverified:?}");
    assert!(
        unverified[0]
            .reason
            .contains("No evidence sources were declared searched")
    );
    assert!(
        unverified[1]
            .reason
            .contains("does not match period_end 2026-06-30")
    );

    // Declaring searched evidence sources raises a client statement query.
    let review_searched = fs::read_to_string(dir.join("self_review.json"))
        .unwrap()
        .replace(
            "\"period_end\": \"2026-06-30\",",
            "\"period_end\": \"2026-06-30\",\n  \"bank_evidence_searched\": [\"client_folder\", \"mailbox\"],",
        );
    fs::write(dir.join("self_review.json"), review_searched).unwrap();
    fs::remove_file(dir.join("bank_rec.csv")).unwrap();
    let missing_pack = engine::review_pack("year_end", &dir, None, tolerance(), &[]).unwrap();
    assert_eq!(missing_pack.status, "BLOCKED");
    assert!(missing_pack.findings.iter().any(|f| {
        f.code == "BANK_REC_UNVERIFIED"
            && f.reason.contains("Client query:")
            && f.reason.contains("client_folder, mailbox")
    }));

    // Month-end lists unverified Bank accounts under controls_not_run.
    let me = copy_pack(&tmp, "month-end-ready", "me-bank");
    fs::write(me.join("trial_balance.csv"), tb).unwrap();
    fs::write(me.join("bank_rec.csv"), rec).unwrap();
    let me_pack = engine::review_pack("month_end", &me, None, tolerance(), &[]).unwrap();
    let not_run = me_pack.controls_not_run.unwrap();
    assert!(not_run.iter().any(|c| c.slot == "bank_rec:103"));
    assert!(not_run.iter().any(|c| c.slot == "bank_rec:104"));

    // Invalid Status value in bank_rec.csv is refused.
    let bad_rec = tmp.join("bad_bank_rec.csv");
    fs::write(
        &bad_rec,
        "AccountID,StatementBalance,GLBalance,Status\n100,10.00,10.00,BOGUS\n",
    )
    .unwrap();
    let snap = SourceSnapshot::capture(&bad_rec, "bank_rec").unwrap();
    assert!(crate::loader::load_bank_rec(&snap).is_err());

    // Direct classification helper verifies all 3 verdicts and empty TB path.
    let empty_loaded = Loaded::default();
    assert!(
        classify_bank_accounts(&empty_loaded, &tolerance())
            .unwrap()
            .is_empty()
    );
    let tb_snap = SourceSnapshot::capture(&me.join("trial_balance.csv"), "tb").unwrap();
    let rec_snap = SourceSnapshot::capture(&me.join("bank_rec.csv"), "rec").unwrap();
    let sr_snap = SourceSnapshot::capture(&me.join("self_review.json"), "sr").unwrap();
    let loaded = Loaded {
        trial_balance: Some(crate::loader::load_canonical_tb(&tb_snap).unwrap()),
        bank_rec: Some(crate::loader::load_bank_rec(&rec_snap).unwrap()),
        self_review: Some(crate::loader::load_self_review(&sr_snap, "month_end").unwrap()),
        ..Loaded::default()
    };
    let cov = classify_bank_accounts(&loaded, &tolerance()).unwrap();
    assert_eq!(cov.len(), 5);
    assert_eq!(cov[0].verdict, BankVerdict::Reconciled);
    assert!(matches!(cov[1].verdict, BankVerdict::Difference(_)));
    assert!(matches!(cov[2].verdict, BankVerdict::Difference(_)));
    assert!(matches!(cov[3].verdict, BankVerdict::Unverified(_)));
    assert!(matches!(cov[4].verdict, BankVerdict::Unverified(_)));
}

#[test]
fn pack_evidence_controls_flag_unreferenced_damaged_and_schedule_required_files() {
    let tmp = TempDir::new();
    let dir = copy_pack(&tmp, "year-end-ready", "ev-ctrl");
    let review = fs::read_to_string(dir.join("self_review.json"))
        .unwrap()
        .replace(
            "\"period_end\": \"2026-06-30\",",
            "\"period_end\": \"2026-06-30\",\n  \"schedules\": [\"fixed_assets\", \"loans\"],",
        );
    fs::write(dir.join("self_review.json"), review).unwrap();
    fs::write(dir.join("loans_empty_placeholder"), b"").unwrap();
    fs::remove_file(dir.join("loans_empty_placeholder")).unwrap();
    fs::write(dir.join("loan_schedule.csv"), b"").unwrap();
    fs::write(dir.join("broken.pdf"), b"%PDF-1.4 truncated").unwrap();
    fs::write(
        dir.join("zero_sheets.xlsx"),
        minimal_zip(&["xl/workbook.xml"]),
    )
    .unwrap();
    fs::write(dir.join("stray.txt"), b"unreferenced file").unwrap();
    let note = dir.join("ack.json");
    fs::write(
        &note,
        r#"{"reviewer_initials":"RV","reviewed_on":"2026-07-05","comment":"checked"}"#,
    )
    .unwrap();
    let pack = engine::review_pack("year_end", &dir, Some(&note), tolerance(), &[]).unwrap();
    assert_eq!(pack.status, "NOT_READY");
    let has = |code: &str, needle: &str| {
        pack.findings
            .iter()
            .any(|f| f.code == code && f.reason.contains(needle))
    };
    assert!(has("MISSING_ARTEFACT", "fixed_asset_register.csv"));
    assert!(has("EMPTY_ARTEFACT", "loan_schedule.csv"));
    assert!(has("DAMAGED_ARTEFACT", "broken.pdf"));
    assert!(has("DAMAGED_ARTEFACT", "zero_sheets.xlsx"));
    assert!(has("UNREFERENCED_EVIDENCE", "stray.txt"));
    assert!(!has("UNREFERENCED_EVIDENCE", "ack.json"));

    // Intact PDF and intact XLSX with a worksheet pass structural validation.
    assert!(pdf_intact(b"%PDF-1.7\nbody\n%%EOF\n"));
    assert!(!pdf_intact(b"NOTPDF\n%%EOF\n"));
    let valid_xlsx = minimal_zip(&["xl/workbook.xml", "xl/worksheets/sheet1.xml"]);
    assert!(xlsx_has_worksheets(&valid_xlsx));
    assert!(!xlsx_has_worksheets(b"short"));
    assert!(!xlsx_has_worksheets(&[
        b'P', b'K', 3, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
    ]));
    let mut bad_cd = valid_xlsx.clone();
    let cd_pos = bad_cd.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
    bad_cd[cd_pos] = b'X';
    assert!(!xlsx_has_worksheets(&bad_cd));
    let mut trunc_cd = valid_xlsx.clone();
    trunc_cd[cd_pos + 28] = 250;
    trunc_cd[cd_pos + 29] = 10;
    assert!(!xlsx_has_worksheets(&trunc_cd));

    // When required schedule file exists and is non-empty, no schedule finding is raised.
    fs::write(dir.join("fixed_asset_register.csv"), b"id,name\n1,Ute\n").unwrap();
    fs::write(dir.join("loan_schedule.csv"), b"id,bal\n1,100.00\n").unwrap();
    fs::remove_file(dir.join("broken.pdf")).unwrap();
    fs::remove_file(dir.join("zero_sheets.xlsx")).unwrap();
    fs::remove_file(dir.join("stray.txt")).unwrap();
    let clean = engine::review_pack("year_end", &dir, Some(&note), tolerance(), &[]).unwrap();
    assert_eq!(clean.status, "READY", "{:?}", clean.findings);
}

#[test]
fn evidence_links_round_trip_through_render_and_view_refuses_tampering_and_escapes() {
    let tmp = TempDir::new();
    let dir = copy_pack(&tmp, "bas-ready", "ev-link");
    fs::write(dir.join("extra note.txt"), b"unreferenced").unwrap();
    let pack = engine::review_pack("bas", &dir, None, tolerance(), &[]).unwrap();
    let output = tmp.join("rendered");
    report::write_review_pack(&pack, &output).unwrap();

    let verified = viewer::verify_pack(&output).unwrap();
    assert!(verified.summary.contains("| Evidence |"));
    assert!(
        verified
            .summary
            .contains("[extra note.txt (file)](extra%20note.txt)")
    );
    let csv = fs::read_to_string(output.join("findings.csv")).unwrap();
    assert!(csv.contains("evidence_file"));
    assert!(csv.contains("extra note.txt"));
    assert_eq!(report::evidence_markdown(&[]), "-");

    // Validate unsafe evidence path rejections.
    for bad in [
        "",
        " leading.txt",
        "../outside.txt",
        "/etc/passwd",
        "a\\b.txt",
        "https://example.com/a.pdf",
        "javascript:alert(1)",
        "data:text/plain,hi",
        "C:file.txt",
        "a//b.txt",
        "./a.txt",
        "bad(name).txt",
    ] {
        assert!(report::validate_evidence_file(bad).is_err(), "{bad}");
    }

    // Tampered CSV evidence_file disagrees.
    let bad_csv = tmp.join("bad-csv");
    report::write_review_pack(&pack, &bad_csv).unwrap();
    let edited = fs::read_to_string(bad_csv.join("findings.csv"))
        .unwrap()
        .replace("extra note.txt", "other.txt");
    fs::write(bad_csv.join("findings.csv"), edited).unwrap();
    assert!(
        viewer::verify_pack(&bad_csv)
            .unwrap_err()
            .contains("evidence_file disagrees")
    );

    // Tampered Markdown evidence link disagrees.
    let bad_md = tmp.join("bad-md");
    report::write_review_pack(&pack, &bad_md).unwrap();
    let edited = fs::read_to_string(bad_md.join("readiness-summary.md"))
        .unwrap()
        .replace("extra%20note.txt", "other.txt");
    fs::write(bad_md.join("readiness-summary.md"), edited).unwrap();
    assert!(
        viewer::verify_pack(&bad_md)
            .unwrap_err()
            .contains("evidence disagrees")
    );

    // Invalid JSON evidence shapes are rejected by verify_schema.
    let base_json = fs::read_to_string(output.join("readiness-pack.json")).unwrap();
    let mut doc = pyjson::loads(&base_json).unwrap();
    let bad_ev_values = [
        Json::Array(Vec::new()),
        Json::Null,
        Json::Array(vec![Json::object([("file", Json::str("a.txt"))])]),
        Json::Array(vec![Json::object([
            ("file", Json::int(1)),
            ("sha256", Json::str(&"a".repeat(64))),
            ("locator", Json::str("row:1")),
        ])]),
        Json::Array(vec![Json::object([
            ("file", Json::str("../escape.txt")),
            ("sha256", Json::str(&"a".repeat(64))),
            ("locator", Json::str("row:1")),
        ])]),
        Json::Array(vec![Json::object([
            ("file", Json::str("ok.txt")),
            ("sha256", Json::str("not-a-sha")),
            ("locator", Json::str("row:1")),
        ])]),
        Json::Array(vec![Json::object([
            ("file", Json::str("ok.txt")),
            ("sha256", Json::str(&"a".repeat(64))),
            ("locator", Json::str("   ")),
        ])]),
    ];
    for bad_ev in bad_ev_values {
        let mut cloned = doc.clone();
        let finding = cloned
            .get_mut("findings")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .first_mut()
            .unwrap();
        *finding.get_mut("evidence").unwrap() = bad_ev;
        assert!(viewer::verify_schema(&cloned).is_err());
    }
    // Silence unused mut warning if doc wasn't mutated.
    let _ = &mut doc;
}

#[cfg(unix)]
#[test]
fn view_refuses_an_evidence_link_whose_symlink_escapes_the_pack_directory() {
    use std::os::unix::fs::symlink;
    let tmp = TempDir::new();
    let dir = copy_pack(&tmp, "bas-ready", "sym-pack");
    fs::write(dir.join("escape.txt"), b"stray").unwrap();
    let pack = engine::review_pack("bas", &dir, None, tolerance(), &[]).unwrap();
    let output = tmp.join("rendered-sym");
    report::write_review_pack(&pack, &output).unwrap();
    let outside = tmp.join("secret.txt");
    fs::write(&outside, b"outside").unwrap();
    symlink(&outside, output.join("escape.txt")).unwrap();
    assert!(
        viewer::verify_pack(&output)
            .unwrap_err()
            .contains("escapes the pack directory")
    );
}
