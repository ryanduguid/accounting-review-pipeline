//! Local filesystem and git guard, preserving checks and their order.
use crate::{entities, oserror, patterns::Result, pyjson, pypath};
use std::{
    ffi::OsString,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub fn text(path: &Path) -> String {
    pypath::text(path)
}

pub fn resolve(path: &Path) -> Result<PathBuf> {
    pypath::resolve(path, false).map_err(|_| "cannot resolve path".into())
}

pub fn suffix(path: &Path, suffix: &str) -> Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} has an empty name", text(path)))?;
    let mut name = name.to_os_string();
    name.push(suffix);
    Ok(path.with_file_name(name))
}

pub fn mkdir(path: &Path) -> Result<()> {
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty() && *p != path) {
                mkdir(parent)?;
            }
            match std::fs::create_dir(path) {
                Ok(()) => Ok(()),
                Err(_) if path.is_dir() => Ok(()),
                Err(e) => Err(oserror::os_error(&e, &text(path), None)),
            }
        }
        Err(_) if path.is_dir() => Ok(()),
        Err(e) => Err(oserror::os_error(&e, &text(path), None)),
    }
}

pub fn write(path: &Path, content: &str, ending: &str) -> Result<()> {
    mkdir(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    let content = if ending == "\n" {
        content.to_string()
    } else {
        content.replace('\n', ending)
    };
    let file = std::fs::File::create(path).map_err(|e| oserror::file_error(&e, &text(path)))?;
    crate::persist::text(file, content.as_bytes()).map_err(|e| oserror::file_error(&e, &text(path)))
}

pub fn remove(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(oserror::os_error(&e, &text(path), None)),
    }
}

pub fn bytes(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| oserror::file_error(&e, &text(path)))
}

pub fn read(path: &Path) -> Result<(String, &'static str)> {
    let raw = String::from_utf8(bytes(path)?).map_err(|e| utf8_error(e.as_bytes(), e.utf8_error()))?;
    let crlf = raw.matches("\r\n").count();
    let ending = if crlf > raw.matches('\n').count() - crlf {
        "\r\n"
    } else {
        "\n"
    };
    Ok((raw.replace("\r\n", "\n"), ending))
}

pub fn utf8_error(bytes: &[u8], error: std::str::Utf8Error) -> String {
    let pos = error.valid_up_to();
    let size = error.error_len().unwrap_or(bytes.len() - pos);
    let reason = if error.error_len().is_none() {
        "unexpected end of data"
    } else if size == 1 && (bytes[pos] < 0xC2 || bytes[pos] > 0xF4) {
        "invalid start byte"
    } else {
        "invalid continuation byte"
    };
    if size == 1 {
        format!(
            "'utf-8' codec can't decode byte 0x{:02x} in position {pos}: {reason}",
            bytes[pos]
        )
    } else {
        format!(
            "'utf-8' codec can't decode bytes in position {pos}-{}: {reason}",
            pos + size - 1
        )
    }
}

pub fn load(path: &Path) -> Result<Vec<entities::Entity>> {
    let raw = bytes(path).map_err(|error| format!("cannot read entity map {}: {error}", self::text(path)))?;
    let text = String::from_utf8(raw).map_err(|e| utf8_error(e.as_bytes(), e.utf8_error()))?;
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let document = pyjson::loads_plain(&text).map_err(|e| match e {
        pyjson::LoadError::Crash(message) => message,
        pyjson::LoadError::Decode(message) => format!("cannot read entity map {}: {message}", self::text(path)),
        _ => format!("cannot read entity map {}: invalid JSON", self::text(path)),
    })?;
    entities::load(&document)
}

/// os.path.abspath: lexical normalisation, without following links.
pub fn absolute(path: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(path).map_err(|e| oserror::os_error(&e, &text(path), None))?;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            _ => out.push(part.as_os_str()),
        }
    }
    Ok(out)
}

/// shutil's `_access_check`: the path exists, is not a directory and, on POSIX, passes X_OK.
fn executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if metadata.is_dir() {
        return false;
    }
    #[cfg(unix)]
    {
        {
            use std::os::unix::ffi::OsStrExt;
            let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
                return false;
            };
            // SAFETY: path is a valid NUL-terminated pathname; access only checks it.
            unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
        }
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The directories CPython 3.14's `shutil.which("git")` searches, in order.
///
/// An unset PATH falls back to `os.confstr("CS_PATH")` on POSIX and `os.defpath`
/// on Windows; an empty PATH searches nothing. The list is split on the bare
/// separator, as `str.split(os.pathsep)` does. On Windows the current directory
/// goes first only when `NeedCurrentDirectoryForExePathW("git")` is true, which
/// for a name without a backslash means NoDefaultCurrentDirectoryInExePath is unset.
fn which_dirs(path: Option<OsString>, no_default_curdir: bool) -> Vec<PathBuf> {
    let path = path.unwrap_or_else(|| OsString::from(if cfg!(windows) { ".;C:\\bin" } else { "/bin:/usr/bin" }));
    if path.is_empty() {
        return Vec::new();
    }
    let separator = if cfg!(windows) { ';' } else { ':' };
    let mut dirs: Vec<PathBuf> = path.to_string_lossy().split(separator).map(PathBuf::from).collect();
    if cfg!(windows) && !no_default_curdir {
        dirs.insert(0, PathBuf::from("."));
    }
    dirs
}

/// The file names `shutil.which("git")` tries in each directory.
fn which_names(pathext: Option<String>) -> Vec<String> {
    if !cfg!(windows) {
        return vec!["git".into()];
    }
    let source = pathext
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD;.VBS;.JS;.WS;.MSC".into());
    let extensions: Vec<&str> = source
        .split(';')
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.trim_end_matches('.'))
        .collect();
    let mut names: Vec<String> = extensions.iter().map(|ext| format!("git{ext}")).collect();
    if extensions.iter().any(|ext| "GIT".ends_with(&ext.to_uppercase())) {
        names.insert(0, "git".into());
    }
    names
}

fn git_executable() -> Option<PathBuf> {
    let dirs = which_dirs(
        std::env::var_os("PATH"),
        std::env::var_os("NoDefaultCurrentDirectoryInExePath").is_some(),
    );
    let names = which_names(std::env::var("PATHEXT").ok());
    for dir in dirs {
        for name in &names {
            let candidate = dir.join(name);
            if executable(&candidate) {
                return absolute(&candidate).ok();
            }
        }
    }
    None
}

fn git_argv(executable: &Path, subcommand: &[&str], name: &std::ffi::OsStr) -> String {
    let mut argv = vec![crate::pytext::repr_os(&text(executable))];
    argv.extend(subcommand.iter().map(|s| crate::pytext::repr(s)));
    argv.push("'--'".into());
    argv.push(crate::pytext::repr_os(&text(Path::new(name))));
    format!("[{}]", argv.join(", "))
}

fn exit_code(status: std::process::ExitStatus) -> i64 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        i64::from(status.code().unwrap_or_else(|| -status.signal().unwrap_or(1)))
    }
    #[cfg(not(unix))]
    {
        // CPython exposes Windows' unsigned DWORD exit code.
        i64::from(status.code().unwrap_or(1) as u32)
    }
}

fn git(subcommand: &[&str], target: &Path) -> Result<i64> {
    let parent = target.parent().unwrap_or(target);
    let name = target.file_name().unwrap_or(std::ffi::OsStr::new(""));
    let preface = format!("cannot ask git about {} in {}: ", text(Path::new(name)), text(parent));
    let ending = "; put git on PATH and re-run once the repository is idle";
    let executable = git_executable().ok_or_else(|| format!("{preface}git is not on PATH{ending}"))?;
    let mut command = Command::new(&executable);
    command
        .args(subcommand)
        .arg("--")
        .arg(name)
        .current_dir(parent)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        command.env_remove(var);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("{preface}{}{ending}", oserror::file_error(&e, &text(&executable))))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("{preface}{e}{ending}"))? {
            return Ok(exit_code(status));
        }
        if started.elapsed() >= Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{preface}Command '{}' timed out after 30 seconds{ending}",
                git_argv(&executable, subcommand, name)
            ));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

pub fn ignored(path: &Path, description: &str) -> Result<()> {
    let target = resolve(path)?;
    let tracked = git(&["ls-files", "--error-unmatch"], &target)?;
    let ignored = git(&["check-ignore", "--quiet"], &target)?;
    let Some(problem) = git_problem(tracked, ignored) else {
        return Ok(());
    };
    Err(format!(
        "{description} must never be committed. {}: {problem}",
        text(&target)
    ))
}

fn git_problem(tracked: i64, ignored: i64) -> Option<String> {
    Some(if tracked == 0 {
        "git already tracks it; run git rm --cached, then purge it from history".into()
    } else if tracked == 128 {
        "it is not inside a git work tree; move it into the repository that ignores it".into()
    } else if tracked != 1 || ![0, 1].contains(&ignored) {
        format!("git answered unexpectedly: ls-files {tracked}, check-ignore {ignored}")
    } else if ignored == 1 {
        "git does not ignore it; add a covering rule to .gitignore".into()
    } else {
        return None;
    })
}

#[cfg(any(windows, test))]
fn attribute_requires_lstat(attributes: u32, last_error: Option<i32>) -> std::result::Result<bool, io::Error> {
    if attributes == u32::MAX {
        if [Some(2), Some(3)].contains(&last_error) {
            return Ok(false);
        }
        return Err(io::Error::from_raw_os_error(last_error.unwrap_or(0)));
    }
    if attributes & 0x400 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "disclosure record paths must not use links",
        ));
    }
    Ok(true)
}

fn inspect(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{GetFileAttributesW, INVALID_FILE_ATTRIBUTES};
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: path is NUL terminated and the API only reads its attributes.
        let attr = unsafe { GetFileAttributesW(wide.as_ptr()) };
        let last = if attr == INVALID_FILE_ATTRIBUTES {
            io::Error::last_os_error().raw_os_error()
        } else {
            None
        };
        if !attribute_requires_lstat(attr, last).map_err(|e| {
            if e.kind() == io::ErrorKind::InvalidInput && e.raw_os_error().is_none() {
                e.to_string()
            } else {
                oserror::os_error(&e, &text(path), None)
            }
        })? {
            return Ok(());
        }
    }
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err("disclosure record paths must not use links".into()),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(oserror::os_error(&e, &text(path), None)),
    }
}

pub fn record_path(path: &Path) -> Result<()> {
    let lexical: Vec<_> = path
        .ancestors()
        .map(|p| {
            if p.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                p.to_path_buf()
            }
        })
        .collect();
    for candidate in &lexical {
        inspect(candidate)?;
    }
    let normalised = lexical.iter().map(|p| absolute(p)).collect::<Result<Vec<_>>>()?;
    for candidate in &normalised {
        inspect(candidate)?;
    }
    for parent in normalised.last().unwrap().ancestors().skip(1) {
        inspect(parent)?;
    }
    Ok(())
}

fn one_link_before(path: &Path, metadata: &std::fs::Metadata) -> Result<bool> {
    if !metadata.is_file() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        Ok(metadata.nlink() == 1)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .access_mode(0)
            .custom_flags(0x0200_0000)
            .open(path)
            .map_err(|e| oserror::os_error(&e, &text(path), None))?;
        one_link_open(&file)
    }
}

fn one_link_open(file: &std::fs::File) -> Result<bool> {
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(metadata.nlink() == 1)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
        // SAFETY: the file owns this handle and info is writable, correctly sized storage.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        // SAFETY: the call succeeded and initialised the structure.
        Ok(unsafe { info.assume_init() }.nNumberOfLinks == 1)
    }
}

pub fn record_read(path: &Path) -> Result<Vec<u8>> {
    record_path(path)?;
    let before = std::fs::symlink_metadata(path).map_err(|e| oserror::os_error(&e, &text(path), None))?;
    if !one_link_before(path, &before)? {
        return Err("disclosure record must be a regular file with one link".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| oserror::file_error(&e, &text(path)))?;
    if !one_link_open(&file)? {
        return Err("disclosure record must be a regular file with one link".into());
    }
    let mut bytes = Vec::new();
    file.take((crate::disclosure::MAX_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_windows_entries_short_circuit_metadata_but_unknown_errors_do_not() {
        for code in [2, 3] {
            assert!(!attribute_requires_lstat(u32::MAX, Some(code)).unwrap());
        }
        for code in [0, 5, 32, 1920] {
            assert!(attribute_requires_lstat(u32::MAX, Some(code)).is_err());
        }
        for attributes in [0x400, 0x410, 0x420] {
            assert!(attribute_requires_lstat(attributes, None).is_err());
        }
        for attributes in [0x80, 0x10] {
            assert!(attribute_requires_lstat(attributes, None).unwrap());
        }
    }
    #[test]
    fn an_empty_path_searches_no_directory_not_even_the_current_one() {
        assert!(which_dirs(Some(OsString::new()), false).is_empty());
        assert!(which_dirs(Some(OsString::new()), true).is_empty());
    }
    #[test]
    fn the_search_follows_shutil_which() {
        if cfg!(windows) {
            let dirs = |no_curdir| which_dirs(Some(OsString::from("C:\\a;;D:\\b")), no_curdir);
            let expected = |items: &[&str]| items.iter().map(PathBuf::from).collect::<Vec<_>>();
            assert_eq!(dirs(false), expected(&[".", "C:\\a", "", "D:\\b"]));
            assert_eq!(dirs(true), expected(&["C:\\a", "", "D:\\b"]));
            assert_eq!(which_dirs(None, true), expected(&[".", "C:\\bin"]));
            let defaults = [
                "git.COM", "git.EXE", "git.BAT", "git.CMD", "git.VBS", "git.JS", "git.WS", "git.MSC",
            ];
            assert_eq!(which_names(None), defaults);
            assert_eq!(which_names(Some(String::new())), defaults);
            assert_eq!(which_names(Some(".exe.;;.cmd".into())), ["git.exe", "git.cmd"]);
            assert_eq!(which_names(Some(".;.EXE".into())), ["git", "git", "git.EXE"]);
        } else {
            let dirs = which_dirs(Some(OsString::from("/a::/b")), false);
            assert_eq!(dirs, ["/a", "", "/b"].map(PathBuf::from));
            assert_eq!(which_dirs(None, false), ["/bin", "/usr/bin"].map(PathBuf::from));
            assert_eq!(which_names(Some(".EXE".into())), ["git"]);
        }
    }
    #[test]
    fn git_argv_timeout_text_preserves_the_complete_argument_list() {
        assert_eq!(
            git_argv(
                Path::new("/usr/bin/git"),
                &["ls-files", "--error-unmatch"],
                std::ffi::OsStr::new("entities.json")
            ),
            "['/usr/bin/git', 'ls-files', '--error-unmatch', '--', 'entities.json']"
        );
    }

    #[test]
    fn every_git_status_branch_preserves_priority_and_refuses_unexpected_answers() {
        for ignored in [0, 1, 3, 128, -15] {
            assert_eq!(
                git_problem(0, ignored).unwrap(),
                "git already tracks it; run git rm --cached, then purge it from history"
            );
            assert_eq!(
                git_problem(128, ignored).unwrap(),
                "it is not inside a git work tree; move it into the repository that ignores it"
            );
        }
        assert!(git_problem(1, 0).is_none());
        assert_eq!(
            git_problem(1, 1).unwrap(),
            "git does not ignore it; add a covering rule to .gitignore"
        );
        for (tracked, ignored) in [(1, 3), (3, 0), (3, 1), (-15, 0), (1, -15)] {
            assert_eq!(
                git_problem(tracked, ignored).unwrap(),
                format!("git answered unexpectedly: ls-files {tracked}, check-ignore {ignored}")
            );
        }
    }
}
