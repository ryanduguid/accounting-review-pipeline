//! The external Payday Super producer CSV contract and the source pins on
//! `PaydaySuper.Report.pq`.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use common::{Dec, Source, records, root};

const FIXTURE: &str = "samples/sample-payday-super-report.csv";

const HEADERS: [&str; 18] = [
    "row",
    "employee_id",
    "qe_day",
    "pathway",
    "due_date",
    "verdict",
    "days_late",
    "lateness_measured_to",
    "sg_amount",
    "final_shortfall",
    "notional_earnings",
    "uplift_best_case",
    "uplift_worst_case",
    "sgc_estimate_low",
    "sgc_estimate_high",
    "caveats",
    "notes",
    "unassessable_between",
];

const AMOUNT_HEADERS: [&str; 7] = [
    "sg_amount",
    "final_shortfall",
    "notional_earnings",
    "uplift_best_case",
    "uplift_worst_case",
    "sgc_estimate_low",
    "sgc_estimate_high",
];

/// One contribution row: amounts typed or null, every other field kept as text.
struct Contribution {
    text: BTreeMap<String, String>,
    amounts: BTreeMap<&'static str, Option<Dec>>,
}

impl Contribution {
    /// A field kept as text, so a leading zero or formula-like text stays data.
    fn text(&self, header: &str) -> &str {
        self.text
            .get(header)
            .unwrap_or_else(|| panic!("{header} is not a text field"))
    }

    fn amount(&self, header: &str) -> Option<Dec> {
        *self
            .amounts
            .get(header)
            .unwrap_or_else(|| panic!("{header} is not an amount field"))
    }
}

fn typed(mut text: BTreeMap<String, String>, index: usize) -> Result<Contribution, String> {
    let mut amounts = BTreeMap::new();
    for header in AMOUNT_HEADERS {
        let raw = text
            .remove(header)
            .expect("the header check found every amount");
        let amount = if raw.is_empty() {
            None
        } else {
            Some(Dec::parse(&raw).map_err(|_| format!("invalid amount {header} on row {index}"))?)
        };
        amounts.insert(header, amount);
    }
    Ok(Contribution { text, amounts })
}

/// Named headers: extras are tolerated, but none may repeat or be missing.
fn check_headers(headers: &[String]) -> Result<(), String> {
    if headers.iter().collect::<BTreeSet<_>>().len() != headers.len() {
        return Err("duplicate header".into());
    }
    let missing: Vec<_> = HEADERS
        .into_iter()
        .filter(|header| !headers.iter().any(|present| present == header))
        .collect();
    if !missing.is_empty() {
        return Err(format!("missing header: {}", missing.join(", ")));
    }
    Ok(())
}

/// Small specification port for the external producer CSV contract.
///
/// CI has no M engine. This tests the prescribed input boundary, whilst the source pins
/// below pin the checks the Power Query adapter must retain. It deliberately does not
/// calculate an exposure or alter a producer verdict. Amounts are read as plain decimal
/// text.
fn parse_report(content: &str) -> Result<(Vec<Contribution>, String), String> {
    let rows = accounting_excel_toolkit::aged::records(content.as_bytes())
        .map_err(|_| "unterminated quoted field or malformed CSV".to_owned())?;
    let Some((headers, body)) = rows.split_first() else {
        return Err("no header row".into());
    };
    check_headers(headers)?;
    let mut contributions = Vec::new();
    let mut provenance = None;
    for (offset, values) in body.iter().enumerate() {
        let index = offset + 2;
        if values.len() != headers.len() {
            return Err(format!("malformed row {index}"));
        }
        let record: BTreeMap<_, _> = headers
            .iter()
            .cloned()
            .zip(values.iter().cloned())
            .collect();
        let is_terminal_note = record["employee_id"] == "NOTE"
            && HEADERS
                .iter()
                .filter(|header| !matches!(**header, "employee_id" | "notes"))
                .all(|header| record[*header].is_empty());
        if is_terminal_note {
            if index != rows.len() || provenance.is_some() {
                return Err("NOTE row must be terminal and unique".into());
            }
            provenance = Some(record["notes"].clone());
            continue;
        }
        if record["employee_id"].is_empty() || record["row"].is_empty() {
            return Err(format!("malformed row {index}"));
        }
        contributions.push(typed(record, index)?);
    }
    let provenance = provenance
        .filter(|text| !text.is_empty())
        .ok_or("no terminal NOTE provenance")?;
    if contributions.is_empty() {
        return Err("no contribution rows".into());
    }
    Ok((contributions, provenance))
}

/// `csv.writer` output in the excel dialect: minimal quoting and CRLF line ends.
fn csv_text(rows: &[Vec<String>]) -> String {
    rows.iter()
        .map(|row| {
            let fields: Vec<_> = row
                .iter()
                .map(|value| {
                    if value.contains([',', '"', '\r', '\n'])
                        || (row.len() == 1 && value.is_empty())
                    {
                        format!("\"{}\"", value.replace('"', "\"\""))
                    } else {
                        value.clone()
                    }
                })
                .collect();
            fields.join(",") + "\r\n"
        })
        .collect()
}

/// The fixture as `open(newline="", encoding="utf-8-sig")` reads it.
fn fixture_text() -> String {
    String::from_utf8(fs::read(root().join(FIXTURE)).unwrap()).unwrap()
}

fn fixture_rows() -> Vec<Vec<String>> {
    records(FIXTURE, &fixture_text())
}

fn accepted(content: &str) -> (Vec<Contribution>, String) {
    parse_report(content).unwrap_or_else(|error| panic!("refused a valid report: {error}"))
}

fn refused(content: &str, expected: &str) {
    let Err(error) = parse_report(content) else {
        panic!("accepted a report that must be refused with {expected:?}");
    };
    assert!(
        error.contains(expected),
        "{error:?} does not contain {expected:?}"
    );
}

fn column(rows: &[Vec<String>], header: &str) -> usize {
    rows[0].iter().position(|name| name == header).unwrap()
}

fn appended(row: &[String], value: &str) -> Vec<String> {
    let mut row = row.to_vec();
    row.push(value.to_owned());
    row
}

#[test]
fn fabricated_producer_fixture_preserves_the_18_field_contract() {
    let (records, provenance) = accepted(&fixture_text());

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].text("employee_id"), "000123");
    assert_eq!(records[1].text("employee_id"), "'=FORMULA");
    assert_eq!(records[0].text("verdict"), "LATE");
    assert_eq!(records[1].text("verdict"), "UNKNOWN");
    assert_eq!(
        records[0].text("notes"),
        "Fabricated, \"late\" contribution"
    );
    assert_eq!(records[1].text("unassessable_between"), "LATE or ON_TIME");
    assert!(records[1].text("caveats").contains("calendar coverage"));
    assert!(provenance.contains("payday-super-checker"), "{provenance}");
    assert!(
        records
            .iter()
            .all(|record| record.text("employee_id") != "NOTE")
    );
}

#[test]
fn literal_note_employee_identifier_remains_a_contribution() {
    let mut changed = fixture_rows();
    let employee_id = column(&changed, "employee_id");
    changed[1][employee_id] = "NOTE".into();

    let (records, provenance) = accepted(&csv_text(&changed));

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].text("employee_id"), "NOTE");
    assert!(provenance.contains("payday-super-checker"), "{provenance}");
}

#[test]
fn amounts_are_typed_or_null_without_recalculation() {
    let (records, _) = accepted(&fixture_text());
    let [first, second] = records.as_slice() else {
        panic!("expected 2 contributions, found {}", records.len());
    };

    assert_eq!(first.amount("sg_amount"), Some(Dec::of("780.00")));
    assert_eq!(first.amount("final_shortfall"), Some(Dec::of("780.00")));
    assert_eq!(first.amount("sgc_estimate_low"), Some(Dec::of("833.34")));
    assert_eq!(second.amount("final_shortfall"), None);
    assert_eq!(second.amount("uplift_best_case"), None);
    assert_eq!(second.amount("sgc_estimate_high"), None);
}

#[test]
fn named_headers_tolerate_extras_but_reject_missing_or_duplicate_names() {
    let source_rows = fixture_rows();
    let verdict = column(&source_rows, "verdict");
    // One more column: this header, and a value from each record below it.
    let extended = |header: &str, value: &dyn Fn(&[String]) -> String| -> Vec<Vec<String>> {
        std::iter::once(appended(&source_rows[0], header))
            .chain(
                source_rows[1..]
                    .iter()
                    .map(|row| appended(row, &value(row))),
            )
            .collect()
    };

    let extra = extended("future_column", &|_| "ignored".into());
    assert_eq!(accepted(&csv_text(&extra)).0.len(), 2);

    let missing: Vec<Vec<String>> = source_rows
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .filter(|(index, _)| *index != verdict)
                .map(|(_, value)| value.clone())
                .collect()
        })
        .collect();
    refused(&csv_text(&missing), "missing header: verdict");

    let duplicate = extended("verdict", &|row| row[verdict].clone());
    refused(&csv_text(&duplicate), "duplicate header");
}

#[test]
fn malformed_rows_bad_amounts_and_missing_provenance_refuse() {
    let source_rows = fixture_rows();
    let (note, without_note) = source_rows.split_last().unwrap();

    let mut malformed = without_note.to_vec();
    malformed.push(note[..note.len() - 1].to_vec());
    refused(&csv_text(&malformed), "malformed row");

    let mut bad_amount = source_rows.clone();
    bad_amount[1][column(&source_rows, "sg_amount")] = "TBC".into();
    refused(&csv_text(&bad_amount), "invalid amount sg_amount");

    refused(&csv_text(without_note), "no terminal NOTE provenance");
}

#[test]
fn zero_contributions_and_unterminated_quotes_refuse() {
    let source_rows = fixture_rows();
    let (header, note) = (&source_rows[0], source_rows.last().unwrap());

    refused(
        &csv_text(&[header.clone(), note.clone()]),
        "no contribution rows",
    );

    let unterminated_contract = format!("{}\n1,\"unterminated", header.join(","));
    refused(&unterminated_contract, "unterminated quoted field");

    let with_extra: Vec<_> = std::iter::once(appended(header, "future_column"))
        .chain(
            source_rows[1..source_rows.len() - 1]
                .iter()
                .map(|row| appended(row, "ignored")),
        )
        .collect();
    let unterminated_extra = format!("{}{},\"unterminated", csv_text(&with_extra), note.join(","));
    refused(&unterminated_extra, "unterminated quoted field");
}

#[test]
fn quoted_multiline_field_remains_one_record() {
    let mut changed = fixture_rows();
    let notes = column(&changed, "notes");
    changed[1][notes] = "Fabricated multiline\ncontribution".into();

    let (records, provenance) = accepted(&csv_text(&changed));

    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0].text("notes"),
        "Fabricated multiline\ncontribution"
    );
    assert!(provenance.contains("payday-super-checker"), "{provenance}");
}

/// The patterns `PaydaySuper.Report.pq` must keep: named refusals, metadata provenance,
/// buffered reads and the record-width and quote checks over framed records.
const POWER_QUERY_PINS: &[&str] = &[
    r#"Error\.Record\(\s*"PaydaySuper\.Report",\s*"Missing required header""#,
    r#"Error\.Record\(\s*"PaydaySuper\.Report",\s*"Duplicate header""#,
    r#"Error\.Record\(\s*"PaydaySuper\.Report",\s*"Malformed report row""#,
    r#"Error\.Record\(\s*"PaydaySuper\.Report",\s*"Invalid amount""#,
    r#"Error\.Record\(\s*"PaydaySuper\.Report",\s*"Missing terminal NOTE provenance""#,
    r"Value\.ReplaceMetadata\(",
    r"PaydaySuperProvenance = Provenance",
    r"fileBinary\s*=\s*Binary\.Buffer\(File\.Contents\(FilePath\)\)",
    r"Table\.SelectColumns\(HeadersChecked, RequiredHeaders, MissingField\.Error\)",
    r"Table\.Buffer\(AmountTyped\)",
    r"CharacterCount\s*=\s*Text\.Length\(CsvText\)",
    r"Lines\.FromText\(CsvText, QuoteStyle\.Csv, false\)",
    r"MeasureRecord\s*=\s*\(recordText as text, recordNumber as number\)",
    r"quoteCount\s*=\s*Text\.Length\(recordText\)",
    r"parsedAttempt\s*=\s*if Number\.Mod\(quoteCount, 2\) <> 0 then",
    r"else try Csv\.Document\(\s*Text\.ToBinary\(recordText, TextEncoding\.Utf8\)",
    r"FirstMalformedRecord\s*=\s*Table\.FirstN\(",
    r"RecordWidthsChecked",
    r"Text\.From\(malformed\[RecordNumber\]\)",
    r"IsTerminalNoteCandidate",
    r"DataRowCount\s*=\s*Table\.RowCount\(DataRows\)",
    r#"Error\.Record\(\s*"PaydaySuper\.Report",\s*"Missing contribution rows""#,
    concat!(
        r"ValidatedResult\s*=\s*if\s*Text\.Length\(ProvenanceChecked\) >= 0 ",
        r"and ContributionRowCount > 0\s*then Result",
    ),
];

#[test]
fn power_query_pins_contract_validation_and_metadata_provenance() {
    let source = Source::adapter("powerquery/PaydaySuper.Report.pq");
    source.matches_all(&[
        r"(?m)^\s*PaydaySuperReport = \(FilePath as text\) as table =>",
        r"(?m)^\s*PaydaySuperReport$",
    ]);
    let quoted: Vec<_> = HEADERS
        .iter()
        .map(|header| format!("\"{header}\""))
        .collect();
    source.has_all(&quoted.iter().map(String::as_str).collect::<Vec<_>>());
    source.matches_all(POWER_QUERY_PINS);
    source.lacks_all(&[
        "List.Count(CsvCharacters)",
        "Widths = {}",
        "{\"Widths\", each _ &",
        "ScanCharacterRange",
        "CheckRecordWidth",
        "quoteChecked =",
    ]);
}

#[test]
fn documentation_states_the_contract_and_future_parser_boundary() {
    Source::adapter("README.md").has_all(&[
        "fabricated Payday Super report",
        "`employee_id = NOTE`",
        "`PaydaySuperProvenance`",
        "raw `verdict`",
        "literal employee identifier is `NOTE` remains data",
        "present empty trailing field is valid",
        "report with no contribution rows",
        "ignored extra producer column",
        "quoted multiline field",
    ]);
    Source::adapter("docs/close-input-contract-roadmap.md").has_all(&[
        "observed Xero aged receivables/payables",
        "MYOB-specific parsers",
        "Do not infer a schema",
    ]);
}
