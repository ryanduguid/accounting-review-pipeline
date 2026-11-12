// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Offline service boundaries, using only fabricated reports and temporary caches.
#[path = "support/report_fixture.rs"]
mod report_fixture;
#[path = "support/scratch.rs"]
mod scratch;

use report_fixture::balanced_report;
use scratch::Scratch;
use std::{
    cell::Cell,
    fs,
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    thread,
    time::Duration,
};
use xero_trial_balance_export::{
    Result, auth, export,
    http::{self, Credentials, Http, Response},
    pyjson::{self, Json},
    tokens::TokenSession,
};

struct ReportEndpoint {
    cash: bool,
    gets: Cell<usize>,
}
impl Http for ReportEndpoint {
    fn post(&self, _: &str, _: &Credentials, _: &[(&str, &str)]) -> Result<Response> {
        panic!("a fresh fabricated cache must not need a refresh POST")
    }
    fn get(
        &self,
        url: &str,
        access_token: &str,
        tenant: Option<&str>,
        query: &[(&str, &str)],
    ) -> Result<Response> {
        assert_eq!(url, http::REPORT_URL);
        assert_eq!(access_token, "fabricated-access");
        assert_eq!(tenant, Some("fabricated-tenant"));
        let mut expected = vec![("date", "2026-06-30")];
        if self.cash {
            expected.push(("paymentsOnly", "true"));
        }
        assert_eq!(query, expected);
        self.gets.set(self.gets.get() + 1);
        Ok(Response {
            status: 200,
            body: pyjson::canonical(&Json::object([(
                "Reports",
                Json::Array(vec![balanced_report()]),
            )])),
            retry_after: None,
        })
    }
}

#[test]
fn report_fetch_preserves_the_requested_tenant_date_basis_and_balanced_rows() {
    let scratch = Scratch::new();
    let session = TokenSession::new(&scratch.0.join("token.json")).unwrap();
    session
        .save(&Json::object([
            ("access_token", Json::str("fabricated-access")),
            ("refresh_token", Json::str("fabricated-refresh")),
            ("expires_in", Json::int(1800)),
        ]))
        .unwrap();
    let credentials = Credentials {
        id: "fabricated-client".into(),
        secret: "fabricated-client-secret".into(),
    };
    let tenant = Json::object([("tenantId", Json::str("fabricated-tenant"))]);
    for cash in [false, true] {
        let http = ReportEndpoint {
            cash,
            gets: Cell::new(0),
        };
        let rows =
            export::fetch_rows(&http, &session, &credentials, &tenant, "2026-06-30", cash).unwrap();
        assert_eq!(http.gets.get(), 1);
        assert_balanced_rows(&rows);
    }
}

fn assert_balanced_rows(rows: &[xero_trial_balance_export::report::Record]) {
    assert_eq!(rows.len(), 2);
    for (row, id, name, debit, credit) in [
        (
            &rows[0],
            "00000000-0000-0000-0000-000000000001",
            "Bank (090)",
            "1.00",
            "",
        ),
        (
            &rows[1],
            "00000000-0000-0000-0000-000000000002",
            "Payables (800)",
            "",
            "1.00",
        ),
    ] {
        for (key, value) in [
            ("AccountID", id),
            ("Account", name),
            ("Debit", debit),
            ("Credit", credit),
            ("YTD Debit", debit),
            ("YTD Credit", credit),
        ] {
            let actual = &row.iter().find(|(name, _)| name == key).unwrap().1;
            assert!(actual.is_str(value), "{key}: {actual:?}");
        }
    }
}

#[test]
fn report_envelopes_refuse_missing_malformed_and_ambiguous_reports() {
    let report = balanced_report();
    for (payload, message) in [
        (Json::Array(vec![]), "response is not a JSON object"),
        (Json::object([]), "Empty Reports payload"),
        (
            Json::object([("Reports", Json::Array(vec![]))]),
            "Empty Reports payload",
        ),
        (
            Json::object([("Reports", Json::str("unexpected"))]),
            "unexpected Reports shape",
        ),
        (
            Json::object([("Reports", Json::Array(vec![Json::Null]))]),
            "unexpected Reports shape",
        ),
        (
            Json::object([("Reports", Json::Array(vec![report.clone(), report]))]),
            "exactly one report",
        ),
    ] {
        let error = export::validated_rows(&payload, "2026-06-30").unwrap_err();
        assert_eq!(error.code, 1);
        assert!(error.stderr.contains(message), "{}", error.stderr);
    }
}

fn envelope(payload: Json) -> Json {
    Json::object([
        ("format", Json::str("xero-trial-balance-export-token-cache")),
        ("version", Json::int(1)),
        ("protection", Json::str("windows-dpapi-current-user")),
        ("payload", payload),
    ])
}

fn assert_cache_refused(document: &Json, message: &str) {
    let scratch = Scratch::new();
    let path = scratch.0.join("token.json");
    let bytes = pyjson::canonical(document).into_bytes();
    fs::write(&path, &bytes).unwrap();
    let session = TokenSession::new(&path).unwrap();
    let error = session.load().unwrap_err();
    assert_eq!(error.code, 1);
    assert!(error.stderr.contains(message), "{}", error.stderr);
    for force in [false, true] {
        let error = session
            .access_token(force, |_| panic!("an unusable cache must not refresh"))
            .unwrap_err();
        assert!(error.stderr.contains(message), "{}", error.stderr);
    }
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn unknown_and_corrupt_cache_envelopes_are_never_replaced_or_refreshed() {
    assert_cache_refused(&Json::Array(vec![]), "is not a JSON object");
    let mut extra = envelope(Json::str(""));
    xero_trial_balance_export::tokens::set(&mut extra, "extra", Json::Null);
    assert_cache_refused(&extra, "invalid or unknown cache envelope");
    for (key, value) in [
        ("format", Json::str("unknown-format")),
        ("version", Json::int(2)),
        ("protection", Json::str("unknown-protection")),
        ("payload", Json::int(42)),
    ] {
        let mut document = envelope(Json::str(""));
        xero_trial_balance_export::tokens::set(&mut document, key, value);
        assert_cache_refused(&document, "unsupported cache format, version or protection");
    }
}

#[cfg(not(windows))]
#[test]
fn windows_cache_explains_its_platform_requirement_without_changing_bytes() {
    assert_cache_refused(&envelope(Json::str("AQID")), "Windows user who created it");
}

#[cfg(windows)]
#[test]
fn unusable_dpapi_payloads_preserve_the_fabricated_cache() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use xero_trial_balance_export::{dpapi, pyjson::PyStr};
    for (payload, message) in [
        (Json::Str(PyStr(vec![0xd800])), "corrupt DPAPI payload"),
        (Json::str("!not-base64!"), "corrupt DPAPI payload"),
        (Json::str(""), "empty DPAPI payload"),
        (Json::str("AQID"), "corrupt or cannot be opened"),
        (
            Json::str(&STANDARD.encode(dpapi::protect(b"[]").unwrap())),
            "decrypted payload is not a JSON object",
        ),
        (
            Json::str(&STANDARD.encode(dpapi::protect(b"not-json").unwrap())),
            "decrypted payload",
        ),
    ] {
        assert_cache_refused(&envelope(payload), message);
    }
}

#[test]
fn callback_verification_distinguishes_code_denial_and_missing_authorisation() {
    let mut callback = auth::Callback {
        code: Some("fabricated+code".into()),
        state: Some("fabricated-state".into()),
        error: None,
    };
    assert_eq!(
        auth::verify_callback(&callback, "fabricated-state").unwrap(),
        "fabricated+code"
    );
    callback.error = Some("access_denied".into());
    let denied = auth::verify_callback(&callback, "fabricated-state").unwrap_err();
    assert!(denied.stderr.contains("'access_denied'"));
    assert!(denied.stderr.contains("consent was denied or cancelled"));
    for error in [
        "unsafe\ncallback".to_string(),
        "x".repeat(65),
        String::new(),
    ] {
        callback.error = Some(error.clone());
        let failure = auth::verify_callback(&callback, "fabricated-state").unwrap_err();
        assert!(
            failure
                .stderr
                .contains("error code this script could not read")
        );
        if !error.is_empty() {
            assert!(!failure.stderr.contains(&error));
        }
    }
    callback.error = None;
    callback.code = None;
    assert!(
        auth::verify_callback(&callback, "fabricated-state")
            .unwrap_err()
            .stderr
            .contains("no authorisation code")
    );
}

#[test]
fn authorisation_preflight_refusals_preserve_existing_and_absent_caches() {
    let occupied = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let redirect = format!(
        "http://localhost:{}/callback",
        occupied.local_addr().unwrap().port()
    );
    let endpoint = ReportEndpoint {
        cash: false,
        gets: Cell::new(0),
    };
    let credentials = Credentials {
        id: "fabricated-client".into(),
        secret: "fabricated-secret".into(),
    };
    for present in [false, true] {
        let scratch = Scratch::new();
        let path = scratch.0.join("token.json");
        let sentinel = b"fabricated cache must remain untouched\n";
        if present {
            fs::write(&path, sentinel).unwrap();
        }
        let session = TokenSession::new(&path).unwrap();
        for (uri, diagnostic) in [
            ("https://localhost/callback", "Invalid XERO_REDIRECT_URI:"),
            (redirect.as_str(), "error: cannot listen on localhost:"),
        ] {
            let failure = auth::authorise(&endpoint, &session, &credentials, uri).unwrap_err();
            assert_eq!(failure.code, 1);
            assert!(failure.stdout.is_empty());
            assert!(failure.stderr.starts_with(diagnostic), "{}", failure.stderr);
            if present {
                assert_eq!(fs::read(&path).unwrap(), sentinel);
            } else {
                assert!(!path.exists());
            }
            assert_eq!(
                fs::read_dir(&scratch.0).unwrap().count(),
                usize::from(present)
            );
        }
    }
    assert_eq!(endpoint.gets.get(), 0);
}

#[test]
fn callback_listener_ignores_an_unrelated_request_then_returns_the_code() {
    let config = auth::CallbackConfig {
        host: "localhost".into(),
        port: 0,
        path: "/callback".into(),
    };
    let listener = auth::bind(&config).unwrap();
    let address = listener.local_addr().unwrap();
    let client = thread::spawn(move || {
        for (path, expected) in [
            ("/favicon.ico", "404"),
            (
                "/callback?code=fabricated%2Bcode&state=fabricated-state",
                "200",
            ),
        ] {
            let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(
                response.starts_with(&format!("HTTP/1.0 {expected} ")),
                "{response}"
            );
        }
    });
    let result = auth::wait_for_callback(&listener, &config, Duration::from_secs(5));
    client.join().unwrap();
    let callback = result.unwrap();
    assert_eq!(callback.state.as_deref(), Some("fabricated-state"));
    assert_eq!(
        auth::verify_callback(&callback, "fabricated-state").unwrap(),
        "fabricated+code"
    );
}
