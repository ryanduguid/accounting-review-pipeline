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
        control: &str,
        status: Status,
        row: Option<&TrialBalanceRow>,
        reason: &str,
        action: &str,
    ) -> Self {
        Self {
            control: control.into(),
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
