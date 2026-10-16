//! Rank local transaction evidence without clearing or explaining a variance.
use crate::{
    Result,
    decimal::{self, Decimal},
    isodate::{self, Date},
    loader::{self, SourceSnapshot},
    publication,
    pyjson::{self, Json, PyStr},
    pytext,
    reconciliation::{self, COLUMNS},
    reconciliation_report, report,
    viewer::{self, VerifiedPack},
};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub const BOUNDARY: &str = "Review evidence only. The listed transactions are candidates for the reviewer's explanation; they do not explain, approve or clear a variance.";
const DRIVER_COLUMNS: [&str; 13] = [
    "Tenant",
    "AccountID",
    "AccountCode",
    "AccountName",
    "Movement",
    "TransactionsTotal",
    "Unexplained",
    "Rank",
    "TransactionID",
    "Date",
    "Reference",
    "Description",
    "Amount",
];
#[derive(Clone, Debug)]
pub(crate) struct Transaction {
    pub row: Json,
    pub date: Date,
    pub amount: Decimal,
    pub identifier: String,
}
pub(crate) struct Selection {
    pub by_account: BTreeMap<(String, String), Vec<Transaction>>,
    pub order: Vec<(String, String)>,
}
pub(crate) fn field(value: &Json, key: &str) -> String {
    value.at(key).as_pystr().unwrap().to_utf8().unwrap()
}
pub(crate) fn transactions(source: &SourceSnapshot, currency: &str) -> Result<Selection> {
    let mut by_account: BTreeMap<(String, String), Vec<Transaction>> = BTreeMap::new();
    let mut order = Vec::new();
    let mut seen = BTreeSet::new();
    for row in loader::csv_rows(source, &COLUMNS, "Transactions")? {
        let row = row?;
        let item = Json::Object(
            COLUMNS
                .iter()
                .map(|key| {
                    row.text(key, ["Reference", "Description"].contains(key))
                        .map(|value| (PyStr::from(*key), Json::str(&value)))
                })
                .collect::<Result<_>>()?,
        );
        if !item.at("Currency").is_str(currency) {
            return Err(format!(
                "{}: row {} is in {}, not the declared --currency {}.",
                crate::pypath::text(&source.path),
                row.number,
                pyjson::repr(item.at("Currency")),
                pytext::repr(currency)
            ));
        }
        let tenant = field(&item, "Tenant");
        let account = field(&item, "AccountID");
        let identifier = field(&item, "TransactionID");
        if !seen.insert((tenant.clone(), account.clone(), identifier.clone())) {
            return Err(format!(
                "{}: row {} repeats TransactionID {}.",
                crate::pypath::text(&source.path),
                row.number,
                pytext::repr(&identifier)
            ));
        }
        let debit =
            reconciliation::money(&field(&item, "Debit"), "Debit", &source.path, row.number)?;
        let credit =
            reconciliation::money(&field(&item, "Credit"), "Credit", &source.path, row.number)?;
        if debit < Decimal::zero()
            || credit < Decimal::zero()
            || (debit > Decimal::zero()) == (credit > Decimal::zero())
        {
            return Err(format!(
                "{}: row {} needs exactly one positive Debit or Credit.",
                crate::pypath::text(&source.path),
                row.number
            ));
        }
        let date = reconciliation::date(&field(&item, "Date"))?;
        let key = (tenant, account);
        if !by_account.contains_key(&key) {
            order.push(key.clone());
        }
        by_account.entry(key).or_default().push(Transaction {
            row: item,
            date,
            amount: debit.sub(&credit).value,
            identifier,
        });
    }
    Ok(Selection { by_account, order })
}
fn pack_date(value: &Json) -> Result<Date> {
    let raw = value.as_pystr().unwrap();
    let Some(raw_text) = raw.to_utf8() else {
        return Err(format!("Invalid isoformat string: {}", pyjson::repr(value)));
    };
    if let Some(date) = isodate::python_date_fromisoformat(&raw_text) {
        return Ok(date);
    }
    if raw_text.len() == 10
        && raw_text.as_bytes()[4] == b'-'
        && raw_text.as_bytes()[7] == b'-'
        && raw_text
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return reconciliation::date(&raw_text);
    }
    Err(format!("Invalid isoformat string: {}", pyjson::repr(value)))
}
fn validate_top(top: &BigInt) -> Result<()> {
    if top < &BigInt::from(1) {
        return Err("--top must be at least 1.".into());
    }
    Ok(())
}
pub fn variance_drivers(
    pack_directory: &Path,
    transaction_path: &Path,
    currency: &str,
    top: &BigInt,
) -> Result<Json> {
    validate_top(top)?;
    let pack = viewer::verify_pack(pack_directory)?;
    drivers_with(
        &pack,
        transaction_path,
        currency,
        top,
        &mut SourceSnapshot::capture,
    )
}
pub fn drivers_with(
    pack: &VerifiedPack,
    path: &Path,
    currency: &str,
    top: &BigInt,
    capture: &mut impl FnMut(&Path, &str) -> Result<SourceSnapshot>,
) -> Result<Json> {
    validate_top(top)?;
    let prior = pack.document.at("prior_report_dates").as_array().unwrap();
    let current = pack.document.at("current_report_dates").as_array().unwrap();
    if prior.len() != 1 || current.len() != 1 {
        return Err("The pack must carry one prior and one current report date.".into());
    }
    let prior = pack_date(&prior[0])?;
    let current = pack_date(&current[0])?;
    let source = capture(path, "Transactions")?;
    let findings = pack.document.at("exceptions").as_array().unwrap();
    let accounts = decimal::with_precision(40, || -> Result<Vec<Json>> {
        let selection = transactions(&source, currency)?;
        let mut accounts = Vec::new();
        for finding in findings {
            if !finding.at("control").is_str("period_variance") {
                continue;
            }
            let key = (field(finding, "tenant"), field(finding, "account_id"));
            let window: Vec<_> = selection
                .by_account
                .get(&key)
                .into_iter()
                .flatten()
                .filter(|item| prior < item.date && item.date <= current)
                .collect();
            let movement = Decimal::parse(&field(finding, "difference"))
                .map_err(|_| "[<class 'decimal.ConversionSyntax'>]")?;
            if !movement.is_finite() {
                // Native 1.0 refuses non-finite values in hand-edited packs.
                return Err("The variance difference must be a finite decimal.".into());
            }
            let total = window
                .iter()
                .fold(Decimal::zero(), |sum, row| sum.add(&row.amount).value);
            let difference = movement.sub(&total).value;
            let mut ranked: Vec<_> = window
                .iter()
                .map(|row| (row.amount.copy_abs(), *row))
                .collect();
            ranked.sort_by(|(a_amount, a), (b_amount, b)| {
                b_amount
                    .cmp_value(a_amount)
                    .then_with(|| a.date.cmp(&b.date))
                    .then_with(|| a.identifier.cmp(&b.identifier))
            });
            ranked.truncate(top.to_usize().unwrap_or(usize::MAX));
            let mut drivers = Vec::new();
            for (_, item) in ranked {
                drivers.push(Json::object([
                    ("TransactionID", item.row.at("TransactionID").clone()),
                    ("Date", item.row.at("Date").clone()),
                    ("Reference", item.row.at("Reference").clone()),
                    ("Description", item.row.at("Description").clone()),
                    ("Amount", Json::str(&reconciliation::fixed(&item.amount))),
                ]));
            }
            let formatted = |value: &Decimal| {
                value
                    .format_places(2)
                    .ok_or_else(|| "decimal fixed-point output exceeds available memory".to_owned())
            };
            accounts.push(Json::object([
                ("tenant", finding.at("tenant").clone()),
                ("account_id", finding.at("account_id").clone()),
                ("account_code", finding.at("account_code").clone()),
                ("account_name", finding.at("account_name").clone()),
                ("movement", Json::str(&formatted(&movement)?)),
                ("transactions_total", Json::str(&formatted(&total)?)),
                ("unexplained", Json::str(&formatted(&difference)?)),
                ("transactions_in_window", Json::int(window.len() as i64)),
                ("drivers", Json::Array(drivers)),
            ]));
        }
        Ok(accounts)
    })?;
    let reset = findings
        .iter()
        .any(|row| row.at("control").is_str("financial_year_reset"));
    let covered = accounts.iter().all(|row| {
        Decimal::parse(&field(row, "unexplained"))
            .unwrap()
            .is_zero()
    });
    let mut sources = vec![(PyStr::from("transactions"), Json::str(&source.sha256))];
    for name in viewer::PACK_NAMES {
        if let Some(hash) = pack.artefact_sha256.get(name) {
            sources.push((
                PyStr::from(format!("pack:{name}").as_str()),
                Json::str(hash),
            ));
        }
    }
    Ok(Json::object([
        (
            "status",
            Json::str(if covered && !reset { "PASS" } else { "REVIEW" }),
        ),
        ("financial_year_reset", Json::Bool(reset)),
        ("currency", Json::str(currency)),
        ("boundary", Json::str(BOUNDARY)),
        (
            "window",
            Json::object([
                ("after", Json::str(&prior.iso())),
                ("through", Json::str(&current.iso())),
            ]),
        ),
        ("top", Json::Int(top.clone())),
        ("source_sha256", Json::Object(sources)),
        ("accounts", Json::Array(accounts)),
    ]))
}
pub fn rendered(result: &Json) -> Result<Vec<(String, Vec<u8>)>> {
    let mut rows = Vec::new();
    for account in result.at("accounts").as_array().unwrap() {
        let mut head: Vec<_> = ["tenant", "account_id", "account_code", "account_name"]
            .iter()
            .map(|key| report::csv_safe(&field(account, key)))
            .collect();
        head.extend(
            ["movement", "transactions_total", "unexplained"].map(|key| field(account, key)),
        );
        let drivers = account.at("drivers").as_array().unwrap();
        if drivers.is_empty() {
            let mut row = head;
            row.extend([""; 6].map(str::to_owned));
            rows.push(row);
        } else {
            for (index, driver) in drivers.iter().enumerate() {
                let mut row = head.clone();
                row.extend([
                    (index + 1).to_string(),
                    report::csv_safe(&field(driver, "TransactionID")),
                    field(driver, "Date"),
                    report::csv_safe(&field(driver, "Reference")),
                    report::csv_safe(&field(driver, "Description")),
                    field(driver, "Amount"),
                ]);
                rows.push(row);
            }
        }
    }
    let mut csv = b"\xef\xbb\xbf".to_vec();
    csv.extend(reconciliation_report::csv(&DRIVER_COLUMNS, &rows).as_bytes());
    Ok(vec![
        (
            "variance-drivers.json".into(),
            (pyjson::pretty_utf8(result)? + "\n").into_bytes(),
        ),
        ("variance-drivers.csv".into(), csv),
    ])
}
pub fn write_drivers(result: &Json, output: &Path) -> Result<PathBuf> {
    Ok(publication::write_new_files(output, || rendered(result))?.join("variance-drivers.csv"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn top_limits_display_while_all_window_transactions_determine_the_remainder() {
        let mut pack = VerifiedPack {
            document: Json::object([
                (
                    "prior_report_dates",
                    Json::Array(vec![Json::str("2026-08-31")]),
                ),
                (
                    "current_report_dates",
                    Json::Array(vec![Json::str("2026-09-30")]),
                ),
                (
                    "exceptions",
                    Json::Array(vec![Json::object([
                        ("control", Json::str("period_variance")),
                        ("tenant", Json::str("Synthetic")),
                        ("account_id", Json::str("A")),
                        ("account_code", Json::str("001")),
                        ("account_name", Json::str("Synthetic account")),
                        ("difference", Json::str("30")),
                    ])]),
                ),
            ]),
            summary_text: String::new(),
            exceptions: vec![],
            queries: None,
            artefact_sha256: BTreeMap::new(),
        };
        let csv = "Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit\nSynthetic,A,AUD,BEFORE,2026-08-31,,,1000,0\nSynthetic,A,AUD,Z,2026-09-02,,,20,0\nSynthetic,A,AUD,B,2026-09-01,,,20,0\nSynthetic,A,AUD,C,2026-09-03,,,0,10\nSynthetic,A,AUD,AFTER,2026-10-01,,,1000,0\n";
        let mut capture =
            |path: &Path, _: &str| Ok(SourceSnapshot::new(path, csv.as_bytes().to_vec()));
        let result = drivers_with(
            &pack,
            Path::new("transactions.csv"),
            "AUD",
            &1.into(),
            &mut capture,
        )
        .unwrap();
        assert!(result.at("status").is_str("PASS"));
        let account = &result.at("accounts").as_array().unwrap()[0];
        assert!(account.at("transactions_total").is_str("30.00"));
        assert!(account.at("unexplained").is_str("0.00"));
        assert_eq!(
            account.at("transactions_in_window").as_plain_int(),
            Some(&3.into())
        );
        assert!(
            account.at("drivers").as_array().unwrap()[0]
                .at("TransactionID")
                .is_str("B")
        );
        let mut findings = pack.document.at("exceptions").as_array().unwrap().to_vec();
        findings.push(Json::object([(
            "control",
            Json::str("financial_year_reset"),
        )]));
        pack.document.set("exceptions", Json::Array(findings));
        assert!(
            drivers_with(
                &pack,
                Path::new("transactions.csv"),
                "AUD",
                &5.into(),
                &mut capture
            )
            .unwrap()
            .at("status")
            .is_str("REVIEW")
        );
        assert_eq!(
            variance_drivers(Path::new("missing"), Path::new("missing"), "AUD", &0.into())
                .unwrap_err(),
            "--top must be at least 1."
        );
        for value in ["NaN", "Infinity", "-Infinity"] {
            let mut findings = pack.document.at("exceptions").as_array().unwrap().to_vec();
            findings[0].set("difference", Json::str(value));
            pack.document.set("exceptions", Json::Array(findings));
            assert_eq!(
                drivers_with(
                    &pack,
                    Path::new("transactions.csv"),
                    "AUD",
                    &1.into(),
                    &mut capture
                )
                .unwrap_err(),
                "The variance difference must be a finite decimal."
            );
        }
    }
}
