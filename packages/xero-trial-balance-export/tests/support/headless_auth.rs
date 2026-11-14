// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Real Linux consent without a browser, remote account or forwarding proxy.

use super::*;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    process::Child,
    sync::mpsc::{self, Receiver},
    thread::JoinHandle,
};
use xero_trial_balance_export::auth;

const HEADLESS: &str =
    "Could not open a browser (SSH or headless session?). Paste this URL into one:\n";
const SAVED: &str = "Tokens saved to token.json. Next: export-tb --date 2026-06-30\n";

// Readers drain both pipes whilst consent is pending. Every exit, including an
// assertion panic, reaps the child before joining the readers or deleting files.
struct ConsentChild {
    child: Child,
    lines: Receiver<String>,
    stdout: Option<JoinHandle<Vec<u8>>>,
    stderr: Option<JoinHandle<Vec<u8>>>,
    deadline: Instant,
}

impl ConsentChild {
    fn start(mut command: Command) -> Self {
        let child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (sender, lines) = mpsc::channel();
        let mut running = Self {
            child,
            lines,
            stdout: None,
            stderr: None,
            deadline: Instant::now() + Duration::from_secs(40),
        };
        let mut stdout = BufReader::new(running.child.stdout.take().unwrap());
        running.stdout = Some(thread::spawn(move || {
            let mut bytes = Vec::new();
            loop {
                let mut line = String::new();
                if stdout.read_line(&mut line).unwrap() == 0 {
                    return bytes;
                }
                bytes.extend_from_slice(line.as_bytes());
                let _ = sender.send(line);
            }
        }));
        let mut stderr = running.child.stderr.take().unwrap();
        running.stderr = Some(thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).unwrap();
            bytes
        }));
        running
    }

    fn line(&self) -> String {
        self.lines
            .recv_timeout(self.deadline.saturating_duration_since(Instant::now()))
            .expect("consent command must print its URL before the shared deadline")
    }

    fn finish(mut self) -> Output {
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < self.deadline, "consent command deadline");
            thread::sleep(Duration::from_millis(10));
        };
        Output {
            status,
            stdout: self.stdout.take().unwrap().join().unwrap(),
            stderr: self.stderr.take().unwrap().join().unwrap(),
        }
    }
}

impl Drop for ConsentChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        for reader in [&mut self.stdout, &mut self.stderr] {
            if let Some(reader) = reader.take() {
                let _ = reader.join();
            }
        }
    }
}

fn contain(command: &mut Command, layout: &TokenLayout) {
    command
        .env("HOME", &layout.home)
        .env("USERPROFILE", &layout.home)
        .env("TMPDIR", &layout.temp)
        .env("TMP", &layout.temp)
        .env("TEMP", &layout.temp);
}

fn redirect() -> String {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    format!(
        "http://localhost:{}/callback",
        listener.local_addr().unwrap().port()
    )
}

fn auth_command(layout: &TokenLayout, redirect: &str, proxy: &Proxy) -> Command {
    let empty_path = layout.root.0.join("empty-path");
    fs::create_dir(&empty_path).unwrap();
    let mut cmd = command(env!("CARGO_BIN_EXE_xero-tb-auth"), &layout.work);
    contain(&mut cmd, layout);
    cmd.env("PATH", &empty_path)
        .env("XERO_TOKEN_FILE", layout.work.join("token.json"))
        .env("XERO_REDIRECT_URI", redirect)
        .env("HTTPS_PROXY", format!("http://{}", proxy.address))
        .env(
            "REQUESTS_CA_BUNDLE",
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native-https/ca.pem"),
        );
    cmd
}

fn consent_state(child: &ConsentChild, redirect: &str) -> String {
    assert_eq!(child.line(), HEADLESS);
    let printed = child.line();
    let url = reqwest::Url::parse(printed.trim()).unwrap();
    assert_eq!(url.origin().ascii_serialization(), "https://login.xero.com");
    assert_eq!(url.path(), "/identity/connect/authorize");
    assert!(url.fragment().is_none());
    let pairs: Vec<_> = url.query_pairs().collect();
    let query: BTreeMap<_, _> = pairs
        .iter()
        .map(|(k, v)| (k.as_ref(), v.as_ref()))
        .collect();
    assert_eq!(pairs.len(), 5);
    assert_eq!(query.len(), 5);
    assert_eq!(query["response_type"], "code");
    assert_eq!(query["client_id"], "fabricated-client");
    assert_eq!(query["redirect_uri"], redirect);
    assert_eq!(query["scope"], auth::SCOPES);
    let state = query["state"];
    assert_eq!(state.len(), 22);
    assert_eq!(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(state)
            .unwrap()
            .len(),
        16
    );
    state.into()
}

fn callback(redirect: &str, state: &str, field: &str, value: &str) {
    let mut url = reqwest::Url::parse(redirect).unwrap();
    url.query_pairs_mut()
        .extend_pairs([("state", state), (field, value)]);
    let address = (Ipv4Addr::LOCALHOST, url.port().unwrap()).into();
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "GET {}?{} HTTP/1.1\r\nHost: localhost\r\n\r\n",
        url.path(),
        url.query().unwrap()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.0 200 OK\r\n"), "{response}");
}

fn exchange_reply(request: Request, redirect: &str, complete: bool) -> String {
    assert_eq!(request.authority, "identity.xero.com:443");
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/connect/token");
    let basic = base64::engine::general_purpose::STANDARD
        .encode("fabricated-client:fabricated-client-secret");
    assert_eq!(request.headers["authorization"], format!("Basic {basic}"));
    assert_eq!(
        request.headers["content-type"],
        "application/x-www-form-urlencoded"
    );
    let url = reqwest::Url::parse(&format!(
        "http://fixture.invalid/?{}",
        String::from_utf8(request.body).unwrap()
    ))
    .unwrap();
    let form: Vec<_> = url.query_pairs().collect();
    assert_eq!(
        form,
        [
            ("grant_type".into(), "authorization_code".into()),
            ("code".into(), "fabricated+code & consent".into()),
            ("redirect_uri".into(), redirect.into()),
        ]
    );
    let response = if complete {
        token_pair("fabricated-access", "fabricated-refresh", 1800)
    } else {
        Json::object([("access_token", Json::str("fabricated-access"))])
    };
    pyjson::canonical(&response)
}

#[test]
fn native_headless_authorisation_saves_tokens_then_exports_a_balanced_report() {
    let layout = TokenLayout::new();
    let cache = layout.work.join("token.json");
    let redirect = redirect();
    let reply_redirect = redirect.clone();
    let proxy = Proxy::new(1, move |request| {
        exchange_reply(request, &reply_redirect, true)
    });
    let child = ConsentChild::start(auth_command(&layout, &redirect, &proxy));
    let state = consent_state(&child, &redirect);
    callback(&redirect, &state, "code", "fabricated+code & consent");
    let output = child.finish();
    proxy.finish();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with(HEADLESS));
    assert!(stdout.ends_with(SAVED));
    let saved = TokenSession::new(&cache).unwrap().load().unwrap();
    assert!(saved.at("access_token").is_str("fabricated-access"));
    assert!(saved.at("refresh_token").is_str("fabricated-refresh"));
    let before = fs::read(&cache).unwrap();
    let server_cache = cache.clone();
    let proxy = Proxy::new(2, move |request| {
        service_reply(request, &server_cache, false, false)
    });
    let path = layout.work.join("export/tb.csv");
    let mut cmd = export_command(&layout.work, &cache, &path, &proxy, false);
    contain(&mut cmd, &layout);
    let output = run(cmd);
    proxy.finish();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Balance check OK: movement debits = credits = 1.00; YTD = 1.00")
    );
    assert_published(&path, false);
    assert_eq!(fs::read(cache).unwrap(), before);
}

#[test]
fn native_headless_authorisation_refusals_preserve_the_existing_cache() {
    let mut previous_state = String::new();
    for denied in [false, true] {
        let layout = TokenLayout::new();
        let cache = layout.work.join("token.json");
        let before = seed_cache(&cache, "fabricated-prior-access");
        let redirect = redirect();
        let proxy = Proxy::new(0, |_| panic!("refused callback must not exchange tokens"));
        let cmd = auth_command(&layout, &redirect, &proxy);
        let inventory = owned_inventory(&layout.root.0);
        let child = ConsentChild::start(cmd);
        let state = consent_state(&child, &redirect);
        assert_ne!(state, previous_state);
        previous_state = state.clone();
        let message = if denied {
            callback(&redirect, &state, "error", "access_denied");
            "Xero returned 'access_denied' - consent was denied or cancelled. Run again.\n"
        } else {
            callback(
                &redirect,
                "fabricated-stale-state",
                "code",
                "fabricated+code & consent",
            );
            "State mismatch - possible CSRF or stale callback. Run again.\n"
        };
        let output = child.finish();
        proxy.finish();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(String::from_utf8(output.stderr).unwrap(), message);
        assert!(!String::from_utf8(output.stdout).unwrap().contains(SAVED));
        assert_eq!(fs::read(cache).unwrap(), before);
        assert_eq!(owned_inventory(&layout.root.0), inventory);
    }
}

#[test]
fn native_headless_authorisation_rejects_an_incomplete_token_pair() {
    let layout = TokenLayout::new();
    let cache = layout.work.join("token.json");
    let before = seed_cache(&cache, "fabricated-prior-access");
    let redirect = redirect();
    let reply_redirect = redirect.clone();
    let proxy = Proxy::new(1, move |request| {
        exchange_reply(request, &reply_redirect, false)
    });
    let cmd = auth_command(&layout, &redirect, &proxy);
    let inventory = owned_inventory(&layout.root.0);
    let child = ConsentChild::start(cmd);
    let state = consent_state(&child, &redirect);
    callback(&redirect, &state, "code", "fabricated+code & consent");
    let output = child.finish();
    proxy.finish();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "error: Xero token response has no usable refresh_token; no token was saved.\n"
    );
    assert!(!String::from_utf8(output.stdout).unwrap().contains(SAVED));
    assert_eq!(fs::read(cache).unwrap(), before);
    assert_eq!(owned_inventory(&layout.root.0), inventory);
}
