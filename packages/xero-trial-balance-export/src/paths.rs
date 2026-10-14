//! Destination and token-cache containment, including non-existing link tails.
use crate::{Failure, Result, pypath, pytext};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub fn resolved(path: &Path) -> Result<PathBuf> {
    pypath::resolve(path, false)
        .map_err(|_| Failure::message(format!("error: cannot resolve path {}", pypath::text(path))))
}

pub fn home() -> Result<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(key)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| Failure::message("error: cannot determine the per-user state directory"))
}

fn expand_home(path: &Path) -> Result<PathBuf> {
    let raw = pypath::decode_os(path.as_os_str());
    if raw == "~" {
        return home();
    }
    if raw.starts_with("~/") || (cfg!(windows) && raw.starts_with("~\\")) {
        return Ok(home()?.join(pypath::encode_os(&raw[2..])));
    }
    Ok(path.to_path_buf())
}

pub fn default_token_path() -> Result<PathBuf> {
    let base = if cfg!(windows) {
        home()?.join("AppData/Local")
    } else {
        home()?.join(".local/state")
    };
    Ok(base.join("xero-trial-balance-export/token.json"))
}

pub fn allowed_token_roots() -> Result<Vec<PathBuf>> {
    let install = std::env::current_exe().map_err(|e| Failure::message(e.to_string()))?;
    let install = install
        .parent()
        .ok_or_else(|| Failure::message("error: executable has no directory"))?;
    let cwd = std::env::current_dir().map_err(|e| Failure::message(e.to_string()))?;
    [
        home()?,
        default_token_path()?.parent().unwrap().to_path_buf(),
        cwd,
        std::env::temp_dir(),
        install.to_path_buf(),
    ]
    .iter()
    .map(|p| resolved(p))
    .collect()
}

pub fn safe_token_path(path: &Path) -> Result<PathBuf> {
    let candidate = resolved(&expand_home(path)?)?;
    if candidate.file_name() != Some(OsStr::new("token.json")) {
        return Err(Failure::message(
            "error: token cache path must be named token.json",
        ));
    }
    if allowed_token_roots()?
        .iter()
        .any(|root| candidate == *root || candidate.starts_with(root))
    {
        return Ok(candidate);
    }
    Err(Failure::message(
        "error: token cache path must stay under the home directory, the process working directory, the system temp directory, or the install directory.",
    ))
}

pub fn token_path(cli: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = cli.filter(|p| !pytext::strip(&pypath::decode_os(p.as_os_str())).is_empty())
    {
        return safe_token_path(path);
    }
    if let Some(value) = std::env::var_os("XERO_TOKEN_FILE")
        .filter(|v| !pytext::strip(&pypath::decode_os(v)).is_empty())
    {
        return safe_token_path(Path::new(&value));
    }
    let exe = std::env::current_exe().map_err(|e| Failure::message(e.to_string()))?;
    let legacy = exe.parent().unwrap().join("token.json");
    if legacy.is_file() {
        return safe_token_path(&legacy);
    }
    safe_token_path(&default_token_path()?)
}

pub fn output_path(requested: Option<&Path>, default: &str, cwd: &Path) -> Result<PathBuf> {
    let root = resolved(cwd)?;
    let candidate = resolved(&root.join(requested.unwrap_or_else(|| Path::new(default))))?;
    if !candidate
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("csv"))
    {
        return Err(Failure::message("--out must name a .csv file"));
    }
    if candidate != root && candidate.starts_with(&root) {
        return Ok(candidate);
    }
    Err(Failure::message(
        "--out must be a relative path beneath the current working directory",
    ))
}

pub fn entry_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(false)
        }
        Err(e) => Err(Failure::message(format!(
            "error: cannot inspect {} ({e}); export stopped.",
            pypath::text(path)
        ))),
    }
}

fn enclosing_checkout(directory: &Path) -> Result<Option<PathBuf>> {
    let mut current = directory.to_path_buf();
    loop {
        if entry_exists(&current.join(".git"))? {
            return Ok(Some(current));
        }
        if !current.pop() {
            return Ok(None);
        }
    }
}

fn ignored(checkout: &Path, target: &Path) -> Result<bool> {
    let relative = target
        .strip_prefix(checkout)
        .map_err(|_| Failure::message("error: output is outside the inspected checkout"))?;
    let mut child = Command::new("git").args(["check-ignore", "--quiet", "--"]).arg(relative).current_dir(checkout).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        .map_err(|e| Failure::message(format!("git could not be run to ask whether {} ignores the output ({e}), so this run cannot show the export would stay out of a commit.", pypath::text(checkout))))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| Failure::message(e.to_string()))?
        {
            return match status.code() {
                Some(0) => Ok(true),
                Some(1) => Ok(false),
                code => Err(Failure::message(format!(
                    "git check-ignore exited {} in {}, so this run cannot show whether a commit there would carry the output.",
                    code.unwrap_or(-1),
                    pypath::text(checkout)
                ))),
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Failure::message(
                "git could not be run to ask whether the checkout ignores the output (30-second timeout), so this run cannot show the export would stay out of a commit.",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn random_name(suffix: &str) -> Result<String> {
    Ok(format!(
        "tmp{}{suffix}",
        random_chars(8, b"abcdefghijklmnopqrstuvwxyz0123456789_")?
    ))
}

fn random_chars(length: usize, alphabet: &[u8]) -> Result<String> {
    let mut bytes = [0u8; 32];
    let ceiling = 256 / alphabet.len() * alphabet.len();
    let mut middle = String::new();
    while middle.len() < length {
        getrandom::fill(&mut bytes)
            .map_err(|_| Failure::message("error: secure random bytes unavailable"))?;
        for byte in bytes {
            if usize::from(byte) < ceiling && middle.len() < length {
                middle.push(alphabet[usize::from(byte) % alphabet.len()] as char);
            }
        }
    }
    Ok(middle)
}

pub fn require_ignored_output(
    output: &Path,
    manifest: bool,
    quiet: bool,
    representative: bool,
) -> Result<()> {
    let output = resolved(output)?;
    let directory = output.parent().unwrap();
    let Some(checkout) = enclosing_checkout(directory)? else {
        return Ok(());
    };
    let mut targets = vec![
        output.clone(),
        directory.join(random_name(".csv.tmp")?),
        directory.join(random_name(".csv.tmp")?),
    ];
    let sidecar = crate::publication::manifest_path(&output);
    if manifest || entry_exists(&sidecar)? {
        targets.extend([
            sidecar.clone(),
            crate::publication::append(&sidecar, ".previous"),
        ]);
    }
    if manifest {
        targets.extend([
            directory.join(random_name(".manifest.json.tmp")?),
            directory.join(random_name(".manifest.json.tmp")?),
        ]);
    }
    let mut unignored = Vec::new();
    for target in targets {
        if !ignored(&checkout, &target)? {
            unignored.push(pypath::decode_os(target.file_name().unwrap()));
        }
    }
    if unignored.is_empty() {
        return Ok(());
    }
    let written = if quiet {
        format!(
            "{} output file names, withheld under --quiet,",
            unignored.len()
        )
    } else {
        unignored.join(", ")
    };
    let note = if quiet {
        ""
    } else if representative {
        " The names are stand-ins: a default filename takes the organisation's name, and the staged files take a random one."
    } else {
        " The staged and parked names are stand-ins, because mkstemp chooses their random part."
    };
    Err(Failure::message(format!(
        "a run here would write {written} inside the version-control checkout at {}, and git ignores none of those.{note} An export holds a client's balances and its manifest names the organisation, so write it outside the checkout, or ignore the output directory there first.",
        pypath::text(&checkout)
    )))
}

pub fn require_output_directory(cwd: &Path, manifest: bool, quiet: bool) -> Result<()> {
    for _ in 0..2 {
        let filename = format!(
            "{}-{}-tb-2000-01-01-accrual.csv",
            random_chars(12, b"abcdefghijklmnopqrstuvwxyz")?,
            random_chars(8, b"abcdefghijklmnopqrstuvwxyz0123456789")?
        );
        require_ignored_output(&cwd.join(filename), manifest, quiet, true)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_and_cache_containment() {
        let cwd = std::env::temp_dir();
        assert!(output_path(Some(Path::new("sub/tb.csv")), "unused.csv", &cwd).is_ok());
        assert!(output_path(Some(Path::new("../tb.csv")), "unused.csv", &cwd).is_err());
        assert!(output_path(Some(Path::new("tb.txt")), "unused.csv", &cwd).is_err());
        assert!(safe_token_path(&cwd.join("other.json")).is_err());
    }

    #[test]
    fn git_guard_checks_csv_sidecar_parked_and_random_staging_names() {
        let scratch = crate::testing::Scratch::new();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .arg(&scratch.0)
                .status()
                .unwrap()
                .success()
        );
        let output = scratch.0.join("synthetic-organisation.csv");
        let refused = require_ignored_output(&output, true, true, false).unwrap_err();
        assert!(refused.stderr.contains("withheld under --quiet"));
        assert!(!refused.stderr.contains("synthetic-organisation"));
        std::fs::write(scratch.0.join(".gitignore"), "*.csv\n*.csv.tmp\n").unwrap();
        assert!(require_ignored_output(&output, false, false, false).is_ok());
        assert!(require_ignored_output(&output, true, false, false).is_err());
        std::fs::write(
            crate::publication::manifest_path(&output),
            "synthetic prior sidecar",
        )
        .unwrap();
        assert!(require_ignored_output(&output, false, false, false).is_err());
        std::fs::write(
            scratch.0.join(".gitignore"),
            "*.csv\n*.csv.tmp\n*.manifest.json\n*.manifest.json.previous\n*.manifest.json.tmp\n",
        )
        .unwrap();
        assert!(require_ignored_output(&output, true, false, false).is_ok());
        assert!(require_output_directory(&scratch.0, true, true).is_ok());
        let entries: Vec<_> = std::fs::read_dir(&scratch.0)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries.len(),
            3,
            "the preflight wrote output or staging files"
        );
    }

    #[test]
    fn invalid_checkout_marker_fails_closed() {
        let scratch = crate::testing::Scratch::new();
        std::fs::write(
            scratch.0.join(".git"),
            "synthetic invalid git directory marker",
        )
        .unwrap();
        assert!(
            require_ignored_output(&scratch.0.join("report.csv"), false, true, false)
                .unwrap_err()
                .stderr
                .contains("git check-ignore exited")
        );
    }

    #[cfg(unix)]
    #[test]
    fn output_rejects_directory_and_file_symlink_escapes() {
        let scratch = crate::testing::Scratch::new();
        let root = scratch.0.join("root");
        let outside = scratch.0.join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(outside.join("report.csv"), root.join("report.csv")).unwrap();
        for path in [
            "elsewhere/report.csv",
            "report.csv",
            "elsewhere/missing/../report.csv",
        ] {
            assert!(output_path(Some(Path::new(path)), "unused.csv", &root).is_err());
        }
    }
}
