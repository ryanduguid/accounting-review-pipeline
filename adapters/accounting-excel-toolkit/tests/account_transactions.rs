use accounting_excel_toolkit::transactions::{
    amount, convert, inspect_export, output_csv, parse_date,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const SOURCE: &[u8] = include_bytes!("../samples/xero-account-transactions.csv");
const TENANT: &str = "Fabricated Services";
const HEADER: &str =
    "Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit\n";
static SERIAL: AtomicUsize = AtomicUsize::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap();
        for _ in 0..100 {
            let path = root.join(format!(
                "excel-transactions-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("{error}"),
            }
        }
        panic!("could not allocate test directory");
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let root = std::env::temp_dir().canonicalize().unwrap();
        assert_eq!(self.0.parent(), Some(root.as_path()));
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn source_text() -> String {
    String::from_utf8(SOURCE.to_vec()).unwrap()
}

fn changed(old: &str, new: &str) -> Vec<u8> {
    let source = source_text();
    assert!(source.contains(old), "missing fixture text: {old}");
    source.replace(old, new).into_bytes()
}

fn inspect(source: &[u8]) -> Value {
    inspect_export(source, TENANT, "Clearing").unwrap()
}

fn mapping(source: &[u8]) -> Value {
    json!({
        "source_sha256": Sha256::digest(source).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "lines": {"9": "source-line-1", "10": "source-line-2"}
    })
}

fn converted(source: &[u8], map: &Value) -> Result<Vec<Value>, String> {
    convert(source, map, TENANT, "Clearing", "090", "AUD")
}

fn cli(source: &Path, map: Option<&Path>, extra: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xero-account-transactions"));
    command
        .arg(source)
        .args(["--tenant", TENANT, "--account-name", "Clearing"]);
    if let Some(map) = map {
        command.arg("--mapping").arg(map);
    }
    command.args(extra).output().unwrap()
}

fn refused(output: &Output, status: i32, message: &str) {
    assert_eq!(output.status.code(), Some(status));
    assert!(output.stdout.is_empty(), "unexpected partial output");
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains(message), "{error}");
    assert!(!error.contains("Traceback"), "{error}");
}

fn terminal(text: &str) -> Vec<u8> {
    if cfg!(windows) {
        text.replace('\n', "\r\n").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

#[test]
fn inspection_retains_identity_totals_and_original_detail() {
    let report = inspect(SOURCE);
    assert_eq!(report["source_sha256"], mapping(SOURCE)["source_sha256"]);
    assert_eq!(report["tenant"], TENANT);
    assert_eq!(report["account_name"], "Clearing");
    assert_eq!(report["period_start"], "2026-09-01");
    assert_eq!(report["period_end"], "2026-09-30");
    assert_eq!(report["debit"], "110.00");
    assert_eq!(report["credit"], "100.00");
    assert_eq!(
        report["detail"],
        json!([
            {"source_row": 9, "Date": "2026-09-01", "Source": "Receive Money",
             "Reference": "REC-1", "Description": "Receipt", "Debit": "110.00", "Credit": "0"},
            {"source_row": 10, "Date": "2026-09-03", "Source": "Spend Money",
             "Reference": "SET-1", "Description": "Settlement", "Debit": "0", "Credit": "100.00"}
        ])
    );
}

#[test]
fn conversion_preserves_account_code_and_uses_only_supplied_line_identity() {
    let rows = converted(SOURCE, &mapping(SOURCE)).unwrap();
    assert_eq!(
        rows,
        vec![
            json!({"Tenant": TENANT, "AccountID": "090", "Currency": "AUD",
                   "TransactionID": "source-line-1", "Date": "2026-09-01", "Reference": "REC-1",
                   "Description": "Receipt", "Debit": "110.00", "Credit": "0"}),
            json!({"Tenant": TENANT, "AccountID": "090", "Currency": "AUD",
                   "TransactionID": "source-line-2", "Date": "2026-09-03", "Reference": "SET-1",
                   "Description": "Settlement", "Debit": "0", "Credit": "100.00"})
        ]
    );
}

#[test]
fn extra_title_records_move_locators_but_embedded_newlines_do_not() {
    let source = [b"Prepared copy,,,,,,,,\n".as_slice(), SOURCE].concat();
    let report = inspect(&source);
    assert_eq!(report["detail"][0]["source_row"], 10);
    assert_eq!(report["detail"][1]["source_row"], 11);
    let source = changed("Receipt", "\"Receipt, \"\"quoted\"\"\ncontinued\"");
    let report = inspect(&source);
    assert_eq!(report["detail"][0]["source_row"], 9);
    assert_eq!(report["detail"][1]["source_row"], 10);
    assert_eq!(
        report["detail"][0]["Description"],
        "Receipt, \"quoted\"\ncontinued"
    );
    assert!(converted(&source, &mapping(&source)).is_ok());
}

#[test]
fn bom_and_line_endings_are_part_of_the_hash_without_changing_detail() {
    let lf = source_text().replace("\r\n", "\n").into_bytes();
    let crlf = String::from_utf8(lf.clone()).unwrap().replace('\n', "\r\n");
    let bom = [b"\xef\xbb\xbf".as_slice(), &lf].concat();
    let expected = inspect(&lf);
    for source in [&bom[..], crlf.as_bytes()] {
        let report = inspect(source);
        assert_eq!(report["detail"], expected["detail"]);
        assert_eq!(report["source_sha256"], mapping(source)["source_sha256"]);
        assert_ne!(report["source_sha256"], expected["source_sha256"]);
        assert!(
            converted(source, &mapping(&lf))
                .unwrap_err()
                .contains("SHA-256")
        );
        assert!(converted(source, &mapping(source)).is_ok());
    }
}

#[test]
fn reordering_equal_total_detail_invalidates_the_original_mapping() {
    let text = source_text().replace("\r\n", "\n");
    let mut lines: Vec<_> = text.lines().collect();
    lines.swap(8, 9);
    let source = (lines.join("\n") + "\n").into_bytes();
    assert_eq!(inspect(&source)["debit"], "110.00");
    assert!(
        converted(&source, &mapping(SOURCE))
            .unwrap_err()
            .contains("SHA-256")
    );
}

#[test]
fn native_display_dates_and_correctly_grouped_amounts_are_supported() {
    let source = source_text()
        .replace("01/09/2026", "1 Sep 2026")
        .replace("03/09/2026", "3 Sep 2026");
    let report = inspect(source.as_bytes());
    assert_eq!(report["detail"][0]["Date"], "2026-09-01");
    assert_eq!(report["detail"][1]["Date"], "2026-09-03");
    assert_eq!(amount("6,661.60").unwrap().to_string(), "6661.60");
    assert!(amount("66,61.60").is_err());
}

#[test]
fn changed_headers_missing_fields_and_extra_fields_are_refused() {
    for (old, new) in [
        ("Running Balance", "Balance"),
        ("110.00,0,110.00", "110.00,110.00"),
        ("110.00,0,110.00", "110.00,0,110.00,extra"),
    ] {
        assert!(inspect_export(&changed(old, new), TENANT, "Clearing").is_err());
    }
}

#[test]
fn subtotal_and_out_of_period_postings_fail_independent_checks() {
    for (old, new, error) in [
        (
            "Total Clearing,,,,110.00",
            "Total Clearing,,,,0",
            "subtotal",
        ),
        ("01/09/2026", "01/10/2026", "outside report period"),
        ("03/09/2026", "31/08/2026", "outside report period"),
    ] {
        let result = inspect_export(&changed(old, new), TENANT, "Clearing");
        assert!(result.unwrap_err().contains(error));
    }
    let source = changed("03/09/2026", "30/09/2026");
    assert_eq!(inspect(&source)["detail"][1]["Date"], "2026-09-30");
}

#[test]
fn title_tenant_period_and_header_must_be_unambiguous() {
    for (old, new) in [
        ("Account Transactions", "Other Report"),
        (TENANT, "Other Entity"),
        ("For the period ", "Period "),
        (
            "1 September 2026 to 30 September 2026",
            "30 September 2026 to 1 September 2026",
        ),
        (
            "1 September 2026 to 30 September 2026",
            "1 Sep 2026 to 30 Sep 2026",
        ),
    ] {
        assert!(inspect_export(&changed(old, new), TENANT, "Clearing").is_err());
    }
    for prefix in [
        "For the period 1 September 2026 to 30 September 2026\n",
        "Date,Source,Description,Reference,Debit,Credit,Running Balance,Gross,GST\n",
    ] {
        let source = [prefix.as_bytes(), SOURCE].concat();
        assert!(inspect_export(&source, TENANT, "Clearing").is_err());
    }
}

#[test]
fn exactly_one_complete_account_section_is_required() {
    let text = source_text();
    for source in [
        text.replace("Clearing", "Other Account"),
        text.lines().take(10).collect::<Vec<_>>().join("\n"),
        format!("{text}Clearing,,,,,,,,\nTotal Clearing,,,,110.00,100.00,0,0,0\n"),
        text.replace("03/09/2026,Spend Money", "Opening Balance,Spend Money"),
    ] {
        assert!(inspect_export(source.as_bytes(), TENANT, "Clearing").is_err());
    }
}

#[test]
fn selected_detail_requires_exactly_one_positive_side() {
    for (old, new) in [
        ("110.00,0,110.00", "0.00,0,110.00"),
        ("110.00,0,110.00", "110.00,1,110.00"),
        ("110.00,0,110.00", "-110.00,0,110.00"),
    ] {
        assert!(inspect_export(&changed(old, new), TENANT, "Clearing").is_err());
    }
}

#[test]
fn ignored_balance_columns_cannot_replace_independent_detail_totals() {
    let source = source_text()
        .replace(
            "Opening Balance,,,,0,0,0,0,0",
            "Opening Balance,,,,ignored,ignored,=SUM(A1),ignored,ignored",
        )
        .replace(
            "110.00,0,110.00,110.00,10.00",
            "110.00,0,=SUM(A1),not-a-number,",
        )
        .replace(
            "Total Clearing,,,,110.00,100.00",
            "Total Clearing,,,,110,100",
        );
    let report = inspect(source.as_bytes());
    assert_eq!(report["debit"], "110.00");
    assert_eq!(report["credit"], "100.00");
}

#[test]
fn amounts_preserve_scale_and_accept_decimal_unicode_digits() {
    for (input, expected) in [
        ("0", "0"),
        ("0.0", "0.0"),
        ("0001.20", "1.20"),
        ("999,999,999,999,999.99", "999999999999999.99"),
        (
            "\u{0661},\u{0662}\u{0663}\u{0664}.\u{0665}\u{0660}",
            "1234.50",
        ),
        ("\u{ff11}\u{ff12}.\u{ff13}\u{ff14}", "12.34"),
        ("\u{1d7d9}.\u{1d7da}", "1.2"),
    ] {
        assert_eq!(amount(input).unwrap().to_string(), expected, "{input:?}");
    }
}

#[test]
fn ambiguous_signed_oversized_and_malformed_amounts_are_refused() {
    for input in [
        "",
        " ",
        " 1",
        "1 ",
        "+1",
        "-1",
        "(1)",
        "1e2",
        "NaN",
        "Infinity",
        ".1",
        "1.",
        "1.001",
        "1000000000000000",
        "1,00",
        "1234,567",
        "1,,000",
        "1,000,000,000,000,000",
        "1 000",
        "1\u{066b}00",
        "\u{00b2}",
    ] {
        assert!(amount(input).is_err(), "accepted {input:?}");
    }
}

#[test]
fn detail_sums_are_exact_at_the_fifteen_digit_limit_and_keep_zero_scale() {
    let source = source_text()
        .replace("110.00,0,110.00", "999999999999999.90,0.0,110.00")
        .replace("SET-1,0,100.00,10.00", "SET-1,0.09,0.00,10.00")
        .replace(
            "Total Clearing,,,,110.00,100.00",
            "Total Clearing,,,,999999999999999.99,0",
        );
    let report = inspect(source.as_bytes());
    assert_eq!(report["debit"], "999999999999999.99");
    assert_eq!(report["credit"], "0.00");
}

#[test]
fn date_parsing_enforces_gregorian_leap_years_and_python_year_limits() {
    for (input, full, expected) in [
        ("1/9/2026", false, "2026-09-01"),
        ("29/02/2000", false, "2000-02-29"),
        ("29 Feb 2024", false, "2024-02-29"),
        ("1 January 0001", true, "0001-01-01"),
        ("31 December 9999", true, "9999-12-31"),
    ] {
        assert_eq!(parse_date(input, full).unwrap().iso(), expected);
    }
    for input in [
        "29/02/1900",
        "29/02/2100",
        "31/04/2026",
        "0/1/2026",
        "1/0/2026",
        "1/1/0000",
        "01/01/10000",
        "2026-09-01",
        "1 September 2026",
        "1 Sep 26",
    ] {
        assert!(parse_date(input, false).is_none(), "accepted {input:?}");
    }
    assert!(parse_date("1 Sep 2026", true).is_none());
}

#[test]
fn line_mapping_requires_exact_hash_shape_and_complete_logical_row_coverage() {
    let valid = mapping(SOURCE);
    for map in [
        Value::Null,
        json!({"source_sha256": "wrong", "lines": valid["lines"]}),
        json!({"source_sha256": valid["source_sha256"], "lines": valid["lines"], "extra": 1}),
        json!({"source_sha256": valid["source_sha256"]}),
        json!({"source_sha256": valid["source_sha256"], "lines": []}),
        json!({"source_sha256": valid["source_sha256"], "lines": {"9": "one"}}),
        json!({"source_sha256": valid["source_sha256"], "lines": {"09": "one", "10": "two"}}),
        json!({"source_sha256": valid["source_sha256"], "lines": {"9": "one", "10": "two", "11": "extra"}}),
    ] {
        assert!(converted(SOURCE, &map).is_err(), "accepted {map}");
    }
}

#[test]
fn line_ids_are_unique_ascii_schema_identifiers_with_an_eighty_character_limit() {
    let mut map = mapping(SOURCE);
    for id in [
        json!(""),
        json!("_id"),
        json!("id space"),
        json!("id\n"),
        json!("\u{00e9}id"),
        json!("a".repeat(81)),
        json!(12),
        Value::Null,
        json!("source-line-2"),
    ] {
        map["lines"]["9"] = id;
        assert!(converted(SOURCE, &map).is_err());
    }
    for id in ["A._:-9".to_owned(), "a".repeat(80)] {
        map["lines"]["9"] = json!(id);
        assert_eq!(converted(SOURCE, &map).unwrap()[0]["TransactionID"], id);
    }
}

#[test]
fn conversion_requires_explicit_account_identity_and_uppercase_currency() {
    let map = mapping(SOURCE);
    for (account, currency) in [
        ("", "AUD"),
        (" \t\u{1c}\u{a0}", "AUD"),
        ("090", "aud"),
        ("090", "AU"),
        ("090", "AUDD"),
        ("090", "A\u{00dc}D"),
    ] {
        assert!(convert(SOURCE, &map, TENANT, "Clearing", account, currency).is_err());
    }
    assert_eq!(
        convert(SOURCE, &map, TENANT, "Clearing", " 090 ", "AUD").unwrap()[0]["AccountID"],
        " 090 "
    );
}

#[test]
fn formula_prefixes_are_refused_after_python_whitespace_in_every_emitted_text_field() {
    let spaces = [
        '\t', '\n', '\u{b}', '\u{c}', '\r', ' ', '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', '\u{85}',
        '\u{a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}',
        '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}',
        '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
    ];
    let original = converted(SOURCE, &mapping(SOURCE)).unwrap();
    for field in ["Tenant", "AccountID", "Reference", "Description"] {
        for prefix in ['=', '+', '-', '@'] {
            for whitespace in spaces {
                let mut rows = original.clone();
                rows[1][field] = json!(format!("{whitespace}{prefix}SUM(A1)"));
                assert!(output_csv(&rows).unwrap_err().starts_with(field));
            }
        }
    }
    let mut rows = original;
    rows[1]["Description"] = json!("\u{200b}=literal");
    assert!(output_csv(&rows).unwrap().contains("\u{200b}=literal"));
}

#[test]
fn csv_uses_the_clearing_header_exact_scale_and_csv_escaping() {
    let source = changed("Receipt", "\"Receipt, \"\"quoted\"\"\ncontinued\"");
    let rows = converted(&source, &mapping(&source)).unwrap();
    let expected = format!(
        "{HEADER}Fabricated Services,090,AUD,source-line-1,2026-09-01,REC-1,\"Receipt, \"\"quoted\"\"\ncontinued\",110.00,0\n\
         Fabricated Services,090,AUD,source-line-2,2026-09-03,SET-1,Settlement,0,100.00\n"
    );
    assert_eq!(output_csv(&rows).unwrap(), expected);
    let temp = Temp::new();
    let path = temp.write("source.csv", &source);
    let map_bytes = serde_json::to_vec(&mapping(&source)).unwrap();
    let map_path = temp.write("mapping.json", &map_bytes);
    let output = cli(
        &path,
        Some(&map_path),
        &["--account-id", "090", "--currency", "AUD"],
    );
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, terminal(&expected));
    assert_eq!(fs::read(path).unwrap(), source);
    assert_eq!(fs::read(map_path).unwrap(), map_bytes);
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 2);
}

#[test]
fn cli_inspection_is_ascii_json_with_platform_line_endings_and_no_input_changes() {
    let source = changed("Receipt", "Receipt \u{1f680}");
    let temp = Temp::new();
    let path = temp.write("source.csv", &source);
    let output = cli(&path, None, &[]);
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert!(output.stdout.is_ascii());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        inspect(&source)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Receipt \\ud83d\\ude80"));
    assert!(text.ends_with(if cfg!(windows) { "\r\n" } else { "\n" }));
    assert_eq!(fs::read(path).unwrap(), source);
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
}

#[test]
fn cli_refuses_formula_in_the_last_row_before_emitting_any_csv() {
    let source = changed("Settlement", "\u{1f}\u{a0}=SUM(A1)");
    let temp = Temp::new();
    let path = temp.write("source.csv", &source);
    let map_bytes = serde_json::to_vec(&mapping(&source)).unwrap();
    let map_path = temp.write("mapping.json", &map_bytes);
    let output = cli(
        &path,
        Some(&map_path),
        &["--account-id", "090", "--currency", "AUD"],
    );
    refused(
        &output,
        1,
        "Description starts with a spreadsheet formula prefix",
    );
    assert_eq!(fs::read(path).unwrap(), source);
    assert_eq!(fs::read(map_path).unwrap(), map_bytes);
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 2);
}

#[test]
fn cli_runtime_failures_have_empty_stdout_and_leave_both_inputs_unchanged() {
    let temp = Temp::new();
    let path = temp.write("source.csv", SOURCE);
    let options = ["--account-id", "090", "--currency", "AUD"];
    for bytes in [
        b"{".to_vec(),
        serde_json::to_vec(&json!({})).unwrap(),
        b"\xff".to_vec(),
    ] {
        let map_path = temp.write("mapping.json", &bytes);
        refused(
            &cli(&path, Some(&map_path), &options),
            1,
            "Account Transactions:",
        );
        assert_eq!(fs::read(&path).unwrap(), SOURCE);
        assert_eq!(fs::read(map_path).unwrap(), bytes);
    }
    let map_path = temp.write(
        "mapping.json",
        &serde_json::to_vec(&mapping(SOURCE)).unwrap(),
    );
    refused(
        &cli(&path, Some(&map_path), &[]),
        1,
        "--account-id and --currency",
    );
    let invalid = changed("Total Clearing,,,,110.00", "Total Clearing,,,,109.99");
    fs::write(&path, &invalid).unwrap();
    refused(&cli(&path, Some(&map_path), &options), 1, "subtotal");
    assert_eq!(fs::read(&path).unwrap(), invalid);
    assert_eq!(
        fs::read(map_path).unwrap(),
        serde_json::to_vec(&mapping(SOURCE)).unwrap()
    );
}

#[test]
fn cli_distinguishes_help_usage_and_file_errors() {
    for argument in ["--help", "-hh", "-hhh", "-hfoo", "-h_-"] {
        let help = Command::new(env!("CARGO_BIN_EXE_xero-account-transactions"))
            .arg(argument)
            .output()
            .unwrap();
        assert_eq!(help.status.code(), Some(0), "{argument}");
        assert!(help.stderr.is_empty());
        assert!(String::from_utf8_lossy(&help.stdout).starts_with("usage:"));
    }
    for argument in ["-h-h", "-hh-h", "-h=foo", "-h="] {
        let usage = Command::new(env!("CARGO_BIN_EXE_xero-account-transactions"))
            .arg(argument)
            .output()
            .unwrap();
        refused(&usage, 2, "usage:");
    }
    let usage = Command::new(env!("CARGO_BIN_EXE_xero-account-transactions"))
        .output()
        .unwrap();
    refused(&usage, 2, "usage:");
    let temp = Temp::new();
    refused(
        &cli(&temp.0.join("missing.csv"), None, &[]),
        1,
        "Account Transactions:",
    );
    let path = temp.write("invalid.csv", b"\xff");
    refused(&cli(&path, None, &[]), 1, "Account Transactions:");
    assert_eq!(fs::read(path).unwrap(), b"\xff");
}

#[test]
fn transaction_test_source_remains_ascii() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/account_transactions.rs");
    assert!(fs::read(path).unwrap().is_ascii());
}

#[test]
fn python_date_digit_widths_and_lone_cr_output_quoting_are_retained() {
    assert!(parse_date("\u{661}/09/2026", false).is_none());
    assert!(parse_date("0\u{661}/09/2026", false).is_none());
    assert_eq!(
        parse_date("1\u{661}/09/2026", false).unwrap().iso(),
        "2026-09-11"
    );
    let source = changed("Receipt", "\"bare\rreturn\"");
    let rows = converted(&source, &mapping(&source)).unwrap();
    assert!(
        output_csv(&rows)
            .unwrap()
            .contains(",\"bare\rreturn\",110.00,0\n")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_xero-account-transactions"))
        .args(["--unknown", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
}
