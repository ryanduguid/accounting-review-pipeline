// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! A checksum-valid TFN or ABN planted in random Markdown never survives: redact
//! replaces it, or the run halts on a candidate that holds it. Numbers are made
//! up as the test runs and persist nowhere, so no failure file can hold one.
use evatt::{patterns, redact};
use proptest::{prelude::*, test_runner::Config};

const TFN_GROUPS: [usize; 3] = [3, 3, 3];
const ABN_GROUPS: [usize; 4] = [2, 3, 3, 3];

/// What can sit between digit groups in a workpaper or a PDF conversion.
const GAPS: [&str; 26] = [
    "",
    " ",
    "  ",
    "   ",
    "      ",
    "-",
    " - ",
    "\t",
    "\t\t\t",
    "\n",
    "\n  ",
    "\n> ",
    "\u{a0}",
    "\u{a0}\u{a0}\u{a0}",
    "\u{2009}",
    "\u{2013}",
    " \u{2013} ",
    ".",
    ",",
    "/",
    "\u{200b}",
    "\u{ad}",
    "*",
    "**",
    "_",
    "<br>",
];

/// Markdown around the number, as the text before it and the text after it.
const CONTEXTS: [(&str, &str); 38] = [
    ("", ""),
    ("| ", " |"),
    ("| x | ", " | y |"),
    ("|", "|"),
    ("**", "**"),
    ("__", "__"),
    ("*", "*"),
    ("_", "_"),
    ("~~", "~~"),
    ("`", "`"),
    ("```\n", "\n```"),
    ("    ", ""),
    ("[", "](https://example.com/a)"),
    ("[a](https://example.com/", ")"),
    ("![", "](a.png)"),
    ("<", ">"),
    ("<td>", "</td>"),
    ("<!-- ", " -->"),
    ("# ", ""),
    ("## ", " ##"),
    ("> ", ""),
    ("> > ", ""),
    ("- ", ""),
    ("* ", ""),
    ("1. ", ""),
    ("[^1]: ", ""),
    ("TFN: ", ""),
    ("TFN ", "."),
    ("**TFN:** ", ""),
    ("Tax file number: ", ""),
    ("ABN: ", ""),
    ("ABN ", ","),
    ("**ABN** ", ""),
    ("Australian Business Number: ", ""),
    ("(", ")"),
    ("\"", "\""),
    ("ref=", ";"),
    ("a_", "_b"),
];

fn tfn(head: Vec<u8>) -> Option<String> {
    let head: String = head.iter().map(|d| char::from(b'0' + d)).collect();
    (0..=9)
        .map(|check| format!("{head}{check}"))
        .find(|digits| patterns::valid("tfn", digits).unwrap())
}

fn abn(tail: Vec<u8>) -> String {
    let tail: String = tail.iter().map(|d| char::from(b'0' + d)).collect();
    (10..=99)
        .map(|check| format!("{check}{tail}"))
        .find(|digits| patterns::valid("abn", digits).unwrap())
        .expect("the 90 check pairs cover every remainder modulo 89")
}

/// The digits and the widths of their usual groups.
fn identifier() -> impl Strategy<Value = (String, &'static [usize])> {
    prop_oneof![
        prop::collection::vec(0u8..=9, 8)
            .prop_filter_map("no check digit fits", tfn)
            .prop_map(|digits| (digits, TFN_GROUPS.as_slice())),
        prop::collection::vec(0u8..=9, 9).prop_map(|tail| (abn(tail), ABN_GROUPS.as_slice())),
    ]
}

fn spelled(digits: &str, groups: &[usize], gap: &str) -> String {
    let mut parts = Vec::new();
    let mut start = 0;
    for width in groups {
        parts.push(&digits[start..start + width]);
        start += width;
    }
    parts.join(gap)
}

fn filler() -> impl Strategy<Value = String> {
    let fragment = prop_oneof![
        "[a-z]{1,9}",
        "[A-Z][a-z]{1,9}",
        "[0-9]{1,7}",
        r"\$?[0-9]{1,3}(,[0-9]{3}){0,2}\.[0-9]{2}",
        "20[0-9]{2}-[01][0-9]-[0-3][0-9]",
        r"[-*_`#>|\[\]()!~]{1,4}",
        "[ -~]{0,16}",
        r"\PC{0,6}",
    ];
    let joined = prop_oneof![Just(" "), Just("\n"), Just("\n\n"), Just(" | ")];
    prop::collection::vec((fragment, joined), 0..12).prop_map(|parts| {
        parts
            .into_iter()
            .flat_map(|(text, gap)| [text, gap.to_owned()])
            .collect()
    })
}

fn digits(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

/// The output's digits, without the ones in its placeholders.
fn surviving_digits(text: &str) -> String {
    let placeholders = patterns::matches("CARRIED_PLACEHOLDER", text).unwrap();
    let mut kept = String::new();
    let mut cursor = 0;
    for placeholder in &placeholders {
        let token = placeholder.name("token").unwrap();
        kept.push_str(&text[cursor..token.start()]);
        cursor = token.end();
    }
    kept.push_str(&text[cursor..]);
    digits(&kept)
}

/// The fixed grid: a known TFN and ABN, in every spelling and every context.
#[test]
fn every_spelling_in_every_context_is_replaced_or_flagged() {
    let mut survivors = Vec::new();
    for (number, groups) in [("123456782", &TFN_GROUPS[..]), ("51824753556", &ABN_GROUPS[..])] {
        for gap in GAPS {
            for (open, close) in CONTEXTS {
                let plant = spelled(number, groups, gap);
                let document = format!("Notes.\n\n{open}{plant}{close}\n\nMore notes.\n");
                let result = redact::redact(&document, &[]).unwrap();
                let flagged = result.unknowns.iter().any(|u| digits(&u.value).contains(number));
                if !flagged && surviving_digits(&result.text).contains(number) {
                    survivors.push(document);
                }
            }
        }
    }
    assert!(survivors.is_empty(), "{} survived: {survivors:#?}", survivors.len());
}

/// Digits beside the number once chained into a shorter match that hid its tail,
/// or into a token the number's own candidate was then dropped for.
#[test]
fn digits_beside_a_split_number_never_hide_it() {
    let hidden: Vec<_> = [
        ("1. 63   095   645   545\n", "63095645545"),
        ("12345678 123   456   782\n", "123456782"),
    ]
    .into_iter()
    .filter(|(document, number)| {
        let result = redact::redact(document, &[]).unwrap();
        !result.unknowns.iter().any(|u| digits(&u.value).contains(number))
    })
    .collect();
    assert!(hidden.is_empty(), "{hidden:?}");
}

proptest! {
    #![proptest_config(Config {
        cases: 512,
        failure_persistence: None,
        ..Config::default()
    })]

    #[test]
    fn a_planted_tfn_or_abn_never_survives_redaction(
        (number, groups) in identifier(),
        gap in prop::sample::select(GAPS.as_slice()),
        context in prop::sample::select(CONTEXTS.as_slice()),
        before in filler(),
        after in filler(),
        crlf: bool,
    ) {
        let (open, close) = context;
        let plant = spelled(&number, groups, gap);
        let document = format!("{before}\n{open}{plant}{close}\n{after}");
        let document = if crlf { document.replace('\n', "\r\n") } else { document };
        let result = redact::redact(&document, &[]).unwrap();
        // A halt writes nothing, but a candidate that holds some other text would
        // send the operator away with the number still in the file.
        let flagged = result.unknowns.iter().any(|u| digits(&u.value).contains(&number));
        prop_assert!(
            flagged || !surviving_digits(&result.text).contains(&number),
            "{plant:?} survived in {document:?}"
        );
    }
}
