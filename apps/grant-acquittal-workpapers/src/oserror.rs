//! How CPython prints a path and an `OSError` raised for it.

use std::io;
use std::path::Path;

use crate::pytext;

/// `str(Path(raw))`: separators collapsed and `.` parts dropped, `..` kept.
pub fn display(raw: &str) -> String {
    if cfg!(windows) {
        let text = raw.replace('/', "\\");
        let (anchor, rest) = windows_anchor(&text);
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

fn windows_anchor(text: &str) -> (String, &str) {
    if text.starts_with("\\\\") && !text.starts_with("\\\\\\") {
        let mut pieces = text[2..].splitn(3, '\\');
        let drive = format!("\\\\{}\\{}", pieces.next().unwrap_or(""), pieces.next().unwrap_or(""));
        return match pieces.next() {
            Some(tail) => (format!("{drive}\\"), tail),
            None => (drive, ""),
        };
    }
    let bytes = text.as_bytes();
    let (drive, rest) = if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        (&text[..2], &text[2..])
    } else {
        ("", text)
    };
    match rest.strip_prefix('\\') {
        Some(stripped) => (format!("{drive}\\"), stripped.trim_start_matches('\\')),
        None => (drive.to_string(), rest),
    }
}

/// `str(Path(raw) / name)`.
pub fn join(raw: &str, name: &str) -> String {
    let sep = if cfg!(windows) { "\\" } else { "/" };
    display(&format!("{raw}{sep}{name}"))
}

fn quoted(path: &str) -> String {
    pytext::repr(path)
}

fn message(error: &io::Error) -> String {
    let mut text = error.to_string();
    if let Some(at) = text.rfind(" (os error ") {
        text.truncate(at);
    }
    text
}

/// An `OSError` from `open()` and file reads and writes, which report a C
/// runtime errno on every platform (Windows maps its system error codes).
pub fn file_error(error: &io::Error, path: &str) -> String {
    match error.raw_os_error() {
        Some(code) if cfg!(windows) => {
            let (errno, text) = match code {
                2 | 3 | 15 | 161 => (2, "No such file or directory"),
                5 | 32 | 33 | 19 => (13, "Permission denied"),
                80 | 183 => (17, "File exists"),
                267 => (20, "Not a directory"),
                112 => (28, "No space left on device"),
                _ => (22, "Invalid argument"),
            };
            format!("[Errno {errno}] {text}: {}", quoted(path))
        }
        Some(code) => format!("[Errno {code}] {}: {}", message(error), quoted(path)),
        None => message(error),
    }
}

/// An `OSError` from an `os` function (`os.mkdir`, `os.lstat`, `os.rename`),
/// which reports the system error code itself on Windows.
pub fn os_error(error: &io::Error, path: &str, other: Option<&str>) -> String {
    let Some(code) = error.raw_os_error() else {
        return message(error);
    };
    let (tag, text) = if cfg!(windows) {
        let text = message(error);
        (
            format!("[WinError {code}]"),
            text.trim_end_matches(|c: char| c == '.' || c.is_whitespace())
                .to_string(),
        )
    } else {
        (format!("[Errno {code}]"), message(error))
    };
    match other {
        Some(other) => format!("{tag} {text}: {} -> {}", quoted(path), quoted(other)),
        None => format!("{tag} {text}: {}", quoted(path)),
    }
}

pub fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn paths_print_like_pathlib() {
        assert_eq!(display("in//./x/"), "in/x");
        assert_eq!(join("in/", "agreements.json"), "in/agreements.json");
        assert_eq!(join(".", "x"), "x");
        assert_eq!(display(""), ".");
    }

    #[test]
    fn os_errors_print_like_cpython() {
        let missing = io::Error::from_raw_os_error(2);
        assert_eq!(
            file_error(&missing, "in/agreements.json"),
            "[Errno 2] No such file or directory: 'in/agreements.json'"
        );
        let exists = io::Error::from_raw_os_error(17);
        assert_eq!(os_error(&exists, "out", None), "[Errno 17] File exists: 'out'");
    }
}
