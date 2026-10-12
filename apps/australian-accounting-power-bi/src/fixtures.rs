//! Deterministic synthetic fixtures for the Australian Accounting Power BI model.
//!
//! Generates balanced double-entry general ledger journals, the chart of accounts,
//! multi-entity masters, budgets, Payday Super payroll events (STP Phase 2) and
//! fabricated industry benchmark reference values. No client or real taxpayer data is
//! used; every entity has a fictional place name.

use std::fs;
use std::io;
use std::path::Path;

use crate::calendar::add_business_days;
use crate::date::Date;
use crate::decimal::Decimal;
use crate::payday::{ADMIN_UPLIFT_RATE, GicRates, notional_earnings};

/// One CSV file: a header and its rows, written as `csv.DictWriter` writes them.
pub struct Table {
    pub header: Vec<&'static str>,
    pub rows: Vec<Vec<String>>,
}

/// A field as the excel dialect's QUOTE_MINIMAL writes it.
fn csv_field(field: &str) -> String {
    if field.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

impl Table {
    /// The file bytes, with `\n` line endings.
    pub fn to_csv(&self) -> String {
        let header: Vec<String> = self.header.iter().map(|h| (*h).to_owned()).collect();
        std::iter::once(&header)
            .chain(&self.rows)
            .map(|row| row.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(",") + "\n")
            .collect()
    }

    /// A table written as `|`-separated lines, the first holding the column names.
    fn from_text(text: &'static str) -> Table {
        let mut lines = text.lines().map(|line| line.split('|'));
        let header = lines.next().expect("a header line").collect();
        let rows = lines.map(|fields| fields.map(str::to_owned).collect()).collect();
        Table { header, rows }
    }
}

fn d(text: &str) -> Decimal {
    Decimal::parse(text)
}

/// Every ABN satisfies the statutory modulus-89 checksum that Fx_ValidateABN applies,
/// so the shipped entity master passes the model's own validator. Keep it that way.
const ENTITIES: &str = "\
EntityID|LegalName|TradingName|ABN|ACN|TaxStructure|EntityRole|ANZSIC_Code|Currency|ConsolidationWeight
ENT001|Varrock Ventures Pty Ltd|Varrock Advisory & Tech|51824753556|123456789|Company|Parent Operating|6962|AUD|1.0
ENT002|Draynor Produce Pty Ltd|Draynor Fresh Foods|82147609350|234567890|Company|Trading Subsidiary|4122|AUD|1.0
ENT003|Falador Freight Pty Ltd|Falador National Logistics|33087542991|345678901|Company|Logistics Subsidiary|4610|AUD|1.0
ENT004|Ardougne Holdings Trust|Ardougne Commercial Property|12325678901||Unit Trust|Property Asset Trust|6712|AUD|1.0
";

/// Assets, liabilities, equity, revenue, cost of sales and operating expenses.
const CHART_OF_ACCOUNTS: &str = "\
AccountCode|AccountName|Class|SubClass|ReportSection|BalanceSheetGroup|CashFlowCategory|NormalBalance|SortOrder
100|Cash at Bank - Operating|Asset|Current Assets|Balance Sheet|Cash & Cash Equivalents|Operating|Debit|10
110|Trade Receivables|Asset|Current Assets|Balance Sheet|Receivables|Operating|Debit|20
120|Inventories on Hand|Asset|Current Assets|Balance Sheet|Inventory|Operating|Debit|30
180|Intercompany Loan Receivable|Asset|Current Assets|Balance Sheet|Intercompany Receivables|Financing|Debit|40
200|Property, Plant & Equipment|Asset|Non-Current Assets|Balance Sheet|Fixed Assets|Investing|Debit|50
210|Accumulated Depreciation|Asset|Non-Current Assets|Balance Sheet|Fixed Assets|Investing|Credit|60
250|Commercial Real Estate (Property)|Asset|Non-Current Assets|Balance Sheet|Investment Property|Investing|Debit|70
300|Trade Payables|Liability|Current Liabilities|Balance Sheet|Payables|Operating|Credit|110
310|GST Clearing Account|Liability|Current Liabilities|Balance Sheet|Tax Liabilities|Operating|Credit|120
320|PAYG Withholding Payable|Liability|Current Liabilities|Balance Sheet|Tax Liabilities|Operating|Credit|130
330|Superannuation Guarantee Payable|Liability|Current Liabilities|Balance Sheet|Payroll Liabilities|Operating|Credit|140
380|Intercompany Loan Payable|Liability|Current Liabilities|Balance Sheet|Intercompany Payables|Financing|Credit|150
400|Bank Commercial Loan Facility|Liability|Non-Current Liabilities|Balance Sheet|Borrowings|Financing|Credit|160
500|Share Capital / Settled Sum|Equity|Equity|Balance Sheet|Contributed Capital|Financing|Credit|210
510|Retained Earnings - Prior Years|Equity|Equity|Balance Sheet|Retained Profits|Operating|Credit|220
600|Commercial Trading Revenue|Revenue|Operating Revenue|Profit and Loss|Revenue|Operating|Credit|310
610|Professional Advisory Fees|Revenue|Operating Revenue|Profit and Loss|Revenue|Operating|Credit|320
620|Commercial Rental Income|Revenue|Operating Revenue|Profit and Loss|Revenue|Operating|Credit|330
650|Intercompany Management Fees Revenue|Revenue|Intercompany Revenue|Profit and Loss|Intercompany Revenue|Operating|Credit|340
700|Direct Cost of Goods Sold|Expense|Cost of Sales|Profit and Loss|Cost of Sales|Operating|Debit|410
710|Direct Freight & Fuel Expense|Expense|Cost of Sales|Profit and Loss|Cost of Sales|Operating|Debit|420
750|Intercompany Freight Expense|Expense|Cost of Sales|Profit and Loss|Cost of Sales|Operating|Debit|430
800|Salaries & Wages|Expense|Operating Expenses|Profit and Loss|Operating Expenses|Operating|Debit|510
805|Superannuation Guarantee Expense|Expense|Operating Expenses|Profit and Loss|Operating Expenses|Operating|Debit|520
810|Motor Vehicle Running Costs|Expense|Operating Expenses|Profit and Loss|Operating Expenses|Operating|Debit|530
820|Rent & Property Occupancy|Expense|Operating Expenses|Profit and Loss|Operating Expenses|Operating|Debit|540
830|Insurance & Compliance|Expense|Operating Expenses|Profit and Loss|Operating Expenses|Operating|Debit|550
880|Intercompany Management Fees Expense|Expense|Operating Expenses|Profit and Loss|Operating Expenses|Operating|Debit|560
890|Depreciation Expense|Expense|Operating Expenses|Profit and Loss|Operating Expenses|Operating|Debit|570
895|Finance & Bank Interest Expense|Expense|Operating Expenses|Profit and Loss|Operating Expenses|Operating|Debit|580
";

const LEDGER_HEADER: &[&str] = &[
    "JournalID",
    "LineNumber",
    "PostingDate",
    "EntityID",
    "AccountCode",
    "Debit",
    "Credit",
    "Amount",
    "Description",
    "IntercompanyEntityID",
    "IsIntercompany",
];

type Lines<'a> = &'a [(&'a str, Decimal)];

/// Balanced journals in posting order, numbered from JNL1001.
struct Ledger {
    rows: Vec<Vec<String>>,
    journal_id: u32,
}

/// `round(sum(amounts), 2)`, summed from integer zero as Python's `sum` does.
fn total(lines: Lines<'_>) -> Decimal {
    lines
        .iter()
        .fold(Decimal::zero(), |sum, (_, amount)| sum.add(amount))
        .round(2)
}

impl Ledger {
    fn journal(&mut self, header: [&str; 3], debits: Lines<'_>, credits: Lines<'_>) -> Result<(), String> {
        self.journal_with(header, debits, credits, "", None)
    }

    /// One journal. `counterparty` marks an intercompany journal; `key` keeps an
    /// existing identifier instead of taking the next number.
    fn journal_with(
        &mut self,
        [date, entity, description]: [&str; 3],
        debits: Lines<'_>,
        credits: Lines<'_>,
        counterparty: &str,
        key: Option<String>,
    ) -> Result<(), String> {
        let key = key.unwrap_or_else(|| {
            self.journal_id += 1;
            format!("JNL{}", self.journal_id)
        });
        let (debit_total, credit_total) = (total(debits), total(credits));
        if debit_total != credit_total {
            return Err(format!(
                "Unbalanced journal {}: Debits {} != Credits {}",
                self.journal_id,
                debit_total.to_fixed(2),
                credit_total.to_fixed(2)
            ));
        }
        let flag = if counterparty.is_empty() { "FALSE" } else { "TRUE" };
        let lines = debits
            .iter()
            .map(|line| (line, true))
            .chain(credits.iter().map(|line| (line, false)));
        for (number, ((code, amount), debit)) in lines.enumerate() {
            let shown = amount.to_fixed(2);
            let (debit_text, credit_text, net) = if debit {
                (shown.clone(), "0.00".to_owned(), shown)
            } else {
                ("0.00".to_owned(), shown, amount.neg().to_fixed(2))
            };
            self.rows.push(
                [
                    &key,
                    &(number + 1).to_string(),
                    date,
                    entity,
                    code,
                    &debit_text,
                    &credit_text,
                    &net,
                    description,
                    counterparty,
                    flag,
                ]
                .map(|field| (*field).to_owned())
                .to_vec(),
            );
        }
        Ok(())
    }
}

/// Opening balances at 1 July 2024, balanced per entity.
fn opening_balances(ledger: &mut Ledger) -> Result<(), String> {
    let lines = |pairs: &[(&'static str, &str)]| {
        pairs
            .iter()
            .map(|(code, amount)| (*code, d(amount)))
            .collect::<Vec<_>>()
    };
    let opening = [
        (
            "ENT001",
            lines(&[("100", "350000.0"), ("110", "120000.0"), ("200", "80000.0")]),
            lines(&[("300", "90000.0"), ("500", "100000.0"), ("510", "360000.0")]),
        ),
        (
            "ENT002",
            lines(&[
                ("100", "180000.0"),
                ("110", "95000.0"),
                ("120", "150000.0"),
                ("200", "65000.0"),
            ]),
            lines(&[("300", "110000.0"), ("500", "50000.0"), ("510", "330000.0")]),
        ),
        (
            "ENT003",
            lines(&[("100", "95000.0"), ("110", "140000.0"), ("200", "450000.0")]),
            lines(&[
                ("210", "90000.0"),
                ("300", "75000.0"),
                ("400", "250000.0"),
                ("500", "50000.0"),
                ("510", "220000.0"),
            ]),
        ),
        (
            "ENT004",
            lines(&[("100", "60000.0"), ("250", "1800000.0")]),
            lines(&[("400", "900000.0"), ("500", "100.0"), ("510", "959900.0")]),
        ),
    ];
    for (entity, debits, credits) in &opening {
        ledger.journal(["2024-07-01", entity, "Opening Balance FY25"], debits, credits)?;
    }
    Ok(())
}

/// One month of the operating cycle: the 15th and 28th of the month.
struct Month {
    number: u32,
    year: i32,
    month: u32,
    mid: String,
    end: String,
    factor: Decimal,
}

impl Month {
    /// Month `index` of the 36 from July 2024, with its growth factor.
    fn new(index: u32) -> Month {
        let year = 2024 + ((7 + index - 1) / 12) as i32;
        let month = (7 + index - 1) % 12 + 1;
        let factor = d("1.0").add(&Decimal::from_int(i64::from(index)).mul(&d("0.015")));
        let day = |day| Date::new(year, month, day).to_string();
        Month {
            number: index + 1,
            year,
            month,
            mid: day(15),
            end: day(28),
            factor,
        }
    }

    fn scaled(&self, base: &str) -> Decimal {
        d(base).mul(&self.factor).round(2)
    }

    fn label(&self, text: &str) -> String {
        format!("{text} - M{}", self.number)
    }
}

fn super_on(salary: &Decimal) -> Decimal {
    salary.mul(&d("0.12")).round(2)
}

/// Varrock Ventures (advisory) and Draynor Produce (fresh foods).
fn advisory_and_produce(ledger: &mut Ledger, m: &Month) -> Result<(), String> {
    let (revenue, salary) = (m.scaled("160000.0"), m.scaled("75000.0"));
    let super_amount = super_on(&salary);
    ledger.journal(
        [&m.mid, "ENT001", &m.label("Advisory Billings")],
        &[("110", revenue.clone())],
        &[("610", revenue)],
    )?;
    let (pay, liability) = (
        [("800", salary.clone()), ("805", super_amount.clone())],
        [("100", salary), ("330", super_amount)],
    );
    ledger.journal([&m.end, "ENT001", &m.label("Monthly Payroll")], &pay, &liability)?;
    let rent = [("820", d("12000.0")), ("830", d("3500.0"))];
    ledger.journal(
        [&m.end, "ENT001", &m.label("Operating Costs & Rent")],
        &rent,
        &[("100", d("15500.0"))],
    )?;

    let revenue = m.scaled("280000.0");
    let cost = revenue.mul(&d("0.58")).round(2);
    let salary = m.scaled("45000.0");
    let super_amount = super_on(&salary);
    ledger.journal(
        [&m.mid, "ENT002", &m.label("Food Retailing Sales")],
        &[("100", revenue.clone())],
        &[("600", revenue)],
    )?;
    ledger.journal(
        [&m.mid, "ENT002", &m.label("Produce Inventory Purchase")],
        &[("700", cost.clone())],
        &[("100", cost)],
    )?;
    let (pay, liability) = (
        [("800", salary.clone()), ("805", super_amount.clone())],
        [("100", salary), ("330", super_amount)],
    );
    ledger.journal([&m.end, "ENT002", &m.label("Monthly Payroll")], &pay, &liability)?;
    let store = [("820", d("8500.0")), ("810", d("4200.0"))];
    ledger.journal(
        [&m.end, "ENT002", &m.label("Store Operating Costs")],
        &store,
        &[("100", d("12700.0"))],
    )
}

/// Falador Freight (logistics) and Ardougne Holdings Trust (property).
fn freight_and_property(ledger: &mut Ledger, m: &Month) -> Result<(), String> {
    let revenue = m.scaled("190000.0");
    let fuel = revenue.mul(&d("0.28")).round(2);
    let salary = m.scaled("60000.0");
    let super_amount = super_on(&salary);
    ledger.journal(
        [&m.mid, "ENT003", &m.label("Freight Service Revenue")],
        &[("110", revenue.clone())],
        &[("600", revenue)],
    )?;
    let fleet = [("710", fuel.clone()), ("810", d("6500.0"))];
    let paid = [("100", fuel.add(&d("6500.0")))];
    ledger.journal([&m.mid, "ENT003", &m.label("Fleet Fuel & Maintenance")], &fleet, &paid)?;
    let (pay, liability) = (
        [("800", salary.clone()), ("805", super_amount.clone())],
        [("100", salary), ("330", super_amount)],
    );
    ledger.journal([&m.end, "ENT003", &m.label("Driver Payroll")], &pay, &liability)?;
    ledger.journal(
        [&m.end, "ENT003", &m.label("Vehicle Depreciation")],
        &[("890", d("4500.0"))],
        &[("210", d("4500.0"))],
    )?;

    ledger.journal(
        [&m.mid, "ENT004", &m.label("Property Rent Collection")],
        &[("100", d("25000.0"))],
        &[("620", d("25000.0"))],
    )?;
    ledger.journal(
        [&m.end, "ENT004", &m.label("Bank Facility Interest")],
        &[("895", d("5200.0"))],
        &[("100", d("5200.0"))],
    )
}

/// Strictly matched intercompany pairs for elimination: ENT001 charges ENT002 and
/// ENT003 management fees, and ENT003 bills ENT002 for internal freight. Columns:
/// entity, description, debit account, credit account, amount, counterparty.
const INTERCOMPANY: &str = "\
ENT001|Intercompany Management Fees Charged|180|650|10000.0|ENT002
ENT001|Intercompany Management Fees Charged|180|650|6000.0|ENT003
ENT002|Intercompany Management Fee Incurred|880|380|10000.0|ENT001
ENT003|Intercompany Management Fee Incurred|880|380|6000.0|ENT001
ENT003|Internal Logistics Billed to Draynor|180|600|8500.0|ENT002
ENT002|Intercompany Freight Costs|750|380|8500.0|ENT003
";

/// The month's intercompany journals. The second fee keeps its existing identifier
/// from when the counterparties were split.
fn intercompany(ledger: &mut Ledger, m: &Month) -> Result<(), String> {
    for (index, line) in INTERCOMPANY.lines().enumerate() {
        let fields: Vec<&str> = line.split('|').collect();
        let [entity, description, debit, credit, amount, counterparty] = fields[..] else {
            unreachable!("six intercompany columns")
        };
        let key = (index == 1).then(|| format!("ICM3-{}-{:02}", m.year, m.month));
        let (debits, credits) = ([(debit, d(amount))], [(credit, d(amount))]);
        ledger.journal_with([&m.end, entity, description], &debits, &credits, counterparty, key)?;
    }
    Ok(())
}

/// Balanced double-entry journals over 36 months, July 2024 to June 2027.
pub fn general_ledger() -> Result<Table, String> {
    let mut ledger = Ledger {
        rows: Vec::new(),
        journal_id: 1000,
    };
    opening_balances(&mut ledger)?;
    for index in 0..36 {
        let month = Month::new(index);
        advisory_and_produce(&mut ledger, &month)?;
        freight_and_property(&mut ledger, &month)?;
        intercompany(&mut ledger, &month)?;
    }
    Ok(Table {
        header: LEDGER_HEADER.to_vec(),
        rows: ledger.rows,
    })
}

const BUDGET_HEADER: &[&str] = &["PeriodDate", "EntityID", "AccountCode", "BudgetAmount"];

const BUDGET_LINES: &[(&str, &str, &str)] = &[
    ("ENT001", "610", "175000.00"),
    ("ENT001", "800", "78000.00"),
    ("ENT001", "805", "9360.00"),
    ("ENT002", "600", "300000.00"),
    ("ENT002", "700", "170000.00"),
    ("ENT002", "800", "48000.00"),
    ("ENT003", "600", "210000.00"),
    ("ENT003", "710", "58000.00"),
    ("ENT003", "800", "62000.00"),
    ("ENT004", "620", "25000.00"),
    ("ENT004", "895", "5000.00"),
];

/// Monthly budgets for the same 36 months.
pub fn budgets() -> Table {
    let rows = (0..36)
        .flat_map(|index| {
            let month = Month::new(index);
            let period = format!("{}-{:02}-01", month.year, month.month);
            BUDGET_LINES.iter().map(move |(entity, code, amount)| {
                vec![
                    period.clone(),
                    (*entity).to_owned(),
                    (*code).to_owned(),
                    (*amount).to_owned(),
                ]
            })
        })
        .collect();
    Table {
        header: BUDGET_HEADER.to_vec(),
        rows,
    }
}

const PAYROLL_HEADER: &[&str] = &[
    "EventID",
    "EmployeeID",
    "EntityID",
    "EmployeeName",
    "PayDate",
    "GrossEarnings",
    "QualifyingEarnings_CodeQ",
    "SuperLiability_CodeL",
    "SuperFundUSI",
    "SuperFundName",
    "RemittanceDate",
    "FundReceiptDate",
    "StatutoryDueDate",
    "ComplianceStatus",
    "SGC_Shortfall",
    "GIC_NominalInterest",
];

/// (EmployeeID, EntityID, Name, annual salary, weekly, fund USI, fund name).
const EMPLOYEES: &[(&str, &str, &str, &str, bool, &str, &str)] = &[
    (
        "EMP101",
        "ENT001",
        "A. Vance",
        "145000.0",
        false,
        "19323234999001",
        "AustralianSuper",
    ),
    (
        "EMP102",
        "ENT001",
        "B. Sterling",
        "115000.0",
        false,
        "16457520308001",
        "ART Super",
    ),
    (
        "EMP201",
        "ENT002",
        "C. Miller",
        "68000.0",
        true,
        "19323234999001",
        "AustralianSuper",
    ),
    (
        "EMP202",
        "ENT002",
        "D. Chen",
        "72000.0",
        true,
        "60905115063001",
        "Hostplus",
    ),
    (
        "EMP301",
        "ENT003",
        "E. Kowalski",
        "92000.0",
        false,
        "16457520308001",
        "ART Super",
    ),
    (
        "EMP302",
        "ENT003",
        "F. O'Connor",
        "88000.0",
        false,
        "19323234999001",
        "AustralianSuper",
    ),
];

/// One pay event's due date, receipt and charge.
struct Settlement {
    due: Date,
    received: Date,
    status: &'static str,
    charge: Decimal,
    interest: Decimal,
}

/// From 1 July 2026 super is due 7 national business days after payday; before it,
/// on the quarterly date. Most contributions reach the fund 4 business days after
/// payday; every 13th live event simulates a clearing-house delay of 10 business days.
fn settle(rates: &GicRates, event: u32, payday: Date, liability: &Decimal) -> Result<Settlement, String> {
    let live = payday >= Date::new(2026, 7, 1);
    let due = if live {
        add_business_days(payday, 7)?
    } else if payday.month() <= 3 {
        Date::new(2026, 4, 28)
    } else {
        Date::new(2026, 7, 28)
    };
    if !(event.is_multiple_of(13) && live) {
        let received = add_business_days(payday, 4)?;
        return Ok(Settlement {
            due,
            received,
            status: "ON_TIME",
            charge: d("0.0"),
            interest: d("0.0"),
        });
    }
    let received = add_business_days(payday, 10)?;
    // Notional earnings compound daily at each day's own GIC rate (SGAA 1992 s 19A).
    let interest = notional_earnings(rates, liability, due, received);
    // s 18D: a contribution the fund receives before the Commissioner assesses the charge
    // reduces the final shortfall to nil. No earlier assessment is modelled, and the test
    // is strictly received > due so a timely contribution cannot zero a real shortfall.
    let shortfall = if received > due { d("0.0") } else { liability.clone() };
    // Administrative uplift (s 19B) on final shortfalls plus notional earnings; choice
    // loading (s 20A) is nil because every fabricated employee has a nominated fund.
    let uplift = shortfall.add(&interest).mul(&d(ADMIN_UPLIFT_RATE)).round(2);
    let charge = shortfall.add(&interest).add(&uplift).round(2);
    Ok(Settlement {
        due,
        received,
        status: "LATE_BREACH",
        charge,
        interest,
    })
}

/// Payday Super payroll events from January 2026 to June 2027, covering the
/// 1 July 2026 transition.
pub fn payroll(rates: &GicRates) -> Result<Table, String> {
    let mut rows = Vec::new();
    let mut event = 5000;
    for &(employee, entity, name, salary, weekly, usi, fund) in EMPLOYEES {
        let (period, periods, first) = if weekly { (7, 52, 7) } else { (14, 26, 14) };
        let gross = d(salary).div(&Decimal::from_int(periods)).round(2);
        // Qualifying earnings (code Q) and super liability (code L at 12.0%).
        let liability = super_on(&gross);
        let mut payday = Date::new(2026, 1, first);
        while payday <= Date::new(2027, 6, 30) {
            event += 1;
            let settled = settle(rates, event, payday, &liability)?;
            let fields = [
                format!("STP{event}"),
                employee.to_owned(),
                entity.to_owned(),
                name.to_owned(),
                payday.to_string(),
                gross.to_fixed(2),
                gross.to_fixed(2),
                liability.to_fixed(2),
                usi.to_owned(),
                fund.to_owned(),
                payday.add_days(2).to_string(),
                settled.received.to_string(),
                settled.due.to_string(),
                settled.status.to_owned(),
                settled.charge.to_fixed(2),
                settled.interest.to_fixed(2),
            ];
            rows.push(fields.to_vec());
            payday = payday.add_days(period);
        }
    }
    Ok(Table {
        header: PAYROLL_HEADER.to_vec(),
        rows,
    })
}

/// Fabricated sample values invented for this model, not an ATO benchmark
/// distribution or percentile, for ANZSIC 6962 (management advice), 4122 (fresh meat,
/// fish and poultry retailing), 4610 (road freight) and 6712 (non-residential property
/// operators). The rows carry no source date, citation or percentile definition.
const BENCHMARKS: &str = "\
ANZSIC_Code|TurnoverRange|GrossProfitPct_Low|GrossProfitPct_Avg|GrossProfitPct_High|TotalExpensesPct_Avg|RentPct_Avg|MotorVehiclePct_Avg|LabourPct_Avg
6962|$500k-$1m|88.0|92.5|96.0|75.5|4.5|3.2|42.0
6962|$1m-$5m|85.0|90.0|94.0|82.0|5.0|2.5|46.0
4122|$500k-$1m|34.0|39.5|45.0|88.5|5.2|1.8|14.5
4122|$1m-$5m|36.0|41.0|47.0|88.5|4.8|1.2|16.0
4610|$500k-$1m|72.0|78.0|84.0|87.0|2.8|18.5|31.0
4610|$1m-$5m|70.0|76.5|82.0|90.0|2.5|16.0|33.5
6712|$100k-$500k|92.0|96.0|98.5|46.0|0.0|1.5|5.0
";

/// A failure while generating: a refused calculation or a file that could not be written.
#[derive(Debug)]
pub enum Error {
    Calculation(String),
    Io(io::Error),
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Error {
        Error::Io(error)
    }
}

impl From<String> for Error {
    fn from(message: String) -> Error {
        Error::Calculation(message)
    }
}

/// Writes the six fixtures into `directory` in the original order, each file as soon
/// as it is built.
pub fn generate(directory: &Path) -> Result<(), Error> {
    fs::create_dir_all(directory)?;
    let write = |name: &str, table: Table| fs::write(directory.join(name), table.to_csv());
    write("sample-entities.csv", Table::from_text(ENTITIES))?;
    write("sample-chart-of-accounts.csv", Table::from_text(CHART_OF_ACCOUNTS))?;
    write("sample-general-ledger.csv", general_ledger()?)?;
    write("sample-budgets.csv", budgets())?;
    write("sample-payroll-super.csv", payroll(&GicRates::published())?)?;
    write("sample-ato-benchmarks.csv", Table::from_text(BENCHMARKS))?;
    Ok(())
}
