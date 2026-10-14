//! The HTTP seam is synchronous to callers; connect and read timeouts are separate.
use crate::{
    Failure, Result,
    pyjson::{self, Json},
    tokens::TokenSession,
};
use std::time::Duration;

pub const TOKEN_URL: &str = "https://identity.xero.com/connect/token";
pub const CONNECTIONS_URL: &str = "https://api.xero.com/connections";
pub const REPORT_URL: &str = "https://api.xero.com/api.xro/2.0/Reports/TrialBalance";

pub struct Credentials {
    pub id: String,
    pub secret: String,
}

pub struct Response {
    pub status: u16,
    pub body: String,
    pub retry_after: Option<String>,
}

pub trait Http {
    fn post(&self, url: &str, credentials: &Credentials, form: &[(&str, &str)])
    -> Result<Response>;
    fn get(
        &self,
        url: &str,
        access_token: &str,
        tenant_id: Option<&str>,
        query: &[(&str, &str)],
    ) -> Result<Response>;
}

pub struct NativeHttp {
    client: reqwest::Client,
    runtime: tokio::runtime::Runtime,
}

impl NativeHttp {
    pub fn new(environment: &crate::config::Environment) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| Failure::message(e.to_string()))?;
        let client = {
            let _entered = runtime.enter();
            let custom = environment
                .get("REQUESTS_CA_BUNDLE")
                .filter(|v| !v.is_empty())
                .or_else(|| environment.get("CURL_CA_BUNDLE").filter(|v| !v.is_empty()));
            let roots=match custom {
                Some(path)=>std::fs::read(crate::pypath::encode_os(&path)).map_err(|_|Failure::message("error: the configured TLS CA bundle cannot be read; no Xero request was made."))?,
                None=>include_bytes!("../data/cacert.pem").to_vec(),
            };
            let certificates = reqwest::Certificate::from_pem_bundle(&roots)
                .map_err(|_| Failure::message("error: bundled Xero TLS roots are unreadable"))?;
            if certificates.is_empty() {
                return Err(Failure::message(
                    "error: the TLS CA bundle contains no certificates; no Xero request was made.",
                ));
            }
            let mut builder = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(30))
                .read_timeout(Duration::from_secs(30))
                .retry(reqwest::retry::never())
                .redirect(reqwest::redirect::Policy::limited(30))
                .http1_only()
                .tls_certs_only(certificates)
                .user_agent("python-requests/2.34.2");
            if environment.has_dotenv_proxy() {
                builder = builder.no_proxy();
                let bypass = environment
                    .get("no_proxy")
                    .or_else(|| environment.get("NO_PROXY"))
                    .and_then(|v| reqwest::NoProxy::from_string(&v));
                for (lower, upper, kind) in [
                    ("http_proxy", "HTTP_PROXY", 0),
                    ("https_proxy", "HTTPS_PROXY", 1),
                    ("all_proxy", "ALL_PROXY", 2),
                ] {
                    let value = environment.get(lower).or_else(|| {
                        if kind == 0 && environment.get("REQUEST_METHOD").is_some() {
                            None
                        } else {
                            environment.get(upper)
                        }
                    });
                    if let Some(value) = value.filter(|v| !v.is_empty()) {
                        let proxy=match kind {0=>reqwest::Proxy::http(&value),1=>reqwest::Proxy::https(&value),_=>reqwest::Proxy::all(&value)}.map_err(|_|Failure::message("error: a configured HTTP proxy is invalid; no Xero request was made."))?;
                        builder = builder.proxy(proxy.no_proxy(bypass.clone()));
                    }
                }
            }
            builder.build().map_err(|e| {
                Failure::message(format!(
                    "error: could not initialise the Xero HTTP client ({e})"
                ))
            })?
        };
        Ok(Self { client, runtime })
    }

    fn send(&self, builder: reqwest::RequestBuilder) -> Result<Response> {
        self.runtime.block_on(async {
            let response = builder.send().await.map_err(transport)?;
            let status = response.status().as_u16();
            let retry_after = response
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let body = response.text().await.map_err(transport)?;
            Ok(Response {
                status,
                body,
                retry_after,
            })
        })
    }
}

fn transport(error: reqwest::Error) -> Failure {
    Failure::message(format!(
        "error: could not reach Xero ({error}) - re-run later."
    ))
}

impl Http for NativeHttp {
    fn post(
        &self,
        url: &str,
        credentials: &Credentials,
        form: &[(&str, &str)],
    ) -> Result<Response> {
        self.send(
            self.client
                .post(url)
                .basic_auth(&credentials.id, Some(&credentials.secret))
                .form(form),
        )
    }
    fn get(
        &self,
        url: &str,
        access_token: &str,
        tenant_id: Option<&str>,
        query: &[(&str, &str)],
    ) -> Result<Response> {
        let mut request = self
            .client
            .get(url)
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .query(query);
        if let Some(tenant) = tenant_id.filter(|s| !s.is_empty()) {
            request = request.header("Xero-tenant-id", tenant);
        }
        self.send(request)
    }
}

fn status(response: &Response, url: &str) -> Result<()> {
    if response.status < 400 {
        return Ok(());
    }
    Err(Failure::message(format!(
        "error: Xero returned HTTP {} for {url}.",
        response.status
    )))
}

pub fn exchange(
    http: &impl Http,
    credentials: &Credentials,
    code: &str,
    redirect: &str,
) -> Result<Json> {
    let response = http.post(
        TOKEN_URL,
        credentials,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect),
        ],
    )?;
    if response.status == 400 && response.body.contains("invalid_grant") {
        return Err(Failure::message(
            "Xero rejected the authorisation code (already used or expired). Nothing was saved. Run again and complete the consent promptly.",
        ));
    }
    if response.status == 401 {
        return Err(Failure::message(
            "Xero rejected this app's credentials when exchanging the authorisation code (HTTP 401). Check XERO_CLIENT_ID and XERO_CLIENT_SECRET in .env against the app at developer.xero.com. Nothing was saved.",
        ));
    }
    status(&response, TOKEN_URL)?;
    pyjson::loads_response(&response.body).map_err(|_| {
        Failure::message("error: Xero returned a non-JSON token response. Nothing was saved.")
    })
}

pub fn refresh(http: &impl Http, credentials: &Credentials, refresh_token: &str) -> Result<Json> {
    let response = http.post(
        TOKEN_URL,
        credentials,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ],
    )?;
    if response.status == 400 && response.body.contains("invalid_grant") {
        return Err(Failure::message(
            "Refresh token rejected (already used or expired). Re-authorise with: xero-tb-auth",
        ));
    }
    if response.status == 401 {
        return Err(Failure::message(
            "Xero rejected this app's credentials when refreshing the token (HTTP 401). Check XERO_CLIENT_ID and XERO_CLIENT_SECRET in .env against the app at developer.xero.com; token.json was left as it was.",
        ));
    }
    status(&response, TOKEN_URL)?;
    pyjson::loads_response(&response.body).map_err(|_| Failure::message("error: Xero returned a non-JSON token response; the existing token cache was left untouched."))
}

pub fn retry_after(value: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> u64 {
    let Some(value) = value else {
        return 5;
    };
    let value = crate::pytext::strip(value);
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        let digits = value.trim_start_matches('0');
        if digits.len() > 5 {
            return 86400;
        }
        return digits.parse::<u64>().unwrap_or(0).min(86400);
    }
    let parsed = crate::http_date::parse(value);
    match parsed {
        Some(time) => {
            let milliseconds = time.timestamp_millis() - now.timestamp_millis();
            if milliseconds <= 0 {
                0
            } else {
                (milliseconds as u64).div_ceil(1000).min(86400)
            }
        }
        None => 5,
    }
}

pub fn api_get(
    http: &impl Http,
    session: &TokenSession,
    credentials: &Credentials,
    url: &str,
    tenant: Option<&str>,
    query: &[(&str, &str)],
) -> Result<Json> {
    let token = |forced| {
        session.access_token(forced, |refresh_token| {
            refresh(http, credentials, refresh_token)
        })
    };
    let mut access = token(false)?;
    let (mut refreshed, mut waited, mut wait) = (false, false, 0);
    loop {
        let response = http.get(url, &access, tenant, query)?;
        if response.status == 429 {
            if waited {
                return Err(Failure::message(format!(
                    "error: Xero is still rate limiting after a {wait}s wait - re-run later."
                )));
            }
            wait = retry_after(response.retry_after.as_deref(), chrono::Utc::now());
            if wait > 60 {
                let asked = crate::amount::shown(response.retry_after.as_deref().unwrap_or("None"));
                let reset = (chrono::Local::now() + chrono::Duration::seconds(wait as i64))
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
                return Err(Failure::message(format!(
                    "error: Xero rate limit hit and sent Retry-After: {asked}, over the 60s cap this script will sleep for. The limit resets at or after {reset} - re-run after that (computed from the clamped {wait}s wait; the real reset may be later)."
                )));
            }
            std::thread::sleep(Duration::from_secs(wait));
            waited = true;
            continue;
        }
        if response.status == 401 {
            if refreshed {
                return Err(Failure::message(
                    "Xero rejected the access token even after a forced refresh. Re-authorise with: xero-tb-auth",
                ));
            }
            access = token(true)?;
            refreshed = true;
            continue;
        }
        status(&response, url)?;
        return pyjson::loads_response(&response.body)
            .map_err(|_| Failure::message("error: Xero returned a non-JSON API response."));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Scratch;
    use std::{cell::RefCell, collections::VecDeque};
    struct Scripted {
        replies: RefCell<VecDeque<Response>>,
        posts: RefCell<Vec<String>>,
        gets: RefCell<Vec<String>>,
        session: TokenSession,
    }
    fn pair(access: &str, refresh: &str) -> Json {
        Json::object([
            ("access_token", Json::str(access)),
            ("refresh_token", Json::str(refresh)),
            ("expires_in", Json::int(1800)),
        ])
    }
    fn response(status: u16, body: &str, retry: Option<&str>) -> Response {
        Response {
            status,
            body: body.into(),
            retry_after: retry.map(str::to_owned),
        }
    }
    fn credentials() -> Credentials {
        Credentials {
            id: "synthetic-id".into(),
            secret: "synthetic-secret".into(),
        }
    }

    struct TokenEndpoint {
        response: RefCell<Option<Response>>,
        form: RefCell<Vec<(String, String)>>,
    }
    impl Http for TokenEndpoint {
        fn post(&self, url: &str, creds: &Credentials, form: &[(&str, &str)]) -> Result<Response> {
            assert_eq!(url, TOKEN_URL);
            assert_eq!(creds.id, "synthetic-id");
            assert_eq!(creds.secret, "synthetic-secret");
            *self.form.borrow_mut() = form
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            Ok(self
                .response
                .borrow_mut()
                .take()
                .expect("the token POST was replayed"))
        }
        fn get(&self, _: &str, _: &str, _: Option<&str>, _: &[(&str, &str)]) -> Result<Response> {
            panic!("a token exchange made a GET")
        }
    }

    #[test]
    fn token_endpoint_refusals_are_reported_without_replaying_single_use_inputs() {
        for authorization in [false, true] {
            for (status, body, message) in [
                (400, "invalid_grant", "rejected"),
                (401, "{}", "app's credentials"),
                (503, "{}", "HTTP 503"),
                (200, "not JSON", "non-JSON token response"),
            ] {
                let endpoint = TokenEndpoint {
                    response: RefCell::new(Some(response(status, body, None))),
                    form: RefCell::new(Vec::new()),
                };
                let result = if authorization {
                    exchange(
                        &endpoint,
                        &credentials(),
                        "synthetic-code",
                        "http://localhost:8400/callback",
                    )
                } else {
                    refresh(&endpoint, &credentials(), "synthetic-refresh")
                };
                let failure = result.unwrap_err();
                assert!(failure.stderr.contains(message));
                assert!(endpoint.response.borrow().is_none());
                let form = endpoint.form.borrow();
                assert!(
                    form.contains(&(
                        "grant_type".into(),
                        if authorization {
                            "authorization_code"
                        } else {
                            "refresh_token"
                        }
                        .into()
                    ))
                );
                assert!(
                    form.contains(&(
                        if authorization {
                            "code"
                        } else {
                            "refresh_token"
                        }
                        .into(),
                        if authorization {
                            "synthetic-code"
                        } else {
                            "synthetic-refresh"
                        }
                        .into()
                    ))
                );
                if authorization {
                    assert!(form.contains(&(
                        "redirect_uri".into(),
                        "http://localhost:8400/callback".into()
                    )));
                }
            }
        }
    }
    impl Scripted {
        fn new(path: &std::path::Path, replies: Vec<Response>) -> Self {
            let session = TokenSession::new(path).unwrap();
            session
                .save(&pair("synthetic-before-access", "synthetic-before-refresh"))
                .unwrap();
            Self {
                replies: RefCell::new(replies.into()),
                posts: RefCell::new(Vec::new()),
                gets: RefCell::new(Vec::new()),
                session,
            }
        }
    }
    impl Http for Scripted {
        fn post(&self, url: &str, creds: &Credentials, form: &[(&str, &str)]) -> Result<Response> {
            assert_eq!(url, TOKEN_URL);
            assert_eq!(creds.id, "synthetic-id");
            assert_eq!(creds.secret, "synthetic-secret");
            assert!(form.contains(&("grant_type", "refresh_token")));
            let refresh = form.iter().find(|(k, _)| *k == "refresh_token").unwrap().1;
            self.posts.borrow_mut().push(refresh.into());
            Ok(response(
                200,
                &pyjson::canonical(&pair("synthetic-after-access", "synthetic-after-refresh")),
                None,
            ))
        }
        fn get(
            &self,
            url: &str,
            access: &str,
            tenant: Option<&str>,
            query: &[(&str, &str)],
        ) -> Result<Response> {
            assert_eq!(url, REPORT_URL);
            assert_eq!(tenant, Some("synthetic-tenant"));
            assert_eq!(query, [("date", "2026-06-30")]);
            let persisted = self.session.load().unwrap();
            assert!(persisted.at("access_token").is_str(access));
            self.gets.borrow_mut().push(access.into());
            Ok(self
                .replies
                .borrow_mut()
                .pop_front()
                .expect("no extra HTTP call"))
        }
    }
    fn scripted_get(http: &Scripted) -> Result<Json> {
        api_get(
            http,
            &http.session,
            &credentials(),
            REPORT_URL,
            Some("synthetic-tenant"),
            &[("date", "2026-06-30")],
        )
    }
    #[test]
    fn retry_header_grammar_and_clamp() {
        let now = chrono::DateTime::from_timestamp(1780000000, 0).unwrap();
        for (text, expected) in [
            ("0", 0),
            ("000010", 10),
            ("1_0", 5),
            ("+7", 5),
            ("٧", 5),
            ("999999999999999999999999", 86400),
        ] {
            assert_eq!(retry_after(Some(text), now), expected);
        }
        assert_eq!(retry_after(None, now), 5);
    }
    #[test]
    fn one_rate_retry_and_one_forced_refresh_use_the_persisted_pair() {
        let scratch = Scratch::new();
        let http = Scripted::new(
            &scratch.0.join("token.json"),
            vec![
                response(429, "", Some("0")),
                response(401, "", None),
                response(200, "{\"ok\":true}", None),
            ],
        );
        assert!(matches!(
            scripted_get(&http).unwrap().at("ok"),
            Json::Bool(true)
        ));
        assert_eq!(*http.posts.borrow(), ["synthetic-before-refresh"]);
        assert_eq!(
            *http.gets.borrow(),
            [
                "synthetic-before-access",
                "synthetic-before-access",
                "synthetic-after-access"
            ]
        );
    }
    #[test]
    fn repeated_refusals_and_long_rate_limit_do_not_replay_forever() {
        let scratch = Scratch::new();
        for (index, replies, gets, posts) in [
            (
                0,
                vec![response(429, "", Some("0")), response(429, "", Some("0"))],
                2,
                0,
            ),
            (1, vec![response(429, "", Some("86400"))], 1, 0),
            (
                2,
                vec![response(401, "", None), response(401, "", None)],
                2,
                1,
            ),
        ] {
            let directory = scratch.0.join(index.to_string());
            std::fs::create_dir(&directory).unwrap();
            let http = Scripted::new(&directory.join("token.json"), replies);
            assert!(scripted_get(&http).is_err());
            assert_eq!(http.gets.borrow().len(), gets);
            assert_eq!(http.posts.borrow().len(), posts);
        }
    }
    #[test]
    fn native_request_carries_headers_query_and_basic_auth_without_replaying_a_post() {
        use std::{
            io::{BufRead, BufReader, Write},
            net::TcpListener,
        };
        let environment = crate::config::Environment::isolated();
        let http = NativeHttp::new(&environment).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..2 {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let (mut socket, _) = loop {
                    match listener.accept() {
                        Ok(socket) => break socket,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && std::time::Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) => panic!("synthetic HTTP accept: {e}"),
                    }
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                let mut lines = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    lines.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                let length = lines
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                std::io::Read::read_exact(&mut reader, &mut body).unwrap();
                requests.push(lines);
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").unwrap();
            }
            requests
        });
        let url = format!("http://{address}/trial");
        http.get(
            &url,
            "synthetic-access",
            Some("synthetic-tenant"),
            &[("date", "2026-06-30")],
        )
        .unwrap();
        http.post(
            &url,
            &credentials(),
            &[
                ("grant_type", "authorization_code"),
                ("code", "synthetic-code"),
            ],
        )
        .unwrap();
        let requests = server.join().unwrap();
        let get = requests[0].to_ascii_lowercase();
        assert!(get.starts_with("get /trial?date=2026-06-30 http/1.1\r\n"));
        assert!(get.contains("authorization: bearer synthetic-access\r\n"));
        assert!(get.contains("xero-tenant-id: synthetic-tenant\r\n"));
        assert!(get.contains("accept: application/json\r\n"));
        assert!(requests[1].contains("Basic c3ludGhldGljLWlkOnN5bnRoZXRpYy1zZWNyZXQ="));
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("content-type: application/x-www-form-urlencoded\r\n")
        );
    }
}
