//! How CPython prints a path and an `OSError` raised for it.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;

use crate::pytext;

const SEP: char = if cfg!(windows) { '\\' } else { '/' };

/// `os.path.splitroot(p)` (Python 3.15) for a path already using `SEP`.
fn splitroot(p: &str) -> (&str, &str, &str) {
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
        let start = if p.get(..8).is_some_and(|head| head.eq_ignore_ascii_case("\\\\?\\UNC\\")) {
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
    let mut tail: Vec<&str> = rel.split(SEP).filter(|part| !part.is_empty() && *part != ".").collect();
    let sep = SEP.to_string();
    let text = if !drive.is_empty() || !root.is_empty() {
        format!("{drive}{root}{}", tail.join(&sep))
    } else {
        // A first part that reads as a drive keeps a leading "." so it stays relative.
        if cfg!(windows) && tail.first().is_some_and(|first| !splitroot(first).0.is_empty()) {
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

/// `str(Path(raw) / name)` for a plain file name: `os.path.join(raw, name)`, then parsed.
pub fn join(raw: &str, name: &str) -> String {
    if !cfg!(windows) {
        let joined = if raw.is_empty() || raw.ends_with('/') {
            format!("{raw}{name}")
        } else {
            format!("{raw}/{name}")
        };
        return parse_and_format(&joined);
    }
    let raw = raw.replace('/', "\\");
    let (drive, root, path) = splitroot(&raw);
    let mut path = path.to_string();
    if !path.is_empty() && !path.ends_with('\\') {
        path.push('\\');
    }
    path.push_str(name);
    // ntpath.join adds a separator between a UNC drive and a relative path.
    let joined = if root.is_empty() && !drive.is_empty() && !drive.ends_with([':', '\\']) {
        format!("{drive}\\{path}")
    } else {
        format!("{drive}{root}{path}")
    };
    parse_and_format(&joined)
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
    decode_os(path.as_os_str())
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
        // Roots, from Python 3.15's pathlib on Linux.
        for (raw, shown, joined) in [
            ("/", "/", "/agreements.json"),
            ("//", "//", "//agreements.json"),
            ("///", "/", "/agreements.json"),
            ("//x/./y", "//x/y", "//x/y/agreements.json"),
            ("..", "..", "../agreements.json"),
        ] {
            assert_eq!(display(raw), shown, "{raw}");
            assert_eq!(join(raw, "agreements.json"), joined, "{raw}");
        }
    }

    #[test]
    fn undecodable_arguments_round_trip_like_surrogateescape() {
        use std::os::unix::ffi::OsStrExt;
        let raw = OsStr::from_bytes(b"in\xff");
        let text = decode_os(raw);
        assert_eq!(pytext::repr(&text), "'in\\udcff'");
        assert_eq!(encode_os(&text), raw);
        // A genuine private-use character from the carrier's range stays itself.
        let genuine = OsStr::from_bytes("in\u{10FCFF}\u{10F7FF}".as_bytes());
        let genuine_text = decode_os(genuine);
        assert_ne!(genuine_text, text);
        assert_eq!(encode_os(&genuine_text), genuine);
        assert_eq!(pytext::repr(&genuine_text), "'in\\U0010fcff\\U0010f7ff'");
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

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn paths_print_like_pathlib() {
        // From Python 3.15's pathlib on Windows.
        for (raw, shown, joined) in [
            ("/", r"\", r"\agreements.json"),
            ("C:", "C:", "C:agreements.json"),
            ("C:/", r"C:\", r"C:\agreements.json"),
            ("C:x", "C:x", r"C:x\agreements.json"),
            ("//server/share", r"\\server\share\", r"\\server\share\agreements.json"),
            ("//server", r"\\server", r"\\server\agreements.json\"),
            ("a/", "a", r"a\agreements.json"),
            ("/a//b/.", r"\a\b", r"\a\b\agreements.json"),
        ] {
            assert_eq!(display(raw), shown, "{raw}");
            assert_eq!(join(raw, "agreements.json"), joined, "{raw}");
        }
    }

    #[test]
    fn unpaired_surrogates_round_trip() {
        use std::os::windows::ffi::OsStringExt;
        let raw = OsString::from_wide(&[0x69, 0x6E, 0xD800]);
        let text = decode_os(&raw);
        assert_eq!(pytext::repr(&text), r"'in\ud800'");
        assert_eq!(encode_os(&text), raw);
    }
}
