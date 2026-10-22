//! Path resolution and containment reused from Monthly Close.
#[cfg(windows)]
use std::ffi::OsString;
use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
};
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
