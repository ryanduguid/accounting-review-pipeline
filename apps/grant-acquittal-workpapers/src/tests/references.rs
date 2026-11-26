// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Every total on the workpaper names the source rows behind it.

use super::*;

/// The IDs a reference cell lists.
fn ids(value: &Json) -> Vec<String> {
    let Json::Array(items) = value else {
        panic!("a reference list, got {}", pyjson::repr(value))
    };
    items.iter().map(text).collect()
}

/// Each (total, its reference) pair a table row must carry.
fn total_cells(table: &str) -> Vec<(&'static str, &'static str)> {
    match table {
        "budget_lines" => vec![
            ("allocated", "allocated_ledger_rows"),
            ("within_period", "within_period_ledger_rows"),
            (
                "approved_evidenced_in_period",
                "approved_evidenced_in_period_ledger_rows",
            ),
            ("cash_allocated", "cash_allocated_ledger_rows"),
        ],
        "funding_movements" => vec![
            ("funding_received", "funding_received_receipt_rows"),
            ("expenditure_allocated", "expenditure_allocated_ledger_rows"),
            ("cash_paid_allocated", "cash_paid_allocated_ledger_rows"),
        ],
        "contributions" => vec![
            ("interest", "interest_contribution_rows"),
            ("cash_contribution", "cash_contribution_contribution_rows"),
            ("in_kind", "in_kind_contribution_rows"),
        ],
        _ => vec![
            ("ledger_total", "ledger_total_ledger_rows"),
            ("allocated_total", "allocated_total_ledger_rows"),
            ("unallocated_total", "unallocated_total_ledger_rows"),
        ],
    }
}

/// The references on one row, keyed by total, as `"total=ID ID"` strings.
fn references(row: &Json, table: &str) -> Vec<String> {
    total_cells(table)
        .into_iter()
        .map(|(total, reference)| format!("{total}={}", ids(row.at(reference)).join(" ")))
        .collect()
}

/// The example with fabricated contributions, so every table holds a total.
fn example_with_contributions() -> Json {
    let fixture = Fixture::new();
    std::fs::write(
        fixture.source().join("contributions.csv"),
        "contribution_id,grant_id,date,kind,amount,evidence\n\
         C1,G-A,2026-08-01,interest,12.5,Fabricated statement\n\
         C2,G-A,2026-09-01,interest,0.5,Fabricated statement\n\
         C3,G-B,2026-10-02,in_kind,300,Fabricated letter\n",
    )
    .unwrap();
    fixture.review().unwrap()
}

#[test]
fn every_ledger_and_budget_line_total_names_its_ledger_rows() {
    let result = example_with_contributions();
    assert_eq!(
        references(&result, "workpaper"),
        [
            "ledger_total=L1 L2 L3 L4",
            "allocated_total=L1 L2 L3 L4",
            "unallocated_total=L4",
        ]
    );
    let lines: Vec<Vec<String>> = rows(&result, "budget_lines")
        .iter()
        .map(|row| references(row, "budget_lines"))
        .collect();
    assert_eq!(
        lines,
        [
            // G-A Delivery: A1 (L1) is approved and evidenced; A4 (L3) is not approved and paid nothing.
            vec![
                "allocated=L1 L3",
                "within_period=L1 L3",
                "approved_evidenced_in_period=L1",
                "cash_allocated=L1",
            ],
            // G-A Administration: A5 allocates 300 of L4.
            vec![
                "allocated=L4",
                "within_period=L4",
                "approved_evidenced_in_period=L4",
                "cash_allocated=L4",
            ],
            // G-B Delivery: A3 (L2) predates the agreement.
            vec![
                "allocated=L1 L2",
                "within_period=L1",
                "approved_evidenced_in_period=L1",
                "cash_allocated=L1 L2",
            ],
            vec![
                "allocated=",
                "within_period=",
                "approved_evidenced_in_period=",
                "cash_allocated="
            ],
        ]
    );
}

#[test]
fn every_funding_and_contribution_total_names_its_rows() {
    let result = example_with_contributions();
    let movements: Vec<Vec<String>> = rows(&result, "funding_movements")
        .iter()
        .map(|row| references(row, "funding_movements"))
        .collect();
    assert_eq!(
        movements,
        [
            [
                "funding_received=R1",
                "expenditure_allocated=L1 L3 L4",
                "cash_paid_allocated=L1 L4"
            ],
            [
                "funding_received=R2",
                "expenditure_allocated=L1 L2",
                "cash_paid_allocated=L1 L2"
            ],
        ]
    );
    let contributed: Vec<Vec<String>> = rows(&result, "contributions")
        .iter()
        .map(|row| references(row, "contributions"))
        .collect();
    assert_eq!(
        contributed,
        [
            ["interest=C1 C2", "cash_contribution=", "in_kind="],
            ["interest=", "cash_contribution=", "in_kind=C3"],
        ]
    );
}

#[test]
fn a_total_is_zero_exactly_when_it_names_no_rows() {
    let result = example_with_contributions();
    let mut checked = 0;
    let tables = ["budget_lines", "funding_movements", "contributions"];
    let table_rows = tables
        .iter()
        .flat_map(|table| rows(&result, table).iter().map(move |row| (*table, row)));
    for (table, row) in table_rows.chain([("workpaper", &result)]) {
        for (total, reference) in total_cells(table) {
            let amount = workpaper::money(&text(row.at(total))).unwrap();
            assert_eq!(amount.is_zero(), ids(row.at(reference)).is_empty(), "{table} {total}");
            checked += 1;
        }
    }
    assert_eq!(checked, 4 * 4 + 2 * 3 + 2 * 3 + 3);
}

#[test]
fn a_ledger_row_split_across_lines_is_named_once_per_total() {
    let fixture = Fixture::new();
    fixture.edit_csv("allocations.csv", |rows| {
        // Move part of A1 to G-A Administration, which A5 (L4) already uses.
        put(&mut rows[0], "amount", "500");
        put(&mut rows[0], "cash_allocated", "400");
        let mut split = rows[0].clone();
        put(&mut split, "allocation_id", "A6");
        put(&mut split, "budget_line", "Administration");
        put(&mut split, "amount", "100");
        put(&mut split, "cash_allocated", "80");
        rows.push(split);
    });
    let result = fixture.review().unwrap();
    let grant_a = &rows(&result, "funding_movements")[0];
    assert_eq!(ids(grant_a.at("expenditure_allocated_ledger_rows")), ["L1", "L3", "L4"]);
    assert_eq!(ids(grant_a.at("cash_paid_allocated_ledger_rows")), ["L1", "L4"]);
    let admin = &rows(&result, "budget_lines")[1];
    assert!(admin.at("allocated").is_str("400"));
    assert_eq!(ids(admin.at("allocated_ledger_rows")), ["L4", "L1"]);
}

#[test]
fn grant_references_follow_allocation_order_not_budget_order() {
    let fixture = Fixture::new();
    // A5 (L4, Administration) now comes first, though Delivery is the first budget line.
    fixture.edit_csv("allocations.csv", |rows| {
        let administration = rows.remove(4);
        rows.insert(0, administration);
    });
    let result = fixture.review().unwrap();
    let grant_a = &rows(&result, "funding_movements")[0];
    assert_eq!(ids(grant_a.at("expenditure_allocated_ledger_rows")), ["L4", "L1", "L3"]);
    assert_eq!(ids(grant_a.at("cash_paid_allocated_ledger_rows")), ["L4", "L1"]);
}

#[test]
fn reference_cells_reach_the_csv_tables() {
    let fixture = Fixture::new();
    let result = fixture.review().unwrap();
    fixture.write(&result, "out").unwrap();
    let lines = read_csv(&fixture.root().join("out/budget_lines.csv"));
    assert_eq!(get(&lines[0], "allocated_ledger_rows"), "['L1', 'L3']");
    assert_eq!(get(&lines[3], "allocated_ledger_rows"), "[]");
    let movements = read_csv(&fixture.root().join("out/funding_movements.csv"));
    assert_eq!(get(&movements[1], "funding_received_receipt_rows"), "['R2']");
}
