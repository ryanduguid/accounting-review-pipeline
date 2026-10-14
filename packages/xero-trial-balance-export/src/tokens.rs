//! One owner for the locked load, migration, refresh and durable-save transaction.
use crate::{
    Failure, Result, cache_json, dpapi, paths,
    publication::{self, Stage},
    pyjson::{self, Json, PyStr},
    pytext, report,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use num_traits::ToPrimitive;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FORMAT: &str = "xero-trial-balance-export-token-cache";
const PROTECTION: &str = "windows-dpapi-current-user";

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

pub fn number(value: &Json) -> Option<f64> {
    match value {
        Json::Int(n) => n.to_f64(),
        Json::Float(n) => Some(*n),
        _ => None,
    }
}

fn expires_in(value: Option<&Json>) -> Option<i64> {
    value
        .and_then(Json::as_plain_int)
        .and_then(ToPrimitive::to_i64)
        .filter(|n| (1..=86400).contains(n))
}

fn require_pair(payload: &Json, label: &str) -> Result<()> {
    if !matches!(payload, Json::Object(_)) {
        return Err(Failure::message(format!(
            "error: {label} is not a JSON object; no token was saved."
        )));
    }
    for key in ["access_token", "refresh_token"] {
        if !payload
            .get(key)
            .and_then(Json::as_pystr)
            .is_some_and(|s| !pytext::strip(&s.os_text()).is_empty())
        {
            return Err(Failure::message(format!(
                "error: {label} has no usable {key}; no token was saved."
            )));
        }
    }
    Ok(())
}

pub fn set(payload: &mut Json, key: &str, value: Json) {
    let Json::Object(pairs) = payload else {
        panic!("object checked by caller");
    };
    if let Some((_, old)) = pairs.iter_mut().find(|(name, _)| name.is(key)) {
        *old = value;
    } else {
        pairs.push((PyStr::from(key), value));
    }
}

pub fn validate_rotated(payload: &Json) -> Result<(Json, String)> {
    require_pair(payload, "Xero token response")?;
    let mut tokens = payload.clone();
    let mut warning = String::new();
    if expires_in(payload.get("expires_in")).is_none() {
        let shown = match payload.get("expires_in") {
            Some(Json::Str(s)) => pytext::repr_code_points(&s.0),
            Some(v) => report::text(v),
            None => "None".into(),
        };
        warning = format!(
            "warning: Xero token response had an unusable expires_in ({shown}); treating the access token as valid for 1800s.\n"
        );
        set(&mut tokens, "expires_in", Json::int(1800));
    }
    Ok((tokens, warning))
}

pub fn validate_cached(payload: &Json, label: &str, cached: bool) -> Result<()> {
    require_pair(payload, label)?;
    if expires_in(payload.get("expires_in")).is_none() {
        return Err(Failure::message(format!(
            "error: {label} has an invalid expires_in; no token was saved."
        )));
    }
    if cached
        && !payload
            .get("obtained_at")
            .and_then(number)
            .is_some_and(|n| (0.0..100_000_000_000.0).contains(&n))
    {
        return Err(Failure::message(
            "error: token.json has an invalid obtained_at; run: xero-tb-auth",
        ));
    }
    Ok(())
}

fn parse_document(raw: &[u8], label: &str) -> Result<Json> {
    let value = std::str::from_utf8(raw)
        .ok()
        .and_then(|s| pyjson::loads(s).ok());
    value.ok_or_else(|| Failure::message(format!("error: {label} is unreadable or corrupt; the file was left unchanged. Run: xero-tb-auth")))
}

pub fn encode_cache(tokens: &Json) -> Result<Vec<u8>> {
    if !cfg!(windows) {
        return cache_json::dumps(tokens, true, false, false);
    }
    let plaintext = cache_json::dumps(tokens, false, true, false)?;
    let protected = dpapi::protect(&plaintext)?;
    let envelope = Json::object([
        ("format", Json::str(FORMAT)),
        ("version", Json::int(1)),
        ("protection", Json::str(PROTECTION)),
        ("payload", Json::str(&STANDARD.encode(protected))),
    ]);
    cache_json::dumps(&envelope, true, false, true)
}

pub fn decode_cache(raw: &[u8]) -> Result<(Json, bool)> {
    let document = parse_document(raw, "token.json")?;
    if !matches!(document, Json::Object(_)) {
        return Err(Failure::message(
            "error: token.json is not a JSON object; the file was left unchanged. Run: xero-tb-auth",
        ));
    }
    if document.get("format").is_none() {
        return Ok((document, true));
    }
    if !document.has_exact_keys(&["format", "version", "protection", "payload"]) {
        return Err(Failure::message(
            "error: token.json has an invalid or unknown cache envelope; the file was left unchanged. Run: xero-tb-auth",
        ));
    }
    if !document.at("format").is_str(FORMAT)
        || document.at("version").as_plain_int() != Some(&num_bigint::BigInt::from(1))
        || !document.at("protection").is_str(PROTECTION)
        || document.at("payload").as_pystr().is_none()
    {
        return Err(Failure::message(
            "error: token.json uses an unsupported cache format, version or protection method; the file was left unchanged. Run: xero-tb-auth",
        ));
    }
    if !cfg!(windows) {
        return Err(Failure::message(
            "error: token.json is protected by Windows DPAPI and can only be opened by the Windows user who created it; the file was left unchanged. Run xero-tb-auth under that Windows account.",
        ));
    }
    let encoded = document.at("payload").as_pystr().and_then(PyStr::to_utf8).ok_or_else(|| Failure::message("error: token.json has a corrupt DPAPI payload; the file was left unchanged. Run: xero-tb-auth"))?;
    let ciphertext = STANDARD.decode(encoded).map_err(|_| Failure::message("error: token.json has a corrupt DPAPI payload; the file was left unchanged. Run: xero-tb-auth"))?;
    if ciphertext.is_empty() {
        return Err(Failure::message(
            "error: token.json has an empty DPAPI payload; the file was left unchanged. Run: xero-tb-auth",
        ));
    }
    let tokens = parse_document(
        &dpapi::unprotect(&ciphertext)?,
        "token.json's decrypted payload",
    )?;
    if !matches!(tokens, Json::Object(_)) {
        return Err(Failure::message(
            "error: token.json's decrypted payload is not a JSON object; the file was left unchanged. Run: xero-tb-auth",
        ));
    }
    Ok((tokens, false))
}

pub fn enforce_owner_only(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|e| Failure::message(format!("error: could not restrict plaintext token cache {} to mode 0600 ({e}); the cache was not used.", crate::pypath::text(path))))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

pub struct TokenSession {
    path: PathBuf,
    root: PathBuf,
}

impl TokenSession {
    pub fn new(path: &Path) -> Result<Self> {
        let path = paths::safe_token_path(path)?;
        let root = paths::allowed_token_roots()?.into_iter().find(|root| path != *root && path.starts_with(root)).ok_or_else(|| Failure::message("error: token cache path must stay under the home directory, the process working directory, the system temp directory, or the install directory."))?;
        Ok(Self { path, root })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn revalidate(&self) -> Result<PathBuf> {
        let path = paths::resolved(&self.path)?;
        if path.file_name() != Some(std::ffi::OsStr::new("token.json")) {
            return Err(Failure::message(
                "error: token cache path must be named token.json",
            ));
        }
        if path == self.root || !path.starts_with(&self.root) {
            return Err(Failure::message(
                "error: token cache path escaped its allowed directory",
            ));
        }
        Ok(path)
    }

    pub fn lock(&self) -> Result<File> {
        self.lock_with_budget(Duration::from_secs(90))
    }

    fn lock_with_budget(&self, budget: Duration) -> Result<File> {
        let path = self.revalidate()?;
        let lock = publication::append(&path, ".lock");
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| lock_error(&lock, &e))?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock)
            .map_err(|e| lock_error(&lock, &e))?;
        if file.metadata().map_err(|e| lock_error(&lock, &e))?.len() == 0 {
            file.write_all(&[0])
                .and_then(|()| file.flush())
                .map_err(|e| lock_error(&lock, &e))?;
        }
        let deadline = Instant::now() + budget;
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) => {}
                Err(std::fs::TryLockError::Error(e)) => return Err(lock_error(&lock, &e)),
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Failure::message(
                    "error: another process held the token cache lock for 90s; token.json was not read or changed.",
                ));
            }
            thread::sleep(remaining.min(Duration::from_millis(100)));
        }
    }

    fn load_unlocked(&self) -> Result<Json> {
        let path = self.revalidate()?;
        if !path.exists() {
            return Err(Failure::message("No token.json - run: xero-tb-auth"));
        }
        if !cfg!(windows) {
            enforce_owner_only(&path)?;
        }
        let raw = fs::read(&path).map_err(|e| Failure::message(format!("error: token.json could not be read ({e}); the file was left unchanged. Run: xero-tb-auth")))?;
        let (tokens, legacy) = decode_cache(&raw)?;
        validate_cached(&tokens, "token.json", true)?;
        if legacy && cfg!(windows) {
            self.persist(&tokens, true)?;
        }
        Ok(tokens)
    }

    pub fn load(&self) -> Result<Json> {
        let _lock = self.lock()?;
        self.load_unlocked()
    }

    fn persist(&self, tokens: &Json, migration: bool) -> Result<()> {
        let path = self.revalidate()?;
        let bytes = encode_cache(tokens)?;
        let (temporary, file) = publication::temporary(path.parent().unwrap(), ".tmp")?;
        if let Err(e) = enforce_owner_only(&temporary) {
            drop(file);
            let _ = fs::remove_file(&temporary);
            return Err(e);
        }
        let error = |stage: Stage, e: &std::io::Error| {
            let temp = crate::pypath::text(&temporary);
            let dest = crate::pypath::text(&path);
            match (migration, stage) {
                (_, Stage::Write) => Failure::message(format!(
                    "error: could not save the token cache ({e}); token.json was left unchanged."
                )),
                (true, Stage::Flush) => Failure::message(format!(
                    "error: wrote an encrypted replacement for the legacy token cache to {temp} but could not flush it to disk ({e}). {dest} was left unchanged and no Xero request was made. Delete the temp file and retry."
                )),
                (true, Stage::Replace) => Failure::message(format!(
                    "error: wrote an encrypted replacement for the legacy token cache to {temp} but could not move it onto {dest} after 5 attempts ({e}). The legacy cache was left unchanged and no Xero request was made. Delete the temp file and retry."
                )),
                (false, Stage::Flush) => Failure::message(format!(
                    "error: wrote the new Xero token pair to {temp} but could not flush it to disk ({e}). {dest} still holds the refresh token Xero has already consumed, so leave it alone: copy {temp} over {dest} within Xero's 30-minute rotation grace window, or run: xero-tb-auth"
                )),
                (false, Stage::Replace) => Failure::message(format!(
                    "error: wrote the new Xero token pair to {temp} but could not move it onto {dest} after 5 attempts ({e}). {dest} still holds the refresh token Xero has already consumed, so leave it alone: copy {temp} over {dest} within Xero's 30-minute rotation grace window, or run: xero-tb-auth"
                )),
            }
        };
        publication::durable_replace(file, &temporary, &path, &bytes, error)
    }

    pub fn save(&self, tokens: &Json) -> Result<()> {
        let _lock = self.lock()?;
        let mut saved = tokens.clone();
        set(&mut saved, "obtained_at", Json::Float(now()));
        self.persist(&saved, false)
    }

    pub fn access_token<F>(&self, force: bool, refresh: F) -> Result<String>
    where
        F: FnOnce(&str) -> Result<Json>,
    {
        let _lock = self.lock()?;
        let tokens = self.load_unlocked()?;
        let age = now() - number(tokens.at("obtained_at")).expect("timestamp checked");
        if !force
            && age >= -300.0
            && age < expires_in(tokens.get("expires_in")).expect("expiry checked") as f64 - 60.0
        {
            return Ok(report::text(tokens.at("access_token")));
        }
        let response = refresh(&report::text(tokens.at("refresh_token")))?;
        let (mut rotated, warning) = validate_rotated(&response)?;
        crate::err!("{warning}");
        set(&mut rotated, "obtained_at", Json::Float(now()));
        self.persist(&rotated, false)?;
        Ok(report::text(rotated.at("access_token")))
    }
}

fn lock_error(path: &Path, error: &std::io::Error) -> Failure {
    Failure::message(format!(
        "error: could not open the token cache lock {} ({error}); token.json was not read or changed.",
        crate::pypath::text(path)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Scratch;
    fn synthetic(expires: i64) -> Json {
        Json::object([
            ("access_token", Json::str("synthetic-access")),
            ("refresh_token", Json::str("synthetic-refresh")),
            ("expires_in", Json::int(expires)),
        ])
    }
    #[test]
    fn pair_and_cache_validation_are_distinct() {
        let value = Json::object([
            ("access_token", Json::str("synthetic-access")),
            ("refresh_token", Json::str("synthetic-refresh")),
            ("expires_in", Json::Bool(true)),
        ]);
        assert!(validate_cached(&value, "token.json", false).is_err());
        let (saved, warning) = validate_rotated(&value).unwrap();
        assert_eq!(
            saved.at("expires_in").as_plain_int(),
            Some(&num_bigint::BigInt::from(1800))
        );
        assert!(!warning.is_empty());
        assert!(decode_cache(br#"{"access_token":"a","access_token":"b"}"#).is_err());
    }
    #[test]
    fn lock_timeout_leaves_a_fabricated_unreadable_cache_unchanged() {
        let scratch = Scratch::new();
        let path = scratch.0.join("token.json");
        std::fs::write(&path, b"synthetic unreadable cache").unwrap();
        let session = TokenSession::new(&path).unwrap();
        let held = session.lock().unwrap();
        assert!(
            session
                .lock_with_budget(Duration::from_millis(1))
                .unwrap_err()
                .stderr
                .contains("another process held")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"synthetic unreadable cache");
        drop(held);
        assert!(session.lock_with_budget(Duration::from_millis(1)).is_ok());
    }
    #[test]
    fn saves_and_loads_fabricated_pairs_with_platform_protection() {
        let scratch = Scratch::new();
        let path = scratch.0.join("token.json");
        let session = TokenSession::new(&path).unwrap();
        session.save(&synthetic(1800)).unwrap();
        let loaded = session.load().unwrap();
        assert!(loaded.at("access_token").is_str("synthetic-access"));
        assert!(number(loaded.at("obtained_at")).unwrap() > 0.0);
        let bytes = fs::read(&path).unwrap();
        if cfg!(windows) {
            assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-access"));
        }
        assert_eq!(fs::read(publication::append(&path, ".lock")).unwrap(), [0]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn copied_plaintext_is_migrated_or_restricted_before_use_without_restamping() {
        let scratch = Scratch::new();
        let path = scratch.0.join("token.json");
        let mut tokens = synthetic(1800);
        set(&mut tokens, "obtained_at", Json::int(1));
        fs::write(
            &path,
            cache_json::dumps(&tokens, true, false, false).unwrap(),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        }
        let session = TokenSession::new(&path).unwrap();
        assert_eq!(number(session.load().unwrap().at("obtained_at")), Some(1.0));
        let (_, legacy) = decode_cache(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(legacy, !cfg!(windows));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn concurrent_expiry_refreshes_once_and_returns_the_persisted_pair() {
        use std::sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
        };
        let scratch = Scratch::new();
        let path = scratch.0.join("token.json");
        TokenSession::new(&path)
            .unwrap()
            .save(&synthetic(1))
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(3));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let calls = calls.clone();
                let barrier = barrier.clone();
                let path = path.clone();
                thread::spawn(move || {
                    barrier.wait();
                    TokenSession::new(&path)
                        .unwrap()
                        .access_token(false, |refresh| {
                            assert_eq!(refresh, "synthetic-refresh");
                            calls.fetch_add(1, Ordering::SeqCst);
                            thread::sleep(Duration::from_millis(40));
                            let mut next = synthetic(1800);
                            set(
                                &mut next,
                                "access_token",
                                Json::str("synthetic-rotated-access"),
                            );
                            set(
                                &mut next,
                                "refresh_token",
                                Json::str("synthetic-rotated-refresh"),
                            );
                            Ok(next)
                        })
                        .unwrap()
                })
            })
            .collect();
        barrier.wait();
        for worker in workers {
            assert_eq!(worker.join().unwrap(), "synthetic-rotated-access");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            TokenSession::new(&path)
                .unwrap()
                .load()
                .unwrap()
                .at("refresh_token")
                .is_str("synthetic-rotated-refresh")
        );
    }
    #[test]
    fn malformed_cache_and_unusable_rotation_leave_prior_bytes_unchanged() {
        let scratch = Scratch::new();
        let path = scratch.0.join("token.json");
        let session = TokenSession::new(&path).unwrap();
        session.save(&synthetic(1)).unwrap();
        let prior = fs::read(&path).unwrap();
        assert!(
            session
                .access_token(false, |_| Ok(Json::object([(
                    "access_token",
                    Json::str("orphaned-synthetic")
                )])))
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), prior);
        let malformed = b"{\"access_token\":\"synthetic-a\",\"access_token\":\"synthetic-b\"}";
        fs::write(&path, malformed).unwrap();
        assert!(session.load().is_err());
        assert_eq!(fs::read(&path).unwrap(), malformed);
    }

    struct Rotation {
        posts: std::cell::Cell<usize>,
        gets: std::cell::Cell<usize>,
    }
    impl crate::http::Http for Rotation {
        fn post(
            &self,
            _: &str,
            _: &crate::http::Credentials,
            _: &[(&str, &str)],
        ) -> Result<crate::http::Response> {
            self.posts.set(self.posts.get() + 1);
            let mut next = synthetic(1800);
            set(
                &mut next,
                "access_token",
                Json::str("synthetic-rotated-access"),
            );
            set(
                &mut next,
                "refresh_token",
                Json::str("synthetic-rotated-refresh"),
            );
            Ok(crate::http::Response {
                status: 200,
                body: pyjson::canonical(&next),
                retry_after: None,
            })
        }
        fn get(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: &[(&str, &str)],
        ) -> Result<crate::http::Response> {
            self.gets.set(self.gets.get() + 1);
            Ok(crate::http::Response {
                status: 200,
                body: "{}".into(),
                retry_after: None,
            })
        }
    }

    fn check_save_failure(migration: bool) {
        for stage in [Stage::Write, Stage::Flush, Stage::Replace] {
            let scratch = Scratch::new();
            let path = scratch.0.join("token.json");
            let session = TokenSession::new(&path).unwrap();
            if migration {
                let mut legacy = synthetic(1);
                set(&mut legacy, "obtained_at", Json::int(1));
                fs::write(
                    &path,
                    cache_json::dumps(&legacy, true, false, false).unwrap(),
                )
                .unwrap();
            } else {
                session.save(&synthetic(1)).unwrap();
            }
            let prior = fs::read(&path).unwrap();
            let http = Rotation {
                posts: std::cell::Cell::new(0),
                gets: std::cell::Cell::new(0),
            };
            let credentials = crate::http::Credentials {
                id: "synthetic-id".into(),
                secret: "synthetic-secret".into(),
            };
            let failure = publication::with_failure(stage, || {
                crate::http::api_get(
                    &http,
                    &session,
                    &credentials,
                    crate::http::REPORT_URL,
                    None,
                    &[],
                )
            })
            .unwrap_err();
            assert_eq!(http.posts.get(), usize::from(!migration));
            assert_eq!(http.gets.get(), 0);
            assert_eq!(fs::read(&path).unwrap(), prior);
            let recovery: Vec<_> = fs::read_dir(&scratch.0)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|p| p.extension().is_some_and(|e| e == "tmp"))
                .collect();
            assert_eq!(recovery.len(), usize::from(stage != Stage::Write));
            for temporary in recovery {
                let (saved, legacy) = decode_cache(&fs::read(temporary).unwrap()).unwrap();
                assert_eq!(legacy, !cfg!(windows));
                assert!(saved.at("access_token").is_str(if migration {
                    "synthetic-access"
                } else {
                    "synthetic-rotated-access"
                }));
                if migration {
                    assert_eq!(number(saved.at("obtained_at")), Some(1.0));
                } else {
                    assert!(number(saved.at("obtained_at")).unwrap() > 0.0);
                    assert!(
                        saved
                            .at("refresh_token")
                            .is_str("synthetic-rotated-refresh")
                    );
                }
            }
            if stage != Stage::Write {
                assert!(failure.stderr.contains(if migration {
                    "no Xero request was made"
                } else {
                    "30-minute rotation grace window"
                }));
            }
        }
    }

    #[test]
    fn rotation_save_failures_prevent_api_use_and_retain_only_complete_recovery() {
        check_save_failure(false);
    }

    #[cfg(windows)]
    #[test]
    fn migration_save_failures_prevent_refresh_and_api_use_without_restamping() {
        check_save_failure(true);
    }
}
