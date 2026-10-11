//! The `pathlib` behaviour the gateway's guards rely on: how a path prints,
//! `Path.resolve()` (which is `os.path.realpath`), and containment.

use std::io;
#[cfg(not(windows))]
use std::path::Component;
use std::path::{Path, PathBuf};

/// `str(Path(raw))`: separators collapsed and `.` parts dropped, `..` kept.
pub fn display(raw: &str) -> String {
    if cfg!(windows) {
        let text = raw.replace('/', "\\");
        let (anchor, rest) = split_windows_anchor(&text);
        let parts: Vec<&str> = rest.split('\\').filter(|p| !p.is_empty() && *p != ".").collect();
        let joined = format!("{anchor}{}", parts.join("\\"));
        if joined.is_empty() { ".".into() } else { joined }
    } else {
        let root = if raw.starts_with("//") && !raw.starts_with("///") {
            "//"
        } else if raw.starts_with('/') {
            "/"
        } else {
            ""
        };
        let parts: Vec<&str> = raw.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
        let joined = format!("{root}{}", parts.join("/"));
        if joined.is_empty() { ".".into() } else { joined }
    }
}

/// Drive and root of a backslash-separated Windows path, as `PureWindowsPath` splits it.
fn split_windows_anchor(text: &str) -> (String, &str) {
    let bytes = text.as_bytes();
    if text.starts_with("\\\\") && !text.starts_with("\\\\\\") {
        // UNC: \\server\share is the drive.
        let rest = &text[2..];
        let mut pieces = rest.splitn(3, '\\');
        let server = pieces.next().unwrap_or("");
        let share = pieces.next().unwrap_or("");
        let tail = pieces.next();
        let drive = format!("\\\\{server}\\{share}");
        return match tail {
            Some(tail) => (format!("{drive}\\"), tail),
            None => (drive, ""),
        };
    }
    let (drive, rest) = if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        (&text[..2], &text[2..])
    } else {
        ("", text)
    };
    if let Some(stripped) = rest.strip_prefix('\\') {
        (format!("{drive}\\"), stripped.trim_start_matches('\\'))
    } else {
        (drive.to_string(), rest)
    }
}

/// Is the path absolute in `pathlib`'s sense (POSIX root, or Windows drive and root, or UNC)?
pub fn is_absolute(raw: &str) -> bool {
    Path::new(raw).is_absolute()
}

/// Does the path carry an anchor (drive or root), so that `root / path` discards `root`?
pub fn has_anchor(raw: &str) -> bool {
    if cfg!(windows) {
        let text = raw.replace('/', "\\");
        !split_windows_anchor(&text).0.is_empty()
    } else {
        raw.starts_with('/')
    }
}

#[derive(Debug)]
pub enum ResolveError {
    /// `FileNotFoundError` from a strict resolve.
    NotFound,
    /// Any other `OSError` or `ValueError` (embedded NUL, invalid name).
    Unusable,
}

/// `Path(path).resolve(strict=strict)`.
pub fn resolve(path: &Path, strict: bool) -> Result<PathBuf, ResolveError> {
    if path.as_os_str().to_string_lossy().contains('\0') {
        return Err(ResolveError::Unusable);
    }
    let absolute = std::path::absolute(path).map_err(|_| ResolveError::Unusable)?;
    resolve_absolute(&absolute, strict)
}

#[cfg(windows)]
fn resolve_absolute(path: &Path, strict: bool) -> Result<PathBuf, ResolveError> {
    // ntpath.realpath: normpath (std::path::absolute already did it), then the
    // final path of the longest existing prefix with the missing tail appended.
    if strict {
        return std::fs::canonicalize(path)
            .map(strip_verbatim)
            .map_err(|e| classify(&e));
    }
    // _getfinalpathname_nonstrict: when Windows cannot resolve a prefix, follow
    // a link by reading it (as _readlink_deep does), else drop to the parent.
    let mut current = path.to_path_buf();
    let mut tail = Vec::new();
    loop {
        let base = match std::fs::canonicalize(&current) {
            Ok(found) => Some(strip_verbatim(found)),
            Err(_) => read_link_deep(&current).filter(|followed| *followed != current),
        };
        if let Some(mut result) = base {
            for name in tail.iter().rev() {
                result.push(name);
            }
            return Ok(result);
        }
        let Some(name) = current.file_name().map(|n| n.to_owned()) else {
            return Ok(path.to_path_buf());
        };
        tail.push(name);
        current.pop();
    }
}

/// ntpath._readlink_deep: follow links by reading them, resolving relative
/// targets against the link's own directory.
#[cfg(windows)]
fn read_link_deep(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    let mut seen = std::collections::HashSet::new();
    while seen.insert(current.to_string_lossy().to_lowercase()) {
        let Ok(target) = std::fs::read_link(&current) else {
            break;
        };
        let joined = if target.is_absolute() {
            target
        } else {
            current.parent()?.join(target)
        };
        current = std::path::absolute(joined).ok()?;
    }
    Some(current)
}

#[cfg(windows)]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(local) = text.strip_prefix(r"\\?\") {
        PathBuf::from(local)
    } else {
        path
    }
}

#[cfg(not(windows))]
fn resolve_absolute(path: &Path, strict: bool) -> Result<PathBuf, ResolveError> {
    // posixpath.realpath: walk the components, following symbolic links of
    // the parts that exist and joining the rest lexically.
    let mut pending: Vec<std::ffi::OsString> = path
        .components()
        .rev()
        .filter_map(|c| match c {
            Component::Normal(name) => Some(name.to_owned()),
            Component::ParentDir => Some("..".into()),
            _ => None,
        })
        .collect();
    let mut resolved = PathBuf::from("/");
    let mut hops = 0;
    let mut missing = false;
    while let Some(name) = pending.pop() {
        if name == ".." {
            resolved.pop();
            continue;
        }
        let candidate = resolved.join(&name);
        if missing {
            resolved = candidate;
            continue;
        }
        match std::fs::symlink_metadata(&candidate) {
            Ok(meta) if meta.file_type().is_symlink() => {
                hops += 1;
                if hops > 40 {
                    if strict {
                        return Err(ResolveError::Unusable);
                    }
                    resolved = candidate;
                    missing = true;
                    continue;
                }
                let target = std::fs::read_link(&candidate).map_err(|e| classify(&e))?;
                if target.is_absolute() {
                    resolved = PathBuf::from("/");
                }
                for component in target.components().rev() {
                    match component {
                        Component::Normal(part) => pending.push(part.to_owned()),
                        Component::ParentDir => pending.push("..".into()),
                        _ => {}
                    }
                }
            }
            Ok(_) => resolved = candidate,
            Err(error) => {
                // Python 3.13+ realpath treats every lstat failure as missing unless strict.
                if strict {
                    return Err(classify(&error));
                }
                resolved = candidate;
                missing = true;
            }
        }
    }
    Ok(resolved)
}

fn classify(error: &io::Error) -> ResolveError {
    if error.kind() == io::ErrorKind::NotFound {
        ResolveError::NotFound
    } else {
        ResolveError::Unusable
    }
}

/// `resolved.relative_to(root)` succeeds: lexical containment, case-insensitive on Windows.
pub fn is_within(resolved: &Path, root: &Path) -> bool {
    let fold = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| {
                let text = c.as_os_str().to_string_lossy().into_owned();
                if cfg!(windows) { text.to_lowercase() } else { text }
            })
            .collect()
    };
    let (inner, outer) = (fold(resolved), fold(root));
    inner.len() >= outer.len() && inner[..outer.len()] == outer[..]
}

/// `Path.__eq__`, which is case-insensitive for Windows paths.
pub fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// `(st_dev, st_ino)`-style identity, for `os.path.samefile` and the persist guards.
pub fn file_id(path: &Path) -> io::Result<(u64, u64)> {
    let meta = std::fs::metadata(path)?;
    identity(path, &meta)
}

#[cfg(unix)]
fn identity(_path: &Path, meta: &std::fs::Metadata) -> io::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Ok((meta.dev(), meta.ino()))
}

#[cfg(windows)]
fn identity(path: &Path, _meta: &std::fs::Metadata) -> io::Result<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    // os.stat on Windows fills st_dev and st_ino from the volume serial number
    // and file index of an opened handle, following links.
    #[repr(C)]
    struct ByHandleFileInformation {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume_serial: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut ByHandleFileInformation) -> i32;
    }
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    let file = std::fs::OpenOptions::new()
        .access_mode(0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?;
    let mut info = std::mem::MaybeUninit::<ByHandleFileInformation>::zeroed();
    // SAFETY: the handle is open for the duration of the call and info is a
    // correctly sized, writable BY_HANDLE_FILE_INFORMATION.
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call succeeded and filled the structure.
    let info = unsafe { info.assume_init() };
    Ok((
        info.volume_serial as u64,
        ((info.index_high as u64) << 32) | info.index_low as u64,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn display_matches_pure_posix_path() {
        assert_eq!(display("samples//decisions/./x.json"), "samples/decisions/x.json");
        assert_eq!(display("a/../b/"), "a/../b");
        assert_eq!(display(""), ".");
        assert_eq!(display("//srv/x"), "//srv/x");
        assert_eq!(display("///x"), "/x");
        assert_eq!(display("./"), ".");
    }

    #[cfg(windows)]
    #[test]
    fn display_matches_pure_windows_path() {
        assert_eq!(display("samples/decisions/./x.json"), "samples\\decisions\\x.json");
        assert_eq!(display("C:/a//b"), "C:\\a\\b");
        assert_eq!(display("C:a"), "C:a");
        assert_eq!(display(""), ".");
    }

    #[cfg(not(windows))]
    #[test]
    fn resolve_follows_links_and_appends_missing_parts() {
        let dir = std::env::temp_dir().join(format!("eaa-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("real")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
        let root = resolve(&dir, true).unwrap();
        assert_eq!(resolve(&dir.join("link/new/../x"), false).unwrap(), root.join("real/x"));
        assert!(matches!(
            resolve(&dir.join("link/missing"), true),
            Err(ResolveError::NotFound)
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
