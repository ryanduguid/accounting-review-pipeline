// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Joined offline conformance for the data-only Xero trial-balance contract:
//! every offline review consumer keeps the root fixtures' accept/reject
//! boundary, identity and account codes.

use super::*;
use monthly_close_control_plane::decimal::Decimal;

const CONTRACT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../contracts/xero-trial-balance-v1"
);
const TENANT: &str = "Catherby Fisheries Pty Ltd";
// Tenant plus the stable Xero AccountID is the control identity in every consumer.
const IDENTITY: [(&str, &str); 2] = [
    (TENANT, "00000000-0000-0000-0000-000000000001"),
    (TENANT, "00000000-0000-0000-0000-000000000002"),
];
// AccountCode is presentation metadata; the leading zero must survive as text.
const ACCOUNT_CODES: [&str; 2] = ["090", "800"];

fn decimal(text: &str) -> Decimal {
    Decimal::parse(text).unwrap()
}

/// `Decimal(value) == want`.
fn same(value: &Json, want: &Decimal) -> bool {
    decimal(&text(value)).cmp_value(want).is_eq()
}

fn assert_rows(rows: &Json, fixture: &str) {
    let rows = rows.as_array().unwrap();
    let identity: Vec<(String, String)> = rows
        .iter()
        .map(|row| (text(row.at("tenant")), text(row.at("account_id"))))
        .collect();
    let want = IDENTITY.map(|(tenant, id)| (tenant.to_owned(), id.to_owned()));
    assert_eq!(identity, want, "{fixture}");
    let codes: Vec<String> = rows
        .iter()
        .map(|row| text(row.at("account_code")))
        .collect();
    assert_eq!(codes, ACCOUNT_CODES, "{fixture}");
}

#[test]
fn all_offline_review_consumers_keep_the_same_accept_reject_boundary() {
    for (name, accept, movement, ytd) in [
        ("passing.csv", true, "0", "0"),
        ("failing_movement.csv", false, "0.01", "0"),
        ("failing_ytd.csv", false, "0", "0.01"),
    ] {
        let (movement, ytd) = (decimal(movement), decimal(ytd));
        let fixture = Path::new(CONTRACT).join("fixtures").join(name);

        // Readiness gate: exact Decimal movement and YTD differences, no tolerance.
        let request = Json::object([
            ("op", Json::str("tb_integrity")),
            ("path", path_text(&fixture)),
        ]);
        let readiness = ready(&request);
        let readiness = readiness.at("ok");
        assert!(same(readiness.at("movement"), &movement), "{name}");
        assert!(same(readiness.at("ytd"), &ytd), "{name}");
        assert_rows(readiness.at("rows"), name);

        // Monthly close: a blocked integrity exception carries the exact 0.01 difference.
        let close = monthly(&file_request("tb_integrity", None, &fixture));
        let close = close.at("ok");
        let findings = close.at("findings").as_array().unwrap();
        assert_eq!(findings.is_empty(), accept, "{name}");
        if !accept {
            let difference = movement.add(&ytd).value;
            assert_eq!(findings.len(), 1, "{name}");
            assert!(findings[0].at("status").is_str("BLOCKED"), "{name}");
            assert!(same(findings[0].at("difference"), &difference), "{name}");
        }
        assert_rows(close.at("rows"), name);
    }
}
