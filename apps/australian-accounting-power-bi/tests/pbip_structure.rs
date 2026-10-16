//! Contracts for a loadable, source-controlled Power BI project.
//!
//! JSON is read with Python's access rules (`Py::at`, `Py::py_get`) and in the
//! earlier suite's order, so a malformed document fails the same checks it failed there.

mod common;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use common::*;
use regex::Regex;
use serde_json::Value;

/// Columns and measures declared by each TMDL table, keyed by table name then kind.
fn tmdl_field_inventory() -> HashMap<String, HashMap<&'static str, HashSet<String>>> {
    let mut inventory = HashMap::new();
    for path in table_files() {
        let mut fields: HashMap<&'static str, HashSet<String>> =
            HashMap::from([("Column", HashSet::new()), ("Measure", HashSet::new())]);
        for line in splitlines(&read(&path)) {
            for (kind, prefix) in [("Column", "\tcolumn "), ("Measure", "\tmeasure ")] {
                let Some(raw) = line.strip_prefix(prefix) else { continue };
                let raw = if kind == "Measure" {
                    raw.split_once(" =").map_or(raw, |(name, _)| name)
                } else {
                    raw
                };
                fields
                    .get_mut(kind)
                    .unwrap()
                    .insert(raw.trim().trim_matches('\'').replace("''", "'"));
            }
        }
        inventory.insert(stem(&path), fields);
    }
    inventory
}

/// Metric words a visual title may only use when the visual binds a field whose table
/// or name carries the same word. "Intercompany" is deliberately absent: the rollup
/// pivot applies the elimination through CalcGroup_Consolidation.
const TITLE_METRIC_WORDS: [&str; 10] = [
    "Cash",
    "Revenue",
    "EBITDA",
    "Working Capital",
    "Net Assets",
    "GIC",
    "SGC",
    "Benchmark",
    "Compliance",
    "Expense",
];

/// A visual's rendered title literal, or an empty string when it has none.
fn visual_title(visual: Option<&Value>) -> String {
    let Some(visual @ Value::Object(_)) = visual else {
        return String::new();
    };
    let empty = Value::Object(serde_json::Map::new());
    let objects = visual.py_get("visualContainerObjects").unwrap_or(&empty);
    let Some(Value::Array(titles)) = objects.py_get("title") else {
        return String::new();
    };
    let Some(first) = titles.first() else {
        return String::new();
    };
    let mut node = first;
    for key in ["properties", "text", "expr", "Literal"] {
        node = node.py_get(key).unwrap_or(&empty);
    }
    match node.py_get("Value") {
        Some(Value::String(text)) => text.trim_matches('\'').to_owned(),
        _ => String::new(),
    }
}

fn expression_names() -> Vec<String> {
    let text = read(&definition().join("expressions.tmdl"));
    let pattern = Regex::new(r"(?m)^expression\s+('[^']+'|[^\s=]+)\s*=").unwrap();
    let mut names: Vec<String> = pattern
        .captures_iter(&text)
        .map(|c| c[1].trim_matches('\'').to_owned())
        .collect();
    names.sort();
    names
}

fn visuals_of(page: &Path) -> Vec<Value> {
    visual_files(page).iter().map(|path| read_json(path)).collect()
}

fn position(visual: &Value, key: &str) -> f64 {
    num(visual.at("position").at(key))
}

fn name_of(visual: &Value) -> String {
    visual.at("name").to_string()
}

// Semantic model structure.

#[test]
fn semantic_model_uses_the_supported_tmdl_folder_layout() {
    let descriptor = read_json(&model().join("definition.pbism"));
    assert_eq!(descriptor.at("version"), "4.0");
    assert!(
        descriptor
            .at("$schema")
            .as_str()
            .unwrap()
            .contains("semanticModel/definitionProperties/1.0.0")
    );
    for file in ["database.tmdl", "model.tmdl", "relationships.tmdl", "expressions.tmdl"] {
        assert!(definition().join(file).is_file(), "{file}");
    }
    assert_eq!(files_ending(&model(), ".tmdl"), Vec::<PathBuf>::new());
}

#[test]
fn model_references_every_table_and_named_expression() {
    let model = read(&definition().join("model.tmdl"));
    let tables: Vec<String> = table_files().iter().map(|path| stem(path)).collect();
    let expressions = expression_names();
    assert_eq!(tables.len(), 14);
    let shared: Vec<_> = tables.iter().filter(|name| expressions.contains(name)).collect();
    assert!(
        shared.is_empty(),
        "Power Query table queries and named expressions must have distinct names: {shared:?}"
    );
    let expected = [
        "Dim_Date_AU",
        "Fx_MoneyDisplay",
        "Fx_RequireRows",
        "Fx_ValidateABN",
        "IndustryLookup",
        "ModelEndDate",
        "ModelStartDate",
        "SampleFolder",
        "Source_Dim_Account",
        "Source_Dim_Entity",
        "Source_Fact_ATOBenchmark",
        "Source_Fact_Budget",
        "Source_Fact_GeneralLedger",
        "Source_Fact_PayrollSuper",
    ];
    assert_eq!(expressions, expected);
    for name in &tables {
        assert!(model.contains(&format!("ref table {name}")), "ref table {name}");
    }
    for name in &expressions {
        assert!(
            model.contains(&format!("ref expression {name}")),
            "ref expression {name}"
        );
    }
}

#[test]
fn every_partition_symbol_resolves_to_a_named_expression() {
    let names: HashSet<String> = expression_names().into_iter().collect();
    let partition = Regex::new(r"(?m)^\tpartition\s").unwrap();
    let source = Regex::new(r"(?m)^\t\t\t\tSource = ([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let mut unresolved = Vec::new();
    let mut partitions = 0;
    for path in table_files() {
        let text = read(&path);
        partitions += partition.find_iter(&text).count();
        for symbol in source.captures_iter(&text) {
            if !names.contains(&symbol[1]) && &symbol[1] != "Table" {
                unresolved.push(format!("{}:{}", file_name(&path), &symbol[1]));
            }
        }
    }
    // Nine import partitions are declared here. The 2 calculation-group
    // tables receive implicit partitions when the model is loaded.
    assert_eq!(partitions, 12);
    assert_eq!(unresolved, Vec::<String>::new());
}

#[test]
fn measure_descriptions_use_supported_triple_slash_syntax() {
    let (mut missing, mut unsupported, mut count) = (Vec::new(), Vec::new(), 0);
    for path in table_files() {
        let text = read(&path);
        let lines = splitlines(&text);
        for (index, line) in lines.iter().enumerate() {
            if line.starts_with("\t\tdescription:") {
                unsupported.push(format!("{}:{}", file_name(&path), index + 1));
            }
            if line.starts_with("\tmeasure ") {
                count += 1;
                if index == 0 || !lines[index - 1].starts_with("\t/// ") {
                    missing.push(format!("{}:{}", file_name(&path), index + 1));
                }
            }
        }
    }
    assert_eq!(count, 63);
    assert_eq!(unsupported, Vec::<String>::new());
    assert_eq!(missing, Vec::<String>::new());
}

#[test]
fn power_query_has_one_canonical_tmdl_source() {
    assert!(!app().join("powerquery").exists());
    let mut sources = vec![read(&definition().join("expressions.tmdl"))];
    sources.extend(table_files().iter().map(|path| read(path)));
    let joined = sources.join("\n");
    let pattern = Regex::new(r"File\.Contents\(([^)]+)\)").unwrap();
    let paths: Vec<&str> = pattern
        .captures_iter(&joined)
        .map(|c| c.get(1).unwrap().as_str())
        .collect();
    assert_eq!(paths.len(), 9);
    for path in paths {
        assert!(
            path.starts_with("SampleFolder & "),
            "CSV imports need the configured absolute sample folder: {path}"
        );
    }
}

// Report structure.

fn pages() -> Value {
    read_json(&report_definition().join("pages").join("pages.json"))
}

#[test]
fn report_scaffold_uses_published_pbir_contracts() {
    let platform = read_json(&report().join(".platform"));
    let binding = read_json(&report().join("definition.pbir"));
    let version = read_json(&report_definition().join("version.json"));
    let report = read_json(&report_definition().join("report.json"));
    let pages = pages();
    assert_eq!(platform.at("metadata").at("type"), "Report");
    assert_eq!(binding.at("version"), "4.0");
    assert!(
        binding
            .at("$schema")
            .as_str()
            .unwrap()
            .contains("report/definitionProperties/1.0.0")
    );
    assert_eq!(version.at("version"), "2.0.0");
    assert!(report.at("$schema").as_str().unwrap().contains("report/"));
    assert_eq!(pages.at("pageOrder").as_array().unwrap().len(), 6);
    assert_eq!(pages.at("activePageName"), pages.at("pageOrder").at(0));
}

/// Whether a visual binds data: `visual.get("query", {}).get("queryState")` is truthy.
fn is_bound(visual: &Value) -> bool {
    let empty = Value::Object(serde_json::Map::new());
    visual
        .py_get("query")
        .unwrap_or(&empty)
        .py_get("queryState")
        .is_some_and(truthy)
}

#[test]
fn all_pages_and_review_visuals_are_materialised() {
    let order: Vec<Value> = pages().at("pageOrder").as_array().unwrap().clone();
    let mut types: HashMap<String, usize> = HashMap::new();
    let mut unbound = Vec::new();
    for name in &order {
        let directory = report_definition().join("pages").join(name.as_str().unwrap());
        let page = read_json(&directory.join("page.json"));
        assert_eq!(page.at("name"), name);
        assert_eq!((num(page.at("width")), num(page.at("height"))), (1280.0, 720.0));
        for path in visual_files(&directory) {
            let document = read_json(&path);
            assert_eq!(
                document.at("name").as_str(),
                path.parent().unwrap().file_name().unwrap().to_str()
            );
            let visual = document.at("visual");
            let kind = visual.at("visualType").as_str().unwrap();
            *types.entry(kind.to_owned()).or_default() += 1;
            if !["textbox", "pageNavigator", "actionButton"].contains(&kind) && !is_bound(visual) {
                unbound.push(path.strip_prefix(app()).unwrap().display().to_string());
            }
        }
    }
    let count = |kind: &str| types.get(kind).copied().unwrap_or(0);
    assert_eq!(count("pageNavigator"), order.len() - 1);
    assert!(count("slicer") >= 2 * (order.len() - 1));
    assert!(count("lineChart") > 0);
    assert_eq!(unbound, Vec::<String>::new());
}

#[test]
fn drillthrough_binds_complete_review_context_and_preserves_back_navigation() {
    let pages = report_definition().join("pages");
    let evidence = pages.join("a02closeevid202410002");
    let page = read_json(&evidence.join("page.json"));
    assert_eq!(page["type"], "Drillthrough");
    let properties: HashSet<_> = page["pageBinding"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|parameter| parameter["fieldExpr"]["Column"]["Property"].as_str().unwrap())
        .collect();
    assert_eq!(
        properties,
        HashSet::from(["RunID", "Entity", "Period", "Basis", "ExceptionKey"])
    );
    let back = read_json(&evidence.join("visuals/reviewevidback00001/visual.json"));
    assert_eq!(
        back["visual"]["visualContainerObjects"]["visualLink"][0]["properties"]["type"]["expr"]["Literal"]["Value"],
        "'Back'"
    );
    let finding = read_json(&pages.join("a01closehome202410001/visuals/reviewhomefinding01/visual.json"));
    assert_eq!(
        finding["visual"]["query"]["queryState"]["Values"]["projections"][0]["field"]["Column"]["Property"],
        "Account"
    );
    assert_eq!(
        finding["visual"]["objects"]["selection"][0]["properties"]["singleSelect"]["expr"]["Literal"]["Value"],
        "true"
    );
}

#[test]
fn pages_have_selectors_alt_text_and_unique_keyboard_order() {
    for page_dir in page_dirs() {
        let visuals = visuals_of(&page_dir);
        let selectors: Vec<&Value> = visuals
            .iter()
            .filter(|obj| obj.at("visual").at("visualType") == "slicer")
            .collect();
        let bindings: HashSet<String> = selectors
            .iter()
            .flat_map(|obj| query_field_bindings(obj).into_iter().map(|(_, _, field)| field))
            .collect();
        let name = file_name(&page_dir);
        let required: &[&str] = match name.as_str() {
            "a01closehome202410001" => &["Entity", "Period"],
            "a02closeevid202410002" => &[],
            _ => &["TradingName", "FinancialYear"],
        };
        assert!(
            required.iter().all(|field| bindings.contains(*field)),
            "{name}: {required:?}"
        );
        let mut orders: Vec<f64> = visuals.iter().map(|obj| position(obj, "tabOrder")).collect();
        let count = orders.len();
        orders.sort_by(order);
        orders.dedup();
        assert_eq!(orders.len(), count, "{name}: duplicate tabOrder");
        // Native Desktop visits the higher tabOrder values first.
        let mut keyboard: Vec<&Value> = visuals.iter().collect();
        keyboard.sort_by(|a, b| order(&position(b, "tabOrder"), &position(a, "tabOrder")));
        let mut reading: Vec<&Value> = visuals.iter().collect();
        reading.sort_by(|a, b| {
            order(
                &(position(a, "y"), position(a, "x")),
                &(position(b, "y"), position(b, "x")),
            )
        });
        let names = |list: &[&Value]| list.iter().map(|obj| name_of(obj)).collect::<Vec<_>>();
        assert_eq!(names(&keyboard), names(&reading), "{name}");
        for obj in &visuals {
            let page = read_json(&page_dir.join("page.json"));
            assert!(position(obj, "x") >= 0.0 && position(obj, "y") >= 0.0);
            assert!(position(obj, "x") + position(obj, "width") <= num(page.at("width")));
            assert!(position(obj, "y") + position(obj, "height") <= num(page.at("height")));
            let general = obj.at("visual").at("visualContainerObjects").at("general").at(0);
            let alt = literal(general.at("properties").at("altText"));
            let label = name_of(obj);
            assert!(alt.chars().count() > 15, "{label}");
        }
    }
}

#[test]
fn phone_layouts_preserve_reading_order_and_touch_spacing() {
    for page_dir in page_dirs() {
        let mut expected = visuals_of(&page_dir);
        expected.sort_by(|a, b| {
            order(
                &(position(a, "y"), position(a, "x")),
                &(position(b, "y"), position(b, "x")),
            )
        });
        let mobile: Vec<Value> = expected
            .iter()
            .map(|obj| {
                read_json(
                    &page_dir
                        .join("visuals")
                        .join(obj.at("name").as_str().unwrap())
                        .join("mobile.json"),
                )
            })
            .collect();
        let mut last_bottom = -8.0;
        for (index, (visual, portrait)) in expected.iter().zip(&mobile).enumerate() {
            let context = format!("{}/{}", file_name(&page_dir), name_of(visual));
            assert!(
                portrait
                    .at("$schema")
                    .as_str()
                    .unwrap()
                    .contains("visualContainerMobileState/2.4.0/schema.json"),
                "{context}"
            );
            let pos = |key: &str| num(portrait.at("position").at(key));
            assert_eq!((pos("x"), pos("width")), (0.0, 324.0), "{context}");
            assert!(pos("y") >= last_bottom + 8.0, "{context}");
            assert!(pos("height") >= 56.0, "{context}");
            assert_eq!(pos("tabOrder"), (expected.len() - index - 1) as f64, "{context}");
            assert_eq!(pos("y") % 4.0, 0.0, "{context}");
            assert_eq!(pos("height") % 4.0, 0.0, "{context}");
            last_bottom = pos("y") + pos("height");
            if visual.at("visual").at("visualType") == "pageNavigator" {
                let layout = portrait.at("objects").at("layout").at(0).at("properties");
                assert_eq!(literal(layout.at("rowCount")), "3D", "{context}");
                assert_eq!(literal(layout.at("columnCount")), "2D", "{context}");
                assert!((pos("height") - 16.0) / 3.0 >= 44.0, "{context}");
            }
        }
    }
}

/// A `12D`-style literal as a number.
fn number_literal(node: &Value) -> f64 {
    let text = literal(node);
    text[..text.len() - 1].parse().expect("numeric literal")
}

#[test]
fn phone_selected_finding_details_fit_the_canvas() {
    let path = report_definition().join("pages/a01closehome202410001/visuals/reviewhomequestions/mobile.json");
    let portrait = read_json(&path);
    let objects = portrait.at("objects");
    let label = number_literal(
        objects
            .at("columnHeaders")
            .at(0)
            .at("properties")
            .at("defaultColumnWidth"),
    );
    let value = number_literal(objects.at("columnWidth").at(0).at("properties").at("value"));
    assert!(label + value <= num(portrait.at("position").at("width")) - 16.0);
    assert!(value >= 150.0);
}

#[test]
fn phone_results_use_the_native_vertical_card_layout() {
    for path in all_visual_files() {
        let document = read_json(&path);
        let visual = document.at("visual");
        if visual.at("visualType") != "cardVisual" {
            continue;
        }
        let portrait = read_json(&path.with_file_name("mobile.json"));
        let layout = portrait.at("objects").at("layout").at(0);
        // Desktop writes card layout properties without a selector.
        // A default selector is silently ignored for this category.
        assert!(!py_in("selector", layout), "{}", path.display());
        assert_eq!(literal(layout.at("properties").at("orientation")), "1D");
        let projections = visual.at("query").at("queryState").at("Data").at("projections");
        let count = projections.as_array().unwrap().len() as f64;
        assert!(
            num(portrait.at("position").at("height")) >= count * 100.0,
            "{}",
            path.display()
        );
    }
}

#[test]
fn every_visual_field_binding_resolves_to_the_semantic_model() {
    let inventory = tmdl_field_inventory();
    let mut bindings = Vec::new();
    for path in all_visual_files() {
        let document = read_json(&path);
        let query = document
            .py_get("visual")
            .filter(|v| v.is_object())
            .and_then(|v| v.get("query"));
        let state = query.filter(|q| q.is_object()).and_then(|q| q.get("queryState"));
        if state.is_none_or(Value::is_null) {
            continue;
        }
        let relative = path.strip_prefix(app()).unwrap().display().to_string();
        bindings.extend(
            query_field_bindings(&document)
                .into_iter()
                .map(|(k, t, f)| (k, t, f, relative.clone())),
        );
    }
    let resolves = |kind: &str, table: &str, field: &str| inventory.get(table).is_some_and(|f| f[kind].contains(field));
    let missing: Vec<String> = bindings
        .iter()
        .filter(|(kind, table, field, _)| !resolves(kind, table, field))
        .map(|(kind, table, field, path)| format!("{path}: {kind} {table}[{field}]"))
        .collect();
    assert!(!bindings.is_empty(), "Expected data-bound PBIR visuals");
    assert_eq!(missing, Vec::<String>::new());
}

#[test]
fn budget_visuals_use_contextual_title_measures() {
    for (id, measure) in [
        ("39ec83f2c78b1ea20481", "Revenue Budget Title"),
        ("2f440022aab813e0dbbb", "Monthly Revenue Title"),
    ] {
        let path = report_definition()
            .join("pages/062c0d9cd2ba960997fe/visuals")
            .join(id)
            .join("visual.json");
        let document = read_json(&path);
        let title = document
            .at("visual")
            .at("visualContainerObjects")
            .at("title")
            .at(0)
            .at("properties")
            .at("text");
        let expected = (
            "Measure".to_owned(),
            "Fact_GeneralLedger".to_owned(),
            measure.to_owned(),
        );
        assert_eq!(query_field_bindings(title), [expected]);
    }
}

/// A title is the only label a reader gets, so it must not name an absent metric.
#[test]
fn visual_titles_do_not_name_metrics_the_visual_does_not_plot() {
    let mut titled = 0;
    let mut unsupported = Vec::new();
    for path in all_visual_files() {
        let document = read_json(&path);
        let visual = document.py_get("visual");
        let title = visual_title(visual);
        if title.is_empty() {
            continue;
        }
        titled += 1;
        let query = visual.filter(|v| v.is_object()).and_then(|v| v.get("query"));
        let state = query
            .filter(|q| q.is_object())
            .and_then(|q| q.get("queryState"))
            .cloned()
            .unwrap_or(Value::Null);
        let bound: Vec<String> = query_field_bindings(&state)
            .iter()
            .map(|(_, t, f)| format!("{t} {f}"))
            .collect();
        let bound = bound.join(" ").to_lowercase();
        for word in TITLE_METRIC_WORDS {
            let word_lower = word.to_lowercase();
            if title.to_lowercase().contains(&word_lower) && !bound.contains(&word_lower) {
                let id = file_name(path.parent().unwrap());
                unsupported.push(format!("{id}: title says '{word}', no field binding does"));
            }
        }
    }
    assert!(titled > 20, "Expected titled visuals across all four pages");
    assert_eq!(unsupported, Vec::<String>::new());
}

/// A statement labelled consolidated must apply the consolidation calculation group,
/// which is the only binding that removes intercompany rows.
#[test]
fn only_consolidated_bindings_may_carry_a_consolidated_label() {
    let (mut claims, mut checked) = (Vec::new(), 0);
    for page_dir in page_dirs() {
        let files = visual_files(&page_dir);
        let consolidated = files.iter().any(|path| {
            let document = read_json(path);
            let bindings = document.py_get("visual").map(query_field_bindings).unwrap_or_default();
            bindings.iter().any(|(_, table, _)| table == "CalcGroup_Consolidation")
        });
        let page = read_json(&page_dir.join("page.json"));
        let mut labels = vec![(
            file_name(&page_dir),
            page.py_get("displayName").cloned().unwrap_or(Value::from("")),
        )];
        for path in &files {
            let title = visual_title(read_json(path).py_get("visual"));
            labels.push((file_name(path.parent().unwrap()), Value::from(title)));
        }
        for (name, label) in labels {
            let Some(label) = label.as_str().filter(|text| !text.is_empty()) else {
                continue;
            };
            checked += 1;
            if label.to_lowercase().contains("consolidat") && !consolidated {
                claims.push(format!("{name}: '{label}' claims consolidation with no binding"));
            }
        }
    }
    assert!(checked > 20, "Expected every page and titled visual to be checked");
    assert_eq!(claims, Vec::<String>::new());
}

// Report uniformity.

#[test]
fn result_cards_keep_shared_labels_surfaces_and_padding() {
    let visuals: Vec<Value> = all_visual_files().iter().map(|p| read_json(p)).collect();
    let cards: Vec<&Value> = visuals
        .iter()
        .filter(|v| v.at("visual").at("visualType") == "cardVisual")
        .collect();
    assert_eq!(cards.len(), 4);
    for card in &cards {
        let objects = card.at("visual").at("objects");
        for role in ["label", "fillCustom", "outline", "padding"] {
            assert_eq!(objects.at(role), cards[0].at("visual").at("objects").at(role), "{role}");
        }
        assert_eq!(
            literal(objects.at("layout").at(0).at("properties").at("cellPadding")),
            "12D"
        );
        let expected = if card.at("name") == "3fd74ef9d4436795bbdd" {
            "14D"
        } else {
            "28D"
        };
        assert_eq!(
            literal(objects.at("value").at(0).at("properties").at("fontSize")),
            expected
        );
    }
}

/// `document["visual"].get("query", {}).get("queryState", {}).values()`.
fn query_roles(document: &Value) -> Vec<&Value> {
    let Some(query) = document.at("visual").py_get("query") else {
        return Vec::new();
    };
    match query.py_get("queryState") {
        None => Vec::new(),
        Some(Value::Object(roles)) => roles.values().collect(),
        Some(other) => panic!("AttributeError: values on {other}"),
    }
}

#[test]
fn query_projection_aliases_are_unique() {
    for path in all_visual_files() {
        let document = read_json(&path);
        let mut aliases = Vec::new();
        for role in query_roles(&document) {
            let projections = role.py_get("projections").cloned().unwrap_or(Value::Array(Vec::new()));
            for projection in projections.as_array().expect("a projection list") {
                if py_in("nativeQueryRef", projection) {
                    aliases.push(projection.at("nativeQueryRef").to_string());
                }
            }
        }
        let label = name_of(&document);
        assert_eq!(aliases.len(), aliases.iter().collect::<BTreeSet<_>>().len(), "{label}");
    }
}

#[test]
fn visuals_follow_the_shared_grid_and_do_not_overlap() {
    for page_dir in page_dirs() {
        let visuals = visuals_of(&page_dir);
        for (index, visual) in visuals.iter().enumerate() {
            visual.at("position");
            for key in ["x", "y", "width", "height"] {
                let label = (name_of(visual), key);
                assert_eq!(position(visual, key) % 4.0, 0.0, "{label:?}");
            }
            let (x, y, w, h) = (
                position(visual, "x"),
                position(visual, "y"),
                position(visual, "width"),
                position(visual, "height"),
            );
            assert!(x >= 20.0 && x + w <= 1260.0 && y + h <= 700.0, "{}", name_of(visual));
            for other in &visuals[index + 1..] {
                let corner = |key| position(other, key);
                let overlap = x < corner("x") + corner("width")
                    && corner("x") < x + w
                    && y < corner("y") + corner("height")
                    && corner("y") < y + h;
                let label = (name_of(visual), name_of(other));
                assert!(!overlap, "{label:?}");
            }
        }
    }
}

#[test]
fn navigation_and_filters_keep_shared_geometry_and_style() {
    let mut navigators = Vec::new();
    for path in all_visual_files() {
        let document = read_json(&path);
        let visual = document.at("visual");
        let place = document.at("position");
        if visual.at("visualType") == "pageNavigator" {
            let alt = visual
                .at("visualContainerObjects")
                .at("general")
                .at(0)
                .at("properties")
                .at("altText");
            assert_eq!(
                literal(alt),
                "'Navigate report pages. Each page retains its own selections.'"
            );
            let geometry: Vec<f64> = ["x", "y", "width", "height"]
                .iter()
                .map(|key| num(place.at(*key)))
                .collect();
            assert_eq!(geometry, [520.0, 8.0, 740.0, 40.0]);
            navigators.push(visual.at("objects").clone());
        } else if visual.at("visualType") == "slicer" {
            let height = num(place.at("height"));
            let label = name_of(&document);
            assert_eq!(height, 60.0, "{label}");
            assert_eq!(
                literal(visual.at("objects").at("header").at(0).at("properties").at("show")),
                "false"
            );
        }
    }
    assert_eq!(navigators.len(), 5);
    for objects in &navigators[1..] {
        assert_eq!(objects, &navigators[0]);
    }
}

#[test]
fn tables_keep_shared_typography_and_row_styles() {
    for path in all_visual_files() {
        let document = read_json(&path);
        let visual = document.at("visual");
        if !["tableEx", "pivotTable"]
            .iter()
            .any(|kind| visual.at("visualType") == *kind)
        {
            continue;
        }
        let objects = visual.at("objects");
        for group in ["columnHeaders", "values"] {
            let size = literal(objects.at(group).at(0).at("properties").at("fontSize"));
            let label = name_of(&document);
            assert_eq!(size, "12D", "{label}");
        }
        let headers = objects.at("columnHeaders").at(0).at("properties");
        assert_eq!(literal(headers.at("backColor").at("solid").at("color")), "'#E7EDEB'");
        assert_eq!(
            literal(objects.at("grid").at(0).at("properties").at("rowPadding")),
            "2D"
        );
    }
}
