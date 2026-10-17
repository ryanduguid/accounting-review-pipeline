//! Publish complete bytes, retaining recovery files after flush and rename failures.
use crate::{
    Failure, Result, paths,
    pyjson::{self, Json},
    pypath, report,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

pub fn append(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

pub fn manifest_path(path: &Path) -> PathBuf {
    append(path, ".manifest.json")
}

pub fn shown_path(path: &Path, quiet: bool) -> String {
    if quiet {
        pypath::text(
            &path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .join("<filename withheld under --quiet>"),
        )
    } else {
        pypath::text(path)
    }
}

pub fn temporary(directory: &Path, suffix: &str) -> Result<(PathBuf, File)> {
    loop {
        let path = directory.join(paths::random_name(suffix)?);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(Failure::message(format!(
                    "error: cannot write a temporary file in {} ({e})",
                    pypath::text(directory)
                )));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Write,
    Flush,
    Replace,
}

#[cfg(test)]
thread_local! {
    static FAILURE: std::cell::Cell<Option<(Stage,usize)>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_failure<T>(stage: Stage, action: impl FnOnce() -> T) -> T {
    with_failure_after(stage, 0, action)
}

#[cfg(test)]
pub(crate) fn with_failure_after<T>(stage: Stage, passes: usize, action: impl FnOnce() -> T) -> T {
    struct Reset(Option<(Stage, usize)>);
    impl Drop for Reset {
        fn drop(&mut self) {
            FAILURE.set(self.0);
        }
    }
    let _reset = Reset(FAILURE.replace(Some((stage, passes))));
    action()
}

#[cfg(test)]
fn test_failure(stage: Stage) -> std::io::Result<()> {
    if let Some((selected, passes)) = FAILURE.get()
        && selected == stage
    {
        if passes == 0 {
            return Err(std::io::Error::other("synthetic durable-save failure"));
        }
        FAILURE.set(Some((selected, passes - 1)));
    }
    Ok(())
}

pub fn durable_replace<F>(
    file: File,
    temporary: &Path,
    destination: &Path,
    payload: &[u8],
    error: F,
) -> Result<()>
where
    F: Fn(Stage, &std::io::Error) -> Failure,
{
    replace_with(file, temporary, destination, payload, error)
}

trait DurableFile: Write {
    fn sync(&self) -> std::io::Result<()>;
}
impl DurableFile for File {
    fn sync(&self) -> std::io::Result<()> {
        self.sync_all()
    }
}

fn replace_with<F>(
    mut file: impl DurableFile,
    temporary: &Path,
    destination: &Path,
    payload: &[u8],
    error: F,
) -> Result<()>
where
    F: Fn(Stage, &std::io::Error) -> Failure,
{
    let written = file.write_all(payload).and_then(|()| file.flush());
    #[cfg(test)]
    let written = written.and_then(|()| test_failure(Stage::Write));
    if let Err(e) = written {
        drop(file);
        let _ = fs::remove_file(temporary);
        return Err(error(Stage::Write, &e));
    }
    let synced = file.sync();
    #[cfg(test)]
    let synced = synced.and_then(|()| test_failure(Stage::Flush));
    if let Err(e) = synced {
        return Err(error(Stage::Flush, &e));
    }
    drop(file);
    let mut last = None;
    for attempt in 0..5 {
        #[cfg(test)]
        let replaced =
            test_failure(Stage::Replace).and_then(|()| fs::rename(temporary, destination));
        #[cfg(not(test))]
        let replaced = fs::rename(temporary, destination);
        match replaced {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
        if attempt < 4 {
            thread::sleep(Duration::from_millis(200 * (attempt + 1)));
        }
    }
    Err(error(
        Stage::Replace,
        &last.expect("failed replacement retained its error"),
    ))
}

pub fn write_csv(rows: &[report::OutputRow], path: &Path, quiet: bool) -> Result<String> {
    let payload = report::csv_bytes(rows)?;
    let digest: String = Sha256::digest(&payload)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let directory = path.parent().unwrap();
    fs::create_dir_all(directory).map_err(|e| {
        Failure::message(format!(
            "error: cannot create the output directory {} ({e}).",
            pypath::text(directory)
        ))
    })?;
    let (temp, file) = temporary(directory, ".csv.tmp").map_err(|e| {
        Failure::message(format!(
            "{}; the report was fetched but nothing was written.",
            e.stderr.trim_end()
        ))
    })?;
    let error = |stage: Stage, e: &std::io::Error| {
        let target = shown_path(path, quiet);
        let recovery = pypath::text(&temp);
        match stage {
            Stage::Write => Failure::message(format!(
                "error: could not write the export ({e}); nothing was written."
            )),
            Stage::Flush => Failure::message(format!(
                "error: wrote the balanced export to {recovery} but could not flush it to disk ({e}). The file is complete and balance-checked: rename {recovery} over {target} once the disk is writable - the report has been fetched already and re-running spends another refresh token."
            )),
            Stage::Replace => Failure::message(format!(
                "error: wrote the balanced export to {recovery} but could not move it onto {target} after 5 attempts ({e}). Close whatever holds {target} open (Excel or Power BI Desktop keep a lock on it), then rename {recovery} over it - the report has been fetched already and re-running spends another refresh token."
            )),
        }
    };
    durable_replace(file, &temp, path, &payload, error)?;
    Ok(digest)
}

pub fn set_aside_manifest(path: &Path, quiet: bool) -> Result<Option<PathBuf>> {
    let manifest = manifest_path(path);
    if !paths::entry_exists(&manifest)? {
        return Ok(None);
    }
    let aside = append(&manifest, ".previous");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let reserve = if paths::entry_exists(&aside)? {
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "recovery path is already occupied",
        ))
    } else {
        options.open(&aside)
    };
    drop(reserve.map_err(|e| Failure::message(format!("error: cannot reserve the recovery path {} ({e}); the CSV and current manifest are unchanged. Inspect any existing recovery file before moving or removing it, then run again.", shown_path(&aside, quiet))))?);
    if let Err(e) = fs::rename(&manifest, &aside) {
        if let Err(cleanup) = fs::remove_file(&aside) {
            crate::errln!(
                "warning: the empty recovery reservation at {} could not be removed ({cleanup}); inspect it before running again.",
                shown_path(&aside, quiet)
            );
        }
        return Err(Failure::message(format!(
            "error: a manifest from an earlier export, {}, could not be set aside ({e}), so the CSV and current manifest are unchanged and the fetched report is not kept.",
            shown_path(&manifest, quiet)
        )));
    }
    Ok(Some(aside))
}

pub fn restore_manifest(aside: &Path, path: &Path, quiet: bool) {
    let manifest = manifest_path(path);
    if let Err(e) = fs::rename(aside, &manifest) {
        crate::errln!(
            "warning: the earlier manifest is at {} and could not be restored to {} ({e}); rename it back by hand. The earlier CSV is unchanged.",
            shown_path(aside, quiet),
            shown_path(&manifest, quiet)
        );
    }
}

pub fn discard_manifest(aside: &Path, quiet: bool) {
    if let Err(e) = fs::remove_file(aside) {
        crate::errln!(
            "warning: the earlier manifest set aside at {} could not be removed ({e}); delete it by hand. It describes a previous export.",
            shown_path(aside, quiet)
        );
    }
}

pub fn manifest(
    path: &Path,
    digest: &str,
    tenant: &Json,
    date: &str,
    basis: &str,
    generated_at: &str,
) -> Json {
    Json::object([
        ("schema_version", Json::str("xero-source-manifest.v1")),
        ("mode", Json::str("live")),
        ("source_system", Json::str("xero-trial-balance-export")),
        (
            "entity_ref",
            Json::object([
                ("tenant_id", tenant.at("tenantId").clone()),
                ("tenant_name", tenant.at("tenantName").clone()),
            ]),
        ),
        (
            "report",
            Json::object([
                ("name", Json::str("Trial Balance")),
                ("as_at", Json::str(date)),
                ("basis", Json::str(basis)),
            ]),
        ),
        (
            "export",
            Json::object([
                ("schema", Json::str("xero-tb-csv.v1")),
                (
                    "csv",
                    Json::str(&pypath::decode_os(path.file_name().unwrap())),
                ),
                ("sha256", Json::str(digest)),
                ("generated_at", Json::str(generated_at)),
            ]),
        ),
    ])
}

pub fn write_manifest(
    path: &Path,
    digest: &str,
    tenant: &Json,
    date: &str,
    basis: &str,
    quiet: bool,
) -> Result<PathBuf> {
    let generated_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
    let payload = format!(
        "{}\n",
        pyjson::pretty(&manifest(path, digest, tenant, date, basis, &generated_at))
    )
    .into_bytes();
    let manifest = manifest_path(path);
    let directory = path.parent().unwrap();
    let (temp, file) = temporary(directory, ".manifest.json.tmp").map_err(|e| Failure::message(format!("error: wrote {} but cannot write a temporary manifest in {} ({}); the export is complete and its SHA-256 is {digest}.", shown_path(path, quiet), pypath::text(directory), e.stderr.trim())))?;
    let error = |stage: Stage, e: &std::io::Error| {
        let recovery = pypath::text(&temp);
        let target = shown_path(&manifest, quiet);
        let written = shown_path(path, quiet);
        match stage {
            Stage::Write => Failure::message(format!(
                "error: could not write the manifest ({e}); the export is complete."
            )),
            Stage::Flush => Failure::message(format!(
                "error: wrote {written} and its manifest to {recovery} but could not flush the manifest to disk ({e}). Rename {recovery} over {target} once the disk is writable; the export is complete."
            )),
            Stage::Replace => Failure::message(format!(
                "error: wrote {written} and its manifest to {recovery} but could not move it onto {target} after 5 attempts ({e}). Rename {recovery} over it; the export is complete."
            )),
        }
    };
    durable_replace(file, &temp, &manifest, &payload, error)?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Scratch;
    struct Fault {
        file: File,
        stage: Stage,
    }
    impl Write for Fault {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.stage == Stage::Write {
                self.file.write_all(&bytes[..bytes.len().min(3)])?;
                return Err(std::io::Error::other("synthetic write failure"));
            }
            self.file.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }
    impl DurableFile for Fault {
        fn sync(&self) -> std::io::Result<()> {
            if self.stage == Stage::Flush {
                Err(std::io::Error::other("synthetic fsync failure"))
            } else {
                self.file.sync_all()
            }
        }
    }
    fn diagnostic(stage: Stage, _: &std::io::Error) -> Failure {
        Failure::message(format!("{stage:?}"))
    }

    #[test]
    fn incomplete_write_is_deleted_and_complete_flush_failure_is_retained() {
        for stage in [Stage::Write, Stage::Flush] {
            let scratch = Scratch::new();
            let destination = scratch.0.join("tb.csv");
            fs::write(&destination, b"earlier balanced export").unwrap();
            let (temp, file) = temporary(&scratch.0, ".csv.tmp").unwrap();
            let result = replace_with(
                Fault { file, stage },
                &temp,
                &destination,
                b"new balanced export",
                diagnostic,
            )
            .unwrap_err();
            assert_eq!(result.stderr, format!("{stage:?}\n"));
            assert_eq!(fs::read(&destination).unwrap(), b"earlier balanced export");
            if stage == Stage::Write {
                assert!(!temp.exists());
            } else {
                assert_eq!(fs::read(&temp).unwrap(), b"new balanced export");
            }
        }
    }
    #[test]
    fn exhausted_replace_retains_complete_recovery_file() {
        let scratch = Scratch::new();
        let destination = scratch.0.join("blocked.csv");
        fs::create_dir(&destination).unwrap();
        let (temp, file) = temporary(&scratch.0, ".csv.tmp").unwrap();
        assert_eq!(
            durable_replace(
                file,
                &temp,
                &destination,
                b"complete balanced export",
                diagnostic
            )
            .unwrap_err()
            .stderr,
            "Replace\n"
        );
        assert_eq!(fs::read(&temp).unwrap(), b"complete balanced export");
        assert!(destination.is_dir());
    }
    #[test]
    fn parked_manifest_restores_without_overwriting_an_existing_recovery_file() {
        let scratch = Scratch::new();
        let csv = scratch.0.join("tb.csv");
        let manifest = manifest_path(&csv);
        fs::write(&manifest, b"earlier manifest").unwrap();
        let aside = set_aside_manifest(&csv, false).unwrap().unwrap();
        assert!(!manifest.exists());
        assert_eq!(fs::read(&aside).unwrap(), b"earlier manifest");
        restore_manifest(&aside, &csv, false);
        assert_eq!(fs::read(&manifest).unwrap(), b"earlier manifest");
        fs::write(&aside, b"reserved recovery").unwrap();
        assert!(set_aside_manifest(&csv, false).is_err());
        assert_eq!(fs::read(&manifest).unwrap(), b"earlier manifest");
        assert_eq!(fs::read(&aside).unwrap(), b"reserved recovery");
    }
}
