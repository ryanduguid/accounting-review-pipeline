// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::*;
use crate::{export, http::Response, publication, testing::Scratch};
use std::{cell::RefCell, collections::VecDeque, fs};

struct StubHttp {
    expected_tenant: String,
    replies: RefCell<VecDeque<(String, Vec<(String, String)>, Response)>>,
}

impl StubHttp {
    fn new(tenant: &str, items: Vec<(&str, Vec<(&str, &str)>, &str)>) -> Self {
        let queue = items
            .into_iter()
            .map(|(url, query, body)| {
                (
                    url.to_owned(),
                    query
                        .into_iter()
                        .map(|(k, v)| (k.to_owned(), v.to_owned()))
                        .collect(),
                    Response {
                        status: 200,
                        body: body.to_owned(),
                        retry_after: None,
                    },
                )
            })
            .collect();
        Self {
            expected_tenant: tenant.to_owned(),
            replies: RefCell::new(queue),
        }
    }
}

impl Http for StubHttp {
    fn post(&self, _: &str, _: &Credentials, _: &[(&str, &str)]) -> Result<Response> {
        Err(Failure::message(
            "read-only evidence exports must never issue POST",
        ))
    }

    fn get(
        &self,
        url: &str,
        _: &str,
        tenant_id: Option<&str>,
        query: &[(&str, &str)],
    ) -> Result<Response> {
        assert_eq!(tenant_id, Some(self.expected_tenant.as_str()));
        let (expected_url, expected_query, response) =
            self.replies.borrow_mut().pop_front().expect("queued reply");
        assert_eq!(url, expected_url);
        let actual_query: Vec<(String, String)> = query
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        assert_eq!(actual_query, expected_query);
        Ok(response)
    }
}

fn setup_session() -> (Scratch, TokenSession, Credentials, Json) {
    let scratch = Scratch::new();
    let token_path = scratch.0.join("token.json");
    let session = TokenSession::new(&token_path).unwrap();
    let token_doc = Json::object([
        ("access_token", Json::str("synthetic-access")),
        ("refresh_token", Json::str("synthetic-refresh")),
        ("expires_at", Json::int(4_000_000_000)),
    ]);
    session.save(&token_doc).unwrap();
    let creds = Credentials {
        id: "synthetic-id".into(),
        secret: "synthetic-secret".into(),
    };
    let tenant = Json::object([
        ("tenantId", Json::str("tenant-001")),
        ("tenantName", Json::str("Synthetic Pty Ltd")),
    ]);
    (scratch, session, creds, tenant)
}

#[test]
fn scopes_enforce_read_only_suffix_and_reject_write_or_broad_scopes() {
    assert!(all_read_only(&READ_ONLY_SCOPES));
    assert!(!all_read_only(&["assets"]));
    assert!(READ_ONLY_SCOPES.iter().all(|s| is_read_only_scope(s)));
    assert!(!is_read_only_scope(".read"));
    assert!(!is_read_only_scope("assets"));
    assert!(assert_read_only_scopes(&["offline_access", "assets.read"]).is_ok());
    assert!(assert_read_only_scopes(&READ_ONLY_SCOPES).is_ok());
    assert!(evidence_scope_string().starts_with("offline_access "));
    assert!(
        assert_read_only_scopes(&["accounting.reports.read"])
            .unwrap_err()
            .stderr
            .contains("deprecated broad scope")
    );
    assert!(
        assert_read_only_scopes(&["accounting.transactions"])
            .unwrap_err()
            .stderr
            .contains("non-read-only")
    );
    let (_scratch, _session, creds, _tenant) = setup_session();
    let stub = StubHttp::new("tenant-001", vec![]);
    assert!(stub.post("https://example.invalid", &creds, &[]).is_err());
}

fn sample_tb_row(id: &str, debit: &str, credit: &str) -> Vec<(String, Json)> {
    [
        ("AccountID", id),
        ("Account", "Cash (100)"),
        ("Section", "Assets"),
        ("Debit", debit),
        ("Credit", credit),
        ("YTD Debit", debit),
        ("YTD Credit", credit),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), Json::str(v)))
    .collect()
}

const SYNTHETIC_ORG_BODY: &str = r#"{
  "Organisations": [{
    "Name": "Synthetic Pty Ltd",
    "CountryCode": "AU",
    "BaseCurrency": "AUD",
    "SalesTaxBasis": "ACCRUALS",
    "FinancialYearEndDay": 30,
    "FinancialYearEndMonth": 6,
    "OrganisationEntityType": "COMPANY"
  }]
}"#;

#[test]
fn organisation_read_returns_profile_and_export_with_limitations() {
    let (_scratch, session, creds, tenant) = setup_session();
    let http = StubHttp::new(
        "tenant-001",
        vec![(ORGANISATION_URL, vec![], SYNTHETIC_ORG_BODY)],
    );
    let org = fetch_organisation(&http, &session, &creds, &tenant).unwrap();
    assert_eq!(org.base_currency, "AUD");
    assert_eq!(org.gst_basis, "ACCRUALS");
    assert_eq!(org.financial_year_end_month, Some(6));
    assert_eq!(org.financial_year_end_day, Some(30));

    let exported = organisation_export(&tenant, &org);
    let org_json = exported.at("organisation");
    assert!(org_json.at("base_currency").is_str("AUD"));
    assert!(org_json.at("gst_basis").is_str("ACCRUALS"));
    assert!(matches!(exported.at("limitations"), Json::Array(items) if !items.is_empty()));
}

#[test]
fn organisation_profile_folds_into_tb_manifest() {
    let (scratch, session, creds, tenant) = setup_session();
    let http = StubHttp::new(
        "tenant-001",
        vec![(ORGANISATION_URL, vec![], SYNTHETIC_ORG_BODY)],
    );
    let org = fetch_organisation(&http, &session, &creds, &tenant).unwrap();
    let rows = vec![
        sample_tb_row("a1", "10.00", "0"),
        sample_tb_row("a2", "0", "10.00"),
    ];
    let csv_path = scratch.0.join("tb.csv");
    let (res, _, _) = crate::stdio::capture(|| {
        export::publish_with_organisation(
            &rows,
            &tenant,
            "2026-06-30",
            &csv_path,
            false,
            true,
            false,
            Some(&org),
        )
    });
    res.unwrap();
    let manifest_raw = fs::read_to_string(publication::manifest_path(&csv_path)).unwrap();
    let manifest = crate::pyjson::loads(&manifest_raw).unwrap();
    let org_json = manifest.at("organisation");
    assert!(org_json.at("base_currency").is_str("AUD"));
    assert!(org_json.at("gst_basis").is_str("ACCRUALS"));
}

#[test]
fn organisation_parser_handles_defaults_and_rejects_malformed_payloads() {
    let minimal = crate::pyjson::loads(
        r#"{"Organisations":[{"Name":"Min","CountryCode":"NZ","BaseCurrency":"NZD","SalesTaxBasis":"CASH"}]}"#,
    )
    .unwrap();
    let parsed_min = parse_organisation(&minimal).unwrap();
    assert_eq!(parsed_min.entity_type, "UNKNOWN");
    assert_eq!(parsed_min.financial_year_end_month, None);
    let _ = parsed_min.to_manifest_json();

    for bad in [
        r#"{}"#,
        r#"{"Organisations":[]}"#,
        r#"{"Organisations":[{"Name":"A","CountryCode":"AU","SalesTaxBasis":"CASH"}]}"#,
    ] {
        let doc = crate::pyjson::loads(bad).unwrap();
        assert!(parse_organisation(&doc).is_err(), "{bad}");
    }
}

#[test]
fn asset_register_unions_all_three_statuses_across_pages_with_limitations() {
    let (_scratch, session, creds, tenant) = setup_session();
    let page = |status, p, body| (ASSETS_URL, vec![("status", status), ("page", p)], body);
    let http = StubHttp::new(
        "tenant-001",
        vec![
            page(
                "DRAFT",
                "1",
                r#"{"pagination":{"page":1,"pageCount":2},"items":[{"assetId":"d1"}]}"#,
            ),
            page(
                "DRAFT",
                "2",
                r#"{"pagination":{"page":2,"pageCount":2},"items":[{"assetId":"d2"}]}"#,
            ),
            page(
                "REGISTERED",
                "1",
                r#"{"pagination":{"page":1,"pageCount":1},"items":[{"assetId":"r1"}]}"#,
            ),
            page(
                "DISPOSED",
                "1",
                r#"{"pagination":{"page":1,"pageCount":1},"items":[]}"#,
            ),
        ],
    );
    let register = fetch_asset_register(&http, &session, &creds, &tenant).unwrap();
    assert!(matches!(register.at("items"), Json::Array(items) if items.len() == 3));
    let counts = register.at("counts");
    for (status, want) in [("DRAFT", 2), ("REGISTERED", 1), ("DISPOSED", 0)] {
        assert!(crate::pyjson::py_eq(
            counts.at(status),
            &Json::int(want),
            false
        ));
    }
    assert!(
        crate::pyjson::canonical(register.at("limitations")).contains("book depreciation only")
    );

    let bad = StubHttp::new("tenant-001", vec![page("DRAFT", "1", r#"{}"#)]);
    assert!(fetch_asset_register(&bad, &session, &creds, &tenant).is_err());
}

#[test]
fn bank_summary_validates_date_range_shape_and_limitations() {
    let (_scratch, session, creds, tenant) = setup_session();
    let summary_body =
        r#"{"Reports":[{"ReportID":"BankSummary","ReportType":"BankSummary","Rows":[]}]}"#;
    let http = StubHttp::new(
        "tenant-001",
        vec![(
            BANK_SUMMARY_URL,
            vec![("fromDate", "2026-04-01"), ("toDate", "2026-06-30")],
            summary_body,
        )],
    );
    let bank =
        fetch_bank_summary(&http, &session, &creds, &tenant, "2026-04-01", "2026-06-30").unwrap();
    assert!(bank.at("from_date").is_str("2026-04-01"));
    assert!(matches!(bank.at("limitations"), Json::Array(items) if !items.is_empty()));
    assert!(
        fetch_bank_summary(&http, &session, &creds, &tenant, "2026-07-01", "2026-06-30").is_err()
    );

    for bad_body in [
        r#"{}"#,
        r#"{"Reports":[]}"#,
        r#"{"Reports":[{"ReportType":"TrialBalance"}]}"#,
    ] {
        let bad_http = StubHttp::new(
            "tenant-001",
            vec![(
                BANK_SUMMARY_URL,
                vec![("fromDate", "2026-04-01"), ("toDate", "2026-06-30")],
                bad_body,
            )],
        );
        assert!(
            fetch_bank_summary(
                &bad_http,
                &session,
                &creds,
                &tenant,
                "2026-04-01",
                "2026-06-30"
            )
            .is_err()
        );
    }
}

#[test]
fn published_bas_distinguishes_nothing_published_and_filters_by_period() {
    let (_scratch, session, creds, tenant) = setup_session();
    let empty_http = StubHttp::new(
        "tenant-001",
        vec![(REPORTS_URL, vec![], r#"{"Reports":[]}"#)],
    );
    let none_pub = fetch_published_bas(&empty_http, &session, &creds, &tenant, None).unwrap();
    assert!(none_pub.at("status").is_str("nothing_published"));
    assert!(matches!(none_pub.at("reports"), Json::Array(items) if items.is_empty()));
    let lim = crate::pyjson::canonical(none_pub.at("limitations"));
    assert!(lim.contains("nothing_published") && lim.contains("Draft"));

    let list_body = r#"{"Reports":[
      {"ReportID":"r-other","ReportType":"ProfitAndLoss","ReportName":"P&L"},
      {"ReportID":"bas-q3-2026","ReportType":"BAS","ReportName":"BAS Q3 2026","ReportDate":"2026-03-31"},
      {"ReportID":"bas-q4-2026","ReportType":"BAS","ReportName":"BAS Q4 2026","ReportDate":"2026-06-30"}
    ]}"#;
    let detail_body = r#"{"Reports":[{"ReportID":"bas-q4-2026","ReportType":"BAS","Rows":[]}]}"#;
    let pub_http = StubHttp::new(
        "tenant-001",
        vec![
            (REPORTS_URL, vec![], list_body),
            (&format!("{REPORTS_URL}/bas-q4-2026"), vec![], detail_body),
        ],
    );
    let published =
        fetch_published_bas(&pub_http, &session, &creds, &tenant, Some("Q4 2026")).unwrap();
    assert!(published.at("status").is_str("published"));
    assert!(matches!(published.at("reports"), Json::Array(items) if items.len() == 1));
}

#[test]
fn published_bas_rejects_malformed_list_id_and_detail_payloads() {
    let (_scratch, session, creds, tenant) = setup_session();
    let bad_list = StubHttp::new("tenant-001", vec![(REPORTS_URL, vec![], r#"{}"#)]);
    assert!(fetch_published_bas(&bad_list, &session, &creds, &tenant, None).is_err());

    let bad_id = StubHttp::new(
        "tenant-001",
        vec![(
            REPORTS_URL,
            vec![],
            r#"{"Reports":[{"ReportID":"../evil","ReportType":"BAS","ReportName":"BAS"}]}"#,
        )],
    );
    assert!(fetch_published_bas(&bad_id, &session, &creds, &tenant, None).is_err());

    for bad_detail in [r#"{}"#, r#"{"Reports":[]}"#] {
        let bad_det_http = StubHttp::new(
            "tenant-001",
            vec![
                (
                    REPORTS_URL,
                    vec![],
                    r#"{"Reports":[{"ReportID":"bas-1","ReportType":"BAS","ReportName":"BAS"}]}"#,
                ),
                (&format!("{REPORTS_URL}/bas-1"), vec![], bad_detail),
            ],
        );
        assert!(fetch_published_bas(&bad_det_http, &session, &creds, &tenant, None).is_err());
    }
}
