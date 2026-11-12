// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use xero_trial_balance_export::pyjson::{self, Json};

pub fn balanced_report() -> Json {
    let fixture = pyjson::loads(include_str!("../fixtures/python-value-cases.json")).unwrap();
    let Json::Array(cases) = fixture.at("cases") else {
        panic!("fixture cases must be an array")
    };
    let case = cases
        .iter()
        .find(|case| {
            case.at("test").is_str(
                "test_cli_characterisation.ReportFailures.test_final_output_rejection_stops_before_building_rows",
            ) && case.at("request").at("op").is_str("flatten_report")
        })
        .unwrap();
    let Json::Array(args) = case.at("request").at("args") else {
        panic!("fixture arguments must be an array")
    };
    args[0].clone()
}
