//! EMAIL's leftmost-first regex semantics with bounded scanning.
//! The original `\b[\w.+-]+@...` retries the entire suffix at every word
//! boundary of a failing `9-9-...` run, giving quadratic work. Index a maximal
//! local-part run once. A domain is examined once after each '@'; characters
//! in a rejected domain can then be revisited by one local-part run. Each
//! character is inspected a bounded number of times, including terminal
//! boundary backoff. The literal regex remains in the probe as the oracle.
use crate::{patterns::Span, pytext};

fn word(c: char) -> bool {
    pytext::is_word(c as u32)
}
fn local(c: char) -> bool {
    word(c) || ".+-".contains(c)
}
fn domain(c: char) -> bool {
    word(c) || c == '-'
}

fn boundary(chars: &[(usize, char)], pos: usize) -> bool {
    let before = pos > 0 && word(chars[pos - 1].1);
    let after = pos < chars.len() - 1 && word(chars[pos].1);
    before != after
}

fn domain_end(chars: &[(usize, char)], mut pos: usize) -> Option<usize> {
    let mut end = None;
    let first = pos;
    while pos + 1 < chars.len() && domain(chars[pos].1) {
        pos += 1;
    }
    if pos == first {
        return None;
    }
    while pos + 1 < chars.len() && chars[pos].1 == '.' {
        pos += 1;
        let start = pos;
        while pos + 1 < chars.len() && domain(chars[pos].1) {
            pos += 1;
            if boundary(chars, pos) {
                end = Some(pos);
            }
        }
        if start == pos {
            break;
        }
    }
    end
}

pub fn spans(text: &str) -> Vec<Span> {
    let mut chars: Vec<_> = text.char_indices().collect();
    chars.push((text.len(), '\0'));
    let mut pos = 0;
    let mut found = Vec::new();
    while pos + 1 < chars.len() {
        if !local(chars[pos].1) {
            pos += 1;
            continue;
        }
        let first = pos;
        let mut start = None;
        while pos + 1 < chars.len() && local(chars[pos].1) {
            if start.is_none() && boundary(&chars, pos) {
                start = Some(pos);
            }
            pos += 1;
        }
        if chars[pos].1 != '@' {
            continue;
        }
        let at = pos;
        pos += 1;
        if let Some(start) = start
            && start < at
            && let Some(end) = domain_end(&chars, pos)
        {
            let (start, end_byte) = (chars[start].0, chars[end].0);
            found.push(Span {
                start,
                end: end_byte,
                kind: "email".into(),
                value: text[start..end_byte].into(),
            });
            pos = end;
        }
        debug_assert!(pos > first);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn email_scanner_keeps_boundaries_and_greedy_terminal_backoff() {
        for text in [
            "a@b@c.d",
            "a@b.com---",
            "a@b.com.--",
            "a@b.com.--x",
            ".a@b.c",
            "a@b.c--@d.e",
            "a+a@b-c.d-e",
        ] {
            let expected: Vec<_> = crate::patterns::matches("EMAIL", text)
                .unwrap()
                .iter()
                .map(|m| {
                    let m = m.get(0).unwrap();
                    (m.start(), m.end(), m.as_str().to_string())
                })
                .collect();
            let actual: Vec<_> = spans(text).iter().map(|s| (s.start, s.end, s.value.clone())).collect();
            assert_eq!(actual, expected, "{text}");
        }
    }
}
