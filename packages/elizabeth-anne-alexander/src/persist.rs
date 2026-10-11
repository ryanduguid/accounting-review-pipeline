//! Stage and replace a synthetic evaluation pack (model, evidence, receipt),
//! and write a validation summary (`persist.py`).

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::pyjson::{self, Json};
use crate::pytext;
use crate::util::{Env, Failure, Location, Result, refuse};

/// `json.dumps(payload, indent=2, sort_keys=True) + "\n"` written in text
/// mode, which translates `\n` to `\r\n` on Windows.
fn text_mode_json(payload: &Json) -> Vec<u8> {
    let text = pyjson::pretty(payload) + "\n";
    if cfg!(windows) {
        text.replace('\n', "\r\n").into_bytes()
    } else {
        text.into_bytes()
    }
}

/// `str(OSError)`: `[Errno N] message: 'filename'` (with ` -> 'other'` for two paths).
pub fn os_error(error: &io::Error, filename: Option<&Path>, other: Option<&Path>) -> String {
    let code = error.raw_os_error();
    let mut message = error.to_string();
    if let Some(at) = message.rfind(" (os error ") {
        message.truncate(at);
    }
    if cfg!(windows) {
        message = message
            .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
            .to_string();
    }
    let tag = match code {
        Some(code) if cfg!(windows) => format!("[WinError {code}] "),
        Some(code) => format!("[Errno {code}] "),
        None => String::new(),
    };
    let quoted = |p: &Path| pytext::repr(&p.to_string_lossy());
    match (filename, other) {
        (Some(a), Some(b)) => format!("{tag}{message}: {} -> {}", quoted(a), quoted(b)),
        (Some(a), None) => format!("{tag}{message}: {}", quoted(a)),
        _ => format!("{tag}{message}"),
    }
}

/// `Path.mkdir(parents=True, exist_ok=True)`, reporting the path that failed.
fn mkdir_parents(path: &Path) -> std::result::Result<(), (io::Error, PathBuf)> {
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match path.parent() {
                Some(parent) if parent != path && !parent.as_os_str().is_empty() => mkdir_parents(parent)?,
                _ => return Err((error, path.to_path_buf())),
            }
            match std::fs::create_dir(path) {
                Ok(()) => Ok(()),
                Err(_) if path.is_dir() => Ok(()),
                Err(error) => Err((error, path.to_path_buf())),
            }
        }
        Err(_) if path.is_dir() => Ok(()),
        Err(error) => Err((error, path.to_path_buf())),
    }
}

fn unique_name(prefix: &str, suffix: &str, attempt: u32) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!(
        "{prefix}{:08x}{suffix}",
        nanos ^ std::process::id().rotate_left(16) ^ attempt.wrapping_mul(0x9E37_79B9)
    )
}

/// `tempfile.TemporaryDirectory(dir=parent, prefix=prefix)`: a new private directory.
fn make_temp_dir(parent: &Path, prefix: &str) -> io::Result<PathBuf> {
    for attempt in 0..100 {
        let candidate = parent.join(unique_name(prefix, "", attempt));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        match builder.create(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::from(io::ErrorKind::AlreadyExists))
}

pub struct Outputs {
    pub model: PathBuf,
    pub evidence: PathBuf,
    pub receipt: PathBuf,
}

/// The file operations the staged evaluation write performs; tests substitute failing ones.
pub(crate) struct Ops<'a> {
    pub write: &'a dyn Fn(&Path, &[u8]) -> io::Result<()>,
    pub rename: &'a dyn Fn(&Path, &Path) -> io::Result<()>,
    pub cleanup: &'a dyn Fn(&Path) -> io::Result<()>,
}

fn fs_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    std::fs::write(path, bytes)
}

fn fs_rename(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::rename(from, to)
}

fn fs_cleanup(path: &Path) -> io::Result<()> {
    std::fs::remove_dir_all(path)
}

pub(crate) const REAL_OPS: Ops<'static> = Ops {
    write: &fs_write,
    rename: &fs_rename,
    cleanup: &fs_cleanup,
};

/// Stage all 3 artefacts, then move them into place, receipt last.
pub fn write_evaluation(
    env: &Env,
    model: &Json,
    evidence: &Json,
    receipt: &Json,
    output_dir: &Path,
) -> Result<Outputs> {
    write_evaluation_with(env, model, evidence, receipt, output_dir, &REAL_OPS)
}

pub(crate) fn write_evaluation_with(
    env: &Env,
    model: &Json,
    evidence: &Json,
    receipt: &Json,
    output_dir: &Path,
    ops: &Ops,
) -> Result<Outputs> {
    let output_dir = env.within_build(output_dir, "output directory", false)?;
    let outputs = Outputs {
        model: output_dir.join("model-result.json"),
        evidence: output_dir.join("reviewer-evidence.json"),
        receipt: output_dir.join("receipt.json"),
    };
    let fail = |message: String| -> Failure {
        Failure::Gateway(format!(
            "run output cannot be written to {}: {message}.",
            output_dir.display()
        ))
    };
    mkdir_parents(&output_dir).map_err(|(error, at)| fail(os_error(&error, Some(&at), None)))?;
    let staging = make_temp_dir(&output_dir, ".evaluation-")
        .map_err(|error| fail(os_error(&error, Some(&output_dir.join(".evaluation-")), None)))?;
    let result = (|| -> std::result::Result<(), String> {
        let pack = [
            (model, &outputs.model),
            (evidence, &outputs.evidence),
            (receipt, &outputs.receipt),
        ];
        let staged: Vec<PathBuf> = pack
            .iter()
            .map(|(_, dest)| staging.join(dest.file_name().unwrap()))
            .collect();
        for ((payload, _), path) in pack.iter().zip(&staged) {
            (ops.write)(path, &text_mode_json(payload)).map_err(|e| os_error(&e, Some(path), None))?;
        }
        for ((_, dest), path) in pack.iter().zip(&staged) {
            (ops.rename)(path, dest).map_err(|e| os_error(&e, Some(path), Some(dest)))?;
        }
        Ok(())
    })();
    // Cleanup is best effort and never replaces the write's own result.
    let _ = (ops.cleanup)(&staging);
    result.map_err(fail)?;
    Ok(outputs)
}

/// Refuse a destination that is a second name for a review input, comparing `(st_dev, st_ino)`.
fn refuse_protected(destination: Option<(u64, u64)>, protected: &[Location]) -> Result<()> {
    let Some(destination) = destination else { return Ok(()) };
    for input in protected {
        let Location::Disk(path) = input else { continue };
        match crate::pypath::file_id(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return refuse(format!(
                    "validation output cannot be checked against {}: {}.",
                    path.display(),
                    os_error(&error, Some(path), None)
                ));
            }
            Ok(id) if id == destination => {
                return refuse(format!(
                    "validation output must not overwrite a review input: {}.",
                    path.display()
                ));
            }
            Ok(_) => {}
        }
    }
    Ok(())
}

/// Write validation output without following a changed destination pathname.
pub fn write_validation(payload: &Json, output: &Path, protected: &[Location]) -> Result<()> {
    let fail = |message: String| -> Failure {
        Failure::Gateway(format!(
            "validation output cannot be written to {}: {message}.",
            output.display()
        ))
    };
    let parent = output.parent().unwrap_or(output);
    mkdir_parents(parent).map_err(|(error, at)| fail(os_error(&error, Some(&at), None)))?;
    write_staged(payload, output, parent, protected, &fail)
}

#[cfg(unix)]
fn write_staged(
    payload: &Json,
    output: &Path,
    parent: &Path,
    protected: &[Location],
    fail: &dyn Fn(String) -> Failure,
) -> Result<()> {
    // The parent is opened once and every later step is named relative to it,
    // so renaming the parent afterwards cannot redirect the write.
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::FromRawFd;

    let c = |p: &Path| CString::new(p.as_os_str().as_bytes()).map_err(|_| fail("embedded null byte".into()));
    let parent_c = c(parent)?;
    // SAFETY: parent_c is a valid NUL-terminated path.
    let dir_fd = unsafe { libc::open(parent_c.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
    if dir_fd < 0 {
        return Err(fail(os_error(&io::Error::last_os_error(), Some(parent), None)));
    }
    let name = output.file_name().expect("output names a file");
    let temporary = format!(".{}.partial", name.to_string_lossy());
    let temporary_c = CString::new(temporary.as_bytes()).expect("no NUL in a file name");
    let name_c = CString::new(name.as_bytes()).expect("no NUL in a file name");
    let mut created = false;
    let result = (|| -> Result<()> {
        // O_EXCL prevents a pre-existing pathname (including a symlink) from being used as the staging file.
        // SAFETY: dir_fd is an open directory and temporary_c is a valid NUL-terminated name.
        let fd = unsafe {
            libc::openat(
                dir_fd,
                temporary_c.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(fail(os_error(
                &io::Error::last_os_error(),
                Some(Path::new(&temporary)),
                None,
            )));
        }
        created = true;
        // SAFETY: fd is a freshly opened descriptor this File now owns.
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        file.write_all(&text_mode_json(payload))
            .and_then(|()| file.flush())
            .and_then(|()| file.sync_all())
            .map_err(|e| fail(os_error(&e, None, None)))?;
        drop(file);
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: dir_fd and name_c are valid; stat is writable storage for one struct stat.
        let destination = if unsafe { libc::fstatat(dir_fd, name_c.as_ptr(), stat.as_mut_ptr(), 0) } == 0 {
            // SAFETY: fstatat succeeded and filled the struct.
            let stat = unsafe { stat.assume_init() };
            // dev_t and ino_t are not u64 on every Unix.
            #[allow(clippy::unnecessary_cast)]
            Some((stat.st_dev as u64, stat.st_ino as u64))
        } else {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::NotFound {
                return Err(fail(os_error(&error, Some(Path::new(name)), None)));
            }
            None
        };
        refuse_protected(destination, protected)?;
        // SAFETY: both names are valid and relative to the same open directory.
        if unsafe { libc::renameat(dir_fd, temporary_c.as_ptr(), dir_fd, name_c.as_ptr()) } != 0 {
            let error = io::Error::last_os_error();
            return Err(fail(os_error(
                &error,
                Some(Path::new(&temporary)),
                Some(Path::new(name)),
            )));
        }
        created = false;
        Ok(())
    })();
    if created {
        // SAFETY: valid directory descriptor and name; failure is ignored as in the Python original.
        unsafe { libc::unlinkat(dir_fd, temporary_c.as_ptr(), 0) };
    }
    // SAFETY: dir_fd was opened above and is closed exactly once.
    unsafe { libc::close(dir_fd) };
    result
}

#[cfg(not(unix))]
fn write_staged(
    payload: &Json,
    output: &Path,
    parent: &Path,
    protected: &[Location],
    fail: &dyn Fn(String) -> Failure,
) -> Result<()> {
    write_staged_by_path(payload, output, parent, protected, fail)
}

/// `write_validation` through the by-path branch, which Unix tests exercise too.
#[cfg(test)]
pub(crate) fn write_validation_by_path(payload: &Json, output: &Path, protected: &[Location]) -> Result<()> {
    let fail = |message: String| -> Failure {
        Failure::Gateway(format!(
            "validation output cannot be written to {}: {message}.",
            output.display()
        ))
    };
    let parent = output.parent().unwrap_or(output);
    mkdir_parents(parent).map_err(|(error, at)| fail(os_error(&error, Some(&at), None)))?;
    write_staged_by_path(payload, output, parent, protected, &fail)
}

#[cfg_attr(unix, allow(dead_code))]
fn write_staged_by_path(
    payload: &Json,
    output: &Path,
    parent: &Path,
    protected: &[Location],
    fail: &dyn Fn(String) -> Failure,
) -> Result<()> {
    // shortcut: Windows has no directory descriptors, so between the protected-input
    // check and the rename a concurrent swap of the parent can redirect the write,
    // exactly as the Python by-path branch documents. Closing it needs handle-relative
    // NT APIs, which the synthetic-only scope does not justify.
    let mut temporary = None;
    for attempt in 0..100 {
        let candidate = parent.join(unique_name(
            &format!(".{}.", output.file_name().unwrap().to_string_lossy()),
            ".partial",
            attempt,
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(fail(os_error(&error, Some(&candidate), None))),
        }
    }
    let (temporary, mut file) = temporary.ok_or_else(|| fail("no unique staging name".into()))?;
    let result = (|| -> Result<()> {
        file.write_all(&text_mode_json(payload))
            .and_then(|()| file.flush())
            .and_then(|()| file.sync_all())
            .map_err(|e| fail(os_error(&e, None, None)))?;
        let destination = match crate::pypath::file_id(output) {
            Ok(id) => Some(id),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(fail(os_error(&error, Some(output), None))),
        };
        refuse_protected(destination, protected)?;
        std::fs::rename(&temporary, output).map_err(|e| fail(os_error(&e, Some(&temporary), Some(output))))
    })();
    drop(file);
    let _ = std::fs::remove_file(&temporary);
    result
}
