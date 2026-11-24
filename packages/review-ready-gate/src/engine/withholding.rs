// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! PAYG withholding and the activity-statement labels outside GST: the
//! statement's own arithmetic, and the PAYG withholding control file tied to
//! the statement and rolled forward to the trial balance.

use super::{BAS_PRECISION, Loaded, exact_sub, exact_sum, first_three};
use crate::{Result, decimal::Decimal, isodate::Date, models::*};
use std::{cmp::Ordering, collections::BTreeSet};

#[cfg(test)]
mod tests;

const SLOT: &str = "paygw_control_gl";
/// The labels the ATO forbids negative figures at, of those this module reads.
const NON_NEGATIVE: [&str; 10] = ["W1", "W2", "W3", "W4", "W5", "4", "1C", "1D", "7C", "7D"];
/// The labels W5 totals, in the ATO's order.
const WITHHELD: [&str; 3] = ["W2", "W4", "W3"];
/// The activity-statement label that declares what the PAYG withholding
/// accounts carry in from before the period, signed like `GST_OPENING`.
const OPENING: &str = "PAYGW_OPENING";

fn label<'a>(statement: &'a [ActivityLabel], name: &str) -> Option<&'a Decimal> {
    statement
        .iter()
        .find(|r| r.label == name)
        .map(|r| &r.amount)
}

fn statement_break(reason: String) -> Finding {
    Finding::new(
        "TIEOUT_BREAK",
        "NOT_READY",
        "activity_statement",
        reason,
        "Return the pack. Correct the activity statement before review.",
    )
}

/// Runs every check in this module on what the pack supplied.
pub(super) fn apply(
    loaded: &Loaded,
    findings: &mut Vec<Finding>,
    tolerance: &Decimal,
) -> Result<()> {
    if let Some(statement) = &loaded.activity_statement {
        apply_statement(statement, findings)?;
        if let Some(control) = &loaded.paygw_control_gl {
            apply_tie(statement, control, findings, tolerance)?;
        }
    }
    apply_roll_forward(loaded, findings, tolerance)?;
    apply_period(loaded, findings);
    Ok(())
}

/// The statement's own arithmetic: no negative figures, W5 is W2 + W4 + W3,
/// 4 is W5, and W2 is withheld from the payments at W1.
fn apply_statement(statement: &[ActivityLabel], findings: &mut Vec<Finding>) -> Result<()> {
    for name in NON_NEGATIVE {
        if let Some(amount) = label(statement, name).filter(|a| a.is_signed() && !a.is_zero()) {
            findings.push(statement_break(format!(
                "Activity statement label {name} is negative ({}); the ATO accepts no negative figures.",
                amount.to_sci_string()
            )));
        }
    }
    let zero = Decimal::zero();
    let withheld = exact_sum(
        WITHHELD
            .iter()
            .map(|name| Ok(label(statement, name).unwrap_or(&zero).clone())),
        BAS_PRECISION,
    )?;
    let w5 = label(statement, "W5");
    if let Some(w5) = w5
        && *w5 != withheld
    {
        findings.push(statement_break(format!(
            "Activity statement W5 ({}) is not W2 + W4 + W3 ({}).",
            w5.to_sci_string(),
            withheld.to_sci_string()
        )));
    }
    if let (Some(four), Some(w5)) = (label(statement, "4"), w5)
        && four != w5
    {
        findings.push(statement_break(format!(
            "Activity statement label 4 ({}) is not W5 ({}); the W5 total is copied to 4.",
            four.to_sci_string(),
            w5.to_sci_string()
        )));
    }
    // A payer reporting through Single Touch Payroll leaves W1 out.
    if let (Some(w2), Some(w1)) = (label(statement, "W2"), label(statement, "W1"))
        && w2 > w1
    {
        findings.push(statement_break(format!(
            "Activity statement W2 ({}) exceeds W1 ({}); W2 is withheld from the payments shown at W1.",
            w2.to_sci_string(),
            w1.to_sci_string()
        )));
    }
    Ok(())
}

fn untied() -> Finding {
    Finding::new(
        "TIEOUT_BREAK",
        "NOT_READY",
        SLOT,
        "Activity statement has none of W2, W3, W4 or W5, so the PAYG withholding control file cannot be tied.",
        "Return the pack. Report the amounts withheld, or leave the PAYG withholding control file out for a large withholder that reports W1 only.",
    )
}

/// The withholding the statement reports against the control file's net
/// credits for the period. Whole-dollar labels lose their cents, each rounded
/// down, so a statement lower than the ledger by under $1 for each label it
/// reports also ties.
fn apply_tie(
    statement: &[ActivityLabel],
    control: &[GstControlRow],
    findings: &mut Vec<Finding>,
    tolerance: &Decimal,
) -> Result<()> {
    let reported: Vec<&Decimal> = WITHHELD
        .iter()
        .filter_map(|name| label(statement, name))
        .collect();
    let (withheld, source) = match label(statement, "W5") {
        Some(w5) => (w5.clone(), "W5"),
        None if !reported.is_empty() => (
            exact_sum(reported.iter().map(|a| Ok((*a).clone())), BAS_PRECISION)?,
            "W2 + W4 + W3",
        ),
        None => {
            findings.push(untied());
            return Ok(());
        }
    };
    let movement = movement(control)?;
    let difference = exact_sub(&withheld, &movement, BAS_PRECISION)?;
    // W5 alone stands for W2, W4 and W3, each rounded down.
    let labels = if reported.is_empty() {
        WITHHELD.len()
    } else {
        reported.len()
    };
    let whole = withheld.is_integral() && reported.iter().all(|a| a.is_integral());
    let allowance = Decimal::parse(&labels.to_string()).unwrap();
    let rounded = whole && difference.is_signed() && difference.abs() < allowance;
    if difference.abs() > *tolerance && !rounded {
        let note = if whole {
            format!(
                " or the allowance of under $1 for each of the {labels} whole-dollar withholding labels"
            )
        } else {
            String::new()
        };
        let figures = [&withheld, &movement, &difference, tolerance].map(Decimal::to_sci_string);
        findings.push(Finding::new(
            "TIEOUT_BREAK",
            "NOT_READY",
            SLOT,
            format!(
                "PAYG withholding on the activity statement ({source} {}) does not tie to PAYG withholding control movement ({}); difference {} exceeds tolerance {}{note}.{}",
                figures[0],
                figures[1],
                figures[2],
                figures[3],
                super::difference_hint(&difference, control)
            ),
            "Return the pack. Complete the PAYG withholding tie-out before review.",
        ));
    }
    Ok(())
}

/// The control file's net movement, credits positive.
fn movement(control: &[GstControlRow]) -> Result<Decimal> {
    exact_sum(
        control
            .iter()
            .map(|r| exact_sub(&r.credit, &r.debit, BAS_PRECISION)),
        BAS_PRECISION,
    )
}

fn accounts_missing(accounts: &BTreeSet<&str>, tb: &[TrialBalanceRow]) -> Option<Finding> {
    let held: BTreeSet<_> = tb.iter().map(|r| r.account_id.as_str()).collect();
    let missing: Vec<String> = accounts
        .difference(&held)
        .map(|a| (*a).to_owned())
        .collect();
    (!missing.is_empty()).then(|| Finding::new(
        "TIEOUT_BREAK",
        "NOT_READY",
        SLOT,
        format!(
            "PAYG withholding control file accounts are not in the trial balance: {}. The control accounts cannot be rolled forward to the ledger.",
            first_three(&missing)
        ),
        "Return the pack. Export the trial balance and the PAYG withholding control file from the same ledger.",
    ))
}

/// Accounts in both control files, whose closing balance neither file's
/// roll-forward can explain alone.
fn shared(accounts: &BTreeSet<&str>, loaded: &Loaded) -> Option<Finding> {
    let gst: BTreeSet<_> = loaded
        .gst_control_gl
        .iter()
        .flatten()
        .map(|r| r.account_id.as_str())
        .collect();
    let both: Vec<String> = accounts
        .intersection(&gst)
        .map(|a| (*a).to_owned())
        .collect();
    (!both.is_empty()).then(|| Finding::new(
        "TIEOUT_BREAK",
        "NOT_READY",
        SLOT,
        format!(
            "Accounts are in both the GST and the PAYG withholding control files: {}. Neither roll-forward explains their closing balance alone.",
            first_three(&both)
        ),
        "Return the pack. Keep GST and PAYG withholding in separate control accounts, or reconcile the shared account by hand.",
    ))
}

/// The control accounts' closing balance in the trial balance against the
/// balance they carry in, the statement's `PAYGW_OPENING` or nil, plus the
/// period's movement.
fn apply_roll_forward(
    loaded: &Loaded,
    findings: &mut Vec<Finding>,
    tolerance: &Decimal,
) -> Result<()> {
    let (Some(control), Some(tb)) = (&loaded.paygw_control_gl, &loaded.trial_balance) else {
        return Ok(());
    };
    let accounts: BTreeSet<_> = control.iter().map(|r| r.account_id.as_str()).collect();
    if let Some(finding) = accounts_missing(&accounts, tb).or_else(|| shared(&accounts, loaded)) {
        findings.push(finding);
        return Ok(());
    }
    let closing = exact_sum(
        tb.iter()
            .filter(|r| accounts.contains(r.account_id.as_str()))
            .map(|r| exact_sub(&r.ytd_credit, &r.ytd_debit, BAS_PRECISION)),
        BAS_PRECISION,
    )?;
    let opening = loaded
        .activity_statement
        .as_deref()
        .and_then(|statement| label(statement, OPENING));
    let movement = movement(control)?;
    let carried = match opening {
        Some(opening) => exact_sub(&closing, opening, BAS_PRECISION)?,
        None => closing.clone(),
    };
    let difference = exact_sub(&carried, &movement, BAS_PRECISION)?;
    if difference.abs() > *tolerance {
        let carried_in = opening.map_or_else(
            || "nil, as the activity statement declares no PAYGW_OPENING".to_owned(),
            |o| {
                format!(
                    "{}, the activity statement's PAYGW_OPENING",
                    o.to_sci_string()
                )
            },
        );
        findings.push(Finding::new(
            "TIEOUT_BREAK",
            "NOT_READY",
            SLOT,
            format!(
                "PAYG withholding control accounts close at {} in the trial balance, carry in {carried_in}, and the control file moves them by {}; difference {} exceeds tolerance {}. The file may be missing postings, or the accounts carried in a balance, such as an earlier period still unpaid, which PAYGW_OPENING declares.{}",
                closing.to_sci_string(),
                movement.to_sci_string(),
                difference.to_sci_string(),
                tolerance.to_sci_string(),
                super::difference_hint(&difference, control)
            ),
            "Return the pack. Complete the PAYG withholding control file, or declare the balance carried in as PAYGW_OPENING on the activity statement, before review.",
        ));
    }
    Ok(())
}

/// Control postings dated before a declared `period_start` or after `period_end`.
fn apply_period(loaded: &Loaded, findings: &mut Vec<Finding>) {
    let (Some(s), Some(control)) = (&loaded.self_review, &loaded.paygw_control_gl) else {
        return;
    };
    let bounds = [
        s.period_start
            .map(|d| (Ordering::Less, d, "before", "period_start")),
        Some((Ordering::Greater, s.period_end, "after", "period_end")),
    ];
    for (side, bound, word, key) in bounds.into_iter().flatten() {
        let dates: BTreeSet<Date> = control
            .iter()
            .map(|r| r.posted_on)
            .filter(|d| d.cmp(&bound) == side)
            .collect();
        if dates.is_empty() {
            continue;
        }
        let dates: Vec<_> = dates.iter().map(Date::iso).collect();
        findings.push(Finding::new(
            "PERIOD_ORDER",
            "NOT_READY",
            SLOT,
            format!(
                "PAYG withholding control postings are dated {word} the declared {key} {}: {}.",
                bound.iso(),
                first_three(&dates)
            ),
            "Return the pack. The PAYG withholding control export must be for the period the self-review declares.",
        ));
    }
}
