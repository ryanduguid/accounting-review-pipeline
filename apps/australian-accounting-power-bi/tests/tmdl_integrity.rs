//! TMDL definitions, star schema relationships and measure hygiene.

mod common;

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use common::*;
use regex::Regex;

/// The star schema drawn by README.md and docs/data-model.md, as (fromColumn, toColumn) pairs.
const DOCUMENTED_RELATIONSHIPS: [(&str, &str); 12] = [
    ("Review_Evidence.ExceptionKey", "Review_Exception.ExceptionKey"),
    ("Fact_GeneralLedger.AccountCode", "Dim_Account.AccountCode"),
    ("Fact_GeneralLedger.EntityID", "Dim_Entity.EntityID"),
    ("Fact_GeneralLedger.PostingDate", "Dim_Date.Date"),
    ("Fact_Budget.AccountCode", "Dim_Account.AccountCode"),
    ("Fact_Budget.EntityID", "Dim_Entity.EntityID"),
    ("Fact_Budget.PeriodDate", "Dim_Date.Date"),
    ("Fact_PayrollSuper.EntityID", "Dim_Entity.EntityID"),
    ("Fact_PayrollSuper.PayDate", "Dim_Date.Date"),
    ("Fact_PayrollSuper.EmployeeID", "Dim_Employee.EmployeeID"),
    ("Dim_Entity.ANZSIC_Code", "Dim_ANZSIC.ANZSIC_Code"),
    ("Fact_ATOBenchmark.ANZSIC_Code", "Dim_ANZSIC.ANZSIC_Code"),
];

fn relationships_text() -> String {
    read(&definition().join("relationships.tmdl"))
}

#[test]
fn review_account_mapping_matches_the_pinned_case() {
    let case = read_json(&samples().join("shared-review-case.json"));
    let text = read(&tables_dir().join("Review_Evidence.tmdl"));
    let mapping = text
        .split_once("AccountMap = #table")
        .unwrap()
        .1
        .split_once("Run = Review_Run")
        .unwrap()
        .0;
    let pattern = Regex::new(r#"\{"([^"]+)", "([^"]+)"\}"#).unwrap();
    let pairs: Vec<_> = pattern
        .captures_iter(mapping)
        .map(|pair| (pair[1].to_owned(), serde_json::Value::String(pair[2].to_owned())))
        .collect();
    assert_eq!(pairs.len(), case["account_ids"].as_object().unwrap().len());
    let pairs: serde_json::Map<_, _> = pairs.into_iter().collect();
    assert_eq!(serde_json::Value::Object(pairs), case["account_ids"]);
}

#[test]
fn review_island_has_only_its_own_single_direction_relationship_and_text_amounts() {
    let text = relationships_text();
    let selected: Vec<_> = text
        .split("relationship ")
        .filter(|block| block.contains("Review_"))
        .collect();
    assert_eq!(selected.len(), 1);
    assert!(selected[0].contains("fromColumn: Review_Evidence.ExceptionKey"));
    assert!(selected[0].contains("toColumn: Review_Exception.ExceptionKey"));
    assert!(!selected[0].contains("BothDirections"));
    for path in table_files()
        .into_iter()
        .filter(|path| file_name(path).starts_with("Review_"))
    {
        let text = read(&path);
        assert!(!text.contains("dataType: double"), "{}", path.display());
        assert!(!text.contains("SUM("), "{}", path.display());
    }
}

fn declared_edges() -> BTreeSet<(String, String)> {
    let pattern = Regex::new(r"fromColumn:\s+([\w\.]+)\s+toColumn:\s+([\w\.]+)").unwrap();
    pattern
        .captures_iter(&relationships_text())
        .map(|c| (c[1].to_owned(), c[2].to_owned()))
        .collect()
}

fn documented() -> BTreeSet<(String, String)> {
    DOCUMENTED_RELATIONSHIPS
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

/// Every `one ||--o{ many : "key"` edge in a document's mermaid ER diagram.
fn mermaid_er_edges(document: &Path) -> BTreeSet<(String, String, String)> {
    let diagram = Regex::new(r"(?s)```mermaid\s*\nerDiagram\n(.*?)```").unwrap();
    let text = read(document);
    let Some(found) = diagram.captures(&text) else {
        return BTreeSet::new();
    };
    let edge = Regex::new(r#"(\w+)\s+\|\|--o\{\s+(\w+)\s*:\s*"([^"]+)""#).unwrap();
    edge.captures_iter(&found[1])
        .map(|c| (c[1].to_owned(), c[2].to_owned(), c[3].to_owned()))
        .collect()
}

fn general_ledger() -> HashMap<String, String> {
    measure_expressions(&tables_dir().join("Fact_GeneralLedger.tmdl"))
}

#[test]
fn project_path_graph_uses_one_canonical_identifier() {
    let project = read_json(&app().join(format!("{NAME}.pbip")));
    let binding = read_json(&report().join("definition.pbir"));
    let database = read(&definition().join("database.tmdl"));
    assert_eq!(
        *project.at("artifacts").at(0).at("report").at("path"),
        format!("{NAME}.Report")
    );
    assert_eq!(
        *binding.at("datasetReference").at("byPath").at("path"),
        format!("../{NAME}.SemanticModel")
    );
    assert!(database.contains(&format!("database '{NAME}'")));
    let level = Regex::new(r"compatibilityLevel:\s*(\d+)")
        .unwrap()
        .captures(&database)
        .expect("compatibilityLevel");
    // Desktop saves this project at 1606 and rejects a subsequent downgrade.
    assert!(level[1].parse::<u64>().unwrap() >= 1606);
}

#[test]
fn tables_directory_exists_and_populated() {
    assert!(tables_dir().is_dir(), "TMDL tables directory must exist");
    assert!(table_files().len() >= 8, "Expected at least 8 TMDL table definitions");
}

#[test]
fn relationships_defined_and_valid() {
    let path = definition().join("relationships.tmdl");
    assert!(path.is_file(), "relationships.tmdl must exist");
    let pattern = Regex::new(r"relationship\s+'([^']+)'\s+fromColumn:\s+([\w\.]+)\s+toColumn:\s+([\w\.]+)").unwrap();
    let text = read(&path);
    let blocks: Vec<_> = pattern.captures_iter(&text).collect();
    assert!(blocks.len() >= 5, "Expected at least 5 star schema relationships");
    for block in blocks {
        assert!(
            block[2].contains('.'),
            "Relationship {} fromColumn must include table name",
            &block[1]
        );
        assert!(
            block[3].contains('.'),
            "Relationship {} toColumn must include table name",
            &block[1]
        );
    }
}

/// A documented edge that is missing leaves its dimension orphaned: filters never
/// propagate and the dimension repeats the grand total on every row.
#[test]
fn documented_star_schema_edges_all_exist() {
    let declared = declared_edges();
    let documented = documented();
    let missing: Vec<_> = documented.difference(&declared).collect();
    assert!(
        missing.is_empty(),
        "relationships.tmdl is missing a documented star schema edge: {missing:?}"
    );
    let extra: Vec<_> = declared.difference(&documented).collect();
    assert!(
        extra.is_empty(),
        "relationships.tmdl declares an edge the ER diagrams do not document: {extra:?}"
    );
}

/// Mermaid `A ||--o{ B : "Key"` means one A row to many B rows, so B holds the foreign
/// key and TMDL must declare fromColumn B.Key. A reversed edge tells the reader that
/// filters propagate the opposite way.
#[test]
fn er_diagrams_draw_each_edge_in_the_direction_the_model_declares() {
    let declared = declared_edges();
    let table = |column: &str| column.split('.').next().unwrap().to_owned();
    let from_column_by_tables: HashMap<(String, String), String> = declared
        .iter()
        .map(|(from, to)| ((table(from), table(to)), from.clone()))
        .collect();
    for document in [app().join("README.md"), app().join("docs").join("data-model.md")] {
        let edges = mermaid_er_edges(&document);
        let name = file_name(&document);
        assert_eq!(
            edges.len(),
            declared.len(),
            "{name} draws {} edges for {} relationships",
            edges.len(),
            declared.len()
        );
        for (one, many, key) in edges {
            assert_eq!(
                from_column_by_tables.get(&(many.clone(), one.clone())),
                Some(&format!("{many}.{key}")),
                "{name} draws '{one} ||--o{{ {many} : {key}' but the model declares no {many}.{key} relationship into {one}"
            );
        }
    }
}

/// Dim_ANZSIC is a literal lookup, so both ANZSIC relationships die without these codes.
#[test]
fn dim_anzsic_declares_every_industry_code_the_model_joins_on() {
    let partition = read(&definition().join("expressions.tmdl"));
    let declared: BTreeSet<String> = Regex::new(r#"\{"(\d{4})","#)
        .unwrap()
        .captures_iter(&partition)
        .map(|c| c[1].to_owned())
        .collect();
    let joined: BTreeSet<String> = ["sample-entities.csv", "sample-ato-benchmarks.csv"]
        .iter()
        .flat_map(|fixture| csv_rows(&samples().join(fixture)))
        .map(|row| row["ANZSIC_Code"].clone())
        .collect();
    assert!(
        !joined.is_empty(),
        "fixtures must supply the ANZSIC codes the model joins on"
    );
    let missing: Vec<_> = joined.difference(&declared).collect();
    assert!(
        missing.is_empty(),
        "Dim_ANZSIC must declare every ANZSIC code Dim_Entity and Fact_ATOBenchmark join on: {missing:?}"
    );
}

/// Depreciation (890) and finance interest (895) are in the Operating Expenses SubClass.
/// Without an explicit add-back, EBITDA is the same arithmetic as Net Profit Before Tax.
#[test]
fn ebitda_adds_back_depreciation_and_interest() {
    let ebitda = &general_ledger()["EBITDA"];
    for code in [r#""890""#, r#""895""#] {
        assert!(
            ebitda.contains(code),
            "EBITDA must add back account {code} to differ from Net Profit Before Tax"
        );
    }
}

/// Current-period earnings are never closed to account 510 in the ledger fixture, so a
/// check of Assets less Liabilities and Equity alone returns the period's net profit.
#[test]
fn balance_sheet_check_accounts_for_earnings_not_closed_to_equity() {
    assert!(
        general_ledger()["Balance Sheet Check"].contains("[Net Profit Before Tax]"),
        "Balance Sheet Check must include earnings to date or it returns the period's net profit instead of $0.00 wherever that profit is not nil"
    );
}

/// A plain class-filtered SUM returns only the period's movement under a date context.
#[test]
fn balance_sheet_measures_accumulate_to_the_last_date_in_context() {
    let expressions = general_ledger();
    let running_total = "DATESBETWEEN(Dim_Date[Date], BLANK(), MAX(Dim_Date[Date]))";
    for (measure, legs) in [
        ("Total Assets", 1),
        ("Total Liabilities", 1),
        ("Total Equity", 1),
        ("Working Capital", 2),
    ] {
        assert_eq!(
            expressions[measure].matches(running_total).count(),
            legs,
            "[{measure}] must accumulate all {legs} of its legs to the last visible date"
        );
    }
}

/// A bare column predicate inside CALCULATE replaces the matrix's own row filter;
/// KEEPFILTERS intersects the category with the row instead. The cumulative
/// DATESBETWEEN override stays bare on purpose.
#[test]
fn account_category_predicates_keep_the_row_selection() {
    let mut offenders = Vec::new();
    for table in ["Fact_GeneralLedger", "Fact_Budget"] {
        let text = read(&tables_dir().join(format!("{table}.tmdl")));
        for (number, line) in splitlines(&text).into_iter().enumerate() {
            if line.contains("Dim_Account[") && !line.contains("KEEPFILTERS(") && !line.contains("ISINSCOPE(") {
                offenders.push(format!("{table}.tmdl:{}: {}", number + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "Every account category predicate must be wrapped in KEEPFILTERS or it overwrites the account row heading the financial matrices select on: {offenders:?}"
    );
    let total_assets = &general_ledger()["Total Assets"];
    assert!(total_assets.contains(r#"KEEPFILTERS(Dim_Account[Class] = "Asset")"#));
    assert!(
        total_assets.contains("DATESBETWEEN(Dim_Date[Date], BLANK(), MAX(Dim_Date[Date]))"),
        "The cumulative date override must keep replacing the period filter"
    );
    assert!(!total_assets.contains("KEEPFILTERS(DATESBETWEEN"));
}

/// Many entities point at one industry row, so selecting an entity leaves Dim_ANZSIC
/// and Fact_ATOBenchmark unfiltered unless each benchmark applies TREATAS.
#[test]
fn benchmark_measures_follow_the_selected_entity_industry() {
    let expressions = measure_expressions(&tables_dir().join("Fact_ATOBenchmark.tmdl"));
    let industry_filter = "TREATAS(VALUES(Dim_Entity[ANZSIC_Code]), Fact_ATOBenchmark[ANZSIC_Code])";
    let benchmarks: Vec<_> = expressions
        .keys()
        .filter(|name| name.starts_with("ATO Benchmark "))
        .collect();
    assert_eq!(benchmarks.len(), 5);
    for name in benchmarks {
        assert!(
            expressions[name].contains(industry_filter),
            "[{name}] must be scoped to the industries of the entities in context"
        );
    }
}

/// A period with no payroll events is missing data, not full statutory compliance.
#[test]
fn compliance_rate_is_blank_when_no_super_was_accrued() {
    let expressions = measure_expressions(&tables_dir().join("Fact_PayrollSuper.tmdl"));
    let rate = &expressions["Payday Super Compliance Rate %"];
    assert!(rate.contains("BLANK()"));
    assert!(
        !rate.contains("1.0"),
        "A zero super liability must not render as 100% compliance"
    );
}

const TEXT_MEASURES_IN_FILE_ORDER: [&str; 12] = [
    "Fact_ATOBenchmark.tmdl:[ATO Compliance Risk Profile]",
    "Fact_ATOBenchmark.tmdl:[Benchmark Turnover Band]",
    "Fact_GeneralLedger.tmdl:[Consolidation Scope]",
    "Fact_GeneralLedger.tmdl:[Report Selection]",
    "Fact_GeneralLedger.tmdl:[Revenue Budget Title]",
    "Fact_GeneralLedger.tmdl:[Monthly Revenue Title]",
    "Fact_PayrollSuper.tmdl:[Payroll Review Note]",
    "Review_Exception.tmdl:[Finding Action]",
    "Review_Exception.tmdl:[Finding Question]",
    "Review_Exception.tmdl:[Finding Reason]",
    "Review_Exception.tmdl:[Finding Evidence requested]",
    "Review_Exception.tmdl:[Finding Evidence state]",
];

/// Require descriptions for every measure and formats for non-text measures.
#[test]
fn all_measures_have_supported_descriptions_and_numeric_formats() {
    let mut count = 0;
    let mut unformatted = Vec::new();
    for path in table_files() {
        let text = read(&path);
        let lines = splitlines(&text);
        let starts = object_starts(&lines, OBJECT_START);
        for (index, line) in lines.iter().enumerate() {
            if !line.starts_with("\tmeasure ") {
                continue;
            }
            let name = measure_name(line);
            count += 1;
            let block = lines[index..next_object(&starts, index, lines.len())].join("\n");
            if !block.contains("formatString:") && !block.contains("formatStringDefinition") {
                unformatted.push(format!("{}:[{name}]", file_name(&path)));
            }
            assert!(
                index > 0 && lines[index - 1].starts_with("\t/// "),
                "Measure [{name}] in {} is missing a supported description",
                file_name(&path)
            );
        }
    }
    assert_eq!(count, 63, "Expected exactly 63 explicit DAX measures across model");
    assert_eq!(
        unformatted, TEXT_MEASURES_IN_FILE_ORDER,
        "Only the twelve text measures may omit a numeric format string"
    );
}

/// Calculation groups must declare explicit precedence and ordinals on calculation items.
#[test]
fn calculation_groups_have_precedence_and_ordinals() {
    let groups: Vec<_> = table_files()
        .into_iter()
        .filter(|path| file_name(path).contains("CalcGroup"))
        .collect();
    assert!(groups.len() >= 2, "Expected at least 2 calculation groups");
    let model = read(&definition().join("model.tmdl"));
    assert!(
        Regex::new(r"(?m)^\tdiscourageImplicitMeasures\s*$")
            .unwrap()
            .is_match(&model)
    );
    for path in groups {
        let text = read(&path);
        let name = file_name(&path);
        assert!(
            text.contains("calculationGroup"),
            "{name} must declare calculationGroup block"
        );
        assert!(text.contains("precedence:"), "{name} must specify precedence");
        assert!(text.contains("ordinal:"), "{name} calculation items must have ordinals");
    }
}
