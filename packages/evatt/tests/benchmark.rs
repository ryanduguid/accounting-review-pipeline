// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Seeded workpaper benchmark, on the model of Monthly Close Controls' detection
//! benchmark. Each template in `evaluation/workpapers` is filled in several times
//! from one seed, so the identifiers exist only while the test runs. The counts
//! measure these fabricated documents only, never real-world accuracy, and the
//! README's Detection benchmark table must match them.
use evatt::{
    entities::Entity,
    patterns,
    pyjson::PyStr,
    redact::{self, Unknown},
};
use std::collections::BTreeSet;
use std::path::Path;

const README: &str = include_str!("../README.md");
const SEED: u64 = 20_261_106;
/// Documents per template: half with every name in the map, half without.
const DOCUMENTS: usize = 100;
const GIVEN: [&str; 8] = ["Aris", "Bryn", "Cato", "Dara", "Elio", "Fenn", "Galen", "Hana"];
const FAMILY: [&str; 8] = [
    "Varrock",
    "Falador",
    "Ardougne",
    "Lumbridge",
    "Seers",
    "Karamja",
    "Taverley",
    "Rimmington",
];
const TRADES: [&str; 4] = ["Fisheries", "Foundry", "Mining", "Crafting"];
/// ACMA's fiction range for mobile numbers.
const PHONES: [&str; 6] = [
    "0491 570 006",
    "0491 570 110",
    "0491 570 156",
    "0491 570 157",
    "0491 570 158",
    "0491 570 159",
];
const STREETS: [&str; 3] = ["Street", "Road", "Avenue"];
/// Spellings the structured patterns do not take, so only the number sweep can
/// catch them.
const SPLIT_GAPS: [&str; 7] = ["   ", " - ", " | ", " / ", "\u{200b}", "<br>", "~~"];
const IDENTIFIERS: [(&str, usize, &[usize]); 4] = [
    ("tfn", 9, &[3, 3, 3]),
    ("abn", 11, &[2, 3, 3, 3]),
    ("acn", 9, &[3, 3, 3]),
    ("medicare", 10, &[4, 5, 1]),
];
const MONTHS: [&str; 4] = ["March", "June", "October", "December"];
const HEADER: [&str; 2] = [
    "| Workpaper | Documents | Planted values | Misses | Residual fragments | Blind-spot values missed | False halts | False candidates | Halt rate |",
    "| --- | --- | --- | --- | --- | --- | --- | --- | --- |",
];

/// splitmix64, so the corpus needs no dependency and never changes under one.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, limit: usize) -> usize {
        (self.next() % limit as u64) as usize
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
    fn digits(&mut self, count: usize) -> String {
        (0..count).map(|_| char::from(b'0' + self.below(10) as u8)).collect()
    }
    /// Random digits, retried until the kind's check digit holds.
    fn checked(&mut self, kind: &str, count: usize) -> String {
        loop {
            let digits = self.digits(count);
            if patterns::valid(kind, &digits).unwrap() {
                return digits;
            }
        }
    }
}

/// How a slot's value should fare.
#[derive(Clone, Copy, PartialEq)]
enum Class {
    /// Must be replaced, or named by a halt.
    Planted,
    /// From the README's limits: expected to pass unnoticed.
    BlindSpot,
    /// An ordinary figure. A halt on it is a false candidate.
    Benign,
}

/// A value the document holds, with the text a check looks for.
struct Value {
    class: Class,
    text: String,
}

/// One filled-in template.
struct Document {
    text: String,
    values: Vec<Value>,
    entries: Vec<Entity>,
}

fn spelled(digits: &str, widths: &[usize], gap: &str) -> String {
    let mut parts = Vec::new();
    let mut start = 0;
    for width in widths {
        parts.push(&digits[start..start + width]);
        start += width;
    }
    parts.join(gap)
}

fn grouped(digits: &str, widths: &[usize]) -> String {
    spelled(digits, widths, " ")
}

fn person(rng: &mut Rng) -> String {
    format!("{} {}", rng.pick(&GIVEN), rng.pick(&FAMILY))
}

fn whole_dollars(rng: &mut Rng) -> String {
    let whole = rng.below(900_000) + 1_000;
    format!("{},{:03}", whole / 1_000, whole % 1_000)
}

fn amount(rng: &mut Rng) -> String {
    let whole = rng.below(900_000) + 1_000;
    format!("${},{:03}.{:02}", whole / 1_000, whole % 1_000, rng.below(100))
}

/// A checksum-valid number in its groups, wide apart for a `_split` kind.
fn identifier(kind: &str, rng: &mut Rng) -> Option<String> {
    let bare = kind.strip_suffix("_split").unwrap_or(kind);
    let (_, count, widths) = IDENTIFIERS.iter().find(|identifier| identifier.0 == bare)?;
    let gap = (bare != kind).then(|| rng.pick(&SPLIT_GAPS));
    let digits = rng.checked(bare, *count);
    Some(gap.map_or_else(|| grouped(&digits, widths), |gap| spelled(&digits, widths, gap)))
}

/// A slot's text and class. An `entity` kind is what the map can hold.
fn fill(kind: &str, rng: &mut Rng) -> (String, Class, Option<&'static str>) {
    if let Some(number) = identifier(kind, rng) {
        return (number, Class::Planted, None);
    }
    match kind {
        "bsb" => (format!("{}-{}", rng.digits(3), rng.digits(3)), Class::Planted, None),
        "email" => {
            let (given, family) = (rng.pick(&GIVEN), rng.pick(&FAMILY));
            (format!("{given}.{family}@example.com"), Class::Planted, None)
        }
        "phone" => (rng.pick(&PHONES).to_owned(), Class::Planted, None),
        "person" => (person(rng), Class::Planted, Some("person")),
        "client" | "entity" => {
            let name = format!("{} {} Pty Ltd", rng.pick(&FAMILY), rng.pick(&TRADES));
            (name, Class::Planted, Some("client"))
        }
        "address" => {
            let number = rng.below(200) + 1;
            let street = format!("{} {}", rng.pick(&FAMILY), rng.pick(&STREETS));
            (format!("{number} {street}"), Class::Planted, Some("entity"))
        }
        "dob" => {
            let date = format!("{} {} 19{}", rng.below(27) + 1, rng.pick(&MONTHS), rng.digits(2));
            (date, Class::Planted, Some("entity"))
        }
        "account_number" => (rng.digits(6), Class::BlindSpot, None),
        "client_reference" => (rng.digits(7), Class::BlindSpot, None),
        "initials" => (
            format!("{}. {}", &rng.pick(&GIVEN)[..1], rng.pick(&FAMILY)),
            Class::BlindSpot,
            None,
        ),
        "lower_name" => (person(rng).to_lowercase(), Class::BlindSpot, None),
        "amount" => (amount(rng), Class::Benign, None),
        "whole_dollars" => (whole_dollars(rng), Class::Benign, None),
        "digest" => {
            let digest = (0..64)
                .map(|_| char::from(b"0123456789abcdef"[rng.below(16)]))
                .collect();
            (digest, Class::Benign, None)
        }
        other => panic!("unknown slot {other}"),
    }
}

fn entry(text: &str, kind: &str, ordinal: usize) -> Entity {
    let prefix = evatt::entities::prefix(kind).unwrap();
    Entity {
        value: PyStr::from(text),
        placeholder: format!("{prefix}_{ordinal:02}"),
        kind: kind.into(),
        added: "2026-11-06".into(),
    }
}

/// Fills one template. A client is always mapped; a `mapped` document maps every
/// other name, address and date too, so nothing in it should halt.
fn render(template: &str, seed: u64, mapped: bool) -> Document {
    let mut rng = Rng(seed);
    let mut out = String::new();
    let mut values = Vec::new();
    let mut entries = Vec::new();
    let mut client = None;
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let end = start + rest[start..].find("}}").unwrap();
        let kind = &rest[start + 2..end];
        out.push_str(&rest[..start]);
        rest = &rest[end + 2..];
        // One client per document, however many times the template names it.
        let (text, class, entity) = match (kind, &client) {
            ("client", Some(name)) => (String::clone(name), Class::Planted, None),
            _ => fill(kind, &mut rng),
        };
        if kind == "client" {
            client = Some(text.clone());
        }
        if let Some(entity) = entity
            && (mapped || kind == "client" || kind == "entity")
        {
            entries.push(entry(&text, entity, entries.len() + 1));
        }
        out.push_str(&text);
        values.push(Value { class, text });
    }
    out.push_str(rest);
    Document {
        text: out,
        values,
        entries,
    }
}

/// One template's counts: documents, planted, misses, blind-spot values missed,
/// blind-spot values planted, false halts, false candidates and halts.
#[derive(Default)]
struct Counts([usize; 9]);

impl Counts {
    fn add(&mut self, other: &Counts) {
        for (total, count) in self.0.iter_mut().zip(other.0) {
            *total += count;
        }
    }
    fn row(&self, label: &str) -> String {
        let [
            documents,
            planted,
            misses,
            blind,
            blind_planted,
            false_halts,
            candidates,
            halts,
            residual,
        ] = self.0;
        let rate = (halts * 1000 + documents / 2) / documents;
        format!(
            "| {label} | {documents} | {planted} | {misses} | {residual} | {blind} of {blind_planted} | {false_halts} | {candidates} | {}.{}% |",
            rate / 10,
            rate % 10
        )
    }
}

/// How often `fragment` stands alone in `text`.
fn standing(text: &str, fragment: &str) -> usize {
    let alone = |at: usize| {
        !text[..at].chars().next_back().is_some_and(char::is_alphanumeric)
            && !text[at + fragment.len()..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric)
    };
    text.match_indices(fragment).filter(|(at, _)| alone(*at)).count()
}

/// `text` with each candidate blanked where it sits, on its own line, so that a
/// candidate elsewhere cannot excuse a fragment.
fn without_candidates(text: &str, unknowns: &[Unknown]) -> String {
    let mut bytes = text.as_bytes().to_vec();
    for unknown in unknowns {
        for (at, _) in text.match_indices(&unknown.value) {
            if 1 + text[..at].matches('\n').count() == unknown.line {
                bytes[at..at + unknown.value.len()].fill(b' ');
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Whether a word or digit group of 3 or more characters from `value` stays in
/// the output beyond what the rest of the document explains, outside every
/// candidate. A whole value left behind does this too.
fn leaves_a_fragment(document: &Document, value: &Value, result: &redact::Redaction) -> bool {
    let output = without_candidates(&result.text, &result.unknowns);
    let elsewhere = without_candidates(&document.text, &result.unknowns).replace(&value.text, "");
    value
        .text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|fragment| fragment.chars().count() >= 3)
        .any(|fragment| standing(&output, fragment) > standing(&elsewhere, fragment))
}

fn measure(document: &Document, mapped: bool, counts: &mut Counts) -> Vec<String> {
    let result = redact::redact(&document.text, &document.entries).unwrap();
    let flagged = |value: &str| result.unknowns.iter().any(|u| u.value.contains(value));
    let planted: Vec<_> = document.values.iter().filter(|v| v.class == Class::Planted).collect();
    let misses: Vec<String> = planted
        .iter()
        .filter(|v| result.text.contains(&v.text) && !flagged(&v.text))
        .map(|v| v.text.clone())
        .collect();
    let blind_spots: Vec<_> = document.values.iter().filter(|v| v.class == Class::BlindSpot).collect();
    let blind = blind_spots
        .iter()
        .filter(|v| result.text.contains(&v.text) && !flagged(&v.text))
        .count();
    let false_candidates = result
        .unknowns
        .iter()
        .filter(|u| {
            !planted
                .iter()
                .any(|v| u.value.contains(&v.text) || v.text.contains(&u.value))
        })
        .count();
    let halted = !result.unknowns.is_empty();
    counts.0[0] += 1;
    counts.0[1] += planted.len();
    counts.0[2] += misses.len();
    counts.0[3] += blind;
    counts.0[4] += blind_spots.len();
    counts.0[5] += usize::from(mapped && false_candidates > 0);
    counts.0[6] += false_candidates;
    counts.0[7] += usize::from(halted);
    counts.0[8] += planted
        .iter()
        .filter(|v| leaves_a_fragment(document, v, &result))
        .count();
    misses
}

/// Each template by file name, in name order.
fn templates() -> Vec<(String, String)> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluation/workpapers");
    let mut found: Vec<_> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    found.sort();
    found
        .iter()
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(path).unwrap().replace("\r\n", "\n");
            (name, text)
        })
        .collect()
}

/// The table, and every planted value that passed unnoticed.
fn results() -> (Vec<String>, Vec<String>) {
    let mut table: Vec<String> = HEADER.iter().map(|line| (*line).to_owned()).collect();
    let mut misses = Vec::new();
    let mut totals = Counts::default();
    for (index, (name, template)) in templates().iter().enumerate() {
        let mut counts = Counts::default();
        for document in 0..DOCUMENTS {
            let seed = SEED + (index * DOCUMENTS + document) as u64;
            let mapped = document % 2 == 0;
            let rendered = render(template, seed, mapped);
            let missed = measure(&rendered, mapped, &mut counts);
            misses.extend(missed.into_iter().map(|value| format!("{name} {document}: {value}")));
        }
        table.push(counts.row(&format!("`{name}`")));
        totals.add(&counts);
    }
    table.push(totals.row("Total"));
    (table, misses)
}

fn documented_results() -> Vec<&'static str> {
    README
        .lines()
        .skip_while(|line| *line != "## Detection benchmark")
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .filter(|line| line.starts_with('|'))
        .collect()
}

#[test]
fn readme_benchmark_table_matches_the_corpus() {
    let (table, misses) = results();
    eprintln!("{}\n{misses:#?}", table.join("\n"));
    assert_eq!(
        documented_results(),
        table,
        "README Detection benchmark table differs from tests/benchmark.rs"
    );
}

#[test]
fn the_corpus_holds_every_kind_and_only_slots_it_knows() {
    let kinds: BTreeSet<_> = templates()
        .iter()
        .flat_map(|(_, template)| {
            template
                .split("{{")
                .skip(1)
                .map(|part| part.split("}}").next().unwrap().to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(kinds.len(), 23, "{kinds:?}");
    let mut rng = Rng(SEED);
    for kind in kinds {
        fill(&kind, &mut rng);
    }
}

/// The false candidates are what the README says to map, so mapping them must
/// clear a document that holds nothing else unmapped. Digit runs inside a digest
/// are the exception the README names: the map cannot reach inside a hex word.
#[test]
fn mapping_the_false_candidates_clears_a_fully_mapped_document() {
    for (name, template) in templates().iter().filter(|(name, _)| name != "split-identifiers") {
        let mut document = render(template, SEED, true);
        let unknowns = redact::redact(&document.text, &document.entries).unwrap().unknowns;
        assert!(!unknowns.is_empty() || name == "columnar-balances", "{name}");
        for unknown in &unknowns {
            let ordinal = document.entries.len() + 1;
            document.entries.push(entry(&unknown.value, "entity", ordinal));
        }
        let left = redact::redact(&document.text, &document.entries).unwrap().unknowns;
        assert!(
            left.iter()
                .all(|u| name == "close-review" && u.value.chars().all(|c| c.is_ascii_digit())),
            "{name}: {:?}",
            left.iter().map(|u| u.value.to_string()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn a_seed_always_gives_the_same_document() {
    let (_, template) = &templates()[0];
    let (first, second) = (render(template, SEED, true), render(template, SEED, true));
    assert_eq!(first.text, second.text);
}

#[test]
fn a_partial_redaction_is_counted_as_a_residual_fragment() {
    // A map entry for the given name alone leaves the family name in the output.
    let document = Document {
        text: "the client is Hana falador today".into(),
        values: vec![Value {
            class: Class::Planted,
            text: "Hana falador".into(),
        }],
        entries: vec![entry("Hana", "person", 1)],
    };
    let mut counts = Counts::default();
    assert!(measure(&document, true, &mut counts).is_empty());
    assert_eq!((counts.0[2], counts.0[8]), (0, 1));
}

#[test]
fn a_candidate_elsewhere_does_not_excuse_a_fragment() {
    // `Falador` is left from the planted name, and the candidate on line 3 holds it.
    let document = Document {
        text: "the client is Hana Falador today\n\nwrote to Alice Falador\n".into(),
        values: vec![Value {
            class: Class::Planted,
            text: "Hana Falador".into(),
        }],
        entries: vec![entry("Hana", "person", 1)],
    };
    let result = redact::redact(&document.text, &document.entries).unwrap();
    assert!(
        result
            .unknowns
            .iter()
            .any(|u| u.line == 3 && u.value.contains("Falador"))
    );
    let mut counts = Counts::default();
    assert!(measure(&document, true, &mut counts).is_empty());
    assert_eq!((counts.0[2], counts.0[8]), (0, 1));
}

#[test]
fn a_planted_value_that_survives_is_counted_as_a_miss() {
    // A lower-case name passes the sweep, so the harness must report it.
    let document = Document {
        text: "the client is hana falador".into(),
        values: vec![Value {
            class: Class::Planted,
            text: "hana falador".into(),
        }],
        entries: Vec::new(),
    };
    let mut counts = Counts::default();
    assert_eq!(measure(&document, true, &mut counts), ["hana falador"]);
    assert_eq!(counts.0[2], 1);
}
