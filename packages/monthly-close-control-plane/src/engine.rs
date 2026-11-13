// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Exact trial-balance controls. Findings and acknowledgements remain separate.
use crate::{
    Result,
    calculation_evidence::{self, CalculationEvidence, covers_period},
    decimal::{self, Decimal},
    equity,
    isodate::Date,
    loader::{self, SourceSnapshot},
    models::{
        ClientQuery, Contact, Control, ExceptionItem, GstClass, GstSettlements,
        ReviewerAcknowledgement, Status, TrialBalanceRow,
    },
    pyjson::Json,
    pytext, queries, report,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub type RowMap = BTreeMap<(String, String), TrialBalanceRow>;

#[derive(Clone, Debug)]
pub struct CloseReviewPack {
    pub status: Status,
    pub current_report_dates: Vec<String>,
    pub prior_report_dates: Vec<String>,
    pub source_hashes: BTreeMap<String, String>,
    pub absolute_threshold: Decimal,
    pub percentage_threshold: Decimal,
    pub reconciliation_tolerance: Decimal,
    /// The GST tolerance, absolute then percentage, when a GST mapping was supplied.
    pub gst_tolerance: Option<(Decimal, Decimal)>,
    pub exceptions: Vec<ExceptionItem>,
    pub acknowledgement: Option<ReviewerAcknowledgement>,
    pub calculation_evidence: Vec<CalculationEvidence>,
    pub required_calculations: Vec<String>,
    pub required_controls: Vec<RequiredControlRun>,
    pub controls_not_run: Vec<String>,
    pub relied_on: BTreeSet<String>,
    pub equity_reconciliation: Option<Json>,
    /// The build that wrote the pack. Only the command line sets it; the
    /// library leaves it unset so its output matches the frozen Python baseline.
    pub producer: Option<Json>,
}
impl CloseReviewPack {
    pub fn client_queries(&self) -> Result<Vec<ClientQuery>> {
        queries::derive_client_queries(&self.exceptions)
    }
}

/// How a control named by `--require-control` fared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    /// It read its input. Anything it found is in its own exceptions.
    Ran,
    /// Its input was not supplied.
    Skipped,
    /// Its input was supplied, but it could not finish on it.
    Failed,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ran => "ran",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Ran, Self::Skipped, Self::Failed]
            .into_iter()
            .find(|state| state.as_str() == text)
    }
}

#[derive(Clone, Debug)]
pub struct RequiredControlRun {
    pub control: Control,
    pub state: RunState,
}

#[derive(Clone, Debug)]
pub struct ReviewConfig {
    pub current_path: PathBuf,
    pub prior_path: PathBuf,
    pub mapping_path: Option<PathBuf>,
    pub mapping_policy_path: Option<PathBuf>,
    pub balance_policy_path: Option<PathBuf>,
    pub subledger_path: Option<PathBuf>,
    pub gst_mapping_path: Option<PathBuf>,
    pub gst_settlements_path: Option<PathBuf>,
    pub acknowledgement_path: Option<PathBuf>,
    pub equity_schedule_path: Option<PathBuf>,
    pub contacts_path: Option<PathBuf>,
    pub equity_currency: Option<String>,
    pub equity_currency_evidence: Option<String>,
    pub calculation_evidence_paths: Vec<PathBuf>,
    pub required_calculations: Vec<String>,
    pub required_controls: Vec<String>,
    pub absolute_threshold: Decimal,
    pub percentage_threshold: Decimal,
    pub reconciliation_tolerance: Decimal,
    pub gst_absolute_tolerance: Decimal,
    pub gst_percentage_tolerance: Decimal,
}
impl ReviewConfig {
    pub fn new(current: PathBuf, prior: PathBuf) -> Self {
        Self {
            current_path: current,
            prior_path: prior,
            mapping_path: None,
            mapping_policy_path: None,
            balance_policy_path: None,
            subledger_path: None,
            gst_mapping_path: None,
            gst_settlements_path: None,
            acknowledgement_path: None,
            equity_schedule_path: None,
            contacts_path: None,
            equity_currency: None,
            equity_currency_evidence: None,
            calculation_evidence_paths: Vec::new(),
            required_calculations: Vec::new(),
            required_controls: Vec::new(),
            absolute_threshold: Decimal::parse("1000").unwrap(),
            percentage_threshold: Decimal::parse("0.10").unwrap(),
            reconciliation_tolerance: Decimal::parse("0.01").unwrap(),
            gst_absolute_tolerance: Decimal::parse("1.00").unwrap(),
            gst_percentage_tolerance: Decimal::parse("0.01").unwrap(),
        }
    }
}

pub fn financial_year(when: Date) -> i32 {
    if when.month >= 7 {
        when.year
    } else {
        when.year - 1
    }
}

pub fn financial_year_reset_exceptions(current: Date, prior: Date) -> Vec<ExceptionItem> {
    if financial_year(current) == financial_year(prior) {
        return Vec::new();
    }
    vec![ExceptionItem::new(
        Control::FinancialYearReset,
        Status::Review,
        None,
        &format!(
            "Current ReportDate {} and prior ReportDate {} fall in different Australian financial years (1 July to 30 June). YTD figures reset on 1 July, so this YTD-vs-YTD comparison crosses a year reset and the period_variance verdicts for profit-and-loss-style rows are not meaningful.",
            current.iso(),
            prior.iso()
        ),
        "Do not rely on period_variance results for revenue and expense rows in this pack. Compare two report dates inside one financial year, or review the movement against a source that does not reset at year end.",
    )]
}

pub fn percent_change(current: &Decimal, prior: &Decimal) -> Option<Decimal> {
    if prior.is_zero() {
        None
    } else {
        Some(current.sub(prior).value.div(prior).abs())
    }
}

pub fn exact_total<'a>(
    values: impl IntoIterator<Item = &'a Decimal>,
    period: &str,
    field: &str,
) -> Result<Decimal> {
    let mut total = Decimal::zero();
    for value in values {
        let next = total.add(value);
        if next.inexact {
            return Err(format!(
                "{period} {field} total exceeds the supported 28-digit precision, so debits and credits cannot be compared exactly."
            ));
        }
        total = next.value;
    }
    Ok(total)
}

pub fn integrity_exceptions(rows: &[TrialBalanceRow], period: &str) -> Result<Vec<ExceptionItem>> {
    let debit = exact_total(rows.iter().map(|row| &row.debit), period, "Debit")?;
    let credit = exact_total(rows.iter().map(|row| &row.credit), period, "Credit")?;
    let ytd_debit = exact_total(rows.iter().map(|row| &row.ytd_debit), period, "YTDDebit")?;
    let ytd_credit = exact_total(rows.iter().map(|row| &row.ytd_credit), period, "YTDCredit")?;
    let mut result = Vec::new();
    for (label, debit, credit) in [("movement", debit, credit), ("YTD", ytd_debit, ytd_credit)] {
        if debit == credit {
            continue;
        }
        let mut item = ExceptionItem::new(
            Control::TrialBalanceIntegrity,
            Status::Blocked,
            None,
            &format!("{period} {label} debit and credit totals do not balance."),
            "Correct or re-export the source trial balance before relying on any review result.",
        );
        item.difference = Some(debit.sub(&credit).value);
        item.current_value = Some(debit);
        item.prior_value = Some(credit);
        item.threshold = Some(Decimal::zero());
        result.push(item);
    }
    Ok(result)
}

#[expect(
    clippy::too_many_lines,
    reason = "raises each period comparison exception"
)]
pub fn period_comparison_exceptions(
    current: &RowMap,
    prior: &RowMap,
    absolute_threshold: &Decimal,
    percentage_threshold: &Decimal,
) -> Vec<ExceptionItem> {
    let keys: BTreeSet<_> = current.keys().chain(prior.keys()).collect();
    let mut result = Vec::new();
    for key in keys {
        let current = current.get(key);
        let prior = prior.get(key);
        match (current, prior) {
            (None, Some(prior)) => {
                let mut item = ExceptionItem::new(
                    Control::PeriodComparison,
                    Status::Review,
                    Some(prior),
                    "Account was present in the prior period but is absent from the current period.",
                    "Confirm the account was closed, reclassified, or omitted intentionally.",
                );
                item.prior_value = Some(prior.ytd_net());
                result.push(item);
            }
            (Some(current), None) => {
                let mut item = ExceptionItem::new(
                    Control::PeriodComparison,
                    Status::Review,
                    Some(current),
                    "New account is present in the current period only.",
                    "Confirm the account setup, mapping, and first-period activity.",
                );
                item.current_value = Some(current.ytd_net());
                result.push(item);
            }
            (Some(current), Some(prior)) => {
                let current_net = current.ytd_net();
                let prior_net = prior.ytd_net();
                let difference = current_net.sub(&prior_net).value;
                let percentage = percent_change(&current_net, &prior_net);
                let by_amount = !difference.is_zero() && difference.abs() >= *absolute_threshold;
                let by_percentage = percentage
                    .as_ref()
                    .is_none_or(|value| value >= percentage_threshold);
                if by_amount && by_percentage {
                    let reason = if percentage.is_none() {
                        "YTD net balance movement met or exceeded the configured absolute threshold; no percentage change could be computed from the prior YTD balance, so the percentage threshold was not tested."
                    } else if !current_net.is_zero()
                        && (current_net < Decimal::zero()) != (prior_net < Decimal::zero())
                    {
                        "YTD net balance changed sign, and its movement met or exceeded both configured materiality thresholds; the percentage change measures the move across zero."
                    } else {
                        "YTD net balance movement met or exceeded both configured materiality thresholds."
                    };
                    let mut item = ExceptionItem::new(
                        Control::PeriodVariance,
                        Status::Review,
                        Some(current),
                        reason,
                        "Investigate the driver, retain supporting evidence, and document the reviewer conclusion.",
                    );
                    item.current_value = Some(current_net.clone());
                    item.prior_value = Some(prior_net.clone());
                    item.difference = Some(difference.clone());
                    item.threshold = Some(absolute_threshold.clone());
                    item.percentage_change = percentage;
                    result.push(item);
                }
                let mut changed = Vec::new();
                if current.account_code != prior.account_code {
                    changed.push("account code");
                }
                if current.account_name != prior.account_name {
                    changed.push("account name");
                }
                if current.section != prior.section {
                    changed.push("section");
                }
                if !changed.is_empty() {
                    let mut item = ExceptionItem::new(
                        Control::AccountMetadata,
                        Status::Review,
                        Some(current),
                        &format!(
                            "Stable AccountID has changed {} since the prior period.",
                            changed.join(" and ")
                        ),
                        "Confirm that the chart-of-accounts change and any reporting impact were reviewed.",
                    );
                    item.current_value = Some(current_net);
                    item.prior_value = Some(prior_net);
                    item.difference = Some(difference);
                    result.push(item);
                }
            }
            (None, None) => unreachable!("key came from the source maps"),
        }
    }
    result
}

fn ordered_rows(rows: &[TrialBalanceRow]) -> Vec<&TrialBalanceRow> {
    let mut rows: Vec<_> = rows.iter().collect();
    rows.sort_by(|a, b| (&a.tenant, &a.account_id).cmp(&(&b.tenant, &b.account_id)));
    rows
}

pub fn mapping_exceptions(
    rows: &[TrialBalanceRow],
    mapping: &BTreeMap<String, String>,
) -> Vec<ExceptionItem> {
    ordered_rows(rows).into_iter().filter(|row| !mapping.contains_key(&row.account_id)).map(|row| {
        let mut item = ExceptionItem::new(Control::AccountMapping, Status::Review, Some(row),
            "Current account has no supplied review-group mapping.",
            "Assign and review an appropriate reporting group before using a grouped close pack.");
        item.current_value = Some(row.ytd_net()); item
    }).collect()
}

pub fn mapping_compatibility_exceptions(
    rows: &[TrialBalanceRow],
    mapping: &BTreeMap<String, String>,
    policy: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<ExceptionItem> {
    let mut result = Vec::new();
    for row in ordered_rows(rows) {
        let Some(group) = mapping.get(&row.account_id) else {
            continue;
        };
        let permitted = policy.get(group);
        if permitted.is_some_and(|values| values.contains(&row.section)) {
            continue;
        }
        let rule = if let Some(values) = permitted.filter(|values| !values.is_empty()) {
            format!(
                "Permitted Section values: {}.",
                values
                    .iter()
                    .map(|value| pytext::repr(value))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            "The supplied policy declares no permitted Section for this group.".into()
        };
        let mut item = ExceptionItem::new(
            Control::MappingCompatibility,
            Status::Review,
            Some(row),
            &format!(
                "Source Section {} is not permitted for supplied ReviewGroup {}. {rule}",
                pytext::repr(&row.section),
                pytext::repr(group)
            ),
            "Compare the source chart of accounts, supplied mapping and mapping policy. Retain evidence for the reporting treatment before changing either file; this exception does not reclassify the account.",
        );
        item.current_value = Some(row.ytd_net());
        result.push(item);
    }
    result
}

pub fn balance_policy_exceptions(
    current: &RowMap,
    tenant: &str,
    policy: &BTreeMap<String, (String, bool)>,
) -> Vec<ExceptionItem> {
    let mut result = Vec::new();
    for (id, (expected, movement)) in policy {
        let row = current.get(&(tenant.into(), id.clone()));
        let balance = row.map_or_else(Decimal::zero, TrialBalanceRow::ytd_net);
        let side = if balance > Decimal::zero() {
            "debit"
        } else if balance < Decimal::zero() {
            "credit"
        } else {
            "nil"
        };
        let mut reasons = Vec::new();
        if expected == "nil" && side != "nil" {
            reasons.push(format!(
                "The policy expects a nil balance; the account carries a {side} balance."
            ));
        } else if ["debit", "credit"].contains(&expected.as_str())
            && side != expected
            && side != "nil"
        {
            reasons.push(format!(
                "The policy expects a {expected} balance; the account carries a {side} balance."
            ));
        }
        if *movement && row.is_none_or(|row| row.debit.is_zero() && row.credit.is_zero()) {
            reasons.push("The policy expects movement every period; the account has none in the current period".to_owned()
                + if row.is_none() { " and is absent from the current trial balance." } else { "." });
        }
        for reason in reasons {
            let mut item = ExceptionItem::new(
                Control::BalancePolicy,
                Status::Review,
                row,
                &reason,
                "Trace the balance or the missing entries to the ledger and retain the evidence. Correct the policy file only if the account's expected behaviour has changed.",
            );
            item.current_value = Some(balance.clone());
            if row.is_none() {
                item.tenant = tenant.into();
                item.account_id = id.clone();
            }
            result.push(item);
        }
    }
    result
}

pub fn subledger_exceptions(
    current: &RowMap,
    subledger: &BTreeMap<(String, String), Decimal>,
    tolerance: &Decimal,
) -> Vec<ExceptionItem> {
    let mut result = Vec::new();
    for (key, value) in subledger {
        let row = current.get(key);
        if let Some(row) = row {
            let balance = row.ytd_net();
            let difference = balance.sub(value).value;
            if difference.abs() > *tolerance {
                let mut item = ExceptionItem::new(
                    Control::SubledgerReconciliation,
                    Status::Review,
                    Some(row),
                    "Current trial-balance balance differs from the supplied subledger beyond tolerance.",
                    "Reconcile the difference to timing, mapping, or source records and retain support.",
                );
                item.current_value = Some(balance);
                item.prior_value = Some(value.clone());
                item.difference = Some(difference);
                item.threshold = Some(tolerance.clone());
                result.push(item);
            }
        } else {
            let mut item = ExceptionItem::new(
                Control::SubledgerReconciliation,
                Status::Review,
                None,
                "Supplied subledger balance has no matching current trial-balance account.",
                "Confirm the control-account mapping or correct the source files.",
            );
            item.tenant = key.0.clone();
            item.account_id = key.1.clone();
            item.prior_value = Some(value.clone());
            item.threshold = Some(tolerance.clone());
            result.push(item);
        }
    }
    result
}

/// `REVIEW` for each current account with a closing balance that no supplied
/// subledger balance or computed equity reconciliation ties out, when its
/// `Section` is Assets, Liabilities or Equity, or is none of those nor Revenue
/// or Expenses, so whether it needs a reconciliation cannot be told.
pub fn reconciliation_coverage_exceptions(
    rows: &[TrialBalanceRow],
    subledger: &BTreeMap<(String, String), Decimal>,
    equity: Option<&Json>,
) -> Vec<ExceptionItem> {
    let reconciled: BTreeSet<String> = equity
        .and_then(|result| result.at("accounts").as_array())
        .into_iter()
        .flatten()
        .filter_map(|account| account.at("account_id").as_pystr())
        .map(|id| id.to_utf8().unwrap_or_else(|| id.os_text()))
        .collect();
    let mut result = Vec::new();
    for row in ordered_rows(rows) {
        let balance = row.ytd_net();
        if balance.is_zero()
            || subledger.contains_key(&row.key())
            || reconciled.contains(&row.account_id)
        {
            continue;
        }
        let reason = match pytext::casefold(&row.section).as_str() {
            "revenue" | "expenses" => continue,
            "assets" | "liabilities" | "equity" => {
                "Balance-sheet account with a closing balance and no supplied subledger balance or equity reconciliation: this pack ties it to nothing."
            }
            _ => {
                "Account with a closing balance and no supplied subledger balance or equity reconciliation, in a Section that is not Assets, Liabilities, Equity, Revenue or Expenses, so whether it needs a reconciliation cannot be told."
            }
        };
        let mut item = ExceptionItem::new(
            Control::ReconciliationCoverage,
            Status::Review,
            Some(row),
            reason,
            "Supply its reconciliation as a subledger balance, or select the equity account in an equity schedule, and run the review again; or retain why it needs none. This exception adjusts nothing.",
        );
        item.current_value = Some(balance);
        result.push(item);
    }
    result
}

/// Whether 11 ASCII digits pass the ABN check: subtract 1 from the first
/// digit, weight the digits 10, 1, 3, 5, ..., 19, and the sum divides by 89.
/// Copied from evatt's `patterns::valid`, since a component may not import a
/// sibling.
fn abn_check_digits_hold(digits: &str) -> bool {
    const WEIGHTS: [u32; 11] = [10, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19];
    let values: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    digits.len() == 11
        && values.len() == 11
        && values.iter().zip(WEIGHTS).map(|(v, w)| v * w).sum::<u32>() % 89 == 10
}

/// The contacts named in a finding, as `ID (name)`.
fn contact_list(contacts: &[&Contact]) -> String {
    contacts
        .iter()
        .map(|c| format!("{} ({})", pytext::repr(&c.id), pytext::repr(&c.name)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn contact_item(reason: &str, action: &str) -> ExceptionItem {
    ExceptionItem::new(
        Control::ContactRegister,
        Status::Review,
        None,
        reason,
        action,
    )
}

/// The finding for a recorded ABN that `digits`, its spaces removed, show is
/// not a valid ABN.
fn invalid_abn(contact: &Contact, digits: &str) -> ExceptionItem {
    let fault = if digits.len() == 11 && digits.bytes().all(|b| b.is_ascii_digit()) {
        "fails the ABN check digits"
    } else {
        "is not 11 digits"
    };
    contact_item(
        &format!(
            "Contact {} has ABN {}, which {fault}, so it is not a valid ABN.",
            contact_list(&[contact]),
            pytext::repr(&contact.abn)
        ),
        "Confirm the ABN on the Australian Business Register's ABN Lookup and correct the contact. This exception changes nothing in the ledger.",
    )
}

/// `REVIEW` for each contact whose recorded ABN is not 11 digits or fails the
/// check digits, each valid ABN held by more than one contact, and each name
/// held by more than one contact once case and spacing are ignored. A blank
/// ABN is not checked.
pub fn contact_register_exceptions(contacts: &[Contact]) -> Vec<ExceptionItem> {
    let mut result = Vec::new();
    let mut by_abn: BTreeMap<String, Vec<&Contact>> = BTreeMap::new();
    let mut by_name: BTreeMap<String, Vec<&Contact>> = BTreeMap::new();
    for contact in contacts {
        let name = pytext::casefold(&contact.name);
        by_name
            .entry(name.split_whitespace().collect::<Vec<_>>().join(" "))
            .or_default()
            .push(contact);
        let digits = contact.abn.replace(' ', "");
        if abn_check_digits_hold(&digits) {
            by_abn.entry(digits).or_default().push(contact);
        } else if !digits.is_empty() {
            result.push(invalid_abn(contact, &digits));
        }
    }
    for (abn, holders) in by_abn.into_iter().filter(|(_, h)| h.len() > 1) {
        result.push(contact_item(
            &format!(
                "ABN {abn} is recorded on {} contacts: {}.",
                holders.len(),
                contact_list(&holders)
            ),
            "Confirm whether they are one business entered more than once, which should be merged, or separate contacts that share an ABN, such as branches. This exception changes nothing in the ledger.",
        ));
    }
    for holders in by_name.into_values().filter(|h| h.len() > 1) {
        result.push(contact_item(
            &format!(
                "{} contacts have the same name once case and spacing are ignored: {}.",
                holders.len(),
                contact_list(&holders)
            ),
            "Confirm whether they are one contact entered more than once, which should be merged. This exception changes nothing in the ledger.",
        ));
    }
    result
}

/// Compares the period's net GST movement, with the declared settlements added
/// back, with 1/10 of the GST-exclusive taxable sales less 1/10 of the
/// creditable purchases, and says whether the control ran. Sales and GST
/// amounts are credits less debits; purchases and settlements are debits less
/// credits. A mapping that splits GST into `gst_collected` and `gst_paid`
/// accounts also compares each side with its own expectation, so equal
/// shortfalls on both sides, which offset in the net figure, are still raised.
/// `settlements` is the declaration, or why there is none to use.
pub fn gst_reasonableness_exceptions(
    rows: &[TrialBalanceRow],
    mapping: &BTreeMap<String, GstClass>,
    settlements: std::result::Result<&GstSettlements, &str>,
    absolute: &Decimal,
    percentage: &Decimal,
) -> Result<(Vec<ExceptionItem>, bool)> {
    let (mut unclassed, mut mixed) = (Vec::new(), Vec::new());
    let mut sides = GstSides::default();
    let mut recorded = Vec::new();
    for row in ordered_rows(rows) {
        let moved = !(row.debit.is_zero() && row.credit.is_zero());
        let credit_less_debit = row.credit.sub(&row.debit).value;
        match mapping.get(&row.account_id) {
            None if moved => unclassed.push(row),
            Some(GstClass::Mixed) if moved => mixed.push(row),
            Some(GstClass::TaxableSales) => sides.sales.push(credit_less_debit),
            Some(GstClass::CreditablePurchases) => sides.purchases.push(row.current_net()),
            Some(GstClass::Gst) => recorded.push(credit_less_debit),
            Some(GstClass::GstCollected) => {
                recorded.push(credit_less_debit.clone());
                sides.collected.push(credit_less_debit);
            }
            Some(GstClass::GstPaid) => {
                recorded.push(credit_less_debit);
                sides.paid.push(row.current_net());
            }
            _ => {}
        }
    }
    let sided = mapping
        .values()
        .any(|class| *class == GstClass::GstCollected);
    let settlements = settlements.and_then(|declared| gst_declaration_fits(sided, declared));
    let ran = settlements.is_ok() && unclassed.is_empty() && mixed.is_empty();
    let mut result = match settlements {
        Ok(declared) if ran => {
            let settled: Vec<_> = declared.rows.iter().map(|(_, a)| a.clone()).collect();
            let (sales, purchases) = (&sides.sales, &sides.purchases);
            let net = gst_difference(sales, purchases, &recorded, &settled, absolute, percentage)?;
            let mut found = Vec::from_iter(net);
            if sided {
                found.extend(gst_side_differences(
                    &sides, declared, absolute, percentage,
                )?);
            }
            found
        }
        _ => vec![gst_not_run(&unclassed, &mixed, settlements.err())],
    };
    for item in &mut result {
        item.tenant.clone_from(&rows[0].tenant);
    }
    Ok((result, ran))
}

/// A declaration with a `Side` column goes with a mapping that splits GST by
/// side, and one without it with a mapping that does not. A header alone
/// declares no settlement, so either shape fits either mapping.
fn gst_declaration_fits(
    sided: bool,
    declared: &GstSettlements,
) -> std::result::Result<&GstSettlements, &'static str> {
    if declared.rows.is_empty() {
        return Ok(declared);
    }
    match (sided, declared.sided) {
        (true, false) => Err(
            "The GST settlements declaration cannot be used: the GST mapping splits GST into gst_collected and gst_paid accounts, so each settlement needs a Side saying which side it was posted to.",
        ),
        (false, true) => Err(
            "The GST settlements declaration cannot be used: it has a Side column, but the GST mapping classes its GST accounts gst rather than splitting them into gst_collected and gst_paid.",
        ),
        _ => Ok(declared),
    }
}

/// The movements a GST mapping classes: sales and GST collected as credits less
/// debits, purchases and GST paid as debits less credits.
#[derive(Default)]
struct GstSides {
    sales: Vec<Decimal>,
    purchases: Vec<Decimal>,
    collected: Vec<Decimal>,
    paid: Vec<Decimal>,
}

/// GST collected against 1/10 of taxable sales, and GST paid against 1/10 of
/// creditable purchases, each with the settlements declared against its side
/// added back: a settlement's net debit adds to collected and takes from paid.
/// The tolerance is the absolute amount plus the percentage of that side's
/// expected GST.
fn gst_side_differences(
    sides: &GstSides,
    declared: &GstSettlements,
    absolute: &Decimal,
    percentage: &Decimal,
) -> Result<Vec<ExceptionItem>> {
    let on = |side: GstClass, sign: fn(&Decimal) -> Decimal| -> Vec<Decimal> {
        let rows = declared.rows.iter().filter(|(s, _)| *s == Some(side));
        rows.map(|(_, amount)| sign(amount)).collect()
    };
    let mut found = Vec::new();
    for (label, base_label, base, movement, settled) in [
        (
            "collected",
            "taxable sales",
            &sides.sales,
            &sides.collected,
            on(GstClass::GstCollected, Decimal::clone),
        ),
        (
            "paid",
            "creditable purchases",
            &sides.purchases,
            &sides.paid,
            on(GstClass::GstPaid, |amount| {
                Decimal::zero().sub(amount).value
            }),
        ),
    ] {
        let base = exact_total(base, "Current", base_label)?;
        let movement = exact_total(movement, "Current", &format!("GST {label} movement"))?;
        let settled = exact_total(&settled, "Declared", &format!("GST {label} settlements"))?;
        let recorded = exact_total([&movement, &settled], "Current", "GST recorded")?;
        let expected = base.div(&Decimal::parse("10").unwrap());
        let difference = recorded.sub(&expected).value;
        let tolerance = absolute.add(&percentage.mul(&expected.abs())).value;
        if difference.abs() > tolerance {
            let figures = [
                &recorded,
                &movement,
                &settled,
                &expected,
                &base,
                &difference,
            ];
            found.push(gst_side_item(label, base_label, figures, tolerance)?);
        }
    }
    Ok(found)
}

/// The exception for one side of a split GST mapping beyond its tolerance;
/// `figures` are recorded, movement, settled, expected, base and difference.
fn gst_side_item(
    label: &str,
    base_label: &str,
    figures: [&Decimal; 6],
    tolerance: Decimal,
) -> Result<ExceptionItem> {
    let [recorded, movement, settled, expected, base, difference] = figures;
    let money = report::money;
    let mut item = ExceptionItem::new(
        Control::GstReasonableness,
        Status::Review,
        None,
        &format!(
            "GST {label} recorded this period is {} ({} of movement in the gst_{label} accounts and {} for the settlements declared against them), against {} expected: 1/10 of {base_label} {}. The difference of {} is beyond the {} review heuristic, which is not a rounding bound. Each side is compared on its own because equal shortfalls in GST collected and GST paid offset in the net figure. The settlements are as declared; the trial balance cannot verify them.",
            money(recorded)?,
            money(movement)?,
            money(settled)?,
            money(expected)?,
            money(base)?,
            money(difference)?,
            money(&tolerance)?,
        ),
        "Trace the difference through that side's GST account postings and the tax codes on its sales or purchases: a sale or purchase coded with the wrong GST treatment, a manual GST journal, a settlement declared against the wrong side, or an account in the wrong GST class. Retain the evidence; this exception adjusts nothing.",
    );
    item.current_value = Some(recorded.clone());
    item.prior_value = Some(expected.clone());
    item.difference = Some(difference.clone());
    item.threshold = Some(tolerance);
    Ok(item)
}

/// The exception for a net GST movement, with the declared settlements added
/// back, further from expectation than the review heuristic: the absolute
/// amount plus the percentage of the expected GST on sales and on purchases.
/// It is not a rounding bound.
fn gst_difference(
    sales: &[Decimal],
    purchases: &[Decimal],
    movement: &[Decimal],
    settled: &[Decimal],
    absolute: &Decimal,
    percentage: &Decimal,
) -> Result<Option<ExceptionItem>> {
    let sales = exact_total(sales, "Current", "taxable sales movement")?;
    let purchases = exact_total(purchases, "Current", "creditable purchases movement")?;
    let movement = exact_total(movement, "Current", "GST account movement")?;
    let settled = exact_total(settled, "Declared", "GST settlements")?;
    let recorded = exact_total([&movement, &settled], "Current", "GST recorded")?;
    let ten = Decimal::parse("10").unwrap();
    let (collected, paid) = (sales.div(&ten), purchases.div(&ten));
    let expected = collected.sub(&paid).value;
    let difference = recorded.sub(&expected).value;
    let gross = collected.abs().add(&paid.abs()).value;
    let tolerance = absolute.add(&percentage.mul(&gross)).value;
    if difference.abs() <= tolerance {
        return Ok(None);
    }
    let money = report::money;
    let mut item = ExceptionItem::new(
        Control::GstReasonableness,
        Status::Review,
        None,
        &format!(
            "Net GST recorded this period is {} ({} of movement in the GST accounts plus {} of declared settlements), against {} expected: GST collected {} (1/10 of taxable sales {}) less GST paid {} (1/10 of creditable purchases {}). The difference of {} is beyond the {} review heuristic, which is not a rounding bound: rounding across many small invoices can exceed it. Amounts are net GST payable; a negative amount is a net refund. The settlements are as declared; the trial balance cannot verify them.",
            money(&recorded)?,
            money(&movement)?,
            money(&settled)?,
            money(&expected)?,
            money(&collected)?,
            money(&sales)?,
            money(&paid)?,
            money(&purchases)?,
            money(&difference)?,
            money(&tolerance)?,
        ),
        "Check the declared settlements against the BAS and the ATO account, then trace the rest through the GST accounts' postings and the tax codes on sales and purchases: rounding across many small invoices, an undeclared BAS payment, refund or clearing journal, a manual GST journal, a sale or purchase coded with the wrong GST treatment, or an account in the wrong GST class. Retain the evidence; this exception adjusts nothing.",
    );
    item.current_value = Some(recorded);
    item.prior_value = Some(expected);
    item.difference = Some(difference);
    item.threshold = Some(tolerance);
    Ok(Some(item))
}

/// The comparison is not made: the mapping cannot tell taxable amounts from the
/// rest, or `settlements` says why no settlements declaration can be used.
fn gst_not_run(
    unclassed: &[&TrialBalanceRow],
    mixed: &[&TrialBalanceRow],
    settlements: Option<&str>,
) -> ExceptionItem {
    let names = |rows: &[&TrialBalanceRow]| {
        rows.iter()
            .map(|r| report::account_name(&r.account_code, &r.account_name, &r.account_id))
            .collect::<Vec<_>>()
            .join("; ")
    };
    let mut reason = Vec::new();
    if !unclassed.is_empty() {
        reason.push(format!(
            "The GST mapping does not class {} account(s) with movement this period, so their taxable amounts cannot be told from GST-free, input-taxed or excluded ones: {}.",
            unclassed.len(),
            names(unclassed)
        ));
    }
    if !mixed.is_empty() {
        reason.push(format!(
            "{} account(s) classed mixed moved this period, so their taxable amounts cannot be separated from the rest: {}.",
            mixed.len(),
            names(mixed)
        ));
    }
    reason.extend(settlements.map(str::to_owned));
    reason.push("The GST reasonableness control did not run, so this pack carries no GST result; this is not a pass.".into());
    ExceptionItem::new(
        Control::GstReasonableness,
        Status::Review,
        None,
        &reason.join(" "),
        "Class each listed account in the GST mapping, move a mixed account's taxable and other amounts into separate accounts, or supply a well-formed GST settlements declaration, as the reason requires, then run the review again.",
    )
}

pub fn overall_status(exceptions: &[ExceptionItem]) -> Status {
    if exceptions.iter().any(|item| item.status == Status::Blocked) {
        Status::Blocked
    } else if exceptions.iter().any(|item| item.status == Status::Review) {
        Status::Review
    } else {
        Status::Pass
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "raises each calculation evidence exception"
)]
pub fn calculation_evidence_exceptions(
    evidence: &[CalculationEvidence],
    unreadable: &[String],
    required: &[String],
    report_date: Date,
) -> Vec<ExceptionItem> {
    let mut result = Vec::new();
    let mut add = |status, reason: String, action: &str| {
        result.push(ExceptionItem::new(
            Control::CalculationEvidence,
            status,
            None,
            &reason,
            action,
        ))
    };
    for message in unreadable {
        add(
            Status::Blocked,
            format!("A supplied calculation-evidence file could not be read: {message}"),
            "Obtain a readable evidence file, or run this close without it and record why the calculation is absent.",
        );
    }
    for name in required {
        if !evidence.iter().any(|e| &e.label == name) {
            add(
                Status::Review,
                format!(
                    "Calculation {} is configured as required for this close and no evidence for it was supplied.",
                    pytext::repr(name)
                ),
                "Produce the calculation and supply its evidence file, or remove the requirement deliberately. It cannot be left out silently.",
            );
        }
    }
    for item in evidence {
        let label = pytext::repr(&item.label);
        for finding in &item.findings {
            add(
                Status::Blocked,
                format!("Calculation evidence {label}: {finding}"),
                "Do not rely on this figure. Re-produce the evidence from the calculator that owns it.",
            );
        }
        if item.status != "COMPUTED" {
            add(
                Status::Review,
                format!(
                    "Calculation evidence {label} records status {}, so no figure was produced.",
                    item.status
                ),
                "Read the recorded reason. A refusal, an outage or a contract failure is a result in itself and must not be treated as a nil amount.",
            );
            continue;
        }
        match covers_period(item, report_date) {
            None => add(
                Status::Review,
                format!(
                    "Calculation evidence {label} carries period {}, which this pipeline cannot read as a period covering {}.",
                    pytext::repr(&item.period),
                    report_date.iso()
                ),
                "Confirm by hand which period the figure covers before relying on it.",
            ),
            Some(false) => add(
                Status::Review,
                format!(
                    "Calculation evidence {label} covers period {}, which does not include the current report date {}.",
                    pytext::repr(&item.period),
                    report_date.iso()
                ),
                "Obtain evidence for the period under review, or record why an earlier period's figure is the right one.",
            ),
            Some(true) => (),
        }
        if item.rate_tables.is_empty() && !item.values.is_empty() {
            add(
                Status::Review,
                format!(
                    "Calculation evidence {label} names no rate table, so the figure's statutory inputs cannot be traced from the pack."
                ),
                "Confirm the figure derives from the supplied inputs alone. If it consumed a rate, obtain evidence that names it.",
            );
        }
    }
    result
}

pub fn review_close(config: &ReviewConfig) -> Result<CloseReviewPack> {
    review_with(config, &mut SourceSnapshot::capture)
}

/// The controls `--require-control` accepts: all but the one that reports a
/// required control missing.
pub fn requirable_controls() -> impl Iterator<Item = Control> {
    Control::ALL
        .iter()
        .copied()
        .filter(|control| *control != Control::Required)
}

/// The named controls, sorted and without repeats, or a refusal: a mistyped
/// name would otherwise require nothing and pass quietly.
fn required_controls_named(names: &[String]) -> Result<Vec<Control>> {
    let mut controls = Vec::new();
    for name in names {
        let Some(control) = requirable_controls().find(|c| c.as_str() == name) else {
            return Err(format!(
                "--require-control {} is not a control. Name one of: {}.",
                pytext::repr(name),
                requirable_controls()
                    .map(Control::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        controls.push(control);
    }
    controls.sort_by_key(|control| control.as_str());
    controls.dedup();
    Ok(controls)
}

/// The reason a required control's exception gives, if its state needs one.
pub fn required_control_reason(name: &str, state: RunState) -> Option<String> {
    let outcome = match state {
        RunState::Ran => return None,
        RunState::Skipped => "was skipped: its input was not supplied",
        RunState::Failed => "failed: its input was supplied but it could not finish",
    };
    Some(format!(
        "Required control {name} {outcome}, so this pack says nothing about it."
    ))
}

/// `REVIEW` for each required control that did not run to its end.
fn required_control_exceptions(required: &[RequiredControlRun]) -> Vec<ExceptionItem> {
    required
        .iter()
        .filter_map(|run| {
            let reason = required_control_reason(run.control.as_str(), run.state)?;
            Some(ExceptionItem::new(
                Control::Required,
                Status::Review,
                None,
                &reason,
                "Supply a usable input and run the review again, or remove the requirement deliberately.",
            ))
        })
        .collect()
}

/// One capture per input keeps parsing and provenance bound to the same bytes.
/// The capture boundary also allows the frozen-oracle probe to supply snapshots.
#[expect(
    clippy::too_many_lines,
    reason = "captures each input once, then reviews"
)]
pub fn review_with(
    config: &ReviewConfig,
    capture: &mut impl FnMut(&Path, &str) -> Result<SourceSnapshot>,
) -> Result<CloseReviewPack> {
    if config.mapping_policy_path.is_some() && config.mapping_path.is_none() {
        return Err("A mapping policy requires a supplied account mapping.".into());
    }
    if config.gst_settlements_path.is_some() && config.gst_mapping_path.is_none() {
        return Err("A GST settlements declaration requires a GST mapping.".into());
    }
    if config.equity_schedule_path.is_none()
        && [
            config.equity_currency.as_deref(),
            config.equity_currency_evidence.as_deref(),
        ]
        .iter()
        .any(|s| s.is_some_and(|s| !s.is_empty()))
    {
        return Err("Equity currency declarations require an equity schedule.".into());
    }
    for (name, value) in [
        ("absolute_threshold", &config.absolute_threshold),
        ("percentage_threshold", &config.percentage_threshold),
        ("reconciliation_tolerance", &config.reconciliation_tolerance),
        ("gst_absolute_tolerance", &config.gst_absolute_tolerance),
        ("gst_percentage_tolerance", &config.gst_percentage_tolerance),
    ] {
        if !value.is_finite() || value < &Decimal::zero() {
            return Err(format!("{name} must be a finite non-negative decimal."));
        }
    }
    if config.percentage_threshold > Decimal::parse("1").unwrap() {
        return Err("percentage_threshold is a fraction from 0 to 1: 0.10 means 10%.".into());
    }
    if config.gst_percentage_tolerance > Decimal::parse("1").unwrap() {
        return Err("gst_percentage_tolerance is a fraction from 0 to 1: 0.01 means 1%.".into());
    }
    let current_source = capture(&config.current_path, "Trial-balance file")?;
    let current_rows = loader::load_canonical_tb(&current_source)?;
    let prior_source = capture(&config.prior_path, "Trial-balance file")?;
    let prior_rows = loader::load_canonical_tb(&prior_source)?;
    let mut optional = |path: &Option<PathBuf>, label: &str| {
        path.as_deref().map(|p| capture(p, label)).transpose()
    };
    let mapping_source = optional(&config.mapping_path, "Mapping file")?;
    let mapping = loader::load_mapping(mapping_source.as_ref())?;
    let mapping_policy_source = optional(&config.mapping_policy_path, "Mapping policy")?;
    let mapping_policy = mapping_policy_source
        .as_ref()
        .map(loader::load_mapping_policy)
        .transpose()?;
    let balance_policy_source = optional(&config.balance_policy_path, "Balance policy")?;
    let balance_policy = balance_policy_source
        .as_ref()
        .map(loader::load_balance_policy)
        .transpose()?;
    let subledger_source = optional(&config.subledger_path, "Subledger file")?;
    let subledger = loader::load_subledger(subledger_source.as_ref())?;
    let gst_mapping_source = optional(&config.gst_mapping_path, "GST mapping")?;
    let gst_mapping = gst_mapping_source
        .as_ref()
        .map(loader::load_gst_mapping)
        .transpose()?;
    let contacts_source = optional(&config.contacts_path, "Contacts file")?;
    let contacts = contacts_source
        .as_ref()
        .map(loader::load_contacts)
        .transpose()?;
    let acknowledgement_source = optional(&config.acknowledgement_path, "Review-note file")?;
    let acknowledgement = loader::load_reviewer_acknowledgement(acknowledgement_source.as_ref())?;
    // An absent or unusable declaration stops only the GST control, which says why.
    let gst_settlements_source = config
        .gst_settlements_path
        .as_deref()
        .map(|p| capture(p, "GST settlements file"));
    let gst_settlements = match &gst_settlements_source {
        None => Err("No GST settlements declaration was supplied, so a BAS payment, refund or clearing journal in the GST accounts cannot be told from GST on sales and purchases.".to_owned()),
        Some(source) => source
            .as_ref()
            .map_err(Clone::clone)
            .and_then(loader::load_gst_settlements)
            .map_err(|error| format!("The GST settlements declaration cannot be used: {error}")),
    };
    let (evidence, unreadable) =
        calculation_evidence::load_all_with(&config.calculation_evidence_paths, capture);
    let current_tenant = &current_rows[0].tenant;
    let current_date = current_rows[0].report_date;
    let prior_date = prior_rows[0].report_date;
    if current_tenant != &prior_rows[0].tenant {
        return Err("Current and prior trial balances must contain the same tenant.".into());
    }
    if prior_date >= current_date {
        return Err(
            "Prior trial-balance ReportDate must be earlier than the current ReportDate.".into(),
        );
    }
    if acknowledgement
        .as_ref()
        .is_some_and(|note| note.reviewed_on < current_date)
    {
        return Err(
            "Review-note reviewed_on cannot be earlier than the current ReportDate.".into(),
        );
    }
    let required_controls = required_controls_named(&config.required_controls)?;
    let current: RowMap = current_rows.iter().map(|r| (r.key(), r.clone())).collect();
    let prior: RowMap = prior_rows.iter().map(|r| (r.key(), r.clone())).collect();
    let mut gst_ran = true;
    let mut exceptions = decimal::with_precision(28, || -> Result<Vec<ExceptionItem>> {
        let mut exceptions = integrity_exceptions(&current_rows, "Current")?;
        exceptions.extend(integrity_exceptions(&prior_rows, "Prior")?);
        exceptions.extend(financial_year_reset_exceptions(current_date, prior_date));
        exceptions.extend(period_comparison_exceptions(
            &current,
            &prior,
            &config.absolute_threshold,
            &config.percentage_threshold,
        ));
        if config.mapping_path.is_some() {
            exceptions.extend(mapping_exceptions(&current_rows, &mapping));
        }
        if let Some(policy) = &mapping_policy {
            exceptions.extend(mapping_compatibility_exceptions(
                &current_rows,
                &mapping,
                policy,
            ));
        }
        if let Some(policy) = &balance_policy {
            exceptions.extend(balance_policy_exceptions(&current, current_tenant, policy));
        }
        exceptions.extend(subledger_exceptions(
            &current,
            &subledger,
            &config.reconciliation_tolerance,
        ));
        if let Some(mapping) = &gst_mapping {
            let (found, ran) = gst_reasonableness_exceptions(
                &current_rows,
                mapping,
                gst_settlements.as_ref().map_err(String::as_str),
                &config.gst_absolute_tolerance,
                &config.gst_percentage_tolerance,
            )?;
            exceptions.extend(found);
            gst_ran = ran;
        }
        if let Some(contacts) = &contacts {
            let mut found = contact_register_exceptions(contacts);
            for item in &mut found {
                item.tenant.clone_from(current_tenant);
            }
            exceptions.extend(found);
        }
        for name in &config.required_calculations {
            if !calculation_evidence::valid_label(name) {
                return Err(format!(
                    "--require-calculation {} is not a slug. A required calculation is named the way its evidence file labels it: lower-case letters, digits and single hyphens.",
                    pytext::repr(name)
                ));
            }
        }
        exceptions.extend(calculation_evidence_exceptions(
            &evidence,
            &unreadable,
            &config.required_calculations,
            current_date,
        ));
        Ok(exceptions)
    })?;
    let equity_source = config
        .equity_schedule_path
        .as_deref()
        .map(|p| capture(p, "Equity schedule"))
        .transpose()?;
    let equity_result = if let Some(source) = &equity_source {
        let (result, findings) = equity::reconcile(
            source,
            equity::Inputs {
                prior: &prior_rows,
                current: &current_rows,
                prior_sha256: &prior_source.sha256,
                current_sha256: &current_source.sha256,
                currency: config.equity_currency.as_deref(),
                currency_evidence: config.equity_currency_evidence.as_deref(),
                tolerance: &config.reconciliation_tolerance,
            },
        )?;
        exceptions.extend(findings);
        Some(result)
    } else {
        None
    };
    // Runs only when required: it lists accounts no input ties out, which
    // would otherwise change every pack that leaves one out.
    if required_controls.contains(&Control::ReconciliationCoverage) {
        exceptions.extend(decimal::with_precision(28, || {
            reconciliation_coverage_exceptions(&current_rows, &subledger, equity_result.as_ref())
        }));
    }
    let supplied = |present| {
        if present {
            RunState::Ran
        } else {
            RunState::Skipped
        }
    };
    // No wildcard, so a new control must say how it can fail to run.
    let run_state = |control| match control {
        Control::TrialBalanceIntegrity
        | Control::FinancialYearReset
        | Control::PeriodComparison
        | Control::PeriodVariance
        | Control::AccountMetadata
        | Control::ReconciliationCoverage
        | Control::Required => RunState::Ran,
        Control::AccountMapping if config.mapping_path.is_some() && mapping.is_empty() => {
            RunState::Failed
        }
        Control::AccountMapping => supplied(config.mapping_path.is_some()),
        Control::MappingCompatibility => supplied(mapping_policy.is_some()),
        Control::BalancePolicy => supplied(balance_policy.is_some()),
        Control::SubledgerReconciliation
            if config.subledger_path.is_some() && subledger.is_empty() =>
        {
            RunState::Failed
        }
        Control::SubledgerReconciliation => supplied(!subledger.is_empty()),
        Control::GstReasonableness if gst_mapping.is_some() && !gst_ran => RunState::Failed,
        Control::GstReasonableness => supplied(gst_mapping.is_some()),
        Control::CalculationEvidence if !unreadable.is_empty() => RunState::Failed,
        Control::CalculationEvidence => supplied(!evidence.is_empty()),
        Control::EquityReconciliation => supplied(equity_source.is_some()),
        Control::ContactRegister if contacts.as_ref().is_some_and(Vec::is_empty) => {
            RunState::Failed
        }
        Control::ContactRegister => supplied(contacts.is_some()),
    };
    let required_controls: Vec<_> = required_controls
        .into_iter()
        .map(|control| RequiredControlRun {
            control,
            state: run_state(control),
        })
        .collect();
    exceptions.extend(required_control_exceptions(&required_controls));
    for item in &mut exceptions {
        item.review_group = mapping.get(&item.account_id).cloned().unwrap_or_default();
    }
    let mut source_hashes = BTreeMap::from([
        ("current_trial_balance".into(), current_source.sha256),
        ("prior_trial_balance".into(), prior_source.sha256),
    ]);
    for (role, source) in [
        ("account_mapping", mapping_source),
        ("mapping_policy", mapping_policy_source),
        ("balance_policy", balance_policy_source),
        ("subledger", subledger_source),
        ("gst_mapping", gst_mapping_source),
        (
            "gst_settlements",
            gst_settlements_source.and_then(Result::ok),
        ),
        ("review_note", acknowledgement_source),
        ("equity_schedule", equity_source),
        ("contacts", contacts_source),
    ] {
        if let Some(source) = source {
            source_hashes.insert(role.into(), source.sha256);
        }
    }
    for item in &evidence {
        source_hashes.insert(
            format!("calculation_evidence:{}", item.label),
            item.sha256.clone(),
        );
    }
    let controls_not_run = [
        ("account_mapping", config.mapping_path.is_some()),
        ("balance_policy", balance_policy.is_some()),
        ("subledger", !subledger.is_empty()),
        // Tracked only once a GST mapping is supplied, so packs without one are unchanged.
        ("gst_reasonableness", gst_ran),
        // Named only when a supplied contacts file held no contacts.
        (
            "contact_register",
            contacts.as_ref().is_none_or(|c| !c.is_empty()),
        ),
        (
            "calculation_evidence",
            !evidence.is_empty()
                || !unreadable.is_empty()
                || !config.required_calculations.is_empty(),
        ),
    ]
    .into_iter()
    .filter(|(_, ran)| !ran)
    .map(|(s, _)| s.into())
    .collect();
    exceptions.sort_by(|a, b| {
        (
            a.status != Status::Blocked,
            &a.control,
            &a.tenant,
            &a.account_id,
            &a.reason,
        )
            .cmp(&(
                b.status != Status::Blocked,
                &b.control,
                &b.tenant,
                &b.account_id,
                &b.reason,
            ))
    });
    queries::derive_client_queries(&exceptions)?;
    let relied_on = evidence
        .iter()
        .filter(|item| {
            item.usable()
                && covers_period(item, current_date) == Some(true)
                && (!item.rate_tables.is_empty() || item.values.is_empty())
        })
        .map(|e| e.label.clone())
        .collect();
    Ok(CloseReviewPack {
        status: overall_status(&exceptions),
        current_report_dates: vec![current_date.iso()],
        prior_report_dates: vec![prior_date.iso()],
        source_hashes,
        absolute_threshold: config.absolute_threshold.clone(),
        percentage_threshold: config.percentage_threshold.clone(),
        reconciliation_tolerance: config.reconciliation_tolerance.clone(),
        gst_tolerance: gst_mapping.as_ref().map(|_| {
            (
                config.gst_absolute_tolerance.clone(),
                config.gst_percentage_tolerance.clone(),
            )
        }),
        exceptions,
        acknowledgement,
        calculation_evidence: evidence,
        required_calculations: config.required_calculations.clone(),
        required_controls,
        controls_not_run,
        relied_on,
        equity_reconciliation: equity_result,
        producer: None,
    })
}
