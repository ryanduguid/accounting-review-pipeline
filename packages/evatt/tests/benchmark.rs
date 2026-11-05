// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Seeded workpaper benchmark, on the model of Monthly Close Controls' detection
//! benchmark. Each template in `evaluation/workpapers` is filled in several times
//! from one seed, so the identifiers exist only while the test runs. The counts
//! measure these fabricated documents only, never real-world accuracy, and the
//! README's Detection benchmark table must match them.
use evatt::{entities::Entity, patterns, pyjson::PyStr, redact};
use std::collections::BTreeSet;
use std::path::Path;

const README: &str = include_str!("../README.md");
const SEED: u64 = 20_261_106;
/// Documents per template: half with every name in the map, half without.
const DOCUMENTS: usize = 20;
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
const MONTHS: [&str; 4] = ["March", "June", "October", "December"];
const HEADER: [&str; 2] = [
    "| Workpaper | Documents | Planted values | Misses | Blind-spot values missed | False halts | False candidates | Halt rate |",
    "| --- | --- | --- | --- | --- | --- | --- | --- |",
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

fn grouped(digits: &str, widths: &[usize]) -> String {
    let mut parts = Vec::new();
    let mut start = 0;
    for width in widths {
        parts.push(&digits[start..start + width]);
        start += width;
    }
    parts.join(" ")
}

fn person(rng: &mut Rng) -> String {
    format!("{} {}", rng.pick(&GIVEN), rng.pick(&FAMILY))
}

fn amount(rng: &mut Rng) -> String {
    let whole = rng.below(900_000) + 1_000;
    format!("${},{:03}.{:02}", whole / 1_000, whole % 1_000, rng.below(100))
}

/// A slot's text and class. An `entity` kind is what the map can hold.
fn fill(kind: &str, rng: &mut Rng) -> (String, Class, Option<&'static str>) {
    match kind {
        "tfn" => (grouped(&rng.checked("tfn", 9), &[3, 3, 3]), Class::Planted, None),
        "abn" => (grouped(&rng.checked("abn", 11), &[2, 3, 3, 3]), Class::Planted, None),
        "acn" => (grouped(&rng.checked("acn", 9), &[3, 3, 3]), Class::Planted, None),
        "medicare" => (grouped(&rng.checked("medicare", 10), &[4, 5, 1]), Class::Planted, None),
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
struct Counts([usize; 8]);

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
        ] = self.0;
        let rate = (halts * 1000 + documents / 2) / documents;
        format!(
            "| {label} | {documents} | {planted} | {misses} | {blind} of {blind_planted} | {false_halts} | {candidates} | {}.{}% |",
            rate / 10,
            rate % 10
        )
    }
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
    counts.0[5] += usize::from(mapped && halted);
    counts.0[6] += false_candidates;
    counts.0[7] += usize::from(halted);
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
    assert_eq!(kinds.len(), 18, "{kinds:?}");
    let mut rng = Rng(SEED);
    for kind in kinds {
        fill(&kind, &mut rng);
    }
}

#[test]
fn a_seed_always_gives_the_same_document() {
    let (_, template) = &templates()[0];
    let (first, second) = (render(template, SEED, true), render(template, SEED, true));
    assert_eq!(first.text, second.text);
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
