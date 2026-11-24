// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Deterministic flux anomaly checks for sudden-zero recurring costs and normal-balance sign flips.

use crate::{
    Result,
    decimal::{self, Decimal},
    models::{Status, TrialBalanceRow},
    pyjson::Json,
    pytext, reconciliation,
};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "flux-checks-v1";
pub const BOUNDARY: &str = "Flux checks highlight review candidates only; they do not approve a close or post an adjustment.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FluxKind {
    SuddenZeroRecurringCost,
    NormalBalanceSignFlip,
}

impl FluxKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SuddenZeroRecurringCost => "sudden_zero_recurring_cost",
            Self::NormalBalanceSignFlip => "normal_balance_sign_flip",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalSign {
    Debit,
    Credit,
    Either,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FluxPolicy {
    pub min_recurring_expense: Decimal,
    pub min_recurring_periods: usize,
    pub min_sign_flip_amount: Decimal,
    pub prior_active_periods: BTreeMap<(String, String), usize>,
    pub seasonal_or_one_off: BTreeSet<(String, String)>,
    pub prepaid_links: BTreeMap<(String, String), String>,
    pub reclassified_to: BTreeMap<(String, String), String>,
    pub sign_overrides: BTreeMap<(String, String), NormalSign>,
}

impl Default for FluxPolicy {
    fn default() -> Self {
        Self {
            min_recurring_expense: Decimal::parse("100.00").unwrap(),
            min_recurring_periods: 2,
            min_sign_flip_amount: Decimal::parse("1.00").unwrap(),
            prior_active_periods: BTreeMap::new(),
            seasonal_or_one_off: BTreeSet::new(),
            prepaid_links: BTreeMap::new(),
            reclassified_to: BTreeMap::new(),
            sign_overrides: BTreeMap::new(),
        }
    }
}

impl FluxPolicy {
    pub fn validate(&self) -> Result<()> {
        let zero = Decimal::zero();
        if !self.min_recurring_expense.is_finite() || self.min_recurring_expense < zero {
            return Err("min_recurring_expense must be finite and non-negative.".into());
        }
        if self.min_recurring_periods == 0 {
            return Err("min_recurring_periods must be at least 1.".into());
        }
        if !self.min_sign_flip_amount.is_finite() || self.min_sign_flip_amount < zero {
            return Err("min_sign_flip_amount must be finite and non-negative.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FluxFinding {
    pub kind: FluxKind,
    pub status: Status,
    pub tenant: String,
    pub account_id: String,
    pub account_code: String,
    pub account_name: String,
    pub section: String,
    pub prior_value: Decimal,
    pub current_value: Decimal,
    pub reason: String,
    pub reviewer_action: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FluxReport {
    pub status: Status,
    pub findings: Vec<FluxFinding>,
}

fn finding_to_json(item: &FluxFinding) -> Json {
    Json::object([
        ("kind", Json::str(item.kind.as_str())),
        ("status", Json::str(item.status.as_str())),
        ("tenant", Json::str(&item.tenant)),
        ("account_id", Json::str(&item.account_id)),
        ("account_code", Json::str(&item.account_code)),
        ("account_name", Json::str(&item.account_name)),
        ("section", Json::str(&item.section)),
        (
            "prior_value",
            Json::str(&reconciliation::fixed(&item.prior_value)),
        ),
        (
            "current_value",
            Json::str(&reconciliation::fixed(&item.current_value)),
        ),
        ("reason", Json::str(&item.reason)),
        ("reviewer_action", Json::str(&item.reviewer_action)),
    ])
}

impl FluxReport {
    pub fn to_json(&self) -> Json {
        Json::object([
            ("schema", Json::str(SCHEMA)),
            ("status", Json::str(self.status.as_str())),
            ("boundary", Json::str(BOUNDARY)),
            (
                "findings",
                Json::Array(self.findings.iter().map(finding_to_json).collect()),
            ),
        ])
    }
}

fn row_map<'a>(
    rows: &'a [TrialBalanceRow],
    label: &str,
) -> Result<BTreeMap<(String, String), &'a TrialBalanceRow>> {
    let mut map = BTreeMap::new();
    for row in rows {
        if map.insert(row.key(), row).is_some() {
            return Err(format!(
                "{label} trial balance contains duplicate account '{}' for tenant '{}'.",
                row.account_id, row.tenant
            ));
        }
    }
    Ok(map)
}

fn is_recurring(prior: &TrialBalanceRow, prior_charge: &Decimal, policy: &FluxPolicy) -> bool {
    if let Some(&periods) = policy.prior_active_periods.get(&prior.key()) {
        return periods >= policy.min_recurring_periods;
    }
    prior.ytd_net() > *prior_charge
}

fn prepaid_unwind_state(
    key: &(String, String),
    current: &BTreeMap<(String, String), &TrialBalanceRow>,
    prior: &BTreeMap<(String, String), &TrialBalanceRow>,
    policy: &FluxPolicy,
) -> Option<bool> {
    let prepaid_id = policy.prepaid_links.get(key)?;
    let prepaid_key = (key.0.clone(), prepaid_id.clone());
    let prior_prepaid = prior.get(&prepaid_key)?.ytd_net();
    let current_prepaid = current.get(&prepaid_key)?.ytd_net();
    Some(current_prepaid < prior_prepaid)
}

fn is_reclassified(
    key: &(String, String),
    current: &BTreeMap<(String, String), &TrialBalanceRow>,
    policy: &FluxPolicy,
) -> bool {
    policy
        .reclassified_to
        .get(key)
        .and_then(|target| current.get(&(key.0.clone(), target.clone())))
        .is_some_and(|row| row.current_net() > Decimal::zero())
}

fn make_finding(
    kind: FluxKind,
    row: &TrialBalanceRow,
    prior_value: Decimal,
    current_value: Decimal,
    reason: String,
    action: &str,
) -> FluxFinding {
    FluxFinding {
        kind,
        status: Status::Review,
        tenant: row.tenant.clone(),
        account_id: row.account_id.clone(),
        account_code: row.account_code.clone(),
        account_name: row.account_name.clone(),
        section: row.section.clone(),
        prior_value,
        current_value,
        reason,
        reviewer_action: action.into(),
    }
}

fn check_sudden_zero(
    current: &BTreeMap<(String, String), &TrialBalanceRow>,
    prior: &BTreeMap<(String, String), &TrialBalanceRow>,
    policy: &FluxPolicy,
) -> Vec<FluxFinding> {
    let mut findings = Vec::new();
    for (key, current_row) in current {
        if pytext::casefold(&current_row.section) != "expenses" {
            continue;
        }
        let Some(prior_row) = prior.get(key) else {
            continue;
        };
        let prior_charge = prior_row.current_net();
        let current_charge = current_row.current_net();
        if prior_charge < policy.min_recurring_expense
            || !current_charge.is_zero()
            || policy.seasonal_or_one_off.contains(key)
            || !is_recurring(prior_row, &prior_charge, policy)
            || is_reclassified(key, current, policy)
        {
            continue;
        }
        let unwind = prepaid_unwind_state(key, current, prior, policy);
        if unwind == Some(true) {
            continue;
        }
        let suffix = if unwind == Some(false) {
            " whilst its linked prepaid balance did not unwind."
        } else {
            "."
        };
        let reason = format!(
            "Recurring expense dropped from {} in the prior period to 0.00{suffix}",
            reconciliation::fixed(&prior_charge)
        );
        findings.push(make_finding(
            FluxKind::SuddenZeroRecurringCost,
            current_row,
            prior_charge,
            current_charge,
            reason,
            "Confirm whether an accrual, supplier invoice, or prepayment amortisation was missed, or record the cost as ended.",
        ));
    }
    findings
}

fn normal_sign(row: &TrialBalanceRow, policy: &FluxPolicy) -> NormalSign {
    if let Some(&override_sign) = policy.sign_overrides.get(&row.key()) {
        return override_sign;
    }
    match pytext::casefold(&row.section).as_str() {
        "assets" | "expenses" => NormalSign::Debit,
        "liabilities" | "equity" | "revenue" => NormalSign::Credit,
        _ => NormalSign::Either,
    }
}

fn check_sign_flips(
    current: &BTreeMap<(String, String), &TrialBalanceRow>,
    prior: &BTreeMap<(String, String), &TrialBalanceRow>,
    policy: &FluxPolicy,
) -> Vec<FluxFinding> {
    let zero = Decimal::zero();
    let mut findings = Vec::new();
    for (key, row) in current {
        let closing = row.ytd_net();
        if closing.is_zero() || closing.copy_abs() < policy.min_sign_flip_amount {
            continue;
        }
        let expected = normal_sign(row, policy);
        let flipped = match expected {
            NormalSign::Debit => closing < zero,
            NormalSign::Credit => closing > zero,
            NormalSign::Either => false,
        };
        if !flipped {
            continue;
        }
        let prior_value = prior.get(key).map_or_else(Decimal::zero, |p| p.ytd_net());
        let expected_label = if expected == NormalSign::Debit {
            "debit"
        } else {
            "credit"
        };
        let reason = format!(
            "Closing balance {} sits opposite the expected {expected_label} normal sign for {}.",
            reconciliation::fixed(&closing),
            row.section
        );
        findings.push(make_finding(
            FluxKind::NormalBalanceSignFlip,
            row,
            prior_value,
            closing,
            reason,
            "Investigate mispostings, unallocated credits, or reversed accruals, or document a legitimate contra balance.",
        ));
    }
    findings
}

pub fn evaluate_flux(
    current: &[TrialBalanceRow],
    prior: &[TrialBalanceRow],
    policy: &FluxPolicy,
) -> Result<FluxReport> {
    policy.validate()?;
    decimal::with_precision(40, || {
        let current_map = row_map(current, "Current")?;
        let prior_map = row_map(prior, "Prior")?;
        let mut findings = check_sudden_zero(&current_map, &prior_map, policy);
        findings.extend(check_sign_flips(&current_map, &prior_map, policy));
        findings.sort_by(|a, b| {
            a.kind
                .cmp(&b.kind)
                .then_with(|| a.tenant.cmp(&b.tenant))
                .then_with(|| a.account_id.cmp(&b.account_id))
        });
        let status = if findings.is_empty() {
            Status::Pass
        } else {
            Status::Review
        };
        Ok(FluxReport { status, findings })
    })
}
