// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;
#[path = "../../../tests/support/directory_link.rs"]
mod directory_link;
#[cfg(target_os = "linux")]
#[path = "support/headless_auth.rs"]
mod headless_auth;
#[path = "support/https_proxy.rs"]
mod https_proxy;
#[path = "support/report_fixture.rs"]
mod report_fixture;
#[path = "support/scratch.rs"]
mod scratch;

use base64::Engine;
use https_proxy::{Proxy, Request};
use scratch::Scratch;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use xero_trial_balance_export::{
    export, publication,
    pyjson::{self, Json},
    tokens::TokenSession,
};

const EXPECTED_CSV: &str = concat!(
    "\u{feff}ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\r\n",
    "2026-06-30,Fabricated Organisation,Assets,00000000-0000-0000-0000-000000000001,Bank,090,1.0,0.0,1.0,0.0\r\n",
    "2026-06-30,Fabricated Organisation,Assets,00000000-0000-0000-0000-000000000002,Payables,800,0.0,1.0,0.0,1.0\r\n",
);

fn command(program: &str, directory: &Path) -> Command {
    let mut cmd = child_profile::command(program);
    cmd.current_dir(directory);
    for key in [
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "NO_PROXY",
        "no_proxy",
        "REQUESTS_CA_BUNDLE",
        "CURL_CA_BUNDLE",
        "XERO_CLIENT_ID",
        "XERO_CLIENT_SECRET",
        "XERO_TOKEN_FILE",
        "XERO_REDIRECT_URI",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("XERO_CLIENT_ID", "fabricated-client")
        .env("XERO_CLIENT_SECRET", "fabricated-client-secret")
        // A nonempty override prevents Windows registry proxy-bypass fallback.
        .env("NO_PROXY", "localhost,127.0.0.1");
    cmd
}

fn run(mut command: Command) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "native command deadline; stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn tenant() -> Json {
    Json::object([
        ("tenantId", Json::str("fabricated-tenant")),
        ("tenantName", Json::str("Fabricated Organisation")),
    ])
}

fn token_pair(access: &str, refresh: &str, expiry: i64) -> Json {
    Json::object([
        ("access_token", Json::str(access)),
        ("refresh_token", Json::str(refresh)),
        ("expires_in", Json::int(expiry)),
    ])
}

fn refresh_reply(request: Request) -> String {
    assert_eq!(
        (request.authority.as_str(), request.method.as_str()),
        ("identity.xero.com:443", "POST")
    );
    let basic = base64::engine::general_purpose::STANDARD
        .encode("fabricated-client:fabricated-client-secret");
    assert_eq!(request.headers["authorization"], format!("Basic {basic}"));
    let form: Vec<_> = reqwest::Url::parse(&format!(
        "http://fixture.invalid/?{}",
        String::from_utf8(request.body).unwrap()
    ))
    .unwrap()
    .query_pairs()
    .map(|(k, v)| (k.into_owned(), v.into_owned()))
    .collect();
    assert_eq!(
        form,
        [
            ("grant_type".into(), "refresh_token".into()),
            ("refresh_token".into(), "fabricated-refresh".into())
        ]
    );
    pyjson::canonical(&token_pair(
        "fabricated-rotated-access",
        "fabricated-rotated-refresh",
        1800,
    ))
}

fn service_reply(request: Request, cache: &Path, refresh: bool, cash: bool) -> String {
    if request.target == "/connect/token" {
        assert!(refresh);
        return refresh_reply(request);
    }
    assert_eq!(
        (request.authority.as_str(), request.method.as_str()),
        ("api.xero.com:443", "GET")
    );
    assert!(request.body.is_empty());
    let expected = if refresh {
        "fabricated-rotated-access"
    } else {
        "fabricated-access"
    };
    assert_eq!(
        request.headers["authorization"],
        format!("Bearer {expected}")
    );
    // Read the actual protected cache before returning the first API response.
    let saved = TokenSession::new(cache).unwrap().load().unwrap();
    assert!(saved.at("access_token").is_str(expected));
    if refresh {
        assert!(
            saved
                .at("refresh_token")
                .is_str("fabricated-rotated-refresh")
        );
    }
    if request.target == "/connections" {
        assert!(!request.headers.contains_key("xero-tenant-id"));
        return pyjson::canonical(&Json::Array(vec![tenant()]));
    }
    let suffix = if cash { "&paymentsOnly=true" } else { "" };
    assert_eq!(
        request.target,
        format!("/api.xro/2.0/Reports/TrialBalance?date=2026-06-30{suffix}")
    );
    assert_eq!(request.headers["xero-tenant-id"], "fabricated-tenant");
    pyjson::canonical(&Json::object([(
        "Reports",
        Json::Array(vec![report_fixture::balanced_report()]),
    )]))
}

fn assert_published(path: &Path, cash: bool) {
    let bytes = fs::read(path).unwrap();
    assert_eq!(bytes, EXPECTED_CSV.as_bytes());
    let manifest =
        pyjson::loads(&fs::read_to_string(publication::manifest_path(path)).unwrap()).unwrap();
    assert!(
        manifest
            .at("schema_version")
            .is_str("xero-source-manifest.v1")
    );
    assert!(manifest.at("mode").is_str("live"));
    let entity = manifest.at("entity_ref");
    assert!(entity.at("tenant_id").is_str("fabricated-tenant"));
    assert!(entity.at("tenant_name").is_str("Fabricated Organisation"));
    let report = manifest.at("report");
    assert!(report.at("name").is_str("Trial Balance"));
    assert!(report.at("as_at").is_str("2026-06-30"));
    assert!(
        report
            .at("basis")
            .is_str(if cash { "cash" } else { "accrual" })
    );
    assert!(manifest.at("export").at("csv").is_str("tb.csv"));
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert!(manifest.at("export").at("sha256").is_str(&digest));
}

fn export_command(
    directory: &Path,
    cache: &Path,
    path: &Path,
    proxy: &Proxy,
    cash: bool,
) -> Command {
    let mut cmd = command(env!("CARGO_BIN_EXE_export-tb"), directory);
    cmd.env("HTTPS_PROXY", format!("http://{}", proxy.address))
        .env(
            "REQUESTS_CA_BUNDLE",
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native-https/ca.pem"),
        )
        .args([
            "--date",
            "2026-06-30",
            "--tenant",
            "fabricated-tenant",
            "--token-file",
        ])
        .arg(cache)
        .arg("--out")
        .arg(path);
    if cash {
        cmd.arg("--payments-only");
    }
    cmd
}

fn native_export(refresh: bool, cash: bool) {
    let scratch = Scratch::new();
    let cache = scratch.0.join("token.json");
    let session = TokenSession::new(&cache).unwrap();
    session
        .save(&token_pair(
            "fabricated-access",
            "fabricated-refresh",
            if refresh { 1 } else { 1800 },
        ))
        .unwrap();
    let before = fs::read(&cache).unwrap();
    let server_cache = cache.clone();
    let proxy = Proxy::new(if refresh { 3 } else { 2 }, move |request| {
        service_reply(request, &server_cache, refresh, cash)
    });
    let path = scratch.0.join("export").join("tb.csv");
    let output = run(export_command(&scratch.0, &cache, &path, &proxy, cash));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    proxy.finish();
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(stdout.starts_with("Tenant: Fabricated Organisation\n"));
    assert!(stdout.contains("Wrote 2 accounts to "));
    assert!(stdout.contains("Balance check OK: movement debits = credits = 1.00; YTD = 1.00"));
    assert!(stdout.contains("Wrote manifest "));
    assert_published(&path, cash);
    if refresh {
        assert_ne!(fs::read(&cache).unwrap(), before);
        assert!(
            session
                .load()
                .unwrap()
                .at("refresh_token")
                .is_str("fabricated-rotated-refresh")
        );
    } else {
        assert_eq!(fs::read(&cache).unwrap(), before);
    }
}

#[test]
fn native_export_fetches_and_publishes_a_balanced_accrual_report_over_real_tls() {
    native_export(false, false);
}

fn dangling_link_target(scratch: &Path, name: &Path, spelling: &str) -> PathBuf {
    match spelling {
        // Windows TEMP can use an 8.3 spelling that the working directory expands.
        "absolute" => xero_trial_balance_export::paths::resolved(scratch)
            .unwrap()
            .join(name),
        "canonical-parent" | "long-canonical" => fs::canonicalize(scratch).unwrap().join(name),
        "relative" => PathBuf::from("future-exports"),
        _ => unreachable!(),
    }
}

#[test]
fn dangling_output_links_publish_only_beneath_the_working_directory() {
    for spelling in ["absolute", "canonical-parent", "relative", "long-canonical"] {
        let scratch = Scratch::new();
        let name = if spelling == "long-canonical" {
            // Two valid filename components exceed MAX_PATH even in a short temp root.
            PathBuf::from("fabricated-output-".repeat(9)).join("fabricated-output-".repeat(9))
        } else {
            PathBuf::from("future-exports")
        };
        let target = scratch.0.join(&name);
        let alias = scratch.0.join("linked");
        let link_target = dangling_link_target(&scratch.0, &name, spelling);
        if !directory_link::create(&link_target, &alias) {
            return;
        }
        assert!(!target.exists());
        let cache = scratch.0.join("token.json");
        let session = TokenSession::new(&cache).unwrap();
        session
            .save(&token_pair("fabricated-access", "fabricated-refresh", 1800))
            .unwrap();
        let before = fs::read(&cache).unwrap();
        let server_cache = cache.clone();
        let proxy = Proxy::new(2, move |request| {
            service_reply(request, &server_cache, false, false)
        });
        let output = run(export_command(
            &scratch.0,
            &cache,
            Path::new("linked/tb.csv"),
            &proxy,
            false,
        ));
        assert!(
            output.status.success(),
            "spelling={spelling} root={:?} link_target={link_target:?}: {output:?}",
            scratch.0
        );
        proxy.finish();
        assert!(output.stderr.is_empty());
        assert_published(&target.join("tb.csv"), false);
        assert_eq!(fs::read(cache).unwrap(), before);
        assert!(
            fs::symlink_metadata(alias)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

#[test]
fn dangling_output_links_outside_the_working_directory_are_refused() {
    let scratch = Scratch::new();
    let outside = Scratch::new();
    let target = outside.0.join("future-exports");
    if !directory_link::create(&target, &scratch.0.join("linked")) {
        return;
    }
    let mut cmd = command(env!("CARGO_BIN_EXE_export-tb"), &scratch.0);
    cmd.args(["--date", "2026-06-30", "--out", "linked/tb.csv"]);
    let output = run(cmd);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("--out must be a relative path beneath the current working directory")
    );
    assert!(!target.exists());
}

#[test]
fn native_export_persists_a_refresh_before_fetching_a_cash_report_over_real_tls() {
    native_export(true, true);
}

#[test]
fn explicit_short_help_value_is_refused_before_configuration_or_file_access() {
    let scratch = Scratch::new();
    let marker = scratch.0.join("fixture.txt");
    fs::write(&marker, b"fabricated unchanged input").unwrap();
    let mut cmd = command(env!("CARGO_BIN_EXE_export-tb"), &scratch.0);
    cmd.arg("-h=1");
    let output = run(cmd);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .trim_end_matches(['\r', '\n'])
            .ends_with("argument -h/--help: ignored explicit argument '1'")
    );
    assert_eq!(fs::read(&marker).unwrap(), b"fabricated unchanged input");
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
}

#[test]
fn native_authorisation_rejects_an_invalid_redirect_without_creating_a_cache() {
    let scratch = Scratch::new();
    let cache = scratch.0.join("token.json");
    let mut cmd = command(env!("CARGO_BIN_EXE_xero-tb-auth"), &scratch.0);
    cmd.env("XERO_TOKEN_FILE", &cache)
        .env("XERO_REDIRECT_URI", "https://localhost/callback");
    let output = run(cmd);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("Invalid XERO_REDIRECT_URI:")
    );
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 0);
}

#[test]
fn blocked_output_parent_preserves_the_existing_file() {
    let scratch = Scratch::new();
    let occupied = scratch.0.join("occupied");
    fs::write(&occupied, b"fabricated obstruction").unwrap();
    let modified = fs::metadata(&occupied).unwrap().modified().unwrap();
    let rows = export::validated_rows(
        &Json::object([(
            "Reports",
            Json::Array(vec![report_fixture::balanced_report()]),
        )]),
        "2026-06-30",
    )
    .unwrap();
    let error = export::publish(
        &rows,
        &tenant(),
        "2026-06-30",
        &occupied.join("tb.csv"),
        false,
        false,
        false,
    )
    .unwrap_err();
    assert_eq!(error.code, 1);
    assert!(error.stderr.contains("cannot create the output directory"));
    assert_eq!(fs::read(&occupied).unwrap(), b"fabricated obstruction");
    assert_eq!(
        fs::metadata(&occupied).unwrap().modified().unwrap(),
        modified
    );
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 1);
}

fn assert_blocked_manifest(scratch: &Scratch, path: &Path, sentinel: &Path) {
    let rows = export::validated_rows(
        &Json::object([(
            "Reports",
            Json::Array(vec![report_fixture::balanced_report()]),
        )]),
        "2026-06-30",
    )
    .unwrap();
    let error =
        export::publish(&rows, &tenant(), "2026-06-30", path, false, false, false).unwrap_err();
    assert_eq!(error.code, 1);
    assert!(error.stderr.contains("could not be set aside"));
    assert_eq!(fs::read(path).unwrap(), b"fabricated prior csv");
    assert_eq!(fs::read(sentinel).unwrap(), b"fabricated prior manifest");
    assert_eq!(fs::read_dir(&scratch.0).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn occupied_manifest_directory_keeps_both_prior_outputs_and_cleans_the_reservation() {
    let scratch = Scratch::new();
    let path = scratch.0.join("tb.csv");
    let manifest = publication::manifest_path(&path);
    fs::write(&path, b"fabricated prior csv").unwrap();
    fs::create_dir(&manifest).unwrap();
    let sentinel = manifest.join("sentinel");
    fs::write(&sentinel, b"fabricated prior manifest").unwrap();
    assert_blocked_manifest(&scratch, &path, &sentinel);
    assert_eq!(fs::read_dir(&manifest).unwrap().count(), 1);
}

#[cfg(windows)]
#[test]
fn locked_manifest_keeps_both_prior_outputs_and_cleans_the_reservation() {
    use std::os::windows::fs::OpenOptionsExt;
    let scratch = Scratch::new();
    let path = scratch.0.join("tb.csv");
    let manifest = publication::manifest_path(&path);
    fs::write(&path, b"fabricated prior csv").unwrap();
    fs::write(&manifest, b"fabricated prior manifest").unwrap();
    // Permit reads and writes but deny delete sharing, which also prevents rename.
    let _lock = fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(&manifest)
        .unwrap();
    assert_blocked_manifest(&scratch, &path, &manifest);
}

struct TokenLayout {
    root: Scratch,
    home: PathBuf,
    work: PathBuf,
    temp: PathBuf,
    install: PathBuf,
    outside: PathBuf,
    program: PathBuf,
}

impl TokenLayout {
    fn new() -> Self {
        let root = Scratch::new();
        let [home, work, temp, install, outside] =
            ["home", "work", "temp", "install", "outside"].map(|name| root.0.join(name));
        for directory in [&home, &work, &temp, &install, &outside] {
            fs::create_dir(directory).unwrap();
        }
        let original = Path::new(env!("CARGO_BIN_EXE_export-tb"));
        let program = install.join(original.file_name().unwrap());
        fs::copy(original, &program).unwrap();
        #[cfg(unix)]
        fs::set_permissions(&program, fs::metadata(original).unwrap().permissions()).unwrap();
        Self {
            root,
            home,
            work,
            temp,
            install,
            outside,
            program,
        }
    }

    fn command(&self) -> Command {
        let mut cmd = command(self.program.to_str().unwrap(), &self.work);
        cmd.env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("TMPDIR", &self.temp)
            .env("TMP", &self.temp)
            .env("TEMP", &self.temp)
            .args([
                "--date",
                "2026-06-30",
                "--tenant",
                "fabricated-tenant",
                "--out",
                "tb.csv",
            ]);
        cmd
    }

    fn caches(&self) -> [PathBuf; 4] {
        let state = if cfg!(windows) {
            "AppData/Local"
        } else {
            ".local/state"
        };
        [
            self.home.join(".xero/token.json"),
            self.home.join("environment/token.json"),
            self.install.join("token.json"),
            self.home
                .join(state)
                .join("xero-trial-balance-export/token.json"),
        ]
    }
}

#[derive(Clone, Copy)]
enum TokenSelection {
    Tilde,
    BlankFlag,
    Install,
    State,
}

fn seed_cache(path: &Path, access: &str) -> Vec<u8> {
    TokenSession::new(path)
        .unwrap()
        .save(&token_pair(access, "fabricated-refresh", 1800))
        .unwrap();
    fs::read(path).unwrap()
}

fn selection_command(layout: &TokenLayout, selection: TokenSelection, proxy: &Proxy) -> Command {
    let mut cmd = layout.command();
    cmd.env("HTTPS_PROXY", format!("http://{}", proxy.address))
        .env(
            "REQUESTS_CA_BUNDLE",
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native-https/ca.pem"),
        );
    match selection {
        TokenSelection::Tilde => {
            cmd.env("XERO_TOKEN_FILE", &layout.caches()[1])
                .args(["--token-file", "~/.xero/token.json"]);
        }
        TokenSelection::BlankFlag => {
            cmd.env("XERO_TOKEN_FILE", &layout.caches()[1])
                .arg("--token-file=");
        }
        TokenSelection::Install => {}
        TokenSelection::State => {
            cmd.env("XERO_TOKEN_FILE", " \t ");
        }
    }
    cmd
}

fn assert_token_selection(selection: TokenSelection) {
    let layout = TokenLayout::new();
    let caches = layout.caches();
    let selected = match selection {
        TokenSelection::Tilde => 0,
        TokenSelection::BlankFlag => 1,
        TokenSelection::Install => 2,
        TokenSelection::State => 3,
    };
    let before: Vec<_> = caches
        .iter()
        .enumerate()
        .filter(|(index, _)| selected != 3 || *index != 2)
        .map(|(index, path)| {
            let access = if index == selected {
                "fabricated-access"
            } else {
                "fabricated-decoy"
            };
            (path.clone(), seed_cache(path, access))
        })
        .collect();
    let server_cache = caches[selected].clone();
    let proxy = Proxy::new(2, move |request| {
        service_reply(request, &server_cache, false, false)
    });
    let output = run(selection_command(&layout, selection, &proxy));
    proxy.finish();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(stdout.starts_with("Tenant: Fabricated Organisation\n"));
    assert!(stdout.contains("Wrote 2 accounts to "));
    assert!(stdout.contains("Balance check OK: movement debits = credits = 1.00; YTD = 1.00"));
    assert!(stdout.contains("Wrote manifest "));
    assert_published(&layout.work.join("tb.csv"), false);
    for (path, bytes) in before {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    if selected == 3 {
        assert!(!caches[2].exists());
    }
}

#[test]
fn native_token_cache_selection_honours_flags_environment_install_and_state() {
    for selection in [
        TokenSelection::Tilde,
        TokenSelection::BlankFlag,
        TokenSelection::Install,
        TokenSelection::State,
    ] {
        assert_token_selection(selection);
    }
}

fn owned_inventory(directory: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        result.push(path.clone());
        if path.is_dir() {
            result.extend(owned_inventory(&path));
        }
    }
    result.sort();
    result
}

#[test]
fn native_token_cache_refusals_preserve_all_owned_files() {
    let layout = TokenLayout::new();
    let environment = layout.home.join("environment/token.json");
    let outside = layout.outside.join("token.json");
    let caches = [&environment, &outside].map(|path| (path, seed_cache(path, "fabricated-decoy")));
    let before = owned_inventory(&layout.root.0);
    for (flag, message) in [
        (
            outside.as_os_str(),
            "error: token cache path must stay under the home directory, the process working directory, the system temp directory, or the install directory.\n",
        ),
        (
            std::ffi::OsStr::new("~"),
            "error: token cache path must be named token.json\n",
        ),
    ] {
        let mut cmd = layout.command();
        cmd.env("XERO_TOKEN_FILE", &environment)
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .arg("--token-file")
            .arg(flag);
        let output = run(cmd);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr)
                .unwrap()
                .replace("\r\n", "\n"),
            message
        );
        assert_eq!(owned_inventory(&layout.root.0), before);
        for (path, bytes) in &caches {
            assert_eq!(fs::read(path).unwrap(), *bytes);
        }
    }
}
