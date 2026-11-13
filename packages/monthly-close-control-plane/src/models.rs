// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use crate::{decimal::Decimal, isodate::Date};

#[derive(Clone, Debug)]
pub struct TrialBalanceRow {
    pub report_date: Date,
    pub tenant: String,
    pub section: String,
    pub account_id: String,
    pub account_name: String,
    pub account_code: String,
    pub debit: Decimal,
    pub credit: Decimal,
    pub ytd_debit: Decimal,
    pub ytd_credit: Decimal,
}

impl TrialBalanceRow {
    pub fn key(&self) -> (String, String) {
        (self.tenant.clone(), self.account_id.clone())
    }

    pub fn current_net(&self) -> Decimal {
        self.debit.sub(&self.credit).value
    }

    pub fn ytd_net(&self) -> Decimal {
        self.ytd_debit.sub(&self.ytd_credit).value
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pass,
    Review,
    Blocked,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Review => "REVIEW",
            Self::Blocked => "BLOCKED",
        }
    }
}

macro_rules! controls {
    ($($variant:ident => $id:literal,)+) => {
        /// Every control the review engine raises, in the order it runs them.
        /// `ExceptionItem::new` accepts nothing else, and a test holds the
        /// README's Controls table to `Control::ALL`.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Control {
            $($variant,)+
        }

        impl Control {
            pub const ALL: &[Self] = &[$(Self::$variant,)+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $id,)+
                }
            }
        }
    };
}

controls! {
    TrialBalanceIntegrity => "trial_balance_integrity",
    FinancialYearReset => "financial_year_reset",
    PeriodComparison => "period_comparison",
    PeriodVariance => "period_variance",
    AccountMetadata => "account_metadata",
    AccountMapping => "account_mapping",
    MappingCompatibility => "mapping_compatibility",
    BalancePolicy => "balance_policy",
    SubledgerReconciliation => "subledger_reconciliation",
    GstReasonableness => "gst_reasonableness",
    CalculationEvidence => "calculation_evidence",
    EquityReconciliation => "equity_reconciliation",
    ReconciliationCoverage => "reconciliation_coverage",
    Required => "required_control",
}

/// How a GST mapping classes an account for `gst_reasonableness`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GstClass {
    TaxableSales,
    CreditablePurchases,
    /// Purchases whose GST cannot be claimed: no GST paid is expected.
    NonCreditable,
    GstFree,
    InputTaxed,
    BasExcluded,
    Mixed,
    Gst,
    /// An account that records only GST collected, compared with sales on its own.
    GstCollected,
    /// An account that records only GST paid, compared with purchases on its own.
    GstPaid,
}

/// A GST settlements declaration: each settlement's net debit to the GST
/// accounts and, when the declaration has a `Side` column, the side it was
/// posted to (`GstCollected` or `GstPaid`).
#[derive(Clone, Debug)]
pub struct GstSettlements {
    pub sided: bool,
    pub rows: Vec<(Option<GstClass>, Decimal)>,
}

impl GstClass {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "taxable_sales" => Self::TaxableSales,
            "creditable_purchases" => Self::CreditablePurchases,
            "non_creditable" => Self::NonCreditable,
            "gst_free" => Self::GstFree,
            "input_taxed" => Self::InputTaxed,
            "bas_excluded" => Self::BasExcluded,
            "mixed" => Self::Mixed,
            "gst" => Self::Gst,
            "gst_collected" => Self::GstCollected,
            "gst_paid" => Self::GstPaid,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct ExceptionItem {
    pub control: String,
    pub status: Status,
    pub tenant: String,
    pub account_id: String,
    pub account_code: String,
    pub account_name: String,
    pub current_value: Option<Decimal>,
    pub prior_value: Option<Decimal>,
    pub difference: Option<Decimal>,
    pub threshold: Option<Decimal>,
    pub percentage_change: Option<Decimal>,
    pub reason: String,
    pub reviewer_action: String,
    pub review_group: String,
}

impl ExceptionItem {
    pub fn new(
        control: Control,
        status: Status,
        row: Option<&TrialBalanceRow>,
        reason: &str,
        action: &str,
    ) -> Self {
        Self {
            control: control.as_str().into(),
            status,
            tenant: row.map_or_else(String::new, |r| r.tenant.clone()),
            account_id: row.map_or_else(String::new, |r| r.account_id.clone()),
            account_code: row.map_or_else(String::new, |r| r.account_code.clone()),
            account_name: row.map_or_else(String::new, |r| r.account_name.clone()),
            current_value: None,
            prior_value: None,
            difference: None,
            threshold: None,
            percentage_change: None,
            reason: reason.into(),
            reviewer_action: action.into(),
            review_group: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ClientQuery {
    pub query_id: String,
    pub control: String,
    pub tenant: String,
    pub account_id: String,
    pub account_code: String,
    pub account_name: String,
    pub review_group: String,
    pub difference: Option<Decimal>,
    pub question: String,
    pub evidence_requested: String,
}

#[derive(Clone, Debug)]
pub struct ReviewerAcknowledgement {
    pub reviewer_initials: String,
    pub reviewed_on: Date,
    pub comment: String,
}
