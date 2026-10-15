//! Derive draft client questions without recording an answer or sending anything.
use crate::{
    Result,
    models::{ClientQuery, ExceptionItem, Status},
    pytext, sha256,
};
use std::collections::HashSet;

const YEAR_RESET_CAVEAT: &str = " This comparison crosses a 30 June year-to-date reset, so confirm the movement is real before putting this query to anyone: a profit-and-loss account restarts at nil on 1 July.";

fn classify(item: &ExceptionItem) -> Option<(&'static str, &'static str, &'static str)> {
    match item.control.as_str() {
        "equity_reconciliation"
            if item.status != Status::Blocked && !item.account_id.is_empty() =>
        {
            Some((
                "difference",
                "Which supporting movement explains this equity difference?",
                "The dated contribution, withdrawal or evidenced transfer records for this account.",
            ))
        }
        "period_comparison" if item.current_value.is_none() => Some((
            "absent",
            "Was this account closed, reclassified or renamed during the period, or is it missing from the export?",
            "Confirmation of the change and the account the balance moved to, or a corrected export.",
        )),
        "period_comparison" => Some((
            "new",
            "What is this new account used for, and when did it start being used?",
            "The reason the account was opened and the first transaction posted to it.",
        )),
        "subledger_reconciliation" if item.current_value.is_some() => Some((
            "difference",
            "What makes up the difference between the general ledger balance and the subledger balance for this account?",
            "The reconciling items with their dates and amounts, and support for any that are timing differences.",
        )),
        "period_variance" => Some((
            "",
            "What drove the year-to-date movement in this account against the prior period?",
            "The transactions or documents behind the movement, and confirmation that no part of it belongs to another period.",
        )),
        "account_metadata" => Some((
            "",
            "Who changed this account's code, name or section since the prior period, and why?",
            "The date of the chart-of-accounts change, who approved it, and any effect on how the prior period was reported.",
        )),
        _ => None,
    }
}

pub fn query_id(item: &ExceptionItem, variant: &str) -> String {
    let material = [&item.control, variant, &item.tenant, &item.account_id]
        .into_iter()
        .map(|part| format!("{}:{part}", part.chars().count()))
        .collect::<Vec<_>>()
        .join("|");
    format!("Q-{}", &sha256(material.as_bytes())[..12])
}

pub fn derive_client_queries(exceptions: &[ExceptionItem]) -> Result<Vec<ClientQuery>> {
    let crosses_year = exceptions
        .iter()
        .any(|item| item.control == "financial_year_reset");
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for item in exceptions {
        let Some((variant, question, evidence)) = classify(item) else {
            continue;
        };
        let query_id = query_id(item, variant);
        if !seen.insert(query_id.clone()) {
            return Err(format!(
                "two client queries share {query_id}: control {} raised more than one exception for account {}",
                pytext::repr(&item.control),
                pytext::repr(&item.account_id)
            ));
        }
        let mut evidence_requested = evidence.to_string();
        if crosses_year && item.control == "period_variance" {
            evidence_requested.push_str(YEAR_RESET_CAVEAT);
        }
        result.push(ClientQuery {
            query_id,
            control: item.control.clone(),
            tenant: item.tenant.clone(),
            account_id: item.account_id.clone(),
            account_code: item.account_code.clone(),
            account_name: item.account_name.clone(),
            review_group: item.review_group.clone(),
            difference: item.difference.clone(),
            question: question.into(),
            evidence_requested,
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_identity_is_length_bound_and_duplicates_fail_closed() {
        let mut first = ExceptionItem::new("period_variance", Status::Review, None, "", "");
        first.tenant = "A|B".into();
        first.account_id = "C".into();
        let mut other = first.clone();
        other.tenant = "A".into();
        other.account_id = "B|C".into();
        assert_ne!(query_id(&first, ""), query_id(&other, ""));
        let reset = ExceptionItem::new("financial_year_reset", Status::Review, None, "", "");
        let alone = derive_client_queries(&[first.clone()]).unwrap();
        let with_reset = derive_client_queries(&[reset, first.clone()]).unwrap();
        assert_eq!(alone[0].query_id, with_reset[0].query_id);
        assert!(
            with_reset[0]
                .evidence_requested
                .ends_with(YEAR_RESET_CAVEAT)
        );
        assert!(
            derive_client_queries(&[first.clone(), first])
                .unwrap_err()
                .contains("two client queries share")
        );
    }
}
