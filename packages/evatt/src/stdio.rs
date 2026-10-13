//! CPython 3.14's native text-stream encoding, independent of installed Python.
//! Data files are always explicitly UTF-8; this applies only to CLI diagnostics.
use crate::pytext;

pub enum Encoding {
    Utf8,
    Utf8Sig,
    Ascii,
    Latin1,
    Windows1252,
    #[cfg(windows)]
    Native(u32),
}

fn override_encoding() -> Option<Encoding> {
    let raw = std::env::var("PYTHONIOENCODING").ok()?;
    match raw
        .split(':')
        .next()?
        .to_ascii_lowercase()
        .replace(['-', '_'], "")
        .as_str()
    {
        "utf8" => Some(Encoding::Utf8),
        "utf8sig" => Some(Encoding::Utf8Sig),
        "ascii" | "usascii" => Some(Encoding::Ascii),
        "latin1" | "iso88591" => Some(Encoding::Latin1),
        "cp1252" | "windows1252" => Some(Encoding::Windows1252),
        _ => None,
    }
}

pub fn encoding(stderr: bool) -> Encoding {
    #[cfg(not(windows))]
    {
        let _ = stderr;
        override_encoding().unwrap_or(Encoding::Utf8)
    }
    #[cfg(windows)]
    {
        let console = console(stderr) && std::env::var_os("PYTHONLEGACYWINDOWSSTDIO").is_none();
        if console {
            return Encoding::Utf8;
        }
        if let Some(encoding) = override_encoding() {
            return encoding;
        }
        if std::env::var("PYTHONUTF8").is_ok_and(|v| v == "1") {
            return Encoding::Utf8;
        }
        unsafe extern "system" {
            fn GetACP() -> u32;
        }
        // SAFETY: GetACP has no arguments and only reports a code page.
        match unsafe { GetACP() } {
            65001 => Encoding::Utf8,
            1252 => Encoding::Windows1252,
            code => Encoding::Native(code),
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
        #[cfg(windows)]
        Encoding::Native(code) => native(ch, *code),
    }
}

#[cfg(windows)]
fn native(ch: char, code: u32) -> Option<Vec<u8>> {
    unsafe extern "system" {
        fn WideCharToMultiByte(
            code: u32,
            flags: u32,
            wide: *const u16,
            len: i32,
            bytes: *mut u8,
            capacity: i32,
            default: *const u8,
            used_default: *mut i32,
        ) -> i32;
    }
    let mut wide = [0; 2];
    let wide = ch.encode_utf16(&mut wide);
    let mut out = [0; 16];
    let mut replaced = 0;
    // SAFETY: both buffers live through the call and their lengths are exact.
    let size = unsafe {
        WideCharToMultiByte(
            code,
            0x400,
            wide.as_ptr(),
            wide.len() as i32,
            out.as_mut_ptr(),
            16,
            std::ptr::null(),
            &mut replaced,
        )
    };
    (size > 0 && replaced == 0).then(|| out[..size as usize].to_vec())
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

fn stdout_policy() -> ErrorPolicy {
    let env = std::env::var("PYTHONIOENCODING").unwrap_or_default();
    let encoding_given = !env.split(':').next().unwrap_or_default().is_empty();
    match env.split_once(':').map(|(_, errors)| errors) {
        None | Some("") if encoding_given => ErrorPolicy::Strict,
        None | Some("") | Some("surrogateescape") => ErrorPolicy::SurrogateEscape,
        Some("surrogatepass") => ErrorPolicy::SurrogatePass,
        Some("ignore") => ErrorPolicy::Ignore,
        Some("replace") => ErrorPolicy::Replace,
        Some("backslashreplace") => ErrorPolicy::BackslashReplace,
        _ => ErrorPolicy::Strict,
    }
}

pub fn encode(text: &str, stderr: bool, encoding: &Encoding) -> Result<Vec<u8>, EncodeError> {
    encode_with(
        text,
        encoding,
        if stderr {
            ErrorPolicy::BackslashReplace
        } else {
            stdout_policy()
        },
    )
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
