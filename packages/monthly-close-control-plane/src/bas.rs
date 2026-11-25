// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! A drafted BAS workpaper: labels G1, G2, G3, G10, G11, 1A and 1B from a
//! ledger export placed by a supplied tax-code map, and W1 and W2 from a
//! payroll register, each total naming the rows behind it. It lodges nothing.
use crate::{
    Result,
    decimal::{self, Decimal},
    duplicates, engine,
    isodate::Date,
    loader::{self, SourceSnapshot},
    models::{ExceptionItem, GstClass, GstSettlements, Status, TrialBalanceRow},
    payroll,
    pyjson::Json,
    pytext, reconciliation,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const COLUMNS: [&str; 9] = [
    "Tenant",
    "LineID",
    "Date",
    "PaidDate",
    "AccountID",
    "TaxCode",
    "Gross",
    "GST",
    "Currency",
];
pub const MAP_COLUMNS: [&str; 3] = ["TaxCode", "AccountID", "Treatment"];
pub const BOUNDARY: &str = "A draft for a reviewer; it lodges nothing and is not a BAS. Each label is the sum of the rows it names, placed by the supplied tax-code map. The map, each row's coding, and anything the labels cannot show (adjustments, instalments, private use, the financial acquisitions threshold) are the reviewer's to confirm.";
/// The review heuristic `review --gst-mapping` applies by default.
const GST_ABSOLUTE_TOLERANCE: &str = "1.00";
const GST_PERCENTAGE_TOLERANCE: &str = "0.01";

/// Where a supplied treatment sends a row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Treatment {
    TaxableSale,
    ExportSale,
    GstFreeSale,
    InputTaxedSale,
    /// A purchase: capital or not, and how its GST is claimed.
    Purchase(bool, Credit),
    Excluded,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Credit {
    /// GST in the price, claimed at 1B.
    Creditable,
    /// No GST in the price: GST-free, input-taxed or from an unregistered supplier.
    NoGst,
    /// GST in the price but not claimed: private use, non-deductible or for
    /// making input-taxed sales.
    NotCreditable,
}

const TREATMENTS: [(&str, Treatment); 11] = [
    ("taxable_sale", Treatment::TaxableSale),
    ("export_sale", Treatment::ExportSale),
    ("gst_free_sale", Treatment::GstFreeSale),
    ("input_taxed_sale", Treatment::InputTaxedSale),
    (
        "capital_creditable",
        Treatment::Purchase(true, Credit::Creditable),
    ),
    ("capital_no_gst", Treatment::Purchase(true, Credit::NoGst)),
    (
        "capital_not_creditable",
        Treatment::Purchase(true, Credit::NotCreditable),
    ),
    (
        "non_capital_creditable",
        Treatment::Purchase(false, Credit::Creditable),
    ),
    (
        "non_capital_no_gst",
        Treatment::Purchase(false, Credit::NoGst),
    ),
    (
        "non_capital_not_creditable",
        Treatment::Purchase(false, Credit::NotCreditable),
    ),
    ("bas_excluded", Treatment::Excluded),
];

impl Treatment {
    /// The G labels its gross amount goes to.
    fn labels(self) -> &'static [&'static str] {
        match self {
            Self::TaxableSale | Self::InputTaxedSale => &["G1"],
            Self::ExportSale => &["G1", "G2"],
            Self::GstFreeSale => &["G1", "G3"],
            Self::Purchase(true, _) => &["G10"],
            Self::Purchase(false, _) => &["G11"],
            Self::Excluded => &[],
        }
    }

    /// Whether the row's GST is reported, and whether it may carry any.
    fn gst(self) -> (Option<&'static str>, bool) {
        match self {
            Self::TaxableSale => (Some("1A"), true),
            Self::Purchase(_, Credit::Creditable) => (Some("1B"), true),
            Self::Purchase(_, Credit::NotCreditable) => (None, true),
            _ => (None, false),
        }
    }
}

const LABELS: [(&str, &str); 9] = [
    ("G1", "Total sales, including GST"),
    ("G2", "Export sales"),
    ("G3", "Other GST-free sales"),
    ("G10", "Capital purchases, including GST"),
    ("G11", "Non-capital purchases, including GST"),
    ("1A", "GST on sales"),
    ("1B", "GST on purchases"),
    ("W1", "Total salary, wages and other payments"),
    ("W2", "Amounts withheld from payments shown at W1"),
];

struct Line {
    id: String,
    account: String,
    tax_code: String,
    gross: Decimal,
    gst: Decimal,
    /// The date the row falls in a period on the declared basis; None when
    /// unpaid on the cash basis.
    placed: Option<Date>,
}

fn load(source: &SourceSnapshot, currency: &str, cash: bool) -> Result<(String, Vec<Line>)> {
    let path = crate::pypath::text(&source.path);
    let mut tenant = None;
    let mut lines = Vec::new();
    let mut seen = BTreeSet::new();
    for row in loader::csv_rows(source, &COLUMNS, "Ledger")? {
        let row = row?;
        let text = |field| row.text(field, field == "PaidDate");
        let name = text("Tenant")?;
        if *tenant.get_or_insert_with(|| name.clone()) != name {
            return Err("A ledger must belong to one tenant; draft each tenant separately.".into());
        }
        let refuse = |what: String| Err(format!("{path}: row {} {what}", row.number));
        if text("Currency")? != currency {
            return refuse(format!(
                "is not in the declared --currency {}.",
                pytext::repr(currency)
            ));
        }
        let id = text("LineID")?;
        if !seen.insert(id.clone()) {
            return refuse(format!("repeats LineID {}.", pytext::repr(&id)));
        }
        let date = row.date("Date")?;
        let paid = if text("PaidDate")?.is_empty() {
            None
        } else {
            Some(row.date("PaidDate")?)
        };
        let money = |field| reconciliation::money(&text(field)?, field, &source.path, row.number);
        lines.push(Line {
            id,
            account: text("AccountID")?,
            tax_code: text("TaxCode")?,
            gross: money("Gross")?,
            gst: money("GST")?,
            placed: if cash {
                paid
            } else {
                Some(paid.map_or(date, |paid| paid.min(date)))
            },
        });
    }
    match tenant {
        Some(tenant) => Ok((tenant, lines)),
        None => Err(format!("{path}: no ledger rows were supplied.")),
    }
}

/// The map from tax code, and optionally account, to a treatment. A row for
/// one account overrides the tax code's row with a blank `AccountID`.
type TaxCodeMap = BTreeMap<(String, String), (&'static str, Treatment)>;

fn load_map(source: &SourceSnapshot) -> Result<TaxCodeMap> {
    let path = crate::pypath::text(&source.path);
    let mut map = BTreeMap::new();
    for row in loader::csv_rows(source, &MAP_COLUMNS, "Tax-code map")? {
        let row = row?;
        let refuse = |what: String| Err(format!("{path}: row {} {what}", row.number));
        let code = row.text("TaxCode", false)?;
        let account = row.text("AccountID", true)?;
        let named = row.text("Treatment", false)?;
        let Some(&(name, treatment)) = TREATMENTS.iter().find(|(name, _)| *name == named) else {
            return refuse(format!(
                "has Treatment {}, which is not one of {}.",
                pytext::repr(&named),
                TREATMENTS.map(|(name, _)| name).join(", ")
            ));
        };
        if map.insert((code, account), (name, treatment)).is_some() {
            return refuse("repeats a TaxCode and AccountID already mapped.".into());
        }
    }
    if map.is_empty() {
        return Err(format!("{path}: no tax codes were mapped."));
    }
    Ok(map)
}

/// Why a row in the period cannot be placed, if it cannot.
fn unplaceable(line: &Line, treatment: Option<(&str, Treatment)>) -> Option<String> {
    let Some((name, treatment)) = treatment else {
        return Some(format!(
            "the tax-code map has no row for TaxCode {} on AccountID {} or on any account",
            pytext::repr(&line.tax_code),
            pytext::repr(&line.account)
        ));
    };
    let zero = Decimal::zero();
    let sign = |value: &Decimal| value.cmp_value(&zero);
    if !line.gst.is_zero() && !treatment.gst().1 {
        return Some(format!(
            "carries GST of {}, but {name} takes none",
            reconciliation::fixed(&line.gst)
        ));
    }
    if !line.gst.is_zero()
        && (sign(&line.gst) != sign(&line.gross)
            || line.gst.abs().cmp_value(&line.gross.abs()).is_gt())
    {
        return Some(format!(
            "has GST {} that is not part of its Gross {}",
            reconciliation::fixed(&line.gst),
            reconciliation::fixed(&line.gross)
        ));
    }
    None
}

struct Label {
    amount: Decimal,
    rows: Vec<String>,
}

impl Default for Label {
    fn default() -> Self {
        Self {
            amount: Decimal::zero(),
            rows: Vec::new(),
        }
    }
}

impl Label {
    fn add(&mut self, amount: &Decimal, row: String) -> Result<()> {
        self.amount = decimal_total([&self.amount, amount])?;
        self.rows.push(row);
        Ok(())
    }
}

fn decimal_total<'a>(values: impl IntoIterator<Item = &'a Decimal>) -> Result<Decimal> {
    engine::exact_total(values, "Period", "BAS label")
}

/// Whole dollars, cents dropped, as the BAS takes them; None when negative.
fn whole_dollars(value: &Decimal) -> Option<String> {
    if value.cmp_value(&Decimal::zero()).is_lt() {
        return None;
    }
    let text = reconciliation::fixed(value);
    Some(text.split('.').next().unwrap_or_default().to_owned())
}

/// Draft the BAS workpaper for the period `start` to `end`.
pub struct BasConfig<'a> {
    pub ledger: &'a Path,
    pub tax_codes: &'a Path,
    pub payroll: Option<&'a Path>,
    pub currency: &'a str,
    pub period_start: &'a str,
    pub period_end: &'a str,
    pub basis: &'a str,
}

pub fn bas_workpaper(config: &BasConfig) -> Result<Json> {
    let (start, end) =
        duplicates::period_options(config.currency, config.period_start, config.period_end)?;
    let cash = match config.basis {
        "cash" => true,
        "accruals" => false,
        _ => return Err("--basis must be accruals or cash.".into()),
    };
    let ledger = SourceSnapshot::capture(config.ledger, "Ledger")?;
    let map = SourceSnapshot::capture(config.tax_codes, "Tax-code map")?;
    let payroll = match config.payroll {
        Some(path) => Some(SourceSnapshot::capture(path, "Payroll register")?),
        None => None,
    };
    let sources = Sources {
        ledger: &ledger,
        map: &map,
        payroll: payroll.as_ref(),
    };
    decimal::with_precision(40, || draft(&sources, config.currency, (start, end), cash))
}

struct Sources<'a> {
    ledger: &'a SourceSnapshot,
    map: &'a SourceSnapshot,
    payroll: Option<&'a SourceSnapshot>,
}

/// The labels one ledger fills, the rows that could not be placed, and the
/// GST-exclusive bases behind 1A and 1B.
#[derive(Default)]
struct Placed {
    labels: BTreeMap<&'static str, Label>,
    unplaced: Vec<Json>,
    excluded: Vec<Json>,
    sales_base: Vec<Decimal>,
    purchases_base: Vec<Decimal>,
    in_period: i64,
    outside: i64,
    unpaid: i64,
}

impl Placed {
    fn add(&mut self, line: &Line, treatment: Treatment) -> Result<()> {
        if treatment == Treatment::Excluded {
            self.excluded.push(Json::str(&line.id));
        }
        for label in treatment.labels() {
            let entry = self.labels.entry(label).or_default();
            entry.add(&line.gross, line.id.clone())?;
        }
        if let (Some(label), _) = treatment.gst() {
            let entry = self.labels.entry(label).or_default();
            entry.add(&line.gst, line.id.clone())?;
            let base = line.gross.sub(&line.gst).value;
            if label == "1A" {
                self.sales_base.push(base);
            } else {
                self.purchases_base.push(base);
            }
        }
        Ok(())
    }
}

/// The treatment for a row: its tax code on its account, else on any account.
fn treatment_for(map: &TaxCodeMap, line: &Line) -> Option<(&'static str, Treatment)> {
    map.get(&(line.tax_code.clone(), line.account.clone()))
        .or_else(|| map.get(&(line.tax_code.clone(), String::new())))
        .copied()
}

fn unplaced_row(line: &Line, reason: &str) -> Json {
    Json::object([
        ("line_id", Json::str(&line.id)),
        ("account_id", Json::str(&line.account)),
        ("tax_code", Json::str(&line.tax_code)),
        ("gross", Json::str(&reconciliation::fixed(&line.gross))),
        ("gst", Json::str(&reconciliation::fixed(&line.gst))),
        ("reason", Json::str(reason)),
    ])
}

fn place(lines: &[Line], map: &TaxCodeMap, (start, end): (Date, Date)) -> Result<Placed> {
    let mut placed = Placed::default();
    for line in lines {
        let Some(date) = line.placed else {
            placed.unpaid += 1;
            continue;
        };
        if date < start || date > end {
            placed.outside += 1;
            continue;
        }
        placed.in_period += 1;
        let treatment = treatment_for(map, line);
        match (unplaceable(line, treatment), treatment) {
            (None, Some((_, treatment))) => placed.add(line, treatment)?,
            (reason, _) => placed
                .unplaced
                .push(unplaced_row(line, &reason.unwrap_or_default())),
        }
    }
    Ok(placed)
}

/// W1 and W2 from the register's pay lines paid in the period.
fn add_payroll(
    source: &SourceSnapshot,
    tenant: &str,
    (start, end): (Date, Date),
    labels: &mut BTreeMap<&'static str, Label>,
) -> Result<()> {
    let (payroll_tenant, lines) = payroll::load(source)?;
    if payroll_tenant != tenant {
        return Err("The payroll register belongs to another tenant than the ledger.".into());
    }
    for line in lines
        .iter()
        .filter(|line| start <= line.paid && line.paid <= end)
    {
        let row = format!("payroll row {}", line.row);
        labels
            .entry("W1")
            .or_default()
            .add(&line.earnings, row.clone())?;
        labels.entry("W2").or_default().add(&line.withheld, row)?;
    }
    Ok(())
}

/// Each drafted label with the rows behind it; a negative total adds a finding.
fn label_entries(
    labels: &BTreeMap<&str, Label>,
    payroll: bool,
    findings: &mut Vec<Json>,
) -> Vec<Json> {
    let empty = Label::default();
    let mut drafted = Vec::new();
    for (label, description) in LABELS {
        let from_payroll = label.starts_with('W');
        if from_payroll && !payroll {
            continue;
        }
        let entry = labels.get(label).unwrap_or(&empty);
        let amount = reconciliation::fixed(&entry.amount);
        let whole = whole_dollars(&entry.amount);
        if whole.is_none() {
            findings.push(Json::object([
                ("code", Json::str("NEGATIVE_LABEL")),
                ("label", Json::str(label)),
                ("amount", Json::str(&amount)),
                ("reason", Json::str(NEGATIVE_REASON)),
            ]));
        }
        let rows = entry.rows.iter().map(|row| Json::str(row)).collect();
        drafted.push(Json::object([
            ("label", Json::str(label)),
            ("description", Json::str(description)),
            ("amount", Json::str(&amount)),
            (
                "whole_dollars",
                whole.map_or(Json::Null, |text| Json::str(&text)),
            ),
            (
                "source",
                Json::str(if from_payroll { "payroll" } else { "ledger" }),
            ),
            ("rows", Json::Array(rows)),
        ]));
    }
    drafted
}

const NEGATIVE_REASON: &str = "The BAS takes no negative figure. Credit notes or refunds larger than the period's own amounts are usually an adjustment, which this draft does not place.";

fn draft(sources: &Sources, currency: &str, period: (Date, Date), cash: bool) -> Result<Json> {
    let (tenant, lines) = load(sources.ledger, currency, cash)?;
    let map = load_map(sources.map)?;
    let mut placed = place(&lines, &map, period)?;
    let mut digests = vec![
        ("ledger".into(), Json::str(&sources.ledger.sha256)),
        ("tax_codes".into(), Json::str(&sources.map.sha256)),
    ];
    let mut not_drafted = Vec::new();
    if let Some(source) = sources.payroll {
        add_payroll(source, &tenant, period, &mut placed.labels)?;
        digests.push(("payroll".into(), Json::str(&source.sha256)));
    } else {
        for label in ["W1", "W2"] {
            not_drafted.push(Json::object([
                ("label", Json::str(label)),
                ("reason", Json::str("no --payroll register was supplied")),
            ]));
        }
    }
    let mut findings = reasonableness(&tenant, &placed)?;
    let drafted = label_entries(&placed.labels, sources.payroll.is_some(), &mut findings);
    let status = if !placed.unplaced.is_empty() {
        Status::Blocked
    } else if placed.in_period == 0 || !findings.is_empty() {
        Status::Review
    } else {
        Status::Pass
    };
    let mut result = duplicates::period_result(false, &tenant, currency, period, BOUNDARY);
    for (key, value) in [
        ("status", Json::str(status.as_str())),
        ("basis", Json::str(if cash { "cash" } else { "accruals" })),
        ("amounts_include_gst", Json::Bool(true)),
        ("source_sha256", Json::Object(digests)),
        ("row_count", Json::int(lines.len() as i64)),
        ("rows_in_period", Json::int(placed.in_period)),
        ("rows_outside_period", Json::int(placed.outside)),
        ("rows_unpaid", Json::int(placed.unpaid)),
        ("labels", Json::Array(drafted)),
        ("labels_not_drafted", Json::Array(not_drafted)),
        ("bas_excluded_rows", Json::Array(placed.excluded)),
        ("unplaced", Json::Array(placed.unplaced)),
        ("findings", Json::Array(findings)),
    ] {
        result.set(key, value);
    }
    Ok(result)
}

/// The 4 trial-balance rows the GST reasonableness control reads: the
/// GST-exclusive taxable sales and creditable purchases, and 1A and 1B as GST
/// collected and GST paid.
fn reasonableness_rows(tenant: &str, placed: &Placed) -> Result<Vec<TrialBalanceRow>> {
    let amount = |label| {
        placed
            .labels
            .get(label)
            .map_or_else(Decimal::zero, |entry: &Label| entry.amount.clone())
    };
    let row = |id: &str, debit: Decimal, credit: Decimal| TrialBalanceRow {
        report_date: Date::new(2000, 1, 1).expect("valid date"),
        tenant: tenant.to_owned(),
        section: String::new(),
        account_id: id.to_owned(),
        account_name: String::new(),
        account_code: String::new(),
        debit,
        credit,
        ytd_debit: Decimal::zero(),
        ytd_credit: Decimal::zero(),
    };
    let zero = Decimal::zero;
    Ok(vec![
        row("sales", zero(), decimal_total(&placed.sales_base)?),
        row("purchases", decimal_total(&placed.purchases_base)?, zero()),
        row("1A", zero(), amount("1A")),
        row("1B", amount("1B"), zero()),
    ])
}

/// 1A and 1B against 1/10 of their GST-exclusive bases, through the GST
/// reasonableness control `review --gst-mapping` runs, with its default
/// review heuristic and each side compared on its own.
fn reasonableness(tenant: &str, placed: &Placed) -> Result<Vec<Json>> {
    let rows = reasonableness_rows(tenant, placed)?;
    let mapping = BTreeMap::from([
        ("sales".to_owned(), GstClass::TaxableSales),
        ("purchases".to_owned(), GstClass::CreditablePurchases),
        ("1A".to_owned(), GstClass::GstCollected),
        ("1B".to_owned(), GstClass::GstPaid),
    ]);
    let none = GstSettlements {
        sided: true,
        rows: Vec::new(),
    };
    let tolerance = |text| Decimal::parse(text).expect("literal decimal");
    let (items, _) = engine::gst_reasonableness_exceptions(
        &rows,
        &mapping,
        Ok(&none),
        &tolerance(GST_ABSOLUTE_TOLERANCE),
        &tolerance(GST_PERCENTAGE_TOLERANCE),
    )?;
    Ok(items.iter().map(reasonableness_finding).collect())
}

fn reasonableness_finding(item: &ExceptionItem) -> Json {
    let (label, base) = if item.reason.starts_with("GST collected") {
        ("1A", "taxable sales")
    } else if item.reason.starts_with("GST paid") {
        ("1B", "creditable purchases")
    } else {
        ("1A less 1B", "taxable sales less creditable purchases")
    };
    let figure = |value: &Option<Decimal>| {
        Json::str(&reconciliation::fixed(
            value.as_ref().expect("set by the control"),
        ))
    };
    Json::object([
        ("code", Json::str("GST_REASONABLENESS")),
        ("label", Json::str(label)),
        ("drafted", figure(&item.current_value)),
        ("expected", figure(&item.prior_value)),
        ("difference", figure(&item.difference)),
        ("tolerance", figure(&item.threshold)),
        (
            "reason",
            Json::str(&format!(
                "{label} differs from 1/10 of the GST-exclusive {base} by more than the review heuristic (1.00 plus 1% of the expected GST), which is not a rounding bound. Check the tax codes and GST amounts on the rows behind it."
            )),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pyjson;

    const HEADER: &str = "Tenant,LineID,Date,PaidDate,AccountID,TaxCode,Gross,GST,Currency\n";
    const MAP: &str = "TaxCode,AccountID,Treatment\nS,,taxable_sale\nF,,gst_free_sale\nX,,export_sale\nP,,non_capital_creditable\nP,9,capital_creditable\nN,,non_capital_not_creditable\nE,,bas_excluded\n";
    const PAYROLL: &str = "Tenant,EmployeeID,PayDate,Frequency,Scale,Adjusted,Earnings,Withheld,QualifyingEarnings,Super,FundReceived\nT,E1,2026-07-09,WEEKLY,2,N,1333.45,245.00,1333.45,160.01,2026-07-14\nT,E1,2026-10-01,WEEKLY,2,N,1333.45,245.00,1333.45,160.01,2026-10-06\n";
    /// `(line, date, paid date, account, tax code, gross, GST)`.
    type Row<'a> = (
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
        &'a str,
    );

    fn ledger(rows: &[Row]) -> String {
        let mut text = HEADER.to_owned();
        for (id, date, paid, account, code, gross, gst) in rows {
            text += &format!("T,{id},{date},{paid},{account},{code},{gross},{gst},AUD\n");
        }
        text
    }

    fn run(ledger: &str, map: &str, payroll: Option<&str>, cash: bool) -> Result<Json> {
        let snapshot =
            |name: &str, text: &str| SourceSnapshot::new(Path::new(name), text.as_bytes().to_vec());
        let (ledger, map) = (snapshot("ledger.csv", ledger), snapshot("map.csv", map));
        let payroll = payroll.map(|text| snapshot("payroll.csv", text));
        let sources = Sources {
            ledger: &ledger,
            map: &map,
            payroll: payroll.as_ref(),
        };
        let period = (
            Date::new(2026, 7, 1).unwrap(),
            Date::new(2026, 9, 30).unwrap(),
        );
        decimal::with_precision(40, || draft(&sources, "AUD", period, cash))
    }

    fn text(value: &Json) -> String {
        value.as_pystr().unwrap().to_utf8().unwrap()
    }

    /// Each drafted label as `label amount whole [rows]`.
    fn labels(result: &Json) -> Vec<String> {
        let items = result.at("labels").as_array().unwrap();
        items
            .iter()
            .map(|item| {
                let rows: Vec<_> = item
                    .at("rows")
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(text)
                    .collect();
                format!(
                    "{} {} {} [{}]",
                    text(item.at("label")),
                    text(item.at("amount")),
                    pyjson::canonical(item.at("whole_dollars")),
                    rows.join(" ")
                )
            })
            .collect()
    }

    fn codes(result: &Json, key: &str, field: &str) -> Vec<String> {
        let items = result.at(key).as_array().unwrap();
        items.iter().map(|item| text(item.at(field))).collect()
    }

    const SALE: Row = ("A", "2026-07-03", "", "1", "S", "1100.00", "100.00");
    const BUY: Row = ("B", "2026-07-04", "", "2", "P", "330.00", "30.00");

    #[test]
    fn each_label_names_its_rows_and_a_clean_draft_passes() {
        let rows = [
            SALE,
            BUY,
            ("C", "2026-07-05", "", "9", "P", "2200.55", "200.05"),
            ("D", "2026-07-06", "", "3", "X", "500.00", "0.00"),
            ("F", "2026-07-07", "", "3", "F", "40.00", "0.00"),
            ("G", "2026-07-08", "", "4", "N", "110.00", "10.00"),
            ("H", "2026-07-09", "", "5", "E", "999.00", "0.00"),
        ];
        let result = run(&ledger(&rows), MAP, None, false).unwrap();
        assert!(result.at("status").is_str("PASS"), "{result:?}");
        assert_eq!(
            labels(&result),
            [
                "G1 1640.00 \"1640\" [A D F]",
                "G2 500.00 \"500\" [D]",
                "G3 40.00 \"40\" [F]",
                "G10 2200.55 \"2200\" [C]",
                "G11 440.00 \"440\" [B G]",
                "1A 100.00 \"100\" [A]",
                "1B 230.05 \"230\" [B C]",
            ]
        );
        assert_eq!(pyjson::canonical(result.at("bas_excluded_rows")), "[\"H\"]");
        assert_eq!(codes(&result, "labels_not_drafted", "label"), ["W1", "W2"]);
    }

    #[test]
    fn rows_are_placed_by_the_declared_basis() {
        let rows = [
            ("A", "2026-06-28", "2026-07-02", "1", "S", "110.00", "10.00"),
            ("B", "2026-07-02", "", "1", "S", "220.00", "20.00"),
            ("C", "2026-09-29", "2026-10-02", "1", "S", "330.00", "30.00"),
            ("D", "2026-10-01", "", "1", "S", "440.00", "40.00"),
        ];
        let accruals = run(&ledger(&rows), MAP, None, false).unwrap();
        assert_eq!(labels(&accruals)[0], "G1 550.00 \"550\" [B C]");
        assert_eq!(pyjson::canonical(accruals.at("rows_outside_period")), "2");
        let cash = run(&ledger(&rows), MAP, None, true).unwrap();
        assert_eq!(labels(&cash)[0], "G1 110.00 \"110\" [A]");
        assert_eq!(pyjson::canonical(cash.at("rows_unpaid")), "2");
        assert_eq!(pyjson::canonical(cash.at("rows_outside_period")), "1");
    }

    #[test]
    fn rows_that_cannot_be_placed_block_the_draft() {
        let rows = [
            SALE,
            ("U", "2026-07-03", "", "1", "Z", "10.00", "1.00"),
            ("V", "2026-07-03", "", "1", "F", "10.00", "1.00"),
            ("W", "2026-07-03", "", "1", "S", "10.00", "-1.00"),
            ("Y", "2026-07-03", "", "1", "S", "10.00", "11.00"),
        ];
        let result = run(&ledger(&rows), MAP, None, false).unwrap();
        assert!(result.at("status").is_str("BLOCKED"));
        let reasons = codes(&result, "unplaced", "reason");
        assert_eq!(codes(&result, "unplaced", "line_id"), ["U", "V", "W", "Y"]);
        assert!(reasons[0].contains("no row for TaxCode 'Z'"), "{reasons:?}");
        assert!(
            reasons[1].contains("gst_free_sale takes none"),
            "{reasons:?}"
        );
        assert!(reasons[2].contains("not part of its Gross"), "{reasons:?}");
        assert!(reasons[3].contains("not part of its Gross"), "{reasons:?}");
    }

    #[test]
    fn gst_away_from_one_tenth_of_its_base_needs_review() {
        let rows = [
            ("A", "2026-07-03", "", "1", "S", "1080.00", "80.00"),
            ("B", "2026-07-04", "", "2", "P", "350.00", "50.00"),
        ];
        let result = run(&ledger(&rows), MAP, None, false).unwrap();
        assert!(result.at("status").is_str("REVIEW"));
        assert_eq!(
            codes(&result, "findings", "label"),
            ["1A less 1B", "1A", "1B"]
        );
        let first = &result.at("findings").as_array().unwrap()[1];
        assert_eq!(text(first.at("drafted")), "80.00");
        assert_eq!(text(first.at("expected")), "100.00");
        assert_eq!(text(first.at("tolerance")), "2.00");
    }

    #[test]
    fn a_negative_label_needs_review_and_has_no_whole_dollars() {
        let rows = [("A", "2026-07-03", "", "1", "S", "-110.00", "-10.00")];
        let result = run(&ledger(&rows), MAP, None, false).unwrap();
        assert!(result.at("status").is_str("REVIEW"));
        assert_eq!(labels(&result)[0], "G1 -110.00 null [A]");
        assert_eq!(
            codes(&result, "findings", "code"),
            ["NEGATIVE_LABEL", "NEGATIVE_LABEL"]
        );
    }

    #[test]
    fn a_ledger_with_nothing_in_the_period_needs_review() {
        let rows = [("A", "2026-06-03", "", "1", "S", "110.00", "10.00")];
        let result = run(&ledger(&rows), MAP, None, false).unwrap();
        assert!(result.at("status").is_str("REVIEW"));
        assert!(result.at("findings").as_array().unwrap().is_empty());
    }

    #[test]
    fn a_payroll_register_drafts_w1_and_w2() {
        let result = run(&ledger(&[SALE, BUY]), MAP, Some(PAYROLL), false).unwrap();
        assert!(result.at("status").is_str("PASS"));
        let drafted = labels(&result);
        assert_eq!(drafted[7], "W1 1333.45 \"1333\" [payroll row 2]");
        assert_eq!(drafted[8], "W2 245.00 \"245\" [payroll row 2]");
        assert!(
            result
                .at("labels_not_drafted")
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(result.at("source_sha256").get("payroll").is_some());
        let other = PAYROLL.replace("\nT,", "\nU,");
        let error = run(&ledger(&[SALE]), MAP, Some(&other), false).unwrap_err();
        assert!(error.contains("another tenant"), "{error}");
    }

    #[test]
    fn malformed_ledgers_and_maps_are_refused() {
        let refused = |ledger: &str, map: &str, message: &str| {
            let error = run(ledger, map, None, false).unwrap_err();
            assert!(error.contains(message), "{error}");
        };
        let one = ledger(&[SALE]);
        refused(HEADER, MAP, "no ledger rows");
        refused(&ledger(&[SALE, SALE]), MAP, "repeats LineID 'A'");
        refused(&one.replace(",AUD\n", ",NZD\n"), MAP, "--currency 'AUD'");
        refused(
            &(one.clone() + "Q,B,2026-07-03,,1,S,1.00,0.00,AUD\n"),
            MAP,
            "one tenant",
        );
        refused(
            &one.replace("1100.00", "1100.001"),
            MAP,
            "two decimal places",
        );
        refused(&one.replace(",1,S,", ",,S,"), MAP, "AccountID");
        refused(&one, "TaxCode,AccountID,Treatment\n", "no tax codes");
        refused(
            &one,
            &(MAP.to_owned() + "S,,gst_free_sale\n"),
            "repeats a TaxCode",
        );
        refused(
            &one,
            "TaxCode,AccountID,Treatment\nS,,taxable\n",
            "'taxable', which is not one of",
        );
    }

    #[test]
    fn options_are_checked_before_reading() {
        let missing = Path::new("absent.csv");
        for (currency, start, basis, message) in [
            ("aud", "2026-07-01", "accruals", "three-letter currency"),
            ("AUD", "2026-10-01", "accruals", "must not follow"),
            (
                "AUD",
                "2026-07-01",
                "payments",
                "--basis must be accruals or cash",
            ),
            ("AUD", "2026-07-01", "cash", "absent.csv"),
        ] {
            let config = BasConfig {
                ledger: missing,
                tax_codes: missing,
                payroll: Some(missing),
                currency,
                period_start: start,
                period_end: "2026-09-30",
                basis,
            };
            let error = bas_workpaper(&config).unwrap_err();
            assert!(error.contains(message), "{error}");
        }
    }
}
