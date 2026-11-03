//! The Xero aged receivables and payables adapters: source pins on each `.pq` file and
//! the fabricated samples they load.
//!
//! The 2 parsers are separate copies, so each file is pinned by its own tests: fixing one
//! parser and leaving the other silently mistyping has to fail.

mod common;

use common::{Dec, Source, read, record_widths, root};

const RECEIVABLES: &str = "powerquery/Xero.AgedReceivables.pq";
const PAYABLES: &str = "powerquery/Xero.AgedPayables.pq";
const RECEIVABLES_SAMPLE: &str = "samples/sample-xero-aged-receivables.csv";
const PAYABLES_SAMPLE: &str = "samples/sample-xero-aged-payables.csv";

/// The row filter `Xero.AgedReceivables.pq` and `Xero.AgedPayables.pq` share, ported:
/// returns the contact rows plus the summary Total row.
///
/// A section row is any row whose amount cells are all blank; only "Total " plus that
/// name is dropped as its subtotal, so a contact called "Total Tools Trade Card"
/// survives. The "Percentage of total" row and blank rows go too.
fn aged_body_rows(data_rows: &[Vec<String>]) -> Vec<Vec<String>> {
    let blank_amounts = |row: &[String]| row.iter().skip(1).all(|cell| cell.trim().is_empty());
    let subtotals: Vec<String> = data_rows
        .iter()
        .filter(|row| !row[0].trim().is_empty() && blank_amounts(row))
        .map(|row| format!("Total {}", row[0].trim()))
        .collect();
    data_rows
        .iter()
        .filter(|row| {
            let name = row[0].trim();
            !(name.is_empty()
                || blank_amounts(row)
                || name.to_lowercase() == "percentage of total"
                || subtotals.iter().any(|subtotal| subtotal == name))
        })
        .cloned()
        .collect()
}

/// The sample as `read_text(encoding="utf-8-sig")` reads it.
fn sample(relative: &str) -> String {
    let text = read(&root().join(relative));
    text.strip_prefix('\u{feff}')
        .map(str::to_owned)
        .unwrap_or(text)
}

fn is_total(row: &[String]) -> bool {
    row[0].trim().to_lowercase() == "total"
}

/// The parser's header-detection markers, plus the amount parser they feed.
///
/// Every non-contact column used to be forced through `try Value.FromText(...) otherwise
/// 0.0` with blanks coalesced to "0". `Value.FromText` never raises on text, so a bucket
/// the culture could not read (a bracketed negative like "(150.00)", "TBC", "#REF!") came
/// back as text and loaded into a column ascribed `type number`, silently mistyped; the
/// failure surfaces, if ever, only when something downstream sums the column. Pinned as
/// the shape `Xero.TrialBalance`'s parseAmount already proves: blank is a genuine zero,
/// anything else must parse as a number or raises a named `Error.Record`, and the record
/// has to carry the column and the offending cell, or the accountant is told only that
/// some cell somewhere in the export will not parse.
fn assert_amount_and_row_guards(relative: &str, reason: &str, label: &str, missing_label: &str) {
    let source = Source::adapter(relative);
    source.has_all(&[
        reason,
        "Lines.FromBinary",
        "Csv.Document",
        "Header row not found",
        "Table.PromoteHeaders",
    ]);
    source.matches(&format!(
        concat!(
            r"(?m)parseAmount = \(columnName as text\) =>\s*",
            r"\(raw\) =>\s*",
            r#"let t = Text\.Trim\(Text\.From\(raw \?\? ""\)\) in\s*"#,
            r#"if t = "" then 0\s*"#,
            r"else\s*",
            r#"try Currency\.From\(t, "en-AU"\)\s*"#,
            r"otherwise error Error\.Record\(\s*",
            r#""{}",\s*"#,
            r#""Amount is not a number","#,
        ),
        regex::escape(reason)
    ));
    source.has_all(&[
        "\"Column \"\"\" & columnName & \"\"\" cell \"\"\" & t & \"\"\" won't parse as a number. Fix the export before loading.\"",
        // Binding the parser is not using it, and the column name only reaches the
        // message through a named parameter: the nested each this replaced shadowed it
        // with the cell value.
        "(columnName as text) => {columnName, parseAmount(columnName), Currency.Type}",
    ]);
    // The silent-zero form cannot come back beside the parser.
    source.lacks_all(&["otherwise 0.0", "Value.FromText"]);
    // Observed 5 September 2026: section, subtotal and Percentage of total rows are
    // dropped by these exact rules, mirrored by aged_body_rows above.
    let unlabelled = format!("each {label}(_) = \"\" and not isSectionRow(_)");
    source.has_all(&[
        "not isSectionRow(_)",
        "not List.Contains(footerRows, [__agedRow])",
        unlabelled.as_str(),
        "Labelled = Table.SelectRows(Validated,",
        missing_label,
        "not List.Contains(sectionSubtotals, val)",
        "each \"Total \" & Text.Trim(Text.From(_))",
    ]);
}

/// A short record used to load with its absent Total padded to a genuine zero, and a
/// ninth field was dropped in silence, because `Csv.Document` takes every record's width
/// from Columns alone. The contact's Current amount stayed $1,250, so nothing downstream
/// had a reason to look. Pinned as: the header's field count is measured from the framed
/// record, every record below it has to match, and a present empty cell stays valid.
fn assert_record_width_refusal(relative: &str, row: &str, people: usize, total: &str) {
    let fixture = sample(relative);
    // Control: the fabricated export is unchanged and still reconciles.
    let (width, body) = record_widths(&fixture, &["Contact"]).unwrap();
    assert_eq!(width, 8, "{relative}");
    let named: Vec<_> = aged_body_rows(&body)
        .into_iter()
        .filter(|row| !is_total(row))
        .collect();
    assert_eq!(named.len(), people, "{relative}");
    let summed: Dec = named.iter().map(|row| Dec::of(row.last().unwrap())).sum();
    assert_eq!(summed, Dec::of(total), "{relative}");

    assert!(
        fixture.contains(row),
        "{relative} no longer carries {row:?}"
    );
    let last_comma = row.rfind(',').unwrap();
    // A present blank amount is a genuine zero, not a missing field.
    let blank_total = fixture.replace(row, &row[..=last_comma]);
    assert_eq!(
        record_widths(&blank_total, &["Contact"]).unwrap().0,
        8,
        "{relative}"
    );
    // The audited cases: the final field removed, and a ninth field added.
    for malformed in [row[..last_comma].to_owned(), format!("{row},extra")] {
        assert!(
            record_widths(&fixture.replace(row, &malformed), &["Contact"]).is_err(),
            "{relative} accepted {malformed:?}"
        );
    }
}

fn assert_record_width_and_bucket_guards(relative: &str) {
    let source = Source::adapter(relative);
    // The width check runs on the framed records, so a quoted newline stays inside one
    // record rather than becoming a short one.
    source.has_all(&[
        "Binary.Buffer(File.Contents(FilePath))",
        "Lines.FromText(csvText, QuoteStyle.Csv, false)",
        "recordWidth = (recordText as text, recordNumber as number) as number =>",
        "\"Record width does not match the header\"",
        "ExtraValues = ExtraValues.Error",
    ]);
    // Csv.Document is shaped to the checked header width, so the refusal cannot be
    // stepped over.
    source.before("firstBadRecord =", "Raw = Csv.Document");
    source.has_all(&[
        "Columns = columnCount,",
        // A missing Total header is still refused before any row is typed.
        "requiredBuckets = {\"< 1 Month\", \"1 Month\", \"2 Months\", \"3 Months\", \"Older\", \"Total\"}",
        "List.ContainsAll(Table.ColumnNames(PromotedRaw), requiredBuckets)",
    ]);
}

/// The sample's header and the rows below it, each nonblank line trimmed and split
/// naively on commas. The header is the first row whose first cell is one of the names
/// given.
fn reconciled_sample(relative: &str, header_names: &[&str]) -> (Vec<String>, Vec<Vec<String>>) {
    let rows: Vec<Vec<String>> = sample(relative)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.split(',').map(str::to_owned).collect())
        .collect();
    let header_index = rows
        .iter()
        .position(|row| header_names.contains(&row[0].as_str()))
        .unwrap_or_else(|| panic!("Header row not found in sample fixture {relative}"));
    let headers = rows[header_index].clone();
    assert_eq!(headers[0], "Contact", "{relative}");
    assert_eq!(headers.last().unwrap(), "Total", "{relative}");
    (headers, rows[header_index + 1..].to_vec())
}

fn assert_reconciled(relative: &str, headers: &[String], data_rows: &[Vec<String>]) {
    let body = aged_body_rows(data_rows);
    let total_row = body
        .iter()
        .rev()
        .find(|row| row[0].to_lowercase() == "total")
        .unwrap_or_else(|| panic!("Summary Total row required in sample fixture {relative}"));
    let named: Vec<_> = body
        .iter()
        .filter(|row| row[0].to_lowercase() != "total")
        .collect();
    assert_eq!(named.len(), 4, "{relative}");

    // Check row-level bucket summation.
    for row in &named {
        let buckets: Dec = row[1..row.len() - 1]
            .iter()
            .map(|value| Dec::of(value))
            .sum();
        assert_eq!(
            buckets,
            Dec::of(row.last().unwrap()),
            "Row {} buckets do not equal row total",
            row[0]
        );
    }
    // Check column totals match the summary Total row.
    for (index, header) in headers.iter().enumerate().skip(1) {
        let summed: Dec = named.iter().map(|row| Dec::of(&row[index])).sum();
        assert_eq!(
            summed,
            Dec::of(&total_row[index]),
            "Column {header} sum does not match Total row"
        );
    }
}

#[test]
fn aged_receivables_pq_source_guards() {
    assert_amount_and_row_guards(
        RECEIVABLES,
        "Xero.AgedReceivables",
        "contactOf",
        "\"Missing contact label\"",
    );
}

#[test]
fn aged_receivables_refuses_a_record_the_header_width_does_not_match() {
    assert_record_width_refusal(
        RECEIVABLES_SAMPLE,
        "Apex Building Supplies,1250.00,0.00,0.00,0.00,0.00,0.00,1250.00",
        4,
        "3920.50",
    );
}

#[test]
fn aged_receivables_pins_the_record_width_and_bucket_guards() {
    assert_record_width_and_bucket_guards(RECEIVABLES);
}

#[test]
fn aged_receivables_sample_fixture_is_reconciled() {
    assert!(root().join(RECEIVABLES_SAMPLE).is_file());
    let (headers, data_rows) = reconciled_sample(RECEIVABLES_SAMPLE, &["Contact", "Customer"]);
    // Observed 5 September 2026: the export closes with a "Percentage of total" row. The
    // fixture must keep carrying it, or the filter that drops it stops being exercised.
    assert!(
        data_rows.iter().any(|row| row[0] == "Percentage of total"),
        "{RECEIVABLES_SAMPLE} lost its Percentage of total row"
    );
    assert_reconciled(RECEIVABLES_SAMPLE, &headers, &data_rows);
}

/// The payables half of the same pin; see `assert_amount_and_row_guards` for what the old
/// `try Value.FromText(...) otherwise 0.0` shape let through.
#[test]
fn aged_payables_pq_source_guards() {
    assert_amount_and_row_guards(
        PAYABLES,
        "Xero.AgedPayables",
        "supplierOf",
        "\"Missing supplier label\"",
    );
}

/// The section row and its subtotal are full-width in the observed payables export, so
/// the width rule does not have to make an exception for them.
#[test]
fn aged_payables_refuses_a_record_the_header_width_does_not_match() {
    assert_record_width_refusal(
        PAYABLES_SAMPLE,
        "Alpha Hardware Supplies,2340.00,0.00,0.00,0.00,0.00,0.00,2340.00",
        4,
        "5030.00",
    );
}

#[test]
fn aged_payables_pins_the_record_width_and_bucket_guards() {
    assert_record_width_and_bucket_guards(PAYABLES);
}

#[test]
fn aged_payables_sample_fixture_is_reconciled() {
    assert!(root().join(PAYABLES_SAMPLE).is_file());
    let (headers, data_rows) =
        reconciled_sample(PAYABLES_SAMPLE, &["Contact", "Supplier", "Vendor"]);
    // Observed 5 September 2026: the payables export groups suppliers under an "Aged
    // Payables" section row with blank amounts, closes it with "Total Aged Payables", and
    // ends with "Percentage of total". The fixture must keep carrying all 3, or the
    // filters that drop them stop being exercised.
    for marker in [
        "Aged Payables",
        "Total Aged Payables",
        "Percentage of total",
    ] {
        assert!(
            data_rows.iter().any(|row| row[0] == marker),
            "{PAYABLES_SAMPLE} lost its {marker:?} row"
        );
    }
    assert_reconciled(PAYABLES_SAMPLE, &headers, &data_rows);
}
