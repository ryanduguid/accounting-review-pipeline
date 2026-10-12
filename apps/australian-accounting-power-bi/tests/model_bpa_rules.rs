//! Each Best Practice Analyzer rule selects violations, not compliant objects.
//!
//! A BPA rule expression selects the objects that break the rule, so a predicate written
//! the right way round is false for the shipped model. Tabular Editor is not run here, so
//! this suite evaluates the committed expressions against metadata parsed from TMDL.

mod common;

use std::collections::{BTreeMap, HashMap};

use common::bpa::{Datum, Object, evaluate, text};
use common::*;
use regex::Regex;
use serde_json::Value;

/// TMDL declares no dataType on a measure; the engine infers it. These twelve return
/// text, which is why the format rule exempts them.
const TEXT_MEASURES: [&str; 12] = [
    "ATO Compliance Risk Profile",
    "Benchmark Turnover Band",
    "Consolidation Scope",
    "Report Selection",
    "Revenue Budget Title",
    "Monthly Revenue Title",
    "Payroll Review Note",
    "Finding Action",
    "Finding Question",
    "Finding Reason",
    "Finding Evidence requested",
    "Finding Evidence state",
];

fn shipped_measures() -> Vec<Object> {
    let format = Regex::new(r"(?m)^\t\tformatString: (.+)$").unwrap();
    let hidden = Regex::new(r"(?m)^\t\tisHidden").unwrap();
    let mut measures = Vec::new();
    for path in table_files() {
        let content = read(&path);
        let lines = splitlines(&content);
        let starts = object_starts(&lines, OBJECT_START);
        for (index, line) in lines.iter().enumerate() {
            if !line.starts_with("\tmeasure ") {
                continue;
            }
            let name = measure_name(line);
            let block = lines[index..next_object(&starts, index, lines.len())].join("\n");
            let previous = if index > 0 { lines[index - 1] } else { "" };
            let data_type = if TEXT_MEASURES.contains(&name.as_str()) {
                "String"
            } else {
                "Decimal"
            };
            measures.push(Object::from([
                (
                    "FormatString",
                    text(format.captures(&block).map_or("", |c| c.get(1).unwrap().as_str())),
                ),
                ("Description", text(previous.strip_prefix("\t/// ").unwrap_or(""))),
                ("IsHidden", Datum::Bool(hidden.is_match(&block))),
                ("DataType", text(data_type)),
                ("Name", Datum::Text(name)),
            ]));
        }
    }
    measures
}

fn shipped_relationships() -> Vec<Object> {
    let content = read(&definition().join("relationships.tmdl"));
    let header = Regex::new(r"(?m)^relationship '([^']+)'").unwrap();
    let behaviour = Regex::new(r"crossFilteringBehavior: (\w+)").unwrap();
    let starts: Vec<_> = header.captures_iter(&content).collect();
    starts
        .iter()
        .enumerate()
        .map(|(index, found)| {
            let begin = found.get(0).unwrap().end();
            let end = starts
                .get(index + 1)
                .map_or(content.len(), |next| next.get(0).unwrap().start());
            // TMDL omits crossFilteringBehavior when the relationship filters one way.
            let filter = behaviour
                .captures(&content[begin..end])
                .map_or("OneDirection", |c| c.get(1).unwrap().as_str());
            Object::from([("Name", text(&found[1])), ("CrossFilteringBehavior", text(filter))])
        })
        .collect()
}

fn shipped_calculation_groups() -> Vec<Object> {
    let precedence = Regex::new(r"precedence: (-?\d+)").unwrap();
    table_files()
        .iter()
        .map(|path| (path, read(path)))
        .filter(|(_, content)| content.contains("calculationGroup"))
        .map(|(path, content)| {
            let value = precedence.captures(&content).map_or(0, |c| c[1].parse().unwrap());
            Object::from([("Name", Datum::Text(stem(path))), ("Precedence", Datum::Int(value))])
        })
        .collect()
}

fn rules() -> BTreeMap<String, Value> {
    let rules = read_json(&app().join("tests").join("model_bpa_rules.json"));
    rules
        .as_array()
        .unwrap()
        .iter()
        .map(|rule| (rule.at("ID").as_str().unwrap().to_owned(), rule.clone()))
        .collect()
}

fn expression(rules: &BTreeMap<String, Value>, id: &str) -> String {
    rules[id].at("Expression").as_str().unwrap().to_owned()
}

#[test]
fn every_rule_declares_the_fields_the_analyzer_reads() {
    for (id, rule) in rules() {
        for field in ["Name", "Category", "Severity", "Scope", "Expression"] {
            assert!(rule.get(field).is_some(), "{id} is missing {field}");
        }
    }
}

/// The shipped model satisfies all 4 rules, so a correct expression selects nothing.
#[test]
fn no_rule_selects_a_compliant_shipped_object() {
    let scopes = HashMap::from([
        ("Measure", shipped_measures()),
        ("Relationship", shipped_relationships()),
        ("CalculationGroup", shipped_calculation_groups()),
    ]);
    for (name, objects) in &scopes {
        assert!(!objects.is_empty(), "No {name} metadata was parsed out of the TMDL");
    }
    for (id, rule) in rules() {
        let scope = &scopes[rule.at("Scope").as_str().unwrap()];
        let selected: Vec<&Datum> = scope
            .iter()
            .filter(|object| evaluate(rule.at("Expression").as_str().unwrap(), object))
            .map(|object| &object["Name"])
            .collect();
        assert!(
            selected.is_empty(),
            "{id} selects compliant objects; a BPA expression selects violations: {selected:?}"
        );
    }
}

/// A crafted violator and a nearby valid control for each rule.
fn violation_cases() -> BTreeMap<&'static str, (Object, Object)> {
    let measure = |name, format, data_type| {
        Object::from([
            ("Name", text(name)),
            ("FormatString", text(format)),
            ("DataType", text(data_type)),
        ])
    };
    let described = |name, description| {
        Object::from([
            ("Name", text(name)),
            ("IsHidden", Datum::Bool(false)),
            ("Description", text(description)),
        ])
    };
    let relationship = |name, filter| Object::from([("Name", text(name)), ("CrossFilteringBehavior", text(filter))]);
    let group = |name, precedence| Object::from([("Name", text(name)), ("Precedence", Datum::Int(precedence))]);
    BTreeMap::from([
        (
            "BPA_MEASURE_FORMAT_STRING",
            (
                measure("violator", "", "Decimal"),
                measure("control", "$#,##0.00", "Decimal"),
            ),
        ),
        (
            "BPA_MEASURE_DESCRIPTION",
            (described("violator", ""), described("control", "Documented.")),
        ),
        (
            "BPA_STAR_SCHEMA_RELATIONSHIPS",
            (
                relationship("violator", "BothDirections"),
                relationship("control", "OneDirection"),
            ),
        ),
        (
            "BPA_CALC_GROUP_PRECEDENCE",
            (group("violator", 0), group("control", 10)),
        ),
    ])
}

/// Each rule must flag a crafted violator while leaving a nearby valid object alone.
#[test]
fn each_rule_selects_its_own_violation_and_spares_the_control() {
    let cases = violation_cases();
    let rules = rules();
    assert_eq!(
        cases.keys().copied().collect::<Vec<_>>(),
        rules.keys().map(String::as_str).collect::<Vec<_>>(),
        "Every rule needs a violation case"
    );
    for (id, (violator, control)) in &cases {
        let expression = expression(&rules, id);
        assert!(evaluate(&expression, violator), "{id} must select {violator:?}");
        assert!(!evaluate(&expression, control), "{id} must not select {control:?}");
    }
}

/// A measure that returns text has no numeric format to require.
#[test]
fn the_format_rule_exempts_a_text_measure() {
    let expression = expression(&rules(), "BPA_MEASURE_FORMAT_STRING");
    let measure = |data_type: &str| {
        Object::from([
            ("Name", text("m")),
            ("FormatString", text("")),
            ("DataType", text(data_type)),
        ])
    };
    assert!(!evaluate(&expression, &measure("String")));
    assert!(evaluate(&expression, &measure("Decimal")));
}

#[test]
fn evaluator_follows_the_rule_grammar() {
    let object = Object::from([
        ("A", text("x")),
        ("B", text("")),
        ("N", Datum::Int(3)),
        ("F", Datum::Bool(false)),
    ]);
    for (expression, expected) in [
        ("string.IsNullOrEmpty(B) && A == \"x\"", true),
        ("STRING.ISNULLOREMPTY(A) || N > 2", true),
        ("!F && !string.IsNullOrEmpty(A)", true),
        ("N <= 0 || A <> \"x\"", false),
        ("(N >= 3) && !(B != \"\")", true),
        ("A == 3", false),
        // Python skips the right operand once the left decides, so these never reach Unknown.
        ("A == \"never\" && string.IsNullOrEmpty(Unknown)", false),
        ("N > 2 || Unknown == 1", true),
        ("!(F || (A == \"y\" && Unknown < 3))", true),
    ] {
        assert_eq!(evaluate(expression, &object), expected, "{expression}");
    }
    for reached in ["A == \"x\" && string.IsNullOrEmpty(Unknown)", "F || Unknown"] {
        let outcome = std::panic::catch_unwind(|| evaluate(reached, &object));
        assert!(outcome.is_err(), "{reached} must reach the undefined property");
    }
}
