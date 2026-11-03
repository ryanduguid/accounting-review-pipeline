//! The fabricated Xero trial-balance fixtures and the source pins on `Xero.TrialBalance.pq`.
//!
//! CI has no Power Query host, so a port of the parser's documented contract runs over
//! the fixtures, which makes their assertions about behaviour; the source pins are what
//! catch the M drifting away from it.

mod common;

use std::collections::BTreeMap;

use common::{Dec, Source, cell, read, record_widths, root, sample_records};

const COMBINED: &str = "samples/sample-xero-trial-balance.csv";
const SEPARATE: &str = "samples/sample-xero-trial-balance-columns.csv";
const EXCEL: &str = "samples/sample-xero-trial-balance-excel-export.csv";
const PARSER: &str = "powerquery/Xero.TrialBalance.pq";
const HEADER_NAMES: &[&str] = &["Account", "Account Code"];
const AMOUNT_COLUMNS: [&str; 4] = ["Debit", "Credit", "YTD Debit", "YTD Credit"];

/// The labels the Excel export carries when saved as CSV, observed on the Demo Company
/// (AU) on 5 September 2026, mapped onto the CSV-export names the way
/// `Xero.TrialBalance.pq` does before it chooses a pair.
const EXCEL_EXPORT_LABELS: [(&str, &str); 3] = [
    ("Account", "Account Name"),
    ("Debit - Year to date", "YTD Debit"),
    ("Credit - Year to date", "YTD Credit"),
];

#[derive(Clone, Debug, PartialEq)]
struct Account {
    code: Option<String>,
    name: String,
    kind: String,
    debit: Dec,
    credit: Dec,
}

/// The header row is the one whose first field is exactly "Account" or "Account Code",
/// the same test `Xero.TrialBalance.pq` runs.
fn header_index(rows: &[Vec<String>], name: &str) -> Result<usize, String> {
    rows.iter()
        .position(|row| {
            row.first()
                .is_some_and(|first| HEADER_NAMES.contains(&first.trim()))
        })
        .ok_or_else(|| format!("no header row in {name}"))
}

/// The same rows as a pre-YTD Xero export: every YTD column dropped.
///
/// Both committed fixtures carry both pairs, which is what makes the default and
/// useYTD = true the same column choice on them. This is the export shape that tells
/// those 2 branches apart.
fn without_ytd_columns(rows: &[Vec<String>], name: &str) -> Vec<Vec<String>> {
    let header = &rows[header_index(rows, name).unwrap()];
    let keep: Vec<_> = (0..header.len())
        .filter(|&index| !header[index].trim().starts_with("YTD "))
        .collect();
    rows.iter()
        .map(|row| {
            keep.iter()
                .filter_map(|&index| row.get(index).cloned())
                .collect()
        })
        .collect()
}

/// The `isCode` predicate from `Xero.TrialBalance.pq`, ported.
///
/// ASCII alphanumeric (`Text.Select` over "0".."9", "A".."Z" and "a".."z", not every
/// Unicode letter and digit), at most 10 characters, at least one digit, and not the
/// whole cell, so "Rent (Sydney)" keeps its parenthetical.
fn is_code(candidate: Option<&str>, whole_cell: &str) -> bool {
    candidate.is_some_and(|candidate| {
        !candidate.is_empty()
            && candidate != whole_cell
            && candidate.chars().count() <= 10
            && candidate
                .chars()
                .any(|character| character.is_ascii_digit())
            && candidate
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
    })
}

/// The combined-layout code and name split, ported: a trailing " (code)" first, a
/// "code - name" dash prefix as the fallback, otherwise no code.
fn split_combined_account(cell: &str) -> (Option<String>, String) {
    let trimmed = cell.trim();
    let paren_candidate = trimmed
        .strip_suffix(')')
        .and(trimmed.rfind(" ("))
        .map(|position| &trimmed[position + 2..trimmed.len() - 1]);
    let dash_candidate = trimmed.split_once(" - ").map(|(code, _)| code);
    let Some(code) = [paren_candidate, dash_candidate]
        .into_iter()
        .find(|candidate| is_code(*candidate, trimmed))
        .flatten()
    else {
        return (None, trimmed.to_owned());
    };
    let name = if trimmed.ends_with(&format!(" ({code})")) {
        &trimmed[..trimmed.len() - code.len() - 3]
    } else {
        trimmed.split_once(" - ").unwrap().1
    };
    (Some(code.to_owned()), name.to_owned())
}

/// A blank amount is a genuine zero. Exact decimals, not floats: the fixtures are money.
fn parse_amount(cell: &str) -> Dec {
    let text = cell.trim();
    if text.is_empty() {
        Dec::ZERO
    } else {
        Dec::of(text)
    }
}

/// Debit and Credit are the current-period movement, YTD Debit and YTD Credit the as-at
/// balances. `None` (the default) prefers as-at; an explicit choice demands its pair and
/// errors when the export lacks it. Returns the debit and credit column indexes.
fn pair_columns(
    header: &[String],
    name: &str,
    use_ytd: Option<bool>,
) -> Result<(usize, usize), String> {
    let column = |label: &str| header.iter().position(|cell| cell == label);
    let has_period = column("Debit").is_some() && column("Credit").is_some();
    let has_ytd = column("YTD Debit").is_some() && column("YTD Credit").is_some();
    let take_ytd = match use_ytd {
        Some(true) if !has_ytd => return Err(format!("useYTD = true but {name} has no YTD pair")),
        Some(false) if !has_period => {
            return Err(format!("useYTD = false but {name} has no period pair"));
        }
        Some(choice) => choice,
        None => has_ytd,
    };
    let (debit, credit) = if take_ytd {
        ("YTD Debit", "YTD Credit")
    } else {
        ("Debit", "Credit")
    };
    Ok((column(debit).unwrap(), column(credit).unwrap()))
}

fn parse_trial_balance(relative: &str, use_ytd: Option<bool>) -> Result<Vec<Account>, String> {
    parse_rows(&sample_records(relative), relative, use_ytd)
}

/// `Xero.TrialBalance.pq`'s documented contract, ported for the fixtures.
///
/// CI has no Power Query host, so this executes the specification, not the M. It is what
/// makes the fixture assertions about behaviour; the source pins that follow are what
/// catch the M drifting away from it.
fn parse_rows(
    rows: &[Vec<String>],
    name: &str,
    use_ytd: Option<bool>,
) -> Result<Vec<Account>, String> {
    let start = header_index(rows, name)?;
    let mut header: Vec<String> = rows[start]
        .iter()
        .map(|field| field.trim().to_owned())
        .collect();
    let split_layout = header[0] == "Account Code";
    if split_layout {
        for field in &mut header {
            if let Some((_, renamed)) = EXCEL_EXPORT_LABELS
                .iter()
                .find(|(label, _)| *label == field.as_str())
            {
                *field = (*renamed).to_owned();
            }
        }
    }
    let (debit, credit) = pair_columns(&header, name, use_ytd)?;
    let kind = header
        .iter()
        .position(|field| field == "Account Type")
        .unwrap_or_else(|| panic!("{name} has no Account Type column"));
    Ok(rows[start + 1..]
        .iter()
        .filter(|row| !row.is_empty())
        .filter_map(|row| {
            let (code, name) = account_identity(row, split_layout)?;
            Some(Account {
                code,
                name,
                kind: cell(row, kind).trim().to_owned(),
                debit: parse_amount(cell(row, debit)),
                credit: parse_amount(cell(row, credit)),
            })
        })
        .collect())
}

/// The row's code and name, or `None` for a blank or Total row.
fn account_identity(row: &[String], split_layout: bool) -> Option<(Option<String>, String)> {
    if split_layout {
        let (code, name) = (cell(row, 0).trim(), cell(row, 1).trim());
        if (code.is_empty() && name.is_empty()) || code == "Total" || name == "Total" {
            return None;
        }
        Some(((!code.is_empty()).then(|| code.to_owned()), name.to_owned()))
    } else {
        let account = cell(row, 0).trim();
        if account.is_empty() || account == "Total" {
            return None;
        }
        Some(split_combined_account(account))
    }
}

fn parsed(relative: &str, use_ytd: Option<bool>) -> Vec<Account> {
    parse_trial_balance(relative, use_ytd).unwrap_or_else(|error| panic!("{error}"))
}

/// Sum every amount column of a fixture, and read the fixture's own trailing Total row
/// (the one the parser drops) for comparison.
fn column_totals(relative: &str) -> (BTreeMap<&'static str, Dec>, BTreeMap<&'static str, Dec>) {
    let rows = sample_records(relative);
    let start = header_index(&rows, relative).unwrap();
    let header: Vec<_> = rows[start].iter().map(|cell| cell.trim()).collect();
    let mut summed: BTreeMap<_, _> = AMOUNT_COLUMNS
        .iter()
        .map(|&name| (name, Dec::ZERO))
        .collect();
    let mut declared = BTreeMap::new();
    for row in rows[start + 1..].iter().filter(|row| !row.is_empty()) {
        let is_total_row = row.iter().take(2).any(|cell| cell.trim() == "Total");
        for name in AMOUNT_COLUMNS {
            let index = header.iter().position(|cell| *cell == name).unwrap();
            let value = parse_amount(cell(row, index));
            if is_total_row {
                declared.insert(name, value);
            } else {
                summed.insert(name, summed[name] + value);
            }
        }
    }
    (summed, declared)
}

fn debit_total(accounts: &[Account]) -> Dec {
    accounts.iter().map(|account| account.debit).sum()
}

fn credit_total(accounts: &[Account]) -> Dec {
    accounts.iter().map(|account| account.credit).sum()
}

fn parser() -> Source {
    Source::adapter(PARSER)
}

/// The trial balance carried no width check while both aged adapters and
/// `PaydaySuper.Report` did, so a short record loaded with its absent figure padded to a
/// genuine zero and a surplus field was dropped in silence, and the tie-out check
/// downstream had no reason to look. Pinned as: the header's field count is measured from
/// the framed record, every record below it has to match, and a present empty cell stays
/// valid.
#[test]
fn trial_balance_refuses_a_record_the_header_width_does_not_match() {
    let text = read(&root().join(COMBINED));
    let fixture = text.strip_prefix('\u{feff}').unwrap_or(&text);
    // Control: the fabricated export is unchanged and still frames cleanly.
    let (width, body) = record_widths(fixture, HEADER_NAMES).unwrap();
    assert_eq!(width, 6);
    assert!(!body.is_empty());

    let row = "Accounts Receivable (610),Current Asset,1830.00,,42100.00,";
    assert!(
        fixture.contains(row),
        "{COMBINED} no longer carries {row:?}"
    );
    // A present blank amount is a genuine zero, not a missing field.
    assert_eq!(record_widths(fixture, HEADER_NAMES).unwrap().0, 6);
    // The audited cases: the final field removed, and a seventh field added.
    for malformed in [
        row[..row.rfind(',').unwrap()].to_owned(),
        format!("{row},extra"),
    ] {
        assert!(
            record_widths(&fixture.replace(row, &malformed), HEADER_NAMES).is_err(),
            "accepted {malformed:?}"
        );
    }
}

#[test]
fn trial_balance_pins_the_record_width_guard() {
    let source = parser();
    // The width check runs on the framed records, so a quoted newline stays inside one
    // record rather than becoming a short one.
    source.has_all(&[
        "Lines.FromText(csvText, QuoteStyle.Csv, false)",
        "recordWidth = (recordText as text, recordNumber as number) as number =>",
        "\"Record width does not match the header\"",
        "ExtraValues.Error",
    ]);
    // Csv.Document is shaped to the checked header width, so the refusal cannot be
    // stepped over.
    source.before("firstBadRecord =", "Raw = Csv.Document");
    source.has("Columns = columnCount,");
    // Text.Split on the header line counted a quoted comma as a separator.
    source.lacks("columnCount = List.Count(Text.Split(headerLine, \",\"))");
}

/// The doc comments below justify themselves by quoting the README. A quote beats a line
/// number (a one-line insert moves every number and makes the comment silently false),
/// but a reworded sentence would rot a quote just as quietly, so the sentences are pinned
/// here. This pins the side that changes: if a rewrite lands, this fails and names the
/// doc comments that have to be rewritten with it.
#[test]
fn the_readme_promises_these_tests_quote_are_still_in_the_readme() {
    Source::adapter("README.md").has_all(&[
        // both_fixtures_balance_and_match_their_own_total_row
        "After loading a TB, the first check is always",
        // both_layouts_carry_the_same_accounts_and_amounts
        "carrying the same accounts and amounts in both shapes",
        // code_less_parenthetical_account_parses_with_no_code
        "it must load with a null `AccountCode` and its full name intact",
        // default_takes_the_as_at_pair_and_use_ytd_false_the_movement
        "default returns as-at, `useYTD = false` for movement",
        // excel_export_saved_as_csv_loads_as_the_as_at_pair
        "The Excel export saved as CSV loads as the as-at pair",
    ]);
}

#[test]
fn code_less_parenthetical_account_is_present_in_both_layouts() {
    let combined = sample_records(COMBINED);
    let separate = sample_records(SEPARATE);
    assert!(
        combined
            .iter()
            .any(|row| *row == ["Rent (Sydney)", "Expense", "", "", "", ""]),
        "{COMBINED} lost the code-less Rent (Sydney) row"
    );
    assert!(
        separate
            .iter()
            .any(|row| *row == ["", "Rent (Sydney)", "Expense", "", "", "", ""]),
        "{SEPARATE} lost the code-less Rent (Sydney) row"
    );
}

/// The README: "After loading a TB, the first check is always" that debits equal credits
/// within tolerance. Nothing checked that of the fixtures themselves, so one mistyped
/// amount would send everyone following those instructions off to debug a parser that is
/// working correctly.
#[test]
fn both_fixtures_balance_and_match_their_own_total_row() {
    for path in [COMBINED, SEPARATE] {
        let (summed, declared) = column_totals(path);
        assert_eq!(summed["Debit"], summed["Credit"], "{path}");
        assert_eq!(summed["YTD Debit"], summed["YTD Credit"], "{path}");
        assert_eq!(summed["Debit"], Dec::of("6995.00"), "{path}");
        assert_eq!(summed["YTD Debit"], Dec::of("129934.50"), "{path}");
        // The fixture's own Total row is a second opinion on the same numbers; the parser
        // drops it, so it can drift unnoticed.
        assert_eq!(summed, declared, "{path}");
    }
}

/// The README calls the 2 fixtures "fabricated, balanced TBs carrying the same accounts
/// and amounts in both shapes". Stripping the 090 code from the separate-column fixture,
/// or editing one amount in either, left the whole suite green.
#[test]
fn both_layouts_carry_the_same_accounts_and_amounts() {
    fn projection(path: &str) -> Vec<(Option<String>, [String; 6])> {
        let as_at = parsed(path, None);
        let movement = parsed(path, Some(false));
        assert_eq!(as_at.len(), movement.len(), "{path}");
        let mut rows: Vec<_> = as_at
            .iter()
            .zip(&movement)
            .map(|(account, period)| {
                (
                    account.code.clone(),
                    [
                        account.name.clone(),
                        account.kind.clone(),
                        account.debit.to_string(),
                        account.credit.to_string(),
                        period.debit.to_string(),
                        period.credit.to_string(),
                    ],
                )
            })
            .collect();
        // A code-less account carries None, so order as if it were empty and keep the None.
        rows.sort_by(|a, b| {
            (a.0.as_deref().unwrap_or(""), &a.1).cmp(&(b.0.as_deref().unwrap_or(""), &b.1))
        });
        rows
    }
    let combined = projection(COMBINED);
    assert_eq!(combined.len(), 12);
    assert_eq!(combined, projection(SEPARATE));
}

/// The contract the README states, in "picks the right Debit/Credit pair (plain pair =
/// period movement, YTD pair = as-at balances; default returns as-at, `useYTD = false`
/// for movement)". The 2 answers both balance, so a swap is invisible to the balance
/// check the README prescribes; only the totals tell them apart.
#[test]
fn default_takes_the_as_at_pair_and_use_ytd_false_the_movement() {
    for path in [COMBINED, SEPARATE] {
        let as_at = parsed(path, None);
        assert_eq!(debit_total(&as_at), Dec::of("129934.50"), "{path}");
        assert_eq!(credit_total(&as_at), Dec::of("129934.50"), "{path}");

        let movement = parsed(path, Some(false));
        assert_eq!(debit_total(&movement), Dec::of("6995.00"), "{path}");
        assert_eq!(credit_total(&movement), Dec::of("6995.00"), "{path}");
        assert_ne!(movement, as_at, "{path}");

        // Asserting useYTD = true equals the default on a fixture that carries both pairs
        // compares the port with itself: both take the YTD pair down the same branch. The
        // branch worth exercising is the export WITHOUT the YTD pair, where the default
        // falls back to the movement and useYTD = true has to refuse rather than hand back
        // the movement labelled as an as-at balance.
        let rows = without_ytd_columns(&sample_records(path), path);
        assert_eq!(parse_rows(&rows, path, None).unwrap(), movement, "{path}");
        let refused = parse_rows(&rows, path, Some(true)).expect_err("useYTD = true must refuse");
        assert!(refused.contains("no YTD pair"), "{refused}");
    }
}

/// The README: "The Excel export saved as CSV loads as the as-at pair". Observed on the
/// Demo Company (AU) on 5 September 2026: the separate columns are labelled "Account",
/// "Debit - Year to date" and "Credit - Year to date", a comparative column is named for
/// the prior date, and there is no period pair at all. So the default and useYTD = true
/// both take the as-at pair, useYTD = false refuses, and the accounts and balances agree
/// with the 2 CSV-export fixtures.
#[test]
fn excel_export_saved_as_csv_loads_as_the_as_at_pair() {
    let as_at = parsed(EXCEL, None);
    assert_eq!(as_at.len(), 12);
    assert_eq!(debit_total(&as_at), Dec::of("129934.50"));
    assert_eq!(credit_total(&as_at), Dec::of("129934.50"));
    assert_eq!(as_at, parsed(EXCEL, Some(true)));

    let balances = |mut accounts: Vec<Account>| {
        accounts.sort_by(|a, b| {
            (a.code.as_deref().unwrap_or(""), &a.name)
                .cmp(&(b.code.as_deref().unwrap_or(""), &b.name))
        });
        accounts
    };
    assert_eq!(balances(as_at), balances(parsed(SEPARATE, None)));
    let refused = parse_trial_balance(EXCEL, Some(false)).expect_err("useYTD = false must refuse");
    assert!(refused.contains("no period pair"), "{refused}");
    // The M maps the same 3 labels, in the split layout only, before the pair is chosen;
    // the port above is what the fixture exercises.
    parser().matches(concat!(
        r"(?m)Promoted =\s*if splitLayout then\s*Table\.RenameColumns\(\s*PromotedRaw,\s*\{\s*",
        r#"\{"Account", "Account Name"\},\s*"#,
        r#"\{"Debit - Year to date", "YTD Debit"\},\s*"#,
        r#"\{"Credit - Year to date", "YTD Credit"\}\s*\},\s*"#,
        r"MissingField\.Ignore\s*\)\s*else\s*PromotedRaw,",
    ));
}

/// The other half of the same README sentence: the fixtures "include a code-less
/// `Rent (Sydney)` account: it must load with a null `AccountCode` and its full name
/// intact". Split as code "Sydney" and name "Rent" it collides with the real Rent (469)
/// account in any lead schedule keyed on AccountName.
#[test]
fn code_less_parenthetical_account_parses_with_no_code() {
    for path in [COMBINED, SEPARATE] {
        let accounts = parsed(path, None);
        let has = |code: Option<&str>, name: &str| {
            accounts
                .iter()
                .any(|account| account.code.as_deref() == code && account.name == name)
        };
        assert!(has(None, "Rent (Sydney)"), "{path}");
        assert!(!has(Some("Sydney"), "Rent"), "{path}");
        assert!(has(Some("469"), "Rent"), "{path}");
        // The leading zero survives because codes are text, not numbers.
        assert!(has(Some("090"), "Business Bank Account"), "{path}");
    }
}

/// Behaviour above is a specification; this is what fails when the M drifts. Drop the
/// at-least-one-digit rule, or widen the 10-character cap, and "Rent (Sydney)" splits
/// into code "Sydney" and name "Rent".
#[test]
fn combined_parser_pins_the_is_code_predicate() {
    let source = parser();
    source.matches(concat!(
        r"(?m)isCode = \(c as nullable text\) as logical =>\s*",
        r"c <> null\s*",
        r#"and c <> ""\s*"#,
        r"and c <> trimmed\s*",
        r"and Text\.Length\(c\) <= 10\s*",
        r#"and Text\.Select\(c, \{"0"\.\."9"\}\) <> ""\s*"#,
        r#"and Text\.Select\(c, \{"0"\.\."9", "A"\.\."Z", "a"\.\."z"\}\) = c,"#,
    ));
    // Binding the predicate is not using it.
    source.has_all(&[
        "if isCode(parenCandidate) then parenCandidate",
        "else if isCode(dashCandidate) then dashCandidate",
    ]);
}

/// The arm that actually extracts the code, pinned like the 2 it sits between. isCode
/// says what counts as a code and the AccountName arithmetic says what is left as the
/// name, but neither looks at the offsets that lift "090" out of "Business Bank Account
/// (090)", and those offsets have the same off-by-one shape: a " (" 2 characters wide
/// and a " (" plus ")" 3 characters wide.
///
/// Change the + 2 to a + 1 and every parenthetical candidate keeps a leading "(", which
/// isCode rejects, so AccountCode goes null on every coded account and AccountName keeps
/// the raw "Accounts Receivable (610)" cell. Every lead schedule keyed on code or name
/// breaks, and isCode, the dash fallback and the fixture assertions all stay green
/// because the fixture assertions run the port, not the M.
///
/// Occurrence.Last and the trailing-")" gate are pinned in the same pattern because the
/// 3 are one rule: on "Rent (Sydney) (469)" First takes "Sydney) (469" and the real 469
/// is lost, and the "- 3" length is only correct because the gate has already
/// established that the last character is the ")" it subtracts.
#[test]
fn combined_parser_pins_the_code_taken_by_the_parenthetical_split() {
    parser().matches(concat!(
        r"(?m)parenPos =\s*",
        r#"if Text\.EndsWith\(trimmed, "\)"\)\s*"#,
        r#"then Text\.PositionOf\(trimmed, " \(", Occurrence\.Last\)\s*"#,
        r"else -1,\s*",
        r"parenCandidate =\s*",
        r"if parenPos >= 0\s*",
        r"then Text\.Range\(trimmed, parenPos \+ 2, Text\.Length\(trimmed\) - parenPos - 3\)\s*",
        r"else null,",
    ));
}

/// The other arm of the same code and name split. isCode decides which text is the
/// code; this decides what is left as the name, and it is arithmetic on lengths: the
/// " (" and the ")" it strips are 3 characters. Change that 3 to a 2 and "Business Bank
/// Account (090)" loads as "Business Bank Account " with a trailing space, which
/// silently breaks every lead schedule and recon keyed on AccountName; the isCode pin
/// above and every fixture assertion stay green, because the code itself is still "090".
#[test]
fn combined_parser_pins_the_name_left_by_the_split() {
    let source = parser();
    source.matches(concat!(
        r#"(?m)WithName = Table\.AddColumn\(\s*WithCode,\s*"AccountName",\s*each\s*"#,
        r"let trimmed = Text\.Trim\(\[Account\]\)\s*in\s*",
        r"if \[AccountCode\] = null then trimmed\s*",
        r#"else if Text\.EndsWith\(trimmed, " \(" & \[AccountCode\] & "\)"\)\s*"#,
        r"then Text\.Start\(\s*trimmed,\s*",
        r"Text\.Length\(trimmed\) - Text\.Length\(\[AccountCode\]\) - 3\s*\)\s*",
        r#"else Text\.AfterDelimiter\(trimmed, " - "\),"#,
    ));
    // The derived name replaces the raw combined cell rather than sitting beside it, so
    // nothing downstream can read the unsplit one.
    source.has("Table.RemoveColumns(WithName, {\"Account\"})");
}

/// Both pairs balance, so choosing the wrong one still passes the README's balance check
/// and gives the accountant positive confirmation of a materially wrong trial balance.
/// Pin all 3 branches: the default prefers YTD, and each explicit choice errors rather
/// than falling back to the pair the caller did not ask for. Pin the 2 predicates those
/// branches read as well, because pinning a branch without its input leaves the default
/// flippable from the other end. Rename either YTD column header upstream and hasYTD goes
/// false, so the default hands back the current-period MOVEMENT in columns still named
/// Debit and Credit; that table balances too, so the README's prescribed check confirms
/// it.
#[test]
fn combined_parser_pins_the_debit_credit_pair_selection() {
    let source = parser();
    source.has_all(&[
        "hasPeriod = List.Contains(cols, \"Debit\") and List.Contains(cols, \"Credit\"),",
        "hasYTD = List.Contains(cols, \"YTD Debit\") and List.Contains(cols, \"YTD Credit\"),",
    ]);
    source.matches_all(&[
        r"(?m)takeYTD =\s*if useYTD = true then\s*if hasYTD then true\s*else error",
        r"(?m)else if useYTD = false then\s*if hasPeriod then false\s*else error",
        r"(?m)else\s*hasYTD,\s*Selected =",
        // ... and that takeYTD = true means the YTD pair, not the other one.
        concat!(
            r"(?m)else if takeYTD then\s*Table\.RenameColumns\(\s*",
            r#"Table\.RemoveColumns\(Promoted, \{"Debit", "Credit"\}, MissingField\.Ignore\),\s*"#,
            r#"\{\{"YTD Debit", "Debit"\}, \{"YTD Credit", "Credit"\}\}"#,
        ),
    ]);
}

#[test]
fn combined_parser_guards_the_dash_fallback() {
    parser().matches(concat!(
        r#"(?m)dashCandidate\s*=\s*if Text\.Contains\(trimmed, " - "\)\s*"#,
        r#"then Text\.BeforeDelimiter\(trimmed, " - "\)"#,
    ));
}
