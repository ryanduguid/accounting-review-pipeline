//! UTF-8 with replacement and the platform's text newline convention.
use std::io::Write;

#[cfg(test)]
thread_local! {
    static CAPTURE: std::cell::RefCell<Option<(String,String)>> = const {std::cell::RefCell::new(None)};
}

#[cfg(test)]
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

#[cfg(test)]
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
    #[cfg(test)]
    if captured(text, false) {
        return;
    }
    let _ = std::io::stdout().lock().write_all(&bytes(text));
}
pub fn err(text: &str) {
    #[cfg(test)]
    if captured(text, true) {
        return;
    }
    let _ = std::io::stderr().lock().write_all(&bytes(text));
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
