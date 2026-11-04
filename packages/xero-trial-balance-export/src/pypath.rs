//! The `pathlib` behaviour the gateway's guards rely on: how a path prints,
//! `Path.resolve()` (which is `os.path.realpath`), and containment.

use crate::pytext;

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};

const SEP: char = if cfg!(windows) { '\\' } else { '/' };

/// `os.path.splitroot(p)` (Python 3.15) for a path already using `SEP`.
pub fn splitroot(p: &str) -> (&str, &str, &str) {
    if !cfg!(windows) {
        return if !p.starts_with('/') {
            ("", "", p)
        } else if !p[1..].starts_with('/') || p[2..].starts_with('/') {
            ("", "/", &p[1..])
        } else {
            ("", "//", &p[2..])
        };
    }
    if let Some(rest) = p.strip_prefix('\\') {
        if !rest.starts_with('\\') {
            // Relative path with root, e.g. \Windows
            return ("", "\\", rest);
        }
        // UNC drives, e.g. \\server\share or \\?\UNC\server\share; device drives, e.g. \\.\device
        let start = if p
            .get(..8)
            .is_some_and(|head| head.eq_ignore_ascii_case("\\\\?\\UNC\\"))
        {
            8
        } else {
            2
        };
        let Some(index) = p[start..].find('\\').map(|i| i + start) else {
            return (p, "", "");
        };
        let Some(index2) = p[index + 1..].find('\\').map(|i| i + index + 1) else {
            return (p, "", "");
        };
        return (&p[..index2], &p[index2..index2 + 1], &p[index2 + 1..]);
    }
    let mut chars = p.char_indices();
    match (chars.next(), chars.next()) {
        (Some(_), Some((colon, ':'))) => {
            let (drive, rest) = p.split_at(colon + 1);
            match rest.strip_prefix('\\') {
                // Absolute drive-letter path, e.g. X:\Windows
                Some(tail) => (drive, "\\", tail),
                // Relative path with drive, e.g. X:Windows
                None => (drive, "", rest),
            }
        }
        _ => ("", "", p),
    }
}

/// `str(PurePath(path))`: `_parse_path` then `_format_parsed_parts`.
fn parse_and_format(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let path = if cfg!(windows) {
        path.replace('/', "\\")
    } else {
        path.to_string()
    };
    let (drive, mut root, rel) = splitroot(&path);
    if root.is_empty() && drive.starts_with(SEP) && !drive.ends_with(SEP) {
        let parts: Vec<&str> = drive.split(SEP).collect();
        // e.g. //server/share, or //?/unc/server/share
        if (parts.len() == 4 && !matches!(parts[2], "" | "?" | "." | "?.")) || parts.len() == 6 {
            root = if cfg!(windows) { "\\" } else { "/" };
        }
    }
    let mut tail: Vec<&str> = rel
        .split(SEP)
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    let sep = SEP.to_string();
    let text = if !drive.is_empty() || !root.is_empty() {
        format!("{drive}{root}{}", tail.join(&sep))
    } else {
        // A first part that reads as a drive keeps a leading "." so it stays relative.
        if cfg!(windows)
            && tail
                .first()
                .is_some_and(|first| !splitroot(first).0.is_empty())
        {
            tail.insert(0, ".");
        }
        tail.join(&sep)
    };
    if text.is_empty() { ".".into() } else { text }
}

/// `str(Path(raw))`: separators collapsed and `.` parts dropped, `..` kept.
pub fn display(raw: &str) -> String {
    parse_and_format(raw)
}

/// `os.fsdecode` of an OS string as Python holds it: undecodable bytes (Unix)
/// or unpaired UTF-16 surrogates (Windows) become lone surrogates, which a
/// Rust string carries as `pytext::push_code_point` escapes.
pub fn decode_os(text: &OsStr) -> String {
    let mut out = String::new();
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        for chunk in text.as_bytes().utf8_chunks() {
            chunk
                .valid()
                .chars()
                .for_each(|ch| pytext::push_code_point(&mut out, ch as u32));
            // surrogateescape: each undecodable byte becomes U+DC00 + byte.
            chunk
                .invalid()
                .iter()
                .for_each(|&byte| pytext::push_code_point(&mut out, 0xDC00 + byte as u32));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        for unit in char::decode_utf16(text.encode_wide()) {
            let cp = unit.map_or_else(|error| error.unpaired_surrogate() as u32, |ch| ch as u32);
            pytext::push_code_point(&mut out, cp);
        }
    }
    out
}

/// `os.fsencode`: the OS string a Python str names, reversing `decode_os`.
pub fn encode_os(text: &str) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let mut bytes = Vec::with_capacity(text.len());
        for cp in pytext::code_points(text) {
            match (cp, char::from_u32(cp)) {
                (0xDC80..=0xDCFF, _) => bytes.push((cp - 0xDC00) as u8),
                (_, Some(ch)) => bytes.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes()),
                // Python's fsencode refuses any other surrogate; callers check first.
                (_, None) => bytes.extend_from_slice("\u{FFFD}".as_bytes()),
            }
        }
        OsString::from_vec(bytes)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        let mut units = Vec::with_capacity(text.len());
        for cp in pytext::code_points(text) {
            match char::from_u32(cp) {
                Some(ch) => units.extend_from_slice(ch.encode_utf16(&mut [0; 2])),
                None => units.push(cp as u16),
            }
        }
        OsString::from_wide(&units)
    }
}

/// `str(path)` for a path Rust holds: the inverse of `encode_os`.
pub fn text(path: &Path) -> String {
    decode_os(path.as_os_str())
}

/// Is the path absolute in `pathlib`'s sense (POSIX root, or Windows drive and root, or UNC)?
pub fn is_absolute(raw: &str) -> bool {
    Path::new(raw).is_absolute()
}

/// Does the path carry an anchor (drive or root), so that `root / path` discards `root`?
pub fn has_anchor(raw: &str) -> bool {
    if cfg!(windows) {
        let text = raw.replace('/', "\\");
        let (drive, root, _) = splitroot(&text);
        !drive.is_empty() || !root.is_empty()
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
    while seen.insert(fold(current.as_os_str())) {
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
    // On UTF-16 units, so an unpaired surrogate in the name survives.
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let units = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
    if let Some(unc) = wide.strip_prefix(units(r"\\?\UNC\").as_slice()) {
        let mut out = units(r"\\");
        out.extend_from_slice(unc);
        PathBuf::from(OsString::from_wide(&out))
    } else if let Some(local) = wide.strip_prefix(units(r"\\?\").as_slice()) {
        PathBuf::from(OsString::from_wide(local))
    } else {
        path
    }
}

/// Python 3.15's posixpath.realpath on bytes, step by step. Every component is
/// examined, including those after a missing one, so a later `..` that returns
/// to an existing directory still follows its symlinks.
#[cfg(not(windows))]
struct RealPath {
    /// Unprocessed parts, last first; `None` marks a resolved symlink target,
    /// and the link's own path is popped next.
    rest: Vec<Option<Vec<u8>>>,
    /// Parts left in `rest`, not counting the markers.
    part_count: usize,
    current: Vec<u8>,
    /// Symlink paths and their fully resolved targets (`None` while resolving).
    seen: std::collections::HashMap<Vec<u8>, Option<Vec<u8>>>,
    strict: bool,
}

#[cfg(not(windows))]
fn os_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(windows))]
impl RealPath {
    fn push_parts(&mut self, text: &[u8]) {
        let parts: Vec<&[u8]> = text.split(|&b| b == b'/').collect();
        self.part_count += parts.len();
        self.rest
            .extend(parts.into_iter().rev().map(|p| Some(p.to_vec())));
    }

    fn step(&mut self, name: &[u8]) -> Result<(), ResolveError> {
        if name.is_empty() || name == b"." {
            return Ok(());
        }
        if name == b".." {
            let cut = self.current.iter().rposition(|&b| b == b'/').unwrap_or(0);
            self.current.truncate(cut.max(1));
            return Ok(());
        }
        let mut newpath = self.current.clone();
        if newpath != b"/" {
            newpath.push(b'/');
        }
        newpath.extend_from_slice(name);
        let target = match std::fs::symlink_metadata(os_path(&newpath)) {
            Ok(meta) if !meta.file_type().is_symlink() => {
                if self.strict && self.part_count > 0 && !meta.is_dir() {
                    // ENOTDIR: an OSError other than FileNotFoundError.
                    return Err(ResolveError::Unusable);
                }
                self.current = newpath;
                return Ok(());
            }
            Ok(_) => match self.seen.get(&newpath) {
                Some(Some(resolved)) => {
                    self.current = resolved.clone();
                    return Ok(());
                }
                // A symlink loop: ELOOP when strict, else the link path itself.
                Some(None) if self.strict => return Err(ResolveError::Unusable),
                Some(None) => {
                    self.current = newpath;
                    return Ok(());
                }
                None => std::fs::read_link(os_path(&newpath)),
            },
            Err(error) => Err(error),
        };
        match target {
            Ok(target) => self.follow(newpath, &target),
            // Non-strict realpath ignores every OSError and keeps the name.
            Err(error) if self.strict => return Err(classify(&error)),
            Err(_) => self.current = newpath,
        }
        Ok(())
    }

    /// Queue a symlink's target, marking the link to record where it resolved.
    fn follow(&mut self, link: Vec<u8>, target: &Path) {
        use std::os::unix::ffi::OsStrExt;
        let target = target.as_os_str().as_bytes();
        if target.starts_with(b"/") {
            self.current = b"/".to_vec();
        }
        self.seen.insert(link.clone(), None);
        self.rest.push(Some(link));
        self.rest.push(None);
        self.push_parts(target);
    }
}

#[cfg(not(windows))]
fn resolve_absolute(path: &Path, strict: bool) -> Result<PathBuf, ResolveError> {
    use std::os::unix::ffi::OsStrExt;
    let filename = path.as_os_str().as_bytes();
    // pathlib's str(path) never ends in a separator.
    let trimmed =
        &filename[..filename.len() - filename.iter().rev().take_while(|&&b| b == b'/').count()];
    let mut walk = RealPath {
        rest: Vec::new(),
        part_count: 0,
        current: b"/".to_vec(),
        seen: std::collections::HashMap::new(),
        strict,
    };
    walk.push_parts(trimmed);
    while walk.part_count > 0 {
        let Some(name) = walk.rest.pop().expect("part_count counts the entries left") else {
            let link = walk
                .rest
                .pop()
                .flatten()
                .expect("a link path follows its marker");
            walk.seen.insert(link, Some(walk.current.clone()));
            continue;
        };
        walk.part_count -= 1;
        walk.step(&name)?;
    }
    Ok(os_path(&walk.current))
}

fn classify(error: &io::Error) -> ResolveError {
    if error.kind() == io::ErrorKind::NotFound {
        ResolveError::NotFound
    } else {
        ResolveError::Unusable
    }
}

/// The key pathlib compares a path part by: the exact OS string on POSIX,
/// `str.lower()` on Windows, with unpaired surrogates kept distinct. Never
/// lossy, so two different names can never compare equal.
fn fold(text: &OsStr) -> Vec<u32> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        char::decode_utf16(text.encode_wide())
            .flat_map(|unit| match unit {
                Ok(ch) => ch.to_lowercase().map(|c| c as u32).collect::<Vec<_>>(),
                Err(error) => vec![error.unpaired_surrogate() as u32],
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::ffi::OsStrExt;
        text.as_bytes().iter().map(|&b| b as u32).collect()
    }
}

/// `resolved.relative_to(root)` succeeds: lexical containment, case-insensitive on Windows.
pub fn is_within(resolved: &Path, root: &Path) -> bool {
    let parts =
        |p: &Path| -> Vec<Vec<u32>> { p.components().map(|c| fold(c.as_os_str())).collect() };
    let (inner, outer) = (parts(resolved), parts(root));
    inner.len() >= outer.len() && inner[..outer.len()] == outer[..]
}

/// `Path.__eq__`, which is case-insensitive for Windows paths.
pub fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        fold(a.as_os_str()) == fold(b.as_os_str())
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
        fn GetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            info: *mut ByHandleFileInformation,
        ) -> i32;
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
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call succeeded and filled the structure.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
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
    fn undecodable_arguments_round_trip_like_surrogateescape() {
        use std::os::unix::ffi::OsStrExt;
        let raw = OsStr::from_bytes(b"build/out\xff");
        let text = decode_os(raw);
        assert_eq!(pytext::repr_os(&text), "'build/out\\udcff'");
        assert_eq!(encode_os(&text), raw);
        assert_eq!(display("//x/./y"), "//x/y");
        assert_eq!(display("///etc"), "/etc");
        // A genuine private-use character from the carrier's range stays itself,
        // distinct from the surrogate an undecodable byte becomes.
        let genuine = OsStr::from_bytes("build/out\u{10FCFF}\u{10F7FF}".as_bytes());
        let genuine_text = decode_os(genuine);
        assert_ne!(genuine_text, text);
        assert_eq!(encode_os(&genuine_text), genuine);
        assert_eq!(
            pytext::repr_os(&genuine_text),
            "'build/out\\U0010fcff\\U0010f7ff'"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn containment_never_equates_two_different_names() {
        use std::os::unix::ffi::OsStrExt;
        let inner = Path::new(OsStr::from_bytes(b"/w/review-\xfe/build/out"));
        let root = Path::new(OsStr::from_bytes(b"/w/review-\xff/build"));
        assert!(!is_within(inner, root));
        assert!(is_within(
            Path::new(OsStr::from_bytes(b"/w/review-\xff/build/out")),
            root
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn resolve_follows_a_symlink_reached_after_a_missing_part() {
        // a test fixture directory, never used for a security decision.
        // nosemgrep: rust.lang.security.temp-dir.temp-dir
        let dir = std::env::temp_dir().join(format!("eaa-realpath-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("build")).unwrap();
        std::fs::create_dir_all(dir.join("outside")).unwrap();
        std::os::unix::fs::symlink(dir.join("outside"), dir.join("build/escape")).unwrap();
        let resolved = resolve(&dir.join("build/nonexistent/../escape/pack"), false).unwrap();
        let outside = resolve(&dir.join("outside"), true).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(resolved, outside.join("pack"));
    }

    #[cfg(not(windows))]
    #[test]
    fn display_matches_pure_posix_path() {
        assert_eq!(
            display("samples//decisions/./x.json"),
            "samples/decisions/x.json"
        );
        assert_eq!(display("a/../b/"), "a/../b");
        assert_eq!(display(""), ".");
        assert_eq!(display("//srv/x"), "//srv/x");
        assert_eq!(display("///x"), "/x");
        assert_eq!(display("./"), ".");
    }

    #[cfg(windows)]
    #[test]
    fn display_matches_pure_windows_path() {
        assert_eq!(
            display("samples/decisions/./x.json"),
            "samples\\decisions\\x.json"
        );
        assert_eq!(display("C:/a//b"), "C:\\a\\b");
        assert_eq!(display("C:a"), "C:a");
        assert_eq!(display(""), ".");
    }

    #[cfg(not(windows))]
    #[test]
    fn resolve_follows_links_and_appends_missing_parts() {
        // a test fixture directory, never used for a security decision.
        // nosemgrep: rust.lang.security.temp-dir.temp-dir
        let dir = std::env::temp_dir().join(format!("eaa-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("real")).unwrap();
        std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
        let root = resolve(&dir, true).unwrap();
        assert_eq!(
            resolve(&dir.join("link/new/../x"), false).unwrap(),
            root.join("real/x")
        );
        assert!(matches!(
            resolve(&dir.join("link/missing"), true),
            Err(ResolveError::NotFound)
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
