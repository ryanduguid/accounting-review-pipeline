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

#[derive(Clone, Debug)]
pub struct OpenItem {
    pub item_id: String,
    pub severity: String,
    pub owner: String,
    pub due_date: Date,
    pub status: String,
    pub description: String,
    pub resolution: String,
}

#[derive(Clone, Debug)]
pub struct SelfReview {
    pub preparer_initials: String,
    pub prepared_on: Date,
    pub engagement_type: String,
    pub period_end: Date,
    pub assertions: Vec<(String, bool)>,
}

#[derive(Clone, Debug)]
pub struct ActivityLabel {
    pub label: String,
    pub amount: Decimal,
}

#[derive(Clone, Debug)]
pub struct GstControlRow {
    pub posted_on: Date,
    pub account_id: String,
    pub account_name: String,
    pub debit: Decimal,
    pub credit: Decimal,
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct BankRecRow {
    pub account_id: String,
    pub statement_balance: Decimal,
    pub gl_balance: Decimal,
}

#[derive(Clone, Debug)]
pub struct TieOutRow {
    pub statement_line: String,
    pub workpaper_ref: String,
    pub source_file: String,
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct PriorFinding {
    pub finding_code: String,
    pub slot: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub code: String,
    pub status: String,
    pub slot: String,
    pub reason: String,
    pub reviewer_action: String,
    pub repeat: bool,
}

impl Finding {
    pub fn new(
        code: &str,
        status: &str,
        slot: &str,
        reason: impl Into<String>,
        action: &str,
    ) -> Self {
        Self {
            code: code.into(),
            status: status.into(),
            slot: slot.into(),
            reason: reason.into(),
            reviewer_action: action.into(),
            repeat: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ReviewerAcknowledgement {
    pub reviewer_initials: String,
    pub reviewed_on: Date,
    pub comment: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ControlNotRun {
    pub slot: String,
    pub filename: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct SourceEvidence {
    pub slot: String,
    pub filename: String,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct ReadinessPack {
    pub status: String,
    pub engagement_type: String,
    pub period_end: String,
    pub preparer_initials: String,
    pub findings: Vec<Finding>,
    pub source_evidence: Vec<SourceEvidence>,
    pub tieout_tolerance: Decimal,
    pub acknowledgement: Option<ReviewerAcknowledgement>,
    /// None means a legacy pack states no control coverage.
    pub controls_not_run: Option<Vec<ControlNotRun>>,
}
