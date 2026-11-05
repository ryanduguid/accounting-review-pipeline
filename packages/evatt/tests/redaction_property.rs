// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! A checksum-valid TFN, ABN, ACN or Medicare number planted in random Markdown
//! never survives: redact replaces it, or the run halts on a candidate that holds
//! it. Numbers are made up as the test runs and persist nowhere, so no failure
//! file can hold one.
use evatt::{
    entities::Entity,
    patterns,
    pyjson::PyStr,
    redact::{self, Unknown},
};
use proptest::{prelude::*, test_runner::Config};

const TFN_GROUPS: [usize; 3] = [3, 3, 3];
const ABN_GROUPS: [usize; 4] = [2, 3, 3, 3];
const MEDICARE_GROUPS: [usize; 3] = [4, 5, 1];

/// What can sit between digit groups in a workpaper or a PDF conversion.
const GAPS: [&str; 46] = [
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
    " / ",
    " . ",
    " , ",
    ",   ",
    ".   ",
    "; ",
    ":",
    ") (",
    " | ",
    "|",
    "\u{feff}",
    "\u{200e}",
    "\u{180e}",
    "\u{ff0d}",
    "~~",
    "`",
    "&nbsp;",
    "&#160;",
    "<td>",
    "<span>",
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

fn acn(head: Vec<u8>) -> String {
    let head: String = head.iter().map(|d| char::from(b'0' + d)).collect();
    (0..=9)
        .map(|check| format!("{head}{check}"))
        .find(|digits| patterns::valid("acn", digits).unwrap())
        .expect("the check digit is a remainder from 0 to 9")
}

/// A card number: the check digit sits in the ninth place, before the issue digit.
fn medicare(digits: Vec<u8>) -> Option<String> {
    let digits: String = digits.iter().map(|d| char::from(b'0' + d)).collect();
    let (head, issue) = digits.split_at(8);
    (0..=9)
        .map(|check| format!("{head}{check}{issue}"))
        .find(|number| patterns::valid("medicare", number).unwrap())
}

/// The digits and the widths of their usual groups.
fn identifier() -> impl Strategy<Value = (String, &'static [usize])> {
    prop_oneof![
        prop::collection::vec(0u8..=9, 8)
            .prop_filter_map("no check digit fits", tfn)
            .prop_map(|digits| (digits, TFN_GROUPS.as_slice())),
        prop::collection::vec(0u8..=9, 9).prop_map(|tail| (abn(tail), ABN_GROUPS.as_slice())),
        prop::collection::vec(0u8..=9, 8).prop_map(|head| (acn(head), TFN_GROUPS.as_slice())),
        (2u8..=6, prop::collection::vec(0u8..=9, 8)).prop_filter_map("no check digit fits", |(first, rest)| {
            let digits = std::iter::once(first).chain(rest).collect();
            medicare(digits).map(|number| (number, MEDICARE_GROUPS.as_slice()))
        }),
    ]
}

/// A listed gap, or any same-line text that holds no letter or digit.
fn gap() -> impl Strategy<Value = String> {
    prop_oneof![
        prop::sample::select(GAPS.as_slice()).prop_map(str::to_owned),
        r"[^\p{L}\p{N}#\n]{1,8}".prop_filter("a letter by another name", |gap| {
            !gap.chars().any(char::is_alphanumeric)
        }),
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

/// `text` as a reader takes it for numbers: the digits of `&#160;` are no part of one.
fn number_text(text: &str) -> String {
    let mut kept = String::new();
    let mut rest = text;
    while let Some((head, tail)) = rest.split_once("&#") {
        kept.push_str(head);
        rest = tail.split_once(';').map_or(tail, |(_, after)| after);
    }
    kept.push_str(rest);
    kept
}

fn number_digits(text: &str) -> String {
    digits(&number_text(text))
}

/// The output as `number_text`, without its placeholders.
fn surviving_text(text: &str) -> String {
    let placeholders = patterns::matches("CARRIED_PLACEHOLDER", text).unwrap();
    let mut kept = String::new();
    let mut cursor = 0;
    for placeholder in &placeholders {
        let token = placeholder.name("token").unwrap();
        kept.push_str(&text[cursor..token.start()]);
        cursor = token.end();
    }
    kept.push_str(&text[cursor..]);
    number_text(&kept)
}

/// How often `group` stands as a whole run of digits in `text`.
fn group_count(text: &str, group: &str) -> usize {
    text.split(|c: char| !c.is_ascii_digit())
        .filter(|run| *run == group)
        .count()
}

/// Whether any of the plant is left in the output with no candidate covering it:
/// the whole spelling, its digits, or one digit group the rest of the document
/// does not explain.
fn survives(document: &str, text: &str, unknowns: &[Unknown], plant: &str, number: &str) -> bool {
    let named = unknowns
        .iter()
        .any(|u| u.value.contains(plant) || number_digits(&u.value).contains(number));
    let kept = surviving_text(text);
    let elsewhere = number_text(&document.replace(plant, ""));
    let group_left = number_text(plant)
        .split(|c: char| !c.is_ascii_digit())
        .filter(|group| !group.is_empty())
        .any(|group| group_count(&kept, group) > group_count(&elsewhere, group));
    !named && (text.contains(plant) || digits(&kept).contains(number) || group_left)
}

/// The fixed grid: a known TFN and ABN, in every spelling and every context.
#[test]
fn every_spelling_in_every_context_is_replaced_or_flagged() {
    let mut survivors = Vec::new();
    for (number, groups) in [
        ("123456782", &TFN_GROUPS[..]),
        ("51824753556", &ABN_GROUPS[..]),
        ("004085616", &TFN_GROUPS[..]),
        ("2123456701", &MEDICARE_GROUPS[..]),
    ] {
        for gap in GAPS {
            for (open, close) in CONTEXTS {
                let plant = spelled(number, groups, gap);
                let document = format!("Notes.\n\n{open}{plant}{close}\n\nMore notes.\n");
                let result = redact::redact(&document, &[]).unwrap();
                if survives(&document, &result.text, &result.unknowns, &plant, number) {
                    survivors.push(document);
                }
            }
        }
    }
    assert!(survivors.is_empty(), "{} survived: {survivors:#?}", survivors.len());
}

/// Each document holds the number, and a candidate must quote all of it.
fn hidden<'a>(cases: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut hidden = cases.to_vec();
    hidden.retain(|(document, number)| {
        let result = redact::redact(document, &[]).unwrap();
        !result.unknowns.iter().any(|u| number_digits(&u.value).contains(number))
    });
    hidden
}

/// Digits beside the number once chained into a shorter match that hid its tail,
/// or into a token the number's own candidate was then dropped for.
#[test]
fn digits_beside_a_split_number_never_hide_it() {
    let found = hidden(&[
        ("1. 63   095   645   545\n", "63095645545"),
        ("12345678 123   456   782\n", "123456782"),
    ]);
    assert!(found.is_empty(), "{found:?}");
}

/// A digit run was once atomic, so digits touching the number hid it.
#[test]
fn digits_touching_a_split_number_never_hide_it() {
    let found = hidden(&[
        ("9123   456   782\n", "123456782"),
        ("123   456   7829\n", "123456782"),
        ("2026123   456   782\n", "123456782"),
        ("951   824   753   556\n", "51824753556"),
        ("51   824   753   5569\n", "51824753556"),
    ]);
    assert!(found.is_empty(), "{found:?}");
}

/// Gaps with no letter or digit in them, which a list of joiners once missed.
#[test]
fn a_gap_with_no_letter_or_digit_never_hides_a_number() {
    let number = "123456782";
    let found = hidden(&[
        ("123 / 456 / 782\n", number),
        ("123 . 456 . 782\n", number),
        ("123 , 456 , 782\n", number),
        ("123,   456,   782\n", number),
        ("123.   456.   782\n", number),
        ("123; 456; 782\n", number),
        ("123:456:782\n", number),
        ("(123) (456) (782)\n", number),
        ("| 123 | 456 | 782 |\n", number),
        ("123|456|782\n", number),
        ("123\u{feff}456\u{feff}782\n", number),
        ("123\u{200e}456\u{200e}782\n", number),
        ("123\u{180e}456\u{180e}782\n", number),
        ("123\u{ff0d}456\u{ff0d}782\n", number),
        ("123~~456~~782\n", number),
        ("123`456`782\n", number),
        ("123&nbsp;456&nbsp;782\n", number),
        ("123&#160;456&#160;782\n", number),
        ("123&456&782;\n", number),
        ("<td>123</td><td>456</td><td>782</td>\n", number),
        ("<span>123</span> <span>456</span> <span>782</span>\n", number),
    ]);
    assert!(found.is_empty(), "{found:?}");
}

/// Plain digit concatenation reads this spelling as `123160456160782`, which would
/// let an unchanged document pass the grid.
#[test]
fn an_unchanged_reference_spelling_counts_as_a_survivor() {
    let (plant, number) = ("123&#160;456&#160;782", "123456782");
    assert!(!digits(plant).contains(number));
    assert!(survives(plant, plant, &[], plant, number));
    let document = format!("{plant}\n");
    let result = redact::redact(&document, &[]).unwrap();
    assert!(!survives(&document, &result.text, &result.unknowns, plant, number));
}

/// Neither the spelling nor its joined digits remain here, yet a group does.
#[test]
fn a_partial_replacement_counts_as_a_survivor() {
    let (plant, number) = ("123 456 782", "123456782");
    let document = format!("Notes {plant}\n");
    assert!(survives(&document, "Notes TFN_01 782\n", &[], plant, number));
    assert!(!survives(&document, "Notes TFN_01\n", &[], plant, number));
}

/// The partial test found this: `404` and the wrapped `667330` are a valid ACN by
/// chance, so pass one replaced them and left `142   240` with no halt.
#[test]
fn a_number_cut_by_a_chance_match_is_named() {
    let (document, plant, number) = ("\n> 142   240   404\n667330 ", "142   240   404", "142240404");
    let result = redact::redact(document, &[]).unwrap();
    assert!(result.text.contains("142   240"));
    assert!(!survives(document, &result.text, &result.unknowns, plant, number));
    assert_eq!(result.unknowns[0].line, 2);
    assert_eq!(&*result.unknowns[0].context, "> 142   240   ACN_01");
}

/// The triage file says to map a false positive, and that must clear this halt.
#[test]
fn a_mapped_remainder_clears_a_cut_number() {
    let entries = [Entity {
        value: PyStr::from("142   240"),
        placeholder: "CLIENT_01".into(),
        kind: "client".into(),
        added: "2026-11-06".into(),
    }];
    let result = redact::redact("\n> 142   240   404\n667330 ", &entries).unwrap();
    assert!(result.unknowns.is_empty(), "{:?}", result.unknowns);
}

/// A label says a figure is an identifier, however like money it looks.
#[test]
fn money_beside_an_identifier_label_is_joined() {
    let found = hidden(&[
        ("TFN: 1234.56   782\n", "123456782"),
        ("tax file number 1234.56 | 782\n", "123456782"),
        ("1234.56   782 (TFN)\n", "123456782"),
        ("| ABN | 5182.47   53556 |\n", "51824753556"),
        ("Medicare card: 2123.45   6701\n", "2123456701"),
        ("T<span>F</span>N: 1234.56   782\n", "123456782"),
        ("<b>TFN</b> 1234.56   782\n", "123456782"),
        ("<td>Holder</td><td>TFN</td><td>1234.56   782</td>\n", "123456782"),
        ("tax<br>file number 1234.56   782\n", "123456782"),
    ]);
    assert!(found.is_empty(), "{found:?}");
    let document = "TFN: 1234.56   782\n";
    let before = redact::redact(document, &[]).unwrap();
    assert_eq!(before.text, document, "a label halts; it never replaces");
}

/// A chain of money would nearly always pass a check somewhere, so money ends one.
/// The digits of each row hold a valid TFN across the gap.
#[test]
fn amounts_side_by_side_do_not_join() {
    for document in [
        "$1234.56   $782.00\n",
        "| 1234.56 | 782.00 |\n",
        "1,234.56 / 782.00\n",
        "1234.56\n782.00\n",
        "(1,234.56)   (782.00)\n",
    ] {
        let result = redact::redact(document, &[]).unwrap();
        assert!(result.unknowns.is_empty(), "{document:?}: {:?}", result.unknowns);
    }
}

/// The 20,000 case run found this: the address mask took the last group, so the
/// document halted on `108\nAa Ct` with the number named nowhere.
#[test]
fn a_number_running_into_an_address_is_named() {
    let found = hidden(&[
        ("000\u{2013}500\u{2013}108\nAa Ct\n", "000500108"),
        ("123   456   782 Aa Street\n", "123456782"),
        ("12345 678\nAa Ct\n", "12345678"),
        ("9123   456   782\nAa Ct\n", "9123456782"),
    ]);
    assert!(found.is_empty(), "{found:?}");
}

/// The 40,000 case run found this: a vertical tab starts a line, so `00002.` read as
/// a list marker, and its ambiguous candidate dropped the number's own. A digit on
/// the end leaves the whole span invalid, and only a window inside it valid.
#[test]
fn an_ambiguous_group_inside_a_valid_number_does_not_hide_it() {
    let found = hidden(&[
        ("TFN: 2000.\u{b}00002.\u{b}0\n", "2000000020"),
        ("TFN: 2000.\u{b}00002.\u{b}09\n", "2000000020"),
    ]);
    assert!(found.is_empty(), "{found:?}");
}

/// A valid number wrapped across lines still halts; a digit beside it does not
/// make part of an invalid one halt. Python 0.2.0 holds the second half.
#[test]
fn a_wrap_joins_whole_groups_only() {
    assert!(hidden(&[("123   456\n782\n", "123456782")]).is_empty());
    let result = redact::redact("51 824\n753557. next\n", &[]).unwrap();
    assert!(result.unknowns.is_empty(), "{:?}", result.unknowns);
}

/// Tokens the sweep already names stay separate values, line by line.
#[test]
fn a_split_window_over_named_tokens_adds_nothing() {
    let unknowns = redact::redact("1234 5678; 8765 4321\n", &[]).unwrap().unknowns;
    let values: Vec<_> = unknowns.iter().map(|u| u.value.to_string()).collect();
    assert_eq!(values, ["1234 5678", "8765 4321"]);
}

/// The README's limits: each of these leaves redaction with no halt.
#[test]
fn the_documented_limits_still_pass() {
    for document in [
        "12   345   678\n",
        "1234.56   782\n",
        "TFN\n1234.56   782\n",
        "abnormal 1234.56   782\n",
        "123 and 456 and 782\n",
        "123 # 456 # 782\n",
        "123   456\n\n782\n",
        "9123   456\n782\n",
        "<td id=\"7\">123</td><td id=\"7\">456</td><td id=\"7\">782</td>\n",
        "T&#70;N: 1234.56   782\n",
    ] {
        let result = redact::redact(document, &[]).unwrap();
        assert!(result.unknowns.is_empty(), "{document:?}: {:?}", result.unknowns);
        assert_eq!(result.text, document);
    }
}

/// The name pattern is frozen with the Python contract, and it crosses blank lines.
#[test]
fn a_name_candidate_can_cross_a_blank_line() {
    let unknowns = redact::redact("Bank Balance\n\nTrade Debtors\n", &[]).unwrap().unknowns;
    assert!(
        unknowns.iter().any(|u| u.value.to_string().contains("\n\n")),
        "{unknowns:?}"
    );
}

/// The structured pattern takes this spelling, so no sweep need hold it.
#[test]
fn groups_on_their_own_paragraphs_are_replaced() {
    for (document, group) in [("123\n\n456\n\n782\n", "456"), ("51\n\n824\n\n753\n\n556\n", "824")] {
        let result = redact::redact(document, &[]).unwrap();
        assert!(
            !digits(&surviving_text(&result.text)).contains(group),
            "{}",
            result.text
        );
    }
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
        gap in gap(),
        context in prop::sample::select(CONTEXTS.as_slice()),
        before in filler(),
        after in filler(),
        crlf: bool,
    ) {
        let (open, close) = context;
        let plant = spelled(&number, groups, &gap);
        let document = format!("{before}\n{open}{plant}{close}\n{after}");
        let (document, plant) = if crlf {
            (document.replace('\n', "\r\n"), plant.replace('\n', "\r\n"))
        } else {
            (document, plant)
        };
        let result = redact::redact(&document, &[]).unwrap();
        // A halt writes nothing, but a candidate that holds some other text would
        // send the operator away with the number still in the file.
        prop_assert!(
            !survives(&document, &result.text, &result.unknowns, &plant, &number),
            "{plant:?} survived in {document:?}"
        );
    }
}
