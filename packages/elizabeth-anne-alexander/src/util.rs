//! Snapshots, JSON loading and the path guards (`util.py`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::pyjson::{self, Json, LoadError, PyStr};
use crate::pypath::{self, ResolveError};
use crate::pytext;

/// How a run ends early: a refusal (`GatewayError`, exit 2) or what CPython
/// would raise as an uncaught exception (exit 1).
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    Gateway(String),
    Crash(String),
}

pub type Result<T> = std::result::Result<T, Failure>;

pub fn refuse<T>(message: impl Into<String>) -> Result<T> {
    Err(Failure::Gateway(message.into()))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The display name of the embedded data root in messages; the Python
/// package printed its install directory here.
pub const BUNDLE_ROOT: &str = "<bundled>";

/// A path below the bundled data as CPython printed it: `root / rel`, with
/// backslashes on Windows.
pub fn bundled(rel: &str) -> String {
    let sep = if cfg!(windows) { "\\" } else { "/" };
    format!("{BUNDLE_ROOT}{sep}{}", rel.replace('/', sep))
}

/// `package_root / text` for an anchored path. On Windows a rooted path with
/// no drive takes the package's drive, which is the drive this binary runs from.
fn anchored(text: &str) -> String {
    let bare_root = text.starts_with(['/', '\\']) && !text.starts_with("\\\\") && !text.starts_with("//");
    if !cfg!(windows) || !bare_root {
        return text.to_string();
    }
    format!("{}{text}", package_drive())
}

/// `root / text` for an anchored `text` discards root, so CPython resolved the
/// absolute path itself; whatever it finds, it is outside the bundled data.
fn refuse_anchored<T>(text: &str, subdir: &str, label: &str) -> Result<T> {
    let text = anchored(text);
    let shown = pypath::display(&text);
    match pypath::resolve(Path::new(&pypath::encode_os(&text)), true) {
        Ok(_) => refuse(format!("{label} must stay within {}.", bundled(subdir))),
        Err(ResolveError::NotFound) => refuse(format!("{label} does not exist: {shown}.")),
        Err(ResolveError::Unusable) => refuse(format!("{label} is not a usable path: {shown}.")),
    }
}

/// The package's drive on Windows: the drive this binary runs from.
fn package_drive() -> String {
    // only the drive letter is used, to resolve rooted bundled paths as Python did; it grants nothing.
    // nosemgrep: rust.lang.security.current-exe.current-exe
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.to_str().and_then(|s| s.get(..2)).map(str::to_string))
        .filter(|d| d.ends_with(':'))
        .unwrap_or_else(|| "C:".to_string())
}

/// `ntpath.join(package_root, text)` keeps a drive-relative path on the
/// package's own drive relative to the package: "C:x.json" names x.json there.
fn on_package_drive(text: &str) -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let normal = text.replace('/', "\\");
    let (drive, root, rest) = pypath::splitroot(&normal);
    (root.is_empty() && !drive.is_empty() && drive.eq_ignore_ascii_case(&package_drive())).then(|| rest.to_string())
}

/// The policy and sample files shipped inside the binary, keyed by their
/// `/`-separated path below the package root (`samples/...`, `policy/...`).
#[derive(Clone)]
pub struct Bundle {
    pub files: BTreeMap<String, Vec<u8>>,
}

macro_rules! bundled {
    ($($path:literal),* $(,)?) => {
        [$(($path, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/elizabeth_anne_alexander/", $path)).as_slice())),*]
    };
}

impl Bundle {
    pub fn shipped() -> Bundle {
        let files = bundled!(
            "policy/demo-policy-v1.json",
            "samples/contexts/sample-monthly-variance.context.json",
            "samples/decisions/sample-review-decision.json",
            "samples/inputs/sample-tb-2026-05-31.csv",
            "samples/inputs/sample-tb-2026-06-30.csv",
            "samples/manifests/sample-tb-2026-05-31.manifest.json",
            "samples/manifests/sample-tb-2026-06-30.manifest.json",
            "samples/requests/sample-revenue-variance.request.json",
        );
        Bundle {
            files: files.into_iter().map(|(k, v)| (k.to_string(), v.to_vec())).collect(),
        }
    }

    /// Load `policy/` and `samples/` from a directory. Only the differential
    /// test build has this; release builds read nothing but the embedded data.
    #[cfg(feature = "differential")]
    pub fn from_dir(root: &Path) -> Bundle {
        fn walk(dir: &Path, rel: &str, files: &mut BTreeMap<String, Vec<u8>>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let key = format!("{rel}/{name}");
                if entry.path().is_dir() {
                    walk(&entry.path(), &key, files);
                } else if let Ok(bytes) = std::fs::read(entry.path()) {
                    files.insert(key, bytes);
                }
            }
        }
        let mut files = BTreeMap::new();
        for top in ["policy", "samples"] {
            walk(&root.join(top), top, &mut files);
        }
        Bundle { files }
    }

    /// The stored key for a normalised path, case-insensitively on Windows as the filesystem would.
    fn lookup(&self, rel: &str) -> Option<String> {
        if cfg!(windows) {
            let folded = rel.to_lowercase();
            let mut keys = self.files.keys().flat_map(|k| prefixes(k));
            keys.find(|k| k.to_lowercase() == folded)
        } else {
            self.files.keys().flat_map(|k| prefixes(k)).find(|k| k == rel)
        }
    }
}

/// A file path and every directory above it: "a/b/c" gives "a/b/c", "a/b", "a".
fn prefixes(key: &str) -> Vec<String> {
    let parts: Vec<&str> = key.split('/').collect();
    (1..=parts.len()).rev().map(|n| parts[..n].join("/")).collect()
}

/// Where a resolved input lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Location {
    Bundled(String),
    Disk(PathBuf),
}

impl Location {
    pub fn display(&self) -> String {
        match self {
            Location::Bundled(rel) => bundled(rel),
            Location::Disk(path) => pypath::text(path),
        }
    }
}

pub struct Env {
    pub bundle: Bundle,
    pub cwd: PathBuf,
}

impl Env {
    pub fn build_root(&self) -> PathBuf {
        self.cwd.join("build")
    }

    fn absolute(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        }
    }

    /// `path_within(path, build_root(), ...)` for a command-line path.
    pub fn within_build(&self, raw: &Path, label: &str, require_exists: bool) -> Result<PathBuf> {
        let root = pypath::resolve(&self.build_root(), false).unwrap_or_else(|_| self.build_root());
        let shown = pypath::display(&pypath::text(raw));
        let resolved = match pypath::resolve(&self.absolute(raw), require_exists) {
            Ok(resolved) => resolved,
            Err(ResolveError::NotFound) => return refuse(format!("{label} does not exist: {shown}.")),
            Err(ResolveError::Unusable) => return refuse(format!("{label} is not a usable path: {shown}.")),
        };
        if !pypath::is_within(&resolved, &root) {
            return refuse(format!("{label} must stay within {}.", pypath::text(&root)));
        }
        Ok(resolved)
    }

    /// `path_within(root / raw, root / subdir, ...)` against the bundled data.
    pub fn within_bundle(&self, raw: &PyStr, subdir: &str, label: &str) -> Result<Location> {
        // os.fsencode refuses NUL, and on Unix any surrogate that surrogateescape did not make.
        let encodable = |cp: &u32| cfg!(windows) || !(0xD800..=0xDFFF).contains(cp) || (0xDC80..=0xDCFF).contains(cp);
        let usable = !raw.0.contains(&0) && raw.0.iter().all(encodable);
        let Some(text) = usable.then(|| raw.os_text()) else {
            return refuse(format!("{label} is not a usable path: {}.", self.bundle_display(raw)));
        };
        let text = on_package_drive(&text).unwrap_or(text);
        if pypath::has_anchor(&text) {
            return refuse_anchored(&text, subdir, label);
        }
        let rel = self.walk(&text, label)?;
        if rel.is_empty() {
            // The package directory itself: it exists, outside every data subdirectory.
            return refuse(format!("{label} must stay within {}.", bundled(subdir)));
        }
        let Some(found) = self.bundle.lookup(&rel) else {
            return refuse(format!("{label} does not exist: {}.", bundled(&pypath::display(&text))));
        };
        if found != subdir && !found.starts_with(&format!("{subdir}/")) {
            return refuse(format!("{label} must stay within {}.", bundled(subdir)));
        }
        Ok(Location::Bundled(found))
    }

    /// The bundled key `text` names, walking its parts as `Path.resolve(strict=True)`
    /// does. POSIX visits every part in order, so a missing directory or a file
    /// used as one is refused even when a later ".." would leave it; Windows
    /// normalises ".." away before resolving.
    fn walk(&self, text: &str, label: &str) -> Result<String> {
        let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
        let missing = || refuse(format!("{label} does not exist: {}.", bundled(&pypath::display(text))));
        let steps: Vec<&str> = text
            .split(separators)
            .filter(|part| !matches!(*part, "" | "."))
            .collect();
        let mut parts: Vec<&str> = Vec::new();
        for (index, step) in steps.iter().enumerate() {
            if *step == ".." {
                // Above the package directory CPython found nothing to resolve.
                if parts.pop().is_none() {
                    return missing();
                }
                continue;
            }
            parts.push(step);
            if cfg!(not(windows)) {
                let Some(found) = self.bundle.lookup(&parts.join("/")) else {
                    return missing();
                };
                if index + 1 < steps.len() && self.bundle.files.contains_key(&found) {
                    return refuse(format!(
                        "{label} is not a usable path: {}.",
                        bundled(&pypath::display(text))
                    ));
                }
            }
        }
        Ok(parts.join("/"))
    }

    fn bundle_display(&self, raw: &PyStr) -> String {
        bundled(&pypath::display(&raw.os_text()))
    }
}

/// Text for a message: lone surrogates print as `\udXXX`, as Python's
/// backslashreplace error handler writes them.
pub fn render(text: &PyStr) -> String {
    text.0
        .iter()
        .map(|&cp| char::from_u32(cp).map_or_else(|| format!("\\u{cp:04x}"), String::from))
        .collect()
}

/// Immutable bytes and the digest calculated from that exact read.
pub struct FileSnapshot {
    pub location: Location,
    pub content: Vec<u8>,
    pub sha256: String,
}

pub fn snapshot(env: &Env, location: &Location, label: &str, missing_is_unreadable: bool) -> Result<FileSnapshot> {
    let shown = location.display();
    let content = match location {
        Location::Bundled(rel) => match env.bundle.files.get(rel) {
            Some(bytes) => bytes.clone(),
            // A bundled directory resolves but cannot be read, like IsADirectoryError.
            None => return refuse(format!("{label} cannot be read: {shown}.")),
        },
        Location::Disk(path) => match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !missing_is_unreadable => {
                return refuse(format!("{label} does not exist: {shown}."));
            }
            Err(_) => return refuse(format!("{label} cannot be read: {shown}.")),
        },
    };
    let sha256 = sha256_hex(&content);
    Ok(FileSnapshot {
        location: location.clone(),
        content,
        sha256,
    })
}

pub fn snapshot_file(env: &Env, location: &Location, label: &str) -> Result<FileSnapshot> {
    snapshot(env, location, label, true)
}

pub fn canonical_json(value: &Json) -> Vec<u8> {
    pyjson::canonical(value).into_bytes()
}

pub fn load_json_object_snapshot(env: &Env, location: &Location, label: &str) -> Result<(Json, FileSnapshot)> {
    let snapshot = snapshot(env, location, label, false)?;
    let shown = location.display();
    let Ok(text) = std::str::from_utf8(&snapshot.content) else {
        return refuse(format!("{label} is not valid UTF-8 text: {shown}."));
    };
    let raw = match pyjson::loads(text) {
        Ok(raw) => raw,
        Err(LoadError::Invalid) => return refuse(format!("{label} is not valid JSON: {shown}.")),
        Err(LoadError::DuplicateKey(key)) => {
            return refuse(format!(
                "{label} has duplicate key {}: {shown}.",
                pytext::repr_code_points(&key.0)
            ));
        }
        Err(LoadError::Crash(message)) => return Err(Failure::Crash(message)),
    };
    if !matches!(raw, Json::Object(_)) {
        return refuse(format!("{label} must be a JSON object: {shown}."));
    }
    Ok((raw, snapshot))
}

pub fn load_json_object(env: &Env, location: &Location, label: &str) -> Result<Json> {
    load_json_object_snapshot(env, location, label).map(|(raw, _)| raw)
}

pub fn load_json_exact_snapshot(
    env: &Env,
    location: &Location,
    required: &[&str],
    label: &str,
) -> Result<(Json, FileSnapshot)> {
    let (raw, snapshot) = load_json_object_snapshot(env, location, label)?;
    if !raw.has_exact_keys(required) {
        let mut sorted = required.to_vec();
        sorted.sort_unstable();
        return refuse(format!("{label} must contain exactly: {}.", sorted.join(", ")));
    }
    Ok((raw, snapshot))
}

pub fn load_json_exact(env: &Env, location: &Location, required: &[&str], label: &str) -> Result<Json> {
    load_json_exact_snapshot(env, location, required, label).map(|(raw, _)| raw)
}
