//! UTF-8 with replacement and the platform's text newline convention.
use std::io::Write;
thread_local! {
    static FAILED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub(crate) fn failed() -> bool {
    FAILED.replace(false)
}
fn write(mut stream: impl Write, bytes: &[u8]) {
    if stream.write_all(bytes).is_err() {
        FAILED.set(true);
    }
}

#[cfg(any(test, feature = "probe"))]
thread_local! {
    static CAPTURE: std::cell::RefCell<Option<(String,String)>> = const {std::cell::RefCell::new(None)};
}

#[cfg(any(test, feature = "probe"))]
pub(crate) fn capture<T>(action: impl FnOnce() -> T) -> (T, String, String) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CAPTURE.replace(None);
        }
    }
    assert!(
        CAPTURE
            .replace(Some((String::new(), String::new())))
            .is_none()
    );
    let _reset = Reset;
    let result = action();
    let (stdout, stderr) = CAPTURE.take().unwrap();
    (result, stdout, stderr)
}

#[cfg(any(test, feature = "probe"))]
fn captured(text: &str, stderr: bool) -> bool {
    CAPTURE.with_borrow_mut(|captured| {
        if let Some((out, err)) = captured {
            if stderr {
                err.push_str(text)
            } else {
                out.push_str(text)
            };
            true
        } else {
            false
        }
    })
}

fn bytes(text: &str) -> Vec<u8> {
    let mut encoded = String::new();
    for cp in crate::pytext::code_points(text) {
        let ch = char::from_u32(cp).unwrap_or('?');
        if cfg!(windows) && ch == '\n' {
            encoded.push('\r');
        }
        encoded.push(ch);
    }
    encoded.into_bytes()
}
pub fn out(text: &str) {
    #[cfg(any(test, feature = "probe"))]
    if captured(text, false) {
        return;
    }
    write(std::io::stdout().lock(), &bytes(text));
}
/// A verified UTF-8 file's text contains no OS surrogate carriers.
pub fn out_literal(text: &str) {
    #[cfg(any(test, feature = "probe"))]
    if captured(text, false) {
        return;
    }
    let text = if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.to_owned()
    };
    write(std::io::stdout().lock(), text.as_bytes());
}
pub fn err(text: &str) {
    #[cfg(any(test, feature = "probe"))]
    if captured(text, true) {
        return;
    }
    write(std::io::stderr().lock(), &bytes(text));
}

#[macro_export]
macro_rules! out { ($($arg:tt)*) => { $crate::stdio::out(&format!($($arg)*)) }; }
#[macro_export]
macro_rules! outln { ($($arg:tt)*) => { $crate::stdio::out(&format!("{}\n",format!($($arg)*))) }; }
#[macro_export]
macro_rules! err { ($($arg:tt)*) => { $crate::stdio::err(&format!($($arg)*)) }; }
#[macro_export]
macro_rules! errln { ($($arg:tt)*) => { $crate::stdio::err(&format!("{}\n",format!($($arg)*))) }; }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_stream_writes_are_observable() {
        struct Refused;
        impl Write for Refused {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(!failed());
        assert_eq!(
            crate::cli::command_thread(|| {
                write(Refused, b"status");
                0
            }),
            1
        );
        write(Refused, b"status");
        assert!(failed());
        assert!(!failed());
    }
    #[test]
    fn replaces_surrogates_and_preserves_text_newlines() {
        let text = crate::pyjson::PyStr(vec![0xD800, 10, 0x10f7ff, 0x10f800]).os_text();
        assert_eq!(
            bytes(&text),
            format!(
                "?{}\u{10f7ff}\u{10f800}",
                if cfg!(windows) { "\r\n" } else { "\n" }
            )
            .as_bytes()
        );
    }
}
