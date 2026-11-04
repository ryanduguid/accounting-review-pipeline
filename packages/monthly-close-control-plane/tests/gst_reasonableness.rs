//! The GST reasonableness control, run through the command a reviewer uses.

#[path = "support/temp.rs"]
mod temp;

use monthly_close_control_plane::pyjson::{self, Json};
use std::{fs, process::Command};
use temp::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_close-control");
const TENANT: &str = "Fabricated Lumbridge";

/// One account's movement for the month, in cents, and its GST class; an
/// empty class leaves the account out of the GST mapping.
#[derive(Clone)]
struct Line {
    id: &'static str,
    section: &'static str,
    name: &'static str,
    class: &'static str,
    debit: i64,
    credit: i64,
}

fn line(id: &'static str, section: &'static str, name: &'static str, class: &'static str) -> Line {
    Line {
        id,
        section,
        name,
        class,
        debit: 0,
        credit: 0,
    }
}

/// A month whose GST agrees with its sales and purchases. Taxable sales of
/// 10000.00 expect 1000.00 collected and creditable purchases of 4000.00 expect
/// 400.00 paid, so the GST account moves 600.00 net. GST-free, input-taxed and
/// excluded amounts sit beside them, and the bank balances the month.
fn matching() -> Vec<Line> {
    let mut lines = vec![
        line("210", "Liabilities", "GST", "gst"),
        line("300", "Revenue", "Sales", "taxable_sales"),
        line("310", "Revenue", "Export Sales", "gst_free"),
        line("320", "Revenue", "Interest Income", "input_taxed"),
        line("400", "Expenses", "Purchases", "creditable_purchases"),
        line("410", "Expenses", "Wages", "bas_excluded"),
        line("420", "Expenses", "Bank Fees", "input_taxed"),
        line("900", "Equity", "Retained Earnings", ""),
    ];
    for (id, debit, credit) in [
        ("210", 40_000, 100_000),
        ("300", 0, 1_000_000),
        ("310", 0, 200_000),
        ("320", 0, 10_000),
        ("400", 400_000, 0),
        ("410", 300_000, 0),
        ("420", 5_000, 0),
    ] {
        let found = lines.iter_mut().find(|l| l.id == id).unwrap();
        (found.debit, found.credit) = (debit, credit);
    }
    lines
}

fn set(lines: &mut [Line], id: &str, debit: i64, credit: i64) {
    let found = lines.iter_mut().find(|l| l.id == id).unwrap();
    (found.debit, found.credit) = (debit, credit);
}

fn money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    format!("{sign}{}.{:02}", cents.abs() / 100, cents.abs() % 100)
}

/// The month's trial balance with the balancing bank row, as at `date`. July
/// is the first month of the year, so July's YTD is its movement and August's
/// is twice it.
fn trial_balance(lines: &[Line], date: &str, months: i64) -> String {
    let net: i64 = lines.iter().map(|l| l.credit - l.debit).sum();
    let mut rows = lines.to_vec();
    rows.push(Line {
        debit: net.max(0),
        credit: (-net).max(0),
        ..line("100", "Assets", "Operating Bank", "bas_excluded")
    });
    let mut csv = String::from(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n",
    );
    for l in rows {
        csv += &format!(
            "{date},{TENANT},{},{},{},{},{},{},{},{}\n",
            l.section,
            l.id,
            l.name,
            l.id,
            money(l.debit),
            money(l.credit),
            money(l.debit * months),
            money(l.credit * months),
        );
    }
    csv
}

fn mapping(lines: &[Line]) -> String {
    let mut csv = String::from("AccountID,GSTClass\n100,bas_excluded\n");
    for l in lines.iter().filter(|l| !l.class.is_empty()) {
        csv += &format!("{},{}\n", l.id, l.class);
    }
    csv
}

struct Run {
    _temp: TempDir,
    code: i32,
    stderr: String,
    pack: Option<Json>,
    dir: std::path::PathBuf,
}

/// A declaration that no BAS payment, refund or clearing journal reached the
/// GST accounts this period.
const NO_SETTLEMENTS: &str = "Reference,Amount\n";

/// Run `close-control review` on the month, with `gst` as the GST mapping when
/// supplied and, with it, a declaration of no settlements.
fn review(lines: &[Line], gst: Option<String>, extra: &[&str]) -> Run {
    let settlements = gst.as_ref().map(|_| NO_SETTLEMENTS.to_owned());
    review_settled(lines, gst, settlements, extra)
}

/// Run `close-control review` on the month, with `gst` as the GST mapping and
/// `settlements` as the GST settlements declaration, each when supplied. The
/// variance threshold is set out of reach: every account doubles its YTD
/// balance, and this file tests GST alone.
fn review_settled(
    lines: &[Line],
    gst: Option<String>,
    settlements: Option<String>,
    extra: &[&str],
) -> Run {
    let temp = TempDir::new();
    let write = |name: &str, contents: String| {
        let path = temp.join(name);
        fs::write(&path, contents).unwrap();
        path.into_os_string()
    };
    let dir = temp.join("pack");
    let mut command = Command::new(BIN);
    command
        .arg("review")
        .arg("--current")
        .arg(write("current.csv", trial_balance(lines, "2026-08-31", 2)))
        .arg("--prior")
        .arg(write("prior.csv", trial_balance(lines, "2026-07-31", 1)))
        .args(["--absolute-threshold", "1000000", "--output"])
        .arg(&dir)
        .args(extra);
    if let Some(gst) = gst {
        command
            .arg("--gst-mapping")
            .arg(write("gst-mapping.csv", gst));
    }
    if let Some(settlements) = settlements {
        command
            .arg("--gst-settlements")
            .arg(write("gst-settlements.csv", settlements));
    }
    let output = command.output().unwrap();
    let pack = fs::read_to_string(dir.join("close-review-pack.json"))
        .ok()
        .map(|text| pyjson::loads(&text).unwrap());
    Run {
        _temp: temp,
        code: output.status.code().unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
        pack,
        dir,
    }
}

fn gst_findings(pack: &Json) -> Vec<&Json> {
    pack.at("exceptions")
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e.at("control").is_str("gst_reasonableness"))
        .collect()
}

fn text(value: &Json) -> String {
    value.as_pystr().unwrap().os_text()
}

fn not_run(pack: &Json) -> bool {
    pack.at("controls_not_run")
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c.is_str("gst_reasonableness"))
}

#[test]
fn matching_gst_raises_nothing_and_the_pack_records_the_tolerance() {
    let lines = matching();
    let run = review(&lines, Some(mapping(&lines)), &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let pack = run.pack.unwrap();
    assert!(pack.at("overall_status").is_str("PASS"));
    assert!(gst_findings(&pack).is_empty());
    assert!(!not_run(&pack));
    assert!(pack.at("source_sha256").get("gst_mapping").is_some());
    let thresholds = pack.at("thresholds");
    assert!(thresholds.at("gst_absolute_tolerance").is_str("1.00"));
    assert!(thresholds.at("gst_percentage_tolerance").is_str("1.00%"));
    let view = Command::new(BIN)
        .args(["view", "--pack-dir"])
        .arg(&run.dir)
        .output()
        .unwrap();
    assert_eq!(view.status.code(), Some(0));
    assert!(
        String::from_utf8(view.stdout).unwrap().contains(
            "- GST reasonableness review heuristic, not a rounding bound: $1.00 plus 1.00% of the expected GST on sales and purchases, after the declared GST settlements, which the trial balance cannot verify"
        )
    );
    // A tolerance edited out of the JSON alone no longer verifies.
    let json = run.dir.join("close-review-pack.json");
    let edited: Vec<_> = fs::read_to_string(&json)
        .unwrap()
        .lines()
        .filter(|line| !line.contains("gst_absolute_tolerance"))
        .map(str::to_owned)
        .collect();
    fs::write(&json, edited.join("\n") + "\n").unwrap();
    let view = Command::new(BIN)
        .args(["view", "--pack-dir"])
        .arg(&run.dir)
        .output()
        .unwrap();
    assert_eq!(view.status.code(), Some(1));
    assert!(
        String::from_utf8(view.stderr)
            .unwrap()
            .contains("thresholds must hold exactly")
    );
}

#[test]
fn under_collected_gst_beyond_tolerance_is_raised_with_both_figures() {
    // A 1000.00 taxable sale recorded with no GST: 700.00 expected, 600.00 recorded.
    let mut lines = matching();
    set(&mut lines, "300", 0, 1_100_000);
    let run = review(&lines, Some(mapping(&lines)), &[]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    let pack = run.pack.unwrap();
    let found = gst_findings(&pack);
    assert_eq!(found.len(), 1);
    let finding = found[0];
    assert!(finding.at("status").is_str("REVIEW"));
    assert!(finding.at("tenant").is_str(TENANT));
    assert!(finding.at("current_value").is_str("600.00"));
    assert!(finding.at("prior_value").is_str("700.00"));
    assert!(finding.at("difference").is_str("-100.00"));
    // 1.00 plus 1% of the 1100.00 collected and 400.00 paid expected.
    assert!(finding.at("threshold").is_str("16.0000"));
    assert_eq!(
        text(finding.at("reason")),
        "Net GST recorded this period is 600.00 (600.00 of movement in the GST accounts plus 0.00 of declared settlements), against 700.00 expected: GST collected 1100.00 (1/10 of taxable sales 11000.00) less GST paid 400.00 (1/10 of creditable purchases 4000.00). The difference of -100.00 is beyond the 16.0000 review heuristic, which is not a rounding bound: rounding across many small invoices can exceed it. Amounts are net GST payable; a negative amount is a net refund. The settlements are as declared; the trial balance cannot verify them."
    );
    assert!(!not_run(&pack));
    let queries = pack.at("client_queries").as_array().unwrap();
    assert!(queries.is_empty(), "the firm traces GST; no client query");
}

#[test]
fn a_difference_at_the_tolerance_is_not_raised_and_one_cent_more_is() {
    // 600.00 expected; the tolerance is 1.00 plus 1% of 1400.00, so 15.00.
    let mut lines = matching();
    set(&mut lines, "210", 40_000, 98_500);
    let run = review(&lines, Some(mapping(&lines)), &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(gst_findings(run.pack.as_ref().unwrap()).is_empty());
    set(&mut lines, "210", 40_000, 98_499);
    let run = review(&lines, Some(mapping(&lines)), &[]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    let pack = run.pack.unwrap();
    let found = gst_findings(&pack);
    assert_eq!(found.len(), 1);
    assert!(found[0].at("difference").is_str("-15.01"));
    assert!(found[0].at("threshold").is_str("15.0000"));
}

#[test]
fn an_unclassed_account_with_movement_stops_the_control_with_its_reason() {
    let mut lines = matching();
    lines.iter_mut().find(|l| l.id == "410").unwrap().class = "";
    let run = review(&lines, Some(mapping(&lines)), &[]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    let pack = run.pack.unwrap();
    assert!(pack.at("overall_status").is_str("REVIEW"));
    assert!(not_run(&pack));
    let found = gst_findings(&pack);
    assert_eq!(found.len(), 1);
    assert!(found[0].at("difference").is_str(""));
    assert_eq!(
        text(found[0].at("reason")),
        "The GST mapping does not class 1 account(s) with movement this period, so their taxable amounts cannot be told from GST-free, input-taxed or excluded ones: 410 / Wages. The GST reasonableness control did not run, so this pack carries no GST result; this is not a pass."
    );
}

#[test]
fn a_mixed_account_with_movement_stops_the_control_with_its_reason() {
    let mut lines = matching();
    lines.iter_mut().find(|l| l.id == "300").unwrap().class = "mixed";
    let run = review(&lines, Some(mapping(&lines)), &[]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    let pack = run.pack.unwrap();
    assert!(not_run(&pack));
    let found = gst_findings(&pack);
    assert_eq!(found.len(), 1);
    assert!(text(found[0].at("reason")).starts_with(
        "1 account(s) classed mixed moved this period, so their taxable amounts cannot be separated from the rest: 300 / Sales. The GST reasonableness control did not run"
    ));
}

#[test]
fn the_tolerance_options_configure_the_comparison() {
    let mut lines = matching();
    set(&mut lines, "300", 0, 1_100_000);
    let run = review(
        &lines,
        Some(mapping(&lines)),
        &[
            "--gst-absolute-tolerance",
            "100",
            "--gst-percentage-tolerance",
            "0",
        ],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let thresholds = run.pack.as_ref().unwrap().at("thresholds");
    assert!(thresholds.at("gst_absolute_tolerance").is_str("100.00"));
    assert!(thresholds.at("gst_percentage_tolerance").is_str("0.00%"));
    let run = review(
        &lines,
        Some(mapping(&lines)),
        &["--gst-percentage-tolerance", "1.5"],
    );
    assert_eq!(run.code, 1);
    assert!(run.stderr.contains("argument --gst-percentage-tolerance"));
}

#[test]
fn a_review_without_a_gst_mapping_keeps_its_previous_pack() {
    let run = review(&matching(), None, &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    let pack = run.pack.unwrap();
    assert!(
        pack.at("thresholds")
            .get("gst_absolute_tolerance")
            .is_none()
    );
    assert!(pack.at("source_sha256").get("gst_mapping").is_none());
    assert!(!not_run(&pack));
}

#[test]
fn malformed_gst_mappings_are_refused_before_any_pack_is_written() {
    let lines = matching();
    for (csv, message) in [
        (
            "AccountID,GSTClass\n210,gst\n300,taxable\n",
            "row 3 GSTClass 'taxable' is not one of",
        ),
        (
            "AccountID,GSTClass\n210,gst\n210,gst_free\n",
            "duplicate AccountID '210'",
        ),
        (
            "AccountID,GSTClass\n300,taxable_sales\n",
            "classes no account gst",
        ),
        ("AccountID,GSTClass\n", "classes no account gst"),
        ("AccountID,Class\n210,gst\n", "canonical schema mismatch"),
        ("AccountID,GSTClass\n210,\n", "GSTClass"),
    ] {
        let run = review(&lines, Some(csv.into()), &[]);
        assert_eq!(run.code, 1, "{csv}");
        assert!(run.stderr.contains(message), "{csv}: {}", run.stderr);
        assert!(run.pack.is_none(), "{csv}");
    }
}

#[test]
fn a_non_creditable_purchase_is_accepted_and_expects_no_gst_paid() {
    // 550.00 spent on a purchase whose GST cannot be claimed: left out, so the
    // 600.00 net GST still matches.
    let mut lines = matching();
    lines.push(Line {
        debit: 55_000,
        ..line("430", "Expenses", "Private Use", "non_creditable")
    });
    let run = review(&lines, Some(mapping(&lines)), &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert!(gst_findings(run.pack.as_ref().unwrap()).is_empty());
    // Classed creditable, the same purchase would expect 55.00 of GST paid.
    lines.last_mut().unwrap().class = "creditable_purchases";
    let run = review(&lines, Some(mapping(&lines)), &[]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    let pack = run.pack.unwrap();
    assert!(gst_findings(&pack)[0].at("prior_value").is_str("545.00"));
}

#[test]
fn rounding_on_many_small_sales_can_exceed_the_heuristic_and_is_raised_for_review() {
    // 300 taxable sales of 0.06, each invoice's 0.006 of GST rounded to the
    // nearest cent: 3.00 recorded against 1.80 expected. The 1.20 is legitimate
    // rounding, and the 1.0180 heuristic still asks for a review of it.
    let lines = vec![
        Line {
            credit: 300,
            ..line("210", "Liabilities", "GST", "gst")
        },
        Line {
            credit: 1_800,
            ..line("300", "Revenue", "Sales", "taxable_sales")
        },
    ];
    let run = review(&lines, Some(mapping(&lines)), &[]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    let pack = run.pack.unwrap();
    let found = gst_findings(&pack);
    assert_eq!(found.len(), 1);
    assert!(found[0].at("status").is_str("REVIEW"));
    assert!(found[0].at("difference").is_str("1.20"));
    assert!(found[0].at("threshold").is_str("1.0180"));
    assert_eq!(
        text(found[0].at("reason")),
        "Net GST recorded this period is 3.00 (3.00 of movement in the GST accounts plus 0.00 of declared settlements), against 1.80 expected: GST collected 1.80 (1/10 of taxable sales 18.00) less GST paid 0.00 (1/10 of creditable purchases 0.00). The difference of 1.20 is beyond the 1.0180 review heuristic, which is not a rounding bound: rounding across many small invoices can exceed it. Amounts are net GST payable; a negative amount is a net refund. The settlements are as declared; the trial balance cannot verify them."
    );
    let view = Command::new(BIN)
        .args(["view", "--pack-dir"])
        .arg(&run.dir)
        .output()
        .unwrap();
    assert!(String::from_utf8(view.stdout).unwrap().contains(
        "- GST reasonableness review heuristic, not a rounding bound: $1.00 plus 1.00% of the expected GST on sales and purchases, after the declared GST settlements, which the trial balance cannot verify"
    ));
}

/// The matching month with its 600.00 of net GST paid to the ATO: a debit to
/// the GST account with no sale or purchase behind it.
fn paid_to_the_ato() -> Vec<Line> {
    let mut lines = matching();
    set(&mut lines, "210", 100_000, 100_000);
    lines
}

/// Purchases of 13000.00 expect 1300.00 of GST paid against 1000.00 collected,
/// a 300.00 refund, and the ATO's refund is credited to the GST account.
fn refunded_by_the_ato() -> Vec<Line> {
    let mut lines = matching();
    set(&mut lines, "400", 1_300_000, 0);
    set(&mut lines, "210", 130_000, 130_000);
    lines
}

/// The matching month's 600.00 of net GST journalled out of the GST account
/// into the ATO integrated client account.
fn cleared_to_the_ato_account() -> Vec<Line> {
    let mut lines = matching();
    set(&mut lines, "210", 100_000, 100_000);
    lines.push(Line {
        credit: 60_000,
        ..line(
            "220",
            "Liabilities",
            "ATO Integrated Client Account",
            "bas_excluded",
        )
    });
    lines
}

#[test]
fn a_declared_bas_payment_refund_or_clearing_journal_is_set_aside() {
    for (lines, declared) in [
        (
            paid_to_the_ato(),
            "BAS July 2026 payment (ATO account),600.00\n",
        ),
        (
            refunded_by_the_ato(),
            "BAS July 2026 refund (ATO account),-300.00\n",
        ),
        (
            cleared_to_the_ato_account(),
            "GST cleared to ICA (journal 41),600.00\n",
        ),
    ] {
        let settlements = format!("{NO_SETTLEMENTS}{declared}");
        let run = review_settled(&lines, Some(mapping(&lines)), Some(settlements), &[]);
        assert_eq!(run.code, 0, "{declared}: {}", run.stderr);
        let pack = run.pack.unwrap();
        assert!(gst_findings(&pack).is_empty(), "{declared}");
        assert!(!not_run(&pack), "{declared}");
        assert!(pack.at("source_sha256").get("gst_settlements").is_some());
        // Declared as none, the same entry moves the GST account with no sale
        // or purchase behind it, and the control raises it.
        let run = review(&lines, Some(mapping(&lines)), &[]);
        assert_eq!(run.code, 2, "{declared}: {}", run.stderr);
        assert_eq!(
            gst_findings(run.pack.as_ref().unwrap()).len(),
            1,
            "{declared}"
        );
    }
}

#[test]
fn a_declared_payment_does_not_hide_gst_recorded_twice() {
    // 1200.00 of GST recorded against 600.00 expected, then 600.00 paid to the
    // ATO: the account moves 600.00 net, as if it agreed.
    let mut lines = matching();
    set(&mut lines, "210", 100_000, 160_000);
    let declared = format!("{NO_SETTLEMENTS}BAS July 2026 payment (ATO account),600.00\n");
    let run = review_settled(&lines, Some(mapping(&lines)), Some(declared), &[]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    let pack = run.pack.unwrap();
    let found = gst_findings(&pack);
    assert_eq!(found.len(), 1);
    assert!(found[0].at("current_value").is_str("1200.00"));
    assert!(found[0].at("prior_value").is_str("600.00"));
    assert!(found[0].at("difference").is_str("600.00"));
    assert!(found[0].at("threshold").is_str("15.0000"));
    assert_eq!(
        text(found[0].at("reason")),
        "Net GST recorded this period is 1200.00 (600.00 of movement in the GST accounts plus 600.00 of declared settlements), against 600.00 expected: GST collected 1000.00 (1/10 of taxable sales 10000.00) less GST paid 400.00 (1/10 of creditable purchases 4000.00). The difference of 600.00 is beyond the 15.0000 review heuristic, which is not a rounding bound: rounding across many small invoices can exceed it. Amounts are net GST payable; a negative amount is a net refund. The settlements are as declared; the trial balance cannot verify them."
    );
    // Declared as none, the payment hides the error: the control relies on
    // the declaration and cannot check it against the trial balance.
    let run = review(&lines, Some(mapping(&lines)), &[]);
    assert_eq!(run.code, 0, "{}", run.stderr);
}

#[test]
fn a_gst_mapping_without_a_settlements_declaration_does_not_run() {
    let lines = matching();
    let run = review_settled(&lines, Some(mapping(&lines)), None, &[]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    let pack = run.pack.unwrap();
    assert!(pack.at("overall_status").is_str("REVIEW"));
    assert!(not_run(&pack));
    let found = gst_findings(&pack);
    assert_eq!(found.len(), 1);
    assert_eq!(
        text(found[0].at("reason")),
        "No GST settlements declaration was supplied, so a BAS payment, refund or clearing journal in the GST accounts cannot be told from GST on sales and purchases. The GST reasonableness control did not run, so this pack carries no GST result; this is not a pass."
    );
}

#[test]
fn a_malformed_or_absent_settlements_file_does_not_run_and_says_why() {
    let lines = matching();
    for (csv, message) in [
        (
            "Reference,Amount\nBAS July 2026,six hundred\n",
            "row 2 has invalid Amount: 'six hundred'.",
        ),
        (
            "Reference,Amount\nBAS July 2026,600.00\nBAS July 2026,600.00\n",
            "duplicate Reference 'BAS July 2026'.",
        ),
        (
            "Reference,Amount\n,600.00\n",
            "row 2 has an empty Reference.",
        ),
        (
            "Reference,Value\nBAS July 2026,600.00\n",
            "canonical schema mismatch (missing Amount; unexpected Value).",
        ),
        ("", "CSV has no header row."),
    ] {
        let run = review_settled(&lines, Some(mapping(&lines)), Some(csv.into()), &[]);
        assert_eq!(run.code, 2, "{csv}: {}", run.stderr);
        let pack = run.pack.unwrap();
        assert!(not_run(&pack), "{csv}");
        assert!(pack.at("source_sha256").get("gst_settlements").is_some());
        let reason = text(gst_findings(&pack)[0].at("reason"));
        assert!(
            reason.starts_with("The GST settlements declaration cannot be used: "),
            "{reason}"
        );
        assert!(reason.contains(message), "{csv}: {reason}");
        assert!(reason.ends_with("this is not a pass."), "{reason}");
    }
    let absent = TempDir::new();
    let path = absent.join("gst-settlements.csv");
    let run = review_settled(
        &lines,
        Some(mapping(&lines)),
        None,
        &["--gst-settlements", path.to_str().unwrap()],
    );
    assert_eq!(run.code, 2, "{}", run.stderr);
    let pack = run.pack.unwrap();
    assert!(not_run(&pack));
    assert!(pack.at("source_sha256").get("gst_settlements").is_none());
    let reason = text(gst_findings(&pack)[0].at("reason"));
    assert!(
        reason.contains("GST settlements file does not exist"),
        "{reason}"
    );
}

#[test]
fn a_settlements_declaration_without_a_gst_mapping_is_refused() {
    let run = review_settled(&matching(), None, Some(NO_SETTLEMENTS.into()), &[]);
    assert_eq!(run.code, 1);
    assert!(
        run.stderr
            .contains("A GST settlements declaration requires a GST mapping."),
        "{}",
        run.stderr
    );
    assert!(run.pack.is_none());
}

/// The review command's GST usage and help lines, checked here on their own as
/// well as through the differential probe, which drops them before comparing
/// its output with the frozen baseline.
const GST_USAGE: &str = concat!(
    "                            [--reconciliation-tolerance RECONCILIATION_TOLERANCE]\n",
    "                            [--gst-mapping GST_MAPPING]\n",
    "                            [--gst-settlements GST_SETTLEMENTS]\n",
    "                            [--gst-absolute-tolerance GST_ABSOLUTE_TOLERANCE]\n",
    "                            [--gst-percentage-tolerance GST_PERCENTAGE_TOLERANCE]\n",
);
const GST_HELP: &str = concat!(
    "  --reconciliation-tolerance RECONCILIATION_TOLERANCE\n",
    "                        maximum permitted GL/subledger difference\n",
    "  --gst-mapping GST_MAPPING\n",
    "                        optional AccountID,GSTClass CSV; runs the GST\n",
    "                        reasonableness control\n",
    "  --gst-settlements GST_SETTLEMENTS\n",
    "                        Reference,Amount CSV of BAS payments, refunds and\n",
    "                        clearing journals in the GST accounts, each as its\n",
    "                        net debit; required with --gst-mapping, header only\n",
    "                        when there were none\n",
    "  --gst-absolute-tolerance GST_ABSOLUTE_TOLERANCE\n",
    "                        GST difference in dollars allowed before review,\n",
    "                        added to the percentage tolerance; a review heuristic,\n",
    "                        not a rounding bound\n",
    "  --gst-percentage-tolerance GST_PERCENTAGE_TOLERANCE\n",
    "                        GST difference allowed before review, as a fraction\n",
    "                        of the expected GST on sales and purchases, e.g. 0.01\n",
    "                        for 1%\n",
);

#[test]
fn review_and_workbench_help_and_usage_show_the_gst_options() {
    let lf = |bytes: Vec<u8>| String::from_utf8(bytes).unwrap().replace("\r\n", "\n");
    for command in ["review", "workbench"] {
        let help = Command::new(BIN)
            .args([command, "--help"])
            .output()
            .unwrap();
        assert_eq!(help.status.code(), Some(0), "{command}");
        let help = lf(help.stdout);
        assert!(help.contains(GST_USAGE), "{command}: {help}");
        assert!(help.ends_with(GST_HELP), "{command}: {help}");
        let usage = Command::new(BIN).arg(command).output().unwrap();
        assert_eq!(usage.status.code(), Some(1), "{command}");
        let usage = lf(usage.stderr);
        assert!(usage.contains(GST_USAGE), "{command}: {usage}");
    }
}
