// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Integration tests for named reconciliation residual taxonomy and tamper-evident close binders.

#[path = "support/temp.rs"]
mod temp;

use monthly_close_control_plane::{
    decimal::Decimal,
    engine::{self, ReviewConfig},
    publication::{
        self,
        binder::{self, AuditEventInput, BinderAttachment, BinderRequest, MANIFEST_PATH},
    },
    pyjson::{self, Json},
    reconciliation::{
        self, COLUMNS, ReconcileConfig,
        residuals::{self, ResidualClass, ResidualPolicy},
    },
};
use std::{fs, path::Path};
use temp::TempDir;

fn make_reconciliation_pack(rows: &str, closing: &str) -> Json {
    let csv = format!("{}\n{rows}", COLUMNS.join(","));
    let cfg = ReconcileConfig {
        tenant: "Fabricated Varrock".into(),
        account_id: "CLEAR-100".into(),
        currency: "AUD".into(),
        period_start: "2026-08-01".into(),
        period_end: "2026-08-31".into(),
        opening_balance: "0.00".into(),
        closing_balance: closing.into(),
        transactions: "tx.csv".into(),
        opening_items: None,
        decisions: None,
        decisions_escaped: false,
    };
    reconciliation::reconcile_with(&cfg, &mut |path, _| {
        Ok(monthly_close_control_plane::loader::SourceSnapshot::new(
            path,
            csv.as_bytes().to_vec(),
        ))
    })
    .unwrap()
}

#[test]
#[allow(clippy::too_many_lines)]
fn residuals_classify_all_six_named_classes_and_avoid_keyword_traps() {
    let rows = "\
Fabricated Varrock,CLEAR-100,AUD,FX1,2026-08-10,INV-FX-1,USD invoice settlement,1000.00,0.00
Fabricated Varrock,CLEAR-100,AUD,FX2,2026-08-12,INV-FX-1,USD wire payout,0.00,985.00
Fabricated Varrock,CLEAR-100,AUD,FEE1,2026-08-11,PAY-10,Card settlement batch,200.00,0.00
Fabricated Varrock,CLEAR-100,AUD,FEE2,2026-08-12,PAY-10,Merchant fee deduction,0.00,196.50
Fabricated Varrock,CLEAR-100,AUD,PART1,2026-08-05,INV-PART-1,Customer invoice,500.00,0.00
Fabricated Varrock,CLEAR-100,AUD,PART2,2026-08-08,INV-PART-1,Instalment receipt,0.00,300.00
Fabricated Varrock,CLEAR-100,AUD,DUP1,2026-08-10,BILL-77,Duplicate supplier debit,75.00,0.00
Fabricated Varrock,CLEAR-100,AUD,DUP2,2026-08-12,BILL-77,Duplicate supplier debit,75.00,0.00
Fabricated Varrock,CLEAR-100,AUD,DUP3,2026-08-29,BILL-77,Far apart same amount,75.00,0.00
Fabricated Varrock,CLEAR-100,AUD,DUPD1,2026-08-14,,Blank ref same desc,40.00,0.00
Fabricated Varrock,CLEAR-100,AUD,DUPD2,2026-08-15,,Blank ref same desc,40.00,0.00
Fabricated Varrock,CLEAR-100,AUD,SGLFEE,2026-08-10,BANK,Bank charge August,12.00,0.00
Fabricated Varrock,CLEAR-100,AUD,SGLFX,2026-08-10,ADJ,Foreign exchange adjustment,80.00,0.00
Fabricated Varrock,CLEAR-100,AUD,SGLFX2,2026-08-10,FX-ADJ,FX small diff,10.00,0.00
Fabricated Varrock,CLEAR-100,AUD,TIMING,2026-08-29,DEP-99,Late month deposit,120.00,0.00
Fabricated Varrock,CLEAR-100,AUD,UNEXP,2026-08-02,OLD-1,Coffee prefix supplies,250.00,0.00
Fabricated Varrock,CLEAR-100,AUD,BIGUSD,2026-08-02,WIRE-9,USD customer deposit,5000.00,0.00
Fabricated Varrock,CLEAR-100,AUD,ZERO1,2026-08-04,EVEN-1,Unaccepted pair leg 1,60.00,0.00
Fabricated Varrock,CLEAR-100,AUD,ZERO2,2026-08-04,EVEN-1,Unaccepted pair leg 2,0.00,60.00
Fabricated Varrock,CLEAR-100,AUD,BLANK,2026-08-03,,,15.00,0.00
";
    // Net total = 15 + 3.50 + 200 + 150 + 75 + 80 + 12 + 80 + 10 + 120 + 250 + 5000 + 0 + 15 = 6010.50
    let pack = make_reconciliation_pack(rows, "6010.50");
    let report = residuals::classify_pack(&pack, &ResidualPolicy::default()).unwrap();
    assert_eq!(report.total_outstanding_items, 20);
    assert_eq!(reconciliation::fixed(&report.total_net_amount), "6010.50");

    let by_ids = |ids: &[&str]| {
        let want: Vec<String> = ids.iter().map(|s| (*s).to_owned()).collect();
        report
            .items
            .iter()
            .find(|item| item.transaction_ids == want)
            .unwrap_or_else(|| panic!("missing item {want:?}"))
    };
    assert_eq!(by_ids(&["FX1", "FX2"]).class, ResidualClass::Fx);
    assert_eq!(by_ids(&["FEE1", "FEE2"]).class, ResidualClass::BankFee);
    assert_eq!(by_ids(&["PART1", "PART2"]).class, ResidualClass::Partial);
    assert_eq!(by_ids(&["DUP1", "DUP2"]).class, ResidualClass::Duplicate);
    assert_eq!(by_ids(&["DUPD1", "DUPD2"]).class, ResidualClass::Duplicate);
    assert_eq!(by_ids(&["DUP3"]).class, ResidualClass::Timing);
    assert_eq!(by_ids(&["SGLFEE"]).class, ResidualClass::BankFee);
    assert_eq!(by_ids(&["SGLFX"]).class, ResidualClass::Fx);
    assert_eq!(by_ids(&["SGLFX2"]).class, ResidualClass::Fx);
    assert_eq!(by_ids(&["TIMING"]).class, ResidualClass::Timing);
    // "Coffee" must NOT trigger "fee"; "prefix" must NOT trigger "fx";
    // $5,000 standalone "USD customer deposit" must NOT trigger "fx".
    assert_eq!(by_ids(&["UNEXP"]).class, ResidualClass::Unexplained);
    assert_eq!(by_ids(&["BIGUSD"]).class, ResidualClass::Unexplained);
    assert_eq!(by_ids(&["BLANK"]).class, ResidualClass::Unexplained);
    assert_eq!(by_ids(&["ZERO1"]).class, ResidualClass::Unexplained);
    assert_eq!(by_ids(&["ZERO2"]).class, ResidualClass::Unexplained);

    let json = report.to_json();
    assert!(json.at("schema").is_str(residuals::SCHEMA));
    assert_eq!(json.at("summary").as_array().unwrap().len(), 6);
    assert!(pyjson::pretty_utf8(&json).unwrap().contains("bank_fee"));
}

#[test]
#[allow(clippy::too_many_lines)]
fn residuals_validate_policy_and_pack_errors() {
    let bad_fx = ResidualPolicy {
        fx_ratio_limit: Decimal::parse("1.5").unwrap(),
        ..Default::default()
    };
    assert!(bad_fx.validate().is_err());

    let bad_fee = ResidualPolicy {
        bank_fee_limit: Decimal::parse("-1").unwrap(),
        ..Default::default()
    };
    assert!(bad_fee.validate().is_err());

    let bad_win = ResidualPolicy {
        timing_window_days: -1,
        ..Default::default()
    };
    assert!(bad_win.validate().is_err());

    let policy = ResidualPolicy::default();
    assert!(residuals::classify_pack(&Json::object([]), &policy).is_err());

    let missing_outstanding = Json::object([
        ("schema", Json::str("clearing-reconciliation-v1")),
        ("period_end", Json::str("2026-08-31")),
    ]);
    assert!(residuals::classify_pack(&missing_outstanding, &policy).is_err());

    let future_item = Json::object([
        ("schema", Json::str("clearing-reconciliation-v1")),
        ("period_end", Json::str("2026-08-31")),
        (
            "outstanding",
            Json::Array(vec![Json::object([
                ("TransactionID", Json::str("T1")),
                ("Date", Json::str("2026-09-01")),
                ("Reference", Json::str("R")),
                ("Description", Json::str("D")),
                ("Debit", Json::str("10.00")),
                ("Credit", Json::str("0.00")),
            ])]),
        ),
    ]);
    assert!(
        residuals::classify_pack(&future_item, &policy)
            .unwrap_err()
            .contains("after period_end")
    );

    let bad_amount = Json::object([
        ("schema", Json::str("clearing-reconciliation-v1")),
        ("period_end", Json::str("2026-08-31")),
        (
            "outstanding",
            Json::Array(vec![Json::object([
                ("TransactionID", Json::str("T1")),
                ("Date", Json::str("2026-08-15")),
                ("Reference", Json::str("R")),
                ("Description", Json::str("D")),
                ("Debit", Json::str("-5.00")),
                ("Credit", Json::str("0.00")),
            ])]),
        ),
    ]);
    assert!(
        residuals::classify_pack(&bad_amount, &policy)
            .unwrap_err()
            .contains("invalid Debit")
    );

    let bad_str = Json::object([
        ("schema", Json::str("clearing-reconciliation-v1")),
        ("period_end", Json::int(20260831)),
    ]);
    assert!(residuals::classify_pack(&bad_str, &policy).is_err());
}

fn write_clean_pack(root: &Path) -> std::path::PathBuf {
    let inputs = root.join("inputs");
    fs::create_dir_all(&inputs).unwrap();
    let header = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n";
    let prior = format!(
        "{header}\
2026-07-31,Fabricated Varrock,Assets,100,Operating Bank,1000,1000.00,0.00,10000.00,0.00\n\
2026-07-31,Fabricated Varrock,Equity,900,Retained Earnings,3000,0.00,1000.00,0.00,10000.00\n"
    );
    let current = format!(
        "{header}\
2026-08-31,Fabricated Varrock,Assets,100,Operating Bank,1000,100.00,0.00,10100.00,0.00\n\
2026-08-31,Fabricated Varrock,Equity,900,Retained Earnings,3000,0.00,100.00,0.00,10100.00\n"
    );
    let prior_path = inputs.join("prior.csv");
    let current_path = inputs.join("current.csv");
    fs::write(&prior_path, prior).unwrap();
    fs::write(&current_path, current).unwrap();
    let pack = engine::review_close(&ReviewConfig::new(current_path, prior_path)).unwrap();
    let pack_dir = root.join("pack");
    publication::write_review_pack(&pack, &pack_dir).unwrap();
    pack_dir
}

#[test]
#[allow(clippy::too_many_lines)]
fn binder_export_verify_audit_chain_and_tamper_detection() {
    let dir = TempDir::new();
    let pack_dir = write_clean_pack(&dir.0);
    let chain = binder::build_audit_chain(&[
        AuditEventInput {
            timestamp: "2026-08-31T09:00:00Z".into(),
            actor: "RD".into(),
            action: "review_run".into(),
            target: "pack".into(),
            detail: "Generated monthly close review pack".into(),
        },
        AuditEventInput {
            timestamp: "2026-08-31T09:15:00Z".into(),
            actor: "RD".into(),
            action: "residual_check".into(),
            target: "clearing".into(),
            detail: "Classified clearing residuals".into(),
        },
    ])
    .unwrap();
    let chain_summary = binder::verify_audit_chain(&chain).unwrap();
    assert_eq!(chain_summary.count, 2);

    let zip_path = dir.join("binders/2026-08-close.zip");
    let req = BinderRequest {
        pack_dir: pack_dir.clone(),
        output_zip: zip_path.clone(),
        attachments: vec![BinderAttachment {
            archive_path: "evidence/residuals.json".into(),
            bytes: br#"{"schema":"clearing-residuals-v1"}"#.to_vec(),
        }],
        audit_chain: Some(chain.clone()),
    };
    let exported = binder::export_binder(&req).unwrap();
    assert_eq!(exported.pack_status, "PASS");
    assert_eq!(exported.entry_count, 6);
    assert_eq!(
        exported.audit_chain_head,
        Some(chain_summary.head_sha256.clone())
    );

    let verified = binder::verify_binder(&zip_path).unwrap();
    assert_eq!(verified, exported);

    // Export without audit chain works cleanly too.
    let zip_no_chain = dir.join("binders/2026-08-plain.zip");
    let plain = binder::export_binder(&BinderRequest {
        pack_dir: pack_dir.clone(),
        output_zip: zip_no_chain.clone(),
        attachments: Vec::new(),
        audit_chain: None,
    })
    .unwrap();
    assert_eq!(plain.audit_chain_head, None);
    assert_eq!(binder::verify_binder(&zip_no_chain).unwrap(), plain);

    // Determinism: exporting the same inputs to a second path produces identical ZIP bytes.
    let zip_copy = dir.join("binders/2026-08-copy.zip");
    binder::export_binder(&BinderRequest {
        output_zip: zip_copy.clone(),
        ..req.clone()
    })
    .unwrap();
    assert_eq!(fs::read(&zip_path).unwrap(), fs::read(&zip_copy).unwrap());

    // Tampering a single byte fails verification.
    let mut tampered = fs::read(&zip_path).unwrap();
    let idx = tampered
        .windows(10)
        .position(|w| w == b"residuals.")
        .unwrap()
        + 15;
    tampered[idx] ^= 0x01;
    assert!(binder::verify_binder_bytes(&tampered).is_err());

    // Missing zip file fails cleanly.
    assert!(binder::verify_binder(&dir.join("missing.zip")).is_err());

    // Path validation & reserved/duplicate collisions.
    for bad in [
        "",
        "/abs",
        "dir/",
        "a\\b",
        "../escape",
        "./dot",
        "a//b",
        "bad\0name",
    ] {
        assert!(binder::validate_archive_path(bad).is_err(), "{bad}");
    }
    for bad_attach in [MANIFEST_PATH, "close-review-pack.json"] {
        let err = binder::export_binder(&BinderRequest {
            pack_dir: pack_dir.clone(),
            output_zip: dir.join("binders/conflict.zip"),
            attachments: vec![BinderAttachment {
                archive_path: bad_attach.into(),
                bytes: b"x".to_vec(),
            }],
            audit_chain: None,
        })
        .unwrap_err();
        assert!(err.contains("conflicts"));
    }

    // Audit chain error cases.
    assert!(binder::build_audit_chain(&[]).is_err());
    assert!(
        binder::build_audit_chain(&[AuditEventInput {
            timestamp: " ".into(),
            actor: "RD".into(),
            action: "a".into(),
            target: "t".into(),
            detail: "d".into(),
        }])
        .is_err()
    );
    assert!(binder::verify_audit_chain(&[0xff]).is_err());
    assert!(binder::verify_audit_chain(b"").is_err());
    assert!(binder::verify_audit_chain(b"{\n").is_err());
    assert!(binder::verify_audit_chain(b"{}\n").is_err());
    let mut bad_seq = String::from_utf8(chain.clone()).unwrap();
    bad_seq = bad_seq.replacen("\"seq\":1", "\"seq\":9", 1);
    assert!(
        binder::verify_audit_chain(bad_seq.as_bytes())
            .unwrap_err()
            .contains("sequence")
    );
    let bad_prev =
        String::from_utf8(chain.clone())
            .unwrap()
            .replacen(binder::GENESIS_HASH, "1111", 1);
    assert!(
        binder::verify_audit_chain(bad_prev.as_bytes())
            .unwrap_err()
            .contains("prev_sha256")
    );
    let bad_hash = String::from_utf8(chain).unwrap().replacen(
        "Generated monthly close review pack",
        "Altered monthly close review pack",
        1,
    );
    assert!(
        binder::verify_audit_chain(bad_hash.as_bytes())
            .unwrap_err()
            .contains("tampered")
    );
}
