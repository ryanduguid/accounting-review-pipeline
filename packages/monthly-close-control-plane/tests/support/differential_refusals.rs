use super::*;

fn rejected(
    check: fn(&Corpus, &[Json]) -> Result<()>,
    cases: &[(&str, &str)],
    expected: &str,
) -> Result<()> {
    let mut corpus = Corpus(Vec::new());
    let mut responses = Vec::new();
    for (name, body) in cases {
        corpus.0.push(Json::object([("name", Json::str(name))]));
        responses.push(probe::decode(body)?);
    }
    assert_eq!(
        check(&corpus, &responses).unwrap_err().to_string(),
        expected
    );
    Ok(())
}

#[test]
fn legacy_financial_assertions_refuse_missing_incorrect_and_incomplete_results() -> Result<()> {
    let name = "review-equity-legacy-nil-movement";
    rejected(
        assert_legacy_financial_cases,
        &[(
            "render-review-equity-legacy-example",
            r#"{"ok":["a","b","c"]}"#,
        )],
        "legacy equity rendering failed for render-review-equity-legacy-example",
    )?;
    rejected(
        assert_legacy_financial_cases,
        &[(name, r#"{"error":"fabricated refusal"}"#)],
        &format!("legacy financial case {name} did not produce a result"),
    )?;
    rejected(
        assert_legacy_financial_cases,
        &[(
            name,
            r#"{"ok":{"status":"PASS","equity_reconciliation":{"total":{"movement":"1"}}}}"#,
        )],
        &format!("legacy financial outcome assertion failed for {name}"),
    )?;
    rejected(
        assert_legacy_financial_cases,
        &[(
            name,
            r#"{"ok":{"status":"PASS","equity_reconciliation":{"total":{"movement":"0"}}}}"#,
        )],
        "expected 13 legacy financial cases and 8 equity renders, checked 1 and 0",
    )
}

#[test]
fn legacy_comparison_assertions_refuse_missing_unknown_and_incorrect_findings() -> Result<()> {
    let name = "core-legacy-april-may";
    rejected(
        assert_legacy_comparison_cases,
        &[(name, r#"{"ok":{}}"#)],
        &format!("legacy comparison {name} did not produce findings"),
    )?;
    rejected(
        assert_legacy_comparison_cases,
        &[("core-legacy-unknown", r#"{"ok":{"exceptions":[]}}"#)],
        "unrecognised legacy comparison core-legacy-unknown",
    )?;
    rejected(
        assert_legacy_comparison_cases,
        &[(
            name,
            r#"{"ok":{"exceptions":[{"control":"financial_year_reset"}]}}"#,
        )],
        &format!("legacy comparison outcome assertion failed for {name}"),
    )?;
    rejected(
        assert_legacy_comparison_cases,
        &[(name, r#"{"ok":{"exceptions":[]}}"#)],
        "expected 5 legacy comparisons, checked 1",
    )
}

#[test]
fn legacy_equity_verification_refuses_missing_documents_and_wrong_status() -> Result<()> {
    let name = "verify-render-review-equity-legacy-nil-movement";
    for body in [r#"{}"#, r#"{"ok":{"verified":[]}}"#] {
        rejected(
            assert_legacy_equity_verification,
            &[(name, body)],
            &format!("legacy equity saved-pack verification failed for {name}"),
        )?;
    }
    rejected(
        assert_legacy_equity_verification,
        &[(
            name,
            r#"{"ok":{"verified":[{"equity_reconciliation":{"status":"REVIEW"}}]}}"#,
        )],
        &format!("legacy equity saved-pack status assertion failed for {name}"),
    )?;
    rejected(
        assert_legacy_equity_verification,
        &[(
            "verify-render-review-equity-legacy-aggregate-tolerance",
            r#"{"ok":{"verified":[{"equity_reconciliation":{"status":"REVIEW"},"client_queries":[{}]}]}}"#,
        )],
        "aggregate-only equity finding must not draft a client query",
    )
}

#[test]
fn legacy_equity_verification_requires_all_recognised_packs() -> Result<()> {
    rejected(
        assert_legacy_equity_verification,
        &[
            ("unrelated", "{}"),
            ("verify-render-review-equity-legacy-unknown", "{}"),
        ],
        "expected 8 verified legacy equity packs, checked 0",
    )?;
    rejected(
        assert_legacy_equity_verification,
        &[(
            "verify-render-review-equity-legacy-nil-movement",
            r#"{"ok":{"verified":[{"equity_reconciliation":{"status":"PASS"}}]}}"#,
        )],
        "expected 8 verified legacy equity packs, checked 1",
    )
}

#[test]
fn failed_rendering_is_not_turned_into_driver_or_read_utility_cases() -> Result<()> {
    let corpus = Corpus(vec![Json::object([(
        "name",
        Json::str("render-review-config-absolute-0"),
    )])]);
    for body in [r#"{"error":"fabricated refusal"}"#, r#"{"ok":{}}"#] {
        let responses = [probe::decode(body)?];
        assert!(driver_cases(&corpus, &responses)?.0.is_empty());
        assert!(read_utility_cases(&corpus, &responses)?.0.is_empty());
    }
    Ok(())
}

#[test]
fn fixture_field_updates_preserve_siblings_when_appending_and_replacing() -> Result<()> {
    let mut object = probe::decode(r#"{"keep":"sentinel","change":"old"}"#)?;
    put(&mut object, "new", Json::str("added"));
    put(&mut object, "change", Json::str("updated"));
    assert_eq!(
        probe::encode(&object),
        r#"{"change":"updated","keep":"sentinel","new":"added"}"#
    );
    Ok(())
}

#[test]
fn differential_refuses_wrong_argument_counts_and_missing_frozen_source() -> Result<()> {
    for count in [0, 1, 4] {
        let args = vec![std::ffi::OsString::from("unused"); count];
        assert_eq!(
            run(args).unwrap_err().to_string(),
            "usage: close-control-differential CPYTHON ORACLE_COMPONENT [SEED]"
        );
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(!root.join("closecontrol/cli.py").exists());
    assert_eq!(
        run(vec!["unused-python".into(), root.as_os_str().to_owned()])
            .unwrap_err()
            .to_string(),
        "frozen component source is missing"
    );
    Ok(())
}

#[test]
fn exchange_reports_the_real_child_stderr_and_reaps_successful_children() -> Result<()> {
    let version = Command::new("git").arg("--version").output()?;
    assert!(version.status.success());
    let mut command = Command::new("git");
    command.arg("--version");
    assert_eq!(
        exchange(command, "")?,
        String::from_utf8(version.stdout)?
            .lines()
            .collect::<Vec<_>>()
    );
    let missing = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("missing-differential-input-{}", std::process::id()));
    assert!(!missing.exists());
    let git = || {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&missing)
            .args(["rev-parse", "--show-toplevel"]);
        command
    };
    let failure = git().output()?;
    assert!(!failure.status.success());
    assert!(!failure.stderr.is_empty());
    assert_eq!(
        exchange(git(), "").unwrap_err().to_string(),
        format!(
            "comparison process failed: {}",
            String::from_utf8_lossy(&failure.stderr)
        )
    );
    Ok(())
}
