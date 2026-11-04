//! Seeded-condition benchmark. Each case seeds one labelled condition in a clean,
//! fabricated close and records which controls should raise. The counts measure these planted
//! cases only, never real-world accuracy, and the README's Detection benchmark table must match
//! them.

#[path = "support/temp.rs"]
mod temp;

use monthly_close_control_plane::{
    engine::{self, ReviewConfig},
    models::Control,
};
use std::{collections::BTreeSet, fs};
use temp::TempDir;

const README: &str = include_str!("../README.md");
const TENANT: &str = "Fabricated Varrock";

/// Amounts are in cents.
#[derive(Clone)]
struct Account {
    section: &'static str,
    id: &'static str,
    name: &'static str,
    code: &'static str,
    debit: i64,
    credit: i64,
    ytd_debit: i64,
    ytd_credit: i64,
}

#[derive(Clone)]
struct Close {
    current_date: &'static str,
    prior_date: &'static str,
    current: Vec<Account>,
    prior: Vec<Account>,
    mapping: Vec<(&'static str, &'static str)>,
    mapping_policy: Vec<(&'static str, &'static str)>,
    balance_policy: Vec<(&'static str, &'static str, &'static str)>,
    subledger: Vec<(&'static str, i64)>,
    gst_mapping: Vec<(&'static str, &'static str)>,
}

impl Close {
    fn account(&mut self, id: &str) -> &mut Account {
        self.current.iter_mut().find(|a| a.id == id).unwrap()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// Inside the target control's contract: it should raise an exception naming this
    /// account, or an empty account for a file-level exception.
    Planted(&'static str),
    /// From the target control's 'Cannot detect' column: it should not raise.
    BlindSpot,
}

struct Case {
    id: &'static str,
    control: Control,
    kind: Kind,
    /// Every control that should raise, including the target for a planted case.
    expected: &'static [Control],
    plant: fn(&mut Close),
}

fn dollars(amount: i64) -> i64 {
    amount * 100
}

fn money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    format!("{sign}{}.{:02}", cents.abs() / 100, cents.abs() % 100)
}

#[allow(clippy::too_many_arguments)]
fn account(
    section: &'static str,
    id: &'static str,
    name: &'static str,
    code: &'static str,
    debit: i64,
    credit: i64,
    ytd_debit: i64,
    ytd_credit: i64,
) -> Account {
    Account {
        section,
        id,
        name,
        code,
        debit: dollars(debit),
        credit: dollars(credit),
        ytd_debit: dollars(ytd_debit),
        ytd_credit: dollars(ytd_credit),
    }
}

/// A balanced July to August close in one financial year, with every movement under the
/// variance thresholds and every optional input consistent, so no control raises.
#[expect(
    clippy::too_many_lines,
    reason = "one literal fixture of a balanced close"
)]
fn clean() -> Close {
    Close {
        current_date: "2026-08-31",
        prior_date: "2026-07-31",
        prior: vec![
            account(
                "Assets",
                "100",
                "Operating Bank",
                "1000",
                5000,
                0,
                105000,
                0,
            ),
            account("Assets", "110", "Trade Debtors", "1100", 3000, 0, 73000, 0),
            account(
                "Liabilities",
                "200",
                "Trade Creditors",
                "2000",
                0,
                4000,
                0,
                58000,
            ),
            account("Liabilities", "210", "GST Payable", "2100", 0, 100, 0, 800),
            account(
                "Revenue",
                "300",
                "Sales Revenue",
                "4000",
                0,
                10000,
                0,
                120000,
            ),
            account(
                "Expenses",
                "400",
                "Cost of Sales",
                "5000",
                6000,
                0,
                72000,
                0,
            ),
            account(
                "Expenses",
                "500",
                "Operating Expenses",
                "6000",
                7100,
                0,
                84800,
                0,
            ),
            account(
                "Equity",
                "900",
                "Retained Earnings",
                "3000",
                0,
                7000,
                0,
                156000,
            ),
        ],
        current: vec![
            account("Assets", "100", "Operating Bank", "1000", 430, 0, 105430, 0),
            account("Assets", "110", "Trade Debtors", "1100", 300, 0, 73300, 0),
            account(
                "Liabilities",
                "200",
                "Trade Creditors",
                "2000",
                0,
                400,
                0,
                58400,
            ),
            account("Liabilities", "210", "GST Payable", "2100", 0, 30, 0, 830),
            account(
                "Revenue",
                "300",
                "Sales Revenue",
                "4000",
                0,
                6300,
                0,
                126300,
            ),
            account(
                "Expenses",
                "400",
                "Cost of Sales",
                "5000",
                3000,
                0,
                75000,
                0,
            ),
            account(
                "Expenses",
                "500",
                "Operating Expenses",
                "6000",
                3000,
                0,
                87800,
                0,
            ),
            account(
                "Equity",
                "900",
                "Retained Earnings",
                "3000",
                0,
                0,
                0,
                156000,
            ),
        ],
        mapping: vec![
            ("100", "Cash and cash equivalents"),
            ("110", "Receivables"),
            ("200", "Payables"),
            ("210", "Payables"),
            ("300", "Revenue"),
            ("400", "Cost of sales"),
            ("500", "Operating expenses"),
            ("900", "Equity"),
        ],
        mapping_policy: vec![
            ("Assets", "Cash and cash equivalents"),
            ("Assets", "Receivables"),
            ("Liabilities", "Payables"),
            ("Revenue", "Revenue"),
            ("Expenses", "Cost of sales"),
            ("Expenses", "Operating expenses"),
            ("Equity", "Equity"),
        ],
        balance_policy: vec![
            ("100", "debit", "yes"),
            ("200", "credit", "yes"),
            ("210", "credit", "no"),
            ("900", "credit", "no"),
        ],
        subledger: vec![("110", dollars(73300)), ("200", dollars(-58400))],
        // 6300 of taxable sales less 6000 of creditable purchases: 30 of net GST.
        gst_mapping: vec![
            ("100", "bas_excluded"),
            ("110", "bas_excluded"),
            ("200", "bas_excluded"),
            ("210", "gst"),
            ("300", "taxable_sales"),
            ("400", "creditable_purchases"),
            ("500", "creditable_purchases"),
            ("900", "bas_excluded"),
        ],
    }
}

use Control::*;

const CASES: &[Case] = &[
    Case {
        id: "period-debits-off-by-a-cent",
        control: TrialBalanceIntegrity,
        kind: Kind::Planted(""),
        expected: &[TrialBalanceIntegrity],
        plant: |c| c.account("100").debit += 1,
    },
    Case {
        id: "ytd-credits-off-by-5",
        control: TrialBalanceIntegrity,
        kind: Kind::Planted(""),
        expected: &[TrialBalanceIntegrity],
        plant: |c| c.account("300").ytd_credit += dollars(5),
    },
    Case {
        id: "1000-posted-to-the-wrong-expense-account",
        control: TrialBalanceIntegrity,
        kind: Kind::BlindSpot,
        expected: &[],
        plant: |c| {
            for (id, change) in [("400", -1000), ("500", 1000)] {
                let a = c.account(id);
                a.debit += dollars(change);
                a.ytd_debit += dollars(change);
            }
        },
    },
    Case {
        id: "prior-period-in-the-previous-financial-year",
        control: FinancialYearReset,
        kind: Kind::Planted(""),
        expected: &[FinancialYearReset],
        plant: |c| c.prior_date = "2026-06-30",
    },
    Case {
        id: "new-account-in-the-current-period",
        control: PeriodComparison,
        kind: Kind::Planted("510"),
        expected: &[PeriodComparison],
        plant: |c| {
            c.current
                .push(account("Expenses", "510", "Travel", "6100", 0, 0, 0, 0));
            c.mapping.push(("510", "Operating expenses"));
        },
    },
    Case {
        id: "account-dropped-from-the-current-period",
        control: PeriodComparison,
        kind: Kind::Planted("210"),
        expected: &[PeriodComparison, GstReasonableness],
        plant: drop_gst_payable,
    },
    Case {
        id: "expenses-overstated-by-20000",
        control: PeriodVariance,
        kind: Kind::Planted("500"),
        expected: &[PeriodVariance],
        plant: |c| overstate_expenses(c, 20000),
    },
    Case {
        id: "expenses-overstated-by-5000-under-10-percent",
        control: PeriodVariance,
        kind: Kind::BlindSpot,
        expected: &[],
        plant: |c| overstate_expenses(c, 5000),
    },
    Case {
        id: "offsetting-entries-leave-ytd-unchanged",
        control: PeriodVariance,
        kind: Kind::BlindSpot,
        expected: &[],
        plant: |c| {
            let bank = c.account("100");
            bank.debit += dollars(5000);
            bank.credit += dollars(5000);
            bank.ytd_debit += dollars(5000);
            bank.ytd_credit += dollars(5000);
        },
    },
    Case {
        id: "account-renamed",
        control: AccountMetadata,
        kind: Kind::Planted("400"),
        expected: &[AccountMetadata],
        plant: |c| c.account("400").name = "Purchases",
    },
    Case {
        id: "account-code-changed",
        control: AccountMetadata,
        kind: Kind::Planted("500"),
        expected: &[AccountMetadata],
        plant: |c| c.account("500").code = "6100",
    },
    Case {
        id: "account-id-changed",
        control: AccountMetadata,
        kind: Kind::BlindSpot,
        expected: &[PeriodComparison, AccountMapping, GstReasonableness],
        plant: |c| c.account("400").id = "401",
    },
    Case {
        id: "account-with-no-mapping-row",
        control: AccountMapping,
        kind: Kind::Planted("500"),
        expected: &[AccountMapping],
        plant: |c| c.mapping.retain(|(id, _)| *id != "500"),
    },
    Case {
        id: "prior-only-account-with-no-mapping-row",
        control: AccountMapping,
        kind: Kind::BlindSpot,
        expected: &[PeriodComparison, GstReasonableness],
        plant: |c| {
            drop_gst_payable(c);
            c.mapping.retain(|(id, _)| *id != "210");
        },
    },
    Case {
        id: "expense-account-mapped-to-revenue",
        control: MappingCompatibility,
        kind: Kind::Planted("500"),
        expected: &[MappingCompatibility],
        plant: |c| remap(c, "500", "Revenue"),
    },
    Case {
        id: "account-mapped-to-a-group-with-no-policy-entry",
        control: MappingCompatibility,
        kind: Kind::Planted("500"),
        expected: &[MappingCompatibility],
        plant: |c| remap(c, "500", "Sundry"),
    },
    Case {
        id: "debtors-mapped-to-cash-which-the-policy-permits",
        control: MappingCompatibility,
        kind: Kind::BlindSpot,
        expected: &[],
        plant: |c| remap(c, "110", "Cash and cash equivalents"),
    },
    Case {
        id: "bank-account-with-no-movement",
        control: BalancePolicy,
        kind: Kind::Planted("100"),
        expected: &[BalancePolicy],
        plant: |c| {
            c.account("100").debit = 0;
            c.account("110").debit += dollars(430);
        },
    },
    Case {
        id: "gst-payable-on-the-debit-side",
        control: BalancePolicy,
        kind: Kind::Planted("210"),
        expected: &[BalancePolicy],
        plant: gst_debit_balance,
    },
    Case {
        id: "gst-payable-on-the-debit-side-unlisted",
        control: BalancePolicy,
        kind: Kind::BlindSpot,
        expected: &[],
        plant: |c| {
            gst_debit_balance(c);
            c.balance_policy.retain(|(id, _, _)| *id != "210");
        },
    },
    Case {
        id: "debtors-ledger-differs-by-100",
        control: SubledgerReconciliation,
        kind: Kind::Planted("110"),
        expected: &[SubledgerReconciliation],
        plant: |c| c.subledger[0].1 += dollars(100),
    },
    Case {
        id: "subledger-row-with-no-account",
        control: SubledgerReconciliation,
        kind: Kind::Planted("999"),
        expected: &[SubledgerReconciliation],
        plant: |c| c.subledger.push(("999", dollars(100))),
    },
    Case {
        id: "creditors-understated-with-no-subledger-row",
        control: SubledgerReconciliation,
        kind: Kind::BlindSpot,
        expected: &[],
        plant: |c| {
            c.subledger.retain(|(id, _)| *id != "200");
            c.account("200").ytd_credit -= dollars(500);
            c.account("300").ytd_credit += dollars(500);
        },
    },
    Case {
        id: "taxable-sale-recorded-without-gst",
        control: GstReasonableness,
        kind: Kind::Planted(""),
        expected: &[GstReasonableness],
        plant: |c| sell_through_the_bank(c, 1000),
    },
    Case {
        id: "gst-paid-recorded-with-no-purchase",
        control: GstReasonableness,
        kind: Kind::Planted(""),
        expected: &[GstReasonableness],
        plant: |c| {
            let gst = c.account("210");
            gst.debit += dollars(100);
            gst.ytd_debit += dollars(100);
            let bank = c.account("100");
            bank.credit += dollars(100);
            bank.ytd_credit += dollars(100);
        },
    },
    Case {
        id: "gst-charged-on-a-gst-free-sale-in-taxable-sales",
        control: GstReasonableness,
        kind: Kind::BlindSpot,
        expected: &[],
        plant: |c| {
            sell_through_the_bank(c, 1000);
            let gst = c.account("210");
            gst.credit += dollars(100);
            gst.ytd_credit += dollars(100);
            let bank = c.account("100");
            bank.debit += dollars(100);
            bank.ytd_debit += dollars(100);
        },
    },
    Case {
        id: "gst-short-by-the-same-on-a-sale-and-a-purchase",
        control: GstReasonableness,
        kind: Kind::BlindSpot,
        expected: &[],
        plant: |c| overstate_expenses(c, 1000),
    },
];

/// A taxable sale banked with no GST recorded, in the period and year to date.
fn sell_through_the_bank(c: &mut Close, amount: i64) {
    let sales = c.account("300");
    sales.credit += dollars(amount);
    sales.ytd_credit += dollars(amount);
    let bank = c.account("100");
    bank.debit += dollars(amount);
    bank.ytd_debit += dollars(amount);
}

/// Overstate operating expenses against sales, in the period and year to date.
fn overstate_expenses(c: &mut Close, amount: i64) {
    let expenses = c.account("500");
    expenses.debit += dollars(amount);
    expenses.ytd_debit += dollars(amount);
    let sales = c.account("300");
    sales.credit += dollars(amount);
    sales.ytd_credit += dollars(amount);
}

/// GST Payable leaves the current period; equity absorbs its balance and movement.
fn drop_gst_payable(c: &mut Close) {
    c.current.retain(|a| a.id != "210");
    let equity = c.account("900");
    equity.credit += dollars(30);
    equity.ytd_credit += dollars(830);
}

fn remap(c: &mut Close, id: &str, group: &'static str) {
    c.mapping.iter_mut().find(|(row, _)| *row == id).unwrap().1 = group;
}

/// GST Payable moves to a 170.00 debit balance, balanced against sales.
fn gst_debit_balance(c: &mut Close) {
    c.account("210").ytd_debit += dollars(1000);
    c.account("300").ytd_credit += dollars(1000);
}

fn trial_balance(date: &str, accounts: &[Account]) -> String {
    let mut csv = String::from(
        "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n",
    );
    for a in accounts {
        csv += &format!(
            "{date},{TENANT},{},{},{},{},{},{},{},{}\n",
            a.section,
            a.id,
            a.name,
            a.code,
            money(a.debit),
            money(a.credit),
            money(a.ytd_debit),
            money(a.ytd_credit),
        );
    }
    csv
}

/// Each control that raised, with the account its exception names (empty for a
/// file-level exception).
#[expect(
    clippy::too_many_lines,
    reason = "writes the close inputs, then runs the review"
)]
fn raised(close: &Close) -> BTreeSet<(&'static str, String)> {
    let temp = TempDir::new();
    let write = |name: &str, contents: String| {
        let path = temp.join(name);
        fs::write(&path, contents).unwrap();
        path
    };
    let table = |header: &str, rows: Vec<String>| format!("{header}\n{}\n", rows.join("\n"));
    let mut config = ReviewConfig::new(
        write(
            "current.csv",
            trial_balance(close.current_date, &close.current),
        ),
        write("prior.csv", trial_balance(close.prior_date, &close.prior)),
    );
    config.mapping_path = Some(write(
        "mapping.csv",
        table(
            "AccountID,ReviewGroup",
            close
                .mapping
                .iter()
                .map(|(id, group)| format!("{id},{group}"))
                .collect(),
        ),
    ));
    config.mapping_policy_path = Some(write(
        "mapping_policy.csv",
        table(
            "Section,ReviewGroup",
            close
                .mapping_policy
                .iter()
                .map(|(s, g)| format!("{s},{g}"))
                .collect(),
        ),
    ));
    config.balance_policy_path = Some(write(
        "balance_policy.csv",
        table(
            "AccountID,ExpectedBalance,ExpectMovement",
            close
                .balance_policy
                .iter()
                .map(|(id, side, moves)| format!("{id},{side},{moves}"))
                .collect(),
        ),
    ));
    config.subledger_path = Some(write(
        "subledger.csv",
        table(
            "Tenant,AccountID,SubledgerBalance",
            close
                .subledger
                .iter()
                .map(|(id, cents)| format!("{TENANT},{id},{}", money(*cents)))
                .collect(),
        ),
    ));
    config.gst_mapping_path = Some(write(
        "gst_mapping.csv",
        table(
            "AccountID,GSTClass",
            close
                .gst_mapping
                .iter()
                .map(|(id, class)| format!("{id},{class}"))
                .collect(),
        ),
    ));
    let pack = engine::review_close(&config).unwrap();
    pack.exceptions
        .iter()
        .map(|e| {
            let control = Control::ALL
                .iter()
                .map(|c| c.as_str())
                .find(|c| *c == e.control);
            (control.unwrap(), e.account_id.clone())
        })
        .collect()
}

fn controls(raised: &BTreeSet<(&'static str, String)>) -> BTreeSet<&'static str> {
    raised.iter().map(|(control, _)| *control).collect()
}

fn planted(case: &Case) -> Close {
    let mut close = clean();
    (case.plant)(&mut close);
    close
}

const HEADER: [&str; 2] = [
    "| Control | Planted cases detected | Blind-spot cases missed, as documented | False alarms |",
    "| --- | --- | --- | --- |",
];

/// The benchmark table: the header, one row per benchmarked control in `models::Control`
/// order, then the totals. The clean close counts towards false alarms too.
fn results() -> Vec<String> {
    let outcomes: Vec<(&Case, BTreeSet<(&str, String)>)> = CASES
        .iter()
        .map(|case| (case, raised(&planted(case))))
        .collect();
    let clean_raised = controls(&raised(&clean()));
    let mut table: Vec<String> = HEADER.iter().map(|line| (*line).to_owned()).collect();
    let mut totals = [0; 5];
    for control in Control::ALL {
        let name = control.as_str();
        let cases: Vec<_> = outcomes
            .iter()
            .filter(|(case, _)| case.control == *control)
            .collect();
        if cases.is_empty() {
            continue;
        }
        // Planted, detected, blind spots, missed, false alarms.
        let mut counts = [0; 5];
        for (case, raised) in &cases {
            match case.kind {
                Kind::Planted(account) => {
                    counts[0] += 1;
                    counts[1] += usize::from(raised.contains(&(name, account.to_owned())));
                }
                Kind::BlindSpot => {
                    counts[2] += 1;
                    counts[3] += usize::from(!controls(raised).contains(name));
                }
            }
        }
        counts[4] = outcomes
            .iter()
            .filter(|(case, raised)| {
                controls(raised).contains(name) && !case.expected.iter().any(|c| c.as_str() == name)
            })
            .count()
            + usize::from(clean_raised.contains(name));
        for (total, count) in totals.iter_mut().zip(counts) {
            *total += count;
        }
        table.push(row(&format!("`{name}`"), counts));
    }
    table.push(row("Total", totals));
    table
}

fn row(label: &str, [planted, detected, blind, missed, false_alarms]: [usize; 5]) -> String {
    let blind_cell = if blind == 0 {
        "none planted".to_owned()
    } else {
        format!("{missed} of {blind}")
    };
    format!("| {label} | {detected} of {planted} | {blind_cell} | {false_alarms} |")
}

fn documented_results() -> Vec<&'static str> {
    let section: Vec<&str> = README
        .lines()
        .skip_while(|line| *line != "## Detection benchmark")
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .collect();
    assert!(
        !section.is_empty(),
        "README.md has no '## Detection benchmark' section"
    );
    section
        .into_iter()
        .filter(|line| line.starts_with('|'))
        .collect()
}

#[test]
fn each_case_raises_exactly_its_expected_controls() {
    assert_eq!(raised(&clean()), BTreeSet::new(), "the clean close raised");
    for case in CASES {
        let raised = raised(&planted(case));
        let expected: BTreeSet<&str> = case.expected.iter().map(|c| c.as_str()).collect();
        assert_eq!(controls(&raised), expected, "case {}", case.id);
        if let Kind::Planted(account) = case.kind {
            let target = (case.control.as_str(), account.to_owned());
            assert!(
                raised.contains(&target),
                "case {}: no {} exception names account '{account}'; raised {raised:?}",
                case.id,
                target.0,
            );
        }
    }
}

#[test]
fn readme_benchmark_table_matches_the_cases() {
    assert_eq!(
        documented_results(),
        results(),
        "README Detection benchmark table differs from tests/benchmark.rs"
    );
}
