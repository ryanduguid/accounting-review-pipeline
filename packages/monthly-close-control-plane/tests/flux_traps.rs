// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Flux anomaly tests (sudden-zero recurring costs and normal-balance sign flips)
//! plus explicit false-positive trap fixtures across engine controls and flux rules.

#[path = "support/temp.rs"]
mod temp;

use monthly_close_control_plane::{
    decimal::Decimal,
    drivers::flux::{self, FluxKind, FluxPolicy, NormalSign},
    engine::{self, ReviewConfig},
    isodate::Date,
    models::{Status, TrialBalanceRow},
    pyjson,
};
use std::fs;
use temp::TempDir;

const TENANT: &str = "Fabricated Lumbridge";

#[allow(clippy::too_many_arguments)]
fn row(
    date: Date,
    section: &str,
    id: &str,
    code: &str,
    name: &str,
    debit: &str,
    credit: &str,
    ytd_debit: &str,
    ytd_credit: &str,
) -> TrialBalanceRow {
    TrialBalanceRow {
        report_date: date,
        tenant: TENANT.into(),
        section: section.into(),
        account_id: id.into(),
        account_name: name.into(),
        account_code: code.into(),
        debit: Decimal::parse(debit).unwrap(),
        credit: Decimal::parse(credit).unwrap(),
        ytd_debit: Decimal::parse(ytd_debit).unwrap(),
        ytd_credit: Decimal::parse(ytd_credit).unwrap(),
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn flux_detects_planted_sudden_zero_recurring_and_sign_flips() {
    let july = Date::new(2026, 7, 31).unwrap();
    let aug = Date::new(2026, 8, 31).unwrap();

    let prior = vec![
        row(
            july,
            "Expenses",
            "601",
            "6010",
            "Cloud Hosting",
            "450.00",
            "0.00",
            "1350.00",
            "0.00",
        ),
        row(
            july,
            "Expenses",
            "602",
            "6020",
            "Insurance Expense",
            "300.00",
            "0.00",
            "900.00",
            "0.00",
        ),
        row(
            july,
            "Assets",
            "140",
            "1400",
            "Prepaid Insurance",
            "0.00",
            "0.00",
            "2400.00",
            "0.00",
        ),
        row(
            july,
            "Assets",
            "110",
            "1100",
            "Trade Debtors",
            "200.00",
            "0.00",
            "1500.00",
            "0.00",
        ),
        row(
            july,
            "Liabilities",
            "200",
            "2000",
            "Trade Creditors",
            "0.00",
            "500.00",
            "0.00",
            "2000.00",
        ),
    ];
    let current = vec![
        row(
            aug,
            "Expenses",
            "601",
            "6010",
            "Cloud Hosting",
            "0.00",
            "0.00",
            "1350.00",
            "0.00",
        ),
        row(
            aug,
            "Expenses",
            "602",
            "6020",
            "Insurance Expense",
            "0.00",
            "0.00",
            "900.00",
            "0.00",
        ),
        row(
            aug,
            "Assets",
            "140",
            "1400",
            "Prepaid Insurance",
            "0.00",
            "0.00",
            "2400.00",
            "0.00",
        ),
        row(
            aug,
            "Assets",
            "110",
            "1100",
            "Trade Debtors",
            "0.00",
            "1750.00",
            "0.00",
            "250.00",
        ),
        row(
            aug,
            "Liabilities",
            "200",
            "2000",
            "Trade Creditors",
            "2180.00",
            "0.00",
            "180.00",
            "0.00",
        ),
        row(
            aug,
            "Expenses",
            "699",
            "6990",
            "New Expense",
            "0.00",
            "0.00",
            "0.00",
            "0.00",
        ),
    ];

    let mut policy = FluxPolicy::default();
    policy
        .prepaid_links
        .insert((TENANT.into(), "602".into()), "140".into());

    let report = flux::evaluate_flux(&current, &prior, &policy).unwrap();
    assert_eq!(report.status, Status::Review);
    assert_eq!(report.findings.len(), 4);

    let find = |kind: FluxKind, id: &str| {
        report
            .findings
            .iter()
            .find(|f| f.kind == kind && f.account_id == id)
            .unwrap_or_else(|| panic!("missing {kind:?} for {id}"))
    };
    assert!(
        find(FluxKind::SuddenZeroRecurringCost, "601")
            .reason
            .contains("450.00")
    );
    assert!(
        find(FluxKind::SuddenZeroRecurringCost, "602")
            .reason
            .contains("linked prepaid balance did not unwind")
    );
    assert!(
        find(FluxKind::NormalBalanceSignFlip, "110")
            .reason
            .contains("expected debit")
    );
    assert!(
        find(FluxKind::NormalBalanceSignFlip, "200")
            .reason
            .contains("expected credit")
    );

    let json = report.to_json();
    assert!(json.at("schema").is_str(flux::SCHEMA));
    assert!(json.at("status").is_str("REVIEW"));
    assert!(
        pyjson::pretty_utf8(&json)
            .unwrap()
            .contains("sudden_zero_recurring_cost")
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn flux_false_positive_traps_produce_zero_findings() {
    let july = Date::new(2026, 7, 31).unwrap();
    let aug = Date::new(2026, 8, 31).unwrap();

    let prior = vec![
        row(
            july,
            "Expenses",
            "610",
            "6100",
            "Office Fitout One-off",
            "800.00",
            "0.00",
            "800.00",
            "0.00",
        ),
        row(
            july,
            "Expenses",
            "611",
            "6110",
            "Council Rates",
            "600.00",
            "0.00",
            "1800.00",
            "0.00",
        ),
        row(
            july,
            "Expenses",
            "612",
            "6120",
            "Annual Audit Fee",
            "1200.00",
            "0.00",
            "2400.00",
            "0.00",
        ),
        row(
            july,
            "Expenses",
            "613",
            "6130",
            "Prepaid Sub Expense",
            "300.00",
            "0.00",
            "900.00",
            "0.00",
        ),
        row(
            july,
            "Assets",
            "141",
            "1410",
            "Prepaid Subscriptions",
            "0.00",
            "0.00",
            "2400.00",
            "0.00",
        ),
        row(
            july,
            "Expenses",
            "614",
            "6140",
            "Old Software Account",
            "250.00",
            "0.00",
            "750.00",
            "0.00",
        ),
        row(
            july,
            "Expenses",
            "615",
            "6150",
            "New Software Account",
            "0.00",
            "0.00",
            "0.00",
            "0.00",
        ),
        row(
            july,
            "Expenses",
            "616",
            "6160",
            "Small Postage",
            "40.00",
            "0.00",
            "120.00",
            "0.00",
        ),
        row(
            july,
            "Assets",
            "155",
            "1550",
            "Accumulated Depreciation",
            "0.00",
            "200.00",
            "0.00",
            "4000.00",
        ),
        row(
            july,
            "Liabilities",
            "250",
            "2500",
            "GST Clearing",
            "0.00",
            "100.00",
            "0.00",
            "500.00",
        ),
        row(
            july,
            "Assets",
            "190",
            "1900",
            "Rounding Clearing",
            "0.00",
            "0.50",
            "0.00",
            "0.50",
        ),
        row(
            july,
            "Memo",
            "990",
            "9900",
            "Statistical Account",
            "10.00",
            "0.00",
            "10.00",
            "0.00",
        ),
    ];
    let current = vec![
        row(
            aug,
            "Expenses",
            "610",
            "6100",
            "Office Fitout One-off",
            "0.00",
            "0.00",
            "800.00",
            "0.00",
        ),
        row(
            aug,
            "Expenses",
            "611",
            "6110",
            "Council Rates",
            "0.00",
            "0.00",
            "1800.00",
            "0.00",
        ),
        row(
            aug,
            "Expenses",
            "612",
            "6120",
            "Annual Audit Fee",
            "0.00",
            "0.00",
            "2400.00",
            "0.00",
        ),
        row(
            aug,
            "Expenses",
            "613",
            "6130",
            "Prepaid Sub Expense",
            "0.00",
            "0.00",
            "900.00",
            "0.00",
        ),
        row(
            aug,
            "Assets",
            "141",
            "1410",
            "Prepaid Subscriptions",
            "0.00",
            "300.00",
            "2100.00",
            "0.00",
        ),
        row(
            aug,
            "Expenses",
            "614",
            "6140",
            "Old Software Account",
            "0.00",
            "0.00",
            "750.00",
            "0.00",
        ),
        row(
            aug,
            "Expenses",
            "615",
            "6150",
            "New Software Account",
            "250.00",
            "0.00",
            "250.00",
            "0.00",
        ),
        row(
            aug,
            "Expenses",
            "616",
            "6160",
            "Small Postage",
            "0.00",
            "0.00",
            "120.00",
            "0.00",
        ),
        row(
            aug,
            "Assets",
            "155",
            "1550",
            "Accumulated Depreciation",
            "0.00",
            "200.00",
            "0.00",
            "4200.00",
        ),
        row(
            aug,
            "Liabilities",
            "250",
            "2500",
            "GST Clearing",
            "700.00",
            "0.00",
            "200.00",
            "0.00",
        ),
        row(
            aug,
            "Assets",
            "190",
            "1900",
            "Rounding Clearing",
            "0.00",
            "0.00",
            "0.00",
            "0.50",
        ),
        row(
            aug,
            "Memo",
            "990",
            "9900",
            "Statistical Account",
            "0.00",
            "20.00",
            "0.00",
            "10.00",
        ),
    ];

    let mut policy = FluxPolicy::default();
    policy
        .seasonal_or_one_off
        .insert((TENANT.into(), "611".into()));
    policy
        .prior_active_periods
        .insert((TENANT.into(), "612".into()), 1);
    policy
        .prepaid_links
        .insert((TENANT.into(), "613".into()), "141".into());
    policy
        .reclassified_to
        .insert((TENANT.into(), "614".into()), "615".into());
    policy
        .sign_overrides
        .insert((TENANT.into(), "155".into()), NormalSign::Credit);
    policy
        .sign_overrides
        .insert((TENANT.into(), "250".into()), NormalSign::Either);

    let report = flux::evaluate_flux(&current, &prior, &policy).unwrap();
    assert_eq!(report.status, Status::Pass);
    assert!(
        report.findings.is_empty(),
        "false positives: {:?}",
        report.findings
    );

    let p = FluxPolicy {
        min_recurring_expense: Decimal::parse("-1").unwrap(),
        ..Default::default()
    };
    assert!(flux::evaluate_flux(&current, &prior, &p).is_err());
    let p = FluxPolicy {
        min_recurring_periods: 0,
        ..Default::default()
    };
    assert!(flux::evaluate_flux(&current, &prior, &p).is_err());
    let p = FluxPolicy {
        min_sign_flip_amount: Decimal::parse("-0.01").unwrap(),
        ..Default::default()
    };
    assert!(flux::evaluate_flux(&current, &prior, &p).is_err());
}

#[test]
#[allow(clippy::too_many_lines)]
fn engine_false_positive_traps_raise_zero_exceptions() {
    let dir = TempDir::new();
    let write = |name: &str, body: &str| {
        let p = dir.join(name);
        fs::write(&p, body).unwrap();
        p
    };
    let header = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n";
    let prior = format!(
        "{header}\
2026-07-31,{TENANT},Assets,100,Operating Bank,1000,1000.00,0.00,50000.00,0.00\n\
2026-07-31,{TENANT},Assets,150,Office Equipment,1500,0.00,0.00,10000.00,0.00\n\
2026-07-31,{TENANT},Liabilities,210,GST Payable,2100,0.00,500.00,0.00,1000.00\n\
2026-07-31,{TENANT},Liabilities,220,Customer Deposits,2200,0.00,0.00,0.00,1500.00\n\
2026-07-31,{TENANT},Revenue,300,Taxable Sales,4000,0.00,5000.00,0.00,60000.00\n\
2026-07-31,{TENANT},Revenue,310,GST-Free Export Sales,4100,0.00,2000.00,0.00,25000.00\n\
2026-07-31,{TENANT},Revenue,320,Input-Taxed Interest,4200,0.00,500.00,0.00,6000.00\n\
2026-07-31,{TENANT},Expenses,400,Non-Creditable Entertainment,5000,300.00,0.00,3000.00,0.00\n\
2026-07-31,{TENANT},Expenses,410,General Expenses,5100,6700.00,0.00,75000.00,0.00\n\
2026-07-31,{TENANT},Equity,900,Retained Earnings,3000,0.00,0.00,0.00,44500.00\n"
    );
    let current = format!(
        "{header}\
2026-08-31,{TENANT},Assets,100,Operating Bank,1000,450.00,0.00,50450.00,0.00\n\
2026-08-31,{TENANT},Assets,150,Office Equipment,1500,800.00,0.00,10800.00,0.00\n\
2026-08-31,{TENANT},Liabilities,210,GST Payable,2100,0.00,500.00,0.00,1500.00\n\
2026-08-31,{TENANT},Liabilities,220,Customer Deposits,2200,0.00,100.00,0.00,1600.00\n\
2026-08-31,{TENANT},Revenue,300,Taxable Sales,4000,0.00,5000.00,0.00,65000.00\n\
2026-08-31,{TENANT},Revenue,310,GST-Free Export Sales,4100,0.00,2100.00,0.00,27100.00\n\
2026-08-31,{TENANT},Revenue,320,Input-Taxed Interest,4200,0.00,510.00,0.00,6510.00\n\
2026-08-31,{TENANT},Expenses,400,Non-Creditable Entertainment,5000,310.00,0.00,3310.00,0.00\n\
2026-08-31,{TENANT},Expenses,410,General Expenses,5100,6650.00,0.00,81650.00,0.00\n\
2026-08-31,{TENANT},Equity,900,Retained Earnings,3000,0.00,0.00,0.00,44500.00\n"
    );
    let mut cfg = ReviewConfig::new(write("current.csv", &current), write("prior.csv", &prior));
    cfg.balance_policy_path = Some(write(
        "balance_policy.csv",
        "AccountID,ExpectedBalance,ExpectMovement\n220,credit,yes\n",
    ));
    cfg.subledger_path = Some(write(
        "subledger.csv",
        &format!(
            "Tenant,AccountID,SubledgerBalance\n\
{TENANT},100,50450.00\n\
{TENANT},150,10800.00\n\
{TENANT},210,-1500.00\n\
{TENANT},220,-1600.00\n\
{TENANT},900,-44500.00\n"
        ),
    ));
    cfg.gst_mapping_path = Some(write(
        "gst_mapping.csv",
        "AccountID,GSTClass\n\
100,bas_excluded\n\
150,bas_excluded\n\
210,gst\n\
220,bas_excluded\n\
300,taxable_sales\n\
310,gst_free\n\
320,input_taxed\n\
400,non_creditable\n\
410,bas_excluded\n\
900,bas_excluded\n",
    ));
    cfg.gst_settlements_path = Some(write("gst_settlements.csv", "Reference,Amount\n"));
    cfg.contacts_path = Some(write(
        "contacts.csv",
        "ContactID,Name,ABN\nC1,Fabricated Lumbridge Supplies Pty Ltd,80 000 000 001\nC2,Fabricated Overseas Vendor,\n",
    ));
    let pack = engine::review_close(&cfg).unwrap();
    assert_eq!(
        pack.status,
        Status::Pass,
        "unexpected exceptions: {:?}",
        pack.exceptions
    );
    assert!(pack.exceptions.is_empty());
}
