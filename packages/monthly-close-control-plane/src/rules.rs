// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! The `rules list` catalogue: each review control's id, when it runs, what it
//! checks and what it rests on. The match is exhaustive, so a new control does
//! not compile until it is described here.

use crate::models::Control;

/// When the control runs, what it checks, and the source of its rule.
#[expect(
    clippy::too_many_lines,
    reason = "one exhaustive match arm per control"
)]
pub fn describe(control: Control) -> (&'static str, &'static str, &'static str) {
    match control {
        Control::TrialBalanceIntegrity => (
            "every review",
            "the current and prior files' Debit and Credit totals, and YTDDebit and YTDCredit totals, agree; BLOCKED otherwise",
            "double-entry: debits equal credits (README 'Controls')",
        ),
        Control::FinancialYearReset => (
            "every review",
            "the current and prior ReportDate fall in the same Australian financial year",
            "the Australian financial year, 1 July to 30 June (README 'Controls')",
        ),
        Control::PeriodComparison => (
            "every review",
            "each account, keyed by Tenant and AccountID, is present in both periods",
            "this tool's review contract (README 'Controls')",
        ),
        Control::PeriodVariance => (
            "every review",
            "no account's YTD net balance moves by both --absolute-threshold and --percentage-threshold",
            "the firm's thresholds, defaults 1000 and 0.10 (README 'Controls')",
        ),
        Control::AccountMetadata => (
            "every review",
            "an account in both periods keeps its AccountCode, AccountName and Section",
            "this tool's review contract (README 'Controls')",
        ),
        Control::AccountMapping => (
            "with --mapping",
            "every current-period account has a row in the mapping file",
            "the firm's mapping file (README 'Optional mapping and reconciliation inputs')",
        ),
        Control::MappingCompatibility => (
            "with --mapping-policy",
            "each mapped account's Section is permitted for its ReviewGroup",
            "the firm's mapping policy (README 'Mapping compatibility policy')",
        ),
        Control::BalancePolicy => (
            "with --balance-policy",
            "each listed account sits on its declared side, is nil when declared nil, and moves when it should",
            "the firm's balance policy (README 'Balance policy')",
        ),
        Control::SubledgerReconciliation => (
            "with --subledger",
            "each YTD net balance agrees with its SubledgerBalance within --reconciliation-tolerance",
            "the supplied subledger balances (README 'Optional mapping and reconciliation inputs')",
        ),
        Control::GstReasonableness => (
            "with --gst-mapping and --gst-settlements",
            "net GST movement, settlements added back, is near 1/10 of taxable sales less 1/10 of creditable purchases",
            "the ATO's GST pages cited, with the date checked, in README 'GST reasonableness'",
        ),
        Control::CalculationEvidence => (
            "with --calculation-evidence or --require-calculation",
            "each evidence file is readable, self-consistent, covers the period and carries every required label",
            "the supplied evidence files (README 'Calculation evidence, optional')",
        ),
        Control::EquityReconciliation => (
            "with --equity-schedule",
            "each selected equity account closes at opening plus its scheduled movements",
            "the supplied equity schedule (docs/equity-reconciliation.md)",
        ),
        Control::ReconciliationCoverage => (
            "with --require-control reconciliation_coverage",
            "every balance-sheet account with a closing balance has a subledger row or a computed equity reconciliation",
            "this tool's review contract (README 'Requiring a control')",
        ),
        Control::ContactRegister => (
            "with --contacts",
            "each ABN has 11 digits and passes the ABN check digits, and no ABN or name repeats",
            "the ABN check-digit algorithm; nothing is looked up (README 'Optional mapping and reconciliation inputs')",
        ),
        Control::Required => (
            "with --require-control",
            "each named control ran and passed",
            "the firm's --require-control list (README 'Requiring a control')",
        ),
    }
}

/// Every control in the order a pack reports them, one block each.
pub fn render() -> String {
    let mut out = String::new();
    for &control in Control::ALL {
        let (runs, checks, source) = describe(control);
        out.push_str(&format!(
            "{}\n  runs:   {runs}\n  checks: {checks}\n  source: {source}\n",
            control.as_str()
        ));
    }
    out.push_str(
        "\nThese are review rules: each raises an exception for a reviewer and none approves a close.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_lists_every_control_once_in_order() {
        let text = render();
        let ids: Vec<&str> = text
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with(' ') && !line.starts_with("These"))
            .collect();
        let expected: Vec<&str> = Control::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(ids, expected);
        for &control in Control::ALL {
            let (runs, checks, source) = describe(control);
            assert!(!runs.is_empty() && !checks.is_empty() && !source.is_empty());
        }
    }
}
