// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! A fixed-asset register tied to the trial balance: each cost account's
//! closing balance against the cost of the assets in service, each
//! accumulated depreciation account's against their accumulated depreciation.
use crate::{
    Result,
    decimal::{self, Decimal},
    isodate::Date,
    loader::{self, CsvRow, SourceSnapshot},
    models::TrialBalanceRow,
    pyjson::Json,
    pytext, reconciliation,
};
use std::{collections::BTreeMap, path::Path};

pub const COLUMNS: [&str; 10] = [
    "Tenant",
    "AssetID",
    "Description",
    "CostAccountID",
    "DepreciationAccountID",
    "Cost",
    "AccumulatedDepreciation",
    "AcquisitionDate",
    "DisposalDate",
    "Currency",
];
pub const BOUNDARY: &str = "Review evidence only. A tie shows that the register's totals agree with the trial balance for the listed accounts; it does not show that the register is complete, that any cost is right or that depreciation was calculated correctly.";
const COST: &str = "cost";
const DEPRECIATION: &str = "accumulated_depreciation";

struct Asset {
    id: String,
    description: String,
    cost_account: String,
    depreciation_account: String,
    cost: Decimal,
    accumulated: Decimal,
    disposed: bool,
}

/// One register row as an asset, its tenant and currency already checked.
fn asset(row: &CsvRow, path: &str, report_date: Date) -> Result<Asset> {
    let text = |field| row.text(field, field == "DisposalDate");
    let refuse = |what: &str| Err(format!("{path}: row {} {what}", row.number));
    let money = |field| reconciliation::money(&text(field)?, field, &row.path, row.number);
    let (cost, accumulated) = (money("Cost")?, money("AccumulatedDepreciation")?);
    if cost <= Decimal::zero() || accumulated < Decimal::zero() {
        return refuse("needs a positive Cost and an AccumulatedDepreciation of zero or more.");
    }
    let (cost_account, depreciation_account) =
        (text("CostAccountID")?, text("DepreciationAccountID")?);
    if cost_account == depreciation_account {
        return refuse("needs different cost and accumulated depreciation accounts.");
    }
    let acquired = row.date("AcquisitionDate")?;
    if acquired > report_date {
        return refuse(
            "was acquired after the trial balance's ReportDate; supply the register at that date.",
        );
    }
    let disposed = if text("DisposalDate")?.is_empty() {
        false
    } else {
        let date = row.date("DisposalDate")?;
        if date < acquired {
            return refuse("has a DisposalDate before its AcquisitionDate.");
        }
        date <= report_date
    };
    Ok(Asset {
        id: text("AssetID")?,
        description: text("Description")?,
        cost_account,
        depreciation_account,
        cost,
        accumulated,
        disposed,
    })
}

fn load(
    source: &SourceSnapshot,
    currency: &str,
    report_date: Date,
) -> Result<(String, Vec<Asset>)> {
    let path = crate::pypath::text(&source.path);
    let mut tenant = None;
    let mut assets: Vec<Asset> = Vec::new();
    for row in loader::csv_rows(source, &COLUMNS, "Register")? {
        let row = row?;
        let name = row.text("Tenant", false)?;
        if *tenant.get_or_insert_with(|| name.clone()) != name {
            return Err(
                "The register must belong to one tenant; review each tenant separately.".into(),
            );
        }
        if row.text("Currency", false)? != currency {
            return Err(format!(
                "{path}: row {} is not in the declared --currency {}.",
                row.number,
                pytext::repr(currency)
            ));
        }
        let asset = asset(&row, &path, report_date)?;
        if assets.iter().any(|seen| seen.id == asset.id) {
            return Err(format!(
                "{path}: row {} repeats AssetID {}.",
                row.number,
                pytext::repr(&asset.id)
            ));
        }
        assets.push(asset);
    }
    match tenant {
        Some(tenant) => Ok((tenant, assets)),
        None => Err(format!("{path}: no register rows were supplied.")),
    }
}

/// Tie the register in `register` to the trial balance in `trial_balance`
/// at its `ReportDate`.
pub fn fixed_assets(register: &Path, trial_balance: &Path, currency: &str) -> Result<Json> {
    if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err("Declare a three-letter currency, such as AUD.".into());
    }
    let register = SourceSnapshot::capture(register, "Register")?;
    let trial_balance = SourceSnapshot::capture(trial_balance, "Trial-balance file")?;
    decimal::with_precision(40, || review(&register, &trial_balance, currency))
}

/// Each account's role, the register's total for the assets in service in it
/// and their count.
type Totals<'a> = BTreeMap<&'a str, (&'static str, Decimal, i64)>;

fn totals(assets: &[Asset]) -> Result<Totals<'_>> {
    let mut accounts = Totals::new();
    for asset in assets {
        for (account, role, amount) in [
            (asset.cost_account.as_str(), COST, &asset.cost),
            (
                asset.depreciation_account.as_str(),
                DEPRECIATION,
                &asset.accumulated,
            ),
        ] {
            let entry = accounts
                .entry(account)
                .or_insert((role, Decimal::zero(), 0));
            if entry.0 != role {
                return Err(format!(
                    "Account {} is used as both a cost and an accumulated depreciation account; give each its own account.",
                    pytext::repr(account)
                ));
            }
            if !asset.disposed {
                entry.1 = entry.1.add(amount).value;
                entry.2 += 1;
            }
        }
    }
    Ok(accounts)
}

fn over_depreciated(asset: &Asset) -> Json {
    Json::object([
        ("code", Json::str("DEPRECIATION_EXCEEDS_COST")),
        ("asset_id", Json::str(&asset.id)),
        ("description", Json::str(&asset.description)),
        ("cost", Json::str(&reconciliation::fixed(&asset.cost))),
        (
            "accumulated_depreciation",
            Json::str(&reconciliation::fixed(&asset.accumulated)),
        ),
    ])
}

/// One account's tie, and its finding when the ledger does not agree.
fn tie(
    account: &str,
    (role, total, count): &(&str, Decimal, i64),
    row: Option<&TrialBalanceRow>,
) -> (Json, Option<Json>) {
    // Cost is a debit balance and accumulated depreciation a credit one.
    let balance = row.map(|row| {
        let net = row.ytd_net();
        if *role == COST {
            net
        } else {
            Decimal::zero().sub(&net).value
        }
    });
    let difference = balance.as_ref().map(|balance| balance.sub(total).value);
    let amount = |value: Option<&Decimal>| {
        value.map_or(Json::Null, |value| Json::str(&value.to_sci_string()))
    };
    let tie = Json::object([
        ("account_id", Json::str(account)),
        (
            "account_name",
            row.map_or(Json::Null, |row| Json::str(&row.account_name)),
        ),
        ("role", Json::str(role)),
        ("assets_in_service", Json::int(*count)),
        ("register_total", Json::str(&reconciliation::fixed(total))),
        ("ledger_balance", amount(balance.as_ref())),
        ("difference", amount(difference.as_ref())),
    ]);
    let code = match (row, difference.as_ref()) {
        (_, Some(difference)) if difference.is_zero() => return (tie, None),
        (None, _) => "ACCOUNT_NOT_IN_TRIAL_BALANCE",
        _ if *role == COST => "COST_DIFFERENCE",
        _ => "ACCUMULATED_DEPRECIATION_DIFFERENCE",
    };
    let mut finding = tie.clone();
    finding.set("code", Json::str(code));
    (tie, Some(finding))
}

fn review(
    register: &SourceSnapshot,
    trial_balance: &SourceSnapshot,
    currency: &str,
) -> Result<Json> {
    let rows = loader::load_canonical_tb(trial_balance)?;
    let (tenant, report_date) = (rows[0].tenant.clone(), rows[0].report_date);
    let (register_tenant, assets) = load(register, currency, report_date)?;
    if register_tenant != tenant {
        return Err(format!(
            "The register's tenant {} is not the trial balance's {}.",
            pytext::repr(&register_tenant),
            pytext::repr(&tenant)
        ));
    }
    let ledger: BTreeMap<&str, &TrialBalanceRow> = rows
        .iter()
        .map(|row| (row.account_id.as_str(), row))
        .collect();
    let in_service: Vec<&Asset> = assets.iter().filter(|asset| !asset.disposed).collect();
    let mut findings: Vec<Json> = in_service
        .iter()
        .filter(|asset| asset.accumulated > asset.cost)
        .map(|asset| over_depreciated(asset))
        .collect();
    let mut ties = Vec::new();
    for (account, entry) in &totals(&assets)? {
        let (tie, finding) = tie(account, entry, ledger.get(account).copied());
        ties.push(tie);
        findings.extend(finding);
    }
    let passed = !in_service.is_empty() && findings.is_empty();
    Ok(Json::object([
        ("schema_version", Json::int(1)),
        ("status", Json::str(if passed { "PASS" } else { "REVIEW" })),
        ("tenant", Json::str(&tenant)),
        ("currency", Json::str(currency)),
        ("report_date", Json::str(&report_date.iso())),
        ("boundary", Json::str(BOUNDARY)),
        (
            "source_sha256",
            Json::object([
                ("register", Json::str(&register.sha256)),
                ("trial_balance", Json::str(&trial_balance.sha256)),
            ]),
        ),
        ("asset_count", Json::int(assets.len() as i64)),
        ("assets_in_service", Json::int(in_service.len() as i64)),
        ("accounts", Json::Array(ties)),
        ("findings", Json::Array(findings)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pyjson;

    const REGISTER: &str = "Tenant,AssetID,Description,CostAccountID,DepreciationAccountID,Cost,AccumulatedDepreciation,AcquisitionDate,DisposalDate,Currency\n";
    const TB: &str = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n";
    type Row<'a> = (
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
    );

    fn register(rows: &[Row]) -> String {
        let mut text = REGISTER.to_owned();
        for (id, cost_account, depreciation_account, cost, accumulated, acquired, disposed) in rows
        {
            text += &format!(
                "Fabricated Varrock,{id},Asset {id},{cost_account},{depreciation_account},{cost},{accumulated},{acquired},{disposed},AUD\n"
            );
        }
        text
    }

    /// A trial balance at 30 September 2026 of `(account, YTD debit, YTD credit)`.
    fn ledger(accounts: &[(&str, &str, &str)]) -> String {
        let mut text = TB.to_owned();
        for (id, debit, credit) in accounts {
            text += &format!(
                "2026-09-30,Fabricated Varrock,Assets,{id},Account {id},{id},0.00,0.00,{debit},{credit}\n"
            );
        }
        text
    }

    fn run_text(register: &str, ledger: &str) -> Result<Json> {
        let register = SourceSnapshot::new(Path::new("register.csv"), register.as_bytes().to_vec());
        let ledger = SourceSnapshot::new(Path::new("tb.csv"), ledger.as_bytes().to_vec());
        decimal::with_precision(40, || review(&register, &ledger, "AUD"))
    }

    fn text(value: &Json) -> String {
        value.as_pystr().unwrap().to_utf8().unwrap()
    }

    /// Each finding's code with its account or asset.
    fn codes(result: &Json) -> Vec<(String, String)> {
        result
            .at("findings")
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| {
                let subject = finding
                    .get("account_id")
                    .or_else(|| finding.get("asset_id"))
                    .unwrap();
                (text(finding.at("code")), text(subject))
            })
            .collect()
    }

    const VAN: Row = (
        "FA-1",
        "710",
        "711",
        "30000.00",
        "6000.00",
        "2025-07-01",
        "",
    );
    const DESK: Row = ("FA-2", "720", "721", "1200.00", "300.00", "2025-08-15", "");

    #[test]
    fn a_register_that_ties_passes() {
        let result = run_text(
            &register(&[VAN, DESK]),
            &ledger(&[
                ("710", "30000.00", "0.00"),
                ("711", "0.00", "6000.00"),
                ("720", "1200", "0"),
                ("721", "0", "300"),
            ]),
        )
        .unwrap();
        assert!(result.at("status").is_str("PASS"));
        assert!(codes(&result).is_empty());
        let first = &result.at("accounts").as_array().unwrap()[1];
        assert!(first.at("role").is_str("accumulated_depreciation"));
        assert!(first.at("ledger_balance").is_str("6000.00"));
        assert!(first.at("difference").is_str("0.00"));
        assert_eq!(pyjson::canonical(result.at("assets_in_service")), "2");
    }

    #[test]
    fn differences_and_missing_accounts_are_listed() {
        let result = run_text(
            &register(&[VAN, DESK]),
            &ledger(&[
                ("710", "31000.00", "0.00"),
                ("711", "0.00", "5500.00"),
                ("720", "1200.00", "0.00"),
            ]),
        )
        .unwrap();
        assert!(result.at("status").is_str("REVIEW"));
        assert_eq!(
            codes(&result),
            [
                ("COST_DIFFERENCE".into(), "710".into()),
                ("ACCUMULATED_DEPRECIATION_DIFFERENCE".into(), "711".into()),
                ("ACCOUNT_NOT_IN_TRIAL_BALANCE".into(), "721".into()),
            ]
        );
        let findings = result.at("findings").as_array().unwrap();
        assert!(findings[0].at("difference").is_str("1000.00"));
        assert!(findings[1].at("difference").is_str("-500.00"));
        assert!(matches!(findings[2].at("account_name"), Json::Null));
        assert!(matches!(findings[2].at("ledger_balance"), Json::Null));
        assert!(findings[2].at("register_total").is_str("300.00"));
    }

    #[test]
    fn a_disposed_asset_leaves_the_totals_and_the_ledger_must_follow() {
        let sold = (
            "FA-3",
            "710",
            "711",
            "5000.00",
            "4000.00",
            "2024-01-01",
            "2026-09-12",
        );
        let later = (
            "FA-4",
            "710",
            "711",
            "800.00",
            "900.00",
            "2026-01-01",
            "2026-10-02",
        );
        let result = run_text(
            &register(&[VAN, sold, later]),
            &ledger(&[("710", "35800.00", "0.00"), ("711", "0.00", "6900.00")]),
        )
        .unwrap();
        assert_eq!(
            codes(&result),
            [
                ("DEPRECIATION_EXCEEDS_COST".into(), "FA-4".into()),
                ("COST_DIFFERENCE".into(), "710".into()),
            ]
        );
        assert!(
            result.at("findings").as_array().unwrap()[1]
                .at("difference")
                .is_str("5000.00")
        );
        assert_eq!(pyjson::canonical(result.at("assets_in_service")), "2");
        assert_eq!(pyjson::canonical(result.at("asset_count")), "3");
    }

    #[test]
    fn an_asset_bought_on_the_report_date_counts_and_one_sold_then_does_not() {
        let bought = ("FA-5", "710", "711", "100.00", "0.00", "2026-09-30", "");
        let sold = (
            "FA-6",
            "710",
            "711",
            "50.00",
            "10.00",
            "2025-01-01",
            "2026-09-30",
        );
        let result = run_text(
            &register(&[VAN, bought, sold]),
            &ledger(&[("710", "30100.00", "0.00"), ("711", "0.00", "6000.00")]),
        )
        .unwrap();
        assert!(result.at("status").is_str("PASS"));
        assert_eq!(pyjson::canonical(result.at("assets_in_service")), "2");
    }

    #[test]
    fn a_ledger_balance_finer_than_a_cent_is_compared_exactly() {
        // The canonical trial balance carries any precision; nothing is rounded.
        let result = run_text(
            &register(&[VAN]),
            &ledger(&[("710", "30000.001", "0.00"), ("711", "0.005", "6000.005")]),
        )
        .unwrap();
        assert_eq!(codes(&result), [("COST_DIFFERENCE".into(), "710".into())]);
        let finding = &result.at("findings").as_array().unwrap()[0];
        assert!(finding.at("ledger_balance").is_str("30000.001"));
        assert!(finding.at("difference").is_str("0.001"));
    }

    #[test]
    fn a_disposed_asset_s_accounts_keep_their_roles() {
        let sold = (
            "FA-3",
            "711",
            "712",
            "5.00",
            "1.00",
            "2024-01-01",
            "2025-01-01",
        );
        let error = run_text(&register(&[VAN, sold]), &ledger(&[("710", "1", "0")])).unwrap_err();
        assert!(error.contains("both a cost and"), "{error}");
    }

    #[test]
    fn a_register_with_nothing_in_service_needs_review() {
        let sold = (
            "FA-3",
            "710",
            "711",
            "5000.00",
            "4000.00",
            "2024-01-01",
            "2026-09-12",
        );
        let result = run_text(
            &register(&[sold]),
            &ledger(&[("710", "0", "0"), ("711", "0", "0")]),
        )
        .unwrap();
        assert!(result.at("status").is_str("REVIEW"));
        assert!(codes(&result).is_empty());
    }

    #[test]
    #[expect(clippy::too_many_lines, reason = "one refused register per line")]
    fn malformed_registers_are_refused() {
        let tb = ledger(&[("710", "1", "0")]);
        let refused = |rows: &str, message: &str| {
            let error = run_text(rows, &tb).unwrap_err();
            assert!(error.contains(message), "{error}");
        };
        refused(REGISTER, "no register rows were supplied");
        refused(&register(&[VAN, VAN]), "repeats AssetID 'FA-1'");
        refused(
            &register(&[("A", "710", "711", "0.00", "0.00", "2025-01-01", "")]),
            "positive Cost",
        );
        refused(
            &register(&[("A", "710", "711", "5.00", "-1.00", "2025-01-01", "")]),
            "positive Cost",
        );
        refused(
            &register(&[("A", "710", "710", "5.00", "1.00", "2025-01-01", "")]),
            "different cost",
        );
        refused(
            &register(&[("A", "710", "711", "5.00", "1.00", "2026-10-01", "")]),
            "after the trial balance",
        );
        refused(
            &register(&[(
                "A",
                "710",
                "711",
                "5.00",
                "1.00",
                "2025-01-01",
                "2024-12-31",
            )]),
            "before its AcquisitionDate",
        );
        refused(
            &register(&[(
                "A",
                "710",
                "711",
                "5.00",
                "1.00",
                "2025-01-01",
                "31/12/2025",
            )]),
            "DisposalDate",
        );
        refused(
            &register(&[VAN, ("A", "711", "712", "5.00", "1.00", "2025-01-01", "")]),
            "both a cost and",
        );
        refused(
            &register(&[VAN]).replace(",AUD\n", ",NZD\n"),
            "declared --currency 'AUD'",
        );
        refused(
            &(register(&[VAN]) + "Other Tenant,B,B,1,2,1.00,0.00,2025-01-01,,AUD\n"),
            "one tenant",
        );
        refused(
            &register(&[VAN]).replace("Fabricated Varrock", "Other Tenant"),
            "is not the trial balance's",
        );
    }

    #[test]
    fn the_currency_must_be_declared() {
        let error =
            fixed_assets(Path::new("register.csv"), Path::new("tb.csv"), "aud").unwrap_err();
        assert!(error.contains("three-letter currency"), "{error}");
    }
}
