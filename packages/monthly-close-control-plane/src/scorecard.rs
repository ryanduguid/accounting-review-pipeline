// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! One client's close results summarised by category, with a score: the
//! results at `PASS` as a whole percentage of the results supplied.
use crate::{
    Result,
    loader::SourceSnapshot,
    pyjson::{self, Json},
    pypath, pytext,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub const BOUNDARY: &str = "Review evidence only. The score counts the results supplied: a check that was not run cannot lower it, and a result at PASS shows only what that check tested.";
pub const FORMULA: &str = "floor(100 * results at PASS / results supplied)";

struct Category {
    label: String,
    status: String,
    sha256: String,
    codes: BTreeMap<String, i64>,
    findings: i64,
}

impl Category {
    fn json(&self) -> Json {
        let codes = self.codes.iter();
        Json::object([
            ("category", Json::str(&self.label)),
            ("status", Json::str(&self.status)),
            ("findings", Json::int(self.findings)),
            (
                "codes",
                Json::Object(
                    codes
                        .map(|(code, count)| (code.as_str().into(), Json::int(*count)))
                        .collect(),
                ),
            ),
            ("source_sha256", Json::str(&self.sha256)),
        ])
    }
}

/// One result file, which must carry `status`, `tenant` and `findings` as the
/// register and schedule commands write them.
fn category(path: &Path) -> Result<(String, Category)> {
    let source = SourceSnapshot::capture(path, "Result")?;
    let name = pypath::text(path);
    let refuse = || {
        format!(
            "{name}: not a result of jet, duplicates, coding, fixed-assets or schedule (status, tenant and findings are needed)."
        )
    };
    let value = pyjson::loads(source.text("Result")?).map_err(|_| refuse())?;
    let text = |key| {
        value
            .get(key)
            .and_then(Json::as_pystr)
            .and_then(|text| text.to_utf8())
    };
    let (Some(status), Some(tenant), Some(findings)) = (
        text("status").filter(|status| ["PASS", "REVIEW"].contains(&status.as_str())),
        text("tenant"),
        value.get("findings").and_then(Json::as_array),
    ) else {
        return Err(refuse());
    };
    let mut codes = BTreeMap::new();
    for finding in findings {
        let code = ["code", "test"]
            .iter()
            .find_map(|key| finding.get(key).and_then(Json::as_pystr))
            .and_then(|code| code.to_utf8())
            .unwrap_or_else(|| "(no code)".into());
        *codes.entry(code).or_default() += 1;
    }
    let label = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    Ok((
        tenant,
        Category {
            label,
            status,
            sha256: source.sha256,
            codes,
            findings: findings.len() as i64,
        },
    ))
}

/// Summarise the results in `paths`, all for one tenant.
pub fn scorecard(paths: &[PathBuf]) -> Result<Json> {
    let mut tenant: Option<String> = None;
    let mut categories: Vec<Category> = Vec::new();
    for path in paths {
        let (name, category) = category(path)?;
        if *tenant.get_or_insert_with(|| name.clone()) != name {
            return Err(format!(
                "{}: tenant {} is not {}; summarise each tenant separately.",
                pypath::text(path),
                pytext::repr(&name),
                pytext::repr(tenant.as_deref().unwrap_or_default())
            ));
        }
        if categories.iter().any(|seen| seen.label == category.label) {
            return Err(format!(
                "{}: a second result named {}; give each result its own file name.",
                pypath::text(path),
                pytext::repr(&category.label)
            ));
        }
        categories.push(category);
    }
    let passed = categories.iter().filter(|c| c.status == "PASS").count();
    let total = categories.len();
    let score = (100 * passed).checked_div(total).unwrap_or_default();
    let results = categories.iter().map(Category::json).collect();
    let clear = total > 0 && passed == total;
    Ok(Json::object([
        ("schema_version", Json::int(1)),
        ("status", Json::str(if clear { "PASS" } else { "REVIEW" })),
        ("tenant", Json::str(tenant.as_deref().unwrap_or_default())),
        ("boundary", Json::str(BOUNDARY)),
        ("score_formula", Json::str(FORMULA)),
        ("score_percent", Json::int(score as i64)),
        ("results_supplied", Json::int(total as i64)),
        ("results_at_pass", Json::int(passed as i64)),
        (
            "findings_total",
            Json::int(categories.iter().map(|c| c.findings).sum()),
        ),
        ("results", Json::Array(results)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn write(dir: &TempDir, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    fn result(tenant: &str, status: &str, findings: &str) -> String {
        format!(r#"{{"status":"{status}","tenant":"{tenant}","findings":[{findings}]}}"#)
    }

    #[test]
    fn categories_are_counted_and_scored() {
        let dir = TempDir::new();
        let paths = [
            write(
                &dir,
                "duplicates.json",
                &result(
                    "Varrock",
                    "REVIEW",
                    r#"{"code":"DUPLICATE_BILL"},{"code":"DUPLICATE_BILL"},{"code":"MISSING_RECURRING_BILL"}"#,
                ),
            ),
            write(
                &dir,
                "jet.json",
                &result("Varrock", "REVIEW", r#"{"test":"weekend"},{}"#),
            ),
            write(&dir, "coding.json", &result("Varrock", "PASS", "")),
        ];
        let card = scorecard(&paths).unwrap();
        assert!(card.at("status").is_str("REVIEW"));
        assert_eq!(pyjson::canonical(card.at("score_percent")), "33");
        assert_eq!(pyjson::canonical(card.at("findings_total")), "5");
        let results = card.at("results").as_array().unwrap();
        assert!(results[0].at("category").is_str("duplicates"));
        let count = |result: &Json, code| pyjson::canonical(result.at("codes").at(code));
        assert_eq!(count(&results[0], "DUPLICATE_BILL"), "2");
        assert_eq!(count(&results[0], "MISSING_RECURRING_BILL"), "1");
        assert_eq!(count(&results[1], "(no code)"), "1");
        assert_eq!(count(&results[1], "weekend"), "1");
    }

    #[test]
    fn every_result_at_pass_scores_100_and_none_scores_0() {
        let dir = TempDir::new();
        let pass = [write(&dir, "a.json", &result("Varrock", "PASS", ""))];
        let card = scorecard(&pass).unwrap();
        assert!(card.at("status").is_str("PASS"));
        assert_eq!(pyjson::canonical(card.at("score_percent")), "100");
        let none = scorecard(&[]).unwrap();
        assert!(none.at("status").is_str("REVIEW"));
        assert_eq!(pyjson::canonical(none.at("score_percent")), "0");
    }

    #[test]
    fn unusable_results_are_refused() {
        let dir = TempDir::new();
        let refused = |paths: &[PathBuf], message: &str| {
            let error = scorecard(paths).unwrap_err();
            assert!(error.contains(message), "{error}");
        };
        let a = write(&dir, "a.json", &result("Varrock", "PASS", ""));
        refused(&[write(&dir, "b.json", "not json")], "not a result of");
        refused(
            &[write(&dir, "c.json", &result("Varrock", "DONE", ""))],
            "not a result of",
        );
        refused(
            &[write(
                &dir,
                "d.json",
                r#"{"status":"PASS","tenant":"Varrock"}"#,
            )],
            "not a result of",
        );
        refused(
            &[
                a.clone(),
                write(&dir, "e.json", &result("Falador", "PASS", "")),
            ],
            "summarise each tenant",
        );
        let other = dir.join("other");
        std::fs::create_dir(&other).unwrap();
        refused(
            &[
                a,
                write(&dir, "other/a.json", &result("Varrock", "PASS", "")),
            ],
            "a second result named 'a'",
        );
        refused(&[dir.join("absent.json")], "Result does not exist");
    }
}
