//! CPython 3.14's native text-stream encoding, independent of installed Python.
//! Data files are always explicitly UTF-8; this applies only to CLI diagnostics.
use crate::pytext;

pub enum Encoding {
    Utf8,
    Utf8Sig,
    Ascii,
    Latin1,
    Windows1252,
}

/// Refusal for a stream configuration this binary cannot reproduce byte for byte.
/// CPython would either encode with a codec not implemented here or fail to start;
/// refusing before any command runs keeps a Python failure from becoming a success.
pub const UNSUPPORTED: &str = "unsupported stream encoding; evatt writes diagnostics only in UTF-8, \
ASCII, Latin-1 or Windows-1252, so set PYTHONIOENCODING or the locale to one of them";

/// `encodings.normalize_encoding` followed by the codec alias table, for the
/// codecs implemented here. Any other name, valid in CPython or not, is refused.
fn codec(name: &str) -> Result<Encoding, &'static str> {
    let mut normal = String::new();
    for ch in name.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' {
            normal.push(ch);
        } else if !normal.is_empty() && !normal.ends_with('_') {
            normal.push('_');
        }
    }
    match normal.trim_end_matches('_') {
        "utf_8" | "utf8" | "u8" | "utf" | "utf8_ucs2" | "utf8_ucs4" | "cp65001" => Ok(Encoding::Utf8),
        "utf_8_sig" | "utf8_sig" => Ok(Encoding::Utf8Sig),
        "ascii" | "646" | "ansi_x3.4_1968" | "ansi_x3_4_1968" | "ansi_x3.4_1986" | "cp367" | "csascii" | "ibm367"
        | "iso646_us" | "iso_646.irv_1991" | "iso_ir_6" | "us" | "us_ascii" => Ok(Encoding::Ascii),
        "latin_1" | "latin1" | "iso8859_1" | "iso_8859_1" | "8859" | "cp819" | "csisolatin1" | "ibm819"
        | "iso_8859_1_1987" | "iso_ir_100" | "l1" | "latin" => Ok(Encoding::Latin1),
        "cp1252" | "windows_1252" => Ok(Encoding::Windows1252),
        _ => Err(UNSUPPORTED),
    }
}

/// The encoding part of PYTHONIOENCODING, when it names one.
fn override_name() -> Option<String> {
    let raw = std::env::var("PYTHONIOENCODING").ok()?;
    let name = raw.split(':').next().unwrap_or_default();
    (!name.is_empty()).then(|| name.to_string())
}

/// Whether CPython's UTF-8 mode is on: PYTHONUTF8=1, or unset with a C or POSIX
/// LC_CTYPE (PEP 540). Any other PYTHONUTF8 value stops CPython at start-up.
fn utf8_mode(locale: Option<&str>) -> Result<bool, &'static str> {
    match std::env::var("PYTHONUTF8").as_deref() {
        Ok("1") => Ok(true),
        Ok("0") => Ok(false),
        Err(_) | Ok("") => Ok(matches!(locale, Some("C" | "POSIX"))),
        Ok(_) => Err(UNSUPPORTED),
    }
}

/// LC_CTYPE as CPython leaves it after `setlocale(LC_CTYPE, "")`: its name and codeset.
#[cfg(not(windows))]
fn posix_locale() -> &'static (Option<String>, Option<String>) {
    static LOCALE: std::sync::OnceLock<(Option<String>, Option<String>)> = std::sync::OnceLock::new();
    LOCALE.get_or_init(|| {
        let text = |ptr: *const libc::c_char| {
            // SAFETY: a non-null result is a NUL-terminated string owned by libc.
            (!ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
        };
        // SAFETY: called once, before evatt starts its worker thread or reads locale data.
        let name = text(unsafe { libc::setlocale(libc::LC_CTYPE, c"".as_ptr()) });
        // SAFETY: nl_langinfo only reads the current locale's codeset.
        let codeset = text(unsafe { libc::nl_langinfo(libc::CODESET) });
        (name, codeset)
    })
}

/// Settle the stream configuration once, in the main thread, before any command runs.
pub fn configure() -> Result<(), &'static str> {
    // CPython reads both variables before it sets up any stream and stops on a value
    // it cannot decode or, for PYTHONUTF8, one other than 0 or 1. Check them before
    // the console or explicit-codec branches below can return early.
    for name in ["PYTHONUTF8", "PYTHONIOENCODING"] {
        if matches!(std::env::var(name), Err(std::env::VarError::NotUnicode(_))) {
            return Err(UNSUPPORTED);
        }
    }
    #[cfg(not(windows))]
    utf8_mode(posix_locale().0.as_deref())?;
    #[cfg(windows)]
    utf8_mode(None)?;
    encoding(false)?;
    encoding(true)?;
    stdout_policy()?;
    Ok(())
}

pub fn encoding(stderr: bool) -> Result<Encoding, &'static str> {
    #[cfg(not(windows))]
    {
        let _ = stderr;
        // The locale also decides how CPython decodes argv and paths, so it must
        // resolve to UTF-8 even when PYTHONIOENCODING names the stream codec.
        let (name, codeset) = posix_locale();
        let utf8_locale = utf8_mode(name.as_deref())?
            // PEP 538 coerces C or POSIX to C.UTF-8 unless PYTHONCOERCECLOCALE=0.
            || (matches!(name.as_deref(), Some("C" | "POSIX"))
                && std::env::var("PYTHONCOERCECLOCALE").as_deref() != Ok("0"))
            || codeset.as_deref().is_some_and(|c| codec(c).is_ok_and(|e| matches!(e, Encoding::Utf8)));
        if !utf8_locale {
            return Err(UNSUPPORTED);
        }
        match override_name() {
            Some(name) => codec(&name),
            None => Ok(Encoding::Utf8),
        }
    }
    #[cfg(windows)]
    {
        let console = console(stderr) && std::env::var_os("PYTHONLEGACYWINDOWSSTDIO").is_none();
        if console {
            return Ok(Encoding::Utf8);
        }
        if let Some(name) = override_name() {
            return codec(&name);
        }
        if utf8_mode(None)? {
            return Ok(Encoding::Utf8);
        }
        unsafe extern "system" {
            fn GetACP() -> u32;
        }
        // SAFETY: GetACP has no arguments and only reports a code page.
        // Other code pages would need CPython's own codec tables, so they are refused.
        match unsafe { GetACP() } {
            65001 => Ok(Encoding::Utf8),
            1252 => Ok(Encoding::Windows1252),
            _ => Err(UNSUPPORTED),
        }
    }
}

#[cfg(windows)]
fn console(stderr: bool) -> bool {
    unsafe extern "system" {
        fn GetStdHandle(which: u32) -> *mut core::ffi::c_void;
        fn GetConsoleMode(handle: *mut core::ffi::c_void, mode: *mut u32) -> i32;
    }
    let mut mode = 0;
    let which = if stderr { -12i32 as u32 } else { -11i32 as u32 };
    // SAFETY: the constant selects a process stream; mode is writable storage.
    unsafe { GetConsoleMode(GetStdHandle(which), &mut mode) != 0 }
}

fn windows1252(cp: u32) -> Option<u8> {
    if cp < 128 || (160..=255).contains(&cp) {
        return Some(cp as u8);
    }
    const SPECIAL: &[(u32, u8)] = &[
        (0x20AC, 0x80),
        (0x201A, 0x82),
        (0x0192, 0x83),
        (0x201E, 0x84),
        (0x2026, 0x85),
        (0x2020, 0x86),
        (0x2021, 0x87),
        (0x02C6, 0x88),
        (0x2030, 0x89),
        (0x0160, 0x8A),
        (0x2039, 0x8B),
        (0x0152, 0x8C),
        (0x017D, 0x8E),
        (0x2018, 0x91),
        (0x2019, 0x92),
        (0x201C, 0x93),
        (0x201D, 0x94),
        (0x2022, 0x95),
        (0x2013, 0x96),
        (0x2014, 0x97),
        (0x02DC, 0x98),
        (0x2122, 0x99),
        (0x0161, 0x9A),
        (0x203A, 0x9B),
        (0x0153, 0x9C),
        (0x017E, 0x9E),
        (0x0178, 0x9F),
    ];
    SPECIAL.iter().find(|&&(value, _)| value == cp).map(|&(_, byte)| byte)
}

fn scalar(cp: u32, encoding: &Encoding) -> Option<Vec<u8>> {
    let ch = char::from_u32(cp)?;
    match encoding {
        Encoding::Utf8 | Encoding::Utf8Sig => Some(ch.encode_utf8(&mut [0; 4]).as_bytes().to_vec()),
        Encoding::Ascii => (cp < 128).then_some(vec![cp as u8]),
        Encoding::Latin1 => (cp < 256).then_some(vec![cp as u8]),
        Encoding::Windows1252 => windows1252(cp).map(|byte| vec![byte]),
    }
}

fn escaped(cp: u32) -> Vec<u8> {
    if cp < 256 {
        format!("\\x{cp:02x}").into_bytes()
    } else if cp < 65536 {
        format!("\\u{cp:04x}").into_bytes()
    } else {
        format!("\\U{cp:08x}").into_bytes()
    }
}

#[derive(Debug)]
pub struct EncodeError;

#[derive(Clone, Copy)]
pub enum ErrorPolicy {
    Strict,
    SurrogateEscape,
    SurrogatePass,
    Ignore,
    Replace,
    BackslashReplace,
}

/// CPython 3.14's stdout error handler. An explicit PYTHONIOENCODING handler wins;
/// an encoding named without one means strict. Otherwise UTF-8 mode, a C or POSIX
/// LC_CTYPE or a PEP 538 coercion target means surrogateescape, and anything else strict.
fn stdout_policy() -> Result<ErrorPolicy, &'static str> {
    let env = std::env::var("PYTHONIOENCODING").unwrap_or_default();
    let (name, errors) = env.split_once(':').unwrap_or((env.as_str(), ""));
    if !errors.is_empty() {
        return Ok(match errors {
            "surrogateescape" => ErrorPolicy::SurrogateEscape,
            "surrogatepass" => ErrorPolicy::SurrogatePass,
            "ignore" => ErrorPolicy::Ignore,
            "replace" => ErrorPolicy::Replace,
            "backslashreplace" => ErrorPolicy::BackslashReplace,
            _ => ErrorPolicy::Strict,
        });
    }
    if !name.is_empty() {
        return Ok(ErrorPolicy::Strict);
    }
    #[cfg(not(windows))]
    let escape = {
        let (locale, _) = posix_locale();
        utf8_mode(locale.as_deref())?
            || matches!(locale.as_deref(), Some("C" | "POSIX" | "C.UTF-8" | "C.utf8" | "UTF-8"))
    };
    #[cfg(windows)]
    let escape = utf8_mode(None)?;
    Ok(if escape {
        ErrorPolicy::SurrogateEscape
    } else {
        ErrorPolicy::Strict
    })
}

pub fn encode(text: &str, stderr: bool, encoding: &Encoding) -> Result<Vec<u8>, EncodeError> {
    let policy = if stderr {
        ErrorPolicy::BackslashReplace
    } else {
        // `configure` has already refused any configuration this cannot settle.
        stdout_policy().unwrap_or(ErrorPolicy::Strict)
    };
    encode_with(text, encoding, policy)
}

pub fn encode_with(text: &str, encoding: &Encoding, policy: ErrorPolicy) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::new();
    for cp in pytext::code_points(text) {
        if cfg!(windows) && cp == 10 {
            out.push(13);
        }
        if let Some(bytes) = scalar(cp, encoding) {
            out.extend_from_slice(&bytes);
            continue;
        }
        match policy {
            ErrorPolicy::SurrogateEscape if (0xDC80..=0xDCFF).contains(&cp) => out.push((cp - 0xDC00) as u8),
            ErrorPolicy::SurrogatePass
                if (0xD800..=0xDFFF).contains(&cp) && matches!(encoding, Encoding::Utf8 | Encoding::Utf8Sig) =>
            {
                out.extend_from_slice(&[
                    0xE0 | ((cp >> 12) as u8),
                    0x80 | (((cp >> 6) & 63) as u8),
                    0x80 | ((cp & 63) as u8),
                ]);
            }
            ErrorPolicy::BackslashReplace => out.extend(escaped(cp)),
            ErrorPolicy::Ignore => {}
            ErrorPolicy::Replace => out.push(b'?'),
            _ => return Err(EncodeError),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_implemented_codecs_are_accepted_by_their_cpython_spellings() {
        for name in ["utf-8", "UTF8", "utf_8", "U8", "cp65001"] {
            assert!(matches!(codec(name), Ok(Encoding::Utf8)), "{name}");
        }
        assert!(matches!(codec("utf-8-sig"), Ok(Encoding::Utf8Sig)));
        assert!(matches!(codec("US-ASCII"), Ok(Encoding::Ascii)));
        assert!(matches!(codec("ISO-8859-1"), Ok(Encoding::Latin1)));
        assert!(matches!(codec("windows-1252"), Ok(Encoding::Windows1252)));
        for name in ["cp1251", "utf-16", "shift_jis", "bogus", ""] {
            assert!(codec(name).is_err(), "{name}");
        }
    }
    #[test]
    fn native_code_pages_match_python_strict_and_backslashreplace_vectors() {
        assert_eq!(
            encode("Zoë € —", false, &Encoding::Windows1252).unwrap(),
            b"Zo\xeb \x80 \x97"
        );
        assert!(encode("😀", false, &Encoding::Windows1252).is_err());
        assert_eq!(encode("😀", true, &Encoding::Windows1252).unwrap(), br"\U0001f600");
        assert!(encode("\u{81}", false, &Encoding::Windows1252).is_err());
        assert_eq!(encode("é", true, &Encoding::Ascii).unwrap(), br"\xe9");
        assert_eq!(encode("é", false, &Encoding::Latin1).unwrap(), b"\xe9");
        let mut lone = String::new();
        pytext::push_code_point(&mut lone, 0xD800);
        assert!(encode(&lone, false, &Encoding::Utf8).is_err());
        assert_eq!(encode(&lone, true, &Encoding::Utf8).unwrap(), br"\ud800");
    }
}
