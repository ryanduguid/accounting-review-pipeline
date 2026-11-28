// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Read-only Xero evidence exports beyond the Trial Balance:
//! Organisation (with base currency and GST basis for the manifest),
//! Fixed Asset Register across all 3 statuses, Published BAS, and Bank Summary.
use crate::{
    Failure, Result, cli,
    http::{self, Credentials, Http},
    pyjson::{Json, PyStr},
    report,
    tokens::TokenSession,
};

#[cfg(test)]
mod tests;

pub const ORGANISATION_URL: &str = "https://api.xero.com/api.xro/2.0/Organisation";
pub const ASSETS_URL: &str = "https://api.xero.com/assets.xro/1.0/Assets";
pub const BANK_SUMMARY_URL: &str = "https://api.xero.com/api.xro/2.0/Reports/BankSummary";
pub const REPORTS_URL: &str = "https://api.xero.com/api.xro/2.0/Reports";

pub const ASSET_STATUSES: [&str; 3] = ["DRAFT", "REGISTERED", "DISPOSED"];

pub const READ_ONLY_SCOPES: [&str; 5] = [
    "accounting.reports.trialbalance.read",
    "accounting.reports.banksummary.read",
    "accounting.reports.taxreports.read",
    "accounting.settings.read",
    "assets.read",
];

pub const fn is_read_only_scope(scope: &str) -> bool {
    let bytes = scope.as_bytes();
    let suffix = b".read";
    if bytes.len() <= suffix.len() {
        return false;
    }
    let start = bytes.len() - suffix.len();
    let mut i = 0;
    while i < suffix.len() {
        if bytes[start + i] != suffix[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn all_read_only(scopes: &[&str]) -> bool {
    let mut i = 0;
    while i < scopes.len() {
        if !is_read_only_scope(scopes[i]) {
            return false;
        }
        i += 1;
    }
    true
}

const _: () = assert!(
    all_read_only(&READ_ONLY_SCOPES),
    "every Xero resource scope must end with .read"
);

pub fn assert_read_only_scopes(scopes: &[&str]) -> Result<()> {
    for &scope in scopes {
        if scope == "offline_access" {
            continue;
        }
        if scope == "accounting.reports.read" {
            return Err(Failure::message(
                "error: deprecated broad scope 'accounting.reports.read' is refused; use granular .read scopes.",
            ));
        }
        if !is_read_only_scope(scope) {
            return Err(Failure::message(format!(
                "error: non-read-only Xero scope '{scope}' is refused; every resource scope must end in .read."
            )));
        }
    }
    Ok(())
}

pub fn evidence_scope_string() -> String {
    format!("offline_access {}", READ_ONLY_SCOPES.join(" "))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganisationProfile {
    pub name: String,
    pub country_code: String,
    pub base_currency: String,
    pub gst_basis: String,
    pub financial_year_end_day: Option<i64>,
    pub financial_year_end_month: Option<i64>,
    pub entity_type: String,
}

impl OrganisationProfile {
    pub fn to_manifest_json(&self) -> Json {
        let mut pairs = vec![
            (PyStr::from("base_currency"), Json::str(&self.base_currency)),
            (PyStr::from("gst_basis"), Json::str(&self.gst_basis)),
            (PyStr::from("country_code"), Json::str(&self.country_code)),
            (PyStr::from("entity_type"), Json::str(&self.entity_type)),
        ];
        if let Some(month) = self.financial_year_end_month {
            pairs.push((PyStr::from("financial_year_end_month"), Json::int(month)));
        }
        if let Some(day) = self.financial_year_end_day {
            pairs.push((PyStr::from("financial_year_end_day"), Json::int(day)));
        }
        Json::Object(pairs)
    }
}

fn required_string(obj: &Json, key: &str, context: &str) -> Result<String> {
    obj.get(key)
        .and_then(Json::as_pystr)
        .and_then(|s| s.to_utf8())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| Failure::message(format!("error: {context} is missing a valid {key}.")))
}

fn optional_int(obj: &Json, key: &str) -> Option<i64> {
    match obj.get(key) {
        Some(Json::Int(n)) => i64::try_from(n).ok(),
        _ => None,
    }
}

fn tenant_id(tenant: &Json) -> Result<String> {
    required_string(tenant, "tenantId", "Xero tenant selection")
}

fn tenant_block(tenant: &Json) -> Json {
    Json::object([
        ("tenant_id", tenant.at("tenantId").clone()),
        ("tenant_name", tenant.at("tenantName").clone()),
    ])
}

fn limitations_json(items: &[&str]) -> Json {
    Json::Array(items.iter().map(|s| Json::str(s)).collect())
}

pub fn parse_organisation(payload: &Json) -> Result<OrganisationProfile> {
    let Some(Json::Array(orgs)) = payload.get("Organisations") else {
        return Err(Failure::message(
            "error: Xero Organisation response is missing an Organisations list.",
        ));
    };
    let [org] = orgs.as_slice() else {
        return Err(Failure::message(
            "error: Xero Organisation response must contain exactly one organisation.",
        ));
    };
    let name = required_string(org, "Name", "Xero Organisation")?;
    let base_currency = required_string(org, "BaseCurrency", "Xero Organisation")?;
    let gst_basis = required_string(org, "SalesTaxBasis", "Xero Organisation")?;
    let country_code = required_string(org, "CountryCode", "Xero Organisation")?;
    let entity_type = org
        .get("OrganisationEntityType")
        .or_else(|| org.get("OrganisationType"))
        .and_then(Json::as_pystr)
        .and_then(|s| s.to_utf8())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "UNKNOWN".into());
    Ok(OrganisationProfile {
        name,
        country_code,
        base_currency,
        gst_basis,
        financial_year_end_day: optional_int(org, "FinancialYearEndDay"),
        financial_year_end_month: optional_int(org, "FinancialYearEndMonth"),
        entity_type,
    })
}

pub fn fetch_organisation(
    http: &impl Http,
    session: &TokenSession,
    credentials: &Credentials,
    tenant: &Json,
) -> Result<OrganisationProfile> {
    let id = tenant_id(tenant)?;
    let payload = http::api_get(http, session, credentials, ORGANISATION_URL, Some(&id), &[])?;
    parse_organisation(&payload)
}

pub fn organisation_export(tenant: &Json, org: &OrganisationProfile) -> Json {
    Json::object([
        ("schema_version", Json::str("xero-organisation.v1")),
        ("entity_ref", tenant_block(tenant)),
        ("organisation", org.to_manifest_json()),
        (
            "limitations",
            limitations_json(&[
                "Organisation settings reflect Xero configuration only, not live ATO GST registration or lodgement obligations.",
                "Non-AUD BaseCurrency or unexpected SalesTaxBasis must be reviewed before relying on Australian BAS controls.",
            ]),
        ),
    ])
}

pub fn fetch_asset_register(
    http: &impl Http,
    session: &TokenSession,
    credentials: &Credentials,
    tenant: &Json,
) -> Result<Json> {
    let id = tenant_id(tenant)?;
    let mut items = Vec::new();
    let mut counts = Vec::new();
    for status in ASSET_STATUSES {
        let fetched = fetch_assets_for_status(http, session, credentials, &id, status)?;
        counts.push((PyStr::from(status), Json::int(fetched.len() as i64)));
        items.extend(fetched);
    }
    Ok(Json::object([
        ("schema_version", Json::str("xero-asset-register.v1")),
        ("entity_ref", tenant_block(tenant)),
        ("counts", Json::Object(counts)),
        ("total", Json::int(items.len() as i64)),
        ("items", Json::Array(items)),
        (
            "limitations",
            limitations_json(&[
                "Xero Assets API returns book depreciation only; it carries no tax depreciation pool or tax written-down value (WDV).",
                "Each status (DRAFT, REGISTERED, DISPOSED) is queried separately and combined here; draft assets are not yet depreciated.",
            ]),
        ),
    ]))
}

fn fetch_assets_for_status(
    http: &impl Http,
    session: &TokenSession,
    credentials: &Credentials,
    tenant_id: &str,
    status: &str,
) -> Result<Vec<Json>> {
    let mut all = Vec::new();
    let mut page: i64 = 1;
    loop {
        let page_str = page.to_string();
        let payload = http::api_get(
            http,
            session,
            credentials,
            ASSETS_URL,
            Some(tenant_id),
            &[("status", status), ("page", &page_str)],
        )?;
        let Some(Json::Array(batch)) = payload.get("items") else {
            return Err(Failure::message(format!(
                "error: Xero Assets response for status {status} is missing an items list."
            )));
        };
        all.extend(batch.iter().cloned());
        let page_count = payload
            .get("pagination")
            .and_then(|p| optional_int(p, "pageCount"))
            .unwrap_or(1);
        if page >= page_count {
            break;
        }
        page += 1;
    }
    Ok(all)
}

pub fn fetch_bank_summary(
    http: &impl Http,
    session: &TokenSession,
    credentials: &Credentials,
    tenant: &Json,
    from_date: &str,
    to_date: &str,
) -> Result<Json> {
    let from = cli::iso_date(from_date)?;
    let to = cli::iso_date(to_date)?;
    if from > to {
        return Err(Failure::message(
            "error: Bank Summary fromDate must not be later than toDate.",
        ));
    }
    let id = tenant_id(tenant)?;
    let payload = http::api_get(
        http,
        session,
        credentials,
        BANK_SUMMARY_URL,
        Some(&id),
        &[("fromDate", &from), ("toDate", &to)],
    )?;
    let Some(Json::Array(reports)) = payload.get("Reports") else {
        return Err(Failure::message(
            "error: Xero Bank Summary response is missing a Reports list.",
        ));
    };
    let [summary] = reports.as_slice() else {
        return Err(Failure::message(
            "error: Xero Bank Summary response must contain exactly one report.",
        ));
    };
    if !summary
        .get("ReportType")
        .is_some_and(|v| v.is_str("BankSummary"))
    {
        return Err(Failure::message(
            "error: returned report is not identified as a BankSummary.",
        ));
    }
    Ok(Json::object([
        ("schema_version", Json::str("xero-bank-summary.v1")),
        ("entity_ref", tenant_block(tenant)),
        ("from_date", Json::str(&from)),
        ("to_date", Json::str(&to)),
        ("report", summary.clone()),
        (
            "limitations",
            limitations_json(&[
                "Bank Summary reflects ledger cash movement and closing ledger balances only, not external bank-statement reconciliation.",
                "Unreconciled statement lines and missing period-end bank statements are not verified by this report.",
            ]),
        ),
    ]))
}

fn valid_report_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn is_bas_report(item: &Json, period_filter: Option<&str>) -> bool {
    if !item.get("ReportType").is_some_and(|v| v.is_str("BAS")) {
        return false;
    }
    let Some(needle) = period_filter else {
        return true;
    };
    let name = item.get("ReportName").map(report::text).unwrap_or_default();
    let date = item.get("ReportDate").map(report::text).unwrap_or_default();
    name.contains(needle) || date.contains(needle)
}

fn fetch_one_bas(
    http: &impl Http,
    session: &TokenSession,
    credentials: &Credentials,
    tenant_id: &str,
    entry: &Json,
) -> Result<Json> {
    let report_id = required_string(entry, "ReportID", "Xero published BAS entry")?;
    if !valid_report_id(&report_id) {
        return Err(Failure::message(format!(
            "error: invalid Xero published report ID '{report_id}'."
        )));
    }
    let url = format!("{REPORTS_URL}/{report_id}");
    let detail = http::api_get(http, session, credentials, &url, Some(tenant_id), &[])?;
    let Some(Json::Array(items)) = detail.get("Reports") else {
        return Err(Failure::message(
            "error: Xero published BAS detail response is missing a Reports list.",
        ));
    };
    let [bas] = items.as_slice() else {
        return Err(Failure::message(
            "error: Xero published BAS detail response must contain exactly one report.",
        ));
    };
    Ok(bas.clone())
}

pub fn fetch_published_bas(
    http: &impl Http,
    session: &TokenSession,
    credentials: &Credentials,
    tenant: &Json,
    period_filter: Option<&str>,
) -> Result<Json> {
    let id = tenant_id(tenant)?;
    let listing = http::api_get(http, session, credentials, REPORTS_URL, Some(&id), &[])?;
    let Some(Json::Array(entries)) = listing.get("Reports") else {
        return Err(Failure::message(
            "error: Xero Reports list response is missing a Reports list.",
        ));
    };
    let mut reports = Vec::new();
    for entry in entries
        .iter()
        .filter(|item| is_bas_report(item, period_filter))
    {
        reports.push(fetch_one_bas(http, session, credentials, &id, entry)?);
    }
    let status = if reports.is_empty() {
        "nothing_published"
    } else {
        "published"
    };
    Ok(Json::object([
        ("schema_version", Json::str("xero-published-bas.v1")),
        ("entity_ref", tenant_block(tenant)),
        ("status", Json::str(status)),
        ("reports", Json::Array(reports)),
        (
            "limitations",
            limitations_json(&[
                "Xero Reports lists published statements only; status 'nothing_published' means no published BAS was returned, not that no BAS obligation exists.",
                "Draft or unsubmitted activity statements are unavailable through the Xero Accounting API.",
            ]),
        ),
    ]))
}
